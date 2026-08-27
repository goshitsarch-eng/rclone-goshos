//! Angular `app-installation-options` — Recommended / Custom / Existing tabs.

use super::AppCtx;
use crate::installation::{
    binary_status_key, test_rclone_binary, BinaryStatus, InstallLocation, InstallationMode,
    InstallationOptionsData,
};
use gtk::gio;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct InstallationOptions {
    pub root: gtk::Box,
    data: Rc<RefCell<InstallationOptionsData>>,
    mode: InstallationMode,
    on_change: Rc<RefCell<Option<Rc<dyn Fn(InstallationOptionsData)>>>>,
}

impl InstallationOptions {
    pub fn new(ctx: &AppCtx, mode: InstallationMode, include_existing: bool) -> Self {
        let data = Rc::new(RefCell::new(InstallationOptionsData::default()));
        let on_change: Rc<RefCell<Option<Rc<dyn Fn(InstallationOptionsData)>>>> =
            Rc::new(RefCell::new(None));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("installation-options");

        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tabs.add_css_class("linked");
        tabs.set_hexpand(true);
        let recommended = gtk::ToggleButton::with_label(&ctx.t_or(
            match mode {
                InstallationMode::Install => "shared.installationOptions.tabs.quickFix",
                InstallationMode::Config => "repairSheet.configTabs.default",
            },
            "Recommended",
        ));
        recommended.set_active(true);
        recommended.set_hexpand(true);
        let custom = gtk::ToggleButton::with_label(
            &ctx.t_or("shared.installationOptions.tabs.custom", "Custom"),
        );
        custom.set_group(Some(&recommended));
        custom.set_hexpand(true);
        tabs.append(&recommended);
        tabs.append(&custom);
        let existing = gtk::ToggleButton::with_label(
            &ctx.t_or("shared.installationOptions.tabs.existing", "Existing"),
        );
        existing.set_group(Some(&recommended));
        existing.set_hexpand(true);
        if include_existing {
            tabs.append(&existing);
        }
        root.append(&tabs);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(180);

        let default_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let default_desc = gtk::Label::new(Some(&ctx.t_or(
            match mode {
                InstallationMode::Install => {
                    "shared.installationOptions.modes.install.default.description"
                }
                InstallationMode::Config => {
                    "shared.installationOptions.modes.config.default.description"
                }
            },
            "Use the standard installation location.",
        )));
        default_desc.set_wrap(true);
        default_desc.set_xalign(0.0);
        default_desc.add_css_class("dim-label");
        let default_rec = gtk::Label::new(Some(&ctx.t_or(
            match mode {
                InstallationMode::Install => {
                    "shared.installationOptions.modes.install.default.recommendation"
                }
                InstallationMode::Config => {
                    "shared.installationOptions.modes.config.default.recommendation"
                }
            },
            "Recommended for most users.",
        )));
        default_rec.set_wrap(true);
        default_rec.set_xalign(0.0);
        default_box.append(&default_desc);
        default_box.append(&default_rec);
        stack.add_named(&default_box, Some("default"));

        let custom_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let custom_desc = gtk::Label::new(Some(&ctx.t_or(
            match mode {
                InstallationMode::Install => {
                    "shared.installationOptions.modes.install.custom.description"
                }
                InstallationMode::Config => {
                    "shared.installationOptions.modes.config.custom.description"
                }
            },
            "Choose a custom installation path.",
        )));
        custom_desc.set_wrap(true);
        custom_desc.set_xalign(0.0);
        custom_desc.add_css_class("dim-label");
        custom_box.append(&custom_desc);
        let custom_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let custom_entry = gtk::Entry::new();
        custom_entry.set_hexpand(true);
        custom_entry.set_placeholder_text(Some(&ctx.t_or(
            match mode {
                InstallationMode::Install => {
                    "shared.installationOptions.modes.install.custom.placeholder"
                }
                InstallationMode::Config => {
                    "shared.installationOptions.modes.config.custom.placeholder"
                }
            },
            "/path/to/install",
        )));
        custom_entry.set_tooltip_text(Some(&ctx.t_or(
            match mode {
                InstallationMode::Install => {
                    "shared.installationOptions.modes.install.custom.label"
                }
                InstallationMode::Config => "shared.installationOptions.modes.config.custom.label",
            },
            "Install Path",
        )));
        let custom_browse =
            gtk::Button::with_label(&ctx.t_or("shared.installationOptions.browse", "Browse"));
        custom_row.append(&custom_entry);
        custom_row.append(&custom_browse);
        custom_box.append(&custom_row);
        let custom_error = gtk::Label::new(None);
        custom_error.add_css_class("error");
        custom_error.set_xalign(0.0);
        custom_error.set_visible(false);
        custom_box.append(&custom_error);
        stack.add_named(&custom_box, Some("custom"));

        let existing_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let existing_desc = gtk::Label::new(Some(&ctx.t_or(
            match mode {
                InstallationMode::Install => {
                    "shared.installationOptions.modes.install.existing.description"
                }
                InstallationMode::Config => {
                    "shared.installationOptions.modes.config.existing.description"
                }
            },
            "Use an existing rclone binary.",
        )));
        existing_desc.set_wrap(true);
        existing_desc.set_xalign(0.0);
        existing_desc.add_css_class("dim-label");
        existing_box.append(&existing_desc);
        let existing_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let existing_entry = gtk::Entry::new();
        existing_entry.set_hexpand(true);
        existing_entry.set_placeholder_text(Some(&ctx.t_or(
            "shared.installationOptions.existingBinaryPlaceholder",
            "/path/to/rclone",
        )));
        existing_entry.set_tooltip_text(Some(&ctx.t_or(
            "shared.installationOptions.modes.install.existing.label",
            "Binary Path",
        )));
        let existing_browse =
            gtk::Button::with_label(&ctx.t_or("shared.installationOptions.browse", "Browse"));
        existing_row.append(&existing_entry);
        existing_row.append(&existing_browse);
        existing_box.append(&existing_row);
        let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let status = gtk::Label::new(Some(
            &ctx.t_or(binary_status_key(BinaryStatus::Untested), "Not tested"),
        ));
        status.add_css_class("dim-label");
        status.set_hexpand(true);
        status.set_xalign(0.0);
        let test = gtk::Button::with_label(&ctx.t_or("shared.installationOptions.test", "Test"));
        status_row.append(&status);
        status_row.append(&test);
        existing_box.append(&status_row);
        if include_existing {
            stack.add_named(&existing_box, Some("existing"));
        }

        root.append(&stack);

        let emit = {
            let data = data.clone();
            let on_change = on_change.clone();
            Rc::new(move || {
                if let Some(cb) = on_change.borrow().clone() {
                    cb(data.borrow().clone());
                }
            })
        };

        {
            let data = data.clone();
            let stack = stack.clone();
            let custom_entry = custom_entry.clone();
            let existing_entry = existing_entry.clone();
            let emit = emit.clone();
            recommended.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                {
                    let mut slot = data.borrow_mut();
                    slot.location = InstallLocation::Default;
                    slot.custom_path.clear();
                    slot.existing_binary.clear();
                    slot.binary_status = BinaryStatus::Untested;
                }
                custom_entry.set_text("");
                existing_entry.set_text("");
                stack.set_visible_child_name("default");
                emit();
            });
        }
        {
            let data = data.clone();
            let stack = stack.clone();
            let existing_entry = existing_entry.clone();
            let emit = emit.clone();
            custom.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                {
                    let mut slot = data.borrow_mut();
                    slot.location = InstallLocation::Custom;
                    slot.existing_binary.clear();
                    slot.binary_status = BinaryStatus::Untested;
                }
                existing_entry.set_text("");
                stack.set_visible_child_name("custom");
                emit();
            });
        }
        if include_existing {
            let data = data.clone();
            let stack = stack.clone();
            let custom_entry = custom_entry.clone();
            let emit = emit.clone();
            existing.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                {
                    let mut slot = data.borrow_mut();
                    slot.location = InstallLocation::Existing;
                    slot.custom_path.clear();
                    if slot.binary_status != BinaryStatus::Untested {
                        slot.binary_status = BinaryStatus::Untested;
                    }
                }
                custom_entry.set_text("");
                stack.set_visible_child_name("existing");
                emit();
            });
        }

        {
            let data = data.clone();
            let custom_error = custom_error.clone();
            let ctx = ctx.clone();
            let emit = emit.clone();
            custom_entry.connect_changed(move |entry| {
                let text = entry.text().to_string();
                data.borrow_mut().custom_path = text.clone();
                let invalid = !text.trim().is_empty()
                    && crate::validators::validate_absolute_path(text.trim()).is_err();
                custom_error.set_visible(invalid);
                custom_error.set_text(&if invalid {
                    ctx.t_or(
                        "shared.installationOptions.errors.invalidPath",
                        "Please enter a valid absolute path",
                    )
                } else {
                    String::new()
                });
                emit();
            });
        }
        {
            let data = data.clone();
            let status = status.clone();
            let ctx = ctx.clone();
            let emit = emit.clone();
            existing_entry.connect_changed(move |entry| {
                let text = entry.text().to_string();
                {
                    let mut slot = data.borrow_mut();
                    slot.existing_binary = text;
                    slot.binary_status = BinaryStatus::Untested;
                }
                status.set_text(&ctx.t_or(binary_status_key(BinaryStatus::Untested), "Not tested"));
                emit();
            });
        }

        {
            let custom_entry = custom_entry.clone();
            let root = root.clone();
            let folder = mode == InstallationMode::Install;
            custom_browse.connect_clicked(move |_| {
                let Some(win) = root.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let picker = gtk::FileDialog::new();
                let custom_entry = custom_entry.clone();
                if folder {
                    picker.select_folder(
                        Some(&win),
                        None::<gio::Cancellable>.as_ref(),
                        move |result| {
                            if let Ok(file) = result {
                                if let Some(path) = file.path() {
                                    custom_entry.set_text(&path.display().to_string());
                                }
                            }
                        },
                    );
                } else {
                    picker.open(
                        Some(&win),
                        None::<gio::Cancellable>.as_ref(),
                        move |result| {
                            if let Ok(file) = result {
                                if let Some(path) = file.path() {
                                    custom_entry.set_text(&path.display().to_string());
                                }
                            }
                        },
                    );
                }
            });
        }
        {
            let existing_entry = existing_entry.clone();
            let root = root.clone();
            existing_browse.connect_clicked(move |_| {
                let Some(win) = root.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let picker = gtk::FileDialog::new();
                let existing_entry = existing_entry.clone();
                picker.open(
                    Some(&win),
                    None::<gio::Cancellable>.as_ref(),
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                existing_entry.set_text(&path.display().to_string());
                            }
                        }
                    },
                );
            });
        }

        {
            let data = data.clone();
            let existing_entry = existing_entry.clone();
            let status = status.clone();
            let test = test.clone();
            let ctx = ctx.clone();
            let emit = emit.clone();
            test.connect_clicked(move |btn| {
                let path = existing_entry.text().to_string();
                data.borrow_mut().binary_status = BinaryStatus::Testing;
                status.set_text(&ctx.t_or(binary_status_key(BinaryStatus::Testing), "Testing..."));
                btn.set_sensitive(false);
                btn.set_label(&ctx.t_or("shared.installationOptions.testingAction", "Testing..."));
                emit();
                let result = test_rclone_binary(&path);
                data.borrow_mut().binary_status = result;
                status.set_text(&ctx.t_or(binary_status_key(result), "Not tested"));
                btn.set_sensitive(result != BinaryStatus::Valid);
                btn.set_label(&ctx.t_or("shared.installationOptions.test", "Test"));
                emit();
            });
        }

        Self {
            root,
            data,
            mode,
            on_change,
        }
    }

    pub fn data(&self) -> InstallationOptionsData {
        self.data.borrow().clone()
    }

    pub fn mode(&self) -> InstallationMode {
        self.mode
    }

    pub fn connect_changed(&self, f: impl Fn(InstallationOptionsData) + 'static) {
        *self.on_change.borrow_mut() = Some(Rc::new(f));
    }

    pub fn reset(&self) {
        *self.data.borrow_mut() = InstallationOptionsData::default();
    }
}
