use super::dashboard::Dashboard;
use super::dialogs;
use super::flow::FlowView;
use super::nautilus::NautilusView;
use super::onboarding;
use super::AppCtx;
use crate::navigation::{NavTarget, OverlayKind, OverlaySpec};
use crate::operations::{AppTab, MainView};
use crate::rclone::engine::RcloneEngine;
use adw::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

thread_local! {
    static RUNNING: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
}

pub fn activate(app: &adw::Application) {
    if let Some(ctx) = RUNNING.with(|running| running.borrow().clone()) {
        handle_reentry(app, &ctx);
        return;
    }

    let ctx = AppCtx::new();
    ctx.apply_theme();

    let settings = ctx.settings.borrow().clone();
    if settings.core.active_backend.is_empty() || settings.core.active_backend == "local" {
        *ctx.engine.borrow_mut() = Some(RcloneEngine::start(&settings));
    }
    if settings.general.start_on_startup {
        let _ = crate::platform::set_autostart(true);
    }
    ctx.apply_persisted_options();
    ctx.refresh_runtime();

    let args = crate::cli::launch_args();
    if let Some(req) = crate::platform::parse_dialog_args(&args) {
        dialogs::present_standalone(app, ctx, req);
        return;
    }

    if !ctx.settings.borrow().core.completed_onboarding {
        onboarding::present(app, ctx.clone());
        return;
    }

    apply_launch(app, &ctx, &args, true);
    RUNNING.with(|running| *running.borrow_mut() = Some(ctx));
}

fn handle_reentry(app: &adw::Application, ctx: &AppCtx) {
    let args = crate::cli::launch_args();
    if let Some(req) = crate::platform::parse_dialog_args(&args) {
        dialogs::present_standalone(app, ctx.clone(), req);
        return;
    }
    apply_launch(app, ctx, &args, false);
}

fn apply_launch(app: &adw::Application, ctx: &AppCtx, args: &[String], first: bool) {
    if let Some(action) = crate::tray_menu::parse_tray_action_args(args) {
        super::tray::handle(ctx, action);
    }
    if let Some(send) = crate::platform::parse_send_to_args(args) {
        upload_send_to(ctx, &send);
    }
    for path in crate::config_import::take_open_configs() {
        ctx.enqueue_config_import(path);
    }
    for path in crate::config_import::parse_open_config_args(args) {
        ctx.enqueue_config_import(path);
    }
    let standalone_dialogs = ctx.settings.borrow().general.standalone_dialogs;
    let launch = crate::navigation::parse_launch_args(args, standalone_dialogs);
    let overlay_files = launch
        .as_ref()
        .and_then(|l| crate::navigation::launch_overlay_files(args, &l.target));
    if let Some(launch) = &launch {
        if !(launch.standalone && crate::navigation::standalone_skips_main_nav(&launch.target)) {
            ctx.request_nav(launch.target.clone());
        }
        if !launch.standalone {
            if let Some((remote, path)) = overlay_files.clone() {
                *ctx.pending_files_overlay.borrow_mut() = Some((remote, path));
            }
        }
    }
    if !ctx.store.borrow().pending_share_paths.is_empty() && launch.is_none() {
        ctx.request_nav(NavTarget::Files {
            remote: "local".into(),
            path: String::new(),
        });
    }
    if first {
        let hide_main = crate::cli::start_hidden() || launch.as_ref().is_some_and(|l| l.standalone);
        present_main_with(app, ctx.clone(), hide_main);
    } else {
        ctx.request_show();
        for window in app.windows() {
            window.present();
        }
    }
    if let Some(launch) = launch.filter(|l| l.standalone) {
        present_standalone_workspace(app, ctx, &launch.target);
        if let Some((remote, path)) = overlay_files {
            present_files_window(app, ctx, &remote, &path);
        }
    }
}

fn upload_send_to(ctx: &AppCtx, send: &crate::platform::SendToArgs) {
    let Some(client) = ctx.client() else {
        log::error!("rclone engine is not available for Send-to");
        return;
    };
    let dest_fs = crate::rclone::remote_fs(&send.remote, &send.path);
    let (items, dirs) =
        match crate::fileops::collect_local_upload(&send.files, &dest_fs, &send.path) {
            Ok(plan) => plan,
            Err(e) => {
                log::error!("Send-to collect failed: {e}");
                return;
            }
        };
    if items.is_empty() && dirs.is_empty() {
        return;
    }
    for (fs, path) in dirs {
        let _ = client.mkdir(&fs, &path);
    }
    if items.is_empty() {
        return;
    }
    match crate::fileops::start_grouped_transfers(&client, &items, "send-to") {
        Ok((group, ids)) => {
            crate::jobs::remember_grouped(
                &mut ctx.store.borrow_mut().job_meta,
                &ids,
                crate::store::JobMeta {
                    origin: "filemanager".into(),
                    profile: "default".into(),
                    remote: send.remote.clone(),
                    backend: ctx.backend_key(),
                    group,
                    operation: "copy".into(),
                    transfer_snapshot: crate::jobs::transfer_snapshot_from_items(&items),
                    ..Default::default()
                },
            );
            if let Some(id) = ids.first().copied() {
                let bytes: u64 = items
                    .iter()
                    .map(|item| std::fs::metadata(&item.src).map(|m| m.len()).unwrap_or(0))
                    .sum();
                let preparing = crate::jobs::preparing_job(
                    id,
                    &send.remote,
                    &items
                        .first()
                        .map(|item| item.src.clone())
                        .unwrap_or_default(),
                    &send.path,
                    items.len() as u64,
                    bytes,
                );
                ctx.store.borrow_mut().remember_job(preparing);
                ctx.persist();
                ctx.store.borrow_mut().update_job_stats(
                    id,
                    crate::jobs::preparing_progress_stats(
                        0,
                        bytes,
                        0,
                        items.len() as u64,
                        crate::jobs::transfer_snapshot_from_items(&items),
                    ),
                );
            }
        }
        Err(e) => log::error!("Send-to failed: {e}"),
    }
    ctx.request_browse(&send.remote, &send.path);
    ctx.request_show();
    ctx.refresh_runtime();
}

pub fn present_main(app: &adw::Application, ctx: AppCtx) {
    present_main_with(app, ctx, crate::cli::start_hidden());
}

