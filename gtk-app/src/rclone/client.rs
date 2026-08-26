//! rclone Remote Control HTTP client.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use thiserror::Error;

fn missing_rc_methods() -> &'static Mutex<HashSet<String>> {
    static SET: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

fn missing_method_key(base: &str, endpoint: &str) -> String {
    format!("{base}\0{endpoint}")
}

fn is_cached_missing_method(base: &str, endpoint: &str) -> bool {
    missing_rc_methods()
        .lock()
        .map(|set| set.contains(&missing_method_key(base, endpoint)))
        .unwrap_or(false)
}

fn cache_missing_method(base: &str, endpoint: &str) {
    if let Ok(mut set) = missing_rc_methods().lock() {
        set.insert(missing_method_key(base, endpoint));
    }
}

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

    /// Base URL with optional HTTP basic credentials for `--rc-serve` media URLs.
    pub fn authenticated_base_url(&self) -> String {
        authenticated_base_url(&self.base_url, self.user.as_deref(), self.pass.as_deref())
    }

    /// HTTP URL used by rclone `--rc-serve` to stream a remote file.
    ///
    /// Matches Tauri `build_file_url`: `{base}/[{remote}:]/{encoded/path}`.
    pub fn rc_serve_url(&self, remote: &str, path: &str) -> String {
        build_rc_serve_url(&self.authenticated_base_url(), remote, path)
    }

    /// Probe whether `--rc-serve` can return the start of a file (Range 0-1).
    pub fn probe_rc_serve(&self, url: &str) -> bool {
        if url.is_empty() {
            return false;
        }
        let mut request = ureq::get(url)
            .timeout(Duration::from_secs(5))
            .set("Range", "bytes=0-1");
        if let (Some(user), Some(pass)) = (&self.user, &self.pass) {
            request = request.set("Authorization", &basic_auth_header(user, pass));
        }
        match request.call() {
            Ok(resp) => matches!(resp.status(), 200 | 206),
            Err(ureq::Error::Status(code, _)) => matches!(code, 200 | 206),
            Err(_) => false,
        }
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
        let endpoint = endpoint.trim_start_matches('/');
        if is_cached_missing_method(&self.base_url, endpoint) {
            return Err(RcError::message(format!(
                "couldn't find method \"{endpoint}\""
            )));
        }
        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint);
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
                let err = RcError::Message(format!("HTTP {code}: {message}"));
                if looks_missing_endpoint(&err) {
                    cache_missing_method(&self.base_url, endpoint);
                }
                Err(err)
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
        self.call("config/unlock", config_unlock_payload(password))
    }

    pub fn config_setpath(&self, path: &str) -> Result<Value, RcError> {
        self.call("config/setpath", config_setpath_payload(path))
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
        if host_fuse_mounted(mount_point) {
            let _ = host_unmount(mount_point);
        }
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
        match self.call("mount/unmount", json!({ "mountPoint": mount_point })) {
            Ok(value) => Ok(value),
            Err(err) => {
                if host_unmount(mount_point).is_ok() {
                    Ok(json!({ "unmounted": mount_point }))
                } else {
                    Err(err)
                }
            }
        }
    }

    pub fn unmount_all(&self) -> Result<Value, RcError> {
        self.call("mount/unmountall", json!({}))
    }

    pub fn list_mounts(&self) -> Result<Vec<MountedRemote>, RcError> {
        let mut mounts = self
            .call("mount/listmounts", json!({}))
            .map(|v| parse_mount_points(&v))
            .unwrap_or_default();
        merge_host_mounts(&mut mounts);
        Ok(mounts)
    }

    pub fn serve_start(&self, serve_type: &str, fs: &str, addr: &str) -> Result<Value, RcError> {
        self.serve_start_ex(serve_type, fs, addr, &json!({}))
    }

    pub fn serve_start_ex(
        &self,
        serve_type: &str,
        fs: &str,
        addr: &str,
        extra: &Value,
    ) -> Result<Value, RcError> {
        let addr = super::serve::resolve_listen_addr(addr);
        let payload = super::serve::merge_serve_payload(serve_type, fs, &addr, extra);
        match self.call("serve/start", payload) {
            Ok(value) => {
                let id = value.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                let listen = value.get("addr").and_then(|v| v.as_str()).unwrap_or(&addr);
                Ok(super::serve::serve_start_response(id, listen))
            }
            Err(err) if looks_missing_endpoint(&err) => {
                super::serve::start_serve_fallback(self, serve_type, fs, &addr, extra, err)
            }
            Err(err) => Err(err),
        }
    }

    pub fn serve_stop(&self, id: &str) -> Result<Value, RcError> {
        if super::serve::stop_legacy_serve(Some(self), id) {
            return Ok(json!({ "id": id }));
        }
        match self.call("serve/stop", json!({ "id": id })) {
            Ok(value) => Ok(value),
            Err(err) if looks_missing_endpoint(&err) => {
                if let Some(jobid) = super::serve::parse_fallback_job_id(id) {
                    self.job_stop(jobid)
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    pub fn serve_stop_all(&self) -> Result<Value, RcError> {
        super::serve::shutdown_legacy(Some(self));
        match self.call("serve/stopall", json!({})) {
            Ok(value) => Ok(value),
            Err(err) if looks_missing_endpoint(&err) => Ok(json!({ "ok": true })),
            Err(err) => Err(err),
        }
    }

    pub fn serve_list(&self) -> Result<Vec<ServeItem>, RcError> {
        let legacy = super::serve::reap_legacy_serves(Some(self));
        match self.call("serve/list", json!({})) {
            Ok(value) => {
                let list = value
                    .get("list")
                    .and_then(|x| x.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut items: Vec<ServeItem> = list
                    .iter()
                    .filter_map(|item| ServeItem::from_rc(item))
                    .collect();
                for item in legacy {
                    if !items.iter().any(|existing| existing.id == item.id) {
                        items.push(item);
                    }
                }
                Ok(items)
            }
            Err(err) if looks_missing_endpoint(&err) || !legacy.is_empty() => Ok(legacy),
            Err(err) => Err(err),
        }
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

    pub fn options_blocks(&self) -> Result<Vec<String>, RcError> {
        let value = self.call("options/blocks", json!({}))?;
        Ok(crate::flags::parse_options_blocks(&value))
    }

    /// `options/info` plus empty groups named by `options/blocks`.
    pub fn option_flag_blocks(&self) -> Vec<crate::flags::FlagBlock> {
        crate::flags::option_blocks_from_rc(
            &self.options_info().unwrap_or(json!({})),
            &self
                .options_blocks()
                .map(|names| json!({ "options": names }))
                .unwrap_or(json!({})),
        )
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

    pub fn group_list(&self) -> Result<Value, RcError> {
        self.call("core/group-list", json!({}))
    }

    pub fn pid(&self) -> Result<u64, RcError> {
        let value = self.call("core/pid", json!({}))?;
        value
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| RcError::message("rclone did not return a pid"))
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

#[derive(Debug, Clone, Default)]
pub struct MountedRemote {
    pub fs: String,
    pub mount_point: String,
    pub profile: String,
    pub quick_run_id: String,
    pub origin: String,
}

impl MountedRemote {
    pub fn new(fs: impl Into<String>, mount_point: impl Into<String>) -> Self {
        Self {
            fs: fs.into(),
            mount_point: mount_point.into(),
            ..Default::default()
        }
    }
}

/// Carry profile/origin across RC refreshes (rclone has no profile field).
pub fn merge_mount_context(
    incoming: Vec<MountedRemote>,
    existing: &[MountedRemote],
) -> Vec<MountedRemote> {
    incoming
        .into_iter()
        .map(|mut mount| {
            if let Some(prev) = existing
                .iter()
                .find(|item| item.mount_point == mount.mount_point)
            {
                if mount.profile.is_empty() {
                    mount.profile = prev.profile.clone();
                }
                if mount.quick_run_id.is_empty() {
                    mount.quick_run_id = prev.quick_run_id.clone();
                }
                if mount.origin.is_empty() {
                    mount.origin = prev.origin.clone();
                }
            }
            mount
        })
        .collect()
}

/// rclone ≥1.64 returns `{ mountPoint: fs }`. 1.60 returns
/// `[{ Fs, MountPoint, MountedOn }, ...]`.
pub fn parse_mount_points(value: &Value) -> Vec<MountedRemote> {
    let Some(points) = value.get("mountPoints") else {
        return Vec::new();
    };
    if let Some(obj) = points.as_object() {
        return obj
            .iter()
            .map(|(point, fs)| MountedRemote::new(fs.as_str().unwrap_or_default(), point.clone()))
            .collect();
    }
    let Some(arr) = points.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let mount_point = item
                .get("MountPoint")
                .or_else(|| item.get("mountPoint"))
                .and_then(|v| v.as_str())?
                .to_string();
            let fs = item
                .get("Fs")
                .or_else(|| item.get("fs"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(MountedRemote::new(fs, mount_point))
        })
        .collect()
}

fn unescape_mount_field(value: &str) -> String {
    value.replace("\\040", " ").replace("\\011", "\t")
}

/// `/proc/mounts` fuse.rclone rows (covers mounts owned by a previous rcd).
pub fn parse_proc_mounts(text: &str) -> Vec<MountedRemote> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let fs = parts.next()?;
            let point = parts.next()?;
            let fstype = parts.next().unwrap_or("");
            if fstype != "fuse.rclone" {
                return None;
            }
            Some(MountedRemote::new(
                unescape_mount_field(fs),
                unescape_mount_field(point),
            ))
        })
        .collect()
}

fn merge_host_mounts(mounts: &mut Vec<MountedRemote>) {
    let Ok(text) = std::fs::read_to_string("/proc/mounts") else {
        return;
    };
    for extra in parse_proc_mounts(&text) {
        if !mounts.iter().any(|m| m.mount_point == extra.mount_point) {
            mounts.push(extra);
        }
    }
}

pub fn host_fuse_mounts() -> Vec<MountedRemote> {
    std::fs::read_to_string("/proc/mounts")
        .map(|text| parse_proc_mounts(&text))
        .unwrap_or_default()
}

pub fn host_fuse_mounted(mount_point: &str) -> bool {
    let want = mount_point.trim_end_matches('/');
    host_fuse_mounts()
        .iter()
        .any(|m| m.mount_point.trim_end_matches('/') == want)
}

pub fn host_unmount(mount_point: &str) -> Result<(), String> {
    for (bin, args) in [
        ("fusermount3", &["-u", mount_point] as &[&str]),
        ("fusermount", &["-u", mount_point]),
        ("umount", &[mount_point]),
    ] {
        match std::process::Command::new(bin).args(args).status() {
            Ok(status) if status.success() => return Ok(()),
            _ => continue,
        }
    }
    Err(format!("failed to unmount {mount_point}"))
}

#[derive(Debug, Clone, Default)]
pub struct ServeItem {
    pub id: String,
    pub addr: String,
    pub fs: String,
    pub serve_type: String,
    pub origin: String,
    pub profile: String,
    pub option_count: usize,
}

impl ServeItem {
    pub fn from_rc(item: &Value) -> Option<Self> {
        let params = item.get("params").cloned().unwrap_or(json!({}));
        let option_count = params
            .as_object()
            .map(|obj| {
                obj.keys()
                    .filter(|key| {
                        !matches!(
                            key.as_str(),
                            "fs" | "type" | "origin" | "profile" | "url" | "addr"
                        )
                    })
                    .count()
            })
            .unwrap_or(0);
        Some(Self {
            id: item.get("id")?.as_str()?.to_string(),
            addr: item
                .get("addr")
                .or_else(|| params.get("addr"))
                .or_else(|| params.get("url"))
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            fs: params
                .get("fs")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            serve_type: params
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            origin: params
                .get("origin")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            profile: params
                .get("profile")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            option_count,
        })
    }

    pub fn url(&self) -> String {
        let addr = self.addr.trim();
        if addr.is_empty() {
            return String::new();
        }
        if addr.contains("://") {
            return addr.to_string();
        }
        let scheme = match self.serve_type.to_ascii_lowercase().as_str() {
            "sftp" => "sftp",
            "ftp" => "ftp",
            "webdav" | "http" | "dlna" | "restic" | "s3" | "nfs" => "http",
            other if other.contains("https") => "https",
            _ => "http",
        };
        format!("{scheme}://{addr}")
    }
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

/// Count entries from `fscache/entries` (array, `{entries:[…]}`, or `{count:n}`).
pub fn parse_fscache_entry_count(value: &Value) -> usize {
    if let Some(n) = value.get("count").and_then(|v| v.as_u64()) {
        return n as usize;
    }
    if let Some(arr) = value
        .get("entries")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array())
    {
        return arr.len();
    }
    0
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

pub fn config_setpath_payload(path: &str) -> Value {
    json!({ "path": path })
}

pub fn config_unlock_payload(password: &str) -> Value {
    json!({ "configPassword": password })
}

/// Point a running RC at a rclone.conf and unlock it (Tauri `configure_remote_backend`).
pub fn apply_backend_rc_config(
    client: &RcClient,
    config_path: Option<&str>,
    password: Option<&str>,
) {
    if let Some(path) = config_path.map(str::trim).filter(|p| !p.is_empty()) {
        if let Err(e) = client.config_setpath(path) {
            log::warn!("config/setpath failed: {e}");
        }
    }
    if let Some(pass) = password.map(str::trim).filter(|p| !p.is_empty()) {
        if let Err(e) = client.config_unlock(pass) {
            log::warn!("config/unlock failed: {e}");
        }
    }
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
    pub mod_time: String,
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
        mod_time: item
            .get("ModTime")
            .or_else(|| item.get("modTime"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

pub fn format_mod_time(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => raw.to_string(),
    }
}

/// Angular `formatRelativeDate` equivalent for listing and properties.
pub fn format_relative_mod_time(raw: &str) -> String {
    let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return format_mod_time(raw);
    };
    let local = dt.with_timezone(&chrono::Local);
    let delta = chrono::Local::now().signed_duration_since(local);
    let secs = delta.num_seconds();
    if secs < 0 {
        return local.format("%Y-%m-%d %H:%M").to_string();
    }
    if secs < 60 {
        return "Just now".into();
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d ago");
    }
    local.format("%Y-%m-%d").to_string()
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
    let expiry = expiry
        .parse::<i64>()
        .map(Value::from)
        .or_else(|_| expiry.parse::<f64>().map(|n| json!(n)))
        .unwrap_or_else(|_| json!(expiry));
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

/// Angular `calculateBulkHash` result: join the full `hashsum[]` listing.
pub fn parse_hashsum_list(value: &Value) -> Option<String> {
    if let Some(arr) = value.get("hashsum").and_then(|x| x.as_array()) {
        let lines: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
        if !lines.is_empty() {
            return Some(lines.join("\n"));
        }
    }
    if let Some(s) = value.get("hash").and_then(|x| x.as_str()) {
        if !s.trim().is_empty() {
            return Some(s.to_string());
        }
    }
    None
}

/// Angular `expiryOptions`: Never / 1h / 1d / 7d / 30d.
pub const PUBLIC_LINK_EXPIRY_VALUES: [&str; 5] = ["", "1h", "1d", "7d", "30d"];

pub fn public_link_expiry_value(index: u32) -> Option<&'static str> {
    PUBLIC_LINK_EXPIRY_VALUES.get(index as usize).copied()
}

/// Angular `metadataGroups`: System keys vs other MetadataInfo object groups.
pub fn group_metadata_info(metadata: &Value) -> (Vec<(String, Value)>, Vec<(String, Value)>) {
    let Some(obj) = metadata.as_object() else {
        return (Vec::new(), Vec::new());
    };
    let mut system = Vec::new();
    let mut standard = Vec::new();
    if let Some(sys) = obj.get("System").and_then(|x| x.as_object()) {
        for (key, meta) in sys {
            system.push((key.clone(), meta.clone()));
        }
    }
    for (group_name, data) in obj {
        if group_name.eq_ignore_ascii_case("System") || group_name.eq_ignore_ascii_case("Help") {
            continue;
        }
        if let Some(items) = data.as_object() {
            let looks_grouped = items.values().any(|v| {
                v.is_object()
                    && (v.get("Help").is_some()
                        || v.get("help").is_some()
                        || v.get("Type").is_some()
                        || v.get("type").is_some())
            });
            if looks_grouped {
                for (key, meta) in items {
                    standard.push((key.clone(), meta.clone()));
                }
            } else {
                standard.push((group_name.clone(), data.clone()));
            }
        }
    }
    (system, standard)
}

/// Files grid caption: folders show relative date; files show `size · date`.
pub fn listing_caption(is_dir: bool, size: i64, mod_time: &str) -> String {
    let relative = format_relative_mod_time(mod_time);
    if is_dir {
        return relative;
    }
    if relative.is_empty() {
        format_bytes(size)
    } else {
        format!("{} · {relative}", format_bytes(size))
    }
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

/// Build an rclone `--rc-serve` file URL. Path segments are encoded individually
/// and the remote name gets a trailing `:` unless it already looks like an fs.
pub fn build_rc_serve_url(base_url: &str, remote: &str, path: &str) -> String {
    let fs_name = if remote.contains(':') || remote.contains('/') || remote.contains('\\') {
        remote.to_string()
    } else {
        format!("{remote}:")
    };
    let encoded_path = path
        .split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "{}/[{fs_name}]/{}",
        base_url.trim_end_matches('/'),
        encoded_path.trim_start_matches('/')
    )
}

pub fn authenticated_base_url(base_url: &str, user: Option<&str>, pass: Option<&str>) -> String {
    let base = base_url.trim_end_matches('/');
    match (user, pass) {
        (Some(user), Some(pass)) if !user.is_empty() => {
            let auth = format!(
                "{}:{}",
                urlencoding::encode(user),
                urlencoding::encode(pass)
            );
            if let Some(rest) = base.strip_prefix("http://") {
                format!("http://{auth}@{rest}")
            } else if let Some(rest) = base.strip_prefix("https://") {
                format!("https://{auth}@{rest}")
            } else {
                base.to_string()
            }
        }
        _ => base.to_string(),
    }
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

pub fn format_eta_seconds(seconds: i64) -> String {
    if seconds <= 0 {
        return "—".into();
    }
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 || hours > 0 {
        parts.push(format!("{minutes}m"));
    }
    parts.push(format!("{secs}s"));
    parts.join(" ")
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
    pub go: String,
    pub is_beta: bool,
    pub is_git: bool,
}

impl BackendIdentity {
    pub fn summary(&self) -> String {
        format!("{} · {}/{}", self.version, self.os, self.arch)
    }

    pub fn channel_badge(&self) -> Option<&'static str> {
        if self.is_beta {
            Some("beta")
        } else if self.is_git || self.version.to_ascii_uppercase().contains("DEV") {
            Some("dev")
        } else {
            None
        }
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
        go: info
            .get("goVersion")
            .or_else(|| info.get("go_version"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        is_beta: info
            .get("isBeta")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        is_git: info.get("isGit").and_then(|x| x.as_bool()).unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_missing_rc_methods() {
        cache_missing_method("http://127.0.0.1:9", "job/batch");
        assert!(is_cached_missing_method("http://127.0.0.1:9", "job/batch"));
        assert!(!is_cached_missing_method(
            "http://127.0.0.1:9",
            "core/version"
        ));
        assert!(looks_missing_endpoint(&RcError::message(
            "HTTP 500: couldn't find method \"job/batch\""
        )));
    }

    #[test]
    fn parses_mount_points_map_and_array() {
        let map = parse_mount_points(&json!({
            "mountPoints": { "/mnt/drive": "drive:" }
        }));
        assert_eq!(map.len(), 1);
        assert_eq!(map[0].fs, "drive:");
        assert_eq!(map[0].mount_point, "/mnt/drive");
        let arr = parse_mount_points(&json!({
            "mountPoints": [{
                "Fs": "/tmp/rclone-test-remote",
                "MountPoint": "/home/ubuntu/rclone-manager/testdrive",
                "MountedOn": "2026-08-25T23:14:23Z"
            }]
        }));
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].fs, "/tmp/rclone-test-remote");
        assert_eq!(arr[0].mount_point, "/home/ubuntu/rclone-manager/testdrive");
        assert!(parse_mount_points(&json!({})).is_empty());
    }

    #[test]
    fn parses_proc_mounts_fuse_rclone() {
        let mounts = parse_proc_mounts(
            "/tmp/rclone-test-remote /home/ubuntu/rclone-manager/testdrive fuse.rclone rw 0 0\n\
             /dev/sda1 / ext4 rw 0 0\n",
        );
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].fs, "/tmp/rclone-test-remote");
        assert_eq!(
            mounts[0].mount_point,
            "/home/ubuntu/rclone-manager/testdrive"
        );
        assert!(host_fuse_mounted_in(
            &text_with_testdrive(),
            "/home/ubuntu/rclone-manager/testdrive"
        ));
        assert!(!host_fuse_mounted_in(
            &text_with_testdrive(),
            "/home/ubuntu/rclone-manager/other"
        ));
    }

    fn text_with_testdrive() -> String {
        "/tmp/rclone-test-remote /home/ubuntu/rclone-manager/testdrive fuse.rclone rw 0 0\n\
         /dev/sda1 / ext4 rw 0 0\n"
            .into()
    }

    fn host_fuse_mounted_in(text: &str, mount_point: &str) -> bool {
        let want = mount_point.trim_end_matches('/');
        parse_proc_mounts(text)
            .iter()
            .any(|m| m.mount_point.trim_end_matches('/') == want)
    }

    #[test]
    fn merges_mount_profile_context() {
        let live = vec![MountedRemote::new("drive:", "/mnt/drive")];
        let mut remembered = MountedRemote::new("drive:", "/mnt/drive");
        remembered.profile = "nightly".into();
        remembered.origin = "dashboard".into();
        let merged = merge_mount_context(live, &[remembered]);
        assert_eq!(merged[0].profile, "nightly");
        assert_eq!(merged[0].origin, "dashboard");
        let fresh = merge_mount_context(vec![MountedRemote::new("other:", "/mnt/other")], &[]);
        assert!(fresh[0].profile.is_empty());
    }

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
    fn parses_fscache_entry_count() {
        assert_eq!(parse_fscache_entry_count(&json!({ "count": 4 })), 4);
        assert_eq!(
            parse_fscache_entry_count(&json!({ "entries": ["a", "b"] })),
            2
        );
        assert_eq!(parse_fscache_entry_count(&json!(["x", "y", "z"])), 3);
        assert_eq!(parse_fscache_entry_count(&json!({})), 0);
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(-1), "—");
        assert_eq!(format_eta_seconds(0), "—");
        assert_eq!(format_eta_seconds(12), "12s");
        assert_eq!(format_eta_seconds(75), "1m 15s");
        assert_eq!(format_eta_seconds(3661), "1h 1m 1s");
    }

    #[test]
    fn parses_serve_item_origin_and_url() {
        let item = ServeItem::from_rc(&json!({
            "id": "s1",
            "addr": "127.0.0.1:8080",
            "params": {
                "fs": "drive:",
                "type": "webdav",
                "origin": "quickrun",
                "profile": "public",
                "user": "ada",
                "pass": "secret"
            }
        }))
        .unwrap();
        assert_eq!(item.id, "s1");
        assert_eq!(item.origin, "quickrun");
        assert_eq!(item.profile, "public");
        assert_eq!(item.option_count, 2);
        assert_eq!(item.url(), "http://127.0.0.1:8080");
        assert_eq!(
            ServeItem {
                addr: "https://example/s".into(),
                ..ServeItem::default()
            }
            .url(),
            "https://example/s"
        );
        assert_eq!(
            ServeItem {
                addr: "127.0.0.1:22".into(),
                serve_type: "sftp".into(),
                ..ServeItem::default()
            }
            .url(),
            "sftp://127.0.0.1:22"
        );
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
        assert_eq!(id.channel_badge(), None);
        assert_eq!(backend_identity(&json!({})).version, "unknown");
        let git = backend_identity(&json!({
            "version": "v1.60.1-DEV",
            "os": "linux",
            "arch": "amd64",
            "goVersion": "go1.19.4",
            "isGit": true
        }));
        assert_eq!(git.go, "go1.19.4");
        assert_eq!(git.channel_badge(), Some("dev"));
        let beta = backend_identity(&json!({
            "version": "v1.70.0-beta.1",
            "isBeta": true
        }));
        assert_eq!(beta.channel_badge(), Some("beta"));
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
            parse_hashsum_list(&json!({
                "hashsum": ["abc  a.txt", "def  b.txt"]
            }))
            .as_deref(),
            Some("abc  a.txt\ndef  b.txt")
        );
        assert_eq!(
            parse_hashsum_list(&json!({ "hash": "only-hash" })).as_deref(),
            Some("only-hash")
        );
        assert_eq!(parse_hashsum_list(&json!({})), None);
        assert_eq!(public_link_expiry_value(0), Some(""));
        assert_eq!(public_link_expiry_value(2), Some("1d"));
        assert_eq!(public_link_expiry_value(9), None);
        let (system, standard) = group_metadata_info(&json!({
            "System": { "mtime": { "Help": "mod time", "Type": "date" } },
            "Help": "ignored string",
            "User": { "comment": { "Help": "user field" } }
        }));
        assert_eq!(system.len(), 1);
        assert_eq!(system[0].0, "mtime");
        assert_eq!(standard.len(), 1);
        assert_eq!(standard[0].0, "comment");
        assert!(listing_caption(true, 0, "").is_empty());
        assert!(listing_caption(false, 1024, "").contains("1.0"));
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
        assert!(item.mod_time.is_empty());
        let stamped = parse_stat(&json!({
            "item": {
                "Name": "notes.txt",
                "IsDir": false,
                "Size": 12,
                "MimeType": "text/plain",
                "ModTime": "2024-05-01T12:00:00Z"
            }
        }))
        .unwrap();
        assert_eq!(stamped.mod_time, "2024-05-01T12:00:00Z");
        assert_eq!(format_mod_time("2024-05-01T12:00:00Z").len(), 16);
        assert_eq!(format_mod_time(""), "");
        assert_eq!(format_mod_time("not-a-date"), "not-a-date");
        assert_eq!(format_relative_mod_time(""), "");
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
        assert_eq!(
            vfs_queue_expiry_payload("drive:", "3", "-999999999", false),
            json!({ "fs": "drive:", "id": "3", "expiry": -999_999_999, "relative": false })
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

    #[test]
    fn rc_serve_url_matches_tauri() {
        assert_eq!(
            build_rc_serve_url("http://127.0.0.1:5572", "drive", "Photos/video.mp4"),
            "http://127.0.0.1:5572/[drive:]/Photos/video.mp4"
        );
        assert_eq!(
            build_rc_serve_url("http://127.0.0.1:5572/", "drive:", "/Photos/My File.mp4"),
            "http://127.0.0.1:5572/[drive:]/Photos/My%20File.mp4"
        );
        assert_eq!(
            build_rc_serve_url("http://127.0.0.1:5572", "alias/inner", "a"),
            "http://127.0.0.1:5572/[alias/inner]/a"
        );
        assert_eq!(
            build_rc_serve_url("http://127.0.0.1:5572", "drive", ""),
            "http://127.0.0.1:5572/[drive:]/"
        );
        let client =
            RcClient::new("127.0.0.1", 5572).with_auth(Some("ada".into()), Some("s ecret".into()));
        assert_eq!(
            client.rc_serve_url("drive", "Photos/clip.mkv"),
            "http://ada:s%20ecret@127.0.0.1:5572/[drive:]/Photos/clip.mkv"
        );
        assert!(!client.probe_rc_serve("http://127.0.0.1:1/[drive:]/missing.bin"));
        assert!(!client.probe_rc_serve(""));
    }

    #[test]
    fn config_setpath_and_unlock_payloads() {
        assert_eq!(
            config_setpath_payload("/tmp/rclone.conf"),
            json!({ "path": "/tmp/rclone.conf" })
        );
        assert_eq!(
            config_unlock_payload("secret"),
            json!({ "configPassword": "secret" })
        );
    }
}
