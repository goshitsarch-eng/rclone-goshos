use super::dialogs;
use super::AppCtx;
use crate::config_steps::{self, wizard_steps, EditorStep, HELPERS, QUICK_ADD_OPS};
use crate::flags::parse_flag_value;
use crate::interactive::{
    allows_custom_value, apply_interactive_response, example_label, is_continue_disabled,
    selected_example_index, update_interactive_answer, InteractiveAnswer, InteractiveFlowState,
};
use crate::jobs::{
    first_path, flatten_rclone, parse_cli_flags, profile_src_dst, DEST_KEYS, SOURCE_KEYS,
};
use crate::operations::OperationType;
use crate::providers::{parse_providers, Provider, ProviderOption};
use crate::rclone::remote_fs;
use crate::store::{AppConfig, ProfileConfig, RemoteMeta};
use crate::value_mapper::{
    control_kind, filter_examples, human_to_machine, is_default_display, machine_to_human,
    matches_provider_rule, ControlKind,
};
use adw::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

struct WizardState {
    providers: Vec<Provider>,
    fields: HashMap<String, FieldWidget>,
    flow: InteractiveFlowState,
    parameters: Value,
}

#[derive(Clone)]
enum FieldWidget {
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

impl FieldWidget {
    fn add_to(&self, group: &adw::PreferencesGroup) {
        match self {
            Self::Entry(row) => group.add(row),
            Self::Switch(row) => group.add(row),
            Self::Combo(row, _) => group.add(row),
            Self::SearchCombo(search, _, _, _) => group.add(search),
            Self::Spin(row) => group.add(row),
            Self::Multi(row, _) => group.add(row),
        }
    }

    fn remove_from(&self, group: &adw::PreferencesGroup) {
        match self {
            Self::Entry(row) => Self::remove_child(group, row),
            Self::Switch(row) => Self::remove_child(group, row),
            Self::Combo(row, _) => Self::remove_child(group, row),
            Self::SearchCombo(search, _, _, _) => Self::remove_child(group, search),
            Self::Spin(row) => Self::remove_child(group, row),
            Self::Multi(row, _) => Self::remove_child(group, row),
        }
    }

    fn remove_child(group: &adw::PreferencesGroup, child: &impl IsA<gtk::Widget>) {
        if child.parent().is_some_and(|parent| parent == *group) {
            group.remove(child);
        }
    }

    fn set_visible(&self, visible: bool) {
        match self {
            Self::Entry(row) => row.set_visible(visible),
            Self::Switch(row) => row.set_visible(visible),
            Self::Combo(row, _) => row.set_visible(visible),
            Self::SearchCombo(search, combo, _, _) => {
                search.set_visible(visible);
                combo.set_visible(false);
            }
            Self::Spin(row) => row.set_visible(visible),
            Self::Multi(row, _) => row.set_visible(visible),
        }
    }

    fn search_help(&self) -> String {
        match self {
            Self::Entry(row) => row.tooltip_text().unwrap_or_default().to_string(),
            Self::Switch(row) => row.subtitle().unwrap_or_default().to_string(),
            Self::Combo(row, _) => row.subtitle().unwrap_or_default().to_string(),
            Self::SearchCombo(_, combo, _, _) => combo.subtitle().unwrap_or_default().to_string(),
            Self::Spin(row) => row.subtitle().unwrap_or_default().to_string(),
            Self::Multi(row, _) => row.subtitle().to_string(),
        }
    }

    fn display_text(&self) -> String {
        match self {
            Self::Entry(row) => row.text().to_string(),
            Self::Switch(row) => row.is_active().to_string(),
            Self::Combo(row, values) | Self::SearchCombo(_, row, values, _) => values
                .get(row.selected() as usize)
                .cloned()
                .unwrap_or_default(),
            Self::Spin(row) => {
                let v = row.value();
                if v.fract() == 0.0 {
                    format!("{}", v as i64)
                } else {
                    v.to_string()
                }
            }
            Self::Multi(_, items) => items
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(value, _)| value.clone())
                .collect::<Vec<_>>()
                .join(","),
        }
    }

    fn connect_change(&self, callback: impl Fn() + 'static) {
        let callback = Rc::new(callback);
        match self {
            Self::Entry(row) => {
                let callback = callback.clone();
                row.connect_changed(move |_| callback());
            }
            Self::Switch(row) => {
                let callback = callback.clone();
                row.connect_active_notify(move |_| callback());
            }
            Self::Combo(row, _) | Self::SearchCombo(_, row, _, _) => {
                let callback = callback.clone();
                row.connect_selected_notify(move |_| callback());
            }
            Self::Spin(row) => {
                let callback = callback.clone();
                row.connect_changed(move |_| callback());
            }
            Self::Multi(_, items) => {
                for (_, check) in items.iter() {
                    let callback = callback.clone();
                    check.connect_active_notify(move |_| callback());
                }
            }
        }
    }