fn present_main_with(app: &adw::Application, ctx: AppCtx, hidden: bool) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&ctx.t_or("overviews.headers.general", "RClone Manager"))
        .default_width(1280)
        .default_height(820)
        .build();

    let toast = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let view_stack = adw::ViewStack::new();
    let dashboard = Dashboard::new(ctx.clone(), toast.clone());
    let nautilus = NautilusView::new(ctx.clone(), toast.clone());
    let flow = FlowView::new(ctx.clone(), toast.clone());

    view_stack.add_titled_with_icon(
        &dashboard.root,
        Some("main_menu"),
        &ctx.t("sidebar.remotes"),
        "view-grid-symbolic",
    );
    view_stack.add_titled_with_icon(
        &nautilus.root,
        Some("nautilus"),
        &ctx.t_or("nautilus.titles.files", "Files"),
        "folder-symbolic",
    );
    view_stack.add_titled_with_icon(
        &flow.root,
        Some("flow"),
        &ctx.t_or("titlebar.menu.flowWorkspace", "Flow"),
        "media-playlist-consecutive-symbolic",
    );

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&view_stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    header.set_title_widget(Some(&switcher));

    let add_btn = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .build();
    let add_menu = gio::Menu::new();
    add_menu.append(
        Some(&ctx.t_or("titlebar.menu.quickRemote", "Quick Add Remote")),
        Some("win.quick-add"),
    );
    add_menu.append(
        Some(&ctx.t_or("titlebar.menu.detailedRemote", "Detailed Remote")),
        Some("win.remote-config"),
    );
    add_menu.append(
        Some(&ctx.t_or("titlebar.menu.quickRun", "New Quick Run")),
        Some("win.quick-run-new"),
    );
    add_menu.append(
        Some(&ctx.t_or("flow.tabs.workflow", "New Workflow")),
        Some("win.view::flow"),
    );
    add_btn.set_menu_model(Some(&add_menu));
    add_btn.set_tooltip_text(Some(
        &ctx.t_or("titlebar.menu.add", "Add remote or quick run"),
    ));
    header.pack_start(&add_btn);

    let conn_btn = gtk::Button::from_icon_name("network-offline-symbolic");
    conn_btn.set_tooltip_text(Some(
        &ctx.t_or("titlebar.internetStatus", "Internet connection status"),
    ));
    conn_btn.add_css_class("flat");
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let conn_btn_sync = conn_btn.clone();
        conn_btn.connect_clicked(move |_| {
            ctx.refresh_connection();
            sync_connection_button(&ctx, &conn_btn_sync);
            let urls = ctx.settings.borrow().core.connection_check_urls.clone();
            let results = crate::connection::check_links(&urls, 2);
            let body = results
                .iter()
                .map(|r| {
                    format!(
                        "{} — {} ({})",
                        r.url,
                        if r.ok { "ok" } else { "fail" },
                        r.detail
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let alert =
                adw::AlertDialog::new(Some(&crate::connection::summarize(&results)), Some(&body));
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&window));
        });
    }
    sync_connection_button(&ctx, &conn_btn);
    header.pack_start(&conn_btn);

    let home_btn = gtk::Button::from_icon_name("go-home-symbolic");
    home_btn.set_tooltip_text(Some(&ctx.t_or("titlebar.home", "Home")));
    home_btn.add_css_class("flat");
    home_btn.set_visible(false);
    {
        let ctx = ctx.clone();
        let dash = dashboard.clone();
        let stack = view_stack.clone();
        home_btn.connect_clicked(move |_| {
            *ctx.selected_remote.borrow_mut() = None;
            stack.set_visible_child_name("main_menu");
            dash.refresh();
        });
    }
    header.pack_start(&home_btn);

    let card_btn = gtk::Button::from_icon_name("view-list-symbolic");
    card_btn.add_css_class("flat");
    card_btn.set_action_name(Some("win.toggle-card-variant"));
    sync_card_variant_button(&ctx, &card_btn);
    header.pack_start(&card_btn);

    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(&ctx.t_or("titlebar.appMenu", "Application menu"))
        .build();
    menu_btn.set_menu_model(Some(&app_menu(&ctx)));
    let menu_badge = gtk::Label::new(None);
    menu_badge.add_css_class("osd");
    menu_badge.add_css_class("numeric");
    menu_badge.set_valign(gtk::Align::Start);
    menu_badge.set_halign(gtk::Align::End);
    menu_badge.set_can_target(false);
    menu_badge.set_visible(false);
    let menu_wrap = gtk::Overlay::new();
    menu_wrap.set_child(Some(&menu_btn));
    menu_wrap.add_overlay(&menu_badge);
    header.pack_end(&menu_wrap);
    sync_menu_badge(&ctx, &menu_badge);

    let detach_btn = gtk::Button::from_icon_name("view-restore-symbolic");
    detach_btn.add_css_class("flat");
    detach_btn.set_tooltip_text(Some(&ctx.t_or(
        "titlebar.detach",
        "Detach the current workspace into a new window",
    )));
    detach_btn.set_action_name(Some("win.detach-workspace"));
    header.pack_end(&detach_btn);

    let close_overlay = gtk::Button::from_icon_name("window-close-symbolic");
    close_overlay.add_css_class("flat");
    close_overlay.set_tooltip_text(Some(&ctx.t_or(
        "titlebar.closeOverlay",
        "Close overlay and return to the base view",
    )));
    close_overlay.set_visible(false);
    close_overlay.set_action_name(Some("win.close-overlay"));
    header.pack_end(&close_overlay);
    {
        let close_overlay = close_overlay.clone();
        let ctx_btn = ctx.clone();
        *ctx.on_overlays_changed.borrow_mut() = Some(Rc::new(move || {
            close_overlay.set_visible(ctx_btn.overlay_count() > 0);
        }));
    }

    let notice_btn = gtk::Button::from_icon_name("dialog-warning-symbolic");
    notice_btn.add_css_class("flat");
    notice_btn.set_visible(false);
    {
        let ctx = ctx.clone();
        let window = window.clone();
        notice_btn.connect_clicked(move |_| {
            if ctx.settings.borrow().runtime.app_restart_required {
                ctx.settings.borrow_mut().runtime.app_restart_required = false;
                ctx.persist();
                if let Err(e) = crate::platform::relaunch() {
                    log::error!("relaunch failed: {e}");
                } else if let Some(app) = window.application() {
                    app.quit();
                }
            } else if ctx.settings.borrow().runtime.rclone_restart_required {
                ctx.settings.borrow_mut().runtime.rclone_restart_required = false;
                ctx.persist();
                ctx.restart_engine();
            } else if ctx.updates.borrow().has_updates() {
                ctx.request_nav(NavTarget::Updates);
            } else {
                ctx.request_nav(NavTarget::Alerts);
            }
        });
    }
    sync_notice_button(&ctx, &notice_btn);
    header.pack_end(&notice_btn);

    let banner = adw::Banner::new("");
    banner.set_button_label(Some(&ctx.t_or("repairSheet.actions.repair", "Repair")));
    let banner_kind = Rc::new(std::cell::RefCell::new(BannerKind::None));
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        let banner_kind = banner_kind.clone();
        let banner_ref = banner.clone();
        banner.connect_button_clicked(move |_| {
            // Read the kind out before matching: the scrutinee's `Ref` would
            // otherwise be held for the whole `match`, and the Flatpak arm
            // calls `update_banner`, which takes `borrow_mut()` on it.
            let kind = *banner_kind.borrow();
            match kind {
                BannerKind::Repair => {
                    let version = ctx.client().and_then(|c| c.version().ok());
                    let issues = crate::repair::diagnose(
                        &ctx.settings.borrow(),
                        ctx.engine_ready(),
                        ctx.client().as_ref(),
                        version.as_deref(),
                    );
                    if crate::repair::banner_opens_password(&issues) {
                        dialogs::password_prompt(&window, ctx.clone(), toast.clone());
                    } else {
                        dialogs::repair(&window, ctx.clone(), toast.clone());
                    }
                }
                BannerKind::Flatpak => {
                    ctx.settings.borrow_mut().runtime.flatpak_warn = false;
                    ctx.persist();
                    update_banner(&ctx, &banner_ref, &banner_kind);
                }
                BannerKind::Update => ctx.request_nav(NavTarget::Updates),
                BannerKind::RcloneRestart => {
                    ctx.settings.borrow_mut().runtime.rclone_restart_required = false;
                    ctx.persist();
                    ctx.restart_engine();
                }
                BannerKind::AppRestart => {
                    ctx.settings.borrow_mut().runtime.app_restart_required = false;
                    ctx.persist();
                    if let Err(e) = crate::platform::relaunch() {
                        ctx.toast_error(&toast, &e);
                    } else if let Some(app) = window.application() {
                        app.quit();
                    }
                }
                BannerKind::Metered | BannerKind::Development | BannerKind::None => {}
            }
        });
    }
    update_banner(&ctx, &banner, &banner_kind);
    let auto_prompted = Rc::new(RefCell::new(HashSet::<crate::repair::RepairKind>::new()));
    maybe_auto_prompt_repair(&ctx, &window, &toast, &auto_prompted);

    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&banner);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view_stack));
    let loading = adw::StatusPage::new();
    loading.add_css_class("startup-loading");
    loading.set_hexpand(true);
    loading.set_vexpand(true);
    loading.set_halign(gtk::Align::Fill);
    loading.set_valign(gtk::Align::Fill);
    loading.set_icon_name(Some("emblem-synchronizing-symbolic"));
    loading.set_title(&ctx.t_or("onboarding.loadingTitle", "Initializing RClone Manager"));
    loading.set_description(Some(&ctx.t_or(
        "onboarding.loadingMessage",
        "Checking system configuration...",
    )));
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    spinner.set_halign(gtk::Align::Center);
    loading.set_child(Some(&spinner));
    apply_startup_css();
    overlay.add_overlay(&loading);
    loading.set_visible(!ctx.engine_ready());
    toolbar.set_content(Some(&overlay));
    toast.set_child(Some(&toolbar));
    window.set_content(Some(&toast));

    install_actions(
        app,
        &window,
        &ctx,
        &toast,
        &view_stack,
        &dashboard,
        &nautilus,
        &flow,
        &banner,
    );
    {
        let ctx = ctx.clone();
        let dash = dashboard.clone();
        let card_btn = card_btn.clone();
        let action = gio::SimpleAction::new("toggle-card-variant", None);
        action.connect_activate(move |_, _| {
            toggle_card_variant(&ctx);
            sync_card_variant_button(&ctx, &card_btn);
            dash.refresh();
        });
        window.add_action(&action);
    }
    install_shortcuts(&window);
    install_debug_context_menu(&window, &ctx);

    let default_view = if ctx.store.borrow().pending_share_paths.is_empty() {
        ctx.settings.borrow().default_view()
    } else {
        MainView::Nautilus
    };
    let restore = ctx.active_workspace.borrow().clone();
    let shown = if !restore.is_empty() && view_stack.child_by_name(&restore).is_some() {
        view_stack.set_visible_child_name(&restore);
        restore
    } else {
        view_stack.set_visible_child_name(default_view.as_str());
        default_view.as_str().to_string()
    };
    *ctx.active_workspace.borrow_mut() = shown;
    {
        let ctx = ctx.clone();
        let stack = view_stack.clone();
        let window = window.clone();
        let reverting = Rc::new(std::cell::Cell::new(false));
        view_stack.connect_visible_child_notify(move |_| {
            let Some(name) = stack.visible_child_name() else {
                return;
            };
            if reverting.get() {
                *ctx.active_workspace.borrow_mut() = name.to_string();
                return;
            }
            let default = ctx.settings.borrow().general.default_view.clone();
            if crate::navigation::workspace_open_plan(&default, name.as_str())
                != crate::navigation::WorkspaceOpen::SwitchStack
            {
                reverting.set(true);
                stack.set_visible_child_name(&default);
                reverting.set(false);
                let _ = open_workspace_target(&ctx, &window, &stack, name.as_str());
                *ctx.active_workspace.borrow_mut() = default;
                return;
            }
            *ctx.active_workspace.borrow_mut() = name.to_string();
        });
    }

    let tray = super::tray::start_or_reuse(&ctx);
    let generation = ctx.ui_generation.get();
    {
        let ctx = ctx.clone();
        let conn_btn = conn_btn.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
            if ctx.ui_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            ctx.refresh_connection();
            sync_connection_button(&ctx, &conn_btn);
            glib::ControlFlow::Break
        });
    }
    {
        let ctx = ctx.clone();
        let banner = banner.clone();
        let banner_kind = banner_kind.clone();
        let window = window.clone();
        let toast = toast.clone();
        let auto_prompted = auto_prompted.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(4), move || {
            if ctx.ui_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            ctx.refresh_updates();
            update_banner(&ctx, &banner, &banner_kind);
            maybe_auto_prompt_repair(&ctx, &window, &toast, &auto_prompted);
            glib::ControlFlow::Break
        });
    }
    let ctx_poll = ctx.clone();
    let dash_poll = dashboard.clone();
    let flow_poll = flow.clone();
    let nautilus_poll = nautilus.clone();
    let banner_poll = banner.clone();
    let banner_kind_poll = banner_kind.clone();
    let conn_btn_poll = conn_btn.clone();
    let home_btn_poll = home_btn.clone();
    let card_btn_poll = card_btn.clone();
    let notice_btn_poll = notice_btn.clone();
    let menu_badge_poll = menu_badge.clone();
    let menu_btn_poll = menu_btn.clone();
    let last_menu_sig = Rc::new(RefCell::new(menu_signature(&ctx)));
    {
        let ctx_nav = ctx.clone();
        let stack_nav = view_stack.clone();
        let nautilus_nav = nautilus.clone();
        let dash_nav = dashboard.clone();
        let flow_nav = flow.clone();
        let window_nav = window.clone();
        let toast_nav = toast.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if ctx_nav.ui_generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            if ctx_nav.take_reload() {
                if let Some(name) = stack_nav.visible_child_name() {
                    *ctx_nav.active_workspace.borrow_mut() = name.to_string();
                }
                let overlays = ctx_nav.snapshot_overlay_specs();
                ctx_nav.close_overlays();
                let hidden = !window_nav.is_visible();
                let app = window_nav
                    .application()
                    .and_then(|app| app.downcast::<adw::Application>().ok());
                ctx_nav.bump_generation();
                ctx_nav.reload_destroy.set(true);
                window_nav.close();
                if let Some(app) = app {
                    present_main_with(&app, ctx_nav.clone(), hidden);
                    for spec in overlays {
                        reopen_overlay(&app, &ctx_nav, spec);
                    }
                }
                return glib::ControlFlow::Break;
            }
            for target in crate::platform::drain_notification_clicks() {
                ctx_nav.request_show();
                match target {
                    crate::platform::NotificationTarget::ShowWindow => {}
                    crate::platform::NotificationTarget::Job(id) => {
                        ctx_nav.request_nav(crate::navigation::NavTarget::Job { id });
                    }
                    crate::platform::NotificationTarget::Alerts => {
                        ctx_nav.request_nav(crate::navigation::NavTarget::Alerts);
                    }
                    crate::platform::NotificationTarget::Repair => {
                        ctx_nav.request_nav(crate::navigation::NavTarget::Repair);
                    }
                }
            }
            for (id, title) in ctx_nav.take_job_toasts() {
                let ctx = ctx_nav.clone();
                ctx_nav.toast_action(
                    &toast_nav,
                    &title,
                    &ctx_nav.t_or("common.viewDetails", "View Details"),
                    move || {
                        ctx.request_show();
                        ctx.request_nav(crate::navigation::NavTarget::Job { id });
                    },
                );
            }
            if ctx_nav.take_show() {
                window_nav.set_visible(true);
                window_nav.present();
            }
            if ctx_nav.take_quit() {
                let window = window_nav.clone();
                dialogs::confirm_shutdown(&window_nav, ctx_nav.clone(), move || {
                    if let Some(app) = window.application() {
                        app.quit();
                    }
                });
            }
            if let Some(target) = ctx_nav.take_nav() {
                apply_nav(
                    &ctx_nav,
                    &stack_nav,
                    &dash_nav,
                    &flow_nav,
                    &window_nav,
                    &toast_nav,
                    target,
                );
            }
            if ctx_nav.pending_picker.borrow().is_some()
                || ctx_nav.pending_browse.borrow().is_some()
            {
                stack_nav.set_visible_child_name("nautilus");
            }
            if let Some((remote, path)) = ctx_nav.pending_browse.borrow_mut().take() {
                let target = files_target(&remote, &path);
                nautilus_nav.navigate_to(&target);
            }
            if let Some((remote, path)) = ctx_nav.take_files_overlay() {
                present_files_at(&window_nav, &ctx_nav, &remote, &path);
            }
            nautilus_nav.apply_pending_picker();
            if let Some(path) = ctx_nav.take_config_import() {
                dialogs::import_rclone_config(
                    &window_nav,
                    ctx_nav.clone(),
                    toast_nav.clone(),
                    path,
                    {
                        let dash = dash_nav.clone();
                        Rc::new(move || dash.refresh())
                    },
                );
            }
            glib::ControlFlow::Continue
        });
    }
    let poll_tick = Rc::new(Cell::new(0u32));
    let poll_window = window.clone();
    let poll_toast = toast.clone();
    let poll_auto_prompted = auto_prompted.clone();
    let loading_poll = loading.clone();
    let first_refresh_done = Rc::new(Cell::new(false));
    glib::timeout_add_local(crate::refresh::BUSY_POLL, move || {
        if ctx_poll.ui_generation.get() != generation {
            return glib::ControlFlow::Break;
        }
        let busy = ctx_poll.runtime_busy();
        let visible = poll_window.is_visible();
        let tick = poll_tick.get();
        if crate::refresh::should_refresh(
            tick,
            busy,
            crate::refresh::idle_ticks_for(crate::refresh::poll_interval_for(busy, visible)),
        ) {
            ctx_poll.tick_automations();
            ctx_poll.refresh_runtime();
            dash_poll.poll_refresh();
            flow_poll.poll_refresh();
            nautilus_poll.poll_refresh();
            if !first_refresh_done.get() {
                first_refresh_done.set(true);
                loading_poll.set_visible(false);
            }
            if ctx_poll.connection_stale(std::time::Duration::from_secs(300)) {
                ctx_poll.refresh_connection();
            }
            if ctx_poll.updates_stale(std::time::Duration::from_secs(1800)) {
                ctx_poll.refresh_updates();
            }
            sync_connection_button(&ctx_poll, &conn_btn_poll);
            sync_home_button(&ctx_poll, &home_btn_poll);
            sync_card_variant_button(&ctx_poll, &card_btn_poll);
            sync_notice_button(&ctx_poll, &notice_btn_poll);
            sync_menu_badge(&ctx_poll, &menu_badge_poll);
            let menu_sig = menu_signature(&ctx_poll);
            if *last_menu_sig.borrow() != menu_sig {
                *last_menu_sig.borrow_mut() = menu_sig;
                menu_btn_poll.set_menu_model(Some(&app_menu(&ctx_poll)));
            }
            update_banner(&ctx_poll, &banner_poll, &banner_kind_poll);
            maybe_auto_prompt_repair(&ctx_poll, &poll_window, &poll_toast, &poll_auto_prompted);
        }
        poll_tick.set(tick.wrapping_add(1));
        if crate::platform::take_metered_change().is_some() {
            ctx_poll.apply_effective_bandwidth();
            update_banner(&ctx_poll, &banner_poll, &banner_kind_poll);
            maybe_auto_prompt_repair(&ctx_poll, &poll_window, &poll_toast, &poll_auto_prompted);
        }
        if let Some(tray) = &tray {
            tray.drain(&ctx_poll);
            tray.refresh(&ctx_poll);
        }
        glib::ControlFlow::Continue
    });

    ctx.start_autostarts();
    {
        let ctx = ctx.clone();
        window.connect_close_request(move |win| {
            if ctx.take_reload_destroy() {
                return glib::Propagation::Proceed;
            }
            if ctx.settings.borrow().developer.destroy_window_on_close {
                let window = win.clone();
                dialogs::confirm_shutdown(win, ctx.clone(), move || {
                    if let Some(app) = window.application() {
                        app.quit();
                    }
                });
                glib::Propagation::Stop
            } else {
                win.set_visible(false);
                glib::Propagation::Stop
            }
        });
    }
    window.present();
    if hidden {
        window.set_visible(false);
    }
    if ctx.take_reopen_prefs() {
        dialogs::preferences(&window, ctx.clone());
    }
}

