//! Lightweight syntax highlighting for the in-app text viewer.
//!
//! Language coverage matches the Angular file-viewer CodeMirror map:
//! js/ts, json, css/scss/sass, html/xml, python, rust, yaml, sql, go,
//! sh/bash/zsh, and markdown source.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
}

impl TokenKind {
    pub fn tag_name(self) -> &'static str {
        match self {
            Self::Keyword => "syn-keyword",
            Self::String => "syn-string",
            Self::Comment => "syn-comment",
            Self::Number => "syn-number",
        }
    }

    pub fn color(self) -> &'static str {
        match self {
            Self::Keyword => "#3584e4",
            Self::String => "#26a269",
            Self::Comment => "#9a9996",
            Self::Number => "#e66100",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

pub fn language_from_name(name: &str) -> Option<&'static str> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let ext = base.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "json" => "json",
        "toml" => "toml",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "sh" | "bash" | "zsh" => "shell",
        "md" | "markdown" => "markdown",
        "html" | "htm" | "xml" => "markup",
        "css" | "scss" | "sass" => "css",
        "yml" | "yaml" => "yaml",
        "go" => "go",
        "sql" => "sql",
        _ => return None,
    })
}

pub fn highlight(text: &str, lang: &str) -> Vec<Span> {
    if lang == "markdown" {
        return highlight_markdown(text);
    }
    let keywords = keywords_for(lang);
    let case_insensitive = lang == "sql";
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if is_line_comment(&chars, i, lang) {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if is_block_comment_start(&chars, i, lang) {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i = chars.len();
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if lang == "json" && chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            // jsonc-style, treat as comment
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::Comment,
            });
            continue;
        }
        if matches!(chars[i], '"' | '\'') {
            let quote = chars[i];
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if lang == "sql"
                    && quote == '\''
                    && chars[i] == '\''
                    && i + 1 < chars.len()
                    && chars[i + 1] == '\''
                {
                    i += 2;
                    continue;
                }
                if chars[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::String,
            });
            continue;
        }
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::Number,
            });
            continue;
        }
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let is_keyword = if case_insensitive {
                keywords.iter().any(|k| k.eq_ignore_ascii_case(&word))
            } else {
                keywords.iter().any(|k| *k == word)
            };
            if is_keyword {
                spans.push(Span {
                    start,
                    end: i,
                    kind: TokenKind::Keyword,
                });
            }
            continue;
        }
        i += 1;
    }
    spans
}

fn highlight_markdown(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < chars.len() {
        if chars[i] == '\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if line_start {
            let mut j = i;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '#' {
                let start = j;
                while j < chars.len() && chars[j] == '#' {
                    j += 1;
                }
                if j == chars.len() || matches!(chars[j], ' ' | '\t' | '\n') {
                    while j < chars.len() && chars[j] != '\n' {
                        j += 1;
                    }
                    spans.push(Span {
                        start,
                        end: j,
                        kind: TokenKind::Keyword,
                    });
                    i = j;
                    continue;
                }
            }
            if j < chars.len() && chars[j] == '>' {
                let start = j;
                while j < chars.len() && chars[j] != '\n' {
                    j += 1;
                }
                spans.push(Span {
                    start,
                    end: j,
                    kind: TokenKind::Comment,
                });
                i = j;
                continue;
            }
            if j + 2 < chars.len() && chars[j] == '`' && chars[j + 1] == '`' && chars[j + 2] == '`'
            {
                let start = j;
                j += 3;
                while j + 2 < chars.len()
                    && !(chars[j] == '`' && chars[j + 1] == '`' && chars[j + 2] == '`')
                {
                    j += 1;
                }
                if j + 2 < chars.len() {
                    j += 3;
                } else {
                    j = chars.len();
                }
                spans.push(Span {
                    start,
                    end: j,
                    kind: TokenKind::String,
                });
                i = j;
                line_start = false;
                continue;
            }
            line_start = false;
            i = j;
            if i == chars.len() {
                break;
            }
        }
        if chars[i] == '`' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '`' && chars[i] != '\n' {
                i += 1;
            }
            if i < chars.len() && chars[i] == '`' {
                i += 1;
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::String,
            });
            continue;
        }
        if chars[i] == '[' {
            let start = i;
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ']' && chars[j] != '\n' {
                j += 1;
            }
            if j < chars.len() && chars[j] == ']' && j + 1 < chars.len() && chars[j + 1] == '(' {
                j += 2;
                while j < chars.len() && chars[j] != ')' && chars[j] != '\n' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ')' {
                    j += 1;
                    spans.push(Span {
                        start,
                        end: j,
                        kind: TokenKind::String,
                    });
                    i = j;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '*') {
                if chars[i] == '\n' {
                    break;
                }
                i += 1;
            }
            if i + 1 < chars.len() && chars[i] == '*' && chars[i + 1] == '*' {
                i += 2;
            }
            spans.push(Span {
                start,
                end: i,
                kind: TokenKind::Keyword,
            });
            continue;
        }
        i += 1;
    }
    spans
}

