//! User preset templates — Angular `UserPresetTemplate` / `TEMPLATE_CATEGORIES`.

use crate::operations::OperationType;
use crate::store::{ProfileConfig, RemoteMeta};
use serde_json::{Map, Value};

/// Flag-type keys plus `remote`, matching Angular `TEMPLATE_CATEGORIES`.
pub const TEMPLATE_CATEGORIES: &[&str] = &[
    "mount",
    "sync",
    "copy",
    "move",
    "bisync",
    "serve",
    "check",
    "delete",
    "copyurl",
    "archivecreate",
    "cryptcheck",
    "filter",
    "vfs",
    "backend",
    "remote",
];

pub fn is_template_category(value: &str) -> bool {
    TEMPLATE_CATEGORIES
        .iter()
        .any(|category| *category == value)
}

/// True when every key is a template category and every value is an object.
pub fn is_categorized(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.is_empty() {
        return false;
    }
    obj.iter()
        .all(|(key, val)| is_template_category(key) && (val.is_object() || val.is_null()))
}

pub fn merge_category(dest: &mut Map<String, Value>, src: &Map<String, Value>, overwrite: bool) {
    for (key, value) in src {
        if overwrite || !dest.contains_key(key) {
            dest.insert(key.clone(), value.clone());
        }
    }
}

fn helper_kind(category: &str) -> Option<&'static str> {
    match category {
        "vfs" => Some("vfs"),
        "filter" => Some("filter"),
        "backend" => Some("backend"),
        "runtime" | "runtimeRemote" => Some("runtime"),
        _ => None,
    }
}

fn default_helper_name(meta: &RemoteMeta, kind: &str) -> String {
    meta.helper_names(kind)
        .into_iter()
        .next()
        .unwrap_or_else(|| "default".into())
}

fn default_profile_name(meta: &RemoteMeta, op: OperationType) -> String {
    meta.profile_names(op)
        .into_iter()
        .next()
        .unwrap_or_else(|| "default".into())
}

/// Apply categorized template values onto helper maps and operation profiles.
pub fn apply_to_meta(
    meta: &mut RemoteMeta,
    values: &Value,
    categories: Option<&[&str]>,
    overwrite: bool,
) -> usize {
    let Some(obj) = values.as_object() else {
        return 0;
    };
    let wanted: Vec<&str> = match categories {
        Some(list) if !list.is_empty() => list.to_vec(),
        _ => obj.keys().map(|key| key.as_str()).collect(),
    };
    let mut applied = 0;
    for category in wanted {
        if !is_template_category(category) {
            continue;
        }
        let Some(src) = obj.get(category).and_then(|value| value.as_object()) else {
            continue;
        };
        if src.is_empty() {
            continue;
        }
        if let Some(kind) = helper_kind(category) {
            let name = default_helper_name(meta, kind);
            let mut current = meta
                .helper_profile(kind, &name)
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            merge_category(&mut current, src, overwrite);
            meta.upsert_helper(kind, &name, Value::Object(current));
            applied += 1;
            continue;
        }
        if category == "remote" {
            let mut current = meta
                .helper_profile("runtime", "remote")
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            merge_category(&mut current, src, overwrite);
            meta.upsert_helper("runtime", "remote", Value::Object(current));
            applied += 1;
            continue;
        }
        if let Some(op) = OperationType::parse(category) {
            let name = default_profile_name(meta, op);
            let mut profile = meta
                .get_profile(op, &name)
                .unwrap_or_else(|| ProfileConfig {
                    name: name.clone(),
                    ..ProfileConfig::default()
                });
            let mut rclone = profile.rclone.as_object().cloned().unwrap_or_default();
            merge_category(&mut rclone, src, overwrite);
            profile.rclone = Value::Object(rclone);
            meta.upsert_profile(op, profile);
            applied += 1;
        }
    }
    applied
}

/// Snapshot helper + operation values for the selected categories.
pub fn capture_from_meta(meta: &RemoteMeta, categories: &[&str]) -> Value {
    let selected: Vec<&str> = if categories.is_empty() {
        TEMPLATE_CATEGORIES.to_vec()
    } else {
        categories.to_vec()
    };
    let mut out = Map::new();
    for category in selected {
        if let Some(kind) = helper_kind(category) {
            if let Some(name) = meta.helper_names(kind).into_iter().next() {
                if let Some(value) = meta.helper_profile(kind, &name) {
                    if value.as_object().is_some_and(|obj| !obj.is_empty()) {
                        out.insert(category.to_string(), value);
                    }
                }
            }
            continue;
        }
        if category == "remote" {
            if let Some(value) = meta.helper_profile("runtime", "remote") {
                if value.as_object().is_some_and(|obj| !obj.is_empty()) {
                    out.insert(category.to_string(), value);
                }
            }
            continue;
        }
        if let Some(op) = OperationType::parse(category) {
            if let Some(name) = meta.profile_names(op).into_iter().next() {
                if let Some(profile) = meta.get_profile(op, &name) {
                    if profile
                        .rclone
                        .as_object()
                        .is_some_and(|obj| !obj.is_empty())
                    {
                        out.insert(category.to_string(), profile.rclone);
                    }
                }
            }
        }
    }
    Value::Object(out)
}

