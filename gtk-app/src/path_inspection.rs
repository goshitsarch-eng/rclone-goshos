//! Mount / destination path inspection — mirrors Angular `PathInspectionService`.

use crate::operations::OperationType;
use crate::rclone::{MountedRemote, RcClient};
use crate::store::{AppStore, ProfileConfig};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStatus {
    Empty,
    WillCreate,
    NonEmpty,
    Collision { remote: String, profile: String },
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    pub remote: String,
    pub profile: String,
    pub path: String,
}

pub fn normalize_mount_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(rest)
    } else if trimmed == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
    } else {
        PathBuf::from(trimmed)
    };
    if let Ok(canon) = std::fs::canonicalize(&expanded) {
        return canon.to_string_lossy().into_owned();
    }
    expanded.to_string_lossy().into_owned()
}

pub fn inspect_local_path(path: &str) -> PathStatus {
    if path.trim().is_empty() {
        return PathStatus::Empty;
    }
    let expanded = PathBuf::from(normalize_mount_path(path));
    if expanded.exists() {
        if expanded.is_file() {
            return PathStatus::Invalid;
        }
        match std::fs::read_dir(&expanded) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    PathStatus::NonEmpty
                } else {
                    PathStatus::Empty
                }
            }
            Err(_) => PathStatus::Invalid,
        }
    } else {
        PathStatus::WillCreate
    }
}

pub fn find_mount_collision(
    store: &AppStore,
    path: &str,
    current_remote: &str,
    live_mounts: &[MountedRemote],
) -> Option<Collision> {
    let wanted = normalize_mount_path(path);
    if wanted.is_empty() {
        return None;
    }
    for mount in live_mounts {
        if normalize_mount_path(&mount.mount_point) == wanted {
            let remote = mount
                .fs
                .split_once(':')
                .map(|(n, _)| n.to_string())
                .unwrap_or_else(|| mount.fs.clone());
            if remote != current_remote {
                return Some(Collision {
                    remote,
                    profile: "live".into(),
                    path: mount.mount_point.clone(),
                });
            }
        }
    }
    for (remote, meta) in &store.remotes {
        if remote == current_remote {
            continue;
        }
        if let Some(profiles) = meta.profiles.get(OperationType::Mount.as_str()) {
            for (profile_name, profile) in profiles {
                if mount_point_of(profile)
                    .map(|p| normalize_mount_path(&p) == wanted)
                    .unwrap_or(false)
                {
                    return Some(Collision {
                        remote: remote.clone(),
                        profile: profile_name.clone(),
                        path: wanted,
                    });
                }
            }
        }
    }
    None
}

