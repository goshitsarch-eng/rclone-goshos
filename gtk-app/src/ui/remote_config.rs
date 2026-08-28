//! Angular-style remote configuration sidenav: per-operation profiles,
//! helper configs (VFS / filter / backend / runtime), and remote metadata.

use super::dialogs;
use super::flag_widget::{FlagRow, FlagWidget, ServeFlagRow};
use super::interactive::InteractivePanel;
use super::AppCtx;
use crate::cli_import::CliImportApply;
use crate::config_steps::{
    edit_profile_names, editor_steps, is_sensitive_flag, navigate_to_shared, parse_open_step,
    return_from_shared, shared_sidebar_types, show_shared_sidebar, toggle_slide_panel, EditorStep,
    SlidePanel, REMOTE_EDIT_SECTIONS,
};
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
    search.set_placeholder_text(Some(&ctx.t_or("shared.search.placeholder", "Search pages")));
    side_col.append(&search);
    side_col.append(&side_scroll);
    let slide = Rc::new(Cell::new(SlidePanel::Hidden));
    let obscure_fields = Rc::new(RefCell::new(sensitive_fields_for(&ctx, &remote)));
    let cli_apply: Rc<RefCell<Box<dyn Fn(CliImportApply)>>> =
        Rc::new(RefCell::new(Box::new(|_| {})));
    let obscure_apply: Rc<RefCell<Box<dyn Fn(&str, &str)>>> = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        Rc::new(RefCell::new(Box::new(move |key: &str, value: &str| {
            apply_obscured_remote(&ctx, &remote, key, value);
        }) as Box<dyn Fn(&str, &str)>))
    };
    let cli_btn = gtk::ToggleButton::new();
    cli_btn.set_label(&ctx.t_or("wizards.cliImport.title", "Import from CLI"));
    cli_btn.set_widget_name("slide-cli-import");
    cli_btn.set_tooltip_text(Some(&ctx.t_or(
        "wizards.cliImport.description",
        "Paste an rclone command, preview mapped flags, then apply them.",
    )));
    let obscure_btn = gtk::ToggleButton::new();
    obscure_btn.set_label(&ctx.t_or("wizards.obscure.title", "Obscure Password"));
    obscure_btn.set_widget_name("slide-obscure");
    obscure_btn.set_tooltip_text(Some(&ctx.t_or(
        "wizards.obscure.description",
        "Obscure a password with rclone and apply it to a field.",
    )));
    let footer = gtk::Box::new(gtk::Orientation::Vertical, 4);
    footer.set_margin_start(8);
    footer.set_margin_end(8);
    footer.append(&cli_btn);
    footer.append(&obscure_btn);
    side_col.append(&footer);
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
    let page_search_btn = gtk::ToggleButton::new();
    page_search_btn.set_icon_name("system-search-symbolic");
    page_search_btn.set_tooltip_text(Some(&ctx.t_or("shared.search.toggle", "Search")));
    title_row.append(&page_search_btn);
    let page_search = gtk::SearchEntry::new();
    page_search.set_placeholder_text(Some(
        &ctx.t_or("shared.search.placeholder", "Search this page"),
    ));
    page_search.set_hexpand(true);
    let page_search_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    page_search_bar.set_margin_start(12);
    page_search_bar.set_margin_end(12);
    page_search_bar.append(&page_search);
    page_search_bar.set_visible(false);
    let page_query = Rc::new(RefCell::new(String::new()));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_hexpand(true);
    body.set_vexpand(true);
    let body_scroll = gtk::ScrolledWindow::new();
    body_scroll.set_vexpand(true);
    body_scroll.set_child(Some(&body));
    let overlay_stack = gtk::Stack::new();
    overlay_stack.set_widget_name("slide-overlay");
    overlay_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let cli_options = Rc::new(RefCell::new(dialogs::CliImportOptions {
        preferred: None,
        remote_type: remote_type_of(&ctx, &remote),
        is_quick_run: false,
        can_create_new: true,
        can_patch: true,
        existing_profiles: Vec::new(),
        initial_cli: String::new(),
    }));
    let hide_slide: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let open_cli: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let remote_obscure = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        Rc::new(move |key: &str, value: &str| {
            apply_obscured_remote(&ctx, &remote, key, value);
        }) as Rc<dyn Fn(&str, &str)>
    };
    let overlay_hooks = OverlayHooks {
        cli_apply: cli_apply.clone(),
        obscure_fields: obscure_fields.clone(),
        obscure_apply: obscure_apply.clone(),
        cli_options: cli_options.clone(),
        open_cli: open_cli.clone(),
        remote_obscure: remote_obscure.clone(),
    };
    let cli_panel = dialogs::cli_import_widget(
        ctx.clone(),
        cli_options.clone(),
        Rc::new({
            let cli_apply = cli_apply.clone();
            let hide_slide = hide_slide.clone();
            move |apply| {
                cli_apply.borrow()(apply);
                hide_slide.borrow()();
            }
        }),
    );
    let cli_scroll = gtk::ScrolledWindow::new();
    cli_scroll.set_min_content_height(220);
    cli_scroll.set_max_content_height(380);
    cli_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    cli_scroll.set_child(Some(&cli_panel));
    overlay_stack.add_named(&cli_scroll, Some("cli"));
    let obscure_tool = dialogs::obscure_tool(
        &ctx,
        obscure_fields.clone(),
        Rc::new({
            let obscure_apply = obscure_apply.clone();
            move |key, value| obscure_apply.borrow()(key, value)
        }),
    );
    let obscure_page = obscure_tool.panel(&ctx);
    overlay_stack.add_named(&obscure_page, Some("obscure"));
    let overlay_reveal = gtk::Revealer::new();
    overlay_reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    overlay_reveal.set_reveal_child(false);
    overlay_reveal.set_child(Some(&overlay_stack));
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
    content.append(&page_search_bar);
    content.append(&overlay_reveal);
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
    let edit_stack: Rc<RefCell<Vec<EditorStep>>> = Rc::new(RefCell::new(Vec::new()));
    let persist_step: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let preferred_profile = Rc::new(RefCell::new(open.profile.clone()));
    let auto_add = Rc::new(Cell::new(open.auto_add));
    let initial_step = parse_open_step(open.initial.as_deref());
    *current.borrow_mut() = initial_step;
    {
        let sidebar = sidebar.clone();
        search.connect_search_changed(move |entry| {
            filter_sidebar_rows(&sidebar, &entry.text());
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
        let overlay_hooks = overlay_hooks.clone();
        Rc::new(move || {
            persist_step.borrow()();
            while let Some(child) = body.first_child() {
                body.remove(&child);
            }
            let step = *current.borrow();
            title.set_text(&step_label(&ctx, step));
            refresh_cli_options(&ctx, &remote, step, &overlay_hooks.cli_options);
            *overlay_hooks.obscure_fields.borrow_mut() = sensitive_fields_for(&ctx, &remote);
            let remote_obscure = overlay_hooks.remote_obscure.clone();
            *overlay_hooks.obscure_apply.borrow_mut() =
                Box::new(move |key, value| remote_obscure(key, value));
            *overlay_hooks.cli_apply.borrow_mut() = Box::new(|_| {});
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
                        Some(&overlay_hooks),
                    );
                    body.append(&page);
                    *persist_step.borrow_mut() = Box::new(saver);
                }
                EditorStep::Helper(kind) => {
                    let profile = preferred_profile.borrow().clone();
                    let (page, saver) = helper_page(
                        &parent,
                        ctx.clone(),
                        &remote,
                        kind,
                        flag_blocks.as_ref(),
                        profile.as_deref(),
                        Some(&overlay_hooks),
                    );
                    body.append(&page);
                    *persist_step.borrow_mut() = Box::new(saver);
                }
                EditorStep::QuickOps => {}
            }
        }) as Rc<dyn Fn()>
    };
    let rebuild = {
        let inner = rebuild;
        let body = body.clone();
        let page_query = page_query.clone();
        let ctx = ctx.clone();
        let remote = remote.clone();
        let sidebar = sidebar.clone();
        let current = current.clone();
        let edit_stack = edit_stack.clone();
        let preferred_profile = preferred_profile.clone();
        let obscure_tool = obscure_tool.clone();
        Rc::new(move || {
            inner();
            apply_page_search(&body, &page_query.borrow());
            obscure_tool.refresh_targets();
            fill_edit_sidebar(
                &ctx,
                &remote,
                &sidebar,
                *current.borrow(),
                &edit_stack.borrow(),
                preferred_profile.borrow().as_deref(),
            );
        }) as Rc<dyn Fn()>
    };
    {
        let page_query = page_query.clone();
        let body = body.clone();
        let page_search_bar = page_search_bar.clone();
        let page_search_btn = page_search_btn.clone();
        page_search.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            *page_query.borrow_mut() = query.clone();
            apply_page_search(&body, &query);
            let open = page_search_bar.is_visible();
            page_search_btn.set_active(open);
        });
    }
    {
        let page_search_bar = page_search_bar.clone();
        let page_search = page_search.clone();
        page_search_btn.connect_toggled(move |btn| {
            let show = btn.is_active();
            page_search_bar.set_visible(show);
            if show {
                page_search.grab_focus();
            } else {
                page_search.set_text("");
            }
        });
    }
    {
        let page_search_btn = page_search_btn.clone();
        let page_search = page_search.clone();
        let slide = slide.clone();
        let hide_slide = hide_slide.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, keyval, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                || modifier.contains(gtk::gdk::ModifierType::SUPER_MASK);
            if ctrl && keyval == gtk::gdk::Key::f {
                page_search_btn.set_active(true);
                page_search.grab_focus();
                return glib::Propagation::Stop;
            }
            if keyval == gtk::gdk::Key::Escape && slide.get() != SlidePanel::Hidden {
                hide_slide.borrow()();
                return glib::Propagation::Stop;
            }
            if keyval == gtk::gdk::Key::Escape && page_search_btn.is_active() {
                page_search_btn.set_active(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        split.add_controller(keys);
    }
    let syncing_slide = Rc::new(Cell::new(false));
    let apply_slide = {
        let slide = slide.clone();
        let overlay_reveal = overlay_reveal.clone();
        let overlay_stack = overlay_stack.clone();
        let cli_btn = cli_btn.clone();
        let obscure_btn = obscure_btn.clone();
        let obscure_tool = obscure_tool.clone();
        let syncing_slide = syncing_slide.clone();
        Rc::new(move |next: SlidePanel| {
            syncing_slide.set(true);
            show_slide_panel(
                next,
                &slide,
                &overlay_reveal,
                &overlay_stack,
                &cli_btn,
                &obscure_btn,
                &obscure_tool,
            );
            syncing_slide.set(false);
        }) as Rc<dyn Fn(SlidePanel)>
    };
    *hide_slide.borrow_mut() = Box::new({
        let apply_slide = apply_slide.clone();
        move || apply_slide(SlidePanel::Hidden)
    });
    *open_cli.borrow_mut() = Box::new({
        let apply_slide = apply_slide.clone();
        move || apply_slide(SlidePanel::CliImport)
    });
    {
        let slide = slide.clone();
        let apply_slide = apply_slide.clone();
        let syncing_slide = syncing_slide.clone();
        cli_btn.connect_toggled(move |_| {
            if syncing_slide.get() {
                return;
            }
            apply_slide(toggle_slide_panel(slide.get(), SlidePanel::CliImport));
        });
    }
    {
        let slide = slide.clone();
        let apply_slide = apply_slide.clone();
        let syncing_slide = syncing_slide.clone();
        obscure_btn.connect_toggled(move |_| {
            if syncing_slide.get() {
                return;
            }
            apply_slide(toggle_slide_panel(slide.get(), SlidePanel::Obscure));
        });
    }
    {
        let apply_slide = apply_slide.clone();
        let slide = slide.clone();
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk::gdk::Key::Escape && slide.get() != SlidePanel::Hidden {
                apply_slide(SlidePanel::Hidden);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        overlay_reveal.add_controller(keys);
    }
    side_col.append(&preset_bar(
        parent,
        ctx.clone(),
        remote.clone(),
        rebuild.clone(),
    ));
    rebuild();

    {
        let current = current.clone();
        let edit_stack = edit_stack.clone();
        let preferred_profile = preferred_profile.clone();
        let rebuild = rebuild.clone();
        let body = body.clone();
        let body_scroll = body_scroll.clone();
        sidebar.connect_row_activated(move |_, row| {
            handle_sidebar_nav(
                row,
                &current,
                &edit_stack,
                &preferred_profile,
                &rebuild,
                &body_scroll,
                &body,
            );
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

#[derive(Clone)]
struct OverlayHooks {
    cli_apply: Rc<RefCell<Box<dyn Fn(CliImportApply)>>>,
    obscure_fields: Rc<RefCell<Vec<(String, String)>>>,
    obscure_apply: Rc<RefCell<Box<dyn Fn(&str, &str)>>>,
    cli_options: Rc<RefCell<dialogs::CliImportOptions>>,
    open_cli: Rc<RefCell<Box<dyn Fn()>>>,
    remote_obscure: Rc<dyn Fn(&str, &str)>,
}

fn show_slide_panel(
    next: SlidePanel,
    slide: &Rc<Cell<SlidePanel>>,
    reveal: &gtk::Revealer,
    stack: &gtk::Stack,
    cli_btn: &gtk::ToggleButton,
    obscure_btn: &gtk::ToggleButton,
    obscure_tool: &dialogs::ObscureTool,
) {
    slide.set(next);
    reveal.set_reveal_child(next != SlidePanel::Hidden);
    match next {
        SlidePanel::CliImport => {
            stack.set_visible_child_name("cli");
        }
        SlidePanel::Obscure => {
            stack.set_visible_child_name("obscure");
            obscure_tool.refresh_targets();
        }
        SlidePanel::Hidden => {}
    }
    cli_btn.set_active(next == SlidePanel::CliImport);
    obscure_btn.set_active(next == SlidePanel::Obscure);
}

fn apply_obscured_remote(ctx: &AppCtx, remote: &str, key: &str, value: &str) {
    if let Some(client) = ctx.client() {
        let mut params = serde_json::Map::new();
        params.insert(key.to_string(), json!(value));
        if client
            .update_remote(remote, Value::Object(params), None)
            .is_ok()
        {
            ctx.refresh_runtime();
        }
    }
}

fn refresh_cli_options(
    ctx: &AppCtx,
    remote: &str,
    step: EditorStep,
    options: &Rc<RefCell<dialogs::CliImportOptions>>,
) {
    let names = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .map(|meta| edit_profile_names(meta, step))
        .unwrap_or_default();
    let mut opts = options.borrow_mut();
    opts.preferred = match step {
        EditorStep::Op(op) => Some(op.as_str().to_string()),
        EditorStep::Helper(kind) => Some(kind.to_string()),
        _ => None,
    };
    opts.remote_type = remote_type_of(ctx, remote);
    opts.existing_profiles = names;
    opts.can_create_new = !matches!(step, EditorStep::Remote | EditorStep::QuickOps);
    opts.can_patch = true;
}

fn apply_cli_to_operation(
    ctx: &AppCtx,
    remote: &str,
    op: OperationType,
    apply: &CliImportApply,
    names: &Rc<RefCell<Vec<String>>>,
    combo: &adw::ComboRow,
    selected: &Rc<RefCell<String>>,
    flags_group: &adw::PreferencesGroup,
    flag_rows: &Rc<RefCell<Vec<FlagRow>>>,
    src: &adw::EntryRow,
    dst: &adw::EntryRow,
    serve: &adw::ComboRow,
    serve_types: &[String],
) {
    match apply.profile_mode {
        crate::cli_import::ProfileMode::New if !apply.profile_name.is_empty() => {
            mutate_profiles(ctx, remote, Some(op), None, |meta| {
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
            refresh_combo(combo, &names.borrow());
            if let Some(idx) = names.borrow().iter().position(|n| n == &apply.profile_name) {
                combo.set_selected(idx as u32);
            }
            *selected.borrow_mut() = apply.profile_name.clone();
        }
        crate::cli_import::ProfileMode::Override if !apply.profile_name.is_empty() => {
            if let Some(idx) = names.borrow().iter().position(|n| n == &apply.profile_name) {
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
    let serve_types = serve_types.to_vec();
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

fn sensitive_from_flags(rows: &[(String, FlagWidget, String)]) -> Vec<(String, String)> {
    rows.iter()
        .filter(|(name, _, ty)| is_sensitive_flag(name, ty))
        .map(|(name, row, _)| (name.clone(), row.title()))
        .collect()
}

fn bind_flag_obscure(hooks: &OverlayHooks, flag_rows: &Rc<RefCell<Vec<FlagRow>>>) {
    let fields = sensitive_from_flags(&flag_rows.borrow());
    if fields.is_empty() {
        *hooks.obscure_fields.borrow_mut() = Vec::new();
    } else {
        *hooks.obscure_fields.borrow_mut() = fields;
    }
    let rows = flag_rows.clone();
    let fallback = hooks.remote_obscure.clone();
    *hooks.obscure_apply.borrow_mut() = Box::new(move |key, value| {
        let mut found = false;
        for (name, row, _) in rows.borrow().iter() {
            if name == key || name.replace('-', "_") == key.replace('-', "_") {
                row.set_text(value);
                found = true;
            }
        }
        if !found {
            fallback(key, value);
        }
    });
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
    let apply_defaults = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let rebuild = rebuild.clone();
        let parent = parent.clone();
        Rc::new(move || {
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
        }) as Rc<dyn Fn()>
    };
    let apply_template = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let rebuild = rebuild.clone();
        let parent = parent.clone();
        Rc::new(move |template: &crate::store::UserTemplate| {
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
        }) as Rc<dyn Fn(&crate::store::UserTemplate)>
    };
    let on_save = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let parent = parent.clone();
        Rc::new(move |refresh: Rc<dyn Fn()>| {
            dialogs::templates_capture_for_remote_ex(&parent, ctx.clone(), &remote, Some(refresh));
        }) as Rc<dyn Fn(Rc<dyn Fn()>)>
    };
    let bar = dialogs::template_picker_bar(
        parent,
        ctx,
        true,
        apply_defaults,
        apply_template,
        Some(on_save),
    );
    bar.set_margin_bottom(10);
    bar
}

fn step_icon(step: EditorStep) -> &'static str {
    step.icon_name()
}

fn nav_row(title: &str, icon: &str, name: &str, subtitle: Option<&str>) -> adw::ActionRow {
    let row = crate::ui::rows::action_row();
    row.set_title(title);
    row.set_activatable(true);
    row.set_widget_name(name);
    if let Some(subtitle) = subtitle {
        row.set_subtitle(subtitle);
    }
    row.add_prefix(&gtk::Image::from_icon_name(icon));
    row
}

fn header_row(title: &str, name: &str) -> adw::ActionRow {
    let row = crate::ui::rows::action_row();
    row.set_title(title);
    row.set_sensitive(false);
    row.set_activatable(false);
    row.set_widget_name(name);
    row
}

fn fill_edit_sidebar(
    ctx: &AppCtx,
    remote: &str,
    sidebar: &gtk::ListBox,
    current: EditorStep,
    stack: &[EditorStep],
    selected_profile: Option<&str>,
) {
    while let Some(child) = sidebar.first_child() {
        sidebar.remove(&child);
    }
    let meta = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .cloned()
        .unwrap_or_default();
    if current.is_remote() {
        sidebar.append(&header_row(
            &ctx.t_or("modals.remoteConfig.editMode.sections.label", "Sections"),
            "header-sections",
        ));
        for section in REMOTE_EDIT_SECTIONS {
            sidebar.append(&nav_row(
                &ctx.t_or(section.i18n_key, section.fallback),
                section.icon,
                section.id,
                None,
            ));
        }
    } else {
        let type_label = step_label(ctx, current);
        sidebar.append(&header_row(
            &ctx.tf_or(
                "modals.remoteConfig.editMode.profiles",
                &format!("{type_label} profiles"),
                &[("type", &type_label)],
            ),
            "header-profiles",
        ));
        if let Some(prev) = stack.last() {
            sidebar.append(&nav_row(
                &step_label(ctx, *prev),
                "go-previous-symbolic",
                "nav-back",
                None,
            ));
        }
        let names = edit_profile_names(&meta, current);
        let active = selected_profile
            .map(|s| s.to_string())
            .or_else(|| names.first().cloned());
        for name in &names {
            let row = nav_row(name, step_icon(current), &format!("profile:{name}"), None);
            if active.as_deref() == Some(name.as_str()) {
                row.add_css_class("success");
            }
            sidebar.append(&row);
        }
        if show_shared_sidebar(stack) {
            sidebar.append(&header_row(
                &ctx.t_or(
                    "modals.remoteConfig.editMode.sharedProfiles",
                    "Shared profiles",
                ),
                "header-shared",
            ));
            for item in shared_sidebar_types(current) {
                let count = edit_profile_names(&meta, item).len();
                sidebar.append(&nav_row(
                    &step_label(ctx, item),
                    step_icon(item),
                    &format!("shared:{}", item.alias()),
                    Some(&count.to_string()),
                ));
            }
        }
    }
    sidebar.append(&header_row(
        &ctx.t_or("common.all", "All pages"),
        "header-all",
    ));
    for step in editor_steps() {
        sidebar.append(&nav_row(
            &step_label(ctx, step),
            step_icon(step),
            &format!("step:{}", step.alias()),
            None,
        ));
    }
}

fn filter_sidebar_rows(sidebar: &gtk::ListBox, query: &str) {
    let mut child = sidebar.first_child();
    while let Some(row) = child {
        let next = row.next_sibling();
        let name = row.widget_name();
        let title = row
            .downcast_ref::<adw::ActionRow>()
            .map(|r| r.title().to_string())
            .unwrap_or_default();
        let visible = name.starts_with("header-")
            || crate::pref_search::any_field_matches(&[&title, &name], query);
        row.set_visible(visible);
        child = next;
    }
}

fn handle_sidebar_nav(
    row: &gtk::ListBoxRow,
    current: &Rc<RefCell<EditorStep>>,
    edit_stack: &Rc<RefCell<Vec<EditorStep>>>,
    preferred_profile: &Rc<RefCell<Option<String>>>,
    rebuild: &Rc<dyn Fn()>,
    body_scroll: &gtk::ScrolledWindow,
    body: &gtk::Box,
) {
    let name = row.widget_name().to_string();
    if name.starts_with("header-") {
        return;
    }
    if name == "nav-back" {
        // The `RefMut` would live for the whole `if let`, and `rebuild()` reads
        // `edit_stack` — end the mutable borrow at this statement instead.
        let previous = return_from_shared(&mut edit_stack.borrow_mut());
        if let Some(prev) = previous {
            *current.borrow_mut() = prev;
            preferred_profile.borrow_mut().take();
            rebuild();
        }
        return;
    }
    if name.starts_with("section-") {
        scroll_to_named(body_scroll, body, &name);
        return;
    }
    if let Some(profile) = name.strip_prefix("profile:") {
        *preferred_profile.borrow_mut() = Some(profile.to_string());
        rebuild();
        return;
    }
    if let Some(alias) = name.strip_prefix("shared:") {
        let next = parse_open_step(Some(alias));
        let curr = *current.borrow();
        *current.borrow_mut() = navigate_to_shared(&mut edit_stack.borrow_mut(), curr, next);
        preferred_profile.borrow_mut().take();
        rebuild();
        return;
    }
    if let Some(alias) = name.strip_prefix("step:") {
        *current.borrow_mut() = parse_open_step(Some(alias));
        edit_stack.borrow_mut().clear();
        preferred_profile.borrow_mut().take();
        rebuild();
    }
}

fn scroll_to_named(scroll: &gtk::ScrolledWindow, root: &impl IsA<gtk::Widget>, name: &str) {
    if let Some(target) = find_named(root.upcast_ref(), name) {
        if let Some((_, y)) = target.translate_coordinates(root, 0.0, 0.0) {
            let adj = scroll.vadjustment();
            let value = y
                .max(adj.lower())
                .min((adj.upper() - adj.page_size()).max(0.0));
            adj.set_value(value);
        }
    }
}

fn find_named(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if widget.widget_name() == name {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        if let Some(found) = find_named(&next, name) {
            return Some(found);
        }
        child = next.next_sibling();
    }
    None
}

fn apply_page_search(root: &impl IsA<gtk::Widget>, query: &str) {
    apply_page_search_widget(root.upcast_ref(), query);
}

fn apply_page_search_widget(widget: &gtk::Widget, query: &str) {
    if let Ok(row) = widget.clone().downcast::<adw::PreferencesRow>() {
        let title = row.title();
        let subtitle = row
            .downcast_ref::<adw::ActionRow>()
            .and_then(|row| row.subtitle().map(|s| s.to_string()))
            .or_else(|| {
                row.downcast_ref::<adw::ExpanderRow>()
                    .map(|row| row.subtitle().to_string())
            })
            .unwrap_or_default();
        let tooltip = row.tooltip_text().unwrap_or_default();
        row.set_visible(crate::config_search::page_field_visible(
            &title, &subtitle, &tooltip, query,
        ));
        return;
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        apply_page_search_widget(&next, query);
        child = next.next_sibling();
    }
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
    let tray = crate::ui::rows::switch_row();
    tray.set_title(&ctx.t_or("remoteConfig.showOnTray", "Show on tray"));
    tray.set_active(meta.show_on_tray);
    let hidden = crate::ui::rows::switch_row();
    hidden.set_title(&ctx.t_or("remoteConfig.hideFromSidebar", "Hide from sidebar"));
    hidden.set_active(meta.hidden);
    let primary_ids = Rc::new(RefCell::new(meta.primary_actions.clone()));
    let sync_ids = Rc::new(RefCell::new(meta.sync_actions.clone()));
    let primary_row = crate::ui::rows::action_row();
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
    let sync_row = crate::ui::rows::action_row();
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
    group.set_widget_name("section-general");
    group.set_title(&crate::ui::rows::escape(
        ctx.t_or("remoteConfig.metadata", "Remote metadata"),
    ));
    group.add(&tray);
    group.add(&hidden);
    group.add(&primary_row);
    group.add(&sync_row);
    let actions = adw::PreferencesGroup::new();
    actions.set_widget_name("section-auth");
    actions.set_title(&crate::ui::rows::escape(
        ctx.t_or("remoteConfig.provider", "Provider"),
    ));
    let helper_row = crate::ui::rows::action_row();
    helper_row.set_title(&ctx.t_or("remoteConfig.namedHelpers", "Named helper profiles"));
    helper_row.add_suffix(&helpers);
    let auth_row = crate::ui::rows::action_row();
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
    hooks: Option<&OverlayHooks>,
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
    let src = crate::ui::rows::entry_row();
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
    let url_filename = crate::ui::rows::entry_row();
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
    let dst = crate::ui::rows::entry_row();
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
    let mount_usage = crate::ui::rows::action_row();
    mount_usage.set_title(&ctx.t_or("dashboard.appDetail.mountDiskUsage", "Mount point usage"));
    mount_usage.set_visible(op == OperationType::Mount);
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
    let dest_status = crate::ui::rows::action_row();
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
    if op == OperationType::Mount {
        apply_mount_usage(&ctx, &dst, &mount_usage);
        let ctx = ctx.clone();
        let mount_usage = mount_usage.clone();
        dst.connect_changed(move |row| apply_mount_usage(&ctx, row, &mount_usage));
    }

    let serve_types = Rc::new(ctx.serve_types());
    let mount_types = Rc::new(ctx.mount_types());
    let serve = crate::ui::rows::combo_row();
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
    let mount_type = crate::ui::rows::combo_row();
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

    let auto_start = crate::ui::rows::switch_row();
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
    let cron_enabled = crate::ui::rows::switch_row();
    cron_enabled.set_title(&ctx.tf_or(
        "wizards.appOperation.enableScheduled",
        "Enable Scheduled {type}",
        &[("type", &op_label)],
    ));
    cron_enabled.set_active(initial.app.cron_enabled);
    cron_enabled.set_visible(op.is_automatable());
    let cron = crate::ui::rows::entry_row();
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
    let watch_enabled = crate::ui::rows::switch_row();
    watch_enabled.set_title(&ctx.t_or("wizards.appOperation.enableWatch", "Watch local sources"));
    watch_enabled.set_subtitle(&ctx.t_or(
        "wizards.appOperation.watchDescription",
        "Watch local source directories for file modifications and sync changes automatically.",
    ));
    watch_enabled.set_active(initial.app.watch_enabled && watch_supported);
    watch_enabled.set_visible(watch_supported);
    let watch_delay = crate::ui::rows::entry_row();
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
    let watch_changed = crate::ui::rows::switch_row();
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
    identity.set_title(&crate::ui::rows::escape(
        ctx.t_or("wizards.appOperation.sourcePaths", "Paths"),
    ));
    if let Some(kind) = &src_kind {
        identity.add(kind);
    }
    identity.add(&src);
    let src_status = crate::ui::rows::action_row();
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
        let add_row = crate::ui::rows::action_row();
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
    identity.add(&mount_usage);
    identity.add(&dest_status);
    identity.add(&serve);
    identity.add(&mount_type);

    let automation = adw::PreferencesGroup::new();
    automation.set_title(&crate::ui::rows::escape(
        ctx.t_or("remoteConfig.automation", "Automation"),
    ));
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
    helpers.set_title(&crate::ui::rows::escape(
        ctx.t_or("remoteConfig.linkedHelpers", "Linked helper profiles"),
    ));
    helpers.add(&vfs_row);
    helpers.add(&filter_row);
    helpers.add(&backend_row);
    helpers.add(&runtime_row);

    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&crate::ui::rows::escape(
        ctx.t_or("remoteConfig.flags", "Flags"),
    ));
    let search = crate::ui::rows::entry_row();
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
    let json_toggle = crate::ui::rows::switch_row();
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

    let cli_row = crate::ui::rows::action_row();
    cli_row.set_title(&ctx.t_or("remoteConfig.cliImport", "CLI import"));
    cli_row.set_subtitle(&ctx.t_or(
        "wizards.cliImport.description",
        "Paste an rclone command, preview mapped flags, then apply them.",
    ));
    let preview = gtk::Button::with_label(&ctx.t_or("wizards.cliImport.preview", "Preview"));
    {
        let open_cli = hooks.map(|h| h.open_cli.clone());
        preview.connect_clicked(move |_| {
            if let Some(open) = &open_cli {
                open.borrow()();
            }
        });
    }
    if let Some(hooks) = hooks {
        bind_flag_obscure(hooks, &flag_rows);
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let names = switcher.names.clone();
        let combo = switcher.combo.clone();
        let selected = selected.clone();
        let flag_rows = flag_rows.clone();
        let flags_group = flags_group.clone();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        *hooks.cli_apply.borrow_mut() = Box::new(move |apply| {
            apply_cli_to_operation(
                &ctx,
                &remote,
                op,
                &apply,
                &names,
                &combo,
                &selected,
                &flags_group,
                &flag_rows,
                &src,
                &dst,
                &serve,
                &serve_types,
            );
        });
    }
    cli_row.add_suffix(&preview);
    flags_group.add(&cli_row);
    flags_group.add(&json_toggle);
    let json_holder = crate::ui::rows::action_row();
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
    preferred_profile: Option<&str>,
    hooks: Option<&OverlayHooks>,
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
    let selected_name = preferred_profile
        .filter(|name| names.iter().any(|n| n == *name))
        .unwrap_or(&names[0])
        .to_string();
    let selected = Rc::new(RefCell::new(selected_name.clone()));
    let current = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .and_then(|m| m.helper_profile(kind, &selected_name))
        .unwrap_or_else(|| json!({}));
    let switcher = profile_switcher(&ctx, &names, &selected_name);
    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&crate::ui::rows::escape(
        ctx.t_or("remoteConfig.options", "Options"),
    ));
    let search = crate::ui::rows::entry_row();
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
    let json_toggle = crate::ui::rows::switch_row();
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

    let json_holder = crate::ui::rows::action_row();
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
    if let Some(hooks) = hooks {
        bind_flag_obscure(hooks, &flag_rows);
        let flag_rows = flag_rows.clone();
        let flags_group = flags_group.clone();
        *hooks.cli_apply.borrow_mut() = Box::new(move |apply| {
            dialogs::apply_cli_to_form(
                &apply,
                Some(&flags_group),
                &flag_rows,
                None,
                None,
                None,
                &[],
                None,
            );
        });
    }
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
    let combo = crate::ui::rows::combo_row();
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
    group.set_title(&crate::ui::rows::escape(
        ctx.t_or("modals.remoteConfig.profiles", "Profiles"),
    ));
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
    let row = crate::ui::rows::entry_row();
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
    let row = crate::ui::rows::entry_row();
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

fn apply_mount_usage(ctx: &AppCtx, dest: &adw::EntryRow, usage: &adw::ActionRow) {
    let text = dest.text().to_string();
    let hint = crate::media::local_path_field_hint(&text, &ctx.engine_os(), None);
    usage.set_subtitle(hint.as_deref().unwrap_or(""));
    usage.set_visible(!text.trim().is_empty());
    if let Some(hint) = hint {
        dest.set_tooltip_text(Some(&format!("{hint} — {text}")));
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
