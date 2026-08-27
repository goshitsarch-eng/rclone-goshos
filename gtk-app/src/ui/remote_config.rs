//! Angular-style remote configuration sidenav: per-operation profiles,
//! helper configs (VFS / filter / backend / runtime), and remote metadata.

use super::dialogs;
use super::flag_widget::{FlagRow, FlagWidget, ServeFlagRow};
use super::interactive::InteractivePanel;
use super::AppCtx;
use crate::config_steps::{editor_steps, parse_open_step, EditorStep};
use crate::flags::{
    flag_category_for_op, options_for_category, parse_flag_value, static_flags_for, FlagBlock,
    FlagOption,
};
use crate::jobs::{
    assemble_rclone, default_dest, default_source, flatten_rclone, path_list, SOURCE_KEYS,
};
use crate::operations::OperationType;
use crate::rclone::validate_cron;
use crate::store::{AppConfig, ProfileConfig, RemoteMeta};
use adw::prelude::*;
use gtk::glib;
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Default)]
pub struct RemoteConfigOpen {
    pub initial: Option<String>,
    pub profile: Option<String>,
    pub auto_add: bool,
    pub clone_from: Option<String>,
}

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
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(
        &ctx.t_or("modals.remoteConfig.search", "Search pages"),
    ));
    side_col.append(&search);
    side_col.append(&side_scroll);
    let obscure_fields = Rc::new(RefCell::new(sensitive_fields_for(&ctx, &remote)));
    {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let fields = obscure_fields.clone();
        let apply_ctx = ctx.clone();
        dialogs::obscure_tool(
            &ctx,
            fields,
            Rc::new(move |key, value| {
                if let Some(client) = apply_ctx.client() {
                    let mut params = serde_json::Map::new();
                    params.insert(key.to_string(), json!(value));
                    if client
                        .update_remote(&remote, Value::Object(params), None)
                        .is_ok()
                    {
                        apply_ctx.refresh_runtime();
                    }
                }
            }),
        )
        .add_to_box(&side_col);
    }
    split.set_sidebar(Some(&side_col));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_hexpand(true);
    content.set_vexpand(true);
    let title = gtk::Label::new(Some(
        &ctx.t_or("modals.remoteConfig.steps.remote", "Remote"),
    ));
    title.add_css_class("title-3");
    title.set_xalign(0.0);
    title.set_hexpand(true);
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
    title_row.append(&title);
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
    content.append(&title_row);
    content.append(&body_scroll);
    content.append(&save_bar);
    split.set_content(Some(&content));
    dialog.set_child(Some(&split));

    let flag_blocks: Rc<Vec<FlagBlock>> = Rc::new(
        ctx.client()
            .map(|c| c.option_flag_blocks())
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
    let initial_step = parse_open_step(open.initial.as_deref());
    *current.borrow_mut() = initial_step;
    if let Some(idx) = editor_steps().iter().position(|step| *step == initial_step) {
        if let Some(row) = sidebar.row_at_index(idx as i32) {
            sidebar.select_row(Some(&row));
        }
    } else if let Some(first) = sidebar.row_at_index(0) {
        sidebar.select_row(Some(&first));
    }
    {
        let sidebar = sidebar.clone();
        let ctx = ctx.clone();
        search.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            for (idx, step) in editor_steps().into_iter().enumerate() {
                let Some(row) = sidebar.row_at_index(idx as i32) else {
                    continue;
                };
                let label = step_label(&ctx, step);
                let alias = step.alias();
                row.set_visible(crate::pref_search::any_field_matches(
                    &[&label, alias],
                    &query,
                ));
            }
        });
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
                EditorStep::QuickOps => {}
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

