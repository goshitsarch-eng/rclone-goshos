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
    let hay = input.to_lowercase();
    let needle = find.to_lowercase();
    let mut out = String::new();
    let mut rest = input;
    let mut rest_lower = hay.as_str();
    while let Some(idx) = rest_lower.find(&needle) {
        out.push_str(&rest[..idx]);
        out.push_str(replace);
        rest = &rest[idx + find.len()..];
        rest_lower = &rest_lower[idx + needle.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn hidden_dotfiles_keep_name() {
        let (base, ext) = split_base_ext(".gitignore");
        assert_eq!(base, ".gitignore");
        assert!(ext.is_empty());
    }
}
