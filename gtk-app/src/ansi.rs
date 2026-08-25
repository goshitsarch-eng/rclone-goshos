//! ANSI SGR → Pango markup, matching the Angular `ansiToHtml` pipe.

const COLORS: &[(&str, &str)] = &[
    ("30", "#000000"),
    ("31", "#ef5350"),
    ("32", "#4caf50"),
    ("33", "#ffca28"),
    ("34", "#42a5f5"),
    ("35", "#ab47bc"),
    ("36", "#26c6da"),
    ("37", "#e0e0e0"),
    ("90", "#757575"),
    ("91", "#e57373"),
    ("92", "#81c784"),
    ("93", "#fff176"),
    ("94", "#64b5f6"),
    ("95", "#ba68c8"),
    ("96", "#4dd0e1"),
    ("97", "#ffffff"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnsiSpan {
    pub text: String,
    pub color: Option<&'static str>,
}

pub fn color_for_code(code: &str) -> Option<&'static str> {
    COLORS
        .iter()
        .find(|(key, _)| *key == code)
        .map(|(_, color)| *color)
}

pub fn escape_pango(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn parse_ansi(input: &str) -> Vec<AnsiSpan> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut color: Option<&'static str> = None;
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some((consumed, codes)) = read_sgr(&input[i..]) {
                if !current.is_empty() {
                    spans.push(AnsiSpan {
                        text: std::mem::take(&mut current),
                        color,
                    });
                }
                for code in codes {
                    if code == "0" || code.is_empty() {
                        color = None;
                    } else if let Some(next) = color_for_code(&code) {
                        color = Some(next);
                    }
                }
                i += consumed;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\u{fffd}');
        current.push(ch);
        i += ch.len_utf8();
    }
    if !current.is_empty() {
        spans.push(AnsiSpan {
            text: current,
            color,
        });
    }
    spans
}

fn read_sgr(input: &str) -> Option<(usize, Vec<String>)> {
    let rest = input.strip_prefix("\u{1b}[")?;
    let end = rest.find('m')?;
    let body = &rest[..end];
    if !body.is_empty() && !body.bytes().all(|b| b.is_ascii_digit() || b == b';') {
        return None;
    }
    let codes = if body.is_empty() {
        vec!["0".into()]
    } else {
        body.split(';')
            .map(|part| part.to_string())
            .collect::<Vec<_>>()
    };
    Some((2 + end + 1, codes))
}

pub fn strip_ansi(input: &str) -> String {
    parse_ansi(input)
        .into_iter()
        .map(|span| span.text)
        .collect()
}

pub fn ansi_to_pango(input: &str) -> String {
    let mut out = String::new();
    for span in parse_ansi(input) {
        let escaped = escape_pango(&span.text);
        if let Some(color) = span.color {
            out.push_str(&format!("<span foreground=\"{color}\">{escaped}</span>"));
        } else {
            out.push_str(&escaped);
        }
    }
    out
}

pub fn has_ansi(input: &str) -> bool {
    input.contains('\u{1b}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_and_maps_colors() {
        let raw = "\u{1b}[31merror\u{1b}[0m ok \u{1b}[32mgood\u{1b}[0m";
        assert_eq!(strip_ansi(raw), "error ok good");
        assert_eq!(
            parse_ansi(raw),
            vec![
                AnsiSpan {
                    text: "error".into(),
                    color: Some("#ef5350"),
                },
                AnsiSpan {
                    text: " ok ".into(),
                    color: None,
                },
                AnsiSpan {
                    text: "good".into(),
                    color: Some("#4caf50"),
                },
            ]
        );
        let markup = ansi_to_pango(raw);
        assert!(markup.contains("foreground=\"#ef5350\">error</span>"));
        assert!(markup.contains("foreground=\"#4caf50\">good</span>"));
        assert!(markup.contains(" ok "));
    }

    #[test]
    fn escapes_markup_and_ignores_unknown_codes() {
        let raw = "\u{1b}[1ma <b>\u{1b}[0m &";
        assert_eq!(strip_ansi(raw), "a <b> &");
        assert_eq!(ansi_to_pango(raw), "a &lt;b&gt; &amp;");
        assert!(!has_ansi("plain"));
        assert!(has_ansi("\u{1b}[31mred"));
        assert_eq!(color_for_code("94"), Some("#64b5f6"));
        assert_eq!(color_for_code("1"), None);
    }

    #[test]
    fn handles_combined_and_empty_sgr() {
        assert_eq!(strip_ansi("\u{1b}[31;1mred\u{1b}[m"), "red");
        assert_eq!(
            parse_ansi("\u{1b}[31;1mred\u{1b}[m")[0].color,
            Some("#ef5350")
        );
        assert_eq!(ansi_to_pango(""), "");
        assert_eq!(escape_pango("<x>&"), "&lt;x&gt;&amp;");
    }
}
