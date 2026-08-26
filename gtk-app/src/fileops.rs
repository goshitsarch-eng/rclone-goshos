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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransferItem {
    pub src_fs: String,
    pub src: String,
    pub dst_fs: String,
    pub dst: String,
    pub cut: bool,
    pub is_dir: bool,
}

impl TransferItem {
    pub fn endpoint(&self) -> &'static str {
        match (self.cut, self.is_dir) {
            (true, true) => "sync/move",
            (false, true) => "sync/copy",
            (true, false) => "operations/movefile",
            (false, false) => "operations/copyfile",
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
    if item.is_dir {
        let mut payload = json!({
            "srcFs": join_fs_path(&item.src_fs, &item.src),
            "dstFs": join_fs_path(&item.dst_fs, &item.dst),
            "createEmptySrcDirs": true,
            "_async": true,
            "_group": group,
        });
        if item.cut {
            payload["deleteEmptySrcDirs"] = json!(true);
        }
        payload
    } else {
        json!({
            "srcFs": item.src_fs,
            "srcRemote": item.src,
            "dstFs": item.dst_fs,
            "dstRemote": item.dst,
            "_async": true,
            "_group": group,
        })
    }
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

const MANAGER_CLIPBOARD_MARK: &str = "rclone-manager-clipboard";

/// Encode Files copy/cut items so paste still works after a view rebuild.
pub fn encode_manager_clipboard(items: &[(String, String, bool, bool)]) -> String {
    let mut out = String::from(MANAGER_CLIPBOARD_MARK);
    out.push('\n');
    for (remote, path, cut, is_dir) in items {
        out.push_str(&format!("{remote}\t{path}\t{cut}\t{is_dir}\n"));
    }
    out
}

pub fn parse_manager_clipboard(text: &str) -> Option<Vec<(String, String, bool, bool)>> {
    let mut lines = text.lines();
    if lines.next()?.trim() != MANAGER_CLIPBOARD_MARK {
        return None;
    }
    let mut items = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let remote = parts.next()?.to_string();
        let path = parts.next()?.to_string();
        if remote.is_empty() || path.is_empty() {
            continue;
        }
        let cut = parts.next().is_some_and(|v| v == "true");
        let is_dir = parts.next().is_some_and(|v| v == "true");
        items.push((remote, path, cut, is_dir));
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

/// Name stored on a Files list row. `AdwActionRow` is itself a `ListBoxRow`,
/// so callers must read `widget_name` from the row, not from its inner child.
pub fn listing_row_name(widget_name: &str, title: &str) -> Option<String> {
    if widget_name == "column-header" {
        return None;
    }
    if !widget_name.is_empty()
        && !matches!(
            widget_name,
            "AdwActionRow" | "GtkListBoxRow" | "GtkBox" | "GtkFlowBoxChild"
        )
    {
        return Some(widget_name.to_string());
    }
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteItem {
    pub fs: String,
    pub path: String,
    pub is_dir: bool,
}

impl DeleteItem {
    pub fn endpoint(&self) -> &'static str {
        if self.is_dir {
            "operations/purge"
        } else {
            "operations/deletefile"
        }
    }

    pub fn file_op(&self, trash: Option<String>) -> FileOp {
        FileOp::Delete {
            fs: self.fs.clone(),
            path: self.path.clone(),
            trash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameItem {
    pub fs: String,
    pub from: String,
    pub to: String,
    pub is_dir: bool,
}

impl RenameItem {
    pub fn endpoint(&self) -> &'static str {
        if self.is_dir {
            "sync/move"
        } else {
            "operations/movefile"
        }
    }

    pub fn file_op(&self) -> FileOp {
        FileOp::Rename {
            fs: self.fs.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
        }
    }
}

/// If exactly one destination folder is selected (and it is not a clipboard
/// source), paste *into* that folder instead of the current listing.
pub fn paste_dest_dir(
    current_remote: &str,
    current_path: &str,
    selected: &[String],
    listing_dirs: &[String],
    clipboard: &[(String, String, bool, bool)],
) -> String {
    if selected.len() != 1 {
        return current_path.to_string();
    }
    let name = &selected[0];
    if name.is_empty() || !listing_dirs.iter().any(|dir| dir == name) {
        return current_path.to_string();
    }
    let into_source = clipboard.iter().any(|(remote, path, _, _)| {
        remote == current_remote && path.rsplit('/').next() == Some(name.as_str())
    });
    if into_source {
        return current_path.to_string();
    }
    let path = current_path.trim_end_matches('/');
    if path.is_empty() {
        name.clone()
    } else {
        format!("{path}/{name}")
    }
}

pub fn join_fs_path(fs: &str, path: &str) -> String {
    let path = path.trim_start_matches('/');
    if fs == "/" || fs.is_empty() {
        if path.is_empty() {
            "/".into()
        } else {
            format!("/{path}")
        }
    } else if path.is_empty() {
        fs.to_string()
    } else if fs.ends_with(':') {
        format!("{fs}{path}")
    } else if fs.ends_with('/') {
        format!("{fs}{path}")
    } else {
        format!("{fs}/{path}")
    }
}

pub fn delete_payload(item: &DeleteItem, group: &str) -> Value {
    json!({
        "fs": item.fs,
        "remote": item.path,
        "_async": true,
        "_group": group,
    })
}

pub fn rename_payload(item: &RenameItem, group: &str) -> Value {
    if item.is_dir {
        json!({
            "srcFs": join_fs_path(&item.fs, &item.from),
            "dstFs": join_fs_path(&item.fs, &item.to),
            "createEmptySrcDirs": true,
            "deleteEmptySrcDirs": true,
            "_async": true,
            "_group": group,
        })
    } else {
        json!({
            "srcFs": item.fs,
            "srcRemote": item.from,
            "dstFs": item.fs,
            "dstRemote": item.to,
            "_async": true,
            "_group": group,
        })
    }
}

pub fn start_grouped_deletes(
    client: &RcClient,
    items: &[DeleteItem],
    origin: &str,
) -> Result<(String, Vec<u64>), String> {
    if items.is_empty() {
        return Err("nothing to delete".into());
    }
    let group = transfer_group_id(origin);
    let mut ids = Vec::new();
    for item in items {
        let id = client
            .start_job(item.endpoint(), delete_payload(item, &group))
            .map_err(|e| e.to_string())?;
        ids.push(id);
    }
    Ok((group, ids))
}

pub fn start_grouped_renames(
    client: &RcClient,
    items: &[RenameItem],
    origin: &str,
) -> Result<(String, Vec<u64>), String> {
    if items.is_empty() {
        return Err("nothing to rename".into());
    }
    let group = transfer_group_id(origin);
    let mut ids = Vec::new();
    for item in items {
        let id = client
            .start_job(item.endpoint(), rename_payload(item, &group))
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

pub const ENGINE_OFFLINE: &str = "Rclone engine is offline";

fn dest_child(dest_dir: &str, name: &str) -> String {
    let dest = dest_dir.trim_matches('/');
    if dest.is_empty() {
        name.to_string()
    } else {
        format!("{dest}/{name}")
    }
}

/// Walk local files and folders into copyfile transfer items that share one group.
pub fn collect_local_upload_items(
    paths: &[std::path::PathBuf],
    dest_fs: &str,
    dest_dir: &str,
) -> Result<Vec<TransferItem>, String> {
    let mut items = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upload")
            .to_string();
        let dest = dest_child(dest_dir, &name);
        if path.is_dir() {
            collect_local_dir(path, dest_fs, &dest, &mut items)?;
        } else if path.is_file() {
            items.push(TransferItem {
                src_fs: "/".into(),
                src: path.to_string_lossy().into_owned(),
                dst_fs: dest_fs.to_string(),
                dst: dest,
                cut: false,
                is_dir: false,
            });
        }
    }
    Ok(items)
}

fn collect_local_dir(
    local: &std::path::Path,
    dest_fs: &str,
    dest_dir: &str,
    items: &mut Vec<TransferItem>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(local).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("item")
            .to_string();
        let dest = dest_child(dest_dir, &name);
        if path.is_dir() {
            collect_local_dir(&path, dest_fs, &dest, items)?;
        } else if path.is_file() {
            items.push(TransferItem {
                src_fs: "/".into(),
                src: path.to_string_lossy().into_owned(),
                dst_fs: dest_fs.to_string(),
                dst: dest,
                cut: false,
                is_dir: false,
            });
        }
    }
    Ok(())
}

pub fn upload_dest_dirs(items: &[TransferItem]) -> Vec<(String, String)> {
    let mut dirs = std::collections::BTreeSet::new();
    for item in items {
        if let Some((parent, _)) = item.dst.rsplit_once('/') {
            if !parent.is_empty() {
                dirs.insert((item.dst_fs.clone(), parent.to_string()));
            }
        }
    }
    dirs.into_iter().collect()
}

/// Volume metadata from lsblk / df, keyed by mount point.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VolumeInfo {
    pub label: String,
    pub file_system: String,
    pub is_removable: bool,
    pub total_space: u64,
    pub available_space: u64,
}

/// Local sidebar drive matching Angular `LocalDrive`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDrive {
    pub path: String,
    pub label: String,
    pub label_is_key: bool,
    pub show_name: bool,
    pub is_removable: bool,
    pub total_space: u64,
    pub available_space: u64,
    pub file_system: String,
}

impl LocalDrive {
    pub fn title(&self, t_or: impl Fn(&str, &str) -> String) -> String {
        if self.label_is_key {
            let fallback = match self.label.as_str() {
                "titlebar.home" => "Home",
                "nautilus.titles.fileSystem" => "File System",
                _ => "Local Disk",
            };
            t_or(&self.label, fallback)
        } else {
            self.label.clone()
        }
    }

    pub fn subtitle(&self) -> Option<String> {
        if self.show_name {
            Some(self.path.clone())
        } else {
            None
        }
    }
}

pub fn normalize_drive_path(path: &str) -> String {
    let mut p = path.replace('\\', "/");
    if p.len() == 2 && p.ends_with(':') && p.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    {
        p.push('/');
    }
    if p == "/sdcard" {
        p = "/storage/emulated/0".into();
    } else if let Some(suffix) = p.strip_prefix("/sdcard/") {
        p = format!("/storage/emulated/0/{suffix}");
    }
    p
}

pub fn is_windows_root(path: &str) -> bool {
    let p = path.trim_end_matches(['/', '\\']);
    p.len() == 2 && p.ends_with(':') && p.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

pub fn is_home_directory(path: &str, home: Option<&str>) -> bool {
    let clean = normalize_drive_path(path);
    let clean = clean.trim_end_matches('/');
    if let Some(home) = home {
        let home = normalize_drive_path(home);
        let home = home.trim_end_matches('/');
        if !home.is_empty() && clean.eq_ignore_ascii_case(home) {
            return true;
        }
    }
    let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    matches!(parts.as_slice(), ["home", _] | ["Users", _])
}

/// Sidebar label for a local disk path (Home / File system / last component).
pub fn local_disk_label(path: &str, home: Option<&str>) -> String {
    let normalized = normalize_drive_path(path);
    let trimmed = normalized.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" || is_windows_root(&normalized) {
        return "File System".into();
    }
    if is_home_directory(&normalized, home) {
        return "Home".into();
    }
    trimmed
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

pub fn lookup_volume<'a>(
    volumes: &'a std::collections::HashMap<String, VolumeInfo>,
    path: &str,
) -> Option<&'a VolumeInfo> {
    let normalized = normalize_drive_path(path);
    volumes
        .get(&normalized)
        .or_else(|| volumes.get(path))
        .or_else(|| {
            if normalized.ends_with('/') {
                volumes.get(normalized.trim_end_matches('/'))
            } else {
                volumes.get(&format!("{normalized}/"))
            }
        })
}

pub fn build_local_drive(
    path: &str,
    home: Option<&str>,
    volume: Option<&VolumeInfo>,
) -> LocalDrive {
    let normalized = normalize_drive_path(path);
    let is_home = is_home_directory(&normalized, home);
    let is_root = {
        let trimmed = normalized.trim_end_matches('/');
        trimmed.is_empty() || trimmed == "/" || is_windows_root(&normalized)
    };
    let (label, label_is_key, show_name) = if is_home {
        ("titlebar.home".into(), true, false)
    } else if is_root {
        ("nautilus.titles.fileSystem".into(), true, false)
    } else if let Some(volume) = volume {
        if !volume.label.is_empty() {
            (volume.label.clone(), false, true)
        } else {
            let raw = local_disk_label(&normalized, home);
            (raw, false, false)
        }
    } else {
        let raw = local_disk_label(&normalized, home);
        (raw, false, false)
    };
    LocalDrive {
        path: if normalized.is_empty() {
            "/".into()
        } else {
            normalized
        },
        label,
        label_is_key,
        show_name,
        is_removable: volume.is_some_and(|v| v.is_removable),
        total_space: volume.map(|v| v.total_space).unwrap_or(0),
        available_space: volume.map(|v| v.available_space).unwrap_or(0),
        file_system: volume.map(|v| v.file_system.clone()).unwrap_or_default(),
    }
}

pub fn parse_lsblk_json(text: &str) -> std::collections::HashMap<String, VolumeInfo> {
    let mut out = std::collections::HashMap::new();
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return out;
    };
    if let Some(devices) = value.get("blockdevices").and_then(|v| v.as_array()) {
        walk_lsblk(devices, &mut out);
    }
    out
}

fn walk_lsblk(devices: &[Value], out: &mut std::collections::HashMap<String, VolumeInfo>) {
    for dev in devices {
        let label = json_string(dev.get("label"));
        let file_system = json_string(dev.get("fstype"));
        let is_removable = json_boolish(dev.get("rm"));
        for mount in lsblk_mounts(dev) {
            out.entry(normalize_drive_path(&mount))
                .or_insert(VolumeInfo {
                    label: label.clone(),
                    file_system: file_system.clone(),
                    is_removable,
                    total_space: 0,
                    available_space: 0,
                });
        }
        if let Some(children) = dev.get("children").and_then(|v| v.as_array()) {
            walk_lsblk(children, out);
        }
    }
}

fn lsblk_mounts(dev: &Value) -> Vec<String> {
    let mut mounts = Vec::new();
    if let Some(mount) = dev.get("mountpoint").and_then(|v| v.as_str()) {
        if !mount.is_empty() && mount != "null" {
            mounts.push(mount.to_string());
        }
    }
    if let Some(arr) = dev.get("mountpoints").and_then(|v| v.as_array()) {
        for value in arr {
            if let Some(mount) = value.as_str() {
                if !mount.is_empty() && mount != "null" {
                    mounts.push(mount.to_string());
                }
            }
        }
    }
    mounts
}

fn json_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) if s != "null" => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn json_boolish(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Free and total bytes for a local path via `df` (Tauri `get_local_disk_usage`).
pub fn local_path_disk_usage(path: &str) -> Option<(u64, u64)> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut probe = std::path::PathBuf::from(trimmed);
    while !probe.as_os_str().is_empty() && !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    if probe.as_os_str().is_empty() {
        return None;
    }
    let output = std::process::Command::new("df")
        .args(["-B1", "--output=size,avail,target"])
        .arg(&probe)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    parse_df_output(&text)
        .into_values()
        .next()
        .map(|(total, avail)| (avail, total))
}

