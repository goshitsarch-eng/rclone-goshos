//! App settings persisted as JSON — mirrors `src-tauri` settings schema.

use crate::operations::MainView;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub language: String,
    pub default_view: String,
    pub tray_enabled: bool,
    pub tray_icon_theme: String,
    pub start_on_startup: bool,
    pub notifications: bool,
    pub restrict: bool,
    pub standalone_dialogs: bool,
    pub prevent_sleep: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            language: crate::i18n::I18n::detect_language(),
            default_view: "main_menu".into(),
            tray_enabled: true,
            tray_icon_theme: "color".into(),
            start_on_startup: false,
            notifications: true,
            restrict: true,
            standalone_dialogs: false,
            prevent_sleep: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreSettings {
    pub max_tray_items: usize,
    pub rclone_binary: String,
    pub rclone_additional_flags: Vec<String>,
    pub rclone_env_vars: Vec<String>,
    pub connection_check_urls: Vec<String>,
    pub bandwidth_limit: String,
    #[serde(default)]
    pub metered_bandwidth_limit: String,
    pub completed_onboarding: bool,
    #[serde(default)]
    pub extra_backends: Vec<BackendEntry>,
    #[serde(default)]
    pub active_backend: String,
    #[serde(default)]
    pub config_password: String,
    #[serde(default)]
    pub default_mount_directory: String,
    #[serde(default)]
    pub default_bisync_directory: String,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            max_tray_items: 5,
            rclone_binary: String::new(),
            rclone_additional_flags: vec![],
            rclone_env_vars: vec![],
            connection_check_urls: vec![
                "https://www.google.com".into(),
                "https://www.dropbox.com".into(),
                "https://onedrive.live.com".into(),
            ],
            bandwidth_limit: String::new(),
            metered_bandwidth_limit: String::new(),
            completed_onboarding: false,
            extra_backends: vec![],
            active_backend: String::new(),
            config_password: String::new(),
            default_mount_directory: String::new(),
            default_bisync_directory: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BackendEntry {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperSettings {
    pub log_level: String,
    pub destroy_window_on_close: bool,
}

impl Default for DeveloperSettings {
    fn default() -> Self {
        Self {
            log_level: "info".into(),
            destroy_window_on_close: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub theme: String,
    pub app_auto_check_updates: bool,
    pub app_skipped_updates: Vec<String>,
    pub app_update_channel: String,
    pub rclone_auto_check_updates: bool,
    pub rclone_skipped_updates: Vec<String>,
    pub rclone_update_channel: String,
    pub dashboard_layout: serde_json::Value,
    pub quick_run_layout: serde_json::Value,
    pub remote_layouts: serde_json::Value,
    pub dashboard_card_variant: String,
    #[serde(default)]
    pub show_json_mode: bool,
    #[serde(default = "default_true")]
    pub flatpak_warn: bool,
    #[serde(default)]
    pub selected_sync_ops: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            app_auto_check_updates: true,
            app_skipped_updates: vec![],
            app_update_channel: "stable".into(),
            rclone_auto_check_updates: true,
            rclone_skipped_updates: vec![],
            rclone_update_channel: "stable".into(),
            dashboard_layout: serde_json::json!({}),
            quick_run_layout: serde_json::json!({}),
            remote_layouts: serde_json::json!({}),
            dashboard_card_variant: "compact".into(),
            show_json_mode: false,
            flatpak_warn: true,
            selected_sync_ops: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NautilusSettings {
    pub starred: Vec<serde_json::Value>,
    pub bookmarks: Vec<serde_json::Value>,
    pub sidebar_visible: bool,
    pub show_hidden: bool,
    pub layout: String,
    pub sort_by: String,
    pub sort_desc: bool,
    pub icon_size: i32,
    #[serde(default)]
    pub sidebar_drive_order: Vec<String>,
    #[serde(default)]
    pub sidebar_hidden_drives: Vec<String>,
    #[serde(default)]
    pub file_type_filter: String,
    #[serde(default)]
    pub split_enabled: bool,
    #[serde(default)]
    pub grid_icon_size: i32,
    #[serde(default)]
    pub split_divider_pos: i32,
}

impl NautilusSettings {
    pub fn normalized(mut self) -> Self {
        if self.layout.is_empty() {
            self.layout = "list".into();
        }
        if self.sort_by.is_empty() {
            self.sort_by = "name".into();
        }
        if self.icon_size == 0 {
            self.icon_size = 48;
        }
        if self.grid_icon_size == 0 {
            self.grid_icon_size = self.icon_size.max(48);
        }
        self.sidebar_visible = true;
        self
    }

    pub fn list_icon_px(&self) -> i32 {
        if self.icon_size > 0 {
            self.icon_size
        } else {
            32
        }
    }

    pub fn grid_icon_px(&self) -> i32 {
        if self.grid_icon_size > 0 {
            self.grid_icon_size
        } else {
            self.icon_size.max(48)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub general: GeneralSettings,
    pub core: CoreSettings,
    pub developer: DeveloperSettings,
    pub runtime: RuntimeSettings,
    pub nautilus: NautilusSettings,
}

impl AppSettings {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rclone-manager")
    }

    pub fn settings_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn log_path() -> PathBuf {
        Self::config_dir().join("rclone.log")
    }

    pub fn cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| Self::config_dir().join("cache"))
            .join("rclone-manager")
    }

    pub fn load() -> Self {
        let path = Self::settings_path();
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<AppSettings>(&text) {
                return settings;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        std::fs::write(Self::settings_path(), text)
    }

    pub fn default_view(&self) -> MainView {
        MainView::parse(&self.general.default_view)
    }
}

pub fn sort_sidebar_ids(mut ids: Vec<String>, order: &[String]) -> Vec<String> {
    if !order.is_empty() {
        ids.sort_by_key(|id| order.iter().position(|n| n == id).unwrap_or(usize::MAX));
    }
    ids
}

pub fn sidebar_id_hidden(hidden: &[String], id: &str) -> bool {
    hidden.iter().any(|h| h == id)
}

pub fn collection_path(item: &serde_json::Value) -> Option<String> {
    item.get("path")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub fn collection_contains(list: &[serde_json::Value], path: &str) -> bool {
    list.iter()
        .any(|item| collection_path(item).as_deref() == Some(path))
}

/// Toggle `path` in a starred/bookmark collection. Returns true if the path is now present.
pub fn toggle_collection(list: &mut Vec<serde_json::Value>, path: &str, name: &str) -> bool {
    if let Some(idx) = list
        .iter()
        .position(|item| collection_path(item).as_deref() == Some(path))
    {
        list.remove(idx);
        false
    } else {
        list.push(serde_json::json!({
            "name": if name.is_empty() {
                path.rsplit(['/', ':']).next().unwrap_or(path)
            } else {
                name
            },
            "path": path
        }));
        true
    }
}

impl AppSettings {
    pub fn get_by_path(&self, path: &str) -> Option<serde_json::Value> {
        let value = serde_json::to_value(self).ok()?;
        let mut cur = &value;
        for part in path.split('.') {
            cur = cur.get(part)?;
        }
        Some(cur.clone())
    }

    pub fn set_by_path(&mut self, path: &str, new_value: serde_json::Value) -> Result<(), String> {
        let mut root = serde_json::to_value(&*self).map_err(|e| e.to_string())?;
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() < 2 {
            return Err("invalid setting path".into());
        }
        let mut target = &mut root;
        for part in &parts[..parts.len() - 1] {
            target = target
                .get_mut(*part)
                .ok_or_else(|| format!("unknown setting {path}"))?;
        }
        let last = parts[parts.len() - 1];
        let obj = target
            .as_object_mut()
            .ok_or_else(|| format!("not an object: {path}"))?;
        obj.insert(last.to_string(), new_value);
        *self = serde_json::from_value(root).map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Setting paths that require an rclone engine restart, matching the Angular
/// `engine_restart` schema on `rclone_binary`, `rclone_additional_flags`,
/// and `rclone_env_vars`.
pub const ENGINE_RESTART_PATHS: &[&str] = &[
    "core.rclone_binary",
    "core.rclone_additional_flags",
    "core.rclone_env_vars",
];

pub fn requires_engine_restart(path: &str) -> bool {
    ENGINE_RESTART_PATHS.contains(&path)
}

pub fn default_for_path(path: &str) -> Option<serde_json::Value> {
    AppSettings::default().get_by_path(path)
}

pub fn values_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    left == right
}

pub fn apply_path_values(
    settings: &mut AppSettings,
    values: &[(String, serde_json::Value)],
) -> Result<(), String> {
    for (path, value) in values {
        settings.set_by_path(path, value.clone())?;
    }
    Ok(())
}

pub fn display_setting(value: &serde_json::Value, sep: &str) -> String {
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(s) => Some(s.clone()),
                other if !other.is_null() => Some(other.to_string().trim_matches('"').to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(sep),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_nested_setting() {
        let mut settings = AppSettings::default();
        settings
            .set_by_path("general.language", serde_json::json!("tr-TR"))
            .unwrap();
        assert_eq!(settings.general.language, "tr-TR");
        assert_eq!(
            settings.get_by_path("core.max_tray_items"),
            Some(serde_json::json!(5))
        );
    }

    #[test]
    fn rejects_unknown_path() {
        let mut settings = AppSettings::default();
        assert!(settings
            .set_by_path("nope.value", serde_json::json!(1))
            .is_err());
    }

    #[test]
    fn roundtrip_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = AppSettings {
            general: GeneralSettings {
                language: "ja-JP".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_string(&settings).unwrap()).unwrap();
        let loaded: AppSettings =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(loaded.general.language, "ja-JP");
    }

    #[test]
    fn loads_legacy_settings_without_new_fields() {
        let json = r#"{
            "general": {"language":"en-US","default_view":"main_menu","tray_enabled":true,"tray_icon_theme":"color","start_on_startup":false,"notifications":true,"restrict":true,"standalone_dialogs":false,"prevent_sleep":true},
            "core": {"max_tray_items":5,"rclone_binary":"","rclone_additional_flags":[],"rclone_env_vars":[],"connection_check_urls":[],"bandwidth_limit":"2M","completed_onboarding":true,"extra_backends":[],"active_backend":"","config_password":""},
            "developer": {"log_level":"info","destroy_window_on_close":true},
            "runtime": {"theme":"system","app_auto_check_updates":true,"app_skipped_updates":[],"app_update_channel":"stable","rclone_auto_check_updates":true,"rclone_skipped_updates":[],"rclone_update_channel":"stable","dashboard_layout":{},"quick_run_layout":{},"remote_layouts":{},"dashboard_card_variant":"compact"},
            "nautilus": {"starred":[],"bookmarks":[],"sidebar_visible":true,"show_hidden":false,"layout":"list","sort_by":"name","sort_desc":false,"icon_size":48}
        }"#;
        let loaded: AppSettings = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.core.bandwidth_limit, "2M");
        assert!(loaded.core.metered_bandwidth_limit.is_empty());
        assert!(loaded.core.default_mount_directory.is_empty());
        assert!(loaded.core.default_bisync_directory.is_empty());
        assert!(loaded.runtime.flatpak_warn);
        assert!(loaded.nautilus.sidebar_drive_order.is_empty());
        assert!(loaded.nautilus.sidebar_hidden_drives.is_empty());
        assert!(loaded.nautilus.file_type_filter.is_empty());
        assert!(loaded.runtime.selected_sync_ops.is_empty());
        assert_eq!(loaded.nautilus.grid_icon_size, 0);
        assert_eq!(loaded.nautilus.split_divider_pos, 0);
    }

    #[test]
    fn sidebar_order_and_hidden() {
        let ids = sort_sidebar_ids(
            vec!["b:".into(), "a:".into(), "/home".into()],
            &["/home".into(), "a:".into()],
        );
        assert_eq!(ids, vec!["/home", "a:", "b:"]);
        assert!(sidebar_id_hidden(&["a:".into()], "a:"));
        assert!(!sidebar_id_hidden(&["a:".into()], "b:"));
    }

    #[test]
    fn engine_restart_paths_match_angular_schema() {
        for path in [
            "core.rclone_binary",
            "core.rclone_additional_flags",
            "core.rclone_env_vars",
        ] {
            assert!(requires_engine_restart(path), "{path}");
        }
        assert!(!requires_engine_restart("core.bandwidth_limit"));
        assert!(!requires_engine_restart("general.language"));
    }

    #[test]
    fn default_for_path_reads_app_defaults() {
        assert_eq!(
            default_for_path("core.max_tray_items"),
            Some(serde_json::json!(5))
        );
        assert_eq!(
            default_for_path("core.rclone_additional_flags"),
            Some(serde_json::json!([]))
        );
        assert_eq!(
            default_for_path("developer.destroy_window_on_close"),
            Some(serde_json::json!(true))
        );
        assert!(default_for_path("nope.value").is_none());
    }

    #[test]
    fn apply_pending_restart_batch() {
        let mut settings = AppSettings::default();
        apply_path_values(
            &mut settings,
            &[
                (
                    "core.rclone_binary".into(),
                    serde_json::json!("/usr/bin/rclone"),
                ),
                (
                    "core.rclone_additional_flags".into(),
                    serde_json::json!(["--transfers", "8"]),
                ),
                (
                    "core.rclone_env_vars".into(),
                    serde_json::json!(["RCLONE_VERBOSE=1"]),
                ),
            ],
        )
        .unwrap();
        assert_eq!(settings.core.rclone_binary, "/usr/bin/rclone");
        assert_eq!(
            settings.core.rclone_additional_flags,
            vec!["--transfers", "8"]
        );
        assert_eq!(settings.core.rclone_env_vars, vec!["RCLONE_VERBOSE=1"]);
    }

    #[test]
    fn display_setting_joins_arrays() {
        assert_eq!(
            display_setting(&serde_json::json!(["--rc", "--vfs-cache-mode"]), " "),
            "--rc --vfs-cache-mode"
        );
        assert_eq!(
            display_setting(&serde_json::json!(["A=1", "B=2"]), ";"),
            "A=1;B=2"
        );
        assert_eq!(display_setting(&serde_json::json!("plain"), " "), "plain");
        assert!(values_equal(
            &serde_json::json!(["a"]),
            &serde_json::json!(["a"])
        ));
        assert!(!values_equal(
            &serde_json::json!(["a"]),
            &serde_json::json!(["b"])
        ));
    }

    #[test]
    fn toggles_starred_collection() {
        let mut list = vec![];
        assert!(toggle_collection(&mut list, "drive:Photos", "Photos"));
        assert!(collection_contains(&list, "drive:Photos"));
        assert_eq!(collection_path(&list[0]).as_deref(), Some("drive:Photos"));
        assert!(!toggle_collection(&mut list, "drive:Photos", "Photos"));
        assert!(!collection_contains(&list, "drive:Photos"));
    }
}
