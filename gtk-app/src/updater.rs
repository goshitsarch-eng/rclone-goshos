//! App and rclone update checks (GitHub releases + rclone.org).

use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub url: String,
    pub available: bool,
    pub download_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

impl DownloadProgress {
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.downloaded as f64 / self.total as f64).clamp(0.0, 1.0)
    }

    pub fn label(&self) -> String {
        if self.total == 0 {
            return crate::rclone::format_bytes(self.downloaded as i64);
        }
        format!(
            "{} / {}",
            crate::rclone::format_bytes(self.downloaded as i64),
            crate::rclone::format_bytes(self.total as i64)
        )
    }
}

pub fn version_is_newer(latest: &str, current: &str) -> bool {
    let latest = normalize_version(latest);
    let current = normalize_version(current);
    if latest.is_empty() || current.is_empty() {
        return false;
    }
    compare_semver(&latest, &current) == std::cmp::Ordering::Greater
}

fn normalize_version(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('v')
        .trim_start_matches("rclone-")
        .split(['-', '+', ' '])
        .next()
        .unwrap_or("")
        .to_string()
}

fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| {
        s.split('.')
            .take(3)
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let aa = parse(a);
    let bb = parse(b);
    for i in 0..3 {
        let av = aa.get(i).copied().unwrap_or(0);
        let bv = bb.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

pub fn linux_asset_url(json: &Value) -> Option<String> {
    let assets = json.get("assets")?.as_array()?;
    let mut appimage = None;
    let mut tarball = None;
    let mut deb = None;
    for asset in assets {
        let name = asset.get("name")?.as_str()?.to_ascii_lowercase();
        let url = asset.get("browser_download_url")?.as_str()?.to_string();
        if !asset_matches_arch(&name) {
            continue;
        }
        if name.ends_with(".appimage") {
            appimage = Some(url);
        } else if name.contains("linux") && (name.ends_with(".tar.gz") || name.ends_with(".tgz")) {
            tarball = Some(url);
        } else if name.ends_with(".deb") {
            deb = Some(url);
        }
    }
    appimage.or(tarball).or(deb)
}

fn asset_matches_arch(name: &str) -> bool {
    let wants_arm = cfg!(target_arch = "aarch64");
    let is_arm = name.contains("aarch64") || name.contains("arm64");
    let is_x64 = name.contains("x86_64") || name.contains("amd64") || name.contains("x64");
    if wants_arm {
        return is_arm || (!is_x64 && !is_arm);
    }
    is_x64 || (!is_arm && !is_x64)
}

pub fn parse_github_release(body: &Value, current: &str) -> Option<UpdateInfo> {
    let tag = body.get("tag_name")?.as_str()?.to_string();
    let url = body
        .get("html_url")
        .and_then(|x| x.as_str())
        .unwrap_or("https://github.com/Zarestia-Dev/rclone-manager/releases")
        .to_string();
    Some(UpdateInfo {
        available: version_is_newer(&tag, current),
        latest: tag,
        current: current.to_string(),
        url,
        download_url: linux_asset_url(body),
    })
}

pub fn fetch_app_update(current: &str) -> Result<UpdateInfo, String> {
    let resp =
        ureq::get("https://api.github.com/repos/Zarestia-Dev/rclone-manager/releases/latest")
            .set("User-Agent", "rclone-manager-gtk")
            .timeout(Duration::from_secs(8))
            .call()
            .map_err(|e| e.to_string())?;
    let body: Value = resp.into_json().map_err(|e| e.to_string())?;
    parse_github_release(&body, current).ok_or_else(|| "invalid GitHub response".into())
}

pub fn rclone_linux_zip_url() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "https://downloads.rclone.org/rclone-current-linux-arm64.zip"
    } else {
        "https://downloads.rclone.org/rclone-current-linux-amd64.zip"
    }
}

