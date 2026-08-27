//! User preset templates — Angular `UserPresetTemplate` / `TEMPLATE_CATEGORIES`.

use crate::operations::OperationType;
use crate::presets::PresetValues;
use crate::store::{ProfileConfig, RemoteMeta, UserTemplate};
use serde_json::{json, Map, Value};

/// Path keys Angular skips when patching Quick Run flag groups.
pub const QUICK_RUN_PATH_KEYS: &[&str] = &[
    "srcFs",
    "dstFs",
    "path1",
    "path2",
    "fs",
    "mountPoint",
    "source",
    "dest",
];

/// Form-level patch produced by Apply Default Presets / Apply Template on a Quick Run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuickRunFormPatch {
    pub vfs: Map<String, Value>,
    pub filter: Map<String, Value>,
    pub backend: Map<String, Value>,
    pub mount: Map<String, Value>,
    pub operation: Map<String, Value>,
    pub runtime: Map<String, Value>,
    pub source: Option<String>,
    pub dest: Option<String>,
}

pub fn is_path_key(key: &str) -> bool {
    QUICK_RUN_PATH_KEYS.iter().any(|wanted| *wanted == key)
}

pub fn normalize_flag_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

pub fn flag_keys_match(field: &str, preset: &str) -> bool {
    if field == preset {
        return true;
    }
    let field = normalize_flag_key(field);
    let preset = normalize_flag_key(preset);
    if field.is_empty() || preset.is_empty() {
        return false;
    }
    field == preset || field.ends_with(&preset) || preset.ends_with(&field)
}

pub fn category_object<'a>(values: &'a Value, category: &str) -> Option<&'a Map<String, Value>> {
    values.get(category).and_then(Value::as_object)
}

pub fn first_string(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    })
}

pub fn without_path_keys(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .filter(|(key, _)| !is_path_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub fn lookup_flag_value<'a>(values: &'a Map<String, Value>, field: &str) -> Option<&'a Value> {
    values.get(field).or_else(|| {
        values
            .iter()
            .find(|(key, _)| flag_keys_match(field, key))
            .map(|(_, value)| value)
    })
}

/// Angular `applyDefaultPresets()` — vfs / backend / mount (when Mount) / runtimeRemote.
pub fn default_presets_patch(presets: &PresetValues, op: OperationType) -> QuickRunFormPatch {
    QuickRunFormPatch {
        vfs: presets.vfs.clone(),
        backend: presets.backend.clone(),
        mount: if op == OperationType::Mount {
            presets.mount.clone()
        } else {
            Map::new()
        },
        runtime: presets.remote.clone(),
        ..QuickRunFormPatch::default()
    }
}

/// Angular `onApplyTemplate()` — categorized maps plus src/dst from the current operation.
pub fn template_form_patch(values: &Value, op: OperationType) -> QuickRunFormPatch {
    let op_map = category_object(values, op.as_str())
        .cloned()
        .unwrap_or_default();
    QuickRunFormPatch {
        vfs: category_object(values, "vfs").cloned().unwrap_or_default(),
        filter: category_object(values, "filter")
            .cloned()
            .unwrap_or_default(),
        backend: category_object(values, "backend")
            .cloned()
            .unwrap_or_default(),
        mount: category_object(values, "mount")
            .cloned()
            .unwrap_or_default(),
        source: first_string(&op_map, &["srcFs", "path1", "fs", "source"]),
        dest: first_string(&op_map, &["mountPoint", "dstFs", "path2", "dest"]),
        operation: without_path_keys(&op_map),
        runtime: category_object(values, "remote")
            .or_else(|| category_object(values, "runtimeRemote"))
            .cloned()
            .unwrap_or_default(),
    }
}

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

fn display_leaf(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn flatten_inner(value: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                let child = &map[key];
                if child.is_object() {
                    flatten_inner(child, &path, out);
                } else {
                    out.push((path, display_leaf(child)));
                }
            }
        }
        other if !prefix.is_empty() => out.push((prefix.to_string(), display_leaf(other))),
        _ => {}
    }
}

/// Dotted paths of every non-object leaf, for the template key picker.
pub fn flatten_leaf_paths(value: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    flatten_inner(value, "", &mut out);
    out
}

fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn set_path(dest: &mut Value, path: &str, leaf: Value) {
    let mut parts = path.split('.').peekable();
    let mut cursor = dest;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            if let Some(map) = cursor.as_object_mut() {
                map.insert(part.to_string(), leaf);
            }
            return;
        }
        if !cursor.is_object() {
            *cursor = Value::Object(Map::new());
        }
        let map = cursor.as_object_mut().expect("object");
        cursor = map.entry(part.to_string()).or_insert(json!({}));
        if !cursor.is_object() {
            *cursor = json!({});
        }
    }
}

