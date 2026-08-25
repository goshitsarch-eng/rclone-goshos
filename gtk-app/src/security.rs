//! Rclone config password and encryption — mirrors Tauri security commands.

use crate::rclone::engine::resolve_rclone_binary;
use std::process::Command;

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
    cmd.arg(format!("--password-command={}", password_command(password)));
    cmd.arg("--ask-password=false");
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
    cmd.arg(format!("--password-command={}", password_command(password)));
    cmd.arg("--ask-password=false");
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
    fn rejects_empty_password() {
        assert!(validate_password("rclone", "").is_err());
        assert!(encrypt_config("rclone", "abc").is_err());
    }
}
