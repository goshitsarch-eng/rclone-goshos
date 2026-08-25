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
    pub categories: Vec<String>,
}

pub fn export_categories() -> Vec<(&'static str, &'static str)> {
    vec![
        ("FullBackup", "Full backup"),
        ("settings", "App settings"),
        ("store", "Quick runs, alerts, templates"),
        ("rclone", "Rclone remotes"),
        ("nautilus", "Bookmarks and starred items"),
    ]
}

pub fn includes_file(export_type: &str, file: &str) -> bool {
    if file == "manifest.json" {
        return true;
    }
    match export_type {
        "" | "FullBackup" | "All" | "full" => true,
        "settings" | "Settings" => file == "settings.json",
        "store" | "alerts" | "connections" | "nautilus" => file == "store.json",
        "rclone" | "remotes" => file == "rclone.json",
        other if other.starts_with("remote:") => file == "rclone.json",
        _ => true,
    }
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
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let file = File::create(dest).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let encrypted = password.is_some_and(|p| p.trim().len() >= 4);
    let mut options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    if encrypted {
        if let Some(pw) = password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, pw);
        }
    }

    let remotes = rclone_dump
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
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

    let rclone_dump = filter_rclone_dump(rclone_dump, export_type);

    if includes_file(export_type, "settings.json") {
        zip.start_file("settings.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(serde_json::to_string_pretty(settings).unwrap().as_bytes())
            .map_err(|e| e.to_string())?;
    }

    if includes_file(export_type, "store.json") {
        zip.start_file("store.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(serde_json::to_string_pretty(store).unwrap().as_bytes())
            .map_err(|e| e.to_string())?;
    }

    if includes_file(export_type, "rclone.json") {
        zip.start_file("rclone.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(
            serde_json::to_string_pretty(&rclone_dump)
                .unwrap()
                .as_bytes(),
        )
        .map_err(|e| e.to_string())?;
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
    Ok(BackupAnalysis {
        valid: names
            .iter()
            .any(|n| n == "settings.json" || n == "store.json"),
        has_settings: names.iter().any(|n| n == "settings.json"),
        has_store: names.iter().any(|n| n == "store.json"),
        has_rclone_config: names.iter().any(|n| n == "rclone.json"),
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
    if let Some(meta) = store.remotes.remove(from) {
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

pub fn restore_backup_scoped(
    path: &Path,
    password: Option<&str>,
    profile: Option<&str>,
    restore_as: Option<&str>,
) -> Result<(Option<AppSettings>, Option<AppStore>, Option<Value>), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let settings = read_zip_json::<AppSettings>(&mut archive, "settings.json", password);
    let store = read_zip_json::<AppStore>(&mut archive, "store.json", password)
        .map(|s| scoped_store(s, profile, restore_as));
    let rclone = read_zip_json::<Value>(&mut archive, "rclone.json", password)
        .map(|d| scoped_rclone(d, profile, restore_as));
    Ok((settings, store, rclone))
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
    fn category_filter_omits_unselected_files() {
        assert!(includes_file("settings", "settings.json"));
        assert!(!includes_file("settings", "store.json"));
        assert!(includes_file("store", "store.json"));
        assert!(!includes_file("rclone", "settings.json"));
        assert!(includes_file("remote:demo", "rclone.json"));
        let dump = serde_json::json!({ "demo": { "type": "local" }, "other": { "type": "drive" } });
        let filtered = filter_rclone_dump(&dump, "remote:demo");
        assert!(filtered.get("demo").is_some());
        assert!(filtered.get("other").is_none());
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
}