    fn set_display_text(&self, text: &str) {
        match self {
            Self::Entry(row) => row.set_text(text),
            Self::Switch(row) => row.set_active(text.eq_ignore_ascii_case("true")),
            Self::Combo(row, values) => {
                if let Some(idx) = values.iter().position(|v| v == text) {
                    row.set_selected(idx as u32);
                }
            }
            Self::SearchCombo(search, row, values, labels) => {
                if let Some(idx) = values.iter().position(|v| v == text) {
                    row.set_selected(idx as u32);
                    if let Some(label) = labels.get(idx) {
                        search.set_text(label);
                    }
                }
            }
            Self::Spin(row) => {
                if let Ok(v) = text.parse::<f64>() {
                    row.set_value(v);
                }
            }
            Self::Multi(_, items) => {
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
}

pub fn present(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    on_done: Rc<dyn Fn()>,
) {
    present_ex(parent, ctx, existing, false, None, on_done);
}

pub fn present_quick_add(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    present_ex(parent, ctx, None, true, None, on_done);
}

pub fn present_clone(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    source: String,
    on_done: Rc<dyn Fn()>,
) {
    present_ex(parent, ctx, None, false, Some(source), on_done);
}

fn present_ex(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    oauth_only: bool,
    clone_from: Option<String>,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    let title = if let Some(name) = existing.as_deref() {
        ctx.tf("modals.remoteConfig.title.edit", &[("target", name)])
    } else if let Some(source) = clone_from.as_deref() {
        format!(
            "{} — {source}",
            ctx.t_or("home.options.cloneRemote", "Clone Remote")
        )
    } else if oauth_only {
        ctx.t_or("wizards.quickAdd.title", "Quick Add Remote")
    } else {
        ctx.t_or("modals.remoteConfig.title.add", "Add New Remote")
    };
    dialog.set_title(&title);
    dialog.set_content_width(980);
    dialog.set_content_height(780);

    let mut providers = ctx
        .client()
        .and_then(|c| c.providers().ok())
        .map(|v| parse_providers(&v))
        .unwrap_or_default();
    if oauth_only {
        providers = crate::providers::oauth_supported_providers(&providers);
    }
    let seed_remote = existing.as_ref().or(clone_from.as_ref());
    let existing_params = seed_remote.and_then(|name| {
        ctx.client()
            .and_then(|c| c.dump_config().ok())
            .and_then(|dump| crate::providers::dump_remote_params(&dump, name))
    });
    let existing_meta = seed_remote.and_then(|name| ctx.store.borrow().remotes.get(name).cloned());

    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("wizards.remoteConfig.remoteName", "Remote name"));
    if let Some(existing) = &existing {
        name.set_text(existing);
        name.set_sensitive(false);
    } else if let Some(source) = &clone_from {
        let names = ctx.store.borrow().remote_names();
        name.set_text(&crate::store::unique_remote_name(&names, source));
    }
    {
        let ctx = ctx.clone();
        let editing = existing.clone();
        name.connect_changed(move |row| {
            let names = ctx.store.borrow().remote_names();
            match crate::validators::validate_remote_name(&row.text(), &names, editing.as_deref()) {
                Ok(()) => {
                    row.remove_css_class("error");
                    row.set_tooltip_text(None);
                }
                Err(key) => {
                    row.add_css_class("error");
                    row.set_tooltip_text(Some(&ctx.t_or(&key, &key)));
                }
            }
        });
    }

    let type_row = adw::ComboRow::new();
    type_row.set_title(&ctx.t_or("wizards.remoteConfig.remoteType", "Provider"));
    let labels: Vec<String> = if providers.is_empty() {
        [
            "drive", "s3", "dropbox", "onedrive", "sftp", "webdav", "local", "crypt",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    } else {
        providers
            .iter()
            .map(|p| format!("{} — {}", p.name, p.description))
            .collect()
    };
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    type_row.set_model(Some(&gtk::StringList::new(&label_refs)));
    let type_search = adw::EntryRow::new();
    type_search.set_title(&ctx.t_or("wizards.remoteConfig.remoteType", "Provider"));
    if let Some(label) = labels.first() {
        type_search.set_text(label);
    }
    attach_provider_typeahead(&ctx, &type_search, &type_row, &providers, &labels);
    type_row.set_visible(false);

    let fields_group = adw::PreferencesGroup::new();
    fields_group.set_title(&ctx.t_or("wizards.remoteConfig.fields", "Provider options"));
    let advanced_group = adw::PreferencesGroup::new();
    advanced_group.set_title(&ctx.t_or("wizards.remoteConfig.advancedOptions", "Advanced options"));
    let field_query = Rc::new(RefCell::new(String::new()));
    let state = Rc::new(RefCell::new(WizardState {
        providers: providers.clone(),
        fields: HashMap::new(),
        flow: InteractiveFlowState::default(),
        parameters: json!({}),
    }));
    let obscure_fields = Rc::new(RefCell::new(
        providers
            .first()
            .map(|provider| crate::providers::sensitive_field_labels(&providers, &provider.name))
            .unwrap_or_default(),
    ));
    let obscure_refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rebuilding = Rc::new(Cell::new(false));
    let command_options = Rc::new(RefCell::new(
        crate::command_options::initial_command_options(),
    ));
    let custom_options: Rc<RefCell<Vec<crate::command_options::CustomCommandOption>>> =
        Rc::new(RefCell::new(Vec::new()));
    let show_advanced = Rc::new(Cell::new(false));
    let json_mode = Rc::new(Cell::new(ctx.settings.borrow().runtime.show_json_mode));
    let show_cmd = Rc::new(Cell::new(false));
    let json_view = gtk::TextView::new();
    json_view.set_wrap_mode(gtk::WrapMode::WordChar);
    json_view.set_monospace(true);
    json_view.set_left_margin(8);
    json_view.set_right_margin(8);
    json_view.set_top_margin(8);
    json_view.set_bottom_margin(8);
    fill_json_view(
        &json_view,
        &collect_params(&state),
        ctx.settings.borrow().general.restrict,
    );
    rebuild_fields(
        parent,
        &ctx,
        &fields_group,
        &advanced_group,
        &state,
        providers.first(),
        true,
        &rebuilding,
        &field_query.borrow(),
    );
    if !current_vendor(&state).is_empty() {
        rebuild_fields(
            parent,
            &ctx,
            &fields_group,
            &advanced_group,
            &state,
            providers.first(),
            true,
            &rebuilding,
            &field_query.borrow(),
        );
    }
    if let Some(ref params) = existing_params {
        if let Some(type_name) = crate::providers::dump_provider_type(params) {
            if let Some(idx) = crate::providers::provider_index_by_name(&providers, &type_name) {
                type_row.set_selected(idx as u32);
                if let Some(label) = labels.get(idx) {
                    type_search.set_text(label);
                }
            }
        }
        apply_dump_to_wizard_fields(&state, params);
        fill_json_view(
            &json_view,
            &collect_params(&state),
            ctx.settings.borrow().general.restrict,
        );
    }

    {
        let fields_group = fields_group.clone();
        let advanced_group = advanced_group.clone();
        let state = state.clone();
        let providers = providers.clone();
        let parent = parent.clone();
        let ctx = ctx.clone();
        let existing_params = existing_params.clone();
        let rebuilding = rebuilding.clone();
        let json_mode = json_mode.clone();
        let json_view = json_view.clone();
        let obscure_fields = obscure_fields.clone();
        let obscure_refresh = obscure_refresh.clone();
        let field_query = field_query.clone();
        type_row.connect_selected_notify(move |row| {
            let provider = providers.get(row.selected() as usize);
            *obscure_fields.borrow_mut() = provider
                .map(|p| crate::providers::sensitive_field_labels(&providers, &p.name))
                .unwrap_or_default();
            if let Some(refresh) = obscure_refresh.borrow().clone() {
                refresh();
            }
            rebuild_fields(
                &parent,
                &ctx,
                &fields_group,
                &advanced_group,
                &state,
                provider,
                true,
                &rebuilding,
                &field_query.borrow(),
            );
            if json_mode.get() {
                fill_json_view(
                    &json_view,
                    &collect_params(&state),
                    ctx.settings.borrow().general.restrict,
                );
            }
            if let Some(params) = existing_params.as_ref() {
                let matches_type = crate::providers::dump_provider_type(params).is_some_and(|t| {
                    provider.is_some_and(|p| {
                        p.name.eq_ignore_ascii_case(&t) || p.prefix.eq_ignore_ascii_case(&t)
                    })
                });
                if matches_type {
                    apply_dump_to_wizard_fields(&state, params);
                }
            }
        });
    }

    let mount = adw::EntryRow::new();
    mount.set_title(&ctx.t_or("wizards.cliImport.mountPoint", "Mount point"));
    super::dialogs::attach_path_picker(
        &ctx,
        &mount,
        crate::picker::FilePickerConfig::local_mount_folders(),
    );
    let src = adw::EntryRow::new();
    src.set_title(&ctx.t_or("remoteConfig.source", "Default source path"));
    super::dialogs::attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
    let dst = adw::EntryRow::new();
    dst.set_title(&ctx.t_or("remoteConfig.dest", "Default destination path"));
    super::dialogs::attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
    let serve_types = ctx.serve_types();
    let mount_types = ctx.mount_types();
    let serve = adw::ComboRow::new();
    serve.set_title(&ctx.t_or("wizards.cliImport.serveType", "Default serve type"));
    serve.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&serve_types),
    )));
    let mount_type = adw::ComboRow::new();
    mount_type.set_title(&ctx.t_or("remoteConfig.mountType", "Default mount type"));
    mount_type.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&mount_types),
    )));
    let cron = adw::EntryRow::new();
    cron.set_title(&ctx.t_or("remoteConfig.cron", "Default cron"));
    let tray = adw::SwitchRow::new();
    tray.set_title(&ctx.t_or("remoteConfig.showOnTray", "Show in tray"));
    tray.set_active(true);
    let autostart = adw::SwitchRow::new();
    autostart.set_title(&ctx.t_or("remoteConfig.autoStart", "Auto-start mount / jobs"));
    let cli = adw::EntryRow::new();
    cli.set_title(&ctx.t_or(
        "wizards.cliImport.placeholder",
        "Import CLI flags (--transfers 8 --vfs-cache-mode full)",
    ));
    let cli_preview = gtk::Button::with_label(&ctx.t_or("wizards.cliImport.preview", "Preview"));
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let cli = cli.clone();
        let src = src.clone();
        let dst = dst.clone();
        let mount = mount.clone();
        let serve = serve.clone();
        let mount_type = mount_type.clone();
        let serve_types = serve_types.clone();
        let mount_types = mount_types.clone();
        let type_row = type_row.clone();
        let providers = providers.clone();
        cli_preview.connect_clicked(move |_| {
            let remote_type = provider_type(&providers, type_row.selected());
            super::dialogs::present_cli_import(
                &dialog,
                ctx.clone(),
                super::dialogs::CliImportOptions {
                    preferred: None,
                    remote_type,
                    is_quick_run: false,
                    can_create_new: false,
                    can_patch: true,
                    existing_profiles: Vec::new(),
                    initial_cli: cli.text().to_string(),
                },
                {
                    let cli = cli.clone();
                    let src = src.clone();
                    let dst = dst.clone();
                    let mount = mount.clone();
                    let serve = serve.clone();
                    let mount_type = mount_type.clone();
                    let serve_types = serve_types.clone();
                    let mount_types = mount_types.clone();
                    move |apply| {
                        match apply.verb.as_deref() {
                            Some("mount") => {
                                if let Some(path) = &apply.source_path {
                                    src.set_text(path);
                                }
                                if let Some(path) = &apply.dest_path {
                                    mount.set_text(path);
                                }
                            }
                            _ => {
                                if let Some(path) = &apply.source_path {
                                    src.set_text(path);
                                }
                                if let Some(path) = &apply.dest_path {
                                    dst.set_text(path);
                                }
                            }
                        }
                        if let Some(subtype) = &apply.serve_subtype {
                            if let Some(idx) = serve_types.iter().position(|s| s == subtype) {
                                serve.set_selected(idx as u32);
                            }
                        }
                        if let Some(subtype) = &apply.mount_subtype {
                            if let Some(idx) = mount_types.iter().position(|s| s == subtype) {
                                mount_type.set_selected(idx as u32);
                            }
                        }
                        let reconstructed = crate::cli_import::reconstruct_cli(&apply);
                        if !reconstructed.is_empty() {
                            cli.set_text(&reconstructed);
                        }
                    }
                },
            );
        });
    }
    cli.add_suffix(&cli_preview);
    let obscure = {
        let state = state.clone();
        dialogs::obscure_tool(
            &ctx,
            obscure_fields.clone(),
            Rc::new(move |key, value| {
                if let Some(widget) = state.borrow().fields.get(key) {
                    widget.set_display_text(value);
                }
                state.borrow_mut().parameters[key] = json!(value);
            }),
        )
    };
    {
        let obscure = obscure.clone();
        *obscure_refresh.borrow_mut() = Some(Rc::new(move || obscure.refresh_targets()));
    }
    let op_flags: Rc<RefCell<Vec<(OperationType, adw::SwitchRow, adw::EntryRow, adw::EntryRow)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let mut op_pages: Vec<(OperationType, adw::PreferencesPage)> = Vec::new();
    let ops_group = adw::PreferencesGroup::new();
    let ops_for_page: &[OperationType] = if oauth_only {
        &QUICK_ADD_OPS
    } else {
        &OperationType::ALL
    };
    ops_group.set_title(&ctx.t_or(
        if oauth_only {
            "modals.quickAdd.operations.title"
        } else {
            "modals.remoteConfig.steps.profiles"
        },
        if oauth_only {
            "Operation Options (Optional)"
        } else {
            "Per-operation profiles"
        },
    ));
    if oauth_only {
        ops_group.set_description(Some(&ctx.t_or(
            "modals.quickAdd.operations.description",
            "Configure operations to run automatically after the remote is created.",
        )));
    }
    for op in ops_for_page {
        let op = *op;
        let (label_key, desc_key, fallback_label, fallback_desc) = match op {
            OperationType::Mount => (
                "modals.quickAdd.operations.mount.label",
                "modals.quickAdd.operations.mount.description",
                "Mount",
                "Automatically mount this remote as a drive.",
            ),
            OperationType::Sync => (
                "modals.quickAdd.operations.sync.label",
                "modals.quickAdd.operations.sync.description",
                "Sync",
                "Sync this remote to a local folder.",
            ),
            OperationType::Copy => (
                "modals.quickAdd.operations.copy.label",
                "modals.quickAdd.operations.copy.description",
                "Copy",
                "Copy contents to a local folder.",
            ),
            OperationType::Bisync => (
                "modals.quickAdd.operations.bisync.label",
                "modals.quickAdd.operations.bisync.description",
                "Bisync",
                "Bidirectional sync with a local folder.",
            ),
            OperationType::Move => (
                "modals.quickAdd.operations.move.label",
                "modals.quickAdd.operations.move.description",
                "Move",
                "Move contents to a local folder.",
            ),
            OperationType::Serve => (
                "modals.quickAdd.operations.serve.label",
                "modals.quickAdd.operations.serve.description",
                "Serve",
                "Run a background server to share files.",
            ),
            _ => ("", "", op.api_label(), ""),
        };
        let enable = adw::SwitchRow::new();
        if oauth_only && !label_key.is_empty() {
            enable.set_title(&ctx.t_or(label_key, fallback_label));
            enable.set_subtitle(&ctx.t_or(desc_key, fallback_desc));
        } else {
            enable.set_title(&format!(
                "{} {}",
                ctx.t_or("common.configure", "Configure"),
                op.api_label()
            ));
        }
        enable.set_active(matches!(
            op,
            OperationType::Mount | OperationType::Sync | OperationType::Serve
        ));
        let osrc = adw::EntryRow::new();
        osrc.set_title(&format!(
            "{} {}",
            op.as_str(),
            ctx.t_or("remoteConfig.source", "source")
        ));
        if op != OperationType::Copyurl {
            super::dialogs::attach_path_picker(
                &ctx,
                &osrc,
                crate::picker::FilePickerConfig::folders(),
            );
        }
        let odst = adw::EntryRow::new();
        odst.set_title(&format!(
            "{} {}",
            op.as_str(),
            ctx.t_or("remoteConfig.dest", "destination / mount / addr")
        ));
        if op == OperationType::Mount {
            super::dialogs::attach_path_picker(
                &ctx,
                &odst,
                crate::picker::FilePickerConfig::local_mount_folders(),
            );
        } else if op != OperationType::Serve && op != OperationType::Delete {
            super::dialogs::attach_path_picker(
                &ctx,
                &odst,
                crate::picker::FilePickerConfig::folders(),
            );
        }
        if oauth_only {
            ops_group.add(&enable);
            ops_group.add(&osrc);
            ops_group.add(&odst);
        } else {
            let page = adw::PreferencesPage::new();
            let group = adw::PreferencesGroup::new();
            group.set_title(&ctx.t_or(
                &format!("modals.remoteConfig.steps.{}", op.as_str()),
                op.api_label(),
            ));
            group.set_description(Some(&ctx.t_or(
                "modals.remoteConfig.steps.profiles",
                "Enable this operation and set source / destination paths.",
            )));
            group.add(&enable);
            group.add(&osrc);
            group.add(&odst);
            page.add(&group);
            op_pages.push((op, page));
        }
        op_flags.borrow_mut().push((op, enable, osrc, odst));
    }
    if let Some(ref meta) = existing_meta {
        apply_existing_meta(
            meta,
            &mount,
            &src,
            &dst,
            &serve,
            &serve_types,
            &mount_type,
            &mount_types,
            &cron,
            &tray,
            &autostart,
            &op_flags,
        );
    }

    let question_title = gtk::Label::new(Some(&ctx.t_or(
        "wizards.remoteConfig.configRequired",
        "Interactive configuration",
    )));
    question_title.add_css_class("title-4");
    question_title.set_xalign(0.0);
    let question_help = gtk::Label::new(Some(&ctx.t_or(
        "wizards.remoteConfig.nextQuestionHelp",
        "Authorize the provider or answer rclone's configuration questions.",
    )));
    question_help.set_wrap(true);
    question_help.set_xalign(0.0);
    question_help.add_css_class("dim-label");
    let question_error = gtk::Label::new(None);
    question_error.add_css_class("error");
    question_error.set_wrap(true);
    question_error.set_xalign(0.0);
    let answer_row = adw::EntryRow::new();
    answer_row.set_title(&ctx.t_or("wizards.remoteConfig.enterValue", "Answer"));
    let answer_switch = adw::SwitchRow::new();
    answer_switch.set_title(&ctx.t_or("wizards.remoteConfig.yes", "Yes / enabled"));
    let example_row = adw::ComboRow::new();
    example_row.set_title(&ctx.t_or("wizards.remoteConfig.chooseOption", "Choose an option"));
    let oauth = super::interactive::OAuthHelper::new(&ctx);

    let nav = adw::ViewStack::new();
    let setup = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    identity.set_title(&ctx.t_or("wizards.remoteConfig.remoteName", "Identity"));
    identity.add(&name);
    identity.add(&type_search);
    identity.add(&type_row);
    setup.add(&identity);

    let mode_group = adw::PreferencesGroup::new();
    mode_group.set_title(&ctx.t_or("wizards.remoteConfig.fields", "Fields"));
    let field_search = gtk::SearchEntry::new();
    field_search.set_placeholder_text(Some(
        &ctx.t_or("wizards.remoteConfig.searchFields", "Search fields"),
    ));
    field_search.set_hexpand(true);
    {
        let state = state.clone();
        let field_query = field_query.clone();
        field_search.connect_search_changed(move |entry| {
            *field_query.borrow_mut() = entry.text().to_string();
            apply_field_search(&state, &field_query.borrow());
        });
    }
    let field_search_row = adw::ActionRow::new();
    field_search_row.set_title(&ctx.t_or("wizards.remoteConfig.searchFields", "Search fields"));
    field_search_row.add_suffix(&field_search);
    mode_group.add(&field_search_row);
    let adv_switch = adw::SwitchRow::new();
    let adv_on = ctx.t_or("wizards.remoteConfig.hideAdvanced", "Hide Advanced Options");
    let adv_off = ctx.t_or("wizards.remoteConfig.showAdvanced", "Show Advanced Options");
    adv_switch.set_title(if show_advanced.get() {
        &adv_on
    } else {
        &adv_off
    });
    adv_switch.set_active(show_advanced.get());
    let json_switch = adw::SwitchRow::new();
    let json_on = ctx.t_or("wizards.remoteConfig.switchToForm", "Switch to Form Mode");
    let json_off = ctx.t_or("wizards.remoteConfig.switchToJson", "Switch to JSON Mode");
    json_switch.set_title(if json_mode.get() { &json_on } else { &json_off });
    json_switch.set_active(json_mode.get());
    let cmd_switch = adw::SwitchRow::new();
    let cmd_on = ctx.t_or(
        "wizards.remoteConfig.hideCommandOptions",
        "Hide Command Options",
    );
    let cmd_off = ctx.t_or(
        "wizards.remoteConfig.showCommandOptions",
        "Show Command Options",
    );
    cmd_switch.set_title(if show_cmd.get() { &cmd_on } else { &cmd_off });
    cmd_switch.set_active(show_cmd.get());
    mode_group.add(&adv_switch);
    mode_group.add(&json_switch);
    mode_group.add(&cmd_switch);
    setup.add(&mode_group);

    let cmd_group = adw::PreferencesGroup::new();
    cmd_group.set_title(&ctx.t_or("wizards.remoteConfig.showCommandOptions", "Command options"));
    cmd_group.set_visible(show_cmd.get());
    for def in crate::command_options::PREDEFINED_OPTIONS {
        let row = adw::SwitchRow::new();
        row.set_title(&ctx.t_or(def.label_key, def.key));
        row.set_subtitle(&ctx.t_or(def.description_key, ""));
        row.set_active(crate::command_options::option_enabled(
            &command_options.borrow(),
            def.key,
        ));
        {
            let command_options = command_options.clone();
            let key = def.key;
            row.connect_active_notify(move |row| {
                crate::command_options::set_option(
                    &mut command_options.borrow_mut(),
                    key,
                    row.is_active(),
                );
            });
        }
        cmd_group.add(&row);
    }
    let custom_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let rebuild_custom: Rc<RefCell<Rc<dyn Fn()>>> = Rc::new(RefCell::new(Rc::new(|| {})));
    let refresh_custom = {
        let custom_box = custom_box.clone();
        let custom_options = custom_options.clone();
        let rebuild_custom = rebuild_custom.clone();
        let ctx = ctx.clone();
        Rc::new(move || {
            while let Some(child) = custom_box.first_child() {
                custom_box.remove(&child);
            }
            let items = custom_options.borrow().clone();
            for (idx, option) in items.iter().enumerate() {
                let row = match option.kind {
                    crate::command_options::CustomValueKind::Bool => {
                        let switch = adw::SwitchRow::new();
                        switch.set_title(&option.key);
                        switch.set_subtitle(option.kind.as_str());
                        switch.set_active(option.bool_value);
                        let custom_options = custom_options.clone();
                        switch.connect_active_notify(move |row| {
                            if let Some(item) = custom_options.borrow_mut().get_mut(idx) {
                                item.bool_value = row.is_active();
                            }
                        });
                        switch.upcast::<gtk::Widget>()
                    }
                    crate::command_options::CustomValueKind::Number => {
                        let spin = adw::SpinRow::with_range(-1_000_000_000.0, 1_000_000_000.0, 1.0);
                        spin.set_title(&option.key);
                        spin.set_subtitle(option.kind.as_str());
                        spin.set_value(option.number_value);
                        let custom_options = custom_options.clone();
                        spin.connect_changed(move |row| {
                            if let Some(item) = custom_options.borrow_mut().get_mut(idx) {
                                item.number_value = row.value();
                            }
                        });
                        spin.upcast::<gtk::Widget>()
                    }
                    crate::command_options::CustomValueKind::Array => {
                        let entry = adw::EntryRow::new();
                        entry.set_title(&format!("{} ({})", option.key, option.kind.as_str()));
                        entry.set_text(&option.array_value.join(", "));
                        let custom_options = custom_options.clone();
                        entry.connect_changed(move |row| {
                            if let Some(item) = custom_options.borrow_mut().get_mut(idx) {
                                item.array_value =
                                    crate::command_options::parse_array_chips(&row.text());
                            }
                        });
                        entry.upcast::<gtk::Widget>()
                    }
                    crate::command_options::CustomValueKind::Text => {
                        let entry = adw::EntryRow::new();
                        entry.set_title(&format!("{} ({})", option.key, option.kind.as_str()));
                        entry.set_text(&option.text_value);
                        let custom_options = custom_options.clone();
                        entry.connect_changed(move |row| {
                            if let Some(item) = custom_options.borrow_mut().get_mut(idx) {
                                item.text_value = row.text().to_string();
                            }
                        });
                        entry.upcast::<gtk::Widget>()
                    }
                };
                let remove = gtk::Button::from_icon_name("user-trash-symbolic");
                remove.set_valign(gtk::Align::Center);
                let remove_title = ctx.t_or("wizards.remoteConfig.removeOption", "Remove option");
                remove.set_tooltip_text(Some(&remove_title));
                let custom_options = custom_options.clone();
                let rebuild_custom = rebuild_custom.clone();
                remove.connect_clicked(move |_| {
                    let mut items = custom_options.borrow_mut();
                    if idx < items.len() {
                        items.remove(idx);
                    }
                    drop(items);
                    rebuild_custom.borrow()();
                });
                if let Ok(action) = row.clone().downcast::<adw::ActionRow>() {
                    action.add_suffix(&remove);
                } else if let Ok(entry) = row.clone().downcast::<adw::EntryRow>() {
                    entry.add_suffix(&remove);
                } else if let Ok(switch) = row.clone().downcast::<adw::SwitchRow>() {
                    switch.add_suffix(&remove);
                } else if let Ok(spin) = row.clone().downcast::<adw::SpinRow>() {
                    spin.add_suffix(&remove);
                }
                custom_box.append(&row);
            }
        })
    };
    *rebuild_custom.borrow_mut() = refresh_custom.clone();
    refresh_custom();
    cmd_group.add(&{
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("wizards.remoteConfig.addOption", "Custom options"));
        row.set_activatable(false);
        row
    });
    cmd_group.add(&{
        let holder = adw::ActionRow::new();
        holder.set_activatable(false);
        holder.add_suffix(&custom_box);
        holder
    });
    let custom_key = adw::EntryRow::new();
    custom_key.set_title(&ctx.t_or("wizards.remoteConfig.optionKey", "Option Key"));
    custom_key.set_tooltip_text(Some(
        &ctx.t_or("wizards.remoteConfig.optionKeyPlaceholder", "e.g., myFlag"),
    ));
    let custom_type = adw::ComboRow::new();
    custom_type.set_title(&ctx.t_or("wizards.remoteConfig.optionType", "Option Type"));
    custom_type.set_model(Some(&gtk::StringList::new(&[
        "boolean", "string", "number", "array",
    ])));
    let add_custom =
        gtk::Button::with_label(&ctx.t_or("wizards.remoteConfig.addOption", "Add Option"));
    {
        let custom_options = custom_options.clone();
        let custom_key = custom_key.clone();
        let custom_type = custom_type.clone();
        let refresh_custom = refresh_custom.clone();
        add_custom.connect_clicked(move |_| {
            let kind = match custom_type.selected() {
                0 => crate::command_options::CustomValueKind::Bool,
                2 => crate::command_options::CustomValueKind::Number,
                3 => crate::command_options::CustomValueKind::Array,
                _ => crate::command_options::CustomValueKind::Text,
            };
            let key = custom_key.text().to_string();
            if custom_options
                .borrow()
                .iter()
                .any(|item| item.key.eq_ignore_ascii_case(key.trim()))
            {
                return;
            }
            if let Some(option) = crate::command_options::new_custom(&key, kind) {
                custom_options.borrow_mut().push(option);
                custom_key.set_text("");
                refresh_custom();
            }
        });
    }
    cmd_group.add(&custom_key);
    cmd_group.add(&custom_type);
    cmd_group.add(&{
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("wizards.remoteConfig.addOption", "Add Option"));
        row.add_suffix(&add_custom);
        row
    });
    setup.add(&cmd_group);
    setup.add(&fields_group);
    setup.add(&advanced_group);
    advanced_group.set_visible(show_advanced.get() && !json_mode.get());
    fields_group.set_visible(!json_mode.get());

    let json_group = adw::PreferencesGroup::new();
    json_group.set_title(&ctx.t_or("wizards.remoteConfig.switchToJson", "Parameters JSON"));
    json_group.set_description(Some(&ctx.t_or(
        "wizards.remoteConfig.jsonEditorInfo.runtimeRemote",
        "Edit the remote parameters as JSON. Switch back to form to apply.",
    )));
    let json_scroll = gtk::ScrolledWindow::new();
    json_scroll.set_min_content_height(240);
    json_scroll.set_hexpand(true);
    json_scroll.set_vexpand(true);
    json_scroll.set_child(Some(&json_view));
    json_group.add(&json_scroll);
    json_group.set_visible(json_mode.get());
    setup.add(&json_group);

    {
        let show_advanced = show_advanced.clone();
        let json_mode = json_mode.clone();
        let advanced_group = advanced_group.clone();
        let adv_on = adv_on.clone();
        let adv_off = adv_off.clone();
        adv_switch.connect_active_notify(move |row| {
            show_advanced.set(row.is_active());
            row.set_title(if row.is_active() { &adv_on } else { &adv_off });
            advanced_group.set_visible(row.is_active() && !json_mode.get());
        });
    }
    {
        let json_mode = json_mode.clone();
        let ctx = ctx.clone();
        let state = state.clone();
        let json_view = json_view.clone();
        let fields_group = fields_group.clone();
        let advanced_group = advanced_group.clone();
        let show_advanced = show_advanced.clone();
        let json_on = json_on.clone();
        let json_off = json_off.clone();
        let json_group = json_group.clone();
        json_switch.connect_active_notify(move |row| {
            let next = row.is_active();
            if next == json_mode.get() {
                row.set_title(if next { &json_on } else { &json_off });
                return;
            }
            if next {
                fill_json_view(
                    &json_view,
                    &collect_params(&state),
                    ctx.settings.borrow().general.restrict,
                );
                json_mode.set(true);
            } else if let Err(e) = apply_json_to_form(&state, &json_view) {
                row.set_active(true);
                let err = adw::AlertDialog::new(
                    Some(&ctx.t_or(
                        "wizards.remoteConfig.unknownTopLevelProperty",
                        "Invalid JSON",
                    )),
                    Some(&e),
                );
                err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                err.present(Some(row));
                return;
            } else {
                json_mode.set(false);
            }
            ctx.settings.borrow_mut().runtime.show_json_mode = json_mode.get();
            ctx.persist();
            row.set_title(if json_mode.get() { &json_on } else { &json_off });
            fields_group.set_visible(!json_mode.get());
            advanced_group.set_visible(show_advanced.get() && !json_mode.get());
            json_group.set_visible(json_mode.get());
        });
    }
    {
        let show_cmd = show_cmd.clone();
        let cmd_group = cmd_group.clone();
        let cmd_on = cmd_on.clone();
        let cmd_off = cmd_off.clone();
        cmd_switch.connect_active_notify(move |row| {
            show_cmd.set(row.is_active());
            row.set_title(if row.is_active() { &cmd_on } else { &cmd_off });
            cmd_group.set_visible(row.is_active());
        });
    }
    let defaults = adw::PreferencesGroup::new();
    defaults.set_title(&ctx.t_or("modals.remoteConfig.steps.profiles", "Default profiles"));
    defaults.add(&mount);
    defaults.add(&src);
    defaults.add(&dst);
    defaults.add(&serve);
    defaults.add(&mount_type);
    defaults.add(&cron);
    let cron_row = super::dialogs::attach_cron_builder_row(&cron, &ctx);
    defaults.add(&cron_row);
    defaults.add(&tray);
    defaults.add(&autostart);
    defaults.add(&cli);
    obscure.add_to_group(&defaults);
    setup.add(&defaults);

    nav.add_titled(
        &setup,
        Some("remote"),
        &ctx.t_or("modals.remoteConfig.steps.remote", "Remote"),
    );
    if oauth_only {
        let operations = adw::PreferencesPage::new();
        operations.add(&ops_group);
        nav.add_titled(
            &operations,
            Some("operations"),
            &ctx.t_or("modals.quickAdd.operations.title", "Operations"),
        );
    } else {
        for (op, page) in op_pages {
            nav.add_titled(
                &page,
                Some(op.as_str()),
                &ctx.t_or(
                    &format!("modals.remoteConfig.steps.{}", op.as_str()),
                    op.api_label(),
                ),
            );
        }
    }

    let interactive_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    interactive_box.set_margin_top(16);
    interactive_box.set_margin_start(16);
    interactive_box.set_margin_end(16);
    interactive_box.append(&question_title);
    interactive_box.append(&question_help);
    interactive_box.append(&question_error);
    interactive_box.append(&example_row);
    interactive_box.append(&answer_row);
    interactive_box.append(&answer_switch);
    interactive_box.append(&oauth.root);
    let cancel_oauth =
        gtk::Button::with_label(&ctx.t_or("modals.remoteConfig.cancelOauth", "Cancel OAuth"));
    {
        let ctx = ctx.clone();
        let oauth = oauth.clone();
        let name = name.clone();
        let editing = existing.clone();
        cancel_oauth.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                match client.oauth_stop() {
                    Ok(_) => {
                        if editing.is_none() {
                            let created = name.text().to_string();
                            if !created.is_empty() {
                                let _ = client.delete_remote(&created);
                            }
                        }
                        oauth.set_status(
                            &ctx.t_or("modals.remoteConfig.oauthCancelled", "OAuth cancelled"),
                        );
                    }
                    Err(e) => oauth.set_status(&e.to_string()),
                }
            }
        });
    }
    interactive_box.append(&cancel_oauth);
    nav.add_titled(
        &interactive_box,
        Some("interactive"),
        &ctx.t_or("wizards.remoteConfig.authenticationMethod", "Authorize"),
    );

    let helper_views: Rc<RefCell<Vec<(&'static str, gtk::TextView)>>> =
        Rc::new(RefCell::new(Vec::new()));
    if !oauth_only {
        let restrict = ctx.settings.borrow().general.restrict;
        for (kind, label, _) in HELPERS.iter().filter(|(kind, _, _)| *kind != "runtime") {
            let (page, view) =
                helper_json_page(&ctx, kind, label, existing_meta.as_ref(), restrict);
            helper_views.borrow_mut().push((*kind, view));
            nav.add_titled(
                &page,
                Some(*kind),
                &ctx.t_or(&format!("modals.remoteConfig.steps.{kind}"), label),
            );
        }
    }

    let runtime_name = adw::EntryRow::new();
    runtime_name.set_title(&ctx.t_or("remoteConfig.runtimeProfile", "Runtime profile"));
    let runtime_view = gtk::TextView::new();
    runtime_view.set_wrap_mode(gtk::WrapMode::WordChar);
    runtime_view.set_monospace(true);
    runtime_view.set_left_margin(8);
    runtime_view.set_right_margin(8);
    runtime_view.set_top_margin(8);
    runtime_view.set_bottom_margin(8);
    if let Some(meta) = &existing_meta {
        let names = meta.helper_names("runtime");
        let chosen = names
            .iter()
            .find(|n| n.eq_ignore_ascii_case("default"))
            .cloned()
            .or_else(|| names.into_iter().next())
            .unwrap_or_else(|| "default".into());
        runtime_name.set_text(&chosen);
        if let Some(value) = meta.helper_profile("runtime", &chosen) {
            fill_json_view(
                &runtime_view,
                &value,
                ctx.settings.borrow().general.restrict,
            );
        } else {
            fill_json_view(&runtime_view, &json!({}), false);
        }
    } else {
        runtime_name.set_text("default");
        fill_json_view(&runtime_view, &json!({}), false);
    }
    let runtime_page = adw::PreferencesPage::new();
    let runtime_group = adw::PreferencesGroup::new();
    runtime_group.set_title(&ctx.t_or("modals.remoteConfig.steps.runtimeRemote", "Runtime remote"));
    runtime_group.set_description(Some(&ctx.t_or(
        "wizards.remoteConfig.runtimeRemoteWarning.description",
        "These options are applied to jobs at runtime and are not written into rclone.conf.",
    )));
    runtime_group.add(&runtime_name);
    let runtime_typed = adw::PreferencesGroup::new();
    runtime_typed.set_title(&ctx.t_or("remoteConfig.options", "Provider options"));
    let runtime_flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let initial_runtime_json = parse_runtime_json(&textview_text(&runtime_view));
    let initial_runtime_type = provider_type(&state.borrow().providers, type_row.selected());
    fill_runtime_flag_rows(
        &ctx,
        &runtime_typed,
        &runtime_flag_rows,
        &initial_runtime_type,
        &initial_runtime_json,
    );
    {
        let ctx = ctx.clone();
        let runtime_typed = runtime_typed.clone();
        let runtime_flag_rows = runtime_flag_rows.clone();
        let runtime_view = runtime_view.clone();
        let state = state.clone();
        type_row.connect_selected_notify(move |row| {
            let remote_type = provider_type(&state.borrow().providers, row.selected());
            fill_runtime_flag_rows(
                &ctx,
                &runtime_typed,
                &runtime_flag_rows,
                &remote_type,
                &parse_runtime_json(&textview_text(&runtime_view)),
            );
        });
    }
    runtime_page.add(&runtime_typed);
    let runtime_scroll = gtk::ScrolledWindow::new();
    runtime_scroll.set_min_content_height(160);
    runtime_scroll.set_hexpand(true);
    runtime_scroll.set_vexpand(true);
    runtime_scroll.set_child(Some(&runtime_view));
    runtime_group.add(&runtime_scroll);
    runtime_page.add(&runtime_group);
    if !oauth_only {
        nav.add_titled(
            &runtime_page,
            Some("runtime"),
            &ctx.t_or("modals.remoteConfig.steps.runtimeRemote", "Runtime"),
        );
    }

    let steps = wizard_steps(oauth_only);
    let current_idx = Rc::new(Cell::new(0));
    let remote_valid = Rc::new(Cell::new(false));
    let nav_locked = Rc::new(Cell::new(false));
    {
        let names = ctx.store.borrow().remote_names();
        remote_valid.set(
            crate::validators::validate_remote_name(&name.text(), &names, existing.as_deref())
                .is_ok(),
        );
    }

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(230.0);
    split.set_show_sidebar(true);

    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_selection_mode(gtk::SelectionMode::Single);
    for step in &steps {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(&step.i18n_key(), step.fallback_label()));
        row.set_activatable(true);
        row.add_prefix(&gtk::Image::from_icon_name(step.icon_name()));
        sidebar.append(&row);
    }
    let side_scroll = gtk::ScrolledWindow::new();
    side_scroll.set_vexpand(true);
    side_scroll.set_child(Some(&sidebar));
    split.set_sidebar(Some(&side_scroll));

    let step_title = gtk::Label::new(Some(
        &ctx.t_or("modals.remoteConfig.steps.remote", "Remote"),
    ));
    step_title.add_css_class("title-3");
    step_title.set_xalign(0.0);
    step_title.set_hexpand(true);
    let side_toggle = gtk::Button::from_icon_name("sidebar-show-symbolic");
    side_toggle.set_tooltip_text(Some(&ctx.t_or("sidebar.toggleSidebar", "Toggle Sidebar")));
    {
        let split = split.clone();
        side_toggle.connect_clicked(move |_| {
            split.set_show_sidebar(!split.shows_sidebar());
        });
    }
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title_row.set_margin_start(12);
    title_row.set_margin_end(12);
    title_row.set_margin_top(10);
    title_row.append(&side_toggle);
    title_row.append(&step_title);

    let continue_btn = gtk::Button::with_label(&ctx.t_or(
        "wizards.remoteConfig.readyToContinue",
        "Continue / Authorize",
    ));
    continue_btn.add_css_class("suggested-action");
    let back = gtk::Button::with_label(&ctx.t_or("common.back", "Back"));
    let next = gtk::Button::with_label(&ctx.t_or("common.next", "Next"));
    next.add_css_class("suggested-action");
    let save = gtk::Button::with_label(&ctx.t_or(
        if existing.is_none() && clone_from.is_none() {
            "common.create"
        } else {
            "common.save"
        },
        if existing.is_none() && clone_from.is_none() {
            "Create"
        } else {
            "Save"
        },
    ));
    save.add_css_class("pill");

    let refresh_nav: Rc<RefCell<Rc<dyn Fn()>>> = Rc::new(RefCell::new(Rc::new(|| {})));
    let go_to: Rc<RefCell<Rc<dyn Fn(usize)>>> = Rc::new(RefCell::new(Rc::new(|_| {})));
    {
        let steps = steps.clone();
        let current_idx = current_idx.clone();
        let remote_valid = remote_valid.clone();
        let nav_locked = nav_locked.clone();
        let nav = nav.clone();
        let refresh_nav = refresh_nav.clone();
        *go_to.borrow_mut() = Rc::new(move |idx| {
            if !config_steps::is_step_clickable(
                idx,
                current_idx.get(),
                remote_valid.get(),
                nav_locked.get(),
            ) {
                return;
            }
            let Some(step) = steps.get(idx).copied() else {
                return;
            };
            current_idx.set(idx);
            nav.set_visible_child_name(step.page_name());
            refresh_nav.borrow()();
        });
    }
    {
        let steps = steps.clone();
        let current_idx = current_idx.clone();
        let remote_valid = remote_valid.clone();
        let nav_locked = nav_locked.clone();
        let nav = nav.clone();
        let split = split.clone();
        let sidebar = sidebar.clone();
        let step_title = step_title.clone();
        let back = back.clone();
        let next = next.clone();
        let continue_btn = continue_btn.clone();
        let save = save.clone();
        let ctx = ctx.clone();
        *refresh_nav.borrow_mut() = Rc::new(move || {
            let interactive = nav.visible_child_name().as_deref() == Some("interactive");
            let locked = nav_locked.get() || interactive;
            split.set_show_sidebar(!locked);
            let idx = current_idx.get();
            let step = steps.get(idx).copied().unwrap_or(EditorStep::Remote);
            if !interactive {
                nav.set_visible_child_name(step.page_name());
                if let Some(row) = sidebar.row_at_index(idx as i32) {
                    sidebar.select_row(Some(&row));
                }
            }
            let title = if interactive {
                ctx.t_or("wizards.remoteConfig.authenticationMethod", "Authorize")
            } else {
                ctx.t_or(&step.i18n_key(), step.fallback_label())
            };
            step_title.set_text(&title);
            for (i, _) in steps.iter().enumerate() {
                if let Some(row) = sidebar.row_at_index(i as i32) {
                    row.set_sensitive(config_steps::is_step_clickable(
                        i,
                        idx,
                        remote_valid.get(),
                        locked,
                    ));
                }
            }
            back.set_visible(idx > 0 && !locked);
            back.set_sensitive(!locked);
            next.set_visible(idx + 1 < steps.len() && !locked);
            next.set_sensitive(!config_steps::is_next_disabled(
                step.is_remote(),
                remote_valid.get(),
                locked,
            ));
            continue_btn.set_sensitive(remote_valid.get());
            save.set_visible(!locked);
            save.set_sensitive(remote_valid.get() && !locked);
        });
    }
    {
        let current_idx = current_idx.clone();
        let go_to = go_to.clone();
        sidebar.connect_row_selected(move |_, row| {
            let Some(row) = row else {
                return;
            };
            let idx = row.index();
            if idx < 0 || idx as usize == current_idx.get() {
                return;
            }
            go_to.borrow()(idx as usize);
        });
    }
    {
        let current_idx = current_idx.clone();
        let go_to = go_to.clone();
        back.connect_clicked(move |_| {
            if let Some(idx) = config_steps::prev_step_index(current_idx.get()) {
                go_to.borrow()(idx);
            }
        });
    }
    {
        let current_idx = current_idx.clone();
        let go_to = go_to.clone();
        let steps_len = steps.len();
        next.connect_clicked(move |_| {
            if let Some(idx) = config_steps::next_step_index(current_idx.get(), steps_len) {
                go_to.borrow()(idx);
            }
        });
    }
    {
        let remote_valid = remote_valid.clone();
        let refresh_nav = refresh_nav.clone();
        let ctx = ctx.clone();
        let editing = existing.clone();
        name.connect_changed(move |row| {
            let names = ctx.store.borrow().remote_names();
            remote_valid.set(
                crate::validators::validate_remote_name(&row.text(), &names, editing.as_deref())
                    .is_ok(),
            );
            refresh_nav.borrow()();
        });
    }

    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let name = name.clone();
        let type_row = type_row.clone();
        let state = state.clone();
        let question_title = question_title.clone();
        let question_help = question_help.clone();
        let question_error = question_error.clone();
        let answer_row = answer_row.clone();
        let answer_switch = answer_switch.clone();
        let example_row = example_row.clone();
        let oauth = oauth.clone();
        let nav = nav.clone();
        let command_options = command_options.clone();
        let custom_options = custom_options.clone();
        let json_mode = json_mode.clone();
        let json_view = json_view.clone();
        let nav_locked = nav_locked.clone();
        let refresh_nav = refresh_nav.clone();
        let go_to = go_to.clone();
        let steps = steps.clone();
        continue_btn.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            if remote_name.is_empty() {
                return;
            }
            let r#type = provider_type(&state.borrow().providers, type_row.selected());
            let params = match collect_wizard_params(&state, &json_mode, &json_view) {
                Ok(value) => value,
                Err(e) => {
                    let err = adw::AlertDialog::new(
                        Some(&ctx.t_or(
                            "wizards.remoteConfig.unknownTopLevelProperty",
                            "Invalid JSON",
                        )),
                        Some(&e),
                    );
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                    return;
                }
            };
            state.borrow_mut().parameters = params.clone();
            let Some(client) = ctx.client() else {
                return;
            };
            let result = {
                let flow = state.borrow().flow.clone();
                let opt = Some(crate::command_options::build_opt_with_custom(
                    &crate::command_options::sync_non_interactive(&command_options.borrow(), true),
                    &custom_options.borrow(),
                ));
                if !flow.is_active {
                    client.create_remote_interactive(&remote_name, &r#type, params, opt)
                } else {
                    if is_continue_disabled(&flow) {
                        return;
                    }
                    let option_type = flow
                        .question
                        .as_ref()
                        .and_then(|q| q.option.as_ref())
                        .map(|o| o.type_name.as_str())
                        .unwrap_or("string");
                    let answer = current_answer(&flow, &answer_row, &answer_switch, &example_row);
                    let token = flow
                        .question
                        .as_ref()
                        .map(|q| q.state.clone())
                        .unwrap_or_default();
                    client.continue_create_remote(
                        &remote_name,
                        &token,
                        answer.as_rc_result(option_type),
                        params,
                        opt,
                    )
                }
            };
            match result {
                Ok(value) => {
                    let next = apply_interactive_response(&value);
                    if let Ok((_, Some(url))) = client.oauth_status() {
                        let _ = open::that(&url);
                        oauth.set_url(&ctx, Some(&url));
                    }
                    if !next.is_active {
                        question_title.set_text(&ctx.t_or(
                            "wizards.remoteConfig.readyToContinue",
                            "Authorization complete",
                        ));
                        question_help.set_text(&ctx.t_or(
                            "wizards.remoteConfig.setupSubtitle",
                            "rclone finished the interactive flow. Review profiles and save.",
                        ));
                        question_error.set_text("");
                        nav_locked.set(false);
                        if steps.len() > 1 {
                            go_to.borrow()(1);
                        } else {
                            nav.set_visible_child_name("remote");
                            refresh_nav.borrow()();
                        }
                    } else {
                        apply_question_widgets(
                            &ctx,
                            &next,
                            &question_title,
                            &question_help,
                            &question_error,
                            &answer_row,
                            &answer_switch,
                            &example_row,
                        );
                        nav_locked.set(true);
                        nav.set_visible_child_name("interactive");
                        refresh_nav.borrow()();
                    }
                    state.borrow_mut().flow = next;
                }
                Err(e) => {
                    let err = adw::AlertDialog::new(
                        Some(&ctx.t_or(
                            "wizards.remoteConfig.configRequired",
                            "Configuration failed",
                        )),
                        Some(&e.to_string()),
                    );
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                }
            }
        });
    }

    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let existing = existing.clone();
        let clone_from = clone_from.clone();
        let state = state.clone();
        let name = name.clone();
        let type_row = type_row.clone();
        let mount = mount.clone();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let cron = cron.clone();
        let tray = tray.clone();
        let autostart = autostart.clone();
        let op_flags = op_flags.clone();
        let cli = cli.clone();
        let command_options = command_options.clone();
        let custom_options = custom_options.clone();
        let json_mode = json_mode.clone();
        let json_view = json_view.clone();
        let runtime_name = runtime_name.clone();
        let runtime_view = runtime_view.clone();
        let runtime_flag_rows = runtime_flag_rows.clone();
        let helper_views = helper_views.clone();
        save.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            let existing_names = ctx.store.borrow().remote_names();
            if let Err(e) = crate::validators::validate_remote_name(
                &remote_name,
                &existing_names,
                existing.as_deref(),
            ) {
                let err = adw::AlertDialog::new(
                    Some(&ctx.t_or(
                        "wizards.remoteConfig.remoteNameRequired",
                        "Invalid remote name",
                    )),
                    Some(&ctx.t_or(&e, &e)),
                );
                err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                err.present(Some(&dialog));
                return;
            }
            let r#type = provider_type(&state.borrow().providers, type_row.selected());
            let mut params = match collect_wizard_params(&state, &json_mode, &json_view) {
                Ok(value) => value,
                Err(e) => {
                    let err = adw::AlertDialog::new(
                        Some(&ctx.t_or(
                            "wizards.remoteConfig.unknownTopLevelProperty",
                            "Invalid JSON",
                        )),
                        Some(&e),
                    );
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                    return;
                }
            };
            if let Some(provider) = state
                .borrow()
                .providers
                .iter()
                .find(|p| p.prefix == r#type || p.name == r#type)
                .cloned()
            {
                let fields = state.borrow().fields.clone();
                for option in &provider.options {
                    let value = if json_mode.get() {
                        params
                            .get(&option.name)
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                other => other.to_string().trim_matches('"').to_string(),
                            })
                            .unwrap_or_default()
                    } else {
                        let Some(widget) = fields.get(&option.name) else {
                            continue;
                        };
                        widget.display_text()
                    };
                    if let Err(e) = crate::validators::validate_option(option, &value) {
                        let err = adw::AlertDialog::new(
                            Some(&ctx.t_or(
                                "wizards.remoteConfig.fieldRequired",
                                "Invalid provider field",
                            )),
                            Some(&e),
                        );
                        err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                        err.present(Some(&dialog));
                        return;
                    }
                }
            }
            let engine_os = ctx.engine_os();
            let mount_status = crate::path_inspection::inspect_local_ex(
                &mount.text(),
                ctx.client().as_ref(),
                &engine_os,
            );
            if let Err(e) = crate::path_inspection::ensure_path_ex(
                &mount_status,
                &mount.text(),
                ctx.client().as_ref(),
                &engine_os,
            ) {
                let err = adw::AlertDialog::new(
                    Some(&ctx.t_or(
                        "wizards.remoteConfig.allowOtherWarning.title",
                        "Could not create mount path",
                    )),
                    Some(&e),
                );
                err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                err.present(Some(&dialog));
                return;
            }
            if let Some(client) = ctx.client() {
                let selected = command_options.borrow().clone();
                let opt = Some(crate::command_options::build_opt_with_custom(
                    &selected,
                    &custom_options.borrow(),
                ));
                if !crate::command_options::option_enabled(&selected, "noObscure") {
                    for (key, value) in params
                        .clone()
                        .as_object()
                        .unwrap_or(&serde_json::Map::new())
                    {
                        if let Some(s) = value.as_str() {
                            if looks_secret(key) && !s.is_empty() {
                                if let Ok(obscured) = client.obscure(s) {
                                    params[key] = json!(obscured);
                                }
                            }
                        }
                    }
                }
                if existing.is_none() && clone_from.is_none() {
                    let vendor = params.get("vendor").and_then(|v| v.as_str());
                    let presets =
                        crate::presets::resolve_presets(&r#type, vendor, std::env::consts::OS);
                    crate::presets::merge_remote_params(&mut params, &presets);
                }
                let result = if existing.is_some() || state.borrow().flow.is_active {
                    client.update_remote(&remote_name, params, opt)
                } else {
                    client.create_remote(&remote_name, &r#type, params, opt)
                };
                match result {
                    Ok(_) => {
                        if let Some(source) = clone_from.as_deref() {
                            if let Some(meta) = crate::store::clone_remote_meta(
                                &ctx.store.borrow(),
                                source,
                                &remote_name,
                            ) {
                                ctx.store
                                    .borrow_mut()
                                    .remotes
                                    .insert(remote_name.clone(), meta);
                            }
                        }
                        persist_meta(
                            &ctx,
                            &remote_name,
                            &r#type,
                            &mount.text(),
                            &src.text(),
                            &dst.text(),
                            crate::operations::selected_or(
                                &serve_types,
                                serve.selected(),
                                "webdav",
                            ),
                            crate::operations::selected_or(
                                &mount_types,
                                mount_type.selected(),
                                "mount",
                            ),
                            &cron.text(),
                            tray.is_active(),
                            autostart.is_active(),
                            &cli.text(),
                            &op_flags.borrow(),
                            &runtime_name.text(),
                            &collect_runtime_json(&runtime_view, &runtime_flag_rows.borrow()),
                            &helper_views
                                .borrow()
                                .iter()
                                .map(|(kind, view)| (*kind, textview_text(view)))
                                .collect::<Vec<_>>(),
                        );
                        on_done();
                        dialog.close();
                    }
                    Err(e) => {
                        let err = adw::AlertDialog::new(
                            Some(&ctx.t_or("common.error", "Error")),
                            Some(
                                &ctx.tf("settings.remoteSaveFailed", &[("error", &e.to_string())]),
                            ),
                        );
                        err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                        err.present(Some(&dialog));
                    }
                }
            }
        });
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_hexpand(true);
    content.set_vexpand(true);
    content.append(&title_row);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&nav));
    content.append(&scroll);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_start(12);
    buttons.set_margin_end(12);
    buttons.set_margin_bottom(10);
    buttons.append(&back);
    buttons.append(&continue_btn);
    buttons.append(&next);
    buttons.append(&save);
    content.append(&buttons);
    split.set_content(Some(&content));
    dialog.set_child(Some(&split));
    refresh_nav.borrow()();
    dialog.present(Some(parent));
}

