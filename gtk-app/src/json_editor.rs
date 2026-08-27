//! Structured JSON editor helpers — Angular `json-editor.component`.

use crate::config_search::{matches_config_search, normalize_rclone_key};
use crate::operations::OperationType;
use crate::restrict::{is_sensitive_key, redact_value, RESTRICTED_LABEL};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct JsonFieldDef {
    pub key: String,
    pub help: String,
    pub type_name: String,
    pub default: Value,
    pub examples: Vec<(String, String)>,
    pub sensitive: bool,
}

impl JsonFieldDef {
    pub fn from_flag(flag: &crate::flags::FlagOption) -> Self {
        let key = if flag.field_name.is_empty() {
            flag.name.clone()
        } else {
            flag.field_name.clone()
        };
        let default = if flag.default_str.is_empty() {
            flag.value.clone()
        } else {
            json!(flag.default_str)
        };
        Self {
            key,
            help: flag.help.clone(),
            type_name: flag.type_name.clone(),
            default,
            examples: flag.examples.clone(),
            sensitive: is_sensitive_key(&flag.name) || is_sensitive_key(&flag.field_name),
        }
    }

    pub fn from_provider(option: &crate::providers::ProviderOption) -> Self {
        Self {
            key: option.name.clone(),
            help: option.help.clone(),
            type_name: option.type_name.clone(),
            default: if option.default_str.is_empty() {
                option.default.clone()
            } else {
                json!(option.default_str)
            },
            examples: option.examples.clone(),
            sensitive: option.is_password || is_sensitive_key(&option.name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipSpec {
    pub key: String,
    pub display_value: String,
    pub active: bool,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnosis {
    pub error: bool,
    pub i18n_key: &'static str,
    pub params: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonCursorKind {
    Property,
    Value { key: String },
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonCursor {
    pub kind: JsonCursorKind,
    pub prefix: String,
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathRecon {
    pub sources: Option<Vec<String>>,
    pub dest: Option<String>,
    pub mount_type: Option<String>,
    pub serve_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRule {
    pub key: String,
    pub array_ok: bool,
}

pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
}

pub fn parse_object(text: &str) -> Result<Map<String, Value>, Diagnosis> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(Diagnosis {
            error: true,
            i18n_key: "shared.jsonEditor.parseError",
            params: Vec::new(),
        }),
        Err(_) => Err(Diagnosis {
            error: true,
            i18n_key: "shared.jsonEditor.parseError",
            params: Vec::new(),
        }),
    }
}

pub fn display_text(value: &Value, restrict: bool) -> String {
    let shown = if restrict {
        redact_value(value)
    } else {
        value.clone()
    };
    pretty(&shown)
}

pub fn restore_restricted(edited: &str, original: &Value) -> Result<Value, Diagnosis> {
    let mut parsed = serde_json::from_str::<Value>(edited.trim()).map_err(|_| Diagnosis {
        error: true,
        i18n_key: "shared.jsonEditor.parseError",
        params: Vec::new(),
    })?;
    restore_restricted_in(&mut parsed, original);
    Ok(parsed)
}

fn restore_restricted_in(edited: &mut Value, original: &Value) {
    match (edited, original) {
        (Value::Object(edited), Value::Object(original)) => {
            for (key, value) in edited.iter_mut() {
                if is_sensitive_key(key) && value.as_str() == Some(RESTRICTED_LABEL) {
                    if let Some(orig) = original.get(key) {
                        *value = orig.clone();
                    }
                } else if let Some(orig) = original.get(key) {
                    restore_restricted_in(value, orig);
                }
            }
        }
        _ => {}
    }
}

pub fn cli_to_snake(key: &str) -> String {
    key.trim()
        .trim_start_matches('-')
        .replace('-', "_")
        .to_string()
}

pub fn suggest_key(key: &str, fields: &[JsonFieldDef]) -> Option<String> {
    let norm = normalize_rclone_key(&cli_to_snake(key));
    if norm.is_empty() {
        return None;
    }
    fields
        .iter()
        .find(|field| normalize_rclone_key(&field.key) == norm)
        .map(|field| field.key.clone())
}

pub fn chips(
    fields: &[JsonFieldDef],
    value: &Value,
    explicit: &HashSet<String>,
    query: &str,
    restrict: bool,
) -> Vec<ChipSpec> {
    let obj = value.as_object();
    fields
        .iter()
        .filter(|field| matches_config_search(&field.key, &field.help, &field.key, query))
        .map(|field| {
            let current = obj.and_then(|map| map.get(&field.key));
            let changed = current.is_some_and(|v| v != &field.default && !v.is_null());
            let active = current.is_some() || explicit.contains(&field.key);
            let raw = match current {
                Some(_) if restrict && field.sensitive => "••••••••".into(),
                Some(Value::Array(items)) => items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                Some(Value::String(s)) => s.clone(),
                Some(other) if !other.is_null() => other.to_string(),
                _ => match &field.default {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                },
            };
            let display_value = if raw.chars().count() > 20 {
                format!("{}…", raw.chars().take(18).collect::<String>())
            } else {
                raw
            };
            ChipSpec {
                key: field.key.clone(),
                display_value,
                active,
                changed,
            }
        })
        .collect()
}

pub fn toggle_chip(json: &str, key: &str, default: &Value) -> Result<String, Diagnosis> {
    let mut map = parse_object(json)?;
    if map.contains_key(key) {
        map.remove(key);
    } else {
        let value = if default.is_null() {
            json!("")
        } else {
            default.clone()
        };
        map.insert(key.to_string(), value);
    }
    Ok(pretty(&Value::Object(map)))
}

pub fn structural_keys(op: Option<OperationType>) -> Vec<String> {
    match op {
        Some(OperationType::Mount) => vec![
            "fs".into(),
            "srcFs".into(),
            "mountPoint".into(),
            "mountType".into(),
        ],
        Some(OperationType::Serve) => vec!["fs".into(), "type".into(), "addr".into()],
        Some(OperationType::Bisync) => {
            vec![
                "path1".into(),
                "path2".into(),
                "srcFs".into(),
                "dstFs".into(),
            ]
        }
        Some(OperationType::Copyurl) => vec![
            "url".into(),
            "srcFs".into(),
            "dstFs".into(),
            "fs".into(),
            "filenames".into(),
        ],
        Some(OperationType::Delete) => vec!["srcFs".into()],
        Some(_) => vec!["srcFs".into(), "dstFs".into()],
        None => Vec::new(),
    }
}

pub fn path_rules(op: Option<OperationType>) -> Vec<PathRule> {
    let Some(op) = op else {
        return Vec::new();
    };
    let source = PathRule {
        key: source_key(op).into(),
        array_ok: op.supports_multi_source(),
    };
    let dest = PathRule {
        key: dest_key(op).into(),
        array_ok: false,
    };
    if dest.key.is_empty() {
        vec![source]
    } else {
        vec![source, dest]
    }
}

pub fn source_key(op: OperationType) -> &'static str {
    match op {
        OperationType::Mount | OperationType::Serve => "fs",
        OperationType::Bisync => "path1",
        OperationType::Copyurl => "url",
        _ => "srcFs",
    }
}

pub fn dest_key(op: OperationType) -> &'static str {
    match op {
        OperationType::Mount => "mountPoint",
        OperationType::Serve => "addr",
        OperationType::Bisync => "path2",
        OperationType::Delete => "",
        _ => "dstFs",
    }
}

pub fn diagnose(
    obj: &Map<String, Value>,
    fields: &[JsonFieldDef],
    structural: &[String],
    rules: &[PathRule],
) -> Option<Diagnosis> {
    for rule in rules {
        if !rule.array_ok {
            if let Some(Value::Array(_)) = obj.get(&rule.key) {
                return Some(Diagnosis {
                    error: true,
                    i18n_key: "shared.jsonEditor.invalidArrayPath",
                    params: vec![("key".into(), rule.key.clone())],
                });
            }
        }
    }
    let known: HashSet<&str> = fields
        .iter()
        .map(|f| f.key.as_str())
        .chain(structural.iter().map(String::as_str))
        .collect();
    let mut unknown = Vec::new();
    for key in obj.keys() {
        if known.contains(key.as_str()) {
            continue;
        }
        if key.starts_with('-') {
            let suggestion = suggest_key(key, fields).unwrap_or_else(|| cli_to_snake(key));
            return Some(Diagnosis {
                error: false,
                i18n_key: "shared.jsonEditor.cliArgumentWithSuggestion",
                params: vec![
                    ("key".into(), key.clone()),
                    ("suggestion".into(), suggestion),
                ],
            });
        }
        if let Some(suggestion) = suggest_key(key, fields) {
            if suggestion != *key {
                return Some(Diagnosis {
                    error: false,
                    i18n_key: "shared.jsonEditor.camelCaseSuggestionWarning",
                    params: vec![
                        ("key".into(), key.clone()),
                        ("suggestion".into(), suggestion),
                    ],
                });
            }
        }
        unknown.push(key.clone());
    }
    if unknown.is_empty() {
        None
    } else {
        Some(Diagnosis {
            error: false,
            i18n_key: "shared.jsonEditor.unknownWarning",
            params: vec![(
                "keys".into(),
                unknown
                    .iter()
                    .map(|k| format!("'{k}'"))
                    .collect::<Vec<_>>()
                    .join(", "),
            )],
        })
    }
}

pub fn complete_keys(
    prefix: &str,
    fields: &[JsonFieldDef],
    structural: &[String],
    already: &[String],
) -> Vec<Completion> {
    let term = prefix.trim().trim_matches('"').to_ascii_lowercase();
    let taken: HashSet<&str> = already.iter().map(String::as_str).collect();
    let mut out = Vec::new();
    for key in structural {
        if taken.contains(key.as_str()) {
            continue;
        }
        if term.is_empty() || key.to_ascii_lowercase().contains(&term) {
            out.push(Completion {
                label: key.clone(),
                detail: "Top-Level key".into(),
            });
        }
    }
    for field in fields {
        if taken.contains(field.key.as_str()) {
            continue;
        }
        if term.is_empty()
            || field.key.to_ascii_lowercase().contains(&term)
            || field.help.to_ascii_lowercase().contains(&term)
        {
            out.push(Completion {
                label: field.key.clone(),
                detail: if field.type_name.is_empty() {
                    "Option".into()
                } else {
                    field.type_name.clone()
                },
            });
        }
    }
    out.truncate(40);
    out
}

pub fn complete_values(key: &str, prefix: &str, fields: &[JsonFieldDef]) -> Vec<Completion> {
    let term = prefix.trim().trim_matches('"').to_ascii_lowercase();
    let Some(field) = fields.iter().find(|f| f.key == key) else {
        return Vec::new();
    };
    field
        .examples
        .iter()
        .filter(|(value, help)| {
            term.is_empty()
                || value.to_ascii_lowercase().contains(&term)
                || help.to_ascii_lowercase().contains(&term)
        })
        .take(40)
        .map(|(value, help)| Completion {
            label: value.clone(),
            detail: help.clone(),
        })
        .collect()
}

pub fn cursor_at(text: &str, cursor: usize) -> JsonCursor {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let quote_count = before.matches('"').count() - escaped_quote_pairs(before);
    if quote_count % 2 == 0 {
        return JsonCursor {
            kind: JsonCursorKind::None,
            prefix: String::new(),
            from: cursor,
            to: cursor,
        };
    }
    let from = before.rfind('"').unwrap_or(0);
    let after = &text[cursor..];
    let to = after
        .find('"')
        .map(|idx| cursor + idx)
        .unwrap_or(text.len());
    let prefix = text[from + 1..cursor].to_string();
    let prior = text[..from].trim_end();
    let kind = if prior.ends_with(':') {
        JsonCursorKind::Value {
            key: property_key_before(prior),
        }
    } else {
        JsonCursorKind::Property
    };
    JsonCursor {
        kind,
        prefix,
        from: from + 1,
        to,
    }
}

fn escaped_quote_pairs(text: &str) -> usize {
    text.matches("\\\"").count()
}

fn property_key_before(prior: &str) -> String {
    let stripped = prior.trim_end().trim_end_matches(':').trim_end();
    if let Some(end) = stripped.rfind('"') {
        let head = &stripped[..end];
        if let Some(start) = head.rfind('"') {
            return stripped[start + 1..end].to_string();
        }
    }
    String::new()
}

pub fn complete_at(
    text: &str,
    cursor: usize,
    fields: &[JsonFieldDef],
    structural: &[String],
) -> Vec<Completion> {
    let cursor_info = cursor_at(text, cursor);
    match cursor_info.kind {
        JsonCursorKind::Property => {
            let already: Vec<String> = parse_object(text)
                .map(|map| map.keys().cloned().collect())
                .unwrap_or_default();
            complete_keys(&cursor_info.prefix, fields, structural, &already)
        }
        JsonCursorKind::Value { key } => complete_values(&key, &cursor_info.prefix, fields),
        JsonCursorKind::None => Vec::new(),
    }
}

pub fn reconcile_paths(obj: &Map<String, Value>, op: Option<OperationType>) -> PathRecon {
    let mut recon = PathRecon::default();
    if let Some(op) = op {
        let source = source_key(op);
        if let Some(value) = obj.get(source).or_else(|| {
            obj.get("srcFs")
                .or_else(|| obj.get("fs"))
                .or_else(|| obj.get("url"))
        }) {
            recon.sources = Some(value_paths(value));
        }
        let dest = dest_key(op);
        if !dest.is_empty() {
            if let Some(value) = obj.get(dest).or_else(|| {
                obj.get("dstFs")
                    .or_else(|| obj.get("mountPoint"))
                    .or_else(|| obj.get("addr"))
            }) {
                recon.dest = value_paths(value).into_iter().next();
            }
        }
        if op == OperationType::Mount {
            recon.mount_type = obj
                .get("mountType")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
        if op == OperationType::Serve {
            recon.serve_type = obj
                .get("type")
                .and_then(Value::as_str)
                .map(ToString::to_string);
        }
    } else {
        if let Some(value) = obj.get("srcFs").or_else(|| obj.get("fs")) {
            recon.sources = Some(value_paths(value));
        }
        if let Some(value) = obj.get("dstFs").or_else(|| obj.get("mountPoint")) {
            recon.dest = value_paths(value).into_iter().next();
        }
    }
    recon
}

fn value_paths(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn info_banner_key(op: Option<OperationType>, helper: Option<&str>) -> Option<&'static str> {
    if let Some(kind) = helper {
        return Some(match kind {
            "vfs" => "wizards.remoteConfig.jsonEditorInfo.vfs",
            "filter" => "wizards.remoteConfig.jsonEditorInfo.filter",
            "backend" => "wizards.remoteConfig.jsonEditorInfo.backend",
            "runtime" | "runtime_remote" => "wizards.remoteConfig.jsonEditorInfo.runtimeRemote",
            _ => return None,
        });
    }
    Some(match op? {
        OperationType::Sync | OperationType::Copy | OperationType::Move => {
            "wizards.remoteConfig.jsonEditorInfo.sync"
        }
        OperationType::Bisync => "wizards.remoteConfig.jsonEditorInfo.bisync",
        OperationType::Check | OperationType::Cryptcheck => {
            "wizards.remoteConfig.jsonEditorInfo.check"
        }
        OperationType::Mount => "wizards.remoteConfig.jsonEditorInfo.mount",
        OperationType::Serve => "wizards.remoteConfig.jsonEditorInfo.serve",
        _ => return None,
    })
}

pub fn highlight_spans(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    for key in crate::config_search::filter_json_keys(&value, query) {
        let needle = format!("\"{key}\"");
        let mut search_from = 0;
        while let Some(pos) = text[search_from..].find(&needle) {
            let start = search_from + pos;
            spans.push((start, start + needle.len()));
            search_from = start + needle.len();
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(key: &str, help: &str) -> JsonFieldDef {
        JsonFieldDef {
            key: key.into(),
            help: help.into(),
            type_name: "string".into(),
            default: json!(""),
            examples: vec![("AWS".into(), "Amazon".into())],
            sensitive: key.contains("token"),
        }
    }

    #[test]
    fn parses_objects_and_rejects_arrays() {
        assert!(parse_object("{\"a\":1}").unwrap().contains_key("a"));
        assert!(parse_object("").unwrap().is_empty());
        assert!(parse_object("[1]").unwrap_err().error);
        assert!(parse_object("{").unwrap_err().error);
    }

    #[test]
    fn toggles_chip_presence() {
        let added = toggle_chip("{}", "chunk_size", &json!("5Mi")).unwrap();
        assert!(added.contains("chunk_size"));
        let removed = toggle_chip(&added, "chunk_size", &json!("5Mi")).unwrap();
        assert!(!removed.contains("chunk_size"));
    }

    #[test]
    fn chips_mark_active_and_mask_secrets() {
        let fields = vec![field("token", "OAuth"), field("acl", "Access")];
        let value = json!({ "token": "secret", "acl": "private" });
        let specs = chips(&fields, &value, &HashSet::new(), "", true);
        assert!(specs
            .iter()
            .any(|c| c.key == "token" && c.display_value == "••••••••"));
        assert!(specs.iter().any(|c| c.key == "acl" && c.active));
        let filtered = chips(&fields, &value, &HashSet::new(), "acl", false);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].key, "acl");
    }

    #[test]
    fn diagnoses_cli_unknown_and_array_paths() {
        let fields = vec![field("transfers", "Parallel transfers")];
        let structural = structural_keys(Some(OperationType::Copy));
        let rules = path_rules(Some(OperationType::Copy));
        let cli = diagnose(
            &parse_object("{\"--transfers\": 4}").unwrap(),
            &fields,
            &structural,
            &rules,
        )
        .unwrap();
        assert!(!cli.error);
        assert_eq!(cli.i18n_key, "shared.jsonEditor.cliArgumentWithSuggestion");
        assert_eq!(cli.params[1].1, "transfers");

        let camel = diagnose(
            &parse_object("{\"Transfers\": 4}").unwrap(),
            &fields,
            &structural,
            &rules,
        )
        .unwrap();
        assert_eq!(
            camel.i18n_key,
            "shared.jsonEditor.camelCaseSuggestionWarning"
        );

        let unknown = diagnose(
            &parse_object("{\"nope\": true}").unwrap(),
            &fields,
            &structural,
            &rules,
        )
        .unwrap();
        assert_eq!(unknown.i18n_key, "shared.jsonEditor.unknownWarning");

        let array = diagnose(
            &parse_object("{\"dstFs\": [\"/tmp/a\", \"/tmp/b\"]}").unwrap(),
            &fields,
            &structural,
            &rules,
        )
        .unwrap();
        assert!(array.error);
        assert_eq!(array.i18n_key, "shared.jsonEditor.invalidArrayPath");

        let multi_ok = diagnose(
            &parse_object("{\"srcFs\": [\"a:\", \"b:\"]}").unwrap(),
            &fields,
            &structural,
            &rules,
        );
        assert!(multi_ok.is_none());
    }

    #[test]
    fn completes_keys_and_values() {
        let fields = vec![field("provider", "Cloud vendor")];
        let keys = complete_keys("pro", &fields, &["srcFs".into()], &[]);
        assert!(keys.iter().any(|c| c.label == "provider"));
        assert!(!keys.iter().any(|c| c.label == "srcFs"));
        let all = complete_keys("", &fields, &["srcFs".into()], &[]);
        assert!(all.iter().any(|c| c.label == "srcFs"));
        let values = complete_values("provider", "aw", &fields);
        assert_eq!(values[0].label, "AWS");
    }

    #[test]
    fn cursor_detects_property_and_value() {
        let text = "{\n  \"pro\": \"AW\"\n}";
        let prop = cursor_at(text, text.find("pro").unwrap() + 2);
        assert_eq!(prop.kind, JsonCursorKind::Property);
        assert_eq!(prop.prefix, "pr");
        let value_at = text.find("AW").unwrap() + 1;
        let val = cursor_at(text, value_at);
        match val.kind {
            JsonCursorKind::Value { key } => assert_eq!(key, "pro"),
            other => panic!("{other:?}"),
        }
        assert_eq!(val.prefix, "A");
    }

    #[test]
    fn restores_restricted_secrets() {
        let original = json!({ "token": "real-secret", "acl": "private" });
        let edited = display_text(&original, true);
        assert!(edited.contains(RESTRICTED_LABEL));
        let restored = restore_restricted(&edited, &original).unwrap();
        assert_eq!(restored["token"], "real-secret");
        assert_eq!(restored["acl"], "private");
    }

    #[test]
    fn reconciles_copy_and_mount_paths() {
        let copy = reconcile_paths(
            &parse_object("{\"srcFs\":[\"testdrive:Photos\",\"testdrive:\"],\"dstFs\":\"/tmp\"}")
                .unwrap(),
            Some(OperationType::Copy),
        );
        assert_eq!(
            copy.sources.unwrap(),
            vec!["testdrive:Photos".to_string(), "testdrive:".into()]
        );
        assert_eq!(copy.dest.as_deref(), Some("/tmp"));

        let mount = reconcile_paths(
            &parse_object(
                "{\"fs\":\"testdrive:\",\"mountPoint\":\"/mnt\",\"mountType\":\"mount\"}",
            )
            .unwrap(),
            Some(OperationType::Mount),
        );
        assert_eq!(mount.sources.unwrap(), vec!["testdrive:".to_string()]);
        assert_eq!(mount.dest.as_deref(), Some("/mnt"));
        assert_eq!(mount.mount_type.as_deref(), Some("mount"));
    }

    #[test]
    fn highlight_spans_find_matching_keys() {
        let text = "{\n  \"chunk_size\": \"5Mi\",\n  \"acl\": \"private\"\n}";
        let hits = highlight_spans(text, "chunk");
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].0..hits[0].1], "\"chunk_size\"");
        assert!(highlight_spans(text, "").is_empty());
        assert!(highlight_spans("{", "acl").is_empty());
    }

    #[test]
    fn info_banner_keys_match_angular() {
        assert_eq!(
            info_banner_key(Some(OperationType::Copy), None),
            Some("wizards.remoteConfig.jsonEditorInfo.sync")
        );
        assert_eq!(
            info_banner_key(None, Some("vfs")),
            Some("wizards.remoteConfig.jsonEditorInfo.vfs")
        );
        assert_eq!(info_banner_key(Some(OperationType::Delete), None), None);
    }
}
