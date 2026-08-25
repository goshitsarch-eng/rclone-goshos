//! Start rclone jobs from saved remote profiles — mirrors
//! `start_profile_batch` / `parse_common_config`.

use crate::operations::OperationType;
use crate::rclone::{remote_fs, RcClient, RcError};
use crate::store::{quick_run_paths, ProfileConfig};
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

pub fn start_profile(
    client: &RcClient,
    remote: &str,
    op: OperationType,
    profile: &ProfileConfig,
) -> Result<String, String> {
    let rclone = flatten_rclone(&profile.rclone);
    let source = default_source(remote, &rclone);
    let dest = default_dest(remote, &rclone, op);
    let request = build_job_params(op, remote, &source, &dest, &rclone)?;
    start_request(client, &request).map_err(|e| e.to_string())
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
}
