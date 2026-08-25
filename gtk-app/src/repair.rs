//! Diagnose rclone / engine / FUSE / config issues — mirrors Angular `RepairService`.

use crate::rclone::RcClient;
use crate::settings::AppSettings;
use crate::updater::version_is_newer;
use std::path::Path;

/// Oldest rclone the GTK client expects (matches typical RC `options/info` surface).
pub const MIN_RCLONE_VERSION: &str = "1.65.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairKind {
    MissingBinary,
    VersionTooOld,
    PasswordRequired,
    FuseMissing,
    EngineUnreachable,
    ConfigUnreadable,
    AuthFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairIssue {
    pub kind: RepairKind,
    pub title: String,
    pub detail: String,
    pub action: String,
}

pub fn fuse_available() -> bool {
    Path::new("/dev/fuse").exists()
        || which::which("fusermount3").is_ok()
        || which::which("fusermount").is_ok()
}

pub fn config_path_from_flags(flags: &[String]) -> Option<String> {
    flags.iter().find_map(|flag| {
        flag.strip_prefix("--config=")
            .or_else(|| flag.strip_prefix("--config"))
            .map(|s| s.trim().trim_start_matches('=').to_string())
            .filter(|s| !s.is_empty())
    })
}

pub fn looks_like_password_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("password") || lower.contains("encrypted") || lower.contains("decrypt")
}

pub fn looks_like_auth_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("http 401") || lower.contains("unauthorized") || lower.contains("authentication")
}

pub fn fuse_install_hint() -> &'static str {
    "On Debian/Ubuntu: sudo apt install fuse3\nOn Fedora: sudo dnf install fuse3\nThen restart the session so /dev/fuse is available."
}

pub fn try_install_fuse() -> Result<String, String> {
    let (bin, args): (&str, &[&str]) = if which::which("apt-get").is_ok() {
        ("apt-get", &["install", "-y", "fuse3"])
    } else if which::which("dnf").is_ok() {
        ("dnf", &["install", "-y", "fuse3"])
    } else if which::which("pacman").is_ok() {
        ("pacman", &["-S", "--noconfirm", "fuse3"])
    } else {
        return Err(fuse_install_hint().into());
    };
    let helper = if which::which("pkexec").is_ok() {
        "pkexec"
    } else {
        bin
    };
    let mut cmd = std::process::Command::new(helper);
    if helper == "pkexec" {
        cmd.arg(bin);
    }
    let status = cmd.args(args).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok("FUSE package install finished. Re-check /dev/fuse.".into())
    } else {
        Err(format!(
            "Installer exited with {status}. {}",
            fuse_install_hint()
        ))
    }
}

