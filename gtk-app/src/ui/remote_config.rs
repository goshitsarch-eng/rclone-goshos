//! Angular-style remote configuration sidenav: per-operation profiles,
//! helper configs (VFS / filter / backend / runtime), and remote metadata.

use super::dialogs;
use super::AppCtx;
use crate::flags::{
    flag_category_for_op, options_for_category, parse_flag_value, parse_options_info,
    static_flags_for, FlagBlock, FlagOption,
};
use crate::jobs::{
    assemble_rclone, default_dest, default_source, extra_flags, flatten_rclone, path_list,
    SOURCE_KEYS,
};
use crate::operations::OperationType;
use crate::rclone::validate_cron;
use crate::store::{AppConfig, ProfileConfig, RemoteMeta};
use adw::prelude::*;
use gtk::glib;
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorStep {
    Remote,
    Op(OperationType),
    Helper(&'static str),
}

#[derive(Clone, Default)]
pub struct RemoteConfigOpen {
    pub initial: Option<String>,
    pub profile: Option<String>,
    pub auto_add: bool,
}

const HELPERS: &[(&str, &str, &str)] = &[
    ("vfs", "VFS", "drive-harddisk-symbolic"),
    ("filter", "Filter", "view-filter-symbolic"),
    ("backend", "Backend", "preferences-system-symbolic"),
    ("runtime", "Runtime", "emblem-system-symbolic"),
];

pub fn present(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: String, on_done: Rc<dyn Fn()>) {
    present_with(parent, ctx, remote, RemoteConfigOpen::default(), on_done);
}

pub fn present_with(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: String,
    open: RemoteConfigOpen,
    on_done: Rc<dyn Fn()>,
) {
    ctx.store
        .borrow_mut()
        .remotes
        .entry(remote.clone())
        .or_default();

    let dialog = adw::Dialog::new();
    dialog.set_title(&format!(
        "{} {remote}",
        ctx.t_or("modals.remoteConfig.steps.remoteConfig", "Configure")
    ));
    dialog.set_content_width(980);
    dialog.set_content_height(780);

    let split = adw::OverlaySplitView::new();
    split.set_min_sidebar_width(230.0);
    split.set_show_sidebar(true);

    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_selection_mode(gtk::SelectionMode::Single);
    let side_scroll = gtk::ScrolledWindow::new();
    side_scroll.set_vexpand(true);
    side_scroll.set_child(Some(&sidebar));
    let side_col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    side_col.append(&side_scroll);
    split.set_sidebar(Some(&side_col));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let title = gtk::Label::new(Some(
        &ctx.t_or("modals.remoteConfig.steps.remote", "Remote"),
    ));
    title.add_css_class("title-3");
    title.set_xalign(0.0);
    title.set_margin_start(12);
    title.set_margin_top(10);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_hexpand(true);
    body.set_vexpand(true);
    let body_scroll = gtk::ScrolledWindow::new();
    body_scroll.set_vexpand(true);
    body_scroll.set_child(Some(&body));
    let save_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    save_bar.set_margin_start(12);
    save_bar.set_margin_end(12);
    save_bar.set_margin_bottom(10);
    let save = gtk::Button::with_label(&ctx.t_or("modals.remoteConfig.saveStep", "Save step"));
    save.add_css_class("suggested-action");
    let close = gtk::Button::with_label(&ctx.t_or("common.close", "Close"));
    save_bar.append(&save);
    save_bar.append(&close);
    content.append(&title);
    content.append(&body_scroll);
    content.append(&save_bar);
    split.set_content(Some(&content));
    dialog.set_child(Some(&split));

    let flag_blocks: Rc<Vec<FlagBlock>> = Rc::new(
        ctx.client()
            .and_then(|c| c.options_info().ok())
            .map(|v| parse_options_info(&v))
            .unwrap_or_default(),
    );

    let current = Rc::new(RefCell::new(EditorStep::Remote));
    let persist_step: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let preferred_profile = Rc::new(RefCell::new(open.profile.clone()));
    let auto_add = Rc::new(Cell::new(open.auto_add));

    let steps = editor_steps();
    for step in &steps {
        let row = adw::ActionRow::new();
        row.set_title(&step_label(&ctx, *step));
        row.set_activatable(true);
        let icon = gtk::Image::from_icon_name(step_icon(*step));
        row.add_prefix(&icon);
        sidebar.append(&row);
    }
    let initial_step = parse_open_step(&open);
    *current.borrow_mut() = initial_step;
    if let Some(idx) = editor_steps().iter().position(|step| *step == initial_step) {
        if let Some(row) = sidebar.row_at_index(idx as i32) {
            sidebar.select_row(Some(&row));
        }
    } else if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }

    let rebuild = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let body = body.clone();
        let title = title.clone();
        let persist_step = persist_step.clone();
        let current = current.clone();
        let parent = parent.clone();
        let flag_blocks = flag_blocks.clone();
        let on_done = on_done.clone();
        let preferred_profile = preferred_profile.clone();
        let auto_add = auto_add.clone();
        Rc::new(move || {
            persist_step.borrow()();
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }
            let step = *current.borrow();
            title.set_text(&step_label(&ctx, step));
            match step {
                EditorStep::Remote => {
                    let (page, saver) = remote_page(&parent, ctx.clone(), &remote, on_done.clone());
                    body.append(&page);
                    *persist_step.borrow_mut() = Box::new(saver);
                }
                EditorStep::Op(op) => {
                    let add_now = auto_add.replace(false);
                    let profile = preferred_profile.borrow().clone();
                    let (page, saver) = operation_page(
                        &parent,
                        ctx.clone(),
                        &remote,
                        op,
                        flag_blocks.as_ref(),
                        profile.as_deref(),
                        add_now,
                    );
                    body.append(&page);
                    *persist_step.borrow_mut() = Box::new(saver);
                }
                EditorStep::Helper(kind) => {
                    let (page, saver) =
                        helper_page(&parent, ctx.clone(), &remote, kind, flag_blocks.as_ref());
                    body.append(&page);
                    *persist_step.borrow_mut() = Box::new(saver);
                }
            }
        }) as Rc<dyn Fn()>
    };
    side_col.append(&preset_bar(
        parent,
        ctx.clone(),
        remote.clone(),
        rebuild.clone(),
    ));
    rebuild();

    {
        let current = current.clone();
        let rebuild = rebuild.clone();
        sidebar.connect_row_selected(move |_, row| {
            let Some(row) = row else {
                return;
            };
            let idx = row.index();
            if idx < 0 {
                return;
            }
            if let Some(step) = editor_steps().get(idx as usize).copied() {
                *current.borrow_mut() = step;
                rebuild();
            }
        });
    }
    {
        let persist_step = persist_step.clone();
        save.connect_clicked(move |_| persist_step.borrow()());
    }
    {
        let dialog = dialog.clone();
        let persist_step = persist_step.clone();
        let on_done = on_done.clone();
        close.connect_clicked(move |_| {
            persist_step.borrow()();
            on_done();
            dialog.close();
        });
    }

    dialogs::present_window_or_dialog(parent, &ctx, &dialog);
}

