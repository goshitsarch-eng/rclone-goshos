//! Mount helper detection and install (FUSE / WinFsp / FUSE-T).

use serde_json::Value;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::path::Path;
#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPluginInfo {
    pub name: String,
    pub download_url: String,
    pub filename: String,
}

pub fn plugin_label() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "WinFsp"
    }
    #[cfg(target_os = "macos")]
    {
        "FUSE-T"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "FUSE"
    }
}

pub fn missing_title() -> String {
    format!("{} is not available", plugin_label())
}

pub fn missing_detail() -> String {
    #[cfg(target_os = "windows")]
    {
        return "Mounts need WinFsp. Install it, then retry the mount.".into();
    }
    #[cfg(target_os = "macos")]
    {
        return "Mounts need FUSE-T or MacFUSE.".into();
    }
    crate::repair::fuse_install_hint().into()
}

pub fn is_installed() -> bool {
    #[cfg(target_os = "windows")]
    {
        return winfsp_installed();
    }
    #[cfg(target_os = "macos")]
    {
        return fuse_t_or_macfuse_installed();
    }
    crate::repair::fuse_available()
}

#[cfg(target_os = "windows")]
fn winfsp_installed() -> bool {
    [
        r"C:\Program Files\WinFsp",
        r"C:\Program Files (x86)\WinFsp",
        r"C:\Windows\System32\drivers\winfsp.sys",
    ]
    .iter()
    .any(|p| Path::new(p).exists())
        || std::process::Command::new("sc")
            .args(["query", "WinFsp.Launcher"])
            .output()
            .is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).contains("WinFsp.Launcher")
            })
}

#[cfg(target_os = "macos")]
fn fuse_t_or_macfuse_installed() -> bool {
    [
        "/Library/Application Support/fuse-t",
        "/Library/Frameworks/FuseT.framework",
        "/usr/local/bin/mount_fuse-t",
        "/Library/Receipts/MacFUSE.pkg",
        "/Library/Filesystems/macfuse.fs",
        "/usr/local/bin/mount_macfuse",
    ]
    .iter()
    .any(|p| Path::new(p).exists())
}

pub fn winfsp_msi_from_release(body: &Value) -> Option<MountPluginInfo> {
    asset_from_release(body, ".msi", "WinFsp")
}

pub fn fuse_t_pkg_from_release(body: &Value) -> Option<MountPluginInfo> {
    asset_from_release(body, ".pkg", "FUSE-T")
}

fn asset_from_release(body: &Value, suffix: &str, name: &str) -> Option<MountPluginInfo> {
    let assets = body.get("assets")?.as_array()?;
    for asset in assets {
        let filename = asset.get("name")?.as_str()?.to_string();
        if filename.to_ascii_lowercase().ends_with(suffix) {
            return Some(MountPluginInfo {
                name: name.into(),
                download_url: asset.get("browser_download_url")?.as_str()?.to_string(),
                filename,
            });
        }
    }
    None
}

#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub fn fetch_latest_info() -> Result<MountPluginInfo, String> {
    #[cfg(target_os = "windows")]
    {
        let body = fetch_github("https://api.github.com/repos/winfsp/winfsp/releases/latest")?;
        return winfsp_msi_from_release(&body)
            .ok_or_else(|| "No WinFsp MSI in latest release".into());
    }
    #[cfg(target_os = "macos")]
    {
        let body =
            fetch_github("https://api.github.com/repos/macos-fuse-t/fuse-t/releases/latest")?;
        return fuse_t_pkg_from_release(&body)
            .ok_or_else(|| "No FUSE-T PKG in latest release".into());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err("Linux uses the distribution FUSE package".into())
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn fetch_github(url: &str) -> Result<Value, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "rclone-manager-gtk")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| e.to_string())?;
    resp.into_json().map_err(|e| e.to_string())
}

pub fn install() -> Result<String, String> {
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        return crate::repair::try_install_fuse();
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let info = fetch_latest_info()?;
        let dest = std::env::temp_dir().join(&info.filename);
        crate::updater::download_file(&info.download_url, &dest, None, None)?;
        install_local(&dest)?;
        let _ = std::fs::remove_file(&dest);
        if is_installed() {
            Ok(format!("{} installed", info.name))
        } else {
            Err(format!(
                "{} installer finished but {} is still missing",
                info.name,
                plugin_label()
            ))
        }
    }
}

#[cfg(target_os = "windows")]
fn install_local(path: &Path) -> Result<(), String> {
    let status = std::process::Command::new("msiexec")
        .args(["/i", &path.to_string_lossy(), "/qn"])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("msiexec exited with {status}"))
    }
}

#[cfg(target_os = "macos")]
fn install_local(path: &Path) -> Result<(), String> {
    let status = std::process::Command::new("open")
        .arg(path)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn linux_plugin_label_is_fuse() {
        if cfg!(not(any(target_os = "windows", target_os = "macos"))) {
            assert_eq!(plugin_label(), "FUSE");
            assert!(missing_title().contains("FUSE"));
        }
    }

    #[test]
    fn parses_winfsp_and_fuse_t_assets() {
        let winfsp = winfsp_msi_from_release(&json!({
            "assets": [
                { "name": "notes.txt", "browser_download_url": "https://example.com/notes" },
                { "name": "winfsp-2.1.msi", "browser_download_url": "https://example.com/winfsp.msi" }
            ]
        }))
        .unwrap();
        assert_eq!(winfsp.filename, "winfsp-2.1.msi");
        let fuse = fuse_t_pkg_from_release(&json!({
            "assets": [{ "name": "fuse-t-1.0.pkg", "browser_download_url": "https://example.com/fuse.pkg" }]
        }))
        .unwrap();
        assert_eq!(fuse.name, "FUSE-T");
        assert!(fuse_t_pkg_from_release(&json!({ "assets": [] })).is_none());
    }
}
