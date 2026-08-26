//! Import remotes from an opened `rclone.conf` / `rclone.json` (MIME / CLI).

use crate::backup::rclone_create_params;
use crate::rclone::{RcClient, RcError};
use crate::security::apply_config_password_env;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

static PENDING_OPEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn enqueue_open_configs(files: &[PathBuf]) {
    let mut pending = PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner());
    for file in files {
        if file.as_os_str().is_empty() {
            continue;
        }
        if pending.iter().any(|p| p == file) {
            continue;
        }
        pending.push(file.clone());
    }
}

pub fn take_open_configs() -> Vec<PathBuf> {
    std::mem::take(&mut *PENDING_OPEN.lock().unwrap_or_else(|e| e.into_inner()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigImportReport {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl ConfigImportReport {
    pub fn created_count(&self) -> usize {
        self.created.len()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }
}

pub fn looks_like_rclone_conf(text: &str) -> bool {
    if text.contains("RCLONE_ENCRYPT_V0") {
        return true;
    }
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            in_section = true;
            continue;
        }
        if in_section {
            let key = line.split_once('=').map(|(k, _)| k.trim()).unwrap_or("");
            if key.eq_ignore_ascii_case("type") {
                return true;
            }
        }
    }
    false
}

pub fn looks_like_rclone_dump(value: &Value) -> bool {
    value.as_object().is_some_and(|obj| {
        !obj.is_empty()
            && obj.values().all(|cfg| {
                cfg.as_object()
                    .is_some_and(|section| section.contains_key("type"))
            })
    })
}

pub fn parse_rclone_conf(text: &str) -> Result<Value, String> {
    if text.contains("RCLONE_ENCRYPT_V0") {
        return Err("encrypted rclone.conf requires rclone to decrypt".into());
    }
    let mut map = Map::new();
    let mut current = None::<String>;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let name = name.trim();
            if name.is_empty() {
                current = None;
                continue;
            }
            current = Some(name.to_string());
            map.entry(name.to_string()).or_insert_with(|| json!({}));
            continue;
        }
        let Some(section) = current.clone() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some(obj) = map.get_mut(&section).and_then(|v| v.as_object_mut()) {
            obj.insert(key.trim().to_string(), json!(value.trim()));
        }
    }
    Ok(Value::Object(map))
}

fn value_to_conf(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Write a dump (`config/dump` JSON) as rclone.conf INI.
pub fn dump_to_rclone_conf(dump: &Value) -> String {
    let mut out = String::new();
    for name in dump_remote_names(dump) {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push('[');
        out.push_str(&name);
        out.push_str("]\n");
        let Some(obj) = dump.get(&name).and_then(|value| value.as_object()) else {
            continue;
        };
        let mut keys: Vec<_> = obj.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let value = value_to_conf(&obj[&key]);
            if value.is_empty() {
                continue;
            }
            out.push_str(&key);
            out.push_str(" = ");
            out.push_str(&value);
            out.push('\n');
        }
    }
    out
}

pub fn write_rclone_conf(path: &Path, dump: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, dump_to_rclone_conf(dump)).map_err(|e| e.to_string())
}

pub fn dump_remote_names(dump: &Value) -> Vec<String> {
    let mut names = dump
        .as_object()
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    names.sort();
    names
}

pub fn file_uri_to_path(input: &str) -> PathBuf {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("file://") {
        let decoded = urlencoding::decode(rest).unwrap_or(std::borrow::Cow::Borrowed(rest));
        let path = decoded.split('?').next().unwrap_or(&decoded);
        #[cfg(windows)]
        {
            let path = path.trim_start_matches('/');
            if path.chars().nth(1) == Some(':') {
                return PathBuf::from(path);
            }
        }
        return PathBuf::from(path);
    }
    PathBuf::from(trimmed)
}

pub fn path_looks_like_rclone_config(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == "rclone.conf" || name.ends_with(".rclone.conf") || name == "rclone.json" {
        return true;
    }
    if name.ends_with(".conf") {
        return std::fs::read_to_string(path)
            .map(|text| looks_like_rclone_conf(&text))
            .unwrap_or(false);
    }
    if name.ends_with(".json") {
        return std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|value| looks_like_rclone_dump(&value));
    }
    std::fs::read_to_string(path)
        .map(|text| looks_like_rclone_conf(&text))
        .unwrap_or(false)
}

fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "--import-config"
            | "--send-to-remote"
            | "--send-to-path"
            | "--browse"
            | "--browse-path"
            | "--dialog"
            | "--dialog-data"
            | "--dialog-result"
            | "--data-dir"
            | "--cache-dir"
            | "--logs-dir"
            | "--tray-action"
            | "--tab"
            | "--remote"
            | "--quick-run"
            | "--job"
            | "--serve"
            | "--automation"
            | "--remote-config"
            | "--auto-add"
            | "--logs"
            | "--step"
            | "--profile"
            | "--preferences"
            | "--settings"
            | "--standalone"
    )
}

/// Positional / `--import-config` rclone.conf paths from a desktop MIME or CLI launch.
pub fn parse_open_config_args(args: &[String]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--import-config" {
            i += 1;
            if let Some(value) = args.get(i) {
                files.push(file_uri_to_path(value));
            }
        } else if let Some(value) = arg.strip_prefix("--import-config=") {
            files.push(file_uri_to_path(value));
        } else if arg.starts_with('-') {
            if takes_value(arg) {
                i += 1;
            }
        } else {
            let path = file_uri_to_path(arg);
            if path_looks_like_rclone_config(&path) {
                files.push(path);
            }
        }
        i += 1;
    }
    files
}

pub fn load_config_dump(path: &Path) -> Result<Value, String> {
    load_config_dump_with(None, path, "")
}

pub fn load_config_dump_with(
    binary: Option<&Path>,
    path: &Path,
    password: &str,
) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if looks_like_rclone_dump(&value) {
            return Ok(value);
        }
        return Err("JSON file is not an rclone config dump".into());
    }
    if !text.contains("RCLONE_ENCRYPT_V0") {
        if let Ok(value) = parse_rclone_conf(&text) {
            if value.as_object().is_some_and(|o| !o.is_empty()) {
                return Ok(value);
            }
        }
    }
    let Some(binary) = binary.filter(|p| p.as_os_str().len() > 0) else {
        return Err("could not parse rclone.conf".into());
    };
    let mut cmd = Command::new(binary);
    cmd.args(["config", "dump", "--config", &path.to_string_lossy()]);
    apply_config_password_env(&mut cmd, password);
    let output = cmd.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(if err.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            err.trim().to_string()
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}