fn editor_steps() -> Vec<EditorStep> {
    let mut steps = vec![EditorStep::Remote];
    steps.extend(OperationType::ALL.iter().copied().map(EditorStep::Op));
    steps.extend(HELPERS.iter().map(|(kind, _, _)| EditorStep::Helper(*kind)));
    steps
}

fn parse_open_step(open: &RemoteConfigOpen) -> EditorStep {
    let Some(raw) = open.initial.as_deref() else {
        return EditorStep::Remote;
    };
    let lower = raw.to_ascii_lowercase();
    if lower.is_empty() || lower == "remote" {
        return EditorStep::Remote;
    }
    if let Some(op) = OperationType::parse(&lower) {
        return EditorStep::Op(op);
    }
    if let Some((kind, _, _)) = HELPERS.iter().find(|(kind, _, _)| *kind == lower) {
        return EditorStep::Helper(*kind);
    }
    EditorStep::Remote
}

fn step_label(ctx: &AppCtx, step: EditorStep) -> String {
    match step {
        EditorStep::Remote => ctx.t_or("modals.remoteConfig.steps.remote", "Remote"),
        EditorStep::Op(op) => {
            ctx.t_or(&format!("operations.{}.label", op.as_str()), op.api_label())
        }
        EditorStep::Helper(kind) => ctx.t_or(
            &format!("modals.remoteConfig.helpers.{kind}"),
            HELPERS
                .iter()
                .find(|(k, _, _)| *k == kind)
                .map(|(_, label, _)| *label)
                .unwrap_or(kind),
        ),
    }
}

fn remote_type_of(ctx: &AppCtx, remote: &str) -> String {
    ctx.snapshot
        .borrow()
        .remotes
        .iter()
        .find(|info| info.name == remote)
        .map(|info| info.r#type.clone())
        .unwrap_or_default()
}

fn preset_bar(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: String,
    rebuild: Rc<dyn Fn()>,
) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
    box_.set_margin_start(8);
    box_.set_margin_end(8);
    box_.set_margin_bottom(10);
    let title = gtk::Label::new(Some(
        &ctx.t_or("templates.presetTitle", "Presets & Templates"),
    ));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    let defaults =
        gtk::Button::with_label(&ctx.t_or("templates.applyPresets", "Apply Default Presets"));
    {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let rebuild = rebuild.clone();
        let parent = parent.clone();
        defaults.connect_clicked(move |_| {
            let r#type = remote_type_of(&ctx, &remote);
            let presets = crate::presets::resolve_presets(&r#type, None, std::env::consts::OS);
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                crate::presets::apply_to_remote_meta(meta, &presets);
            }
            ctx.persist();
            rebuild();
            let toast = adw::AlertDialog::new(
                Some(&ctx.t_or("templates.applySuccess", "Presets applied")),
                Some("Default provider / OS presets were merged into this remote."),
            );
            toast.add_response("ok", "OK");
            toast.present(Some(&parent));
        });
    }
    let names: Vec<String> = ctx
        .store
        .borrow()
        .templates
        .iter()
        .map(|t| t.name.clone())
        .collect();
    let pick = adw::ComboRow::new();
    pick.set_title(&ctx.t_or("templates.userTemplates", "Saved User Templates"));
    let labels: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    if labels.is_empty() {
        pick.set_model(Some(&gtk::StringList::new(&["—"])));
        pick.set_sensitive(false);
    } else {
        pick.set_model(Some(&gtk::StringList::new(&labels)));
    }
    let apply = gtk::Button::with_label(&ctx.t_or("common.apply", "Apply"));
    apply.add_css_class("suggested-action");
    apply.set_sensitive(!names.is_empty());
    {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let rebuild = rebuild.clone();
        let parent = parent.clone();
        let pick = pick.clone();
        apply.connect_clicked(move |_| {
            let templates = ctx.store.borrow().templates.clone();
            let Some(template) = templates.get(pick.selected() as usize) else {
                return;
            };
            let applied = if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                crate::user_templates::apply_to_meta(meta, &template.values, None, true)
            } else {
                0
            };
            if applied == 0 && !crate::user_templates::is_categorized(&template.values) {
                if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                    for profiles in meta.profiles.values_mut() {
                        for profile in profiles.values_mut() {
                            crate::jobs::merge_template_into(&mut profile.rclone, &template.values);
                        }
                    }
                }
            }
            ctx.persist();
            rebuild();
            let msg = ctx.tf("templates.applySuccess", &[("name", &template.name)]);
            let toast = adw::AlertDialog::new(Some(&msg), None::<&str>);
            toast.add_response("ok", "OK");
            toast.present(Some(&parent));
        });
    }
    let save = gtk::Button::with_label(&ctx.t_or(
        "templates.saveAsTemplate",
        "Save Current Settings as Template...",
    ));
    {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let parent = parent.clone();
        save.connect_clicked(move |_| {
            capture_remote_template(&parent, ctx.clone(), &remote);
        });
    }
    let manage =
        gtk::Button::with_label(&ctx.t_or("templates.manageTemplates", "Manage Templates..."));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        manage.connect_clicked(move |_| {
            dialogs::templates(&parent, ctx.clone());
        });
    }
    box_.append(&title);
    box_.append(&defaults);
    box_.append(&pick);
    box_.append(&apply);
    box_.append(&save);
    box_.append(&manage);
    box_
}

