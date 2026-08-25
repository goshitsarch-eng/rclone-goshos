//! Structured log parsing for rclone.log, store lines, and JSON rclone logs.

use chrono::{Local, TimeZone, Utc};
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Notice,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERROR",
            Self::Warn => "WARN",
            Self::Notice => "NOTICE",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "error" | "err" | "critical" | "fatal" => Self::Error,
            "warn" | "warning" => Self::Warn,
            "notice" => Self::Notice,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Info,
        }
    }

    pub fn matches_filter(self, filter: &str) -> bool {
        if filter.is_empty() {
            return true;
        }
        self.as_str().eq_ignore_ascii_case(filter)
            || matches!(
                (self, filter.to_ascii_lowercase().as_str()),
                (Self::Warn, "warn" | "warning") | (Self::Notice, "notice")
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: String,
    pub remote_name: Option<String>,
    pub level: LogLevel,
    pub message: String,
    pub context: Option<String>,
    pub operation: Option<String>,
    pub raw: String,
}

impl LogEntry {
    pub fn formatted(&self) -> String {
        let mut text = format!(
            "[{}] [{}] {}",
            self.timestamp,
            self.level.as_str(),
            self.message
        );
        if let Some(remote) = &self.remote_name {
            if !remote.is_empty() {
                text.push_str(&format!("\nRemote: {remote}"));
            }
        }
        if let Some(op) = &self.operation {
            text.push_str(&format!("\nOperation: {op}"));
        }
        if let Some(ctx) = &self.context {
            text.push_str("\n\nDetails:\n");
            text.push_str(ctx);
        }
        text
    }

    pub fn search_haystack(&self) -> String {
        let mut hay = format!(
            "{} {} {}",
            self.message.to_ascii_lowercase(),
            self.timestamp.to_ascii_lowercase(),
            self.level.as_str().to_ascii_lowercase()
        );
        if let Some(remote) = &self.remote_name {
            hay.push(' ');
            hay.push_str(&remote.to_ascii_lowercase());
        }
        if let Some(ctx) = &self.context {
            hay.push(' ');
            hay.push_str(&ctx.to_ascii_lowercase());
        }
        hay
    }
}

pub fn format_now(
    level: LogLevel,
    remote: Option<&str>,
    message: &str,
    context: Option<&str>,
) -> String {
    let ts = Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    let mut line = format!("[{ts} [{}]] {message}", level.as_str());
    if let Some(remote) = remote.filter(|r| !r.is_empty()) {
        line.push_str(&format!(" remote={remote}"));
    }
    if let Some(ctx) = context.filter(|c| !c.is_empty()) {
        line.push_str(&format!(" details={ctx}"));
    }
    line
}

pub fn parse_line(line: &str) -> LogEntry {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LogEntry {
            timestamp: String::new(),
            remote_name: None,
            level: LogLevel::Info,
            message: String::new(),
            context: None,
            operation: None,
            raw: line.to_string(),
        };
    }
    if trimmed.starts_with('{') {
        if let Some(entry) = parse_json_line(trimmed) {
            return entry;
        }
    }
    if let Some(entry) = parse_rclone_text(trimmed) {
        return entry;
    }
    if let Some(entry) = parse_app_bracket(trimmed) {
        return entry;
    }
    LogEntry {
        timestamp: String::new(),
        remote_name: infer_remote(trimmed),
        level: infer_level_from_text(trimmed),
        message: trimmed.to_string(),
        context: None,
        operation: infer_operation(trimmed),
        raw: line.to_string(),
    }
}

