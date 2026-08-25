//! Mount / destination path inspection — mirrors Angular `PathInspectionService`.

use crate::operations::OperationType;
use crate::rclone::MountedRemote;
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
    PathStatus::Empty
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
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let base = home.join("mnt").join(remote);
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
            .join("mnt")
            .join("drive");
        let store = store_with_mount("other", &taken.to_string_lossy());
        let suggested = suggest_default_mount_path("drive", &store);
        assert!(
            suggested.ends_with("drive-2") || suggested.contains("/drive-2"),
            "{suggested}"
        );
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