fn capture_remote_template(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("templates.newTitle", "New Template"));
    dialog.set_content_width(420);
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("templates.templateName", "Template Name"));
    name.set_text(&format!("{remote} preset"));
    let desc = adw::EntryRow::new();
    desc.set_title(&ctx.t_or("templates.templateDesc", "Description (optional)"));
    let save = gtk::Button::with_label(&ctx.t_or("common.save", "Save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let dialog = dialog.clone();
        let name = name.clone();
        let desc = desc.clone();
        save.connect_clicked(move |_| {
            let values = ctx
                .store
                .borrow()
                .remotes
                .get(&remote)
                .map(|meta| crate::user_templates::capture_from_meta(meta, &[]))
                .unwrap_or_else(|| serde_json::json!({}));
            let now = chrono::Utc::now().to_rfc3339();
            ctx.store
                .borrow_mut()
                .templates
                .push(crate::store::UserTemplate {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.text().to_string(),
                    description: desc.text().to_string(),
                    icon: "emblem-ok-symbolic".into(),
                    created_at: now.clone(),
                    updated_at: now,
                    values,
                });
            ctx.persist();
            dialog.close();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&desc);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn step_icon(step: EditorStep) -> &'static str {
    match step {
        EditorStep::Remote => "network-server-symbolic",
        EditorStep::Op(op) => op.icon_name(),
        EditorStep::Helper(kind) => HELPERS
            .iter()
            .find(|(k, _, _)| *k == kind)
            .map(|(_, _, icon)| *icon)
            .unwrap_or("preferences-other-symbolic"),
    }
}

fn remote_page(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    on_done: Rc<dyn Fn()>,
) -> (gtk::Box, impl Fn() + 'static) {
    let meta = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .cloned()
        .unwrap_or_default();
    let tray = adw::SwitchRow::new();
    tray.set_title(&ctx.t_or("remoteConfig.showOnTray", "Show on tray"));
    tray.set_active(meta.show_on_tray);
    let hidden = adw::SwitchRow::new();
    hidden.set_title(&ctx.t_or("remoteConfig.hideFromSidebar", "Hide from sidebar"));
    hidden.set_active(meta.hidden);
    let primary_ids = Rc::new(RefCell::new(meta.primary_actions.clone()));
    let sync_ids = Rc::new(RefCell::new(meta.sync_actions.clone()));
    let primary_row = adw::ActionRow::new();
    primary_row.set_title(&ctx.t_or("remoteConfig.primaryActions", "Primary actions"));
    primary_row.set_subtitle(&action_summary(&primary_ids.borrow()));
    let edit_primary = gtk::Button::with_label(&ctx.t_or("common.edit", "Edit"));
    edit_primary.set_valign(gtk::Align::Center);
    {
        let parent = parent.clone();
        let primary_ids = primary_ids.clone();
        let primary_row = primary_row.clone();
        edit_primary.connect_clicked(move |_| {
            let catalog = crate::action_order::catalog_ids();
            let current = primary_ids.borrow().clone();
            dialogs::action_order(&parent, "Primary actions", &catalog, &current, {
                let primary_ids = primary_ids.clone();
                let primary_row = primary_row.clone();
                move |ids| {
                    primary_row.set_subtitle(&action_summary(&ids));
                    *primary_ids.borrow_mut() = ids;
                }
            });
        });
    }
    primary_row.add_suffix(&edit_primary);
    let sync_row = adw::ActionRow::new();
    sync_row.set_title(&ctx.t_or("remoteConfig.syncActions", "Sync actions"));
    sync_row.set_subtitle(&action_summary(&sync_ids.borrow()));
    let edit_sync = gtk::Button::with_label(&ctx.t_or("common.edit", "Edit"));
    edit_sync.set_valign(gtk::Align::Center);
    {
        let parent = parent.clone();
        let sync_ids = sync_ids.clone();
        let sync_row = sync_row.clone();
        edit_sync.connect_clicked(move |_| {
            let catalog: Vec<&str> = OperationType::PRIMARY_SYNC
                .iter()
                .chain(OperationType::MORE_SYNC.iter())
                .map(|op| op.as_str())
                .collect();
            let current = sync_ids.borrow().clone();
            dialogs::action_order(&parent, "Sync actions", &catalog, &current, {
                let sync_ids = sync_ids.clone();
                let sync_row = sync_row.clone();
                move |ids| {
                    sync_row.set_subtitle(&action_summary(&ids));
                    *sync_ids.borrow_mut() = ids;
                }
            });
        });
    }
    sync_row.add_suffix(&edit_sync);

    let provider =
        gtk::Button::with_label(&ctx.t_or("remoteConfig.editProvider", "Edit provider fields…"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let remote = remote.to_string();
        let on_done = on_done.clone();
        provider.connect_clicked(move |_| {
            super::wizard::present(&parent, ctx.clone(), Some(remote.clone()), on_done.clone());
        });
    }
    let helpers =
        gtk::Button::with_label(&ctx.t_or("remoteConfig.helperJsonEditor", "Helper JSON editor…"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let remote = remote.to_string();
        helpers.connect_clicked(move |_| {
            dialogs::helper_profiles(&parent, ctx.clone(), &remote);
        });
    }

    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("remoteConfig.metadata", "Remote metadata"));
    group.add(&tray);
    group.add(&hidden);
    group.add(&primary_row);
    group.add(&sync_row);
    let actions = adw::PreferencesGroup::new();
    actions.set_title(&ctx.t_or("remoteConfig.provider", "Provider"));
    let provider_row = adw::ActionRow::new();
    provider_row.set_title(&ctx.t_or("remoteConfig.rcloneDefinition", "Rclone remote definition"));
    provider_row.add_suffix(&provider);
    let helper_row = adw::ActionRow::new();
    helper_row.set_title(&ctx.t_or("remoteConfig.namedHelpers", "Named helper profiles"));
    helper_row.add_suffix(&helpers);
    actions.add(&provider_row);
    actions.add(&helper_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    page.add(&actions);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    box_.append(&page);

    let saver = {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let tray = tray.clone();
        let hidden = hidden.clone();
        let primary_ids = primary_ids.clone();
        let sync_ids = sync_ids.clone();
        move || {
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                meta.show_on_tray = tray.is_active();
                meta.hidden = hidden.is_active();
                meta.primary_actions = primary_ids.borrow().clone();
                meta.sync_actions = sync_ids.borrow().clone();
            }
            ctx.persist();
        }
    };
    (box_, saver)
}

fn operation_page(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    op: OperationType,
    blocks: &[FlagBlock],
    preferred_profile: Option<&str>,
    auto_add: bool,
) -> (gtk::Box, impl Fn() + 'static) {
    let names = {
        let store = ctx.store.borrow();
        let meta = store.remotes.get(remote);
        let mut names = meta.map(|m| m.profile_names(op)).unwrap_or_default();
        if names.is_empty() {
            names.push("default".into());
        }
        names
    };
    let selected_name = preferred_profile
        .filter(|name| names.iter().any(|n| n == *name))
        .unwrap_or(&names[0])
        .to_string();
    let selected = Rc::new(RefCell::new(selected_name.clone()));
    let initial = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .and_then(|m| m.get_profile(op, &selected_name))
        .unwrap_or_else(|| ProfileConfig {
            name: selected_name.clone(),
            ..Default::default()
        });

    let switcher = profile_switcher(&ctx, &names, &initial.name);
    let rclone = flatten_rclone(&initial.rclone);
    let src = adw::EntryRow::new();
    let src_title = if op == OperationType::Copyurl {
        ctx.t_or("remoteConfig.url", "URL")
    } else {
        ctx.t_or("remoteConfig.source", "Source")
    };
    src.set_title(&src_title);
    src.set_text(&default_source(remote, &rclone));
    if op != OperationType::Copyurl {
        dialogs::attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
    }
    let extra_sources: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    let more = path_list(&rclone, SOURCE_KEYS);
    if more.len() > 1 {
        for extra in more.iter().skip(1) {
            let row = extra_source_row(&ctx, extra);
            extra_sources.borrow_mut().push(row);
        }
    }
    let dst = adw::EntryRow::new();
    let dst_title = match op {
        OperationType::Mount => ctx.t_or("remoteConfig.mountPoint", "Mount point"),
        OperationType::Serve => ctx.t_or("remoteConfig.listenAddress", "Listen address"),
        OperationType::Copyurl => ctx.t_or("remoteConfig.destinationFs", "Destination fs"),
        OperationType::Delete => ctx.t_or("remoteConfig.unusedDestination", "Unused destination"),
        _ => ctx.t_or("remoteConfig.dest", "Destination"),
    };
    dst.set_title(&dst_title);
    dst.set_text(&default_dest(remote, &rclone, op));
    dst.set_visible(op != OperationType::Delete);
    if op == OperationType::Mount {
        dialogs::attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::local_folders());
    } else if op != OperationType::Serve && op != OperationType::Delete {
        dialogs::attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
    }
    let dest_status = adw::ActionRow::new();
    dest_status.set_title(&ctx.t_or("remoteConfig.pathStatusTitle", "Path status"));
    dest_status.set_visible(matches!(
        op,
        OperationType::Mount | OperationType::Sync | OperationType::Copy | OperationType::Bisync
    ));
    {
        let dest_status = dest_status.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let refresh_status = move |path: &str| {
            let resolved = crate::path_kind::resolve_job_path(path, &remote);
            let status = crate::path_inspection::inspect_dest(
                &ctx.store.borrow(),
                &resolved,
                &remote,
                op,
                &ctx.snapshot.borrow().mounts,
            );
            dest_status.set_subtitle(&crate::path_inspection::describe_status(&status));
        };
        refresh_status(&dst.text());
        dst.connect_changed(move |row| refresh_status(&row.text()));
    }

    let serve_types = Rc::new(ctx.serve_types());
    let mount_types = Rc::new(ctx.mount_types());
    let serve = adw::ComboRow::new();
    serve.set_title(&ctx.t_or("remoteConfig.serveType", "Serve type"));
    serve.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&serve_types),
    )));
    serve.set_visible(op == OperationType::Serve);
    if let Some(t) = rclone
        .get("type")
        .and_then(|x| x.as_str())
        .or_else(|| rclone.get("typeName").and_then(|x| x.as_str()))
    {
        if let Some(idx) = serve_types.iter().position(|s| s == t) {
            serve.set_selected(idx as u32);
        }
    }
    let mount_type = adw::ComboRow::new();
    mount_type.set_title(&ctx.t_or("remoteConfig.mountType", "Mount type"));
    mount_type.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&mount_types),
    )));
    mount_type.set_visible(op == OperationType::Mount);
    if let Some(t) = rclone.get("mountType").and_then(|x| x.as_str()) {
        if let Some(idx) = mount_types.iter().position(|s| s == t) {
            mount_type.set_selected(idx as u32);
        }
    }

    let auto_start = adw::SwitchRow::new();
    auto_start.set_title(&ctx.t_or(
        "wizards.appOperation.enableAutoStart",
        "Start with application",
    ));
    auto_start.set_active(initial.app.auto_start);
    let cron_enabled = adw::SwitchRow::new();
    cron_enabled.set_title(&ctx.t_or("remoteConfig.scheduledCron", "Scheduled (cron)"));
    cron_enabled.set_active(initial.app.cron_enabled);
    cron_enabled.set_visible(op.is_automatable());
    let cron = adw::EntryRow::new();
    cron.set_title(&ctx.t_or("remoteConfig.cronExpression", "Cron expression"));
    cron.set_text(&initial.app.cron_expression);
    cron.set_visible(op.is_automatable());
    let cron_hint = gtk::Label::new(None);
    cron_hint.add_css_class("dim-label");
    cron_hint.set_xalign(0.0);
    cron_hint.set_wrap(true);
    cron_hint.set_visible(op.is_automatable());
    update_cron_hint(&ctx, &cron, &cron_hint);
    {
        let cron_hint = cron_hint.clone();
        let ctx = ctx.clone();
        cron.connect_changed(move |row| update_cron_hint(&ctx, row, &cron_hint));
    }
    let watch_enabled = adw::SwitchRow::new();
    watch_enabled.set_title(&ctx.t_or("wizards.appOperation.enableWatch", "Watch local sources"));
    watch_enabled.set_active(initial.app.watch_enabled);
    watch_enabled.set_visible(op.is_automatable());
    let watch_delay = adw::EntryRow::new();
    watch_delay.set_title(&ctx.t_or("wizards.appOperation.watchDelay", "Watch delay (seconds)"));
    watch_delay.set_text(&initial.app.watch_delay.to_string());
    watch_delay.set_visible(op.is_automatable());
    let watch_changed = adw::SwitchRow::new();
    watch_changed.set_title(&ctx.t_or(
        "wizards.appOperation.watchChangedOnly",
        "Changed files only",
    ));
    watch_changed.set_active(initial.app.watch_changed_only);
    watch_changed.set_visible(op.is_automatable());

    let helper_names = |kind: &str| -> Vec<String> {
        let mut names = vec!["—".into()];
        if let Some(meta) = ctx.store.borrow().remotes.get(remote) {
            names.extend(meta.helper_names(kind));
        }
        names
    };
    let vfs_names = helper_names("vfs");
    let filter_names = helper_names("filter");
    let backend_names = helper_names("backend");
    let runtime_names = helper_names("runtime");
    let vfs_title = ctx.t_or("remoteConfig.vfsProfile", "VFS profile");
    let filter_title = ctx.t_or("remoteConfig.filterProfile", "Filter profile");
    let backend_title = ctx.t_or("remoteConfig.backendProfile", "Backend profile");
    let runtime_title = ctx.t_or("remoteConfig.runtimeProfile", "Runtime profile");
    let vfs_row = dialogs::helper_combo(&vfs_title, &vfs_names, &initial.app.vfs_profile);
    let filter_row =
        dialogs::helper_combo(&filter_title, &filter_names, &initial.app.filter_profile);
    let backend_row =
        dialogs::helper_combo(&backend_title, &backend_names, &initial.app.backend_profile);
    let runtime_row = dialogs::helper_combo(
        &runtime_title,
        &runtime_names,
        &initial.app.runtime_remote_profile,
    );
    vfs_row.set_visible(op.supports_vfs());

    let src_kind = if op != OperationType::Copyurl {
        Some(dialogs::attach_path_kind(&src, remote))
    } else {
        None
    };
    let dst_kind = if !matches!(
        op,
        OperationType::Mount | OperationType::Serve | OperationType::Delete
    ) {
        Some(dialogs::attach_path_kind(&dst, remote))
    } else {
        None
    };

    let identity = adw::PreferencesGroup::new();
    identity.set_title(&ctx.t_or("wizards.appOperation.sourcePaths", "Paths"));
    if let Some(kind) = &src_kind {
        identity.add(kind);
    }
    identity.add(&src);
    for row in extra_sources.borrow().iter() {
        identity.add(row);
    }
    if op.supports_multi_source() {
        let add_src = gtk::Button::with_label(&ctx.t_or("remoteConfig.addSource", "Add source"));
        let extra_sources = extra_sources.clone();
        let identity_for_add = identity.clone();
        let ctx_add = ctx.clone();
        add_src.connect_clicked(move |_| {
            let row = extra_source_row(&ctx_add, "");
            identity_for_add.add(&row);
            extra_sources.borrow_mut().push(row);
        });
        let add_row = adw::ActionRow::new();
        add_row.set_title(&ctx.t_or("remoteConfig.multipleSources", "Multiple sources"));
        add_row.add_suffix(&add_src);
        identity.add(&add_row);
    }
    if let Some(kind) = &dst_kind {
        identity.add(kind);
    }
    identity.add(&dst);
    identity.add(&dest_status);
    identity.add(&serve);
    identity.add(&mount_type);

    let automation = adw::PreferencesGroup::new();
    automation.set_title(&ctx.t_or("remoteConfig.automation", "Automation"));
    automation.add(&auto_start);
    automation.add(&cron_enabled);
    automation.add(&cron);
    let cron_preset_row = adw::ActionRow::new();
    cron_preset_row.set_title(&ctx.t_or("remoteConfig.cron", "Cron schedule"));
    cron_preset_row.set_visible(op.is_automatable());
    cron_preset_row.add_suffix(&dialogs::attach_cron_builder(&cron));
    automation.add(&cron_preset_row);
    automation.add(&watch_enabled);
    automation.add(&watch_delay);
    automation.add(&watch_changed);

    let helpers = adw::PreferencesGroup::new();
    helpers.set_title(&ctx.t_or("remoteConfig.linkedHelpers", "Linked helper profiles"));
    helpers.add(&vfs_row);
    helpers.add(&filter_row);
    helpers.add(&backend_row);
    helpers.add(&runtime_row);

    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&ctx.t_or("remoteConfig.flags", "Flags"));
    let search = adw::EntryRow::new();
    search.set_title(&ctx.t_or("remoteConfig.filterFlags", "Filter flags"));
    flags_group.add(&search);
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let mut options: Vec<FlagOption> = static_flags_for(op);
    if let Some(category) = flag_category_for_op(op) {
        for (_, option) in options_for_category(blocks, category) {
            if options.iter().any(|o| o.field_name == option.field_name) {
                continue;
            }
            options.push(option.clone());
        }
    }
    for flag in options {
        if op == OperationType::Serve && flag.field_name == "type" {
            continue;
        }
        let row = flag_entry(&ctx, &flag, &rclone);
        flags_group.add(&row);
        flag_rows
            .borrow_mut()
            .push((flag.field_name, row, flag.type_name));
    }
    let serve_flag_rows: Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    if op == OperationType::Serve {
        for serve_type in serve_types.iter() {
            for flag in crate::flags::collect_serve_flags(blocks, serve_type) {
                let row = flag_entry(&ctx, &flag, &rclone);
                row.set_title(&format!("{serve_type} · {}", flag.name));
                let selected =
                    crate::operations::selected_or(&serve_types, serve.selected(), "http");
                row.set_visible(serve_type == selected);
                flags_group.add(&row);
                serve_flag_rows.borrow_mut().push((
                    serve_type.clone(),
                    flag.field_name,
                    row,
                    flag.type_name,
                ));
            }
        }
        {
            let serve_flag_rows = serve_flag_rows.clone();
            let serve_types = serve_types.clone();
            serve.connect_selected_notify(move |row| {
                let selected = crate::operations::selected_or(&serve_types, row.selected(), "http");
                for (serve_type, _, widget, _) in serve_flag_rows.borrow().iter() {
                    widget.set_visible(serve_type == selected);
                }
            });
        }
    }
    let json_toggle = adw::SwitchRow::new();
    json_toggle.set_title(&ctx.t_or("remoteConfig.jsonMode", "JSON mode"));
    json_toggle.set_subtitle("Edit this profile's rclone flags as a JSON object");
    json_toggle.set_active(ctx.settings.borrow().runtime.show_json_mode);
    let json_view = gtk::TextView::new();
    json_view.set_monospace(true);
    json_view.set_wrap_mode(gtk::WrapMode::WordChar);
    let json_doc = if ctx.settings.borrow().general.restrict {
        crate::restrict::redact_value(&rclone)
    } else {
        rclone.clone()
    };
    json_view
        .buffer()
        .set_text(&serde_json::to_string_pretty(&json_doc).unwrap_or_else(|_| "{}".into()));
    let json_scroll = gtk::ScrolledWindow::new();
    json_scroll.set_min_content_height(180);
    json_scroll.set_child(Some(&json_view));
    json_scroll.set_visible(json_toggle.is_active());
    {
        let ctx = ctx.clone();
        let json_scroll = json_scroll.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let search = search.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        json_toggle.connect_active_notify(move |row| {
            let on = row.is_active();
            ctx.settings.borrow_mut().runtime.show_json_mode = on;
            ctx.persist();
            json_scroll.set_visible(on);
            search.set_visible(!on);
            for (_, widget, _) in flag_rows.borrow().iter() {
                widget.set_visible(!on);
            }
            let selected = crate::operations::selected_or(&serve_types, serve.selected(), "http");
            for (serve_type, _, widget, _) in serve_flag_rows.borrow().iter() {
                widget.set_visible(!on && serve_type == selected);
            }
        });
    }
    {
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        search.connect_changed(move |entry| {
            let query = entry.text().to_ascii_lowercase();
            for (field, row, _) in flag_rows.borrow().iter() {
                row.set_visible(query.is_empty() || field.to_ascii_lowercase().contains(&query));
            }
            let selected = crate::operations::selected_or(&serve_types, serve.selected(), "http");
            for (serve_type, field, row, _) in serve_flag_rows.borrow().iter() {
                let matches = query.is_empty() || field.to_ascii_lowercase().contains(&query);
                row.set_visible(serve_type == selected && matches);
            }
        });
    }

    let cli = adw::EntryRow::new();
    cli.set_title(&ctx.t_or("remoteConfig.importCliFlags", "Import rclone CLI flags"));
    let apply_cli = gtk::Button::with_label(&ctx.t_or("common.apply", "Apply"));
    {
        let flag_rows = flag_rows.clone();
        let cli = cli.clone();
        apply_cli.connect_clicked(move |_| {
            let parsed = crate::jobs::parse_cli_flags(&cli.text());
            for (field, row, _) in flag_rows.borrow().iter() {
                if let Some(value) = parsed
                    .get(field)
                    .or_else(|| parsed.get(&field.replace('-', "_")))
                {
                    row.set_text(&value_to_text(value));
                }
            }
        });
    }
    let cli_row = adw::ActionRow::new();
    cli_row.set_title(&ctx.t_or("remoteConfig.cliImport", "CLI import"));
    cli_row.add_suffix(&apply_cli);
    flags_group.add(&cli);
    flags_group.add(&cli_row);
    flags_group.add(&json_toggle);
    let json_holder = adw::ActionRow::new();
    json_holder.set_title(&ctx.t_or("remoteConfig.jsonDocument", "JSON document"));
    json_holder.set_activatable(false);
    json_holder.set_child(Some(&json_scroll));
    flags_group.add(&json_holder);

    let page = adw::PreferencesPage::new();
    page.add(&switcher.group);
    page.add(&identity);
    page.add(&automation);
    page.add(&helpers);
    page.add(&flags_group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
    box_.set_margin_start(4);
    box_.set_margin_end(4);
    box_.append(&page);
    box_.append(&cron_hint);

    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let selected = selected.clone();
        let combo = switcher.combo.clone();
        let names = switcher.names.clone();
        let src = src.clone();
        let dst = dst.clone();
        combo.connect_selected_notify(move |row| {
            let Some(name) = names.borrow().get(row.selected() as usize).cloned() else {
                return;
            };
            *selected.borrow_mut() = name.clone();
            if let Some(profile) = ctx
                .store
                .borrow()
                .remotes
                .get(&remote)
                .and_then(|m| m.get_profile(op, &name))
            {
                let rclone = flatten_rclone(&profile.rclone);
                src.set_text(&default_source(&remote, &rclone));
                dst.set_text(&default_dest(&remote, &rclone, op));
            }
        });
    }
    wire_profile_actions(
        parent,
        ctx.clone(),
        remote,
        Some(op),
        None,
        &switcher,
        selected.clone(),
    );
    if auto_add {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let names = switcher.names.clone();
        let combo = switcher.combo.clone();
        let selected = selected.clone();
        glib::idle_add_local_once(move || {
            prompt_create_profile(&parent, ctx, remote, Some(op), None, names, combo, selected);
        });
    }

    let saver = {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let selected = selected.clone();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let extra_sources = extra_sources.clone();
        let auto_start = auto_start.clone();
        let cron_enabled = cron_enabled.clone();
        let cron = cron.clone();
        let watch_enabled = watch_enabled.clone();
        let watch_delay = watch_delay.clone();
        let watch_changed = watch_changed.clone();
        let vfs_row = vfs_row.clone();
        let filter_row = filter_row.clone();
        let backend_row = backend_row.clone();
        let runtime_row = runtime_row.clone();
        let vfs_names = vfs_names.clone();
        let filter_names = filter_names.clone();
        let backend_names = backend_names.clone();
        let runtime_names = runtime_names.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let json_toggle = json_toggle.clone();
        let json_view = json_view.clone();
        let serve_types = serve_types.clone();
        let mount_types = mount_types.clone();
        let mount_type = mount_type.clone();
        move || {
            let name = selected.borrow().clone();
            if name.is_empty() {
                return;
            }
            let mut flags = Map::new();
            if op == OperationType::Serve {
                flags.insert(
                    "type".into(),
                    json!(crate::operations::selected_or(
                        &serve_types,
                        serve.selected(),
                        "webdav"
                    )),
                );
            }
            if op == OperationType::Mount {
                flags.insert(
                    "mountType".into(),
                    json!(crate::operations::selected_or(
                        &mount_types,
                        mount_type.selected(),
                        "mount"
                    )),
                );
            }
            if json_toggle.is_active() {
                let buffer = json_view.buffer();
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                if let Ok(map) = crate::flags::parse_json_object(&text) {
                    flags.extend(map);
                }
            } else {
                for (field, row, type_name) in flag_rows.borrow().iter() {
                    let text = row.text().to_string();
                    if !text.is_empty() {
                        flags.insert(field.clone(), parse_flag_value(type_name, &text));
                    }
                }
                let selected_serve =
                    crate::operations::selected_or(&serve_types, serve.selected(), "webdav");
                for (serve_type, field, row, type_name) in serve_flag_rows.borrow().iter() {
                    if serve_type != selected_serve {
                        continue;
                    }
                    let text = row.text().to_string();
                    if !text.is_empty() {
                        flags.insert(field.clone(), parse_flag_value(type_name, &text));
                    }
                }
            }
            let mut sources = vec![src.text().to_string()];
            for row in extra_sources.borrow().iter() {
                let text = row.text().to_string();
                if !text.is_empty() {
                    sources.push(text);
                }
            }
            if op != OperationType::Copyurl {
                sources = sources
                    .into_iter()
                    .map(|s| crate::path_kind::resolve_job_path(&s, &remote))
                    .collect();
            }
            let dest = if matches!(op, OperationType::Serve | OperationType::Delete) {
                dst.text().to_string()
            } else {
                crate::path_kind::resolve_job_path(&dst.text(), &remote)
            };
            let rclone = assemble_rclone(op, &sources, &dest, flags);
            let profile = ProfileConfig {
                name: name.clone(),
                app: AppConfig {
                    auto_start: auto_start.is_active(),
                    cron_enabled: cron_enabled.is_active(),
                    cron_expression: cron.text().to_string(),
                    watch_enabled: watch_enabled.is_active(),
                    watch_delay: watch_delay.text().parse().unwrap_or(0),
                    watch_changed_only: watch_changed.is_active(),
                    vfs_profile: dialogs::helper_selected(&vfs_row, &vfs_names),
                    filter_profile: dialogs::helper_selected(&filter_row, &filter_names),
                    backend_profile: dialogs::helper_selected(&backend_row, &backend_names),
                    runtime_remote_profile: dialogs::helper_selected(&runtime_row, &runtime_names),
                },
                rclone,
            };
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                meta.upsert_profile(op, profile);
            }
            ctx.persist();
        }
    };
    (box_, saver)
}

