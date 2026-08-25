//! Diagnose rclone / engine / FUSE / config issues — mirrors Angular `RepairService`.

use crate::rclone::RcClient;
use crate::settings::AppSettings;
use crate::updater::version_is_newer;
use std::path::Path;

/// Oldest rclone the GTK client expects (matches typical RC `options/info` surface).
pub const MIN_RCLONE_VERSION: &str = "1.65.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairKind {
    MissingBinary,
    VersionTooOld,
    PasswordRequired,
    FuseMissing,
    EngineUnreachable,
    ConfigUnreadable,
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
    if !fuse_available() {
        issues.push(RepairIssue {
            kind: RepairKind::FuseMissing,
            title: "FUSE is not available".into(),
            detail: "Mounts need /dev/fuse or fusermount3 (install fuse3).".into(),
            action: "How to install FUSE".into(),
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
            }
        }
    }
    issues
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
    }
}
