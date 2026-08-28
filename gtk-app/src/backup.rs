//! Settings backup / restore (zip + JSON), matching the existing export modal.

use crate::settings::AppSettings;
use crate::store::AppStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::{ZipArchive, ZipWriter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: String,
    pub note: String,
    pub export_type: String,
    pub encrypted: bool,
    pub remotes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupAnalysis {
    pub valid: bool,
    pub manifest: BackupManifest,
    pub has_settings: bool,
    pub has_store: bool,
    pub has_rclone_config: bool,
    pub has_backend: bool,
    pub categories: Vec<String>,
}

pub fn export_categories() -> Vec<(&'static str, &'static str)> {
    vec![
        ("FullBackup", "Full backup"),
        ("settings", "App settings"),
        ("alerts", "Alert rules and actions"),
        ("templates", "User templates"),
        ("quick_runs", "Quick runs"),
        ("rclone", "Rclone remotes"),
        ("connections", "Saved remote connections"),
        ("nautilus", "Files preferences"),
        ("backend", "Backend"),
    ]
}

pub fn export_category_label(id: &str, i18n: &crate::i18n::I18n) -> String {
    match id {
        "FullBackup" => i18n.t_or("modals.export.fullBackup", "Full Backup"),
        "settings" => i18n.t_or(
            "modals.export.categories.settings.label",
            "Application Settings",
        ),
        "alerts" | "store" => i18n.t_or("modals.export.categories.alerts.label", "Alerts"),
        "templates" => i18n.t_or("templates.title", "Templates"),
        "quick_runs" => i18n.t_or("flow.quickRun.title", "Quick Runs"),
        "rclone" => i18n.t_or("modals.export.categories.remotes.label", "Remotes"),
        "connections" => i18n.t_or("modals.export.categories.connections.label", "Connections"),
        "nautilus" => i18n.t_or(
            "settings.general.default_view.options.nautilus",
            "Files preferences",
        ),
        "backend" => i18n.t_or("modals.export.categories.backend.label", "Backend"),
        _ => id.to_string(),
    }
}

impl BackupAnalysis {
    pub fn content_rows(&self) -> Vec<(&'static str, &'static str)> {
        let mut rows = Vec::new();
        if self.has_settings {
            rows.push(match self.manifest.export_type.as_str() {
                "connections" => ("modals.export.categories.connections.label", "Connections"),
                "nautilus" => (
                    "settings.general.default_view.options.nautilus",
                    "Files preferences",
                ),
                _ => ("backup.restore.settings.title", "Application Settings"),
            });
        }
        if self.has_store {
            rows.push(match self.manifest.export_type.as_str() {
                "alerts" | "store" => ("modals.export.categories.alerts.label", "Alerts"),
                "templates" => ("templates.title", "Templates"),
                "quick_runs" => ("flow.quickRun.title", "Quick Runs"),
                _ => ("backup.restore.profiles.title", "Profiles"),
            });
        }
        if self.has_rclone_config {
            rows.push(("backup.restore.rcloneConfig.title", "Rclone Configuration"));
        }
        if self.has_backend {
            rows.push(("modals.export.categories.backend.label", "Backend"));
        }
        rows
    }
}

pub fn includes_file(export_type: &str, file: &str) -> bool {
    if file == "manifest.json" {
        return true;
    }
    match export_type {
        "" | "FullBackup" | "All" | "full" => true,
        "settings" | "Settings" | "connections" | "nautilus" => file == "settings.json",
        "backend" => file == "settings.json" || file == "backend.json",
        "store" | "alerts" | "templates" | "quick_runs" => file == "store.json",
        "rclone" | "remotes" => file == "rclone.json",
        other if other.starts_with("remote:") => file == "rclone.json",
        _ => true,
    }
}

pub fn filter_store_category(store: &AppStore, export_type: &str) -> AppStore {
    match export_type {
        "alerts" | "store" => {
            let mut scoped = AppStore::default();
            scoped.alert_rules = store.alert_rules.clone();
            scoped.alert_actions = store.alert_actions.clone();
            scoped.alert_history = store.alert_history.clone();
            scoped
        }
        "templates" => {
            let mut scoped = AppStore::default();
            scoped.templates = store.templates.clone();
            scoped
        }
        "quick_runs" => {
            let mut scoped = AppStore::default();
            scoped.quick_runs = store.quick_runs.clone();
            scoped
        }
        _ => store.clone(),
    }
}

pub fn filter_settings_category(settings: &AppSettings, export_type: &str) -> AppSettings {
    match export_type {
        "connections" => {
            let mut scoped = AppSettings::default();
            scoped.core.extra_backends = settings.core.extra_backends.clone();
            scoped
        }
        "nautilus" => {
            let mut scoped = AppSettings::default();
            scoped.nautilus = settings.nautilus.clone();
            scoped
        }
        "backend" => {
            let mut scoped = AppSettings::default();
            scoped.core.extra_backends = settings.core.extra_backends.clone();
            scoped.core.rclone_binary = settings.core.rclone_binary.clone();
            scoped.core.rclone_additional_flags = settings.core.rclone_additional_flags.clone();
            scoped.core.rclone_env_vars = settings.core.rclone_env_vars.clone();
            scoped.core.bandwidth_limit = settings.core.bandwidth_limit.clone();
            scoped.core.metered_bandwidth_limit = settings.core.metered_bandwidth_limit.clone();
            scoped.core.connection_check_urls = settings.core.connection_check_urls.clone();
            scoped
        }
        _ => settings.clone(),
    }
}

pub fn merge_store(current: &AppStore, incoming: &AppStore, export_type: &str) -> AppStore {
    match export_type {
        "alerts" => merge_store_alerts(current, incoming),
        "store" if incoming.remotes.is_empty() => merge_store_alerts(current, incoming),
        "templates" => merge_store_templates(current, incoming),
        "quick_runs" => merge_store_quick_runs(current, incoming),
        "rcman" => merge_store_rcman(current, incoming),
        "settings" | "connections" | "nautilus" | "rclone" | "remotes" => current.clone(),
        "remote" | "profile" => merge_store_remotes(current, incoming),
        _ => incoming.clone(),
    }
}

fn merge_store_remotes(current: &AppStore, incoming: &AppStore) -> AppStore {
    let mut out = current.clone();
    for (name, meta) in &incoming.remotes {
        out.remotes.insert(name.clone(), meta.clone());
    }
    for run in &incoming.quick_runs {
        let exists = out.quick_runs.iter().any(|existing| {
            existing.id == run.id
                || (existing.name == run.name && existing.remote_name == run.remote_name)
        });
        if !exists {
            out.quick_runs.push(run.clone());
        }
    }
    out
}

fn merge_store_alerts(current: &AppStore, incoming: &AppStore) -> AppStore {
    let mut out = current.clone();
    out.alert_rules = incoming.alert_rules.clone();
    out.alert_actions = incoming.alert_actions.clone();
    out.alert_history = incoming.alert_history.clone();
    out
}

fn upsert_by_id<T, F>(dest: &mut Vec<T>, incoming: &[T], id: F)
where
    T: Clone,
    F: Fn(&T) -> &str,
{
    for item in incoming {
        if let Some(existing) = dest.iter_mut().find(|existing| id(existing) == id(item)) {
            *existing = item.clone();
        } else {
            dest.push(item.clone());
        }
    }
}

fn merge_store_templates(current: &AppStore, incoming: &AppStore) -> AppStore {
    let mut out = current.clone();
    upsert_by_id(&mut out.templates, &incoming.templates, |t| t.id.as_str());
    out
}

fn merge_store_quick_runs(current: &AppStore, incoming: &AppStore) -> AppStore {
    let mut out = current.clone();
    upsert_by_id(&mut out.quick_runs, &incoming.quick_runs, |q| q.id.as_str());
    out
}

fn merge_store_rcman(current: &AppStore, incoming: &AppStore) -> AppStore {
    let mut out = merge_store_remotes(current, incoming);
    upsert_by_id(&mut out.templates, &incoming.templates, |t| t.id.as_str());
    upsert_by_id(&mut out.quick_runs, &incoming.quick_runs, |q| q.id.as_str());
    for rule in &incoming.alert_rules {
        if !out.alert_rules.iter().any(|r| r.id == rule.id) {
            out.alert_rules.push(rule.clone());
        }
    }
    for action in &incoming.alert_actions {
        if !out.alert_actions.iter().any(|a| a.id == action.id) {
            out.alert_actions.push(action.clone());
        }
    }
    out
}

pub fn merge_settings(
    current: &AppSettings,
    incoming: &AppSettings,
    export_type: &str,
) -> AppSettings {
    match export_type {
        "connections" => {
            let mut out = current.clone();
            out.core.extra_backends = incoming.core.extra_backends.clone();
            out
        }
        "nautilus" => {
            let mut out = current.clone();
            out.nautilus = incoming.nautilus.clone();
            out
        }
        "backend" => {
            let mut out = current.clone();
            out.core.extra_backends = incoming.core.extra_backends.clone();
            out.core.rclone_binary = incoming.core.rclone_binary.clone();
            out.core.rclone_additional_flags = incoming.core.rclone_additional_flags.clone();
            out.core.rclone_env_vars = incoming.core.rclone_env_vars.clone();
            out.core.bandwidth_limit = incoming.core.bandwidth_limit.clone();
            out.core.metered_bandwidth_limit = incoming.core.metered_bandwidth_limit.clone();
            out.core.connection_check_urls = incoming.core.connection_check_urls.clone();
            out
        }
        "alerts" | "store" | "rclone" | "remotes" | "remote" | "profile" | "templates"
        | "quick_runs" => current.clone(),
        "rcman" => {
            let mut out = current.clone();
            if !incoming.core.extra_backends.is_empty() {
                out.core.extra_backends = incoming.core.extra_backends.clone();
            }
            out
        }
        _ => incoming.clone(),
    }
}

/// Apply a restore onto the live settings/store. A scoped (one-remote) restore
/// merges that remote and keeps the current settings.
pub fn apply_restore(
    current_settings: &AppSettings,
    current_store: &AppStore,
    settings: Option<AppSettings>,
    store: Option<AppStore>,
    export_type: &str,
    scoped: bool,
) -> (AppSettings, AppStore) {
    let kind = if scoped { "remote" } else { export_type };
    let settings = match settings {
        Some(incoming) if !scoped => merge_settings(current_settings, &incoming, export_type),
        _ => current_settings.clone(),
    };
    let store = match store {
        Some(incoming) => merge_store(current_store, &incoming, kind),
        None => current_store.clone(),
    };
    (settings, store)
}

/// `config/create` parameters should not repeat the remote type key.
pub fn rclone_create_params(cfg: &Value) -> Value {
    let mut params = cfg.clone();
    if let Some(obj) = params.as_object_mut() {
        obj.remove("type");
    }
    params
}

pub fn filter_store_remotes(store: &AppStore, names: &[String]) -> AppStore {
    let keep: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut scoped = store.clone();
    scoped
        .remotes
        .retain(|name, _| keep.contains(name.as_str()));
    scoped
        .quick_runs
        .retain(|run| keep.contains(run.remote_name.as_str()));
    scoped
        .alert_history
        .retain(|event| event.remote.is_empty() || keep.contains(event.remote.as_str()));
    scoped
        .logs
        .retain(|remote, _| remote.is_empty() || keep.contains(remote.as_str()));
    scoped.job_history.retain(|job| {
        job.remote.is_empty()
            || keep.contains(job.remote.as_str())
            || job
                .src
                .split_once(':')
                .is_some_and(|(remote, _)| keep.contains(remote))
    });
    scoped
        .job_meta
        .retain(|_, meta| meta.remote.is_empty() || keep.contains(meta.remote.as_str()));
    scoped
}

pub fn filter_rclone_names(dump: &Value, names: &[String]) -> Value {
    let Some(obj) = dump.as_object() else {
        return if names.is_empty() {
            serde_json::json!({})
        } else {
            dump.clone()
        };
    };
    let mut out = serde_json::Map::new();
    for name in names {
        if let Some(cfg) = obj.get(name) {
            out.insert(name.clone(), cfg.clone());
        }
    }
    Value::Object(out)
}

pub fn filter_rclone_dump(dump: &Value, export_type: &str) -> Value {
    if let Some(name) = export_type.strip_prefix("remote:") {
        if let Some(obj) = dump.as_object() {
            if let Some(cfg) = obj.get(name) {
                return serde_json::json!({ name: cfg });
            }
        }
        return serde_json::json!({});
    }
    dump.clone()
}

pub fn create_backup(
    dest: &Path,
    settings: &AppSettings,
    store: &AppStore,
    rclone_dump: &Value,
    export_type: &str,
    note: &str,
    password: Option<&str>,
) -> Result<PathBuf, String> {
    create_backup_with(
        dest,
        settings,
        store,
        rclone_dump,
        export_type,
        note,
        password,
        None,
    )
}

fn dump_contains_secrets(dump: &Value) -> bool {
    const KEYS: &[&str] = &[
        "token",
        "secret",
        "password",
        "password2",
        "pass",
        "client_secret",
        "key",
    ];
    dump.as_object().is_some_and(|remotes| {
        remotes.values().any(|cfg| {
            cfg.as_object().is_some_and(|map| {
                KEYS.iter().any(|key| {
                    map.get(*key).is_some_and(|value| match value {
                        Value::Null => false,
                        Value::String(s) => !s.is_empty(),
                        other => !other.is_null(),
                    })
                })
            })
        })
    })
}

#[allow(clippy::too_many_arguments)]
pub fn create_backup_with(
    dest: &Path,
    settings: &AppSettings,
    store: &AppStore,
    rclone_dump: &Value,
    export_type: &str,
    note: &str,
    password: Option<&str>,
    backend: Option<&Value>,
) -> Result<PathBuf, String> {
    let settings = filter_settings_category(settings, export_type);
    let store = filter_store_category(store, export_type);
    let rclone_dump = filter_rclone_dump(rclone_dump, export_type);
    let encrypted = password.is_some_and(|p| p.trim().len() >= 4);
    if includes_file(export_type, "rclone.json")
        && dump_contains_secrets(&rclone_dump)
        && !encrypted
    {
        return Err("Including secrets requires a zip password of 4+ characters".into());
    }

    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let file = File::create(dest).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let mut options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    if encrypted {
        if let Some(pw) = password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, pw);
        }
    }
    let remotes = if includes_file(export_type, "rclone.json") {
        rclone_dump
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let manifest = BackupManifest {
        version: env!("CARGO_PKG_VERSION").into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        note: note.to_string(),
        export_type: export_type.to_string(),
        encrypted,
        remotes,
    };
    zip.start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;

    if includes_file(export_type, "settings.json") {
        zip.start_file("settings.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(serde_json::to_string_pretty(&settings).unwrap().as_bytes())
            .map_err(|e| e.to_string())?;
    }

    if includes_file(export_type, "store.json") {
        zip.start_file("store.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(serde_json::to_string_pretty(&store).unwrap().as_bytes())
            .map_err(|e| e.to_string())?;
    }

    if includes_file(export_type, "rclone.json") && export_type != "backend" {
        zip.start_file("rclone.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(
            serde_json::to_string_pretty(&rclone_dump)
                .unwrap()
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    }

    let backend_payload = if export_type == "backend" {
        Some(&rclone_dump)
    } else {
        backend
    };
    if includes_file(export_type, "backend.json") {
        if let Some(payload) = backend_payload {
            zip.start_file("backend.json", options)
                .map_err(|e| e.to_string())?;
            zip.write_all(serde_json::to_string_pretty(payload).unwrap().as_bytes())
                .map_err(|e| e.to_string())?;
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(dest.to_path_buf())
}

pub fn analyze_backup(path: &Path) -> Result<BackupAnalysis, String> {
    analyze_backup_with_password(path, None)
}

pub fn analyze_backup_with_password(
    path: &Path,
    password: Option<&str>,
) -> Result<BackupAnalysis, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = zip_entry(&mut archive, i, password) {
            names.push(entry.name().to_string());
        }
    }
    let manifest = if names.iter().any(|n| n == "manifest.json") {
        let text = read_zip_entry(&mut archive, "manifest.json", password)?;
        serde_json::from_str(&text).unwrap_or(BackupManifest {
            version: "unknown".into(),
            created_at: String::new(),
            note: String::new(),
            export_type: "FullBackup".into(),
            encrypted: false,
            remotes: vec![],
        })
    } else {
        BackupManifest {
            version: "unknown".into(),
            created_at: String::new(),
            note: String::new(),
            export_type: "FullBackup".into(),
            encrypted: false,
            remotes: vec![],
        }
    };
    let mut manifest = manifest;
    if manifest.remotes.is_empty() {
        if let Some(store) = read_zip_json::<AppStore>(&mut archive, "store.json", password) {
            manifest.remotes = store_remote_names(&store);
        }
    }
    let gtk_layout = is_gtk_backup_layout(&names);
    if !gtk_layout && looks_like_rcman_archive(path, &names) {
        return analyze_rcman_backup(path, password, names);
    }
    Ok(BackupAnalysis {
        valid: gtk_layout,
        has_settings: names.iter().any(|n| n == "settings.json"),
        has_store: names.iter().any(|n| n == "store.json"),
        has_rclone_config: names.iter().any(|n| n == "rclone.json"),
        has_backend: names.iter().any(|n| n == "backend.json"),
        categories: names,
        manifest,
    })
}

pub fn restore_backup(
    path: &Path,
) -> Result<(Option<AppSettings>, Option<AppStore>, Option<Value>), String> {
    restore_backup_with_password(path, None)
}

pub fn restore_backup_with_password(
    path: &Path,
    password: Option<&str>,
) -> Result<(Option<AppSettings>, Option<AppStore>, Option<Value>), String> {
    restore_backup_scoped(path, password, None, None)
}

pub fn store_remote_names(store: &AppStore) -> Vec<String> {
    let mut names: Vec<String> = store.remotes.keys().cloned().collect();
    names.sort();
    names
}

pub fn scoped_store(
    mut store: AppStore,
    profile: Option<&str>,
    restore_as: Option<&str>,
) -> AppStore {
    let Some(from) = profile.map(str::trim).filter(|s| !s.is_empty()) else {
        return store;
    };
    let dest = restore_as
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(from);
    let mut scoped = AppStore::default();
    if let Some(mut meta) = store.remotes.remove(from) {
        if dest != from {
            for profiles in meta.profiles.values_mut() {
                for profile in profiles.values_mut() {
                    crate::store::rewrite_remote_refs(&mut profile.rclone, from, dest);
                }
            }
        }
        scoped.remotes.insert(dest.to_string(), meta);
    }
    scoped.quick_runs = store
        .quick_runs
        .into_iter()
        .filter_map(|mut q| {
            if q.remote_name != from {
                return None;
            }
            q.remote_name = dest.to_string();
            Some(q)
        })
        .collect();
    scoped.alert_history = store
        .alert_history
        .into_iter()
        .filter(|e| e.remote == from)
        .map(|mut e| {
            e.remote = dest.to_string();
            e
        })
        .collect();
    scoped
}

pub fn scoped_rclone(dump: Value, profile: Option<&str>, restore_as: Option<&str>) -> Value {
    let Some(from) = profile.map(str::trim).filter(|s| !s.is_empty()) else {
        return dump;
    };
    let dest = restore_as
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(from);
    dump.as_object()
        .and_then(|obj| obj.get(from).cloned())
        .map(|cfg| serde_json::json!({ dest: cfg }))
        .unwrap_or_else(|| serde_json::json!({}))
}

#[derive(Debug, Clone, Default)]
pub struct RestoredBackup {
    pub settings: Option<AppSettings>,
    pub store: Option<AppStore>,
    pub rclone: Option<Value>,
    pub backend: Option<Value>,
}

pub fn restore_backup_scoped(
    path: &Path,
    password: Option<&str>,
    profile: Option<&str>,
    restore_as: Option<&str>,
) -> Result<(Option<AppSettings>, Option<AppStore>, Option<Value>), String> {
    let restored = restore_backup_contents(path, password, profile, restore_as)?;
    Ok((restored.settings, restored.store, restored.rclone))
}

pub fn restore_backup_contents(
    path: &Path,
    password: Option<&str>,
    profile: Option<&str>,
    restore_as: Option<&str>,
) -> Result<RestoredBackup, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = zip_entry(&mut archive, i, password) {
            names.push(entry.name().to_string());
        }
    }
    if !is_gtk_backup_layout(&names) && looks_like_rcman_archive(path, &names) {
        drop(archive);
        return restore_rcman_contents(path, password, profile, restore_as);
    }
    let settings = read_zip_json::<AppSettings>(&mut archive, "settings.json", password);
    let store = read_zip_json::<AppStore>(&mut archive, "store.json", password)
        .map(|s| scoped_store(s, profile, restore_as));
    let rclone = read_zip_json::<Value>(&mut archive, "rclone.json", password)
        .map(|d| scoped_rclone(d, profile, restore_as));
    let backend = read_zip_json::<Value>(&mut archive, "backend.json", password);
    Ok(RestoredBackup {
        settings,
        store,
        rclone,
        backend,
    })
}

fn is_gtk_backup_layout(names: &[String]) -> bool {
    names.iter().any(|n| {
        let n = n.replace('\\', "/");
        n == "settings.json" || n == "store.json" || n == "backend.json" || n == "rclone.json"
    })
}

fn looks_like_rcman_archive(path: &Path, names: &[String]) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .eq_ignore_ascii_case("rcman");
    ext || names.iter().any(|n| {
        let n = n.replace('\\', "/");
        n.contains("/remotes/") && n.ends_with(".json")
            || n.starts_with("remotes/") && n.ends_with(".json")
            || n.ends_with("quick_runs.json")
            || n.ends_with("connections.json")
            || n.ends_with("templates.json")
            || n.contains("alerts/rules")
            || n.contains("sub_settings/")
    })
}

