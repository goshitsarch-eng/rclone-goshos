//! Rclone config password and encryption — mirrors Tauri security commands.

use crate::rclone::engine::resolve_rclone_binary;
use crate::rclone::{RcClient, RcError};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityFormError {
    Empty,
    TooShort,
    Mismatch,
}

impl SecurityFormError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "Password cannot be empty",
            Self::TooShort => "Password must be at least 4 characters",
            Self::Mismatch => "Passwords do not match",
        }
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Empty => "backendErrors.security.passwordEmpty",
            Self::TooShort => "backendErrors.security.passwordEmpty",
            Self::Mismatch => "modals.backend.security.passwordsMismatch",
        }
    }
}

pub fn passwords_match(a: &str, b: &str) -> bool {
    a == b
}

pub fn validate_encrypt_form(password: &str, confirm: &str) -> Result<(), SecurityFormError> {
    if password.is_empty() || confirm.is_empty() {
        return Err(SecurityFormError::Empty);
    }
    if password.chars().count() < 4 {
        return Err(SecurityFormError::TooShort);
    }
    if !passwords_match(password, confirm) {
        return Err(SecurityFormError::Mismatch);
    }
    Ok(())
}

pub fn validate_change_password_form(
    current: &str,
    new_password: &str,
    confirm: &str,
) -> Result<(), SecurityFormError> {
    if current.is_empty() {
        return Err(SecurityFormError::Empty);
    }
    validate_encrypt_form(new_password, confirm)
}

pub fn has_stored_password(settings_field: &str) -> bool {
    !settings_field.is_empty() || crate::keyring::load_password().is_some()
}

pub fn probe_config_encrypted(
    client: Option<&RcClient>,
    config_path: Option<&Path>,
) -> Option<bool> {
    if let Some(client) = client {
        if let Ok(flag) = client.config_is_encrypted() {
            return Some(flag);
        }
    }
    config_path.map(crate::repair::config_file_encrypted)
}

pub fn octal_escape(input: &str) -> String {
    let mut out = String::new();
    for byte in input.as_bytes() {
        out.push_str(&format!("\\{byte:03o}"));
    }
    out
}

pub fn password_command(password: &str) -> String {
    format!("sh -c \"printf '{}'\"", octal_escape(password))
}

/// Hand the config password to rclone through the environment.
///
/// `--password-command=...` does the same job, but process arguments are
/// world-readable on Linux (`/proc/<pid>/cmdline`, mode 0444) while the
/// environment is not (`/proc/<pid>/environ`, mode 0400), so passing it as a
/// flag published the master password — octal-escaping only encodes it, it
/// stays trivially recoverable — to every other user on the machine.
pub fn apply_password_command_env(cmd: &mut Command, password: &str) {
    cmd.env("RCLONE_PASSWORD_COMMAND", password_command(password));
    cmd.arg("--ask-password=false");
}

pub fn apply_config_password_env(cmd: &mut Command, password: &str) {
    if password.is_empty() {
        return;
    }
    cmd.env("RCLONE_CONFIG_PASS", password);
    cmd.arg("--ask-password=false");
}

fn rclone_cmd(binary: &str) -> Command {
    Command::new(resolve_rclone_binary(binary))
}

fn prefer_rc(
    client: Option<&RcClient>,
    rc: impl FnOnce(&RcClient) -> Result<(), RcError>,
    fallback: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match client {
        Some(client) => match rc(client) {
            Ok(()) => Ok(()),
            Err(RcError::Unreachable(_)) => fallback(),
            Err(e) => Err(e.to_string()),
        },
        None => fallback(),
    }
}

pub fn validate_password_for(
    client: Option<&RcClient>,
    binary: &str,
    password: &str,
) -> Result<(), String> {
    prefer_rc(
        client,
        |c| c.config_validate_password(password).map(|_| ()),
        || validate_password(binary, password),
    )
}

pub fn encrypt_config_for(
    client: Option<&RcClient>,
    binary: &str,
    password: &str,
) -> Result<(), String> {
    prefer_rc(
        client,
        |c| c.config_encrypt(password).map(|_| ()),
        || encrypt_config(binary, password),
    )
}

pub fn unencrypt_config_for(
    client: Option<&RcClient>,
    binary: &str,
    password: &str,
) -> Result<(), String> {
    prefer_rc(
        client,
        |c| c.config_decrypt(password).map(|_| ()),
        || unencrypt_config(binary, password),
    )
}

pub fn change_password_for(
    client: Option<&RcClient>,
    binary: &str,
    current: &str,
    next: &str,
) -> Result<(), String> {
    unencrypt_config_for(client, binary, current)?;
    encrypt_config_for(client, binary, next)
}

pub fn validate_password(binary: &str, password: &str) -> Result<(), String> {
    if password.trim().is_empty() {
        return Err("Password is empty".into());
    }
    let mut cmd = rclone_cmd(binary);
    cmd.args(["listremotes", "--ask-password=false"]);
    apply_config_password_env(&mut cmd, password);
    let output = cmd.output().map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success()
        && (stderr.contains("wrong password")
            || stderr.contains("Couldn't decrypt")
            || stderr.contains("unable to decrypt"))
    {
        return Err("Wrong rclone config password".into());
    }
    if !output.status.success() && stderr.contains("decrypt") {
        return Err(stderr.trim().to_string());
    }
    Ok(())
}