fn app_menu(ctx: &AppCtx) -> gio::Menu {
    let menu = gio::Menu::new();

    let theme = gio::Menu::new();
    theme.append(
        Some(&ctx.t_or("titlebar.menu.system", "System theme")),
        Some("win.theme::system"),
    );
    theme.append(
        Some(&ctx.t_or("titlebar.menu.light", "Light")),
        Some("win.theme::light"),
    );
    theme.append(
        Some(&ctx.t_or("titlebar.menu.dark", "Dark")),
        Some("win.theme::dark"),
    );
    menu.append_submenu(Some(&ctx.t_or("titlebar.menu.theme", "Theme")), &theme);
    menu.append(
        Some(&card_variant_menu_label(ctx)),
        Some("win.toggle-card-variant"),
    );

    let file = gio::Menu::new();
    file.append(
        Some(&ctx.t_or("titlebar.menu.import", "Import settings")),
        Some("win.import"),
    );
    file.append(
        Some(&ctx.t_or("backup.importConfig.title", "Import rclone.conf")),
        Some("win.import-config"),
    );
    file.append(
        Some(&ctx.t_or("titlebar.menu.export", "Export settings")),
        Some("win.export"),
    );
    menu.append_section(None, &file);

    let prefs = gio::Menu::new();
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.preferences", "Preferences")),
        Some("win.preferences"),
    );
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.flags", "Rclone Flags")),
        Some("win.rclone-flags"),
    );
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.backends", "Backends")),
        Some("win.backends"),
    );
    let alerts = ctx.store.borrow().unacknowledged_alerts();
    let alerts_label = if alerts > 0 {
        format!("{} ({alerts})", ctx.t_or("alerts.title", "Alerts"))
    } else {
        ctx.t_or("alerts.title", "Alerts")
    };
    prefs.append(Some(&alerts_label), Some("win.alerts"));
    prefs.append(
        Some(&ctx.t_or("modals.logs.terminalOutput", "Logs")),
        Some("win.logs"),
    );
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.shortcuts", "Keyboard Shortcuts")),
        Some("win.shortcuts"),
    );
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.templates", "Templates")),
        Some("win.templates"),
    );
    let updates_label = if ctx.settings.borrow().runtime.rclone_restart_required {
        format!(
            "{} (!)",
            ctx.t_or(
                "modals.about.rcloneRestartRequired",
                "Rclone Restart Required"
            )
        )
    } else if ctx.updates.borrow().has_updates() {
        format!("{} (!)", ctx.t_or("modals.updates.title", "Updates"))
    } else {
        ctx.t_or("modals.updates.title", "Updates")
    };
    prefs.append(Some(&updates_label), Some("win.updates"));
    prefs.append(
        Some(&ctx.t_or("modals.about.whatsNew", "What's New")),
        Some("win.whats-new"),
    );
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.installRclone", "Install rclone")),
        Some("win.install-rclone"),
    );
    prefs.append(
        Some(&ctx.t_or("titlebar.menu.remoteOrder", "Remote order")),
        Some("win.item-order"),
    );
    menu.append_section(None, &prefs);

    let tools = gio::Menu::new();
    tools.append(
        Some(&ctx.t_or("titlebar.menu.openConfig", "Open config folder")),
        Some("win.open-config"),
    );
    tools.append(
        Some(&ctx.t_or("titlebar.menu.openCache", "Open cache folder")),
        Some("win.open-cache"),
    );
    tools.append(
        Some(&ctx.t_or("titlebar.menu.openLog", "Open rclone log")),
        Some("win.open-log"),
    );
    tools.append(
        Some(&ctx.t_or("modals.about.memory", "Memory stats")),
        Some("win.memory"),
    );
    tools.append(
        Some(&ctx.t_or("titlebar.menu.runGc", "Run GC")),
        Some("win.gc"),
    );
    tools.append(
        Some(&ctx.t_or("titlebar.menu.clearFsCache", "Clear FS cache")),
        Some("win.fscache"),
    );
    tools.append(
        Some(&ctx.t_or("titlebar.menu.checkConnectivity", "Check connectivity")),
        Some("win.ping"),
    );
    tools.append(
        Some(&ctx.t_or("developerTools.refreshUi", "Refresh UI")),
        Some("win.refresh-ui"),
    );
    tools.append(
        Some(&ctx.t_or("developerTools.debugInfo", "Debug Info")),
        Some("win.debug-info"),
    );
    tools.append(
        Some(&ctx.t_or("developerTools.openDevTools", "Open Developer Tools")),
        Some("win.inspector"),
    );
    tools.append(
        Some(&ctx.t_or("developerTools.relaunch", "Relaunch")),
        Some("win.relaunch"),
    );
    menu.append_submenu(
        Some(&ctx.t_or("titlebar.menu.developer", "Developer")),
        &tools,
    );

    let views = gio::Menu::new();
    let current = ctx.active_workspace.borrow().clone();
    if current != "main_menu" {
        views.append(
            Some(&ctx.t_or(
                "settings.general.default_view.options.main_menu",
                "Main Menu",
            )),
            Some("win.view::main_menu"),
        );
    }
    if current != "nautilus" {
        views.append(
            Some(&ctx.t_or("titlebar.menu.fileBrowser", "File Browser")),
            Some("win.view::nautilus"),
        );
    }
    if current != "flow" {
        views.append(
            Some(&ctx.t_or("titlebar.menu.flowWorkspace", "Flow")),
            Some("win.view::flow"),
        );
    }
    views.append(
        Some(&ctx.t_or("titlebar.detach", "Detach workspace")),
        Some("win.detach-workspace"),
    );
    views.append(
        Some(&ctx.t_or(
            "titlebar.closeOverlay",
            "Close overlay and return to the base view",
        )),
        Some("win.close-overlay"),
    );
    menu.append_section(None, &views);

    let tray = gio::Menu::new();
    tray.append(
        Some(&ctx.t_or("tray.unmountAll", "Unmount All")),
        Some("win.unmount-all"),
    );
    tray.append(
        Some(&ctx.t_or("tray.stopAllJobs", "Stop All Jobs")),
        Some("win.stop-jobs"),
    );
    tray.append(
        Some(&ctx.t_or("tray.stopAllServes", "Stop All Serves")),
        Some("win.stop-serves"),
    );
    menu.append_submenu(
        Some(&ctx.t_or("titlebar.menu.trayActions", "Tray actions")),
        &tray,
    );

    let about = gio::Menu::new();
    let about_label = if ctx.updates.borrow().has_updates()
        || ctx.settings.borrow().runtime.rclone_restart_required
    {
        format!("{} (!)", ctx.t_or("titlebar.menu.about", "About"))
    } else {
        ctx.t_or("titlebar.menu.about", "About")
    };
    about.append(Some(&about_label), Some("win.about"));
    about.append(Some(&ctx.t_or("tray.quit", "Quit")), Some("win.quit"));
    menu.append_section(None, &about);
    menu
}