pub fn download_file(
    url: &str,
    dest: &Path,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<Arc<Mutex<DownloadProgress>>>,
) -> Result<u64, String> {
    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err("cancelled".into());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let resp = ureq::get(url)
        .set("User-Agent", "rclone-manager-gtk")
        .timeout(Duration::from_secs(180))
        .call()
        .map_err(|e| e.to_string())?;
    let total = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    loop {
        if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            drop(out);
            let _ = std::fs::remove_file(dest);
            return Err("cancelled".into());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        if let Some(slot) = &progress {
            if let Ok(mut guard) = slot.lock() {
                guard.downloaded = downloaded;
                guard.total = total;
            }
        }
    }
    Ok(downloaded)
}

pub fn replace_executable(new_bin: &Path, current_exe: &Path) -> Result<(), String> {
    if !new_bin.exists() {
        return Err("update file missing".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(new_bin)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(new_bin, perms).map_err(|e| e.to_string())?;
    }
    let bak = current_exe.with_file_name(format!(
        "{}.bak",
        current_exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "rclone-manager".into())
    ));
    let _ = std::fs::remove_file(&bak);
    if current_exe.exists() {
        if let Err(e) = std::fs::rename(current_exe, &bak) {
            std::fs::copy(current_exe, &bak).map_err(|copy| format!("{e}; {copy}"))?;
        }
    }
    if let Err(e) = std::fs::rename(new_bin, current_exe) {
        std::fs::copy(new_bin, current_exe).map_err(|copy| format!("{e}; {copy}"))?;
        let _ = std::fs::remove_file(new_bin);
    }
    Ok(())
}

pub fn install_app_update(
    url: &str,
    current_exe: &Path,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<Arc<Mutex<DownloadProgress>>>,
) -> Result<PathBuf, String> {
    let parent = current_exe
        .parent()
        .ok_or_else(|| "cannot locate application directory".to_string())?;
    let tmp = parent.join(format!(
        "{}.update",
        current_exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "rclone-manager".into())
    ));
    download_file(url, &tmp, cancel, progress)?;
    replace_executable(&tmp, current_exe)?;
    Ok(current_exe.to_path_buf())
}

pub fn install_rclone_binary(dest_dir: &Path) -> Result<PathBuf, String> {
    install_rclone_binary_ex(dest_dir, None, None)
}

pub fn install_rclone_binary_ex(
    dest_dir: &Path,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<Arc<Mutex<DownloadProgress>>>,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
    let zip_path = dest_dir.join(".rclone-download.zip");
    download_file(rclone_linux_zip_url(), &zip_path, cancel, progress)?;
    let bytes = std::fs::read(&zip_path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&zip_path);
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();
        if name.ends_with("/rclone") || name == "rclone" || name.ends_with("rclone.exe") {
            let dest = dest_dir.join(if name.ends_with(".exe") {
                "rclone.exe"
            } else {
                "rclone"
            });
            let mut out = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&dest)
                    .map_err(|e| e.to_string())?
                    .permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&dest, perms).map_err(|e| e.to_string())?;
            }
            return Ok(dest);
        }
    }
    Err("rclone binary not found in download archive".into())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingUpdates {
    pub app: Option<UpdateInfo>,
    pub rclone: Option<UpdateInfo>,
}

impl PendingUpdates {
    pub fn has_updates(&self) -> bool {
        self.app.as_ref().is_some_and(|u| u.available)
            || self.rclone.as_ref().is_some_and(|u| u.available)
    }

    pub fn banner_kind(&self) -> &'static str {
        let app = self.app.as_ref().is_some_and(|u| u.available);
        let rclone = self.rclone.as_ref().is_some_and(|u| u.available);
        match (app, rclone) {
            (true, true) => "all",
            (true, false) => "app",
            (false, true) => "rclone",
            (false, false) => "none",
        }
    }
}

pub fn filter_skipped(info: Option<UpdateInfo>, skipped: &[String]) -> Option<UpdateInfo> {
    info.filter(|u| {
        u.available
            && !skipped.iter().any(|s| {
                normalize_version(s) == normalize_version(&u.latest)
                    || s == &u.latest
                    || s == &u.current
            })
    })
}

