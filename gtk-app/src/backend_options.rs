//! Persisted rclone `options/set` payload (`backend.json`), matching the Tauri store.

use crate::rclone::RcClient;
use crate::settings::AppSettings;
use serde_json::{json, Value};
use std::path::PathBuf;

pub fn store_path() -> PathBuf {
    AppSettings::config_dir().join("backend.json")
}

pub fn load_all() -> Value {
    let path = store_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

pub fn load_for(backend_key: &str) -> Value {
    let all = load_all();
    all.get(backend_key)
        .cloned()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| json!({}))
}

pub fn save_all(value: &Value) -> Result<(), String> {
    let dir = AppSettings::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(store_path(), text).map_err(|e| e.to_string())
}

pub fn save_for(backend_key: &str, options: &Value) -> Result<(), String> {
    let mut all = load_all();
    let obj = all.as_object_mut().cloned().unwrap_or_default();
    let mut root = obj;
    root.insert(backend_key.to_string(), options.clone());
    save_all(&Value::Object(root))
}

pub fn merge_and_save(backend_key: &str, delta: &Value) -> Result<Value, String> {
    let mut current = load_for(backend_key);
    merge_maps(&mut current, delta);
    save_for(backend_key, &current)?;
    Ok(current)
}

pub fn apply(client: &RcClient, backend_key: &str) {
    let options = load_for(backend_key);
    if options.as_object().is_some_and(|o| !o.is_empty()) {
        if let Err(e) = client.options_set(options) {
            log::warn!("failed to apply persisted backend options: {e}");
        }
    }
}

fn merge_maps(dest: &mut Value, src: &Value) {
    match (dest, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                merge_maps(d.entry(k.clone()).or_insert(json!({})), v);
            }
        }
        (dest, src) => *dest = src.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_nested_option_blocks() {
        let mut current =
            json!({ "main": { "LogLevel": "NOTICE" }, "vfs": { "CacheMode": "off" } });
        merge_maps(
            &mut current,
            &json!({ "main": { "Transfers": 8 }, "vfs": { "CacheMode": "full" } }),
        );
        assert_eq!(current["main"]["LogLevel"], "NOTICE");
        assert_eq!(current["main"]["Transfers"], 8);
        assert_eq!(current["vfs"]["CacheMode"], "full");
    }

    #[test]
    fn load_missing_backend_is_empty_object() {
        let all = json!({ "local": { "main": { "LogLevel": "DEBUG" } } });
        let missing = all
            .get("other")
            .cloned()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({}));
        assert_eq!(missing, json!({}));
        assert_eq!(all["local"]["main"]["LogLevel"], "DEBUG");
    }
}
