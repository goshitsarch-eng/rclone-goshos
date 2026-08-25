//! Lightweight Markdown preview for the in-app file viewer.

pub fn is_markdown(name: &str) -> bool {
    matches!(
        name.rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "md" | "markdown" | "mdown"
    )
}

/// Convert common Markdown constructs into wrapped plain text for a GTK preview.
pub fn to_preview(source: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some(rest) = heading(trimmed) {
            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            out.push_str(rest);
            out.push('\n');
            continue;
        }
        if let Some(item) = list_item(trimmed) {
            out.push_str("• ");
            out.push_str(item);
            out.push('\n');
            continue;
        }
        if trimmed.starts_with("> ") {
            out.push_str(trimmed.trim_start_matches("> ").trim());
            out.push('\n');
            continue;
        }
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            out.push_str("────────\n");
            continue;
        }
        out.push_str(&inline(line));
        out.push('\n');
    }
    out.trim().to_string()
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        Some(line[hashes + 1..].trim())
    } else {
        None
    }
}

fn list_item(line: &str) -> Option<&str> {
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest.trim());
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && line.get(digits..digits + 2) == Some(". ") {
        return Some(line[digits + 2..].trim());
    }
    None
}

fn inline(line: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        if chars[i] == '[' {
            if let Some((label, _href, next)) = parse_link(&chars, i) {
                out.push_str(&label);
                i = next;
                continue;
            }
        }
        if (chars[i] == '*' || chars[i] == '_') && i + 1 < chars.len() {
            let marker = chars[i];
            let double = chars.get(i + 1) == Some(&marker);
            let start = if double { i + 2 } else { i + 1 };
            if let Some(end) = chars[start..].iter().position(|c| *c == marker) {
                let text: String = chars[start..start + end].iter().collect();
                out.push_str(text.trim_start_matches(marker).trim_end_matches(marker));
                i = start + end + 1;
                if double && chars.get(i) == Some(&marker) {
                    i += 1;
                }
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Keep scheme, root, and fragment refs unchanged — same as Angular `resolveRelativePath`.
pub fn is_passthrough_ref(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('/') {
        return true;
    }
    if let Some(idx) = trimmed.find(':') {
        idx > 0 && trimmed[..idx].chars().all(|c| c.is_ascii_alphabetic())
    } else {
        false
    }
}

pub fn resolve_relative_path(base_file: &str, relative: &str) -> String {
    let relative = relative.trim();
    if relative.is_empty() || is_passthrough_ref(relative) {
        return relative.to_string();
    }
    let dir = match base_file.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => "",
    };
    normalize_join(dir, relative)
}

fn normalize_join(dir: &str, relative: &str) -> String {
    let combined = if dir.is_empty() {
        relative.to_string()
    } else {
        format!("{dir}/{relative}")
    };
    let leading = combined.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in combined.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            parts.pop();
        } else {
            parts.push(seg);
        }
    }
    let body = parts.join("/");
    if leading {
        format!("/{body}")
    } else {
        body
    }
}

/// Markdown/HTML relative targets used by the Angular file viewer rewrite pass.
pub fn relative_targets(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    collect_md_targets(source, &mut out);
    collect_html_attr(source, "src", &mut out);
    collect_html_attr(source, "href", &mut out);
    out
}

fn push_target(out: &mut Vec<(String, String)>, label: &str, path: &str) {
    let path = path.trim();
    if path.is_empty()
        || path.starts_with('#')
        || path.starts_with("http://")
        || path.starts_with("https://")
        || path.starts_with("mailto:")
    {
        return;
    }
    if out.iter().any(|(_, existing)| existing == path) {
        return;
    }
    let label = if label.is_empty() { path } else { label };
    out.push((label.to_string(), path.to_string()));
}

