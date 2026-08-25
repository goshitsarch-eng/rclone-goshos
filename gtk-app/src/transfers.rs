//! Job-detail actions for individual rclone transfers.

use crate::rclone::{browse_target, split_remote_path};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRow {
    pub name: String,
    pub src: String,
    pub dst: String,
    pub percentage: i64,
    pub size: i64,
}

pub fn parse_transfer_row(item: &Value) -> TransferRow {
    let name = item
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("transfer")
        .to_string();
    let src = first_str(item, &["srcFs", "src", "group"])
        .map(|s| join_fs_name(&s, &name))
        .unwrap_or_else(|| name.clone());
    let dst = first_str(item, &["dstFs", "dst"])
        .map(|s| join_fs_name(&s, &name))
        .unwrap_or_default();
    let percentage = item
        .get("percentage")
        .or_else(|| item.get("percentageComplete"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let size = item.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
    TransferRow {
        name,
        src,
        dst,
        percentage,
        size,
    }
}

fn first_str(item: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(|x| x.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn join_fs_name(fs: &str, name: &str) -> String {
    if name.is_empty() {
        return fs.to_string();
    }
    if fs.is_empty() || fs == "/" {
        return name.to_string();
    }
    if name.contains(':') || name.starts_with('/') {
        return name.to_string();
    }
    if fs.ends_with(':') || fs.ends_with('/') {
        format!("{fs}{name}")
    } else {
        format!("{fs}/{name}")
    }
}

pub fn can_delete_source(job_type: &str) -> bool {
    !matches!(
        job_type.to_ascii_lowercase().as_str(),
        "delete" | "purge" | "rmdirs" | "cleanup" | "cryptcheck"
    )
}

pub fn can_delete_dest(job_type: &str, completed: bool) -> bool {
    completed
        && !matches!(
            job_type.to_ascii_lowercase().as_str(),
            "delete" | "purge" | "rmdirs" | "cleanup" | "check" | "cryptcheck"
        )
}

pub fn browse_for(path: &str) -> Option<(String, String)> {
    if path.is_empty() {
        return None;
    }
    browse_target(path).or_else(|| {
        let (remote, rest) = split_remote_path(path);
        Some((remote, rest))
    })
}

pub fn fs_and_remote(path: &str) -> (String, String) {
    let (remote, rest) = split_remote_path(path);
    if remote == "local" {
        ("/".into(), rest.trim_start_matches('/').to_string())
    } else {
        (crate::rclone::remote_fs(&remote, ""), rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_transfer_and_joins_fs() {
        let row = parse_transfer_row(&json!({
            "name": "Photos/a.jpg",
            "srcFs": "drive:",
            "dstFs": "/tmp/out",
            "percentage": 40,
            "size": 1024
        }));
        assert_eq!(row.src, "drive:Photos/a.jpg");
        assert_eq!(row.dst, "/tmp/out/Photos/a.jpg");
        assert_eq!(row.percentage, 40);
        assert_eq!(
            browse_for(&row.src).map(|(r, _)| r).as_deref(),
            Some("drive")
        );
        let (fs, remote) = fs_and_remote(&row.src);
        assert_eq!(fs, "drive:");
        assert_eq!(remote, "Photos/a.jpg");
    }

    #[test]
    fn delete_capabilities_match_job_type() {
        assert!(can_delete_source("sync"));
        assert!(!can_delete_source("delete"));
        assert!(can_delete_dest("copy", true));
        assert!(!can_delete_dest("copy", false));
        assert!(!can_delete_dest("check", true));
    }
}
