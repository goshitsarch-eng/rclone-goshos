//! Start rclone jobs from saved remote profiles — mirrors
//! `start_profile_batch` / `parse_common_config`.

use crate::operations::OperationType;
use crate::rclone::{remote_fs, MountedRemote, RcClient, RcError, ServeItem};
use crate::store::{quick_run_paths, JobInfo, JobMeta, ProfileConfig, QuickRun, RemoteMeta};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

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

pub fn is_dry_run(rclone: &Value) -> bool {
    for key in ["DryRun", "dry_run", "dryRun"] {
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
        "path1",
        "fs",
        "dest",
        "dstFs",
        "path2",
        "mountPoint",
        "url",
        "type",
        "addr",
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
            if let Some(source) = filtered.first() {
                obj.insert("url".into(), json!(source));
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

pub fn build_job_params(
    op: OperationType,
    remote: &str,
    source: &str,
    dest: &str,
    rclone: &Value,
) -> Result<JobRequest, String> {
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
            let serve_type = obj
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("webdav")
                .to_string();
            let addr = if dest.is_empty() {
                obj.get("addr")
                    .and_then(|v| v.as_str())
                    .unwrap_or("127.0.0.1:0")
                    .to_string()
            } else {
                dest.to_string()
            };
            Ok(JobRequest::Serve {
                serve_type,
                fs: if source.is_empty() {
                    remote_fs(remote, "")
                } else {
                    source.to_string()
                },
                addr,
            })
        }
        OperationType::Delete => {
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
        JobRequest::Async { endpoint, params } => client
            .start_job(endpoint, params.clone())
            .map(|id| format!("#{id}")),
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
        } => client.serve_start(serve_type, fs, addr).map(|v| {
            v.get("addr")
                .and_then(|x| x.as_str())
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
        if let Some(map) = runtime.as_object() {
            for (k, v) in map {
                obj.entry(k.clone()).or_insert(v.clone());
            }
        }
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

pub fn start_profile(
    client: &RcClient,
    remote: &str,
    op: OperationType,
    profile: &ProfileConfig,
    meta: Option<&RemoteMeta>,
    origin: &str,
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
    let mut sources = path_list(&rclone, SOURCE_KEYS);
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
    let mut ids = Vec::new();
    for source in sources {
        let request = build_job_params(op, remote, &source, &dest, &rclone)?;
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
    for id in parse_started_ids(result) {
        map.insert(id, meta.clone());
    }
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
    }
}

pub fn job_is_running(job: &JobInfo) -> bool {
    matches!(job.status.as_str(), "running" | "starting")
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
    let prefix = format!("{remote}:");
    mounts
        .iter()
        .find(|m| m.fs == remote || m.fs == prefix || m.fs.starts_with(&prefix))
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

pub fn profile_usage(
    jobs: &[JobInfo],
    mounts: &[MountedRemote],
    serves: &[ServeItem],
    remote: &str,
    profile: &str,
    op: Option<OperationType>,
) -> ProfileUsage {
    let mut usage = ProfileUsage::default();
    match op {
        Some(op) => {
            if find_active_job(jobs, remote, op, profile).is_some() {
                usage.jobs = 1;
            }
            if op == OperationType::Mount && find_active_mount(mounts, remote).is_some() {
                usage.mounts = 1;
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
            if find_active_mount(mounts, remote).is_some() {
                usage.mounts = 1;
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
    match op {
        OperationType::Mount => {
            let mount = find_active_mount(mounts, remote)
                .ok_or_else(|| format!("{remote} is not mounted"))?;
            client
                .unmount(&mount.mount_point)
                .map(|_| format!("Unmounted {}", mount.mount_point))
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
    let mut map = Map::new();
    let tokens: Vec<&str> = cli.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i].trim_start_matches('-');
        if token.is_empty() {
            i += 1;
            continue;
        }
        if let Some((k, v)) = token.split_once('=') {
            map.insert(k.replace('-', "_"), json!(v));
        } else if i + 1 < tokens.len() && !tokens[i + 1].starts_with('-') {
            map.insert(token.replace('-', "_"), json!(tokens[i + 1]));
            i += 1;
        } else {
            map.insert(token.replace('-', "_"), json!(true));
        }
        i += 1;
    }
    map
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
    job
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
    }
}

/// Keep preparing uploads in the live list until rclone reports the same job id.
pub fn merge_preparing_jobs(live: Vec<JobInfo>, history: &[JobInfo]) -> Vec<JobInfo> {
    let mut out = live;
    for job in history {
        if job.status == "preparing" && !out.iter().any(|j| j.id == job.id) {
            out.insert(0, job.clone());
        }
    }
    out
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
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("off") || trimmed == "0" {
        "off".into()
    } else {
        trimmed.to_string()
    }
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
    format!(
        "{} → {}",
        src.unwrap_or_else(|| "—".into()),
        dst.unwrap_or_else(|| "—".into())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
            },
        );
        let mut job = running_job(9, "", "sync", "default");
        apply_job_meta(&mut job, map.get(&9));
        assert_eq!(job.origin, "flow");
        assert_eq!(job.profile, "photos");
        assert_eq!(job.remote, "drive");
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
        assert_eq!(normalize_bandwidth("10M"), "10M");
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
            find_active_job(&jobs, "drive", OperationType::Sync, "nightly").map(|j| j.id),
            Some(1)
        );
        assert!(find_active_job(&jobs, "drive", OperationType::Sync, "missing").is_none());
        let mounts = vec![MountedRemote {
            fs: "drive:photos".into(),
            mount_point: "/mnt/drive".into(),
        }];
        assert!(find_active_mount(&mounts, "drive").is_some());
        assert!(find_active_mount(&mounts, "dropbox").is_none());
        let serves = vec![ServeItem {
            id: "abc".into(),
            addr: "127.0.0.1:8080".into(),
            fs: "drive:".into(),
            serve_type: "webdav".into(),
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
        );
        assert!(usage.blocked());
        assert_eq!(usage.jobs, 1);
        let idle = profile_usage(&[], &[], &[], "drive", "default", Some(OperationType::Sync));
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
    fn preparing_job_and_merge() {
        let preparing = preparing_job(9, "drive", "/tmp/a.txt", "Inbox/a.txt", 1, 32);
        assert_eq!(preparing.status, "preparing");
        assert_eq!(preparing.stats["preparing"], true);
        assert_eq!(preparing.stats["totalBytes"], 32);
        let live = vec![running_job(1, "drive", "sync", "nightly")];
        let merged = merge_preparing_jobs(live.clone(), &[preparing.clone()]);
        assert_eq!(merged[0].id, 9);
        assert_eq!(merged.len(), 2);
        let already = merge_preparing_jobs(
            vec![JobInfo {
                status: "running".into(),
                ..preparing.clone()
            }],
            &[preparing],
        );
        assert_eq!(already.len(), 1);
        assert_eq!(already[0].status, "running");
        let from_stats = job_from_status(
            4,
            &json!({ "finished": false, "success": false, "output": { "operation": "upload" } }),
            Some(&json!({ "preparing": true, "bytes": 0 })),
        );
        assert_eq!(from_stats.status, "preparing");
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
}