pub fn encrypt_config(binary: &str, password: &str) -> Result<(), String> {
    if password.trim().len() < 4 {
        return Err("Password must be at least 4 characters".into());
    }
    let mut cmd = rclone_cmd(binary);
    cmd.args(["config", "encryption", "set"]);
    apply_password_command_env(&mut cmd, password);
    let output = cmd.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already encrypted") || stderr.contains("is encrypted") {
            return Ok(());
        }
        return Err(if stderr.trim().is_empty() {
            "Failed to encrypt rclone.conf".into()
        } else {
            stderr.trim().to_string()
        });
    }
    Ok(())
}

pub fn unencrypt_config(binary: &str, password: &str) -> Result<(), String> {
    let mut cmd = rclone_cmd(binary);
    cmd.args(["config", "encryption", "remove"]);
    apply_password_command_env(&mut cmd, password);
    let output = cmd.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not encrypted") {
            return Ok(());
        }
        return Err(if stderr.trim().is_empty() {
            "Failed to remove rclone.conf encryption".into()
        } else {
            stderr.trim().to_string()
        });
    }
    Ok(())
}

pub fn change_password(binary: &str, current: &str, next: &str) -> Result<(), String> {
    unencrypt_config(binary, current)?;
    encrypt_config(binary, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_escapes_bytes() {
        assert_eq!(octal_escape("ab"), "\\141\\142");
        assert_eq!(octal_escape("!"), "\\041");
    }

    #[test]
    fn password_command_wraps_printf() {
        let cmd = password_command("secret");
        assert!(cmd.starts_with("sh -c \"printf '"));
        assert!(cmd.contains("\\163"));
    }

    #[test]
    fn config_password_never_reaches_the_process_arguments() {
        // /proc/<pid>/cmdline is world-readable; /proc/<pid>/environ is not.
        let mut cmd = Command::new("rclone");
        cmd.args(["config", "encryption", "set"]);
        apply_password_command_env(&mut cmd, "hunter2");

        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let encoded = octal_escape("hunter2");
        for arg in &args {
            assert!(
                !arg.contains("hunter2"),
                "plaintext password in argv: {arg}"
            );
            assert!(!arg.contains(&encoded), "encoded password in argv: {arg}");
            assert!(
                !arg.starts_with("--password-command"),
                "the password command must travel in the environment: {arg}"
            );
        }
        assert!(args.iter().any(|a| a == "--ask-password=false"));

        let env: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        let (_, value) = env
            .iter()
            .find(|(k, _)| k == "RCLONE_PASSWORD_COMMAND")
            .expect("rclone reads RCLONE_PASSWORD_COMMAND as --password-command");
        assert_eq!(value, &password_command("hunter2"));
    }

    #[test]
    fn rejects_empty_password() {
        assert!(validate_password("rclone", "").is_err());
        assert!(encrypt_config("rclone", "abc").is_err());
        assert!(validate_password_for(None, "rclone", "").is_err());
        assert!(encrypt_config_for(None, "rclone", "abc").is_err());
    }

    #[test]
    fn encrypt_form_requires_match_and_length() {
        assert_eq!(
            validate_encrypt_form("", "secret"),
            Err(SecurityFormError::Empty)
        );
        assert_eq!(
            validate_encrypt_form("abc", "abc"),
            Err(SecurityFormError::TooShort)
        );
        assert_eq!(
            validate_encrypt_form("secret", "other"),
            Err(SecurityFormError::Mismatch)
        );
        assert!(validate_encrypt_form("secret", "secret").is_ok());
        assert!(passwords_match("a", "a"));
        assert!(!passwords_match("a", "b"));
    }

    #[test]
    fn change_password_form_requires_current() {
        assert_eq!(
            validate_change_password_form("", "secret", "secret"),
            Err(SecurityFormError::Empty)
        );
        assert_eq!(
            validate_change_password_form("old", "secret", "nope"),
            Err(SecurityFormError::Mismatch)
        );
        assert!(validate_change_password_form("old", "secret", "secret").is_ok());
    }

    #[test]
    fn stored_password_reads_settings_field() {
        assert!(has_stored_password("secret"));
        assert!(!has_stored_password(""));
    }

    #[test]
    fn probe_encrypted_uses_file_when_offline() {
        let tmp = std::env::temp_dir().join("rclone-manager-security-probe.conf");
        std::fs::write(&tmp, "RCLONE_ENCRYPT_V0:\n").unwrap();
        assert_eq!(probe_config_encrypted(None, Some(&tmp)), Some(true));
        std::fs::write(&tmp, "[local]\ntype = local\n").unwrap();
        assert_eq!(probe_config_encrypted(None, Some(&tmp)), Some(false));
        let _ = std::fs::remove_file(&tmp);
    }
}
