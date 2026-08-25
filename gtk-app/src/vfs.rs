//! Parse rclone `vfs/stats` and `vfs/queue` payloads.

use serde_json::Value;

/// Angular `vfs-control-panel` priority expiry (`PRIORITY_EXPIRY`).
pub const PRIORITY_EXPIRY: i64 = -999_999_999;
/// Angular `vfs-control-panel` long delay expiry (`DELAY_EXPIRY`).
pub const DELAY_EXPIRY: i64 = 999_999_999;
pub const DELAY_SLIDER_DEFAULT: f64 = 60.0;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VfsStats {
    pub metadata_dirs: i64,
    pub metadata_files: i64,
    pub disk_path: String,
    pub disk_path_meta: String,
    pub disk_bytes: i64,
    pub disk_files: i64,
    pub uploads_in_progress: i64,
    pub uploads_queued: i64,
    pub errored: i64,
    pub in_use: i64,
    pub opt: Value,
}

impl VfsStats {
    pub fn summary(&self) -> String {
        format!(
            "metadata {} dirs / {} files · uploads {} · errors {}",
            self.metadata_dirs, self.metadata_files, self.uploads_in_progress, self.errored
        )
    }

    pub fn metadata_items(&self) -> i64 {
        self.metadata_dirs + self.metadata_files
    }

