const INVALID_NAME_CHARS: &str = r#"<>:"/\|?*"#;

pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if INVALID_NAME_CHARS.contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect()
}

pub fn get_sanitized_name(remote: &str, path: Option<&str>) -> String {
    let path_suffix = path
        .filter(|p| !p.is_empty() && *p != "/")
        .map(|p| {
            format!(
                " - {}",
                p.trim_start_matches('/').replace(['/', '\\'], " - ")
            )
        })
        .unwrap_or_default();

    let app_suffix = if cfg!(feature = "web-server") {
        " (RClone Manager Headless)"
    } else {
        " (RClone Manager)"
    };

    sanitize_name(&format!("{remote}{path_suffix}{app_suffix}"))
}

/// Escape a value that gets interpolated inside a double-quoted shell word —
/// the `"{remote}"` / `"{path}"` slots in the Send-to script and in `Exec=`
/// lines. Inside double quotes the shell still expands `$(…)`, `` `…` `` and
/// `\`, so a remote path such as `Photos$(id)` would otherwise run as code the
/// next time the menu entry is used.
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub fn escape_double_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' | '\r' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a value that gets interpolated into a double-quoted Python string
/// literal in the generated Nautilus `MenuProvider` extension. `exec_path`
/// already carries its own quotes, so without this the emitted module is not
/// valid Python and GNOME Files silently skips it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn escape_python_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn apply_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut content = template.to_string();
    for &(key, value) in replacements {
        content = content.replace(&format!("{{{key}}}"), value);
    }
    content
}

#[cfg(unix)]
pub fn get_home_dir() -> Result<std::path::PathBuf, String> {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .map_err(|_| "Could not find HOME environment variable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_double_quoted_neutralizes_shell_metacharacters() {
        // A remote folder named like this would otherwise run `touch` the next
        // time the generated Send-to menu entry is used.
        assert_eq!(
            escape_double_quoted("Photos$(touch /tmp/pwned)`id`\"x"),
            "Photos\\$(touch /tmp/pwned)\\`id\\`\\\"x"
        );
        assert_eq!(escape_double_quoted("Photos/2024 Trip"), "Photos/2024 Trip");
        assert_eq!(escape_double_quoted("a\nb"), "a b");
        assert_eq!(escape_double_quoted("a\\b"), "a\\\\b");
        assert_eq!(escape_double_quoted(""), "");
    }

    #[test]
    fn escape_python_string_keeps_quoted_exec_path_parseable() {
        // `exec_path` arrives already double-quoted; unescaped it produces
        // `exec_path = ""/opt/app""` which is a Python SyntaxError.
        assert_eq!(escape_python_string("\"/opt/app\""), "\\\"/opt/app\\\"");
        assert_eq!(escape_python_string("plain"), "plain");
        assert_eq!(escape_python_string("a\\b"), "a\\\\b");
        assert_eq!(escape_python_string("a\nb"), "a\\nb");
        assert_eq!(escape_python_string("a\tb"), "a\\tb");
    }

    #[test]
    fn sanitize_name_replaces_path_separators() {
        assert_eq!(sanitize_name("a/b:c"), "a-b-c");
        assert_eq!(sanitize_name("plain name"), "plain name");
    }
}
