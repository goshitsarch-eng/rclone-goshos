//! Import Tauri/rcman on-disk settings into the GTK monolith store.

use crate::operations::OperationType;
use crate::settings::{AppSettings, BackendEntry};
use crate::store::{AppConfig, AppStore, ProfileConfig, QuickRun, RemoteMeta};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub remotes: usize,
    pub quick_runs: usize,
    pub backends: usize,
    pub alert_rules: usize,
    pub alert_actions: usize,
    pub templates: usize,
}

impl ImportReport {
    pub fn changed(&self) -> bool {
        self.remotes
            + self.quick_runs
            + self.backends
            + self.alert_rules
            + self.alert_actions
            + self.templates
            > 0
    }
}

pub fn detect_rcman_layout(config_dir: &Path) -> bool {
    config_dir.join("remotes").is_dir()
        || config_dir.join("quick_runs.json").is_file()
        || config_dir.join("connections.json").is_file()
        || config_dir.join("alerts").is_dir()
}

fn bool_field(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|k| value.get(*k).and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

fn string_field(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| {
            value
                .get(*k)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

fn u64_field(value: &Value, keys: &[&str], default: u64) -> u64 {
    keys.iter()
        .find_map(|k| value.get(*k).and_then(|v| v.as_u64()))
        .unwrap_or(default)
}

pub fn parse_app_config(value: &Value) -> AppConfig {
    AppConfig {
        auto_start: bool_field(value, &["auto_start", "autoStart"]),
        cron_enabled: bool_field(value, &["cron_enabled", "cronEnabled"]),
        cron_expression: string_field(value, &["cron_expression", "cronExpression"]),
        watch_enabled: bool_field(value, &["watch_enabled", "watchEnabled"]),
        watch_delay: u64_field(value, &["watch_delay", "watchDelay"], 0),
        watch_changed_only: bool_field(value, &["watch_changed_only", "watchChangedOnly"]),
        vfs_profile: string_field(value, &["vfs_profile", "vfsProfile"]),
        filter_profile: string_field(value, &["filter_profile", "filterProfile"]),
        backend_profile: string_field(value, &["backend_profile", "backendProfile"]),
        runtime_remote_profile: string_field(
            value,
            &["runtime_remote_profile", "runtimeRemoteProfile"],
        ),
    }
}

pub fn parse_profile(name: &str, value: &Value) -> ProfileConfig {
    ProfileConfig {
        name: {
            let named = string_field(value, &["name"]);
            if named.is_empty() {
                name.to_string()
            } else {
                named
            }
        },
        app: value.get("app").map(parse_app_config).unwrap_or_default(),
        rclone: value
            .get("rclone")
            .cloned()
            .unwrap_or(serde_json::json!({})),
    }
}

fn helper_map(value: &Value, snake: &str, camel: &str) -> std::collections::HashMap<String, Value> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(|v| v.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

pub fn remote_meta_from_rcman(value: &Value) -> RemoteMeta {
    let mut meta = RemoteMeta {
        show_on_tray: bool_field(value, &["show_on_tray", "showOnTray"]),
        primary_actions: value
            .get("primary_actions")
            .or_else(|| value.get("primaryActions"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        sync_actions: value
            .get("sync_actions")
            .or_else(|| value.get("syncActions"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        vfs_configs: helper_map(value, "vfs_configs", "vfsConfigs"),
        filter_configs: helper_map(value, "filter_configs", "filterConfigs"),
        backend_configs: helper_map(value, "backend_configs", "backendConfigs"),
        runtime_remote_configs: helper_map(value, "runtime_remote_configs", "runtimeRemoteConfigs"),
        ..RemoteMeta::default()
    };
    for op in OperationType::ALL {
        if let Some(map) = value.get(op.config_key()).and_then(|v| v.as_object()) {
            for (name, profile) in map {
                meta.upsert_profile(op, parse_profile(name, profile));
            }
        }
    }
    meta
}

fn iter_json_entries(value: &Value) -> Vec<(String, Value)> {
    if let Some(arr) = value.as_array() {
        return arr
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let id = v
                    .get("id")
                    .or_else(|| v.get("name"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| i.to_string());
                (id, v.clone())
            })
            .collect();
    }
    if let Some(obj) = value.as_object() {
        return obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    }
    Vec::new()
}

pub fn parse_quick_run(id: &str, value: &Value) -> Option<QuickRun> {
    let name = string_field(value, &["name"]);
    if name.is_empty() {
        return None;
    }
    let op = value
        .get("operation_type")
        .or_else(|| value.get("operationType"))
        .and_then(|v| v.as_str())
        .and_then(OperationType::parse)
        .unwrap_or(OperationType::Sync);
    let remote = string_field(value, &["remote_name", "remoteName"]);
    let config = value
        .get("config")
        .map(|v| parse_profile(&name, v))
        .unwrap_or_default();
    Some(QuickRun {
        id: {
            let existing = string_field(value, &["id"]);
            if existing.is_empty() {
                id.to_string()
            } else {
                existing
            }
        },
        name,
        description: string_field(value, &["description"]),
        operation_type: op,
        remote_name: remote,
        config,
        status: string_field(value, &["status"]),
        show_on_tray: bool_field(value, &["show_on_tray", "showOnTray"]),
        last_job_id: value
            .get("last_job_id")
            .or_else(|| value.get("lastJobId"))
            .and_then(|v| v.as_u64()),
        run_count: u64_field(value, &["run_count", "runCount"], 0),
    })
}

pub fn parse_backend_entry(name: &str, value: &Value) -> Option<BackendEntry> {
    if bool_field(value, &["is_local", "isLocal"]) {
        return None;
    }
    let host = string_field(value, &["host"]);
    if host.is_empty() {
        return None;
    }
    Some(BackendEntry {
        name: {
            let named = string_field(value, &["name"]);
            if named.is_empty() {
                name.to_string()
            } else {
                named
            }
        },
        host,
        port: value.get("port").and_then(|v| v.as_u64()).unwrap_or(5572) as u16,
        user: string_field(value, &["user", "username"]),
        pass: string_field(value, &["pass", "password"]),
        config_path: string_field(value, &["config_path", "configPath"]),
        config_password: string_field(value, &["config_password", "configPassword"]),
    })
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn import_rcman(
    config_dir: &Path,
    store: &mut AppStore,
    settings: &mut AppSettings,
) -> ImportReport {
    let mut report = ImportReport::default();
    let remotes_dir = config_dir.join("remotes");
    if remotes_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&remotes_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(value) = read_json(&path) else {
                    continue;
                };
                let name = value
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or_default()
                            .to_string()
                    });
                if name.is_empty() || store.remotes.contains_key(&name) {
                    continue;
                }
                store
                    .remotes
                    .insert(name.clone(), remote_meta_from_rcman(&value));
                store.ensure_remote_order(&[name]);
                report.remotes += 1;
            }
        }
    }
    if let Some(value) = read_json(&config_dir.join("quick_runs.json")) {
        for (id, item) in iter_json_entries(&value) {
            if let Some(run) = parse_quick_run(&id, &item) {
                if store
                    .quick_runs
                    .iter()
                    .any(|existing| existing.id == run.id)
                {
                    continue;
                }
                store.quick_runs.push(run);
                report.quick_runs += 1;
            }
        }
    }
    if let Some(value) = read_json(&config_dir.join("connections.json")) {
        for (id, item) in iter_json_entries(&value) {
            if let Some(backend) = parse_backend_entry(&id, &item) {
                if settings
                    .core
                    .extra_backends
                    .iter()
                    .any(|b| b.name == backend.name)
                {
                    continue;
                }
                settings.core.extra_backends.push(backend);
                report.backends += 1;
            }
        }
    }
    if let Some(value) = read_json(&config_dir.join("alerts").join("rules.json")) {
        for (_id, item) in iter_json_entries(&value) {
            if let Ok(rule) = serde_json::from_value::<crate::store::AlertRule>(item) {
                if store.alert_rules.iter().any(|r| r.id == rule.id) {
                    continue;
                }
                store.alert_rules.push(rule);
                report.alert_rules += 1;
            }
        }
    }
    if let Some(value) = read_json(&config_dir.join("alerts").join("actions.json")) {
        for (_id, item) in iter_json_entries(&value) {
            if let Ok(action) = serde_json::from_value::<crate::store::AlertAction>(item) {
                if store.alert_actions.iter().any(|a| a.id == action.id) {
                    continue;
                }
                store.alert_actions.push(action);
                report.alert_actions += 1;
            }
        }
    }
    if let Some(value) = read_json(&config_dir.join("templates.json")) {
        for (_id, item) in iter_json_entries(&value) {
            if let Ok(template) = serde_json::from_value::<crate::store::UserTemplate>(item) {
                if store.templates.iter().any(|t| t.id == template.id) {
                    continue;
                }
                store.templates.push(template);
                report.templates += 1;
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn maps_rcman_remote_json() {
        let value = json!({
            "name": "drive",
            "showOnTray": true,
            "primaryActions": ["mount", "sync"],
            "syncConfigs": {
                "nightly": {
                    "name": "nightly",
                    "app": { "autoStart": true, "cronExpression": "0 2 * * *" },
                    "rclone": { "srcFs": "drive:Photos", "dstFs": "/tmp" }
                }
            },
            "vfsConfigs": { "fast": { "CacheMode": "full" } }
        });
        let meta = remote_meta_from_rcman(&value);
        assert!(meta.show_on_tray);
        assert_eq!(meta.primary_actions, vec!["mount", "sync"]);
        let profile = meta.get_profile(OperationType::Sync, "nightly").unwrap();
        assert!(profile.app.auto_start);
        assert_eq!(profile.app.cron_expression, "0 2 * * *");
        assert_eq!(profile.rclone["srcFs"], "drive:Photos");
        assert_eq!(
            meta.helper_profile("vfs", "fast").unwrap()["CacheMode"],
            "full"
        );
    }

    #[test]
    fn imports_layout_from_temp_dir() {
        let dir =
            std::env::temp_dir().join(format!("rclone-manager-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("remotes")).unwrap();
        fs::write(
            dir.join("remotes").join("drive.json"),
            json!({
                "name": "drive",
                "syncConfigs": { "default": { "rclone": { "srcFs": "drive:" } } }
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            dir.join("quick_runs.json"),
            json!([{
                "id": "qr1",
                "name": "Nightly",
                "operationType": "sync",
                "remoteName": "drive",
                "config": { "rclone": { "srcFs": "drive:a" } }
            }])
            .to_string(),
        )
        .unwrap();
        fs::write(
            dir.join("connections.json"),
            json!({
                "office": { "name": "office", "host": "10.0.0.8", "port": 5572, "isLocal": false }
            })
            .to_string(),
        )
        .unwrap();
        assert!(detect_rcman_layout(&dir));
        let mut store = AppStore::default();
        let mut settings = AppSettings::default();
        let report = import_rcman(&dir, &mut store, &mut settings);
        assert_eq!(report.remotes, 1);
        assert_eq!(report.quick_runs, 1);
        assert_eq!(report.backends, 1);
        assert!(store.remotes.contains_key("drive"));
        assert_eq!(store.quick_runs[0].remote_name, "drive");
        assert_eq!(settings.core.extra_backends[0].host, "10.0.0.8");
        let again = import_rcman(&dir, &mut store, &mut settings);
        assert!(!again.changed());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_local_backend_entries() {
        let backend = parse_backend_entry(
            "Local",
            &json!({ "host": "127.0.0.1", "port": 5572, "isLocal": true }),
        );
        assert!(backend.is_none());
    }
}
