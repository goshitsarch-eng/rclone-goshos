//! Start rclone jobs from saved remote profiles — mirrors
//! `start_profile_batch` / `parse_common_config`.

use crate::operations::OperationType;
use crate::rclone::{format_bytes, remote_fs, MountedRemote, RcClient, RcError, ServeItem};
use crate::store::{quick_run_paths, JobInfo, JobMeta, ProfileConfig, QuickRun, RemoteMeta};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub const SOURCE_KEYS: &[&str] = &["source", "srcFs", "path1", "fs"];
pub const DEST_KEYS: &[&str] = &["dest", "dstFs", "path2", "mountPoint"];

pub fn flatten_rclone(rclone: &Value) -> Value {
    if let Some(obj) = rclone.as_object() {
        if obj.contains_key("srcFs")
            || obj.contains_key("dstFs")
            || obj.contains_key("fs")
            || obj.contains_key("path1")
            || obj.contains_key("mountPoint")
            || obj.contains_key("url")
        {
            return rclone.clone();
        }
        for key in [
            "sync",
            "copy",
            "move",
            "bisync",
            "mount",
            "serve",
            "check",
            "delete",
            "copyurl",
            "archivecreate",
            "cryptcheck",
        ] {
            if let Some(nested) = obj.get(key) {
                if nested.is_object() {
                    return nested.clone();
                }
            }
        }
    }
    rclone.clone()
}

pub fn path_list(value: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        match value.get(*key) {
            Some(Value::String(s)) if !s.is_empty() => return vec![s.clone()],
            Some(Value::Array(arr)) => {
                let paths: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
                if !paths.is_empty() {
                    return paths;
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

pub fn first_path(value: &Value, keys: &[&str]) -> Option<String> {
    path_list(value, keys).into_iter().next()
}

/// Split a job src/dst field that may list multiple rclone paths.
pub fn split_job_paths(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "—" {
        return Vec::new();
    }
    if trimmed.starts_with('[') {
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(trimmed) {
            return items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    if trimmed.contains(',') {
        let parts: Vec<String> = trimmed
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "—")
            .collect();
        if parts.len() > 1 {
            return parts;
        }
    }
    vec![trimmed.to_string()]
}

/// Paths the Angular remote card can open while an operation is active.
/// Serve has no folder blossom. Mount uses the live mount point; sync ops use src/dst.
pub fn active_open_paths(
    op: OperationType,
    src: &str,
    dst: &str,
    mount_point: Option<&str>,
) -> Vec<String> {
    if matches!(op, OperationType::Serve) {
        return Vec::new();
    }
    let mut paths = Vec::new();
    let push = |paths: &mut Vec<String>, value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "—" {
            return;
        }
        if !paths.iter().any(|existing| existing == trimmed) {
            paths.push(trimmed.to_string());
        }
    };
    if op == OperationType::Mount {
        if let Some(point) = mount_point.filter(|s| !s.trim().is_empty()) {
            push(&mut paths, point);
        } else {
            for part in split_job_paths(dst) {
                push(&mut paths, &part);
            }
        }
        return paths;
    }
    for part in split_job_paths(src) {
        push(&mut paths, &part);
    }
    for part in split_job_paths(dst) {
        push(&mut paths, &part);
    }
    paths
}

/// Angular `isFolderOpeningAction` without op/profile filters.
pub fn is_folder_opening(opening: &std::collections::HashSet<String>, remote: &str) -> bool {
    let remote = remote.trim();
    !remote.is_empty() && opening.contains(remote)
}

/// Start a folder-open action. Returns false when one is already in flight.
pub fn begin_folder_open(opening: &mut std::collections::HashSet<String>, remote: &str) -> bool {
    let remote = remote.trim();
    if remote.is_empty() || opening.contains(remote) {
        return false;
    }
    opening.insert(remote.to_string());
    true
}

pub fn end_folder_open(opening: &mut std::collections::HashSet<String>, remote: &str) {
    opening.remove(remote.trim());
}

/// Operations currently running for a remote (mount / serve / jobs).
pub fn active_remote_ops(
    name: &str,
    mounted: bool,
    serving: bool,
    jobs: &[JobInfo],
) -> Vec<OperationType> {
    let mut ops = Vec::new();
    for op in OperationType::ALL {
        let active = match op {
            OperationType::Mount => mounted,
            OperationType::Serve => serving,
            other => jobs.iter().any(|job| {
                job_belongs_to_remote(job, name)
                    && job_operation_matches(&job.operation, other)
                    && job_is_running(job)
            }),
        };
        if active {
            ops.push(op);
        }
    }
    ops
}

pub fn overflow_active_ops(
    displayed: &[OperationType],
    active: &[OperationType],
) -> Vec<OperationType> {
    active
        .iter()
        .copied()
        .filter(|op| !displayed.contains(op))
        .collect()
}

pub fn is_dry_run(rclone: &Value) -> bool {
    for key in ["DryRun", "dry_run", "dryRun"] {
        if rclone.get(key).and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
    }
    false
}

pub fn is_resync(rclone: &Value) -> bool {
    for key in ["Resync", "resync"] {
        if rclone.get(key).and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
    }
    false
}

pub fn extra_flags(rclone: &Value) -> Map<String, Value> {
    let skip = [
        "source",
        "srcFs",
        "srcRemote",
        "path1",
        "fs",
        "dest",
        "dstFs",
        "dstRemote",
        "path2",
        "mountPoint",
        "url",
        "type",
        "addr",
        "remote",
        "filenames",
        "format",
        "prefix",
        "include",
    ];
    let mut out = Map::new();
    if let Some(obj) = flatten_rclone(rclone).as_object() {
        for (k, v) in obj {
            if skip.contains(&k.as_str()) {
                continue;
            }
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

pub fn assemble_rclone(
    op: OperationType,
    sources: &[String],
    dest: &str,
    flags: Map<String, Value>,
) -> Value {
    let mut obj = flags;
    let filtered: Vec<String> = sources
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    match op {
        OperationType::Mount => {
            if let Some(source) = filtered.first() {
                obj.insert("srcFs".into(), json!(source));
                obj.insert("fs".into(), json!(source));
            }
            if !dest.is_empty() {
                obj.insert("mountPoint".into(), json!(dest));
            }
        }
        OperationType::Serve => {
            if let Some(source) = filtered.first() {
                obj.insert("fs".into(), json!(source));
            }
            if !dest.is_empty() {
                obj.insert("addr".into(), json!(dest));
            }
        }
        OperationType::Copyurl => {
            match filtered.as_slice() {
                [] => {}
                [one] => {
                    obj.insert("url".into(), json!(one));
                }
                many => {
                    obj.insert("url".into(), json!(many));
                    obj.insert("srcFs".into(), json!(many));
                }
            }
            if !dest.is_empty() {
                obj.insert("dstFs".into(), json!(dest));
                obj.insert("fs".into(), json!(dest));
            }
        }
        OperationType::Delete => insert_sources(&mut obj, &filtered),
        _ => {
            insert_sources(&mut obj, &filtered);
            if !dest.is_empty() {
                obj.insert("dstFs".into(), json!(dest));
                if op == OperationType::Bisync {
                    obj.insert("path2".into(), json!(dest));
                    if let Some(source) = filtered.first() {
                        obj.insert("path1".into(), json!(source));
                    }
                }
            }
        }
    }
    Value::Object(obj)
}

fn insert_sources(obj: &mut Map<String, Value>, sources: &[String]) {
    match sources {
        [] => {}
        [one] => {
            obj.insert("srcFs".into(), json!(one));
        }
        many => {
            obj.insert("srcFs".into(), json!(many));
        }
    }
}

pub fn default_source(remote: &str, rclone: &Value) -> String {
    first_path(&flatten_rclone(rclone), SOURCE_KEYS).unwrap_or_else(|| remote_fs(remote, ""))
}

pub fn default_dest(remote: &str, rclone: &Value, op: OperationType) -> String {
    first_path(&flatten_rclone(rclone), DEST_KEYS).unwrap_or_else(|| match op {
        OperationType::Mount => crate::path_inspection::suggest_default_op_path(
            remote,
            OperationType::Mount,
            &crate::store::AppStore::default(),
            "",
        ),
        OperationType::Bisync => crate::path_inspection::suggest_default_op_path(
            remote,
            OperationType::Bisync,
            &crate::store::AppStore::default(),
            "",
        ),
        OperationType::Serve => "127.0.0.1:0".into(),
        _ => remote_fs(remote, ""),
    })
}

/// Split `remote:path` (or a local path) the way Tauri `parse_fs` does.
pub fn parse_transfer_fs(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let is_windows_drive =
        bytes.len() > 2 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/');
    if is_windows_drive || trimmed.starts_with('/') {
        return Some((trimmed.to_string(), String::new()));
    }
    let split_idx = trimmed.find(':')?;
    let base = &trimmed[..=split_idx];
    let root = &trimmed[split_idx + 1..];
    if base.starts_with(':') {
        return None;
    }
    if base.len() <= 2 {
        return Some((trimmed.to_string(), String::new()));
    }
    Some((base.to_string(), root.to_string()))
}

fn split_file_sides(path: &str) -> Option<(String, String)> {
    let (fs, remote) = parse_transfer_fs(path)?;
    if !remote.is_empty() {
        let remote = if fs.ends_with(':') {
            remote.trim_start_matches('/').to_string()
        } else {
            remote
        };
        return Some((fs, remote));
    }
    let normalized = fs.replace('\\', "/");
    let (parent, name) = normalized.rsplit_once('/')?;
    if name.is_empty() {
        return None;
    }
    let parent = if parent.is_empty() {
        "/".into()
    } else {
        parent.to_string()
    };
    Some((parent, name.to_string()))
}

fn dest_file_sides(dest: &str, filename: &str) -> Option<(String, String)> {
    let (fs, root) = parse_transfer_fs(dest)?;
    let root = if fs.ends_with(':') {
        root.trim_start_matches('/').to_string()
    } else {
        root
    };
    let dst_remote = if root.is_empty() || root.ends_with('/') || root.ends_with('\\') {
        if root.is_empty() {
            filename.to_string()
        } else {
            format!("{}/{filename}", root.trim_end_matches(['/', '\\']))
        }
    } else if source_looks_like_file(&format!("x:{root}")) {
        root
    } else {
        format!("{root}/{filename}")
    };
    Some((fs, dst_remote))
}

/// Heuristic used when `operations/stat` is unavailable.
pub fn source_looks_like_file(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.ends_with('/')
        || trimmed.ends_with('\\')
        || trimmed.ends_with(':')
    {
        return false;
    }
    let leaf = match parse_transfer_fs(trimmed) {
        Some((_, remote)) if !remote.is_empty() => remote
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&remote)
            .to_string(),
        _ => trimmed
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or(trimmed)
            .to_string(),
    };
    leaf.contains('.') && leaf != "." && leaf != ".."
}

/// Prefer rclone `operations/stat`; fall back to the path heuristic.
pub fn source_is_directory(client: Option<&RcClient>, path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty()
        || trimmed.ends_with('/')
        || trimmed.ends_with('\\')
        || trimmed.ends_with(':')
    {
        return true;
    }
    if let Some(client) = client {
        if let Some((fs, remote)) = parse_transfer_fs(trimmed) {
            match client.stat(&fs, &remote) {
                Ok(Some(item)) => return item.is_dir,
                Ok(None) | Err(_) => {}
            }
        }
    }
    !source_looks_like_file(trimmed)
}

fn file_transfer_request(
    op: OperationType,
    source: &str,
    dest: &str,
    mut body: Value,
) -> Result<JobRequest, String> {
    let (src_fs, src_remote) = split_file_sides(source)
        .ok_or_else(|| format!("Could not parse source '{source}' as a file path"))?;
    if src_remote.is_empty() {
        return Err(format!("Could not parse source '{source}' as a file path"));
    }
    let filename = src_remote
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&src_remote)
        .to_string();
    let (dst_fs, dst_remote) = dest_file_sides(dest, &filename)
        .ok_or_else(|| format!("Could not parse destination '{dest}' as a file path"))?;
    let obj = body.as_object_mut().unwrap();
    obj.insert("srcFs".into(), json!(src_fs));
    obj.insert("srcRemote".into(), json!(src_remote));
    obj.insert("dstFs".into(), json!(dst_fs));
    obj.insert("dstRemote".into(), json!(dst_remote));
    let endpoint = match op {
        OperationType::Copy => "operations/copyfile",
        OperationType::Move => "operations/movefile",
        _ => {
            return Err(format!(
                "{op} does not support single-file operations/copyfile"
            ))
        }
    };
    Ok(JobRequest::Async {
        endpoint,
        params: body,
    })
}

pub fn build_job_params(
    op: OperationType,
    remote: &str,
    source: &str,
    dest: &str,
    rclone: &Value,
) -> Result<JobRequest, String> {
    build_job_params_ex(op, remote, source, dest, rclone, None)
}

pub fn build_job_params_ex(
    op: OperationType,
    remote: &str,
    source: &str,
    dest: &str,
    rclone: &Value,
    is_dir: Option<bool>,
) -> Result<JobRequest, String> {
    let is_dir = is_dir.unwrap_or_else(|| !source_looks_like_file(source));
    let flags = extra_flags(rclone);
    let mut body = Value::Object(flags);
    let obj = body.as_object_mut().unwrap();
    match op {
        OperationType::Mount => {
            if dest.is_empty() {
                return Err("Mount requires a mount point".into());
            }
            Ok(JobRequest::Mount {
                fs: if source.is_empty() {
                    remote_fs(remote, "")
                } else {
                    source.to_string()
                },
                mount_point: dest.to_string(),
                mount_type: obj
                    .get("mountType")
                    .or_else(|| obj.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("mount")
                    .to_string(),
            })
        }
        OperationType::Serve => {
            let flat = flatten_rclone(rclone);
            let serve_type = ["type", "serveType"]
                .iter()
                .find_map(|key| flat.get(*key).and_then(|v| v.as_str()))
                .filter(|s| !s.is_empty())
                .unwrap_or("http")
                .to_string();
            let addr = flat
                .get("addr")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    if dest.is_empty() {
                        None
                    } else {
                        Some(dest.to_string())
                    }
                })
                .unwrap_or_else(|| "127.0.0.1:0".into());
            Ok(JobRequest::Serve {
                serve_type,
                fs: if source.is_empty() {
                    remote_fs(remote, "")
                } else {
                    source.to_string()
                },
                addr,
                extra: body.clone(),
            })
        }
        OperationType::Delete => {
            if !is_dir {
                if let Some((fs, remote_path)) = split_file_sides(source) {
                    obj.insert("fs".into(), json!(fs));
                    obj.insert("remote".into(), json!(remote_path));
                    return Ok(JobRequest::Async {
                        endpoint: "operations/deletefile",
                        params: body,
                    });
                }
            }
            obj.insert(
                "fs".into(),
                json!(if source.is_empty() {
                    remote_fs(remote, "")
                } else {
                    source.to_string()
                }),
            );
            Ok(JobRequest::Async {
                endpoint: "operations/delete",
                params: body,
            })
        }
        OperationType::Copyurl => {
            let url = obj
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or(if source.starts_with("http") {
                    Some(source)
                } else {
                    None
                })
                .ok_or_else(|| "Copyurl requires a URL".to_string())?;
            obj.insert("url".into(), json!(url));
            obj.insert(
                "fs".into(),
                json!(if dest.is_empty() {
                    remote_fs(remote, "")
                } else {
                    dest.to_string()
                }),
            );
            let filename = obj
                .get("filename")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    obj.get("filenames")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                });
            let auto_filename = if filename.is_some() {
                false
            } else {
                obj.get("autoFilename")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
            };
            if let Some(name) = filename {
                obj.insert("remote".into(), json!(name));
            }
            obj.insert("autoFilename".into(), json!(auto_filename));
            Ok(JobRequest::Async {
                endpoint: "operations/copyurl",
                params: body,
            })
        }
        OperationType::Bisync => {
            if source.is_empty() || dest.is_empty() {
                return Err("Bisync requires source and destination".into());
            }
            obj.insert("path1".into(), json!(source));
            obj.insert("path2".into(), json!(dest));
            Ok(JobRequest::Async {
                endpoint: "sync/bisync",
                params: body,
            })
        }
        OperationType::Archivecreate => {
            if source.is_empty() || dest.is_empty() {
                return Err("Archive create requires source and destination".into());
            }
            let flags = flatten_rclone(rclone);
            let format = flags
                .get("format")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("zip")
                .to_string();
            let mut dest = dest.to_string();
            if !has_archive_extension(&dest) {
                dest = format!("{dest}/archive.{format}");
            }
            let include = flags
                .get("include")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let opts = crate::rclone::ArchiveCreateOpts {
                src: source.to_string(),
                dst: dest,
                format: Some(format),
                prefix: flags
                    .get("prefix")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                full_path: flags
                    .get("fullPath")
                    .or_else(|| flags.get("full_path"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                include,
            };
            Ok(JobRequest::Async {
                endpoint: "operations/archive",
                params: crate::rclone::archive_create_payload(&opts),
            })
        }
        other => {
            if source.is_empty() {
                return Err(format!("{other} requires a source path"));
            }
            if dest.is_empty() && other != OperationType::Delete {
                return Err(format!("{other} requires a destination path"));
            }
            if !is_dir && matches!(other, OperationType::Copy | OperationType::Move) {
                return file_transfer_request(other, source, dest, body);
            }
            obj.insert("srcFs".into(), json!(source));
            obj.insert("dstFs".into(), json!(dest));
            let endpoint = other
                .rc_job_endpoint()
                .ok_or_else(|| format!("{other} has no async endpoint"))?;
            Ok(JobRequest::Async {
                endpoint,
                params: body,
            })
        }
    }
}

pub fn has_archive_extension(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".zip", ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz", ".tar.xz", ".txz", ".tar.zst",
        ".tar.br", ".tar.sz", ".tar.mz", ".tar.lz", ".tar.lz4", ".7z",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobRequest {
    Async {
        endpoint: &'static str,
        params: Value,
    },
    Mount {
        fs: String,
        mount_point: String,
        mount_type: String,
    },
    Serve {
        serve_type: String,
        fs: String,
        addr: String,
        extra: Value,
    },
}

pub fn start_request(client: &RcClient, request: &JobRequest) -> Result<String, RcError> {
    match request {
        JobRequest::Async { endpoint, params }
            if *endpoint == "operations/archive"
                && params.get("action").and_then(|v| v.as_str()) == Some("create") =>
        {
            client
                .archive_create(&crate::rclone::archive_create_opts_from_payload(params))
                .map(|id| format!("#{id}"))
        }
        JobRequest::Async { endpoint, params } => {
            let id = client.start_job(endpoint, params.clone())?;
            if let Ok(status) = client.job_status(id) {
                let finished = status
                    .get("finished")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if finished {
                    if let Some(err) = status
                        .get("error")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        return Err(RcError::message(err.to_string()));
                    }
                }
            }
            Ok(format!("#{id}"))
        }
        JobRequest::Mount {
            fs,
            mount_point,
            mount_type,
        } => {
            std::fs::create_dir_all(mount_point).ok();
            client
                .mount(fs, mount_point, mount_type)
                .map(|_| mount_point.clone())
        }
        JobRequest::Serve {
            serve_type,
            fs,
            addr,
            extra,
        } => client.serve_start_ex(serve_type, fs, addr, extra).map(|v| {
            v.get("id")
                .and_then(|x| x.as_str())
                .or_else(|| v.get("addr").and_then(|x| x.as_str()))
                .unwrap_or(addr)
                .to_string()
        }),
    }
}

pub fn apply_helper_options(
    rclone: &mut Value,
    profile: &ProfileConfig,
    meta: Option<&RemoteMeta>,
) {
    let Some(meta) = meta else {
        return;
    };
    let inline = inline_runtime_remote(rclone);
    let Some(obj) = rclone.as_object_mut() else {
        return;
    };
    if let Some(vfs) = meta.helper_profile("vfs", &profile.app.vfs_profile) {
        merge_options_block(obj, "vfsOpt", &vfs);
    }
    if let Some(filter) = meta.helper_profile("filter", &profile.app.filter_profile) {
        merge_options_block(obj, "_filter", &filter);
    }
    if let Some(backend) = meta.helper_profile("backend", &profile.app.backend_profile) {
        merge_options_block(obj, "_config", &backend);
    }
    if let Some(runtime) = meta.helper_profile("runtime", &profile.app.runtime_remote_profile) {
        merge_runtime_overrides(obj, &runtime);
    }
    if let Some(inline) = inline {
        merge_runtime_overrides(obj, &inline);
        obj.remove("runtimeRemote");
    }
}

fn inline_runtime_remote(rclone: &Value) -> Option<Value> {
    rclone
        .get("runtimeRemote")
        .filter(|value| value.is_object())
        .cloned()
}

fn merge_runtime_overrides(obj: &mut Map<String, Value>, value: &Value) {
    let Some(map) = value.as_object() else {
        return;
    };
    for (key, entry) in map {
        if key == "runtimeRemote" || key == "remotes" {
            continue;
        }
        if entry.is_null() || matches!(entry, Value::String(s) if s.is_empty()) {
            continue;
        }
        obj.entry(key.clone()).or_insert(entry.clone());
    }
}

pub fn merge_options_block(obj: &mut Map<String, Value>, block: &str, opts: &Value) {
    let mut nested = obj
        .get(block)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    match opts {
        Value::Object(map) => {
            for (k, v) in map {
                if v.is_null() || matches!(v, Value::String(s) if s.is_empty()) {
                    continue;
                }
                nested.insert(k.clone(), v.clone());
            }
        }
        other => {
            nested.insert("value".into(), other.clone());
        }
    }
    if !nested.is_empty() {
        obj.insert(block.into(), Value::Object(nested));
    }
}

/// Angular remote-card `hasNoProfiles`: start is disabled unless a profile exists.
/// Mount still offers the default mount-point fallback used by `toggle_mount`.
pub fn allows_unconfigured_start(op: OperationType) -> bool {
    matches!(op, OperationType::Mount | OperationType::Serve)
}

pub fn preferred_mount_profile(meta: Option<&RemoteMeta>) -> Option<ProfileConfig> {
    let meta = meta?;
    let names = meta.profile_names(OperationType::Mount);
    if names.is_empty() {
        return None;
    }
    let preferred = names
        .iter()
        .find(|name| name.eq_ignore_ascii_case("default"))
        .or_else(|| names.first())?;
    meta.get_profile(OperationType::Mount, preferred)
}

/// Name used by tray Mount/Unmount when the remote has no `default` profile.
pub fn preferred_mount_profile_name(meta: Option<&RemoteMeta>) -> String {
    preferred_mount_profile(meta)
        .map(|profile| profile.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "default".into())
}

/// True when an rclone fs string belongs to `remote` (`drive`, `drive:`, `drive:path`).
pub fn fs_belongs_to_remote(fs: &str, remote: &str) -> bool {
    let fs = fs.trim();
    let remote = remote.trim();
    if remote.is_empty() || fs.is_empty() {
        return false;
    }
    fs == remote
        || fs == format!("{remote}:")
        || fs.starts_with(&format!("{remote}:"))
        || fs.contains(&format!("/{remote}"))
}

pub fn remote_activity_counts(
    remote: &str,
    mounts: &[crate::rclone::MountedRemote],
    serves: &[crate::rclone::ServeItem],
    jobs: &[crate::store::JobInfo],
) -> (usize, usize, usize) {
    let mounts = mounts
        .iter()
        .filter(|item| fs_belongs_to_remote(&item.fs, remote))
        .count();
    let serves = serves
        .iter()
        .filter(|item| fs_belongs_to_remote(&item.fs, remote))
        .count();
    let jobs = jobs
        .iter()
        .filter(|job| {
            (job.remote == remote || fs_belongs_to_remote(&job.src, remote))
                && (job_is_running(job) || job_is_pending(job))
        })
        .count();
    (mounts, serves, jobs)
}

pub fn origin_label_key(origin: &str) -> &'static str {
    match origin.trim().to_ascii_lowercase().as_str() {
        "quick-run" | "quickrun" | "flow" => "generalOverview.jobs.originQuickRun",
        "automation" | "autostart" => "generalOverview.jobs.originAutomation",
        "filemanager" | "files" | "nautilus" => "generalOverview.jobs.originFiles",
        "dashboard" | "" => "generalOverview.jobs.originDashboard",
        _ => "generalOverview.jobs.originManual",
    }
}

pub fn origin_matches(origin: &str, filter: &str) -> bool {
    if filter.is_empty() || filter.eq_ignore_ascii_case("all") {
        return true;
    }
    let origin = origin.trim().to_ascii_lowercase();
    let filter = filter.trim().to_ascii_lowercase();
    match filter.as_str() {
        "quickrun" => matches!(origin.as_str(), "quickrun" | "quick-run" | "flow"),
        "dashboard" => origin.is_empty() || origin == "dashboard",
        "filemanager" => matches!(origin.as_str(), "filemanager" | "file-manager" | "files"),
        other => origin == other,
    }
}

pub fn automation_origin(id: &str) -> &'static str {
    if id.starts_with("quick:") {
        "quickrun"
    } else {
        "dashboard"
    }
}

/// Overview origin chips. `automation` means every scheduled item.
pub fn automation_matches_filter(id: &str, filter: &str) -> bool {
    let filter = filter.trim();
    if filter.is_empty()
        || filter.eq_ignore_ascii_case("all")
        || filter.eq_ignore_ascii_case("automation")
    {
        return true;
    }
    origin_matches(automation_origin(id), filter)
}

pub fn start_profile(
    client: &RcClient,
    remote: &str,
    op: OperationType,
    profile: &ProfileConfig,
    meta: Option<&RemoteMeta>,
    origin: &str,
) -> Result<String, String> {
    start_profile_ex(client, remote, op, profile, meta, origin, None)
}

pub fn directory_only_source_error(
    op: OperationType,
    source: &str,
    is_dir: bool,
) -> Option<String> {
    if matches!(
        op,
        OperationType::Sync | OperationType::Bisync | OperationType::Check
    ) && !is_dir
    {
        Some(format!(
            "{op:?} only supports directories, not files: {source}"
        ))
    } else {
        None
    }
}

pub fn start_profile_ex(
    client: &RcClient,
    remote: &str,
    op: OperationType,
    profile: &ProfileConfig,
    meta: Option<&RemoteMeta>,
    origin: &str,
    scoped: Option<&[(String, String)]>,
) -> Result<String, String> {
    let mut rclone = flatten_rclone(&profile.rclone);
    apply_helper_options(&mut rclone, profile, meta);
    if let Some(obj) = rclone.as_object_mut() {
        let pname = if profile.name.is_empty() {
            "default"
        } else {
            profile.name.as_str()
        };
        obj.insert("profile".into(), json!(pname));
        obj.insert("origin".into(), json!(origin));
    }
    let dest = default_dest(remote, &rclone, op);
    let source_keys: &[&str] = if op == OperationType::Copyurl {
        &["url", "srcFs", "source"]
    } else {
        SOURCE_KEYS
    };
    let mut sources = path_list(&rclone, source_keys);
    if sources.is_empty() {
        let fallback = default_source(remote, &rclone);
        if !fallback.is_empty() {
            sources.push(fallback);
        }
    }
    if sources.is_empty() && !matches!(op, OperationType::Mount | OperationType::Serve) {
        return Err(format!("{op} requires a source path"));
    }
    if sources.is_empty() {
        sources.push(String::new());
    }
    let filenames: Vec<String> = rclone
        .get("filenames")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    let pairs: Vec<(String, String)> = if op != OperationType::Bisync {
        if let Some(scoped) = scoped.filter(|pairs| !pairs.is_empty()) {
            scoped.to_vec()
        } else {
            sources
                .iter()
                .cloned()
                .map(|source| (source, dest.clone()))
                .collect()
        }
    } else {
        sources
            .iter()
            .cloned()
            .map(|source| (source, dest.clone()))
            .collect()
    };
    let mut ids = Vec::new();
    for (index, (source, pair_dest)) in pairs.into_iter().enumerate() {
        let mut item = rclone.clone();
        if op == OperationType::Copyurl {
            if let Some(name) = filenames
                .get(index)
                .map(String::as_str)
                .filter(|s| !s.is_empty())
            {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("filename".into(), json!(name));
                    obj.insert("autoFilename".into(), json!(false));
                }
            }
        }
        let is_dir = source_is_directory(Some(client), &source);
        if let Some(err) = directory_only_source_error(op, &source, is_dir) {
            return Err(err);
        }
        let request = build_job_params_ex(op, remote, &source, &pair_dest, &item, Some(is_dir))?;
        ids.push(start_request(client, &request).map_err(|e| e.to_string())?);
    }
    Ok(ids.join(", "))
}

pub fn parse_started_ids(result: &str) -> Vec<u64> {
    result
        .split(',')
        .filter_map(|part| {
            let trimmed = part.trim().trim_start_matches('#');
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse().ok()
            }
        })
        .collect()
}