pub fn diagnose(
    settings: &AppSettings,
    engine_ready: bool,
    client: Option<&RcClient>,
    version: Option<&str>,
) -> Vec<RepairIssue> {
    let mut issues = Vec::new();
    let binary = settings.core.rclone_binary.clone();
    if !crate::rclone::rclone_exists(&binary) {
        issues.push(RepairIssue {
            kind: RepairKind::MissingBinary,
            title: "rclone binary missing".into(),
            detail: if binary.is_empty() {
                "rclone was not found on PATH.".into()
            } else {
                format!("Configured binary {binary} is not executable.")
            },
            action: "Install rclone".into(),
        });
    }
    if let Some(ver) = version {
        if version_is_newer(MIN_RCLONE_VERSION, ver) {
            issues.push(RepairIssue {
                kind: RepairKind::VersionTooOld,
                title: "rclone is too old".into(),
                detail: format!("{ver} is older than {MIN_RCLONE_VERSION}."),
                action: "Update rclone".into(),
            });
        }
    }
    if !crate::mount_plugin::is_installed() {
        issues.push(RepairIssue {
            kind: RepairKind::FuseMissing,
            title: crate::mount_plugin::missing_title(),
            detail: crate::mount_plugin::missing_detail(),
            action: format!("Install {}", crate::mount_plugin::plugin_label()),
        });
    }
    if let Some(path) = config_path_from_flags(&settings.core.rclone_additional_flags) {
        if !Path::new(&path).is_file() {
            issues.push(RepairIssue {
                kind: RepairKind::ConfigUnreadable,
                title: "rclone.conf is missing".into(),
                detail: format!("{path} does not exist."),
                action: "Choose rclone.conf".into(),
            });
        }
    }
    if !engine_ready {
        issues.push(RepairIssue {
            kind: RepairKind::EngineUnreachable,
            title: "Engine is offline".into(),
            detail: "The local rclone RC or selected extra backend is not responding.".into(),
            action: "Restart engine".into(),
        });
    } else if let Some(client) = client {
        if let Err(err) = client.list_remotes() {
            let text = err.to_string();
            if looks_like_password_error(&text) && settings.core.config_password.is_empty() {
                issues.push(RepairIssue {
                    kind: RepairKind::PasswordRequired,
                    title: "Config password required".into(),
                    detail: text,
                    action: "Set password".into(),
                });
            } else if looks_like_auth_error(&text) {
                issues.push(RepairIssue {
                    kind: RepairKind::AuthFailed,
                    title: "RC authentication failed".into(),
                    detail: text,
                    action: "Edit backend credentials".into(),
                });
            }
        }
    }
    issues
}

pub fn banner_from_issues(issues: &[RepairIssue]) -> Option<&RepairIssue> {
    const ORDER: &[RepairKind] = &[
        RepairKind::MissingBinary,
        RepairKind::PasswordRequired,
        RepairKind::AuthFailed,
        RepairKind::EngineUnreachable,
        RepairKind::VersionTooOld,
        RepairKind::FuseMissing,
        RepairKind::ConfigUnreadable,
    ];
    for kind in ORDER {
        if let Some(issue) = issues.iter().find(|i| i.kind == *kind) {
            return Some(issue);
        }
    }
    issues.first()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_missing_binary() {
        let mut settings = AppSettings::default();
        settings.core.rclone_binary = "/definitely/missing/rclone".into();
        let issues = diagnose(&settings, false, None, None);
        assert!(issues.iter().any(|i| i.kind == RepairKind::MissingBinary));
        assert!(issues
            .iter()
            .any(|i| i.kind == RepairKind::EngineUnreachable));
    }

    #[test]
    fn detects_old_version() {
        let settings = AppSettings::default();
        let issues = diagnose(&settings, true, None, Some("v1.60.0"));
        assert!(issues.iter().any(|i| i.kind == RepairKind::VersionTooOld));
        assert!(!issues
            .iter()
            .any(|i| i.kind == RepairKind::EngineUnreachable));
    }

    #[test]
    fn parses_config_flag() {
        assert_eq!(
            config_path_from_flags(&["--log-level=INFO".into(), "--config=/tmp/r.conf".into()]),
            Some("/tmp/r.conf".into())
        );
        assert_eq!(config_path_from_flags(&["--verbose".into()]), None);
    }

    #[test]
    fn password_error_words() {
        assert!(looks_like_password_error("Failed to decrypt config"));
        assert!(looks_like_password_error("password required"));
        assert!(!looks_like_password_error("connection refused"));
        assert!(looks_like_auth_error("HTTP 401: unauthorized"));
        assert!(!looks_like_auth_error("HTTP 500: boom"));
    }

    #[test]
    fn banner_prefers_missing_binary() {
        let issues = vec![
            RepairIssue {
                kind: RepairKind::FuseMissing,
                title: "fuse".into(),
                detail: String::new(),
                action: String::new(),
            },
            RepairIssue {
                kind: RepairKind::MissingBinary,
                title: "binary".into(),
                detail: String::new(),
                action: String::new(),
            },
        ];
        assert_eq!(
            banner_from_issues(&issues).map(|i| i.kind),
            Some(RepairKind::MissingBinary)
        );
        assert!(banner_from_issues(&[]).is_none());
    }
}
