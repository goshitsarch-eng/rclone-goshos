//! GTK widget for the Angular `app-json-editor` surface.

use super::AppCtx;
use crate::json_editor::{
    chips, complete_at, cursor_at, diagnose, display_text, highlight_spans, parse_object,
    path_rules, pretty, reconcile_paths, restore_restricted, toggle_chip, ChipSpec, Diagnosis,
    JsonCursorKind, JsonFieldDef, PathRecon,
};
use crate::operations::OperationType;
use adw::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone)]
pub struct JsonEditor {
    pub root: gtk::Box,
    pub view: gtk::TextView,
    chips: gtk::FlowBox,
    chips_empty: gtk::Label,
    info: gtk::Label,
    error: gtk::Label,
    warn: gtk::Label,
    fields: Rc<RefCell<Vec<JsonFieldDef>>>,
    structural: Rc<RefCell<Vec<String>>>,
    explicit: Rc<RefCell<HashSet<String>>>,
    last_good: Rc<RefCell<Value>>,
    restrict: Rc<Cell<bool>>,
    search: Rc<RefCell<String>>,
    suppress: Rc<Cell<bool>>,
    operation: Rc<Cell<Option<OperationType>>>,
    on_paths: Rc<RefCell<Option<Rc<dyn Fn(PathRecon)>>>>,
    popover: gtk::Popover,
    list: gtk::ListBox,
    ctx: AppCtx,
}