fn map_rcman_zip_entry(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let name = name.trim_start_matches("./");
    let file = name.rsplit('/').next().unwrap_or(name);
    if (name.contains("/remotes/") || name.starts_with("remotes/")) && file.ends_with(".json") {
        return Some(PathBuf::from("remotes").join(file));
    }
    match file {
        "quick_runs.json" | "connections.json" | "templates.json" | "rclone.conf"
        | "backend.json" => Some(PathBuf::from(file)),
        "rules.json" if name.contains("alerts") => Some(PathBuf::from("alerts").join("rules.json")),
        "actions.json" if name.contains("alerts") => {
            Some(PathBuf::from("alerts").join("actions.json"))
        }
        _ => None,
    }
}

fn extract_rcman_layout(
    path: &Path,
    password: Option<&str>,
    dest: &Path,
) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut mapped = Vec::new();
    for i in 0..archive.len() {
        let mut entry = match zip_entry(&mut archive, i, password) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let Some(rel) = map_rcman_zip_entry(entry.name()) else {
            continue;
        };
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out_file = File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
        mapped.push(rel.to_string_lossy().replace('\\', "/"));
    }
    Ok(mapped)
}

struct RcmanLoaded {
    settings: AppSettings,
    store: AppStore,
    rclone: Option<Value>,
    backend: Option<Value>,
    mapped: Vec<String>,
}