/// Deep-merge `src` into `dest`. Object keys are merged; leaves overwrite when asked.
pub fn merge_values(dest: &mut Value, src: &Value, overwrite: bool) {
    match (dest, src) {
        (Value::Object(dest_map), Value::Object(src_map)) => {
            for (key, value) in src_map {
                if let Some(existing) = dest_map.get_mut(key) {
                    merge_values(existing, value, overwrite);
                } else {
                    dest_map.insert(key.clone(), value.clone());
                }
            }
        }
        (dest, src) if overwrite || dest.is_null() => *dest = src.clone(),
        _ => {}
    }
}

pub fn template_matches_query(template: &UserTemplate, query: &str) -> bool {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return true;
    }
    if template.name.to_ascii_lowercase().contains(&needle)
        || template.description.to_ascii_lowercase().contains(&needle)
    {
        return true;
    }
    flatten_leaf_paths(&template.values)
        .iter()
        .any(|(path, value)| {
            path.to_ascii_lowercase().contains(&needle)
                || value.to_ascii_lowercase().contains(&needle)
        })
}

/// Set a dotted leaf path, creating parent objects as needed.
pub fn set_leaf(dest: &mut Value, path: &str, leaf: Value) {
    if path.trim().is_empty() {
        return;
    }
    set_path(dest, path, leaf);
}

/// Remove a dotted leaf path. Empty parent objects are left in place.
pub fn remove_leaf(dest: &mut Value, path: &str) {
    let mut parts: Vec<&str> = path.split('.').collect();
    let Some(last) = parts.pop() else {
        return;
    };
    let mut cursor = dest;
    for part in parts {
        match cursor.as_object_mut() {
            Some(map) => {
                cursor = match map.get_mut(part) {
                    Some(child) => child,
                    None => return,
                };
            }
            None => return,
        }
    }
    if let Some(map) = cursor.as_object_mut() {
        map.remove(last);
    }
}

/// Keep only the selected dotted leaf paths.
pub fn filter_by_paths(value: &Value, paths: &[String]) -> Value {
    let mut out = json!({});
    for path in paths {
        if let Some(leaf) = get_path(value, path) {
            set_path(&mut out, path, leaf.clone());
        }
    }
    out
}

pub fn leaf_count(value: &Value) -> usize {
    flatten_leaf_paths(value).len()
}

