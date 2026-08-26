//! JSON i18n loader that reads `resources/i18n/{lang}/main.json`.

use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "en-US", "tr-TR", "es-ES", "zh-CN", "fr-FR", "uk-UA", "ru-RU", "pt-BR", "ja-JP",
];

#[derive(Debug, Clone)]
pub struct I18n {
    lang: String,
    strings: HashMap<String, String>,
}

impl Default for I18n {
    fn default() -> Self {
        Self::load("en-US")
    }
}

impl I18n {
    pub fn detect_language() -> String {
        let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string());
        if SUPPORTED_LANGUAGES.contains(&locale.as_str()) {
            locale
        } else if locale.starts_with("en") {
            "en-US".into()
        } else if locale.starts_with("tr") {
            "tr-TR".into()
        } else if locale.starts_with("es") {
            "es-ES".into()
        } else if locale.starts_with("zh") {
            "zh-CN".into()
        } else if locale.starts_with("fr") {
            "fr-FR".into()
        } else if locale.starts_with("uk") {
            "uk-UA".into()
        } else if locale.starts_with("ru") {
            "ru-RU".into()
        } else if locale.starts_with("pt") {
            "pt-BR".into()
        } else if locale.starts_with("ja") {
            "ja-JP".into()
        } else {
            "en-US".into()
        }
    }

    pub fn load(lang: &str) -> Self {
        let lang = if SUPPORTED_LANGUAGES.contains(&lang) {
            lang.to_string()
        } else {
            "en-US".into()
        };
        let mut strings = HashMap::new();
        if let Some(dir) = i18n_dir() {
            for file in ["main.json", "rclone.json", "rclone-providers.json"] {
                let path = dir.join(&lang).join(file);
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        flatten_json("", &value, &mut strings);
                    }
                }
            }
        }
        Self { lang, strings }
    }

    pub fn lang(&self) -> &str {
        &self.lang
    }

    /// True when the user picked a different catalog and the chrome should rebuild.
    pub fn language_changed(previous: &str, next: &str) -> bool {
        previous != next
    }

    pub fn t(&self, key: &str) -> String {
        self.strings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    pub fn t_or(&self, key: &str, fallback: &str) -> String {
        if self.has(key) {
            self.t(key)
        } else {
            fallback.to_string()
        }
    }

    pub fn tf(&self, key: &str, params: &[(&str, &str)]) -> String {
        interpolate(&self.t(key), params)
    }

    pub fn tf_or(&self, key: &str, fallback: &str, params: &[(&str, &str)]) -> String {
        interpolate(&self.t_or(key, fallback), params)
    }

    pub fn has(&self, key: &str) -> bool {
        self.strings.contains_key(key)
    }

    pub fn translate_backend(&self, message: &str) -> String {
        translate_backend_message(self, message)
    }

    /// Angular `RcloneOptionTranslatePipe`: camelCase / kebab → snake_case `.title` / `.help`.
    pub fn option_label(
        &self,
        name: &str,
        kind: &str,
        fallback: &str,
        provider: Option<&str>,
    ) -> String {
        if name.is_empty() {
            return fallback.to_string();
        }
        let normalized = normalize_option_name(name);
        if let Some(provider) = provider.filter(|p| !p.is_empty()) {
            let key = format!("providers.{provider}.{normalized}.{kind}");
            if self.has(&key) {
                return self.t(&key);
            }
        }
        let key = format!("{normalized}.{kind}");
        self.t_or(&key, fallback)
    }
}

pub fn normalize_option_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut chars = name.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '-' || ch == '.' {
            out.push('_');
            continue;
        }
        if ch.is_ascii_uppercase()
            && !out.is_empty()
            && out
                .chars()
                .last()
                .is_some_and(|prev| prev.is_ascii_lowercase() || prev.is_ascii_digit())
        {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

pub fn translate_backend_message(i18n: &I18n, message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(parsed) = parse_localized(trimmed) {
        return render_localized(i18n, &parsed);
    }
    if let Some((raw, parsed)) = extract_embedded(trimmed) {
        return trimmed.replacen(&raw, &render_localized(i18n, &parsed), 1);
    }
    if looks_like_key(trimmed) && (i18n.has(trimmed) || trimmed.contains('.')) {
        if i18n.has(trimmed) {
            return i18n.t(trimmed);
        }
    }
    message.to_string()
}

#[derive(Debug, Clone)]
struct LocalizedMessage {
    key: String,
    params: Vec<(String, String)>,
}

fn parse_localized(message: &str) -> Option<LocalizedMessage> {
    if !(message.starts_with('{') && message.ends_with('}')) {
        return None;
    }
    localized_from_value(&serde_json::from_str(message).ok()?)
}

fn extract_embedded(message: &str) -> Option<(String, LocalizedMessage)> {
    let start = message.find('{')?;
    let end = message.rfind('}')?;
    if end <= start {
        return None;
    }
    let raw = message[start..=end].to_string();
    let parsed = localized_from_value(&serde_json::from_str(&raw).ok()?)?;
    Some((raw, parsed))
}

fn localized_from_value(value: &Value) -> Option<LocalizedMessage> {
    let key = value.get("key")?.as_str()?.to_string();
    let mut params = Vec::new();
    if let Some(map) = value.get("params").and_then(|v| v.as_object()) {
        for (k, v) in map {
            let text = v
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string());
            params.push((k.clone(), text));
        }
    }
    Some(LocalizedMessage { key, params })
}

