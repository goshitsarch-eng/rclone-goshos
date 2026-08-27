//! Angular `installation-options` — Recommended / Custom / Existing install or config paths.

use crate::validators::validate_absolute_path;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallLocation {
    Default,
    Custom,
    Existing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationMode {
    Install,
    Config,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryStatus {
    Untested,
    Testing,
    Valid,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationOptionsData {
    pub location: InstallLocation,
    pub custom_path: String,
    pub existing_binary: String,
    pub binary_status: BinaryStatus,
}

impl Default for InstallationOptionsData {
    fn default() -> Self {
        Self {
            location: InstallLocation::Default,
            custom_path: String::new(),
            existing_binary: String::new(),
            binary_status: BinaryStatus::Untested,
        }
    }
}

pub fn default_install_dest() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/bin")
}

pub fn rclone_install_dest(location: InstallLocation, custom: &str) -> Option<PathBuf> {
    match location {
        InstallLocation::Default => Some(default_install_dest()),
        InstallLocation::Custom => {
            let trimmed = custom.trim();
            if trimmed.is_empty() || !path_is_absolute(trimmed) {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        }
        InstallLocation::Existing => None,
    }
}

pub fn path_is_absolute(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && validate_absolute_path(trimmed).is_ok()
}

pub fn installation_valid(mode: InstallationMode, data: &InstallationOptionsData) -> bool {
    match data.location {
        InstallLocation::Default => true,
        InstallLocation::Custom => path_is_absolute(&data.custom_path),
        InstallLocation::Existing => {
            mode == InstallationMode::Install
                && path_is_absolute(&data.existing_binary)
                && data.binary_status == BinaryStatus::Valid
        }
    }
}

/// Primary action label keys for install mode (Angular `getInstallModeButtonTextKey`).
pub fn install_action_key(data: &InstallationOptionsData) -> &'static str {
    match data.location {
        InstallLocation::Default => "repairSheet.actions.installRclone",
        InstallLocation::Custom if data.custom_path.trim().is_empty() => {
            "repairSheet.buttons.selectPathFirst"
        }
        InstallLocation::Custom => "repairSheet.actions.installRclone",
        InstallLocation::Existing => match data.binary_status {
            BinaryStatus::Untested if data.existing_binary.trim().is_empty() => {
                "repairSheet.buttons.selectBinaryFirst"
            }
            BinaryStatus::Untested => "repairSheet.buttons.testBinaryFirst",
            BinaryStatus::Testing => "repairSheet.buttons.testingBinary",
            BinaryStatus::Valid => "repairSheet.buttons.useThisBinary",
            BinaryStatus::Invalid => "repairSheet.buttons.invalidBinary",
        },
    }
}

/// Primary action label keys for config mode (Angular `getConfigModeButtonTextKey`).
pub fn config_action_key(data: &InstallationOptionsData) -> &'static str {
    if data.location == InstallLocation::Custom && data.custom_path.trim().is_empty() {
        "repairSheet.buttons.selectConfigFirst"
    } else {
        "repairSheet.buttons.useThisConfig"
    }
}

/// Disabled-state tooltip keys (Angular `repairTooltip`).
pub fn repair_tooltip_key(
    mode: InstallationMode,
    data: &InstallationOptionsData,
) -> Option<&'static str> {
    if installation_valid(mode, data) {
        return None;
    }
    match (mode, data.location) {
        (InstallationMode::Config, InstallLocation::Custom)
            if data.custom_path.trim().is_empty() =>
        {
            Some("repairSheet.tooltips.selectConfigFirst")
        }
        (InstallationMode::Config, _) => Some("repairSheet.tooltips.fixValidationErrors"),
        (InstallationMode::Install, InstallLocation::Custom)
            if data.custom_path.trim().is_empty() =>
        {
            Some("repairSheet.tooltips.selectInstallPathFirst")
        }
        (InstallationMode::Install, InstallLocation::Existing)
            if data.existing_binary.trim().is_empty() =>
        {
            Some("repairSheet.tooltips.selectBinaryFirst")
        }
        (InstallationMode::Install, InstallLocation::Existing)
            if data.binary_status == BinaryStatus::Invalid =>
        {
            Some("repairSheet.tooltips.invalidBinary")
        }
        (InstallationMode::Install, InstallLocation::Existing)
            if data.binary_status == BinaryStatus::Untested
                || data.binary_status == BinaryStatus::Testing =>
        {
            Some("repairSheet.tooltips.testBinaryFirst")
        }
        _ => Some("repairSheet.tooltips.fixValidationErrors"),
    }
}

pub fn binary_status_key(status: BinaryStatus) -> &'static str {
    match status {
        BinaryStatus::Untested => "shared.installationOptions.status.untested",
        BinaryStatus::Testing => "shared.installationOptions.status.testing",
        BinaryStatus::Valid => "shared.installationOptions.status.valid",
        BinaryStatus::Invalid => "shared.installationOptions.status.invalid",
    }
}

