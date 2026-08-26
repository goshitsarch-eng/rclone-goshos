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

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                if is_sensitive_key(key) {
                    out.insert(key.clone(), Value::String(RESTRICTED_LABEL.into()));
                } else {
                    out.insert(key.clone(), redact_value(child));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        other => other.clone(),
    }
}

/// One flattened setting row, matching Angular `SettingEntry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingEntry {
    pub key: String,
    pub display: String,
    pub sensitive: bool,
}

/// Angular `GroupedSettings` (app / rclone categories, or one flat group).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsGroup {
    pub category: String,
    pub entries: Vec<SettingEntry>,
}

/// Group settings the way Angular `settings-panel` does.
pub fn grouped_settings(settings: &Value, restrict: bool) -> Vec<SettingsGroup> {
    let Some(obj) = settings.as_object() else {
        return Vec::new();
    };
    let has_app = obj.get("app").is_some_and(Value::is_object);
    let has_rclone = obj.get("rclone").is_some_and(Value::is_object);
    if has_app || has_rclone {
        let mut groups = Vec::new();
        if has_app {
            let entries = flatten_panel_entries("app", &obj["app"], restrict);
            if !entries.is_empty() {
                groups.push(SettingsGroup {
                    category: "detailShared.settings.categories.app".into(),
                    entries,
                });
            }
        }
        if has_rclone {
            let entries = flatten_panel_entries("rclone", &obj["rclone"], restrict);
            if !entries.is_empty() {
                groups.push(SettingsGroup {
                    category: "detailShared.settings.categories.rclone".into(),
                    entries,
                });
            }
        }
        return groups;
    }
    let entries: Vec<SettingEntry> = obj
        .iter()
        .filter(|(_, value)| !value.is_null())
        .flat_map(|(key, value)| flatten_panel_entries(key, value, restrict))
        .collect();
    if entries.is_empty() {
        Vec::new()
    } else {
        vec![SettingsGroup {
            category: String::new(),
            entries,
        }]
    }
}

fn flatten_panel_entries(key: &str, value: &Value, restrict: bool) -> Vec<SettingEntry> {
    if value.is_null() {
        return Vec::new();
    }
    if let Value::Object(map) = value {
        if map.is_empty() {
            return Vec::new();
        }
        return map
            .iter()
            .flat_map(|(child, nested)| {
                let display_key = if key == "app" || key == "rclone" {
                    child.clone()
                } else {
                    format!("{key}.{child}")
                };
                flatten_panel_entries(&display_key, nested, restrict)
            })
            .collect();
    }
    let sensitive = restrict && is_sensitive_key(key);
    vec![SettingEntry {
        key: key.to_string(),
        display: display_value(key, value, restrict),
        sensitive,
    }]
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
        let redacted = redact_value(&json!({ "token": "abc", "srcFs": "drive:" }));
        assert_eq!(redacted["token"], RESTRICTED_LABEL);
        assert_eq!(redacted["srcFs"], "drive:");
    }

    #[test]
    fn grouped_settings_splits_app_and_rclone() {
        let groups = grouped_settings(
            &json!({
                "app": { "cronEnabled": true, "token": "abc" },
                "rclone": { "srcFs": "drive:Photos", "dstFs": "drive:out" }
            }),
            true,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].category, "detailShared.settings.categories.app");
        assert!(groups[0]
            .entries
            .iter()
            .any(|e| e.key == "cronEnabled" && e.display == "true"));
        assert!(groups[0]
            .entries
            .iter()
            .any(|e| { e.key == "token" && e.sensitive && e.display == RESTRICTED_LABEL }));
        assert_eq!(
            groups[1].category,
            "detailShared.settings.categories.rclone"
        );
        assert!(groups[1]
            .entries
            .iter()
            .any(|e| e.key == "srcFs" && e.display == "drive:Photos"));
    }

    #[test]
    fn grouped_settings_flattens_dump_without_categories() {
        let groups = grouped_settings(
            &json!({ "type": "alias", "remote": "/tmp/rclone-test-remote" }),
            false,
        );
        assert_eq!(groups.len(), 1);
        assert!(groups[0].category.is_empty());
        assert!(groups[0]
            .entries
            .iter()
            .any(|e| e.key == "type" && e.display == "alias"));
        assert!(grouped_settings(&json!({}), false).is_empty());
        assert!(grouped_settings(&json!(null), false).is_empty());
    }
}
