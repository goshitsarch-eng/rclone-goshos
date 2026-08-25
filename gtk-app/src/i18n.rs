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

    pub fn t(&self, key: &str) -> String {
        self.strings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    pub fn tf(&self, key: &str, params: &[(&str, &str)]) -> String {
        let mut out = self.t(key);
        for (name, value) in params {
            out = out.replace(&format!("{{{{{name}}}}}"), value);
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }

    pub fn has(&self, key: &str) -> bool {
        self.strings.contains_key(key)
    }

    pub fn translate_backend(&self, message: &str) -> String {
        translate_backend_message(self, message)
    }
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
}