fn step_label(ctx: &AppCtx, step: EditorStep) -> String {
    ctx.t_or(&step.i18n_key(), step.fallback_label())
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
                Some(&ctx.t_or(
                    "wizards.presets.applied",
                    "Default presets applied successfully",
                )),
                Some(&ctx.t_or(
                    "templates.applySuccess",
                    "Default provider / OS presets were merged into this remote.",
                )),
            );
            toast.add_response("ok", &ctx.t_or("common.ok", "OK"));
            toast.present(Some(&parent));
        });
    }
    let pick = adw::ComboRow::new();
    pick.set_title(&ctx.t_or("templates.userTemplates", "Saved User Templates"));
    let apply = gtk::Button::with_label(&ctx.t_or("common.apply", "Apply"));
    apply.add_css_class("suggested-action");
    let refresh_combo = {
        let ctx = ctx.clone();
        let pick = pick.clone();
        let apply = apply.clone();
        Rc::new(move || {
            let names =
                crate::user_templates::template_display_names(&ctx.store.borrow().templates);
            dialogs::fill_template_combo(&pick, &apply, &names);
        }) as Rc<dyn Fn()>
    };
    refresh_combo();
    {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let rebuild = rebuild.clone();
        let parent = parent.clone();
        let pick = pick.clone();
        apply.connect_clicked(move |_| {
            let templates = ctx.store.borrow().templates.clone();
            let Some(name) = dialogs::combo_selected_label(&pick) else {
                return;
            };
            let Some(template) = crate::user_templates::template_by_name(&templates, &name) else {
                return;
            };
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                crate::user_templates::apply_to_meta(
                    meta,
                    &template.values,
                    Some(crate::user_templates::REMOTE_CONFIG_TEMPLATE_CATEGORIES),
                    true,
                );
            }
            ctx.persist();
            rebuild();
            let msg = ctx.tf("templates.applySuccess", &[("name", &template.name)]);
            let toast = adw::AlertDialog::new(Some(&msg), None::<&str>);
            toast.add_response("ok", &ctx.t_or("common.ok", "OK"));
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
        let refresh_combo = refresh_combo.clone();
        save.connect_clicked(move |_| {
            dialogs::templates_capture_for_remote_ex(
                &parent,
                ctx.clone(),
                &remote,
                Some(refresh_combo.clone()),
            );
        });
    }
    let manage =
        gtk::Button::with_label(&ctx.t_or("templates.manageTemplates", "Manage Templates..."));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let refresh_combo = refresh_combo.clone();
        manage.connect_clicked(move |_| {
            dialogs::templates_with_on_change(&parent, ctx.clone(), refresh_combo.clone());
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

fn step_icon(step: EditorStep) -> &'static str {
    step.icon_name()
}

fn remote_page(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    _on_done: Rc<dyn Fn()>,
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
    primary_row.set_subtitle(&action_summary(&ctx, &primary_ids.borrow()));
    let edit_primary = gtk::Button::with_label(&ctx.t_or("common.edit", "Edit"));
    edit_primary.set_valign(gtk::Align::Center);
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let primary_ids = primary_ids.clone();
        let primary_row = primary_row.clone();
        edit_primary.connect_clicked(move |_| {
            let catalog = crate::action_order::catalog_ids();
            let current = primary_ids.borrow().clone();
            dialogs::action_order(
                &parent,
                &ctx,
                &ctx.t_or("remoteConfig.primaryActions", "Primary actions"),
                &catalog,
                &current,
                Some(3),
                {
                    let primary_ids = primary_ids.clone();
                    let primary_row = primary_row.clone();
                    let ctx = ctx.clone();
                    move |ids| {
                        primary_row.set_subtitle(&action_summary(&ctx, &ids));
                        *primary_ids.borrow_mut() = ids;
                    }
                },
            );
        });
    }
    primary_row.add_suffix(&edit_primary);
    let sync_row = adw::ActionRow::new();
    sync_row.set_title(&ctx.t_or("remoteConfig.syncActions", "Sync actions"));
    sync_row.set_subtitle(&action_summary(&ctx, &sync_ids.borrow()));
    let edit_sync = gtk::Button::with_label(&ctx.t_or("common.edit", "Edit"));
    edit_sync.set_valign(gtk::Align::Center);
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let sync_ids = sync_ids.clone();
        let sync_row = sync_row.clone();
        edit_sync.connect_clicked(move |_| {
            let catalog: Vec<&str> = OperationType::PRIMARY_SYNC
                .iter()
                .chain(OperationType::MORE_SYNC.iter())
                .map(|op| op.as_str())
                .collect();
            let current = sync_ids.borrow().clone();
            dialogs::action_order(
                &parent,
                &ctx,
                &ctx.t_or("remoteConfig.syncActions", "Sync actions"),
                &catalog,
                &current,
                Some(3),
                {
                    let sync_ids = sync_ids.clone();
                    let sync_row = sync_row.clone();
                    let ctx = ctx.clone();
                    move |ids| {
                        sync_row.set_subtitle(&action_summary(&ctx, &ids));
                        *sync_ids.borrow_mut() = ids;
                    }
                },
            );
        });
    }
    sync_row.add_suffix(&edit_sync);

    let reauth = gtk::Button::with_label(&ctx.t_or(
        "wizards.remoteConfig.authenticationMethod",
        "Re-authenticate…",
    ));
    reauth.set_valign(gtk::Align::Center);
    reauth.set_tooltip_text(Some(&ctx.t_or(
        "modals.oauth.manualOpenPrompt",
        "Start the rclone interactive / OAuth flow without leaving this dialog",
    )));
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
    let helper_row = adw::ActionRow::new();
    helper_row.set_title(&ctx.t_or("remoteConfig.namedHelpers", "Named helper profiles"));
    helper_row.add_suffix(&helpers);
    let auth_row = adw::ActionRow::new();
    auth_row.set_title(&ctx.t_or(
        "banners.engine.auth.title",
        "Rclone Authentication Required",
    ));
    auth_row.set_subtitle(&ctx.t_or(
        "banners.engine.auth.subtitle",
        "Please check your credentials or configuration password",
    ));
    auth_row.add_suffix(&reauth);
    actions.add(&helper_row);
    actions.add(&auth_row);

    let panel = InteractivePanel::new(&ctx);
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let parent = parent.clone();
        let panel = panel.clone();
        reauth.connect_clicked(move |_| {
            start_remote_reauth(&parent, ctx.clone(), &remote, &panel);
        });
    }
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let parent = parent.clone();
        let panel = panel.clone();
        panel.continue_btn.clone().connect_clicked(move |_| {
            continue_remote_reauth(&parent, ctx.clone(), &remote, &panel);
        });
    }
    {
        let ctx = ctx.clone();
        let panel = panel.clone();
        panel.cancel_btn.clone().connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                match client.oauth_stop() {
                    Ok(_) => {
                        panel.oauth.set_status(
                            &ctx.t_or("modals.remoteConfig.oauthCancelled", "OAuth cancelled"),
                        );
                        panel.apply(&ctx, crate::interactive::InteractiveFlowState::default());
                    }
                    Err(e) => panel.oauth.set_status(&e.to_string()),
                }
            }
        });
    }

    let (provider_fields, save_provider) =
        super::wizard::inline_provider_editor(parent, &ctx, remote);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    page.add(&actions);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    box_.append(&page);
    box_.append(&provider_fields);
    box_.append(&panel.root);

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
            save_provider();
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
    let copyurl_keys: &[&str] = &["url", "srcFs", "source"];
    let listed = if op == OperationType::Copyurl {
        path_list(&rclone, copyurl_keys)
    } else {
        path_list(&rclone, SOURCE_KEYS)
    };
    if let Some(first) = listed.first() {
        src.set_text(first);
    } else {
        src.set_text(&default_source(remote, &rclone));
    }
    let extra_sources: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    if listed.len() > 1 {
        for extra in listed.iter().skip(1) {
            let row = extra_source_row(&ctx, extra, op != OperationType::Copyurl, remote);
            extra_sources.borrow_mut().push(row);
        }
    }
    let saved_filenames: Vec<String> = rclone
        .get("filenames")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    let url_filename = adw::EntryRow::new();
    url_filename.set_title(&ctx.t_or(
        "wizards.appOperation.copyUrlFilename",
        "Filename (optional)",
    ));
    if let Some(name) = saved_filenames.first() {
        url_filename.set_text(name);
    }
    url_filename.set_visible(op == OperationType::Copyurl);
    let extra_filenames: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    if op == OperationType::Copyurl {
        for name in saved_filenames.iter().skip(1) {
            extra_filenames.borrow_mut().push(filename_row(&ctx, name));
        }
        while extra_filenames.borrow().len() < extra_sources.borrow().len() {
            extra_filenames.borrow_mut().push(filename_row(&ctx, ""));
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
        dialogs::attach_path_picker(
            &ctx,
            &dst,
            crate::picker::FilePickerConfig::local_mount_folders(),
        );
    } else if op != OperationType::Serve && op != OperationType::Delete {
        dialogs::attach_path_picker(
            &ctx,
            &dst,
            crate::picker::FilePickerConfig::folders().with_remote(remote),
        );
    }
    let dest_status = adw::ActionRow::new();
    dest_status.set_title(&ctx.t_or("remoteConfig.pathStatusTitle", "Path status"));
    dest_status.set_visible(crate::path_inspection::shows_dest_status(op));
    {
        let dest_status = dest_status.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let refresh_status = move |path: &str| {
            let resolved = crate::path_kind::resolve_job_path(path, &remote);
            let status = crate::path_inspection::inspect_dest_ex(
                &ctx.store.borrow(),
                &resolved,
                &remote,
                op,
                &ctx.snapshot.borrow().mounts,
                ctx.client().as_ref(),
                &ctx.engine_os(),
            );
            dest_status.set_subtitle(&super::dialogs::path_status_label(&ctx, &status));
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
    let op_label = ctx.t_or(
        &format!("modals.remoteConfig.steps.{}", op.as_str()),
        op.api_label(),
    );
    auto_start.set_title(&ctx.tf_or(
        "wizards.appOperation.enableAutoStart",
        "Enable Auto-{type} on Startup",
        &[("type", &op_label)],
    ));
    auto_start.set_active(initial.app.auto_start);
    let cron_enabled = adw::SwitchRow::new();
    cron_enabled.set_title(&ctx.tf_or(
        "wizards.appOperation.enableScheduled",
        "Enable Scheduled {type}",
        &[("type", &op_label)],
    ));
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
    let watch_supported = op.is_automatable() && ctx.is_local_backend();
    let watch_enabled = adw::SwitchRow::new();
    watch_enabled.set_title(&ctx.t_or("wizards.appOperation.enableWatch", "Watch local sources"));
    watch_enabled.set_subtitle(&ctx.t_or(
        "wizards.appOperation.watchDescription",
        "Watch local source directories for file modifications and sync changes automatically.",
    ));
    watch_enabled.set_active(initial.app.watch_enabled && watch_supported);
    watch_enabled.set_visible(watch_supported);
    let watch_delay = adw::EntryRow::new();
    watch_delay.set_title(&ctx.t_or("wizards.appOperation.watchDelay", "Watch delay (seconds)"));
    watch_delay.set_tooltip_text(Some(&ctx.t_or(
        "wizards.appOperation.watchDelayHint",
        "Seconds to wait after a change before starting the job.",
    )));
    watch_delay.set_text(&initial.app.watch_delay.to_string());
    watch_delay.set_visible(watch_supported);
    let watch_zero = gtk::Label::new(None);
    watch_zero.add_css_class("dim-label");
    watch_zero.set_xalign(0.0);
    watch_zero.set_wrap(true);
    watch_zero.set_visible(watch_supported);
    let refresh_watch_zero = {
        let ctx = ctx.clone();
        let watch_zero = watch_zero.clone();
        move |text: &str| {
            let zero = text.trim().is_empty() || text.trim() == "0";
            watch_zero.set_text(&if zero {
                ctx.t_or(
                    "wizards.appOperation.watchZeroDelayWarning",
                    "Instant mode (0s) triggers sync immediately on change. For large files or multiple edits, 3–5s is recommended so writes can finish.",
                )
            } else {
                String::new()
            });
        }
    };
    refresh_watch_zero(&watch_delay.text());
    {
        let refresh_watch_zero = refresh_watch_zero.clone();
        watch_delay.connect_changed(move |row| refresh_watch_zero(&row.text()));
    }
    let watch_changed = adw::SwitchRow::new();
    watch_changed.set_title(&ctx.t_or(
        "wizards.appOperation.watchChangedOnly",
        "Changed files only",
    ));
    watch_changed.set_active(initial.app.watch_changed_only);
    watch_changed.set_visible(watch_supported);
    let (guidance, refresh_guidance) = dialogs::attach_operation_guidance(
        &ctx,
        false,
        &watch_enabled,
        &src,
        &extra_sources,
        &dst,
        Rc::new(move || op),
    );

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
        Some(dialogs::attach_path_kind(&ctx, &src, remote))
    } else {
        None
    };
    let dst_kind = if !matches!(
        op,
        OperationType::Mount | OperationType::Serve | OperationType::Delete
    ) {
        Some(dialogs::attach_path_kind(&ctx, &dst, remote))
    } else {
        None
    };

    let identity = adw::PreferencesGroup::new();
    identity.set_title(&ctx.t_or("wizards.appOperation.sourcePaths", "Paths"));
    if let Some(kind) = &src_kind {
        identity.add(kind);
    }
    identity.add(&src);
    let src_status = adw::ActionRow::new();
    src_status.set_title(&ctx.t_or("remoteConfig.pathStatusTitle", "Path status"));
    src_status.set_visible(false);
    if crate::path_inspection::shows_source_status(op) {
        let src_status = src_status.clone();
        let ctx = ctx.clone();
        let remote_name = remote.to_string();
        let refresh_src = move |path: &str| {
            let resolved = crate::path_kind::resolve_job_path(path, &remote_name);
            let local = crate::path_kind::is_truly_local_path(&resolved, &ctx.engine_os());
            src_status.set_visible(local);
            if !local {
                return;
            }
            let status = crate::path_inspection::inspect_dest_ex(
                &ctx.store.borrow(),
                &resolved,
                &remote_name,
                op,
                &ctx.snapshot.borrow().mounts,
                ctx.client().as_ref(),
                &ctx.engine_os(),
            );
            src_status.set_subtitle(&super::dialogs::path_status_label(&ctx, &status));
        };
        refresh_src(&src.text());
        src.connect_changed(move |row| refresh_src(&row.text()));
    }
    identity.add(&src_status);
    identity.add(&url_filename);
    for (idx, row) in extra_sources.borrow().iter().enumerate() {
        identity.add(row);
        if let Some(name) = extra_filenames.borrow().get(idx) {
            identity.add(name);
        }
    }
    if op.supports_multi_source() {
        let add_src = gtk::Button::with_label(&ctx.t_or("remoteConfig.addSource", "Add source"));
        let extra_sources = extra_sources.clone();
        let extra_filenames = extra_filenames.clone();
        let identity_for_add = identity.clone();
        let ctx_add = ctx.clone();
        let refresh_guidance = refresh_guidance.clone();
        let is_copyurl = op == OperationType::Copyurl;
        let remote_name = remote.to_string();
        add_src.connect_clicked(move |_| {
            let row = extra_source_row(&ctx_add, "", !is_copyurl, &remote_name);
            {
                let refresh_guidance = refresh_guidance.clone();
                row.connect_changed(move |_| refresh_guidance());
            }
            identity_for_add.add(&row);
            extra_sources.borrow_mut().push(row);
            if is_copyurl {
                let name = filename_row(&ctx_add, "");
                identity_for_add.add(&name);
                extra_filenames.borrow_mut().push(name);
            }
            refresh_guidance();
        });
        let add_row = adw::ActionRow::new();
        add_row.set_title(&ctx.t_or("remoteConfig.multipleSources", "Multiple sources"));
        add_row.add_suffix(&add_src);
        identity.add(&add_row);
    }
    if op != OperationType::Copyurl {
        let extra_sources = extra_sources.clone();
        let extra_filenames = extra_filenames.clone();
        let identity_for_pick = identity.clone();
        let ctx_pick = ctx.clone();
        let src_row = src.clone();
        let refresh_guidance = refresh_guidance.clone();
        let is_copyurl = op == OperationType::Copyurl;
        let remote_name = remote.to_string();
        let mut cfg = crate::picker::FilePickerConfig::folders().with_remote(remote);
        cfg.multi = op.supports_multi_source();
        dialogs::attach_path_picker_with(
            &ctx,
            &src,
            cfg,
            Some(Rc::new(move |extras| {
                for path in extras {
                    if path.is_empty() || src_row.text() == path.as_str() {
                        continue;
                    }
                    if extra_sources
                        .borrow()
                        .iter()
                        .any(|row| row.text() == path.as_str())
                    {
                        continue;
                    }
                    let row = extra_source_row(&ctx_pick, &path, !is_copyurl, &remote_name);
                    {
                        let refresh_guidance = refresh_guidance.clone();
                        row.connect_changed(move |_| refresh_guidance());
                    }
                    identity_for_pick.add(&row);
                    extra_sources.borrow_mut().push(row);
                    if is_copyurl {
                        let name = filename_row(&ctx_pick, "");
                        identity_for_pick.add(&name);
                        extra_filenames.borrow_mut().push(name);
                    }
                    refresh_guidance();
                }
            })),
        );
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
    let cron_preset_row = dialogs::attach_cron_builder_row(&cron, &ctx);
    cron_preset_row.set_visible(op.is_automatable());
    automation.add(&cron_preset_row);
    automation.add(&watch_enabled);
    automation.add(&watch_delay);
    watch_zero.set_margin_start(12);
    watch_zero.set_margin_end(12);
    automation.add(&watch_zero);
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
    let flag_rows: Rc<RefCell<Vec<FlagRow>>> = Rc::new(RefCell::new(Vec::new()));
    let mut options: Vec<FlagOption> = static_flags_for(op);
    if let Some(category) = flag_category_for_op(op) {
        for (_, option) in options_for_category(blocks, category) {
            if options.iter().any(|o| o.field_name == option.field_name) {
                continue;
            }
            options.push(option.clone());
        }
    }
    let field_defs: Vec<crate::json_editor::JsonFieldDef> = {
        let mut defs: Vec<_> = options
            .iter()
            .filter(|flag| !(op == OperationType::Serve && flag.field_name == "type"))
            .map(crate::json_editor::JsonFieldDef::from_flag)
            .collect();
        if op == OperationType::Serve {
            for serve_type in serve_types.iter() {
                for flag in crate::flags::collect_serve_flags(blocks, serve_type) {
                    if !defs.iter().any(|def| def.key == flag.field_name) {
                        defs.push(crate::json_editor::JsonFieldDef::from_flag(&flag));
                    }
                }
            }
        }
        defs
    };
    for flag in options {
        if op == OperationType::Serve && flag.field_name == "type" {
            continue;
        }
        let row = flag_entry(&ctx, &flag, &rclone);
        row.add_to(&flags_group);
        flag_rows
            .borrow_mut()
            .push((flag.field_name, row, flag.type_name));
    }
    let serve_flag_rows: Rc<RefCell<Vec<ServeFlagRow>>> = Rc::new(RefCell::new(Vec::new()));
    if op == OperationType::Serve {
        for serve_type in serve_types.iter() {
            for flag in crate::flags::collect_serve_flags(blocks, serve_type) {
                let mut row = flag_entry(&ctx, &flag, &rclone);
                row.set_title(&format!("{serve_type} · {}", flag.name));
                let selected =
                    crate::operations::selected_or(&serve_types, serve.selected(), "http");
                row.set_visible(serve_type == selected);
                row.add_to(&flags_group);
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
    json_toggle.set_subtitle(&ctx.t_or(
        "remoteConfig.jsonPayloadHelp",
        "Edit this profile's rclone flags as a JSON object",
    ));
    json_toggle.set_active(ctx.settings.borrow().runtime.show_json_mode);
    let editor = super::json_editor::JsonEditor::new(&ctx);
    editor.set_fields(field_defs);
    editor.set_structural(crate::json_editor::structural_keys(Some(op)));
    editor.set_operation(Some(op));
    if let Some(key) = crate::json_editor::info_banner_key(Some(op), None) {
        editor.set_info(Some(&ctx.t_or(key, "")));
    }
    editor.set_restrict(ctx.settings.borrow().general.restrict);
    editor.set_value(&rclone);
    editor.root.set_visible(json_toggle.is_active());
    {
        let src = src.clone();
        let dst = dst.clone();
        let extra_sources = extra_sources.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let mount_type = mount_type.clone();
        let mount_types = mount_types.clone();
        editor.on_paths(Rc::new(move |recon| {
            if let Some(sources) = recon.sources {
                if let Some(first) = sources.first() {
                    if src.text() != first.as_str() {
                        src.set_text(first);
                    }
                }
                for (idx, path) in sources.iter().skip(1).enumerate() {
                    if let Some(row) = extra_sources.borrow().get(idx) {
                        if row.text() != path.as_str() {
                            row.set_text(path);
                        }
                    }
                }
            }
            if let Some(dest) = recon.dest {
                if dst.text() != dest.as_str() {
                    dst.set_text(&dest);
                }
            }
            if let Some(kind) = recon.serve_type {
                if let Some(idx) = serve_types.iter().position(|item| item == &kind) {
                    serve.set_selected(idx as u32);
                }
            }
            if let Some(kind) = recon.mount_type {
                if let Some(idx) = mount_types.iter().position(|item| item == &kind) {
                    mount_type.set_selected(idx as u32);
                }
            }
        }));
    }
    {
        let ctx = ctx.clone();
        let editor_root = editor.root.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let search = search.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        json_toggle.connect_active_notify(move |row| {
            let on = row.is_active();
            ctx.settings.borrow_mut().runtime.show_json_mode = on;
            ctx.persist();
            editor_root.set_visible(on);
            search.set_title(&ctx.t_or(
                if on {
                    "remoteConfig.filterKeys"
                } else {
                    "remoteConfig.filterFlags"
                },
                if on { "Filter keys" } else { "Filter flags" },
            ));
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
        let json_toggle = json_toggle.clone();
        let editor = editor.clone();
        search.connect_changed(move |entry| {
            let query = entry.text().to_string();
            if json_toggle.is_active() {
                editor.highlight_search(&query);
                return;
            }
            for (field, row, _) in flag_rows.borrow().iter() {
                let name = row.title();
                let help = row.help();
                row.set_visible(crate::config_search::matches_config_search(
                    &name, &help, field, &query,
                ));
            }
            let selected = crate::operations::selected_or(&serve_types, serve.selected(), "http");
            for (serve_type, field, row, _) in serve_flag_rows.borrow().iter() {
                let name = row.title();
                let help = row.help();
                let matches =
                    crate::config_search::matches_config_search(&name, &help, field, &query);
                row.set_visible(serve_type == selected && matches);
            }
        });
    }

    let cli_row = adw::ActionRow::new();
    cli_row.set_title(&ctx.t_or("remoteConfig.cliImport", "CLI import"));
    cli_row.set_subtitle(&ctx.t_or(
        "wizards.cliImport.description",
        "Paste an rclone command, preview mapped flags, then apply them.",
    ));
    let preview = gtk::Button::with_label(&ctx.t_or("wizards.cliImport.preview", "Preview"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let flag_rows = flag_rows.clone();
        let flags_group = flags_group.clone();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let remote = remote.to_string();
        let names = switcher.names.clone();
        let combo = switcher.combo.clone();
        let selected = selected.clone();
        preview.connect_clicked(move |_| {
            let flags_group = flags_group.clone();
            let remote_type = remote_type_of(&ctx, &remote);
            dialogs::present_cli_import(
                &parent,
                ctx.clone(),
                dialogs::CliImportOptions {
                    preferred: Some(op.as_str().to_string()),
                    remote_type,
                    is_quick_run: false,
                    can_create_new: true,
                    can_patch: true,
                    existing_profiles: names.borrow().clone(),
                    initial_cli: String::new(),
                },
                {
                    let ctx = ctx.clone();
                    let remote = remote.clone();
                    let names = names.clone();
                    let combo = combo.clone();
                    let selected = selected.clone();
                    let flag_rows = flag_rows.clone();
                    let src = src.clone();
                    let dst = dst.clone();
                    let serve = serve.clone();
                    let serve_types = serve_types.clone();
                    move |apply| {
                        match apply.profile_mode {
                            crate::cli_import::ProfileMode::New
                                if !apply.profile_name.is_empty() =>
                            {
                                mutate_profiles(&ctx, &remote, Some(op), None, |meta| {
                                    meta.upsert_profile(
                                        op,
                                        ProfileConfig {
                                            name: apply.profile_name.clone(),
                                            ..Default::default()
                                        },
                                    );
                                });
                                if !names.borrow().iter().any(|n| n == &apply.profile_name) {
                                    names.borrow_mut().push(apply.profile_name.clone());
                                }
                                refresh_combo(&combo, &names.borrow());
                                if let Some(idx) =
                                    names.borrow().iter().position(|n| n == &apply.profile_name)
                                {
                                    combo.set_selected(idx as u32);
                                }
                                *selected.borrow_mut() = apply.profile_name.clone();
                            }
                            crate::cli_import::ProfileMode::Override
                                if !apply.profile_name.is_empty() =>
                            {
                                if let Some(idx) =
                                    names.borrow().iter().position(|n| n == &apply.profile_name)
                                {
                                    combo.set_selected(idx as u32);
                                }
                                *selected.borrow_mut() = apply.profile_name.clone();
                            }
                            _ => {}
                        }
                        let apply = apply.clone();
                        let flag_rows = flag_rows.clone();
                        let flags_group = flags_group.clone();
                        let src = src.clone();
                        let dst = dst.clone();
                        let serve = serve.clone();
                        let serve_types = serve_types.clone();
                        glib::idle_add_local_once(move || {
                            dialogs::apply_cli_to_form(
                                &apply,
                                Some(&flags_group),
                                &flag_rows,
                                Some(&src),
                                Some(&dst),
                                Some(&serve),
                                serve_types.as_ref(),
                                None,
                            );
                        });
                    }
                },
            );
        });
    }
    cli_row.add_suffix(&preview);
    flags_group.add(&cli_row);
    flags_group.add(&json_toggle);
    let json_holder = adw::ActionRow::new();
    json_holder.set_title(&ctx.t_or("remoteConfig.jsonDocument", "JSON document"));
    json_holder.set_activatable(false);
    json_holder.set_child(Some(&editor.root));
    json_holder.set_visible(json_toggle.is_active());
    {
        let json_holder = json_holder.clone();
        json_toggle.connect_active_notify(move |row| {
            json_holder.set_visible(row.is_active());
        });
    }
    flags_group.add(&json_holder);

    let page = adw::PreferencesPage::new();
    page.add(&switcher.group);
    page.add(&identity);
    page.add(&guidance);
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
        let editor = editor.clone();
        let flag_rows = flag_rows.clone();
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
                editor.set_value(&rclone);
                for (field, row, _) in flag_rows.borrow().iter() {
                    if let Some(value) = rclone.get(field) {
                        row.set_text(&value_to_text(value));
                    } else {
                        row.set_text("");
                    }
                }
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
        let extra_filenames = extra_filenames.clone();
        let url_filename = url_filename.clone();
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
        let editor = editor.clone();
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
                match editor.parsed() {
                    Ok(map) => flags.extend(map),
                    Err(err) => {
                        dialogs::toast_near(&editor.view, &err);
                        return;
                    }
                }
            } else {
                if let Some((row, field, msg)) =
                    dialogs::first_invalid_flag(flag_rows.borrow().iter().map(
                        |(field, row, type_name)| (field.clone(), row.clone(), type_name.clone()),
                    ))
                {
                    dialogs::toast_near(&row.widget(), &format!("{field}: {msg}"));
                    return;
                }
                let selected_serve =
                    crate::operations::selected_or(&serve_types, serve.selected(), "webdav");
                if let Some((row, field, msg)) =
                    dialogs::first_invalid_flag(serve_flag_rows.borrow().iter().filter_map(
                        |(serve_type, field, row, type_name)| {
                            (serve_type == selected_serve).then_some((
                                field.clone(),
                                row.clone(),
                                type_name.clone(),
                            ))
                        },
                    ))
                {
                    dialogs::toast_near(&row.widget(), &format!("{field}: {msg}"));
                    return;
                }
                for (field, row, type_name) in flag_rows.borrow().iter() {
                    let text = row.text().to_string();
                    if !text.is_empty() {
                        flags.insert(field.clone(), parse_flag_value(type_name, &text));
                    }
                }
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
            } else {
                let mut names = vec![url_filename.text().to_string()];
                for row in extra_filenames.borrow().iter() {
                    names.push(row.text().to_string());
                }
                if names.iter().any(|s| !s.is_empty()) {
                    flags.insert("filenames".into(), json!(names));
                }
            }
            let dest = if matches!(op, OperationType::Serve | OperationType::Delete) {
                dst.text().to_string()
            } else {
                crate::path_kind::resolve_job_path(&dst.text(), &remote)
            };
            if crate::path_inspection::mount_dest_is_invalid(op, &dest, &ctx.engine_os()) {
                super::dialogs::toast_near(
                    &dst,
                    &ctx.t_or(
                        "wizards.appOperation.mountDestMustBeLocal",
                        "Mount destination must be a local folder",
                    ),
                );
                return;
            }
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
        "runtime" => "runtime",
        other => other,
    };
    let runtime_flags: Vec<crate::flags::FlagOption> = if kind == "runtime" {
        runtime_provider_flags(&ctx, remote)
    } else {
        Vec::new()
    };
    let flag_rows: Rc<RefCell<Vec<FlagRow>>> = Rc::new(RefCell::new(Vec::new()));
    let runtime_refs: Vec<(&str, &crate::flags::FlagOption)> = runtime_flags
        .iter()
        .map(|option| ("runtime", option))
        .collect();
    let options: Vec<(&str, &crate::flags::FlagOption)> =
        if kind == "runtime" && !runtime_refs.is_empty() {
            runtime_refs
        } else {
            options_for_category(
                blocks,
                if category == "runtime" {
                    "backend"
                } else {
                    category
                },
            )
        };
    for (_, option) in &options {
        let row = flag_entry(&ctx, option, &current);
        row.add_to(&flags_group);
        flag_rows
            .borrow_mut()
            .push((option.field_name.clone(), row, option.type_name.clone()));
    }
    let json_toggle = adw::SwitchRow::new();
    json_toggle.set_title(&ctx.t_or("remoteConfig.jsonMode", "JSON mode"));
    json_toggle.set_active(flag_rows.borrow().is_empty());
    let helper_defs: Vec<crate::json_editor::JsonFieldDef> = options
        .iter()
        .map(|(_, option)| crate::json_editor::JsonFieldDef::from_flag(option))
        .collect();
    let editor = super::json_editor::JsonEditor::new(&ctx);
    editor.set_fields(helper_defs);
    if let Some(key) = crate::json_editor::info_banner_key(None, Some(kind)) {
        editor.set_info(Some(&ctx.t_or(key, "")));
    }
    editor.set_restrict(ctx.settings.borrow().general.restrict);
    editor.set_value(&current);
    editor.root.set_visible(json_toggle.is_active());
    flags_group.add(&json_toggle);
    {
        let editor_root = editor.root.clone();
        let flag_rows = flag_rows.clone();
        let search = search.clone();
        let ctx = ctx.clone();
        json_toggle.connect_active_notify(move |row| {
            let on = row.is_active();
            editor_root.set_visible(on);
            search.set_title(&ctx.t_or(
                if on {
                    "remoteConfig.filterKeys"
                } else {
                    "remoteConfig.filterFlags"
                },
                if on { "Filter keys" } else { "Filter flags" },
            ));
            for (_, widget, _) in flag_rows.borrow().iter() {
                widget.set_visible(!on);
            }
        });
    }
    if json_toggle.is_active() {
        for (_, widget, _) in flag_rows.borrow().iter() {
            widget.set_visible(false);
        }
    }
    {
        let flag_rows = flag_rows.clone();
        let json_toggle = json_toggle.clone();
        let editor = editor.clone();
        search.connect_changed(move |entry| {
            let query = entry.text().to_string();
            if json_toggle.is_active() {
                editor.highlight_search(&query);
                return;
            }
            for (field, row, _) in flag_rows.borrow().iter() {
                let name = row.title();
                let help = row.help();
                row.set_visible(crate::config_search::matches_config_search(
                    &name, &help, field, &query,
                ));
            }
        });
    }

    let json_holder = adw::ActionRow::new();
    json_holder.set_title(&ctx.t_or("remoteConfig.jsonDocument", "JSON document"));
    json_holder.set_activatable(false);
    json_holder.set_child(Some(&editor.root));
    json_holder.set_visible(json_toggle.is_active());
    {
        let json_holder = json_holder.clone();
        json_toggle.connect_active_notify(move |row| {
            json_holder.set_visible(row.is_active());
        });
    }
    flags_group.add(&json_holder);

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
        let editor = editor.clone();
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
            editor.set_value(&value);
            for (field, row, _) in flag_rows.borrow().iter() {
                if let Some(v) = value.get(field) {
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
        let json_toggle = json_toggle.clone();
        let editor = editor.clone();
        move || {
            let name = selected.borrow().clone();
            if name.is_empty() {
                return;
            }
            let mut obj = Map::new();
            if json_toggle.is_active() {
                match editor.parsed() {
                    Ok(map) => obj = map,
                    Err(err) => {
                        dialogs::toast_near(&editor.view, &err);
                        return;
                    }
                }
            } else {
                if let Some((row, field, msg)) =
                    dialogs::first_invalid_flag(flag_rows.borrow().iter().map(
                        |(field, row, type_name)| (field.clone(), row.clone(), type_name.clone()),
                    ))
                {
                    dialogs::toast_near(&row.widget(), &format!("{field}: {msg}"));
                    return;
                }
                for (field, row, type_name) in flag_rows.borrow().iter() {
                    let text = row.text().to_string();
                    if !text.is_empty() {
                        obj.insert(field.clone(), parse_flag_value(type_name, &text));
                    }
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
    combo.set_title(&ctx.t_or("modals.remoteConfig.profile.label", "Profile"));
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
    let ctx_for_prompt = ctx.clone();
    dialogs::prompt(
        parent,
        &ctx_for_prompt,
        &title,
        &label,
        "default",
        move |name| {
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
        },
    );
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
                    &ctx,
                    &ctx.t_or("modals.remoteConfig.cloneProfile", "Clone profile"),
                    &ctx.t_or("modals.remoteConfig.newName", "New name"),
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
                "rename" => {
                    let snap = ctx.snapshot.borrow();
                    let usage = crate::jobs::profile_usage(
                        &snap.jobs,
                        &snap.mounts,
                        &snap.serves,
                        &remote,
                        &current,
                        op,
                        &ctx.remote_cfg_alias(&remote),
                    );
                    drop(snap);
                    if crate::jobs::profile_rename_blocked(op, &usage) {
                        let alert = adw::AlertDialog::new(
                            Some(&ctx.t_or(
                                "modals.remoteConfig.profile.inUseWarning",
                                "Profile is in use",
                            )),
                            Some(&usage.summary()),
                        );
                        alert.add_response("ok", &ctx.t_or("common.ok", "OK"));
                        alert.present(Some(&parent));
                        return;
                    }
                    dialogs::prompt(
                        &parent,
                        &ctx,
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
                                    crate::jobs::rename_serves_profile(
                                        &mut ctx.snapshot.borrow_mut().serves,
                                        &remote,
                                        &current,
                                        &name,
                                    );
                                    crate::jobs::rename_mounts_profile(
                                        &mut ctx.snapshot.borrow_mut().mounts,
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
                    );
                }
                "delete" => {
                    let snap = ctx.snapshot.borrow();
                    let usage = crate::jobs::profile_usage(
                        &snap.jobs,
                        &snap.mounts,
                        &snap.serves,
                        &remote,
                        &current,
                        op,
                        &ctx.remote_cfg_alias(&remote),
                    );
                    drop(snap);
                    if usage.blocked() {
                        let alert = adw::AlertDialog::new(
                            Some(&ctx.t_or(
                                "modals.remoteConfig.profile.inUseWarning",
                                "Profile is in use",
                            )),
                            Some(&usage.summary()),
                        );
                        alert.add_response("ok", &ctx.t_or("common.ok", "OK"));
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
    apply_profile_action_state(
        &ctx,
        remote,
        op,
        &selected.borrow(),
        &switcher.rename,
        &switcher.delete,
    );
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let names = switcher.names.clone();
        let rename = switcher.rename.clone();
        let delete = switcher.delete.clone();
        switcher.combo.connect_selected_notify(move |combo| {
            let current = names
                .borrow()
                .get(combo.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "default".into());
            apply_profile_action_state(&ctx, &remote, op, &current, &rename, &delete);
        });
    }
}

fn apply_profile_action_state(
    ctx: &AppCtx,
    remote: &str,
    op: Option<OperationType>,
    profile: &str,
    rename: &gtk::Button,
    delete: &gtk::Button,
) {
    let snap = ctx.snapshot.borrow();
    let usage = crate::jobs::profile_usage(
        &snap.jobs,
        &snap.mounts,
        &snap.serves,
        remote,
        profile,
        op,
        &ctx.remote_cfg_alias(remote),
    );
    drop(snap);
    let in_use = ctx.tf_or(
        "modals.remoteConfig.profile.disabledReason.inUse",
        "Profile is in use by a running {{operation}}. Stop it first.",
        &[(
            "operation",
            &match op {
                Some(op) if op.is_sync_type() => format!("{} job", op.as_str()),
                Some(op) => op.as_str().to_string(),
                None => "profile".into(),
            },
        )],
    );
    if crate::jobs::profile_rename_blocked(op, &usage) {
        rename.set_sensitive(false);
        rename.set_tooltip_text(Some(&in_use));
    } else {
        rename.set_sensitive(true);
        rename.set_tooltip_text(Some(
            &ctx.t_or("modals.remoteConfig.renameProfile", "Rename profile"),
        ));
    }
    if usage.blocked() {
        delete.set_sensitive(false);
        delete.set_tooltip_text(Some(&in_use));
    } else {
        delete.set_sensitive(true);
        delete.set_tooltip_text(Some(
            &ctx.t_or("modals.remoteConfig.deleteProfile", "Delete profile"),
        ));
    }
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

fn extra_source_row(ctx: &AppCtx, value: &str, pick_folders: bool, remote: &str) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&ctx.t_or("remoteConfig.additionalSource", "Additional source"));
    row.set_text(value);
    if pick_folders {
        dialogs::attach_path_picker(
            ctx,
            &row,
            crate::picker::FilePickerConfig::folders().with_remote(remote),
        );
    }
    row
}

fn filename_row(ctx: &AppCtx, value: &str) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&ctx.t_or(
        "wizards.appOperation.copyUrlFilename",
        "Filename (optional)",
    ));
    row.set_text(value);
    row
}

pub(super) fn runtime_flags_for_type(ctx: &AppCtx, remote_type: &str) -> Vec<FlagOption> {
    if remote_type.is_empty() {
        return Vec::new();
    }
    let Some(client) = ctx.client() else {
        return Vec::new();
    };
    let Ok(value) = client.providers() else {
        return Vec::new();
    };
    crate::providers::parse_providers(&value)
        .into_iter()
        .find(|provider| {
            provider.prefix.eq_ignore_ascii_case(remote_type)
                || provider.name.eq_ignore_ascii_case(remote_type)
        })
        .map(|provider| {
            provider
                .options
                .iter()
                .map(FlagOption::from_provider)
                .collect()
        })
        .unwrap_or_default()
}

fn runtime_provider_flags(ctx: &AppCtx, remote: &str) -> Vec<FlagOption> {
    let remote_type = ctx
        .snapshot
        .borrow()
        .remotes
        .iter()
        .find(|item| item.name == remote)
        .map(|item| item.r#type.clone())
        .unwrap_or_default();
    runtime_flags_for_type(ctx, &remote_type)
}

fn flag_entry(ctx: &AppCtx, flag: &FlagOption, current: &Value) -> FlagWidget {
    FlagWidget::from_flag(ctx, flag, current)
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

fn action_summary(ctx: &AppCtx, ids: &[String]) -> String {
    if ids.is_empty() {
        ctx.t_or(
            "remoteConfig.defaultActionOrder",
            "All actions (default order)",
        )
    } else {
        ids.join(" · ")
    }
}

fn start_remote_reauth(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    panel: &InteractivePanel,
) {
    let Some(client) = ctx.client() else {
        show_reauth_error(
            parent,
            &ctx,
            &ctx.t_or(
                "notification.title.engineConnectionFailed",
                "Engine Connection Error",
            ),
        );
        return;
    };
    let dump = match client.dump_config() {
        Ok(value) => value,
        Err(e) => {
            show_reauth_error(parent, &ctx, &e.to_string());
            return;
        }
    };
    let Some((r#type, params)) = crate::providers::interactive_remote_params(&dump, remote) else {
        show_reauth_error(
            parent,
            &ctx,
            &ctx.t_or(
                "modals.remoteConfig.errors.interactiveProcessingFailed",
                "Could not start interactive configuration",
            ),
        );
        return;
    };
    let opt = Some(crate::command_options::build_opt(
        &crate::command_options::sync_non_interactive(
            &crate::command_options::initial_command_options(),
            true,
        ),
    ));
    match client.create_remote_interactive(remote, &r#type, params, opt) {
        Ok(value) => apply_reauth_response(&ctx, panel, &value),
        Err(e) => show_reauth_error(parent, &ctx, &e.to_string()),
    }
}

fn continue_remote_reauth(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    panel: &InteractivePanel,
) {
    let flow = panel.flow.borrow().clone();
    if crate::interactive::is_continue_disabled(&flow) {
        return;
    }
    let Some(client) = ctx.client() else {
        show_reauth_error(
            parent,
            &ctx,
            &ctx.t_or(
                "notification.title.engineConnectionFailed",
                "Engine Connection Error",
            ),
        );
        return;
    };
    let dump = client.dump_config().unwrap_or(serde_json::json!({}));
    let params = crate::providers::interactive_remote_params(&dump, remote)
        .map(|(_, params)| params)
        .unwrap_or(serde_json::json!({}));
    let option_type = flow
        .question
        .as_ref()
        .and_then(|q| q.option.as_ref())
        .map(|o| o.type_name.as_str())
        .unwrap_or("string");
    let answer = panel.current_answer();
    let token = flow
        .question
        .as_ref()
        .map(|q| q.state.clone())
        .unwrap_or_default();
    let opt = Some(crate::command_options::build_opt(
        &crate::command_options::sync_non_interactive(
            &crate::command_options::initial_command_options(),
            true,
        ),
    ));
    match client.continue_create_remote(
        remote,
        &token,
        answer.as_rc_result(option_type),
        params,
        opt,
    ) {
        Ok(value) => apply_reauth_response(&ctx, panel, &value),
        Err(e) => show_reauth_error(parent, &ctx, &e.to_string()),
    }
}

fn apply_reauth_response(ctx: &AppCtx, panel: &InteractivePanel, value: &serde_json::Value) {
    let next = panel.apply_response(ctx, value);
    if let Some(url) = super::interactive::poll_oauth_url(ctx) {
        let _ = open::that(&url);
        panel.oauth.set_url(ctx, Some(&url));
    }
    if !next.is_active {
        panel.root.set_visible(true);
        panel.oauth.set_status(&ctx.t_or(
            "wizards.remoteConfig.readyToContinue",
            "Authorization complete",
        ));
    }
}

fn show_reauth_error(parent: &impl IsA<gtk::Widget>, ctx: &AppCtx, detail: &str) {
    let err = adw::AlertDialog::new(
        Some(&ctx.t_or(
            "modals.remoteConfig.errors.interactiveProcessingFailed",
            "Interactive configuration failed",
        )),
        Some(detail),
    );
    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
    err.present(Some(parent));
}

fn sensitive_fields_for(ctx: &AppCtx, remote: &str) -> Vec<(String, String)> {
    let Some(client) = ctx.client() else {
        return Vec::new();
    };
    let dump = client.dump_config().unwrap_or(json!({}));
    let Some(params) = crate::providers::dump_remote_params(&dump, remote) else {
        return Vec::new();
    };
    let Some(type_name) = crate::providers::dump_provider_type(&params) else {
        return Vec::new();
    };
    let providers = client
        .providers()
        .ok()
        .map(|value| crate::providers::parse_providers(&value))
        .unwrap_or_default();
    crate::providers::sensitive_field_labels(&providers, &type_name)
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