fn helper_page(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    kind: &'static str,
    blocks: &[FlagBlock],
) -> (gtk::Box, impl Fn() + 'static) {
    let names = {
        let mut names = ctx
            .store
            .borrow()
            .remotes
            .get(remote)
            .map(|m| m.helper_names(kind))
            .unwrap_or_default();
        if names.is_empty() {
            names.push("default".into());
        }
        names
    };
    let selected = Rc::new(RefCell::new(names[0].clone()));
    let current = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .and_then(|m| m.helper_profile(kind, &names[0]))
        .unwrap_or_else(|| json!({}));
    let switcher = profile_switcher(&ctx, &names, &names[0]);
    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&ctx.t_or("remoteConfig.options", "Options"));
    let search = adw::EntryRow::new();
    search.set_title(&ctx.t_or("remoteConfig.filterFlags", "Filter flags"));
    flags_group.add(&search);
    let category = match kind {
        "runtime" => "backend",
        other => other,
    };
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for (_, option) in options_for_category(blocks, category) {
        let row = flag_entry(&ctx, option, &current);
        flags_group.add(&row);
        flag_rows
            .borrow_mut()
            .push((option.field_name.clone(), row, option.type_name.clone()));
    }
    if flag_rows.borrow().is_empty() {
        let json_row = adw::EntryRow::new();
        json_row.set_title(&ctx.t_or("remoteConfig.jsonObject", "JSON object"));
        json_row.set_text(&serde_json::to_string(&current).unwrap_or_else(|_| "{}".into()));
        flags_group.add(&json_row);
        flag_rows
            .borrow_mut()
            .push(("_json".into(), json_row, "string".into()));
    }
    {
        let flag_rows = flag_rows.clone();
        search.connect_changed(move |entry| {
            let query = entry.text().to_ascii_lowercase();
            for (field, row, _) in flag_rows.borrow().iter() {
                row.set_visible(query.is_empty() || field.to_ascii_lowercase().contains(&query));
            }
        });
    }

    let page = adw::PreferencesPage::new();
    page.add(&switcher.group);
    page.add(&flags_group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    box_.append(&page);

    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let selected = selected.clone();
        let combo = switcher.combo.clone();
        let names = switcher.names.clone();
        let flag_rows = flag_rows.clone();
        combo.connect_selected_notify(move |row| {
            let Some(name) = names.borrow().get(row.selected() as usize).cloned() else {
                return;
            };
            *selected.borrow_mut() = name.clone();
            let value = ctx
                .store
                .borrow()
                .remotes
                .get(&remote)
                .and_then(|m| m.helper_profile(kind, &name))
                .unwrap_or_else(|| json!({}));
            for (field, row, _) in flag_rows.borrow().iter() {
                if field == "_json" {
                    row.set_text(&serde_json::to_string(&value).unwrap_or_else(|_| "{}".into()));
                } else if let Some(v) = value.get(field) {
                    row.set_text(&value_to_text(v));
                } else {
                    row.set_text("");
                }
            }
        });
    }
    wire_profile_actions(
        parent,
        ctx.clone(),
        remote,
        None,
        Some(kind),
        &switcher,
        selected.clone(),
    );

    let saver = {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let selected = selected.clone();
        let flag_rows = flag_rows.clone();
        move || {
            let name = selected.borrow().clone();
            if name.is_empty() {
                return;
            }
            let mut obj = Map::new();
            for (field, row, type_name) in flag_rows.borrow().iter() {
                let text = row.text().to_string();
                if field == "_json" {
                    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&text) {
                        obj = map;
                    }
                    break;
                }
                if !text.is_empty() {
                    obj.insert(field.clone(), parse_flag_value(type_name, &text));
                }
            }
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                meta.upsert_helper(kind, &name, Value::Object(obj));
            }
            ctx.persist();
        }
    };
    (box_, saver)
}