pub fn fetch_rclone_update(current: &str) -> Result<UpdateInfo, String> {
    let resp = ureq::get("https://downloads.rclone.org/version.txt")
        .set("User-Agent", "rclone-manager-gtk")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| e.to_string())?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let latest = text
        .split_whitespace()
        .find(|p| p.starts_with('v') || p.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or(text.trim())
        .to_string();
    Ok(UpdateInfo {
        available: version_is_newer(&latest, current),
        latest,
        current: current.to_string(),
        url: "https://rclone.org/downloads/".into(),
        download_url: Some(rclone_linux_zip_url().into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_app() -> UpdateInfo {
        UpdateInfo {
            current: "0.3.2".into(),
            latest: "v0.4.0".into(),
            url: "https://example.com".into(),
            available: true,
            download_url: Some("https://example.com/app.AppImage".into()),
        }
    }

    #[test]
    fn compares_versions() {
        assert!(version_is_newer("v0.4.0", "0.3.2"));
        assert!(!version_is_newer("0.3.2", "0.3.2"));
        assert!(!version_is_newer("0.3.1", "0.3.2"));
        assert!(version_is_newer("1.68.0", "rclone-v1.67.3"));
    }

    #[test]
    fn parses_github_payload() {
        let info = parse_github_release(
            &json!({
                "tag_name": "v0.9.0",
                "html_url": "https://example.com/r",
                "assets": [{
                    "name": "rclone-manager_0.9.0_amd64.AppImage",
                    "browser_download_url": "https://example.com/app.AppImage"
                }]
            }),
            "0.3.2",
        )
        .unwrap();
        assert!(info.available);
        assert_eq!(info.latest, "v0.9.0");
        assert_eq!(info.url, "https://example.com/r");
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://example.com/app.AppImage")
        );
    }

    #[test]
    fn linux_asset_prefers_appimage() {
        let url = linux_asset_url(&json!({
            "assets": [
                {
                    "name": "rclone-manager_0.9.0_amd64.deb",
                    "browser_download_url": "https://example.com/app.deb"
                },
                {
                    "name": "rclone-manager-0.9.0-linux-x86_64.tar.gz",
                    "browser_download_url": "https://example.com/app.tgz"
                },
                {
                    "name": "rclone-manager_0.9.0_amd64.AppImage",
                    "browser_download_url": "https://example.com/app.AppImage"
                }
            ]
        }));
        assert_eq!(url.as_deref(), Some("https://example.com/app.AppImage"));
    }

    #[test]
    fn linux_zip_url_is_official() {
        assert!(rclone_linux_zip_url().contains("downloads.rclone.org"));
    }

    #[test]
    fn pending_updates_classify_banner() {
        let app = sample_app();
        let pending = PendingUpdates {
            app: Some(app.clone()),
            rclone: None,
        };
        assert!(pending.has_updates());
        assert_eq!(pending.banner_kind(), "app");
        assert!(filter_skipped(Some(app.clone()), &["v0.4.0".into()]).is_none());
        assert!(filter_skipped(Some(app), &["0.3.1".into()]).is_some());
    }

    #[test]
    fn download_progress_fraction_and_label() {
        let p = DownloadProgress {
            downloaded: 512,
            total: 1024,
        };
        assert!((p.fraction() - 0.5).abs() < f64::EPSILON);
        assert!(p.label().contains('/'));
        assert_eq!(DownloadProgress::default().fraction(), 0.0);
    }

    #[test]
    fn replace_executable_swaps_files() {
        let dir = std::env::temp_dir().join(format!("rm-gtk-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let current = dir.join("rclone-manager");
        let incoming = dir.join("rclone-manager.update");
        std::fs::write(&current, b"old").unwrap();
        std::fs::write(&incoming, b"new").unwrap();
        replace_executable(&incoming, &current).unwrap();
        assert_eq!(std::fs::read(&current).unwrap(), b"new");
        assert_eq!(
            std::fs::read(dir.join("rclone-manager.bak")).unwrap(),
            b"old"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelled_download_errors_before_network() {
        let cancel = Arc::new(AtomicBool::new(true));
        let dest = std::env::temp_dir().join("rm-gtk-cancelled.bin");
        let err =
            download_file("https://example.invalid/file", &dest, Some(cancel), None).unwrap_err();
        assert_eq!(err, "cancelled");
    }
}
