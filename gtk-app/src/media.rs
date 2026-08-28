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

/// First 10 MiB — matches the Angular/Tauri `audio-cover` probe window.
pub const COVER_PROBE_BYTES: u64 = 10_485_760;
/// Warn before downloading a whole remote file this large for preview.
pub const REMOTE_PREVIEW_WARN_BYTES: i64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureData {
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl PictureData {
    pub fn extension(&self) -> &'static str {
        if self.mime_type.contains("png") {
            "png"
        } else if self.mime_type.contains("webp") {
            "webp"
        } else if self.mime_type.contains("gif") {
            "gif"
        } else {
            "jpg"
        }
    }
}

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

pub fn extract_picture_from_path(path: &Path) -> Option<PictureData> {
    let bytes = std::fs::read(path).ok()?;
    extract_picture_from_bytes(&bytes, path.extension().and_then(|e| e.to_str()))
}

pub fn extract_picture_from_bytes(data: &[u8], extension: Option<&str>) -> Option<PictureData> {
    if data.is_empty() {
        return None;
    }
    id3_picture(data)
        .or_else(|| flac_picture(data))
        .or_else(|| mp4_picture(data))
        .or_else(
            || match extension.unwrap_or("").to_ascii_lowercase().as_str() {
                "mp3" | "aac" | "aiff" | "aif" | "wav" => id3_picture(data),
                "flac" | "oga" => flac_picture(data),
                "m4a" | "mp4" | "m4b" | "alac" => mp4_picture(data),
                _ => None,
            },
        )
}

pub fn write_temp_picture(pic: &PictureData) -> Option<PathBuf> {
    if pic.data.is_empty() {
        return None;
    }
    let path = std::env::temp_dir().join(format!(
        "rm-cover-{}-{}.{}",
        std::process::id(),
        pic.data.len(),
        pic.extension()
    ));
    std::fs::write(&path, &pic.data).ok()?;
    path.is_file().then_some(path)
}

pub fn best_audio_cover(path: &Path) -> Option<PathBuf> {
    sibling_cover(path)
        .or_else(|| extract_picture_from_path(path).and_then(|pic| write_temp_picture(&pic)))
}

pub fn should_warn_remote_preview(size: Option<i64>) -> bool {
    size.unwrap_or(0) >= REMOTE_PREVIEW_WARN_BYTES
}

pub fn looks_like_pdf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF")
}

/// Write preview bytes to a temp file named after the remote item.
pub fn write_preview_temp(name: &str, bytes: &[u8]) -> Option<PathBuf> {
    if bytes.is_empty() {
        return None;
    }
    let safe = name.replace(['/', '\\', ':'], "_");
    let dest = std::env::temp_dir().join(format!("rclone-md-preview-{safe}"));
    std::fs::write(&dest, bytes).ok()?;
    dest.is_file().then_some(dest)
}

pub fn read_remote_prefix(
    binary: &Path,
    extra_flags: &[String],
    config: Option<&str>,
    src: &str,
    count: u64,
) -> Option<Vec<u8>> {
    if src.is_empty() || count == 0 {
        return None;
    }
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("cat").arg(format!("--count={count}")).arg(src);
    for flag in extra_flags {
        if crate::rclone::engine::is_reserved_flag(flag) {
            continue;
        }
        cmd.arg(flag);
    }
    if let Some(path) = config {
        if !path.is_empty() {
            cmd.arg(format!("--config={path}"));
        }
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = cmd.output().ok()?;
    if output.stdout.is_empty() {
        return None;
    }
    Some(output.stdout)
}

fn id3_picture(data: &[u8]) -> Option<PictureData> {
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return None;
    }
    let version = data[3];
    let tag_size = synchsafe(&data[6..10])?;
    let end = 10usize.saturating_add(tag_size).min(data.len());
    let mut offset = 10;
    let mut found = None;
    while offset + 10 <= end {
        let id = &data[offset..offset + 4];
        if id.iter().all(|b| *b == 0) {
            break;
        }
        let size = if version >= 4 {
            synchsafe(&data[offset + 4..offset + 8])?
        } else {
            u32_be(&data[offset + 4..offset + 8])? as usize
        };
        let body_start = offset + 10;
        let body_end = body_start.saturating_add(size);
        if body_end > data.len() {
            break;
        }
        if id == b"APIC" {
            if let Some(pic) = parse_apic(&data[body_start..body_end]) {
                if pic.mime_type.contains("jpeg") || pic.mime_type.contains("png") {
                    return Some(pic);
                }
                found = Some(pic);
            }
        }
        offset = body_end;
    }
    found
}

