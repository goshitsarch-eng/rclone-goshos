//! Parse rclone check/cryptcheck results and decide resolve actions.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: String,
    pub status: String,
    pub src_fs: String,
    pub dst_fs: String,
}

impl CheckResult {
    pub fn resolve_kind(&self) -> Option<&'static str> {
        match self.status.as_str() {
            "missing_dst" | "partial" | "differ" => Some("copy_src_to_dst"),
            "missing_src" => Some("copy_dst_to_src"),
            _ => None,
        }
    }

    pub fn needs_overwrite_confirm(&self) -> bool {
        matches!(self.status.as_str(), "partial" | "differ")
    }
}

pub fn parse_check_items(
    value: &Value,
    fallback_src: &str,
    fallback_dst: &str,
) -> Vec<CheckResult> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| parse_one(item, fallback_src, fallback_dst))
            .collect(),
        Value::Object(map) => {
            if let Some(arr) = map.get("results").or_else(|| map.get("checks")) {
                return parse_check_items(arr, fallback_src, fallback_dst);
            }
            parse_one(value, fallback_src, fallback_dst)
                .into_iter()
                .collect()
        }
        Value::String(line) => parse_combined_line(line, fallback_src, fallback_dst)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_one(item: &Value, fallback_src: &str, fallback_dst: &str) -> Option<CheckResult> {
    if let Some(line) = item.as_str() {
        return parse_combined_line(line, fallback_src, fallback_dst);
    }
    let name = item
        .get("name")
        .or_else(|| item.get("Path"))
        .or_else(|| item.get("path"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let status = item
        .get("status")
        .or_else(|| item.get("Status"))
        .and_then(|x| x.as_str())
        .unwrap_or("checked")
        .to_string();
    Some(CheckResult {
        name,
        status: normalize_status(&status),
        src_fs: item
            .get("srcFs")
            .or_else(|| item.get("src_fs"))
            .and_then(|x| x.as_str())
            .unwrap_or(fallback_src)
            .to_string(),
        dst_fs: item
            .get("dstFs")
            .or_else(|| item.get("dst_fs"))
            .and_then(|x| x.as_str())
            .unwrap_or(fallback_dst)
            .to_string(),
    })
}

fn parse_combined_line(line: &str, fallback_src: &str, fallback_dst: &str) -> Option<CheckResult> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (status, name) = if let Some(rest) = trimmed.strip_prefix("= ") {
        ("match", rest)
    } else if let Some(rest) = trimmed.strip_prefix("- ") {
        ("missing_dst", rest)
    } else if let Some(rest) = trimmed.strip_prefix("+ ") {
        ("missing_src", rest)
    } else if let Some(rest) = trimmed.strip_prefix("* ") {
        ("differ", rest)
    } else if let Some(rest) = trimmed.strip_prefix("! ") {
        ("error", rest)
    } else {
        return None;
    };
    Some(CheckResult {
        name: name.to_string(),
        status: status.into(),
        src_fs: fallback_src.into(),
        dst_fs: fallback_dst.into(),
    })
}

fn normalize_status(status: &str) -> String {
    match status.to_ascii_lowercase().as_str() {
        "missingondst" | "missing_on_dst" | "missing-dst" => "missing_dst".into(),
        "missingonsrc" | "missing_on_src" | "missing-src" => "missing_src".into(),
        "difference" | "diff" => "differ".into(),
        other => other.to_string(),
    }
}

pub fn is_check_operation(op: &str) -> bool {
    matches!(
        op.trim().to_ascii_lowercase().as_str(),
        "check" | "cryptcheck"
    )
}

pub fn check_source_from_job(stats: &Value, output: &Value) -> Value {
    stats
        .get("checks")
        .or_else(|| output.get("results"))
        .or_else(|| output.get("cryptcheck").and_then(|v| v.get("results")))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]))
}

pub fn parent_remote_path(path: &str) -> String {
    if let Some((parent, _)) = path.rsplit_once('/') {
        parent.to_string()
    } else {
        String::new()
    }
}

pub fn leaf_name(path: &str) -> String {
    path.rsplit(['/', ':']).next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_object_and_combined_lines() {
        let items = parse_check_items(
            &json!([
                { "name": "a.txt", "status": "missing_dst", "srcFs": "drive:", "dstFs": "/tmp" },
                "- photos/b.jpg",
                "+ only-dst.bin",
                "* clash.dat"
            ]),
            "src:",
            "dst:",
        );
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].resolve_kind(), Some("copy_src_to_dst"));
        assert_eq!(items[1].status, "missing_dst");
        assert_eq!(items[2].resolve_kind(), Some("copy_dst_to_src"));
        assert!(items[3].needs_overwrite_confirm());
    }

    #[test]
    fn normalizes_status_aliases() {
        let items = parse_check_items(
            &json!([{ "name": "x", "status": "missingOnDst" }]),
            "a:",
            "b:",
        );
        assert_eq!(items[0].status, "missing_dst");
        assert_eq!(parent_remote_path("dir/sub/file"), "dir/sub");
        assert_eq!(leaf_name("drive:dir/file.txt"), "file.txt");
    }

    #[test]
    fn ignores_empty_and_unknown_lines() {
        assert!(parse_check_items(&json!(["hello", ""]), "a:", "b:").is_empty());
    }

    #[test]
    fn detects_check_operations_and_job_source() {
        assert!(is_check_operation("check"));
        assert!(is_check_operation("CryptCheck"));
        assert!(!is_check_operation("sync"));
        let source = check_source_from_job(
            &json!({ "bytes": 1 }),
            &json!({ "results": [{ "name": "a.txt", "status": "differ" }] }),
        );
        assert_eq!(source[0]["name"], "a.txt");
        let from_stats = check_source_from_job(
            &json!({ "checks": [{ "name": "b.bin", "status": "missing_dst" }] }),
            &json!({}),
        );
        assert_eq!(from_stats[0]["name"], "b.bin");
    }
}
