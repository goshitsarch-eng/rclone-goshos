//! Parse rclone check/cryptcheck results and decide resolve actions.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: String,
    pub status: String,
    pub src_fs: String,
    pub dst_fs: String,
    pub job_id: Option<u64>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
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
        job_id: None,
        completed_at: None,
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
        job_id: None,
        completed_at: None,
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

pub fn check_unique_id(job_id: Option<u64>, name: &str) -> String {
    format!("{}-{name}", job_id.unwrap_or(0))
}

pub fn check_item_matches_query(item: &CheckResult, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_ascii_lowercase();
    [&item.name, &item.status, &item.src_fs, &item.dst_fs]
        .iter()
        .any(|value| value.to_ascii_lowercase().contains(&q))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckDeleteOutcome {
    Hide,
    Override(&'static str),
}

pub fn check_delete_outcome(status: &str, deleted_source: bool) -> CheckDeleteOutcome {
    if matches!(status, "partial" | "checked" | "differ") {
        CheckDeleteOutcome::Override(if deleted_source {
            "missing_src"
        } else {
            "missing_dst"
        })
    } else {
        CheckDeleteOutcome::Hide
    }
}

pub fn with_job_id(mut item: CheckResult, job_id: u64) -> CheckResult {
    item.job_id = Some(job_id);
    item
}

pub fn with_job(mut item: CheckResult, job: &crate::store::JobInfo) -> CheckResult {
    item.job_id = Some(job.id);
    if job.start_time.timestamp() > 0 {
        item.completed_at = Some(job.start_time);
    }
    item
}

pub fn relative_time_parts(
    then: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> (&'static str, i64) {
    let secs = now.signed_duration_since(then).num_seconds().max(0);
    if secs < 60 {
        ("shared.transferActivity.time.justNow", 0)
    } else if secs < 3600 {
        ("shared.transferActivity.time.minutesAgo", secs / 60)
    } else if secs < 86_400 {
        ("shared.transferActivity.time.hoursAgo", secs / 3600)
    } else {
        ("shared.transferActivity.time.daysAgo", secs / 86_400)
    }
}

pub fn check_status_icon(status: &str) -> &'static str {
    match status {
        "checked" | "match" => "emblem-ok-symbolic",
        "failed" | "error" => "dialog-error-symbolic",
        _ => "dialog-warning-symbolic",
    }
}

pub fn resolve_icon(status: &str) -> &'static str {
    if status == "missing_src" {
        "go-previous-symbolic"
    } else {
        "go-next-symbolic"
    }
}

pub fn resolve_is_preparing(progress: f64, bytes: i64) -> bool {
    progress <= 0.0 && bytes <= 0
}

pub fn format_resolve_progress(bytes: i64, size: i64, speed: f64, eta_secs: i64) -> String {
    let mut parts = vec![format!(
        "{} / {}",
        crate::rclone::format_bytes(bytes),
        crate::rclone::format_bytes(size)
    )];
    if speed > 0.0 {
        parts.push(format!(
            "{}/s",
            crate::rclone::format_bytes(speed.round() as i64)
        ));
    }
    if eta_secs > 0 {
        parts.push(format!(
            "ETA {}",
            crate::rclone::format_eta_seconds(eta_secs)
        ));
    }
    parts.join(" · ")
}

pub fn check_status_key(status: &str) -> &'static str {
    match status {
        "missing_dst" => "shared.transferActivity.status.missingDst",
        "missing_src" => "shared.transferActivity.status.missingSrc",
        "differ" | "partial" => "shared.transferActivity.status.differ",
        "checked" | "match" => "shared.transferActivity.status.checked",
        "failed" | "error" => "shared.transferActivity.status.error",
        _ => "shared.transferActivity.status.error",
    }
}

pub fn visible_check_items(
    items: impl IntoIterator<Item = CheckResult>,
    hidden: &HashSet<String>,
    overrides: &HashMap<String, String>,
    query: &str,
) -> Vec<CheckResult> {
    items
        .into_iter()
        .filter_map(|mut item| {
            let id = check_unique_id(item.job_id, &item.name);
            if hidden.contains(&id) {
                return None;
            }
            if let Some(status) = overrides.get(&id) {
                item.status = status.clone();
            }
            check_item_matches_query(&item, query).then_some(item)
        })
        .collect()
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

    #[test]
    fn check_result_builds_src_dst_paths() {
        let item = CheckResult {
            name: "Photos/a.jpg".into(),
            status: "missing_dst".into(),
            src_fs: "drive:".into(),
            dst_fs: "/tmp/out".into(),
            job_id: None,
            completed_at: None,
        };
        assert_eq!(
            crate::transfers::join_fs_name(&item.src_fs, &item.name),
            "drive:Photos/a.jpg"
        );
        assert_eq!(
            crate::transfers::join_fs_name(&item.dst_fs, &item.name),
            "/tmp/out/Photos/a.jpg"
        );
        assert_eq!(item.resolve_kind(), Some("copy_src_to_dst"));
    }

    #[test]
    fn filters_and_overrides_check_results() {
        let items = vec![
            CheckResult {
                name: "keep.txt".into(),
                status: "differ".into(),
                src_fs: "src:".into(),
                dst_fs: "dst:".into(),
                job_id: Some(9),
                completed_at: None,
            },
            CheckResult {
                name: "gone.txt".into(),
                status: "missing_dst".into(),
                src_fs: "src:".into(),
                dst_fs: "dst:".into(),
                job_id: Some(9),
                completed_at: None,
            },
            CheckResult {
                name: "other.bin".into(),
                status: "checked".into(),
                src_fs: "src:".into(),
                dst_fs: "dst:".into(),
                job_id: Some(9),
                completed_at: None,
            },
        ];
        let mut hidden = HashSet::new();
        hidden.insert(check_unique_id(Some(9), "gone.txt"));
        let mut overrides = HashMap::new();
        overrides.insert(check_unique_id(Some(9), "keep.txt"), "missing_src".into());
        let visible = visible_check_items(items, &hidden, &overrides, "keep");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].status, "missing_src");
        assert!(check_item_matches_query(&visible[0], "missing"));
        assert_eq!(
            check_delete_outcome("differ", true),
            CheckDeleteOutcome::Override("missing_src")
        );
        assert_eq!(
            check_delete_outcome("missing_dst", false),
            CheckDeleteOutcome::Hide
        );
        assert_eq!(with_job_id(visible[0].clone(), 12).job_id, Some(12));
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-25T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let hour_ago = now - chrono::Duration::minutes(90);
        assert_eq!(
            relative_time_parts(hour_ago, now),
            ("shared.transferActivity.time.hoursAgo", 1)
        );
        assert_eq!(
            check_status_key("missing_dst"),
            "shared.transferActivity.status.missingDst"
        );
        assert_eq!(check_status_icon("checked"), "emblem-ok-symbolic");
        assert_eq!(check_status_icon("missing_dst"), "dialog-warning-symbolic");
        assert_eq!(resolve_icon("missing_src"), "go-previous-symbolic");
        assert_eq!(resolve_icon("partial"), "go-next-symbolic");
        assert!(resolve_is_preparing(0.0, 0));
        assert!(!resolve_is_preparing(0.4, 0));
        assert_eq!(
            format_resolve_progress(512, 1024, 0.0, 12),
            "512 B / 1.0 KiB · ETA 12s"
        );
    }
}
