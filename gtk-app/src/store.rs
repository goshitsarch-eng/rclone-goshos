//! Persistent app store: remotes metadata, quick runs, alerts, templates, jobs.

use crate::operations::OperationType;
use crate::rclone::{format_bytes, DirEntry, MountedRemote, ServeItem};

pub const DEFAULT_ALERT_ACTION_ID: &str = "default-os-toast";
pub const DEFAULT_ALERT_RULE_ID: &str = "default-rule";
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RemoteMeta {
    pub show_on_tray: bool,
    pub primary_actions: Vec<String>,
    pub sync_actions: Vec<String>,
    pub hidden: bool,
    pub profiles: HashMap<String, HashMap<String, ProfileConfig>>,
    #[serde(default)]
    pub vfs_configs: HashMap<String, Value>,
    #[serde(default)]
    pub filter_configs: HashMap<String, Value>,
    #[serde(default)]
    pub backend_configs: HashMap<String, Value>,
    #[serde(default)]
    pub runtime_remote_configs: HashMap<String, Value>,
}

impl RemoteMeta {
    pub fn helper_profile(&self, kind: &str, name: &str) -> Option<Value> {
        if name.is_empty() {
            return None;
        }
        match kind {
            "vfs" => self.vfs_configs.get(name).cloned(),
            "filter" => self.filter_configs.get(name).cloned(),
            "backend" => self.backend_configs.get(name).cloned(),
            "runtime" | "runtime_remote" => self.runtime_remote_configs.get(name).cloned(),
            _ => None,
        }
    }

    pub fn helper_names(&self, kind: &str) -> Vec<String> {
        let mut names: Vec<String> = self.helper_map(kind).keys().cloned().collect();
        names.sort();
        names
    }

    pub fn helper_map(&self, kind: &str) -> &HashMap<String, Value> {
        match kind {
            "vfs" => &self.vfs_configs,
            "filter" => &self.filter_configs,
            "backend" => &self.backend_configs,
            _ => &self.runtime_remote_configs,
        }
    }

    pub fn helper_map_mut(&mut self, kind: &str) -> &mut HashMap<String, Value> {
        match kind {
            "vfs" => &mut self.vfs_configs,
            "filter" => &mut self.filter_configs,
            "backend" => &mut self.backend_configs,
            _ => &mut self.runtime_remote_configs,
        }
    }

    pub fn upsert_helper(&mut self, kind: &str, name: &str, value: Value) {
        if name.is_empty() {
            return;
        }
        self.helper_map_mut(kind).insert(name.to_string(), value);
    }

    pub fn remove_helper(&mut self, kind: &str, name: &str) -> bool {
        self.helper_map_mut(kind).remove(name).is_some()
    }

    pub fn rename_helper(&mut self, kind: &str, from: &str, to: &str) -> bool {
        if to.is_empty() || from == to {
            return false;
        }
        if let Some(value) = self.helper_map_mut(kind).remove(from) {
            self.helper_map_mut(kind).insert(to.to_string(), value);
            true
        } else {
            false
        }
    }

