//! Desktop CLI flags matching Tauri `core/cli.rs` `GeneralArgs`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static START_HIDDEN: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliArgs {
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub logs_dir: Option<PathBuf>,
    pub tray: bool,
    pub hidden: bool,
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
}
