//! Typed rclone flag editors — Angular `setting-control` / `flag-config-step`.

use super::AppCtx;
use crate::flags::FlagOption;
use crate::value_mapper::{control_kind, ControlKind};
use adw::prelude::*;
use serde_json::Value;
use std::rc::Rc;

pub type FlagRow = (String, FlagWidget, String);
pub type ServeFlagRow = (String, String, FlagWidget, String);

#[derive(Clone)]
enum FlagInner {
    Entry(adw::EntryRow),
    Switch(adw::SwitchRow),
    Combo(adw::ComboRow, Rc<Vec<String>>),
    SearchCombo(
        adw::EntryRow,
        adw::ComboRow,
        Rc<Vec<String>>,
        Rc<Vec<String>>,
    ),
    Spin(adw::SpinRow),
    Multi(adw::ExpanderRow, Rc<Vec<(String, gtk::CheckButton)>>),
}

#[derive(Clone)]
pub struct FlagWidget {
    inner: FlagInner,
    title: String,
    help: String,
}

impl FlagWidget {
    pub fn from_flag(ctx: &AppCtx, flag: &FlagOption, current: &Value) -> Self {
        let title = ctx.option_label(&flag.name, "title", &flag.name);
        let help = ctx.option_label(&flag.name, "help", &flag.help);
        let initial = crate::flags::flag_display_text(flag, current);
        let kind = control_kind(&flag.type_name, flag.exclusive, flag.examples.len());
        let inner = match kind {
            ControlKind::Bool => {
                let row = crate::ui::rows::switch_row();
                row.set_title(&title);
                if !help.is_empty() {
                    row.set_subtitle(&help);
                }
                row.set_active(initial.eq_ignore_ascii_case("true"));
                FlagInner::Switch(row)
            }
            ControlKind::Tristate => {
                let values = Rc::new(vec![
                    "unset".to_string(),
                    "true".to_string(),
                    "false".to_string(),
                ]);
                let row = crate::ui::rows::combo_row();
                row.set_title(&title);
                if !help.is_empty() {
                    row.set_subtitle(&help);
                }
                row.set_model(Some(&gtk::StringList::new(&["unset", "true", "false"])));
                if let Some(idx) = values.iter().position(|v| v.eq_ignore_ascii_case(&initial)) {
                    row.set_selected(idx as u32);
                }
                FlagInner::Combo(row, values)
            }
            ControlKind::Select => {
                let labels: Vec<String> = flag
                    .examples
                    .iter()
                    .map(|(value, hint)| crate::config_search::example_choice_label(value, hint))
                    .collect();
                let values = Rc::new(
                    flag.examples
                        .iter()
                        .map(|(value, _)| value.clone())
                        .collect::<Vec<_>>(),
                );
                let row = crate::ui::rows::combo_row();
                row.set_title(&title);
                if !help.is_empty() {
                    row.set_subtitle(&help);
                }
                let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                row.set_model(Some(&gtk::StringList::new(&refs)));
                if let Some(idx) = values.iter().position(|v| v == &initial) {
                    row.set_selected(idx as u32);
                }
                if crate::config_search::should_search_examples(
                    &flag.field_name,
                    flag.examples.len(),
                ) || crate::config_search::should_search_examples(
                    &flag.name,
                    flag.examples.len(),
                ) {
                    let search = crate::ui::rows::entry_row();
                    search.set_title(&title);
                    if !help.is_empty() {
                        search.set_tooltip_text(Some(&help));
                    }
                    if let Some(label) = labels.get(row.selected() as usize) {
                        search.set_text(label);
                    }
                    super::dialogs::attach_example_typeahead(ctx, &search, &row, &flag.examples);
                    row.set_visible(false);
                    FlagInner::SearchCombo(search, row, values, Rc::new(labels))
                } else {
                    FlagInner::Combo(row, values)
                }
            }
            ControlKind::MultiSelect => {
                let row = crate::ui::rows::expander_row();
                row.set_title(&title);
                if !help.is_empty() {
                    row.set_subtitle(&help);
                }
                let selected: Vec<String> = initial
                    .split([',', ' '])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_lowercase())
                    .collect();
                let mut items = Vec::new();
                for (value, hint) in &flag.examples {
                    let check = gtk::CheckButton::with_label(value);
                    if !hint.is_empty() {
                        check.set_tooltip_text(Some(hint));
                    }
                    check.set_active(selected.iter().any(|s| s == &value.to_ascii_lowercase()));
                    let wrap = crate::ui::rows::action_row();
                    wrap.set_title(value);
                    if !hint.is_empty() {
                        wrap.set_subtitle(hint);
                    }
                    wrap.add_prefix(&check);
                    wrap.set_activatable(true);
                    {
                        let check = check.clone();
                        wrap.connect_activated(move |_| check.set_active(!check.is_active()));
                    }
                    row.add_row(&wrap);
                    items.push((value.clone(), check));
                }
                FlagInner::Multi(row, Rc::new(items))
            }
            ControlKind::Numeric => {
                let row =
                    crate::ui::rows::spin_row_with_range(-1_000_000_000.0, 1_000_000_000.0, 1.0);
                row.set_title(&title);
                if !help.is_empty() {
                    row.set_subtitle(&help);
                }
                if let Ok(v) = initial.parse::<f64>() {
                    row.set_value(v);
                }
                row.set_digits(if crate::value_mapper::is_float_type(&flag.type_name) {
                    3
                } else {
                    0
                });
                FlagInner::Spin(row)
            }
            ControlKind::Input => {
                let row = crate::ui::rows::entry_row();
                let titled = if flag.type_name == "Duration" {
                    format!("{title} (1h / 30s / 500ms)")
                } else if flag.type_name == "SizeSuffix" {
                    format!("{title} (1Gi / 512Mi / off)")
                } else {
                    title.clone()
                };
                row.set_title(&titled);
                if !help.is_empty() {
                    row.set_tooltip_text(Some(&help));
                }
                row.set_text(&initial);
                FlagInner::Entry(row)
            }
        };
        Self { inner, title, help }
    }

    pub fn plain_entry(field: &str, text: &str) -> Self {
        let row = crate::ui::rows::entry_row();
        row.set_title(field);
        row.set_text(text);
        Self {
            inner: FlagInner::Entry(row),
            title: field.to_string(),
            help: String::new(),
        }
    }

    pub fn title(&self) -> String {
        self.title.clone()
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
        match &self.inner {
            FlagInner::Entry(row) => row.set_title(title),
            FlagInner::Switch(row) => row.set_title(title),
            FlagInner::Combo(row, _) => row.set_title(title),
            FlagInner::SearchCombo(search, combo, _, _) => {
                search.set_title(title);
                combo.set_title(title);
            }
            FlagInner::Spin(row) => row.set_title(title),
            FlagInner::Multi(row, _) => row.set_title(title),
        }
    }

    pub fn help(&self) -> String {
        match &self.inner {
            FlagInner::Entry(row) => row.tooltip_text().unwrap_or_default().to_string(),
            FlagInner::Switch(row) => row.subtitle().unwrap_or_default().to_string(),
            FlagInner::Combo(row, _) | FlagInner::SearchCombo(_, row, _, _) => {
                row.subtitle().unwrap_or_default().to_string()
            }
            FlagInner::Spin(row) => row.subtitle().unwrap_or_default().to_string(),
            FlagInner::Multi(row, _) => row.subtitle().to_string(),
        }
        .if_empty(&self.help)
    }

    pub fn add_to(&self, group: &adw::PreferencesGroup) {
        match &self.inner {
            FlagInner::Entry(row) => group.add(row),
            FlagInner::Switch(row) => group.add(row),
            FlagInner::Combo(row, _) => group.add(row),
            FlagInner::SearchCombo(search, _, _, _) => group.add(search),
            FlagInner::Spin(row) => group.add(row),
            FlagInner::Multi(row, _) => group.add(row),
        }
    }

    pub fn remove_from(&self, group: &adw::PreferencesGroup) {
        let child = self.widget();
        if child.parent().is_some_and(|parent| parent == *group) {
            group.remove(&child);
        }
    }

    pub fn set_visible(&self, visible: bool) {
        match &self.inner {
            FlagInner::Entry(row) => row.set_visible(visible),
            FlagInner::Switch(row) => row.set_visible(visible),
            FlagInner::Combo(row, _) => row.set_visible(visible),
            FlagInner::SearchCombo(search, combo, _, _) => {
                search.set_visible(visible);
                combo.set_visible(false);
            }
            FlagInner::Spin(row) => row.set_visible(visible),
            FlagInner::Multi(row, _) => row.set_visible(visible),
        }
    }

    pub fn widget(&self) -> gtk::Widget {
        match &self.inner {
            FlagInner::Entry(row) => row.clone().upcast(),
            FlagInner::Switch(row) => row.clone().upcast(),
            FlagInner::Combo(row, _) => row.clone().upcast(),
            FlagInner::SearchCombo(search, _, _, _) => search.clone().upcast(),
            FlagInner::Spin(row) => row.clone().upcast(),
            FlagInner::Multi(row, _) => row.clone().upcast(),
        }
    }

    pub fn text(&self) -> String {
        match &self.inner {
            FlagInner::Entry(row) => row.text().to_string(),
            FlagInner::Switch(row) => row.is_active().to_string(),
            FlagInner::Combo(row, values) | FlagInner::SearchCombo(_, row, values, _) => values
                .get(row.selected() as usize)
                .cloned()
                .unwrap_or_default(),
            FlagInner::Spin(row) => {
                let v = row.value();
                if v.fract() == 0.0 {
                    format!("{}", v as i64)
                } else {
                    v.to_string()
                }
            }
            FlagInner::Multi(_, items) => items
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(value, _)| value.clone())
                .collect::<Vec<_>>()
                .join(","),
        }
    }

    pub fn set_text(&self, text: &str) {
        match &self.inner {
            FlagInner::Entry(row) => row.set_text(text),
            FlagInner::Switch(row) => row.set_active(text.eq_ignore_ascii_case("true")),
            FlagInner::Combo(row, values) => {
                if let Some(idx) = values
                    .iter()
                    .position(|v| v == text || v.eq_ignore_ascii_case(text))
                {
                    row.set_selected(idx as u32);
                }
            }
            FlagInner::SearchCombo(search, row, values, labels) => {
                if let Some(idx) = values
                    .iter()
                    .position(|v| v == text || v.eq_ignore_ascii_case(text))
                {
                    row.set_selected(idx as u32);
                    if let Some(label) = labels.get(idx) {
                        search.set_text(label);
                    }
                } else {
                    search.set_text(text);
                }
            }
            FlagInner::Spin(row) => {
                if let Ok(v) = text.parse::<f64>() {
                    row.set_value(v);
                }
            }
            FlagInner::Multi(_, items) => {
                let selected: Vec<String> = text
                    .split([',', ' '])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_lowercase())
                    .collect();
                for (value, check) in items.iter() {
                    check.set_active(selected.iter().any(|s| s == &value.to_ascii_lowercase()));
                }
            }
        }
    }

    pub fn set_error(&self, error: bool) {
        let widget = self.widget();
        if error {
            widget.add_css_class("error");
        } else {
            widget.remove_css_class("error");
        }
    }
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}
