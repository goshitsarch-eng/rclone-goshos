//! Multi-rename preview matching the Angular `MultiRenameModalComponent`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameMode {
    Template,
    Replace,
}

#[derive(Debug, Clone)]
pub struct RenamePlan {
    pub mode: RenameMode,
    pub template: String,
    pub counter_start: i64,
    pub counter_step: i64,
    pub counter_padding: usize,
    pub find_text: String,
    pub replace_with: String,
    pub case_sensitive: bool,
}

impl Default for RenamePlan {
    fn default() -> Self {
        Self {
            mode: RenameMode::Template,
            template: "[Original file name]".into(),
            counter_start: 1,
            counter_step: 1,
            counter_padding: 2,
            find_text: String::new(),
            replace_with: String::new(),
            case_sensitive: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePreview {
    pub original: String,
    pub new_name: String,
    pub has_error: bool,
}

/// Angular hides counter start/step/padding until the template uses `[Counter]`.
pub fn template_uses_counter(template: &str) -> bool {
    template.contains("[Counter]")
}

pub fn counter_controls_visible(mode: &RenameMode, template: &str) -> bool {
    matches!(mode, RenameMode::Template) && template_uses_counter(template)
}

pub fn split_base_ext(filename: &str) -> (String, String) {
    if filename.starts_with('.') && filename[1..].find('.').is_none() {
        return (filename.to_string(), String::new());
    }
    match filename.rfind('.') {
        Some(idx) if idx > 0 => (filename[..idx].to_string(), filename[idx..].to_string()),
        _ => (filename.to_string(), String::new()),
    }
}

pub fn calculate_new_name(filename: &str, index: usize, plan: &RenamePlan, date: &str) -> String {
    let (base, ext) = split_base_ext(filename);
    match plan.mode {
        RenameMode::Template => {
            if plan.template.is_empty() {
                return filename.to_string();
            }
            let mut tpl = plan.template.clone();
            tpl = tpl.replace("[Original file name]", &base);
            tpl = tpl.replace("[Name]", &base);
            if tpl.contains("[Extension]") {
                let clean = ext.strip_prefix('.').unwrap_or(&ext);
                tpl = tpl.replace("[Extension]", clean);
            }
            if tpl.contains("[Counter]") {
                let count = plan.counter_start + (index as i64) * plan.counter_step;
                let formatted = format!("{count:0width$}", width = plan.counter_padding);
                tpl = tpl.replace("[Counter]", &formatted);
            }
            if tpl.contains("[Date]") {
                tpl = tpl.replace("[Date]", date);
            }
            if !plan.template.contains("[Extension]") {
                format!("{tpl}{ext}")
            } else {
                tpl
            }
        }
        RenameMode::Replace => {
            if plan.find_text.is_empty() {
                return filename.to_string();
            }
            let new_base = replace_str(
                &base,
                &plan.find_text,
                &plan.replace_with,
                plan.case_sensitive,
            );
            format!("{new_base}{ext}")
        }
    }
}

pub fn preview(names: &[String], plan: &RenamePlan, date: &str) -> Vec<RenamePreview> {
    let mut rows: Vec<RenamePreview> = names
        .iter()
        .enumerate()
        .map(|(index, original)| {
            let new_name = calculate_new_name(original, index, plan, date);
            RenamePreview {
                original: original.clone(),
                new_name,
                has_error: false,
            }
        })
        .collect();
    for i in 0..rows.len() {
        let name = rows[i].new_name.clone();
        let empty = name.trim().is_empty();
        let dup = rows.iter().filter(|r| r.new_name == name).count() > 1;
        rows[i].has_error = empty || dup;
    }
    rows
}

pub fn has_errors(rows: &[RenamePreview]) -> bool {
    rows.iter().any(|r| r.has_error)
}

pub fn has_changes(rows: &[RenamePreview]) -> bool {
    rows.iter().any(|r| r.new_name != r.original)
}

fn replace_str(input: &str, find: &str, replace: &str, case_sensitive: bool) -> String {
    if find.is_empty() {
        return input.to_string();
    }
    if case_sensitive {
        return input.replace(find, replace);
    }

    // `to_lowercase()` is not length-preserving — 'İ' (2 bytes) lowercases to
    // 'i' + a combining dot (3 bytes), 'ẞ' (3) to 'ß' (2). Indexing the
    // original string with an offset found in the lowercased one therefore
    // panics on a non-ASCII name. Fold character by character instead and keep
    // a map from every lowercased byte back to the original character it came
    // from, so matches are always cut on real character boundaries.
    let needle = find.to_lowercase();
    if needle.is_empty() {
        return input.to_string();
    }

    let mut hay = String::with_capacity(input.len());
    // For each byte of `hay`: where the source character starts and ends in `input`.
    let mut source_start = Vec::with_capacity(input.len());
    let mut source_end = Vec::with_capacity(input.len());
    for (offset, ch) in input.char_indices() {
        let end = offset + ch.len_utf8();
        let before = hay.len();
        for lowered in ch.to_lowercase() {
            hay.push(lowered);
        }
        for _ in before..hay.len() {
            source_start.push(offset);
            source_end.push(end);
        }
    }

    let mut out = String::with_capacity(input.len());
    let mut cursor = 0; // byte offset into `input`
    let mut search_from = 0; // byte offset into `hay`
    while let Some(found) = hay[search_from..].find(&needle) {
        let start = search_from + found;
        let end = start + needle.len();
        let orig_start = source_start[start];
        let orig_end = source_end[end - 1];
        if orig_start < cursor || orig_end <= orig_start {
            // The match landed inside a character already consumed (or made no
            // progress); step past it rather than looping forever.
            search_from = end;
            continue;
        }
        out.push_str(&input[cursor..orig_start]);
        out.push_str(replace);
        cursor = orig_end;
        search_from = end;
    }
    out.push_str(&input[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_replace_handles_non_ascii_names() {
        // These used to panic: `to_lowercase()` changes byte length for 'İ'
        // (2 -> 3) and 'ẞ' (3 -> 2), so the lowercased offset did not point at
        // a character boundary in the original.
        assert_eq!(replace_str("İstanbul", "l", "L", false), "İstanbuL");
        assert_eq!(replace_str("ẞa", "a", "b", false), "ẞb");
        assert_eq!(replace_str("İİİ", "i", "x", false), "xxx");
        assert_eq!(replace_str("Ünïcödé.txt", "TXT", "md", false), "Ünïcödé.md");
        assert_eq!(
            replace_str("日本語 file", "FILE", "ファイル", false),
            "日本語 ファイル"
        );
    }

    #[test]
    fn case_insensitive_replace_matches_regardless_of_case() {
        assert_eq!(replace_str("Photo.JPG", "jpg", "png", false), "Photo.png");
        assert_eq!(replace_str("aAaA", "a", "-", false), "----");
        assert_eq!(replace_str("report", "xyz", "!", false), "report");
        assert_eq!(replace_str("report", "", "!", false), "report");
        assert_eq!(replace_str("", "a", "b", false), "");
    }

    #[test]
    fn case_sensitive_replace_is_unchanged() {
        assert_eq!(replace_str("Photo.JPG", "jpg", "png", true), "Photo.JPG");
        assert_eq!(replace_str("Photo.jpg", "jpg", "png", true), "Photo.png");
        assert_eq!(replace_str("İstanbul", "l", "L", true), "İstanbuL");
    }

    #[test]
    fn template_counter_and_date() {
        let plan = RenamePlan {
            template: "[Name]-[Counter]-[Date]".into(),
            counter_padding: 3,
            ..Default::default()
        };
        assert_eq!(
            calculate_new_name("photo.jpg", 0, &plan, "2026-08-25"),
            "photo-001-2026-08-25.jpg"
        );
        assert_eq!(
            calculate_new_name("photo.jpg", 1, &plan, "2026-08-25"),
            "photo-002-2026-08-25.jpg"
        );
    }

    #[test]
    fn explicit_extension_placeholder() {
        let plan = RenamePlan {
            template: "report.[Extension]".into(),
            ..Default::default()
        };
        assert_eq!(calculate_new_name("a.TAR.GZ", 0, &plan, ""), "report.GZ");
        let keep = RenamePlan {
            template: "[Name].[Extension]".into(),
            ..Default::default()
        };
        assert_eq!(calculate_new_name("photo.jpg", 0, &keep, ""), "photo.jpg");
    }

    #[test]
    fn replace_is_case_insensitive_by_default() {
        let plan = RenamePlan {
            mode: RenameMode::Replace,
            find_text: "IMG".into(),
            replace_with: "pic".into(),
            case_sensitive: false,
            ..Default::default()
        };
        assert_eq!(
            calculate_new_name("img_001.JPG", 0, &plan, ""),
            "pic_001.JPG"
        );
        let sensitive = RenamePlan {
            mode: RenameMode::Replace,
            find_text: "IMG".into(),
            replace_with: "pic".into(),
            case_sensitive: true,
            ..Default::default()
        };
        assert_eq!(
            calculate_new_name("img_001.JPG", 0, &sensitive, ""),
            "img_001.JPG"
        );
        assert_eq!(
            calculate_new_name("IMG_001.JPG", 0, &sensitive, ""),
            "pic_001.JPG"
        );
    }

    #[test]
    fn preview_flags_duplicates_and_empty() {
        let plan = RenamePlan {
            template: "same".into(),
            ..Default::default()
        };
        let rows = preview(&["a.txt".into(), "b.txt".into()], &plan, "");
        assert!(has_errors(&rows));
        assert!(has_changes(&rows));
    }

    #[test]
    fn counter_controls_follow_template_token() {
        assert!(template_uses_counter("[Name]-[Counter]"));
        assert!(!template_uses_counter("[Original file name]"));
        assert!(counter_controls_visible(
            &RenameMode::Template,
            "shot-[Counter]"
        ));
        assert!(!counter_controls_visible(
            &RenameMode::Template,
            "[Original file name]"
        ));
        assert!(!counter_controls_visible(&RenameMode::Replace, "[Counter]"));
    }

    #[test]
    fn hidden_dotfiles_keep_name() {
        let (base, ext) = split_base_ext(".gitignore");
        assert_eq!(base, ".gitignore");
        assert!(ext.is_empty());
    }
}
