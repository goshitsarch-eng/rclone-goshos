//! Operation path kinds — local / current remote / other remote.

use crate::rclone::{remote_fs, split_remote_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Local,
    CurrentRemote,
    OtherRemote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedPath {
    pub kind: PathKind,
    pub remote: String,
    pub path: String,
}

pub fn expand_user(path: &str) -> String {
    expand_user_for_os(path, std::env::consts::OS)
}

pub fn is_windows_os(os: &str) -> bool {
    os.eq_ignore_ascii_case("windows")
}

pub fn default_local_root(os: &str) -> String {
    if is_windows_os(os) {
        "C:\\".into()
    } else {
        "/".into()
    }
}

/// Angular `getFullDisplayPath` / `pathStyleForRemote`.
/// Local paths follow the engine OS (`\` on Windows, `/` elsewhere); remotes stay posix.
pub fn format_location(remote: &str, path: &str, os: &str) -> String {
    let local = remote == "local" || remote.is_empty();
    if local {
        if path.is_empty() {
            default_local_root(os)
        } else {
            normalize_for_os(path, os)
        }
    } else if path.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{path}")
    }
}

/// Angular `normalizeForPlatform`.
pub fn normalize_for_os(path: &str, os: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    if is_windows_os(os) {
        let mut out = path.replace('/', "\\");
        while out.contains("\\\\") && !out.starts_with("\\\\") {
            out = out.replace("\\\\", "\\");
        }
        out
    } else {
        let mut out = path.replace('\\', "/");
        while out.contains("//") {
            out = out.replace("//", "/");
        }
        out
    }
}

/// Angular `isTrulyLocalPath` for the active engine OS.
pub fn is_truly_local_path(path: &str, os: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_windows_os(os) {
        return is_windows_local_path(trimmed) || is_unc_path(trimmed);
    }
    trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed == "local"
        || trimmed.starts_with("local:")
}

/// Angular `splitLocalForStat`.
pub fn split_local_for_stat(path: &str, os: &str) -> (String, String) {
    if is_windows_os(os) {
        let bytes = path.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            let root = format!("{}:/", bytes[0] as char);
            let remainder = path[2..].replace('\\', "/");
            let relative = remainder.trim_start_matches('/').to_string();
            return (root, relative);
        }
        return ("C:/".into(), path.replace('\\', "/"));
    }
    if path.starts_with('/') {
        ("/".into(), path.trim_start_matches('/').to_string())
    } else {
        (path.to_string(), String::new())
    }
}

pub fn expand_user_for_os(path: &str, os: &str) -> String {
    if is_windows_os(os) {
        if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
            return default_local_root(os);
        }
        return normalize_for_os(path, os);
    }
    if path == "~" {
        return dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|p| p.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
    }
    path.to_string()
}

pub fn is_windows_local_path(path: &str) -> bool {
    let bytes = path.trim().as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return false;
    }
    bytes.len() == 2 || bytes[2] == b'\\' || bytes[2] == b'/'
}

pub fn is_unc_path(path: &str) -> bool {
    let trimmed = path.trim();
    trimmed.starts_with("\\\\") || trimmed.starts_with("//")
}

pub fn infer_path_kind(raw: &str, current_remote: &str) -> PathKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathKind::CurrentRemote;
    }
    if trimmed.starts_with('/')
        || trimmed.starts_with('~')
        || trimmed == "local"
        || is_windows_local_path(trimmed)
        || is_unc_path(trimmed)
    {
        return PathKind::Local;
    }
    if !trimmed.contains(':') {
        return PathKind::CurrentRemote;
    }
    let (remote, _) = split_remote_path(trimmed);
    if remote == "local" {
        PathKind::Local
    } else if remote == current_remote {
        PathKind::CurrentRemote
    } else {
        PathKind::OtherRemote
    }
}

pub fn parse_typed_path(raw: &str, current_remote: &str) -> TypedPath {
    let trimmed = raw.trim();
    let kind = infer_path_kind(trimmed, current_remote);
    match kind {
        PathKind::Local => {
            let path = if let Some(rest) = trimmed.strip_prefix("local:") {
                rest.to_string()
            } else {
                expand_user(trimmed)
            };
            TypedPath {
                kind,
                remote: "local".into(),
                path,
            }
        }
        PathKind::CurrentRemote => {
            if trimmed.contains(':') {
                let (remote, path) = split_remote_path(trimmed);
                TypedPath { kind, remote, path }
            } else {
                TypedPath {
                    kind,
                    remote: current_remote.to_string(),
                    path: trimmed.to_string(),
                }
            }
        }
        PathKind::OtherRemote => {
            let (remote, path) = split_remote_path(trimmed);
            TypedPath { kind, remote, path }
        }
    }
}

