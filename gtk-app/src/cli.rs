//! Desktop CLI flags matching Tauri `core/cli.rs` `GeneralArgs`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static START_HIDDEN: AtomicBool = AtomicBool::new(false);
static LAUNCH_ARGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliArgs {
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
    pub tray: bool,
    pub hidden: bool,
    pub tray_action: Option<String>,
}

impl CliArgs {
    pub fn start_hidden(&self) -> bool {
        self.tray || self.hidden
    }
}

/// Parse `--data-dir`, `--cache-dir`, `--logs-dir`, `--tray`, and `--hidden`.
pub fn parse_cli_args(args: &[String]) -> CliArgs {
    let mut parsed = CliArgs::default();
    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(value) = value_of(arg, "--data-dir") {
            parsed.data_dir = Some(PathBuf::from(value));
        } else if arg == "--data-dir" {
            i += 1;
            if let Some(value) = args.get(i) {
                parsed.data_dir = Some(PathBuf::from(value));
            }
        } else if let Some(value) = value_of(arg, "--cache-dir") {
            parsed.cache_dir = Some(PathBuf::from(value));
        } else if arg == "--cache-dir" {
            i += 1;
            if let Some(value) = args.get(i) {
                parsed.cache_dir = Some(PathBuf::from(value));
            }
        } else if let Some(value) = value_of(arg, "--logs-dir") {
            parsed.logs_dir = Some(PathBuf::from(value));
        } else if arg == "--logs-dir" {
            i += 1;
            if let Some(value) = args.get(i) {
                parsed.logs_dir = Some(PathBuf::from(value));
            }
        } else if arg == "--tray" {
            parsed.tray = true;
        } else if arg == "--hidden" {
            parsed.hidden = true;
        } else if let Some(value) = value_of(arg, "--tray-action") {
            parsed.tray_action = Some(value.to_string());
        } else if arg == "--tray-action" {
            i += 1;
            if let Some(value) = args.get(i) {
                parsed.tray_action = Some(value.clone());
            }
        }
        i += 1;
    }
    parsed
}

pub fn apply(args: &CliArgs) {
    START_HIDDEN.store(args.start_hidden(), Ordering::SeqCst);
    crate::settings::set_path_overrides(
        resolve_override(
            args.data_dir.as_deref(),
            std::env::var("RCLONE_MANAGER_DATA_DIR").ok().as_deref(),
            None,
        ),
        resolve_override(
            args.cache_dir.as_deref(),
            std::env::var("RCLONE_MANAGER_CACHE_DIR").ok().as_deref(),
            None,
        ),
        resolve_override(
            args.logs_dir.as_deref(),
            std::env::var("RCLONE_MANAGER_LOG_DIR").ok().as_deref(),
            None,
        ),
    );
}

pub fn start_hidden() -> bool {
    START_HIDDEN.load(Ordering::SeqCst)
}

pub fn set_launch_args(args: Vec<String>) {
    *LAUNCH_ARGS.lock().unwrap_or_else(|e| e.into_inner()) = args;
}

/// Decode one GApplication `options_dict` value without assuming the variant is a string.
/// Boolean flags such as `--auto-add` panic in glib if they are read as bytestrings first.
pub fn option_flag_from_variant(
    name: &str,
    value: &glib::Variant,
) -> Option<(String, Option<String>)> {
    if *value.type_() == *glib::VariantTy::BOOLEAN {
        return value
            .get::<bool>()
            .filter(|on| *on)
            .map(|_| (name.to_string(), None));
    }
    let text = value
        .str()
        .map(ToOwned::to_owned)
        .or_else(|| value.get::<String>())
        .or_else(|| {
            value
                .get::<PathBuf>()
                .map(|p| p.to_string_lossy().into_owned())
        })?;
    Some((
        name.to_string(),
        if text.is_empty() { None } else { Some(text) },
    ))
}

/// Re-insert GIO-consumed flags so a second instance can deep-link the primary.
///
/// Flags are inserted after argv0 so leftover tokens stay as values. GLib
/// `OptionArg::None` consumes `--about` / `--preferences` and leaves `rclone`
/// in leftover; appending the flag would yield `app rclone --about` and drop
/// the page.
pub fn merge_option_flags(args: &mut Vec<String>, flags: &[(String, Option<String>)]) {
    if args.is_empty() {
        return;
    }
    let mut insert_at = 1;
    for (name, value) in flags {
        let flag = format!("--{name}");
        if args
            .iter()
            .any(|arg| arg == &flag || arg.starts_with(&format!("{flag}=")))
        {
            continue;
        }
        args.insert(insert_at, flag);
        insert_at += 1;
        if let Some(value) = value {
            if !value.is_empty() {
                args.insert(insert_at, value.clone());
                insert_at += 1;
            }
        }
    }
}

/// Args from `GApplication` command-line, or `std::env::args()` on first launch.
pub fn launch_args() -> Vec<String> {
    let stored = LAUNCH_ARGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if stored.is_empty() {
        std::env::args().collect()
    } else {
        stored
    }
}

/// CLI path wins, then a non-empty environment value, then `fallback`.
pub fn resolve_override(
    cli: Option<&Path>,
    env_val: Option<&str>,
    fallback: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = cli.filter(|p| !p.as_os_str().is_empty()) {
        return Some(path.to_path_buf());
    }
    if let Some(value) = env_val.filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(value));
    }
    fallback
}