fn is_line_comment(chars: &[char], i: usize, lang: &str) -> bool {
    match lang {
        "python" | "toml" | "yaml" | "shell" => chars[i] == '#',
        "rust" | "javascript" | "typescript" | "go" | "css" | "scss" => {
            chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/'
        }
        "sql" => chars[i] == '-' && i + 1 < chars.len() && chars[i + 1] == '-',
        "markdown" => false,
        _ => chars[i] == '#' || (chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/'),
    }
}

fn is_block_comment_start(chars: &[char], i: usize, lang: &str) -> bool {
    matches!(
        lang,
        "rust" | "javascript" | "typescript" | "go" | "css" | "scss" | "sql"
    ) && chars[i] == '/'
        && i + 1 < chars.len()
        && chars[i + 1] == '*'
}

fn keywords_for(lang: &str) -> &'static [&'static str] {
    match lang {
        "rust" => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "use", "mod", "match", "if",
            "else", "for", "while", "loop", "return", "const", "static", "async", "await", "true",
            "false", "self", "Self", "crate", "super", "where", "type", "trait",
        ],
        "python" => &[
            "def", "class", "import", "from", "as", "if", "elif", "else", "for", "while", "return",
            "yield", "try", "except", "finally", "with", "lambda", "True", "False", "None", "and",
            "or", "not", "in", "is", "pass", "break", "continue",
        ],
        "javascript" | "typescript" => &[
            "function",
            "const",
            "let",
            "var",
            "class",
            "return",
            "if",
            "else",
            "for",
            "while",
            "import",
            "export",
            "from",
            "async",
            "await",
            "true",
            "false",
            "null",
            "undefined",
            "new",
            "this",
            "typeof",
            "in",
            "of",
        ],
        "go" => &[
            "func",
            "package",
            "import",
            "type",
            "struct",
            "interface",
            "var",
            "const",
            "if",
            "else",
            "for",
            "range",
            "return",
            "go",
            "defer",
            "map",
            "chan",
            "true",
            "false",
            "nil",
        ],
        "json" => &["true", "false", "null"],
        "toml" | "yaml" => &["true", "false", "null"],
        "shell" => &[
            "if", "then", "else", "fi", "for", "do", "done", "while", "case", "esac", "function",
            "return", "export", "local",
        ],
        "css" | "scss" => &["important", "from", "to", "and", "or", "not"],
        "sql" => &[
            "select",
            "from",
            "where",
            "insert",
            "into",
            "values",
            "update",
            "set",
            "delete",
            "create",
            "alter",
            "drop",
            "table",
            "index",
            "view",
            "join",
            "left",
            "right",
            "inner",
            "outer",
            "full",
            "on",
            "and",
            "or",
            "not",
            "null",
            "as",
            "in",
            "is",
            "like",
            "between",
            "exists",
            "distinct",
            "order",
            "group",
            "by",
            "having",
            "limit",
            "offset",
            "union",
            "all",
            "case",
            "when",
            "then",
            "else",
            "end",
            "with",
            "primary",
            "key",
            "foreign",
            "references",
            "default",
            "constraint",
            "unique",
            "check",
            "true",
            "false",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages() {
        assert_eq!(language_from_name("main.rs"), Some("rust"));
        assert_eq!(language_from_name("app.ts"), Some("typescript"));
        assert_eq!(language_from_name("notes.txt"), None);
        assert_eq!(language_from_name("query.sql"), Some("sql"));
        assert_eq!(language_from_name("init.zsh"), Some("shell"));
        assert_eq!(language_from_name("theme.sass"), Some("css"));
        assert_eq!(language_from_name("theme.scss"), Some("css"));
        assert_eq!(language_from_name("README.md"), Some("markdown"));
        assert_eq!(
            language_from_name("testdrive:Photos/query.sql"),
            Some("sql")
        );
    }

    #[test]
    fn highlights_rust_keywords_and_strings() {
        let spans = highlight("fn main() { let x = \"hi\"; // c\n}", "rust");
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword));
        assert!(spans.iter().any(|s| s.kind == TokenKind::String));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
    }

    #[test]
    fn highlights_json_numbers() {
        let spans = highlight("{\"n\": 12}", "json");
        assert!(spans.iter().any(|s| s.kind == TokenKind::Number));
        assert!(spans.iter().any(|s| s.kind == TokenKind::String));
    }

    #[test]
    fn highlights_sql_keywords_comments_and_strings() {
        let src =
            "SELECT name FROM users WHERE id = 7 -- lookup\n/* block */ AND name = 'O''Brien'";
        let spans = highlight(src, "sql");
        let keywords: Vec<String> = spans
            .iter()
            .filter(|s| s.kind == TokenKind::Keyword)
            .map(|s| src[byte_range(src, s.start, s.end)].to_ascii_uppercase())
            .collect();
        assert!(keywords.iter().any(|k| k == "SELECT"));
        assert!(keywords.iter().any(|k| k == "FROM"));
        assert!(keywords.iter().any(|k| k == "WHERE"));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
        assert!(spans.iter().any(|s| s.kind == TokenKind::String));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Number));
        assert!(
            spans
                .iter()
                .filter(|s| s.kind == TokenKind::Comment)
                .count()
                >= 2
        );
    }

    #[test]
    fn highlights_sql_case_insensitively() {
        let src = "select * From T";
        let spans = highlight(src, "sql");
        let words: Vec<String> = spans
            .iter()
            .filter(|s| s.kind == TokenKind::Keyword)
            .map(|s| src[byte_range(src, s.start, s.end)].to_string())
            .collect();
        assert_eq!(words, vec!["select".to_string(), "From".to_string()]);
    }

    #[test]
    fn highlights_markdown_source() {
        let src =
            "# Title\n> quote\nSee `code` and [link](https://ex) plus **bold**\n```\nblock\n```\n";
        let spans = highlight(src, "markdown");
        assert!(spans.iter().any(|s| s.kind == TokenKind::Keyword));
        assert!(spans.iter().any(|s| s.kind == TokenKind::Comment));
        assert!(spans.iter().any(|s| s.kind == TokenKind::String));
        let heading = spans
            .iter()
            .find(|s| s.kind == TokenKind::Keyword)
            .expect("heading");
        assert_eq!(&src[byte_range(src, heading.start, heading.end)], "# Title");
    }

    #[test]
    fn highlights_shell_from_zsh_and_sass_via_css() {
        let shell = highlight("if true; then echo hi; fi # c\n", "shell");
        assert!(shell.iter().any(|s| s.kind == TokenKind::Keyword));
        assert!(shell.iter().any(|s| s.kind == TokenKind::Comment));
        let css = highlight("/* sass */ .a { color: #fff; }", "css");
        assert!(css.iter().any(|s| s.kind == TokenKind::Comment));
    }

    fn byte_range(src: &str, start: usize, end: usize) -> std::ops::Range<usize> {
        let mut chars = src.char_indices();
        let start_b = chars.nth(start).map(|(i, _)| i).unwrap_or(src.len());
        if end <= start {
            return start_b..start_b;
        }
        let end_b = src[start_b..]
            .char_indices()
            .nth(end - start)
            .map(|(i, _)| start_b + i)
            .unwrap_or(src.len());
        start_b..end_b
    }
}
