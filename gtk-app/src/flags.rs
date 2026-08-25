//! Rclone flag editors — parse `options/info` + `options/get` and apply `options/set`.
//! Group taxonomy matches `src-tauri/src/rclone/queries/flags.rs`.

use crate::operations::OperationType;
use serde_json::{json, Map, Value};

const BACKEND_INCLUDE: &[&str] = &[
    "Performance",
    "Networking",
    "Config",
    "Logging",
    "Debugging",
    "Listing",
    "Important",
    "Metadata",
];
const BACKEND_EXCLUDE: &[&str] = &["Copy", "Sync", "Filter", "Mount", "VFS", "RC", "WebDAV"];
const COPY_GROUPS: &[&str] = &["Copy"];
const SYNC_GROUPS: &[&str] = &["Copy", "Sync"];
const CHECK_GROUPS: &[&str] = &["Check"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagOption {
    pub name: String,
    pub field_name: String,
    pub help: String,
    pub type_name: String,
    pub advanced: bool,
    pub groups: String,
    pub default_str: String,
    pub value: Value,
    pub examples: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagBlock {
    pub name: String,
    pub options: Vec<FlagOption>,
}

pub fn flag_has_any_group(groups: &str, set: &[&str]) -> bool {
    groups
        .split(',')
        .map(str::trim)
        .any(|g| set.iter().any(|wanted| *wanted == g))
}

pub fn classify_flag(groups: &str) -> &'static str {
    if flag_has_any_group(groups, &["Filter"]) {
        "filter"
    } else if flag_has_any_group(groups, &["VFS"]) {
        "vfs"
    } else if flag_has_any_group(groups, &["Mount"]) {
        "mount"
    } else if flag_has_any_group(groups, SYNC_GROUPS) && flag_has_any_group(groups, &["Sync"]) {
        "sync"
    } else if flag_has_any_group(groups, COPY_GROUPS) {
        "copy"
    } else if flag_has_any_group(groups, CHECK_GROUPS) {
        "check"
    } else if flag_has_any_group(groups, BACKEND_EXCLUDE) {
        "other"
    } else if flag_has_any_group(groups, BACKEND_INCLUDE) || groups.is_empty() {
        "backend"
    } else {
        "other"
    }
}

pub fn parse_options_info(value: &Value) -> Vec<FlagBlock> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let mut blocks = Vec::new();
    for (name, block) in obj {
        let options = match block.as_array() {
            Some(arr) => arr.iter().filter_map(parse_flag_option).collect(),
            None => Vec::new(),
        };
        if !options.is_empty() {
            blocks.push(FlagBlock {
                name: name.clone(),
                options,
            });
        }
    }
    blocks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    blocks
}