fn provider_type(providers: &[Provider], index: u32) -> String {
    providers
        .get(index as usize)
        .map(|p| p.prefix.clone())
        .unwrap_or_else(|| "drive".into())
}

fn rebuild_fields(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    basic: &adw::PreferencesGroup,
    advanced: &adw::PreferencesGroup,
    state: &Rc<RefCell<WizardState>>,
    provider: Option<&Provider>,
    include_advanced: bool,
    rebuilding: &Rc<Cell<bool>>,
    query: &str,
) {
    if rebuilding.get() {
        return;
    }
    rebuilding.set(true);
    let preserved: HashMap<String, String> = state
        .borrow()
        .fields
        .iter()
        .map(|(key, row)| (key.clone(), row.display_text()))
        .collect();
    for row in state.borrow().fields.values() {
        row.remove_from(basic);
        row.remove_from(advanced);
    }
    state.borrow_mut().fields.clear();
    let Some(provider) = provider else {
        rebuilding.set(false);
        return;
    };
    let vendor = preserved
        .get("provider")
        .filter(|s| !s.is_empty())
        .cloned()
        .or_else(|| preserved.get("vendor").filter(|s| !s.is_empty()).cloned())
        .unwrap_or_default();
    for option in provider.basic_options() {
        if !matches_provider_rule(&option.provider, &vendor) && !option.provider.is_empty() {
            continue;
        }
        let row = option_row(parent, ctx, option, &vendor);
        row.add_to(basic);
        state.borrow_mut().fields.insert(option.name.clone(), row);
    }
    if include_advanced {
        for option in provider.advanced_options() {
            if !matches_provider_rule(&option.provider, &vendor) && !option.provider.is_empty() {
                continue;
            }
            let row = option_row(parent, ctx, option, &vendor);
            row.add_to(advanced);
            state.borrow_mut().fields.insert(option.name.clone(), row);
        }
    }
    for (key, value) in &preserved {
        if let Some(row) = state.borrow().fields.get(key) {
            row.set_display_text(value);
        }
    }
    attach_vendor_watchers(
        parent,
        ctx,
        basic,
        advanced,
        state,
        provider,
        include_advanced,
        rebuilding,
        query,
    );
    apply_field_search(state, query);
    rebuilding.set(false);
}