struct ProfileSwitcher {
    group: adw::PreferencesGroup,
    combo: adw::ComboRow,
    names: Rc<RefCell<Vec<String>>>,
    add: gtk::Button,
    clone: gtk::Button,
    rename: gtk::Button,
    delete: gtk::Button,
}

fn profile_switcher(ctx: &AppCtx, names: &[String], selected: &str) -> ProfileSwitcher {
    let names = Rc::new(RefCell::new(names.to_vec()));
    let combo = adw::ComboRow::new();
    combo.set_title(&ctx.t_or("modals.remoteConfig.profile", "Profile"));
    refresh_combo(&combo, &names.borrow());
    if let Some(idx) = names.borrow().iter().position(|n| n == selected) {
        combo.set_selected(idx as u32);
    }
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some(
        &ctx.t_or("modals.remoteConfig.addProfile", "Add profile"),
    ));
    add.set_valign(gtk::Align::Center);
    let clone = gtk::Button::from_icon_name("edit-copy-symbolic");
    clone.set_tooltip_text(Some(
        &ctx.t_or("modals.remoteConfig.cloneProfile", "Clone profile"),
    ));
    clone.set_valign(gtk::Align::Center);
    let rename = gtk::Button::from_icon_name("document-edit-symbolic");
    rename.set_tooltip_text(Some(
        &ctx.t_or("modals.remoteConfig.renameProfile", "Rename profile"),
    ));
    rename.set_valign(gtk::Align::Center);
    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.set_tooltip_text(Some(
        &ctx.t_or("modals.remoteConfig.deleteProfile", "Delete profile"),
    ));
    delete.set_valign(gtk::Align::Center);
    combo.add_suffix(&add);
    combo.add_suffix(&clone);
    combo.add_suffix(&rename);
    combo.add_suffix(&delete);
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("modals.remoteConfig.profiles", "Profiles"));
    group.add(&combo);
    ProfileSwitcher {
        group,
        combo,
        names,
        add,
        clone,
        rename,
        delete,
    }
}