fn parse_json_line(line: &str) -> Option<LogEntry> {
    let value: Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    let level = LogLevel::parse(obj.get("level").and_then(|v| v.as_str()).unwrap_or("info"));
    let message = obj
        .get("msg")
        .or_else(|| obj.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or(line)
        .to_string();
    let timestamp = obj
        .get("time")
        .or_else(|| obj.get("timestamp"))
        .and_then(|v| v.as_str())
        .map(normalize_timestamp)
        .unwrap_or_default();
    let remote = obj
        .get("object")
        .or_else(|| obj.get("remote"))
        .and_then(|v| v.as_str())
        .map(remote_from_fs)
        .or_else(|| infer_remote(&message));
    let operation = obj
        .get("operation")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| infer_operation(&message));
    let context = leftover_context(obj, &["level", "msg", "message", "time", "timestamp"]);
    Some(LogEntry {
        timestamp,
        remote_name: remote,
        level,
        message,
        context,
        operation,
        raw: line.to_string(),
    })
}

fn leftover_context(obj: &Map<String, Value>, skip: &[&str]) -> Option<String> {
    let extra: Map<String, Value> = obj
        .iter()
        .filter(|(k, v)| !skip.contains(&k.as_str()) && !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if extra.is_empty() {
        return None;
    }
    serde_json::to_string_pretty(&Value::Object(extra)).ok()
}

fn parse_rclone_text(line: &str) -> Option<LogEntry> {
    let (ts, rest) = split_rclone_timestamp(line)?;
    let rest = rest.trim_start();
    let (level_raw, message) = rest.split_once(':')?;
    let level = LogLevel::parse(level_raw);
    let message = message.trim();
    let (message, details) = split_details(message);
    Some(LogEntry {
        timestamp: ts,
        remote_name: infer_remote(message),
        level,
        message: message.to_string(),
        context: details,
        operation: infer_operation(message),
        raw: line.to_string(),
    })
}

fn parse_app_bracket(line: &str) -> Option<LogEntry> {
    // [2024/01/15 12:34:56 [INFO]] started sync 12 remote=gdrive
    let rest = line.strip_prefix('[')?;
    let (ts, after_ts) = rest.split_once(" [")?;
    let (level_raw, after_level) = after_ts.split_once("]] ")?;
    let level = LogLevel::parse(level_raw.trim_end_matches(']'));
    let (message, details) = split_details(after_level);
    let remote = after_level
        .split_whitespace()
        .find_map(|part| part.strip_prefix("remote="))
        .map(|s| s.to_string())
        .or_else(|| infer_remote(message));
    Some(LogEntry {
        timestamp: ts.trim().to_string(),
        remote_name: remote,
        level,
        message: message
            .split(" remote=")
            .next()
            .unwrap_or(message)
            .trim()
            .to_string(),
        context: details,
        operation: infer_operation(message),
        raw: line.to_string(),
    })
}

fn split_rclone_timestamp(line: &str) -> Option<(String, &str)> {
    let bytes = line.as_bytes();
    // YYYY/MM/DD HH:MM:SS
    if bytes.len() < 19 {
        return None;
    }
    let date = &line[..10];
    if date.as_bytes().get(4) != Some(&b'/') || date.as_bytes().get(7) != Some(&b'/') {
        return None;
    }
    if line.as_bytes().get(10) != Some(&b' ') {
        return None;
    }
    let time = line.get(11..19)?;
    if time.as_bytes().get(2) != Some(&b':') || time.as_bytes().get(5) != Some(&b':') {
        return None;
    }
    let mut end = 19;
    if line.as_bytes().get(19) == Some(&b'.') {
        end = 19
            + line[20..]
                .find(|c: char| !c.is_ascii_digit())
                .map(|i| i + 1)
                .unwrap_or(line.len() - 19);
    }
    Some((line[..end].to_string(), line.get(end..).unwrap_or("")))
}

fn split_details(message: &str) -> (&str, Option<String>) {
    if let Some((msg, details)) = message.split_once(" details=") {
        (msg.trim(), Some(details.trim().to_string()))
    } else {
        (message, None)
    }
}

fn infer_level_from_text(text: &str) -> LogLevel {
    let lower = text.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("failed") || lower.contains("fatal") {
        LogLevel::Error
    } else if lower.contains("warn") {
        LogLevel::Warn
    } else if lower.contains("debug") {
        LogLevel::Debug
    } else {
        LogLevel::Info
    }
}

pub fn infer_remote(message: &str) -> Option<String> {
    if let Some(rest) = message.strip_prefix("Failed to create file system for \"") {
        let name = rest.split('"').next().unwrap_or("");
        return Some(remote_from_fs(name));
    }
    let token = message.split_whitespace().next().unwrap_or("");
    if token.contains(':') && !token.contains("://") && !token.starts_with('/') {
        let name = token.split(':').next().unwrap_or("");
        if !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Some(name.to_string());
        }
    }
    None
}