fn attach_vendor_watchers(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    basic: &adw::PreferencesGroup,
    advanced: &adw::PreferencesGroup,
    state: &Rc<RefCell<WizardState>>,
    provider: &Provider,
    include_advanced: bool,
    rebuilding: &Rc<Cell<bool>>,
    query: &str,
) {
    for key in ["provider", "vendor"] {
        let Some(widget) = state.borrow().fields.get(key).cloned() else {
            continue;
        };
        let parent = parent.clone();
        let ctx = ctx.clone();
        let basic = basic.clone();
        let advanced = advanced.clone();
        let state = state.clone();
        let provider = provider.clone();
        let rebuilding = rebuilding.clone();
        let query = query.to_string();
        widget.connect_change(move || {
            if rebuilding.get() {
                return;
            }
            rebuild_fields(
                &parent,
                &ctx,
                &basic,
                &advanced,
                &state,
                Some(&provider),
                include_advanced,
                &rebuilding,
                &query,
            );
        });
    }
}

fn current_vendor(state: &Rc<RefCell<WizardState>>) -> String {
    for key in ["provider", "vendor"] {
        if let Some(row) = state.borrow().fields.get(key) {
            let text = row.display_text();
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

fn option_row(
    _parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    option: &ProviderOption,
    vendor: &str,
) -> FieldWidget {
    let title = if option.required {
        format!(
            "{} ({})",
            option.name,
            ctx.t_or("wizards.remoteConfig.requiredBadge", "Required")
        )
    } else if option.advanced {
        format!(
            "{} ({})",
            option.name,
            ctx.t_or("wizards.remoteConfig.advancedOptions", "advanced")
        )
    } else {
        option.name.clone()
    };
    let examples = if vendor.is_empty() {
        option.examples.clone()
    } else {
        filter_examples(&option.examples, &option.example_providers, vendor)
    };
    let restrict = ctx.settings.borrow().general.restrict;
    let kind = control_kind(&option.type_name, option.exclusive, examples.len());
    let initial = if !option.default_str.is_empty() {
        option.default_str.clone()
    } else if !option.value.is_null() {
        machine_to_human(&option.value, &option.type_name, "")
    } else {
        examples.first().map(|(v, _)| v.clone()).unwrap_or_default()
    };
    match kind {
        ControlKind::Bool => {
            let row = adw::SwitchRow::new();
            row.set_title(&title);
            row.set_subtitle(&option.help);
            row.set_active(initial.eq_ignore_ascii_case("true"));
            FieldWidget::Switch(row)
        }
        ControlKind::Tristate => {
            let values = Rc::new(vec![
                "unset".to_string(),
                "true".to_string(),
                "false".to_string(),
            ]);
            let row = adw::ComboRow::new();
            row.set_title(&title);
            row.set_subtitle(&option.help);
            row.set_model(Some(&gtk::StringList::new(&["unset", "true", "false"])));
            if let Some(idx) = values.iter().position(|v| v.eq_ignore_ascii_case(&initial)) {
                row.set_selected(idx as u32);
            }
            FieldWidget::Combo(row, values)
        }
        ControlKind::Select => {
            let labels: Vec<String> = examples
                .iter()
                .map(|(v, h)| crate::config_search::example_choice_label(v, h))
                .collect();
            let values = Rc::new(examples.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>());
            let row = adw::ComboRow::new();
            row.set_title(&title);
            row.set_subtitle(&option.help);
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            row.set_model(Some(&gtk::StringList::new(&refs)));
            if let Some(idx) = values.iter().position(|v| v == &initial) {
                row.set_selected(idx as u32);
            }
            if crate::config_search::should_search_examples(&option.name, examples.len()) {
                let search = adw::EntryRow::new();
                search.set_title(&title);
                if !option.help.is_empty() {
                    search.set_tooltip_text(Some(&option.help));
                }
                if let Some(label) = labels.get(row.selected() as usize) {
                    search.set_text(label);
                }
                super::dialogs::attach_example_typeahead(ctx, &search, &row, &examples);
                row.set_visible(false);
                FieldWidget::SearchCombo(search, row, values, Rc::new(labels))
            } else {
                FieldWidget::Combo(row, values)
            }
        }
        ControlKind::MultiSelect => {
            let row = adw::ExpanderRow::new();
            row.set_title(&title);
            row.set_subtitle(&option.help);
            let selected: Vec<String> = initial
                .split([',', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase())
                .collect();
            let mut items = Vec::new();
            for (value, help) in &examples {
                let check = gtk::CheckButton::with_label(value);
                if !help.is_empty() {
                    check.set_tooltip_text(Some(help));
                }
                check.set_active(selected.iter().any(|s| s == &value.to_ascii_lowercase()));
                let wrap = adw::ActionRow::new();
                wrap.set_title(value);
                if !help.is_empty() {
                    wrap.set_subtitle(help);
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
            FieldWidget::Multi(row, Rc::new(items))
        }
        ControlKind::Numeric => {
            let row = adw::SpinRow::with_range(-1_000_000_000.0, 1_000_000_000.0, 1.0);
            row.set_title(&title);
            row.set_subtitle(&option.help);
            if let Ok(v) = initial.parse::<f64>() {
                row.set_value(v);
            }
            row.set_digits(if crate::value_mapper::is_float_type(&option.type_name) {
                3
            } else {
                0
            });
            FieldWidget::Spin(row)
        }
        ControlKind::Input => {
            let row = adw::EntryRow::new();
            row.set_title(&title);
            if !option.help.is_empty() {
                row.set_tooltip_text(Some(&option.help));
            }
            if option.is_password || (restrict && crate::restrict::is_sensitive_key(&option.name)) {
                if let Some(child) = row.first_child() {
                    if let Ok(editable) = child.downcast::<gtk::Text>() {
                        editable.set_visibility(false);
                    }
                }
                if restrict {
                    return FieldWidget::Entry(row);
                }
                let obscure = gtk::Button::from_icon_name("dialog-password-symbolic");
                obscure.set_valign(gtk::Align::Center);
                obscure.set_tooltip_text(Some(&ctx.t_or("wizards.obscure.action", "Obscure")));
                let ctx = ctx.clone();
                let target = row.clone();
                obscure.connect_clicked(move |_| {
                    if let Some(client) = ctx.client() {
                        if let Ok(out) = client.obscure(&target.text()) {
                            target.set_text(&out);
                        }
                    }
                });
                row.add_suffix(&obscure);
            }
            row.set_text(&initial);
            if crate::media::is_path_field(&option.name, &option.help) {
                super::dialogs::attach_path_picker(
                    ctx,
                    &row,
                    crate::picker::FilePickerConfig::folders(),
                );
                let engine_os = ctx.engine_os();
                apply_path_usage(&row, &engine_os, Some(&option.help));
                {
                    let engine_os = engine_os.clone();
                    let help = option.help.clone();
                    row.connect_changed(move |row| {
                        apply_path_usage(row, &engine_os, Some(&help));
                    });
                }
            }
            {
                let option = option.clone();
                let engine_os = ctx.engine_os();
                row.connect_changed(move |row| {
                    match crate::validators::validate_option(&option, &row.text()) {
                        Ok(()) => {
                            row.remove_css_class("error");
                            apply_path_usage(row, &engine_os, Some(&option.help));
                        }
                        Err(msg) => {
                            row.add_css_class("error");
                            row.set_tooltip_text(Some(&msg));
                        }
                    }
                });
            }
            FieldWidget::Entry(row)
        }
    }
}

fn apply_path_usage(row: &adw::EntryRow, engine_os: &str, help: Option<&str>) {
    let text = row.text().to_string();
    let mut parts = Vec::new();
    if let Some(help) = help.filter(|s| !s.is_empty()) {
        parts.push(help.to_string());
    }
    if engine_os.eq_ignore_ascii_case(std::env::consts::OS) {
        if let Some(usage) = crate::media::local_path_usage(&text) {
            parts.push(usage);
        }
        if let Some((free, total)) = crate::fileops::local_path_disk_usage(&text) {
            parts.push(format!(
                "{} / {}",
                crate::rclone::format_bytes(free as i64),
                crate::rclone::format_bytes(total as i64)
            ));
        }
    }
    if parts.is_empty() {
        return;
    }
    if text.is_empty() {
        row.set_tooltip_text(Some(&parts.join(" · ")));
    } else {
        row.set_tooltip_text(Some(&format!("{} — {text}", parts.join(" · "))));
    }
}

fn apply_dump_to_wizard_fields(state: &Rc<RefCell<WizardState>>, params: &Value) {
    let type_by_name: HashMap<String, String> = state
        .borrow()
        .providers
        .iter()
        .flat_map(|p| {
            p.options
                .iter()
                .map(|o| (o.name.clone(), o.type_name.clone()))
        })
        .collect();
    let rows: Vec<(String, FieldWidget)> = state
        .borrow()
        .fields
        .iter()
        .map(|(name, row)| (name.clone(), row.clone()))
        .collect();
    for (name, row) in rows {
        if let Some(raw) = params.get(&name) {
            let type_name = type_by_name
                .get(&name)
                .map(String::as_str)
                .unwrap_or("string");
            row.set_display_text(&machine_to_human(raw, type_name, ""));
        } else if let Some(text) = crate::providers::dump_field_text(params, &name) {
            row.set_display_text(&text);
        }
    }
}

fn apply_existing_meta(
    meta: &RemoteMeta,
    mount: &adw::EntryRow,
    src: &adw::EntryRow,
    dst: &adw::EntryRow,
    serve: &adw::ComboRow,
    serve_types: &[String],
    mount_type: &adw::ComboRow,
    mount_types: &[String],
    cron: &adw::EntryRow,
    tray: &adw::SwitchRow,
    autostart: &adw::SwitchRow,
    op_flags: &Rc<RefCell<Vec<(OperationType, adw::SwitchRow, adw::EntryRow, adw::EntryRow)>>>,
) {
    tray.set_active(meta.show_on_tray);
    if let Some(dest) = profile_src_dst(meta, OperationType::Mount).1 {
        if !dest.is_empty() {
            mount.set_text(&dest);
        }
    }
    for op in [
        OperationType::Sync,
        OperationType::Copy,
        OperationType::Move,
        OperationType::Bisync,
    ] {
        let (psrc, pdst) = profile_src_dst(meta, op);
        if src.text().is_empty() {
            if let Some(value) = psrc {
                src.set_text(&value);
            }
        }
        if dst.text().is_empty() {
            if let Some(value) = pdst {
                dst.set_text(&value);
            }
        }
        if !src.text().is_empty() && !dst.text().is_empty() {
            break;
        }
    }
    if let Some(profile) = meta
        .get_profile(OperationType::Mount, "default")
        .or_else(|| meta.get_profile(OperationType::Sync, "default"))
        .or_else(|| meta.get_profile(OperationType::Serve, "default"))
    {
        if profile.app.auto_start {
            autostart.set_active(true);
        }
        if cron.text().is_empty() && !profile.app.cron_expression.is_empty() {
            cron.set_text(&profile.app.cron_expression);
        }
    }
    if let Some(profile) = meta.get_profile(OperationType::Serve, "default") {
        let rclone = flatten_rclone(&profile.rclone);
        if let Some(t) = rclone.get("type").and_then(|v| v.as_str()) {
            if let Some(idx) = serve_types.iter().position(|s| s == t) {
                serve.set_selected(idx as u32);
            }
        }
    }
    if let Some(profile) = meta.get_profile(OperationType::Mount, "default") {
        let rclone = flatten_rclone(&profile.rclone);
        if let Some(t) = rclone.get("mountType").and_then(|v| v.as_str()) {
            if let Some(idx) = mount_types.iter().position(|s| s == t) {
                mount_type.set_selected(idx as u32);
            }
        }
    }
    for (op, enable, osrc, odst) in op_flags.borrow().iter() {
        if let Some(profile) = meta.get_profile(*op, "default") {
            enable.set_active(true);
            let rclone = flatten_rclone(&profile.rclone);
            if let Some(s) = first_path(&rclone, SOURCE_KEYS) {
                osrc.set_text(&s);
            }
            if let Some(d) = first_path(&rclone, DEST_KEYS) {
                odst.set_text(&d);
            }
        }
    }
}

fn collect_params(state: &Rc<RefCell<WizardState>>) -> Value {
    let options: HashMap<String, ProviderOption> = state
        .borrow()
        .providers
        .iter()
        .flat_map(|p| p.options.iter().cloned().map(|o| (o.name.clone(), o)))
        .collect();
    let mut map = serde_json::Map::new();
    for (key, row) in state.borrow().fields.iter() {
        let text = row.display_text();
        if let Some(option) = options.get(key) {
            if is_default_display(&text, option) {
                continue;
            }
            map.insert(key.clone(), human_to_machine(&text, &option.type_name));
        } else if !text.is_empty() {
            map.insert(key.clone(), json!(text));
        }
    }
    Value::Object(map)
}

fn textview_text(view: &gtk::TextView) -> String {
    let buf = view.buffer();
    buf.text(&buf.start_iter(), &buf.end_iter(), false)
        .to_string()
}

fn pretty_params(params: &Value, restrict: bool) -> String {
    let value = if restrict {
        crate::restrict::redact_value(params)
    } else {
        params.clone()
    };
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}

fn fill_json_view(view: &gtk::TextView, params: &Value, restrict: bool) {
    view.buffer().set_text(&pretty_params(params, restrict));
}

fn apply_json_to_form(
    state: &Rc<RefCell<WizardState>>,
    json_view: &gtk::TextView,
) -> Result<(), String> {
    let map = crate::providers::parse_parameters_json(&textview_text(json_view))?;
    apply_dump_to_wizard_fields(state, &Value::Object(map));
    Ok(())
}

fn collect_wizard_params(
    state: &Rc<RefCell<WizardState>>,
    json_mode: &Rc<Cell<bool>>,
    json_view: &gtk::TextView,
) -> Result<Value, String> {
    if json_mode.get() {
        crate::providers::parse_parameters_json(&textview_text(json_view)).map(Value::Object)
    } else {
        Ok(collect_params(state))
    }
}

fn looks_secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("pass") || key.contains("secret") || key.contains("token") || key.contains("key")
}

fn current_answer(
    flow: &InteractiveFlowState,
    answer_row: &adw::EntryRow,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
) -> InteractiveAnswer {
    let option = flow.question.as_ref().and_then(|q| q.option.as_ref());
    if let Some(option) = option {
        if option.type_name == "bool" {
            return InteractiveAnswer::Bool(answer_switch.is_active());
        }
        if !option.examples.is_empty() && option.exclusive {
            if let Some((value, _)) = option.examples.get(example_row.selected() as usize) {
                return InteractiveAnswer::Text(value.clone());
            }
        }
    }
    let text = answer_row.text().to_string();
    if text.is_empty() {
        update_interactive_answer(flow.clone(), InteractiveAnswer::Empty).answer
    } else {
        InteractiveAnswer::Text(text)
    }
}

fn apply_question_widgets(
    ctx: &AppCtx,
    flow: &InteractiveFlowState,
    title: &gtk::Label,
    help: &gtk::Label,
    error: &gtk::Label,
    answer_row: &adw::EntryRow,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
) {
    let Some(step) = &flow.question else {
        return;
    };
    if let Some(option) = &step.option {
        title.set_text(&option.name);
        help.set_text(&option.help);
        error.set_text(step.error.as_deref().unwrap_or(""));
        let is_bool = option.type_name == "bool";
        answer_switch.set_visible(is_bool);
        let show_custom = !is_bool && (option.examples.is_empty() || allows_custom_value(option));
        answer_row.set_visible(show_custom);
        if is_bool {
            answer_switch.set_active(matches!(flow.answer, InteractiveAnswer::Bool(true)));
        } else if show_custom {
            answer_row.set_text(&flow.answer.as_string());
        }
        if option.examples.is_empty() {
            example_row.set_visible(false);
        } else {
            example_row.set_visible(true);
            let labels: Vec<String> = option
                .examples
                .iter()
                .map(|(v, h)| example_label(v, h))
                .collect();
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            example_row.set_model(Some(&gtk::StringList::new(&refs)));
            if let Some(idx) = selected_example_index(option, &flow.answer.as_string()) {
                example_row.set_selected(idx as u32);
            }
        }
    } else {
        title.set_text(&ctx.t_or(
            "wizards.remoteConfig.authenticationMethod",
            "Continue authorization",
        ));
        help.set_text(&step.error.clone().unwrap_or_else(|| {
            ctx.t_or(
                "modals.oauth.openLink",
                "Complete the next step in the browser.",
            )
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_meta(
    ctx: &AppCtx,
    remote_name: &str,
    remote_type: &str,
    mount: &str,
    src: &str,
    dst: &str,
    serve_type: &str,
    mount_type: &str,
    cron: &str,
    tray: bool,
    autostart: bool,
    cli: &str,
    op_flags: &[(OperationType, adw::SwitchRow, adw::EntryRow, adw::EntryRow)],
    runtime_name: &str,
    runtime_json: &str,
    helpers: &[(&str, String)],
) {
    let mut meta = ctx
        .store
        .borrow()
        .remotes
        .get(remote_name)
        .cloned()
        .unwrap_or_default();
    meta.show_on_tray = tray;
    let extra = parse_cli_flags(cli);
    for (op, enable, osrc, odst) in op_flags {
        if !enable.is_active() {
            continue;
        }
        let mut source = osrc.text().to_string();
        let mut dest = odst.text().to_string();
        if source.is_empty() {
            source = src.to_string();
        }
        if dest.is_empty() {
            dest = if *op == OperationType::Mount {
                mount.to_string()
            } else {
                dst.to_string()
            };
        }
        if source.is_empty() {
            source = remote_fs(remote_name, "");
        }
        let mut rclone = json!({
            "srcFs": source,
            "dstFs": dest,
            "mountPoint": if *op == OperationType::Mount { dest.clone() } else { mount.to_string() },
            "fs": remote_fs(remote_name, ""),
            "path1": source,
            "path2": dest,
            "type": serve_type,
            "mountType": mount_type
        });
        if let Some(obj) = rclone.as_object_mut() {
            obj.extend(extra.clone());
        }
        let helper = if runtime_name.trim().is_empty() {
            "default".to_string()
        } else {
            runtime_name.trim().to_string()
        };
        let profile = ProfileConfig {
            name: "default".into(),
            app: AppConfig {
                auto_start: autostart
                    && matches!(
                        *op,
                        OperationType::Mount | OperationType::Sync | OperationType::Serve
                    ),
                cron_enabled: !cron.is_empty(),
                cron_expression: cron.to_string(),
                runtime_remote_profile: helper,
                ..AppConfig::default()
            },
            rclone,
        };
        meta.profiles
            .entry(op.as_str().into())
            .or_default()
            .insert("default".into(), profile);
    }
    let helper = if runtime_name.trim().is_empty() {
        "default"
    } else {
        runtime_name.trim()
    };
    persist_helper_json(&mut meta, "runtime", helper, runtime_json);
    for (kind, json) in helpers {
        persist_helper_json(&mut meta, kind, helper, json);
    }
    crate::presets::apply_to_remote_meta(
        &mut meta,
        &crate::presets::resolve_presets(remote_type, None, std::env::consts::OS),
    );
    ctx.store
        .borrow_mut()
        .remotes
        .insert(remote_name.to_string(), meta);
    ctx.persist();
}

fn persist_helper_json(meta: &mut RemoteMeta, kind: &str, name: &str, json: &str) {
    if let Ok(value) = serde_json::from_str::<Value>(json.trim()) {
        if value.as_object().is_some_and(|obj| !obj.is_empty()) {
            meta.upsert_helper(kind, name, value);
        }
    } else if !json.trim().is_empty() {
        meta.upsert_helper(kind, name, json!({ "raw": json.trim() }));
    }
}

fn helper_json_page(
    ctx: &AppCtx,
    kind: &str,
    fallback: &str,
    existing: Option<&RemoteMeta>,
    restrict: bool,
) -> (adw::PreferencesPage, gtk::TextView) {
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_monospace(true);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    let value = existing
        .and_then(|meta| {
            let names = meta.helper_names(kind);
            let chosen = names
                .iter()
                .find(|n| n.eq_ignore_ascii_case("default"))
                .cloned()
                .or_else(|| names.into_iter().next())
                .unwrap_or_else(|| "default".into());
            meta.helper_profile(kind, &chosen)
        })
        .unwrap_or_else(|| json!({}));
    fill_json_view(&view, &value, restrict);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or(&format!("modals.remoteConfig.steps.{kind}"), fallback));
    group.set_description(Some(&ctx.t_or(
        "wizards.remoteConfig.runtimeRemoteWarning.description",
        "These options are stored on the remote profile and are not written into rclone.conf.",
    )));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(280);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);
    scroll.set_child(Some(&view));
    group.add(&scroll);
    page.add(&group);
    (page, view)
}

fn parse_runtime_json(text: &str) -> Value {
    serde_json::from_str(text.trim()).unwrap_or_else(|_| json!({}))
}

fn fill_runtime_flag_rows(
    ctx: &AppCtx,
    group: &adw::PreferencesGroup,
    rows: &Rc<RefCell<Vec<(String, adw::EntryRow, String)>>>,
    remote_type: &str,
    current: &Value,
) {
    for (_, row, _) in rows.borrow().iter() {
        group.remove(row);
    }
    rows.borrow_mut().clear();
    for flag in super::remote_config::runtime_flags_for_type(ctx, remote_type) {
        let row = adw::EntryRow::new();
        row.set_title(&ctx.option_label(&flag.name, "title", &flag.name));
        let help = ctx.option_label(&flag.name, "help", &flag.help);
        if !help.is_empty() {
            row.set_tooltip_text(Some(&help));
        }
        let text = current
            .get(&flag.field_name)
            .or_else(|| current.get(&flag.name))
            .map(|value| match value {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Null => String::new(),
                other => other.to_string().trim_matches('"').to_string(),
            })
            .unwrap_or_else(|| flag.default_str.clone());
        row.set_text(&text);
        group.add(&row);
        rows.borrow_mut()
            .push((flag.field_name, row, flag.type_name));
    }
}

fn collect_runtime_json(view: &gtk::TextView, rows: &[(String, adw::EntryRow, String)]) -> String {
    let mut value = parse_runtime_json(&textview_text(view));
    if let Some(obj) = value.as_object_mut() {
        for (field, row, type_name) in rows {
            let text = row.text().to_string();
            if text.is_empty() {
                continue;
            }
            obj.insert(field.clone(), parse_flag_value(type_name, &text));
        }
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}

fn apply_field_search(state: &Rc<RefCell<WizardState>>, query: &str) {
    for (name, widget) in state.borrow().fields.iter() {
        let help = widget.search_help();
        widget.set_visible(crate::config_search::matches_config_search(
            name, &help, "", query,
        ));
    }
}

fn attach_provider_typeahead(
    ctx: &AppCtx,
    search: &adw::EntryRow,
    type_row: &adw::ComboRow,
    providers: &[Provider],
    labels: &[String],
) {
    let names: Vec<String> = if providers.is_empty() {
        labels
            .iter()
            .map(|label| label.split(" — ").next().unwrap_or(label).to_string())
            .collect()
    } else {
        providers.iter().map(|p| p.name.clone()).collect()
    };
    let descriptions: Vec<String> = if providers.is_empty() {
        vec![String::new(); names.len()]
    } else {
        providers.iter().map(|p| p.description.clone()).collect()
    };
    let labels = labels.to_vec();
    let popover = gtk::Popover::new();
    popover.set_parent(search);
    popover.set_autohide(true);
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_min_content_height(80);
    scroll.set_max_content_height(280);
    scroll.set_propagate_natural_width(true);
    scroll.set_child(Some(&list));
    popover.set_child(Some(&scroll));
    {
        let popover = popover.clone();
        search.connect_destroy(move |_| popover.unparent());
    }
    let applying = Rc::new(Cell::new(false));
    let generation = Rc::new(Cell::new(0u32));
    let fill = {
        let search = search.clone();
        let type_row = type_row.clone();
        let list = list.clone();
        let popover = popover.clone();
        let applying = applying.clone();
        let names = names.clone();
        let descriptions = descriptions.clone();
        let labels = labels.clone();
        let empty_label = ctx.t_or(
            "wizards.remoteConfig.noMatchingTypes",
            "No matching remote types",
        );
        Rc::new(move || {
            if !super::dialogs::row_is_editing(&search) {
                popover.popdown();
                return;
            }
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let hits =
                crate::config_search::filter_providers(&search.text(), &names, &descriptions);
            if hits.is_empty() {
                let empty = adw::ActionRow::new();
                empty.set_activatable(false);
                empty.set_title(&empty_label);
                list.append(&empty);
            }
            for hit in hits.into_iter().take(40) {
                let item = adw::ActionRow::new();
                let title = labels.get(hit.index).cloned().unwrap_or(hit.name.clone());
                item.set_title(&title);
                if !hit.description.is_empty() {
                    item.set_subtitle(&hit.description);
                }
                item.set_activatable(true);
                let search = search.clone();
                let type_row = type_row.clone();
                let applying = applying.clone();
                let popover = popover.clone();
                let idx = hit.index as u32;
                item.connect_activated(move |_| {
                    applying.set(true);
                    search.set_text(&title);
                    type_row.set_selected(idx);
                    applying.set(false);
                    popover.popdown();
                });
                list.append(&item);
            }
            popover.popup();
        })
    };
    {
        let fill = fill.clone();
        search.connect_notify_local(Some("has-focus"), move |row, _| {
            if row.has_focus() {
                fill();
            }
        });
    }
    {
        let fill = fill.clone();
        let applying = applying.clone();
        let generation = generation.clone();
        search.connect_changed(move |_| {
            if applying.get() {
                return;
            }
            let next = generation.get().wrapping_add(1);
            generation.set(next);
            let fill = fill.clone();
            let generation = generation.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(160), move || {
                if generation.get() == next {
                    fill();
                }
                glib::ControlFlow::Break
            });
        });
    }
}
