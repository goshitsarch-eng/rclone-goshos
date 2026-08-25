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
}
