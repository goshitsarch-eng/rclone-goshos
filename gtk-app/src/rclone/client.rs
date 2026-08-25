//! rclone Remote Control HTTP client.

use serde_json::{json, Value};
use std::collections::BTreeMap;
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

    pub fn version_info(&self) -> Result<Value, RcError> {
        self.call("core/version", json!({}))
    }

    pub fn version(&self) -> Result<String, RcError> {
        Ok(backend_identity(&self.version_info()?).version)
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
        opt: Option<Value>,
    ) -> Result<Value, RcError> {
        self.call(
            "config/create",
            json!({
                "name": name,
                "type": r#type,
                "parameters": parameters,
                "opt": crate::command_options::merge_create_opt(opt)
            }),
        )
    }

    pub fn update_remote(
        &self,
        name: &str,
        parameters: Value,
        opt: Option<Value>,
    ) -> Result<Value, RcError> {
        let mut body = json!({ "name": name, "parameters": parameters });
        if let Some(opt) = opt {
            body["opt"] = opt;
        }
        self.call("config/update", body)
    }

    pub fn delete_remote(&self, name: &str) -> Result<Value, RcError> {
        self.call("config/delete", json!({ "name": name }))
    }

    pub fn clone_remote_config(&self, from: &str, to: &str) -> Result<Value, RcError> {
        let dump = self.dump_config()?;
        let Some(section) = dump.get(from).cloned() else {
            return Err(RcError::message(format!(
                "source remote {from} is not in rclone.conf"
            )));
        };
        let r#type = section
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if r#type.is_empty() {
            return Err(RcError::message(format!(
                "source remote {from} has no type"
            )));
        }
        let mut params = section;
        if let Some(obj) = params.as_object_mut() {
            obj.remove("type");
            obj.remove("name");
        }
        self.create_remote(to, &r#type, params, None)
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

    pub fn copy_url(
        &self,
        url: &str,
        fs: &str,
        remote: &str,
        auto_filename: bool,
    ) -> Result<u64, RcError> {
        self.start_job(
            "operations/copyurl",
            copy_url_payload(url, fs, remote, auto_filename),
        )
    }

    pub fn upload_file(
        &self,
        fs: &str,
        remote: &str,
        name: &str,
        content: &[u8],
    ) -> Result<Value, RcError> {
        match self.upload_file_multipart(fs, remote, name, content) {
            Ok(value) => Ok(value),
            Err(_) => {
                let tmp = std::env::temp_dir().join(format!("rm-upload-{name}"));
                std::fs::write(&tmp, content).map_err(|e| RcError::message(e.to_string()))?;
                let dest = upload_dest_path(remote, name);
                let result = self.copy_file("/", &tmp.to_string_lossy(), fs, &dest);
                let _ = std::fs::remove_file(&tmp);
                result
            }
        }
    }

    fn upload_file_multipart(
        &self,
        fs: &str,
        remote: &str,
        name: &str,
        content: &[u8],
    ) -> Result<Value, RcError> {
        let boundary = "----rclone-manager-gtk";
        let mut body = Vec::new();
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .into_bytes(),
        );
        body.extend(content);
        body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
        let url = format!(
            "{}/operations/uploadfile?fs={}&remote={}",
            self.base_url.trim_end_matches('/'),
            urlencoding::encode(fs),
            urlencoding::encode(remote)
        );
        let mut request = ureq::post(&url).timeout(self.timeout).set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        );
        if let (Some(user), Some(pass)) = (&self.user, &self.pass) {
            request = request.set("Authorization", &basic_auth_header(user, pass));
        }
        match request.send_bytes(&body) {
            Ok(resp) => {
                let text = resp.into_string().unwrap_or_default();
                if text.is_empty() {
                    return Ok(serde_json::json!({}));
                }
                serde_json::from_str(&text).or_else(|_| Ok(serde_json::json!({ "raw": text })))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let text = resp.into_string().unwrap_or_default();
                Err(RcError::Message(format!("HTTP {code}: {text}")))
            }
            Err(e) => Err(RcError::Http(e.to_string())),
        }
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

    pub fn fs_info(&self, fs: &str) -> Result<FsInfo, RcError> {
        Ok(FsInfo::from_value(
            &self.call("operations/fsinfo", json!({ "fs": fs }))?,
        ))
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

    pub fn archive_create(&self, opts: &ArchiveCreateOpts) -> Result<u64, RcError> {
        match self.start_job("operations/archive", archive_create_payload(opts)) {
            Ok(id) => Ok(id),
            Err(e) if looks_missing_endpoint(&e) => {
                let value = self.core_command("archive", archive_create_cli_args(opts), true)?;
                value.get("jobid").and_then(|x| x.as_u64()).ok_or(e)
            }
            Err(e) => Err(e),
        }
    }

    pub fn archive_extract(&self, src: &str, dst: &str) -> Result<u64, RcError> {
        match self.start_job(
            "operations/archive",
            json!({
                "action": "extract",
                "src": src,
                "dst": dst
            }),
        ) {
            Ok(id) => Ok(id),
            Err(e) if looks_missing_endpoint(&e) => {
                let value = self.core_command(
                    "archive",
                    vec!["extract".into(), src.to_string(), dst.to_string()],
                    true,
                )?;
                value.get("jobid").and_then(|x| x.as_u64()).ok_or(e)
            }
            Err(e) => Err(e),
        }
    }

    pub fn archive_list(&self, src: &str, long: bool) -> Result<Vec<ArchiveListItem>, RcError> {
        let v = self.call("operations/archive", archive_list_payload(src, long))?;
        if v.get("error").and_then(|x| x.as_bool()).unwrap_or(false) {
            let result = v.get("result").and_then(|x| x.as_str()).unwrap_or("");
            return Err(RcError::message(format!("Archive list failed: {result}")));
        }
        if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            return Ok(items
                .iter()
                .filter_map(ArchiveListItem::from_value)
                .collect());
        }
        let result = v.get("result").and_then(|x| x.as_str()).unwrap_or("");
        Ok(parse_archive_list(result, long))
    }

    pub fn public_link(&self, fs: &str, remote: &str) -> Result<String, RcError> {
        self.public_link_ex(fs, remote, None, false)
    }

    pub fn public_link_ex(
        &self,
        fs: &str,
        remote: &str,
        expire: Option<&str>,
        unlink: bool,
    ) -> Result<String, RcError> {
        let v = self.call(
            "operations/publiclink",
            public_link_payload(fs, remote, expire, unlink),
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

    pub fn hashsum_file(&self, fs: &str, remote: &str, hash_type: &str) -> Result<Value, RcError> {
        self.call(
            "operations/hashsumfile",
            json!({ "fs": fs, "remote": remote, "hashType": hash_type }),
        )
    }

    pub fn du(&self, dir: Option<&str>) -> Result<DiskUsage, RcError> {
        let params = match dir.filter(|d| !d.is_empty()) {
            Some(path) => json!({ "dir": path }),
            None => json!({}),
        };
        let value = self.call("core/du", params)?;
        parse_du(&value).ok_or_else(|| RcError::message("rclone du returned no usage info"))
    }

    pub fn config_paths(&self) -> Result<Value, RcError> {
        self.call("config/paths", json!({}))
    }

    pub fn config_is_encrypted(&self) -> Result<bool, RcError> {
        let value = self.call("config/isencrypted", json!({}))?;
        Ok(value
            .get("encrypted")
            .or_else(|| value.get("isEncrypted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    pub fn config_unlock(&self, password: &str) -> Result<Value, RcError> {
        self.call("config/unlock", json!({ "configPassword": password }))
    }

    pub fn cat(&self, fs: &str, remote: &str, count: Option<i64>) -> Result<String, RcError> {
        let mut params = json!({ "fs": fs, "remote": remote });
        if let Some(n) = count {
            params["count"] = json!(n);
        }
        let value = self.call("operations/cat", params)?;
        parse_cat_content(&value).ok_or_else(|| RcError::message("rclone cat returned no content"))
    }

    pub fn stat(&self, fs: &str, remote: &str) -> Result<Option<StatItem>, RcError> {
        let value = self.call("operations/stat", json!({ "fs": fs, "remote": remote }))?;
        Ok(parse_stat(&value))
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

    pub fn job_stop_group(&self, group: &str) -> Result<Value, RcError> {
        self.call("job/stopgroup", json!({ "group": group }))
    }

    pub fn stats(&self, group: Option<&str>) -> Result<Value, RcError> {
        match group {
            Some(g) => self.call("core/stats", json!({ "group": g })),
            None => self.call("core/stats", json!({})),
        }
    }

    pub fn transferred(&self, group: Option<&str>) -> Result<Value, RcError> {
        match group {
            Some(g) => self.call("core/transferred", json!({ "group": g })),
            None => self.call("core/transferred", json!({})),
        }
    }

    pub fn stats_delete(&self, group: Option<&str>) -> Result<Value, RcError> {
        match group {
            Some(g) => self.call("core/stats-delete", json!({ "group": g })),
            None => self.call("core/stats-delete", json!({})),
        }
    }

    pub fn batch(&self, inputs: &[Value]) -> Result<Value, RcError> {
        self.call("job/batch", json!({ "inputs": inputs }))
    }

    pub fn mount_types(&self) -> Result<Vec<String>, RcError> {
        let v = self.call("mount/types", json!({}))?;
        Ok(parse_named_list(&v, &["mountTypes", "types"]))
    }

    pub fn serve_types(&self) -> Result<Vec<String>, RcError> {
        let v = self.call("serve/types", json!({}))?;
        Ok(parse_named_list(&v, &["serveTypes", "types"]))
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
        self.vfs_forget_ex(fs, None)
    }

    pub fn vfs_forget_ex(&self, fs: &str, file: Option<&str>) -> Result<Value, RcError> {
        self.call("vfs/forget", vfs_forget_payload(fs, file))
    }

    pub fn vfs_refresh(&self, fs: &str) -> Result<Value, RcError> {
        self.vfs_refresh_ex(fs, None, false)
    }

    pub fn vfs_refresh_ex(
        &self,
        fs: &str,
        dir: Option<&str>,
        recursive: bool,
    ) -> Result<Value, RcError> {
        self.call("vfs/refresh", vfs_refresh_payload(fs, dir, recursive))
    }

    pub fn vfs_queue(&self, fs: &str) -> Result<Value, RcError> {
        self.call("vfs/queue", json!({ "fs": fs }))
    }

    pub fn vfs_poll_interval(&self, fs: &str, interval: Option<&str>) -> Result<Value, RcError> {
        let mut body = json!({ "fs": fs });
        if let Some(interval) = interval.filter(|s| !s.is_empty()) {
            body["interval"] = json!(interval);
        }
        self.call("vfs/poll-interval", body)
    }

    pub fn vfs_queue_set_expiry(&self, fs: &str, id: &str, expiry: &str) -> Result<Value, RcError> {
        self.vfs_queue_set_expiry_ex(fs, id, expiry, false)
    }

    pub fn vfs_queue_set_expiry_ex(
        &self,
        fs: &str,
        id: &str,
        expiry: &str,
        relative: bool,
    ) -> Result<Value, RcError> {
        self.call(
            "vfs/queue-set-expiry",
            vfs_queue_expiry_payload(fs, id, expiry, relative),
        )
    }

    pub fn fscache_clear(&self) -> Result<Value, RcError> {
        self.call("fscache/clear", json!({}))
    }

    pub fn fscache_entries(&self) -> Result<Value, RcError> {
        self.call("fscache/entries", json!({}))
    }

    pub fn copy_remotes_from(&self, source: &RcClient) -> Result<usize, RcError> {
        let dump = source.dump_config()?;
        let mut count = 0;
        if let Some(obj) = dump.as_object() {
            for (name, cfg) in obj {
                let r#type = cfg.get("type").and_then(|x| x.as_str()).unwrap_or("alias");
                let mut params = cfg.clone();
                if let Some(map) = params.as_object_mut() {
                    map.remove("type");
                }
                self.create_remote(name, r#type, params, None)?;
                count += 1;
            }
        }
        Ok(count)
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

    pub fn core_command(
        &self,
        command: &str,
        args: Vec<String>,
        async_job: bool,
    ) -> Result<Value, RcError> {
        self.call(
            "core/command",
            core_command_payload(command, args, async_job),
        )
    }

    pub fn config_validate_password(&self, password: &str) -> Result<Value, RcError> {
        self.call("config/validatepassword", json!({ "password": password }))
    }

    pub fn config_encrypt(&self, password: &str) -> Result<Value, RcError> {
        self.call("config/encrypt", json!({ "password": password }))
    }

    pub fn config_decrypt(&self, password: &str) -> Result<Value, RcError> {
        self.call("config/decrypt", json!({ "password": password }))
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FsInfo {
    pub name: String,
    pub root: String,
    pub precision: i64,
    pub hashes: Vec<String>,
    pub features: BTreeMap<String, bool>,
    pub metadata: Value,
}

impl FsInfo {
    pub fn from_value(v: &Value) -> Self {
        let hashes = v
            .get("Hashes")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let mut features = BTreeMap::new();
        if let Some(obj) = v.get("Features").and_then(|x| x.as_object()) {
            for (key, val) in obj {
                features.insert(key.clone(), val.as_bool().unwrap_or(false));
            }
        }
        Self {
            name: v
                .get("Name")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            root: v
                .get("Root")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            precision: v.get("Precision").and_then(|x| x.as_i64()).unwrap_or(0),
            hashes,
            features,
            metadata: v.get("MetadataInfo").cloned().unwrap_or(json!({})),
        }
    }

    pub fn has_feature(&self, name: &str) -> bool {
        self.features.get(name).copied().unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveListItem {
    pub path: String,
    pub is_dir: bool,
    pub size: i64,
    pub date: String,
    pub time: String,
}

impl ArchiveListItem {
    pub fn from_value(v: &Value) -> Option<Self> {
        let path = v
            .get("path")
            .or_else(|| v.get("Path"))
            .and_then(|x| x.as_str())?
            .to_string();
        let is_dir = v
            .get("isDir")
            .or_else(|| v.get("IsDir"))
            .and_then(|x| x.as_bool())
            .unwrap_or(path.ends_with('/'));
        Some(Self {
            path: path.trim_end_matches('/').to_string(),
            is_dir,
            size: v
                .get("size")
                .or_else(|| v.get("Size"))
                .and_then(|x| x.as_i64())
                .unwrap_or(0),
            date: v
                .get("date")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            time: v
                .get("time")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
        })
    }

    pub fn subtitle(&self) -> String {
        if self.is_dir {
            format!("folder · {} {}", self.date, self.time)
                .trim()
                .to_string()
        } else {
            format!("{} · {} {}", format_bytes(self.size), self.date, self.time)
                .trim()
                .to_string()
        }
    }
}

pub fn archive_list_payload(src: &str, long: bool) -> Value {
    json!({ "action": "list", "src": src, "long": long })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveCreateOpts {
    pub src: String,
    pub dst: String,
    pub format: Option<String>,
    pub prefix: Option<String>,
    pub full_path: bool,
    pub include: Vec<String>,
}

pub fn archive_create_payload(opts: &ArchiveCreateOpts) -> Value {
    let mut body = json!({
        "action": "create",
        "src": opts.src,
        "dst": opts.dst,
    });
    if let Some(obj) = body.as_object_mut() {
        if let Some(format) = opts
            .format
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert("format".into(), json!(format));
        }
        if let Some(prefix) = opts
            .prefix
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert("prefix".into(), json!(prefix));
        }
        if opts.full_path {
            obj.insert("full_path".into(), json!(true));
        }
        if !opts.include.is_empty() {
            obj.insert("include".into(), json!(opts.include));
        }
    }
    body
}

pub fn archive_create_opts_from_payload(params: &Value) -> ArchiveCreateOpts {
    ArchiveCreateOpts {
        src: params
            .get("src")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        dst: params
            .get("dst")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        format: params
            .get("format")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        prefix: params
            .get("prefix")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        full_path: params
            .get("full_path")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        include: params
            .get("include")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

pub fn archive_create_cli_args(opts: &ArchiveCreateOpts) -> Vec<String> {
    let mut args = vec!["create".into(), opts.src.clone(), opts.dst.clone()];
    if let Some(format) = opts
        .format
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push(format!("--format={format}"));
    }
    if let Some(prefix) = opts
        .prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push(format!("--prefix={prefix}"));
    }
    if opts.full_path {
        args.push("--full-path".into());
    }
    for item in &opts.include {
        if !item.is_empty() {
            args.push(format!("--include={item}"));
        }
    }
    args
}

pub fn core_command_payload(command: &str, args: Vec<String>, async_job: bool) -> Value {
    let mut body = json!({
        "command": command,
        "arg": args,
    });
    if async_job {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("_async".into(), json!(true));
            obj.insert("returnType".into(), json!("STREAM"));
        }
    }
    body
}

pub fn copy_url_payload(url: &str, fs: &str, remote: &str, auto_filename: bool) -> Value {
    json!({
        "url": url,
        "fs": fs,
        "remote": remote,
        "autoFilename": auto_filename,
    })
}

pub fn looks_missing_endpoint(err: &RcError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("couldn't find method")
        || text.contains("unknown method")
        || (text.contains("404") && text.contains("method"))
}

pub fn batch_input(path: &str, params: Value) -> Value {
    let mut obj = params.as_object().cloned().unwrap_or_default();
    obj.insert("_path".into(), json!(path));
    Value::Object(obj)
}

pub fn parse_batch_results(value: &Value) -> Vec<Value> {
    value
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

pub fn parse_named_list(value: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(arr) = value.get(*key).and_then(|v| v.as_array()) {
            return arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
    }
    if let Some(arr) = value.as_array() {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }
    Vec::new()
}

pub fn public_link_payload(fs: &str, remote: &str, expire: Option<&str>, unlink: bool) -> Value {
    let mut body = json!({ "fs": fs, "remote": remote });
    if let Some(obj) = body.as_object_mut() {
        if let Some(exp) = expire.filter(|s| !s.is_empty()) {
            obj.insert("expire".into(), json!(exp));
        }
        if unlink {
            obj.insert("unlink".into(), json!(true));
        }
    }
    body
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatItem {
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    pub mime: String,
}

pub const CAT_PREVIEW_BYTES: i64 = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskUsage {
    pub dir: String,
    pub total: i64,
    pub free: i64,
    pub used: i64,
}

/// Object count and byte total from `operations/size`.
pub fn parse_object_size(value: &Value) -> (i64, i64) {
    let count = value.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
    let bytes = value.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0);
    (count, bytes)
}

pub fn parse_du(value: &Value) -> Option<DiskUsage> {
    let info = value.get("info").unwrap_or(value);
    let total = info
        .get("Total")
        .or_else(|| info.get("total"))
        .and_then(|v| v.as_i64())?;
    let free = info
        .get("Free")
        .or_else(|| info.get("free"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Some(DiskUsage {
        dir: value
            .get("dir")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        total,
        free,
        used: total.saturating_sub(free),
    })
}

pub fn upload_dest_path(remote_dir: &str, name: &str) -> String {
    if remote_dir.is_empty() || remote_dir == "/" {
        name.to_string()
    } else if remote_dir.ends_with('/') {
        format!("{remote_dir}{name}")
    } else {
        format!("{remote_dir}/{name}")
    }
}

pub fn parse_cat_content(value: &Value) -> Option<String> {
    if let Some(text) = value.get("content").and_then(|v| v.as_str()) {
        return Some(text.to_string());
    }
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    None
}

pub fn parse_stat(value: &Value) -> Option<StatItem> {
    let item = value.get("item")?;
    if item.is_null() {
        return None;
    }
    Some(StatItem {
        name: item
            .get("Name")
            .or_else(|| item.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        is_dir: item
            .get("IsDir")
            .or_else(|| item.get("isDir"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        size: item
            .get("Size")
            .or_else(|| item.get("size"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        mime: item
            .get("MimeType")
            .or_else(|| item.get("mimeType"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

pub fn vfs_forget_payload(fs: &str, file: Option<&str>) -> Value {
    let mut body = json!({ "fs": fs });
    if let Some(file) = file.filter(|s| !s.is_empty()) {
        body["file"] = json!(file);
    }
    body
}

pub fn vfs_refresh_payload(fs: &str, dir: Option<&str>, recursive: bool) -> Value {
    let mut body = json!({ "fs": fs, "recursive": recursive });
    if let Some(dir) = dir.filter(|s| !s.is_empty()) {
        body["dir"] = json!(dir);
    }
    body
}

pub fn vfs_queue_expiry_payload(fs: &str, id: &str, expiry: &str, relative: bool) -> Value {
    json!({ "fs": fs, "id": id, "expiry": expiry, "relative": relative })
}

pub fn parse_hashsum(value: &Value) -> Option<String> {
    if let Some(s) = value.get("hash").and_then(|x| x.as_str()) {
        let token = s.lines().next().unwrap_or(s);
        let token = token.split_whitespace().next().unwrap_or(token).trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    if let Some(arr) = value.get("hashsum").and_then(|x| x.as_array()) {
        if let Some(first) = arr.iter().find_map(|x| x.as_str()) {
            let token = first.split_whitespace().next().unwrap_or(first).trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

pub fn parse_archive_list(result: &str, long: bool) -> Vec<ArchiveListItem> {
    let mut items = Vec::new();
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if long {
            let parts = split_n_robust(trimmed, 4);
            if parts.len() >= 4 {
                let size = parts[0].parse::<i64>().unwrap_or(0);
                let date = parts[1].to_string();
                let time = parts[2].split('.').next().unwrap_or(parts[2]).to_string();
                let path = parts[3];
                let is_dir = path.ends_with('/');
                items.push(ArchiveListItem {
                    path: path.trim_end_matches('/').to_string(),
                    is_dir,
                    size,
                    date,
                    time,
                });
                continue;
            }
        } else {
            let parts = split_n_robust(trimmed, 2);
            if parts.len() >= 2 && parts[0].parse::<i64>().is_ok() {
                let path = parts[1];
                items.push(ArchiveListItem {
                    path: path.trim_end_matches('/').to_string(),
                    is_dir: path.ends_with('/'),
                    size: parts[0].parse().unwrap_or(0),
                    date: String::new(),
                    time: String::new(),
                });
                continue;
            }
        }
        items.push(ArchiveListItem {
            path: trimmed.trim_end_matches('/').to_string(),
            is_dir: trimmed.ends_with('/'),
            ..ArchiveListItem::default()
        });
    }
    items
}

fn split_n_robust(s: &str, n: usize) -> Vec<&str> {
    let mut parts = Vec::with_capacity(n);
    let mut current = s;
    for _ in 0..n.saturating_sub(1) {
        let trimmed = current.trim_start();
        if let Some(pos) = trimmed.find(|c: char| c.is_whitespace()) {
            parts.push(&trimmed[..pos]);
            current = &trimmed[pos..];
        } else {
            break;
        }
    }
    let final_part = current.trim_start();
    if !final_part.is_empty() {
        parts.push(final_part);
    }
    parts
}

pub fn browse_target(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "—" {
        return None;
    }
    Some(split_remote_path(trimmed))
}

pub fn nanoseconds_to_duration(ns: i64) -> String {
    if ns <= 0 {
        return "—".into();
    }
    if ns % 1_000_000_000 == 0 {
        format!("{}s", ns / 1_000_000_000)
    } else if ns % 1_000_000 == 0 {
        format!("{}ms", ns / 1_000_000)
    } else {
        format!("{ns}ns")
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendIdentity {
    pub version: String,
    pub os: String,
    pub arch: String,
}

impl BackendIdentity {
    pub fn summary(&self) -> String {
        format!("{} · {}/{}", self.version, self.os, self.arch)
    }
}

pub fn backend_identity(info: &Value) -> BackendIdentity {
    BackendIdentity {
        version: info
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string(),
        os: info
            .get("os")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string(),
        arch: info
            .get("arch")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string(),
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
    fn vfs_poll_payload_includes_interval() {
        let body = json!({ "fs": "drive:", "interval": "1m" });
        assert_eq!(body["interval"], "1m");
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

    #[test]
    fn backend_identity_from_core_version() {
        let id = backend_identity(&json!({
            "version": "v1.68.2",
            "os": "linux",
            "arch": "amd64"
        }));
        assert_eq!(id.summary(), "v1.68.2 · linux/amd64");
        assert_eq!(backend_identity(&json!({})).version, "unknown");
    }

    #[test]
    fn fsinfo_parses_features_and_hashes() {
        let info = FsInfo::from_value(&json!({
            "Name": "drive",
            "Root": "Photos",
            "Precision": 1_000_000_000,
            "Hashes": ["md5", "sha1"],
            "Features": { "PublicLink": true, "CleanUp": false, "IsLocal": false },
            "MetadataInfo": { "System": { "mtime": { "Help": "mod time" } } }
        }));
        assert_eq!(info.name, "drive");
        assert_eq!(info.root, "Photos");
        assert!(info.has_feature("PublicLink"));
        assert!(!info.has_feature("CleanUp"));
        assert!(!info.has_feature("Missing"));
        assert_eq!(info.hashes, vec!["md5", "sha1"]);
        assert_eq!(nanoseconds_to_duration(info.precision), "1s");
        assert!(info.metadata.get("System").is_some());
    }

    #[test]
    fn archive_list_parses_long_and_plain() {
        let long = parse_archive_list(
            "6 2025-10-30 09:46:23.000000000 file.txt\n0 2025-10-30 09:46:23.000000000 nested/\n",
            true,
        );
        assert_eq!(long.len(), 2);
        assert_eq!(long[0].path, "file.txt");
        assert_eq!(long[0].size, 6);
        assert_eq!(long[0].date, "2025-10-30");
        assert_eq!(long[0].time, "09:46:23");
        assert!(long[1].is_dir);
        assert_eq!(long[1].path, "nested");
        let short = parse_archive_list("12 readme.md\ndocs/\n", false);
        assert_eq!(short[0].size, 12);
        assert!(short[1].is_dir);
        assert_eq!(
            archive_list_payload("drive:pack.zip", true),
            json!({ "action": "list", "src": "drive:pack.zip", "long": true })
        );
    }

    #[test]
    fn hashsum_and_public_link_helpers() {
        assert_eq!(
            parse_hashsum(&json!({ "hash": "abc123  file.txt" })).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            parse_hashsum(&json!({ "hashsum": ["deadbeef file.bin"] })).as_deref(),
            Some("deadbeef")
        );
        assert_eq!(
            public_link_payload("drive:", "Photos/a.jpg", Some("1d"), false),
            json!({ "fs": "drive:", "remote": "Photos/a.jpg", "expire": "1d" })
        );
        assert_eq!(
            public_link_payload("drive:", "Photos/a.jpg", None, true),
            json!({ "fs": "drive:", "remote": "Photos/a.jpg", "unlink": true })
        );
        assert_eq!(
            browse_target("drive:Photos/a.jpg"),
            Some(("drive".into(), "Photos/a.jpg".into()))
        );
        assert_eq!(browse_target("  "), None);
        assert_eq!(
            browse_target("/tmp/out"),
            Some(("local".into(), "/tmp/out".into()))
        );
        assert_eq!(
            batch_input("job/status", json!({ "jobid": 1 })),
            json!({ "_path": "job/status", "jobid": 1 })
        );
        assert_eq!(
            parse_cat_content(&json!({ "content": "hello" })).as_deref(),
            Some("hello")
        );
        assert_eq!(parse_cat_content(&json!("plain")).as_deref(), Some("plain"));
        assert_eq!(parse_cat_content(&json!({ "hash": "abc" })), None);
        assert_eq!(CAT_PREVIEW_BYTES, 512 * 1024);
        let du = parse_du(&json!({
            "dir": "/home",
            "info": { "Total": 1000, "Free": 400 }
        }))
        .unwrap();
        assert_eq!(du.used, 600);
        assert_eq!(du.dir, "/home");
        assert_eq!(upload_dest_path("docs", "a.txt"), "docs/a.txt");
        assert_eq!(upload_dest_path("docs/", "a.txt"), "docs/a.txt");
        assert_eq!(upload_dest_path("", "a.txt"), "a.txt");
        assert_eq!(
            parse_batch_results(&json!({ "results": [{ "ok": true }] })).len(),
            1
        );
        assert_eq!(
            parse_named_list(
                &json!({ "mountTypes": ["mount", "cmount"] }),
                &["mountTypes"]
            ),
            vec!["mount", "cmount"]
        );
    }

    #[test]
    fn parse_object_size_reads_count_and_bytes() {
        assert_eq!(
            parse_object_size(&json!({ "count": 12, "bytes": 4096 })),
            (12, 4096)
        );
        assert_eq!(parse_object_size(&json!({})), (0, 0));
        assert_eq!(parse_object_size(&json!({ "count": "x" })), (0, 0));
    }

    #[test]
    fn parses_stat_item_and_null() {
        let item = parse_stat(&json!({
            "item": { "Name": "Photos", "IsDir": true, "Size": 0, "MimeType": "inode/directory" }
        }))
        .unwrap();
        assert!(item.is_dir);
        assert_eq!(item.name, "Photos");
        assert_eq!(item.mime, "inode/directory");
        assert!(parse_stat(&json!({ "item": null })).is_none());
    }

    #[test]
    fn vfs_payloads_include_optional_fields() {
        assert_eq!(
            vfs_forget_payload("drive:", None),
            json!({ "fs": "drive:" })
        );
        assert_eq!(
            vfs_forget_payload("drive:", Some("cache.bin")),
            json!({ "fs": "drive:", "file": "cache.bin" })
        );
        assert_eq!(
            vfs_refresh_payload("drive:", Some("Photos"), true),
            json!({ "fs": "drive:", "recursive": true, "dir": "Photos" })
        );
        assert_eq!(
            vfs_queue_expiry_payload("drive:", "3", "1m", true),
            json!({ "fs": "drive:", "id": "3", "expiry": "1m", "relative": true })
        );
    }

    #[test]
    fn archive_create_payload_matches_tauri() {
        let opts = ArchiveCreateOpts {
            src: "drive:Photos".into(),
            dst: "drive:Photos/pack.zip".into(),
            format: Some("zip".into()),
            prefix: Some("backup".into()),
            full_path: true,
            include: vec!["a.txt".into(), "b".into()],
        };
        let payload = archive_create_payload(&opts);
        assert_eq!(payload["action"], "create");
        assert_eq!(payload["src"], "drive:Photos");
        assert_eq!(payload["dst"], "drive:Photos/pack.zip");
        assert_eq!(payload["format"], "zip");
        assert_eq!(payload["prefix"], "backup");
        assert_eq!(payload["full_path"], true);
        assert_eq!(payload["include"], json!(["a.txt", "b"]));
        let args = archive_create_cli_args(&opts);
        assert_eq!(args[0], "create");
        assert!(args.iter().any(|a| a == "--format=zip"));
        assert!(args.iter().any(|a| a == "--full-path"));
        assert!(args.iter().any(|a| a == "--include=a.txt"));
        let empty = archive_create_payload(&ArchiveCreateOpts {
            src: "a:".into(),
            dst: "a:out.zip".into(),
            ..ArchiveCreateOpts::default()
        });
        assert!(empty.get("format").is_none());
        assert!(empty.get("include").is_none());
        assert!(empty.get("full_path").is_none());
    }

    #[test]
    fn copy_url_and_core_command_payloads() {
        assert_eq!(
            copy_url_payload("https://ex/a.bin", "drive:Inbox", "", true),
            json!({
                "url": "https://ex/a.bin",
                "fs": "drive:Inbox",
                "remote": "",
                "autoFilename": true
            })
        );
        assert_eq!(
            copy_url_payload("https://ex/a.bin", "drive:", "Inbox/a.bin", false)["autoFilename"],
            false
        );
        let cmd = core_command_payload("archive", vec!["list".into(), "a.zip".into()], true);
        assert_eq!(cmd["command"], "archive");
        assert_eq!(cmd["arg"], json!(["list", "a.zip"]));
        assert_eq!(cmd["_async"], true);
        assert_eq!(cmd["returnType"], "STREAM");
        assert!(looks_missing_endpoint(&RcError::message(
            "couldn't find method operations/archive"
        )));
        assert!(!looks_missing_endpoint(&RcError::message("path not found")));
    }
}