impl JsonEditor {
    pub fn new(ctx: &AppCtx) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_hexpand(true);
        let info = gtk::Label::new(None);
        info.set_wrap(true);
        info.set_xalign(0.0);
        info.add_css_class("dim-label");
        info.set_visible(false);
        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_min_children_per_line(1);
        chips.set_max_children_per_line(8);
        chips.set_column_spacing(6);
        chips.set_row_spacing(6);
        chips.set_hexpand(true);
        let chips_empty = gtk::Label::new(Some(&ctx.t_or(
            "shared.jsonEditor.noFields",
            "No fields to display in JSON mode",
        )));
        chips_empty.add_css_class("dim-label");
        chips_empty.set_xalign(0.0);
        let chips_scroll = gtk::ScrolledWindow::new();
        chips_scroll.set_min_content_height(48);
        chips_scroll.set_max_content_height(120);
        chips_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        chips_scroll.set_child(Some(&chips));
        let view = gtk::TextView::new();
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_left_margin(8);
        view.set_right_margin(8);
        view.set_top_margin(8);
        view.set_bottom_margin(8);
        view.set_hexpand(true);
        view.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(180);
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&view));
        let error = banner_label("error");
        let warn = banner_label("warning");
        root.append(&info);
        root.append(&chips_empty);
        root.append(&chips_scroll);
        root.append(&scroll);
        root.append(&error);
        root.append(&warn);

        let popover = gtk::Popover::new();
        popover.set_parent(&view);
        popover.set_autohide(true);
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        let pop_scroll = gtk::ScrolledWindow::new();
        pop_scroll.set_min_content_height(80);
        pop_scroll.set_max_content_height(240);
        pop_scroll.set_propagate_natural_width(true);
        pop_scroll.set_child(Some(&list));
        popover.set_child(Some(&pop_scroll));
        {
            let popover = popover.clone();
            view.connect_destroy(move |_| popover.unparent());
        }

        let editor = Self {
            root,
            view,
            chips,
            chips_empty,
            info,
            error,
            warn,
            fields: Rc::new(RefCell::new(Vec::new())),
            structural: Rc::new(RefCell::new(Vec::new())),
            explicit: Rc::new(RefCell::new(HashSet::new())),
            last_good: Rc::new(RefCell::new(serde_json::json!({}))),
            restrict: Rc::new(Cell::new(ctx.settings.borrow().general.restrict)),
            search: Rc::new(RefCell::new(String::new())),
            suppress: Rc::new(Cell::new(false)),
            operation: Rc::new(Cell::new(None)),
            on_paths: Rc::new(RefCell::new(None)),
            popover,
            list,
            ctx: ctx.clone(),
        };
        editor.bind();
        editor
    }

    pub fn set_fields(&self, fields: Vec<JsonFieldDef>) {
        *self.fields.borrow_mut() = fields;
        self.refresh_chips();
    }

    pub fn set_structural(&self, keys: Vec<String>) {
        *self.structural.borrow_mut() = keys;
    }

    pub fn set_operation(&self, op: Option<OperationType>) {
        self.operation.set(op);
    }

    pub fn set_info(&self, text: Option<&str>) {
        match text.filter(|t| !t.is_empty()) {
            Some(text) => {
                self.info.set_text(text);
                self.info.set_visible(true);
            }
            None => self.info.set_visible(false),
        }
    }

    pub fn set_restrict(&self, restrict: bool) {
        self.restrict.set(restrict);
    }

    pub fn set_value(&self, value: &Value) {
        *self.last_good.borrow_mut() = value.clone();
        self.explicit.borrow_mut().extend(
            value
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys().cloned()),
        );
        self.set_text_raw(&display_text(value, self.restrict.get()));
        self.refresh_state();
    }

    pub fn text(&self) -> String {
        let buffer = self.view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    pub fn parsed(&self) -> Result<Map<String, Value>, String> {
        let restored = restore_restricted(&self.text(), &self.last_good.borrow()).map_err(|d| {
            self.ctx
                .tf_or(d.i18n_key, "Invalid JSON syntax.", &params_ref(&d))
        })?;
        match restored {
            Value::Object(map) => Ok(map),
            _ => Err(self
                .ctx
                .t_or("shared.jsonEditor.parseError", "Invalid JSON syntax.")),
        }
    }

    pub fn highlight_search(&self, query: &str) {
        *self.search.borrow_mut() = query.to_string();
        apply_search_tags(&self.view.buffer(), query);
        self.refresh_chips();
    }

    pub fn on_paths(&self, cb: Rc<dyn Fn(PathRecon)>) {
        *self.on_paths.borrow_mut() = Some(cb);
    }

    fn set_text_raw(&self, text: &str) {
        self.suppress.set(true);
        self.view.buffer().set_text(text);
        self.suppress.set(false);
    }

    fn bind(&self) {
        let editor = self.clone();
        self.view.buffer().connect_changed(move |_| {
            if editor.suppress.get() {
                return;
            }
            let editor = editor.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                editor.refresh_state();
                editor.show_completions();
            });
        });
    }

    fn refresh_state(&self) {
        let text = self.text();
        apply_search_tags(&self.view.buffer(), &self.search.borrow());
        match parse_object(&text) {
            Ok(map) => {
                let op = self.operation.get();
                let rules = path_rules(op);
                let diagnosis = diagnose(
                    &map,
                    &self.fields.borrow(),
                    &self.structural.borrow(),
                    &rules,
                );
                self.show_diagnosis(diagnosis.as_ref());
                if diagnosis.as_ref().is_none_or(|d| !d.error) {
                    *self.last_good.borrow_mut() = Value::Object(map.clone());
                    self.explicit.borrow_mut().extend(map.keys().cloned());
                    if let Some(cb) = self.on_paths.borrow().clone() {
                        cb(reconcile_paths(&map, op));
                    }
                }
            }
            Err(diag) => self.show_diagnosis(Some(&diag)),
        }
        self.refresh_chips();
    }

    fn refresh_chips(&self) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        let value = self.last_good.borrow().clone();
        let specs = chips(
            &self.fields.borrow(),
            &value,
            &self.explicit.borrow(),
            &self.search.borrow(),
            self.restrict.get(),
        );
        self.chips_empty.set_visible(specs.is_empty());
        for spec in specs {
            let btn = chip_button(&spec);
            let editor = self.clone();
            let key = spec.key.clone();
            btn.connect_clicked(move |_| {
                let default = editor
                    .fields
                    .borrow()
                    .iter()
                    .find(|f| f.key == key)
                    .map(|f| f.default.clone())
                    .unwrap_or(Value::String(String::new()));
                match toggle_chip(&editor.text(), &key, &default) {
                    Ok(next) => {
                        editor.explicit.borrow_mut().insert(key.clone());
                        editor.set_text_raw(&next);
                        editor.refresh_state();
                    }
                    Err(diag) => editor.show_diagnosis(Some(&diag)),
                }
            });
            self.chips.append(&btn);
        }
    }

    fn show_diagnosis(&self, diagnosis: Option<&Diagnosis>) {
        match diagnosis {
            Some(diag) if diag.error => {
                self.error.set_text(&self.ctx.tf_or(
                    diag.i18n_key,
                    "Invalid JSON syntax.",
                    &params_ref(diag),
                ));
                self.error.set_visible(true);
                self.warn.set_visible(false);
            }
            Some(diag) => {
                self.error.set_visible(false);
                self.warn.set_text(&self.ctx.tf_or(
                    diag.i18n_key,
                    "Unknown option.",
                    &params_ref(diag),
                ));
                self.warn.set_visible(true);
            }
            None => {
                self.error.set_visible(false);
                self.warn.set_visible(false);
            }
        }
    }

    fn show_completions(&self) {
        if self.suppress.get() || !self.view.has_focus() {
            self.popover.popdown();
            return;
        }
        let buffer = self.view.buffer();
        let insert = buffer.iter_at_mark(&buffer.get_insert());
        let char_off = insert.offset() as usize;
        let text = self.text();
        let byte = text.chars().take(char_off).map(|c| c.len_utf8()).sum();
        let hits = complete_at(
            &text,
            byte,
            &self.fields.borrow(),
            &self.structural.borrow(),
        );
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        if hits.is_empty() {
            self.popover.popdown();
            return;
        }
        let cursor = cursor_at(&text, byte);
        for hit in hits {
            let row = crate::ui::rows::action_row();
            row.set_title(&hit.label);
            if !hit.detail.is_empty() {
                row.set_subtitle(&hit.detail);
            }
            row.set_activatable(true);
            let editor = self.clone();
            let label = hit.label.clone();
            let from = cursor.from;
            let to = cursor.to;
            row.connect_activated(move |_| {
                editor.insert_completion(&label, from, to);
                editor.popover.popdown();
            });
            self.list.append(&row);
        }
        self.popover.popup();
    }

    fn insert_completion(&self, label: &str, from: usize, to: usize) {
        let text = self.text();
        let from = from.min(text.len());
        let to = to.min(text.len()).max(from);
        let mut next = String::new();
        next.push_str(&text[..from]);
        next.push_str(label);
        next.push_str(&text[to..]);
        let cursor = cursor_at(&text, from);
        self.set_text_raw(&next);
        if matches!(cursor.kind, JsonCursorKind::Property) {
            self.explicit.borrow_mut().insert(label.to_string());
        }
        self.refresh_state();
    }
}

