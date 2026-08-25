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
        ("settings", "App settings"),
        ("store", "Quick runs, alerts, templates"),
        ("rclone", "Rclone remotes"),
        ("nautilus", "Bookmarks and starred items"),
    ]
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
    let _ = password;
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let file = File::create(dest).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let remotes = rclone_dump
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let manifest = BackupManifest {
        version: env!("CARGO_PKG_VERSION").into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        note: note.to_string(),
        export_type: export_type.to_string(),
        encrypted: false,
        remotes,
    };
    zip.start_file("manifest.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string_pretty(&manifest).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;

    zip.start_file("settings.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string_pretty(settings).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;

    zip.start_file("store.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string_pretty(store).unwrap().as_bytes())
        .map_err(|e| e.to_string())?;

    zip.start_file("rclone.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(
        serde_json::to_string_pretty(rclone_dump)
            .unwrap()
            .as_bytes(),
    )
    .map_err(|e| e.to_string())?;

    zip.finish().map_err(|e| e.to_string())?;
    Ok(dest.to_path_buf())
}

pub fn analyze_backup(path: &Path) -> Result<BackupAnalysis, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            names.push(entry.name().to_string());
        }
    }
    let manifest = if names.iter().any(|n| n == "manifest.json") {
        let mut entry = archive
            .by_name("manifest.json")
            .map_err(|e| e.to_string())?;
        let mut text = String::new();
        entry.read_to_string(&mut text).map_err(|e| e.to_string())?;
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
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let settings = read_zip_json::<AppSettings>(&mut archive, "settings.json");
    let store = read_zip_json::<AppStore>(&mut archive, "store.json");
    let rclone = read_zip_json::<Value>(&mut archive, "rclone.json");
    Ok((settings, store, rclone))
}

fn read_zip_json<T: for<'de> Deserialize<'de>>(
    archive: &mut ZipArchive<File>,
    name: &str,
) -> Option<T> {
    let mut entry = archive.by_name(name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
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
}
