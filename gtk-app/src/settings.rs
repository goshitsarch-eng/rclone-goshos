//! App settings persisted as JSON — mirrors `src-tauri` settings schema.

use crate::operations::MainView;
use serde::{Deserialize, Serialize};
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
        self.sidebar_visible = true;
        self
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
        assert!(loaded.runtime.flatpak_warn);
    }
}