fn prompt_create_profile(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: String,
    op: Option<OperationType>,
    helper: Option<&'static str>,
    names: Rc<RefCell<Vec<String>>>,
    combo: adw::ComboRow,
    selected: Rc<RefCell<String>>,
) {
    let title = ctx.t_or("modals.remoteConfig.addProfile", "Add profile");
    let label = ctx.t_or("common.name", "Name");
    dialogs::prompt(parent, &title, &label, "default", move |name| {
        if name.is_empty() {
            return;
        }
        mutate_profiles(&ctx, &remote, op, helper, |meta| {
            if let Some(op) = op {
                meta.upsert_profile(
                    op,
                    ProfileConfig {
                        name: name.clone(),
                        ..Default::default()
                    },
                );
            } else if let Some(kind) = helper {
                meta.upsert_helper(kind, &name, json!({}));
            }
        });
        names.borrow_mut().push(name.clone());
        refresh_combo(&combo, &names.borrow());
        if let Some(idx) = names.borrow().iter().position(|n| n == &name) {
            combo.set_selected(idx as u32);
        }
        *selected.borrow_mut() = name;
    });
}

fn wire_profile_actions(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    op: Option<OperationType>,
    helper: Option<&'static str>,
    switcher: &ProfileSwitcher,
    selected: Rc<RefCell<String>>,
) {
    let bind = |btn: &gtk::Button, kind: &'static str| {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let remote = remote.to_string();
        let combo = switcher.combo.clone();
        let names = switcher.names.clone();
        let selected = selected.clone();
        btn.connect_clicked(move |_| {
            let current = names
                .borrow()
                .get(combo.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "default".into());
            match kind {
                "add" => prompt_create_profile(
                    &parent,
                    ctx.clone(),
                    remote.clone(),
                    op,
                    helper,
                    names.clone(),
                    combo.clone(),
                    selected.clone(),
                ),
                "clone" => dialogs::prompt(
                    &parent,
                    "Clone profile",
                    "New name",
                    &format!("{current}-copy"),
                    {
                        let ctx = ctx.clone();
                        let remote = remote.clone();
                        let names = names.clone();
                        let combo = combo.clone();
                        let selected = selected.clone();
                        let current = current.clone();
                        move |name| {
                            if name.is_empty() {
                                return;
                            }
                            mutate_profiles(&ctx, &remote, op, helper, |meta| {
                                if let Some(op) = op {
                                    let _ = meta.clone_profile(op, &current, &name);
                                } else if let Some(kind) = helper {
                                    if let Some(value) = meta.helper_profile(kind, &current) {
                                        meta.upsert_helper(kind, &name, value);
                                    }
                                }
                            });
                            names.borrow_mut().push(name.clone());
                            refresh_combo(&combo, &names.borrow());
                            if let Some(idx) = names.borrow().iter().position(|n| n == &name) {
                                combo.set_selected(idx as u32);
                            }
                            *selected.borrow_mut() = name;
                        }
                    },
                ),
                "rename" => dialogs::prompt(
                    &parent,
                    &ctx.t_or("modals.remoteConfig.renameProfile", "Rename profile"),
                    &ctx.t_or("modals.remoteConfig.newName", "New name"),
                    &current,
                    {
                        let ctx = ctx.clone();
                        let remote = remote.clone();
                        let names = names.clone();
                        let combo = combo.clone();
                        let selected = selected.clone();
                        let current = current.clone();
                        move |name| {
                            if name.is_empty() {
                                return;
                            }
                            mutate_profiles(&ctx, &remote, op, helper, |meta| {
                                if let Some(op) = op {
                                    let _ = meta.rename_profile(op, &current, &name);
                                } else if let Some(kind) = helper {
                                    let _ = meta.rename_helper(kind, &current, &name);
                                }
                            });
                            if helper.is_none() {
                                ctx.store
                                    .borrow_mut()
                                    .rename_runtime_profile(&remote, &current, &name);
                                crate::jobs::rename_jobs_profile(
                                    &mut ctx.snapshot.borrow_mut().jobs,
                                    &remote,
                                    &current,
                                    &name,
                                );
                                ctx.persist();
                            }
                            if let Some(slot) =
                                names.borrow_mut().iter_mut().find(|n| *n == &current)
                            {
                                *slot = name.clone();
                            }
                            refresh_combo(&combo, &names.borrow());
                            if let Some(idx) = names.borrow().iter().position(|n| n == &name) {
                                combo.set_selected(idx as u32);
                            }
                            *selected.borrow_mut() = name;
                        }
                    },
                ),
                "delete" => {
                    let snap = ctx.snapshot.borrow();
                    let usage = crate::jobs::profile_usage(
                        &snap.jobs,
                        &snap.mounts,
                        &snap.serves,
                        &remote,
                        &current,
                        op,
                    );
                    drop(snap);
                    if usage.blocked() {
                        let alert = adw::AlertDialog::new(
                            Some("Profile is in use"),
                            Some(&usage.summary()),
                        );
                        alert.add_response("ok", "OK");
                        alert.present(Some(&parent));
                        return;
                    }
                    mutate_profiles(&ctx, &remote, op, helper, |meta| {
                        if let Some(op) = op {
                            let _ = meta.remove_profile(op, &current);
                        } else if let Some(kind) = helper {
                            let _ = meta.remove_helper(kind, &current);
                        }
                    });
                    names.borrow_mut().retain(|n| n != &current);
                    if names.borrow().is_empty() {
                        names.borrow_mut().push("default".into());
                    }
                    refresh_combo(&combo, &names.borrow());
                    combo.set_selected(0);
                    *selected.borrow_mut() = names.borrow()[0].clone();
                }
                _ => {}
            }
        });
    };
    bind(&switcher.add, "add");
    bind(&switcher.clone, "clone");
    bind(&switcher.rename, "rename");
    bind(&switcher.delete, "delete");
}