pub fn resolve_job_path(raw: &str, current_remote: &str) -> String {
    let typed = parse_typed_path(raw, current_remote);
    match typed.kind {
        PathKind::Local => {
            if typed.path.is_empty() {
                "/".into()
            } else {
                typed.path
            }
        }
        PathKind::CurrentRemote | PathKind::OtherRemote => {
            if typed.remote.is_empty() || typed.remote == "local" {
                typed.path
            } else {
                remote_fs(&typed.remote, &typed.path)
            }
        }
    }
}

pub fn kind_index(kind: PathKind) -> u32 {
    match kind {
        PathKind::Local => 0,
        PathKind::CurrentRemote => 1,
        PathKind::OtherRemote => 2,
    }
}

pub fn kind_from_index(idx: u32) -> PathKind {
    match idx {
        0 => PathKind::Local,
        2 => PathKind::OtherRemote,
        _ => PathKind::CurrentRemote,
    }
}

pub fn breadcrumb_targets(remote: &str, path: &str) -> Vec<(String, String)> {
    if (remote == "local" || remote.is_empty()) && is_unc_path(path) {
        return unc_breadcrumb_targets(path);
    }
    if (remote == "local" || remote.is_empty()) && is_windows_local_path(path) {
        return windows_breadcrumb_targets(path);
    }
    let mut crumbs = Vec::new();
    let root_target = if remote == "local" || remote.is_empty() {
        "/".into()
    } else {
        format!("{remote}:")
    };
    crumbs.push((
        if remote == "local" || remote.is_empty() {
            "Local".into()
        } else {
            remote.to_string()
        },
        root_target,
    ));
    let mut acc = String::new();
    for segment in path
        .trim_start_matches('/')
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
    {
        acc = if acc.is_empty() {
            segment.to_string()
        } else {
            format!("{acc}/{segment}")
        };
        let target = if remote == "local" || remote.is_empty() {
            format!("/{acc}")
        } else {
            format!("{remote}:{acc}")
        };
        crumbs.push((segment.to_string(), target));
    }
    crumbs
}

fn windows_breadcrumb_targets(path: &str) -> Vec<(String, String)> {
    let drive = path[..2].to_string();
    let mut crumbs = vec![(drive.clone(), format!("{drive}\\"))];
    let rest = path[2..].trim_start_matches(['/', '\\']);
    let mut acc = format!("{drive}\\");
    for segment in rest.split(['/', '\\']).filter(|s| !s.is_empty()) {
        acc = if acc.ends_with('\\') {
            format!("{acc}{segment}")
        } else {
            format!("{acc}\\{segment}")
        };
        crumbs.push((segment.to_string(), acc.clone()));
    }
    crumbs
}

fn unc_breadcrumb_targets(path: &str) -> Vec<(String, String)> {
    let normalized = path.replace('/', "\\");
    let parts: Vec<&str> = normalized.split('\\').filter(|s| !s.is_empty()).collect();
    let mut crumbs = Vec::new();
    if parts.len() >= 2 {
        let root = format!("\\\\{}\\{}", parts[0], parts[1]);
        crumbs.push((root.clone(), root.clone()));
        let mut acc = root;
        for segment in &parts[2..] {
            acc = format!("{acc}\\{segment}");
            crumbs.push(((*segment).to_string(), acc.clone()));
        }
    } else {
        crumbs.push((normalized.clone(), normalized));
    }
    crumbs
}

pub fn rewrite_path_for_kind(raw: &str, current_remote: &str, kind: PathKind) -> String {
    rewrite_path_for_kind_os(raw, current_remote, kind, std::env::consts::OS)
}

