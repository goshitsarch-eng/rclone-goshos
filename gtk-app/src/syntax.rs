//! Lightweight syntax highlighting for the in-app text viewer.

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
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "json" => "json",
        "toml" => "toml",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "sh" | "bash" => "shell",
        "md" | "markdown" => "markdown",
        "html" | "htm" | "xml" => "markup",
        "css" | "scss" => "css",
        "yml" | "yaml" => "yaml",
        "go" => "go",
        _ => return None,
    })
}

pub fn highlight(text: &str, lang: &str) -> Vec<Span> {
    let keywords = keywords_for(lang);
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
            if keywords.iter().any(|k| *k == word) {
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

fn is_line_comment(chars: &[char], i: usize, lang: &str) -> bool {
    match lang {
        "python" | "toml" | "yaml" | "shell" => chars[i] == '#',
        "rust" | "javascript" | "typescript" | "go" | "css" | "scss" => {
            chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/'
        }
        "markdown" => false,
        _ => chars[i] == '#' || (chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/'),
    }
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
}