    pub fn profile_names(&self, op: OperationType) -> Vec<String> {
        let mut names: Vec<String> = self
            .profiles
            .get(op.as_str())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    pub fn get_profile(&self, op: OperationType, name: &str) -> Option<ProfileConfig> {
        self.profiles
            .get(op.as_str())
            .and_then(|m| m.get(name))
            .cloned()
    }

    pub fn upsert_profile(&mut self, op: OperationType, profile: ProfileConfig) {
        if profile.name.is_empty() {
            return;
        }
        self.profiles
            .entry(op.as_str().to_string())
            .or_default()
            .insert(profile.name.clone(), profile);
    }

    pub fn remove_profile(&mut self, op: OperationType, name: &str) -> bool {
        self.profiles
            .get_mut(op.as_str())
            .map(|m| m.remove(name).is_some())
            .unwrap_or(false)
    }

    pub fn rename_profile(&mut self, op: OperationType, from: &str, to: &str) -> bool {
        if to.is_empty() || from == to {
            return false;
        }
        if let Some(mut profile) = self
            .profiles
            .get_mut(op.as_str())
            .and_then(|m| m.remove(from))
        {
            profile.name = to.to_string();
            self.upsert_profile(op, profile);
            true
        } else {
            false
        }
    }

    pub fn visible_operations(&self) -> Vec<OperationType> {
        let mut ops: Vec<OperationType> = self
            .primary_actions
            .iter()
            .chain(self.sync_actions.iter())
            .filter_map(|s| OperationType::parse(s))
            .collect();
        ops.dedup();
        if ops.is_empty() {
            OperationType::ALL.to_vec()
        } else {
            ops
        }
    }

    pub fn clone_profile(&mut self, op: OperationType, from: &str, to: &str) -> bool {
        if to.is_empty() {
            return false;
        }
        if let Some(mut profile) = self.get_profile(op, from) {
            profile.name = to.to_string();
            self.upsert_profile(op, profile);
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileConfig {
    pub name: String,
    pub app: AppConfig,
    pub rclone: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub auto_start: bool,
    pub cron_enabled: bool,
    pub cron_expression: String,
    pub watch_enabled: bool,
    pub watch_delay: u64,
    pub watch_changed_only: bool,
    #[serde(default)]
    pub vfs_profile: String,
    #[serde(default)]
    pub filter_profile: String,
    #[serde(default)]
    pub backend_profile: String,
    #[serde(default)]
    pub runtime_remote_profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickRun {
    pub id: String,
    pub name: String,
    pub description: String,
    pub operation_type: OperationType,
    pub remote_name: String,
    pub config: ProfileConfig,
    pub status: String,
    pub show_on_tray: bool,
    pub last_job_id: Option<u64>,
    pub run_count: u64,
}

impl QuickRun {
    pub fn new(name: String, operation_type: OperationType, remote_name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            description: String::new(),
            operation_type,
            remote_name,
            config: ProfileConfig::default(),
            status: "idle".into(),
            show_on_tray: false,
            last_job_id: None,
            run_count: 0,
        }
    }

    pub fn paths(&self) -> (Option<String>, Option<String>) {
        quick_run_paths(&self.config.rclone, self.operation_type)
    }
}

pub fn quick_run_paths(rclone: &Value, op: OperationType) -> (Option<String>, Option<String>) {
    let scoped = rclone
        .get(op.as_str())
        .cloned()
        .unwrap_or_else(|| rclone.clone());
    let source = first_string(&scoped, &["srcFs", "path1", "fs", "source"]);
    let dest = first_string(&scoped, &["mountPoint", "dstFs", "path2", "dest"]);
    (source, dest)
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            } else if let Some(arr) = v.as_array() {
                let joined = arr
                    .iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if !joined.is_empty() {
                    return Some(joined);
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: u64,
    pub operation: String,
    pub remote: String,
    pub profile: String,
    pub status: String,
    pub origin: String,
    pub start_time: DateTime<Utc>,
    pub error: Option<String>,
    pub dry_run: bool,
    pub src: String,
    pub dst: String,
    pub group: String,
    #[serde(default)]
    pub stats: Value,
    #[serde(default)]
    pub transferring: Value,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub completed: Value,
    #[serde(default)]
    pub parent_job_id: Option<u64>,
}

/// Local start-time metadata for a job id (persisted so grouped transfer
/// snapshots survive restart).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutomationStatus {
    #[default]
    Enabled,
    Disabled,
    Running,
    Failed,
    Stopping,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutomationRuntime {
    #[serde(default)]
    pub status: AutomationStatus,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub current_job_id: Option<String>,
    #[serde(default)]
    pub run_count: u64,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub failure_count: u64,
    #[serde(default)]
    pub stopped_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobMeta {
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub remote: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub quick_run_id: String,
    #[serde(default)]
    pub execute_id: String,
    #[serde(default)]
    pub parent_job_id: Option<u64>,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub transfer_snapshot: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Critical,
    High,
    Average,
    Warning,
    Info,
}

impl AlertSeverity {
    pub fn rank(self) -> u8 {
        match self {
            Self::Critical => 5,
            Self::High => 4,
            Self::Average => 3,
            Self::Warning => 2,
            Self::Info => 1,
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "critical" => Self::Critical,
            "high" => Self::High,
            "average" => Self::Average,
            "warning" => Self::Warning,
            _ => Self::Info,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Average => "average",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AlertEventKind {
    Job,
    Serve,
    Mount,
    Engine,
    Update,
    Automation,
    System,
}

impl AlertEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Job => "job",
            Self::Serve => "serve",
            Self::Mount => "mount",
            Self::Engine => "engine",
            Self::Update => "update",
            Self::Automation => "automation",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "job" => Some(Self::Job),
            "serve" => Some(Self::Serve),
            "mount" => Some(Self::Mount),
            "engine" => Some(Self::Engine),
            "update" => Some(Self::Update),
            "automation" => Some(Self::Automation),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub event_filter: Vec<AlertEventKind>,
    pub severity_min: AlertSeverity,
    pub remote_filter: Vec<String>,
    pub backend_filter: Vec<String>,
    pub profile_filter: Vec<String>,
    pub origin_filter: Vec<String>,
    pub action_ids: Vec<String>,
    pub cooldown_secs: u64,
    pub auto_acknowledge: bool,
    pub last_fired: Option<DateTime<Utc>>,
    pub fire_count: u64,
}

impl AlertRule {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            enabled: true,
            event_filter: vec![],
            severity_min: AlertSeverity::Info,
            remote_filter: vec![],
            backend_filter: vec![],
            profile_filter: vec![],
            origin_filter: vec![],
            action_ids: vec![],
            cooldown_secs: 0,
            auto_acknowledge: false,
            last_fired: None,
            fire_count: 0,
        }
    }

    pub fn matches(&self, event: &AlertEvent) -> bool {
        if !self.enabled {
            return false;
        }
        if event.severity.rank() < self.severity_min.rank() {
            return false;
        }
        if !self.event_filter.is_empty() && !self.event_filter.contains(&event.kind) {
            return false;
        }
        if !self.remote_filter.is_empty() && !self.remote_filter.iter().any(|r| r == &event.remote)
        {
            return false;
        }
        if !self.origin_filter.is_empty() && !self.origin_filter.iter().any(|o| o == &event.origin)
        {
            return false;
        }
        if !self.backend_filter.is_empty()
            && !self.backend_filter.iter().any(|b| b == &event.backend)
        {
            return false;
        }
        if !self.profile_filter.is_empty()
            && !self.profile_filter.iter().any(|p| p == &event.profile)
        {
            return false;
        }
        if let Some(last) = self.last_fired {
            if self.cooldown_secs > 0 {
                let elapsed = (Utc::now() - last).num_seconds();
                if elapsed >= 0 && (elapsed as u64) < self.cooldown_secs {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertAction {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub kind: String,
    pub config: Value,
}

impl AlertAction {
    pub fn new(name: String, kind: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            enabled: true,
            kind,
            config: json!({}),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AlertActionDraft {
    pub url: String,
    pub method: String,
    pub token: String,
    pub extra: String,
    pub extra2: String,
    pub headers: String,
    pub body: String,
    pub retry_count: u32,
    pub provider: String,
    pub timeout_secs: u32,
    pub tls_verify: bool,
    pub telegram_mode: String,
    pub subject: String,
    pub qos: u32,
    pub retain: bool,
    pub encryption: String,
    pub env_vars: String,
    pub username: String,
    pub use_tls: bool,
}

impl Default for AlertActionDraft {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: String::new(),
            token: String::new(),
            extra: String::new(),
            extra2: String::new(),
            headers: String::new(),
            body: String::new(),
            retry_count: 0,
            provider: String::new(),
            timeout_secs: 8,
            tls_verify: true,
            telegram_mode: "bot".into(),
            subject: String::new(),
            qos: 0,
            retain: false,
            encryption: "starttls".into(),
            env_vars: String::new(),
            username: String::new(),
            use_tls: false,
        }
    }
}

pub fn parse_header_lines(text: &str) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        map.insert(key.to_string(), json!(value.trim()));
    }
    map
}

pub fn header_pairs(config: &Value) -> Vec<(String, String)> {
    config
        .get("headers")
        .and_then(|h| h.as_object())
        .map(|map| {
            map.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn pairs_to_header_text(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .filter(|(k, _)| !k.trim().is_empty())
        .map(|(k, v)| format!("{}: {}", k.trim(), v.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn header_pairs_from_text(text: &str) -> Vec<(String, String)> {
    parse_header_lines(text)
        .into_iter()
        .map(|(k, v)| (k, v.as_str().unwrap_or("").to_string()))
        .collect()
}

pub fn headers_to_text(config: &Value) -> String {
    config
        .get("headers")
        .and_then(|h| h.as_object())
        .map(|map| {
            map.iter()
                .map(|(k, v)| format!("{}: {}", k, v.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub fn script_args(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

pub fn parse_env_lines(text: &str) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        map.insert(key.to_string(), json!(value.trim()));
    }
    map
}

pub fn env_vars_to_text(config: &Value) -> String {
    config
        .get("env_vars")
        .and_then(|h| h.as_object())
        .map(|map| {
            map.iter()
                .map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub fn webhook_preset_body(preset: &str) -> String {
    match preset {
        "discord" => serde_json::to_string_pretty(&json!({
            "content": "@everyone",
            "embeds": [{
                "title": "{{title}}",
                "description": "{{body}}",
                "color": 5814783,
                "fields": [
                    { "name": "Severity", "value": "{{severity}}", "inline": true },
                    { "name": "Time", "value": "{{timestamp}}", "inline": true }
                ]
            }]
        }))
        .unwrap_or_default(),
        "slack" => serde_json::to_string_pretty(&json!({
            "text": "*{{title}}*\n{{body}}\n_Severity: {{severity}}_"
        }))
        .unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn webhook_http_method(method: &str) -> &'static str {
    match method.trim().to_ascii_uppercase().as_str() {
        "GET" => "GET",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        _ => "POST",
    }
}

pub fn wait_script(mut process: std::process::Command, timeout_secs: u64) -> bool {
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(1));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}

pub fn ensure_content_type_json(headers: &str) -> String {
    let existing = parse_header_lines(headers);
    if existing
        .keys()
        .any(|k| k.eq_ignore_ascii_case("content-type"))
    {
        return headers.to_string();
    }
    let extra = "Content-Type: application/json";
    if headers.trim().is_empty() {
        extra.into()
    } else {
        format!("{}\n{extra}", headers.trim_end())
    }
}

pub fn telegram_botless_url(chat_id: &str, body: &str) -> String {
    let user = if chat_id.starts_with('@') {
        chat_id.to_string()
    } else {
        format!("@{chat_id}")
    };
    format!(
        "https://api.callmebot.com/text.php?user={}&text={}",
        urlencoding::encode(&user),
        urlencoding::encode(body)
    )
}

pub fn alert_action_config(kind: &str, draft: &AlertActionDraft) -> Value {
    let body = if draft.body.is_empty() {
        "{{title}}: {{body}}".into()
    } else {
        draft.body.clone()
    };
    let retry = draft.retry_count.min(5);
    let timeout = if draft.timeout_secs == 0 {
        8
    } else {
        draft.timeout_secs
    };
    match kind {
        "os_toast" => json!({ "body_template": body, "retry_count": retry }),
        "webhook" => json!({
            "url": draft.url,
            "method": if draft.method.is_empty() { "POST".into() } else { draft.method.clone() },
            "body_template": body,
            "retry_count": retry,
            "headers": parse_header_lines(&draft.headers),
            "timeout_secs": timeout,
            "tls_verify": draft.tls_verify,
        }),
        "telegram" => json!({
            "bot_token": draft.token,
            "chat_id": draft.extra,
            "mode": if draft.telegram_mode == "botless" { "botless" } else { "bot" },
            "body_template": body,
            "retry_count": retry,
            "timeout_secs": timeout,
        }),
        "whatsapp" => json!({
            "apikey": draft.token,
            "phone": draft.extra,
            "gateway_url": draft.url,
            "provider": if draft.provider == "custom_gateway" {
                "custom_gateway"
            } else {
                "callmebot"
            },
            "body_template": body,
            "retry_count": retry,
            "timeout_secs": timeout,
        }),
        "script" => json!({
            "command": draft.extra,
            "args": script_args(&draft.extra2),
            "env_vars": parse_env_lines(&draft.env_vars),
            "body_template": body,
            "retry_count": retry,
            "timeout_secs": timeout,
        }),
        "email" => json!({
            "smtp_server": draft.url,
            "smtp_port": draft.method.parse::<u16>().unwrap_or(587),
            "username": draft.username,
            "password": draft.token,
            "from": if draft.extra2.is_empty() { draft.extra.clone() } else { draft.extra2.clone() },
            "to": draft.extra,
            "subject_template": if draft.subject.is_empty() { "{{title}}".into() } else { draft.subject.clone() },
            "body_template": body,
            "encryption": match draft.encryption.as_str() {
                "none" | "tls" => draft.encryption.as_str(),
                _ => "starttls",
            },
            "retry_count": retry,
            "timeout_secs": timeout,
        }),
        "mqtt" => json!({
            "broker_url": draft.url,
            "topic": if draft.method.is_empty() { "rclone/alerts".into() } else { draft.method.clone() },
            "username": draft.username,
            "password": draft.token,
            "use_tls": draft.use_tls,
            "body_template": body,
            "retry_count": retry,
            "timeout_secs": timeout,
            "qos": draft.qos.min(2),
            "retain": draft.retain,
        }),
        _ => json!({ "body_template": body, "retry_count": retry }),
    }
}

pub fn whatsapp_request_url(config: &Value, body: &str) -> Option<String> {
    let phone = config
        .get("phone")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())?;
    let key = config
        .get("apikey")
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    let provider = config
        .get("provider")
        .and_then(|x| x.as_str())
        .unwrap_or("callmebot");
    let gateway = config
        .get("gateway_url")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim();
    if provider == "custom_gateway" {
        if gateway.is_empty() {
            return None;
        }
        Some(format!(
            "{gateway}{sep}phone={}&text={}&apikey={}",
            urlencoding::encode(phone),
            urlencoding::encode(body),
            urlencoding::encode(key),
            sep = if gateway.contains('?') { "&" } else { "?" },
        ))
    } else if key.is_empty() {
        None
    } else {
        Some(format!(
            "https://api.callmebot.com/whatsapp.php?phone={}&text={}&apikey={}",
            urlencoding::encode(phone),
            urlencoding::encode(body),
            urlencoding::encode(key)
        ))
    }
}

pub fn alert_retry_count(action: &AlertAction) -> u32 {
    action
        .config
        .get("retry_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(5) as u32
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: String,
    pub kind: AlertEventKind,
    pub severity: AlertSeverity,
    pub title: String,
    pub body: String,
    pub remote: String,
    pub origin: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub rule_id: String,
    pub created_at: DateTime<Utc>,
    pub acknowledged: bool,
}

impl AlertEvent {
    pub fn new(kind: AlertEventKind, severity: AlertSeverity, title: String, body: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            severity,
            title,
            body,
            remote: String::new(),
            origin: "dashboard".into(),
            backend: String::new(),
            profile: String::new(),
            rule_id: String::new(),
            created_at: Utc::now(),
            acknowledged: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub created_at: String,
    pub updated_at: String,
    pub values: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Bookmark {
    pub name: String,
    pub remote: String,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSnapshot {
    pub remotes: Vec<RemoteInfo>,
    pub mounts: Vec<MountedRemote>,
    pub serves: Vec<ServeItem>,
    pub jobs: Vec<JobInfo>,
    pub stats: Value,
    pub local_disks: Vec<String>,
    pub engine_online: bool,
}

#[derive(Debug, Clone)]
pub struct RemoteInfo {
    pub name: String,
    pub r#type: String,
    pub mounted: bool,
    pub serving: bool,
    pub job_active: bool,
    pub hidden: bool,
    pub disk_label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppStore {
    pub remotes: HashMap<String, RemoteMeta>,
    pub quick_runs: Vec<QuickRun>,
    pub alert_rules: Vec<AlertRule>,
    pub alert_actions: Vec<AlertAction>,
    pub alert_history: Vec<AlertEvent>,
    pub templates: Vec<UserTemplate>,
    pub logs: HashMap<String, Vec<String>>,
    pub hidden_remotes: Vec<String>,
    pub remote_order: Vec<String>,
    #[serde(default)]
    pub automation_last_run: HashMap<String, DateTime<Utc>>,
    #[serde(default)]
    pub job_history: Vec<JobInfo>,
    #[serde(default)]
    pub automation_paused: Vec<String>,
    #[serde(default)]
    pub automation_runtime: HashMap<String, AutomationRuntime>,
    #[serde(default)]
    pub pending_share_paths: Vec<String>,
    #[serde(default)]
    pub job_meta: HashMap<u64, JobMeta>,
    /// Jobs and in-session mount/serve context for backends that are not active.
    #[serde(default)]
    pub backend_states: HashMap<String, BackendUiState>,
    #[serde(default, skip)]
    pub notifications_enabled: bool,
}

/// Per-backend UI working set (Tauri `BackendState` / `RemoteCacheContext`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendUiState {
    #[serde(default)]
    pub job_history: Vec<JobInfo>,
    #[serde(default)]
    pub job_meta: HashMap<u64, JobMeta>,
    #[serde(default, skip)]
    pub mounts: Vec<MountedRemote>,
    #[serde(default, skip)]
    pub serves: Vec<ServeItem>,
}

fn finalize_stale_jobs(jobs: &mut [JobInfo], meta: &HashMap<u64, JobMeta>) {
    let has_snapshot = |id: u64| {
        meta.get(&id).is_some_and(|entry| {
            entry
                .transfer_snapshot
                .as_array()
                .is_some_and(|arr| !arr.is_empty())
        })
    };
    for job in jobs {
        if job.status != "preparing" && job.status != "starting" {
            continue;
        }
        let done =
            !job.completed.as_array().is_some_and(|arr| arr.is_empty()) || has_snapshot(job.id);
        job.status = if done { "completed" } else { "failed" }.into();
    }
}

fn sanitize_backend_jobs(state: &mut BackendUiState) {
    finalize_stale_jobs(&mut state.job_history, &state.job_meta);
    state.job_history.retain(crate::jobs::is_managed_job);
}

impl AppStore {
    pub fn path() -> PathBuf {
        crate::settings::AppSettings::config_dir().join("store.json")
    }

    pub fn load() -> Self {
        let mut store = if let Ok(text) = std::fs::read_to_string(Self::path()) {
            serde_json::from_str::<AppStore>(&text).unwrap_or_default()
        } else {
            Self::default()
        };
        store.seed_alert_defaults(true);
        let before: HashMap<u64, String> = store
            .job_history
            .iter()
            .map(|job| (job.id, job.status.clone()))
            .collect();
        store.sanitize_job_history();
        for state in store.backend_states.values_mut() {
            sanitize_backend_jobs(state);
        }
        if store.job_history.len() != before.len()
            || store.job_history.iter().any(|job| {
                before
                    .get(&job.id)
                    .is_some_and(|status| status != &job.status)
            })
        {
            let _ = store.save();
        }
        store
    }

    pub fn sanitize_job_history(&mut self) {
        finalize_stale_jobs(&mut self.job_history, &self.job_meta);
        self.job_history.retain(crate::jobs::is_managed_job);
    }

    /// Preparing rows from a previous process cannot be live; keep their
    /// snapshots but do not re-inject them as in-progress after restart.
    pub fn finalize_stale_preparing(&mut self) {
        finalize_stale_jobs(&mut self.job_history, &self.job_meta);
    }

    /// Park the active job/mount/serve working set and restore the target backend.
    pub fn swap_backend_state(
        &mut self,
        from: &str,
        to: &str,
        from_mounts: Vec<MountedRemote>,
        from_serves: Vec<ServeItem>,
    ) -> (Vec<MountedRemote>, Vec<ServeItem>) {
        let from_key = crate::layout::backend_key(from);
        let to_key = crate::layout::backend_key(to);
        if from_key == to_key {
            return (from_mounts, from_serves);
        }
        self.backend_states.insert(
            from_key,
            BackendUiState {
                job_history: std::mem::take(&mut self.job_history),
                job_meta: std::mem::take(&mut self.job_meta),
                mounts: from_mounts,
                serves: from_serves,
            },
        );
        let incoming = self.backend_states.remove(&to_key).unwrap_or_default();
        self.job_history = incoming.job_history;
        self.job_meta = incoming.job_meta;
        self.sanitize_job_history();
        (incoming.mounts, incoming.serves)
    }

    pub fn seed_alert_defaults(&mut self, notifications_on: bool) -> bool {
        let mut changed = false;
        if !self
            .alert_actions
            .iter()
            .any(|action| action.id == DEFAULT_ALERT_ACTION_ID)
        {
            self.alert_actions.insert(
                0,
                AlertAction {
                    id: DEFAULT_ALERT_ACTION_ID.to_string(),
                    name: "OS Toast".into(),
                    enabled: notifications_on,
                    kind: "os_toast".into(),
                    config: json!({
                        "title": "Rclone Manager",
                        "body_template": "{{title}}: {{body}}"
                    }),
                },
            );
            changed = true;
        }
        if !self
            .alert_rules
            .iter()
            .any(|rule| rule.id == DEFAULT_ALERT_RULE_ID)
        {
            self.alert_rules.insert(
                0,
                AlertRule {
                    id: DEFAULT_ALERT_RULE_ID.to_string(),
                    name: "Notify on events".into(),
                    enabled: notifications_on,
                    event_filter: vec![],
                    severity_min: AlertSeverity::Info,
                    remote_filter: vec![],
                    backend_filter: vec![],
                    profile_filter: vec![],
                    origin_filter: vec![],
                    action_ids: vec![DEFAULT_ALERT_ACTION_ID.to_string()],
                    cooldown_secs: 0,
                    auto_acknowledge: true,
                    last_fired: None,
                    fire_count: 0,
                },
            );
            changed = true;
        }
        changed
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(crate::settings::AppSettings::config_dir())?;
        std::fs::write(
            Self::path(),
            serde_json::to_string_pretty(self).unwrap_or_default(),
        )
    }

    pub fn ensure_remote_order(&mut self, names: &[String]) {
        if self.remote_order.is_empty() {
            self.remote_order = names.to_vec();
            return;
        }
        for name in names {
            if !self.remote_order.iter().any(|n| n == name) {
                self.remote_order.push(name.clone());
            }
        }
    }

    pub fn move_remote(&mut self, name: &str, delta: isize) -> bool {
        let Some(idx) = self.remote_order.iter().position(|n| n == name) else {
            return false;
        };
        let last = self.remote_order.len().saturating_sub(1) as isize;
        let next = (idx as isize + delta).clamp(0, last) as usize;
        if next == idx {
            return false;
        }
        self.remote_order.swap(idx, next);
        true
    }

    pub fn set_remote_hidden(&mut self, name: &str, hidden: bool) {
        if hidden {
            if !self.hidden_remotes.iter().any(|n| n == name) {
                self.hidden_remotes.push(name.to_string());
            }
        } else {
            self.hidden_remotes.retain(|n| n != name);
        }
    }

    pub fn toggle_remote_hidden(&mut self, name: &str) -> bool {
        let hidden = !self.hidden_remotes.iter().any(|n| n == name);
        self.set_remote_hidden(name, hidden);
        hidden
    }

    pub fn is_automation_paused(&self, id: &str) -> bool {
        self.automation_paused.iter().any(|item| item == id)
    }

    pub fn toggle_automation_paused(&mut self, id: &str) -> bool {
        if let Some(idx) = self.automation_paused.iter().position(|item| item == id) {
            self.automation_paused.remove(idx);
            false
        } else {
            self.automation_paused.push(id.to_string());
            true
        }
    }

    pub fn remember_job(&mut self, job: JobInfo) {
        if !crate::jobs::is_managed_job(&job) {
            return;
        }
        self.job_history.retain(|existing| existing.id != job.id);
        self.job_history.insert(0, job);
        if self.job_history.len() > 80 {
            self.job_history.truncate(80);
        }
        self.prune_job_meta();
    }

    pub fn prune_job_meta(&mut self) {
        if self.job_meta.len() <= 200 {
            let history: std::collections::HashSet<u64> =
                self.job_history.iter().map(|job| job.id).collect();
            if self.job_meta.len() <= history.len().saturating_add(120) {
                return;
            }
        }
        let history: std::collections::HashSet<u64> =
            self.job_history.iter().map(|job| job.id).collect();
        let mut ids: Vec<u64> = self.job_meta.keys().copied().collect();
        ids.sort_unstable();
        ids.reverse();
        let mut keep = history;
        for id in ids.into_iter().take(200) {
            keep.insert(id);
        }
        self.job_meta.retain(|id, _| keep.contains(id));
    }

    pub fn update_job_stats(&mut self, jobid: u64, stats: Value) -> bool {
        let Some(job) = self.job_history.iter_mut().find(|j| j.id == jobid) else {
            return false;
        };
        let bytes = stats.get("bytes").and_then(|v| v.as_u64()).unwrap_or(0) as f64;
        let total = stats
            .get("totalBytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as f64;
        if total > 0.0 {
            job.progress = (bytes / total).clamp(0.0, 1.0);
        }
        if let Some(list) = stats.get("transferring") {
            job.transferring = list.clone();
        }
        if stats
            .get("preparing")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && job.status != "running"
        {
            job.status = "preparing".into();
        }
        job.stats = stats;
        true
    }

    pub fn rename_runtime_profile(&mut self, remote: &str, from: &str, to: &str) -> usize {
        if remote.is_empty() || from.is_empty() || to.is_empty() || from == to {
            return 0;
        }
        let mut updated = 0;
        for meta in self.job_meta.values_mut() {
            if meta.remote == remote && meta.profile == from {
                meta.profile = to.to_string();
                updated += 1;
            }
        }
        for job in &mut self.job_history {
            if job.remote == remote && job.profile == from {
                job.profile = to.to_string();
                updated += 1;
            }
        }
        let last_run_keys: Vec<String> = self.automation_last_run.keys().cloned().collect();
        for key in last_run_keys {
            if let Some(next) = rewrite_automation_id(&key, remote, from, to) {
                if let Some(value) = self.automation_last_run.remove(&key) {
                    self.automation_last_run.insert(next, value);
                    updated += 1;
                }
            }
        }
        for id in &mut self.automation_paused {
            if let Some(next) = rewrite_automation_id(id, remote, from, to) {
                *id = next;
                updated += 1;
            }
        }
        let runtime_keys: Vec<String> = self.automation_runtime.keys().cloned().collect();
        for key in runtime_keys {
            if let Some(next) = rewrite_automation_id(&key, remote, from, to) {
                if let Some(value) = self.automation_runtime.remove(&key) {
                    self.automation_runtime.insert(next, value);
                    updated += 1;
                }
            }
        }
        updated
    }

    pub fn dismiss_job(&mut self, id: u64) {
        self.job_history.retain(|job| job.id != id);
    }

    pub fn remote_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.remotes.keys().cloned().collect();
        names.extend(self.remote_order.iter().cloned());
        names.sort();
        names.dedup();
        names
    }

    pub fn reset_remote_settings(&mut self, name: &str) {
        self.remotes.insert(name.to_string(), RemoteMeta::default());
        let markers = [format!("remote:{name}:"), format!(":{name}:")];
        self.automation_paused
            .retain(|id| !markers.iter().any(|m| id.contains(m.as_str())));
        self.automation_last_run
            .retain(|id, _| !markers.iter().any(|m| id.contains(m.as_str())));
        self.automation_runtime
            .retain(|id, _| !markers.iter().any(|m| id.contains(m.as_str())));
    }

    pub fn apply_delete_remote(&mut self, name: &str) {
        self.remotes.remove(name);
        self.quick_runs.retain(|run| run.remote_name != name);
        self.job_history.retain(|job| job.remote != name);
        self.remote_order.retain(|n| n != name);
        self.hidden_remotes.retain(|n| n != name);
        self.logs.remove(name);
        let markers = [format!("remote:{name}:"), format!(":{name}:")];
        self.automation_paused
            .retain(|id| !markers.iter().any(|m| id.contains(m.as_str())));
        self.automation_last_run
            .retain(|id, _| !markers.iter().any(|m| id.contains(m.as_str())));
        self.automation_runtime
            .retain(|id, _| !markers.iter().any(|m| id.contains(m.as_str())));
    }

    pub fn log_operation(
        &mut self,
        remote: &str,
        operation: &str,
        message: &str,
        context: Option<&Value>,
    ) {
        self.push_log(
            remote,
            crate::logs::log_operation(
                crate::logs::LogLevel::Info,
                Some(remote),
                Some(operation),
                message,
                context,
            ),
        );
    }

    pub fn push_log(&mut self, remote: &str, line: String) {
        self.logs.entry(remote.to_string()).or_default().push(line);
        if let Some(lines) = self.logs.get_mut(remote) {
            if lines.len() > 2000 {
                let drain = lines.len() - 2000;
                lines.drain(0..drain);
            }
        }
    }

    pub fn unacknowledged_alerts(&self) -> usize {
        self.alert_history
            .iter()
            .filter(|e| !e.acknowledged)
            .count()
    }

    pub fn alert_stats(&self) -> AlertStats {
        let total = self.alert_history.len();
        let unacknowledged = self.unacknowledged_alerts();
        AlertStats {
            total,
            acknowledged: total.saturating_sub(unacknowledged),
            unacknowledged,
            delivered: self.alert_rules.iter().map(|r| r.fire_count).sum(),
            last_at: self.alert_history.first().map(|e| e.created_at),
        }
    }

    pub fn acknowledge_all(&mut self) {
        for event in &mut self.alert_history {
            event.acknowledged = true;
        }
    }

    pub fn acknowledge_alert(&mut self, id: &str) -> bool {
        if let Some(event) = self.alert_history.iter_mut().find(|e| e.id == id) {
            event.acknowledged = true;
            true
        } else {
            false
        }
    }

    pub fn clear_alert_history(&mut self) {
        self.alert_history.clear();
    }

    pub fn filter_alert_history(&self, query: &str, severity: Option<&str>) -> Vec<AlertEvent> {
        self.filter_alerts(&AlertHistoryFilter {
            query: query.to_string(),
            severity: severity.map(|s| s.to_string()),
            ..AlertHistoryFilter::default()
        })
    }

    pub fn filter_alerts(&self, filter: &AlertHistoryFilter) -> Vec<AlertEvent> {
        let q = filter.query.trim().to_ascii_lowercase();
        let severity = filter
            .severity
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let remote = filter
            .remote
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let profile = filter
            .profile
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let backend = filter
            .backend
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let event_kind = filter
            .event_kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
        let rule_id = filter
            .rule_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        self.alert_history
            .iter()
            .filter(|event| {
                if let Some(wanted) = severity {
                    if !event.severity.as_str().eq_ignore_ascii_case(wanted) {
                        return false;
                    }
                }
                if let Some(wanted) = event_kind {
                    if !event.kind.as_str().eq_ignore_ascii_case(wanted) {
                        return false;
                    }
                }
                if let Some(wanted) = filter.acknowledged {
                    if event.acknowledged != wanted {
                        return false;
                    }
                }
                if let Some(wanted) = rule_id {
                    if !event.rule_id.eq_ignore_ascii_case(wanted) {
                        return false;
                    }
                }
                if !filter.origins.is_empty()
                    && !filter
                        .origins
                        .iter()
                        .any(|o| event.origin.eq_ignore_ascii_case(o))
                {
                    return false;
                }
                if let Some(wanted) = remote {
                    if !event.remote.eq_ignore_ascii_case(wanted) {
                        return false;
                    }
                }
                if let Some(wanted) = profile {
                    if !event.profile.eq_ignore_ascii_case(wanted) {
                        return false;
                    }
                }
                if let Some(wanted) = backend {
                    if !event.backend.eq_ignore_ascii_case(wanted) {
                        return false;
                    }
                }
                if q.is_empty() {
                    return true;
                }
                event.title.to_ascii_lowercase().contains(&q)
                    || event.body.to_ascii_lowercase().contains(&q)
                    || event.kind.as_str().contains(q.as_str())
                    || event.severity.as_str().contains(q.as_str())
                    || event.remote.to_ascii_lowercase().contains(&q)
                    || event.origin.to_ascii_lowercase().contains(&q)
                    || event.profile.to_ascii_lowercase().contains(&q)
                    || event.backend.to_ascii_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }

    pub fn alert_filter_values(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut remotes = Vec::new();
        let mut profiles = Vec::new();
        let mut backends = Vec::new();
        for event in &self.alert_history {
            if !event.remote.is_empty() && !remotes.iter().any(|r| r == &event.remote) {
                remotes.push(event.remote.clone());
            }
            if !event.profile.is_empty() && !profiles.iter().any(|r| r == &event.profile) {
                profiles.push(event.profile.clone());
            }
            if !event.backend.is_empty() && !backends.iter().any(|r| r == &event.backend) {
                backends.push(event.backend.clone());
            }
        }
        remotes.sort();
        profiles.sort();
        backends.sort();
        (remotes, profiles, backends)
    }

    pub fn record_event(&mut self, mut event: AlertEvent) {
        let matching: Vec<String> = self
            .alert_rules
            .iter()
            .filter(|rule| rule.matches(&event))
            .map(|rule| rule.id.clone())
            .collect();
        for id in matching {
            if event.rule_id.is_empty() {
                event.rule_id = id.clone();
            }
            if let Some(rule) = self.alert_rules.iter_mut().find(|r| r.id == id) {
                rule.last_fired = Some(Utc::now());
                rule.fire_count += 1;
                if rule.auto_acknowledge {
                    event.acknowledged = true;
                }
                let action_ids = rule.action_ids.clone();
                for action_id in action_ids {
                    if let Some(action) = self.alert_actions.iter().find(|a| a.id == action_id) {
                        if action.kind == "os_toast" && !self.notifications_enabled {
                            continue;
                        }
                        dispatch_action(action, &event);
                    }
                }
            }
        }
        self.alert_history.insert(0, event);
        if self.alert_history.len() > 500 {
            self.alert_history.truncate(500);
        }
    }

    pub fn remove_alert_rule(&mut self, id: &str) {
        self.alert_rules.retain(|rule| rule.id != id);
    }

    pub fn remove_alert_action(&mut self, id: &str) {
        self.alert_actions.retain(|action| action.id != id);
        for rule in &mut self.alert_rules {
            rule.action_ids.retain(|action_id| action_id != id);
        }
    }
}

pub fn alert_rule_matches(rule: &AlertRule, query: &str) -> bool {
    let severity = rule.severity_min.as_str();
    let state = if rule.enabled {
        "enabled on"
    } else {
        "disabled off"
    };
    crate::pref_search::any_field_matches(&[&rule.name, &rule.id, severity, state], query)
}

pub fn alert_action_matches(action: &AlertAction, query: &str) -> bool {
    let state = if action.enabled {
        "enabled on"
    } else {
        "disabled off"
    };
    crate::pref_search::any_field_matches(&[&action.name, &action.id, &action.kind, state], query)
}

pub fn dispatch_action(action: &AlertAction, event: &AlertEvent) {
    if !action.enabled {
        return;
    }
    let attempts = alert_retry_count(action) + 1;
    for attempt in 0..attempts {
        if dispatch_action_once(action, event) || attempt + 1 == attempts {
            break;
        }
    }
}

fn dispatch_action_once(action: &AlertAction, event: &AlertEvent) -> bool {
    let body = render_template(
        action
            .config
            .get("body_template")
            .and_then(|x| x.as_str())
            .unwrap_or("{{title}}: {{body}}"),
        event,
    );
    match action.kind.as_str() {
        "os_toast" => notify_rust::Notification::new()
            .appname("Rclone Manager")
            .summary(&event.title)
            .body(&event.body)
            .icon("folder-remote")
            .show()
            .is_ok(),
        "webhook" => {
            let Some(url) = action.config.get("url").and_then(|x| x.as_str()) else {
                return false;
            };
            let method = webhook_http_method(
                action
                    .config
                    .get("method")
                    .and_then(|x| x.as_str())
                    .unwrap_or("POST"),
            );
            let timeout = action
                .config
                .get("timeout_secs")
                .and_then(|x| x.as_u64())
                .unwrap_or(8)
                .max(1);
            let mut req = ureq::request(method, url);
            req = req.timeout(std::time::Duration::from_secs(timeout));
            if let Some(headers) = action.config.get("headers").and_then(|x| x.as_object()) {
                for (key, value) in headers {
                    if let Some(value) = value.as_str() {
                        req = req.set(key, value);
                    }
                }
            }
            req.send_string(&body).is_ok()
        }
        "telegram" => {
            let chat = action
                .config
                .get("chat_id")
                .and_then(|x| x.as_str())
                .unwrap_or_default();
            if chat.is_empty() {
                return false;
            }
            let botless = action.config.get("mode").and_then(|x| x.as_str()) == Some("botless");
            if botless {
                return ureq::get(&telegram_botless_url(chat, &body)).call().is_ok();
            }
            let token = action
                .config
                .get("bot_token")
                .and_then(|x| x.as_str())
                .unwrap_or_default();
            if token.is_empty() {
                return false;
            }
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            ureq::post(&url)
                .send_json(json!({
                    "chat_id": chat,
                    "text": body
                }))
                .is_ok()
        }
        "whatsapp" => {
            let Some(url) = whatsapp_request_url(&action.config, &body) else {
                return false;
            };
            ureq::get(&url).call().is_ok()
        }
        "script" => {
            let Some(cmd) = action.config.get("command").and_then(|x| x.as_str()) else {
                return false;
            };
            if cmd.is_empty() {
                return false;
            }
            let mut process = std::process::Command::new(cmd);
            if let Some(args) = action.config.get("args").and_then(|x| x.as_array()) {
                for arg in args {
                    if let Some(arg) = arg.as_str() {
                        process.arg(arg);
                    }
                }
            }
            if let Some(vars) = action.config.get("env_vars").and_then(|x| x.as_object()) {
                for (key, value) in vars {
                    if let Some(value) = value.as_str() {
                        process.env(key, value);
                    }
                }
            }
            process
                .env("ALERT_TITLE", &event.title)
                .env("ALERT_BODY", &event.body)
                .env("ALERT_SEVERITY", event.severity.as_str())
                .env("ALERT_EVENT_KIND", event.kind.as_str())
                .env("ALERT_REMOTE", &event.remote)
                .env("ALERT_PROFILE", &event.profile)
                .env("ALERT_BACKEND", &event.backend)
                .env("ALERT_ORIGIN", &event.origin)
                .env("ALERT_TIMESTAMP", event.created_at.to_rfc3339())
                .env("ALERT_RULE_ID", &event.rule_id)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let timeout = action
                .config
                .get("timeout_secs")
                .and_then(|x| x.as_u64())
                .unwrap_or(8)
                .max(1);
            wait_script(process, timeout)
        }
        "email" => {
            let title = event.title.clone();
            let body = body.clone();
            let config = action.config.clone();
            crate::smtp::send_alert_email(&config, &title, &body).is_ok()
        }
        "mqtt" => crate::mqtt::publish_alert(&action.config, &body).is_ok(),
        other => {
            log::warn!("unknown alert action kind {other}");
            false
        }
    }
}

pub const ALERT_TEMPLATE_KEYS: &[&str] = &[
    "{{title}}",
    "{{body}}",
    "{{severity}}",
    "{{kind}}",
    "{{timestamp}}",
    "{{remote}}",
    "{{origin}}",
    "{{backend}}",
    "{{profile}}",
    "{{rule_id}}",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlertHistoryFilter {
    pub query: String,
    pub severity: Option<String>,
    pub event_kind: Option<String>,
    pub remote: Option<String>,
    pub profile: Option<String>,
    pub backend: Option<String>,
    pub acknowledged: Option<bool>,
    pub rule_id: Option<String>,
    pub origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertStats {
    pub total: usize,
    pub acknowledged: usize,
    pub unacknowledged: usize,
    pub delivered: u64,
    pub last_at: Option<DateTime<Utc>>,
}

pub fn render_template(template: &str, event: &AlertEvent) -> String {
    template
        .replace("{{title}}", &event.title)
        .replace("{{body}}", &event.body)
        .replace("{{severity}}", event.severity.as_str())
        .replace("{{kind}}", event.kind.as_str())
        .replace("{{event_kind}}", event.kind.as_str())
        .replace("{{timestamp}}", &event.created_at.to_rfc3339())
        .replace("{{remote}}", &event.remote)
        .replace("{{origin}}", &event.origin)
        .replace("{{backend}}", &event.backend)
        .replace("{{profile}}", &event.profile)
        .replace("{{rule_id}}", &event.rule_id)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteRemotePlan {
    pub name: String,
    pub mounts: Vec<String>,
    pub serves: Vec<String>,
    pub jobs: Vec<u64>,
    pub quick_runs: Vec<String>,
    pub automations: Vec<String>,
    pub job_history: usize,
}

impl DeleteRemotePlan {
    pub fn summary(&self) -> String {
        format!(
            "Delete {}?\n\nWill stop {} mounts, {} serves, and {} jobs.\nRemove {} quick runs, {} automations, and {} history entries.",
            self.name,
            self.mounts.len(),
            self.serves.len(),
            self.jobs.len(),
            self.quick_runs.len(),
            self.automations.len(),
            self.job_history
        )
    }
}

pub fn rewrite_automation_id(id: &str, remote: &str, from: &str, to: &str) -> Option<String> {
    let prefix = format!("remote:{remote}:");
    let rest = id.strip_prefix(&prefix)?;
    let (op, profile) = rest.rsplit_once(':')?;
    if profile == from && !to.is_empty() && from != to {
        Some(format!("remote:{remote}:{op}:{to}"))
    } else {
        None
    }
}

pub fn fs_belongs_to_remote(fs: &str, name: &str) -> bool {
    fs == name || fs == format!("{name}:") || fs.starts_with(&format!("{name}:"))
}

pub fn plan_delete_remote(
    name: &str,
    store: &AppStore,
    snap: &RuntimeSnapshot,
) -> DeleteRemotePlan {
    let mounts = snap
        .mounts
        .iter()
        .filter(|m| fs_belongs_to_remote(&m.fs, name))
        .map(|m| m.mount_point.clone())
        .collect();
    let serves = snap
        .serves
        .iter()
        .filter(|s| fs_belongs_to_remote(&s.fs, name))
        .map(|s| s.id.clone())
        .collect();
    let jobs = snap
        .jobs
        .iter()
        .filter(|j| j.remote == name && (j.status == "running" || j.status == "active"))
        .map(|j| j.id)
        .collect();
    let quick_runs = store
        .quick_runs
        .iter()
        .filter(|run| run.remote_name == name)
        .map(|run| run.name.clone())
        .collect();
    let automations: Vec<String> = store
        .automation_paused
        .iter()
        .chain(store.automation_last_run.keys())
        .chain(store.automation_runtime.keys())
        .filter(|id| id.contains(&format!("remote:{name}:")) || id.contains(&format!(":{name}:")))
        .cloned()
        .collect();
    let job_history = store
        .job_history
        .iter()
        .filter(|j| j.remote == name)
        .count();
    DeleteRemotePlan {
        name: name.to_string(),
        mounts,
        serves,
        jobs,
        quick_runs,
        automations,
        job_history,
    }
}

pub fn unique_remote_name(existing: &[String], base: &str) -> String {
    let stem = base
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end_matches('-');
    let stem = if stem.is_empty() { base } else { stem };
    if !existing.iter().any(|n| n == stem) {
        return stem.to_string();
    }
    for i in 2..1000 {
        let candidate = format!("{stem}-{i}");
        if !existing.iter().any(|n| n == &candidate) {
            return candidate;
        }
    }
    format!("{stem}-copy")
}

pub fn rewrite_remote_refs(value: &mut Value, from: &str, to: &str) {
    let needle = format!("{from}:");
    let replacement = format!("{to}:");
    match value {
        Value::String(s) => {
            if s.starts_with(&needle) {
                *s = s.replacen(&needle, &replacement, 1);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                rewrite_remote_refs(item, from, to);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                rewrite_remote_refs(item, from, to);
            }
        }
        _ => {}
    }
}

pub fn clone_remote_meta(store: &AppStore, from: &str, to: &str) -> Option<RemoteMeta> {
    let mut meta = store.remotes.get(from)?.clone();
    for profiles in meta.profiles.values_mut() {
        for profile in profiles.values_mut() {
            rewrite_remote_refs(&mut profile.rclone, from, to);
        }
    }
    Some(meta)
}

pub fn remote_is_mounted(name: &str, cfg: &Value, mounts: &[MountedRemote]) -> bool {
    let alias = cfg.get("remote").and_then(|v| v.as_str()).unwrap_or("");
    mounts
        .iter()
        .any(|m| mount_matches_remote(&m.fs, &m.mount_point, name, alias))
}

pub fn mount_matches_remote(fs: &str, mount_point: &str, name: &str, alias: &str) -> bool {
    let prefix = format!("{name}:");
    fs == name
        || fs == prefix
        || fs.starts_with(&prefix)
        || (!alias.is_empty() && paths_equivalent(fs, alias))
        || mount_point_named(mount_point, name)
}

fn paths_equivalent(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}

pub fn mount_point_named(mount_point: &str, name: &str) -> bool {
    let point = mount_point.trim_end_matches('/');
    point.ends_with(&format!("/{name}")) || point == name
}

pub fn build_remote_infos(
    dump: &Value,
    mounts: &[MountedRemote],
    serves: &[ServeItem],
    jobs: &[JobInfo],
    hidden: &[String],
) -> Vec<RemoteInfo> {
    let mut remotes = Vec::new();
    if let Some(obj) = dump.as_object() {
        for (name, cfg) in obj {
            let r#type = cfg
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            let mounted = remote_is_mounted(name, cfg, mounts);
            let serving = serves.iter().any(|s| s.fs.starts_with(&format!("{name}:")));
            let job_active = jobs
                .iter()
                .any(|j| j.remote == *name && (j.status == "running" || j.status == "active"));
            remotes.push(RemoteInfo {
                name: name.clone(),
                r#type,
                mounted,
                serving,
                job_active,
                hidden: hidden.contains(name),
                disk_label: String::new(),
            });
        }
    }
    remotes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    remotes
}

pub fn disk_label_from_about(about: &Value) -> String {
    let total = about.get("total").and_then(|x| x.as_i64()).unwrap_or(-1);
    let used = about.get("used").and_then(|x| x.as_i64()).unwrap_or(-1);
    let free = about.get("free").and_then(|x| x.as_i64()).unwrap_or(-1);
    if total < 0 && used < 0 && free < 0 {
        return "Not supported".into();
    }
    format!(
        "{} used / {} free of {}",
        format_bytes(used),
        format_bytes(free),
        format_bytes(total)
    )
}

pub fn disk_usage_ratio(about: &Value) -> Option<f64> {
    let total = about.get("total").and_then(|x| x.as_i64()).unwrap_or(-1);
    let used = about.get("used").and_then(|x| x.as_i64()).unwrap_or(-1);
    if total > 0 && used >= 0 {
        Some((used as f64 / total as f64).clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Angular `sortOptions` keys: `name-asc`, `name-desc`, `modified-desc`, …
pub fn sort_option_key(sort_by: &str, sort_desc: bool) -> &'static str {
    match (sort_by, sort_desc) {
        ("name", true) => "name-desc",
        ("modified", true) => "modified-desc",
        ("modified", false) => "modified-asc",
        ("size", true) => "size-desc",
        ("size", false) => "size-asc",
        _ => "name-asc",
    }
}

pub fn apply_sort_option(sort_by: &mut String, sort_desc: &mut bool, key: &str) {
    match key {
        "name-desc" => {
            *sort_by = "name".into();
            *sort_desc = true;
        }
        "modified-desc" => {
            *sort_by = "modified".into();
            *sort_desc = true;
        }
        "modified-asc" => {
            *sort_by = "modified".into();
            *sort_desc = false;
        }
        "size-desc" => {
            *sort_by = "size".into();
            *sort_desc = true;
        }
        "size-asc" => {
            *sort_by = "size".into();
            *sort_desc = false;
        }
        _ => {
            *sort_by = "name".into();
            *sort_desc = false;
        }
    }
}

pub fn sort_entries(entries: &mut [DirEntry], sort_by: &str, desc: bool) {
    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            return if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let ord = match sort_by {
            "size" => a.size.cmp(&b.size),
            "modified" | "modtime" => a.mod_time.cmp(&b.mod_time),
            "type" => a.mime.cmp(&b.mime),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_pairs_roundtrip_skips_empty_keys() {
        let config = json!({
            "headers": {
                "Content-Type": "application/json",
                "X-Token": "abc"
            }
        });
        let pairs = header_pairs(&config);
        assert_eq!(pairs.len(), 2);
        let text = pairs_to_header_text(&[
            ("Content-Type".into(), "application/json".into()),
            ("  ".into(), "ignored".into()),
            ("X-Token".into(), "abc".into()),
        ]);
        assert!(text.contains("Content-Type: application/json"));
        assert!(text.contains("X-Token: abc"));
        assert!(!text.contains("ignored"));
        assert_eq!(parse_header_lines(&text).len(), 2);
        let from_text =
            header_pairs_from_text("Content-Type: application/json\n: skip\nX-Token: abc");
        assert_eq!(from_text.len(), 2);
        assert!(from_text
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
    }

    #[test]
    fn mounts_alias_target_and_named_point() {
        let alias = json!({ "type": "alias", "remote": "/tmp/rclone-test-remote" });
        let mounts = vec![MountedRemote::new(
            "/tmp/rclone-test-remote",
            "/home/ubuntu/rclone-manager/testdrive",
        )];
        assert!(remote_is_mounted("testdrive", &alias, &mounts));
        assert!(!remote_is_mounted(
            "dummyexport",
            &json!({ "type": "local" }),
            &mounts
        ));
        let named = vec![MountedRemote::new("/home/ubuntu", "/tmp/mnt/dummyexport")];
        assert!(remote_is_mounted(
            "dummyexport",
            &json!({ "type": "local" }),
            &named
        ));
        let prefixed = vec![MountedRemote::new("drive:Photos", "/mnt/drive")];
        assert!(remote_is_mounted(
            "drive",
            &json!({ "type": "drive" }),
            &prefixed
        ));
        assert!(mount_matches_remote(
            "/tmp/rclone-test-remote",
            "/home/ubuntu/rclone-manager/testdrive",
            "testdrive",
            "/tmp/rclone-test-remote"
        ));
        assert!(mount_point_named(
            "/home/ubuntu/rclone-manager/testdrive",
            "testdrive"
        ));
    }

    #[test]
    fn quick_run_path_extraction() {
        let rclone = json!({
            "srcFs": "drive:src",
            "dstFs": "/tmp/out"
        });
        let (src, dst) = quick_run_paths(&rclone, OperationType::Sync);
        assert_eq!(src.as_deref(), Some("drive:src"));
        assert_eq!(dst.as_deref(), Some("/tmp/out"));
    }

    #[test]
    fn alert_rule_filters() {
        let mut rule = AlertRule::new("jobs".into());
        rule.event_filter = vec![AlertEventKind::Job];
        rule.severity_min = AlertSeverity::Warning;
        let mut event = AlertEvent::new(
            AlertEventKind::Job,
            AlertSeverity::Info,
            "x".into(),
            "y".into(),
        );
        assert!(!rule.matches(&event));
        event.severity = AlertSeverity::High;
        assert!(rule.matches(&event));
        event.kind = AlertEventKind::Mount;
        assert!(!rule.matches(&event));
        event.kind = AlertEventKind::Job;
        rule.backend_filter = vec!["local".into()];
        event.backend = "extra".into();
        assert!(!rule.matches(&event));
        event.backend = "local".into();
        rule.profile_filter = vec!["nightly".into()];
        event.profile = "default".into();
        assert!(!rule.matches(&event));
        event.profile = "nightly".into();
        assert!(rule.matches(&event));
    }

    #[test]
    fn remote_order_and_visibility() {
        let mut store = AppStore::default();
        store.ensure_remote_order(&["b".into(), "a".into(), "c".into()]);
        assert_eq!(store.remote_order, ["b", "a", "c"]);
        store.ensure_remote_order(&["b".into(), "a".into(), "c".into(), "d".into()]);
        assert_eq!(store.remote_order, ["b", "a", "c", "d"]);
        assert!(store.move_remote("a", -1));
        assert_eq!(store.remote_order, ["a", "b", "c", "d"]);
        assert!(!store.move_remote("a", -1));
        assert!(store.toggle_remote_hidden("c"));
        assert_eq!(store.hidden_remotes, ["c"]);
        assert!(!store.toggle_remote_hidden("c"));
        assert!(store.hidden_remotes.is_empty());
        assert!(!store.is_automation_paused("remote:drive:sync:default"));
        assert!(store.toggle_automation_paused("remote:drive:sync:default"));
        assert!(store.is_automation_paused("remote:drive:sync:default"));
        assert!(!store.toggle_automation_paused("remote:drive:sync:default"));
    }

    #[test]
    fn reset_remote_settings_keeps_remote_and_clears_profiles() {
        let mut store = AppStore::default();
        let mut meta = RemoteMeta::default();
        meta.show_on_tray = true;
        let mut profile = ProfileConfig::default();
        profile.name = "nightly".into();
        meta.upsert_profile(OperationType::Sync, profile);
        store.remotes.insert("drive".into(), meta);
        store.remote_order = vec!["drive".into()];
        store
            .automation_last_run
            .insert("remote:drive:sync:nightly".into(), Utc::now());
        store.reset_remote_settings("drive");
        let reset = store.remotes.get("drive").expect("remote stays");
        assert!(reset.profiles.is_empty());
        assert!(!reset.show_on_tray);
        assert_eq!(store.remote_order, ["drive"]);
        assert!(store.automation_last_run.is_empty());
    }

    #[test]
    fn disk_usage_ratio_from_about() {
        assert_eq!(
            disk_usage_ratio(&json!({"total": 100, "used": 25})),
            Some(0.25)
        );
        assert_eq!(disk_usage_ratio(&json!({"total": 0, "used": 1})), None);
        assert_eq!(disk_usage_ratio(&json!({})), None);
        assert_eq!(disk_label_from_about(&json!({})), "Not supported");
    }

    #[test]
    fn render_alert_template() {
        let mut event = AlertEvent::new(
            AlertEventKind::System,
            AlertSeverity::Info,
            "Hello".into(),
            "World".into(),
        );
        assert_eq!(render_template("{{title}} {{body}}", &event), "Hello World");
        assert!(render_template("{{timestamp}}", &event).contains('T'));
        event.rule_id = "rule-1".into();
        assert_eq!(
            render_template("{{rule_id}} {{kind}}", &event),
            "rule-1 system"
        );
    }

    #[test]
    fn job_status_from_flags() {
        fn status(finished: bool, success: bool) -> &'static str {
            if !finished {
                "running"
            } else if success {
                "completed"
            } else {
                "failed"
            }
        }
        assert_eq!(status(false, false), "running");
        assert_eq!(status(true, true), "completed");
        assert_eq!(status(true, false), "failed");
    }

    #[test]
    fn sort_dirs_first() {
        let mut entries = vec![
            DirEntry {
                name: "z.txt".into(),
                is_dir: false,
                size: 10,
                ..Default::default()
            },
            DirEntry {
                name: "a".into(),
                is_dir: true,
                ..Default::default()
            },
            DirEntry {
                name: "b.txt".into(),
                is_dir: false,
                size: 2,
                ..Default::default()
            },
        ];
        sort_entries(&mut entries, "name", false);
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "b.txt");
        assert_eq!(sort_option_key("name", false), "name-asc");
        assert_eq!(sort_option_key("modified", true), "modified-desc");
        assert_eq!(sort_option_key("size", false), "size-asc");
        let mut by = "name".to_string();
        let mut desc = false;
        apply_sort_option(&mut by, &mut desc, "size-desc");
        assert_eq!(by, "size");
        assert!(desc);
        apply_sort_option(&mut by, &mut desc, "name-asc");
        assert_eq!(by, "name");
        assert!(!desc);
    }

    #[test]
    fn profile_and_helper_crud() {
        let mut meta = RemoteMeta::default();
        let mut profile = ProfileConfig::default();
        profile.name = "default".into();
        profile.app.auto_start = true;
        meta.upsert_profile(OperationType::Sync, profile);
        assert_eq!(meta.profile_names(OperationType::Sync), vec!["default"]);
        assert!(meta.clone_profile(OperationType::Sync, "default", "nightly"));
        assert!(meta.rename_profile(OperationType::Sync, "nightly", "weekly"));
        assert_eq!(
            meta.profile_names(OperationType::Sync),
            vec!["default", "weekly"]
        );
        assert!(meta.remove_profile(OperationType::Sync, "weekly"));
        assert_eq!(meta.profile_names(OperationType::Sync), vec!["default"]);

        meta.upsert_helper("vfs", "fast", json!({ "CacheMode": "full" }));
        assert_eq!(meta.helper_names("vfs"), vec!["fast"]);
        assert!(meta.rename_helper("vfs", "fast", "full"));
        assert_eq!(
            meta.helper_profile("vfs", "full").unwrap()["CacheMode"],
            "full"
        );
        assert!(meta.remove_helper("vfs", "full"));
        assert!(meta.helper_names("vfs").is_empty());
        meta.upsert_helper("runtime", "", json!({}));
        assert!(meta.helper_names("runtime").is_empty());
        meta.primary_actions = vec!["sync".into(), "copy".into()];
        assert_eq!(
            meta.visible_operations(),
            vec![OperationType::Sync, OperationType::Copy]
        );
    }

    #[test]
    fn job_history_replaces_and_caps() {
        let mut store = AppStore::default();
        let mk = |id: u64, status: &str| JobInfo {
            id,
            operation: "copy".into(),
            remote: "drive".into(),
            profile: "default".into(),
            status: status.into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: format!("job/{id}"),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        };
        for id in 1..=82u64 {
            store.remember_job(mk(id, "completed"));
        }
        assert_eq!(store.job_history.len(), 80);
        assert_eq!(store.job_history[0].id, 82);
        store.remember_job(mk(82, "failed"));
        assert_eq!(store.job_history.iter().filter(|j| j.id == 82).count(), 1);
        assert_eq!(store.job_history[0].status, "failed");
        store.dismiss_job(82);
        assert!(store.job_history.iter().all(|j| j.id != 82));
        store.remember_job(mk(9, "preparing"));
        assert!(store.update_job_stats(
            9,
            json!({
                "bytes": 16,
                "totalBytes": 32,
                "transferring": [{"name": "a.txt"}],
                "preparing": true
            })
        ));
        assert_eq!(store.job_history[0].progress, 0.5);
        assert_eq!(store.job_history[0].transferring[0]["name"], "a.txt");
        assert_eq!(store.job_history[0].status, "preparing");
        assert!(!store.update_job_stats(99, json!({})));
        store.remember_job(JobInfo {
            id: 540356,
            operation: "job/540356".into(),
            remote: String::new(),
            profile: "default".into(),
            status: "completed".into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: "job/540356".into(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 1.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        });
        assert!(store.job_history.iter().all(|job| job.id != 540356));
    }

    #[test]
    fn swap_backend_state_isolates_jobs() {
        let mk = |id: u64, status: &str, remote: &str| JobInfo {
            id,
            operation: "copy".into(),
            remote: remote.into(),
            profile: "default".into(),
            status: status.into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: format!("job/{id}"),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        };
        let mut store = AppStore::default();
        store.remember_job(mk(11, "completed", "localdrive"));
        store.job_meta.insert(
            11,
            JobMeta {
                origin: "dashboard".into(),
                backend: "local".into(),
                ..Default::default()
            },
        );
        let (mounts, serves) = store.swap_backend_state("local", "office", vec![], vec![]);
        assert!(mounts.is_empty());
        assert!(serves.is_empty());
        assert!(store.job_history.is_empty());
        assert!(store.job_meta.is_empty());
        store.remember_job(mk(11, "running", "officedrive"));
        store.job_meta.insert(
            11,
            JobMeta {
                origin: "dashboard".into(),
                backend: "office".into(),
                ..Default::default()
            },
        );
        store.swap_backend_state("office", "local", vec![], vec![]);
        assert_eq!(store.job_history.len(), 1);
        assert_eq!(store.job_history[0].remote, "localdrive");
        assert_eq!(store.job_history[0].status, "completed");
        assert_eq!(store.job_meta[&11].backend, "local");
        store.swap_backend_state("local", "office", vec![], vec![]);
        assert_eq!(store.job_history[0].remote, "officedrive");
        assert_eq!(store.job_history[0].status, "running");
        assert_eq!(store.job_meta[&11].backend, "office");
        let same = store.swap_backend_state("office", "office", vec![], vec![]);
        assert_eq!(store.job_history[0].id, 11);
        assert!(same.0.is_empty());
        let text = serde_json::to_string(&store).unwrap();
        let loaded: AppStore = serde_json::from_str(&text).unwrap();
        assert!(loaded.backend_states.contains_key("local"));
        assert_eq!(
            loaded.backend_states["local"].job_history[0].remote,
            "localdrive"
        );
        assert!(loaded.backend_states["local"].mounts.is_empty());
    }

    #[test]
    fn job_meta_survives_store_json() {
        let mut store = AppStore::default();
        store.job_meta.insert(
            3,
            JobMeta {
                origin: "filemanager".into(),
                group: "filemanager-upload/abc".into(),
                transfer_snapshot: json!([{ "name": "a.txt", "src": "/tmp/a.txt" }]),
                ..Default::default()
            },
        );
        let text = serde_json::to_string(&store).unwrap();
        let loaded: AppStore = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded.job_meta[&3].group, "filemanager-upload/abc");
        assert_eq!(
            loaded.job_meta[&3].transfer_snapshot[0]["src"],
            "/tmp/a.txt"
        );
    }

    #[test]
    fn finalizes_preparing_jobs_from_previous_session() {
        let mut store = AppStore::default();
        store.job_meta.insert(
            4,
            JobMeta {
                transfer_snapshot: json!([{ "name": "k.txt" }]),
                ..Default::default()
            },
        );
        store.job_history.push(JobInfo {
            id: 4,
            operation: "upload".into(),
            remote: "testdrive".into(),
            profile: "default".into(),
            status: "preparing".into(),
            origin: "filemanager".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: "/tmp/k.txt".into(),
            dst: "k.txt".into(),
            group: "filemanager-upload/x".into(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        });
        store.job_history.push(JobInfo {
            id: 5,
            operation: "upload".into(),
            remote: "testdrive".into(),
            profile: "default".into(),
            status: "starting".into(),
            origin: "filemanager".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: String::new(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        });
        store.finalize_stale_preparing();
        assert_eq!(store.job_history[0].status, "completed");
        assert_eq!(store.job_history[1].status, "failed");
    }

    #[test]
    fn seeds_default_alert_action_and_rule() {
        let mut store = AppStore::default();
        assert!(store.seed_alert_defaults(true));
        assert_eq!(store.alert_actions[0].id, DEFAULT_ALERT_ACTION_ID);
        assert_eq!(store.alert_actions[0].kind, "os_toast");
        assert!(store.alert_actions[0].enabled);
        assert_eq!(store.alert_rules[0].id, DEFAULT_ALERT_RULE_ID);
        assert_eq!(
            store.alert_rules[0].action_ids,
            vec![DEFAULT_ALERT_ACTION_ID]
        );
        assert!(store.alert_rules[0].auto_acknowledge);
        assert!(!store.seed_alert_defaults(false));
        assert!(alert_rule_matches(&store.alert_rules[0], "default"));
        assert!(alert_rule_matches(&store.alert_rules[0], ""));
        assert!(!alert_rule_matches(&store.alert_rules[0], "webhook"));
        assert!(alert_action_matches(&store.alert_actions[0], "toast"));
        assert!(alert_action_matches(&store.alert_actions[0], "enabled"));
        store.remove_alert_action(&store.alert_actions[0].id.clone());
        assert!(store.alert_actions.is_empty());
        assert!(store.alert_rules[0].action_ids.is_empty());
        store.remove_alert_rule(&store.alert_rules[0].id.clone());
        assert!(store.alert_rules.is_empty());
        store.alert_actions.clear();
        store.alert_rules.clear();
        assert!(store.seed_alert_defaults(false));
        store.alert_rules[0].fire_count = 3;
        store.alert_history.push(AlertEvent::new(
            AlertEventKind::System,
            AlertSeverity::Info,
            "hello".into(),
            "world".into(),
        ));
        store.alert_history[0].acknowledged = true;
        let stats = store.alert_stats();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.acknowledged, 1);
        assert_eq!(stats.unacknowledged, 0);
        assert_eq!(stats.delivered, 3);
        assert!(stats.last_at.is_some());
        assert!(!store.alert_actions[0].enabled);
        assert!(!store.alert_rules[0].enabled);
    }

    #[test]
    fn filters_and_acknowledges_alert_history() {
        let mut store = AppStore::default();
        let mut high = AlertEvent::new(
            AlertEventKind::Job,
            AlertSeverity::High,
            "Copy failed".into(),
            "drive:a".into(),
        );
        high.remote = "drive".into();
        let info = AlertEvent::new(
            AlertEventKind::System,
            AlertSeverity::Info,
            "Engine ready".into(),
            "ok".into(),
        );
        let high_id = high.id.clone();
        store.alert_history.push(high);
        store.alert_history.push(info);
        assert_eq!(store.filter_alert_history("copy", None).len(), 1);
        assert_eq!(store.filter_alert_history("", Some("info")).len(), 1);
        assert_eq!(store.filter_alert_history("drive", Some("high")).len(), 1);
        assert!(store.filter_alert_history("missing", None).is_empty());
        store.alert_history[0].profile = "nightly".into();
        store.alert_history[0].backend = "local".into();
        assert_eq!(
            store
                .filter_alerts(&AlertHistoryFilter {
                    remote: Some("drive".into()),
                    profile: Some("nightly".into()),
                    backend: Some("local".into()),
                    ..AlertHistoryFilter::default()
                })
                .len(),
            1
        );
        assert!(store
            .filter_alerts(&AlertHistoryFilter {
                remote: Some("dropbox".into()),
                ..AlertHistoryFilter::default()
            })
            .is_empty());
        assert_eq!(
            store
                .filter_alerts(&AlertHistoryFilter {
                    event_kind: Some("job".into()),
                    ..AlertHistoryFilter::default()
                })
                .len(),
            1
        );
        assert_eq!(
            store
                .filter_alerts(&AlertHistoryFilter {
                    acknowledged: Some(false),
                    ..AlertHistoryFilter::default()
                })
                .len(),
            2
        );
        store.alert_history[0].rule_id = "rule-high".into();
        store.alert_history[0].origin = "dashboard".into();
        assert_eq!(
            store
                .filter_alerts(&AlertHistoryFilter {
                    rule_id: Some("rule-high".into()),
                    origins: vec!["dashboard".into()],
                    ..AlertHistoryFilter::default()
                })
                .len(),
            1
        );
        assert!(store.acknowledge_alert(&high_id));
        assert_eq!(
            store
                .filter_alerts(&AlertHistoryFilter {
                    acknowledged: Some(true),
                    ..AlertHistoryFilter::default()
                })
                .len(),
            1
        );
        assert!(!store.acknowledge_alert("missing"));
        assert!(store
            .alert_history
            .iter()
            .find(|e| e.id == high_id)
            .is_some_and(|e| e.acknowledged));
        store.clear_alert_history();
        assert!(store.alert_history.is_empty());
        assert_eq!(store.unacknowledged_alerts(), 0);
    }

    #[test]
    fn builds_kind_specific_alert_config() {
        let draft = AlertActionDraft {
            url: "https://hooks.example/x".into(),
            method: "PUT".into(),
            token: "secret".into(),
            extra: "123".into(),
            headers: "X-Token: abc".into(),
            body: "{{title}}".into(),
            retry_count: 2,
            provider: String::new(),
            ..Default::default()
        };
        let webhook = alert_action_config("webhook", &draft);
        assert_eq!(webhook["url"], "https://hooks.example/x");
        assert_eq!(webhook["method"], "PUT");
        assert_eq!(webhook["retry_count"], 2);
        assert_eq!(webhook["timeout_secs"], 8);
        assert_eq!(webhook["tls_verify"], true);
        assert_eq!(webhook["headers"]["X-Token"], "abc");
        assert!(webhook.get("bot_token").is_none());
        let telegram = alert_action_config("telegram", &draft);
        assert_eq!(telegram["bot_token"], "secret");
        assert_eq!(telegram["chat_id"], "123");
        assert_eq!(telegram["mode"], "bot");
        let botless = alert_action_config(
            "telegram",
            &AlertActionDraft {
                telegram_mode: "botless".into(),
                extra: "ops".into(),
                ..Default::default()
            },
        );
        assert_eq!(botless["mode"], "botless");
        assert!(telegram_botless_url("ops", "hi").contains("user=%40ops"));
        let email = alert_action_config(
            "email",
            &AlertActionDraft {
                url: "smtp.example".into(),
                method: "465".into(),
                extra: "ops@example.com".into(),
                extra2: "alerts@example.com".into(),
                ..draft.clone()
            },
        );
        assert_eq!(email["smtp_port"], 465);
        assert_eq!(email["to"], "ops@example.com");
        assert_eq!(email["from"], "alerts@example.com");
        assert_eq!(email["subject_template"], "{{title}}");
        assert_eq!(email["encryption"], "starttls");
        assert_eq!(email["username"], "");
        let email_user = alert_action_config(
            "email",
            &AlertActionDraft {
                username: "alerts".into(),
                extra: "ops@example.com".into(),
                ..Default::default()
            },
        );
        assert_eq!(email_user["username"], "alerts");
        let email_tls = alert_action_config(
            "email",
            &AlertActionDraft {
                encryption: "tls".into(),
                extra: "ops@example.com".into(),
                ..Default::default()
            },
        );
        assert_eq!(email_tls["encryption"], "tls");
        let mqtt = alert_action_config(
            "mqtt",
            &AlertActionDraft {
                url: "mqtt://broker:1883".into(),
                method: "rclone/alerts".into(),
                qos: 2,
                retain: true,
                username: "mqtt".into(),
                use_tls: true,
                ..Default::default()
            },
        );
        assert_eq!(mqtt["qos"], 2);
        assert_eq!(mqtt["retain"], true);
        assert_eq!(mqtt["username"], "mqtt");
        assert_eq!(mqtt["use_tls"], true);
        let script = alert_action_config(
            "script",
            &AlertActionDraft {
                extra: "/usr/local/bin/notify".into(),
                extra2: "--once --quiet".into(),
                ..Default::default()
            },
        );
        assert_eq!(script["command"], "/usr/local/bin/notify");
        assert_eq!(script["args"], json!(["--once", "--quiet"]));
        let script_env = alert_action_config(
            "script",
            &AlertActionDraft {
                extra: "/bin/true".into(),
                env_vars: "TEAM=ops\nCHANNEL=alerts".into(),
                ..Default::default()
            },
        );
        assert_eq!(script_env["env_vars"]["TEAM"], "ops");
        assert_eq!(script_env["env_vars"]["CHANNEL"], "alerts");
        assert_eq!(
            env_vars_to_text(&json!({"env_vars": {"TEAM": "ops"}})),
            "TEAM=ops"
        );
        let discord = webhook_preset_body("discord");
        assert!(discord.contains("{{title}}"));
        assert!(discord.contains("{{timestamp}}"));
        let slack = webhook_preset_body("slack");
        assert!(slack.contains("*{{title}}*"));
        assert!(webhook_preset_body("unknown").is_empty());
        assert_eq!(
            ensure_content_type_json(""),
            "Content-Type: application/json"
        );
        assert_eq!(
            ensure_content_type_json("X-Token: abc"),
            "X-Token: abc\nContent-Type: application/json"
        );
        assert_eq!(
            ensure_content_type_json("Content-Type: text/plain"),
            "Content-Type: text/plain"
        );
        assert_eq!(webhook_http_method("put"), "PUT");
        assert_eq!(webhook_http_method("GET"), "GET");
        assert_eq!(webhook_http_method(""), "POST");
        assert!(!wait_script(
            {
                let mut cmd = std::process::Command::new("sleep");
                cmd.arg("2");
                cmd
            },
            1
        ));
        assert!(wait_script(std::process::Command::new("true"), 2));
        let wa = alert_action_config(
            "whatsapp",
            &AlertActionDraft {
                url: "https://gw.example/wa".into(),
                extra: "+1555".into(),
                token: "key".into(),
                provider: "custom_gateway".into(),
                ..Default::default()
            },
        );
        assert_eq!(wa["provider"], "custom_gateway");
        let url = whatsapp_request_url(&wa, "hi").unwrap();
        assert!(url.starts_with("https://gw.example/wa?"));
        assert!(url.contains("phone=%2B1555"));
        assert!(
            whatsapp_request_url(&json!({"phone":"+1","apikey":"k"}), "x")
                .unwrap()
                .contains("callmebot.com")
        );
        assert!(
            whatsapp_request_url(&json!({"phone":"+1","provider":"custom_gateway"}), "x").is_none()
        );
        let headers = parse_header_lines("X-Token: abc\nContent-Type: application/json\nbadline");
        assert_eq!(headers["X-Token"], "abc");
        assert_eq!(headers["Content-Type"], "application/json");
        assert_eq!(headers.len(), 2);
        assert!(headers_to_text(&json!({"headers": {"A": "1"}})).contains("A: 1"));
        assert_eq!(
            script_args("  --once   --quiet "),
            vec!["--once", "--quiet"]
        );
    }

    #[test]
    fn plans_and_applies_remote_delete() {
        let mut store = AppStore::default();
        store.remotes.insert("drive".into(), RemoteMeta::default());
        store.quick_runs.push(QuickRun::new(
            "Nightly".into(),
            OperationType::Sync,
            "drive".into(),
        ));
        store.job_history.push(JobInfo {
            id: 1,
            operation: "sync".into(),
            remote: "drive".into(),
            profile: "default".into(),
            status: "completed".into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: "drive:a".into(),
            dst: "/tmp".into(),
            group: "job/1".into(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        });
        store.remote_order = vec!["drive".into(), "photos".into()];
        store
            .automation_last_run
            .insert("remote:drive:sync:default".into(), Utc::now());
        let snap = RuntimeSnapshot {
            mounts: vec![MountedRemote::new("drive:Photos", "/mnt/drive")],
            serves: vec![ServeItem {
                id: "s1".into(),
                addr: "127.0.0.1:8080".into(),
                fs: "drive:".into(),
                serve_type: "http".into(),
                origin: "dashboard".into(),
                profile: "default".into(),
                option_count: 0,
            }],
            jobs: vec![JobInfo {
                id: 9,
                operation: "sync".into(),
                remote: "drive".into(),
                profile: "default".into(),
                status: "running".into(),
                origin: "dashboard".into(),
                start_time: Utc::now(),
                error: None,
                dry_run: false,
                src: String::new(),
                dst: String::new(),
                group: "job/9".into(),
                stats: json!({}),
                transferring: json!([]),
                duration: 0.0,
                progress: 0.0,
                output: json!({}),
                completed: json!([]),
                parent_job_id: None,
            }],
            ..RuntimeSnapshot::default()
        };
        let plan = plan_delete_remote("drive", &store, &snap);
        assert_eq!(plan.mounts, vec!["/mnt/drive"]);
        assert_eq!(plan.serves, vec!["s1"]);
        assert_eq!(plan.jobs, vec![9]);
        assert_eq!(plan.quick_runs, vec!["Nightly"]);
        assert_eq!(plan.job_history, 1);
        assert!(plan.summary().contains("Delete drive"));
        store.apply_delete_remote("drive");
        assert!(!store.remotes.contains_key("drive"));
        assert!(store.quick_runs.is_empty());
        assert!(store.job_history.is_empty());
        assert_eq!(store.remote_order, vec!["photos"]);
        assert!(store.automation_last_run.is_empty());
    }

    #[test]
    fn clones_remote_meta_and_rewrites_paths() {
        let mut store = AppStore::default();
        let mut meta = RemoteMeta::default();
        meta.upsert_profile(
            OperationType::Sync,
            ProfileConfig {
                name: "default".into(),
                rclone: json!({ "srcFs": "drive:Photos", "dstFs": "/tmp" }),
                ..ProfileConfig::default()
            },
        );
        store.remotes.insert("drive".into(), meta);
        store
            .remotes
            .insert("drive-2".into(), RemoteMeta::default());
        let names = store.remote_names();
        assert_eq!(unique_remote_name(&names, "drive-2"), "drive-3");
        assert_eq!(unique_remote_name(&["drive".into()], "drive"), "drive-2");
        let cloned = clone_remote_meta(&store, "drive", "photos").unwrap();
        assert_eq!(
            cloned
                .get_profile(OperationType::Sync, "default")
                .unwrap()
                .rclone["srcFs"],
            "photos:Photos"
        );
    }

    #[test]
    fn renames_runtime_profile_cache() {
        assert_eq!(
            rewrite_automation_id("remote:drive:sync:nightly", "drive", "nightly", "weekly"),
            Some("remote:drive:sync:weekly".into())
        );
        assert_eq!(
            rewrite_automation_id("remote:drive:sync:nightly", "drive", "other", "weekly"),
            None
        );
        assert_eq!(
            rewrite_automation_id("quick:abc", "drive", "nightly", "weekly"),
            None
        );
        let mut store = AppStore::default();
        store.job_meta.insert(
            7,
            JobMeta {
                origin: "dashboard".into(),
                profile: "nightly".into(),
                remote: "drive".into(),
                backend: "local".into(),
                quick_run_id: String::new(),
                execute_id: "exec-7".into(),
                parent_job_id: None,
                target: String::new(),
                ..Default::default()
            },
        );
        store.job_history.push(JobInfo {
            id: 7,
            operation: "sync".into(),
            remote: "drive".into(),
            profile: "nightly".into(),
            status: "running".into(),
            origin: "dashboard".into(),
            start_time: Utc::now(),
            error: None,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: "job/7".into(),
            stats: json!({}),
            transferring: json!([]),
            duration: 0.0,
            progress: 0.0,
            output: json!({}),
            completed: json!([]),
            parent_job_id: None,
        });
        store
            .automation_last_run
            .insert("remote:drive:sync:nightly".into(), Utc::now());
        store
            .automation_paused
            .push("remote:drive:sync:nightly".into());
        assert!(store.rename_runtime_profile("drive", "nightly", "weekly") >= 4);
        assert_eq!(store.job_meta[&7].profile, "weekly");
        assert_eq!(store.job_history[0].profile, "weekly");
        assert!(store
            .automation_last_run
            .contains_key("remote:drive:sync:weekly"));
        assert_eq!(store.automation_paused, vec!["remote:drive:sync:weekly"]);
        assert_eq!(store.rename_runtime_profile("drive", "weekly", "weekly"), 0);
        assert_eq!(store.rename_runtime_profile("", "a", "b"), 0);
    }
}