fn value_of<'a>(arg: &'a str, flag: &str) -> Option<&'a str> {
    arg.strip_prefix(flag)?.strip_prefix('=')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path_and_tray_flags() {
        let args = parse_cli_args(&[
            "rclone-manager-gtk".into(),
            "--data-dir".into(),
            "/data".into(),
            "--cache-dir=/cache".into(),
            "--logs-dir".into(),
            "/logs".into(),
            "--tray".into(),
        ]);
        assert_eq!(args.data_dir, Some(PathBuf::from("/data")));
        assert_eq!(args.cache_dir, Some(PathBuf::from("/cache")));
        assert_eq!(args.logs_dir, Some(PathBuf::from("/logs")));
        assert!(args.tray);
        assert!(!args.hidden);
        assert!(args.start_hidden());
        let action = parse_cli_args(&["app".into(), "--tray-action".into(), "show-window".into()]);
        assert_eq!(action.tray_action.as_deref(), Some("show-window"));
        assert_eq!(
            parse_cli_args(&["app".into(), "--tray-action=quit".into()])
                .tray_action
                .as_deref(),
            Some("quit")
        );
    }

    #[test]
    fn hidden_flag_starts_in_tray() {
        let args = parse_cli_args(&["app".into(), "--hidden".into()]);
        assert!(args.hidden);
        assert!(args.start_hidden());
        assert!(!parse_cli_args(&["app".into()]).start_hidden());
    }

    #[test]
    fn resolve_prefers_cli_then_env() {
        assert_eq!(
            resolve_override(Some(Path::new("/cli")), Some("/env"), None),
            Some(PathBuf::from("/cli"))
        );
        assert_eq!(
            resolve_override(None, Some("/env"), Some(PathBuf::from("/def"))),
            Some(PathBuf::from("/env"))
        );
        assert_eq!(
            resolve_override(None, Some(""), Some(PathBuf::from("/def"))),
            Some(PathBuf::from("/def"))
        );
        assert_eq!(resolve_override(None, None, None), None);
    }

    #[test]
    fn launch_args_prefer_stored() {
        set_launch_args(vec!["app".into(), "--browse".into(), "drive:".into()]);
        assert_eq!(
            launch_args(),
            vec!["app".to_string(), "--browse".into(), "drive:".into()]
        );
        set_launch_args(Vec::new());
    }

    #[test]
    fn merges_gio_consumed_deep_link_flags() {
        let mut args = vec!["app".into()];
        merge_option_flags(
            &mut args,
            &[
                ("about".into(), None),
                ("preferences".into(), Some("developer".into())),
            ],
        );
        assert_eq!(
            args,
            vec![
                "app".to_string(),
                "--about".into(),
                "--preferences".into(),
                "developer".into()
            ]
        );
        merge_option_flags(&mut args, &[("about".into(), None)]);
        assert_eq!(args.iter().filter(|arg| *arg == "--about").count(), 1);
    }

    #[test]
    fn inserts_consumed_flags_before_leftover_page_tokens() {
        let mut about = vec!["app".into(), "rclone".into()];
        merge_option_flags(&mut about, &[("about".into(), None)]);
        assert_eq!(
            about,
            vec!["app".to_string(), "--about".into(), "rclone".into()]
        );
        assert_eq!(
            crate::navigation::parse_launch_args(&about, false),
            Some(crate::navigation::LaunchRequest {
                target: crate::navigation::NavTarget::About {
                    page: Some("about-rclone".into())
                },
                standalone: false,
            })
        );

        let mut prefs = vec!["app".into(), "security".into()];
        merge_option_flags(&mut prefs, &[("preferences".into(), None)]);
        assert_eq!(
            prefs,
            vec!["app".to_string(), "--preferences".into(), "security".into()]
        );
        assert_eq!(
            crate::navigation::parse_launch_args(&prefs, false),
            Some(crate::navigation::LaunchRequest {
                target: crate::navigation::NavTarget::Preferences {
                    page: Some("security".into())
                },
                standalone: false,
            })
        );

        let mut both = vec!["app".into(), "rclone".into(), "security".into()];
        merge_option_flags(
            &mut both,
            &[("about".into(), None), ("preferences".into(), None)],
        );
        assert_eq!(
            both,
            vec![
                "app".to_string(),
                "--about".into(),
                "--preferences".into(),
                "rclone".into(),
                "security".into()
            ]
        );
    }

    #[test]
    fn merges_dialog_flags_for_restore_preview() {
        let mut args = vec!["app".into()];
        merge_option_flags(
            &mut args,
            &[
                ("dialog".into(), Some("restore-preview".into())),
                (
                    "dialog-data".into(),
                    Some(r#"{"path":"/tmp/rclone-manager-gui-backup.zip"}"#.into()),
                ),
            ],
        );
        let req = crate::platform::parse_dialog_args(&args).expect("dialog request");
        assert_eq!(req.kind, "restore-preview");
        assert_eq!(req.data["path"], "/tmp/rclone-manager-gui-backup.zip");
    }

    #[test]
    fn option_flag_reads_bool_before_string() {
        use glib::prelude::ToVariant;
        assert_eq!(
            option_flag_from_variant("auto-add", &true.to_variant()),
            Some(("auto-add".into(), None))
        );
        assert_eq!(
            option_flag_from_variant("auto-add", &false.to_variant()),
            None
        );
        assert_eq!(
            option_flag_from_variant("logs", &"testdrive".to_variant()),
            Some(("logs".into(), Some("testdrive".into())))
        );
        assert_eq!(
            option_flag_from_variant("logs", &"".to_variant()),
            Some(("logs".into(), None))
        );
    }
}
