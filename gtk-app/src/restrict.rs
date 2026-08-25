//! Mask sensitive setting values when `general.restrict` is enabled.

use serde_json::Value;

pub const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "pass",
    "session_id",
    "2fa",
    "secret",
    "endpoint",
    "token",
    "key",
    "credentials",
    "auth",
    "client_secret",
    "client_id",
    "api_key",
    "drive_id",
];

pub const RESTRICTED_LABEL: &str = "[Restricted]";

pub fn is_sensitive_key(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|key| lower.contains(key))
}

pub fn display_value(key: &str, value: &Value, restrict: bool) -> String {
    if restrict && is_sensitive_key(key) {
        return RESTRICTED_LABEL.to_string();
    }
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

pub fn flatten_settings(prefix: &str, value: &Value, restrict: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    flatten_into(prefix, value, restrict, &mut out);
    out
}

fn flatten_into(prefix: &str, value: &Value, restrict: bool, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_into(&next, child, restrict, out);
            }
        }
        other => {
            let leaf = prefix.rsplit('.').next().unwrap_or(prefix);
            out.push((prefix.to_string(), display_value(leaf, other, restrict)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_sensitive_substrings() {
        assert!(is_sensitive_key("client_secret"));
        assert!(is_sensitive_key("Token"));
        assert!(is_sensitive_key("drive_id"));
        assert!(!is_sensitive_key("srcFs"));
        assert!(!is_sensitive_key("dryRun"));
    }

    #[test]
    fn masks_only_when_restricted() {
        assert_eq!(
            display_value("password", &json!("hunter2"), true),
            RESTRICTED_LABEL
        );
        assert_eq!(
            display_value("password", &json!("hunter2"), false),
            "hunter2"
        );
        assert_eq!(display_value("srcFs", &json!("drive:"), true), "drive:");
    }

    #[test]
    fn flattens_nested_objects() {
        let rows = flatten_settings(
            "",
            &json!({ "app": { "token": "abc", "dryRun": true } }),
            true,
        );
        assert!(rows
            .iter()
            .any(|(k, v)| k == "app.token" && v == RESTRICTED_LABEL));
        assert!(rows.iter().any(|(k, v)| k == "app.dryRun" && v == "true"));
    }
}