/// Run `rclone version` on the given binary. Empty / relative / non-rclone paths are invalid.
pub fn test_rclone_binary(path: &str) -> BinaryStatus {
    let trimmed = path.trim();
    if !path_is_absolute(trimmed) {
        return BinaryStatus::Invalid;
    }
    let bin = Path::new(trimmed);
    if !bin.is_file() {
        return BinaryStatus::Invalid;
    }
    let mut cmd = Command::new(bin);
    cmd.arg("version");
    cmd.env_remove("RCLONE_CONFIG");
    match run_with_timeout(&mut cmd, Duration::from_secs(5)) {
        Some(output) if output.status.success() => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if text.to_ascii_lowercase().contains("rclone") {
                BinaryStatus::Valid
            } else {
                BinaryStatus::Invalid
            }
        }
        _ => BinaryStatus::Invalid,
    }
}

fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<std::process::Output> {
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dest_requires_absolute_custom_path() {
        assert!(rclone_install_dest(InstallLocation::Default, "")
            .unwrap()
            .ends_with(".local/bin"));
        assert!(rclone_install_dest(InstallLocation::Custom, "  ").is_none());
        assert!(rclone_install_dest(InstallLocation::Custom, "relative").is_none());
        assert_eq!(
            rclone_install_dest(InstallLocation::Custom, "/opt/rclone"),
            Some(PathBuf::from("/opt/rclone"))
        );
        assert!(rclone_install_dest(InstallLocation::Existing, "/bin/rclone").is_none());
    }

    #[test]
    fn validity_matches_angular_is_valid() {
        let mut data = InstallationOptionsData::default();
        assert!(installation_valid(InstallationMode::Install, &data));
        assert!(installation_valid(InstallationMode::Config, &data));

        data.location = InstallLocation::Custom;
        assert!(!installation_valid(InstallationMode::Install, &data));
        data.custom_path = "relative".into();
        assert!(!installation_valid(InstallationMode::Install, &data));
        data.custom_path = "/opt/rclone".into();
        assert!(installation_valid(InstallationMode::Install, &data));
        assert!(installation_valid(InstallationMode::Config, &data));

        data.location = InstallLocation::Existing;
        data.existing_binary = "/usr/bin/rclone".into();
        data.binary_status = BinaryStatus::Untested;
        assert!(!installation_valid(InstallationMode::Install, &data));
        data.binary_status = BinaryStatus::Valid;
        assert!(installation_valid(InstallationMode::Install, &data));
        assert!(!installation_valid(InstallationMode::Config, &data));
    }

    #[test]
    fn action_and_tooltip_keys_match_angular() {
        let mut data = InstallationOptionsData::default();
        assert_eq!(
            install_action_key(&data),
            "repairSheet.actions.installRclone"
        );
        assert_eq!(
            config_action_key(&data),
            "repairSheet.buttons.useThisConfig"
        );
        assert_eq!(repair_tooltip_key(InstallationMode::Install, &data), None);

        data.location = InstallLocation::Custom;
        assert_eq!(
            install_action_key(&data),
            "repairSheet.buttons.selectPathFirst"
        );
        assert_eq!(
            repair_tooltip_key(InstallationMode::Install, &data),
            Some("repairSheet.tooltips.selectInstallPathFirst")
        );
        assert_eq!(
            config_action_key(&data),
            "repairSheet.buttons.selectConfigFirst"
        );

        data.location = InstallLocation::Existing;
        assert_eq!(
            install_action_key(&data),
            "repairSheet.buttons.selectBinaryFirst"
        );
        data.existing_binary = "/usr/bin/rclone".into();
        assert_eq!(
            install_action_key(&data),
            "repairSheet.buttons.testBinaryFirst"
        );
        assert_eq!(
            repair_tooltip_key(InstallationMode::Install, &data),
            Some("repairSheet.tooltips.testBinaryFirst")
        );
        data.binary_status = BinaryStatus::Invalid;
        assert_eq!(
            install_action_key(&data),
            "repairSheet.buttons.invalidBinary"
        );
        assert_eq!(
            repair_tooltip_key(InstallationMode::Install, &data),
            Some("repairSheet.tooltips.invalidBinary")
        );
        data.binary_status = BinaryStatus::Valid;
        assert_eq!(
            install_action_key(&data),
            "repairSheet.buttons.useThisBinary"
        );
        assert_eq!(repair_tooltip_key(InstallationMode::Install, &data), None);
    }

    #[test]
    fn test_rclone_binary_rejects_non_rclone_and_missing() {
        assert_eq!(test_rclone_binary(""), BinaryStatus::Invalid);
        assert_eq!(test_rclone_binary("rclone"), BinaryStatus::Invalid);
        assert_eq!(
            test_rclone_binary("/tmp/rclone-manager-no-such-binary"),
            BinaryStatus::Invalid
        );
        if Path::new("/bin/true").is_file() {
            assert_eq!(test_rclone_binary("/bin/true"), BinaryStatus::Invalid);
        }
        if Path::new("/usr/bin/rclone").is_file() {
            assert_eq!(test_rclone_binary("/usr/bin/rclone"), BinaryStatus::Valid);
        }
    }
}