fn banner_label(kind: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class(kind);
    label.set_visible(false);
    label
}

fn chip_button(spec: &ChipSpec) -> gtk::Button {
    let label = if spec.display_value.is_empty() {
        spec.key.clone()
    } else {
        format!("{} · {}", spec.key, spec.display_value)
    };
    let btn = gtk::Button::with_label(&label);
    btn.set_widget_name(&format!("chip:{}", spec.key));
    btn.add_css_class("pill");
    if spec.changed {
        btn.add_css_class("suggested-action");
    } else if spec.active {
        btn.add_css_class("opaque");
    } else {
        btn.add_css_class("flat");
    }
    btn.set_tooltip_text(Some(&spec.key));
    btn
}

fn apply_search_tags(buffer: &gtk::TextBuffer, query: &str) {
    let table = buffer.tag_table();
    let tag = match table.lookup("json-search-hit") {
        Some(tag) => tag,
        None => {
            let tag = gtk::TextTag::builder()
                .name("json-search-hit")
                .background("#f5c21133")
                .build();
            table.add(&tag);
            tag
        }
    };
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_tag(&tag, &start, &end);
    let text = buffer.text(&start, &end, false).to_string();
    for (from, to) in highlight_spans(&text, query) {
        let mut hit_start = buffer.start_iter();
        hit_start.set_offset(text[..from].chars().count() as i32);
        let mut hit_end = hit_start;
        hit_end.forward_chars(text[from..to].chars().count() as i32);
        buffer.apply_tag(&tag, &hit_start, &hit_end);
    }
}

fn params_ref(diag: &Diagnosis) -> Vec<(&str, &str)> {
    diag.params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

#[allow(dead_code)]
pub fn pretty_object(value: &Value) -> String {
    pretty(value)
}
