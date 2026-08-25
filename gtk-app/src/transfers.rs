//! Job-detail actions for individual rclone transfers.

use crate::rclone::{browse_target, split_remote_path};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TransferRow {
    pub name: String,
    pub src: String,
    pub dst: String,
    pub percentage: i64,
    pub size: i64,
    pub bytes: i64,
    pub speed: f64,
    pub eta: f64,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Preparing,
    Progress,
    Finalizing,
    Completed,
    Error,
}

pub fn parse_transfer_row(item: &Value) -> TransferRow {
    parse_transfer_row_with(item, false)
}

pub fn parse_completed_transfer_row(item: &Value) -> TransferRow {
    parse_transfer_row_with(item, true)
}

fn parse_transfer_row_with(item: &Value, completed: bool) -> TransferRow {
    let name = item
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("transfer")
        .to_string();
    let src = resolve_transfer_path(item, &["srcFs"], &["srcRemote"], &["src"], &name);
    let dst = resolve_transfer_path(item, &["dstFs"], &["dstRemote"], &["dst"], &name);
    let percentage = item
        .get("percentage")
        .or_else(|| item.get("percentageComplete"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let size = item.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
    let bytes = item
        .get("bytes")
        .or_else(|| item.get("bytesSoFar"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let speed = item.get("speed").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let eta = item.get("eta").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let error = item
        .get("error")
        .or_else(|| item.get("lastError"))
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    let mut row = TransferRow {
        name,
        src,
        dst,
        percentage,
        size,
        bytes,
        speed,
        eta,
        error,
    };
    if completed {
        finalize_completed_row(&mut row);
    }
    row
}

pub fn finalize_completed_row(row: &mut TransferRow) {
    if row.size > 0 && row.bytes == 0 {
        row.bytes = row.size;
        row.percentage = 100;
    } else if row.size > 0 && row.bytes >= row.size && row.percentage < 100 {
        row.percentage = 100;
    }
}

pub fn transfer_status(completed: bool, row: &TransferRow) -> TransferStatus {
    if !row.error.is_empty() {
        return TransferStatus::Error;
    }
    if completed {
        return TransferStatus::Completed;
    }
    if row.percentage <= 0 && row.bytes <= 0 {
        return TransferStatus::Preparing;
    }
    if row.percentage >= 100 {
        return TransferStatus::Finalizing;
    }
    TransferStatus::Progress
}

pub fn transfer_meta_caption(row: &TransferRow) -> String {
    let size = if row.size > 0 {
        format!(
            "{} / {}",
            crate::rclone::format_bytes(row.bytes.max(0)),
            crate::rclone::format_bytes(row.size)
        )
    } else if row.bytes > 0 {
        crate::rclone::format_bytes(row.bytes)
    } else {
        String::new()
    };
    let mut parts = Vec::new();
    if !size.is_empty() {
        parts.push(size);
    }
    if row.speed > 0.0 {
        parts.push(format!(
            "{}/s",
            crate::rclone::format_bytes(row.speed.round() as i64)
        ));
    }
    if row.eta > 0.0 {
        parts.push(crate::jobs::format_seconds(row.eta));
    }
    parts.join(" · ")
}

fn first_str(item: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| item.get(*key).and_then(|x| x.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Resolve a transfer side from rclone (`srcFs`+`srcRemote`) or a snapshot
/// that already stored a complete `src`/`dst` path. Never treats `group` as a
/// filesystem — that produced captions like `e.txt/e.txt`.
fn resolve_transfer_path(
    item: &Value,
    fs_keys: &[&str],
    remote_keys: &[&str],
    path_keys: &[&str],
    name: &str,
) -> String {
    let remote = first_str(item, remote_keys);
    if let Some(fs) = first_str(item, fs_keys) {
        return join_fs_name(&fs, remote.as_deref().unwrap_or(name));
    }
    if let Some(path) = first_str(item, path_keys) {
        if path_already_complete(&path, name) {
            return path;
        }
        return join_fs_name(&path, name);
    }
    name.to_string()
}

pub fn path_already_complete(path: &str, name: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if !name.is_empty()
        && (path == name
            || path.ends_with(&format!("/{name}"))
            || path.ends_with(&format!("\\{name}")))
    {
        return true;
    }
    path.contains('/')
        || path.contains('\\')
        || crate::path_kind::is_windows_local_path(path)
        || crate::path_kind::is_unc_path(path)
}

pub fn join_fs_name(fs: &str, name: &str) -> String {
    if name.is_empty() {
        return fs.to_string();
    }
    if fs.is_empty() || fs == "/" {
        return name.to_string();
    }
    if name.contains(':')
        || name.starts_with('/')
        || crate::path_kind::is_windows_local_path(name)
        || crate::path_kind::is_unc_path(name)
    {
        return name.to_string();
    }
    if fs.ends_with(':') || fs.ends_with('/') || fs.ends_with('\\') {
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
    fn snapshot_rows_keep_complete_paths() {
        let row = parse_transfer_row(&json!({
            "name": "e.txt",
            "src": "/tmp/gtk-upload-test/e.txt",
            "dst": "e.txt",
            "size": 4
        }));
        assert_eq!(row.src, "/tmp/gtk-upload-test/e.txt");
        assert_eq!(row.dst, "e.txt");
        let grouped = parse_transfer_row(&json!({
            "name": "e.txt",
            "srcFs": "/",
            "srcRemote": "/tmp/gtk-upload-test/e.txt",
            "dstFs": "testdrive:",
            "dstRemote": "e.txt",
            "src": "/tmp/gtk-upload-test/e.txt",
            "dst": "testdrive:e.txt"
        }));
        assert_eq!(grouped.src, "/tmp/gtk-upload-test/e.txt");
        assert_eq!(grouped.dst, "testdrive:e.txt");
        let windows = parse_transfer_row(&json!({
            "name": "a.jpg",
            "src": r"C:\Users\me\a.jpg",
            "dst": r"D:\out\a.jpg"
        }));
        assert_eq!(windows.src, r"C:\Users\me\a.jpg");
        assert_eq!(windows.dst, r"D:\out\a.jpg");
        assert!(!path_already_complete("drive", "a.jpg"));
        assert!(path_already_complete("/tmp/e.txt", "e.txt"));
    }

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
        assert_eq!(row.bytes, 0);
        assert!(row.error.is_empty());
        assert_eq!(transfer_status(false, &row), TransferStatus::Progress);
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

    #[test]
    fn transfer_status_and_caption_match_angular() {
        let preparing = TransferRow {
            name: "a.bin".into(),
            ..TransferRow::default()
        };
        assert_eq!(
            transfer_status(false, &preparing),
            TransferStatus::Preparing
        );
        let done = parse_transfer_row(&json!({
            "name": "b.bin",
            "percentage": 100,
            "bytes": 1024,
            "size": 1024,
            "speed": 512.0,
            "eta": 2
        }));
        assert_eq!(transfer_status(false, &done), TransferStatus::Finalizing);
        assert_eq!(transfer_status(true, &done), TransferStatus::Completed);
        let caption = transfer_meta_caption(&done);
        assert!(caption.contains("KiB"));
        let failed = parse_transfer_row(&json!({
            "name": "c.bin",
            "error": "denied",
            "percentage": 10
        }));
        assert_eq!(transfer_status(false, &failed), TransferStatus::Error);
        let reconstructed = parse_completed_transfer_row(&json!({
            "name": "k.txt",
            "src": "/tmp/k.txt",
            "dst": "testdrive:k.txt",
            "size": 3,
            "bytes": 0,
            "percentage": 0
        }));
        assert_eq!(reconstructed.bytes, 3);
        assert_eq!(reconstructed.percentage, 100);
        assert_eq!(
            transfer_meta_caption(&reconstructed),
            format!(
                "{} / {}",
                crate::rclone::format_bytes(3),
                crate::rclone::format_bytes(3)
            )
        );
        let active = parse_transfer_row(&json!({
            "name": "k.txt",
            "size": 3,
            "bytes": 0
        }));
        assert_eq!(active.bytes, 0);
        assert_eq!(active.percentage, 0);
    }
}