fn mutate_profiles(
    ctx: &AppCtx,
    remote: &str,
    _op: Option<OperationType>,
    _helper: Option<&str>,
    f: impl FnOnce(&mut RemoteMeta),
) {
    if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(remote) {
        f(meta);
    }
    ctx.persist();
}

fn refresh_combo(combo: &adw::ComboRow, names: &[String]) {
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    combo.set_model(Some(&gtk::StringList::new(&refs)));
}

fn extra_source_row(ctx: &AppCtx, value: &str) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&ctx.t_or("remoteConfig.additionalSource", "Additional source"));
    row.set_text(value);
    dialogs::attach_path_picker(ctx, &row, crate::picker::FilePickerConfig::folders());
    row
}

fn flag_entry(ctx: &AppCtx, flag: &FlagOption, current: &Value) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&ctx.option_label(&flag.name, "title", &flag.name));
    let help = ctx.option_label(&flag.name, "help", &flag.help);
    if !help.is_empty() {
        row.set_tooltip_text(Some(&help));
    }
    let text = current
        .get(&flag.field_name)
        .or_else(|| current.get(&flag.name))
        .map(value_to_text)
        .or_else(|| {
            extra_flags(current)
                .get(&flag.field_name)
                .map(value_to_text)
        })
        .unwrap_or_else(|| flag.default_str.clone());
    row.set_text(&text);
    row
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string().trim_matches('"').to_string(),
    }
}

fn action_summary(ids: &[String]) -> String {
    if ids.is_empty() {
        "All actions (default order)".into()
    } else {
        ids.join(" · ")
    }
}

fn update_cron_hint(ctx: &AppCtx, row: &adw::EntryRow, hint: &gtk::Label) {
    let expr = row.text().to_string();
    if expr.is_empty() {
        hint.set_text("");
        return;
    }
    match validate_cron(&expr) {
        Ok(()) => hint.set_text(&crate::rclone::describe_cron_i18n(
            &expr,
            &ctx.i18n.borrow(),
        )),
        Err(e) => hint.set_text(&format!(
            "{}: {e}",
            ctx.t_or("cron.invalid", "Invalid cron")
        )),
    }
}