fn remote_from_fs(fs: &str) -> String {
    fs.split(':').next().unwrap_or(fs).trim().to_string()
}

fn infer_operation(message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    for op in [
        "mount",
        "unmount",
        "sync",
        "copy",
        "move",
        "bisync",
        "serve",
        "check",
        "cryptcheck",
        "delete",
        "copyurl",
        "archive",
    ] {
        if lower.contains(op) {
            return Some(op.to_string());
        }
    }
    None
}

fn normalize_timestamp(raw: &str) -> String {
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) {
        return parsed
            .with_timezone(&Local)
            .format("%Y/%m/%d %H:%M:%S")
            .to_string();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f") {
        if let Some(dt) = Utc.from_local_datetime(&naive).single() {
            return dt
                .with_timezone(&Local)
                .format("%Y/%m/%d %H:%M:%S")
                .to_string();
        }
    }
    raw.to_string()
}

pub fn matches_remote(entry: &LogEntry, remote: Option<&str>) -> bool {
    let Some(wanted) = remote.filter(|r| !r.is_empty() && *r != "_engine") else {
        return true;
    };
    entry
        .remote_name
        .as_deref()
        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        || entry
            .message
            .to_ascii_lowercase()
            .contains(&wanted.to_ascii_lowercase())
        || entry
            .raw
            .to_ascii_lowercase()
            .contains(&wanted.to_ascii_lowercase())
}

pub fn filter_entries<'a>(
    entries: &'a [LogEntry],
    query: &str,
    level: &str,
    remote: Option<&str>,
) -> Vec<&'a LogEntry> {
    let q = query.trim().to_ascii_lowercase();
    entries
        .iter()
        .filter(|entry| {
            entry.level.matches_filter(level)
                && matches_remote(entry, remote)
                && (q.is_empty() || entry.search_haystack().contains(&q))
        })
        .collect()
}

pub fn collect_entries(
    store: &std::collections::HashMap<String, Vec<String>>,
    file_text: &str,
    remote: Option<&str>,
) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for (key, lines) in store {
        if let Some(wanted) = remote.filter(|r| !r.is_empty() && *r != "_engine") {
            if key != wanted && key != "_engine" {
                continue;
            }
        }
        for line in lines {
            push_unique(&mut entries, &mut seen, annotate_store_line(key, line));
        }
    }
    for line in file_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        push_unique(&mut entries, &mut seen, parse_line(line));
    }
    entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    if let Some(wanted) = remote.filter(|r| !r.is_empty() && *r != "_engine") {
        entries.retain(|e| matches_remote(e, Some(wanted)));
    }
    entries
}

fn annotate_store_line(key: &str, line: &str) -> LogEntry {
    let mut entry = parse_line(line);
    if entry.remote_name.is_none() && key != "_engine" {
        entry.remote_name = Some(key.to_string());
    }
    entry
}

fn push_unique(entries: &mut Vec<LogEntry>, seen: &mut HashSet<String>, entry: LogEntry) {
    let key = format!(
        "{}|{}|{}",
        entry.timestamp,
        entry.level.as_str(),
        entry.message
    );
    if seen.insert(key) {
        entries.push(entry);
    }
}