fn render_localized(i18n: &I18n, message: &LocalizedMessage) -> String {
    let pairs: Vec<(&str, &str)> = message
        .params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    if i18n.has(&message.key) {
        i18n.tf(&message.key, &pairs)
    } else {
        message.key.clone()
    }
}

fn looks_like_key(message: &str) -> bool {
    let trimmed = message.trim();
    !trimmed.contains(' ')
        && trimmed.contains('.')
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
}

fn interpolate(template: &str, params: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in params {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn flatten_json(prefix: &str, value: &Value, out: &mut HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_json(&next, v, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        other => {
            out.insert(prefix.to_string(), other.to_string());
        }
    }
}

pub fn i18n_dir() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/i18n"),
        PathBuf::from("resources/i18n"),
        PathBuf::from("../resources/i18n"),
        PathBuf::from("/usr/share/rclone-manager/i18n"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_changed_detects_catalog_switch() {
        assert!(I18n::language_changed("en-US", "tr-TR"));
        assert!(!I18n::language_changed("en-US", "en-US"));
        let en = I18n::load("en-US");
        let tr = I18n::load("tr-TR");
        assert_eq!(en.lang(), "en-US");
        assert_eq!(tr.lang(), "tr-TR");
        if en.has("settings.general.language.label") && tr.has("settings.general.language.label") {
            assert_ne!(
                en.t("settings.general.language.label"),
                tr.t("settings.general.language.label")
            );
        }
    }

    #[test]
    fn flatten_nested_keys() {
        let value = serde_json::json!({
            "tabs": { "general": "General", "mount": "Mount" },
            "plain": "Hello"
        });
        let mut map = HashMap::new();
        flatten_json("", &value, &mut map);
        assert_eq!(map.get("tabs.general").map(String::as_str), Some("General"));
        assert_eq!(map.get("plain").map(String::as_str), Some("Hello"));
    }

    #[test]
    fn interpolate_params() {
        let mut i18n = I18n {
            lang: "en-US".into(),
            strings: HashMap::new(),
        };
        i18n.strings
            .insert("hello".into(), "Hello {name}, id={{id}}".into());
        assert_eq!(
            i18n.tf("hello", &[("name", "Ada"), ("id", "7")]),
            "Hello Ada, id=7"
        );
        assert_eq!(
            i18n.tf_or(
                "missing.sendTo",
                "Added '{{remote}}{{path}}' to File Manager menu",
                &[("remote", "testdrive"), ("path", "")]
            ),
            "Added 'testdrive' to File Manager menu"
        );
    }

    #[test]
    fn send_to_catalog_keys_interpolate() {
        let i18n = I18n::load("en-US");
        if i18n_dir().is_none() {
            return;
        }
        assert!(
            i18n.has("nautilus.notifications.sendToAdded"),
            "nautilus.notifications.sendToAdded must exist in en-US"
        );
        assert!(i18n.has("nautilus.notifications.sendToRemoved"));
        assert!(i18n.has("nautilus.errors.sendToFailed"));
        assert_eq!(
            i18n.tf(
                "nautilus.notifications.sendToAdded",
                &[("remote", "testdrive"), ("path", "")]
            ),
            "Added 'testdrive' to File Manager menu"
        );
    }

    #[test]
    fn loads_english_main_if_repo_present() {
        let i18n = I18n::load("en-US");
        if i18n_dir().is_some() {
            assert!(
                i18n.has("tabs.general")
                    || i18n.t("tabs.general") != "tabs.general"
                    || !i18n.strings.is_empty()
            );
        }
    }

    #[test]
    fn restore_as_exists_in_all_catalogs() {
        if i18n_dir().is_none() {
            return;
        }
        for lang in SUPPORTED_LANGUAGES {
            let i18n = I18n::load(lang);
            assert!(
                i18n.has("backup.restore.restoreAs"),
                "backup.restore.restoreAs missing in {lang}"
            );
        }
    }

    #[test]
    fn detect_language_falls_back() {
        assert!(
            SUPPORTED_LANGUAGES.contains(&I18n::detect_language().as_str())
                || I18n::detect_language() == "en-US"
        );
    }

    #[test]
    fn translates_backend_json_and_keys() {
        let mut i18n = I18n {
            lang: "en-US".into(),
            strings: HashMap::new(),
        };
        i18n.strings.insert(
            "backendErrors.mount.alreadyInUse".into(),
            "Mount {name} is already in use".into(),
        );
        assert_eq!(
            i18n.translate_backend(
                r#"{"key":"backendErrors.mount.alreadyInUse","params":{"name":"drive"}}"#
            ),
            "Mount drive is already in use"
        );
        assert_eq!(
            i18n.translate_backend(
                r#"Start failed: {"key":"backendErrors.mount.alreadyInUse","params":{"name":"x"}}"#
            ),
            "Start failed: Mount x is already in use"
        );
        assert_eq!(
            i18n.translate_backend("backendErrors.mount.alreadyInUse"),
            "Mount {name} is already in use"
        );
        assert_eq!(i18n.translate_backend("legacy English"), "legacy English");
    }

    #[test]
    fn t_or_falls_back_when_missing() {
        let i18n = I18n {
            lang: "en-US".into(),
            strings: HashMap::new(),
        };
        assert_eq!(i18n.t_or("missing.key", "Fallback"), "Fallback");
    }

    #[test]
    fn option_label_normalizes_and_falls_back() {
        assert_eq!(
            normalize_option_name("createEmptySrcDirs"),
            "create_empty_src_dirs"
        );
        assert_eq!(normalize_option_name("allow-other"), "allow_other");
        assert_eq!(normalize_option_name("vfs.CacheMode"), "vfs_cache_mode");
        let mut i18n = I18n {
            lang: "en-US".into(),
            strings: HashMap::new(),
        };
        i18n.strings
            .insert("transfers.title".into(), "Transfers".into());
        i18n.strings
            .insert("providers.s3.acl.title".into(), "S3 ACL".into());
        assert_eq!(
            i18n.option_label("transfers", "title", "transfers", None),
            "Transfers"
        );
        assert_eq!(
            i18n.option_label("acl", "title", "acl", Some("s3")),
            "S3 ACL"
        );
        assert_eq!(
            i18n.option_label("unknownFlag", "title", "Unknown Flag", None),
            "Unknown Flag"
        );
    }

    #[test]
    fn repair_sheet_and_toast_keys_exist() {
        let i18n = I18n::default();
        for key in [
            "repairSheet.titles.missingRclone",
            "repairSheet.actions.installPlugin",
            "modals.remoteConfig.profile.noProfiles",
            "mount.successMount",
            "operations.successStart",
            "notification.title.engineConnectionFailed",
            "modals.about.downloading",
            "rcloneUpdate.cancelled",
            "fileBrowser.fileViewer.openingNative",
            "overviews.status.labels.mounted",
            "serve.serving",
            "automation.status.running",
            "overviews.status.labels.inactive",
            "modals.oauth.manualOpenPrompt",
            "modals.oauth.copyLink",
            "modals.remoteConfig.errors.interactiveProcessingFailed",
            "generalOverview.editLayout",
            "dashboard.statusOverview.title",
            "nautilus.contextMenu.openNative",
            "nautilus.contextMenu.openNewTab",
            "developerTools.debugInfo",
            "wizards.remoteConfig.togglePassword",
            "common.engineOffline",
            "titlebar.menu.memoryUnavailable",
            "tray.tooltipSubtitle",
            "nautilus.notifications.nothingToUndo",
            "nautilus.titles.editPath",
            "nautilus.notifications.selectValid",
            "home.options.cloneFailed",
            "modals.about.killRclone",
            "modals.about.backendCache",
            "nautilus.notifications.sendToAdded",
            "nautilus.notifications.sendToRemoved",
            "nautilus.errors.sendToFailed",
        ] {
            assert!(i18n.has(key), "missing i18n key {key}");
        }
    }
}