fn parse_apic(body: &[u8]) -> Option<PictureData> {
    if body.is_empty() {
        return None;
    }
    let encoding = body[0];
    let rest = &body[1..];
    let mime_end = rest.iter().position(|&b| b == 0)?;
    let mime = String::from_utf8_lossy(&rest[..mime_end]).to_ascii_lowercase();
    let after_mime = &rest[mime_end + 1..];
    if after_mime.is_empty() {
        return None;
    }
    let after_type = &after_mime[1..];
    let desc_len = skip_id3_string(after_type, encoding)?;
    let image = after_type.get(desc_len..).filter(|b| !b.is_empty())?;
    Some(PictureData {
        data: image.to_vec(),
        mime_type: if mime.is_empty() {
            "image/jpeg".into()
        } else {
            mime
        },
    })
}

fn skip_id3_string(data: &[u8], encoding: u8) -> Option<usize> {
    match encoding {
        0 | 3 => data.iter().position(|&b| b == 0).map(|i| i + 1),
        1 | 2 => {
            let mut i = 0;
            while i + 1 < data.len() {
                if data[i] == 0 && data[i + 1] == 0 {
                    return Some(i + 2);
                }
                i += 2;
            }
            None
        }
        _ => None,
    }
}

fn flac_picture(data: &[u8]) -> Option<PictureData> {
    if data.len() < 8 || &data[0..4] != b"fLaC" {
        return None;
    }
    let mut offset = 4;
    while offset + 4 <= data.len() {
        let header = data[offset];
        let is_last = header & 0x80 != 0;
        let block_type = header & 0x7f;
        let size = u24_be(&data[offset + 1..offset + 4])?;
        let start = offset + 4;
        let end = start.saturating_add(size);
        if end > data.len() {
            break;
        }
        if block_type == 6 {
            if let Some(pic) = parse_flac_picture(&data[start..end]) {
                return Some(pic);
            }
        }
        offset = end;
        if is_last {
            break;
        }
    }
    None
}

fn parse_flac_picture(body: &[u8]) -> Option<PictureData> {
    if body.len() < 8 {
        return None;
    }
    let mut offset = 4;
    let mime_len = u32_be(&body[offset..offset + 4])? as usize;
    offset += 4;
    let mime = String::from_utf8_lossy(body.get(offset..offset + mime_len)?).to_ascii_lowercase();
    offset += mime_len;
    let desc_len = u32_be(body.get(offset..offset + 4)?)? as usize;
    offset += 4 + desc_len + 16;
    let data_len = u32_be(body.get(offset..offset + 4)?)? as usize;
    offset += 4;
    let image = body
        .get(offset..offset + data_len)
        .filter(|b| !b.is_empty())?;
    Some(PictureData {
        data: image.to_vec(),
        mime_type: if mime.is_empty() {
            "image/jpeg".into()
        } else {
            mime
        },
    })
}

fn mp4_picture(data: &[u8]) -> Option<PictureData> {
    find_mp4_atom(data, b"covr").and_then(parse_covr_atom)
}

/// Container atoms nest, and a corrupt or crafted file can nest them without
/// bound — unbounded recursion here would overflow the stack and abort the
/// process. Real files are only a few levels deep.
const MP4_MAX_DEPTH: u8 = 16;

fn find_mp4_atom<'a>(data: &'a [u8], name: &[u8; 4]) -> Option<&'a [u8]> {
    find_mp4_atom_at(data, name, 0)
}

fn find_mp4_atom_at<'a>(data: &'a [u8], name: &[u8; 4], depth: u8) -> Option<&'a [u8]> {
    if depth >= MP4_MAX_DEPTH {
        return None;
    }
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let size = u32_be(&data[offset..offset + 4])? as usize;
        if size < 8 {
            break;
        }
        let kind = &data[offset + 4..offset + 8];
        let end = offset.saturating_add(size).min(data.len());
        if kind == name {
            return Some(&data[offset + 8..end]);
        }
        if matches!(
            kind,
            b"moov" | b"udta" | b"meta" | b"ilst" | b"trak" | b"mdia"
        ) {
            let nested_start = if kind == b"meta" {
                offset + 12
            } else {
                offset + 8
            };
            if nested_start < end {
                if let Some(found) = find_mp4_atom_at(&data[nested_start..end], name, depth + 1) {
                    return Some(found);
                }
            }
        }
        offset = end;
    }
    None
}

