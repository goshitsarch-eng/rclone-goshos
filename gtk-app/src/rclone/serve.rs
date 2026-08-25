//! Serve start/list/stop helpers, including a fallback for rclone < 1.64
//! where `serve/start`, `serve/list`, and `serve/stop` do not exist.

use super::client::{RcClient, RcError, ServeItem};
use super::engine::is_reserved_flag;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const FALLBACK_JOB_PREFIX: &str = "cmd-";
const FALLBACK_PROC_PREFIX: &str = "proc-";

#[derive(Debug, Clone)]
pub struct ServeSpawnContext {
    pub binary: PathBuf,
    pub config_path: Option<PathBuf>,
    pub extra_flags: Vec<String>,
    pub extra_env: Vec<(String, String)>,
    pub password: String,
}

struct LegacyServe {
    item: ServeItem,
    jobid: Option<u64>,
    child: Option<Child>,
}

static SPAWN_CTX: LazyLock<Mutex<Option<ServeSpawnContext>>> = LazyLock::new(|| Mutex::new(None));
static LEGACY: LazyLock<Mutex<HashMap<String, LegacyServe>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn install_spawn_context(ctx: ServeSpawnContext) {
    if let Ok(mut slot) = SPAWN_CTX.lock() {
        *slot = Some(ctx);
    }
}

pub fn clear_spawn_context() {
    if let Ok(mut slot) = SPAWN_CTX.lock() {
        *slot = None;
    }
}

pub fn spawn_context_from_settings(
    binary: PathBuf,
    config_path: Option<PathBuf>,
    extra_flags: &[String],
    extra_env: &[String],
    password: &str,
) -> ServeSpawnContext {
    ServeSpawnContext {
        binary,
        config_path,
        extra_flags: extra_flags
            .iter()
            .filter(|flag| !is_reserved_flag(flag))
            .cloned()
            .collect(),
        extra_env: extra_env
            .iter()
            .filter_map(|entry| {
                entry
                    .split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
            })
            .collect(),
        password: password.to_string(),
    }
}

pub fn serve_cli_args(serve_type: &str, fs: &str, addr: &str) -> Vec<String> {
    vec![
        serve_type.to_string(),
        fs.to_string(),
        "--addr".into(),
        addr.to_string(),
    ]
}

pub fn serve_extra_cli_args(extra: &Value) -> Vec<String> {
    let Some(obj) = extra.as_object() else {
        return Vec::new();
    };
    let mut args = Vec::new();
    for (key, value) in obj {
        if matches!(
            key.as_str(),
            "type"
                | "serveType"
                | "fs"
                | "addr"
                | "origin"
                | "profile"
                | "vfsOpt"
                | "_filter"
                | "_config"
                | "url"
        ) {
            continue;
        }
        if value.is_object() || value.is_array() {
            continue;
        }
        let flag = camel_to_cli_flag(key);
        match value {
            Value::Bool(true) => args.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::String(s) if s.is_empty() => {}
            Value::String(s) => args.push(format!("{flag}={s}")),
            Value::Number(n) => args.push(format!("{flag}={n}")),
            _ => {}
        }
    }
    args
}