pub fn export_text(entries: &[&LogEntry]) -> String {
    entries
        .iter()
        .map(|e| e.formatted())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn read_log_file_tail(max_lines: usize) -> String {
    let Ok(file) = std::fs::read_to_string(crate::settings::AppSettings::log_path()) else {
        return String::new();
    };
    let mut tail: Vec<&str> = file.lines().rev().take(max_lines).collect();
    tail.reverse();
    tail.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn parses_rclone_text_line() {
        let entry = parse_line("2024/01/15 12:34:56 ERROR : gdrive: failed to list");
        assert_eq!(entry.level, LogLevel::Error);
        assert_eq!(entry.timestamp, "2024/01/15 12:34:56");
        assert_eq!(entry.remote_name.as_deref(), Some("gdrive"));
        assert!(entry.message.contains("failed to list"));
    }

    #[test]
    fn parses_notice_with_space_before_colon() {
        let entry = parse_line("2024/01/15 12:34:56 NOTICE: Config file not found");
        assert_eq!(entry.level, LogLevel::Notice);
        assert_eq!(entry.message, "Config file not found");
    }

    #[test]
    fn parses_json_rclone_line() {
        let entry = parse_line(
            r#"{"level":"info","msg":"vfs cache: cleaned","time":"2024-01-15T12:34:56Z","object":"box:Photos"}"#,
        );
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.remote_name.as_deref(), Some("box"));
        assert_eq!(entry.message, "vfs cache: cleaned");
        assert!(entry.context.as_deref().unwrap_or("").contains("object"));
    }

    #[test]
    fn parses_app_bracket_line() {
        let entry = parse_line(
            "[2024/01/15 12:34:56 [ERROR]] job 9 failed remote=dropbox details={\"job_id\":9}",
        );
        assert_eq!(entry.level, LogLevel::Error);
        assert_eq!(entry.remote_name.as_deref(), Some("dropbox"));
        assert_eq!(entry.message, "job 9 failed");
        assert_eq!(entry.context.as_deref(), Some(r#"{"job_id":9}"#));
    }

    #[test]
    fn filters_by_level_query_and_remote() {
        let entries = vec![
            parse_line("2024/01/15 12:34:56 ERROR : gdrive: boom"),
            parse_line("2024/01/15 12:34:57 INFO  : local: ok"),
        ];
        let filtered = filter_entries(&entries, "boom", "ERROR", Some("gdrive"));
        assert_eq!(filtered.len(), 1);
        assert!(filter_entries(&entries, "missing", "", None).is_empty());
        assert_eq!(filter_entries(&entries, "", "", None).len(), 2);
    }

    #[test]
    fn collect_merges_store_and_file() {
        let mut store = HashMap::new();
        store.insert(
            "gdrive".into(),
            vec!["[2024/01/15 12:00:00 [INFO]] started sync 1".into()],
        );
        let file = "2024/01/15 12:00:01 NOTICE: gdrive: hello\n2024/01/15 12:00:01 NOTICE: gdrive: hello\n";
        let entries = collect_entries(&store, file, Some("gdrive"));
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.message.contains("started sync")));
        assert!(entries.iter().any(|e| e.message.contains("hello")));
    }

    #[test]
    fn export_includes_details() {
        let entry = parse_line(
            r#"{"level":"error","msg":"copy failed","time":"2024-01-15T12:34:56Z","output":"boom"}"#,
        );
        let text = export_text(&[&entry]);
        assert!(text.contains("copy failed"));
        assert!(text.contains("Details:"));
        assert!(text.contains("boom"));
    }

    #[test]
    fn format_now_embeds_level_and_remote() {
        let line = format_now(LogLevel::Info, Some("drive"), "started copy 3", None);
        let parsed = parse_line(&line);
        assert_eq!(parsed.level, LogLevel::Info);
        assert_eq!(parsed.remote_name.as_deref(), Some("drive"));
        assert!(parsed.message.contains("started copy 3"));
    }

    #[test]
    fn infer_remote_from_fs_error() {
        assert_eq!(
            infer_remote(r#"Failed to create file system for "sftp:inbox""#).as_deref(),
            Some("sftp")
        );
    }
}