pub fn import_dump(
    client: &RcClient,
    dump: &Value,
    existing: &HashSet<String>,
    overwrite: bool,
) -> ConfigImportReport {
    let mut report = ConfigImportReport::default();
    let Some(obj) = dump.as_object() else {
        return report;
    };
    for (name, cfg) in obj {
        if name.trim().is_empty() {
            continue;
        }
        let Some(r#type) = cfg.get("type").and_then(|x| x.as_str()) else {
            report.failed.push((name.clone(), "missing type".into()));
            continue;
        };
        if existing.contains(name) && !overwrite {
            report.skipped.push(name.clone());
            continue;
        }
        let params = rclone_create_params(cfg);
        let result = if existing.contains(name) && overwrite {
            client
                .update_remote(name, params, None)
                .map(|_| ())
                .or_else(|err| match err {
                    RcError::Unreachable(_) => Err(err.to_string()),
                    other => client
                        .create_remote(name, r#type, rclone_create_params(cfg), None)
                        .map(|_| ())
                        .map_err(|_| other.to_string()),
                })
        } else {
            client
                .create_remote(name, r#type, params, None)
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        match result {
            Ok(()) => report.created.push(name.clone()),
            Err(e) => report.failed.push((name.clone(), e)),
        }
    }
    report.created.sort();
    report.skipped.sort();
    report.failed.sort_by(|a, b| a.0.cmp(&b.0));
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_ini_and_encrypted() {
        assert!(looks_like_rclone_conf("[photos]\ntype = drive\n"));
        assert!(looks_like_rclone_conf("RCLONE_ENCRYPT_V0:abc"));
        assert!(!looks_like_rclone_conf("[unit]\nDescription=svc\n"));
        assert!(!looks_like_rclone_conf("hello"));
    }

    #[test]
    fn parses_standard_conf() {
        let dump = parse_rclone_conf(
            "# comment\n[photos]\ntype = drive\nscope = drive\n\n[local]\ntype=local\n",
        )
        .unwrap();
        assert_eq!(dump["photos"]["type"], "drive");
        assert_eq!(dump["photos"]["scope"], "drive");
        assert_eq!(dump["local"]["type"], "local");
        assert_eq!(dump_remote_names(&dump), vec!["local", "photos"]);
    }

    #[test]
    fn rejects_encrypted_parse() {
        assert!(parse_rclone_conf("RCLONE_ENCRYPT_V0:xyz").is_err());
    }

    #[test]
    fn ini_roundtrip_preserves_remotes() {
        let dump = parse_rclone_conf(
            "[photos]\ntype = drive\nscope = drive\n\n[inbox]\ntype = alias\nremote = /tmp\n",
        )
        .unwrap();
        let text = dump_to_rclone_conf(&dump);
        assert!(text.contains("[photos]"));
        assert!(text.contains("type = drive"));
        assert!(text.contains("[inbox]"));
        let again = parse_rclone_conf(&text).unwrap();
        assert_eq!(again["photos"]["scope"], "drive");
        assert_eq!(again["inbox"]["remote"], "/tmp");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.rclone.conf");
        write_rclone_conf(&path, &dump).unwrap();
        assert!(path_looks_like_rclone_config(&path));
        assert_eq!(load_config_dump(&path).unwrap()["photos"]["type"], "drive");
    }

    #[test]
    fn dump_json_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rclone.json");
        std::fs::write(&path, r#"{"demo":{"type":"local","nounc":"true"}}"#).unwrap();
        let dump = load_config_dump(&path).unwrap();
        assert_eq!(dump["demo"]["type"], "local");
        assert!(looks_like_rclone_dump(&dump));
    }

    #[test]
    fn load_plain_conf_without_rclone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rclone.conf");
        std::fs::write(&path, "[inbox]\ntype = alias\nremote = /tmp\n").unwrap();
        let dump = load_config_dump(&path).unwrap();
        assert_eq!(dump["inbox"]["type"], "alias");
        assert_eq!(dump["inbox"]["remote"], "/tmp");
        assert!(path_looks_like_rclone_config(&path));
    }

    #[test]
    fn parses_import_config_cli_and_file_uri() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rclone.conf");
        std::fs::write(&path, "[a]\ntype = local\n").unwrap();
        let uri = format!("file://{}", path.display());
        let parsed = parse_open_config_args(&[
            "app".into(),
            "--import-config".into(),
            path.display().to_string(),
        ]);
        assert_eq!(parsed, vec![path.clone()]);
        let from_uri = parse_open_config_args(&["app".into(), uri]);
        assert_eq!(from_uri, vec![path.clone()]);
        assert!(
            parse_open_config_args(&["app".into(), "--browse".into(), "drive:".into()]).is_empty()
        );
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "hello").unwrap();
        assert!(parse_open_config_args(&["app".into(), other.display().to_string()]).is_empty());
    }

    #[test]
    fn enqueue_and_take_open_configs() {
        let _ = take_open_configs();
        enqueue_open_configs(&[PathBuf::from("/tmp/a.conf"), PathBuf::from("/tmp/a.conf")]);
        enqueue_open_configs(&[PathBuf::from("/tmp/b.conf")]);
        let taken = take_open_configs();
        assert_eq!(
            taken,
            vec![PathBuf::from("/tmp/a.conf"), PathBuf::from("/tmp/b.conf")]
        );
        assert!(take_open_configs().is_empty());
    }

    #[test]
    fn file_uri_decodes_spaces() {
        assert_eq!(
            file_uri_to_path("file:///tmp/My%20Drive/rclone.conf"),
            PathBuf::from("/tmp/My Drive/rclone.conf")
        );
        assert_eq!(
            file_uri_to_path("/plain/path.conf"),
            PathBuf::from("/plain/path.conf")
        );
    }
}