/// Apply categorized values when `meta` is present. Returns 0 for flat `options/set` JSON.
pub fn apply_if_categorized(
    meta: Option<&mut RemoteMeta>,
    values: &Value,
    overwrite: bool,
) -> usize {
    if !is_categorized(values) {
        return 0;
    }
    let Some(meta) = meta else {
        return 0;
    };
    apply_to_meta(meta, values, None, overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn category_set_matches_angular() {
        assert!(is_template_category("vfs"));
        assert!(is_template_category("remote"));
        assert!(is_template_category("archivecreate"));
        assert!(!is_template_category("main"));
        assert!(!is_template_category("network"));
        assert_eq!(TEMPLATE_CATEGORIES.len(), 15);
    }

    #[test]
    fn detects_categorized_versus_flat_options() {
        assert!(is_categorized(&json!({
            "vfs": { "vfs_cache_mode": "full" },
            "mount": { "attr_timeout": "10s" }
        })));
        assert!(!is_categorized(&json!({
            "main": { "transfers": 8 },
            "filter": { "max_age": "1d" }
        })));
        assert!(!is_categorized(&json!({})));
        assert!(!is_categorized(&json!(["vfs"])));
        assert!(!is_categorized(&json!({ "vfs": "full" })));
    }

    #[test]
    fn apply_merges_helpers_and_profiles() {
        let mut meta = RemoteMeta::default();
        meta.upsert_helper("vfs", "fast", json!({ "vfs_cache_mode": "off" }));
        let applied = apply_to_meta(
            &mut meta,
            &json!({
                "vfs": { "vfs_cache_mode": "full", "vfs_read_ahead": "128M" },
                "mount": { "attr_timeout": "10s" },
                "filter": { "max_age": "7d" },
                "remote": { "chunk_size": "64M" }
            }),
            None,
            true,
        );
        assert_eq!(applied, 4);
        assert_eq!(
            meta.helper_profile("vfs", "fast").unwrap()["vfs_cache_mode"],
            "full"
        );
        assert_eq!(
            meta.helper_profile("vfs", "fast").unwrap()["vfs_read_ahead"],
            "128M"
        );
        assert_eq!(
            meta.get_profile(OperationType::Mount, "default")
                .unwrap()
                .rclone["attr_timeout"],
            "10s"
        );
        assert_eq!(
            meta.helper_profile("filter", "default").unwrap()["max_age"],
            "7d"
        );
        assert_eq!(
            meta.helper_profile("runtime", "remote").unwrap()["chunk_size"],
            "64M"
        );
    }

    #[test]
    fn apply_respects_category_filter_and_overwrite() {
        let mut meta = RemoteMeta::default();
        meta.upsert_helper("vfs", "default", json!({ "vfs_cache_mode": "off" }));
        apply_to_meta(
            &mut meta,
            &json!({
                "vfs": { "vfs_cache_mode": "full", "vfs_read_ahead": "64M" },
                "mount": { "attr_timeout": "1s" }
            }),
            Some(&["vfs"]),
            false,
        );
        let vfs = meta.helper_profile("vfs", "default").unwrap();
        assert_eq!(vfs["vfs_cache_mode"], "off");
        assert_eq!(vfs["vfs_read_ahead"], "64M");
        assert!(meta.get_profile(OperationType::Mount, "default").is_none());
    }

    #[test]
    fn capture_round_trips_helpers_and_ops() {
        let mut meta = RemoteMeta::default();
        meta.upsert_helper("backend", "default", json!({ "transfers": 8 }));
        meta.upsert_profile(
            OperationType::Sync,
            ProfileConfig {
                name: "nightly".into(),
                rclone: json!({ "createEmptySrcDirs": true }),
                ..ProfileConfig::default()
            },
        );
        let captured = capture_from_meta(&meta, &[]);
        assert_eq!(captured["backend"]["transfers"], 8);
        assert_eq!(captured["sync"]["createEmptySrcDirs"], true);
        assert!(captured.get("vfs").is_none());
        let mut other = RemoteMeta::default();
        assert_eq!(apply_if_categorized(Some(&mut other), &captured, true), 2);
        assert_eq!(
            other.helper_profile("backend", "default").unwrap()["transfers"],
            8
        );
        assert_eq!(apply_if_categorized(None, &captured, true), 0);
        assert_eq!(
            apply_if_categorized(
                Some(&mut other),
                &json!({ "main": { "transfers": 4 } }),
                true
            ),
            0
        );
    }
}