pub fn rewrite_path_for_kind_os(
    raw: &str,
    current_remote: &str,
    kind: PathKind,
    os: &str,
) -> String {
    let typed = parse_typed_path(raw, current_remote);
    match kind {
        PathKind::Local => {
            if typed.kind == PathKind::Local && !typed.path.is_empty() {
                normalize_for_os(&typed.path, os)
            } else if is_truly_local_path(&typed.path, os)
                || typed.path.starts_with('/')
                || typed.path.starts_with('~')
            {
                expand_user_for_os(&typed.path, os)
            } else {
                default_local_root(os)
            }
        }
        PathKind::CurrentRemote => typed.path,
        PathKind::OtherRemote => {
            if typed.kind == PathKind::OtherRemote && !typed.remote.is_empty() {
                remote_fs(&typed.remote, &typed.path)
            } else {
                String::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_kinds() {
        assert_eq!(infer_path_kind("/tmp/out", "drive"), PathKind::Local);
        assert_eq!(infer_path_kind("~/mnt", "drive"), PathKind::Local);
        assert_eq!(infer_path_kind("Photos", "drive"), PathKind::CurrentRemote);
        assert_eq!(
            infer_path_kind("drive:Photos", "drive"),
            PathKind::CurrentRemote
        );
        assert_eq!(
            infer_path_kind("photos:Inbox", "drive"),
            PathKind::OtherRemote
        );
        assert_eq!(infer_path_kind(r"C:\Users\me", "drive"), PathKind::Local);
        assert_eq!(infer_path_kind("D:/data", "drive"), PathKind::Local);
        assert_eq!(infer_path_kind("E:", "drive"), PathKind::Local);
        assert_eq!(infer_path_kind(r"\\nas\share", "drive"), PathKind::Local);
        assert_eq!(infer_path_kind("C:Photos", "drive"), PathKind::OtherRemote);
        assert!(is_windows_local_path("c:/tmp"));
        assert!(!is_windows_local_path("drive:Photos"));
    }

    #[test]
    fn resolves_relative_and_foreign_paths() {
        assert_eq!(resolve_job_path("Photos", "drive"), "drive:Photos");
        assert_eq!(resolve_job_path("drive:Photos", "drive"), "drive:Photos");
        assert_eq!(resolve_job_path("photos:Inbox", "drive"), "photos:Inbox");
        assert_eq!(resolve_job_path("/tmp/out", "drive"), "/tmp/out");
        assert_eq!(resolve_job_path(r"C:\out", "drive"), r"C:\out");
        assert!(resolve_job_path("~", "drive").starts_with('/'));
    }

    #[test]
    fn parses_empty_as_current_root() {
        let typed = parse_typed_path("", "drive");
        assert_eq!(typed.kind, PathKind::CurrentRemote);
        assert_eq!(typed.remote, "drive");
        assert_eq!(resolve_job_path("", "drive"), "drive:");
    }

    #[test]
    fn rewrites_path_when_kind_changes() {
        assert!(rewrite_path_for_kind("Photos", "drive", PathKind::Local).starts_with('/'));
        assert_eq!(
            rewrite_path_for_kind("/tmp/out", "drive", PathKind::CurrentRemote),
            "/tmp/out"
        );
        assert_eq!(
            rewrite_path_for_kind("photos:Inbox", "drive", PathKind::OtherRemote),
            "photos:Inbox"
        );
        assert_eq!(kind_index(PathKind::OtherRemote), 2);
        assert_eq!(kind_from_index(0), PathKind::Local);
    }

    #[test]
    fn builds_breadcrumb_targets() {
        assert_eq!(
            breadcrumb_targets("local", "/home/ada/docs"),
            vec![
                ("Local".into(), "/".into()),
                ("home".into(), "/home".into()),
                ("ada".into(), "/home/ada".into()),
                ("docs".into(), "/home/ada/docs".into()),
            ]
        );
        assert_eq!(
            breadcrumb_targets("drive", "Photos/2024"),
            vec![
                ("drive".into(), "drive:".into()),
                ("Photos".into(), "drive:Photos".into()),
                ("2024".into(), "drive:Photos/2024".into()),
            ]
        );
        assert_eq!(
            breadcrumb_targets("local", r"C:\Users\me"),
            vec![
                ("C:".into(), r"C:\".into()),
                ("Users".into(), r"C:\Users".into()),
                ("me".into(), r"C:\Users\me".into()),
            ]
        );
        assert_eq!(
            breadcrumb_targets("local", r"\\nas\share\docs"),
            vec![
                (r"\\nas\share".into(), r"\\nas\share".into()),
                ("docs".into(), r"\\nas\share\docs".into()),
            ]
        );
    }

    #[test]
    fn engine_os_path_helpers_match_angular() {
        assert_eq!(normalize_for_os(r"C:\Users\me", "linux"), "C:/Users/me");
        assert_eq!(normalize_for_os("C:/Users/me", "windows"), r"C:\Users\me");
        assert!(is_truly_local_path(r"C:\Users", "windows"));
        assert!(!is_truly_local_path("drive:Photos", "windows"));
        assert!(is_truly_local_path("/tmp/out", "linux"));
        assert!(!is_truly_local_path("C:Photos", "linux"));
        assert_eq!(
            split_local_for_stat(r"C:\Users\me\a.txt", "windows"),
            ("C:/".into(), "Users/me/a.txt".into())
        );
        assert_eq!(
            split_local_for_stat("/home/ada/docs", "linux"),
            ("/".into(), "home/ada/docs".into())
        );
        assert_eq!(default_local_root("windows"), r"C:\");
        assert_eq!(
            rewrite_path_for_kind_os("Photos", "drive", PathKind::Local, "windows"),
            r"C:\"
        );
        assert_eq!(expand_user_for_os("~", "windows"), r"C:\");
        assert_eq!(format_location("local", "", "linux"), "/");
        assert_eq!(format_location("local", "", "windows"), r"C:\");
        assert_eq!(
            format_location("local", r"C:/Users/me/file.txt", "windows"),
            r"C:\Users\me\file.txt"
        );
        assert_eq!(
            format_location("local", r"C:\Users\me\file.txt", "linux"),
            "C:/Users/me/file.txt"
        );
        assert_eq!(
            format_location("drive", "Photos/2024", "windows"),
            "drive:Photos/2024"
        );
        assert_eq!(format_location("drive", "", "linux"), "drive:");
        assert_eq!(format_location("", "/tmp/out", "linux"), "/tmp/out");
    }
}
