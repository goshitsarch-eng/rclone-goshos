//! Internal Files drag-and-drop rules matching Angular `NautilusDragDropService`.

use crate::fileops::TransferItem;
use crate::rclone::{join_remote_path, parent_remote_path, remote_fs, split_remote_path};
use serde::{Deserialize, Serialize};

pub const PAYLOAD_KIND: &str = "rclone-manager-files";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DragItem {
    pub remote: String,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropDest {
    pub remote: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropPlan {
    Ignore,
    Star,
    Transfer { dest: DropDest, move_items: bool },
}

pub fn encode_payload(items: &[DragItem]) -> String {
    serde_json::to_string(&serde_json::json!({
        "kind": PAYLOAD_KIND,
        "items": items,
    }))
    .unwrap_or_default()
}

pub fn decode_payload(text: &str) -> Option<Vec<DragItem>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("kind").and_then(|v| v.as_str()) != Some(PAYLOAD_KIND) {
        return None;
    }
    serde_json::from_value(value.get("items")?.clone()).ok()
}

pub fn normalize_folder(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

pub fn location_string(remote: &str, path: &str) -> String {
    if remote == "local" {
        if path.is_empty() {
            "/".into()
        } else {
            path.to_string()
        }
    } else if path.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{path}")
    }
}

pub fn dest_from_location(input: &str) -> DropDest {
    let (remote, path) = split_remote_path(input);
    DropDest { remote, path }
}

pub fn fs_and_remote(remote: &str, path: &str) -> (String, String) {
    if remote == "local" {
        ("/".into(), path.trim_start_matches('/').to_string())
    } else {
        (remote_fs(remote, ""), path.to_string())
    }
}

pub fn resolve_transfer(items: &[DragItem], dest: &DropDest) -> DropPlan {
    if items.is_empty() {
        return DropPlan::Ignore;
    }
    if items
        .iter()
        .any(|item| item.is_dir && normalize_folder(&item.path) == normalize_folder(&dest.path))
    {
        return DropPlan::Ignore;
    }
    let same_remote = items
        .iter()
        .all(|item| item.remote.eq_ignore_ascii_case(&dest.remote));
    let source_parent = items
        .first()
        .map(|item| normalize_folder(&parent_remote_path(&item.path)))
        .unwrap_or_default();
    if same_remote && source_parent == normalize_folder(&dest.path) {
        return DropPlan::Ignore;
    }
    DropPlan::Transfer {
        dest: dest.clone(),
        move_items: same_remote,
    }
}

pub const HOVER_OPEN_MS: u64 = 1000;

/// Whether hovering `dest` during a drag should navigate into that folder.
/// Matches Angular: skip the current listing and skip a folder that is itself being dragged.
pub fn should_hover_navigate(items: &[DragItem], dest: &DropDest, current: &DropDest) -> bool {
    if dest.remote.eq_ignore_ascii_case(&current.remote)
        && normalize_folder(&dest.path) == normalize_folder(&current.path)
    {
        return false;
    }
    !items.iter().any(|item| {
        item.is_dir
            && item.remote.eq_ignore_ascii_case(&dest.remote)
            && normalize_folder(&item.path) == normalize_folder(&dest.path)
    })
}

pub fn transfer_items(items: &[DragItem], dest: &DropDest, move_items: bool) -> Vec<TransferItem> {
    items
        .iter()
        .map(|item| {
            let name = if item.name.is_empty() {
                item.path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&item.path)
                    .to_string()
            } else {
                item.name.clone()
            };
            let dst_path = join_remote_path(&dest.path, &name);
            let (src_fs, src) = fs_and_remote(&item.remote, &item.path);
            let (dst_fs, dst) = fs_and_remote(&dest.remote, &dst_path);
            TransferItem {
                src_fs,
                src,
                dst_fs,
                dst,
                cut: move_items,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(remote: &str, path: &str, is_dir: bool) -> DragItem {
        DragItem {
            remote: remote.into(),
            path: path.into(),
            name: path.rsplit('/').next().unwrap_or(path).into(),
            is_dir,
        }
    }

    #[test]
    fn payload_roundtrip() {
        let items = vec![item("drive", "Photos/a.png", false)];
        let text = encode_payload(&items);
        assert!(text.contains(PAYLOAD_KIND));
        assert_eq!(decode_payload(&text), Some(items));
        assert!(decode_payload(r#"{"kind":"other","items":[]}"#).is_none());
        assert!(decode_payload("not-json").is_none());
    }

    #[test]
    fn ignores_same_folder_and_self_drop() {
        let photos = item("drive", "Photos", true);
        assert_eq!(
            resolve_transfer(
                &[photos.clone()],
                &DropDest {
                    remote: "drive".into(),
                    path: "Photos".into(),
                }
            ),
            DropPlan::Ignore
        );
        let file = item("drive", "Photos/a.png", false);
        assert_eq!(
            resolve_transfer(
                &[file],
                &DropDest {
                    remote: "drive".into(),
                    path: "Photos".into(),
                }
            ),
            DropPlan::Ignore
        );
        assert_eq!(
            resolve_transfer(
                &[],
                &DropDest {
                    remote: "drive".into(),
                    path: "Inbox".into(),
                }
            ),
            DropPlan::Ignore
        );
    }

    #[test]
    fn same_remote_moves_cross_remote_copies() {
        let file = item("drive", "Photos/a.png", false);
        match resolve_transfer(
            &[file.clone()],
            &DropDest {
                remote: "drive".into(),
                path: "Inbox".into(),
            },
        ) {
            DropPlan::Transfer { move_items, .. } => assert!(move_items),
            other => panic!("{other:?}"),
        }
        match resolve_transfer(
            &[file],
            &DropDest {
                remote: "dropbox".into(),
                path: "Inbox".into(),
            },
        ) {
            DropPlan::Transfer { move_items, dest } => {
                assert!(!move_items);
                assert_eq!(dest.remote, "dropbox");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn builds_grouped_transfer_items() {
        let items = vec![item("drive", "Photos/a.png", false)];
        let dest = DropDest {
            remote: "drive".into(),
            path: "Inbox".into(),
        };
        let transfers = transfer_items(&items, &dest, true);
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].src_fs, "drive:");
        assert_eq!(transfers[0].src, "Photos/a.png");
        assert_eq!(transfers[0].dst_fs, "drive:");
        assert_eq!(transfers[0].dst, "Inbox/a.png");
        assert!(transfers[0].cut);
        let copy = transfer_items(
            &items,
            &DropDest {
                remote: "local".into(),
                path: "/tmp".into(),
            },
            false,
        );
        assert_eq!(copy[0].dst_fs, "/");
        assert_eq!(copy[0].dst, "tmp/a.png");
        assert!(!copy[0].cut);
    }

    #[test]
    fn hover_navigate_skips_current_and_self() {
        let dest = DropDest {
            remote: "drive".into(),
            path: "Photos".into(),
        };
        let current = dest.clone();
        let file = item("drive", "Inbox/a.png", false);
        assert!(!should_hover_navigate(&[file.clone()], &dest, &current));
        let folder = item("drive", "Photos", true);
        assert!(!should_hover_navigate(
            &[folder],
            &dest,
            &DropDest {
                remote: "drive".into(),
                path: "Inbox".into(),
            }
        ));
        assert!(should_hover_navigate(
            &[file],
            &dest,
            &DropDest {
                remote: "drive".into(),
                path: "Inbox".into(),
            }
        ));
    }

    #[test]
    fn locations_and_dest_parse() {
        assert_eq!(location_string("drive", ""), "drive:");
        assert_eq!(location_string("drive", "Photos"), "drive:Photos");
        assert_eq!(location_string("local", "/home/ada"), "/home/ada");
        let dest = dest_from_location("dropbox:Inbox");
        assert_eq!(dest.remote, "dropbox");
        assert_eq!(dest.path, "Inbox");
    }
}
