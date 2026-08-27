//! Provider-field and form validators — port of Angular `ValidatorRegistryService`.

use crate::providers::ProviderOption;

fn empty_or_default(value: &str, default: Option<&str>) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    default.is_some_and(|d| trimmed.eq_ignore_ascii_case(d))
}

fn is_int(value: &str) -> bool {
    let value = value.trim();
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn is_float(value: &str) -> bool {
    let value = value.trim();
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || digits.starts_with('.') || digits.ends_with('.') {
        return false;
    }
    let mut seen_dot = false;
    digits.chars().all(|c| {
        if c == '.' {
            if seen_dot {
                return false;
            }
            seen_dot = true;
            true
        } else {
            c.is_ascii_digit()
        }
    })
}

fn is_duration(value: &str) -> bool {
    let mut rest = value.trim();
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let digits_end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if digits_end == 0 || !is_float(&rest[..digits_end]) {
            return false;
        }
        rest = &rest[digits_end..];
        let unit = if rest.starts_with("ns") {
            2
        } else if rest.starts_with("us") || rest.starts_with("µs") || rest.starts_with("ms") {
            2
        } else if rest.starts_with('s')
            || rest.starts_with('m')
            || rest.starts_with('h')
            || rest.starts_with('d')
        {
            1
        } else {
            return false;
        };
        rest = &rest[unit..];
    }
    true
}

fn is_size_suffix(value: &str) -> bool {
    let value = value.trim();
    let split = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(value.len());
    if split == 0 || !is_float(&value[..split]) {
        return false;
    }
    matches!(
        value[split..].as_bytes(),
        b"" | b"b"
            | b"B"
            | b"k"
            | b"K"
            | b"Ki"
            | b"M"
            | b"Mi"
            | b"G"
            | b"Gi"
            | b"T"
            | b"Ti"
            | b"P"
            | b"Pi"
            | b"E"
            | b"Ei"
    )
}

fn is_file_mode(value: &str) -> bool {
    let value = value.trim();
    (3..=4).contains(&value.len()) && value.chars().all(|c| ('0'..='7').contains(&c))
}

fn is_iso_time(value: &str) -> bool {
    let value = value.trim();
    if chrono::DateTime::parse_from_rfc3339(value).is_ok() {
        return true;
    }
    // YYYY-MM-DDTHH:mm with optional seconds / timezone
    let bytes = value.as_bytes();
    if bytes.len() < 16
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
    {
        return false;
    }
    bytes[..4].iter().all(|c| c.is_ascii_digit())
        && bytes[5..7].iter().all(|c| c.is_ascii_digit())
        && bytes[8..10].iter().all(|c| c.is_ascii_digit())
        && bytes[11..13].iter().all(|c| c.is_ascii_digit())
        && bytes[14..16].iter().all(|c| c.is_ascii_digit())
}

fn is_url(value: &str) -> bool {
    let value = value.trim();
    (value.starts_with("http://") || value.starts_with("https://"))
        && !value.contains(' ')
        && !value.contains(';')
        && value.len() > 8
}

fn is_bandwidth_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let split = token
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(token.len());
    if split == 0 || !is_float(&token[..split]) {
        return false;
    }
    matches!(
        token[split..].to_ascii_lowercase().as_str(),
        "" | "k" | "m" | "g" | "ki" | "mi" | "gi"
    )
}

fn is_bandwidth_side(side: &str) -> bool {
    !side.is_empty() && side.split('|').all(is_bandwidth_token)
}

fn is_bandwidth(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    let mut parts = value.splitn(2, ':');
    let first = parts.next().unwrap_or("");
    let second = parts.next();
    is_bandwidth_side(first) && second.is_none_or(is_bandwidth_side)
}

fn is_remote_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | '@' | ' '))
}

fn is_unix_abs(value: &str) -> bool {
    value.starts_with('/') && !value.contains('\0')
}

pub fn validate_integer(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default) || is_int(value) {
        Ok(())
    } else {
        Err("Must be a valid integer".into())
    }
}

pub fn validate_float(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default) || is_float(value) {
        Ok(())
    } else {
        Err("Must be a valid decimal number".into())
    }
}

pub fn validate_duration(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default) || is_duration(value) {
        Ok(())
    } else {
        Err("Invalid duration format. Use: 1h30m45s, 5m, 1h".into())
    }
}

pub fn validate_size_suffix(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default)
        || value.trim().eq_ignore_ascii_case("off")
        || is_size_suffix(value)
    {
        Ok(())
    } else {
        Err("Invalid size format. Use: 100Ki, 16Mi, 1Gi, or \"off\"".into())
    }
}

pub fn validate_time(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default) || is_iso_time(value) {
        Ok(())
    } else {
        Err("Invalid datetime format. Use ISO 8601: YYYY-MM-DDTHH:mm:ssZ".into())
    }
}

