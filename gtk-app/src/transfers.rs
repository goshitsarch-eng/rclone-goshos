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

pub fn join_fs_name(fs: &str, name: &str) -> String {
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

pub fn download_target(path: &str) -> Option<(String, String, String)> {
    let (remote, rest) = browse_for(path)?;
    if rest.is_empty() || rest.ends_with('/') {
        return None;
    }
    let name = rest
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(&rest)
        .to_string();
    Some((remote, rest, name))
}

pub fn can_public_link(remote: &str, info: Option<&crate::rclone::FsInfo>) -> bool {
    remote != "local" && remote != "/" && info.is_none_or(|i| i.has_feature("PublicLink"))
}

pub fn is_move_job(job_type: &str) -> bool {
    job_type.to_ascii_lowercase().contains("move")
}

pub fn is_delete_job(job_type: &str) -> bool {
    let lower = job_type.to_ascii_lowercase();
    matches!(lower.as_str(), "delete" | "purge" | "rmdirs" | "cleanup")
        || lower.starts_with("delete/")
        || lower.starts_with("purge/")
        || lower.starts_with("rmdirs/")
        || lower.starts_with("cleanup/")
}

pub fn remote_name_from_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let (remote, _) = crate::rclone::split_remote_path(path);
    if remote.is_empty()
        || remote == "local"
        || remote == "/"
        || matches!(remote.as_str(), "http" | "https" | "ftp" | "sftp" | "ftps")
    {
        None
    } else {
        Some(remote)
    }
}

pub fn can_copy_url_source(
    src: &str,
    job_type: &str,
    info: Option<&crate::rclone::FsInfo>,
) -> bool {
    if is_move_job(job_type) || is_delete_job(job_type) {
        return false;
    }
    remote_name_from_path(src).is_some_and(|remote| can_public_link(&remote, info))
}

pub fn can_copy_url_dest(
    dst: &str,
    job_type: &str,
    completed: bool,
    info: Option<&crate::rclone::FsInfo>,
) -> bool {
    completed
        && !is_delete_job(job_type)
        && remote_name_from_path(dst).is_some_and(|remote| can_public_link(&remote, info))
}

pub fn can_download_source(src: &str, job_type: &str) -> bool {
    if is_move_job(job_type) || is_delete_job(job_type) {
        return false;
    }
    remote_name_from_path(src).is_some() && download_target(src).is_some()
}

pub fn can_download_dest(dst: &str, job_type: &str, completed: bool) -> bool {
    completed
        && !is_delete_job(job_type)
        && remote_name_from_path(dst).is_some()
        && download_target(dst).is_some()
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

    #[test]
    fn download_and_public_link_targets() {
        let target = download_target("drive:Photos/a.jpg").unwrap();
        assert_eq!(target.0, "drive");
        assert_eq!(target.1, "Photos/a.jpg");
        assert_eq!(target.2, "a.jpg");
        assert!(download_target("drive:").is_none());
        assert!(can_public_link("drive", None));
        assert!(!can_public_link("local", None));
    }

    #[test]
    fn src_dst_actions_match_angular_rules() {
        assert!(can_copy_url_source("drive:Photos/a.jpg", "copy", None));
        assert!(!can_copy_url_source("/tmp/a.jpg", "copy", None));
        assert!(!can_copy_url_source("drive:Photos/a.jpg", "move", None));
        assert!(!can_copy_url_dest("box:out/a.jpg", "copy", false, None));
        assert!(can_copy_url_dest("box:out/a.jpg", "copy", true, None));
        assert!(!can_copy_url_dest("box:out/a.jpg", "delete", true, None));
        assert!(can_download_source("drive:Photos/a.jpg", "sync"));
        assert!(!can_download_source("drive:Photos/a.jpg", "delete"));
        assert!(!can_download_dest("box:out/a.jpg", "copy", false));
        assert!(can_download_dest("box:out/a.jpg", "copy", true));
        assert_eq!(remote_name_from_path("drive:x").as_deref(), Some("drive"));
        assert!(remote_name_from_path("/tmp/x").is_none());
        assert!(is_move_job("copy/move"));
        assert!(is_delete_job("delete/purge"));
    }
}