/// Parse `df -B1 --output=size,avail,target` (header + rows).
pub fn parse_df_output(text: &str) -> std::collections::HashMap<String, (u64, u64)> {
    let mut out = std::collections::HashMap::new();
    for (idx, line) in text.lines().enumerate() {
        if idx == 0 {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(size) = parts.next().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        let Some(avail) = parts.next().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        let target: String = parts.collect::<Vec<_>>().join(" ");
        if !target.is_empty() {
            out.insert(normalize_drive_path(&target), (size, avail));
        }
    }
    out
}

fn probe_volume_map() -> std::collections::HashMap<String, VolumeInfo> {
    let mut volumes = std::process::Command::new("lsblk")
        .args(["-J", "-o", "NAME,LABEL,FSTYPE,MOUNTPOINT,RM"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| parse_lsblk_json(&text))
        .unwrap_or_default();
    if let Some(df) = std::process::Command::new("df")
        .args(["-B1", "--output=size,avail,target"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
    {
        for (mount, (total, available)) in parse_df_output(&df) {
            let entry = volumes.entry(mount).or_default();
            entry.total_space = total;
            entry.available_space = available;
        }
    }
    volumes
}

/// Enrich rclone `core/disks` paths with lsblk labels, space, and removable flags.
pub fn collect_local_drives(paths: &[String]) -> Vec<LocalDrive> {
    let home = dirs::home_dir().map(|p| p.to_string_lossy().into_owned());
    let volumes = probe_volume_map();
    let mut seen = std::collections::BTreeSet::new();
    let mut drives = Vec::new();
    for path in paths {
        let drive = build_local_drive(path, home.as_deref(), lookup_volume(&volumes, path));
        if seen.insert(drive.path.clone()) {
            drives.push(drive);
        }
    }
    drives
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
    let client = client.ok_or_else(|| ENGINE_OFFLINE.to_string())?;
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
    fn manager_clipboard_roundtrip_and_rejects_plain_text() {
        let items = vec![
            ("testdrive".into(), "Photos".into(), false, true),
            ("testdrive".into(), "a.txt".into(), true, false),
        ];
        let encoded = encode_manager_clipboard(&items);
        assert!(encoded.starts_with("rclone-manager-clipboard"));
        assert_eq!(parse_manager_clipboard(&encoded).as_ref(), Some(&items));
        assert!(parse_manager_clipboard("Photos\n/tmp/a.txt").is_none());
        assert!(parse_manager_clipboard("").is_none());
    }

    #[test]
    fn listing_row_name_reads_widget_name_not_inner_child() {
        assert_eq!(listing_row_name("Photos", "Photos"), Some("Photos".into()));
        assert_eq!(listing_row_name("column-header", "Name"), None);
        assert_eq!(
            listing_row_name("AdwActionRow", "Photos"),
            Some("Photos".into())
        );
        assert_eq!(listing_row_name("GtkListBoxRow", ""), None);
        assert_eq!(listing_row_name("", "a.txt"), Some("a.txt".into()));
    }

    #[test]
    fn paste_into_selected_folder_unless_it_is_the_source() {
        let clip = vec![("testdrive".into(), "Photos".into(), false, true)];
        assert_eq!(
            paste_dest_dir(
                "testdrive",
                "",
                &["verify-gui-folder".into()],
                &["Photos".into(), "verify-gui-folder".into()],
                &clip
            ),
            "verify-gui-folder"
        );
        assert_eq!(
            paste_dest_dir(
                "testdrive",
                "Inbox",
                &["verify-gui-folder".into()],
                &["verify-gui-folder".into()],
                &clip
            ),
            "Inbox/verify-gui-folder"
        );
        assert_eq!(
            paste_dest_dir(
                "testdrive",
                "",
                &["Photos".into()],
                &["Photos".into()],
                &clip
            ),
            ""
        );
        assert_eq!(
            paste_dest_dir("testdrive", "Inbox", &[], &["Photos".into()], &clip),
            "Inbox"
        );
    }

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
            is_dir: false,
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
        let folder = TransferItem {
            src_fs: "drive:".into(),
            src: "Photos".into(),
            dst_fs: "drive:".into(),
            dst: "Backup/Photos".into(),
            cut: false,
            is_dir: true,
        };
        assert_eq!(folder.endpoint(), "sync/copy");
        let folder_payload = transfer_payload(&folder, "filemanager/dir");
        assert_eq!(folder_payload["srcFs"], "drive:Photos");
        assert_eq!(folder_payload["dstFs"], "drive:Backup/Photos");
        assert_eq!(folder_payload["createEmptySrcDirs"], true);
        assert_eq!(
            TransferItem {
                cut: true,
                ..folder
            }
            .endpoint(),
            "sync/move"
        );
        assert!(matches!(cut.file_op(), FileOp::Move { .. }));
        assert!(transfer_group_id("filemanager").starts_with("filemanager/"));
    }

    #[test]
    fn local_open_targets() {
        assert!(is_local_open_target("local"));
        assert!(is_local_open_target(""));
        assert!(!is_local_open_target("drive"));
        assert!(!is_local_open_target("drive:"));
        assert_eq!(ENGINE_OFFLINE, "Rclone engine is offline");
    }

    #[test]
    fn delete_and_rename_payloads_match_tauri_batch() {
        let file = DeleteItem {
            fs: "drive:".into(),
            path: "Photos/a.jpg".into(),
            is_dir: false,
        };
        let dir = DeleteItem {
            fs: "/".into(),
            path: "tmp/folder".into(),
            is_dir: true,
        };
        assert_eq!(file.endpoint(), "operations/deletefile");
        assert_eq!(dir.endpoint(), "operations/purge");
        let payload = delete_payload(&file, "filemanager/del");
        assert_eq!(payload["remote"], "Photos/a.jpg");
        assert_eq!(payload["_group"], "filemanager/del");
        assert_eq!(payload["_async"], true);

        let file_rn = RenameItem {
            fs: "drive:".into(),
            from: "old.txt".into(),
            to: "new.txt".into(),
            is_dir: false,
        };
        let dir_rn = RenameItem {
            fs: "drive:".into(),
            from: "Photos".into(),
            to: "Pictures".into(),
            is_dir: true,
        };
        assert_eq!(file_rn.endpoint(), "operations/movefile");
        assert_eq!(dir_rn.endpoint(), "sync/move");
        let file_payload = rename_payload(&file_rn, "filemanager/rn");
        assert_eq!(file_payload["srcRemote"], "old.txt");
        assert_eq!(file_payload["dstRemote"], "new.txt");
        let dir_payload = rename_payload(&dir_rn, "filemanager/rn");
        assert_eq!(dir_payload["srcFs"], "drive:Photos");
        assert_eq!(dir_payload["dstFs"], "drive:Pictures");
        assert_eq!(dir_payload["createEmptySrcDirs"], true);
        assert_eq!(join_fs_path("/", "home/me"), "/home/me");
        assert_eq!(join_fs_path("alias:", "docs/a"), "alias:docs/a");
        assert!(matches!(file.file_op(None), FileOp::Delete { .. }));
        assert!(matches!(file_rn.file_op(), FileOp::Rename { .. }));
    }

    #[test]
    fn collects_mixed_local_uploads_into_one_item_list() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let nested = dir.path().join("folder");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(&a, "one").unwrap();
        std::fs::write(nested.join("b.txt"), "two").unwrap();
        let items = collect_local_upload_items(&[a.clone(), nested.clone()], "testdrive:", "Inbox")
            .unwrap();
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .all(|item| item.dst_fs == "testdrive:" && !item.cut));
        let dests: Vec<_> = items.iter().map(|item| item.dst.as_str()).collect();
        assert!(dests.contains(&"Inbox/a.txt"));
        assert!(dests.contains(&"Inbox/folder/b.txt"));
        let dirs = upload_dest_dirs(&items);
        assert!(dirs
            .iter()
            .any(|(fs, path)| fs == "testdrive:" && path == "Inbox"));
        assert!(dirs
            .iter()
            .any(|(fs, path)| fs == "testdrive:" && path == "Inbox/folder"));
    }

    #[test]
    fn labels_local_disks() {
        assert_eq!(local_disk_label("/", None), "File System");
        assert_eq!(local_disk_label("/home/me", Some("/home/me")), "Home");
        assert_eq!(local_disk_label("/media/usb", None), "usb");
        assert_eq!(local_disk_label("", None), "File System");
        assert_eq!(local_disk_label("C:\\", None), "File System");
        assert_eq!(
            local_disk_label("C:/Users/me", Some("C:\\Users\\me")),
            "Home"
        );
        assert_eq!(
            normalize_drive_path("/sdcard/DCIM"),
            "/storage/emulated/0/DCIM"
        );
        assert!(is_windows_root("D:"));
        assert!(is_home_directory("/Users/ada", None));
    }

    #[test]
    fn builds_local_drives_from_volume_metadata() {
        let usb = VolumeInfo {
            label: "BACKUP".into(),
            file_system: "exfat".into(),
            is_removable: true,
            total_space: 1000,
            available_space: 400,
        };
        let drive = build_local_drive("/media/usb", None, Some(&usb));
        assert_eq!(drive.label, "BACKUP");
        assert!(drive.show_name && drive.is_removable);
        assert_eq!(drive.file_system, "exfat");
        let home = build_local_drive("/home/me", Some("/home/me"), None);
        assert_eq!(home.label, "titlebar.home");
        assert!(home.label_is_key);
        let root = build_local_drive("/", None, None);
        assert_eq!(root.label, "nautilus.titles.fileSystem");
        assert_eq!(
            root.title(|key, fallback| {
                assert_eq!(key, "nautilus.titles.fileSystem");
                fallback.to_string()
            }),
            "File System"
        );
    }

    #[test]
    fn parses_lsblk_and_df_output() {
        let lsblk = r#"{
            "blockdevices": [
                {
                    "name": "sda",
                    "mountpoint": "/",
                    "label": null,
                    "fstype": "ext4",
                    "rm": false,
                    "children": [
                        {
                            "name": "sdb1",
                            "mountpoints": ["/media/usb"],
                            "label": "BACKUP",
                            "fstype": "exfat",
                            "rm": "1"
                        }
                    ]
                }
            ]
        }"#;
        let volumes = parse_lsblk_json(lsblk);
        assert_eq!(volumes["/"].file_system, "ext4");
        assert!(!volumes["/"].is_removable);
        assert_eq!(volumes["/media/usb"].label, "BACKUP");
        assert!(volumes["/media/usb"].is_removable);
        let df = parse_df_output("1B-blocks Avail Mounted on\n100 40 /\n200 80 /media/usb\n");
        assert_eq!(df["/"], (100, 40));
        assert_eq!(df["/media/usb"], (200, 80));
        let usage = local_path_disk_usage("/tmp");
        assert!(usage.is_some());
        let (free, total) = usage.unwrap();
        assert!(total > 0);
        assert!(free <= total);
        assert!(local_path_disk_usage("").is_none());
    }
}