fn parse_covr_atom(body: &[u8]) -> Option<PictureData> {
    if body.len() < 16 {
        return None;
    }
    let data = if body.len() > 16 && &body[4..8] == b"data" {
        &body[16..]
    } else {
        body
    };
    if data.is_empty() {
        return None;
    }
    let mime = if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else {
        "image/jpeg"
    };
    Some(PictureData {
        data: data.to_vec(),
        mime_type: mime.into(),
    })
}

fn synchsafe(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 4 {
        return None;
    }
    Some(
        ((bytes[0] as usize & 0x7f) << 21)
            | ((bytes[1] as usize & 0x7f) << 14)
            | ((bytes[2] as usize & 0x7f) << 7)
            | (bytes[3] as usize & 0x7f),
    )
}

fn u32_be(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn u24_be(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 3 {
        return None;
    }
    Some(((bytes[0] as usize) << 16) | ((bytes[1] as usize) << 8) | bytes[2] as usize)
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

/// Caption for a local path field: optional help, item count/size, and `df` free/total.
/// Disk stats are only queried when the engine OS matches this desktop.
pub fn local_path_field_hint(path: &str, engine_os: &str, help: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(help) = help.filter(|s| !s.is_empty()) {
        parts.push(help.to_string());
    }
    if engine_os.eq_ignore_ascii_case(std::env::consts::OS) {
        if let Some(usage) = local_path_usage(path) {
            parts.push(usage);
        }
        if let Some((free, total)) = crate::fileops::local_path_disk_usage(path) {
            parts.push(format!(
                "{} / {}",
                format_bytes(free as i64),
                format_bytes(total as i64)
            ));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

pub fn pdf_page_count(path: &Path) -> Option<u32> {
    if !path.is_file() {
        return None;
    }
    let output = std::process::Command::new("pdfinfo")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(rest) = line.strip_prefix("Pages:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

/// Render one PDF page to a PNG via `pdftoppm` when Poppler tools are installed.
pub fn render_pdf_page(path: &Path, page: u32) -> Option<PathBuf> {
    if !path.is_file() || page == 0 {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy();
    let out = std::env::temp_dir().join(format!("rm-pdf-{stem}-{page}"));
    let status = std::process::Command::new("pdftoppm")
        .args([
            "-png",
            "-f",
            &page.to_string(),
            "-l",
            &page.to_string(),
            "-singlefile",
        ])
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

/// Render the first PDF page to a PNG via `pdftoppm` when Poppler tools are installed.
pub fn render_pdf_preview(path: &Path) -> Option<PathBuf> {
    render_pdf_page(path, 1)
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
    #[test]
    fn mp4_atom_search_is_depth_bounded() {
        // A file that nests container atoms without bound used to recurse until
        // the stack overflowed and the process aborted.
        fn payload() -> Vec<u8> {
            let mut atom = Vec::new();
            atom.extend_from_slice(&12u32.to_be_bytes());
            atom.extend_from_slice(b"covr");
            atom.extend_from_slice(&[1, 2, 3, 4]);
            atom
        }
        fn wrap(levels: usize) -> Vec<u8> {
            let mut buf = payload();
            for _ in 0..levels {
                let size = (buf.len() + 8) as u32;
                let mut next = Vec::with_capacity(size as usize);
                next.extend_from_slice(&size.to_be_bytes());
                next.extend_from_slice(b"udta");
                next.extend_from_slice(&buf);
                buf = next;
            }
            buf
        }

        // Shallow nesting still resolves — real files are only a few deep.
        assert_eq!(
            find_mp4_atom(&wrap(4), b"covr"),
            Some(&[1u8, 2, 3, 4][..]),
            "normal files must still work"
        );

        // Past the cap the search gives up instead of recursing further. Without
        // a cap this would keep descending for as many levels as the file has.
        assert_eq!(
            find_mp4_atom(&wrap(usize::from(MP4_MAX_DEPTH) + 4), b"covr"),
            None,
            "the depth cap must engage"
        );

        // And a pathologically deep file returns rather than blowing the stack.
        assert_eq!(find_mp4_atom(&wrap(50_000), b"covr"), None);
    }

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
        assert_eq!(
            local_path_field_hint("", "other-os", Some("Mount point")),
            Some("Mount point".into())
        );
        assert!(local_path_field_hint("", "other-os", None).is_none());
        let hint = local_path_field_hint(&dir.path().to_string_lossy(), std::env::consts::OS, None)
            .unwrap();
        assert!(hint.contains("item"));
        assert!(hint.contains('/'));
    }

    #[test]
    fn pdf_preview_skips_missing_file() {
        assert!(render_pdf_preview(Path::new("/tmp/rm-missing.pdf")).is_none());
        assert!(render_pdf_page(Path::new("/tmp/rm-missing.pdf"), 1).is_none());
        assert!(pdf_page_count(Path::new("/tmp/rm-missing.pdf")).is_none());
        assert!(render_pdf_page(Path::new("/tmp/rm-missing.pdf"), 0).is_none());
    }

    #[test]
    fn classifies_path_fields() {
        assert!(is_path_field("root_folder_id", "folder id"));
        assert!(is_path_field("local_path", "local directory"));
        assert!(!is_path_field("client_id", "OAuth client"));
        assert!(!is_path_field("url", "remote URL path"));
    }

    #[test]
    fn extracts_id3_apic_cover() {
        let jpeg = [0xff, 0xd8, 0xff, 0xd9];
        let mut apic = vec![0u8];
        apic.extend(b"image/jpeg\0");
        apic.push(3);
        apic.push(0);
        apic.extend(jpeg);
        let mut frame = b"APIC".to_vec();
        frame.extend((apic.len() as u32).to_be_bytes());
        frame.extend([0, 0]);
        frame.extend(apic);
        let mut tag = b"ID3".to_vec();
        tag.extend([3, 0, 0]);
        let size = synchsafe_encode(frame.len());
        tag.extend(size);
        tag.extend(frame);
        tag.extend(b"\xff\xfb\x90\x00");
        let pic = extract_picture_from_bytes(&tag, Some("mp3")).unwrap();
        assert_eq!(pic.data, jpeg);
        assert!(pic.mime_type.contains("jpeg"));
        assert!(!should_warn_remote_preview(Some(1024)));
        assert!(should_warn_remote_preview(Some(REMOTE_PREVIEW_WARN_BYTES)));
        assert!(looks_like_pdf(b"%PDF-1.4\n%"));
        assert!(!looks_like_pdf(b"%!PS"));
        assert!(!looks_like_pdf(b""));
        let dest = write_preview_temp("Docs/a.pdf", b"%PDF-1.4\n").unwrap();
        assert!(dest
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("a.pdf"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"%PDF-1.4\n");
        let _ = std::fs::remove_file(&dest);
        assert!(write_preview_temp("empty.pdf", b"").is_none());
    }

    #[test]
    fn extracts_flac_picture_block() {
        let jpeg = [0xff, 0xd8, 0xff, 0xd9];
        let mut body = Vec::new();
        body.extend(3u32.to_be_bytes());
        let mime = b"image/jpeg";
        body.extend((mime.len() as u32).to_be_bytes());
        body.extend(mime);
        body.extend(0u32.to_be_bytes());
        body.extend([0u8; 16]);
        body.extend((jpeg.len() as u32).to_be_bytes());
        body.extend(jpeg);
        let mut data = b"fLaC".to_vec();
        data.push(0x86);
        data.push(((body.len() >> 16) & 0xff) as u8);
        data.push(((body.len() >> 8) & 0xff) as u8);
        data.push((body.len() & 0xff) as u8);
        data.extend(body);
        let pic = extract_picture_from_bytes(&data, Some("flac")).unwrap();
        assert_eq!(pic.data, jpeg);
        assert_eq!(
            read_remote_prefix(Path::new("/nope"), &[], None, "", 10),
            None
        );
    }

    fn synchsafe_encode(len: usize) -> [u8; 4] {
        [
            ((len >> 21) & 0x7f) as u8,
            ((len >> 14) & 0x7f) as u8,
            ((len >> 7) & 0x7f) as u8,
            (len & 0x7f) as u8,
        ]
    }
}
