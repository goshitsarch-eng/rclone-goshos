//! Start rclone jobs from saved remote profiles — mirrors
//! `start_profile_batch` / `parse_common_config`.

use crate::operations::OperationType;
use crate::rclone::{remote_fs, RcClient, RcError};
use crate::store::{quick_run_paths, JobInfo, ProfileConfig, RemoteMeta};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

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
        OperationType::Mount => dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("mnt")
            .join(remote)
            .to_string_lossy()
            .into_owned(),
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
            obj.insert("autoFilename".into(), json!(true));
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
            let mut dest = dest.to_string();
            if !has_archive_extension(&dest) {
                let format = obj.get("format").and_then(|v| v.as_str()).unwrap_or("zip");
                dest = format!("{dest}/archive.{format}");
            }
            Ok(JobRequest::Async {
                endpoint: "operations/copyfile",
                params: json!({
                    "srcFs": source,
                    "srcRemote": "",
                    "dstFs": dest,
                    "dstRemote": ""
                }),
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
) -> Result<String, String> {
    let mut rclone = flatten_rclone(&profile.rclone);
    apply_helper_options(&mut rclone, profile, meta);
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
    let duration = status
        .get("duration")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let progress = progress_from_stats(&stats_value);
    JobInfo {
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
        origin: "dashboard".into(),
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
        stats: stats_value,
        transferring,
        duration,
        progress,
        output,
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
            JobRequest::Async { params, .. } => {
                assert_eq!(params["url"], "https://example.com/a.bin");
                assert_eq!(params["fs"], "drive:Inbox");
            }
            _ => panic!(),
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
            "transferring": [{ "name": "a.bin", "percentage": 50 }]
        });
        let job = job_from_status(9, &status, Some(&stats));
        assert_eq!(job.id, 9);
        assert_eq!(job.status, "running");
        assert_eq!(job.operation, "sync");
        assert_eq!(job.remote, "drive");
        assert_eq!(job.src, "drive:Photos");
        assert_eq!(job.dst, "/tmp/out");
        assert!(job.dry_run);
        assert!((job.progress - 0.5).abs() < f64::EPSILON);
        assert_eq!(job.transferring[0]["name"], "a.bin");
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
}