pub fn remember_started(map: &mut HashMap<u64, JobMeta>, result: &str, meta: JobMeta) {
    let ids = parse_started_ids(result);
    remember_grouped(map, &ids, meta);
}

pub fn rename_jobs_profile(jobs: &mut [JobInfo], remote: &str, from: &str, to: &str) -> usize {
    let mut updated = 0;
    for job in jobs {
        if job.remote == remote && job.profile == from {
            job.profile = to.to_string();
            updated += 1;
        }
    }
    updated
}

pub fn apply_job_meta(job: &mut JobInfo, meta: Option<&JobMeta>) {
    let Some(meta) = meta else {
        return;
    };
    if job.origin.is_empty() || job.origin == "dashboard" && !meta.origin.is_empty() {
        job.origin = meta.origin.clone();
    }
    if (job.profile.is_empty() || job.profile == "default") && !meta.profile.is_empty() {
        job.profile = meta.profile.clone();
    }
    if job.remote.is_empty() && !meta.remote.is_empty() {
        job.remote = meta.remote.clone();
    }
    if job.parent_job_id.is_none() {
        job.parent_job_id = meta.parent_job_id;
    }
    if !meta.group.is_empty() && (job.group.is_empty() || job.group.starts_with("job/")) {
        job.group = meta.group.clone();
    }
}

pub fn is_overview_job(job: &JobInfo) -> bool {
    job.parent_job_id.is_none()
}

pub fn find_job_by_id(live: &[JobInfo], history: &[JobInfo], id: u64) -> Option<JobInfo> {
    live.iter()
        .find(|job| job.id == id)
        .or_else(|| history.iter().find(|job| job.id == id))
        .cloned()
}