fn load_rcman_from_zip(path: &Path, password: Option<&str>) -> Result<RcmanLoaded, String> {
    let dir = std::env::temp_dir().join(format!(
        "rclone-manager-rcman-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| {
        let mapped = extract_rcman_layout(path, password, &dir)?;
        let mut store = AppStore::default();
        let mut settings = AppSettings::default();
        crate::migrate::import_rcman(&dir, &mut store, &mut settings);
        let rclone = std::fs::read_to_string(dir.join("rclone.conf"))
            .ok()
            .and_then(|text| crate::config_import::parse_rclone_conf(&text).ok());
        let backend = std::fs::read_to_string(dir.join("backend.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        Ok(RcmanLoaded {
            settings,
            store,
            rclone,
            backend,
            mapped,
        })
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn analyze_rcman_backup(
    path: &Path,
    password: Option<&str>,
    names: Vec<String>,
) -> Result<BackupAnalysis, String> {
    let loaded = load_rcman_from_zip(path, password)?;
    let remotes = {
        let mut names = store_remote_names(&loaded.store);
        if names.is_empty() {
            if let Some(dump) = &loaded.rclone {
                if let Some(obj) = dump.as_object() {
                    names = obj.keys().cloned().collect();
                    names.sort();
                }
            }
        }
        names
    };
    Ok(BackupAnalysis {
        valid: !loaded.mapped.is_empty(),
        has_settings: !loaded.settings.core.extra_backends.is_empty(),
        has_store: !loaded.store.remotes.is_empty()
            || !loaded.store.quick_runs.is_empty()
            || !loaded.store.alert_rules.is_empty()
            || !loaded.store.templates.is_empty(),
        has_rclone_config: loaded.rclone.is_some(),
        has_backend: loaded.backend.is_some(),
        categories: names,
        manifest: BackupManifest {
            version: "rcman".into(),
            created_at: String::new(),
            note: String::new(),
            export_type: "rcman".into(),
            encrypted: password.is_some(),
            remotes,
        },
    })
}

fn restore_rcman_contents(
    path: &Path,
    password: Option<&str>,
    profile: Option<&str>,
    restore_as: Option<&str>,
) -> Result<RestoredBackup, String> {
    let loaded = load_rcman_from_zip(path, password)?;
    let has_settings = !loaded.settings.core.extra_backends.is_empty();
    Ok(RestoredBackup {
        settings: has_settings.then_some(loaded.settings),
        store: Some(scoped_store(loaded.store, profile, restore_as)),
        rclone: loaded.rclone.map(|d| scoped_rclone(d, profile, restore_as)),
        backend: loaded.backend,
    })
}

fn zip_entry<'a>(
    archive: &'a mut ZipArchive<File>,
    index: usize,
    password: Option<&str>,
) -> Result<zip::read::ZipFile<'a>, zip::result::ZipError> {
    if let Some(pw) = password {
        archive.by_index_decrypt(index, pw.as_bytes())
    } else {
        archive.by_index(index)
    }
}

fn read_zip_entry(
    archive: &mut ZipArchive<File>,
    name: &str,
    password: Option<&str>,
) -> Result<String, String> {
    let mut entry = if let Some(pw) = password {
        archive
            .by_name_decrypt(name, pw.as_bytes())
            .map_err(|e| e.to_string())?
    } else {
        archive.by_name(name).map_err(|e| e.to_string())?
    };
    let mut text = String::new();
    entry.read_to_string(&mut text).map_err(|e| e.to_string())?;
    Ok(text)
}

fn read_zip_json<T: for<'de> Deserialize<'de>>(
    archive: &mut ZipArchive<File>,
    name: &str,
    password: Option<&str>,
) -> Option<T> {
    let text = read_zip_entry(archive, name, password).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("backup.zip");
        let settings = AppSettings::default();
        let store = AppStore::default();
        let dump = serde_json::json!({ "demo": { "type": "local" } });
        create_backup(&dest, &settings, &store, &dump, "FullBackup", "test", None).unwrap();
        let analysis = analyze_backup(&dest).unwrap();
        assert!(analysis.valid);
        assert!(analysis.has_settings);
        assert!(analysis
            .content_rows()
            .iter()
            .any(|(key, _)| *key == "backup.restore.settings.title"));
        assert_eq!(analysis.manifest.note, "test");
        let (s, st, r) = restore_backup(&dest).unwrap();
        assert!(s.is_some());
        assert!(st.is_some());
        assert_eq!(r.unwrap()["demo"]["type"], "local");
    }

    #[test]
    fn scopes_restore_to_one_remote_and_rename() {
        let mut store = AppStore::default();
        store
            .remotes
            .insert("drive".into(), crate::store::RemoteMeta::default());
        store
            .remotes
            .insert("dropbox".into(), crate::store::RemoteMeta::default());
        store.quick_runs.push(crate::store::QuickRun::new(
            "Nightly".into(),
            crate::operations::OperationType::Sync,
            "drive".into(),
        ));
        let scoped = scoped_store(store, Some("drive"), Some("photos"));
        assert!(scoped.remotes.contains_key("photos"));
        assert!(!scoped.remotes.contains_key("drive"));
        assert!(!scoped.remotes.contains_key("dropbox"));
        assert_eq!(scoped.quick_runs[0].remote_name, "photos");
        let dump = serde_json::json!({
            "drive": { "type": "drive" },
            "dropbox": { "type": "dropbox" }
        });
        let rclone = scoped_rclone(dump, Some("drive"), Some("photos"));
        assert_eq!(rclone["photos"]["type"], "drive");
        assert!(rclone.get("dropbox").is_none());
    }

    #[test]
    fn scoped_full_backup_merges_into_current_store() {
        let mut current = AppStore::default();
        current
            .remotes
            .insert("keep".into(), crate::store::RemoteMeta::default());
        let mut incoming = AppStore::default();
        incoming
            .remotes
            .insert("photos".into(), crate::store::RemoteMeta::default());
        incoming.quick_runs.push(crate::store::QuickRun::new(
            "Nightly".into(),
            crate::operations::OperationType::Sync,
            "photos".into(),
        ));
        let (settings, store) = apply_restore(
            &AppSettings::default(),
            &current,
            Some(AppSettings::default()),
            Some(incoming),
            "FullBackup",
            true,
        );
        assert!(store.remotes.contains_key("keep"));
        assert!(store.remotes.contains_key("photos"));
        assert_eq!(store.quick_runs[0].remote_name, "photos");
        assert_eq!(
            settings.general.language,
            AppSettings::default().general.language
        );
        let params = rclone_create_params(&serde_json::json!({
            "type": "alias",
            "remote": "/tmp/rclone-test-remote"
        }));
        assert!(params.get("type").is_none());
        assert_eq!(params["remote"], "/tmp/rclone-test-remote");
    }

    #[test]
    fn restores_gui_backup_zip_as_renamed_remote() {
        let path = PathBuf::from("/tmp/rclone-manager-gui-backup.zip");
        if !path.exists() {
            return;
        }
        let (settings, store, rclone) =
            restore_backup_scoped(&path, None, Some("testdrive"), Some("testdrive2")).unwrap();
        let store = store.expect("store");
        assert!(store.remotes.contains_key("testdrive2"));
        assert!(!store.remotes.contains_key("testdrive"));
        let dump = rclone.expect("rclone");
        assert_eq!(dump["testdrive2"]["type"], "alias");
        assert_eq!(dump["testdrive2"]["remote"], "/tmp/rclone-test-remote");
        let mut current = AppStore::default();
        current
            .remotes
            .insert("keep".into(), crate::store::RemoteMeta::default());
        let (_, merged) = apply_restore(
            &AppSettings::default(),
            &current,
            settings,
            Some(store),
            "FullBackup",
            true,
        );
        assert!(merged.remotes.contains_key("keep"));
        assert!(merged.remotes.contains_key("testdrive2"));
        let params = rclone_create_params(&dump["testdrive2"]);
        assert!(params.get("type").is_none());
        assert_eq!(params["remote"], "/tmp/rclone-test-remote");
    }

    #[test]
    fn scoped_store_rewrites_renamed_remote_paths() {
        let mut store = AppStore::default();
        let mut meta = crate::store::RemoteMeta::default();
        let mut profile = crate::store::ProfileConfig::default();
        profile.rclone = serde_json::json!({ "srcFs": "drive:Photos", "dstFs": "/tmp/out" });
        meta.profiles
            .insert("copy".into(), [("default".into(), profile)].into());
        store.remotes.insert("drive".into(), meta);
        let scoped = scoped_store(store, Some("drive"), Some("photos"));
        assert_eq!(
            scoped.remotes["photos"].profiles["copy"]["default"].rclone["srcFs"],
            "photos:Photos"
        );
        assert_eq!(
            scoped.remotes["photos"].profiles["copy"]["default"].rclone["dstFs"],
            "/tmp/out"
        );
    }

    #[test]
    fn category_filter_omits_unselected_files() {
        assert!(includes_file("settings", "settings.json"));
        assert!(!includes_file("settings", "store.json"));
        assert!(includes_file("store", "store.json"));
        assert!(includes_file("alerts", "store.json"));
        assert!(!includes_file("alerts", "settings.json"));
        assert!(includes_file("connections", "settings.json"));
        assert!(!includes_file("connections", "store.json"));
        assert!(!includes_file("rclone", "settings.json"));
        assert!(includes_file("remote:demo", "rclone.json"));
        let dump = serde_json::json!({ "demo": { "type": "local" }, "other": { "type": "drive" } });
        let filtered = filter_rclone_dump(&dump, "remote:demo");
        assert!(filtered.get("demo").is_some());
        assert!(filtered.get("other").is_none());
    }

    #[test]
    fn filters_full_backup_to_selected_remotes() {
        let mut store = AppStore::default();
        store
            .remotes
            .insert("drive".into(), crate::store::RemoteMeta::default());
        store
            .remotes
            .insert("dropbox".into(), crate::store::RemoteMeta::default());
        store.quick_runs.push(crate::store::QuickRun::new(
            "Nightly".into(),
            crate::operations::OperationType::Sync,
            "drive".into(),
        ));
        store.quick_runs.push(crate::store::QuickRun::new(
            "Photos".into(),
            crate::operations::OperationType::Copy,
            "dropbox".into(),
        ));
        let scoped = filter_store_remotes(&store, &["drive".into()]);
        assert!(scoped.remotes.contains_key("drive"));
        assert!(!scoped.remotes.contains_key("dropbox"));
        assert_eq!(scoped.quick_runs.len(), 1);
        assert_eq!(scoped.quick_runs[0].remote_name, "drive");
        let dump = serde_json::json!({
            "drive": { "type": "drive" },
            "dropbox": { "type": "dropbox" }
        });
        let filtered = filter_rclone_names(&dump, &["drive".into()]);
        assert!(filtered.get("drive").is_some());
        assert!(filtered.get("dropbox").is_none());
        store
            .logs
            .insert("dropbox".into(), vec!["dropbox copy failed".into()]);
        store.job_history.push(crate::store::JobInfo {
            id: 7,
            operation: "copy".into(),
            remote: "dropbox".into(),
            profile: "default".into(),
            status: "completed".into(),
            origin: "dashboard".into(),
            start_time: chrono::Utc::now(),
            error: None,
            dry_run: false,
            src: "dropbox:a".into(),
            dst: "/tmp/a".into(),
            group: String::new(),
            stats: serde_json::json!({}),
            transferring: serde_json::json!([]),
            duration: 0.0,
            progress: 0.0,
            output: serde_json::json!({}),
            completed: serde_json::json!([]),
            parent_job_id: None,
        });
        store.job_meta.insert(
            7,
            crate::store::JobMeta {
                remote: "dropbox".into(),
                ..Default::default()
            },
        );
        let empty = filter_store_remotes(&store, &[]);
        assert!(empty.remotes.is_empty());
        assert!(empty.quick_runs.is_empty());
        assert!(empty.logs.is_empty());
        assert!(empty.job_history.is_empty());
        assert!(empty.job_meta.is_empty());
        let scoped = filter_store_remotes(&store, &["drive".into()]);
        assert!(!scoped.logs.contains_key("dropbox"));
        assert!(scoped.job_history.iter().all(|job| job.remote != "dropbox"));
        assert!(!scoped.job_meta.contains_key(&7));
        assert!(filter_rclone_names(&dump, &[])
            .as_object()
            .is_some_and(|obj| obj.is_empty()));
    }

    #[test]
    fn scopes_alerts_and_connections_categories() {
        let mut store = AppStore::default();
        store
            .remotes
            .insert("drive".into(), crate::store::RemoteMeta::default());
        store.quick_runs.push(crate::store::QuickRun::new(
            "Nightly".into(),
            crate::operations::OperationType::Sync,
            "drive".into(),
        ));
        let mut rule = crate::store::AlertRule::new("Nightly".into());
        rule.id = "rule-1".into();
        store.alert_rules.push(rule);
        store.alert_actions.push(crate::store::AlertAction::new(
            "Toast".into(),
            "os_toast".into(),
        ));
        let alerts = filter_store_category(&store, "alerts");
        assert!(alerts.remotes.is_empty());
        assert!(alerts.quick_runs.is_empty());
        assert_eq!(alerts.alert_rules.len(), 1);
        assert_eq!(alerts.alert_actions.len(), 1);

        let mut settings = AppSettings::default();
        settings.general.language = "tr-TR".into();
        settings
            .core
            .extra_backends
            .push(crate::settings::BackendEntry {
                name: "nas".into(),
                host: "192.168.1.8".into(),
                port: 5572,
                ..crate::settings::BackendEntry::default()
            });
        settings.nautilus.bookmarks = vec![serde_json::json!({"path": "testdrive:photos"})];
        let connections = filter_settings_category(&settings, "connections");
        assert_eq!(connections.core.extra_backends.len(), 1);
        assert_eq!(connections.core.extra_backends[0].name, "nas");
        assert_ne!(connections.general.language, "tr-TR");
        assert!(connections.nautilus.bookmarks.is_empty());
        let nautilus = filter_settings_category(&settings, "nautilus");
        assert_eq!(nautilus.nautilus.bookmarks.len(), 1);
        assert!(nautilus.core.extra_backends.is_empty());

        let merged_store = merge_store(&store, &alerts, "alerts");
        assert!(merged_store.remotes.contains_key("drive"));
        assert_eq!(merged_store.alert_rules[0].id, "rule-1");
        let merged_settings = merge_settings(&settings, &connections, "connections");
        assert_eq!(merged_settings.general.language, "tr-TR");
        assert_eq!(merged_settings.core.extra_backends[0].name, "nas");
        assert_eq!(merged_settings.nautilus.bookmarks.len(), 1);
    }

    #[test]
    fn alerts_backup_omits_settings_and_remotes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("alerts.zip");
        let mut store = AppStore::default();
        store
            .remotes
            .insert("drive".into(), crate::store::RemoteMeta::default());
        store.alert_actions.push(crate::store::AlertAction::new(
            "Toast".into(),
            "os_toast".into(),
        ));
        create_backup(
            &dest,
            &AppSettings::default(),
            &store,
            &serde_json::json!({ "drive": { "type": "drive" } }),
            "alerts",
            "alerts only",
            None,
        )
        .unwrap();
        let analysis = analyze_backup(&dest).unwrap();
        assert!(!analysis.has_settings);
        assert!(analysis.has_store);
        assert!(!analysis.has_rclone_config);
        assert_eq!(analysis.manifest.export_type, "alerts");
        assert!(analysis.manifest.remotes.is_empty());
        assert!(analysis
            .content_rows()
            .iter()
            .any(|(key, _)| *key == "modals.export.categories.alerts.label"));
        let (_, restored, _) = restore_backup(&dest).unwrap();
        let restored = restored.expect("store");
        assert!(restored.remotes.is_empty());
        assert_eq!(restored.alert_actions.len(), 1);
    }

    #[test]
    fn backend_backup_writes_settings_and_options() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("backend.zip");
        let mut settings = AppSettings::default();
        settings.core.rclone_additional_flags = vec!["--transfers".into(), "8".into()];
        settings.general.language = "tr-TR".into();
        create_backup(
            &dest,
            &settings,
            &AppStore::default(),
            &serde_json::json!({ "main": { "transfers": 8 } }),
            "backend",
            "backend only",
            None,
        )
        .unwrap();
        let analysis = analyze_backup(&dest).unwrap();
        assert!(analysis.has_settings);
        assert!(!analysis.has_store);
        assert!(!analysis.has_rclone_config);
        assert!(analysis.categories.iter().any(|n| n == "backend.json"));
        assert_eq!(analysis.manifest.export_type, "backend");
        let scoped = filter_settings_category(&settings, "backend");
        assert_eq!(
            scoped.core.rclone_additional_flags,
            vec!["--transfers", "8"]
        );
        assert_ne!(scoped.general.language, "tr-TR");
        let mut incoming = AppSettings::default();
        incoming.core.rclone_additional_flags = vec!["--fast-list".into()];
        let merged = merge_settings(&settings, &incoming, "backend");
        assert_eq!(merged.general.language, "tr-TR");
        assert_eq!(merged.core.rclone_additional_flags, vec!["--fast-list"]);
        assert!(includes_file("backend", "backend.json"));
        assert!(!includes_file("backend", "rclone.json"));
        assert!(analysis.has_backend);
        assert!(analysis
            .content_rows()
            .iter()
            .any(|(key, _)| *key == "modals.export.categories.backend.label"));
        let restored = restore_backup_contents(&dest, None, None, None).unwrap();
        assert!(restored.rclone.is_none());
        assert_eq!(restored.backend.unwrap()["main"]["transfers"], 8);
    }

    #[test]
    fn settings_only_backup_skips_store() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("settings.zip");
        create_backup(
            &dest,
            &AppSettings::default(),
            &AppStore::default(),
            &serde_json::json!({}),
            "settings",
            "",
            None,
        )
        .unwrap();
        let analysis = analyze_backup(&dest).unwrap();
        assert!(analysis.has_settings);
        assert!(!analysis.has_store);
        assert!(!analysis.has_rclone_config);
        assert_eq!(analysis.manifest.export_type, "settings");
    }

    #[test]
    fn full_backup_writes_backend_json() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("full.zip");
        let backend = serde_json::json!({ "main": { "transfers": 4 } });
        create_backup_with(
            &dest,
            &AppSettings::default(),
            &AppStore::default(),
            &serde_json::json!({ "drive": { "type": "drive" } }),
            "FullBackup",
            "full",
            None,
            Some(&backend),
        )
        .unwrap();
        let analysis = analyze_backup(&dest).unwrap();
        assert!(analysis.has_backend);
        assert!(analysis.has_rclone_config);
        assert!(analysis.has_settings);
        let restored = restore_backup_contents(&dest, None, None, None).unwrap();
        assert_eq!(restored.backend.unwrap()["main"]["transfers"], 4);
        assert_eq!(restored.rclone.unwrap()["drive"]["type"], "drive");
    }

    #[test]
    fn secrets_require_zip_password() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("secrets.zip");
        let dump = serde_json::json!({
            "drive": { "type": "drive", "token": "secret-token" }
        });
        let err = create_backup(
            &dest,
            &AppSettings::default(),
            &AppStore::default(),
            &dump,
            "FullBackup",
            "",
            None,
        )
        .unwrap_err();
        assert!(err.to_lowercase().contains("password"), "{err}");
        assert!(!dest.exists());
        create_backup(
            &dest,
            &AppSettings::default(),
            &AppStore::default(),
            &dump,
            "FullBackup",
            "",
            Some("correct-horse"),
        )
        .unwrap();
        let restored = restore_backup_contents(&dest, Some("correct-horse"), None, None).unwrap();
        assert_eq!(restored.rclone.unwrap()["drive"]["token"], "secret-token");
        create_backup(
            &dest,
            &AppSettings::default(),
            &AppStore::default(),
            &dump,
            "settings",
            "",
            None,
        )
        .unwrap();
    }

    #[test]
    fn password_protects_backup() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("secret.zip");
        create_backup(
            &dest,
            &AppSettings::default(),
            &AppStore::default(),
            &serde_json::json!({ "demo": { "type": "local" } }),
            "FullBackup",
            "secret note",
            Some("correct-horse"),
        )
        .unwrap();
        assert!(restore_backup(&dest).unwrap().0.is_none());
        let (settings, _, dump) =
            restore_backup_with_password(&dest, Some("correct-horse")).unwrap();
        assert!(settings.is_some());
        assert_eq!(dump.unwrap()["demo"]["type"], "local");
        let analysis = analyze_backup_with_password(&dest, Some("correct-horse")).unwrap();
        assert!(analysis.manifest.encrypted);
        assert_eq!(analysis.manifest.note, "secret note");
    }

    #[test]
    fn export_categories_include_templates_quick_runs_and_nautilus() {
        let ids: Vec<&str> = export_categories().into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&"templates"));
        assert!(ids.contains(&"quick_runs"));
        assert!(ids.contains(&"nautilus"));
        assert!(includes_file("templates", "store.json"));
        assert!(!includes_file("templates", "settings.json"));
        assert!(includes_file("quick_runs", "store.json"));
        assert!(includes_file("nautilus", "settings.json"));
        assert!(!includes_file("nautilus", "store.json"));
    }

    #[test]
    fn templates_and_quick_runs_backups_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AppStore::default();
        store
            .remotes
            .insert("drive".into(), crate::store::RemoteMeta::default());
        store.templates.push(crate::store::UserTemplate {
            id: "tpl-1".into(),
            name: "Fast".into(),
            description: String::new(),
            icon: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            values: serde_json::json!({ "vfs": { "CacheMode": "full" } }),
        });
        store.quick_runs.push(crate::store::QuickRun::new(
            "Nightly".into(),
            crate::operations::OperationType::Sync,
            "drive".into(),
        ));
        let templates_dest = dir.path().join("templates.zip");
        create_backup(
            &templates_dest,
            &AppSettings::default(),
            &store,
            &serde_json::json!({}),
            "templates",
            "templates only",
            None,
        )
        .unwrap();
        let analysis = analyze_backup(&templates_dest).unwrap();
        assert!(analysis.has_store);
        assert!(!analysis.has_settings);
        assert_eq!(analysis.manifest.export_type, "templates");
        assert!(analysis
            .content_rows()
            .iter()
            .any(|(key, _)| *key == "templates.title"));
        let (_, restored, _) = restore_backup(&templates_dest).unwrap();
        let restored = restored.expect("store");
        assert!(restored.remotes.is_empty());
        assert_eq!(restored.templates.len(), 1);
        assert_eq!(restored.templates[0].id, "tpl-1");
        assert!(restored.quick_runs.is_empty());
        let merged = merge_store(&store, &restored, "templates");
        assert!(merged.remotes.contains_key("drive"));
        assert_eq!(merged.templates[0].name, "Fast");

        let qr_dest = dir.path().join("quick_runs.zip");
        create_backup(
            &qr_dest,
            &AppSettings::default(),
            &store,
            &serde_json::json!({}),
            "quick_runs",
            "",
            None,
        )
        .unwrap();
        let qr = restore_backup(&qr_dest).unwrap().1.expect("store");
        assert_eq!(qr.quick_runs.len(), 1);
        assert!(qr.templates.is_empty());
        assert!(qr.remotes.is_empty());
    }

    #[test]
    fn restores_rcman_zip_layout_and_rclone_conf() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("legacy.rcman");
        let file = File::create(&dest).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("sub_settings/remotes/photos.json", options)
            .unwrap();
        zip.write_all(
            serde_json::json!({
                "name": "photos",
                "showOnTray": true,
                "syncConfigs": { "default": { "rclone": { "srcFs": "photos:" } } }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        zip.start_file("quick_runs.json", options).unwrap();
        zip.write_all(
            serde_json::json!([{
                "id": "qr-rcman",
                "name": "Photos sync",
                "operationType": "sync",
                "remoteName": "photos"
            }])
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        zip.start_file("external/rclone.conf", options).unwrap();
        zip.write_all(b"[photos]\ntype = alias\nremote = /tmp/photos\n")
            .unwrap();
        zip.finish().unwrap();

        assert!(looks_like_rcman_archive(
            &dest,
            &[
                "sub_settings/remotes/photos.json".into(),
                "quick_runs.json".into(),
                "external/rclone.conf".into()
            ]
        ));
        let analysis = analyze_backup(&dest).unwrap();
        assert!(analysis.valid);
        assert_eq!(analysis.manifest.export_type, "rcman");
        assert!(analysis.manifest.remotes.contains(&"photos".into()));
        assert!(analysis.has_store);
        assert!(analysis.has_rclone_config);

        let restored = restore_backup_contents(&dest, None, None, None).unwrap();
        let store = restored.store.expect("store");
        assert!(store.remotes.contains_key("photos"));
        assert_eq!(store.quick_runs[0].id, "qr-rcman");
        assert_eq!(restored.rclone.as_ref().unwrap()["photos"]["type"], "alias");
        let current = AppStore::default();
        let merged = merge_store(&current, &store, "rcman");
        assert!(merged.remotes.contains_key("photos"));
    }
}
