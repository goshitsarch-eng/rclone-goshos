//! Local media helpers: folder artwork and path-field hints.

use crate::rclone::format_bytes;
use std::path::{Path, PathBuf};

const COVER_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.png",
    "cover.webp",
    "folder.jpg",
    "Folder.jpg",
    "album.jpg",
    "AlbumArt.jpg",
    "AlbumArtSmall.jpg",
];

pub fn sibling_cover(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    for name in COVER_NAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn local_path_usage(path: &str) -> Option<String> {
    let path = Path::new(path.trim());
    if !path.exists() {
        return None;
    }
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        let count = std::fs::read_dir(path).ok()?.count();
        Some(format!("{count} items"))
    } else {
        Some(format_bytes(meta.len() as i64))
    }
}

/// Render the first PDF page to a PNG via `pdftoppm` when Poppler tools are installed.
pub fn render_pdf_preview(path: &Path) -> Option<PathBuf> {
    if !path.is_file() {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy();
    let out = std::env::temp_dir().join(format!("rm-pdf-{stem}"));
    let status = std::process::Command::new("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-singlefile"])
        .arg(path)
        .arg(&out)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    let png = out.with_extension("png");
    png.is_file().then_some(png)
}

pub fn is_path_field(name: &str, help: &str) -> bool {
    let hay = format!("{name} {help}").to_ascii_lowercase();
    [
        "path",
        "folder",
        "directory",
        "dir",
        "root",
        "mountpoint",
        "mount_point",
        "local",
    ]
    .iter()
    .any(|needle| hay.contains(needle))
        && !hay.contains("url")
        && !hay.contains("password")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sibling_cover() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("track.mp3");
        std::fs::write(&audio, b"xx").unwrap();
        assert!(sibling_cover(&audio).is_none());
        let cover = dir.path().join("cover.jpg");
        std::fs::write(&cover, b"yy").unwrap();
        assert_eq!(sibling_cover(&audio).unwrap(), cover);
        assert_eq!(sibling_cover(dir.path()).unwrap(), cover);
    }

    #[test]
    fn path_usage_for_file_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.bin");
        std::fs::write(&file, vec![0u8; 32]).unwrap();
        assert_eq!(local_path_usage(&file.to_string_lossy()).unwrap(), "32 B");
        let usage = local_path_usage(&dir.path().to_string_lossy()).unwrap();
        assert!(usage.contains("item"));
        assert!(local_path_usage("/definitely/missing").is_none());
    }

    #[test]
    fn pdf_preview_skips_missing_file() {
        assert!(render_pdf_preview(Path::new("/tmp/rm-missing.pdf")).is_none());
    }

    #[test]
    fn classifies_path_fields() {
        assert!(is_path_field("root_folder_id", "folder id"));
        assert!(is_path_field("local_path", "local directory"));
        assert!(!is_path_field("client_id", "OAuth client"));
        assert!(!is_path_field("url", "remote URL path"));
    }
}
