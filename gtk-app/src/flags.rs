//! Rclone flag editors — parse `options/info` + `options/get` and apply `options/set`.
//! Group taxonomy matches `src-tauri/src/rclone/queries/flags.rs`.

use crate::operations::OperationType;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};

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
const NETWORK_GROUPS: &[&str] = &["Proxy", "HTTP", "FTP"];

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
    pub exclusive: bool,
}

impl FlagOption {
    pub fn from_provider(option: &crate::providers::ProviderOption) -> Self {
        Self {
            name: option.name.clone(),
            field_name: option.name.clone(),
            help: option.help.clone(),
            type_name: option.type_name.clone(),
            advanced: option.advanced,
            groups: "Runtime".into(),
            default_str: option.default_str.clone(),
            value: option.value.clone(),
            examples: option.examples.clone(),
            exclusive: option.exclusive,
        }
    }
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
    } else if flag_has_any_group(groups, NETWORK_GROUPS) {
        "network"
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

/// rclone `options/blocks` is `{ "options": ["main", "vfs", ...] }` or a raw array.
pub fn parse_options_blocks(value: &Value) -> Vec<String> {
    let arr = value
        .get("options")
        .and_then(|v| v.as_array())
        .or_else(|| value.as_array());
    let Some(arr) = arr else {
        return Vec::new();
    };
    let mut names: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .filter(|name| !name.is_empty())
        .collect();
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    names.dedup();
    names
}

pub fn ensure_named_blocks(blocks: &mut Vec<FlagBlock>, names: &[String]) {
    for name in names {
        if name.is_empty() {
            continue;
        }
        if !blocks
            .iter()
            .any(|block| block.name.eq_ignore_ascii_case(name))
        {
            blocks.push(FlagBlock {
                name: name.clone(),
                options: Vec::new(),
            });
        }
    }
    blocks.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
}

pub fn option_blocks_from_rc(info: &Value, blocks: &Value) -> Vec<FlagBlock> {
    let mut parsed = parse_options_info(info);
    ensure_named_blocks(&mut parsed, &parse_options_blocks(blocks));
    parsed
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
        exclusive: value
            .get("Exclusive")
            .or_else(|| value.get("exclusive"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
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
    crate::value_mapper::human_to_machine(text, type_name)
}

/// Current flag value as the editor should display it.
pub fn flag_display_text(flag: &FlagOption, current: &Value) -> String {
    let value = current
        .get(&flag.field_name)
        .or_else(|| current.get(&flag.name))
        .or_else(|| {
            current
                .get("_config")
                .and_then(|cfg| cfg.get(&flag.field_name).or_else(|| cfg.get(&flag.name)))
        });
    match value {
        Some(value) => {
            let text =
                crate::value_mapper::machine_to_human(value, &flag.type_name, &flag.default_str);
            if text.is_empty() || text == "null" {
                flag.default_str.clone()
            } else {
                text
            }
        }
        None => flag.default_str.clone(),
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
        OperationType::Mount => vec![string_flag("mountType", "Mount type to use.")],
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
                exclusive: false,
            },
            bool_flag("force", "Bypass --max-delete safety check."),
            string_flag("compare", "size,modtime,checksum"),
            string_flag("conflictLoser", "num, pathname, or delete"),
            string_flag("conflictResolve", "none, path1, path2, newer, …"),
            string_flag("conflictSuffix", "Suffix when renaming a conflict loser."),
            bool_flag(
                "createEmptySrcDirs",
                "Sync creation and deletion of empty directories.",
            ),
            bool_flag(
                "removeEmptyDirs",
                "Remove empty directories at the final cleanup step.",
            ),
            bool_flag(
                "recover",
                "Automatically recover from interruptions without --resync.",
            ),
            bool_flag(
                "resilient",
                "Allow future runs to retry after less-serious errors.",
            ),
            string_flag("workdir", "Custom bisync working directory."),
            string_flag("backupDir1", "--backup-dir for Path1."),
            string_flag("backupDir2", "--backup-dir for Path2."),
            bool_flag("noCleanup", "Retain working files."),
            string_flag("checkSync", "true, false, or only."),
            string_flag("maxLock", "Expire lock files older than this (e.g. 2m)."),
        ],
        OperationType::Copyurl => vec![bool_flag(
            "autoFilename",
            "Get the filename from the URL or headers if destination is a directory.",
        )],
        OperationType::Delete => vec![bool_flag(
            "rmdirs",
            "Remove empty directories after deleting files.",
        )],
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
        exclusive: false,
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
        exclusive: false,
    }
}

pub fn merged_flags_for(op: OperationType, blocks: &[FlagBlock]) -> Vec<FlagOption> {
    let mut options = static_flags_for(op);
    if let Some(category) = flag_category_for_op(op) {
        for (_, option) in options_for_category(blocks, category) {
            if options.iter().any(|o| o.field_name == option.field_name) {
                continue;
            }
            options.push(option.clone());
        }
    }
    options
}

pub fn filter_options_for_categories(
    current: &Value,
    blocks: &[FlagBlock],
    categories: &[&str],
) -> Value {
    let Some(obj) = current.as_object() else {
        return json!({});
    };
    if categories.is_empty() {
        return current.clone();
    }
    let mut out = Map::new();
    for (block_name, value) in obj {
        let category = blocks
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(block_name))
            .and_then(|b| b.options.first())
            .map(|o| classify_flag(&o.groups))
            .unwrap_or("backend");
        if categories
            .iter()
            .any(|wanted| *wanted == category || wanted.eq_ignore_ascii_case(block_name))
        {
            out.insert(block_name.clone(), value.clone());
        }
    }
    Value::Object(out)
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

/// Angular rclone-flags home: three top-level buckets.
pub const MAIN_CATEGORY_KEYS: &[&str] = &[
    "generalSettings",
    "fileSystemAndStorage",
    "networkAndServers",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagServiceCategory {
    pub name: String,
    pub options: Vec<FlagOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagService {
    pub name: String,
    pub categories: Vec<FlagServiceCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagSearchHit {
    pub service: String,
    pub category: String,
    pub option: FlagOption,
}

/// First `FieldName` segment, or `General` when the name is not dotted.
pub fn option_field_category(field_name: &str) -> &str {
    field_name
        .split_once('.')
        .map(|(group, _)| group)
        .filter(|group| !group.is_empty())
        .unwrap_or("General")
}

/// Angular `SERVICE_CONFIG[name].mainCategory` (unknown services → Network).
pub fn service_main_category(service: &str) -> &'static str {
    match service.to_ascii_lowercase().as_str() {
        "vfs" | "mount" | "filter" => "fileSystemAndStorage",
        "main" | "log" | "rc" | "proxy" => "generalSettings",
        _ => "networkAndServers",
    }
}

/// Group `options/info` blocks the way Angular `group_options` does:
/// service = block name, category = first `FieldName` segment.
pub fn group_blocks_by_service(blocks: &[FlagBlock]) -> Vec<FlagService> {
    let mut services: BTreeMap<String, BTreeMap<String, Vec<FlagOption>>> = BTreeMap::new();
    for block in blocks {
        if block.options.is_empty() {
            continue;
        }
        let cats = services.entry(block.name.clone()).or_default();
        let mut seen = HashSet::new();
        for option in &block.options {
            let category = option_field_category(&option.field_name).to_string();
            let key = if option.field_name.is_empty() {
                option.name.clone()
            } else {
                option.field_name.clone()
            };
            if !seen.insert((category.clone(), key)) {
                continue;
            }
            cats.entry(category).or_default().push(option.clone());
        }
    }
    services
        .into_iter()
        .map(|(name, cats)| FlagService {
            name,
            categories: cats
                .into_iter()
                .map(|(name, options)| FlagServiceCategory { name, options })
                .collect(),
        })
        .collect()
}

pub fn services_for_main_category<'a>(
    services: &'a [FlagService],
    main: &str,
) -> Vec<&'a FlagService> {
    services
        .iter()
        .filter(|service| service_main_category(&service.name) == main)
        .collect()
}

pub fn search_grouped_flags(services: &[FlagService], query: &str) -> Vec<FlagSearchHit> {
    let clean = crate::config_search::strip_cli_prefix(query);
    if clean.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for service in services {
        let service_hit = service.name.to_ascii_lowercase().contains(&clean);
        for category in &service.categories {
            let category_hit = category.name.to_ascii_lowercase().contains(&clean);
            for option in &category.options {
                if service_hit
                    || category_hit
                    || crate::config_search::matches_config_search(
                        &option.name,
                        &option.help,
                        &option.field_name,
                        query,
                    )
                {
                    hits.push(FlagSearchHit {
                        service: service.name.clone(),
                        category: category.name.clone(),
                        option: option.clone(),
                    });
                }
            }
        }
    }
    hits
}

pub fn find_service_category<'a>(
    services: &'a [FlagService],
    service: &str,
    category: &str,
) -> Option<&'a FlagServiceCategory> {
    services
        .iter()
        .find(|item| item.name.eq_ignore_ascii_case(service))
        .and_then(|item| {
            item.categories
                .iter()
                .find(|cat| cat.name.eq_ignore_ascii_case(category))
        })
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
    fn provider_option_maps_to_runtime_flag() {
        let option = crate::providers::ProviderOption {
            name: "chunk_size".into(),
            help: "Upload chunk".into(),
            required: false,
            advanced: true,
            is_password: false,
            exclusive: false,
            type_name: "SizeSuffix".into(),
            default: json!("8Mi"),
            default_str: "8Mi".into(),
            value: json!("8Mi"),
            value_str: "8Mi".into(),
            examples: vec![],
            provider: String::new(),
            example_providers: vec![],
        };
        let flag = FlagOption::from_provider(&option);
        assert_eq!(flag.field_name, "chunk_size");
        assert_eq!(flag.groups, "Runtime");
        assert_eq!(flag.default_str, "8Mi");
        assert!(flag.advanced);
    }

    #[test]
    fn merges_static_and_live_flags() {
        let info = json!({
            "main": [{
                "Name": "transfers",
                "FieldName": "transfers",
                "Help": "Number of file transfers",
                "Type": "int",
                "Groups": "Copy",
                "DefaultStr": "4"
            }]
        });
        let blocks = parse_options_info(&info);
        let merged = merged_flags_for(OperationType::Copy, &blocks);
        assert!(merged.iter().any(|f| f.field_name == "createEmptySrcDirs"));
        assert!(merged.iter().any(|f| f.field_name == "transfers"));
        assert!(static_flags_for(OperationType::Mount)
            .iter()
            .any(|f| f.field_name == "mountType"));
        let filtered = filter_options_for_categories(
            &json!({ "main": { "transfers": 8 }, "vfs": { "CacheMode": "full" } }),
            &blocks,
            &["copy"],
        );
        assert!(filtered.get("main").is_some());
        assert!(filtered.get("vfs").is_none());
    }

    #[test]
    fn classifies_rclone_groups() {
        assert_eq!(classify_flag("Copy"), "copy");
        assert_eq!(classify_flag("Copy,Sync"), "sync");
        assert_eq!(classify_flag("Filter"), "filter");
        assert_eq!(classify_flag("VFS"), "vfs");
        assert_eq!(classify_flag("Mount"), "mount");
        assert_eq!(classify_flag("Performance,Networking"), "backend");
        assert_eq!(classify_flag("Check"), "check");
        assert_eq!(classify_flag("HTTP"), "network");
        assert_eq!(classify_flag("Proxy"), "network");
        assert_eq!(classify_flag("FTP"), "network");
        assert_eq!(classify_flag("RC"), "other");
        assert_eq!(classify_flag("WebDAV"), "other");
    }

    #[test]
    fn displays_bool_int_and_default_flag_text() {
        let flag = bool_flag("createEmptySrcDirs", "Create empty dirs");
        assert_eq!(
            flag_display_text(&flag, &json!({ "createEmptySrcDirs": true })),
            "true"
        );
        assert_eq!(flag_display_text(&flag, &json!({})), flag.default_str);
        let transfers = FlagOption {
            name: "transfers".into(),
            field_name: "transfers".into(),
            help: "parallel".into(),
            type_name: "int".into(),
            advanced: false,
            groups: "Copy".into(),
            default_str: "4".into(),
            value: json!(4),
            examples: vec![],
            exclusive: false,
        };
        assert_eq!(
            flag_display_text(&transfers, &json!({ "transfers": 8 })),
            "8"
        );
    }

    #[test]
    fn builds_nested_set_payload() {
        let payload = set_option_payload("HTTP", "ListenAddr", json!(":8080"));
        assert_eq!(payload["HTTP"]["ListenAddr"], ":8080");
        let dotted = set_option_payload("main", "a.b", json!(1));
        assert_eq!(dotted["main"]["a"]["b"], 1);
    }

    #[test]
    fn parses_options_blocks_and_fills_empty() {
        let names = parse_options_blocks(&json!({ "options": ["main", "vfs", "HTTP"] }));
        assert_eq!(names, vec!["HTTP", "main", "vfs"]);
        assert_eq!(
            parse_options_blocks(&json!(["rc", "log"])),
            vec!["log", "rc"]
        );
        assert!(parse_options_blocks(&json!({})).is_empty());
        let mut blocks = parse_options_info(&json!({
            "main": [{
                "Name": "transfers",
                "FieldName": "transfers",
                "Type": "int",
                "Groups": "Performance"
            }]
        }));
        ensure_named_blocks(&mut blocks, &names);
        assert!(blocks
            .iter()
            .any(|b| b.name == "main" && !b.options.is_empty()));
        assert!(blocks
            .iter()
            .any(|b| b.name == "vfs" && b.options.is_empty()));
        assert!(blocks
            .iter()
            .any(|b| b.name == "HTTP" && b.options.is_empty()));
        let merged = option_blocks_from_rc(
            &json!({
                "main": [{
                    "Name": "transfers",
                    "FieldName": "transfers",
                    "Type": "int"
                }]
            }),
            &json!({ "options": ["main", "filter"] }),
        );
        assert!(merged.iter().any(|b| b.name == "filter"));
        assert_eq!(merged.iter().filter(|b| b.name == "main").count(), 1);
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
        assert!(static_flags_for(OperationType::Mount)
            .iter()
            .any(|f| f.field_name == "mountType"));
        assert_eq!(flag_category_for_op(OperationType::Mount), Some("mount"));
        assert_eq!(flag_category_for_op(OperationType::Move), Some("copy"));
        assert_eq!(flag_category_for_op(OperationType::Serve), None);
        assert!(!static_flags_for(OperationType::Serve).is_empty());
        assert!(!static_flags_for(OperationType::Archivecreate).is_empty());
        assert!(static_flags_for(OperationType::Copyurl)
            .iter()
            .any(|f| f.field_name == "autoFilename"));
        assert!(static_flags_for(OperationType::Delete)
            .iter()
            .any(|f| f.field_name == "rmdirs"));
        assert!(static_flags_for(OperationType::Bisync)
            .iter()
            .any(|f| f.field_name == "recover"));
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
    fn parses_exclusive_and_human_flag_values() {
        let info = json!({
            "main": [{
                "Name": "logLevel",
                "FieldName": "logLevel",
                "Type": "string",
                "Exclusive": true,
                "Examples": [{"Value": "DEBUG", "Help": "debug"}]
            }]
        });
        let blocks = parse_options_info(&info);
        assert!(blocks[0].options[0].exclusive);
        assert_eq!(blocks[0].options[0].examples[0].0, "DEBUG");
        assert_eq!(parse_flag_value("bool", "true"), json!(true));
        assert_eq!(parse_flag_value("int", "8"), json!(8));
        assert_eq!(parse_flag_value("Duration", "30s"), json!("30s"));
        assert_eq!(parse_flag_value("SizeSuffix", "1Gi"), json!("1Gi"));
        assert_eq!(parse_flag_value("Tristate", "unset"), json!(null));
    }

    #[test]
    fn groups_options_by_service_and_field_prefix() {
        let blocks = parse_options_info(&json!({
            "main": [
                {
                    "Name": "transfers",
                    "FieldName": "transfers",
                    "Help": "parallel",
                    "Type": "int"
                },
                {
                    "Name": "listen-addr",
                    "FieldName": "HTTP.ListenAddr",
                    "Help": "listen",
                    "Type": "string"
                },
                {
                    "Name": "listen-addr",
                    "FieldName": "HTTP.ListenAddr",
                    "Help": "duplicate",
                    "Type": "string"
                }
            ],
            "vfs": [{
                "Name": "cache-mode",
                "FieldName": "CacheMode",
                "Help": "cache",
                "Type": "string"
            }],
            "sftp": [{
                "Name": "user",
                "FieldName": "Auth.User",
                "Help": "user",
                "Type": "string"
            }]
        }));
        let grouped = group_blocks_by_service(&blocks);
        assert_eq!(
            grouped.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["main", "sftp", "vfs"]
        );
        let main = grouped.iter().find(|s| s.name == "main").unwrap();
        assert_eq!(
            main.categories
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["General", "HTTP"]
        );
        assert_eq!(main.categories[1].options.len(), 1);
        assert_eq!(service_main_category("vfs"), "fileSystemAndStorage");
        assert_eq!(service_main_category("main"), "generalSettings");
        assert_eq!(service_main_category("sftp"), "networkAndServers");
        assert_eq!(
            services_for_main_category(&grouped, "fileSystemAndStorage")
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["vfs"]
        );
        let hits = search_grouped_flags(&grouped, "--listen");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].service, "main");
        assert_eq!(hits[0].category, "HTTP");
        let vfs_hits = search_grouped_flags(&grouped, "vfs");
        assert!(vfs_hits.iter().any(|h| h.option.name == "cache-mode"));
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