pub fn validate_file_mode(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default) || is_file_mode(value) {
        Ok(())
    } else {
        Err("Must be octal format (3-4 digits, each 0-7). Example: 755".into())
    }
}

pub fn validate_bandwidth(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || is_bandwidth(value) {
        Ok(())
    } else {
        Err("Invalid bandwidth format. Use: 10M, 1G, 100K or combinations".into())
    }
}

/// Dashboard / Flow apply: empty, `off`, `0`, and `off:off` mean unlimited.
pub fn validate_bandwidth_limit(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("off")
        || trimmed == "0"
        || trimmed.eq_ignore_ascii_case("off:off")
    {
        return Ok(());
    }
    validate_bandwidth(value)
}

pub fn validate_bw_timetable(value: &str, default: Option<&str>) -> Result<(), String> {
    if empty_or_default(value, default) {
        return Ok(());
    }
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("off") {
        return Ok(());
    }
    let has_table = trimmed.contains(',') || trimmed.contains('-') || trimmed.contains(':');
    if is_size_suffix(trimmed) || has_table {
        Ok(())
    } else {
        Err("Invalid bandwidth format".into())
    }
}

pub fn validate_url(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || is_url(value) {
        Ok(())
    } else {
        Err("All URLs must be valid and start with http:// or https://".into())
    }
}

/// Validate every non-empty entry of a connectivity-URL array setting.
pub fn validate_url_list(items: &[String]) -> Result<(), String> {
    for item in items {
        validate_url(item)?;
    }
    Ok(())
}

pub fn validate_absolute_path(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    let ok = if cfg!(windows) {
        let v = value.trim();
        v.len() >= 2 && v.as_bytes()[0].is_ascii_alphabetic() && v.as_bytes()[1] == b':'
            || v.starts_with("\\\\")
    } else {
        is_unix_abs(value)
    };
    if ok {
        Ok(())
    } else {
        Err("Path must be absolute".into())
    }
}

pub fn validate_password(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    if value.len() < 3 {
        return Err("Password must be at least 3 characters".into());
    }
    if value.contains('\'') || value.contains('"') {
        return Err("Password cannot contain quotes".into());
    }
    Ok(())
}

pub fn validate_remote_name(
    value: &str,
    existing: &[String],
    editing: Option<&str>,
) -> Result<(), String> {
    let raw = value;
    let value = value.trim();
    if value.is_empty() {
        return Err("wizards.remoteConfig.remoteNameRequired".into());
    }
    if !is_remote_name(value) {
        return Err("wizards.remoteConfig.invalidChars".into());
    }
    if value.starts_with('-') || value.starts_with(' ') {
        return Err("wizards.remoteConfig.invalidStart".into());
    }
    if raw.ends_with(' ') {
        return Err("wizards.remoteConfig.invalidEnd".into());
    }
    if editing.is_some_and(|current| current.eq_ignore_ascii_case(value)) {
        return Ok(());
    }
    if existing
        .iter()
        .any(|name| name.trim().eq_ignore_ascii_case(value))
    {
        return Err("wizards.remoteConfig.nameTaken".into());
    }
    Ok(())
}

pub fn validate_enum(value: &str, allowed: &[String]) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    if allowed
        .iter()
        .any(|item| item.eq_ignore_ascii_case(value.trim()))
    {
        Ok(())
    } else {
        Err(format!("Must be one of: {}", allowed.join(", ")))
    }
}

/// Validate a typed rclone flag / provider value (empty values are accepted).
pub fn validate_typed_value(
    type_name: &str,
    value: &str,
    exclusive: bool,
    examples: &[(String, String)],
    default: Option<&str>,
) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    match type_name.to_ascii_lowercase().as_str() {
        "int" | "int64" | "uint32" | "uint64" => validate_integer(value, default)?,
        "float" | "float64" => validate_float(value, default)?,
        "duration" => validate_duration(value, default)?,
        "sizesuffix" | "size" => validate_size_suffix(value, default)?,
        "time" => validate_time(value, default)?,
        "filemode" | "bits" => validate_file_mode(value, default)?,
        "bwtimetable" => validate_bw_timetable(value, default)?,
        _ => {}
    }
    if exclusive && !examples.is_empty() {
        let allowed: Vec<String> = examples.iter().map(|(v, _)| v.clone()).collect();
        validate_enum(value, &allowed)?;
    }
    Ok(())
}

/// Type-check a non-empty flag editor value (no exclusive examples).
pub fn validate_flag_text(type_name: &str, value: &str) -> Result<(), String> {
    validate_typed_value(type_name, value, false, &[], None)
}