pub fn camel_to_cli_flag(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.starts_with('-') {
        return trimmed.to_string();
    }
    let mut out = String::from("--");
    for (index, ch) in trimmed.chars().enumerate() {
        if ch == '_' || ch == '-' {
            out.push('-');
        } else if ch.is_uppercase() {
            if index > 0 && !out.ends_with('-') {
                out.push('-');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn resolve_listen_addr(addr: &str) -> String {
    let trimmed = addr.trim();
    if trimmed.is_empty() {
        return format!("127.0.0.1:{}", pick_free_port().unwrap_or(8080));
    }
    let Some((host, port)) = split_host_port(trimmed) else {
        return trimmed.to_string();
    };
    if port != "0" {
        return trimmed.to_string();
    }
    let free = pick_free_port().unwrap_or(8080);
    if host.is_empty() {
        format!("127.0.0.1:{free}")
    } else {
        format!("{host}:{free}")
    }
}

pub fn split_host_port(addr: &str) -> Option<(&str, &str)> {
    if let Some(rest) = addr.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail.strip_prefix(':')?;
        return Some((host, port));
    }
    addr.rsplit_once(':')
}

pub fn fallback_job_id(jobid: u64) -> String {
    format!("{FALLBACK_JOB_PREFIX}{jobid}")
}

pub fn fallback_proc_id(pid: u32) -> String {
    format!("{FALLBACK_PROC_PREFIX}{pid}")
}

pub fn parse_fallback_job_id(id: &str) -> Option<u64> {
    id.strip_prefix(FALLBACK_JOB_PREFIX)?.parse().ok()
}

pub fn parse_fallback_proc_id(id: &str) -> Option<u32> {
    id.strip_prefix(FALLBACK_PROC_PREFIX)?.parse().ok()
}

pub fn parse_serve_job_id(value: &Value) -> Option<u64> {
    value
        .get("jobid")
        .or_else(|| value.get("jobId"))
        .and_then(|v| v.as_u64())
}

pub fn serve_start_response(id: &str, addr: &str) -> Value {
    json!({ "id": id, "addr": addr })
}

pub fn serve_item_from_start(
    id: &str,
    addr: &str,
    fs: &str,
    serve_type: &str,
    extra: &Value,
) -> ServeItem {
    let option_count = extra
        .as_object()
        .map(|obj| {
            obj.keys()
                .filter(|key| {
                    !matches!(
                        key.as_str(),
                        "fs" | "type" | "origin" | "profile" | "url" | "addr" | "serveType"
                    )
                })
                .count()
        })
        .unwrap_or(0);
    ServeItem {
        id: id.to_string(),
        addr: addr.to_string(),
        fs: fs.to_string(),
        serve_type: serve_type.to_string(),
        origin: extra
            .get("origin")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        profile: extra
            .get("profile")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        option_count,
    }
}

pub fn merge_serve_payload(serve_type: &str, fs: &str, addr: &str, extra: &Value) -> Value {
    let mut payload = json!({
        "type": serve_type,
        "fs": fs,
        "addr": addr
    });
    if let (Some(obj), Some(extra_obj)) = (payload.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_obj {
            if matches!(key.as_str(), "type" | "serveType" | "fs" | "addr") {
                continue;
            }
            obj.insert(key.clone(), value.clone());
        }
    }
    payload
}

fn pick_free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|listener| listener.local_addr().ok().map(|addr| addr.port()))
}

fn register_legacy(item: ServeItem, jobid: Option<u64>, child: Option<Child>) {
    if let Ok(mut map) = LEGACY.lock() {
        map.insert(item.id.clone(), LegacyServe { item, jobid, child });
    }
}

pub fn list_legacy_serves() -> Vec<ServeItem> {
    LEGACY
        .lock()
        .map(|map| map.values().map(|item| item.item.clone()).collect())
        .unwrap_or_default()
}

pub fn reap_legacy_serves(client: Option<&RcClient>) -> Vec<ServeItem> {
    let Ok(mut map) = LEGACY.lock() else {
        return Vec::new();
    };
    map.retain(|_, item| {
        if let Some(jobid) = item.jobid {
            return match client.map(|c| c.job_status(jobid)) {
                Some(Ok(status)) => {
                    let finished = status
                        .get("finished")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    !finished
                }
                Some(Err(_)) => false,
                None => true,
            };
        }
        if let Some(child) = item.child.as_mut() {
            return match child.try_wait() {
                Ok(None) => true,
                _ => false,
            };
        }
        true
    });
    map.values().map(|item| item.item.clone()).collect()
}

pub fn stop_legacy_serve(client: Option<&RcClient>, id: &str) -> bool {
    let Some(mut item) = LEGACY.lock().ok().and_then(|mut map| map.remove(id)) else {
        return false;
    };
    if let Some(jobid) = item.jobid {
        if let Some(client) = client {
            let _ = client.job_stop(jobid);
        }
    }
    if let Some(mut child) = item.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    } else if let Some(pid) = parse_fallback_proc_id(id) {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
    true
}

pub fn shutdown_legacy(client: Option<&RcClient>) {
    let ids: Vec<String> = LEGACY
        .lock()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    for id in ids {
        stop_legacy_serve(client, &id);
    }
}

/// rclone &lt; 1.64 has no `serve/start`. Using `core/command serve` on those
/// builds can panic the remote-control server (observed on 1.60), so fallback
/// always runs a separate `rclone serve` process.
pub fn start_serve_fallback(
    _client: &RcClient,
    serve_type: &str,
    fs: &str,
    addr: &str,
    extra: &Value,
    original: RcError,
) -> Result<Value, RcError> {
    spawn_legacy_serve(serve_type, fs, addr, extra).map_err(|spawn_err| {
        RcError::message(format!(
            "{original}; fallback rclone serve failed: {spawn_err}"
        ))
    })
}

fn spawn_legacy_serve(
    serve_type: &str,
    fs: &str,
    addr: &str,
    extra: &Value,
) -> Result<Value, RcError> {
    let ctx = SPAWN_CTX
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or_else(|| RcError::message("no rclone binary available to start serve"))?;
    let mut cmd = Command::new(&ctx.binary);
    cmd.arg("serve")
        .arg(serve_type)
        .arg(fs)
        .arg("--addr")
        .arg(addr);
    for flag in serve_extra_cli_args(extra) {
        cmd.arg(flag);
    }
    if let Some(path) = &ctx.config_path {
        cmd.arg(format!("--config={}", path.display()));
    }
    for flag in &ctx.extra_flags {
        cmd.arg(flag);
    }
    for (key, value) in &ctx.extra_env {
        cmd.env(key, value);
    }
    crate::security::apply_config_password_env(&mut cmd, &ctx.password);
    crate::repair::apply_fusermount_path(&mut cmd);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|err| RcError::message(format!("failed to spawn rclone serve: {err}")))?;
    std::thread::sleep(Duration::from_millis(200));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(RcError::message(format!(
            "rclone serve exited immediately ({status})"
        )));
    }
    let id = fallback_proc_id(child.id());
    register_legacy(
        serve_item_from_start(&id, addr, fs, serve_type, extra),
        None,
        Some(child),
    );
    Ok(serve_start_response(&id, addr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_cli_and_extra_args() {
        assert_eq!(
            serve_cli_args("http", "testdrive:", "127.0.0.1:8080"),
            vec!["http", "testdrive:", "--addr", "127.0.0.1:8080"]
        );
        let extra = json!({
            "readOnly": true,
            "baseurl": "/pub",
            "vfsOpt": { "CacheMode": 2 },
            "origin": "dashboard",
            "empty": ""
        });
        let args = serve_extra_cli_args(&extra);
        assert!(args.iter().any(|a| a == "--read-only"));
        assert!(args.iter().any(|a| a == "--baseurl=/pub"));
        assert!(!args.iter().any(|a| a.contains("vfs")));
        assert!(!args.iter().any(|a| a.contains("origin")));
    }

    #[test]
    fn camel_case_flags_become_cli() {
        assert_eq!(camel_to_cli_flag("readOnly"), "--read-only");
        assert_eq!(camel_to_cli_flag("vfs_cache_mode"), "--vfs-cache-mode");
        assert_eq!(camel_to_cli_flag("--already"), "--already");
        assert_eq!(camel_to_cli_flag("HTTP2"), "--h-t-t-p2");
    }

    #[test]
    fn listen_addr_rewrites_ephemeral_port() {
        let resolved = resolve_listen_addr("127.0.0.1:0");
        assert!(resolved.starts_with("127.0.0.1:"));
        assert_ne!(resolved, "127.0.0.1:0");
        assert_eq!(resolve_listen_addr("127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(resolve_listen_addr(":8080"), ":8080");
        let ephemeral = resolve_listen_addr(":0");
        assert!(ephemeral.starts_with("127.0.0.1:"));
        assert_eq!(split_host_port("[::1]:0"), Some(("::1", "0")));
        assert!(parse_fallback_job_id("cmd-12") == Some(12));
        assert!(parse_fallback_proc_id("proc-99") == Some(99));
        assert!(parse_fallback_job_id("http-abc").is_none());
    }

    #[test]
    fn serve_payload_and_item_keep_options() {
        let extra = json!({
            "origin": "dashboard",
            "profile": "public",
            "readOnly": true,
            "vfsOpt": { "CacheMode": 2 }
        });
        let payload = merge_serve_payload("http", "drive:pub", "127.0.0.1:8080", &extra);
        assert_eq!(payload["type"], "http");
        assert_eq!(payload["fs"], "drive:pub");
        assert_eq!(payload["addr"], "127.0.0.1:8080");
        assert_eq!(payload["origin"], "dashboard");
        assert_eq!(payload["vfsOpt"]["CacheMode"], 2);
        let item = serve_item_from_start("cmd-3", "127.0.0.1:8080", "drive:pub", "http", &extra);
        assert_eq!(item.origin, "dashboard");
        assert_eq!(item.profile, "public");
        assert_eq!(item.option_count, 2);
        assert_eq!(
            serve_start_response("cmd-3", "127.0.0.1:8080"),
            json!({ "id": "cmd-3", "addr": "127.0.0.1:8080" })
        );
        assert_eq!(parse_serve_job_id(&json!({ "jobid": 9 })), Some(9));
    }

    #[test]
    fn spawn_legacy_http_serve_list_and_stop() {
        let binary = which::which("rclone").unwrap_or_else(|_| PathBuf::from("rclone"));
        if which::which("rclone").is_err() {
            return;
        }
        install_spawn_context(ServeSpawnContext {
            binary,
            config_path: None,
            extra_flags: vec![],
            extra_env: vec![],
            password: String::new(),
        });
        let addr = resolve_listen_addr("127.0.0.1:0");
        let started = spawn_legacy_serve("http", "/tmp", &addr, &json!({})).expect("spawn serve");
        let id = started["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("proc-"));
        let listed = list_legacy_serves();
        assert!(listed.iter().any(|item| item.id == id && item.addr == addr));
        assert!(stop_legacy_serve(None, &id));
        assert!(!list_legacy_serves().iter().any(|item| item.id == id));
        clear_spawn_context();
    }
}