    pub fn disk_cache_enabled(&self) -> bool {
        !self.disk_path.is_empty() || self.disk_bytes > 0 || self.disk_files > 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VfsQueueItem {
    pub id: String,
    pub name: String,
    pub size: i64,
    pub expiry: String,
    pub expiry_secs: f64,
    pub uploading: bool,
    pub tries: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    Uploading,
    Delayed,
    Ready,
    Waiting,
}

pub fn parse_vfs_stats(value: &Value) -> VfsStats {
    let meta = value.get("metadataCache").or_else(|| value.get("metadata"));
    let disk = value.get("diskCache");
    VfsStats {
        metadata_dirs: int_field(meta, &["dirs", "Directories"]),
        metadata_files: int_field(meta, &["files", "Files"]),
        disk_path: string_field(disk, &["path"]),
        disk_path_meta: string_field(disk, &["pathMeta"]),
        disk_bytes: int_field(disk, &["bytesUsed", "bytes"]),
        disk_files: int_field(disk, &["files", "Files"]),
        uploads_in_progress: int_field(
            value.get("uploadsInProgress").or(disk),
            &["uploadsInProgress", "uploads"],
        )
        .max(int_field(disk, &["uploadsInProgress"])),
        uploads_queued: int_field(disk, &["uploadsQueued", "queued"]),
        errored: int_field(disk, &["erroredFiles", "errored", "errors"]),
        in_use: int_field(Some(value), &["inUse", "in_use"]),
        opt: value.get("opt").cloned().unwrap_or(Value::Null),
    }
}

pub fn parse_vfs_list(value: &Value) -> Vec<String> {
    value
        .get("vfses")
        .or_else(|| value.get("fs"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn filter_vfs_names(names: &[String], remote: &str) -> Vec<String> {
    names
        .iter()
        .filter(|name| *name == remote || name.starts_with(&format!("{remote}:")))
        .cloned()
        .collect()
}

pub fn is_indexed_vfs(name: &str) -> bool {
    name.rsplit_once(':')
        .map(|(_, suffix)| {
            let inner = suffix
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .unwrap_or("");
            !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit())
        })
        .unwrap_or(false)
}

pub fn queue_item_status(item: &VfsQueueItem) -> QueueStatus {
    if item.uploading {
        QueueStatus::Uploading
    } else if item.expiry_secs >= DELAY_EXPIRY as f64 - 1000.0 {
        QueueStatus::Delayed
    } else if item.expiry_secs <= PRIORITY_EXPIRY as f64 + 1000.0 || item.expiry_secs < 0.0 {
        QueueStatus::Ready
    } else {
        QueueStatus::Waiting
    }
}

pub fn parse_vfs_queue(value: &Value) -> Vec<VfsQueueItem> {
    let items = value
        .get("queue")
        .or_else(|| value.get("items"))
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default();
    items.iter().filter_map(parse_queue_item).collect()
}

fn parse_queue_item(value: &Value) -> Option<VfsQueueItem> {
    let id = value
        .get("id")
        .map(|v| match v {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())?;
    Some(VfsQueueItem {
        id,
        name: value
            .get("name")
            .or_else(|| value.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        size: value.get("size").and_then(|v| v.as_i64()).unwrap_or(0),
        expiry: value
            .get("expiry")
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        expiry_secs: number_field(value, "expiry"),
        uploading: value
            .get("uploading")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        tries: value.get("tries").and_then(|v| v.as_i64()).unwrap_or(0),
    })
}

fn int_field(obj: Option<&Value>, keys: &[&str]) -> i64 {
    let Some(obj) = obj else {
        return 0;
    };
    if let Some(n) = obj.as_i64() {
        return n;
    }
    for key in keys {
        if obj.get(*key).is_some() {
            return number_field(obj, key) as i64;
        }
    }
    0
}

fn string_field(obj: Option<&Value>, keys: &[&str]) -> String {
    let Some(obj) = obj else {
        return String::new();
    };
    for key in keys {
        if let Some(s) = obj.get(*key).and_then(|v| v.as_str()) {
            return s.to_string();
        }
    }
    String::new()
}

fn number_field(obj: &Value, key: &str) -> f64 {
    match obj.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

pub fn parse_expiry_pair(text: &str) -> (String, String) {
    let mut id = String::new();
    let mut expiry = String::new();
    for part in text.split_whitespace() {
        if let Some(v) = part.strip_prefix("id=") {
            id = v.to_string();
        } else if let Some(v) = part.strip_prefix("expiry=") {
            expiry = v.to_string();
        }
    }
    if id.is_empty() {
        let mut parts = text.split_whitespace();
        id = parts.next().unwrap_or("").to_string();
        expiry = parts.next().unwrap_or("").to_string();
    }
    (id, expiry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_stats_grid() {
        let stats = parse_vfs_stats(&json!({
            "metadataCache": { "dirs": 4, "files": 12 },
            "diskCache": { "path": "/tmp/vfs", "errored": 1, "uploadsInProgress": 2 }
        }));
        assert_eq!(stats.metadata_dirs, 4);
        assert_eq!(stats.metadata_files, 12);
        assert_eq!(stats.disk_path, "/tmp/vfs");
        assert_eq!(stats.errored, 1);
        assert_eq!(stats.uploads_in_progress, 2);
        assert!(stats.summary().contains("12 files"));
    }

    #[test]
    fn parses_queue_items() {
        let items = parse_vfs_queue(&json!({
            "queue": [
                { "id": 7, "name": "a.bin", "size": 32, "expiry": 60 },
                { "id": "8", "path": "b.txt" }
            ]
        }));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "7");
        assert_eq!(items[0].name, "a.bin");
        assert_eq!(items[1].id, "8");
        assert_eq!(items[1].name, "b.txt");
    }

    #[test]
    fn parses_expiry_text() {
        assert_eq!(
            parse_expiry_pair("id=3 expiry=1m"),
            ("3".into(), "1m".into())
        );
        assert_eq!(parse_expiry_pair("9 30s"), ("9".into(), "30s".into()));
    }

    #[test]
    fn parses_vfs_list_and_filters_remote() {
        let names = parse_vfs_list(&json!({
            "vfses": ["testdrive:", "testdrive:[0]", "other:", "testdrive"]
        }));
        assert_eq!(
            names,
            vec!["testdrive:", "testdrive:[0]", "other:", "testdrive"]
        );
        assert_eq!(
            filter_vfs_names(&names, "testdrive"),
            vec!["testdrive:", "testdrive:[0]", "testdrive"]
        );
        assert!(is_indexed_vfs("testdrive:[0]"));
        assert!(!is_indexed_vfs("testdrive:"));
    }

    #[test]
    fn classifies_queue_expiry_like_angular() {
        let uploading = VfsQueueItem {
            id: "1".into(),
            name: "a".into(),
            size: 1,
            expiry: "1".into(),
            expiry_secs: 1.0,
            uploading: true,
            tries: 0,
        };
        assert_eq!(queue_item_status(&uploading), QueueStatus::Uploading);
        let delayed = VfsQueueItem {
            uploading: false,
            expiry_secs: DELAY_EXPIRY as f64,
            ..uploading.clone()
        };
        assert_eq!(queue_item_status(&delayed), QueueStatus::Delayed);
        let ready = VfsQueueItem {
            expiry_secs: PRIORITY_EXPIRY as f64,
            ..delayed.clone()
        };
        assert_eq!(queue_item_status(&ready), QueueStatus::Ready);
        let waiting = VfsQueueItem {
            expiry_secs: 12.5,
            ..delayed
        };
        assert_eq!(queue_item_status(&waiting), QueueStatus::Waiting);
    }
}
