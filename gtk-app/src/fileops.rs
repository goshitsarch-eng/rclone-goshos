//! Encoded file-browser undo/redo operations and grouped transfers.

use crate::rclone::{remote_fs, RcClient};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FileOp {
    Mkdir {
        fs: String,
        path: String,
    },
    Upload {
        fs: String,
        path: String,
    },
    Rename {
        fs: String,
        from: String,
        to: String,
    },
    Copy {
        src_fs: String,
        src: String,
        dst_fs: String,
        dst: String,
    },
    Move {
        src_fs: String,
        src: String,
        dst_fs: String,
        dst: String,
    },
    Delete {
        fs: String,
        path: String,
        #[serde(default)]
        trash: Option<String>,
    },
}

impl FileOp {
    pub fn apply(&self, client: &RcClient) -> Result<(), String> {
        match self {
            FileOp::Mkdir { fs, path } => client
                .mkdir(fs, path)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            FileOp::Upload { .. } => {
                // Content is not retained; undo deletes the destination and redo is a no-op.
                Ok(())
            }
            FileOp::Rename { fs, from, to } => client
                .move_file(fs, from, fs, to)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            FileOp::Copy {
                src_fs,
                src,
                dst_fs,
                dst,
            } => client
                .copy_file(src_fs, src, dst_fs, dst)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            FileOp::Move {
                src_fs,
                src,
                dst_fs,
                dst,
            } => client
                .move_file(src_fs, src, dst_fs, dst)
                .map(|_| ())
                .map_err(|e| e.to_string()),
            FileOp::Delete { fs, path, .. } => client
                .purge(fs, path)
                .or_else(|_| client.delete_file(fs, path))
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn decode(text: &str) -> Option<Self> {
        if let Ok(op) = serde_json::from_str::<Self>(text) {
            return Some(op);
        }
        // Legacy tokens from earlier GTK builds.
        if let Some(path) = text.strip_prefix("mkdir:") {
            return Some(Self::Mkdir {
                fs: "/".into(),
                path: path.to_string(),
            });
        }
        if let Some(path) = text.strip_prefix("upload:") {
            return Some(Self::Upload {
                fs: "/".into(),
                path: path.to_string(),
            });
        }
        None
    }

    pub fn invert(&self) -> Option<Self> {
        match self {
            Self::Mkdir { fs, path } | Self::Upload { fs, path } => Some(Self::Delete {
                fs: fs.clone(),
                path: path.clone(),
                trash: None,
            }),
            Self::Rename { fs, from, to } => Some(Self::Rename {
                fs: fs.clone(),
                from: to.clone(),
                to: from.clone(),
            }),
            Self::Copy {
                src_fs: _,
                src: _,
                dst_fs,
                dst,
            } => Some(Self::Delete {
                fs: dst_fs.clone(),
                path: dst.clone(),
                trash: None,
            }),
            Self::Move {
                src_fs,
                src,
                dst_fs,
                dst,
            } => Some(Self::Move {
                src_fs: dst_fs.clone(),
                src: dst.clone(),
                dst_fs: src_fs.clone(),
                dst: src.clone(),
            }),
            Self::Delete {
                fs,
                path,
                trash: Some(trash),
            } => Some(Self::Move {
                src_fs: "/".into(),
                src: trash.clone(),
                dst_fs: fs.clone(),
                dst: path.clone(),
            }),
            Self::Delete { trash: None, .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferItem {
    pub src_fs: String,
    pub src: String,
    pub dst_fs: String,
    pub dst: String,
    pub cut: bool,
}

impl TransferItem {
    pub fn endpoint(&self) -> &'static str {
        if self.cut {
            "operations/movefile"
        } else {
            "operations/copyfile"
        }
    }

    pub fn file_op(&self) -> FileOp {
        if self.cut {
            FileOp::Move {
                src_fs: self.src_fs.clone(),
                src: self.src.clone(),
                dst_fs: self.dst_fs.clone(),
                dst: self.dst.clone(),
            }
        } else {
            FileOp::Copy {
                src_fs: self.src_fs.clone(),
                src: self.src.clone(),
                dst_fs: self.dst_fs.clone(),
                dst: self.dst.clone(),
            }
        }
    }
}

pub fn transfer_group_id(origin: &str) -> String {
    format!("{origin}/{}", uuid::Uuid::new_v4().simple())
}

pub fn transfer_payload(item: &TransferItem, group: &str) -> Value {
    json!({
        "srcFs": item.src_fs,
        "srcRemote": item.src,
        "dstFs": item.dst_fs,
        "dstRemote": item.dst,
        "_async": true,
        "_group": group,
    })
}

pub fn start_grouped_transfers(
    client: &RcClient,
    items: &[TransferItem],
    origin: &str,
) -> Result<(String, Vec<u64>), String> {
    if items.is_empty() {
        return Err("nothing to transfer".into());
    }
    let group = transfer_group_id(origin);
    let mut ids = Vec::new();
    for item in items {
        let id = client
            .start_job(item.endpoint(), transfer_payload(item, &group))
            .map_err(|e| e.to_string())?;
        ids.push(id);
    }
    Ok((group, ids))
}

pub fn trash_dir() -> std::path::PathBuf {
    crate::settings::AppSettings::cache_dir().join("trash")
}

pub fn stash_local_path(path: &str) -> Option<String> {
    let src = std::path::Path::new(path);
    if !src.exists() {
        return None;
    }
    let dir = trash_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!(
        "{}-{}",
        uuid::Uuid::new_v4().simple(),
        src.file_name().and_then(|n| n.to_str()).unwrap_or("item")
    );
    let dest = dir.join(name);
    if src.is_dir() {
        copy_dir(src, &dest).ok()?;
    } else {
        std::fs::copy(src, &dest).ok()?;
    }
    Some(dest.to_string_lossy().into_owned())
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

pub fn is_local_open_target(remote: &str) -> bool {
    remote.is_empty() || remote == "local"
}

/// Open a local path in the OS handler, or download a remote file to `$TMP` first.
pub fn open_file_natively(
    client: Option<&RcClient>,
    remote: &str,
    path: &str,
    name: &str,
) -> Result<PathBuf, String> {
    if is_local_open_target(remote) {
        open::that(path).map_err(|e| e.to_string())?;
        return Ok(PathBuf::from(path));
    }
    let client = client.ok_or_else(|| "Rclone engine is offline".to_string())?;
    let file_name = if name.is_empty() {
        "rclone-open.bin"
    } else {
        name
    };
    let dest = std::env::temp_dir().join(file_name);
    let fs = remote_fs(remote, "");
    client
        .copy_file(&fs, path, "/", &dest.to_string_lossy())
        .map_err(|e| e.to_string())?;
    open::that(&dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_rename_and_move() {
        let rename = FileOp::Rename {
            fs: "drive:".into(),
            from: "a".into(),
            to: "b".into(),
        };
        assert_eq!(
            rename.invert(),
            Some(FileOp::Rename {
                fs: "drive:".into(),
                from: "b".into(),
                to: "a".into(),
            })
        );
        let mv = FileOp::Move {
            src_fs: "a:".into(),
            src: "one".into(),
            dst_fs: "b:".into(),
            dst: "two".into(),
        };
        assert_eq!(
            mv.invert(),
            Some(FileOp::Move {
                src_fs: "b:".into(),
                src: "two".into(),
                dst_fs: "a:".into(),
                dst: "one".into(),
            })
        );
    }

    #[test]
    fn copy_inverts_to_delete_and_json_roundtrips() {
        let copy = FileOp::Copy {
            src_fs: "drive:".into(),
            src: "src".into(),
            dst_fs: "/".into(),
            dst: "dst".into(),
        };
        let inverted = copy.invert().unwrap();
        assert_eq!(
            inverted,
            FileOp::Delete {
                fs: "/".into(),
                path: "dst".into(),
                trash: None,
            }
        );
        assert_eq!(FileOp::decode(&copy.encode()), Some(copy));
        assert_eq!(
            FileOp::decode("mkdir:Photos/new"),
            Some(FileOp::Mkdir {
                fs: "/".into(),
                path: "Photos/new".into(),
            })
        );
        assert!(FileOp::Delete {
            fs: "drive:".into(),
            path: "gone".into(),
            trash: None,
        }
        .invert()
        .is_none());
    }

    #[test]
    fn stashes_local_file_for_undo() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("keep.txt");
        std::fs::write(&file, "hello").unwrap();
        let trash = stash_local_path(&file.to_string_lossy()).unwrap();
        assert_eq!(std::fs::read_to_string(&trash).unwrap(), "hello");
        let op = FileOp::Delete {
            fs: "/".into(),
            path: file.to_string_lossy().into_owned(),
            trash: Some(trash),
        };
        assert!(op.invert().is_some());
    }

    #[test]
    fn grouped_transfer_payload_shares_group_and_endpoint() {
        let item = TransferItem {
            src_fs: "drive:".into(),
            src: "Photos/a.jpg".into(),
            dst_fs: "/".into(),
            dst: "Inbox/a.jpg".into(),
            cut: false,
        };
        let payload = transfer_payload(&item, "filemanager/abc");
        assert_eq!(payload["_group"], "filemanager/abc");
        assert_eq!(payload["_async"], true);
        assert_eq!(payload["srcRemote"], "Photos/a.jpg");
        assert_eq!(item.endpoint(), "operations/copyfile");
        let cut = TransferItem {
            cut: true,
            ..item.clone()
        };
        assert_eq!(cut.endpoint(), "operations/movefile");
        assert!(matches!(cut.file_op(), FileOp::Move { .. }));
        assert!(transfer_group_id("filemanager").starts_with("filemanager/"));
    }

    #[test]
    fn local_open_targets() {
        assert!(is_local_open_target("local"));
        assert!(is_local_open_target(""));
        assert!(!is_local_open_target("drive"));
        assert!(!is_local_open_target("drive:"));
    }
}