fn mount_point_of(profile: &ProfileConfig) -> Option<String> {
    for key in ["mountPoint", "dstFs", "dest"] {
        if let Some(s) = profile.rclone.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

pub fn inspect_dest(
    store: &AppStore,
    path: &str,
    current_remote: &str,
    op: OperationType,
    live_mounts: &[MountedRemote],
) -> PathStatus {
    inspect_dest_ex(store, path, current_remote, op, live_mounts, None)
}

pub fn inspect_dest_ex(
    store: &AppStore,
    path: &str,
    current_remote: &str,
    op: OperationType,
    live_mounts: &[MountedRemote],
    client: Option<&RcClient>,
) -> PathStatus {
    if path.trim().is_empty() {
        return PathStatus::Empty;
    }
    if op == OperationType::Mount {
        if let Some(hit) = find_mount_collision(store, path, current_remote, live_mounts) {
            return PathStatus::Collision {
                remote: hit.remote,
                profile: hit.profile,
            };
        }
        return inspect_local_path(path);
    }
    if Path::new(path).is_absolute() || path.starts_with("~/") {
        return inspect_local_path(path);
    }
    if let Some(client) = client {
        if let Some((fs, remote)) = crate::rclone::browse_target(path) {
            return inspect_remote_path(client, &crate::rclone::remote_fs(&fs, ""), &remote);
        }
        if path.contains(':') {
            let (name, rest) = path.split_once(':').unwrap();
            return inspect_remote_path(client, &crate::rclone::remote_fs(name, ""), rest);
        }
    }
    PathStatus::Empty
}

pub fn inspect_remote_path(client: &RcClient, fs: &str, remote: &str) -> PathStatus {
    match client.stat(fs, remote) {
        Ok(Some(item)) if item.is_dir => match client.size(fs, remote) {
            Ok(value) if value.get("count").and_then(|x| x.as_i64()).unwrap_or(0) > 0 => {
                PathStatus::NonEmpty
            }
            _ => PathStatus::Empty,
        },
        Ok(Some(_)) => PathStatus::Invalid,
        Ok(None) => PathStatus::WillCreate,
        Err(_) => PathStatus::Empty,
    }
}

/// Create a missing local (or remote) directory when inspection says it will be created.
pub fn ensure_path(
    status: &PathStatus,
    path: &str,
    client: Option<&RcClient>,
) -> Result<(), String> {
    if !matches!(status, PathStatus::WillCreate | PathStatus::Empty) {
        return Ok(());
    }
    if path.trim().is_empty() {
        return Ok(());
    }
    if Path::new(path).is_absolute() || path.starts_with("~/") || Path::new(path).exists() {
        let expanded = normalize_mount_path(path);
        if expanded.is_empty() {
            return Ok(());
        }
        return std::fs::create_dir_all(&expanded).map_err(|e| e.to_string());
    }
    if let Some(client) = client {
        if let Some((fs, remote)) = crate::rclone::browse_target(path) {
            if !remote.is_empty() {
                return client
                    .mkdir(&crate::rclone::remote_fs(&fs, ""), &remote)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
            }
        }
    }
    Ok(())
}

pub fn describe_status(status: &PathStatus) -> String {
    match status {
        PathStatus::Empty => "Folder is empty or unused.".into(),
        PathStatus::WillCreate => "Path does not exist yet and will be created.".into(),
        PathStatus::NonEmpty => "Folder already contains files.".into(),
        PathStatus::Collision { remote, profile } => {
            format!("Already used by {remote} ({profile}).")
        }
        PathStatus::Invalid => "Path is not a usable folder.".into(),
    }
}

pub fn suggest_default_mount_path(remote: &str, store: &AppStore) -> String {
    suggest_default_op_path(remote, OperationType::Mount, store, "")
}

/// Angular templates: `{home}/rclone-manager/{remote}` and `{remote}-bisync`.
pub fn suggest_default_op_path(
    remote: &str,
    op: OperationType,
    store: &AppStore,
    template: &str,
) -> String {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let suffix = if op == OperationType::Bisync {
        "-bisync"
    } else {
        ""
    };
    let base = if template.trim().is_empty() {
        home.join("rclone-manager")
            .join(format!("{remote}{suffix}"))
    } else {
        PathBuf::from(
            template
                .replace("{home}", &home.to_string_lossy())
                .replace("{remote}", remote)
                .replace("{suffix}", suffix),
        )
    };
    let mut candidate = base.to_string_lossy().into_owned();
    let mut i = 2;
    while find_mount_collision(store, &candidate, remote, &[]).is_some() {
        candidate = format!("{}-{i}", base.display());
        i += 1;
        if i > 50 {
            break;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ProfileConfig, RemoteMeta};
    use serde_json::json;

    fn store_with_mount(remote: &str, path: &str) -> AppStore {
        let mut store = AppStore::default();
        let mut meta = RemoteMeta::default();
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "default".into(),
                rclone: json!({ "mountPoint": path }),
                ..ProfileConfig::default()
            },
        );
        store.remotes.insert(remote.into(), meta);
        store
    }

    #[test]
    fn detects_profile_collision() {
        let store = store_with_mount("drive", "/mnt/drive");
        let hit = find_mount_collision(&store, "/mnt/drive", "photos", &[]).unwrap();
        assert_eq!(hit.remote, "drive");
        assert_eq!(hit.profile, "default");
        assert!(find_mount_collision(&store, "/mnt/drive", "drive", &[]).is_none());
        assert!(find_mount_collision(&store, "/mnt/other", "photos", &[]).is_none());
    }

    #[test]
    fn detects_live_mount_collision() {
        let store = AppStore::default();
        let live = [MountedRemote {
            fs: "photos:".into(),
            mount_point: "/mnt/photos".into(),
        }];
        let hit = find_mount_collision(&store, "/mnt/photos", "drive", &live).unwrap();
        assert_eq!(hit.remote, "photos");
        assert_eq!(hit.profile, "live");
    }

    #[test]
    fn inspects_missing_and_empty_paths() {
        assert_eq!(inspect_local_path(""), PathStatus::Empty);
        assert_eq!(
            inspect_local_path("/definitely/missing/rclone-manager-path"),
            PathStatus::WillCreate
        );
        let tmp = std::env::temp_dir().join("rclone-manager-empty-inspect");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert_eq!(
            inspect_local_path(&tmp.to_string_lossy()),
            PathStatus::Empty
        );
        std::fs::write(tmp.join("file.txt"), "x").unwrap();
        assert_eq!(
            inspect_local_path(&tmp.to_string_lossy()),
            PathStatus::NonEmpty
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn suggests_unique_mount_path() {
        let taken = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("rclone-manager")
            .join("drive");
        let store = store_with_mount("other", &taken.to_string_lossy());
        let suggested = suggest_default_mount_path("drive", &store);
        assert!(
            suggested.ends_with("drive-2") || suggested.contains("/drive-2"),
            "{suggested}"
        );
        let bisync =
            suggest_default_op_path("drive", OperationType::Bisync, &AppStore::default(), "");
        assert!(bisync.contains("drive-bisync"), "{bisync}");
        let custom = suggest_default_op_path(
            "drive",
            OperationType::Mount,
            &AppStore::default(),
            "{home}/mnt/{remote}",
        );
        assert!(custom.contains("/mnt/drive"), "{custom}");
    }

    #[test]
    fn ensure_path_creates_local_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("will-create");
        assert_eq!(
            inspect_local_path(&path.to_string_lossy()),
            PathStatus::WillCreate
        );
        ensure_path(&PathStatus::WillCreate, &path.to_string_lossy(), None).unwrap();
        assert!(path.is_dir());
    }

    #[test]
    fn describes_collision() {
        let status = PathStatus::Collision {
            remote: "drive".into(),
            profile: "default".into(),
        };
        assert!(describe_status(&status).contains("drive"));
    }
}
