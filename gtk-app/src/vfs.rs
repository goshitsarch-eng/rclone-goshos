//! Parse rclone `vfs/stats` and `vfs/queue` payloads.

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VfsStats {
    pub metadata_dirs: i64,
    pub metadata_files: i64,
    pub disk_path: String,
    pub uploads_in_progress: i64,
    pub errored: i64,
}

impl VfsStats {
    pub fn summary(&self) -> String {
        format!(
            "metadata {} dirs / {} files · uploads {} · errors {}",
            self.metadata_dirs, self.metadata_files, self.uploads_in_progress, self.errored
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsQueueItem {
    pub id: String,
    pub name: String,
    pub size: i64,
    pub expiry: String,
}

pub fn parse_vfs_stats(value: &Value) -> VfsStats {
    let meta = value.get("metadataCache").or_else(|| value.get("metadata"));
    let disk = value.get("diskCache");
    VfsStats {
        metadata_dirs: int_field(meta, &["dirs", "Directories"]),
        metadata_files: int_field(meta, &["files", "Files"]),
        disk_path: disk
            .and_then(|v| v.get("path").or_else(|| v.get("pathMeta")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        uploads_in_progress: int_field(
            value.get("uploadsInProgress").or(disk),
            &["uploadsInProgress", "uploads"],
        )
        .max(int_field(disk, &["uploadsInProgress"])),
        errored: int_field(disk, &["errored", "errors"]),
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
        if let Some(n) = obj.get(*key).and_then(|v| v.as_i64()) {
            return n;
        }
    }
    0
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
}
