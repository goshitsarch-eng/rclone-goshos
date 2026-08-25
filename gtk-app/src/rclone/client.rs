//! rclone Remote Control HTTP client.

use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RcError {
    #[error("{0}")]
    Message(String),
    #[error("rclone RC is not reachable at {0}")]
    Unreachable(String),
    #[error("http error: {0}")]
    Http(String),
}

impl RcError {
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

#[derive(Debug, Clone)]
pub struct RcClient {
    pub base_url: String,
    pub user: Option<String>,
    pub pass: Option<String>,
    timeout: Duration,
}

impl RcClient {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            base_url: format!("http://{host}:{port}"),
            user: None,
            pass: None,
            timeout: Duration::from_secs(30),
        }
    }

    pub fn with_auth(mut self, user: Option<String>, pass: Option<String>) -> Self {
        self.user = user;
        self.pass = pass;
        self
    }

    pub fn call(&self, endpoint: &str, params: Value) -> Result<Value, RcError> {
        self.call_timeout(endpoint, params, self.timeout)
    }

    pub fn call_timeout(
        &self,
        endpoint: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RcError> {
        let url = format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        );
        let mut request = ureq::post(&url)
            .timeout(timeout)
            .set("Content-Type", "application/json");
        if let (Some(user), Some(pass)) = (&self.user, &self.pass) {
            request = request.set("Authorization", &basic_auth_header(user, pass));
        }
        let body = if params.is_null() {
            "{}".to_string()
        } else {
            params.to_string()
        };
        match request.send_string(&body) {
            Ok(resp) => {
                let text = resp.into_string().unwrap_or_default();
                if text.is_empty() {
                    return Ok(json!({}));
                }
                serde_json::from_str(&text).map_err(|e| RcError::Http(e.to_string()))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                let message = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| {
                        v.get("error")
                            .and_then(|e| e.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or(text);
                Err(RcError::Message(format!("HTTP {code}: {message}")))
            }
            Err(ureq::Error::Transport(t)) => {
                if t.kind() == ureq::ErrorKind::ConnectionFailed || t.kind() == ureq::ErrorKind::Io
                {
                    Err(RcError::Unreachable(url))
                } else {
                    Err(RcError::Http(t.to_string()))
                }
            }
        }
    }

    pub fn ping(&self) -> bool {
        self.call_timeout("rc/noop", json!({}), Duration::from_secs(2))
            .is_ok()
    }

    pub fn version(&self) -> Result<String, RcError> {
        let v = self.call("core/version", json!({}))?;
        Ok(v.get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string())
    }

    pub fn list_remotes(&self) -> Result<Vec<String>, RcError> {
        let v = self.call("config/listremotes", json!({}))?;
        Ok(v.get("remotes")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn dump_config(&self) -> Result<Value, RcError> {
        self.call("config/dump", json!({}))
    }

    pub fn providers(&self) -> Result<Value, RcError> {
        self.call("config/providers", json!({}))
    }

    pub fn create_remote_interactive(
        &self,
        name: &str,
        r#type: &str,
        parameters: Value,
        opt: Option<Value>,
    ) -> Result<Value, RcError> {
        let mut body = json!({
            "name": name,
            "type": r#type,
            "parameters": parameters,
            "opt": { "nonInteractive": true }
        });
        if let Some(Value::Object(user)) = opt {
            if let Some(obj) = body["opt"].as_object_mut() {
                obj.extend(user);
            }
        }
        self.call("config/create", body)
    }

    pub fn continue_create_remote(
        &self,
        name: &str,
        state: &str,
        result: Value,
        parameters: Value,
        opt: Option<Value>,
    ) -> Result<Value, RcError> {
        let mut protocol = json!({
            "continue": true,
            "state": state,
            "result": result,
            "nonInteractive": true
        });
        if let Some(Value::Object(user)) = opt {
            if let Some(obj) = protocol.as_object_mut() {
                obj.extend(user);
            }
        }
        self.call(
            "config/update",
            json!({
                "name": name,
                "parameters": parameters,
                "opt": protocol
            }),
        )
    }

    pub fn oauth_status(&self) -> Result<(bool, Option<String>), RcError> {
        let v = self.call("config/oauthstatus", json!({}))?;
        let running = v.get("running").and_then(|x| x.as_bool()).unwrap_or(false);
        let url = v
            .get("authUrl")
            .or_else(|| v.get("auth_url"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        Ok((running, url))
    }

    pub fn oauth_stop(&self) -> Result<Value, RcError> {
        self.call("config/oauthstop", json!({}))
    }

    pub fn create_remote(
        &self,
        name: &str,
        r#type: &str,
        parameters: Value,
    ) -> Result<Value, RcError> {
        self.call(
            "config/create",
            json!({
                "name": name,
                "type": r#type,
                "parameters": parameters,
                "opt": { "nonInteractive": true }
            }),
        )
    }

    pub fn update_remote(&self, name: &str, parameters: Value) -> Result<Value, RcError> {
        self.call(
            "config/update",
            json!({ "name": name, "parameters": parameters }),
        )
    }

    pub fn delete_remote(&self, name: &str) -> Result<Value, RcError> {
        self.call("config/delete", json!({ "name": name }))
    }

    pub fn obscure(&self, value: &str) -> Result<String, RcError> {
        let v = self.call("core/obscure", json!({ "clear": value }))?;
        Ok(v.get("obscured")
            .and_then(|x| x.as_str())
            .unwrap_or(value)
            .to_string())
    }

    pub fn list_dir(&self, fs: &str, remote: &str) -> Result<Vec<DirEntry>, RcError> {
        let v = self.call("operations/list", json!({ "fs": fs, "remote": remote }))?;
        let list = v
            .get("list")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(list.iter().filter_map(DirEntry::from_value).collect())
    }

    pub fn mkdir(&self, fs: &str, remote: &str) -> Result<Value, RcError> {
        self.call("operations/mkdir", json!({ "fs": fs, "remote": remote }))
    }

    pub fn delete_file(&self, fs: &str, remote: &str) -> Result<Value, RcError> {
        self.call(
            "operations/deletefile",
            json!({ "fs": fs, "remote": remote }),
        )
    }

    pub fn purge(&self, fs: &str, remote: &str) -> Result<Value, RcError> {
        self.call("operations/purge", json!({ "fs": fs, "remote": remote }))
    }

    pub fn move_file(
        &self,
        src_fs: &str,
        src_remote: &str,
        dst_fs: &str,
        dst_remote: &str,
    ) -> Result<Value, RcError> {
        self.call(
            "operations/movefile",
            json!({
                "srcFs": src_fs,
                "srcRemote": src_remote,
                "dstFs": dst_fs,
                "dstRemote": dst_remote
            }),
        )
    }

    pub fn copy_url(&self, url: &str, fs: &str, remote: &str) -> Result<Value, RcError> {
        self.call(
            "operations/copyurl",
            json!({ "url": url, "fs": fs, "remote": remote }),
        )
    }

    pub fn copy_file(
        &self,
        src_fs: &str,
        src_remote: &str,
        dst_fs: &str,
        dst_remote: &str,
    ) -> Result<Value, RcError> {
        self.call(
            "operations/copyfile",
            json!({
                "srcFs": src_fs,
                "srcRemote": src_remote,
                "dstFs": dst_fs,
                "dstRemote": dst_remote
            }),
        )
    }

    pub fn about(&self, fs: &str) -> Result<Value, RcError> {
        self.call("operations/about", json!({ "fs": fs }))
    }

    pub fn size(&self, fs: &str, remote: &str) -> Result<Value, RcError> {
        self.call("operations/size", json!({ "fs": fs, "remote": remote }))
    }

    pub fn rmdirs(&self, fs: &str, remote: &str) -> Result<Value, RcError> {
        self.call("operations/rmdirs", json!({ "fs": fs, "remote": remote }))
    }

    pub fn cleanup(&self, fs: &str, remote: Option<&str>) -> Result<Value, RcError> {
        self.call("operations/cleanup", cleanup_payload(fs, remote))
    }

    pub fn archive_extract(&self, src: &str, dst: &str) -> Result<u64, RcError> {
        self.start_job(
            "operations/archive",
            json!({
                "action": "extract",
                "src": src,
                "dst": dst
            }),
        )
    }

    pub fn public_link(&self, fs: &str, remote: &str) -> Result<String, RcError> {
        let v = self.call(
            "operations/publiclink",
            json!({ "fs": fs, "remote": remote }),
        )?;
        Ok(v.get("url")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string())
    }

    pub fn hashsum(&self, fs: &str, remote: &str, hash_type: &str) -> Result<Value, RcError> {
        self.call(
            "operations/hashsum",
            json!({ "fs": fs, "remote": remote, "hashType": hash_type }),
        )
    }

    pub fn start_job(&self, endpoint: &str, mut params: Value) -> Result<u64, RcError> {
        if let Some(obj) = params.as_object_mut() {
            obj.insert("_async".into(), json!(true));
        }
        let v = self.call(endpoint, params)?;
        v.get("jobid")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| RcError::message("rclone did not return a jobid"))
    }

    pub fn job_status(&self, jobid: u64) -> Result<Value, RcError> {
        self.call("job/status", json!({ "jobid": jobid }))
    }

    pub fn job_list(&self) -> Result<Value, RcError> {
        self.call("job/list", json!({}))
    }

    pub fn job_stop(&self, jobid: u64) -> Result<Value, RcError> {
        self.call("job/stop", json!({ "jobid": jobid }))
    }

    pub fn stats(&self, group: Option<&str>) -> Result<Value, RcError> {
        match group {
            Some(g) => self.call("core/stats", json!({ "group": g })),
            None => self.call("core/stats", json!({})),
        }
    }

    pub fn reset_stats(&self, group: Option<&str>) -> Result<Value, RcError> {
        match group {
            Some(g) => self.call("core/stats-reset", json!({ "group": g })),
            None => self.call("core/stats-reset", json!({})),
        }
    }

    pub fn bwlimit(&self, rate: Option<&str>) -> Result<Value, RcError> {
        match rate {
            Some(r) => self.call("core/bwlimit", json!({ "rate": r })),
            None => self.call("core/bwlimit", json!({})),
        }
    }

    pub fn mount(&self, fs: &str, mount_point: &str, mount_type: &str) -> Result<Value, RcError> {
        self.call(
            "mount/mount",
            json!({
                "fs": fs,
                "mountPoint": mount_point,
                "mountType": mount_type
            }),
        )
    }

    pub fn unmount(&self, mount_point: &str) -> Result<Value, RcError> {
        self.call("mount/unmount", json!({ "mountPoint": mount_point }))
    }

    pub fn unmount_all(&self) -> Result<Value, RcError> {
        self.call("mount/unmountall", json!({}))
    }

    pub fn list_mounts(&self) -> Result<Vec<MountedRemote>, RcError> {
        let v = self.call("mount/listmounts", json!({}))?;
        let mounts = v
            .get("mountPoints")
            .and_then(|x| x.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(point, fs)| MountedRemote {
                        fs: fs.as_str().unwrap_or_default().to_string(),
                        mount_point: point.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(mounts)
    }

    pub fn serve_start(&self, serve_type: &str, fs: &str, addr: &str) -> Result<Value, RcError> {
        self.call(
            "serve/start",
            json!({
                "type": serve_type,
                "fs": fs,
                "addr": addr
            }),
        )
    }

    pub fn serve_stop(&self, id: &str) -> Result<Value, RcError> {
        self.call("serve/stop", json!({ "id": id }))
    }

    pub fn serve_stop_all(&self) -> Result<Value, RcError> {
        self.call("serve/stopall", json!({}))
    }

    pub fn serve_list(&self) -> Result<Vec<ServeItem>, RcError> {
        let v = self.call("serve/list", json!({}))?;
        let list = v
            .get("list")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(list
            .iter()
            .filter_map(|item| {
                Some(ServeItem {
                    id: item.get("id")?.as_str()?.to_string(),
                    addr: item
                        .get("addr")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    fs: item
                        .get("params")
                        .and_then(|p| p.get("fs"))
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    serve_type: item
                        .get("params")
                        .and_then(|p| p.get("type"))
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect())
    }

    pub fn vfs_list(&self) -> Result<Value, RcError> {
        self.call("vfs/list", json!({}))
    }

    pub fn vfs_stats(&self, fs: &str) -> Result<Value, RcError> {
        self.call("vfs/stats", json!({ "fs": fs }))
    }

    pub fn vfs_forget(&self, fs: &str) -> Result<Value, RcError> {
        self.call("vfs/forget", json!({ "fs": fs }))
    }

    pub fn vfs_refresh(&self, fs: &str) -> Result<Value, RcError> {
        self.call("vfs/refresh", json!({ "fs": fs }))
    }

    pub fn vfs_queue(&self, fs: &str) -> Result<Value, RcError> {
        self.call("vfs/queue", json!({ "fs": fs }))
    }

    pub fn options_get(&self) -> Result<Value, RcError> {
        self.call("options/get", json!({}))
    }

    pub fn options_set(&self, options: Value) -> Result<Value, RcError> {
        self.call("options/set", options)
    }

    pub fn options_info(&self) -> Result<Value, RcError> {
        self.call("options/info", json!({}))
    }

    pub fn local_disks(&self) -> Result<Vec<String>, RcError> {
        let v = self.call("core/disks", json!({}))?;
        Ok(v.get("disks")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn quit(&self) -> Result<Value, RcError> {
        self.call("core/quit", json!({}))
    }

    pub fn memstats(&self) -> Result<Value, RcError> {
        self.call("core/memstats", json!({}))
    }

    pub fn gc(&self) -> Result<Value, RcError> {
        self.call("core/gc", json!({}))
    }
}

fn basic_auth_header(user: &str, pass: &str) -> String {
    use base64::Engine;
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
    format!("Basic {token}")
}

#[derive(Debug, Clone, Default)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: i64,
    pub mime: String,
    pub mod_time: String,
}

impl DirEntry {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            name: value.get("Name")?.as_str()?.to_string(),
            path: value
                .get("Path")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            is_dir: value
                .get("IsDir")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            size: value.get("Size").and_then(|x| x.as_i64()).unwrap_or(0),
            mime: value
                .get("MimeType")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            mod_time: value
                .get("ModTime")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct MountedRemote {
    pub fs: String,
    pub mount_point: String,
}

#[derive(Debug, Clone)]
pub struct ServeItem {
    pub id: String,
    pub addr: String,
    pub fs: String,
    pub serve_type: String,
}

pub fn remote_fs(name: &str, path: &str) -> String {
    if name == "local" || name == "/" {
        if path.is_empty() {
            "/".into()
        } else {
            path.to_string()
        }
    } else if path.is_empty() {
        format!("{name}:")
    } else {
        format!("{name}:{path}")
    }
}

pub fn split_remote_path(input: &str) -> (String, String) {
    if let Some((remote, path)) = input.split_once(':') {
        if remote == "/" || remote.is_empty() {
            ("local".into(), input.to_string())
        } else {
            (remote.to_string(), path.to_string())
        }
    } else if input.starts_with('/') {
        ("local".into(), input.to_string())
    } else {
        (input.to_string(), String::new())
    }
}

pub fn join_remote_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

pub fn parent_remote_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

pub fn cleanup_payload(fs: &str, remote: Option<&str>) -> Value {
    match remote {
        Some(path) if !path.is_empty() => json!({ "fs": fs, "remote": path }),
        _ => json!({ "fs": fs }),
    }
}

pub fn format_bytes(bytes: i64) -> String {
    if bytes < 0 {
        return "—".into();
    }
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_path_helpers() {
        assert_eq!(remote_fs("gdrive", ""), "gdrive:");
        assert_eq!(remote_fs("gdrive", "Photos"), "gdrive:Photos");
        assert_eq!(
            split_remote_path("gdrive:Photos/x"),
            ("gdrive".into(), "Photos/x".into())
        );
        assert_eq!(
            split_remote_path("/home/ada"),
            ("local".into(), "/home/ada".into())
        );
        assert_eq!(join_remote_path("Photos", "img.png"), "Photos/img.png");
        assert_eq!(parent_remote_path("Photos/2024/img.png"), "Photos/2024");
        assert_eq!(parent_remote_path("Photos"), "");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(-1), "—");
    }

    #[test]
    fn parse_dir_entry() {
        let value = json!({
            "Name": "readme.md",
            "Path": "docs/readme.md",
            "IsDir": false,
            "Size": 12,
            "MimeType": "text/markdown",
            "ModTime": "2024-01-01T00:00:00Z"
        });
        let entry = DirEntry::from_value(&value).unwrap();
        assert_eq!(entry.name, "readme.md");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 12);
    }

    #[test]
    fn basic_auth_encodes() {
        let header = basic_auth_header("user", "pass");
        assert!(header.starts_with("Basic "));
    }

    #[test]
    fn cleanup_payload_omits_empty_remote() {
        assert_eq!(cleanup_payload("drive:", None), json!({ "fs": "drive:" }));
        assert_eq!(
            cleanup_payload("drive:", Some("Trash")),
            json!({ "fs": "drive:", "remote": "Trash" })
        );
        assert_eq!(
            cleanup_payload("drive:", Some("")),
            json!({ "fs": "drive:" })
        );
    }
}