pub fn job_from_meta(id: u64, meta: &JobMeta) -> JobInfo {
    let items = meta
        .transfer_snapshot
        .as_array()
        .cloned()
        .unwrap_or_default();
    let first = items.first();
    let src = first
        .and_then(|item| item.get("src"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let dst = first
        .and_then(|item| item.get("dst"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let total: u64 = items
        .iter()
        .filter_map(|item| item.get("size").and_then(|value| value.as_u64()))
        .sum();
    let count = items.len() as i64;
    let snapshot = meta.transfer_snapshot.clone();
    let operation = match meta.origin.as_str() {
        "filemanager" | "files" | "check-resolve" => "copy",
        other if !other.is_empty() => other,
        _ => "job",
    }
    .to_string();
    let mut job = JobInfo {
        id,
        operation: operation.clone(),
        remote: meta.remote.clone(),
        profile: if meta.profile.is_empty() {
            "default".into()
        } else {
            meta.profile.clone()
        },
        status: "completed".into(),
        origin: if meta.origin.is_empty() {
            "dashboard".into()
        } else {
            meta.origin.clone()
        },
        start_time: DateTime::<Utc>::UNIX_EPOCH,
        error: None,
        dry_run: false,
        src,
        dst,
        group: if meta.group.is_empty() {
            format!("job/{id}")
        } else {
            meta.group.clone()
        },
        stats: json!({
            "bytes": total,
            "totalBytes": total,
            "transfers": count,
            "totalTransfers": count,
            "completed": finalize_completed_list(&snapshot),
        }),
        transferring: json!([]),
        duration: 0.0,
        progress: 1.0,
        output: json!({ "operation": operation, "origin": meta.origin }),
        completed: finalize_completed_list(&snapshot),
        parent_job_id: meta.parent_job_id,
    };
    apply_job_meta(&mut job, Some(meta));
    job
}

/// Prefer a real stored/history job over rclone 1.60 leftover `job/<id>` stubs.
pub fn resolve_detail_job(rc_job: Option<JobInfo>, stored: Option<JobInfo>) -> Option<JobInfo> {
    match (rc_job, stored) {
        (Some(rc), Some(stored)) if is_managed_job(&rc) => Some(merge_rc_with_stored(rc, stored)),
        (Some(rc), None) if is_managed_job(&rc) => Some(rc),
        (_, stored) => stored,
    }
}

fn merge_rc_with_stored(mut rc: JobInfo, stored: JobInfo) -> JobInfo {
    if rc.src.is_empty() {
        rc.src = stored.src;
    }
    if rc.dst.is_empty() {
        rc.dst = stored.dst;
    }
    if rc.remote.is_empty() {
        rc.remote = stored.remote;
    }
    if rc.operation.starts_with("job/") || rc.operation == "job" {
        rc.operation = stored.operation;
    }
    if (rc.origin.is_empty() || rc.origin == "dashboard")
        && !stored.origin.is_empty()
        && stored.origin != "dashboard"
    {
        rc.origin = stored.origin;
    }
    if rc.progress <= 0.0 && stored.progress > 0.0 {
        rc.progress = stored.progress;
    }
    if rc.duration <= 0.0 && stored.duration > 0.0 {
        rc.duration = stored.duration;
    }
    if (transfer_list_empty(&rc.completed) || completed_needs_sizes(&rc.completed))
        && !transfer_list_empty(&stored.completed)
    {
        rc.completed = stored.completed;
    }
    scope_job_transfers(&mut rc);
    rc
}

pub fn finalize_history_job(job: &mut JobInfo) {
    if matches!(job.status.as_str(), "completed" | "failed" | "stopped") && job.progress <= 0.0 {
        job.progress = 1.0;
    }
    if completed_needs_sizes(&job.completed) {
        job.completed = finalize_completed_list(&job.completed);
    }
    let completed = job.completed.as_array().map(|rows| rows.len()).unwrap_or(0);
    if completed == 0 {
        return;
    }
    if let Some(obj) = job.stats.as_object_mut() {
        obj.entry("transfers").or_insert(json!(completed as i64));
        obj.entry("totalTransfers")
            .or_insert(json!(completed as i64));
        if completed_needs_sizes(obj.get("completed").unwrap_or(&Value::Null))
            || !obj.contains_key("completed")
        {
            obj.insert("completed".into(), job.completed.clone());
        }
    }
}

pub fn format_job_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec <= 0.0 {
        "—".into()
    } else {
        format!("{}/s", format_bytes(bytes_per_sec.round() as i64))
    }
}

pub fn job_speed_eta(job: &JobInfo) -> (String, String) {
    let speed = stats_f64(&job.stats, &["speed"]);
    let eta = stats_f64(&job.stats, &["eta", "etaSeconds"]);
    (
        format_job_speed(speed),
        crate::rclone::format_eta_seconds(eta.round() as i64),
    )
}

/// Name + percent/speed subtitle for the Files ops expander.
pub fn job_error_text(job: &JobInfo) -> Option<String> {
    job.error
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

pub fn job_failed_transfers(job: &JobInfo, limit: usize) -> Vec<(String, String)> {
    let Some(items) = job.completed.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .map(crate::transfers::parse_completed_transfer_row)
        .filter(|row| !row.error.is_empty())
        .take(limit)
        .map(|row| (row.name, row.error))
        .collect()
}

pub fn job_transfer_previews(job: &JobInfo, limit: usize) -> Vec<(String, String)> {
    let Some(items) = job.transferring.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .take(limit)
        .map(|item| {
            let row = crate::transfers::parse_transfer_row(item);
            let speed = format_job_speed(row.speed);
            (row.name, format!("{}% · {speed}", row.percentage.max(0)))
        })
        .collect()
}

pub fn job_status_key(status: &str) -> &'static str {
    match status {
        "running" => "detailShared.jobs.status.running",
        "completed" => "detailShared.jobs.status.completed",
        "failed" => "detailShared.jobs.status.failed",
        "stopped" => "detailShared.jobs.status.stopped",
        "preparing" | "starting" => "generalOverview.jobs.starting",
        _ => "detailShared.jobs.status.unknown",
    }
}

/// Angular `JobsPanel` row: type, `#id`, profile, progress, dry-run, relative time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobPanelRow {
    pub operation: String,
    pub id_label: String,
    pub profile: String,
    pub status: String,
    pub progress_pct: Option<u32>,
    pub bytes: i64,
    pub total_bytes: i64,
    pub dry_run: bool,
    pub duration_secs: i64,
    pub relative: Option<(&'static str, i64)>,
    pub error: String,
    pub can_stop: bool,
    pub has_footer: bool,
}

pub fn job_panel_row(job: &JobInfo, now: DateTime<Utc>) -> JobPanelRow {
    let bytes = stats_i64(&job.stats, &["bytes"]);
    let total_bytes = stats_i64(&job.stats, &["totalBytes"]);
    let is_mount = job.operation.eq_ignore_ascii_case("mount");
    let progress_pct = if !is_mount && total_bytes > 0 {
        Some(
            ((bytes as f64 / total_bytes as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u32,
        )
    } else {
        None
    };
    let duration_secs = if job.duration > 0.0 {
        job.duration.round().max(0.0) as i64
    } else if has_known_start_time(job) {
        now.signed_duration_since(job.start_time)
            .num_seconds()
            .max(0)
    } else {
        0
    };
    let relative = if has_known_start_time(job) {
        Some(crate::checks::relative_time_parts(job.start_time, now))
    } else {
        None
    };
    let error = job
        .error
        .clone()
        .or_else(|| {
            job.stats
                .get("lastError")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    let can_stop = job_is_running(job) || job_is_pending(job);
    JobPanelRow {
        operation: job.operation.clone(),
        id_label: format!("#{}", job.id),
        profile: job.profile.clone(),
        status: job.status.clone(),
        progress_pct,
        bytes,
        total_bytes,
        dry_run: job.dry_run,
        duration_secs,
        relative,
        error,
        can_stop,
        has_footer: progress_pct.is_some()
            || job.dry_run
            || duration_secs > 0
            || relative.is_some(),
    }
}

/// Angular `quick-run-card` status pills (cron / watcher / autostart).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuickRunCardBadges {
    pub cron: bool,
    pub cron_expression: String,
    pub watcher: bool,
    pub watcher_changed_only: bool,
    pub autostart: bool,
}

pub fn quick_run_card_badges(qr: &QuickRun) -> QuickRunCardBadges {
    QuickRunCardBadges {
        cron: qr.config.app.cron_enabled && !qr.config.app.cron_expression.is_empty(),
        cron_expression: qr.config.app.cron_expression.clone(),
        watcher: qr.config.app.watch_enabled,
        watcher_changed_only: qr.config.app.watch_changed_only,
        autostart: qr.config.app.auto_start,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickRunFolder {
    pub kind: &'static str,
    pub path: String,
}

/// Source/destination folders the Angular card can open in Files.
pub fn quick_run_openable_folders(qr: &QuickRun) -> Vec<QuickRunFolder> {
    let (src, dst) = qr.paths();
    let mut out = Vec::new();
    if let Some(src) = src {
        for path in split_job_paths(&src) {
            if !path.is_empty() {
                out.push(QuickRunFolder {
                    kind: "source",
                    path,
                });
            }
        }
    }
    if let Some(dst) = dst {
        for path in split_job_paths(&dst) {
            if !path.is_empty() {
                out.push(QuickRunFolder {
                    kind: "destination",
                    path,
                });
            }
        }
    }
    out
}

pub fn job_origin_key(origin: &str) -> &'static str {
    match origin {
        "dashboard" => "generalOverview.jobs.originDashboard",
        "quickrun" | "quick-run" | "flow" => "generalOverview.jobs.originQuickRun",
        "automation" => "generalOverview.jobs.originAutomation",
        "filemanager" | "files" => "generalOverview.jobs.originFiles",
        _ => "generalOverview.jobs.originManual",
    }
}

pub fn has_known_start_time(job: &JobInfo) -> bool {
    job.start_time.timestamp() > 0
}

pub fn find_stored_job(
    live: &[JobInfo],
    history: &[JobInfo],
    meta: &HashMap<u64, JobMeta>,
    id: u64,
) -> Option<JobInfo> {
    find_job_by_id(live, history, id).or_else(|| meta.get(&id).map(|item| job_from_meta(id, item)))
}

pub fn history_with_meta(history: &[JobInfo], meta: &HashMap<u64, JobMeta>) -> Vec<JobInfo> {
    let extra: Vec<JobInfo> = meta
        .iter()
        .filter(|(id, _)| history.iter().all(|job| job.id != **id))
        .map(|(id, item)| job_from_meta(*id, item))
        .collect();
    merge_job_lists(history, &extra)
}

pub fn merge_job_lists(live: &[JobInfo], history: &[JobInfo]) -> Vec<JobInfo> {
    let mut out = live.to_vec();
    let ids: HashSet<u64> = out.iter().map(|job| job.id).collect();
    for job in history {
        if !ids.contains(&job.id) {
            out.push(job.clone());
        }
    }
    out
}

pub fn merge_overview_jobs(
    live: &[JobInfo],
    history: &[JobInfo],
    remote: &str,
    profile: Option<&str>,
    operation: Option<OperationType>,
) -> Vec<JobInfo> {
    let matches = |job: &JobInfo| {
        is_overview_job(job)
            && job.remote == remote
            && profile.is_none_or(|wanted| {
                job.profile == wanted || job.profile.is_empty() || job.profile == "default"
            })
            && operation.is_none_or(|op| job_operation_matches(&job.operation, op))
    };
    let mut out: Vec<JobInfo> = live.iter().filter(|job| matches(job)).cloned().collect();
    let ids: HashSet<u64> = out.iter().map(|job| job.id).collect();
    for job in history
        .iter()
        .filter(|job| matches(job) && !ids.contains(&job.id))
    {
        out.push(job.clone());
    }
    out.sort_by(|a, b| b.start_time.cmp(&a.start_time).then(b.id.cmp(&a.id)));
    out
}

pub fn job_status_value(job: &JobInfo) -> Value {
    json!({
        "group": job.group,
        "duration": job.duration,
        "startTime": job.start_time.to_rfc3339(),
        "output": job.output,
        "finished": !job_is_running(job) && !job_is_pending(job),
        "success": job.status == "completed",
        "error": job.error,
    })
}

pub fn remember_grouped(map: &mut HashMap<u64, JobMeta>, ids: &[u64], meta: JobMeta) {
    let parent = ids.first().copied();
    for (index, id) in ids.iter().enumerate() {
        let mut item = meta.clone();
        if index > 0 {
            item.parent_job_id = parent;
        }
        map.insert(*id, item);
    }
}

pub fn transfer_snapshot_from_items(items: &[crate::fileops::TransferItem]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| {
                let size = std::fs::metadata(&item.src).map(|m| m.len()).unwrap_or(0);
                let name = item
                    .src
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(item.src.as_str());
                json!({
                    "name": name,
                    "srcFs": item.src_fs,
                    "srcRemote": item.src,
                    "dstFs": item.dst_fs,
                    "dstRemote": item.dst,
                    "src": crate::transfers::join_fs_name(&item.src_fs, &item.src),
                    "dst": crate::transfers::join_fs_name(&item.dst_fs, &item.dst),
                    "size": size,
                    "bytes": 0,
                    "percentage": 0
                })
            })
            .collect(),
    )
}

fn transfer_list_empty(value: &Value) -> bool {
    value.as_array().map(|arr| arr.is_empty()).unwrap_or(true)
}

fn transfer_row_size(item: &Value) -> u64 {
    item.get("size")
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn transfer_row_bytes(item: &Value) -> u64 {
    item.get("bytes")
        .or_else(|| item.get("bytesSoFar"))
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

fn completed_needs_sizes(value: &Value) -> bool {
    value.as_array().is_some_and(|arr| {
        arr.iter()
            .any(|item| transfer_row_size(item) > 0 && transfer_row_bytes(item) == 0)
    })
}

fn finalize_completed_item(item: &Value) -> Value {
    let size = transfer_row_size(item);
    let bytes = transfer_row_bytes(item);
    let mut item = item.clone();
    if let Some(obj) = item.as_object_mut() {
        if size > 0 && bytes == 0 {
            obj.insert("bytes".into(), json!(size));
            obj.insert("percentage".into(), json!(100));
        } else if size > 0 && bytes >= size {
            obj.insert("percentage".into(), json!(100));
        }
    }
    item
}

fn finalize_completed_list(value: &Value) -> Value {
    match value {
        Value::Array(arr) => Value::Array(arr.iter().map(finalize_completed_item).collect()),
        other => finalize_completed_item(other),
    }
}

fn child_bytes(job: &JobInfo, size: u64, done: bool) -> u64 {
    job.stats
        .get("bytes")
        .and_then(|value| value.as_u64())
        .filter(|&bytes| bytes > 0)
        .unwrap_or(if done { size } else { 0 })
}

fn child_finished(status: &str) -> bool {
    matches!(status, "completed" | "failed")
}

/// Fill empty `transferring` / `completed` lists on grouped overview jobs from
/// the start-time snapshot and child job statuses (rclone 1.60 `copyfile` jobs
/// often omit per-file `core/stats` transferring rows).
pub fn hydrate_grouped_transfers(jobs: &mut [JobInfo], meta: &HashMap<u64, JobMeta>) {
    let mut children: HashMap<u64, Vec<usize>> = HashMap::new();
    for (idx, job) in jobs.iter().enumerate() {
        if let Some(parent) = job.parent_job_id {
            children.entry(parent).or_default().push(idx);
        }
    }
    let parents: Vec<usize> = jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.parent_job_id.is_none())
        .map(|(idx, _)| idx)
        .collect();
    for parent_idx in parents {
        let parent_id = jobs[parent_idx].id;
        let empty_active = transfer_list_empty(&jobs[parent_idx].transferring);
        let empty_done = transfer_list_empty(&jobs[parent_idx].completed);
        let needs_sizes = completed_needs_sizes(&jobs[parent_idx].completed);
        if !empty_active && !empty_done && !needs_sizes {
            continue;
        }
        let snapshot = meta
            .get(&parent_id)
            .map(|item| item.transfer_snapshot.clone())
            .unwrap_or(json!([]));
        let child_idxs = children.get(&parent_id).cloned().unwrap_or_default();
        if let Some(arr) = snapshot.as_array() {
            if !arr.is_empty() {
                let parent_done = child_finished(&jobs[parent_idx].status);
                let children_done = !child_idxs.is_empty()
                    && child_idxs.len() >= arr.len()
                    && child_idxs
                        .iter()
                        .all(|&idx| child_finished(&jobs[idx].status));
                let all_done = parent_done || children_done;
                let mut transferring = Vec::new();
                let mut completed = Vec::new();
                for item in arr {
                    let src = item
                        .get("src")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let matching = child_idxs.iter().copied().find(|&idx| {
                        jobs[idx].src == src
                            || jobs[idx].src.ends_with(src)
                            || (!src.is_empty() && src.ends_with(&jobs[idx].src))
                    });
                    let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                    let (done, bytes) = if let Some(idx) = matching {
                        let done = child_finished(&jobs[idx].status);
                        (done, child_bytes(&jobs[idx], size, done))
                    } else {
                        (all_done, if all_done { size } else { 0 })
                    };
                    let mut row = item.clone();
                    if let Some(obj) = row.as_object_mut() {
                        obj.insert("bytes".into(), json!(bytes));
                        obj.insert("percentage".into(), json!(if done { 100 } else { 0 }));
                    }
                    if done {
                        completed.push(row);
                    } else {
                        transferring.push(row);
                    }
                }
                apply_hydrated_rows(
                    &mut jobs[parent_idx],
                    empty_active,
                    empty_done,
                    needs_sizes,
                    transferring,
                    completed,
                );
                continue;
            }
        }
        if child_idxs.is_empty() {
            continue;
        }
        let mut transferring = Vec::new();
        let mut completed = Vec::new();
        for idx in child_idxs {
            let size = jobs[idx]
                .stats
                .get("totalBytes")
                .or_else(|| jobs[idx].stats.get("size"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let done = child_finished(&jobs[idx].status);
            let bytes = child_bytes(&jobs[idx], size, done);
            let row = json!({
                "name": jobs[idx].src,
                "src": jobs[idx].src,
                "dst": jobs[idx].dst,
                "size": size,
                "bytes": bytes,
                "percentage": if done { 100 } else { 0 }
            });
            if child_finished(&jobs[idx].status) {
                completed.push(row);
            } else {
                transferring.push(row);
            }
        }
        apply_hydrated_rows(
            &mut jobs[parent_idx],
            empty_active,
            empty_done,
            needs_sizes,
            transferring,
            completed,
        );
    }
}

/// Apply registry metadata and hydrate transfer rows for a single job
/// (Job detail re-fetches RC status and would otherwise drop the snapshot).
pub fn decorate_job_transfers(
    job: &mut JobInfo,
    meta: &HashMap<u64, JobMeta>,
    siblings: &[JobInfo],
) {
    apply_job_meta(job, meta.get(&job.id));
    let mut jobs: Vec<JobInfo> = siblings
        .iter()
        .filter(|item| item.id != job.id)
        .cloned()
        .collect();
    jobs.insert(0, job.clone());
    hydrate_grouped_transfers(&mut jobs, meta);
    if let Some(updated) = jobs.into_iter().find(|item| item.id == job.id) {
        *job = updated;
    }
    scope_job_transfers(job);
}

fn apply_hydrated_rows(
    job: &mut JobInfo,
    empty_active: bool,
    empty_done: bool,
    needs_sizes: bool,
    transferring: Vec<Value>,
    completed: Vec<Value>,
) {
    if empty_active {
        job.transferring = Value::Array(transferring);
    }
    if empty_done || needs_sizes {
        job.completed = finalize_completed_list(&Value::Array(completed));
    }
    if let Some(obj) = job.stats.as_object_mut() {
        if empty_active {
            obj.insert("transferring".into(), job.transferring.clone());
        }
        if empty_done || needs_sizes {
            obj.insert("completed".into(), job.completed.clone());
        }
    }
}

pub fn job_meta_for(
    remote: &str,
    profile: &ProfileConfig,
    origin: &str,
    backend: &str,
    quick_run_id: &str,
) -> JobMeta {
    JobMeta {
        origin: origin.to_string(),
        profile: if profile.name.is_empty() {
            "default".into()
        } else {
            profile.name.clone()
        },
        remote: remote.to_string(),
        backend: backend.to_string(),
        quick_run_id: quick_run_id.to_string(),
        execute_id: uuid::Uuid::new_v4().to_string(),
        parent_job_id: None,
        target: String::new(),
        group: String::new(),
        transfer_snapshot: json!([]),
    }
}

pub fn path_has_leaf(path: &str, name: &str) -> bool {
    !name.is_empty() && crate::checks::leaf_name(path) == name
}

pub fn find_resolve_job<'a>(
    jobs: &'a [JobInfo],
    meta: &HashMap<u64, JobMeta>,
    name: &str,
) -> Option<&'a JobInfo> {
    let mut best = None;
    for job in jobs {
        let item = meta.get(&job.id);
        let is_resolve = job.origin == "check-resolve"
            || item.map(|m| m.origin == "check-resolve").unwrap_or(false);
        if !is_resolve {
            continue;
        }
        let matches_name = item
            .map(|m| !m.target.is_empty() && m.target == name)
            .unwrap_or(false)
            || path_has_leaf(&job.src, name)
            || path_has_leaf(&job.dst, name);
        if !matches_name {
            continue;
        }
        if job_is_running(job) {
            return Some(job);
        }
        best = Some(job);
    }
    best
}

pub fn stats_i64(stats: &Value, keys: &[&str]) -> i64 {
    for key in keys {
        match stats.get(*key) {
            Some(Value::Number(n)) => {
                if let Some(v) = n.as_i64() {
                    return v;
                }
                if let Some(v) = n.as_u64() {
                    return v as i64;
                }
                if let Some(v) = n.as_f64() {
                    return v as i64;
                }
            }
            Some(Value::Bool(v)) => return i64::from(*v),
            Some(Value::String(s)) => {
                if let Ok(v) = s.parse::<i64>() {
                    return v;
                }
            }
            _ => {}
        }
    }
    0
}

pub fn stats_f64(stats: &Value, keys: &[&str]) -> f64 {
    for key in keys {
        match stats.get(*key) {
            Some(Value::Number(n)) => {
                if let Some(v) = n.as_f64() {
                    return v;
                }
            }
            Some(Value::String(s)) => {
                if let Ok(v) = s.parse::<f64>() {
                    return v;
                }
            }
            _ => {}
        }
    }
    0.0
}

pub fn format_seconds(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "—".into();
    }
    let total = secs.round() as i64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

pub fn stats_bool(stats: &Value, keys: &[&str]) -> bool {
    for key in keys {
        match stats.get(*key) {
            Some(Value::Bool(v)) => return *v,
            Some(Value::Number(n)) => return n.as_i64().unwrap_or(0) != 0,
            Some(Value::String(s)) => {
                if s.eq_ignore_ascii_case("true") || s == "1" || s.eq_ignore_ascii_case("yes") {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

pub fn parse_job_end_time(status: &Value) -> Option<DateTime<Utc>> {
    status
        .get("endTime")
        .or_else(|| status.get("end_time"))
        .and_then(|x| x.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

pub fn apply_session_flags(rclone: &mut Value, dry_run: bool, resync: bool) {
    if !rclone.is_object() {
        *rclone = json!({});
    }
    if let Some(obj) = rclone.as_object_mut() {
        if dry_run {
            obj.insert("DryRun".into(), json!(true));
        }
        if resync {
            obj.insert("Resync".into(), json!(true));
        }
    }
}

pub fn job_is_running(job: &JobInfo) -> bool {
    matches!(job.status.as_str(), "running" | "starting")
}

pub fn action_busy_key(remote: &str, op: &str, profile: &str) -> String {
    format!("{remote}|{op}|{profile}")
}

pub fn job_is_pending(job: &JobInfo) -> bool {
    matches!(job.status.as_str(), "starting" | "preparing" | "stopping")
}

pub fn action_in_progress(
    remote: &str,
    op: OperationType,
    profile: &str,
    jobs: &[JobInfo],
    busy: bool,
) -> bool {
    if busy {
        return true;
    }
    jobs.iter().any(|job| {
        job_belongs_to_remote(job, remote)
            && job_operation_matches(&job.operation, op)
            && (profile.is_empty()
                || job.profile.is_empty()
                || job.profile == "default"
                || job.profile == profile)
            && job_is_pending(job)
    })
}

pub fn job_operation_matches(job_op: &str, op: OperationType) -> bool {
    let key = op.as_str();
    let lower = job_op.to_ascii_lowercase();
    if lower == key || lower == op.api_label().to_ascii_lowercase() {
        return true;
    }
    lower.split(['/', ':', ' ', '.']).any(|part| part == key)
}

pub fn job_belongs_to_remote(job: &JobInfo, remote: &str) -> bool {
    let prefix = format!("{remote}:");
    job.remote.eq_ignore_ascii_case(remote)
        || job.src.starts_with(&prefix)
        || job.dst.starts_with(&prefix)
        || job.group.contains(remote)
}

pub fn find_active_job<'a>(
    jobs: &'a [JobInfo],
    remote: &str,
    op: OperationType,
    profile: &str,
) -> Option<&'a JobInfo> {
    let wanted = if profile.is_empty() {
        "default"
    } else {
        profile
    };
    jobs.iter().find(|job| {
        job_is_running(job)
            && job_belongs_to_remote(job, remote)
            && job_operation_matches(&job.operation, op)
            && (job.profile.is_empty()
                || job.profile == wanted
                || (job.profile == "default" && wanted == "default"))
    })
}

pub fn find_active_mount<'a>(
    mounts: &'a [MountedRemote],
    remote: &str,
) -> Option<&'a MountedRemote> {
    find_active_mount_for(mounts, remote, "")
}

pub fn find_active_mount_for<'a>(
    mounts: &'a [MountedRemote],
    remote: &str,
    alias: &str,
) -> Option<&'a MountedRemote> {
    mounts
        .iter()
        .find(|m| crate::store::mount_matches_remote(&m.fs, &m.mount_point, remote, alias))
}

fn paths_equivalent_point(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}

/// Mount-point candidates when RC/list matching misses (alias remotes, default dest).
pub fn mount_unmount_fallbacks(remote: &str, profile: Option<&ProfileConfig>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(profile) = profile {
        let dest = default_dest(remote, &profile.rclone, OperationType::Mount);
        if !dest.is_empty() {
            out.push(dest);
        }
    }
    let suggested = crate::path_inspection::suggest_default_mount_path(
        remote,
        &crate::store::AppStore::default(),
    );
    if !suggested.is_empty()
        && !out
            .iter()
            .any(|point| paths_equivalent_point(point, &suggested))
    {
        out.push(suggested);
    }
    out
}

/// Resolve a live mount point for `remote`, including alias + host fuse fallbacks.
pub fn resolve_unmount_point(
    mounts: &[MountedRemote],
    remote: &str,
    alias: &str,
    fallbacks: &[String],
) -> Option<String> {
    resolve_unmount_point_in(
        mounts,
        remote,
        alias,
        fallbacks,
        &crate::rclone::host_fuse_mounts(),
    )
}

pub fn resolve_unmount_point_in(
    mounts: &[MountedRemote],
    remote: &str,
    alias: &str,
    fallbacks: &[String],
    host_mounts: &[MountedRemote],
) -> Option<String> {
    if let Some(mount) = find_active_mount_for(mounts, remote, alias) {
        return Some(mount.mount_point.clone());
    }
    if alias.is_empty() {
        if let Some(mount) = find_active_mount(mounts, remote) {
            return Some(mount.mount_point.clone());
        }
    }
    for extra in host_mounts {
        if crate::store::mount_matches_remote(&extra.fs, &extra.mount_point, remote, alias) {
            return Some(extra.mount_point.clone());
        }
    }
    for point in fallbacks {
        let point = point.trim();
        if point.is_empty() {
            continue;
        }
        if mounts
            .iter()
            .chain(host_mounts.iter())
            .any(|mount| paths_equivalent_point(&mount.mount_point, point))
        {
            return Some(point.to_string());
        }
    }
    None
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileUsage {
    pub jobs: usize,
    pub mounts: usize,
    pub serves: usize,
}

impl ProfileUsage {
    pub fn blocked(&self) -> bool {
        self.jobs + self.mounts + self.serves > 0
    }

    pub fn summary(&self) -> String {
        format!(
            "{} job(s), {} mount(s), {} serve(s) still using this profile",
            self.jobs, self.mounts, self.serves
        )
    }
}

/// Angular `computeProfileActionState(..., 'rename')`: only sync/job
/// profiles are blocked while a job is active. Mount/serve stay
/// renameable so the runtime cache can cascade.
pub fn profile_rename_blocked(op: Option<OperationType>, usage: &ProfileUsage) -> bool {
    match op {
        Some(op) if op.is_sync_type() => usage.jobs > 0,
        _ => false,
    }
}

/// Angular `displayLimit` page size for transfer/check tables.
pub const ACTIVITY_PAGE: usize = 50;

pub fn activity_visible_end(total: usize, limit: usize) -> usize {
    total.min(limit.max(1))
}

pub fn activity_remaining(total: usize, limit: usize) -> usize {
    total.saturating_sub(activity_visible_end(total, limit))
}

pub fn rename_mounts_profile(
    mounts: &mut [MountedRemote],
    remote: &str,
    from: &str,
    to: &str,
) -> usize {
    if from.is_empty() || from == to {
        return 0;
    }
    let mut updated = 0;
    for mount in mounts {
        if fs_belongs_to_remote(&mount.fs, remote) && mount.profile == from {
            mount.profile = to.to_string();
            updated += 1;
        }
    }
    updated
}

pub fn profile_usage(
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
    remote: &str,
    profile: &str,
    op: Option<OperationType>,
    alias: &str,
) -> ProfileUsage {
    let mut usage = ProfileUsage::default();
    match op {
        Some(op) => {
            if find_active_job(jobs, remote, op, profile).is_some() {
                usage.jobs = 1;
            }
            if op == OperationType::Mount {
                if let Some(mount) = find_active_mount_for(mounts, remote, alias) {
                    if mount.profile.is_empty() || mount.profile == profile {
                        usage.mounts = 1;
                    }
                }
            }
            if op == OperationType::Serve && find_active_serve(serves, remote).is_some() {
                usage.serves = 1;
            }
        }
        None => {
            usage.jobs = jobs
                .iter()
                .filter(|job| {
                    job_is_running(job)
                        && job_belongs_to_remote(job, remote)
                        && (job.profile.is_empty() || job.profile == profile)
                })
                .count();
            if let Some(mount) = find_active_mount_for(mounts, remote, alias) {
                if mount.profile.is_empty() || mount.profile == profile {
                    usage.mounts = 1;
                }
            }
            if find_active_serve(serves, remote).is_some() {
                usage.serves = 1;
            }
        }
    }
    usage
}

pub fn find_active_serve<'a>(serves: &'a [ServeItem], remote: &str) -> Option<&'a ServeItem> {
    let prefix = format!("{remote}:");
    serves
        .iter()
        .find(|s| s.fs == remote || s.fs == prefix || s.fs.starts_with(&prefix))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShutdownSummary {
    pub jobs: usize,
    pub mounts: usize,
    pub serves: usize,
}

impl ShutdownSummary {
    pub fn active(self) -> bool {
        self.jobs + self.mounts + self.serves > 0
    }
}

pub fn shutdown_summary(jobs: &[JobInfo], mounts: usize, serves: usize) -> ShutdownSummary {
    ShutdownSummary {
        jobs: jobs.iter().filter(|job| job_is_running(job)).count(),
        mounts,
        serves,
    }
}

pub fn stop_all_runtime(client: &RcClient, jobs: &[JobInfo], mounts: &[MountedRemote]) {
    for job in jobs {
        if job_is_running(job) {
            let _ = client.job_stop(job.id);
        }
    }
    for mount in mounts {
        let _ = client.unmount(&mount.mount_point);
    }
    let _ = client.serve_stop_all();
}

pub fn profile_is_active(
    remote: &str,
    op: OperationType,
    profile: &str,
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
) -> bool {
    match op {
        OperationType::Mount => find_active_mount(mounts, remote).is_some(),
        OperationType::Serve => find_active_serve(serves, remote).is_some(),
        _ => find_active_job(jobs, remote, op, profile).is_some(),
    }
}

pub fn stop_profile(
    client: &RcClient,
    remote: &str,
    op: OperationType,
    profile: &str,
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
) -> Result<String, String> {
    stop_profile_ex(client, remote, op, profile, jobs, mounts, serves, "", &[])
}

pub fn stop_profile_ex(
    client: &RcClient,
    remote: &str,
    op: OperationType,
    profile: &str,
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
    alias: &str,
    fallbacks: &[String],
) -> Result<String, String> {
    match op {
        OperationType::Mount => {
            let point = resolve_unmount_point(mounts, remote, alias, fallbacks)
                .ok_or_else(|| format!("{remote} is not mounted"))?;
            client
                .unmount(&point)
                .map(|_| format!("Unmounted {point}"))
                .map_err(|e| e.to_string())
        }
        OperationType::Serve => {
            let serve = find_active_serve(serves, remote)
                .ok_or_else(|| format!("{remote} is not serving"))?;
            client
                .serve_stop(&serve.id)
                .map(|_| format!("Stopped serve {}", serve.addr))
                .map_err(|e| e.to_string())
        }
        _ => {
            let job = find_active_job(jobs, remote, op, profile)
                .ok_or_else(|| format!("No running {op} for {remote}/{profile}"))?;
            client
                .job_stop(job.id)
                .map(|_| format!("Stopped job #{}", job.id))
                .map_err(|e| e.to_string())
        }
    }
}

/// Drop rclone `core/stats` leftovers that belong to a different job.
/// rclone 1.60 often returns the global transfer list when a finished
/// group's stats are gone, which mixed e.g. `#14781` files into `#22718`.
pub fn transfer_item_matches_job(item: &Value, job: &JobInfo) -> bool {
    if let Some(id) = item
        .get("jobid")
        .or_else(|| item.get("id"))
        .and_then(|value| value.as_u64())
    {
        if id != 0 {
            return id == job.id;
        }
    }
    if let Some(group) = item.get("group").and_then(|value| value.as_str()) {
        if !group.is_empty() {
            if group == job.group || group == format!("job/{}", job.id) {
                return true;
            }
            if group.starts_with("job/") {
                return false;
            }
        }
    }
    let parsed = crate::transfers::parse_completed_transfer_row(item);
    if path_belongs_to_job(&parsed.src, job)
        || path_belongs_to_job(&parsed.dst, job)
        || path_belongs_to_job(&parsed.name, job)
    {
        return true;
    }
    let foreign_src = path_looks_located(&parsed.src) && !path_belongs_to_job(&parsed.src, job);
    let foreign_dst = path_looks_located(&parsed.dst) && !path_belongs_to_job(&parsed.dst, job);
    !foreign_src && !foreign_dst
}

fn path_looks_located(path: &str) -> bool {
    let path = path.trim();
    path.contains(':') || path.starts_with('/') || path.starts_with('\\')
}

fn path_belongs_to_job(path: &str, job: &JobInfo) -> bool {
    let path = path.trim();
    if path.is_empty() || path == "—" {
        return false;
    }
    let mut bases = vec![&job.src, &job.dst];
    if job.src.is_empty() && job.dst.is_empty() {
        bases.push(&job.remote);
    }
    for base in bases {
        let base = base.trim().trim_end_matches('/');
        if base.is_empty() {
            continue;
        }
        if path == base
            || path.starts_with(&format!("{base}/"))
            || path.starts_with(&format!("{base}:"))
            || (base.ends_with(':') && path.starts_with(base))
        {
            return true;
        }
    }
    false
}

pub fn filter_transfer_list(list: &Value, job: &JobInfo) -> Value {
    let Some(arr) = list.as_array() else {
        return list.clone();
    };
    if job.src.is_empty() && job.dst.is_empty() && job.remote.is_empty() && job.group.is_empty() {
        return list.clone();
    }
    Value::Array(
        arr.iter()
            .filter(|item| transfer_item_matches_job(item, job))
            .cloned()
            .collect(),
    )
}

pub fn scope_job_transfers(job: &mut JobInfo) {
    let transferring = filter_transfer_list(&job.transferring, job);
    let completed = filter_transfer_list(&job.completed, job);
    let stats_transferring = job
        .stats
        .get("transferring")
        .cloned()
        .map(|value| filter_transfer_list(&value, job));
    let stats_completed = job
        .stats
        .get("completed")
        .cloned()
        .map(|value| filter_transfer_list(&value, job));
    job.transferring = transferring;
    job.completed = completed;
    if let Some(obj) = job.stats.as_object_mut() {
        if let Some(value) = stats_transferring {
            obj.insert("transferring".into(), value);
        }
        if let Some(value) = stats_completed {
            obj.insert("completed".into(), value);
        }
    }
}

pub fn find_quick_run_job(live: &[JobInfo], history: &[JobInfo], qr: &QuickRun) -> Option<JobInfo> {
    if let Some(job) = find_active_quick_run(live, qr) {
        return Some(job.clone());
    }
    if let Some(id) = qr.last_job_id {
        if let Some(job) = live.iter().chain(history.iter()).find(|job| job.id == id) {
            return Some(job.clone());
        }
    }
    let (src, dst) = qr.paths();
    live.iter()
        .chain(history.iter())
        .find(|job| {
            (job.origin == "quick-run" || job.origin == "quickrun" || job.origin == "flow")
                && job_belongs_to_remote(job, &qr.remote_name)
                && job_operation_matches(&job.operation, qr.operation_type)
                && src
                    .as_ref()
                    .is_none_or(|path| job.src == *path || job.src.starts_with(path))
                && dst
                    .as_ref()
                    .is_none_or(|path| job.dst == *path || job.dst.starts_with(path))
        })
        .cloned()
}

pub fn find_active_quick_run<'a>(jobs: &'a [JobInfo], qr: &QuickRun) -> Option<&'a JobInfo> {
    if let Some(id) = qr.last_job_id {
        if let Some(job) = jobs.iter().find(|j| j.id == id && job_is_running(j)) {
            return Some(job);
        }
    }
    jobs.iter().find(|job| {
        job_is_running(job)
            && job.origin == "quick-run"
            && job_belongs_to_remote(job, &qr.remote_name)
            && job_operation_matches(&job.operation, qr.operation_type)
    })
}

/// Paths shown in Angular `app-operation-control` (live job wins, delete hides dest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationControlPaths {
    pub source: Option<String>,
    pub destination: Option<String>,
    pub hide_destination: bool,
    pub dest_browseable: bool,
}

pub fn serve_accessible_via(rclone: &Value, default_addr: &str) -> String {
    let flat = flatten_rclone(rclone);
    let kind = first_path(&flat, &["type"]).unwrap_or_else(|| "http".into());
    let addr = first_path(&flat, &["addr"]).unwrap_or_else(|| default_addr.to_string());
    format!("{} at {addr}", kind.to_ascii_uppercase())
}

pub fn is_saf_mount(rclone: &Value) -> bool {
    let flat = flatten_rclone(rclone);
    first_path(&flat, &["mountType"]).as_deref() == Some("saf")
        || first_path(&flat, &["mountPoint"]).is_some_and(|p| p.starts_with("saf://"))
}

/// Configured src/dst for Angular `buildControlConfig` (serve addr + SAF mount).
pub fn operation_control_configured_paths(
    op: OperationType,
    rclone: &Value,
    remote: &str,
    default_addr: &str,
) -> (Option<String>, Option<String>) {
    let (src, dst) = crate::store::quick_run_paths(rclone, op);
    match op {
        OperationType::Serve => (src, Some(serve_accessible_via(rclone, default_addr))),
        OperationType::Mount if is_saf_mount(rclone) => (src, Some(format!("saf://{remote}"))),
        _ => (src, dst),
    }
}

pub fn operation_control_paths(
    op: OperationType,
    configured_src: Option<String>,
    configured_dst: Option<String>,
    live: Option<&JobInfo>,
) -> OperationControlPaths {
    let nonempty = |value: Option<String>| value.filter(|s| !s.is_empty());
    let source = live
        .and_then(|job| nonempty(Some(job.src.clone())))
        .or_else(|| nonempty(configured_src));
    let destination = if op == OperationType::Serve {
        nonempty(configured_dst).or_else(|| live.and_then(|job| nonempty(Some(job.dst.clone()))))
    } else {
        live.and_then(|job| nonempty(Some(job.dst.clone())))
            .or_else(|| nonempty(configured_dst))
    };
    OperationControlPaths {
        source,
        destination,
        hide_destination: op == OperationType::Delete,
        dest_browseable: op != OperationType::Serve,
    }
}

pub fn operation_control_subtitle(op_label: &str, dry_run: bool, dry_label: &str) -> String {
    if dry_run {
        format!("{op_label} · {dry_label}")
    } else {
        op_label.to_string()
    }
}

pub fn operation_shows_session_flags(op: OperationType) -> bool {
    op.is_sync_type()
}

pub fn operation_shows_mount_usage(op: OperationType, active: bool, destination: &str) -> bool {
    op == OperationType::Mount && active && !destination.trim().is_empty()
}

pub fn operation_control_action_kind(op: OperationType, active: bool) -> &'static str {
    match (op == OperationType::Mount, active) {
        (true, false) => "mount",
        (true, true) => "unmount",
        (false, false) => "start",
        (false, true) => "stop",
    }
}

pub fn merge_completed_transfers(stats: &mut Value, transferred: &Value) {
    let list = transferred
        .get("transferred")
        .cloned()
        .or_else(|| transferred.as_array().cloned().map(Value::Array))
        .unwrap_or(json!([]));
    if let Some(obj) = stats.as_object_mut() {
        obj.insert("completed".into(), list);
    } else {
        *stats = json!({ "completed": list });
    }
}

pub fn parse_cli_flags(cli: &str) -> Map<String, Value> {
    crate::cli_import::parsed_to_flag_map(&crate::cli_import::parse(cli, &Default::default()))
}

pub fn job_from_status(jobid: u64, status: &Value, stats: Option<&Value>) -> JobInfo {
    let finished = status
        .get("finished")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let success = status
        .get("success")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let error = status
        .get("error")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let output = status.get("output").cloned().unwrap_or(json!({}));
    let src = first_path(&output, SOURCE_KEYS)
        .or_else(|| first_path(&output, &["srcRemote", "src"]))
        .unwrap_or_default();
    let dst = first_path(&output, DEST_KEYS)
        .or_else(|| first_path(&output, &["dstRemote", "dst"]))
        .unwrap_or_default();
    let operation = output
        .get("operation")
        .and_then(|x| x.as_str())
        .or_else(|| status.get("group").and_then(|x| x.as_str()))
        .unwrap_or("job")
        .to_string();
    let remote = infer_remote(&src)
        .or_else(|| infer_remote(&dst))
        .unwrap_or_default();
    let group = status
        .get("group")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("job/{jobid}"));
    let start_time = status
        .get("startTime")
        .and_then(|x| x.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let stats_value = stats.cloned().unwrap_or(json!({}));
    let transferring = stats_value
        .get("transferring")
        .cloned()
        .unwrap_or(json!([]));
    let completed = stats_value
        .get("completed")
        .or_else(|| stats_value.get("transferringCompleted"))
        .cloned()
        .unwrap_or(json!([]));
    let duration = status
        .get("duration")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let progress = progress_from_stats(&stats_value);
    let parent_job_id = output
        .get("parent_job_id")
        .or_else(|| status.get("parent_job_id"))
        .and_then(|value| value.as_u64());
    let mut job = JobInfo {
        id: jobid,
        operation,
        remote,
        profile: status
            .get("output")
            .and_then(|o| o.get("profile"))
            .and_then(|x| x.as_str())
            .unwrap_or("default")
            .to_string(),
        status: if !finished {
            "running".into()
        } else if success {
            "completed".into()
        } else {
            "failed".into()
        },
        origin: output
            .get("origin")
            .and_then(|x| x.as_str())
            .or_else(|| status.get("origin").and_then(|x| x.as_str()))
            .unwrap_or("dashboard")
            .to_string(),
        start_time,
        error,
        dry_run: output
            .get("dryRun")
            .or_else(|| output.get("dry_run"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        src,
        dst,
        group,
        stats: stats_value.clone(),
        transferring,
        duration,
        progress,
        output,
        completed,
        parent_job_id,
    };
    if finished {
        apply_cryptcheck_outcome(&mut job);
    } else if stats_value
        .get("preparing")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        job.status = "preparing".into();
    }
    scope_job_transfers(&mut job);
    job
}

pub const MAX_JOB_STATUS_FETCH: usize = 48;

/// rclone 1.60 `job/list` can return hundreds of thousands of leftover IDs.
/// Prefer jobs we started; only scan unknowns when the list is small.
pub fn select_job_ids(listed: &[u64], known: &[u64], max: usize) -> Vec<u64> {
    if listed.len() <= max {
        return listed.to_vec();
    }
    let known_set: std::collections::HashSet<u64> = known.iter().copied().collect();
    let mut selected: Vec<u64> = listed
        .iter()
        .copied()
        .filter(|id| known_set.contains(id))
        .collect();
    selected.truncate(max);
    selected
}

/// rclone 1.60 `job/list` includes finished internal RC jobs (empty src/dst,
/// operation `job/<id>`). Unfinished leftovers are also reported as running —
/// keep only jobs the app started or can identify.
pub fn is_identifiable_job(job: &JobInfo) -> bool {
    !job.src.is_empty()
        || !job.dst.is_empty()
        || !job.remote.is_empty()
        || crate::operations::OperationType::parse(&job.operation).is_some()
        || (!job.origin.is_empty() && job.origin != "dashboard")
}

pub fn is_managed_job(job: &JobInfo) -> bool {
    is_identifiable_job(job)
}

/// Local job shown immediately after `start_job` returns, before rclone reports transfers.
pub fn preparing_job(
    id: u64,
    remote: &str,
    src: &str,
    dst: &str,
    total_files: u64,
    total_bytes: u64,
) -> JobInfo {
    JobInfo {
        id,
        operation: "upload".into(),
        remote: remote.into(),
        profile: "default".into(),
        status: "preparing".into(),
        origin: "filemanager".into(),
        start_time: Utc::now(),
        error: None,
        dry_run: false,
        src: src.into(),
        dst: dst.into(),
        group: format!("job/{id}"),
        stats: json!({
            "totalBytes": total_bytes,
            "bytes": 0,
            "transfers": 0,
            "totalTransfers": total_files,
            "completed": [],
            "transferring": [],
            "preparing": true
        }),
        transferring: json!([]),
        duration: 0.0,
        progress: 0.0,
        output: json!({ "operation": "upload", "origin": "filemanager" }),
        completed: json!([]),
        parent_job_id: None,
    }
}

/// Dashboard / start-operation job shown immediately after rclone returns an id.
pub fn started_operation_job(
    id: u64,
    op: &str,
    remote: &str,
    profile: &str,
    origin: &str,
    src: &str,
    dst: &str,
) -> JobInfo {
    JobInfo {
        id,
        operation: op.into(),
        remote: remote.into(),
        profile: profile.into(),
        status: "starting".into(),
        origin: origin.into(),
        start_time: Utc::now(),
        error: None,
        dry_run: false,
        src: src.into(),
        dst: dst.into(),
        group: format!("job/{id}"),
        stats: json!({
            "bytes": 0,
            "totalBytes": 0,
            "transfers": 0,
            "totalTransfers": 0,
            "preparing": true
        }),
        transferring: json!([]),
        duration: 0.0,
        progress: 0.0,
        output: json!({ "operation": op, "origin": origin }),
        completed: json!([]),
        parent_job_id: None,
    }
}

pub fn jobs_from_transfer_start(
    ids: &[u64],
    op: &str,
    remote: &str,
    origin: &str,
    group: &str,
    snapshot: &Value,
) -> Vec<JobInfo> {
    let rows = snapshot.as_array();
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            let row = rows.and_then(|arr| arr.get(index));
            let src = row
                .and_then(|v| v.get("src"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let dst = row
                .and_then(|v| v.get("dst"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut job = started_operation_job(*id, op, remote, "default", origin, src, dst);
            if !group.is_empty() {
                job.group = group.to_string();
            }
            job
        })
        .collect()
}

pub fn preparing_progress_stats(
    bytes: u64,
    total_bytes: u64,
    transfers: u64,
    total_files: u64,
    transferring: Value,
) -> Value {
    json!({
        "totalBytes": total_bytes,
        "bytes": bytes,
        "transfers": transfers,
        "totalTransfers": total_files,
        "completed": [],
        "transferring": transferring,
        "preparing": total_bytes == 0 || bytes < total_bytes
    })
}

/// Keep preparing uploads in the live list until rclone reports the same job id.
pub const PREPARING_TTL_SECS: i64 = 120;

pub fn merge_preparing_jobs(live: Vec<JobInfo>, history: &[JobInfo]) -> Vec<JobInfo> {
    let mut out = live;
    let now = Utc::now();
    for job in history {
        if job.status != "preparing" && job.status != "starting" {
            continue;
        }
        if out.iter().any(|j| j.id == job.id) {
            continue;
        }
        if !job.group.is_empty()
            && out
                .iter()
                .any(|j| !j.group.is_empty() && j.group == job.group)
        {
            continue;
        }
        let age = now.signed_duration_since(job.start_time).num_seconds();
        if age > PREPARING_TTL_SECS {
            continue;
        }
        out.insert(0, job.clone());
    }
    out
}

pub fn finalize_dropped_job(job: &JobInfo) -> JobInfo {
    let mut finished = job.clone();
    match finished.status.as_str() {
        "running" => finished.status = "completed".into(),
        "preparing" | "starting" => {
            // rclone 1.60 often drops a finished job from job/list before the next poll.
            finished.status = "completed".into();
            finished.progress = 1.0;
        }
        _ => {}
    }
    finished
}

/// Parses raw text output from `operations/cryptcheck` into structured JSON.
pub fn parse_cryptcheck_output(raw_result: &str) -> Value {
    let mut differ = Vec::new();
    let mut missing_on_dst = Vec::new();
    let mut missing_on_src = Vec::new();
    let mut error_list = Vec::new();
    let mut success = true;
    let mut status = "OK".to_string();

    for line in raw_result.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let is_error = line.contains("ERROR :") || line.contains("ERROR:");
        let is_notice = line.contains("NOTICE:") || line.contains("NOTICE :");

        if is_error {
            let pos = line
                .find("ERROR :")
                .map(|p| p + 7)
                .or_else(|| line.find("ERROR:").map(|p| p + 6));
            if let Some(start_idx) = pos {
                let rest = &line[start_idx..];
                if let Some(colon_pos) = rest.find(':') {
                    let path = rest[..colon_pos].trim().to_string();
                    let msg = rest[colon_pos + 1..].trim();

                    if msg.contains("file not in Encrypted drive") {
                        missing_on_dst.push(path);
                    } else if msg.contains("file not in") {
                        missing_on_src.push(path);
                    } else if msg.to_lowercase().contains("differ") {
                        differ.push(path);
                    } else {
                        error_list.push(format!("{path}: {msg}"));
                    }
                }
            }
        } else if is_notice {
            let pos = line
                .find("NOTICE :")
                .map(|p| p + 8)
                .or_else(|| line.find("NOTICE:").map(|p| p + 7));
            if let Some(start_idx) = pos {
                let rest = &line[start_idx..];
                if rest.contains("Skipping undecryptable dir name") {
                    if let Some(colon_pos) = rest.find(':') {
                        let path = rest[..colon_pos].trim().to_string();
                        let msg = rest[colon_pos + 1..].trim();
                        error_list.push(format!("{path}: {msg}"));
                    }
                } else if (rest.contains("differences found")
                    && !rest.contains("0 differences found"))
                    || (status == "OK"
                        && (rest.contains("errors while checking")
                            || rest.contains("files missing")))
                {
                    status = rest.trim().to_string();
                    success = false;
                }
            }
        }
    }

    let has_issues = !differ.is_empty()
        || !missing_on_dst.is_empty()
        || !missing_on_src.is_empty()
        || !error_list.is_empty();
    if has_issues {
        success = false;
        if status == "OK" {
            let mut parts = Vec::new();
            if !differ.is_empty() {
                parts.push(format!("{} differences", differ.len()));
            }
            if !missing_on_dst.is_empty() {
                parts.push(format!("{} missing on destination", missing_on_dst.len()));
            }
            if !missing_on_src.is_empty() {
                parts.push(format!("{} missing on source", missing_on_src.len()));
            }
            if !error_list.is_empty() {
                parts.push(format!("{} errors", error_list.len()));
            }
            status = format!("{} found", parts.join(", "));
        }
    }

    json!({
        "results": [
            {
                "success": success,
                "status": status,
                "differ": differ,
                "missingOnDst": missing_on_dst,
                "missingOnSrc": missing_on_src,
                "error": error_list,
            }
        ]
    })
}

pub fn apply_cryptcheck_outcome(job: &mut JobInfo) {
    let looks_like_cryptcheck = job.operation.to_ascii_lowercase().contains("cryptcheck")
        || job
            .output
            .get("result")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("Encrypted drive") || s.contains("cryptcheck"));
    if !looks_like_cryptcheck {
        return;
    }
    let Some(result) = job
        .output
        .get("result")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
    else {
        return;
    };
    let parsed = parse_cryptcheck_output(&result);
    let first = parsed
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .cloned();
    if let Some(first) = first {
        let check_success = first
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let has_issues = first
            .get("differ")
            .and_then(|a| a.as_array())
            .is_some_and(|a| !a.is_empty())
            || first
                .get("missingOnDst")
                .and_then(|a| a.as_array())
                .is_some_and(|a| !a.is_empty())
            || first
                .get("missingOnSrc")
                .and_then(|a| a.as_array())
                .is_some_and(|a| !a.is_empty())
            || first
                .get("error")
                .and_then(|a| a.as_array())
                .is_some_and(|a| !a.is_empty());
        if has_issues && !check_success {
            job.status = "failed".into();
            job.error = first
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or(job.error.clone());
        }
    }
    if let Some(obj) = job.output.as_object_mut() {
        obj.insert("cryptcheck".into(), parsed);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverviewJobStats {
    pub bytes: i64,
    pub total_bytes: i64,
    pub speed: f64,
    pub eta: f64,
    pub errors: i64,
    pub transfers: i64,
    pub total_transfers: i64,
    pub checks: i64,
    pub total_checks: i64,
    pub deletes: i64,
    pub renames: i64,
    pub server_side_copies: i64,
    pub server_side_moves: i64,
    pub last_error: String,
    pub active: usize,
}

impl OverviewJobStats {
    pub fn completion_pct(&self) -> f64 {
        if self.total_bytes > 0 {
            ((self.bytes as f64 / self.total_bytes as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        }
    }
}

pub fn overview_job_stats(jobs: &[JobInfo], core_stats: &Value) -> OverviewJobStats {
    OverviewJobStats {
        bytes: stats_i64(core_stats, &["bytes"]),
        total_bytes: stats_i64(core_stats, &["totalBytes"]),
        speed: stats_f64(core_stats, &["speed"]),
        eta: stats_f64(core_stats, &["eta"]),
        errors: stats_i64(core_stats, &["errors"]),
        transfers: stats_i64(core_stats, &["transfers"]),
        total_transfers: stats_i64(core_stats, &["totalTransfers"]),
        checks: stats_i64(core_stats, &["checks"]),
        total_checks: stats_i64(core_stats, &["totalChecks"]),
        deletes: stats_i64(core_stats, &["deletes"]),
        renames: stats_i64(core_stats, &["renames"]),
        server_side_copies: stats_i64(core_stats, &["serverSideCopies"]),
        server_side_moves: stats_i64(core_stats, &["serverSideMoves"]),
        last_error: core_stats
            .get("lastError")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        active: jobs
            .iter()
            .filter(|job| job_is_running(job) || job_is_pending(job))
            .count(),
    }
}

pub fn job_transfer_caption(job: &JobInfo) -> String {
    let bytes = stats_i64(&job.stats, &["bytes"]);
    let total = stats_i64(&job.stats, &["totalBytes"]);
    let speed = stats_f64(&job.stats, &["speed"]);
    let eta = stats_f64(&job.stats, &["eta"]);
    let size = if total > 0 {
        format!("{} / {}", format_bytes(bytes), format_bytes(total))
    } else {
        format_bytes(bytes)
    };
    let mut parts = vec![size];
    if speed > 0.0 {
        parts.push(format!("{}/s", format_bytes(speed.round() as i64)));
    }
    let eta_s = format_seconds(eta);
    if eta_s != "—" {
        parts.push(eta_s);
    }
    parts.join(" · ")
}

pub fn progress_from_stats(stats: &Value) -> f64 {
    let bytes = stats.get("bytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let total = stats
        .get("totalBytes")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    if total > 0.0 {
        (bytes / total).clamp(0.0, 1.0)
    } else {
        stats
            .get("transferring")
            .and_then(|x| x.as_array())
            .and_then(|arr| arr.first())
            .and_then(|item| {
                item.get("percentage")
                    .or_else(|| item.get("percentageComplete"))
            })
            .and_then(|x| x.as_f64())
            .map(|p| {
                if p > 1.0 {
                    (p / 100.0).clamp(0.0, 1.0)
                } else {
                    p.clamp(0.0, 1.0)
                }
            })
            .unwrap_or(0.0)
    }
}

fn infer_remote(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') {
        return None;
    }
    path.split_once(':').map(|(name, _)| name.to_string())
}

pub const BANDWIDTH_PRESETS: &[(&str, &str)] = &[
    ("off", "Unlimited"),
    ("512K", "512 KB/s"),
    ("1M", "1 MB/s"),
    ("5M", "5 MB/s"),
    ("10M", "10 MB/s"),
    ("50M", "50 MB/s"),
    ("10M:50M", "10M : 50M"),
];

pub fn normalize_bandwidth(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed == "0"
        || trimmed.eq_ignore_ascii_case("off:off")
    {
        "off".into()
    } else {
        trimmed.to_string()
    }
}

/// Normalize a custom bandwidth limit after Angular-style format validation.
pub fn validated_bandwidth_limit(value: &str) -> Result<String, String> {
    crate::validators::validate_bandwidth_limit(value)?;
    Ok(normalize_bandwidth(value))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BwLimitStatus {
    pub rate: String,
    pub bytes_per_sec: i64,
    pub bytes_per_sec_tx: i64,
    pub bytes_per_sec_rx: i64,
}

impl Default for BwLimitStatus {
    fn default() -> Self {
        Self {
            rate: "off".into(),
            bytes_per_sec: 0,
            bytes_per_sec_tx: 0,
            bytes_per_sec_rx: 0,
        }
    }
}

fn json_i64(value: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_u64().map(|n| n as i64))
                .or_else(|| v.as_f64().map(|n| n as i64))
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
}

pub fn parse_bwlimit(value: &Value) -> BwLimitStatus {
    let rate = value
        .get("rate")
        .and_then(|v| v.as_str())
        .map(normalize_bandwidth)
        .unwrap_or_else(|| "off".into());
    BwLimitStatus {
        rate,
        bytes_per_sec: json_i64(value, &["bytesPerSec", "bytesPerSecond"]),
        bytes_per_sec_tx: json_i64(value, &["bytesPerSecTx", "bytesPerSecondTx"]),
        bytes_per_sec_rx: json_i64(value, &["bytesPerSecRx", "bytesPerSecondRx"]),
    }
}

pub fn profile_src_dst(meta: &RemoteMeta, op: OperationType) -> (Option<String>, Option<String>) {
    let Some(profile) = meta.get_profile(op, "default") else {
        return (None, None);
    };
    let rclone = flatten_rclone(&profile.rclone);
    (
        first_path(&rclone, SOURCE_KEYS),
        first_path(&rclone, DEST_KEYS),
    )
}

pub fn merge_template_into(dest: &mut Value, values: &Value) {
    match (dest, values) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                merge_template_into(d.entry(k.clone()).or_insert(json!({})), v);
            }
        }
        (dest, src) => *dest = src.clone(),
    }
}

pub fn profile_summary(op: OperationType, profile: &ProfileConfig) -> String {
    let (src, dst) = quick_run_paths(&profile.rclone, op);
    let mut text = format!(
        "{} → {}",
        src.unwrap_or_else(|| "—".into()),
        dst.unwrap_or_else(|| "—".into())
    );
    if profile.app.cron_enabled && !profile.app.cron_expression.is_empty() {
        text.push_str(" · ");
        text.push_str(&crate::rclone::describe_cron(&profile.app.cron_expression));
    }
    if profile.app.watch_enabled {
        text.push_str(" · watch");
        if profile.app.watch_delay > 0 {
            text.push_str(&format!(" {}s", profile.app.watch_delay));
        }
    }
    text
}

pub fn selected_profile_key(remote: &str, op: OperationType) -> String {
    format!("{remote}:{}", op.as_str())
}

/// Profile a General-detail chip should start/stop — Angular `onToggleAction`.
pub fn chip_action_profile(
    remote: &str,
    op: OperationType,
    configured: &[String],
    jobs: &[JobInfo],
) -> String {
    if let Some(job) = jobs.iter().find(|job| {
        job_is_running(job)
            && job_belongs_to_remote(job, remote)
            && job_operation_matches(&job.operation, op)
    }) {
        if !job.profile.is_empty() {
            return job.profile.clone();
        }
    }
    configured
        .first()
        .cloned()
        .unwrap_or_else(|| "default".into())
}

/// Angular `enrichedProfiles` status: running, scheduled (cron/watch), or idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilePillStatus {
    Running,
    Scheduled,
    Idle,
}

pub fn profile_pill_status(
    active: bool,
    cron_enabled: bool,
    cron_expression: &str,
    watch_enabled: bool,
) -> ProfilePillStatus {
    if active {
        ProfilePillStatus::Running
    } else if watch_enabled || (cron_enabled && !cron_expression.is_empty()) {
        ProfilePillStatus::Scheduled
    } else {
        ProfilePillStatus::Idle
    }
}

pub fn profile_pill_has_watcher(
    cron_enabled: bool,
    cron_expression: &str,
    watch_enabled: bool,
) -> bool {
    watch_enabled && !(cron_enabled && !cron_expression.is_empty())
}

pub fn rename_serves_profile(
    serves: &mut [ServeItem],
    remote: &str,
    from: &str,
    to: &str,
) -> usize {
    if from.is_empty() || from == to {
        return 0;
    }
    let prefix = format!("{remote}:");
    let mut updated = 0;
    for serve in serves {
        let matches_remote =
            serve.fs == remote || serve.fs == prefix || serve.fs.starts_with(&prefix);
        if matches_remote && serve.profile == from {
            serve.profile = to.to_string();
            updated += 1;
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_check_reject_file_sources() {
        assert_eq!(
            directory_only_source_error(OperationType::Sync, "testdrive:a.txt", false).as_deref(),
            Some("Sync only supports directories, not files: testdrive:a.txt")
        );
        assert_eq!(
            directory_only_source_error(OperationType::Check, "/tmp/file.bin", false).as_deref(),
            Some("Check only supports directories, not files: /tmp/file.bin")
        );
        assert!(
            directory_only_source_error(OperationType::Sync, "testdrive:Photos", true).is_none()
        );
        assert!(
            directory_only_source_error(OperationType::Copy, "testdrive:a.txt", false).is_none()
        );
    }

    #[test]
    fn file_copy_uses_copyfile() {
        let req = build_job_params(
            OperationType::Copy,
            "testdrive",
            "testdrive:a.txt",
            "testdrive:verify-persist2/",
            &json!({}),
        )
        .unwrap();
        match req {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/copyfile");
                assert_eq!(params["srcFs"], "testdrive:");
                assert_eq!(params["srcRemote"], "a.txt");
                assert_eq!(params["dstFs"], "testdrive:");
                assert_eq!(params["dstRemote"], "verify-persist2/a.txt");
            }
            other => panic!("expected copyfile, got {other:?}"),
        }
        let moved = build_job_params(
            OperationType::Move,
            "src",
            "src:file.txt",
            "dst:backup/",
            &json!({}),
        )
        .unwrap();
        match moved {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/movefile");
                assert_eq!(params["dstRemote"], "backup/file.txt");
            }
            other => panic!("expected movefile, got {other:?}"),
        }
        let local = build_job_params(
            OperationType::Copy,
            "local",
            "/tmp/rclone-test-remote/a.txt",
            "testdrive:Inbox/",
            &json!({}),
        )
        .unwrap();
        match local {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/copyfile");
                assert_eq!(params["srcFs"], "/tmp/rclone-test-remote");
                assert_eq!(params["srcRemote"], "a.txt");
                assert_eq!(params["dstRemote"], "Inbox/a.txt");
            }
            other => panic!("expected local copyfile, got {other:?}"),
        }
        assert!(source_looks_like_file("testdrive:a.txt"));
        assert!(!source_looks_like_file("testdrive:Photos"));
        assert!(!source_looks_like_file("testdrive:"));
        assert_eq!(
            parse_transfer_fs("testdrive:a.txt"),
            Some(("testdrive:".into(), "a.txt".into()))
        );
    }

    #[test]
    fn directory_copy_keeps_sync_copy() {
        let req = build_job_params(
            OperationType::Copy,
            "testdrive",
            "testdrive:Photos",
            "testdrive:verify-photos/",
            &json!({}),
        )
        .unwrap();
        match req {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "sync/copy");
                assert_eq!(params["srcFs"], "testdrive:Photos");
                assert_eq!(params["dstFs"], "testdrive:verify-photos/");
            }
            other => panic!("expected sync/copy, got {other:?}"),
        }
        let delete_file = build_job_params(
            OperationType::Delete,
            "testdrive",
            "testdrive:a.txt",
            "",
            &json!({}),
        )
        .unwrap();
        match delete_file {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/deletefile");
                assert_eq!(params["fs"], "testdrive:");
                assert_eq!(params["remote"], "a.txt");
            }
            other => panic!("expected deletefile, got {other:?}"),
        }
    }

    #[test]
    fn builds_sync_params() {
        let req = build_job_params(
            OperationType::Sync,
            "drive",
            "drive:Photos",
            "/tmp/out",
            &json!({ "createEmptySrcDirs": true }),
        )
        .unwrap();
        match req {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "sync/sync");
                assert_eq!(params["srcFs"], "drive:Photos");
                assert_eq!(params["dstFs"], "/tmp/out");
                assert_eq!(params["createEmptySrcDirs"], true);
            }
            _ => panic!("expected async"),
        }
    }

    #[test]
    fn builds_bisync_and_copyurl() {
        let bi = build_job_params(
            OperationType::Bisync,
            "a",
            "a:",
            "b:",
            &json!({ "resync": true }),
        )
        .unwrap();
        match bi {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "sync/bisync");
                assert_eq!(params["path1"], "a:");
                assert_eq!(params["path2"], "b:");
            }
            _ => panic!(),
        }
        let cu = build_job_params(
            OperationType::Copyurl,
            "drive",
            "https://example.com/a.bin",
            "drive:Inbox",
            &json!({}),
        )
        .unwrap();
        match cu {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/copyurl");
                assert_eq!(params["url"], "https://example.com/a.bin");
                assert_eq!(params["fs"], "drive:Inbox");
                assert_eq!(params["autoFilename"], true);
            }
            _ => panic!(),
        }
        let named = build_job_params(
            OperationType::Copyurl,
            "drive",
            "https://example.com/a.bin",
            "drive:Inbox",
            &json!({ "filename": "saved.bin" }),
        )
        .unwrap();
        match named {
            JobRequest::Async { params, .. } => {
                assert_eq!(params["remote"], "saved.bin");
                assert_eq!(params["autoFilename"], false);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn builds_archive_create_rc_payload() {
        let req = build_job_params(
            OperationType::Archivecreate,
            "drive",
            "drive:Photos",
            "drive:Photos/out",
            &json!({
                "format": "zip",
                "prefix": "pack",
                "fullPath": true,
                "include": ["a.txt", "b"]
            }),
        )
        .unwrap();
        match req {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/archive");
                assert_eq!(params["action"], "create");
                assert_eq!(params["src"], "drive:Photos");
                assert_eq!(params["dst"], "drive:Photos/out/archive.zip");
                assert_eq!(params["format"], "zip");
                assert_eq!(params["prefix"], "pack");
                assert_eq!(params["full_path"], true);
                assert_eq!(params["include"], json!(["a.txt", "b"]));
            }
            _ => panic!("expected async archive create"),
        }
    }

    #[test]
    fn builds_cryptcheck_endpoint() {
        let req = build_job_params(
            OperationType::Cryptcheck,
            "crypt",
            "crypt:",
            "plain:",
            &json!({}),
        )
        .unwrap();
        match req {
            JobRequest::Async { endpoint, params } => {
                assert_eq!(endpoint, "operations/cryptcheck");
                assert_eq!(params["srcFs"], "crypt:");
                assert_eq!(params["dstFs"], "plain:");
            }
            _ => panic!("expected async cryptcheck"),
        }
    }

    #[test]
    fn rejects_incomplete_sync() {
        assert!(build_job_params(OperationType::Sync, "d", "", "/tmp", &json!({})).is_err());
        assert!(build_job_params(OperationType::Mount, "d", "d:", "", &json!({})).is_err());
    }

    #[test]
    fn archive_extension_and_paths() {
        assert!(has_archive_extension("out.tar.gz"));
        assert!(!has_archive_extension("out"));
        let rclone = json!({ "srcFs": "drive:a", "dstFs": "/tmp/b" });
        assert_eq!(default_source("drive", &rclone), "drive:a");
        assert_eq!(
            first_path(&json!({"source":["x","y"]}), SOURCE_KEYS).as_deref(),
            Some("x")
        );
    }

    #[test]
    fn parses_cli_flags() {
        let map = parse_cli_flags("--transfers 8 --vfs-cache-mode=full --dry-run");
        assert_eq!(map["transfers"], "8");
        assert_eq!(map["vfs_cache_mode"], "full");
        assert_eq!(map["dry_run"], true);
    }

    #[test]
    fn dry_run_and_flatten() {
        assert!(is_dry_run(&json!({ "dryRun": true })));
        assert!(!is_dry_run(&json!({})));
        assert!(is_resync(&json!({ "Resync": true })));
        assert!(is_resync(&json!({ "resync": true })));
        assert!(!is_resync(&json!({})));
        let nested = json!({ "sync": { "srcFs": "a:", "dstFs": "b:" } });
        assert_eq!(flatten_rclone(&nested)["srcFs"], "a:");
    }

    #[test]
    fn job_from_status_parses_stats() {
        let status = json!({
            "finished": false,
            "success": false,
            "duration": 12.5,
            "startTime": "2026-08-25T00:00:00Z",
            "group": "job/9",
            "output": {
                "operation": "sync",
                "srcFs": "drive:Photos",
                "dstFs": "/tmp/out",
                "dryRun": true
            }
        });
        let stats = json!({
            "bytes": 50,
            "totalBytes": 100,
            "speed": 1024,
            "transferring": [{ "name": "a.bin", "percentage": 50 }],
            "completed": [{ "name": "done.bin", "percentage": 100 }]
        });
        let job = job_from_status(9, &status, Some(&stats));
        assert_eq!(job.id, 9);
        assert_eq!(job.status, "running");
        assert_eq!(job.operation, "sync");
        assert_eq!(job.remote, "drive");
        assert_eq!(job.src, "drive:Photos");
        assert_eq!(job.dst, "/tmp/out");
        assert!(job.dry_run);
        assert_eq!(job.origin, "dashboard");
        assert!((job.progress - 0.5).abs() < f64::EPSILON);
        assert_eq!(job.transferring[0]["name"], "a.bin");
        assert_eq!(job.completed[0]["name"], "done.bin");
        assert_eq!(stats_i64(&job.stats, &["bytes"]), 50);
        assert!(is_managed_job(&job));
    }

    #[test]
    fn drops_internal_rc_job_list_noise() {
        let noise = job_from_status(
            99,
            &json!({
                "finished": true,
                "success": true,
                "group": "job/99",
                "output": {}
            }),
            None,
        );
        assert_eq!(noise.operation, "job/99");
        assert!(!is_managed_job(&noise));
        let running_noise = job_from_status(
            100,
            &json!({
                "finished": false,
                "group": "job/100",
                "output": {}
            }),
            None,
        );
        assert_eq!(running_noise.status, "running");
        assert!(!is_managed_job(&running_noise));
        let upload = preparing_job(3, "drive", "/tmp/a.txt", "drive:Inbox", 1, 12);
        assert!(is_managed_job(&upload));
        let started = started_operation_job(
            44,
            "copy",
            "testdrive",
            "gui-copy-test",
            "dashboard",
            "testdrive:a.txt",
            "testdrive:verify-gui-copy/",
        );
        assert!(is_managed_job(&started));
        assert_eq!(started.status, "starting");
        assert_eq!(started.operation, "copy");
        assert_eq!(started.src, "testdrive:a.txt");
        assert_eq!(
            select_job_ids(&[1, 2, 3], &[2], 48),
            vec![1, 2, 3],
            "small lists are fetched in full"
        );
        let huge: Vec<u64> = (1..=200).collect();
        assert_eq!(select_job_ids(&huge, &[7, 9, 400], 48), vec![7, 9]);
    }

    #[test]
    fn job_meta_assigns_execute_id_and_session_flags() {
        let meta = job_meta_for("drive", &ProfileConfig::default(), "dashboard", "local", "");
        assert!(!meta.execute_id.is_empty());
        assert_eq!(meta.origin, "dashboard");
        let mut rclone = json!({});
        apply_session_flags(&mut rclone, true, true);
        assert_eq!(rclone["DryRun"], true);
        assert_eq!(rclone["Resync"], true);
        assert_eq!(
            parse_job_end_time(&json!({"endTime": "2026-08-25T01:02:03Z"}))
                .unwrap()
                .to_rfc3339(),
            "2026-08-25T01:02:03+00:00"
        );
        assert!(stats_bool(
            &json!({"fatalError": true, "retryError": false}),
            &["fatalError"]
        ));
    }

    #[test]
    fn parse_started_ids_and_apply_registry() {
        assert_eq!(parse_started_ids("#12, #34"), vec![12, 34]);
        assert!(parse_started_ids("/mnt/drive").is_empty());
        let mut map = HashMap::new();
        remember_started(
            &mut map,
            "#9",
            JobMeta {
                origin: "flow".into(),
                profile: "photos".into(),
                remote: "drive".into(),
                backend: "extra".into(),
                quick_run_id: "qr-1".into(),
                execute_id: "exec-9".into(),
                parent_job_id: None,
                target: String::new(),
                ..Default::default()
            },
        );
        let mut job = running_job(9, "", "sync", "default");
        apply_job_meta(&mut job, map.get(&9));
        assert_eq!(job.origin, "flow");
        assert!(is_overview_job(&job));
        remember_grouped(
            &mut map,
            &[9, 10, 11],
            JobMeta {
                origin: "filemanager".into(),
                ..Default::default()
            },
        );
        assert_eq!(map.get(&10).and_then(|m| m.parent_job_id), Some(9));
        let mut child = running_job(10, "drive", "copy", "default");
        apply_job_meta(&mut child, map.get(&10));
        assert!(!is_overview_job(&child));
        assert_eq!(job.profile, "photos");
        assert_eq!(job.remote, "drive");
        map.insert(
            21,
            JobMeta {
                origin: "check-resolve".into(),
                target: "photo.jpg".into(),
                ..Default::default()
            },
        );
        let mut resolve = running_job(21, "drive", "copy", "default");
        resolve.origin = "check-resolve".into();
        resolve.src = "drive:album/photo.jpg".into();
        let jobs = vec![resolve];
        assert!(find_resolve_job(&jobs, &map, "photo.jpg").is_some());
        assert!(find_resolve_job(&jobs, &map, "other.jpg").is_none());
        assert!(path_has_leaf("drive:album/photo.jpg", "photo.jpg"));
        assert!(!path_has_leaf("drive:album/photo.jpg", "album"));
    }

    #[test]
    fn job_from_status_reads_origin() {
        let job = job_from_status(
            2,
            &json!({
                "finished": true,
                "success": true,
                "output": { "origin": "quick-run", "srcFs": "drive:" }
            }),
            None,
        );
        assert_eq!(job.origin, "quick-run");
        assert_eq!(job.status, "completed");
    }

    #[test]
    fn failed_job_status() {
        let job = job_from_status(
            1,
            &json!({ "finished": true, "success": false, "error": "boom" }),
            None,
        );
        assert_eq!(job.status, "failed");
        assert_eq!(job.error.as_deref(), Some("boom"));
    }

    #[test]
    fn bandwidth_off_aliases() {
        assert_eq!(normalize_bandwidth(""), "off");
        assert_eq!(normalize_bandwidth("OFF"), "off");
        assert_eq!(normalize_bandwidth("off:off"), "off");
        assert_eq!(normalize_bandwidth("10M"), "10M");
        assert_eq!(validated_bandwidth_limit("off").unwrap(), "off");
        assert_eq!(validated_bandwidth_limit("2M").unwrap(), "2M");
        assert!(validated_bandwidth_limit("xyz").is_err());
    }

    #[test]
    fn parses_core_bwlimit() {
        let status = parse_bwlimit(&json!({
            "rate": "1M:2M",
            "bytesPerSec": 1_572_864,
            "bytesPerSecTx": 1_048_576,
            "bytesPerSecRx": 2_097_152
        }));
        assert_eq!(status.rate, "1M:2M");
        assert_eq!(status.bytes_per_sec, 1_572_864);
        assert_eq!(status.bytes_per_sec_tx, 1_048_576);
        assert_eq!(status.bytes_per_sec_rx, 2_097_152);
        assert_eq!(parse_bwlimit(&json!({})).rate, "off");
        assert_eq!(parse_bwlimit(&json!({ "rate": "OFF" })).rate, "off");
    }

    #[test]
    fn reads_default_profile_paths() {
        let mut meta = crate::store::RemoteMeta::default();
        meta.upsert_profile(
            OperationType::Sync,
            ProfileConfig {
                name: "default".into(),
                app: crate::store::AppConfig::default(),
                rclone: json!({ "srcFs": "drive:photos", "dstFs": "/tmp/out" }),
            },
        );
        let (src, dst) = profile_src_dst(&meta, OperationType::Sync);
        assert_eq!(src.as_deref(), Some("drive:photos"));
        assert_eq!(dst.as_deref(), Some("/tmp/out"));
        assert_eq!(profile_src_dst(&meta, OperationType::Copy), (None, None));
    }

    fn running_job(id: u64, remote: &str, op: &str, profile: &str) -> JobInfo {
        JobInfo {
            id,
            operation: op.into(),
            remote: remote.into(),
            profile: profile.into(),
            status: "running".into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: format!("{remote}:src"),
            dst: "/tmp".into(),
            group: format!("job/{id}"),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        }
    }

    #[test]
    fn finds_and_matches_active_profiles() {
        let jobs = vec![
            running_job(1, "drive", "sync", "nightly"),
            running_job(2, "drive", "copy/move", "default"),
        ];
        assert!(job_operation_matches("sync/copy", OperationType::Sync));
        assert!(job_operation_matches("rc/copy", OperationType::Copy));
        assert!(!job_operation_matches("job/1", OperationType::Sync));
        assert_eq!(
            action_busy_key("drive", "sync", "nightly"),
            "drive|sync|nightly"
        );
        let mut preparing = running_job(3, "drive", "sync", "nightly");
        preparing.status = "preparing".into();
        assert!(job_is_pending(&preparing));
        assert!(!job_is_pending(&jobs[0]));
        assert!(action_in_progress(
            "drive",
            OperationType::Sync,
            "nightly",
            &[preparing],
            false
        ));
        assert!(action_in_progress(
            "drive",
            OperationType::Sync,
            "nightly",
            &[],
            true
        ));
        assert!(!action_in_progress(
            "drive",
            OperationType::Sync,
            "nightly",
            &jobs,
            false
        ));
        assert_eq!(
            origin_label_key("quick-run"),
            "generalOverview.jobs.originQuickRun"
        );
        assert_eq!(origin_label_key(""), "generalOverview.jobs.originDashboard");
        assert_eq!(
            origin_label_key("filemanager"),
            "generalOverview.jobs.originFiles"
        );
        assert!(allows_unconfigured_start(OperationType::Mount));
        assert!(allows_unconfigured_start(OperationType::Serve));
        assert!(!allows_unconfigured_start(OperationType::Sync));
        assert!(!allows_unconfigured_start(OperationType::Copy));
        assert_eq!(
            find_active_job(&jobs, "drive", OperationType::Sync, "nightly").map(|j| j.id),
            Some(1)
        );
        assert!(find_active_job(&jobs, "drive", OperationType::Sync, "missing").is_none());
        let mounts = vec![MountedRemote::new("drive:photos", "/mnt/drive")];
        assert!(find_active_mount(&mounts, "drive").is_some());
        assert!(find_active_mount(&mounts, "dropbox").is_none());
        let alias_mounts = vec![MountedRemote::new(
            "/tmp/rclone-test-remote",
            "/home/ubuntu/rclone-manager/testdrive",
        )];
        assert!(find_active_mount(&alias_mounts, "testdrive").is_some());
        assert!(
            find_active_mount_for(&alias_mounts, "testdrive", "/tmp/rclone-test-remote").is_some()
        );
        assert!(find_active_mount(&alias_mounts, "dummyexport").is_none());
        let default_point = vec![MountedRemote::new(
            "/tmp/rclone-test-remote",
            "/tmp/rclone-testdrive-mnt",
        )];
        assert!(find_active_mount(&default_point, "testdrive").is_none());
        assert!(
            find_active_mount_for(&default_point, "testdrive", "/tmp/rclone-test-remote").is_some()
        );
        assert_eq!(
            resolve_unmount_point_in(
                &default_point,
                "testdrive",
                "/tmp/rclone-test-remote",
                &[],
                &[]
            )
            .as_deref(),
            Some("/tmp/rclone-testdrive-mnt")
        );
        assert_eq!(
            resolve_unmount_point_in(
                &[],
                "testdrive",
                "/tmp/rclone-test-remote",
                &["/tmp/rclone-testdrive-mnt".into()],
                &default_point
            )
            .as_deref(),
            Some("/tmp/rclone-testdrive-mnt")
        );
        assert!(resolve_unmount_point_in(
            &[],
            "testdrive",
            "",
            &["/tmp/rclone-testdrive-mnt".into()],
            &[]
        )
        .is_none());
        let profile = ProfileConfig {
            name: "default".into(),
            rclone: json!({ "mountPoint": "/tmp/rclone-testdrive-mnt" }),
            ..ProfileConfig::default()
        };
        assert!(mount_unmount_fallbacks("testdrive", Some(&profile))
            .iter()
            .any(|point| point == "/tmp/rclone-testdrive-mnt"));
        let serves = vec![ServeItem {
            id: "abc".into(),
            addr: "127.0.0.1:8080".into(),
            fs: "drive:".into(),
            serve_type: "webdav".into(),
            origin: "dashboard".into(),
            profile: "default".into(),
            option_count: 0,
        }];
        assert!(profile_is_active(
            "drive",
            OperationType::Serve,
            "default",
            &[],
            &[],
            &serves
        ));
        assert!(profile_is_active(
            "drive",
            OperationType::Mount,
            "default",
            &[],
            &mounts,
            &[]
        ));
        let mut qr = QuickRun::new("Nightly".into(), OperationType::Sync, "drive".into());
        qr.last_job_id = Some(1);
        let mut job = running_job(1, "drive", "sync", "default");
        job.origin = "quick-run".into();
        assert!(find_active_quick_run(&[job], &qr).is_some());
        let usage = profile_usage(
            &jobs,
            &mounts,
            &serves,
            "drive",
            "nightly",
            Some(OperationType::Sync),
            "",
        );
        assert!(usage.blocked());
        assert_eq!(usage.jobs, 1);
        let idle = profile_usage(
            &[],
            &[],
            &[],
            "drive",
            "default",
            Some(OperationType::Sync),
            "",
        );
        assert!(!idle.blocked());
    }

    #[test]
    fn merges_core_transferred_into_stats() {
        let mut stats = json!({ "bytes": 12 });
        merge_completed_transfers(&mut stats, &json!({ "transferred": [{ "name": "a.bin" }] }));
        assert_eq!(stats["completed"][0]["name"], "a.bin");
        assert_eq!(stats["bytes"], 12);
        let mut empty = json!(null);
        merge_completed_transfers(&mut empty, &json!([{ "name": "b" }]));
        assert_eq!(empty["completed"][0]["name"], "b");
    }

    #[test]
    fn merges_named_helper_profiles() {
        let mut meta = crate::store::RemoteMeta::default();
        meta.vfs_configs
            .insert("fast".into(), json!({ "CacheMode": "full" }));
        meta.filter_configs
            .insert("docs".into(), json!({ "IncludeRule": ["*.md"] }));
        let profile = ProfileConfig {
            name: "default".into(),
            app: crate::store::AppConfig {
                vfs_profile: "fast".into(),
                filter_profile: "docs".into(),
                ..Default::default()
            },
            rclone: json!({ "srcFs": "drive:a", "dstFs": "/tmp" }),
        };
        let mut rclone = flatten_rclone(&profile.rclone);
        apply_helper_options(&mut rclone, &profile, Some(&meta));
        assert_eq!(rclone["vfsOpt"]["CacheMode"], "full");
        assert_eq!(rclone["_filter"]["IncludeRule"][0], "*.md");
    }

    #[test]
    fn merges_inline_runtime_remote_without_clobbering() {
        let mut meta = crate::store::RemoteMeta::default();
        meta.runtime_remote_configs.insert(
            "live".into(),
            json!({ "token": "named", "chunk_size": "8M" }),
        );
        let profile = ProfileConfig {
            name: "default".into(),
            app: crate::store::AppConfig {
                runtime_remote_profile: "live".into(),
                ..Default::default()
            },
            rclone: json!({
                "srcFs": "drive:a",
                "dstFs": "/tmp",
                "runtimeRemote": { "token": "inline", "drive_id": "abc" }
            }),
        };
        let mut rclone = flatten_rclone(&profile.rclone);
        apply_helper_options(&mut rclone, &profile, Some(&meta));
        assert_eq!(rclone["token"], "named");
        assert_eq!(rclone["chunk_size"], "8M");
        assert_eq!(rclone["drive_id"], "abc");
        assert!(rclone.get("runtimeRemote").is_none());
    }

    #[test]
    fn reads_multi_source_arrays() {
        let rclone = json!({ "srcFs": ["drive:a", "drive:b"], "dstFs": "/tmp" });
        assert_eq!(path_list(&rclone, SOURCE_KEYS), vec!["drive:a", "drive:b"]);
        assert!(OperationType::Sync.supports_multi_source());
        assert!(!OperationType::Mount.supports_multi_source());
    }

    #[test]
    fn assembles_profile_rclone() {
        let sync = assemble_rclone(
            OperationType::Sync,
            &["drive:a".into(), "drive:b".into()],
            "/tmp/out",
            Map::from_iter([("createEmptySrcDirs".into(), json!(true))]),
        );
        assert_eq!(sync["srcFs"], json!(["drive:a", "drive:b"]));
        assert_eq!(sync["dstFs"], "/tmp/out");
        assert_eq!(sync["createEmptySrcDirs"], true);
        let mount = assemble_rclone(
            OperationType::Mount,
            &["drive:".into()],
            "/mnt/drive",
            Map::new(),
        );
        assert_eq!(mount["mountPoint"], "/mnt/drive");
        assert_eq!(mount["srcFs"], "drive:");
        let serve = assemble_rclone(
            OperationType::Serve,
            &["drive:pub".into()],
            "127.0.0.1:8080",
            Map::from_iter([("type".into(), json!("webdav"))]),
        );
        assert_eq!(serve["addr"], "127.0.0.1:8080");
        assert_eq!(serve["type"], "webdav");
        let copyurl = assemble_rclone(
            OperationType::Copyurl,
            &[
                "https://example.com/a.txt".into(),
                "https://example.com/b.txt".into(),
            ],
            "drive:Inbox",
            Map::new(),
        );
        assert_eq!(
            copyurl["url"],
            json!(["https://example.com/a.txt", "https://example.com/b.txt"])
        );
        assert_eq!(
            path_list(&copyurl, &["url", "srcFs", "source"]),
            vec![
                "https://example.com/a.txt".to_string(),
                "https://example.com/b.txt".into()
            ]
        );
        let empty = assemble_rclone(OperationType::Delete, &[], "", Map::new());
        assert!(empty.as_object().unwrap().is_empty());
    }

    #[test]
    fn parse_cryptcheck_output_success() {
        let raw = "2026/08/20 10:00:00 NOTICE: Encrypted drive 'enc:': 0 differences found\n";
        let parsed = parse_cryptcheck_output(raw);
        let res = &parsed["results"][0];
        assert_eq!(res["success"], true);
        assert_eq!(res["status"], "OK");
        assert!(res["differ"].as_array().unwrap().is_empty());
    }

    #[test]
    fn parse_cryptcheck_output_with_differences() {
        let raw = r#"
2026/08/20 10:00:00 ERROR : file1.txt: MD5 differ
2026/08/20 10:00:00 ERROR : file2.txt: file not in Encrypted drive
2026/08/20 10:00:00 NOTICE: Encrypted drive 'enc:': 1 differences found
"#;
        let parsed = parse_cryptcheck_output(raw);
        let res = &parsed["results"][0];
        assert_eq!(res["success"], false);
        assert_eq!(
            res["differ"].as_array().unwrap(),
            &vec![serde_json::json!("file1.txt")]
        );
        assert_eq!(
            res["missingOnDst"].as_array().unwrap(),
            &vec![serde_json::json!("file2.txt")]
        );
    }

    #[test]
    fn job_from_status_applies_cryptcheck_failure() {
        let job = job_from_status(
            3,
            &json!({
                "finished": true,
                "success": true,
                "output": {
                    "operation": "cryptcheck",
                    "result": "2026/08/20 10:00:00 ERROR : file1.txt: MD5 differ\n2026/08/20 10:00:00 NOTICE: Encrypted drive 'enc:': 1 differences found\n"
                }
            }),
            None,
        );
        assert_eq!(job.status, "failed");
        assert!(job.error.as_deref().is_some_and(|e| !e.is_empty()));
        assert_eq!(job.output["cryptcheck"]["results"][0]["success"], false);
    }

    #[test]
    fn merges_nested_templates() {
        let mut dest = json!({ "main": { "transfers": 4 }, "keep": true });
        merge_template_into(
            &mut dest,
            &json!({ "main": { "transfers": 8, "checkers": 2 } }),
        );
        assert_eq!(dest["main"]["transfers"], 8);
        assert_eq!(dest["main"]["checkers"], 2);
        assert_eq!(dest["keep"], true);
    }

    #[test]
    fn jobs_from_transfer_start_use_snapshot_paths() {
        let jobs = jobs_from_transfer_start(
            &[7, 8],
            "copy",
            "testdrive",
            "filemanager",
            "filemanager/abc",
            &json!([
                { "src": "testdrive:Photos", "dst": "testdrive:verify/Photos" },
                { "src": "testdrive:a.txt", "dst": "testdrive:verify/a.txt" }
            ]),
        );
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].id, 7);
        assert_eq!(jobs[0].src, "testdrive:Photos");
        assert_eq!(jobs[0].dst, "testdrive:verify/Photos");
        assert_eq!(jobs[0].group, "filemanager/abc");
        assert_eq!(jobs[0].origin, "filemanager");
        assert_eq!(jobs[0].status, "starting");
        assert_eq!(jobs[1].src, "testdrive:a.txt");
        assert!(
            jobs_from_transfer_start(&[1], "copy", "x", "filemanager", "", &json!([])).len() == 1
        );
    }

    #[test]
    fn formats_elapsed_and_eta_seconds() {
        assert_eq!(format_seconds(0.0), "—");
        assert_eq!(format_seconds(-1.0), "—");
        assert_eq!(format_seconds(9.4), "9s");
        assert_eq!(format_seconds(75.0), "1m 15s");
        assert_eq!(format_seconds(3723.0), "1h 2m 3s");
    }

    #[test]
    fn preparing_job_and_merge() {
        let preparing = preparing_job(9, "drive", "/tmp/a.txt", "Inbox/a.txt", 1, 32);
        assert_eq!(preparing.status, "preparing");
        assert_eq!(preparing.stats["preparing"], true);
        assert_eq!(preparing.stats["totalBytes"], 32);
        let live_stats = preparing_progress_stats(
            16,
            32,
            0,
            1,
            json!([{ "name": "a.txt", "bytes": 16, "size": 32 }]),
        );
        assert_eq!(live_stats["bytes"], 16);
        assert_eq!(live_stats["preparing"], true);
        let live = vec![running_job(1, "drive", "sync", "nightly")];
        let merged = merge_preparing_jobs(live.clone(), &[preparing.clone()]);
        assert_eq!(merged[0].id, 9);
        assert_eq!(merged.len(), 2);
        let already = merge_preparing_jobs(
            vec![JobInfo {
                status: "running".into(),
                ..preparing.clone()
            }],
            &[preparing.clone()],
        );
        assert_eq!(already.len(), 1);
        assert_eq!(already[0].status, "running");
        let mut stale = preparing.clone();
        stale.start_time = Utc::now() - chrono::Duration::seconds(PREPARING_TTL_SECS + 5);
        let expired = merge_preparing_jobs(live.clone(), &[stale]);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, 1);
        let mut grouped = preparing.clone();
        grouped.group = "filemanager-upload/abc".into();
        let mut live_group = running_job(2, "drive", "upload", "default");
        live_group.group = "filemanager-upload/abc".into();
        let skipped = merge_preparing_jobs(vec![live_group], &[grouped]);
        assert_eq!(skipped.len(), 1);
        let dropped = finalize_dropped_job(&preparing);
        assert_eq!(dropped.status, "completed");
        assert!(dropped.error.is_none());
        let from_stats = job_from_status(
            4,
            &json!({ "finished": false, "success": false, "output": { "operation": "upload" } }),
            Some(&json!({ "preparing": true, "bytes": 0 })),
        );
        assert_eq!(from_stats.status, "preparing");
    }

    #[test]
    fn hydrates_grouped_transfer_rows_from_snapshot_and_children() {
        let snapshot = json!([
            { "name": "a.txt", "src": "/tmp/a.txt", "dst": "Inbox/a.txt", "size": 4, "bytes": 0 },
            { "name": "b.txt", "src": "/tmp/b.txt", "dst": "Inbox/b.txt", "size": 4, "bytes": 0 }
        ]);
        let mut map = HashMap::new();
        remember_grouped(
            &mut map,
            &[20, 21],
            JobMeta {
                origin: "filemanager".into(),
                group: "filemanager-upload/abc".into(),
                transfer_snapshot: snapshot,
                ..Default::default()
            },
        );
        let mut parent = preparing_job(20, "testdrive", "/tmp/a.txt", "Inbox", 2, 8);
        parent.transferring = json!([]);
        parent.completed = json!([]);
        let mut child = running_job(21, "testdrive", "upload", "default");
        child.src = "/tmp/b.txt".into();
        child.dst = "Inbox/b.txt".into();
        child.status = "completed".into();
        child.parent_job_id = Some(20);
        let mut jobs = vec![parent, child];
        apply_job_meta(&mut jobs[0], map.get(&20));
        apply_job_meta(&mut jobs[1], map.get(&21));
        hydrate_grouped_transfers(&mut jobs, &map);
        assert_eq!(jobs[0].transferring.as_array().unwrap().len(), 1);
        assert_eq!(jobs[0].completed.as_array().unwrap().len(), 1);
        assert_eq!(jobs[0].completed[0]["name"], "b.txt");
        assert_eq!(jobs[0].transferring[0]["name"], "a.txt");
        assert_eq!(jobs[0].group, "filemanager-upload/abc");
        let mut finished = preparing_job(20, "testdrive", "/tmp/a.txt", "Inbox", 2, 8);
        finished.status = "completed".into();
        finished.transferring = json!([]);
        finished.completed = json!([]);
        decorate_job_transfers(&mut finished, &map, &[]);
        assert_eq!(finished.completed.as_array().unwrap().len(), 2);
        assert!(finished.transferring.as_array().unwrap().is_empty());
        assert_eq!(finished.completed[0]["bytes"], 4);
        assert_eq!(finished.completed[1]["bytes"], 4);
        let mut stale = finished.clone();
        stale.completed = json!([
            { "name": "a.txt", "src": "/tmp/a.txt", "dst": "Inbox/a.txt", "size": 4, "bytes": 0 },
            { "name": "b.txt", "src": "/tmp/b.txt", "dst": "Inbox/b.txt", "size": 4, "bytes": 0 }
        ]);
        hydrate_grouped_transfers(&mut [stale.clone()], &map);
        decorate_job_transfers(&mut stale, &map, &[]);
        assert_eq!(stale.completed[0]["bytes"], 4);
        assert_eq!(stale.completed[1]["percentage"], 100);
    }

    #[test]
    fn snapshot_emits_rclone_src_dst_fields() {
        let items = [crate::fileops::TransferItem {
            src_fs: "/".into(),
            src: "/tmp/gtk-upload-test/e.txt".into(),
            dst_fs: "testdrive:".into(),
            dst: "e.txt".into(),
            cut: false,
            is_dir: false,
        }];
        let snapshot = transfer_snapshot_from_items(&items);
        let row = crate::transfers::parse_transfer_row(&snapshot[0]);
        assert_eq!(snapshot[0]["srcFs"], "/");
        assert_eq!(snapshot[0]["srcRemote"], "/tmp/gtk-upload-test/e.txt");
        assert_eq!(snapshot[0]["dstFs"], "testdrive:");
        assert_eq!(snapshot[0]["dstRemote"], "e.txt");
        assert_eq!(row.src, "/tmp/gtk-upload-test/e.txt");
        assert_eq!(row.dst, "testdrive:e.txt");
        assert_ne!(row.dst, "e.txt/e.txt");
    }

    #[test]
    fn profile_summary_includes_cron_and_watch() {
        let profile = ProfileConfig {
            name: "nightly".into(),
            app: crate::store::AppConfig {
                cron_enabled: true,
                cron_expression: "0 2 * * *".into(),
                watch_enabled: true,
                watch_delay: 12,
                ..crate::store::AppConfig::default()
            },
            rclone: json!({ "srcFs": "drive:", "dstFs": "/tmp/out" }),
        };
        let summary = profile_summary(OperationType::Sync, &profile);
        assert!(summary.contains("drive:"));
        assert!(summary.contains("/tmp/out"));
        assert!(summary.contains("watch 12s"));
        assert!(!summary.contains("0 2 * * *") || summary.contains("2"));
    }

    #[test]
    fn renames_serve_profiles_for_remote() {
        let mut serves = vec![
            ServeItem {
                id: "s1".into(),
                fs: "drive:".into(),
                profile: "public".into(),
                ..ServeItem::default()
            },
            ServeItem {
                id: "s2".into(),
                fs: "box:".into(),
                profile: "public".into(),
                ..ServeItem::default()
            },
        ];
        assert_eq!(
            rename_serves_profile(&mut serves, "drive", "public", "web"),
            1
        );
        assert_eq!(serves[0].profile, "web");
        assert_eq!(serves[1].profile, "public");
    }

    #[test]
    fn renames_mount_profiles_for_remote() {
        let mut mounts = vec![
            MountedRemote {
                fs: "drive:".into(),
                mount_point: "/mnt/drive".into(),
                profile: "home".into(),
                ..MountedRemote::default()
            },
            MountedRemote {
                fs: "box:".into(),
                mount_point: "/mnt/box".into(),
                profile: "home".into(),
                ..MountedRemote::default()
            },
        ];
        assert_eq!(
            rename_mounts_profile(&mut mounts, "drive", "home", "desk"),
            1
        );
        assert_eq!(mounts[0].profile, "desk");
        assert_eq!(mounts[1].profile, "home");
        assert_eq!(
            rename_mounts_profile(&mut mounts, "drive", "home", "desk"),
            0
        );
        let busy = ProfileUsage {
            jobs: 1,
            ..ProfileUsage::default()
        };
        assert!(profile_rename_blocked(Some(OperationType::Copy), &busy));
        assert!(!profile_rename_blocked(Some(OperationType::Mount), &busy));
        assert!(!profile_rename_blocked(None, &busy));
        assert!(!profile_rename_blocked(
            Some(OperationType::Copy),
            &ProfileUsage::default()
        ));
        assert_eq!(activity_visible_end(13, 12), 12);
        assert_eq!(activity_remaining(13, 12), 1);
        assert_eq!(activity_remaining(12, 50), 0);
        assert_eq!(activity_visible_end(80, ACTIVITY_PAGE), 50);
        let alias_mounts = [MountedRemote {
            fs: "/tmp/rclone-test-remote".into(),
            mount_point: "/tmp/rclone-testdrive-mnt".into(),
            profile: "default".into(),
            ..MountedRemote::default()
        }];
        assert!(profile_usage(
            &[],
            &alias_mounts,
            &[],
            "testdrive",
            "default",
            Some(OperationType::Mount),
            "/tmp/rclone-test-remote",
        )
        .blocked());
        assert!(!profile_usage(
            &[],
            &alias_mounts,
            &[],
            "testdrive",
            "default",
            Some(OperationType::Mount),
            "",
        )
        .blocked());
    }

    #[test]
    fn prefers_default_mount_profile() {
        let mut meta = RemoteMeta::default();
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "home".into(),
                rclone: json!({ "mountPoint": "/mnt/home" }),
                ..ProfileConfig::default()
            },
        );
        meta.upsert_profile(
            OperationType::Mount,
            ProfileConfig {
                name: "default".into(),
                rclone: json!({ "mountPoint": "/mnt/drive" }),
                ..ProfileConfig::default()
            },
        );
        let profile = preferred_mount_profile(Some(&meta)).unwrap();
        assert_eq!(profile.name, "default");
        assert_eq!(profile.rclone["mountPoint"], "/mnt/drive");
        assert!(preferred_mount_profile(None).is_none());
        assert!(preferred_mount_profile(Some(&RemoteMeta::default())).is_none());
        assert_eq!(preferred_mount_profile_name(Some(&meta)), "default");
        assert_eq!(preferred_mount_profile_name(None), "default");
    }

    #[test]
    fn active_open_paths_match_angular_blossom() {
        assert!(
            active_open_paths(OperationType::Serve, "drive:", "http://127.0.0.1", None).is_empty()
        );
        assert_eq!(
            active_open_paths(
                OperationType::Mount,
                "drive:",
                "/mnt/unused",
                Some("/mnt/drive")
            ),
            vec!["/mnt/drive".to_string()]
        );
        assert_eq!(
            active_open_paths(OperationType::Mount, "drive:", "/mnt/drive", None),
            vec!["/mnt/drive".to_string()]
        );
        assert_eq!(
            active_open_paths(OperationType::Sync, "drive:Photos", "/tmp/out", None),
            vec!["drive:Photos".to_string(), "/tmp/out".to_string()]
        );
        assert_eq!(
            active_open_paths(OperationType::Copy, "drive:", "drive:", None),
            vec!["drive:".to_string()]
        );
        assert!(active_open_paths(OperationType::Sync, "", "—", None).is_empty());
        assert_eq!(
            active_open_paths(OperationType::Copy, "drive:a, drive:b", "box:out", None),
            vec![
                "drive:a".to_string(),
                "drive:b".to_string(),
                "box:out".to_string()
            ]
        );
        let mut opening = std::collections::HashSet::new();
        assert!(begin_folder_open(&mut opening, "testdrive"));
        assert!(is_folder_opening(&opening, "testdrive"));
        assert!(!begin_folder_open(&mut opening, "testdrive"));
        end_folder_open(&mut opening, "testdrive");
        assert!(!is_folder_opening(&opening, "testdrive"));
        assert!(!begin_folder_open(&mut opening, ""));
        assert_eq!(
            split_job_paths("drive:a, drive:b, /tmp/out"),
            vec!["drive:a", "drive:b", "/tmp/out"]
        );
        assert_eq!(split_job_paths("drive:Photos"), vec!["drive:Photos"]);
        assert!(split_job_paths("—").is_empty());
        assert_eq!(
            split_job_paths(r#"["drive:a","drive:b"]"#),
            vec!["drive:a", "drive:b"]
        );
        let overflow = overflow_active_ops(
            &[OperationType::Mount, OperationType::Sync],
            &[
                OperationType::Mount,
                OperationType::Sync,
                OperationType::Serve,
            ],
        );
        assert_eq!(overflow, vec![OperationType::Serve]);
        assert!(active_remote_ops("drive", false, false, &[]).is_empty());
        assert_eq!(
            active_remote_ops("drive", true, true, &[]),
            vec![OperationType::Mount, OperationType::Serve]
        );
    }

    #[test]
    fn fs_and_activity_counts_match_remote() {
        assert!(fs_belongs_to_remote("drive:", "drive"));
        assert!(fs_belongs_to_remote("drive:Photos", "drive"));
        assert!(fs_belongs_to_remote("drive", "drive"));
        assert!(!fs_belongs_to_remote("other:", "drive"));
        let mounts = vec![
            crate::rclone::MountedRemote::new("drive:", "/mnt/a"),
            crate::rclone::MountedRemote::new("drive:share", "/mnt/b"),
        ];
        let serves = vec![crate::rclone::ServeItem {
            id: "1".into(),
            addr: "127.0.0.1:8080".into(),
            fs: "drive:web".into(),
            serve_type: "http".into(),
            origin: "dashboard".into(),
            profile: "web".into(),
            option_count: 0,
        }];
        let jobs = vec![running_job(1, "drive", "copy", "nightly")];
        assert_eq!(
            remote_activity_counts("drive", &mounts, &serves, &jobs),
            (2, 1, 1)
        );
        assert_eq!(
            remote_activity_counts("other", &mounts, &serves, &jobs),
            (0, 0, 0)
        );
    }

    #[test]
    fn origin_filter_matches_angular_chips() {
        assert!(origin_matches("quickrun", "all"));
        assert!(origin_matches("flow", "quickrun"));
        assert!(origin_matches("quick-run", "quickrun"));
        assert!(origin_matches("", "dashboard"));
        assert!(origin_matches("dashboard", "dashboard"));
        assert!(!origin_matches("filemanager", "dashboard"));
        assert!(origin_matches("files", "filemanager"));
        assert!(origin_matches("automation", "automation"));
        assert_eq!(automation_origin("quick:abc"), "quickrun");
        assert_eq!(automation_origin("remote:drive:sync:default"), "dashboard");
        assert!(automation_matches_filter("quick:abc", "automation"));
        assert!(automation_matches_filter(
            "remote:drive:sync:default",
            "all"
        ));
        assert!(automation_matches_filter("quick:abc", "quickrun"));
        assert!(!automation_matches_filter("quick:abc", "dashboard"));
    }

    #[test]
    fn shutdown_summary_counts_running_only() {
        let jobs = vec![
            running_job(1, "drive", "sync", "nightly"),
            JobInfo {
                status: "completed".into(),
                ..running_job(2, "drive", "copy", "nightly")
            },
        ];
        let summary = shutdown_summary(&jobs, 2, 1);
        assert!(summary.active());
        assert_eq!(summary.jobs, 1);
        assert_eq!(summary.mounts, 2);
        assert_eq!(summary.serves, 1);
        assert!(!shutdown_summary(&[], 0, 0).active());
    }

    #[test]
    fn renames_live_job_profiles() {
        let mut jobs = vec![
            running_job(1, "drive", "sync", "nightly"),
            running_job(2, "other", "sync", "nightly"),
        ];
        assert_eq!(
            rename_jobs_profile(&mut jobs, "drive", "nightly", "weekly"),
            1
        );
        assert_eq!(jobs[0].profile, "weekly");
        assert_eq!(jobs[1].profile, "nightly");
    }

    #[test]
    fn builds_serve_from_profile_type_and_addr() {
        let req = build_job_params(
            OperationType::Serve,
            "testdrive",
            "testdrive:pub",
            "127.0.0.1:0",
            &json!({
                "type": "http",
                "addr": "127.0.0.1:18080",
                "readOnly": true,
                "origin": "dashboard"
            }),
        )
        .unwrap();
        match req {
            JobRequest::Serve {
                serve_type,
                fs,
                addr,
                extra,
            } => {
                assert_eq!(serve_type, "http");
                assert_eq!(fs, "testdrive:pub");
                assert_eq!(addr, "127.0.0.1:18080");
                assert_eq!(extra["readOnly"], true);
                assert_eq!(extra["origin"], "dashboard");
            }
            other => panic!("expected serve, got {other:?}"),
        }
        let fallback =
            build_job_params(OperationType::Serve, "box", "", "10.0.0.2:9000", &json!({})).unwrap();
        match fallback {
            JobRequest::Serve {
                serve_type,
                fs,
                addr,
                ..
            } => {
                assert_eq!(serve_type, "http");
                assert_eq!(fs, "box:");
                assert_eq!(addr, "10.0.0.2:9000");
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn overview_job_stats_read_core_stats() {
        let jobs = vec![
            running_job(1, "drive", "sync", "default"),
            JobInfo {
                status: "completed".into(),
                ..running_job(2, "drive", "copy", "default")
            },
        ];
        let stats = overview_job_stats(
            &jobs,
            &json!({
                "bytes": 50,
                "totalBytes": 200,
                "speed": 1024.0,
                "eta": 12,
                "errors": 1,
                "transfers": 2,
                "totalTransfers": 4,
                "checks": 1,
                "totalChecks": 3,
                "deletes": 0,
                "renames": 1,
                "serverSideCopies": 2,
                "serverSideMoves": 0,
                "lastError": "boom"
            }),
        );
        assert_eq!(stats.active, 1);
        assert_eq!(stats.completion_pct(), 25.0);
        assert_eq!(stats.last_error, "boom");
        assert_eq!(stats.server_side_copies, 2);
        let mut job = running_job(3, "drive", "sync", "default");
        job.stats = json!({ "bytes": 1024, "totalBytes": 2048, "speed": 512.0, "eta": 8 });
        let caption = job_transfer_caption(&job);
        assert!(caption.contains("KiB"));
        assert!(caption.contains("/s"));
    }

    #[test]
    fn finds_jobs_and_merges_history() {
        let mut live = running_job(1, "drive", "sync", "nightly");
        live.start_time = DateTime::parse_from_rfc3339("2026-08-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut finished = running_job(2, "drive", "copy", "default");
        finished.status = "completed".into();
        finished.start_time = DateTime::parse_from_rfc3339("2026-08-25T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let other = running_job(3, "box", "sync", "default");
        let child = JobInfo {
            parent_job_id: Some(1),
            ..running_job(4, "drive", "copy", "nightly")
        };
        let history = vec![finished.clone(), other.clone(), child.clone()];
        assert_eq!(find_job_by_id(&[live.clone()], &history, 1).unwrap().id, 1);
        assert_eq!(
            find_job_by_id(&[], &history, 2).unwrap().status,
            "completed"
        );
        assert!(find_job_by_id(&[], &history, 99).is_none());
        let merged = merge_overview_jobs(&[live.clone()], &history, "drive", None, None);
        assert_eq!(
            merged.iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        let mut weekly = running_job(5, "drive", "sync", "weekly");
        weekly.status = "completed".into();
        weekly.start_time = DateTime::parse_from_rfc3339("2026-08-25T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let history = vec![finished.clone(), other, child, weekly];
        let nightly =
            merge_overview_jobs(&[live.clone()], &history, "drive", Some("nightly"), None);
        let sync_only = merge_overview_jobs(
            &[live.clone()],
            &history,
            "drive",
            None,
            Some(OperationType::Sync),
        );
        assert_eq!(
            sync_only.iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![1, 5]
        );
        assert!(!sync_only.iter().any(|job| job.id == 2));
        let check_only = merge_overview_jobs(
            &[live.clone()],
            &history,
            "drive",
            Some("default"),
            Some(OperationType::Check),
        );
        assert!(check_only.is_empty());
        assert_eq!(
            nightly.iter().map(|job| job.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        let siblings = merge_job_lists(&[live.clone()], &history);
        assert_eq!(siblings.len(), 5);
        assert!(!nightly.iter().any(|job| job.id == 5));
        let status = job_status_value(&finished);
        assert_eq!(status["finished"], true);
        assert_eq!(status["success"], true);
        assert_eq!(status["group"], "job/2");
        let mut meta = HashMap::new();
        meta.insert(
            186,
            JobMeta {
                origin: "filemanager".into(),
                remote: "testdrive".into(),
                transfer_snapshot: json!([
                    {
                        "name": "k.txt",
                        "src": "/tmp/gtk-upload-test/k.txt",
                        "dst": "testdrive:k.txt",
                        "size": 3,
                        "bytes": 0
                    }
                ]),
                ..Default::default()
            },
        );
        let restored = find_stored_job(&[], &[], &meta, 186).unwrap();
        assert_eq!(restored.remote, "testdrive");
        assert_eq!(restored.origin, "filemanager");
        assert_eq!(restored.src, "/tmp/gtk-upload-test/k.txt");
        assert_eq!(restored.operation, "copy");
        assert_eq!(restored.completed[0]["bytes"], 3);
        assert_eq!(restored.completed[0]["percentage"], 100);
        assert_eq!(format_job_speed(0.0), "—");
        assert!(format_job_speed(1024.0).contains("KiB"));
        let mut preview = restored.clone();
        preview.stats = json!({ "speed": 2048.0, "eta": 90.0 });
        preview.transferring = json!([{ "name": "a.bin", "percentage": 40, "speed": 1024.0 }]);
        let (speed, eta) = job_speed_eta(&preview);
        assert!(speed.contains("KiB"));
        assert!(eta.contains("1m"));
        let previews = job_transfer_previews(&preview, 4);
        assert_eq!(previews[0].0, "a.bin");
        assert!(previews[0].1.contains("40%"));
        preview.error = Some("  network timeout  ".into());
        preview.completed = json!([
            { "name": "ok.txt" },
            { "name": "bad.txt", "error": "permission denied" }
        ]);
        assert_eq!(job_error_text(&preview).as_deref(), Some("network timeout"));
        assert_eq!(
            job_failed_transfers(&preview, 8),
            vec![("bad.txt".into(), "permission denied".into())]
        );
        assert!((restored.progress - 1.0).abs() < f64::EPSILON);
        assert!(!has_known_start_time(&restored));
        let combined = history_with_meta(&history, &meta);
        assert!(combined.iter().any(|job| job.id == 186));
        assert!(combined.iter().any(|job| job.id == 2));
        let noise = job_from_status(
            186,
            &json!({
                "finished": true,
                "success": true,
                "group": "job/186",
                "output": {}
            }),
            None,
        );
        assert!(!is_managed_job(&noise));
        let resolved = resolve_detail_job(Some(noise), Some(restored.clone())).unwrap();
        assert_eq!(resolved.operation, "copy");
        assert_eq!(resolved.src, "/tmp/gtk-upload-test/k.txt");
        let mut zero = restored.clone();
        zero.progress = 0.0;
        finalize_history_job(&mut zero);
        assert!((zero.progress - 1.0).abs() < f64::EPSILON);
        assert_eq!(
            job_status_key("completed"),
            "detailShared.jobs.status.completed"
        );
        assert_eq!(
            job_origin_key("filemanager"),
            "generalOverview.jobs.originFiles"
        );
    }

    #[test]
    fn job_panel_row_matches_angular_rules() {
        let now = Utc::now();
        let mut copy = running_job(12, "testdrive", "copy", "gui-copy-test");
        copy.stats = json!({ "bytes": 512, "totalBytes": 1024 });
        copy.dry_run = true;
        copy.start_time = now - chrono::TimeDelta::minutes(3);
        let row = job_panel_row(&copy, now);
        assert_eq!(row.id_label, "#12");
        assert_eq!(row.profile, "gui-copy-test");
        assert_eq!(row.progress_pct, Some(50));
        assert_eq!(row.bytes, 512);
        assert_eq!(row.total_bytes, 1024);
        assert!(row.dry_run);
        assert!(row.can_stop);
        assert!(row.has_footer);
        assert_eq!(
            row.relative,
            Some(("shared.transferActivity.time.minutesAgo", 3))
        );

        let mut mount = running_job(3, "testdrive", "mount", "default");
        mount.stats = json!({ "bytes": 10, "totalBytes": 100 });
        assert!(job_panel_row(&mount, now).progress_pct.is_none());

        let mut done = running_job(4, "testdrive", "sync", "default");
        done.status = "completed".into();
        done.stats = json!({});
        done.duration = 12.0;
        done.start_time = chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default();
        let idle = job_panel_row(&done, now);
        assert!(idle.progress_pct.is_none());
        assert!(!idle.can_stop);
        assert_eq!(idle.duration_secs, 12);
        assert!(idle.relative.is_none());
        assert!(idle.has_footer);
    }

    #[test]
    fn quick_run_card_badges_and_openable_folders() {
        let mut qr = QuickRun::new("Nightly".into(), OperationType::Copy, "testdrive".into());
        qr.config.rclone = json!({
            "srcFs": "testdrive:Photos",
            "dstFs": "testdrive:verify-qr"
        });
        qr.config.app.cron_enabled = true;
        qr.config.app.cron_expression = "0 7 * * *".into();
        qr.config.app.watch_enabled = true;
        qr.config.app.watch_changed_only = true;
        qr.config.app.auto_start = true;
        let badges = quick_run_card_badges(&qr);
        assert!(badges.cron);
        assert_eq!(badges.cron_expression, "0 7 * * *");
        assert!(badges.watcher);
        assert!(badges.watcher_changed_only);
        assert!(badges.autostart);
        assert_eq!(
            quick_run_openable_folders(&qr),
            vec![
                QuickRunFolder {
                    kind: "source",
                    path: "testdrive:Photos".into()
                },
                QuickRunFolder {
                    kind: "destination",
                    path: "testdrive:verify-qr".into()
                }
            ]
        );
        qr.config.app.cron_enabled = true;
        qr.config.app.cron_expression.clear();
        assert!(!quick_run_card_badges(&qr).cron);
        qr.config.rclone = json!({});
        assert!(quick_run_openable_folders(&qr).is_empty());
    }

    #[test]
    fn chip_action_profile_prefers_running_job() {
        let running = running_job(1, "drive", "copy", "nightly");
        assert_eq!(
            chip_action_profile(
                "drive",
                OperationType::Copy,
                &["gui-copy-test".into(), "nightly".into()],
                &[running]
            ),
            "nightly"
        );
        assert_eq!(
            chip_action_profile("drive", OperationType::Copy, &["gui-copy-test".into()], &[]),
            "gui-copy-test"
        );
        assert_eq!(
            chip_action_profile("drive", OperationType::Sync, &[], &[]),
            "default"
        );
    }

    #[test]
    fn profile_pill_status_matches_angular() {
        assert_eq!(
            profile_pill_status(true, true, "0 7 * * *", true),
            ProfilePillStatus::Running
        );
        assert_eq!(
            profile_pill_status(false, true, "0 7 * * *", false),
            ProfilePillStatus::Scheduled
        );
        assert_eq!(
            profile_pill_status(false, false, "", true),
            ProfilePillStatus::Scheduled
        );
        assert_eq!(
            profile_pill_status(false, true, "", false),
            ProfilePillStatus::Idle
        );
        assert_eq!(
            profile_pill_status(false, false, "", false),
            ProfilePillStatus::Idle
        );
        assert!(profile_pill_has_watcher(false, "", true));
        assert!(!profile_pill_has_watcher(true, "0 7 * * *", true));
        assert!(!profile_pill_has_watcher(true, "0 7 * * *", false));
    }

    fn sample_job(id: u64, src: &str, dst: &str) -> JobInfo {
        JobInfo {
            id,
            operation: "copy".into(),
            remote: "testdrive".into(),
            profile: "default".into(),
            status: "completed".into(),
            origin: "quick-run".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: src.into(),
            dst: dst.into(),
            group: format!("job/{id}"),
            stats: json!({}),
            transferring: json!([]),
            duration: 1.0,
            progress: 1.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        }
    }

    #[test]
    fn scopes_completed_transfers_to_the_open_job() {
        let mut job = sample_job(22718, "testdrive:Photos", "testdrive:verify-qr");
        job.completed = json!([
            {
                "name": "README.txt",
                "srcFs": "testdrive:Photos",
                "dstFs": "testdrive:verify-qr"
            },
            {
                "name": "other.jpg",
                "srcFs": "testdrive:",
                "dstFs": "testdrive:verify-copy-to"
            },
            { "name": "stale", "group": "job/14781" }
        ]);
        scope_job_transfers(&mut job);
        let names: Vec<_> = job
            .completed
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(names, vec!["README.txt"]);
    }

    #[test]
    fn job_from_status_drops_global_stats_from_another_job() {
        let status = json!({
            "finished": true,
            "success": true,
            "group": "job/22718",
            "output": { "operation": "copy", "srcFs": "testdrive:Photos", "dstFs": "testdrive:verify-qr" }
        });
        let stats = json!({
            "completed": [
                { "name": "README.txt", "srcFs": "testdrive:Photos", "dstFs": "testdrive:verify-qr" },
                { "name": "leaked.jpg", "srcFs": "testdrive:", "dstFs": "testdrive:other", "jobid": 14781 }
            ]
        });
        let job = job_from_status(22718, &status, Some(&stats));
        assert_eq!(job.completed.as_array().unwrap().len(), 1);
        assert_eq!(job.completed[0]["name"], "README.txt");
    }

    #[test]
    fn find_quick_run_job_prefers_live_then_last_id() {
        let mut qr = crate::store::QuickRun::new(
            "gui-qr-copy".into(),
            OperationType::Copy,
            "testdrive".into(),
        );
        qr.last_job_id = Some(22718);
        qr.config.rclone = json!({ "srcFs": "testdrive:Photos", "dstFs": "testdrive:verify-qr" });
        let mut live = sample_job(9, "testdrive:Photos", "testdrive:verify-qr");
        live.status = "running".into();
        let history = vec![sample_job(22718, "testdrive:Photos", "testdrive:verify-qr")];
        let found = find_quick_run_job(&[live], &history, &qr).unwrap();
        assert_eq!(found.id, 9);
        let found = find_quick_run_job(&[], &history, &qr).unwrap();
        assert_eq!(found.id, 22718);
    }

    #[test]
    fn operation_control_paths_prefer_live_job_and_hide_delete_dest() {
        let configured = operation_control_paths(
            OperationType::Copy,
            Some("testdrive:Photos".into()),
            Some("testdrive:verify-qr".into()),
            None,
        );
        assert_eq!(configured.source.as_deref(), Some("testdrive:Photos"));
        assert_eq!(
            configured.destination.as_deref(),
            Some("testdrive:verify-qr")
        );
        assert!(!configured.hide_destination);
        assert!(configured.dest_browseable);

        let live = sample_job(9, "testdrive:live-src", "testdrive:live-dst");
        let from_job = operation_control_paths(
            OperationType::Copy,
            Some("testdrive:Photos".into()),
            Some("testdrive:verify-qr".into()),
            Some(&live),
        );
        assert_eq!(from_job.source.as_deref(), Some("testdrive:live-src"));
        assert_eq!(from_job.destination.as_deref(), Some("testdrive:live-dst"));

        let delete = operation_control_paths(
            OperationType::Delete,
            Some("testdrive:Trash".into()),
            Some("unused".into()),
            None,
        );
        assert!(delete.hide_destination);
        assert_eq!(delete.source.as_deref(), Some("testdrive:Trash"));
        assert_eq!(
            operation_control_subtitle("Copy", true, "Dry run"),
            "Copy · Dry run"
        );
        assert_eq!(operation_control_subtitle("Copy", false, "Dry run"), "Copy");
        assert!(operation_shows_session_flags(OperationType::Copy));
        assert!(!operation_shows_session_flags(OperationType::Mount));
        assert!(operation_shows_mount_usage(
            OperationType::Mount,
            true,
            "/tmp/mnt"
        ));
        assert!(!operation_shows_mount_usage(
            OperationType::Mount,
            false,
            "/tmp/mnt"
        ));
        assert_eq!(
            operation_control_action_kind(OperationType::Mount, false),
            "mount"
        );
        assert_eq!(
            operation_control_action_kind(OperationType::Copy, true),
            "stop"
        );

        let serve = operation_control_configured_paths(
            OperationType::Serve,
            &json!({ "type": "webdav", "addr": "127.0.0.1:18080", "srcFs": "testdrive:" }),
            "testdrive",
            "Default",
        );
        assert_eq!(serve.0.as_deref(), Some("testdrive:"));
        assert_eq!(serve.1.as_deref(), Some("WEBDAV at 127.0.0.1:18080"));
        let empty_serve = operation_control_configured_paths(
            OperationType::Serve,
            &json!({}),
            "testdrive",
            "Default",
        );
        assert_eq!(empty_serve.1.as_deref(), Some("HTTP at Default"));
        let serve_paths =
            operation_control_paths(OperationType::Serve, empty_serve.0, empty_serve.1, None);
        assert!(!serve_paths.dest_browseable);
        assert_eq!(serve_paths.destination.as_deref(), Some("HTTP at Default"));

        let saf = operation_control_configured_paths(
            OperationType::Mount,
            &json!({ "mountType": "saf", "srcFs": "phone:" }),
            "phone",
            "Default",
        );
        assert_eq!(saf.1.as_deref(), Some("saf://phone"));
        assert!(is_saf_mount(&json!({ "mountPoint": "saf://phone" })));
    }
}