fn collect_md_targets(source: &str, out: &mut Vec<(String, String)>) {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' || (chars[i] == '!' && chars.get(i + 1) == Some(&'[')) {
            let start = if chars[i] == '!' { i + 1 } else { i };
            if let Some((label, href, next)) = parse_md_target(&chars, start) {
                push_target(out, &label, &href);
                i = next;
                continue;
            }
        }
        i += 1;
    }
}

fn parse_md_target(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    parse_link(chars, start)
}

fn collect_html_attr(source: &str, attr: &str, out: &mut Vec<(String, String)>) {
    let needle = format!("{attr}=\"");
    let needle_sq = format!("{attr}='");
    for (needle, quote) in [(&needle[..], '"'), (&needle_sq[..], '\'')] {
        let mut rest = source;
        while let Some(idx) = rest.to_ascii_lowercase().find(&needle.to_ascii_lowercase()) {
            let after = &rest[idx + needle.len()..];
            if let Some(end) = after.find(quote) {
                let value = &after[..end];
                push_target(out, value, value);
                rest = &after[end + 1..];
            } else {
                break;
            }
        }
    }
}

fn parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut i = start + 1;
    let mut label = String::new();
    while i < chars.len() && chars[i] != ']' {
        label.push(chars[i]);
        i += 1;
    }
    if chars.get(i) != Some(&']') || chars.get(i + 1) != Some(&'(') {
        return None;
    }
    i += 2;
    let href_start = i;
    while i < chars.len() && chars[i] != ')' {
        i += 1;
    }
    if chars.get(i) != Some(&')') {
        return None;
    }
    let href: String = chars[href_start..i].iter().collect();
    Some((label, href, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_markdown_names() {
        assert!(is_markdown("README.md"));
        assert!(is_markdown("notes.MARKDOWN"));
        assert!(!is_markdown("readme.txt"));
    }

    #[test]
    fn renders_headings_lists_and_links() {
        let preview = to_preview("# Title\n\n- one\n- two\n\nSee [docs](https://example.com).\n");
        assert!(preview.contains("Title"));
        assert!(preview.contains("• one"));
        assert!(preview.contains("docs"));
        assert!(!preview.contains("https://example.com"));
        assert!(!preview.contains("# Title"));
    }

    #[test]
    fn strips_fences_and_emphasis() {
        let preview = to_preview("```\ncode\n```\n**bold** and `tick`\n");
        assert!(preview.contains("code"));
        assert!(preview.contains("bold"));
        assert!(preview.contains("tick"));
        assert!(!preview.contains("```"));
        assert!(!preview.contains("**"));
    }

    #[test]
    fn resolves_relative_markdown_paths() {
        assert_eq!(
            resolve_relative_path("docs/guide.md", "images/a.png"),
            "docs/images/a.png"
        );
        assert_eq!(
            resolve_relative_path("docs/guide.md", "../README.md"),
            "README.md"
        );
        assert_eq!(
            resolve_relative_path("docs/guide.md", "https://example.com/a"),
            "https://example.com/a"
        );
        assert_eq!(
            resolve_relative_path("docs/guide.md", "/abs.png"),
            "/abs.png"
        );
        assert_eq!(resolve_relative_path("docs/guide.md", "#sec"), "#sec");
        assert_eq!(
            resolve_relative_path("docs/guide.md", "drive:Photos"),
            "drive:Photos"
        );
        assert!(is_passthrough_ref("https://ex"));
        assert!(!is_passthrough_ref("img.png"));
        let targets = relative_targets(
            "See [docs](../help.md) and ![cover](art/cover.jpg).\n<img src=\"pic.png\">\n<a href=\"https://ex\">x</a>",
        );
        assert!(targets
            .iter()
            .any(|(l, p)| l == "docs" && p == "../help.md"));
        assert!(targets
            .iter()
            .any(|(l, p)| l == "cover" && p == "art/cover.jpg"));
        assert!(targets.iter().any(|(_, p)| p == "pic.png"));
        assert!(!targets.iter().any(|(_, p)| p.starts_with("https://")));
    }
}