#[allow(clippy::too_many_arguments)]
fn install_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    view_stack: &adw::ViewStack,
    dashboard: &Dashboard,
    nautilus: &NautilusView,
    flow: &FlowView,
    banner: &adw::Banner,
) {
    let add_action = |name: &str, cb: Box<dyn Fn() + 'static>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| cb());
        window.add_action(&action);
    };

    {
        let ctx = ctx.clone();
        let window = window.clone();
        let dash = dashboard.clone();
        add_action(
            "quick-add",
            Box::new(move || {
                dialogs::quick_add_remote(&window, ctx.clone(), {
                    let dash = dash.clone();
                    let ctx = ctx.clone();
                    Rc::new(move || {
                        ctx.refresh_runtime();
                        dash.refresh();
                    })
                });
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let dash = dashboard.clone();
        add_action(
            "remote-config",
            Box::new(move || {
                dialogs::remote_config(&window, ctx.clone(), None, {
                    let dash = dash.clone();
                    let ctx = ctx.clone();
                    Rc::new(move || {
                        ctx.refresh_runtime();
                        dash.refresh();
                    })
                });
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let flow = flow.clone();
        add_action(
            "quick-run-new",
            Box::new(move || {
                dialogs::quick_run_editor(&window, ctx.clone(), None, {
                    let flow = flow.clone();
                    Rc::new(move || flow.refresh())
                });
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "preferences",
            Box::new(move || dialogs::preferences(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "rclone-flags",
            Box::new(move || dialogs::rclone_flags(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "backends",
            Box::new(move || dialogs::backends(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "alerts",
            Box::new(move || dialogs::alerts(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "logs",
            Box::new(move || dialogs::logs(&window, ctx.clone(), None)),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "shortcuts",
            Box::new(move || dialogs::shortcuts(&window, &ctx)),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "templates",
            Box::new(move || dialogs::templates(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "updates",
            Box::new(move || dialogs::updates(&window, ctx.clone(), toast.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "whats-new",
            Box::new(move || dialogs::whats_new(&window, ctx.clone(), "app")),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "install-rclone",
            Box::new(move || dialogs::install_rclone_update(&window, ctx.clone(), toast.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let dash = dashboard.clone();
        add_action(
            "item-order",
            Box::new(move || {
                dialogs::item_order(&window, ctx.clone(), {
                    let dash = dash.clone();
                    Rc::new(move || dash.refresh())
                })
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "about",
            Box::new(move || dialogs::about(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "export",
            Box::new(move || dialogs::export_backup(&window, ctx.clone(), toast.clone(), None)),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        add_action(
            "import",
            Box::new(move || {
                dialogs::import_backup(&window, ctx.clone(), toast.clone(), {
                    let dash = dash.clone();
                    Rc::new(move || dash.refresh())
                })
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        add_action(
            "import-config",
            Box::new(move || {
                dialogs::import_rclone_config_picker(&window, ctx.clone(), toast.clone(), {
                    let dash = dash.clone();
                    Rc::new(move || dash.refresh())
                })
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "unmount-all",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let _ = c.unmount_all();
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "stop-jobs",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let ids: Vec<u64> = ctx
                        .snapshot
                        .borrow()
                        .jobs
                        .iter()
                        .filter(|job| {
                            crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job)
                        })
                        .map(|job| job.id)
                        .collect();
                    for jobid in ids {
                        let _ = c.job_stop(jobid);
                    }
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "stop-serves",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let _ = c.serve_stop_all();
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let window = window.clone();
        let ctx = ctx.clone();
        add_action(
            "quit",
            Box::new(move || {
                let window = window.clone();
                let closer = window.clone();
                dialogs::confirm_shutdown(&window, ctx.clone(), move || {
                    if let Some(app) = closer.application() {
                        app.quit();
                    }
                });
            }),
        );
    }

    install_theme_action(window, ctx);

    let view_action = gio::SimpleAction::new_stateful(
        "view",
        Some(&glib::VariantTy::STRING),
        &glib::Variant::from(ctx.settings.borrow().general.default_view.as_str()),
    );
    {
        let stack = view_stack.clone();
        let ctx = ctx.clone();
        let window = window.clone();
        view_action.connect_activate(move |action, value| {
            if let Some(v) = value.and_then(|v| v.str().map(|s| s.to_string())) {
                if open_workspace_target(&ctx, &window, &stack, &v) {
                    action.set_state(&glib::Variant::from(
                        stack.visible_child_name().unwrap_or_default().as_str(),
                    ));
                } else {
                    action.set_state(&glib::Variant::from(v.as_str()));
                }
            }
        });
    }
    window.add_action(&view_action);

    {
        let stack = view_stack.clone();
        let view_action = view_action.clone();
        let ctx = ctx.clone();
        let window = window.clone();
        let last = Rc::new(std::cell::RefCell::new(
            ctx.settings.borrow().general.default_view.clone(),
        ));
        add_action(
            "toggle-flow",
            Box::new(move || {
                let current = stack.visible_child_name().unwrap_or_default().to_string();
                let next = if current == "flow" {
                    let prev = last.borrow().clone();
                    if prev.is_empty() || prev == "flow" {
                        "main_menu".into()
                    } else {
                        prev
                    }
                } else {
                    *last.borrow_mut() = current;
                    "flow".into()
                };
                if !open_workspace_target(&ctx, &window, &stack, &next) {
                    view_action.set_state(&glib::Variant::from(next.as_str()));
                }
            }),
        );
    }

    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        let action = gio::SimpleAction::new("refresh-mounts", None);
        action.connect_activate(move |_, _| match ctx.force_check_mounts() {
            Ok(_) => {
                dash.refresh();
                toast.add_toast(adw::Toast::new(
                    &ctx.t_or("shortcuts.mountsRefreshSuccess", "Mounts refreshed"),
                ));
            }
            Err(error) => ctx.toast_error(&toast, &error),
        });
        window.add_action(&action);
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        let action = gio::SimpleAction::new("refresh-serves", None);
        action.connect_activate(move |_, _| match ctx.force_check_serves() {
            Ok(_) => {
                dash.refresh();
                toast.add_toast(adw::Toast::new(
                    &ctx.t_or("shortcuts.servesRefreshSuccess", "Serves refreshed"),
                ));
            }
            Err(error) => ctx.toast_error(&toast, &error),
        });
        window.add_action(&action);
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        let flow = flow.clone();
        add_action(
            "refresh-ui",
            Box::new(move || {
                ctx.refresh_runtime();
                dash.refresh();
                flow.refresh();
                toast.add_toast(adw::Toast::new(
                    &ctx.t_or("developerTools.refreshUi", "Refresh UI"),
                ));
            }),
        );
    }

    {
        add_action(
            "open-config",
            Box::new(|| {
                let _ = open::that(crate::settings::AppSettings::config_dir());
            }),
        );
        add_action(
            "open-cache",
            Box::new(|| {
                let dir = crate::settings::AppSettings::cache_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = open::that(dir);
            }),
        );
        add_action(
            "open-log",
            Box::new(|| {
                let _ = open::that(crate::settings::AppSettings::log_path());
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "memory",
            Box::new(move || dialogs::memory_stats(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "gc",
            Box::new(move || {
                if let Some(client) = ctx.client() {
                    match client.gc() {
                        Ok(_) => toast.add_toast(adw::Toast::new(
                            &ctx.t_or("developerTools.gcStarted", "Garbage collection started"),
                        )),
                        Err(e) => ctx.toast_error(&toast, &e.to_string()),
                    }
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "fscache",
            Box::new(move || {
                if let Some(client) = ctx.client() {
                    match client.fscache_clear() {
                        Ok(_) => toast.add_toast(adw::Toast::new(
                            &ctx.t_or("modals.about.cacheCleared", "Cache cleared successfully"),
                        )),
                        Err(e) => ctx.toast_error(&toast, &e.to_string()),
                    }
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "ping",
            Box::new(move || {
                let urls = ctx.settings.borrow().core.connection_check_urls.clone();
                let results = crate::connection::check_links(&urls, 4);
                let body = results
                    .iter()
                    .map(|r| {
                        format!(
                            "{} — {} ({})",
                            r.url,
                            if r.ok { "ok" } else { "fail" },
                            r.detail
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let alert = adw::AlertDialog::new(
                    Some(&crate::connection::summarize(&results)),
                    Some(&body),
                );
                alert.add_response("ok", &ctx.t_or("common.ok", "OK"));
                alert.present(Some(&window));
            }),
        );
    }
    {
        let window = window.clone();
        let ctx = ctx.clone();
        add_action(
            "debug-info",
            Box::new(move || {
                dialogs::debug_info(&window, ctx.clone());
            }),
        );
    }
    add_action(
        "inspector",
        Box::new(|| gtk::Window::set_interactive_debugging(true)),
    );
    {
        let ctx = ctx.clone();
        add_action("close-overlay", Box::new(move || ctx.close_overlays()));
    }
    {
        let app = app.clone();
        let toast = toast.clone();
        let ctx = ctx.clone();
        add_action(
            "relaunch",
            Box::new(move || match crate::platform::relaunch() {
                Ok(()) => app.quit(),
                Err(e) => ctx.toast_error(&toast, &e),
            }),
        );
    }

    {
        let app = app.clone();
        let ctx = ctx.clone();
        let stack = view_stack.clone();
        let nautilus = nautilus.clone();
        add_action(
            "detach-workspace",
            Box::new(move || match stack.visible_child_name().as_deref() {
                Some("nautilus") => nautilus.detach_current_tab(),
                Some("flow") => open_workspace_window(&app, &ctx, MainView::Flow),
                _ => open_workspace_window(&app, &ctx, MainView::MainMenu),
            }),
        );
    }

    let _ = banner;
}

fn install_theme_action(window: &adw::ApplicationWindow, ctx: &AppCtx) {
    let theme_action = gio::SimpleAction::new_stateful(
        "theme",
        Some(&glib::VariantTy::STRING),
        &glib::Variant::from(ctx.settings.borrow().runtime.theme.as_str()),
    );
    {
        let ctx = ctx.clone();
        theme_action.connect_activate(move |action, value| {
            if let Some(v) = value.and_then(|v| v.str().map(|s| s.to_string())) {
                ctx.settings.borrow_mut().runtime.theme = v.clone();
                ctx.persist();
                ctx.apply_theme();
                action.set_state(&glib::Variant::from(v.as_str()));
            }
        });
    }
    window.add_action(&theme_action);
}

fn install_shortcuts(window: &adw::ApplicationWindow) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);
    let add = |accel: &str, action: &str| {
        let trigger = gtk::ShortcutTrigger::parse_string(accel);
        let shortcut = gtk::Shortcut::new(trigger, Some(gtk::NamedAction::new(action)));
        controller.add_shortcut(shortcut);
    };
    add("<Control>q", "win.quit");
    add("<Control>b", "win.view::nautilus");
    add("<Control>n", "win.remote-config");
    add("<Control>r", "win.quick-add");
    add("<Control>i", "win.import");
    add("<Control>e", "win.export");
    add("<Control>comma", "win.preferences");
    add("<Control>period", "win.rclone-flags");
    add("<Control><Alt>a", "win.alerts");
    add("<Control><Alt>f", "win.toggle-flow");
    add("<Control>question", "win.shortcuts");
    add("<Control><Shift>question", "win.shortcuts");
    add("<Control><Shift>m", "win.refresh-mounts");
    add("<Control><Shift>s", "win.refresh-serves");
    add("<Control><Shift>d", "win.detach-workspace");
    window.add_controller(controller);
}

fn present_standalone_workspace(app: &adw::Application, ctx: &AppCtx, target: &NavTarget) {
    match target {
        NavTarget::Files { remote, path } => present_files_window(app, ctx, remote, path),
        NavTarget::Flow { quick_run } => {
            let toast = adw::ToastOverlay::new();
            let flow = FlowView::new(ctx.clone(), toast.clone());
            toast.set_child(Some(&flow.root));
            flow.refresh();
            flow.select_quick_run(quick_run.as_deref());
            present_overlay_window(
                app,
                ctx,
                &ctx.t_or("titlebar.menu.flowWorkspace", "Flow"),
                toast,
                None,
                OverlaySpec::Flow {
                    quick_run: quick_run.clone(),
                },
                false,
            );
        }
        NavTarget::Dashboard { tab, remote } => {
            let toast = adw::ToastOverlay::new();
            let dash = Dashboard::new(ctx.clone(), toast.clone());
            toast.set_child(Some(&dash.root));
            dash.refresh();
            dash.navigate(*tab, remote.as_deref());
            present_overlay_window(
                app,
                ctx,
                &ctx.t_or(
                    "settings.general.default_view.options.main_menu",
                    "Main Menu",
                ),
                toast,
                None,
                OverlaySpec::Dashboard {
                    tab: *tab,
                    remote: remote.clone(),
                },
                false,
            );
        }
        _ => {}
    }
}

fn open_workspace_target(
    ctx: &AppCtx,
    window: &adw::ApplicationWindow,
    stack: &adw::ViewStack,
    target: &str,
) -> bool {
    let default = ctx.settings.borrow().general.default_view.clone();
    match crate::navigation::workspace_open_plan(&default, target) {
        crate::navigation::WorkspaceOpen::SwitchStack => {
            stack.set_visible_child_name(target);
            false
        }
        crate::navigation::WorkspaceOpen::OverlayFiles => {
            present_files_at(window, ctx, "local", "");
            true
        }
        crate::navigation::WorkspaceOpen::OverlayFlow => {
            if let Some(app) = window.application().and_downcast::<adw::Application>() {
                open_workspace_window(&app, ctx, MainView::Flow);
            }
            true
        }
        crate::navigation::WorkspaceOpen::OverlayDashboard => {
            if let Some(app) = window.application().and_downcast::<adw::Application>() {
                open_workspace_window(&app, ctx, MainView::MainMenu);
            }
            true
        }
    }
}

pub fn present_files_at(parent: &impl IsA<gtk::Widget>, ctx: &AppCtx, remote: &str, path: &str) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let Some(app) = win.application().and_downcast::<adw::Application>() else {
        return;
    };
    present_files_overlay(&app, ctx, remote, path, None);
}

fn files_target(remote: &str, path: &str) -> String {
    if remote == "local" {
        if path.is_empty() {
            "/".into()
        } else {
            path.to_string()
        }
    } else if path.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{path}")
    }
}

fn present_files_window(app: &adw::Application, ctx: &AppCtx, remote: &str, path: &str) {
    let window = present_files_overlay_ex(app, ctx, remote, path, None, false);
    {
        let ctx = ctx.clone();
        window.connect_close_request(move |win| {
            let keep = crate::cli::start_hidden() || ctx.settings.borrow().general.tray_enabled;
            if !keep {
                if let Some(app) = win.application() {
                    app.quit();
                }
            }
            glib::Propagation::Proceed
        });
    }
}

pub fn present_files_overlay(
    app: &adw::Application,
    ctx: &AppCtx,
    remote: &str,
    path: &str,
    title: Option<&str>,
) -> adw::ApplicationWindow {
    present_files_overlay_ex(app, ctx, remote, path, title, true)
}

fn present_files_overlay_ex(
    app: &adw::Application,
    ctx: &AppCtx,
    remote: &str,
    path: &str,
    title: Option<&str>,
    register: bool,
) -> adw::ApplicationWindow {
    let toast = adw::ToastOverlay::new();
    let files = NautilusView::new(ctx.clone(), toast.clone());
    toast.set_child(Some(&files.root));
    files.navigate_to(&files_target(remote, path));
    let fallback = ctx.t_or("nautilus.titles.files", "Files");
    present_overlay_window(
        app,
        ctx,
        title.unwrap_or(&fallback),
        toast,
        Some(files),
        OverlaySpec::Files {
            remote: remote.to_string(),
            path: path.to_string(),
        },
        register,
    )
}

fn present_overlay_window(
    app: &adw::Application,
    ctx: &AppCtx,
    title: &str,
    toast: adw::ToastOverlay,
    files: Option<NautilusView>,
    spec: OverlaySpec,
    register: bool,
) -> adw::ApplicationWindow {
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(&ctx.t_or("titlebar.appMenu", "Application menu"))
        .menu_model(&app_menu(ctx))
        .build();
    header.pack_end(&menu_btn);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&toast));
    let window = adw::ApplicationWindow::new(app);
    window.set_title(Some(title));
    window.set_default_width(1100);
    window.set_default_height(760);
    window.set_content(Some(&toolbar));
    install_overlay_actions(&window, ctx, &toast, spec.clone(), files.clone());
    install_shortcuts(&window);
    install_debug_context_menu(&window, ctx);
    if let Some(files) = files {
        let window_poll = window.clone();
        let ctx_poll = ctx.clone();
        // Gate this the same way the main window does. Ungated, a detached
        // Files window drove `refresh_runtime` — and the RC engine behind it —
        // every 400 ms for as long as it stayed open, even with nothing
        // running, on top of the main window's own (throttled) poll.
        let poll_tick = Cell::new(0u32);
        glib::timeout_add_local(crate::refresh::BUSY_POLL, move || {
            if !window_poll.is_visible() {
                return glib::ControlFlow::Break;
            }
            let busy = ctx_poll.runtime_busy();
            let tick = poll_tick.get();
            poll_tick.set(tick.wrapping_add(1));
            if crate::refresh::should_refresh(
                tick,
                busy,
                crate::refresh::idle_ticks_for(crate::refresh::poll_interval_for(busy, true)),
            ) {
                ctx_poll.refresh_runtime();
                files.poll_refresh();
            }
            glib::ControlFlow::Continue
        });
    }
    window.present();
    if register {
        ctx.register_overlay(&window, spec);
    }
    window
}

fn install_overlay_actions(
    window: &adw::ApplicationWindow,
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    spec: OverlaySpec,
    files: Option<NautilusView>,
) {
    let add_action = |name: &str, cb: Box<dyn Fn() + 'static>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| cb());
        window.add_action(&action);
    };
    install_theme_action(window, ctx);
    {
        let ctx = ctx.clone();
        let action = gio::SimpleAction::new("toggle-card-variant", None);
        action.connect_activate(move |_, _| toggle_card_variant(&ctx));
        window.add_action(&action);
    }
    let refresh = {
        let ctx = ctx.clone();
        Rc::new(move || ctx.refresh_runtime())
    };
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let refresh = refresh.clone();
        add_action(
            "quick-add",
            Box::new(move || dialogs::quick_add_remote(&window, ctx.clone(), refresh.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let refresh = refresh.clone();
        add_action(
            "remote-config",
            Box::new(move || dialogs::remote_config(&window, ctx.clone(), None, refresh.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let refresh = refresh.clone();
        add_action(
            "quick-run-new",
            Box::new(move || {
                dialogs::quick_run_editor(&window, ctx.clone(), None, refresh.clone())
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "preferences",
            Box::new(move || dialogs::preferences(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "rclone-flags",
            Box::new(move || dialogs::rclone_flags(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "backends",
            Box::new(move || dialogs::backends(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "alerts",
            Box::new(move || dialogs::alerts(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "logs",
            Box::new(move || dialogs::logs(&window, ctx.clone(), None)),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "shortcuts",
            Box::new(move || dialogs::shortcuts(&window, &ctx)),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "templates",
            Box::new(move || dialogs::templates(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "updates",
            Box::new(move || dialogs::updates(&window, ctx.clone(), toast.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "whats-new",
            Box::new(move || dialogs::whats_new(&window, ctx.clone(), "app")),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "install-rclone",
            Box::new(move || dialogs::install_rclone_update(&window, ctx.clone(), toast.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let refresh = refresh.clone();
        add_action(
            "item-order",
            Box::new(move || dialogs::item_order(&window, ctx.clone(), refresh.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "about",
            Box::new(move || dialogs::about(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "export",
            Box::new(move || dialogs::export_backup(&window, ctx.clone(), toast.clone(), None)),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        let refresh = refresh.clone();
        add_action(
            "import",
            Box::new(move || {
                dialogs::import_backup(&window, ctx.clone(), toast.clone(), refresh.clone())
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        let refresh = refresh.clone();
        add_action(
            "import-config",
            Box::new(move || {
                dialogs::import_rclone_config_picker(
                    &window,
                    ctx.clone(),
                    toast.clone(),
                    refresh.clone(),
                )
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "unmount-all",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let _ = c.unmount_all();
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "stop-jobs",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let ids: Vec<u64> = ctx
                        .snapshot
                        .borrow()
                        .jobs
                        .iter()
                        .filter(|job| {
                            crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job)
                        })
                        .map(|job| job.id)
                        .collect();
                    for jobid in ids {
                        let _ = c.job_stop(jobid);
                    }
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "stop-serves",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let _ = c.serve_stop_all();
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let window = window.clone();
        let ctx = ctx.clone();
        add_action(
            "quit",
            Box::new(move || {
                let window = window.clone();
                let closer = window.clone();
                dialogs::confirm_shutdown(&window, ctx.clone(), move || {
                    if let Some(app) = closer.application() {
                        app.quit();
                    }
                });
            }),
        );
    }
    let kind = spec.kind();
    let view_action = gio::SimpleAction::new_stateful(
        "view",
        Some(&glib::VariantTy::STRING),
        &glib::Variant::from(kind.as_view()),
    );
    {
        let ctx = ctx.clone();
        let window = window.clone();
        view_action.connect_activate(move |action, value| {
            if let Some(v) = value.and_then(|v| v.str().map(|s| s.to_string())) {
                let target = OverlayKind::from_view(&v);
                action.set_state(&glib::Variant::from(v.as_str()));
                if target == kind {
                    return;
                }
                if ctx.focus_overlay_kind(target) {
                    return;
                }
                if let Some(app) = window.application().and_downcast::<adw::Application>() {
                    reopen_overlay(&app, &ctx, OverlaySpec::default_for(target));
                }
            }
        });
    }
    window.add_action(&view_action);
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "toggle-flow",
            Box::new(move || {
                if kind == OverlayKind::Flow {
                    window.close();
                    return;
                }
                if ctx.focus_overlay_kind(OverlayKind::Flow) {
                    return;
                }
                if let Some(app) = window.application().and_downcast::<adw::Application>() {
                    reopen_overlay(&app, &ctx, OverlaySpec::default_for(OverlayKind::Flow));
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "refresh-mounts",
            Box::new(move || match ctx.force_check_mounts() {
                Ok(_) => toast.add_toast(adw::Toast::new(
                    &ctx.t_or("shortcuts.mountsRefreshSuccess", "Mounts refreshed"),
                )),
                Err(error) => ctx.toast_error(&toast, &error),
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "refresh-serves",
            Box::new(move || match ctx.force_check_serves() {
                Ok(_) => toast.add_toast(adw::Toast::new(
                    &ctx.t_or("shortcuts.servesRefreshSuccess", "Serves refreshed"),
                )),
                Err(error) => ctx.toast_error(&toast, &error),
            }),
        );
    }
    add_action(
        "open-config",
        Box::new(|| {
            let _ = open::that(crate::settings::AppSettings::config_dir());
        }),
    );
    add_action(
        "open-cache",
        Box::new(|| {
            let dir = crate::settings::AppSettings::cache_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = open::that(dir);
        }),
    );
    add_action(
        "open-log",
        Box::new(|| {
            let _ = open::that(crate::settings::AppSettings::log_path());
        }),
    );
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "memory",
            Box::new(move || dialogs::memory_stats(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "gc",
            Box::new(move || {
                if let Some(client) = ctx.client() {
                    match client.gc() {
                        Ok(_) => toast.add_toast(adw::Toast::new(
                            &ctx.t_or("developerTools.gcStarted", "Garbage collection started"),
                        )),
                        Err(e) => ctx.toast_error(&toast, &e.to_string()),
                    }
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "fscache",
            Box::new(move || {
                if let Some(client) = ctx.client() {
                    match client.fscache_clear() {
                        Ok(_) => toast.add_toast(adw::Toast::new(
                            &ctx.t_or("modals.about.cacheCleared", "Cache cleared successfully"),
                        )),
                        Err(e) => ctx.toast_error(&toast, &e.to_string()),
                    }
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "ping",
            Box::new(move || {
                let urls = ctx.settings.borrow().core.connection_check_urls.clone();
                let results = crate::connection::check_links(&urls, 4);
                let body = results
                    .iter()
                    .map(|r| {
                        format!(
                            "{} — {} ({})",
                            r.url,
                            if r.ok { "ok" } else { "fail" },
                            r.detail
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let alert = adw::AlertDialog::new(
                    Some(&crate::connection::summarize(&results)),
                    Some(&body),
                );
                alert.add_response("ok", &ctx.t_or("common.ok", "OK"));
                alert.present(Some(&window));
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        add_action(
            "refresh-ui",
            Box::new(move || {
                ctx.refresh_runtime();
                toast.add_toast(adw::Toast::new(
                    &ctx.t_or("developerTools.refreshUi", "Refresh UI"),
                ));
            }),
        );
    }
    {
        let window = window.clone();
        let ctx = ctx.clone();
        add_action(
            "debug-info",
            Box::new(move || dialogs::debug_info(&window, ctx.clone())),
        );
    }
    add_action(
        "inspector",
        Box::new(|| gtk::Window::set_interactive_debugging(true)),
    );
    {
        let ctx = ctx.clone();
        add_action("close-overlay", Box::new(move || ctx.close_overlays()));
    }
    {
        let app = window.application();
        let toast = toast.clone();
        let ctx = ctx.clone();
        add_action(
            "relaunch",
            Box::new(move || match crate::platform::relaunch() {
                Ok(()) => {
                    if let Some(app) = &app {
                        app.quit();
                    }
                }
                Err(e) => ctx.toast_error(&toast, &e),
            }),
        );
    }
    {
        let files = files.clone();
        add_action(
            "detach-workspace",
            Box::new(move || {
                if let Some(files) = &files {
                    files.detach_current_tab();
                }
            }),
        );
    }
}

fn open_workspace_window(app: &adw::Application, ctx: &AppCtx, view: MainView) {
    let spec = match view {
        MainView::Flow => OverlaySpec::default_for(OverlayKind::Flow),
        MainView::Nautilus => OverlaySpec::default_for(OverlayKind::Files),
        MainView::MainMenu => OverlaySpec::default_for(OverlayKind::Dashboard),
    };
    reopen_overlay(app, ctx, spec);
}

fn reopen_overlay(app: &adw::Application, ctx: &AppCtx, spec: OverlaySpec) {
    if ctx.focus_overlay_kind(spec.kind()) {
        return;
    }
    let toast = adw::ToastOverlay::new();
    let title = match &spec {
        OverlaySpec::Flow { .. } => ctx.t_or("titlebar.menu.flowWorkspace", "Flow"),
        OverlaySpec::Files { .. } => ctx.t_or("nautilus.titles.files", "Files"),
        OverlaySpec::Dashboard { .. } => ctx.t_or(
            "settings.general.default_view.options.main_menu",
            "Main Menu",
        ),
    };
    let files = match &spec {
        OverlaySpec::Flow { quick_run } => {
            let flow = FlowView::new(ctx.clone(), toast.clone());
            toast.set_child(Some(&flow.root));
            flow.refresh();
            flow.select_quick_run(quick_run.as_deref());
            None
        }
        OverlaySpec::Files { remote, path } => {
            let files = NautilusView::new(ctx.clone(), toast.clone());
            toast.set_child(Some(&files.root));
            files.navigate_to(&files_target(remote, path));
            Some(files)
        }
        OverlaySpec::Dashboard { tab, remote } => {
            let dash = Dashboard::new(ctx.clone(), toast.clone());
            toast.set_child(Some(&dash.root));
            dash.refresh();
            dash.navigate(*tab, remote.as_deref());
            None
        }
    };
    present_overlay_window(app, ctx, &title, toast, files, spec, true);
}

fn apply_nav(
    ctx: &AppCtx,
    stack: &adw::ViewStack,
    dashboard: &Dashboard,
    flow: &FlowView,
    window: &adw::ApplicationWindow,
    toast: &adw::ToastOverlay,
    target: NavTarget,
) {
    if matches!(
        target,
        NavTarget::Updates
            | NavTarget::Alerts
            | NavTarget::Preferences { .. }
            | NavTarget::RemoteConfig { .. }
            | NavTarget::Onboarding
            | NavTarget::About { .. }
            | NavTarget::Logs { .. }
            | NavTarget::Shortcuts
            | NavTarget::Backends
            | NavTarget::Flags
            | NavTarget::Templates { .. }
            | NavTarget::Export { .. }
            | NavTarget::Repair
            | NavTarget::Job { .. }
            | NavTarget::QuickAdd
            | NavTarget::WhatsNew { .. }
            | NavTarget::Properties { .. }
            | NavTarget::FileViewer { .. }
            | NavTarget::StartOperation { .. }
            | NavTarget::Vfs { .. }
            | NavTarget::DeleteRemote { .. }
            | NavTarget::RemoteAbout { .. }
            | NavTarget::RestorePreview { .. }
            | NavTarget::ArchiveCreate { .. }
            | NavTarget::QuickRunEditor { .. }
    ) && !window.is_mapped()
    {
        let ctx = ctx.clone();
        let stack = stack.clone();
        let dashboard = dashboard.clone();
        let flow = flow.clone();
        let window = window.clone();
        let toast = toast.clone();
        glib::idle_add_local_once(move || {
            apply_nav(&ctx, &stack, &dashboard, &flow, &window, &toast, target);
        });
        return;
    }
    match target {
        NavTarget::Dashboard { tab, remote } => {
            stack.set_visible_child_name("main_menu");
            dashboard.navigate(tab, remote.as_deref());
        }
        NavTarget::Files { remote, path } => {
            if crate::navigation::overlay_files_for_workspace(&ctx.active_workspace.borrow()) {
                *ctx.pending_files_overlay.borrow_mut() = Some((remote, path));
            } else {
                *ctx.pending_browse.borrow_mut() = Some((remote, path));
            }
        }
        NavTarget::Flow { quick_run } => {
            stack.set_visible_child_name("flow");
            flow.select_quick_run(quick_run.as_deref());
        }
        NavTarget::Job { id } => {
            let job = ctx
                .snapshot
                .borrow()
                .jobs
                .iter()
                .find(|j| j.id == id)
                .cloned()
                .or_else(|| {
                    ctx.store
                        .borrow()
                        .job_history
                        .iter()
                        .find(|j| j.id == id)
                        .cloned()
                });
            if let Some(job) = job {
                match NavTarget::for_job(&job) {
                    NavTarget::Flow { quick_run } => {
                        stack.set_visible_child_name("flow");
                        flow.select_quick_run(quick_run.as_deref());
                    }
                    NavTarget::Dashboard { tab, remote } => {
                        stack.set_visible_child_name("main_menu");
                        dashboard.navigate(tab, remote.as_deref());
                    }
                    other => apply_nav(ctx, stack, dashboard, flow, window, toast, other),
                }
            }
            dialogs::job_detail(window, ctx.clone(), id);
        }
        NavTarget::Serve { id } => {
            let serve = ctx
                .snapshot
                .borrow()
                .serves
                .iter()
                .find(|s| s.id == id)
                .cloned();
            if let Some(serve) = serve {
                match NavTarget::for_serve(&serve.fs, None) {
                    NavTarget::Flow { quick_run } => {
                        stack.set_visible_child_name("flow");
                        flow.select_quick_run(quick_run.as_deref());
                    }
                    NavTarget::Dashboard { tab, remote } => {
                        stack.set_visible_child_name("main_menu");
                        dashboard.navigate(tab, remote.as_deref());
                    }
                    other => apply_nav(ctx, stack, dashboard, flow, window, toast, other),
                }
            } else {
                stack.set_visible_child_name("main_menu");
                dashboard.navigate(AppTab::Serve, None);
            }
        }
        NavTarget::Automation { id } => {
            *ctx.selected_automation.borrow_mut() = Some(id.clone());
            let record = crate::automation::collect(&ctx.store.borrow())
                .into_iter()
                .find(|r| r.id == id);
            if let Some(record) = record {
                match NavTarget::for_automation(&record) {
                    NavTarget::Flow { quick_run } => {
                        stack.set_visible_child_name("flow");
                        flow.select_quick_run(quick_run.as_deref());
                    }
                    NavTarget::Dashboard { tab, remote } => {
                        stack.set_visible_child_name("main_menu");
                        dashboard.navigate(tab, remote.as_deref());
                    }
                    other => apply_nav(ctx, stack, dashboard, flow, window, toast, other),
                }
            }
        }
        NavTarget::Updates => dialogs::updates(window, ctx.clone(), toast.clone()),
        NavTarget::Alerts => dialogs::alerts(window, ctx.clone()),
        NavTarget::Preferences { page } => {
            dialogs::preferences_page(window, ctx.clone(), page.as_deref());
        }
        NavTarget::RemoteConfig {
            remote,
            step,
            profile,
            auto_add,
            clone_from,
        } => {
            dialogs::remote_config_open(
                window,
                ctx.clone(),
                if remote.is_empty() {
                    None
                } else {
                    Some(remote)
                },
                super::remote_config::RemoteConfigOpen {
                    initial: step,
                    profile,
                    auto_add,
                    clone_from,
                },
                Rc::new(|| ()),
            );
        }
        NavTarget::Onboarding => {
            if let Some(app) = window.application().and_downcast::<adw::Application>() {
                onboarding::present(&app, ctx.clone());
            }
        }
        NavTarget::About { page } => {
            dialogs::about_open(window, ctx.clone(), page.as_deref().unwrap_or("home"))
        }
        NavTarget::Logs { remote } => dialogs::logs(window, ctx.clone(), remote),
        NavTarget::Shortcuts => dialogs::shortcuts(window, ctx),
        NavTarget::Backends => dialogs::backends(window, ctx.clone()),
        NavTarget::Flags => dialogs::rclone_flags(window, ctx.clone()),
        NavTarget::Templates { save } => dialogs::templates_open(window, ctx.clone(), save),
        NavTarget::Export { remote } => {
            dialogs::export_backup(window, ctx.clone(), toast.clone(), remote.as_deref())
        }
        NavTarget::Repair => dialogs::repair(window, ctx.clone(), toast.clone()),
        NavTarget::QuickAdd => dialogs::quick_add_remote(window, ctx.clone(), Rc::new(|| ())),
        NavTarget::WhatsNew { rclone } => {
            dialogs::whats_new(window, ctx.clone(), if rclone { "rclone" } else { "app" })
        }
        NavTarget::Properties { remote, path, name } => {
            dialogs::properties(window, ctx.clone(), &remote, &path, &name)
        }
        NavTarget::FileViewer {
            remote,
            path,
            name,
            is_dir,
        } => dialogs::file_viewer(
            window,
            ctx.clone(),
            &remote,
            &path,
            &name,
            is_dir,
            &[],
            None,
        ),
        NavTarget::StartOperation { remote, operation } => {
            let op = crate::operations::OperationType::parse(&operation)
                .unwrap_or(crate::operations::OperationType::Copy);
            dialogs::start_operation(
                window,
                ctx.clone(),
                &remote,
                op,
                toast.clone(),
                Rc::new(|| ()),
            );
        }
        NavTarget::Vfs { remote } => {
            dialogs::vfs_control(window, ctx.clone(), &remote, toast.clone())
        }
        NavTarget::DeleteRemote { remote } => {
            dialogs::delete_remote(window, ctx.clone(), &remote, Rc::new(|| ()))
        }
        NavTarget::RemoteAbout { remote } => dialogs::remote_about(window, ctx.clone(), &remote),
        NavTarget::RestorePreview { path } => dialogs::restore_preview(
            window,
            ctx.clone(),
            toast.clone(),
            std::path::PathBuf::from(path),
            Rc::new(|| ()),
        ),
        NavTarget::ArchiveCreate {
            remote,
            path,
            names,
        } => dialogs::archive_create(window, ctx.clone(), &remote, &path, &names),
        NavTarget::QuickRunEditor { id } => {
            stack.set_visible_child_name("flow");
            if let Some(id) = id.as_deref() {
                flow.select_quick_run(Some(id));
            }
            let existing = id.and_then(|id| ctx.store.borrow().quick_run_by_id_or_name(&id));
            dialogs::quick_run_editor(window, ctx.clone(), existing, Rc::new(|| ()));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BannerKind {
    None,
    Repair,
    Flatpak,
    Metered,
    Update,
    RcloneRestart,
    AppRestart,
    Development,
}

fn maybe_auto_prompt_repair(
    ctx: &AppCtx,
    window: &impl IsA<gtk::Widget>,
    toast: &adw::ToastOverlay,
    shown: &Rc<RefCell<HashSet<crate::repair::RepairKind>>>,
) {
    let settings = ctx.settings.borrow().clone();
    let version = ctx.client().and_then(|c| c.version().ok());
    let issues = crate::repair::diagnose(
        &settings,
        ctx.engine_ready(),
        ctx.client().as_ref(),
        version.as_deref(),
    );
    {
        let mut shown = shown.borrow_mut();
        crate::repair::reset_auto_prompt_if_clear(&mut shown, &issues);
        let Some(kind) =
            crate::repair::auto_prompt_kind(settings.core.completed_onboarding, &issues)
        else {
            return;
        };
        if !shown.insert(kind) {
            return;
        }
    }
    if crate::repair::banner_opens_password(&issues) {
        dialogs::password_prompt(window, ctx.clone(), toast.clone());
    } else {
        dialogs::repair(window, ctx.clone(), toast.clone());
    }
}

fn update_banner(ctx: &AppCtx, banner: &adw::Banner, kind: &Rc<std::cell::RefCell<BannerKind>>) {
    let settings = ctx.settings.borrow().clone();
    let version = ctx.client().and_then(|c| c.version().ok());
    let issues = crate::repair::diagnose(
        &settings,
        ctx.engine_ready(),
        ctx.client().as_ref(),
        version.as_deref(),
    );
    if settings.runtime.rclone_restart_required {
        banner.set_title(&ctx.t_or(
            "modals.about.rcloneRestartRequired",
            "Rclone Restart Required",
        ));
        banner.set_button_label(Some(&ctx.t_or("modals.about.restartNow", "Restart Now")));
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::RcloneRestart;
        return;
    }
    if settings.runtime.app_restart_required {
        banner.set_title(&ctx.t_or("modals.about.readyToRestart", "Restart Required"));
        banner.set_button_label(Some(&ctx.t_or("modals.about.restartNow", "Restart Now")));
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::AppRestart;
        return;
    }
    if let Some(issue) = crate::repair::banner_from_issues(&issues) {
        let title =
            if let Some((title_key, detail_key)) = crate::repair::engine_banner_keys(issue.kind) {
                let title = ctx.t_or(title_key, &issue.title);
                let detail = if issue.kind == crate::repair::RepairKind::VersionTooOld {
                    let required = crate::repair::MIN_RCLONE_VERSION;
                    let text = ctx.tf(detail_key, &[("required", required)]);
                    if text.contains("{{") {
                        issue.detail.clone()
                    } else {
                        text
                    }
                } else {
                    ctx.t_or(detail_key, &issue.detail)
                };
                format!("{title} — {detail}")
            } else {
                format!("{} — {}", issue.title, issue.detail)
            };
        banner.set_title(&title);
        banner.set_button_label(Some(&issue.action));
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::Repair;
        return;
    }
    if crate::platform::is_flatpak() && settings.runtime.flatpak_warn {
        banner.set_title(&format!(
            "{} — {}",
            ctx.t_or("banners.flatpak.title", "Flatpak Note"),
            ctx.t_or(
                "banners.flatpak.subtitle",
                "Some features may require manual permission adjustments.",
            )
        ));
        banner.set_button_label(Some(&ctx.t_or("banners.flatpak.dismissTooltip", "Dismiss")));
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::Flatpak;
        return;
    }
    if crate::platform::is_network_metered() {
        banner.set_title(&ctx.t_or(
            "banners.metered.message",
            "You are on a metered connection. Some features may use extra data.",
        ));
        banner.set_button_label(None::<&str>);
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::Metered;
        return;
    }
    let updates = ctx.updates.borrow().clone();
    if updates.has_updates() {
        let title = match updates.banner_kind() {
            "all" => ctx.t_or(
                "titlebar.updates.all",
                "Application and Rclone updates available",
            ),
            "rclone" => ctx.t_or("titlebar.updates.rclone", "Rclone update available"),
            _ => ctx.t_or("titlebar.updates.app", "Application update available"),
        };
        banner.set_title(&title);
        banner.set_button_label(Some(&ctx.t_or("modals.updates.title", "Updates")));
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::Update;
        return;
    }
    if cfg!(debug_assertions) {
        banner.set_title(&format!(
            "{} — {}",
            ctx.t_or("banners.development.title", "Development Build"),
            ctx.t_or(
                "banners.development.subtitle",
                "Features may be unstable • Data loss possible • Not for production use",
            )
        ));
        banner.set_button_label(None::<&str>);
        banner.set_revealed(true);
        *kind.borrow_mut() = BannerKind::Development;
        return;
    }
    banner.set_revealed(false);
    *kind.borrow_mut() = BannerKind::None;
}

fn sync_home_button(ctx: &AppCtx, btn: &gtk::Button) {
    btn.set_visible(ctx.selected_remote.borrow().is_some());
}

fn toggle_card_variant(ctx: &AppCtx) {
    let next = if ctx.settings.borrow().runtime.dashboard_card_variant == "detailed" {
        "compact"
    } else {
        "detailed"
    };
    ctx.settings.borrow_mut().runtime.dashboard_card_variant = next.into();
    ctx.persist();
}

fn card_variant_menu_label(ctx: &AppCtx) -> String {
    if ctx.settings.borrow().runtime.dashboard_card_variant == "detailed" {
        ctx.t_or("generalOverview.layout.showCompact", "Show Compact Cards")
    } else {
        ctx.t_or("generalOverview.layout.showDetailed", "Show Detailed Cards")
    }
}

fn sync_card_variant_button(ctx: &AppCtx, btn: &gtk::Button) {
    let detailed = ctx.settings.borrow().runtime.dashboard_card_variant == "detailed";
    btn.set_icon_name(if detailed {
        "view-grid-symbolic"
    } else {
        "view-list-symbolic"
    });
    btn.set_tooltip_text(Some(&card_variant_menu_label(ctx)));
}

fn menu_signature(ctx: &AppCtx) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        ctx.active_workspace.borrow(),
        menu_notice_text(ctx).unwrap_or_default(),
        ctx.store.borrow().unacknowledged_alerts(),
        u8::from(ctx.settings.borrow().runtime.rclone_restart_required),
        u8::from(ctx.settings.borrow().runtime.app_restart_required),
        ctx.settings.borrow().runtime.dashboard_card_variant,
    )
}

fn menu_notice_text(ctx: &AppCtx) -> Option<String> {
    let alerts = ctx.store.borrow().unacknowledged_alerts();
    if ctx.settings.borrow().runtime.rclone_restart_required
        || ctx.settings.borrow().runtime.app_restart_required
    {
        Some("!".into())
    } else if ctx.updates.borrow().has_updates() {
        Some("!".into())
    } else if alerts > 0 {
        Some(alerts.to_string())
    } else {
        None
    }
}

fn sync_menu_badge(ctx: &AppCtx, badge: &gtk::Label) {
    match menu_notice_text(ctx) {
        Some(text) => {
            badge.set_text(&text);
            badge.set_visible(true);
        }
        None => badge.set_visible(false),
    }
}

fn sync_notice_button(ctx: &AppCtx, btn: &gtk::Button) {
    let updates = ctx.updates.borrow().clone();
    let alerts = ctx.store.borrow().unacknowledged_alerts();
    if ctx.settings.borrow().runtime.app_restart_required {
        btn.set_visible(true);
        btn.set_icon_name("view-refresh-symbolic");
        btn.set_tooltip_text(Some(
            &ctx.t_or("modals.about.readyToRestart", "Restart Required"),
        ));
    } else if ctx.settings.borrow().runtime.rclone_restart_required {
        btn.set_visible(true);
        btn.set_icon_name("view-refresh-symbolic");
        btn.set_tooltip_text(Some(&ctx.t_or(
            "modals.about.rcloneRestartRequired",
            "Rclone Restart Required",
        )));
    } else if updates.has_updates() {
        btn.set_visible(true);
        btn.set_icon_name("software-update-available-symbolic");
        let tip = match updates.banner_kind() {
            "all" => ctx.t_or(
                "titlebar.updates.all",
                "Application and Rclone updates available",
            ),
            "rclone" => ctx.t_or("titlebar.updates.rclone", "Rclone update available"),
            _ => ctx.t_or("titlebar.updates.app", "Application update available"),
        };
        btn.set_tooltip_text(Some(&tip));
    } else if alerts > 0 {
        btn.set_visible(true);
        btn.set_icon_name("dialog-warning-symbolic");
        btn.set_tooltip_text(Some(&format!(
            "{} ({})",
            ctx.t_or("alerts.unacknowledged", "Unacknowledged"),
            alerts
        )));
    } else {
        btn.set_visible(false);
    }
}

fn sync_connection_button(ctx: &AppCtx, btn: &gtk::Button) {
    match *ctx.connection.borrow() {
        crate::connection::ConnectionStatus::Online => {
            btn.set_visible(false);
            btn.set_sensitive(true);
        }
        crate::connection::ConnectionStatus::Checking => {
            btn.set_visible(true);
            btn.set_sensitive(false);
            btn.set_icon_name("view-refresh-symbolic");
            btn.set_tooltip_text(Some(&ctx.t_or(
                "titlebar.connection.checking",
                "Checking internet connection...",
            )));
        }
        crate::connection::ConnectionStatus::Offline => {
            btn.set_visible(true);
            btn.set_sensitive(true);
            btn.set_icon_name("network-offline-symbolic");
            let services = ctx.connection_detail.borrow().clone();
            let tip = if services.is_empty() {
                ctx.t_or(
                    "titlebar.connection.offline",
                    "Cannot connect to some services. Click to retry.",
                )
            } else {
                ctx.tf("titlebar.connection.offline", &[("services", &services)])
            };
            btn.set_tooltip_text(Some(&tip));
        }
    }
}

fn apply_startup_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".startup-loading { background-color: @window_bg_color; }\n\
         .file-item-menu { min-width: 48px; min-height: 48px; padding: 8px; }\n\
         .file-item-menu-hit {\n\
           min-width: 48px;\n\
           min-height: 48px;\n\
           background-color: alpha(@window_bg_color, 0.82);\n\
           border-radius: 24px;\n\
         }\n\
         .path-fade-start {\n\
           min-width: 28px;\n\
           background-image: linear-gradient(to right, @window_bg_color, transparent);\n\
         }\n\
         .path-fade-end {\n\
           min-width: 28px;\n\
           background-image: linear-gradient(to left, @window_bg_color, transparent);\n\
         }\n\
         .tab-fade-start {\n\
           min-width: 28px;\n\
           background-image: linear-gradient(to right, @window_bg_color, transparent);\n\
         }\n\
         .tab-fade-end {\n\
           min-width: 28px;\n\
           background-image: linear-gradient(to left, @window_bg_color, transparent);\n\
         }\n\
         .file-drag-over {\n\
           background-color: alpha(@accent_bg_color, 0.35);\n\
           outline: 2px solid alpha(@accent_color, 0.65);\n\
           outline-offset: -2px;\n\
         }\n\
         .star-button {\n\
           min-width: 32px;\n\
           min-height: 32px;\n\
           padding: 4px;\n\
         }\n\
         levelbar.disk-usage-critical trough block.filled {\n\
           background-color: @destructive_bg_color;\n\
         }\n\
         levelbar.disk-usage-high trough block.filled {\n\
           background-color: mix(@warning_bg_color, @destructive_bg_color, 0.5);\n\
         }\n\
         levelbar.disk-usage-warning trough block.filled {\n\
           background-color: @warning_bg_color;\n\
         }\n\
         levelbar.disk-usage-healthy trough block.filled {\n\
           background-color: @success_bg_color;\n\
         }\n\
         .cut-item { opacity: 0.5; }\n\
         .cut-icon {\n\
           outline: 2px dashed alpha(@window_fg_color, 0.45);\n\
           outline-offset: -2px;\n\
           border-radius: 8px;\n\
         }\n\
         .nautilus-toolbar-scroll { min-width: 0; }\n\
         .nautilus-toolbar { min-width: 0; }\n\
         .speed-dot {\n\
           min-width: 8px;\n\
           min-height: 8px;\n\
           border-radius: 999px;\n\
           background-color: @warning_color;\n\
         }\n\
         .speed-dot.speed-fast { background-color: @success_color; }\n\
         .speed-dot.speed-medium { background-color: @accent_color; }\n\
         .speed-dot.speed-slow { background-color: @warning_color; }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[allow(dead_code)]
pub fn open_view(stack: &adw::ViewStack, view: MainView) {
    stack.set_visible_child_name(view.as_str());
}

fn install_debug_context_menu(window: &adw::ApplicationWindow, ctx: &AppCtx) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.set_propagation_phase(gtk::PropagationPhase::Bubble);
    {
        let window = window.clone();
        let ctx = ctx.clone();
        gesture.connect_pressed(move |g, n_press, x, y| {
            if n_press != 1 {
                return;
            }
            let Some(host) = g.widget() else {
                return;
            };
            let Some(target) = host.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            let mut names = Vec::new();
            let mut current = Some(target);
            while let Some(widget) = current {
                names.push(widget.type_().name().to_string());
                current = widget.parent();
            }
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let items = crate::debug_menu::items_for_ancestry(&refs);
            if items.is_empty() {
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            let popover = gtk::Popover::new();
            popover.set_autohide(true);
            popover.set_has_arrow(false);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            list.set_margin_top(6);
            list.set_margin_bottom(6);
            list.set_margin_start(6);
            list.set_margin_end(6);
            list.add_css_class("material-context-menu");
            for item in items {
                let btn = gtk::Button::with_label(&ctx.t_or(item.i18n_key(), item.fallback()));
                btn.add_css_class("flat");
                btn.set_halign(gtk::Align::Fill);
                let action = item.action_name().to_string();
                let popover = popover.clone();
                let window = window.clone();
                btn.connect_clicked(move |_| {
                    popover.popdown();
                    let _ = gtk::prelude::WidgetExt::activate_action(&window, &action, None);
                });
                list.append(&btn);
            }
            popover.set_child(Some(&list));
            popover.set_parent(&window);
            popover.popup();
        });
    }
    window.add_controller(gesture);
}
