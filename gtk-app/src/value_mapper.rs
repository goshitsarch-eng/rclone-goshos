//! Human ↔ rclone machine value mapping — port of Angular `RcloneValueMapperService`.

use crate::providers::ProviderOption;
use serde_json::{json, Value};

const INT_TYPES: &[&str] = &["int", "int64", "int32", "uint", "uint32", "uint64"];
const FLOAT_TYPES: &[&str] = &["float", "float32", "float64"];

const ARRAY_TYPES: &[&str] = &[
    "Encoding",
    "DumpFlags",
    "CommaSepList",
    "SpaceSepList",
    "Bits",
    "stringArray",
    "CommaSeparatedList",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Bool,
    Tristate,
    Numeric,
    Select,
    MultiSelect,
    Input,
}

pub fn is_array_type(type_name: &str) -> bool {
    ARRAY_TYPES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(type_name))
}

pub fn is_int_type(type_name: &str) -> bool {
    INT_TYPES.iter().any(|t| t.eq_ignore_ascii_case(type_name))
}

pub fn is_float_type(type_name: &str) -> bool {
    FLOAT_TYPES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(type_name))
}

pub fn control_kind(type_name: &str, exclusive: bool, example_count: usize) -> ControlKind {
    if type_name.eq_ignore_ascii_case("bool") {
        return ControlKind::Bool;
    }
    if type_name.eq_ignore_ascii_case("tristate") {
        return ControlKind::Tristate;
    }
    if is_int_type(type_name) || is_float_type(type_name) {
        return ControlKind::Numeric;
    }
    if is_array_type(type_name) && example_count > 0 {
        return ControlKind::MultiSelect;
    }
    if example_count > 0 && exclusive {
        return ControlKind::Select;
    }
    ControlKind::Input
}

pub fn machine_to_human(value: &Value, type_name: &str, fallback: &str) -> String {
    if value.is_null() {
        return fallback.to_string();
    }
    match type_name {
        "Duration" => nanoseconds_to_duration(value.as_i64().unwrap_or(0), fallback),
        "SizeSuffix" => bytes_to_size(value.as_i64().unwrap_or(0), fallback),
        "FileMode" => file_mode_to_string(value, fallback),
        "Tristate" => match parse_tristate(value) {
            Some(true) => "true".into(),
            Some(false) => "false".into(),
            None => "unset".into(),
        },
        _ => match value {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| Some(v.to_string()))
                })
                .collect::<Vec<_>>()
                .join(if type_name == "SpaceSepList" {
                    " "
                } else {
                    ","
                }),
            other => other.to_string().trim_matches('"').to_string(),
        },
    }
}