fn parse_flag_option(value: &Value) -> Option<FlagOption> {
    let name = value
        .get("Name")
        .or_else(|| value.get("name"))
        .and_then(|x| x.as_str())?
        .to_string();
    let field_name = value
        .get("FieldName")
        .or_else(|| value.get("fieldName"))
        .and_then(|x| x.as_str())
        .unwrap_or(&name)
        .to_string();
    Some(FlagOption {
        help: value
            .get("Help")
            .or_else(|| value.get("help"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        type_name: value
            .get("Type")
            .or_else(|| value.get("type"))
            .and_then(|x| x.as_str())
            .unwrap_or("string")
            .to_string(),
        advanced: value
            .get("Advanced")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        groups: value
            .get("Groups")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        default_str: value
            .get("DefaultStr")
            .or_else(|| value.get("Default"))
            .map(|x| match x {
                Value::String(s) => s.clone(),
                other => other.to_string().trim_matches('"').to_string(),
            })
            .unwrap_or_default(),
        value: value.get("Value").cloned().unwrap_or(Value::Null),
        examples: value
            .get("Examples")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|ex| {
                        Some((
                            ex.get("Value")?.as_str()?.to_string(),
                            ex.get("Help")
                                .and_then(|h| h.as_str())
                                .unwrap_or("")
                                .to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        name,
        field_name,
    })
}

pub fn merge_current_values(blocks: &mut [FlagBlock], current: &Value) {
    for block in blocks.iter_mut() {
        let Some(current_block) = current.get(&block.name) else {
            continue;
        };
        for option in &mut block.options {
            let mut node = current_block;
            for part in option.field_name.split('.') {
                if node.is_null() {
                    break;
                }
                node = &node[part];
            }
            if !node.is_null() {
                option.value = node.clone();
            }
        }
    }
}

pub fn set_option_payload(block: &str, option_name: &str, value: Value) -> Value {
    let nested = option_name
        .split('.')
        .rev()
        .fold(value, |acc, part| json!({ part: acc }));
    json!({ block: nested })
}

pub fn parse_flag_value(type_name: &str, text: &str) -> Value {
    match type_name {
        "bool" => json!(text.eq_ignore_ascii_case("true") || text == "1"),
        "int" | "int64" | "SizeSuffix" => text
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| json!(text)),
        "float" | "Duration" => text
            .parse::<f64>()
            .map(|n| json!(n))
            .unwrap_or_else(|_| json!(text)),
        _ => json!(text),
    }
}

pub fn static_flags_for(op: OperationType) -> Vec<FlagOption> {
    match op {
        OperationType::Move => vec![
            bool_flag(
                "createEmptySrcDirs",
                "Create empty source directories on destination after move.",
            ),
            bool_flag(
                "deleteEmptySrcDirs",
                "Delete empty source directories after move.",
            ),
        ],
        OperationType::Copy => vec![bool_flag(
            "createEmptySrcDirs",
            "Create empty source directories on destination after copy.",
        )],
        OperationType::Sync => vec![bool_flag(
            "createEmptySrcDirs",
            "Create empty source directories on destination after sync.",
        )],
        OperationType::Check | OperationType::Cryptcheck => vec![
            bool_flag(
                "oneWay",
                "Do check one way only — find files on source which don't exist on destination.",
            ),
            bool_flag("download", "Check by downloading rather than with hash."),
            string_flag("checkFileHash", "Hash type of the SUM file."),
            string_flag("checkFileFs", "Fs of the SUM file."),
            string_flag("checkFileRemote", "Remote of the SUM file."),
            bool_flag("combined", "Make a combined report of changes."),
            bool_flag("missingOnSrc", "Report all files missing from the source."),
            bool_flag(
                "missingOnDst",
                "Report all files missing from the destination.",
            ),
            bool_flag("match", "Report all matching files."),
            bool_flag("differ", "Report all non-matching files."),
            bool_flag("error", "Report all files with errors."),
        ],
        OperationType::Serve => vec![string_flag("type", "Serve type to use.")],
        OperationType::Archivecreate => vec![
            string_flag("format", "Archive format (zip, tar, tgz, tbz, txz, etc.)"),
            string_flag("prefix", "Add prefix directory in the archive"),
            bool_flag("fullPath", "Use full path of files in the archive"),
        ],
        OperationType::Bisync => vec![
            bool_flag("dryRun", "Perform a dry-run."),
            bool_flag("resync", "Performs the resync run."),
            string_flag("resyncMode", "During resync, prefer path1/path2/newer/…"),
            bool_flag("checkAccess", "Abort if RCLONE_TEST files are not found."),
            string_flag("checkFilename", "File name for --check-access."),
            FlagOption {
                name: "maxDelete".into(),
                field_name: "maxDelete".into(),
                help: "Abort sync if percentage of deleted files is above this threshold.".into(),
                type_name: "int".into(),
                advanced: false,
                groups: "Sync".into(),
                default_str: "50".into(),
                value: json!(50),
                examples: vec![],
            },
            bool_flag("force", "Bypass --max-delete safety check."),
            string_flag("compare", "size,modtime,checksum"),
            string_flag("conflictLoser", "num, pathname, or delete"),
            string_flag("conflictResolve", "none, path1, path2, newer, …"),
        ],
        _ => Vec::new(),
    }
}

fn bool_flag(name: &str, help: &str) -> FlagOption {
    FlagOption {
        name: name.into(),
        field_name: name.into(),
        help: help.into(),
        type_name: "bool".into(),
        advanced: false,
        groups: String::new(),
        default_str: "false".into(),
        value: json!(false),
        examples: vec![],
    }
}

fn string_flag(name: &str, help: &str) -> FlagOption {
    FlagOption {
        name: name.into(),
        field_name: name.into(),
        help: help.into(),
        type_name: "string".into(),
        advanced: false,
        groups: String::new(),
        default_str: String::new(),
        value: json!(""),
        examples: vec![],
    }
}

pub fn flag_category_for_op(op: OperationType) -> Option<&'static str> {
    match op {
        OperationType::Mount => Some("mount"),
        OperationType::Sync => Some("sync"),
        OperationType::Copy | OperationType::Move => Some("copy"),
        OperationType::Check | OperationType::Cryptcheck => Some("check"),
        _ => None,
    }
}

/// Block names used by rclone `options/info` for a serve type.
pub fn serve_block_aliases(serve_type: &str) -> Vec<String> {
    let raw = serve_type.trim();
    if raw.is_empty() {
        return vec!["http".into(), "HTTP".into()];
    }
    let lower = raw.to_ascii_lowercase();
    let mut aliases = vec![
        lower.clone(),
        raw.to_string(),
        raw.to_ascii_uppercase(),
        capitalize_ascii(&lower),
    ];
    match lower.as_str() {
        "webdav" => aliases.extend(["WebDAV".into(), "WEBDAV".into()]),
        "sftp" => aliases.extend(["SFTP".into(), "SFtp".into()]),
        "nfs" => aliases.push("NFS".into()),
        "dlna" => aliases.extend(["DLNA".into(), "Dlna".into()]),
        "s3" => aliases.extend(["S3".into(), "s3".into()]),
        _ => {}
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn capitalize_ascii(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

pub fn simplify_field_name(field: &str) -> String {
    field.rsplit('.').next().unwrap_or(field).to_string()
}

/// Flags for a serve type — matches Tauri `get_serve_flags`.
pub fn options_for_serve_type<'a>(
    blocks: &'a [FlagBlock],
    serve_type: &str,
) -> Vec<&'a FlagOption> {
    let aliases = serve_block_aliases(serve_type);
    for block in blocks {
        if aliases.iter().any(|a| a.eq_ignore_ascii_case(&block.name)) {
            return block.options.iter().collect();
        }
    }
    Vec::new()
}

pub fn collect_serve_flags(blocks: &[FlagBlock], serve_type: &str) -> Vec<FlagOption> {
    options_for_serve_type(blocks, serve_type)
        .into_iter()
        .map(|option| {
            let mut clone = option.clone();
            clone.field_name = simplify_field_name(&clone.field_name);
            clone
        })
        .collect()
}

pub fn flags_to_object(flags: &[(String, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in flags {
        if !key.is_empty() {
            map.insert(key.clone(), value.clone());
        }
    }
    Value::Object(map)
}

pub fn parse_json_object(text: &str) -> Result<Map<String, Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err("JSON must be an object".into()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn options_for_category<'a>(
    blocks: &'a [FlagBlock],
    category: &str,
) -> Vec<(&'a str, &'a FlagOption)> {
    let mut out = Vec::new();
    for block in blocks {
        for option in &block.options {
            if classify_flag(&option.groups) == category
                || (category == "backend" && option.groups.is_empty() && block.name == "main")
            {
                out.push((block.name.as_str(), option));
            }
        }
    }
    out
}

pub fn collect_edits(edits: &[(String, String, Value)]) -> Value {
    let mut root = Map::new();
    for (block, field, value) in edits {
        let payload = set_option_payload(block, field, value.clone());
        if let Some(obj) = payload.as_object() {
            for (k, v) in obj {
                merge_maps(root.entry(k.clone()).or_insert(json!({})), v);
            }
        }
    }
    Value::Object(root)
}

fn merge_maps(dest: &mut Value, src: &Value) {
    match (dest, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s {
                merge_maps(d.entry(k.clone()).or_insert(json!({})), v);
            }
        }
        (dest, src) => *dest = src.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_rclone_groups() {
        assert_eq!(classify_flag("Copy"), "copy");
        assert_eq!(classify_flag("Copy,Sync"), "sync");
        assert_eq!(classify_flag("Filter"), "filter");
        assert_eq!(classify_flag("VFS"), "vfs");
        assert_eq!(classify_flag("Mount"), "mount");
        assert_eq!(classify_flag("Performance,Networking"), "backend");
        assert_eq!(classify_flag("Check"), "check");
    }

    #[test]
    fn builds_nested_set_payload() {
        let payload = set_option_payload("HTTP", "ListenAddr", json!(":8080"));
        assert_eq!(payload["HTTP"]["ListenAddr"], ":8080");
        let dotted = set_option_payload("main", "a.b", json!(1));
        assert_eq!(dotted["main"]["a"]["b"], 1);
    }

    #[test]
    fn parses_and_merges_options() {
        let info = json!({
            "main": [{
                "Name": "transfers",
                "FieldName": "transfers",
                "Help": "Number of file transfers",
                "Type": "int",
                "Groups": "Performance",
                "DefaultStr": "4"
            }]
        });
        let mut blocks = parse_options_info(&info);
        assert_eq!(blocks.len(), 1);
        merge_current_values(&mut blocks, &json!({ "main": { "transfers": 8 } }));
        assert_eq!(blocks[0].options[0].value, json!(8));
        assert_eq!(classify_flag(&blocks[0].options[0].groups), "backend");
    }

    #[test]
    fn static_flags_cover_registry_ops() {
        assert!(!static_flags_for(OperationType::Sync).is_empty());
        assert!(!static_flags_for(OperationType::Bisync).is_empty());
        assert!(!static_flags_for(OperationType::Check).is_empty());
        assert!(static_flags_for(OperationType::Mount).is_empty());
        assert_eq!(flag_category_for_op(OperationType::Mount), Some("mount"));
        assert_eq!(flag_category_for_op(OperationType::Move), Some("copy"));
        assert_eq!(flag_category_for_op(OperationType::Serve), None);
        assert!(!static_flags_for(OperationType::Serve).is_empty());
        assert!(!static_flags_for(OperationType::Archivecreate).is_empty());
    }

    #[test]
    fn serve_flags_match_block_aliases() {
        let info = json!({
            "HTTP": [{
                "Name": "ListenAddr",
                "FieldName": "HTTP.ListenAddr",
                "Help": "listen",
                "Type": "string"
            }],
            "webdav": [{
                "Name": "DisableGET",
                "FieldName": "DisableGET",
                "Help": "off",
                "Type": "bool"
            }]
        });
        let blocks = parse_options_info(&info);
        let http = collect_serve_flags(&blocks, "http");
        assert_eq!(http.len(), 1);
        assert_eq!(http[0].field_name, "ListenAddr");
        assert_eq!(collect_serve_flags(&blocks, "webdav").len(), 1);
        assert!(collect_serve_flags(&blocks, "sftp").is_empty());
        assert!(serve_block_aliases("webdav").iter().any(|a| a == "WebDAV"));
    }

    #[test]
    fn json_object_roundtrip() {
        let obj = flags_to_object(&[("type".into(), json!("http")), ("port".into(), json!(8080))]);
        assert_eq!(obj["type"], "http");
        let parsed = parse_json_object(r#"{ "addr": ":8080" }"#).unwrap();
        assert_eq!(parsed["addr"], ":8080");
        assert!(parse_json_object("[1]").is_err());
        assert!(parse_json_object("{").is_err());
        assert!(parse_json_object("").unwrap().is_empty());
    }

    #[test]
    fn collect_edits_merges_blocks() {
        let value = collect_edits(&[
            ("main".into(), "transfers".into(), json!(8)),
            ("main".into(), "checkers".into(), json!(16)),
            ("vfs".into(), "CacheMode".into(), json!("full")),
        ]);
        assert_eq!(value["main"]["transfers"], 8);
        assert_eq!(value["main"]["checkers"], 16);
        assert_eq!(value["vfs"]["CacheMode"], "full");
    }
}
