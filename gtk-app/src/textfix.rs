//! Text repair and Windows shortcut parsing for the file viewer.
//! Mirrors Angular `file-viewer-modal` `repairText` / `extractLnkInfo` / `looksLikeBinary`.

pub fn is_lnk_name(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .unwrap_or("")
        .eq_ignore_ascii_case("lnk")
}

pub fn decode_preview_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|chunk| {
                (chunk.len() == 2).then_some(u16::from_le_bytes([chunk[0], chunk[1]]))
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|chunk| {
                (chunk.len() == 2).then_some(u16::from_be_bytes([chunk[0], chunk[1]]))
            })
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let lossy = String::from_utf8_lossy(bytes).into_owned();
    repair_text(&lossy)
}

/// Strip interleaved NULLs from mangled UTF-16 that arrived as a UTF-8 string.
pub fn repair_text(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let max_check = content.chars().take(1024).count().max(1);
    let null_count = content.chars().take(1024).filter(|c| *c == '\0').count();
    let null_ratio = null_count as f64 / max_check as f64;
    if null_ratio > 0.4 && null_ratio < 0.6 {
        let repaired = content
            .trim_start_matches('\u{FFFD}')
            .trim_start_matches('\u{FFFD}');
        return repaired.replace('\0', "");
    }
    content.to_string()
}

pub fn looks_like_binary(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }
    let prefix: String = content.chars().take(2).collect();
    if prefix == "\u{FEFF}"
        || content.as_bytes().starts_with(&[0xFF, 0xFE])
        || content.as_bytes().starts_with(&[0xFE, 0xFF])
        || content.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF])
    {
        return false;
    }
    let max_check = content.chars().take(1024).count().max(1);
    let null_count = content.chars().take(1024).filter(|c| *c == '\0').count();
    let null_ratio = null_count as f64 / max_check as f64;
    if null_ratio > 0.1 && (null_ratio < 0.4 || null_ratio > 0.6) {
        return true;
    }
    if content.starts_with("L\0\0\0") || content.as_bytes().starts_with(&[0x4C, 0, 0, 0]) {
        return true;
    }
    let mut non_printable = 0usize;
    let mut checked = 0usize;
    for ch in content.chars().take(1024) {
        if ch == '\0' {
            continue;
        }
        checked += 1;
        let code = ch as u32;
        if code < 32 && code != 9 && code != 10 && code != 13 {
            non_printable += 1;
        }
    }
    checked > 0 && (non_printable as f64 / checked as f64) > 0.3
}

pub fn extract_lnk_targets(content: &str) -> Vec<String> {
    let stripped = content.replace('\0', "");
    let mut targets = collect_win_paths(content);
    for path in collect_win_paths(&stripped) {
        if !targets.iter().any(|existing| existing == &path) {
            targets.push(path);
        }
    }
    targets
}

fn collect_win_paths(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i].is_ascii_alphabetic() && bytes[i + 1] == b':' && bytes[i + 2] == b'\\' {
            let start = i;
            i += 3;
            while i < bytes.len()
                && !matches!(bytes[i], b' ' | 0 | b'\r' | b'\n' | b'\t')
                && bytes[i].is_ascii()
            {
                i += 1;
            }
            if let Ok(candidate) = std::str::from_utf8(&bytes[start..i]) {
                if is_win_target(candidate) && !out.iter().any(|e| e == candidate) {
                    out.push(candidate.to_string());
                }
            }
            continue;
        }
        if bytes[i] == b'%' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'%' && bytes.get(i + 1) == Some(&b'\\') {
                i += 2;
                while i < bytes.len()
                    && !matches!(bytes[i], b' ' | 0 | b'\r' | b'\n' | b'\t')
                    && bytes[i].is_ascii()
                {
                    i += 1;
                }
                if let Ok(candidate) = std::str::from_utf8(&bytes[start..i]) {
                    if !out.iter().any(|e| e == candidate) {
                        out.push(candidate.to_string());
                    }
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_win_target(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".exe")
        || lower.ends_with(".dll")
        || lower.ends_with(".lnk")
        || lower.ends_with(".bat")
        || lower.ends_with(".cmd")
}

/// Right-aligned gutter labels for a CodeMirror-style line-number column.
pub fn line_gutter_text(line_count: i32) -> String {
    let n = line_count.max(1);
    let width = n.to_string().len();
    (1..=n)
        .map(|i| format!("{i:>width$}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_line_gutter() {
        assert_eq!(line_gutter_text(0), "1");
        assert_eq!(line_gutter_text(1), "1");
        assert_eq!(
            line_gutter_text(10),
            " 1\n 2\n 3\n 4\n 5\n 6\n 7\n 8\n 9\n10"
        );
    }

    #[test]
    fn detects_lnk_names() {
        assert!(is_lnk_name("Notepad.lnk"));
        assert!(is_lnk_name("app.LNK"));
        assert!(!is_lnk_name("notes.txt"));
    }

    #[test]
    fn repairs_mangled_utf16() {
        let mangled: String = "H\0e\0l\0l\0o\0 \0W\0o\0r\0l\0d\0".repeat(40);
        let repaired = repair_text(&mangled);
        assert!(repaired.contains("Hello World"));
        assert!(!repaired.contains('\0'));
        assert_eq!(repair_text("plain text"), "plain text");
        assert_eq!(repair_text(""), "");
    }

    #[test]
    fn decodes_utf16_le_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Hi".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(decode_preview_bytes(&bytes), "Hi");
    }

    #[test]
    fn extracts_lnk_windows_paths() {
        let blob = "xxxxC:\\Windows\\notepad.exe\0more %USERPROFILE%\\Apps\\tool.bat yyy";
        let targets = extract_lnk_targets(blob);
        assert!(targets.iter().any(|t| t.ends_with("notepad.exe")));
        assert!(targets.iter().any(|t| t.contains("%USERPROFILE%")));
        assert!(extract_lnk_targets("no targets here").is_empty());
    }

    #[test]
    fn classifies_binary_and_text() {
        assert!(looks_like_binary("L\0\0\0binary"));
        assert!(!looks_like_binary("hello world\nthis is text"));
        assert!(!looks_like_binary(""));
    }
}