/// Validate a rclone provider option using its `Type` and exclusive examples.
pub fn validate_option(option: &ProviderOption, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        if option.required {
            return Err(format!("{} is required", option.name));
        }
        return Ok(());
    }
    if option.is_password {
        return validate_password(value);
    }
    let default = if option.default_str.is_empty() {
        None
    } else {
        Some(option.default_str.as_str())
    };
    validate_typed_value(
        &option.type_name,
        value,
        option.exclusive,
        &option.examples,
        default,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ProviderOption;
    use serde_json::Value;

    fn option(type_name: &str, required: bool) -> ProviderOption {
        ProviderOption {
            name: "field".into(),
            help: String::new(),
            required,
            advanced: false,
            is_password: false,
            exclusive: false,
            type_name: type_name.into(),
            default: Value::Null,
            default_str: String::new(),
            value: Value::Null,
            value_str: String::new(),
            examples: vec![],
            provider: String::new(),
            example_providers: vec![],
        }
    }

    #[test]
    fn integers_and_floats() {
        assert!(validate_integer("42", None).is_ok());
        assert!(validate_integer("-3", None).is_ok());
        assert!(validate_integer("3.2", None).is_err());
        assert!(validate_float("3.2", None).is_ok());
        assert!(validate_integer("off", Some("off")).is_ok());
    }

    #[test]
    fn duration_size_and_time() {
        assert!(validate_duration("1h30m", None).is_ok());
        assert!(validate_duration("5x", None).is_err());
        assert!(validate_size_suffix("16Mi", None).is_ok());
        assert!(validate_size_suffix("off", None).is_ok());
        assert!(validate_size_suffix("nope", None).is_err());
        assert!(validate_time("2024-01-02T03:04:05Z", None).is_ok());
        assert!(validate_time("not-a-time", None).is_err());
    }

    #[test]
    fn bandwidth_url_path_and_password() {
        assert!(validate_bandwidth("10M").is_ok());
        assert!(validate_bandwidth("1G:100K").is_ok());
        assert!(validate_bandwidth("xyz").is_err());
        assert!(validate_bandwidth("off").is_err());
        assert!(validate_bandwidth_limit("off").is_ok());
        assert!(validate_bandwidth_limit("0").is_ok());
        assert!(validate_bandwidth_limit("off:off").is_ok());
        assert!(validate_bandwidth_limit("").is_ok());
        assert!(validate_bandwidth_limit("10M").is_ok());
        assert!(validate_bandwidth_limit("xyz").is_err());
        assert!(validate_flag_text("int", "").is_ok());
        assert!(validate_flag_text("int", "12").is_ok());
        assert!(validate_flag_text("int", "nope").is_err());
        assert!(validate_flag_text("Duration", "5m").is_ok());
        assert!(validate_flag_text("Duration", "soon").is_err());
        assert!(validate_typed_value("string", "any", false, &[], None).is_ok());
        assert!(validate_url("https://example.com/a").is_ok());
        assert!(validate_url("ftp://x").is_err());
        assert!(validate_url_list(&["".into(), "https://example.com".into()]).is_ok());
        assert!(validate_url_list(&["https://ok".into(), "ftp://no".into()]).is_err());
        assert!(validate_absolute_path("/tmp/out").is_ok());
        assert!(validate_absolute_path("relative").is_err());
        assert!(validate_password("secret").is_ok());
        assert!(validate_password("ab").is_err());
        assert!(validate_password("bad\"pass").is_err());
    }

    #[test]
    fn remote_name_rules() {
        assert!(validate_remote_name("drive", &[], None).is_ok());
        assert!(validate_remote_name("my remote", &[], None).is_ok());
        assert!(validate_remote_name("-bad", &[], None).is_err());
        assert!(validate_remote_name("bad/", &[], None).is_err());
        assert!(validate_remote_name("drive ", &[], None).is_err());
        assert_eq!(
            validate_remote_name("Drive", &["drive".into()], None).unwrap_err(),
            "wizards.remoteConfig.nameTaken"
        );
        assert!(validate_remote_name("Drive", &["drive".into()], Some("drive")).is_ok());
        assert_eq!(
            validate_remote_name("", &[], None).unwrap_err(),
            "wizards.remoteConfig.remoteNameRequired"
        );
        assert_eq!(
            validate_remote_name("-bad", &[], None).unwrap_err(),
            "wizards.remoteConfig.invalidStart"
        );
        assert_eq!(
            validate_remote_name("drive ", &[], None).unwrap_err(),
            "wizards.remoteConfig.invalidEnd"
        );
    }

    #[test]
    fn provider_option_required_and_type() {
        let required = option("string", true);
        assert!(validate_option(&required, "").is_err());
        let int = option("int", false);
        assert!(validate_option(&int, "12").is_ok());
        assert!(validate_option(&int, "nope").is_err());
        let mut exclusive = option("string", false);
        exclusive.exclusive = true;
        exclusive.examples = vec![("s3".into(), "S3".into()), ("b2".into(), "B2".into())];
        assert!(validate_option(&exclusive, "s3").is_ok());
        assert!(validate_option(&exclusive, "gcs").is_err());
    }
}
