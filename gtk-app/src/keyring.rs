//! OS keyring (libsecret) for the rclone.conf password.

const SERVICE: &str = "rclone-manager";
const USER: &str = "rclone-config-password";

pub fn store_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return delete_password();
    }
    let entry = keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())?;
    entry.set_password(password).map_err(|e| e.to_string())
}

pub fn load_password() -> Option<String> {
    let entry = keyring::Entry::new(SERVICE, USER).ok()?;
    entry.get_password().ok().filter(|s| !s.is_empty())
}

pub fn delete_password() -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn resolve_config_password(stored: &str) -> String {
    if !stored.is_empty() {
        stored.to_string()
    } else {
        load_password().unwrap_or_default()
    }
}

/// Persist the rclone.conf password in the OS keyring when available.
/// Returns true when the keyring accepted it (settings.json should stay empty).
pub fn persist_password_setting(stored_field: &mut String, password: &str) -> bool {
    match store_password(password) {
        Ok(()) => {
            stored_field.clear();
            true
        }
        Err(_) => {
            *stored_field = password.to_string();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_explicit_store() {
        assert_eq!(resolve_config_password("secret"), "secret");
        // Empty stored value falls through to the keyring, which is absent in CI.
        let _ = resolve_config_password("");
    }

    #[test]
    fn persist_falls_back_when_keyring_unavailable() {
        let mut stored = String::new();
        let in_keyring = persist_password_setting(&mut stored, "abc");
        if in_keyring {
            assert!(stored.is_empty());
        } else {
            assert_eq!(stored, "abc");
        }
        let _ = persist_password_setting(&mut stored, "");
    }
}
