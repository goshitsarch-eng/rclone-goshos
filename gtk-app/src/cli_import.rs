//! Angular-parity rclone CLI import: tokenize, parse, classify, and apply.

use crate::flags::{classify_flag, static_flags_for, FlagBlock, FlagOption};
use crate::operations::OperationType;
use crate::value_mapper::{human_to_machine, parse_tristate};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};

const FLAG_PATTERN_START: &[char] = &['a', 'A'];

const SHORT_FLAG_ALIASES: &[(&str, &str)] = &[
    ("P", "progress"),
    ("v", "verbose"),
    ("vv", "verbose"),
    ("q", "quiet"),
    ("n", "dry-run"),
    ("u", "update"),
    ("L", "copy-links"),
    ("I", "ignore-times"),
    ("c", "checksum"),
    ("R", "raw-list"),
    ("s", "stats"),
];

const WRAPPER_TOKENS: &[&str] = &[
    "sudo", "nohup", "nice", "time", "env", "wsl", "exec", "sh", "bash", "-c",
];

#[derive(Debug, Clone, PartialEq)]
pub enum FlagValue {
    Bool(bool),
    Text(String),
}

impl FlagValue {
    pub fn as_display(&self) -> String {
        match self {
            Self::Bool(b) => b.to_string(),
            Self::Text(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFlag {
    pub raw: String,
    pub key: String,
    pub value: FlagValue,
    pub has_macro: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedCli {
    pub verb: Option<String>,
    pub serve_subtype: Option<String>,
    pub mount_subtype: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub flags: Vec<ParsedFlag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagStatus {
    Mapped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassifiedFlag {
    pub flag: ParsedFlag,
    pub status: FlagStatus,
    pub flag_type: Option<String>,
    pub field_name: Option<String>,
    pub coerced_value: Option<Value>,
    pub guidance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportResult {
    pub verb: Option<String>,
    pub serve_subtype: Option<String>,
    pub mount_subtype: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub classified: Vec<ClassifiedFlag>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LookupOption {
    pub name: String,
    pub field_name: String,
    pub type_name: String,
}

impl From<&FlagOption> for LookupOption {
    fn from(option: &FlagOption) -> Self {
        Self {
            name: option.name.clone(),
            field_name: option.field_name.clone(),
            type_name: option.type_name.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LookupEntry {
    pub option: LookupOption,
    pub flag_type: String,
    pub supported_flag_types: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileMode {
    New,
    Override,
    Patch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CliImportApply {
    pub verb: Option<String>,
    pub serve_subtype: Option<String>,
    pub mount_subtype: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub flags: Vec<(String, Value)>,
    pub profile_mode: ProfileMode,
    pub profile_name: String,
}

pub fn has_macro(val: &str) -> bool {
    if let Some(start) = val.find("$(") {
        if val[start + 2..].contains(')') {
            return true;
        }
    }
    if let Some(start) = val.find('`') {
        if val[start + 1..].contains('`') {
            return true;
        }
    }
    false
}

pub fn tokenize(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_single = false;
    let mut in_subshell = 0usize;
    let mut in_backtick = false;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();
        if ch == '\\' && matches!(next, Some('\n') | Some('\r')) {
            if next == Some('\r') && chars.get(i + 2) == Some(&'\n') {
                i += 3;
            } else {
                i += 2;
            }
            continue;
        }
        if ch == '\\' && in_double {
            if let Some(n) = next {
                current.push(ch);
                current.push(n);
                i += 2;
                continue;
            }
        }
        if ch == '#'
            && !in_double
            && !in_single
            && in_subshell == 0
            && !in_backtick
            && (i == 0 || chars[i - 1].is_whitespace())
        {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '"' && !in_single && in_subshell == 0 && !in_backtick {
            in_double = !in_double;
            current.push(ch);
        } else if ch == '\'' && !in_double && in_subshell == 0 && !in_backtick {
            in_single = !in_single;
            current.push(ch);
        } else if ch == '`' && !in_single {
            in_backtick = !in_backtick;
            current.push(ch);
        } else if ch == '$' && next == Some('(') && !in_single {
            in_subshell += 1;
            current.push_str("$(");
            i += 2;
            continue;
        } else if ch == ')' && in_subshell > 0 && !in_single {
            in_subshell -= 1;
            current.push(')');
        } else if matches!(ch, ' ' | '\t' | '\r' | '\n')
            && !in_double
            && !in_single
            && in_subshell == 0
            && !in_backtick
        {
            if !current.is_empty() {
                tokens.push(strip_quotes(&current));
                current.clear();
            }
        } else {
            current.push(ch);
        }
        i += 1;
    }
    if !current.is_empty() {
        tokens.push(strip_quotes(&current));
    }
    tokens
}

fn strip_quotes(token: &str) -> String {
    let bytes = token.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        token[1..token.len() - 1].to_string()
    } else {
        token.to_string()
    }
}

fn is_rclone_binary(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower == "rclone"
        || lower == "rclone.exe"
        || lower.starts_with("./rclone")
        || lower.starts_with(".\\rclone")
        || lower.ends_with("/rclone")
        || lower.ends_with("\\rclone")
        || lower.ends_with("/rclone.exe")
        || lower.ends_with("\\rclone.exe")
}

fn is_flag_token(token: &str) -> bool {
    if !token.starts_with('-') {
        return false;
    }
    if token.starts_with("--") {
        return token.len() > 2;
    }
    if token.len() == 2 {
        return token
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_ascii_alphanumeric());
    }
    token == "-vv" || token == "-vvv"
}

fn looks_like_flag(token: &str) -> bool {
    let mut chars = token.chars();
    match (chars.next(), chars.next()) {
        (Some('-'), Some(c)) if c.is_ascii_alphabetic() => true,
        (Some('-'), Some('-')) => chars.next().is_some_and(|c| c.is_ascii_alphabetic()),
        _ => false,
    }
}

fn verb_map(token: &str) -> Option<(&'static str, Option<&'static str>)> {
    match token.to_ascii_lowercase().as_str() {
        "sync" => Some(("sync", None)),
        "copy" | "copyto" => Some(("copy", None)),
        "move" | "moveto" => Some(("move", None)),
        "bisync" => Some(("bisync", None)),
        "mount" => Some(("mount", Some("mount"))),
        "mount2" => Some(("mount", Some("mount2"))),
        "cmount" => Some(("mount", Some("cmount"))),
        "nfsmount" => Some(("mount", Some("nfsmount"))),
        "serve" => Some(("serve", None)),
        "check" => Some(("check", None)),
        "delete" | "cleanup" | "purge" | "rmdir" | "rmdirs" => Some(("delete", None)),
        "copyurl" => Some(("copyurl", None)),
        _ => None,
    }
}

fn alias_for(key: &str) -> Option<&'static str> {
    SHORT_FLAG_ALIASES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

pub fn parse(cli: &str, existing_bools: &HashSet<String>) -> ParsedCli {
    let raw_tokens = tokenize(cli);
    let mut start = 0;
    while start < raw_tokens.len() {
        let t = &raw_tokens[start];
        if is_rclone_binary(t) {
            start += 1;
            break;
        }
        if WRAPPER_TOKENS.iter().any(|w| t.eq_ignore_ascii_case(w)) {
            start += 1;
            continue;
        }
        break;
    }
    let tokens = &raw_tokens[start..];
    let mut flags = Vec::new();
    let mut verb = None;
    let mut serve_subtype = None;
    let mut mount_subtype = None;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if token.starts_with('-') && looks_like_flag(token) {
            let raw_key;
            let mut raw_value = FlagValue::Bool(true);
            let mut original = token.clone();
            if let Some(eq) = token.find('=') {
                raw_key = token[..eq].to_string();
                let raw_val = strip_quotes(&token[eq + 1..]);
                raw_value = match raw_val.to_ascii_lowercase().as_str() {
                    "true" => FlagValue::Bool(true),
                    "false" => FlagValue::Bool(false),
                    _ => FlagValue::Text(raw_val),
                };
            } else {
                raw_key = token.clone();
                let clean = raw_key.trim_start_matches('-');
                let lower = clean.to_ascii_lowercase();
                let is_short = !raw_key.starts_with("--") && clean.len() == 1;
                let is_known_bool = is_short
                    || existing_bools.contains(&lower)
                    || existing_bools.contains(&lower.replace('-', "_"))
                    || existing_bools.contains(&lower.replace('_', "-"))
                    || lower.starts_with("no-");
                let next = tokens.get(i + 1);
                if let Some(next_token) = next {
                    if !is_flag_token(next_token) && !is_known_bool {
                        raw_value = FlagValue::Text(next_token.clone());
                        original = format!("{raw_key} {next_token}");
                        i += 1;
                    }
                }
                let _ = FLAG_PATTERN_START;
            }
            let key = raw_key.trim_start_matches('-').to_string();
            let has_macro = matches!(&raw_value, FlagValue::Text(s) if has_macro(s));
            flags.push(ParsedFlag {
                raw: original,
                key,
                value: raw_value,
                has_macro,
            });
        } else {
            let lower = token.to_ascii_lowercase();
            if verb.is_none() {
                if let Some((mapped, mount)) = verb_map(&lower) {
                    verb = Some(mapped.to_string());
                    if let Some(sub) = mount {
                        mount_subtype = Some(sub.to_string());
                    }
                } else {
                    positional.push(token.clone());
                }
            } else if verb.as_deref() == Some("serve") && serve_subtype.is_none() {
                serve_subtype = Some(lower);
            } else {
                positional.push(token.clone());
            }
        }
        i += 1;
    }
    ParsedCli {
        verb,
        serve_subtype,
        mount_subtype,
        source_path: positional.first().cloned(),
        dest_path: positional.get(1).cloned(),
        flags,
    }
}

pub fn build_lookup_table(
    fields: &HashMap<String, Vec<LookupOption>>,
    remote_type: Option<&str>,
) -> HashMap<String, LookupEntry> {
    let mut table = HashMap::new();
    let prefix = remote_type
        .map(|t| format!("{}-", t.to_ascii_lowercase().trim()))
        .unwrap_or_default();
    for (flag_type, options) in fields {
        let is_runtime = flag_type == "runtimeRemote";
        for field in options {
            let names = [field.name.as_str(), field.field_name.as_str()]
                .into_iter()
                .filter(|n| !n.is_empty());
            for raw_name in names {
                let key = raw_name.to_ascii_lowercase().replace('_', "-");
                if key.is_empty() {
                    continue;
                }
                register_key(&mut table, &key, field, flag_type);
                register_key(&mut table, &key.replace('-', ""), field, flag_type);
                register_key(&mut table, &raw_name.to_ascii_lowercase(), field, flag_type);
                if is_runtime && !prefix.is_empty() {
                    let prefixed = format!("{prefix}{key}");
                    register_key(&mut table, &prefixed, field, flag_type);
                    register_key(&mut table, &prefixed.replace('-', ""), field, flag_type);
                }
            }
        }
    }
    table
}

fn register_key(
    table: &mut HashMap<String, LookupEntry>,
    key: &str,
    field: &LookupOption,
    flag_type: &str,
) {
    if let Some(existing) = table.get_mut(key) {
        existing.supported_flag_types.insert(flag_type.to_string());
    } else {
        let mut supported = BTreeSet::new();
        supported.insert(flag_type.to_string());
        table.insert(
            key.to_string(),
            LookupEntry {
                option: field.clone(),
                flag_type: flag_type.to_string(),
                supported_flag_types: supported,
            },
        );
    }
}

pub fn classify(
    parsed: &ParsedCli,
    lookup: &HashMap<String, LookupEntry>,
    preferred_type: Option<&str>,
) -> ImportResult {
    let target = preferred_type.or(parsed.verb.as_deref());
    let classified = parsed
        .flags
        .iter()
        .map(|flag| {
            let mut key_lower = flag.key.to_ascii_lowercase();
            if let Some(alias) = alias_for(&flag.key).or_else(|| alias_for(&key_lower)) {
                key_lower = alias.to_string();
            }
            let mut match_entry = lookup
                .get(&key_lower)
                .or_else(|| lookup.get(&key_lower.replace(['-', '_'], "")));
            let mut negated = false;
            if match_entry.is_none() && key_lower.starts_with("no-") {
                let unnegated = &key_lower[3..];
                if let Some(candidate) = lookup
                    .get(unnegated)
                    .or_else(|| lookup.get(&unnegated.replace(['-', '_'], "")))
                {
                    if candidate.option.type_name.eq_ignore_ascii_case("bool")
                        || candidate.option.type_name.eq_ignore_ascii_case("tristate")
                    {
                        match_entry = Some(candidate);
                        negated = true;
                    }
                }
            }
            if let Some(entry) = match_entry {
                let coerced = if negated {
                    json!(false)
                } else {
                    coerce_value(&flag.value, &entry.option.type_name)
                };
                let resolved = target
                    .filter(|pref| entry.supported_flag_types.iter().any(|t| t == pref))
                    .unwrap_or(entry.flag_type.as_str())
                    .to_string();
                let field_name = if entry.option.name.is_empty() {
                    entry.option.field_name.clone()
                } else {
                    entry.option.name.clone()
                };
                ClassifiedFlag {
                    flag: flag.clone(),
                    status: FlagStatus::Mapped,
                    flag_type: Some(resolved),
                    field_name: Some(field_name),
                    coerced_value: Some(coerced),
                    guidance: None,
                }
            } else {
                ClassifiedFlag {
                    flag: flag.clone(),
                    status: FlagStatus::Unknown,
                    flag_type: None,
                    field_name: None,
                    coerced_value: None,
                    guidance: None,
                }
            }
        })
        .collect();
    ImportResult {
        verb: parsed.verb.clone(),
        serve_subtype: parsed.serve_subtype.clone(),
        mount_subtype: parsed.mount_subtype.clone(),
        source_path: parsed.source_path.clone(),
        dest_path: parsed.dest_path.clone(),
        classified,
    }
}

fn coerce_value(val: &FlagValue, type_name: &str) -> Value {
    match val {
        FlagValue::Bool(b) => json!(*b),
        FlagValue::Text(s) => {
            let lower = s.to_ascii_lowercase();
            if (type_name.eq_ignore_ascii_case("bool")
                || type_name.eq_ignore_ascii_case("tristate"))
                && (lower == "true" || lower == "false")
            {
                return json!(lower == "true");
            }
            if type_name.eq_ignore_ascii_case("tristate") {
                return match parse_tristate(&json!(s)) {
                    Some(v) => json!(v),
                    None => Value::Null,
                };
            }
            human_to_machine(s, type_name)
        }
    }
}

const COMMON_CLI_FLAGS: &[(&str, &str, &str, &str)] = &[
    ("transfers", "transfers", "int", "backend"),
    ("checkers", "checkers", "int", "backend"),
    ("retries", "retries", "int", "backend"),
    ("max_delete", "max_delete", "int", "sync"),
    ("dry_run", "dry_run", "bool", "sync"),
    ("checksum", "checksum", "bool", "sync"),
    ("ignore_times", "ignore_times", "bool", "sync"),
    ("update", "update", "bool", "sync"),
    ("verbose", "verbose", "int", "backend"),
    ("progress", "progress", "bool", "backend"),
    ("bwlimit", "bwlimit", "string", "backend"),
    ("tpslimit", "tpslimit", "float64", "backend"),
    ("tpslimit_burst", "tpslimit_burst", "uint32", "backend"),
    ("backup_dir", "backup_dir", "string", "sync"),
    ("track_renames", "track_renames", "bool", "sync"),
    ("fast_list", "fast_list", "bool", "backend"),
    ("suffix", "suffix", "string", "sync"),
    ("exclude", "exclude", "string", "filter"),
    ("include", "include", "string", "filter"),
    ("filter", "filter", "string", "filter"),
    ("exclude_from", "exclude_from", "string", "filter"),
    ("vfs_cache_mode", "vfs_cache_mode", "string", "vfs"),
    ("vfs_cache_max_size", "vfs_cache_max_size", "string", "vfs"),
    ("vfs_cache_max_age", "vfs_cache_max_age", "string", "vfs"),
    ("createEmptySrcDirs", "createEmptySrcDirs", "bool", "sync"),
];

pub fn boolean_flags_from_blocks(blocks: &[FlagBlock]) -> HashSet<String> {
    let mut bools = HashSet::new();
    for block in blocks {
        for option in &block.options {
            register_bool(&mut bools, option);
        }
    }
    for (name, field, type_name, _) in COMMON_CLI_FLAGS {
        if type_name.eq_ignore_ascii_case("bool") || type_name.eq_ignore_ascii_case("tristate") {
            register_bool_name(&mut bools, name);
            register_bool_name(&mut bools, field);
        }
    }
    for op in OperationType::ALL {
        for option in static_flags_for(op) {
            register_bool(&mut bools, &option);
        }
    }
    bools
}

fn register_bool(bools: &mut HashSet<String>, option: &FlagOption) {
    if !option.type_name.eq_ignore_ascii_case("bool")
        && !option.type_name.eq_ignore_ascii_case("tristate")
    {
        return;
    }
    register_bool_name(bools, &option.name);
}

fn register_bool_name(bools: &mut HashSet<String>, name: &str) {
    let name = name.to_ascii_lowercase();
    if name.is_empty() {
        return;
    }
    bools.insert(name.clone());
    bools.insert(name.replace('_', "-"));
}

pub fn lookup_fields_from_blocks(
    blocks: &[FlagBlock],
    runtime_remote: &[LookupOption],
) -> HashMap<String, Vec<LookupOption>> {
    let mut fields: HashMap<String, Vec<LookupOption>> = HashMap::new();
    for block in blocks {
        for option in &block.options {
            let kind = classify_flag(&option.groups).to_string();
            fields
                .entry(kind)
                .or_default()
                .push(LookupOption::from(option));
        }
    }
    for op in OperationType::ALL {
        for option in static_flags_for(op) {
            fields
                .entry(op.as_str().to_string())
                .or_default()
                .push(LookupOption::from(&option));
        }
    }
    for (name, field, type_name, kind) in COMMON_CLI_FLAGS {
        fields
            .entry((*kind).into())
            .or_default()
            .push(LookupOption {
                name: (*name).into(),
                field_name: (*field).into(),
                type_name: (*type_name).into(),
            });
    }
    if !runtime_remote.is_empty() {
        fields.insert("runtimeRemote".into(), runtime_remote.to_vec());
    }
    fields
}

pub fn import_cli_command(
    cli: &str,
    blocks: &[FlagBlock],
    runtime_remote: &[LookupOption],
    remote_type: Option<&str>,
    preferred_type: Option<&str>,
) -> ImportResult {
    let bools = boolean_flags_from_blocks(blocks);
    let parsed = parse(cli, &bools);
    let fields = lookup_fields_from_blocks(blocks, runtime_remote);
    let table = build_lookup_table(&fields, remote_type);
    classify(&parsed, &table, preferred_type)
}

pub fn parsed_to_flag_map(parsed: &ParsedCli) -> Map<String, Value> {
    let mut map = Map::new();
    for flag in &parsed.flags {
        let key = flag.key.replace('-', "_");
        let value = match &flag.value {
            FlagValue::Bool(b) => json!(*b),
            FlagValue::Text(s) => json!(s),
        };
        map.insert(key, value);
    }
    map
}

pub fn selected_apply(
    result: &ImportResult,
    selected_keys: &HashSet<String>,
    import_source: bool,
    import_dest: bool,
    profile_mode: ProfileMode,
    profile_name: &str,
) -> CliImportApply {
    let flags = result
        .classified
        .iter()
        .filter(|item| item.status == FlagStatus::Mapped && selected_keys.contains(&item.flag.key))
        .filter_map(|item| {
            Some((
                item.field_name.clone()?,
                item.coerced_value.clone().unwrap_or(Value::Null),
            ))
        })
        .collect();
    CliImportApply {
        verb: result.verb.clone(),
        serve_subtype: result.serve_subtype.clone(),
        mount_subtype: result.mount_subtype.clone(),
        source_path: if import_source {
            result.source_path.clone()
        } else {
            None
        },
        dest_path: if import_dest {
            result.dest_path.clone()
        } else {
            None
        },
        flags,
        profile_mode,
        profile_name: profile_name.to_string(),
    }
}

pub fn value_as_text(value: &Value) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string().trim_matches('"').to_string(),
    }
}

pub fn is_valid_import(result: &ImportResult) -> bool {
    result.verb.is_some() || !result.classified.is_empty()
}

pub fn reconstruct_cli(apply: &CliImportApply) -> String {
    apply
        .flags
        .iter()
        .map(|(key, value)| {
            let flag = key.replace('_', "-");
            match value {
                Value::Bool(true) => format!("--{flag}"),
                Value::Bool(false) => format!("--{flag}=false"),
                other => format!("--{flag} {}", value_as_text(other)),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(name: &str, field: &str, type_name: &str) -> LookupOption {
        LookupOption {
            name: name.into(),
            field_name: field.into(),
            type_name: type_name.into(),
        }
    }

    fn fields(pairs: &[(&str, Vec<LookupOption>)]) -> HashMap<String, Vec<LookupOption>> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn tokenizes_spaces_quotes_and_continuations() {
        assert_eq!(
            tokenize("rclone sync source:path dest:path"),
            ["rclone", "sync", "source:path", "dest:path"]
        );
        assert_eq!(
            tokenize("rclone sync \"source path\" 'dest path'"),
            ["rclone", "sync", "source path", "dest path"]
        );
        assert_eq!(
            tokenize("rclone sync \\\n  source:path \\\n  dest:path"),
            ["rclone", "sync", "source:path", "dest:path"]
        );
        assert_eq!(
            tokenize("rclone sync src: /backup/local_$(date +%Y-%m-%d_%H%M) --msg `hello world`"),
            [
                "rclone",
                "sync",
                "src:",
                "/backup/local_$(date +%Y-%m-%d_%H%M)",
                "--msg",
                "`hello world`"
            ]
        );
    }

    #[test]
    fn detects_macros() {
        assert!(has_macro("dest:/archive/pCloud_$(date +%Y-%m-%d)"));
        assert!(has_macro("dest:/archive/pCloud_`date`"));
        assert!(!has_macro("dest:/archive/pCloud_normal"));
    }

    #[test]
    fn parses_verb_paths_and_flags() {
        let bools = HashSet::from(["track-renames".into()]);
        let parsed = parse(
            "rclone sync source:path dest:path --max-delete 50 --track-renames",
            &bools,
        );
        assert_eq!(parsed.verb.as_deref(), Some("sync"));
        assert_eq!(parsed.source_path.as_deref(), Some("source:path"));
        assert_eq!(parsed.dest_path.as_deref(), Some("dest:path"));
        assert_eq!(parsed.flags.len(), 2);
        assert_eq!(parsed.flags[0].raw, "--max-delete 50");
        assert_eq!(parsed.flags[0].key, "max-delete");
        assert_eq!(parsed.flags[0].value, FlagValue::Text("50".into()));
        assert_eq!(parsed.flags[1].raw, "--track-renames");
        assert_eq!(parsed.flags[1].value, FlagValue::Bool(true));
    }

    #[test]
    fn parses_underscore_bools_equals_and_quotes() {
        let bools = HashSet::from(["track_renames".into()]);
        let parsed = parse("rclone sync source:path dest:path --track-renames", &bools);
        assert_eq!(parsed.flags[0].key, "track-renames");
        assert_eq!(parsed.flags[0].value, FlagValue::Bool(true));

        let parsed = parse(
            "rclone sync source:path dest:path --backup-dir=dest:/archive",
            &HashSet::new(),
        );
        assert_eq!(parsed.flags[0].key, "backup-dir");
        assert_eq!(
            parsed.flags[0].value,
            FlagValue::Text("dest:/archive".into())
        );

        let parsed = parse(
            "rclone sync source:path dest:path --exclude-from=\"/path/to/exclude-list.txt\"",
            &HashSet::new(),
        );
        assert_eq!(
            parsed.flags[0].value,
            FlagValue::Text("/path/to/exclude-list.txt".into())
        );
    }

    #[test]
    fn parses_mount_serve_and_aliases() {
        let parsed = parse(
            "rclone mount remote:path /mnt/point --vfs-cache-mode full",
            &HashSet::new(),
        );
        assert_eq!(parsed.verb.as_deref(), Some("mount"));
        assert_eq!(parsed.mount_subtype.as_deref(), Some("mount"));
        assert_eq!(parsed.source_path.as_deref(), Some("remote:path"));
        assert_eq!(parsed.dest_path.as_deref(), Some("/mnt/point"));

        let parsed2 = parse("rclone mount2 remote:path /mnt/point", &HashSet::new());
        assert_eq!(parsed2.mount_subtype.as_deref(), Some("mount2"));
        let parsed_c = parse("rclone cmount remote:path /mnt/point", &HashSet::new());
        assert_eq!(parsed_c.mount_subtype.as_deref(), Some("cmount"));
        let parsed_n = parse("rclone nfsmount remote:path /mnt/point", &HashSet::new());
        assert_eq!(parsed_n.mount_subtype.as_deref(), Some("nfsmount"));

        let parsed = parse(
            "rclone serve http remote:path --addr :8080",
            &HashSet::new(),
        );
        assert_eq!(parsed.verb.as_deref(), Some("serve"));
        assert_eq!(parsed.serve_subtype.as_deref(), Some("http"));
        assert_eq!(parsed.source_path.as_deref(), Some("remote:path"));
        assert!(parsed.dest_path.is_none());
        assert_eq!(parsed.flags[0].key, "addr");
        assert_eq!(parsed.flags[0].value, FlagValue::Text(":8080".into()));
    }

    #[test]
    fn classifies_mapped_unknown_and_coercion() {
        let table = build_lookup_table(
            &fields(&[(
                "sync",
                vec![
                    opt("max_delete", "MaxDelete", "int"),
                    opt("track_renames", "TrackRenames", "bool"),
                ],
            )]),
            None,
        );
        let parsed = ParsedCli {
            verb: Some("sync".into()),
            source_path: Some("src:".into()),
            dest_path: Some("dst:".into()),
            flags: vec![
                ParsedFlag {
                    raw: "--max-delete 50".into(),
                    key: "max-delete".into(),
                    value: FlagValue::Text("50".into()),
                    has_macro: false,
                },
                ParsedFlag {
                    raw: "--track-renames".into(),
                    key: "track-renames".into(),
                    value: FlagValue::Bool(true),
                    has_macro: false,
                },
                ParsedFlag {
                    raw: "--unknown-flag".into(),
                    key: "unknown-flag".into(),
                    value: FlagValue::Text("val".into()),
                    has_macro: false,
                },
            ],
            ..Default::default()
        };
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].status, FlagStatus::Mapped);
        assert_eq!(
            result.classified[0].field_name.as_deref(),
            Some("max_delete")
        );
        assert_eq!(result.classified[0].coerced_value, Some(json!(50)));
        assert_eq!(
            result.classified[1].field_name.as_deref(),
            Some("track_renames")
        );
        assert_eq!(result.classified[2].status, FlagStatus::Unknown);
    }

    #[test]
    fn coerces_uint_and_float() {
        let table = build_lookup_table(
            &fields(&[(
                "sync",
                vec![
                    opt("tpslimit", "TpsLimit", "float64"),
                    opt("tpslimit-burst", "TpsLimitBurst", "uint32"),
                ],
            )]),
            None,
        );
        let parsed = ParsedCli {
            verb: Some("sync".into()),
            source_path: Some("src:".into()),
            dest_path: Some("dst:".into()),
            flags: vec![
                ParsedFlag {
                    raw: "--tpslimit 10.5".into(),
                    key: "tpslimit".into(),
                    value: FlagValue::Text("10.5".into()),
                    has_macro: false,
                },
                ParsedFlag {
                    raw: "--tpslimit-burst 12".into(),
                    key: "tpslimit-burst".into(),
                    value: FlagValue::Text("12".into()),
                    has_macro: false,
                },
            ],
            ..Default::default()
        };
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].coerced_value, Some(json!(10.5)));
        assert_eq!(result.classified[1].coerced_value, Some(json!(12)));
    }

    #[test]
    fn matches_runtime_remote_prefixes() {
        let table = build_lookup_table(
            &fields(&[(
                "runtimeRemote",
                vec![
                    opt("provider", "Provider", "string"),
                    opt("chunk_size", "ChunkSize", "string"),
                ],
            )]),
            Some("s3"),
        );
        let parsed = ParsedCli {
            verb: Some("serve".into()),
            flags: vec![
                ParsedFlag {
                    raw: "--s3-provider AWS".into(),
                    key: "s3-provider".into(),
                    value: FlagValue::Text("AWS".into()),
                    has_macro: false,
                },
                ParsedFlag {
                    raw: "--s3-chunk-size 64M".into(),
                    key: "s3-chunk-size".into(),
                    value: FlagValue::Text("64M".into()),
                    has_macro: false,
                },
            ],
            ..Default::default()
        };
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].status, FlagStatus::Mapped);
        assert_eq!(result.classified[0].field_name.as_deref(), Some("provider"));
        assert_eq!(
            result.classified[1].field_name.as_deref(),
            Some("chunk_size")
        );
    }

    #[test]
    fn strips_comments_and_hyphen_values() {
        assert_eq!(
            tokenize("rclone sync src: dst: --filter \"- /**\" \\\n# comment\n --addr :8080"),
            ["rclone", "sync", "src:", "dst:", "--filter", "- /**", "--addr", ":8080"]
        );
        let parsed = parse(
            "rclone sync src: dst: --filter \"- /**\" --max-delete -10",
            &HashSet::new(),
        );
        assert_eq!(parsed.flags[0].value, FlagValue::Text("- /**".into()));
        assert_eq!(parsed.flags[1].value, FlagValue::Text("-10".into()));
    }

    #[test]
    fn does_not_consume_next_flag() {
        let parsed = parse("rclone sync src: dst: --verbose --dry-run", &HashSet::new());
        assert_eq!(parsed.flags.len(), 2);
        assert_eq!(parsed.flags[0].value, FlagValue::Bool(true));
        assert_eq!(parsed.flags[1].key, "dry-run");
    }

    #[test]
    fn parses_other_verbs_and_wrappers() {
        let check = parse("rclone check remote:path /local/path", &HashSet::new());
        assert_eq!(check.verb.as_deref(), Some("check"));
        let delete = parse("rclone delete remote:path/folder", &HashSet::new());
        assert_eq!(delete.verb.as_deref(), Some("delete"));
        let copyurl = parse(
            "rclone copyurl https://example.com/file.zip remote:path",
            &HashSet::new(),
        );
        assert_eq!(copyurl.verb.as_deref(), Some("copyurl"));
        assert_eq!(
            copyurl.source_path.as_deref(),
            Some("https://example.com/file.zip")
        );
        let purge = parse("rclone purge remote:path/trash", &HashSet::new());
        assert_eq!(purge.verb.as_deref(), Some("delete"));
        let sudo = parse("sudo /usr/bin/rclone sync src: dst:", &HashSet::new());
        assert_eq!(sudo.verb.as_deref(), Some("sync"));
        let wsl = parse("wsl rclone copy src: dst:", &HashSet::new());
        assert_eq!(wsl.verb.as_deref(), Some("copy"));
    }

    #[test]
    fn maps_short_aliases_and_negated_flags() {
        let table = build_lookup_table(
            &fields(&[(
                "sync",
                vec![
                    opt("progress", "Progress", "bool"),
                    opt("verbose", "Verbose", "int"),
                    opt("dry_run", "DryRun", "bool"),
                    opt("update", "Update", "bool"),
                    opt("copy_links", "CopyLinks", "bool"),
                    opt("checksum", "Checksum", "bool"),
                    opt("ignore_times", "IgnoreTimes", "bool"),
                    opt("traverse", "Traverse", "bool"),
                    opt("check_certificate", "CheckCertificate", "bool"),
                ],
            )]),
            None,
        );
        let parsed = parse(
            "rclone sync src: dst: -P -v -n -u -L -c -I",
            &HashSet::new(),
        );
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified.len(), 7);
        assert!(result
            .classified
            .iter()
            .all(|f| f.status == FlagStatus::Mapped));
        assert_eq!(result.classified[0].field_name.as_deref(), Some("progress"));
        assert_eq!(
            result.classified[6].field_name.as_deref(),
            Some("ignore_times")
        );

        let parsed = parse(
            "rclone sync src: dst: --no-traverse --no-check-certificate",
            &HashSet::new(),
        );
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].coerced_value, Some(json!(false)));
        assert_eq!(
            result.classified[1].field_name.as_deref(),
            Some("check_certificate")
        );
    }

    #[test]
    fn parses_explicit_bools_and_suffix() {
        let table = build_lookup_table(
            &fields(&[
                ("backend", vec![opt("fast_list", "FastList", "bool")]),
                (
                    "sync",
                    vec![
                        opt("dry_run", "DryRun", "bool"),
                        opt("suffix", "Suffix", "string"),
                    ],
                ),
            ]),
            None,
        );
        let parsed = parse(
            "rclone sync src: dst: --fast-list=false --dry-run=true",
            &HashSet::new(),
        );
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].coerced_value, Some(json!(false)));
        assert_eq!(result.classified[1].coerced_value, Some(json!(true)));

        let parsed = parse("rclone sync src: dst: --suffix -bak", &HashSet::new());
        assert_eq!(parsed.flags[0].value, FlagValue::Text("-bak".into()));
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].coerced_value, Some(json!("-bak")));
    }

    #[test]
    fn resolves_shared_copy_flags_to_detected_verb() {
        let checksum = opt("checksum", "Checksum", "bool");
        let backup = opt("backup_dir", "BackupDir", "string");
        let table = build_lookup_table(
            &fields(&[
                ("sync", vec![checksum.clone(), backup.clone()]),
                ("copy", vec![checksum.clone(), backup.clone()]),
                ("move", vec![checksum, backup]),
            ]),
            None,
        );
        let parsed = parse(
            "rclone sync src: dst: --checksum --backup-dir dst:_backup",
            &HashSet::from(["checksum".into()]),
        );
        let result = classify(&parsed, &table, None);
        assert_eq!(result.classified[0].flag_type.as_deref(), Some("sync"));
        assert_eq!(
            result.classified[1].field_name.as_deref(),
            Some("backup_dir")
        );
    }

    #[test]
    fn selected_apply_filters_flags_and_paths() {
        let result = ImportResult {
            verb: Some("sync".into()),
            source_path: Some("src:".into()),
            dest_path: Some("dst:".into()),
            classified: vec![
                ClassifiedFlag {
                    flag: ParsedFlag {
                        raw: "--transfers 8".into(),
                        key: "transfers".into(),
                        value: FlagValue::Text("8".into()),
                        has_macro: false,
                    },
                    status: FlagStatus::Mapped,
                    flag_type: Some("sync".into()),
                    field_name: Some("transfers".into()),
                    coerced_value: Some(json!(8)),
                    guidance: None,
                },
                ClassifiedFlag {
                    flag: ParsedFlag {
                        raw: "--unknown".into(),
                        key: "unknown".into(),
                        value: FlagValue::Bool(true),
                        has_macro: false,
                    },
                    status: FlagStatus::Unknown,
                    flag_type: None,
                    field_name: None,
                    coerced_value: None,
                    guidance: None,
                },
            ],
            ..Default::default()
        };
        let selected = HashSet::from(["transfers".into()]);
        let apply = selected_apply(&result, &selected, true, false, ProfileMode::Patch, "");
        assert_eq!(apply.flags.len(), 1);
        assert_eq!(apply.source_path.as_deref(), Some("src:"));
        assert!(apply.dest_path.is_none());
    }

    #[test]
    fn classifies_common_flags_without_options_info() {
        let result = import_cli_command(
            "rclone sync testdrive: /tmp/out --transfers 8 --max-delete 50 --unknown-flag demo --dry-run",
            &[],
            &[],
            None,
            Some("sync"),
        );
        let mapped: Vec<_> = result
            .classified
            .iter()
            .filter(|item| item.status == FlagStatus::Mapped)
            .map(|item| item.field_name.clone().unwrap_or_default())
            .collect();
        let unknown: Vec<_> = result
            .classified
            .iter()
            .filter(|item| item.status == FlagStatus::Unknown)
            .map(|item| item.flag.key.clone())
            .collect();
        assert!(mapped.iter().any(|name| name == "transfers"));
        assert!(mapped.iter().any(|name| name == "max_delete"));
        assert!(mapped.iter().any(|name| name == "dry_run"));
        assert_eq!(unknown, ["unknown-flag"]);
        assert_eq!(
            result
                .classified
                .iter()
                .find(|item| item.field_name.as_deref() == Some("dry_run"))
                .and_then(|item| item.coerced_value.clone()),
            Some(json!(true))
        );
    }
}
