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
            if let Some((label, next)) = parse_link(&chars, i) {
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

fn parse_link(chars: &[char], start: usize) -> Option<(String, usize)> {
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
    while i < chars.len() && chars[i] != ')' {
        i += 1;
    }
    if chars.get(i) != Some(&')') {
        return None;
    }
    Some((label, i + 1))
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
}
