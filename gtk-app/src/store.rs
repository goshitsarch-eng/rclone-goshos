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
}

/// Local start-time metadata for a job id (not persisted).
#[derive(Debug, Clone, Default)]
pub struct JobMeta {
    pub origin: String,
    pub profile: String,
    pub remote: String,
    pub backend: String,
    pub quick_run_id: String,
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

#[derive(Debug, Clone, Default)]
pub struct AlertActionDraft {
    pub url: String,
    pub method: String,
    pub token: String,
    pub extra: String,
    pub body: String,
    pub retry_count: u32,
}

pub fn alert_action_config(kind: &str, draft: &AlertActionDraft) -> Value {
    let body = if draft.body.is_empty() {
        "{{title}}: {{body}}".into()
    } else {
        draft.body.clone()
    };
    let retry = draft.retry_count.min(5);
    match kind {
        "os_toast" => json!({ "body_template": body, "retry_count": retry }),
        "webhook" => json!({
            "url": draft.url,
            "method": if draft.method.is_empty() { "POST".into() } else { draft.method.clone() },
            "body_template": body,
            "retry_count": retry,
        }),
        "telegram" => json!({
            "bot_token": draft.token,
            "chat_id": draft.extra,
            "body_template": body,
            "retry_count": retry,
        }),
        "whatsapp" => json!({
            "apikey": draft.token,
            "phone": draft.extra,
            "gateway_url": draft.url,
            "body_template": body,
            "retry_count": retry,
        }),
        "script" => json!({
            "command": draft.extra,
            "body_template": body,
            "retry_count": retry,
        }),
        "email" => json!({
            "smtp_server": draft.url,
            "smtp_port": draft.method.parse::<u16>().unwrap_or(587),
            "password": draft.token,
            "from": draft.extra,
            "to": draft.extra,
            "body_template": body,
            "retry_count": retry,
        }),
        "mqtt" => json!({
            "broker_url": draft.url,
            "topic": if draft.method.is_empty() { "rclone/alerts".into() } else { draft.method.clone() },
            "password": draft.token,
            "body_template": body,
            "retry_count": retry,
        }),
        _ => json!({ "body_template": body, "retry_count": retry }),
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
    pub pending_share_paths: Vec<String>,
    #[serde(default, skip)]
    pub job_meta: HashMap<u64, JobMeta>,
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
        store
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
        self.job_history.retain(|existing| existing.id != job.id);
        self.job_history.insert(0, job);
        if self.job_history.len() > 80 {
            self.job_history.truncate(80);
        }
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
        self.alert_history
            .iter()
            .filter(|event| {
                if let Some(wanted) = severity {
                    if !event.severity.as_str().eq_ignore_ascii_case(wanted) {
                        return false;
                    }
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
            if let Some(rule) = self.alert_rules.iter_mut().find(|r| r.id == id) {
                rule.last_fired = Some(Utc::now());
                rule.fire_count += 1;
                if rule.auto_acknowledge {
                    event.acknowledged = true;
                }
                let action_ids = rule.action_ids.clone();
                for action_id in action_ids {
                    if let Some(action) = self.alert_actions.iter().find(|a| a.id == action_id) {
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
            .summary(&event.title)
            .body(&event.body)
            .show()
            .is_ok(),
        "webhook" => {
            let Some(url) = action.config.get("url").and_then(|x| x.as_str()) else {
                return false;
            };
            let method = action
                .config
                .get("method")
                .and_then(|x| x.as_str())
                .unwrap_or("POST");
            let payload = json!({
                "title": event.title,
                "body": body,
                "severity": event.severity.as_str(),
                "kind": event.kind.as_str(),
                "remote": event.remote,
                "profile": event.profile,
            });
            let req = if method.eq_ignore_ascii_case("GET") {
                ureq::get(url)
            } else {
                ureq::post(url)
            };
            req.timeout(std::time::Duration::from_secs(8))
                .send_json(payload)
                .is_ok()
        }
        "telegram" => {
            let (Some(token), Some(chat)) = (
                action.config.get("bot_token").and_then(|x| x.as_str()),
                action.config.get("chat_id").and_then(|x| x.as_str()),
            ) else {
                return false;
            };
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
            let (Some(phone), Some(key)) = (
                action.config.get("phone").and_then(|x| x.as_str()),
                action.config.get("apikey").and_then(|x| x.as_str()),
            ) else {
                return false;
            };
            let url = format!(
                "https://api.callmebot.com/whatsapp.php?phone={}&text={}&apikey={}",
                urlencoding::encode(phone),
                urlencoding::encode(&body),
                urlencoding::encode(key)
            );
            ureq::get(&url).call().is_ok()
        }
        "script" => {
            let Some(cmd) = action.config.get("command").and_then(|x| x.as_str()) else {
                return false;
            };
            if cmd.is_empty() {
                return false;
            }
            std::process::Command::new(cmd)
                .env("ALERT_TITLE", &event.title)
                .env("ALERT_BODY", &event.body)
                .env("ALERT_SEVERITY", event.severity.as_str())
                .spawn()
                .is_ok()
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
    "{{remote}}",
    "{{origin}}",
    "{{backend}}",
    "{{profile}}",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlertHistoryFilter {
    pub query: String,
    pub severity: Option<String>,
    pub remote: Option<String>,
    pub profile: Option<String>,
    pub backend: Option<String>,
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
        .replace("{{remote}}", &event.remote)
        .replace("{{origin}}", &event.origin)
        .replace("{{backend}}", &event.backend)
        .replace("{{profile}}", &event.profile)
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
            let mounted = mounts.iter().any(|m| m.fs.starts_with(&format!("{name}:")));
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
    fn render_alert_template() {
        let event = AlertEvent::new(
            AlertEventKind::System,
            AlertSeverity::Info,
            "Hello".into(),
            "World".into(),
        );
        assert_eq!(render_template("{{title}} {{body}}", &event), "Hello World");
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
        assert!(store.acknowledge_alert(&high_id));
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
            body: "{{title}}".into(),
            retry_count: 2,
        };
        let webhook = alert_action_config("webhook", &draft);
        assert_eq!(webhook["url"], "https://hooks.example/x");
        assert_eq!(webhook["method"], "PUT");
        assert_eq!(webhook["retry_count"], 2);
        assert!(webhook.get("bot_token").is_none());
        let telegram = alert_action_config("telegram", &draft);
        assert_eq!(telegram["bot_token"], "secret");
        assert_eq!(telegram["chat_id"], "123");
        let email = alert_action_config(
            "email",
            &AlertActionDraft {
                url: "smtp.example".into(),
                method: "465".into(),
                extra: "ops@example.com".into(),
                ..draft.clone()
            },
        );
        assert_eq!(email["smtp_port"], 465);
        assert_eq!(email["to"], "ops@example.com");
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
        });
        store.remote_order = vec!["drive".into(), "photos".into()];
        store
            .automation_last_run
            .insert("remote:drive:sync:default".into(), Utc::now());
        let snap = RuntimeSnapshot {
            mounts: vec![MountedRemote {
                fs: "drive:Photos".into(),
                mount_point: "/mnt/drive".into(),
            }],
            serves: vec![ServeItem {
                id: "s1".into(),
                addr: "127.0.0.1:8080".into(),
                fs: "drive:".into(),
                serve_type: "http".into(),
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