pub fn human_to_machine(value: &str, type_name: &str) -> Value {
    let trimmed = value.trim();
    if is_int_type(type_name) {
        return trimmed
            .parse::<i64>()
            .map(|n| json!(n))
            .unwrap_or_else(|_| json!(trimmed));
    }
    if is_float_type(type_name) {
        return trimmed
            .parse::<f64>()
            .map(|n| json!(n))
            .unwrap_or_else(|_| json!(trimmed));
    }
    match type_name {
        "bool" => match trimmed.to_ascii_lowercase().as_str() {
            "true" => json!(true),
            "false" => json!(false),
            _ => json!(trimmed),
        },
        "Tristate" => match parse_tristate(&json!(trimmed)) {
            Some(v) => json!(v),
            None => Value::Null,
        },
        "FileMode" => {
            if let Ok(n) = i64::from_str_radix(trimmed, 8) {
                json!(n)
            } else {
                json!(trimmed)
            }
        }
        "CommaSepList" | "Bits" | "Encoding" | "DumpFlags" => {
            json!(trimmed
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(","))
        }
        "SpaceSepList" => json!(trimmed
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")),
        "stringArray" | "[]string" | "List" => json!(trimmed
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()),
        _ => json!(trimmed),
    }
}

pub fn nanoseconds_to_duration(nanoseconds: i64, fallback: &str) -> String {
    if nanoseconds == 0 {
        return "0s".into();
    }
    if nanoseconds < 0 || nanoseconds >= 9_000_000_000_000_000_000 {
        return fallback.to_string();
    }
    let mut remaining = nanoseconds as u64;
    let mut result = String::new();
    let h = remaining / 3_600_000_000_000;
    if h > 0 {
        result.push_str(&format!("{h}h"));
        remaining %= 3_600_000_000_000;
    }
    let m = remaining / 60_000_000_000;
    if m > 0 || !result.is_empty() {
        result.push_str(&format!("{m}m"));
        remaining %= 60_000_000_000;
    }
    let s = remaining / 1_000_000_000;
    if s > 0 || !result.is_empty() {
        result.push_str(&format!("{s}s"));
        remaining %= 1_000_000_000;
    }
    if result.is_empty() {
        let ms = remaining / 1_000_000;
        if ms > 0 {
            result.push_str(&format!("{ms}ms"));
            remaining %= 1_000_000;
        }
        let us = remaining / 1_000;
        if us > 0 {
            result.push_str(&format!("{us}us"));
            remaining %= 1_000;
        }
        if remaining > 0 {
            result.push_str(&format!("{remaining}ns"));
        }
    }
    if result.is_empty() {
        "0s".into()
    } else {
        result
    }
}

pub fn bytes_to_size(bytes: i64, fallback: &str) -> String {
    if bytes == -1 {
        return "off".into();
    }
    if bytes == 0 {
        return "0".into();
    }
    if bytes < 0 {
        return fallback.to_string();
    }
    let units = [
        ("Pi", 1_125_899_906_842_624_i64),
        ("Ti", 1_099_511_627_776),
        ("Gi", 1_073_741_824),
        ("Mi", 1_048_576),
        ("Ki", 1_024),
    ];
    for (suffix, unit) in units {
        if bytes >= unit {
            if bytes % unit == 0 {
                return format!("{}{suffix}", bytes / unit);
            }
            let val = (bytes as f64 / unit as f64 * 1000.0).round() / 1000.0;
            return format!("{val}{suffix}");
        }
    }
    format!("{bytes}B")
}

fn file_mode_to_string(value: &Value, fallback: &str) -> String {
    let min_width = fallback.len().max(3);
    if let Some(n) = value.as_i64() {
        if n < 0 {
            return fallback.to_string();
        }
        return format!("{:0width$o}", n & 0o7777, width = min_width);
    }
    let s = value.as_str().unwrap_or("").trim();
    if s.is_empty() {
        fallback.to_string()
    } else {
        format!("{s:0>min_width$}")
    }
}

pub fn parse_tristate(value: &Value) -> Option<bool> {
    match value {
        Value::Null => None,
        Value::Bool(b) => Some(*b),
        Value::Object(obj) => {
            if obj.get("Valid").and_then(|v| v.as_bool()) == Some(true) {
                obj.get("Value").and_then(|v| v.as_bool())
            } else {
                None
            }
        }
        Value::String(s) => match s.to_ascii_lowercase().trim() {
            "" | "unset" | "null" => None,
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

pub fn is_default_display(value: &str, option: &ProviderOption) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    if !option.default_str.is_empty() && trimmed.eq_ignore_ascii_case(&option.default_str) {
        return true;
    }
    if option.default.is_null() {
        return false;
    }
    machine_to_human(&option.default, &option.type_name, "").eq_ignore_ascii_case(trimmed)
}

/// rclone `Examples[].Provider` / field `Provider` rules (`!a,b` means not those).
pub fn matches_provider_rule(rule: &str, provider: &str) -> bool {
    if rule.is_empty() {
        return true;
    }
    if provider.is_empty() {
        return false;
    }
    let negated = rule.starts_with('!');
    let parts = if negated { &rule[1..] } else { rule };
    let hit = parts.split(',').map(str::trim).any(|p| p == provider);
    if negated {
        !hit
    } else {
        hit
    }
}

pub fn filter_examples(
    examples: &[(String, String)],
    example_providers: &[String],
    provider: &str,
) -> Vec<(String, String)> {
    examples
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let rule = example_providers.get(*i).map(String::as_str).unwrap_or("");
            matches_provider_rule(rule, provider)
        })
        .map(|(_, pair)| pair.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderOption;

    fn option(type_name: &str, default_str: &str) -> ProviderOption {
        ProviderOption {
            name: "field".into(),
            help: String::new(),
            required: false,
            advanced: false,
            is_password: false,
            exclusive: false,
            type_name: type_name.into(),
            default: Value::Null,
            default_str: default_str.into(),
            value: Value::Null,
            value_str: String::new(),
            examples: vec![],
            provider: String::new(),
            example_providers: vec![],
        }
    }

    #[test]
    fn maps_ints_bools_and_lists() {
        assert_eq!(human_to_machine("12", "int"), json!(12));
        assert_eq!(human_to_machine("true", "bool"), json!(true));
        assert_eq!(human_to_machine("a, b", "CommaSepList"), json!("a,b"));
        assert_eq!(human_to_machine("a  b", "SpaceSepList"), json!("a b"));
        assert_eq!(
            human_to_machine("one,two", "stringArray"),
            json!(["one", "two"])
        );
        assert_eq!(human_to_machine("755", "FileMode"), json!(493));
        assert!(human_to_machine("unset", "Tristate").is_null());
    }

    #[test]
    fn converts_duration_and_size() {
        assert_eq!(nanoseconds_to_duration(3_600_000_000_000, "off"), "1h0m0s");
        assert_eq!(nanoseconds_to_duration(5_000_000, "off"), "5ms");
        assert_eq!(bytes_to_size(1_048_576, ""), "1Mi");
        assert_eq!(bytes_to_size(-1, ""), "off");
        assert_eq!(machine_to_human(&json!(1024), "SizeSuffix", ""), "1Ki");
        assert_eq!(
            machine_to_human(&json!(60_000_000_000_i64), "Duration", "off"),
            "1m0s"
        );
    }

    #[test]
    fn control_kinds_match_angular() {
        assert_eq!(control_kind("bool", false, 0), ControlKind::Bool);
        assert_eq!(control_kind("Tristate", false, 0), ControlKind::Tristate);
        assert_eq!(control_kind("int64", false, 0), ControlKind::Numeric);
        assert_eq!(control_kind("string", true, 3), ControlKind::Select);
        assert_eq!(control_kind("string", false, 3), ControlKind::Input);
        assert_eq!(control_kind("Encoding", false, 6), ControlKind::MultiSelect);
        assert_eq!(control_kind("CommaSepList", false, 0), ControlKind::Input);
        assert!(is_array_type("DumpFlags"));
    }

    #[test]
    fn provider_rules_and_defaults() {
        assert!(matches_provider_rule("", "AWS"));
        assert!(matches_provider_rule("AWS,GCS", "AWS"));
        assert!(!matches_provider_rule("AWS", "GCS"));
        assert!(matches_provider_rule("!AWS", "GCS"));
        assert!(!matches_provider_rule("!AWS", "AWS"));
        let opt = option("string", "off");
        assert!(is_default_display("", &opt));
        assert!(is_default_display("off", &opt));
        assert!(!is_default_display("on", &opt));
        let filtered = filter_examples(
            &[("a".into(), "A".into()), ("b".into(), "B".into())],
            &["AWS".into(), "GCS".into()],
            "AWS",
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "a");
    }
}
