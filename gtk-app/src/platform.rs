//! Linux desktop integrations: autostart, sleep inhibit, Send-to.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const INVALID_NAME_CHARS: &str = r#"<>:"/\|?*"#;

pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if INVALID_NAME_CHARS.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect()
}

pub fn send_to_display_name(remote: &str, path: Option<&str>) -> String {
    let path_suffix = path
        .filter(|p| !p.is_empty() && *p != "/")
        .map(|p| {
            format!(
                " - {}",
                p.trim_start_matches('/').replace(['/', '\\'], " - ")
            )
        })
        .unwrap_or_default();
    sanitize_name(&format!("{remote}{path_suffix} (RClone Manager)"))
}

fn apply_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut content = template.to_string();
    for &(key, value) in replacements {
        content = content.replace(&format!("{{{key}}}"), value);
    }
    content
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn write_executable(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

pub fn current_exe_quoted() -> String {
    std::env::current_exe()
        .map(|p| format!("\"{}\"", p.display()))
        .unwrap_or_else(|_| "rclone-manager-gtk".into())
}

pub fn autostart_desktop_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("autostart/rclone-manager-gtk.desktop")
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let path = autostart_desktop_path();
    if !enabled {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let exec = current_exe_quoted();
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Rclone Manager\nComment=Manage rclone remotes, mounts, and transfers\nExec={exec}\nIcon=folder-remote\nTerminal=false\nCategories=Network;FileTransfer;\nX-GNOME-Autostart-enabled=true\n"
    );
    write_executable(&path, &desktop).map_err(|e| e.to_string())
}

pub fn autostart_enabled() -> bool {
    autostart_desktop_path().exists()
}

#[derive(Debug, Default)]
pub struct PowerInhibitor {
    child: Option<Child>,
}

impl PowerInhibitor {
    pub fn new() -> Self {
        Self { child: None }
    }

    pub fn is_inhibited(&self) -> bool {
        self.child.is_some()
    }

    pub fn update(&mut self, should_inhibit: bool, reason: &str) {
        if should_inhibit {
            self.acquire(reason);
        } else {
            self.release();
        }
    }

    pub fn acquire(&mut self, reason: &str) {
        if self.child.is_some() {
            return;
        }
        match Command::new("systemd-inhibit")
            .args([
                "--what=idle:sleep:shutdown",
                "--who=Rclone Manager",
                "--why",
                reason,
                "--mode=block",
                "sleep",
                "infinity",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                log::info!("power inhibitor acquired: {reason}");
                self.child = Some(child);
            }
            Err(err) => log::warn!("systemd-inhibit unavailable: {err}"),
        }
    }

    pub fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            log::info!("power inhibitor released");
        }
    }
}

impl Drop for PowerInhibitor {
    fn drop(&mut self) {
        self.release();
    }
}

const NAUTILUS_SCRIPT: &str = include_str!("../../src-tauri/resources/send_to/nautilus_script.sh");
const DOLPHIN_DESKTOP: &str =
    include_str!("../../src-tauri/resources/send_to/dolphin_action.desktop");
const NEMO_ACTION: &str = include_str!("../../src-tauri/resources/send_to/nemo_action.nemo_action");

pub fn register_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let name = send_to_display_name(remote, path);
    let exec = current_exe_quoted();
    let path_val = path.unwrap_or("");
    let home = home_dir();
    let replacements = [
        ("exec_path", exec.as_str()),
        ("remote", remote),
        ("path", path_val),
        ("name", name.as_str()),
    ];

    write_executable(
        &home.join(".local/share/nautilus/scripts").join(&name),
        &apply_template(NAUTILUS_SCRIPT, &replacements),
    )
    .map_err(|e| e.to_string())?;
    write_executable(
        &home
            .join(".local/share/kio/servicemenus")
            .join(format!("{name}.desktop")),
        &apply_template(DOLPHIN_DESKTOP, &replacements),
    )
    .map_err(|e| e.to_string())?;
    write_executable(
        &home
            .join(".local/share/nemo/actions")
            .join(format!("{name}.nemo_action")),
        &apply_template(NEMO_ACTION, &replacements),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn unregister_send_to(remote: &str, path: Option<&str>) -> Result<(), String> {
    let name = send_to_display_name(remote, path);
    let home = home_dir();
    let _ = std::fs::remove_file(home.join(".local/share/nautilus/scripts").join(&name));
    let _ = std::fs::remove_file(
        home.join(".local/share/kio/servicemenus")
            .join(format!("{name}.desktop")),
    );
    let _ = std::fs::remove_file(
        home.join(".local/share/nemo/actions")
            .join(format!("{name}.nemo_action")),
    );
    Ok(())
}

pub fn is_send_to_registered(remote: &str, path: Option<&str>) -> bool {
    let name = send_to_display_name(remote, path);
    let home = home_dir();
    home.join(".local/share/nautilus/scripts")
        .join(&name)
        .exists()
        || home
            .join(".local/share/kio/servicemenus")
            .join(format!("{name}.desktop"))
            .exists()
        || home
            .join(".local/share/nemo/actions")
            .join(format!("{name}.nemo_action"))
            .exists()
}

#[derive(Debug, Clone, Default)]
pub struct SendToArgs {
    pub remote: String,
    pub path: String,
    pub files: Vec<PathBuf>,
}

pub fn parse_send_to_args(args: &[String]) -> Option<SendToArgs> {
    let mut remote = None;
    let mut path = String::new();
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--send-to-remote" => {
                i += 1;
                remote = args.get(i).cloned();
            }
            "--send-to-path" => {
                i += 1;
                path = args.get(i).cloned().unwrap_or_default();
            }
            other if other.starts_with('-') => {}
            _ if i == 0 => {}
            other => files.push(PathBuf::from(other)),
        }
        i += 1;
    }
    remote.map(|remote| SendToArgs {
        remote,
        path,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_and_names_send_to() {
        assert_eq!(sanitize_name(r#"a/b:c"#), "a-b-c");
        assert_eq!(
            send_to_display_name("drive", Some("Photos/2024")),
            "drive - Photos - 2024 (RClone Manager)"
        );
        assert_eq!(
            send_to_display_name("drive", None),
            "drive (RClone Manager)"
        );
    }

    #[test]
    fn parses_send_to_cli() {
        let args = [
            "rclone-manager-gtk".into(),
            "--send-to-remote".into(),
            "gdrive".into(),
            "--send-to-path".into(),
            "Inbox".into(),
            "/tmp/a.txt".into(),
            "/tmp/b.txt".into(),
        ];
        let parsed = parse_send_to_args(&args).unwrap();
        assert_eq!(parsed.remote, "gdrive");
        assert_eq!(parsed.path, "Inbox");
        assert_eq!(parsed.files.len(), 2);
        assert!(parse_send_to_args(&["app".into()]).is_none());
    }

    #[test]
    fn templates_substitute_placeholders() {
        let out = apply_template("x {remote} {path}", &[("remote", "a"), ("path", "b")]);
        assert_eq!(out, "x a b");
    }
}
