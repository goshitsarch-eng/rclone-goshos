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

pub fn infer_path_kind(raw: &str, current_remote: &str) -> PathKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return PathKind::CurrentRemote;
    }
    if trimmed.starts_with('/') || trimmed.starts_with('~') || trimmed == "local" {
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

pub fn rewrite_path_for_kind(raw: &str, current_remote: &str, kind: PathKind) -> String {
    let typed = parse_typed_path(raw, current_remote);
    match kind {
        PathKind::Local => {
            if typed.kind == PathKind::Local && !typed.path.is_empty() {
                typed.path
            } else if typed.path.starts_with('/') || typed.path.starts_with('~') {
                expand_user(&typed.path)
            } else {
                expand_user("~")
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
    }

    #[test]
    fn resolves_relative_and_foreign_paths() {
        assert_eq!(resolve_job_path("Photos", "drive"), "drive:Photos");
        assert_eq!(resolve_job_path("drive:Photos", "drive"), "drive:Photos");
        assert_eq!(resolve_job_path("photos:Inbox", "drive"), "photos:Inbox");
        assert_eq!(resolve_job_path("/tmp/out", "drive"), "/tmp/out");
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
    }
}