/// Insert or replace a template by `id` without holding other borrows.
pub fn upsert_template(templates: &mut Vec<UserTemplate>, template: UserTemplate) {
    if let Some(slot) = templates.iter_mut().find(|item| item.id == template.id) {
        *slot = template;
    } else {
        templates.push(template);
    }
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

    #[test]
    fn flattens_and_filters_leaf_paths() {
        let value = json!({
            "main": { "transfers": 8, "checkers": 4 },
            "filter": { "max_age": "1d" },
            "empty": {}
        });
        let paths = flatten_leaf_paths(&value);
        assert_eq!(
            paths,
            vec![
                ("filter.max_age".into(), "1d".into()),
                ("main.checkers".into(), "4".into()),
                ("main.transfers".into(), "8".into()),
            ]
        );
        assert_eq!(leaf_count(&value), 3);
        let filtered = filter_by_paths(&value, &["main.transfers".into(), "missing".into()]);
        assert_eq!(filtered["main"]["transfers"], 8);
        assert!(filtered.get("filter").is_none());
        assert!(filtered["main"].get("checkers").is_none());
        assert!(flatten_leaf_paths(&json!({})).is_empty());
        assert!(flatten_leaf_paths(&json!(null)).is_empty());
    }

    #[test]
    fn set_and_remove_leaf_paths() {
        let mut value = json!({ "vfs": { "vfs_cache_mode": "off" } });
        set_leaf(&mut value, "vfs.vfs_read_ahead", json!("128M"));
        set_leaf(&mut value, "mount.attr_timeout", json!("10s"));
        assert_eq!(value["vfs"]["vfs_read_ahead"], "128M");
        assert_eq!(value["mount"]["attr_timeout"], "10s");
        remove_leaf(&mut value, "vfs.vfs_cache_mode");
        assert!(value["vfs"].get("vfs_cache_mode").is_none());
        assert_eq!(value["vfs"]["vfs_read_ahead"], "128M");
        remove_leaf(&mut value, "missing.path");
        set_leaf(&mut value, "", json!("ignored"));
        assert_eq!(value["vfs"]["vfs_read_ahead"], "128M");
    }

    #[test]
    fn merge_values_overwrites_leaves_and_keeps_siblings() {
        let mut dest = json!({
            "vfs": { "vfs_cache_mode": "off", "vfs_read_ahead": "32M" },
            "filter": { "max_age": "1d" }
        });
        merge_values(
            &mut dest,
            &json!({
                "vfs": { "vfs_cache_mode": "full" },
                "mount": { "attr_timeout": "10s" }
            }),
            true,
        );
        assert_eq!(dest["vfs"]["vfs_cache_mode"], "full");
        assert_eq!(dest["vfs"]["vfs_read_ahead"], "32M");
        assert_eq!(dest["filter"]["max_age"], "1d");
        assert_eq!(dest["mount"]["attr_timeout"], "10s");
    }

    #[test]
    fn template_query_matches_name_description_and_keys() {
        let template = UserTemplate {
            id: "1".into(),
            name: "Fast S3".into(),
            description: "cache full".into(),
            icon: "emblem-ok-symbolic".into(),
            created_at: "t0".into(),
            updated_at: "t0".into(),
            values: json!({ "vfs": { "vfs_cache_mode": "full" } }),
        };
        assert!(template_matches_query(&template, ""));
        assert!(template_matches_query(&template, "s3"));
        assert!(template_matches_query(&template, "CACHE"));
        assert!(template_matches_query(&template, "vfs_cache"));
        assert!(template_matches_query(&template, "full"));
        assert!(!template_matches_query(&template, "mount"));
    }

    #[test]
    fn upsert_replaces_matching_id_and_appends_new() {
        let mut templates = vec![UserTemplate {
            id: "keep".into(),
            name: "Old".into(),
            description: "a".into(),
            icon: "emblem-ok-symbolic".into(),
            created_at: "t0".into(),
            updated_at: "t0".into(),
            values: json!({ "main": { "transfers": 4 } }),
        }];
        upsert_template(
            &mut templates,
            UserTemplate {
                id: "keep".into(),
                name: "Renamed".into(),
                description: "b".into(),
                icon: "emblem-ok-symbolic".into(),
                created_at: "t0".into(),
                updated_at: "t1".into(),
                values: json!({ "main": { "transfers": 8 } }),
            },
        );
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "Renamed");
        assert_eq!(templates[0].values["main"]["transfers"], 8);
        upsert_template(
            &mut templates,
            UserTemplate {
                id: "other".into(),
                name: "Second".into(),
                description: String::new(),
                icon: "emblem-ok-symbolic".into(),
                created_at: "t2".into(),
                updated_at: "t2".into(),
                values: json!({}),
            },
        );
        assert_eq!(templates.len(), 2);
        assert_eq!(templates[1].name, "Second");
    }

    #[test]
    fn flag_keys_match_ignores_case_and_underscores() {
        assert!(flag_keys_match("vfs_cache_mode", "VfsCacheMode"));
        assert!(flag_keys_match("VFS.CacheMode", "vfs_cache_mode"));
        assert!(flag_keys_match("chunk_size", "chunk_size"));
        assert!(!flag_keys_match("transfers", "checkers"));
        assert!(!flag_keys_match("", "vfs"));
        assert_eq!(
            normalize_flag_key("HTTP-Options.Listen_Addr"),
            "httpoptionslistenaddr"
        );
    }

    #[test]
    fn default_presets_patch_includes_mount_only_for_mount() {
        let presets = crate::presets::resolve_presets("alias", None, "linux");
        let copy = default_presets_patch(&presets, OperationType::Copy);
        assert!(!copy.vfs.is_empty());
        assert!(!copy.backend.is_empty());
        assert!(copy.mount.is_empty());
        assert!(copy.runtime.is_empty());
        let mount = default_presets_patch(&presets, OperationType::Mount);
        assert!(!mount.mount.is_empty());
        assert_eq!(mount.mount["attr_timeout"], "10s");
    }

    #[test]
    fn template_form_patch_extracts_paths_and_strips_them_from_flags() {
        let values = json!({
            "vfs": { "vfs_cache_mode": "full" },
            "filter": { "max_age": "7d" },
            "backend": { "transfers": 8 },
            "copy": {
                "srcFs": "testdrive:Photos",
                "dstFs": "testdrive2:backup",
                "createEmptySrcDirs": true
            },
            "remote": { "chunk_size": "32M" }
        });
        let patch = template_form_patch(&values, OperationType::Copy);
        assert_eq!(patch.source.as_deref(), Some("testdrive:Photos"));
        assert_eq!(patch.dest.as_deref(), Some("testdrive2:backup"));
        assert_eq!(patch.operation["createEmptySrcDirs"], true);
        assert!(patch.operation.get("srcFs").is_none());
        assert!(patch.operation.get("dstFs").is_none());
        assert_eq!(patch.vfs["vfs_cache_mode"], "full");
        assert_eq!(patch.filter["max_age"], "7d");
        assert_eq!(patch.backend["transfers"], 8);
        assert_eq!(patch.runtime["chunk_size"], "32M");
        assert!(lookup_flag_value(&patch.vfs, "VfsCacheMode").is_some());
        let empty = template_form_patch(&json!({}), OperationType::Sync);
        assert!(empty.source.is_none());
        assert!(empty.operation.is_empty());
    }
}
