//! Persistent app store: remotes metadata, quick runs, alerts, templates, jobs.

use crate::operations::OperationType;
use crate::rclone::{format_bytes, DirEntry, MountedRemote, ServeItem};
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
        let map = match kind {
            "vfs" => &self.vfs_configs,
            "filter" => &self.filter_configs,
            "backend" => &self.backend_configs,
            _ => &self.runtime_remote_configs,
        };
        let mut names: Vec<String> = map.keys().cloned().collect();
        names.sort();
        names
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub id: String,
    pub kind: AlertEventKind,
    pub severity: AlertSeverity,
    pub title: String,
    pub body: String,
    pub remote: String,
    pub origin: String,
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
}

impl AppStore {
    pub fn path() -> PathBuf {
        crate::settings::AppSettings::config_dir().join("store.json")
    }

    pub fn load() -> Self {
        if let Ok(text) = std::fs::read_to_string(Self::path()) {
            if let Ok(store) = serde_json::from_str::<AppStore>(&text) {
                return store;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(crate::settings::AppSettings::config_dir())?;
        std::fs::write(
            Self::path(),
            serde_json::to_string_pretty(self).unwrap_or_default(),
        )
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

    pub fn acknowledge_all(&mut self) {
        for event in &mut self.alert_history {
            event.acknowledged = true;
        }
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
    let body = render_template(
        action
            .config
            .get("body_template")
            .and_then(|x| x.as_str())
            .unwrap_or("{{title}}: {{body}}"),
        event,
    );
    match action.kind.as_str() {
        "os_toast" => {
            let _ = notify_rust::Notification::new()
                .summary(&event.title)
                .body(&event.body)
                .show();
        }
        "webhook" => {
            if let Some(url) = action.config.get("url").and_then(|x| x.as_str()) {
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
                });
                let req = if method.eq_ignore_ascii_case("GET") {
                    ureq::get(url)
                } else {
                    ureq::post(url)
                };
                let _ = req
                    .timeout(std::time::Duration::from_secs(8))
                    .send_json(payload);
            }
        }
        "telegram" => {
            if let (Some(token), Some(chat)) = (
                action.config.get("bot_token").and_then(|x| x.as_str()),
                action.config.get("chat_id").and_then(|x| x.as_str()),
            ) {
                if !token.is_empty() {
                    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
                    let _ = ureq::post(&url).send_json(json!({
                        "chat_id": chat,
                        "text": body
                    }));
                }
            }
        }
        "whatsapp" => {
            if let (Some(phone), Some(key)) = (
                action.config.get("phone").and_then(|x| x.as_str()),
                action.config.get("apikey").and_then(|x| x.as_str()),
            ) {
                let url = format!(
                    "https://api.callmebot.com/whatsapp.php?phone={}&text={}&apikey={}",
                    urlencoding::encode(phone),
                    urlencoding::encode(&body),
                    urlencoding::encode(key)
                );
                let _ = ureq::get(&url).call();
            }
        }
        "script" => {
            if let Some(cmd) = action.config.get("command").and_then(|x| x.as_str()) {
                if !cmd.is_empty() {
                    let _ = std::process::Command::new(cmd)
                        .env("ALERT_TITLE", &event.title)
                        .env("ALERT_BODY", &event.body)
                        .env("ALERT_SEVERITY", event.severity.as_str())
                        .spawn();
                }
            }
        }
        "email" => {
            let title = event.title.clone();
            let body = body.clone();
            let config = action.config.clone();
            std::thread::spawn(move || {
                if let Err(e) = crate::smtp::send_alert_email(&config, &title, &body) {
                    log::warn!("email alert failed: {e}");
                }
            });
        }
        "mqtt" => {
            let body = body.clone();
            let config = action.config.clone();
            std::thread::spawn(move || {
                if let Err(e) = crate::mqtt::publish_alert(&config, &body) {
                    log::warn!("mqtt alert failed: {e}");
                }
            });
        }
        other => log::warn!("unknown alert action kind {other}"),
    }
}

pub fn render_template(template: &str, event: &AlertEvent) -> String {
    template
        .replace("{{title}}", &event.title)
        .replace("{{body}}", &event.body)
        .replace("{{severity}}", event.severity.as_str())
        .replace("{{kind}}", event.kind.as_str())
        .replace("{{remote}}", &event.remote)
        .replace("{{origin}}", &event.origin)
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
            "modified" => a.mod_time.cmp(&b.mod_time),
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
}
