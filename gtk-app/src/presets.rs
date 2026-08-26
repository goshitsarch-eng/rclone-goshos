//! Provider / OS remote presets — port of Angular `remote-presets.ts`.

use crate::operations::OperationType;
use crate::store::{ProfileConfig, RemoteMeta};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFamily {
    S3,
    Webdav,
    Generic,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresetValues {
    pub vfs: Map<String, Value>,
    pub mount: Map<String, Value>,
    pub backend: Map<String, Value>,
    pub remote: Map<String, Value>,
}

fn normalize_type(remote_type: &str) -> String {
    remote_type.to_ascii_lowercase().replace([' ', '_'], "")
}

pub fn storage_family(remote_type: &str) -> StorageFamily {
    if remote_type.is_empty() {
        return StorageFamily::Generic;
    }
    match normalize_type(remote_type).as_str() {
        "s3" | "b2" | "gcs" | "googlecloudstorage" => StorageFamily::S3,
        "webdav" => StorageFamily::Webdav,
        _ => StorageFamily::Generic,
    }
}

fn base_preset() -> PresetValues {
    PresetValues {
        vfs: map([
            ("vfs_cache_mode", json!("full")),
            ("vfs_cache_max_size", json!("250G")),
            ("vfs_cache_min_free_space", json!("10G")),
            ("vfs_cache_max_age", json!("48h")),
            ("vfs_write_back", json!("15s")),
            ("vfs_read_chunk_size", json!("16M")),
            ("vfs_read_chunk_streams", json!(8)),
            ("vfs_read_ahead", json!("128M")),
            ("vfs_refresh", json!(true)),
        ]),
        mount: map([("attr_timeout", json!("10s"))]),
        backend: map([
            ("buffer_size", json!("32M")),
            ("max_buffer_memory", json!("2G")),
            ("log_level", json!("INFO")),
            ("transfers", json!(8)),
        ]),
        remote: Map::new(),
    }
}

fn family_preset(family: StorageFamily) -> PresetValues {
    match family {
        StorageFamily::S3 => PresetValues {
            backend: map([
                ("disable_http2", json!(true)),
                ("use_server_mod_time", json!(true)),
            ]),
            vfs: map([("vfs_fast_fingerprint", json!(true))]),
            ..PresetValues::default()
        },
        StorageFamily::Webdav => PresetValues {
            vfs: map([("vfs_write_back", json!("20s"))]),
            ..PresetValues::default()
        },
        StorageFamily::Generic => PresetValues::default(),
    }
}

fn provider_preset(remote_type: &str) -> PresetValues {
    match normalize_type(remote_type).as_str() {
        "s3" | "b2" => PresetValues {
            remote: map([
                ("disable_checksum", json!(true)),
                ("upload_concurrency", json!(8)),
                ("chunk_size", json!("32M")),
            ]),
            ..PresetValues::default()
        },
        _ => PresetValues::default(),
    }
}

fn vendor_preset(remote_type: &str, vendor: Option<&str>) -> PresetValues {
    let Some(vendor) = vendor.map(normalize_type) else {
        return PresetValues::default();
    };
    if normalize_type(remote_type) == "webdav"
        && matches!(vendor.as_str(), "nextcloud" | "owncloud")
    {
        return PresetValues {
            remote: map([("nextcloud_chunk_size", json!("64M"))]),
            ..PresetValues::default()
        };
    }
    PresetValues::default()
}

fn os_preset(os: &str) -> PresetValues {
    let lower = os.to_ascii_lowercase();
    if lower.contains("android") {
        return PresetValues {
            vfs: map([
                ("vfs_cache_mode", json!("full")),
                ("vfs_cache_max_size", json!("50G")),
                ("vfs_cache_min_free_space", json!("2G")),
                ("vfs_cache_max_age", json!("24h")),
                ("vfs_write_back", json!("10s")),
            ]),
            mount: map([("mountType", json!("saf"))]),
            ..PresetValues::default()
        };
    }
    if lower.contains("darwin") || lower.contains("mac") || lower.contains("ios") {
        return PresetValues {
            mount: map([
                ("no_apple_xattr", json!(true)),
                ("no_apple_double", json!(true)),
            ]),
            ..PresetValues::default()
        };
    }
    if lower.starts_with("win") || lower.contains("windows") {
        return PresetValues {
            mount: map([("network_mode", json!(true))]),
            ..PresetValues::default()
        };
    }
    PresetValues::default()
}

fn map(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

pub fn merge_presets(target: &PresetValues, source: &PresetValues) -> PresetValues {
    PresetValues {
        vfs: merge_map(&target.vfs, &source.vfs),
        mount: merge_map(&target.mount, &source.mount),
        backend: merge_map(&target.backend, &source.backend),
        remote: merge_map(&target.remote, &source.remote),
    }
}

fn merge_map(target: &Map<String, Value>, source: &Map<String, Value>) -> Map<String, Value> {
    let mut out = target.clone();
    for (k, v) in source {
        out.insert(k.clone(), v.clone());
    }
    out
}

impl PresetValues {
    /// Categorized JSON used by the template manager (Angular `resolvePresets('')`).
    pub fn to_template_value(&self) -> Value {
        let mut obj = Map::new();
        if !self.vfs.is_empty() {
            obj.insert("vfs".into(), Value::Object(self.vfs.clone()));
        }
        if !self.mount.is_empty() {
            obj.insert("mount".into(), Value::Object(self.mount.clone()));
        }
        if !self.backend.is_empty() {
            obj.insert("backend".into(), Value::Object(self.backend.clone()));
        }
        if !self.remote.is_empty() {
            obj.insert("remote".into(), Value::Object(self.remote.clone()));
        }
        Value::Object(obj)
    }
}

/// Base + OS presets only — empty remote type, matching Angular Apply Default Presets.
pub fn default_template_presets(os: &str) -> Value {
    resolve_presets("", None, os).to_template_value()
}

pub fn resolve_presets(remote_type: &str, vendor: Option<&str>, os: &str) -> PresetValues {
    let mut merged = base_preset();
    merged = merge_presets(&merged, &family_preset(storage_family(remote_type)));
    merged = merge_presets(&merged, &provider_preset(remote_type));
    merged = merge_presets(&merged, &vendor_preset(remote_type, vendor));
    merge_presets(&merged, &os_preset(os))
}

pub fn apply_to_remote_meta(meta: &mut RemoteMeta, presets: &PresetValues) {
    if !presets.vfs.is_empty() {
        let mut current = meta
            .helper_profile("vfs", "default")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        current = merge_map(&current, &presets.vfs);
        meta.upsert_helper("vfs", "default", Value::Object(current));
    }
    if !presets.backend.is_empty() {
        let mut current = meta
            .helper_profile("backend", "default")
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        current = merge_map(&current, &presets.backend);
        meta.upsert_helper("backend", "default", Value::Object(current));
    }
    if !presets.mount.is_empty() {
        let mut profile = meta
            .get_profile(OperationType::Mount, "default")
            .unwrap_or_else(|| ProfileConfig {
                name: "default".into(),
                ..ProfileConfig::default()
            });
        let mut rclone = profile.rclone.as_object().cloned().unwrap_or_default();
        rclone = merge_map(&rclone, &presets.mount);
        profile.rclone = Value::Object(rclone);
        meta.upsert_profile(OperationType::Mount, profile);
    }
}

pub fn merge_remote_params(params: &mut Value, presets: &PresetValues) {
    let Some(obj) = params.as_object_mut() else {
        return;
    };
    for (k, v) in &presets.remote {
        obj.entry(k.clone()).or_insert(v.clone());
    }
}

pub fn helper_names_from_meta(meta: &RemoteMeta) -> HashMap<String, Vec<String>> {
    ["vfs", "filter", "backend", "runtime"]
        .into_iter()
        .map(|kind| (kind.to_string(), meta.helper_names(kind)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_storage_families() {
        assert_eq!(storage_family("s3"), StorageFamily::S3);
        assert_eq!(storage_family("b2"), StorageFamily::S3);
        assert_eq!(storage_family("gcs"), StorageFamily::S3);
        assert_eq!(storage_family("Google Cloud Storage"), StorageFamily::S3);
        assert_eq!(storage_family("webdav"), StorageFamily::Webdav);
        assert_eq!(storage_family("sftp"), StorageFamily::Generic);
        assert_eq!(storage_family(""), StorageFamily::Generic);
    }

    #[test]
    fn applies_base_preset() {
        let presets = resolve_presets("sftp", None, "linux");
        assert_eq!(presets.vfs["vfs_cache_mode"], "full");
        assert_eq!(presets.backend["log_level"], "INFO");
        assert!(presets.mount.get("network_mode").is_none());
    }

    #[test]
    fn merges_family_and_provider() {
        let s3 = resolve_presets("s3", None, "linux");
        assert_eq!(s3.backend["disable_http2"], true);
        assert_eq!(s3.vfs["vfs_fast_fingerprint"], true);
        assert_eq!(s3.vfs["vfs_cache_mode"], "full");
        let b2 = resolve_presets("b2", None, "linux");
        assert_eq!(b2.remote["disable_checksum"], true);
        assert_eq!(b2.remote["upload_concurrency"], 8);
    }

    #[test]
    fn merges_vendor_and_os() {
        let nextcloud = resolve_presets("webdav", Some("nextcloud"), "linux");
        assert_eq!(nextcloud.remote["nextcloud_chunk_size"], "64M");
        let owncloud = resolve_presets("webdav", Some("owncloud"), "linux");
        assert_eq!(owncloud.remote["nextcloud_chunk_size"], "64M");
        let win = resolve_presets("sftp", None, "windows");
        assert_eq!(win.mount["network_mode"], true);
        let mac = resolve_presets("sftp", None, "darwin");
        assert_eq!(mac.mount["no_apple_xattr"], true);
        assert_eq!(mac.mount["no_apple_double"], true);
        let wsl = resolve_presets("sftp", None, "linux");
        assert!(wsl.mount.get("network_mode").is_none());
    }

    #[test]
    fn writes_helpers_onto_meta() {
        let mut meta = RemoteMeta::default();
        apply_to_remote_meta(&mut meta, &resolve_presets("s3", None, "linux"));
        assert_eq!(
            meta.helper_profile("vfs", "default").unwrap()["vfs_cache_mode"],
            "full"
        );
        assert_eq!(
            meta.helper_profile("backend", "default").unwrap()["disable_http2"],
            true
        );
        let mount = meta.get_profile(OperationType::Mount, "default").unwrap();
        assert_eq!(mount.rclone["attr_timeout"], "10s");
    }

    #[test]
    fn template_value_omits_empty_categories() {
        let linux = resolve_presets("", None, "linux").to_template_value();
        assert_eq!(linux["vfs"]["vfs_cache_mode"], "full");
        assert_eq!(linux["backend"]["log_level"], "INFO");
        assert!(linux.get("remote").is_none());
        let empty_type = default_template_presets("linux");
        assert_eq!(empty_type["mount"]["attr_timeout"], "10s");
        assert!(empty_type.get("remote").is_none());
    }

    #[test]
    fn fills_missing_remote_params_only() {
        let presets = resolve_presets("s3", None, "linux");
        let mut params = json!({ "chunk_size": "8M" });
        merge_remote_params(&mut params, &presets);
        assert_eq!(params["chunk_size"], "8M");
        assert_eq!(params["disable_checksum"], true);
    }
}
