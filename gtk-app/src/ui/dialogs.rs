use super::AppCtx;
use crate::backup;
use crate::guidance::{operation_banners, BannerKind};
use crate::jobs::{
    assemble_rclone, build_job_params, default_dest, default_source, flatten_rclone,
    job_from_status, merge_template_into, path_list, start_request, SOURCE_KEYS,
};
use crate::operations::OperationType;
use crate::rclone::{
    browse_target, describe_cron_i18n, group_metadata_info, nanoseconds_to_duration, parse_hashsum,
    parse_hashsum_list, public_link_expiry_value, remote_fs, validate_cron,
};
use crate::rename::{preview as rename_preview, RenameMode, RenamePlan};
use crate::store::{
    AlertAction, AlertEvent, AlertEventKind, AlertRule, AlertSeverity, ProfileConfig, QuickRun,
};
use adw::prelude::*;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub(super) fn attach_operation_guidance(
    ctx: &AppCtx,
    is_new_remote: bool,
    watch: &adw::SwitchRow,
    src: &adw::EntryRow,
    extra_sources: &Rc<RefCell<Vec<adw::EntryRow>>>,
    dst: &adw::EntryRow,
    current_op: Rc<dyn Fn() -> OperationType>,
) -> (adw::PreferencesGroup, Rc<dyn Fn()>) {
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("remoteConfig.guidance", "Guidance"));
    let refresh = {
        let ctx = ctx.clone();
        let group = group.clone();
        let watch = watch.clone();
        let src = src.clone();
        let extra_sources = extra_sources.clone();
        let dst = dst.clone();
        let current_op = current_op.clone();
        Rc::new(move || {
            let mut sources = vec![src.text().to_string()];
            sources.extend(
                extra_sources
                    .borrow()
                    .iter()
                    .map(|row| row.text().to_string()),
            );
            let banners = operation_banners(
                current_op(),
                is_new_remote,
                watch.is_active(),
                &sources,
                &dst.text(),
            );
            while let Some(child) = group.first_child() {
                group.remove(&child);
            }
            for banner in &banners {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or(banner.key, banner.key));
                row.set_subtitle(&ctx.t_or(
                    match banner.kind {
                        BannerKind::Warning => "common.warning",
                        BannerKind::Info => "common.info",
                    },
                    match banner.kind {
                        BannerKind::Warning => "Warning",
                        BannerKind::Info => "Note",
                    },
                ));
                if banner.kind == BannerKind::Warning {
                    row.add_css_class("warning");
                }
                group.add(&row);
            }
            group.set_visible(!banners.is_empty());
        }) as Rc<dyn Fn()>
    };
    refresh();
    {
        let refresh = refresh.clone();
        watch.connect_active_notify(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        src.connect_changed(move |_| refresh());
    }
    {
        let refresh = refresh.clone();
        dst.connect_changed(move |_| refresh());
    }
    for row in extra_sources.borrow().iter() {
        let refresh = refresh.clone();
        row.connect_changed(move |_| refresh());
    }
    (group, refresh)
}

fn try_spawn_standalone(ctx: &AppCtx, kind: &str, data: serde_json::Value) -> bool {
    if crate::platform::is_standalone_dialog() {
        return false;
    }
    if !ctx.settings.borrow().general.standalone_dialogs {
        return false;
    }
    let Ok((_child, path)) = crate::platform::spawn_standalone_dialog(kind, &data) else {
        return false;
    };
    let ctx = ctx.clone();
    let mut ticks = 0u32;
    glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
        ticks += 1;
        if path.exists() {
            if crate::platform::read_dialog_result(&path).is_some() {
                ctx.refresh_runtime();
                ctx.persist();
            }
            return glib::ControlFlow::Break;
        }
        if ticks > 300 {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    true
}

pub(super) fn settings_list(
    ctx: &AppCtx,
    settings: &serde_json::Value,
    limit: usize,
) -> gtk::ListBox {
    let restrict = ctx.settings.borrow().general.restrict;
    let entries = crate::restrict::flatten_settings("", settings, restrict);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    if entries.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(
            "detailShared.settings.noData",
            "No configuration data available",
        ));
        row.set_subtitle(&ctx.t_or("detailShared.settings.notConfigured", "Not configured"));
        list.append(&row);
        return list;
    }
    let count = adw::ActionRow::new();
    count.set_title(&ctx.tf(
        "detailShared.settings.metrics",
        &[("count", &entries.len().to_string())],
    ));
    list.append(&count);
    for (key, value) in entries.into_iter().take(limit) {
        let row = adw::ActionRow::new();
        row.set_title(&key);
        row.set_subtitle(&value);
        list.append(&row);
    }
    list
}

pub fn present_standalone(
    app: &adw::Application,
    ctx: AppCtx,
    req: crate::platform::DialogRequest,
) {
    crate::platform::set_standalone_dialog(true);
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&req.kind)
        .default_width(780)
        .default_height(640)
        .build();
    let toast = adw::ToastOverlay::new();
    window.set_content(Some(&toast));
    window.present();
    let remote = req
        .data
        .get("remote")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let path = req
        .data
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = req
        .data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let jobid = req.data.get("jobid").and_then(|v| v.as_u64()).unwrap_or(0);
    let noop = Rc::new(|| ());
    match req.kind.as_str() {
        "preferences" => preferences(&window, ctx.clone()),
        "about" => about(&window, ctx.clone()),
        "logs" => logs(
            &window,
            ctx.clone(),
            if remote.is_empty() {
                None
            } else {
                Some(remote.clone())
            },
        ),
        "export" => export_backup(&window, ctx.clone(), toast, None),
        "backend" => backends(&window, ctx.clone()),
        "rclone-flags" => rclone_flags(&window, ctx.clone()),
        "job-detail" => job_detail(&window, ctx.clone(), jobid),
        "properties" => properties(&window, ctx.clone(), &remote, &path, &name),
        "remote-about" => remote_about(&window, ctx.clone(), &remote),
        "keyboard-shortcuts" => shortcuts(&window, &ctx),
        "alerts" => alerts(&window, ctx.clone()),
        "archive-create" => archive_create(&window, ctx.clone(), &remote, &path, &[]),
        "quick-run-editor" => {
            let existing = serde_json::from_value::<QuickRun>(req.data.clone()).ok();
            quick_run_editor(&window, ctx.clone(), existing, noop);
        }
        "template-manager" => templates(&window, ctx.clone()),
        "delete-remote" => delete_remote(&window, ctx.clone(), &remote, noop),
        "remote-config" => remote_config_open(
            &window,
            ctx.clone(),
            if remote.is_empty() {
                None
            } else {
                Some(remote)
            },
            super::remote_config::RemoteConfigOpen {
                initial: req
                    .data
                    .get("initial")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
                profile: req
                    .data
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
                auto_add: req
                    .data
                    .get("autoAdd")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            noop,
        ),
        "vfs" => vfs_control(&window, ctx.clone(), &remote, toast.clone()),
        "repair" => repair(&window, ctx.clone(), toast.clone()),
        "start-operation" => {
            let op = req
                .data
                .get("operation")
                .and_then(|v| v.as_str())
                .and_then(OperationType::parse)
                .unwrap_or(OperationType::Sync);
            start_operation(&window, ctx.clone(), &remote, op, toast.clone(), noop);
        }
        "file-viewer" => file_viewer(
            &window,
            ctx.clone(),
            &remote,
            &path,
            &name,
            req.data
                .get("isDir")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            &[],
        ),
        "quick-add-remote" => quick_add_remote(&window, ctx.clone(), noop),
        "restore-preview" => {
            let backup = req
                .data
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            restore_preview(
                &window,
                ctx.clone(),
                toast.clone(),
                std::path::PathBuf::from(backup),
                noop,
            );
        }
        _ => {}
    }
    let result_path = req.result_path.clone();
    let kind = req.kind.clone();
    window.connect_close_request(move |_| {
        let _ = crate::platform::write_dialog_result(
            result_path.as_deref(),
            true,
            &kind,
            serde_json::json!({}),
        );
        glib::Propagation::Proceed
    });
}

pub fn pick_destination(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    title: &str,
    initial: &str,
    on_ok: impl Fn(String) + 'static,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(480);
    let dest = adw::EntryRow::new();
    dest.set_title(&ctx.t_or(
        "fileBrowser.operations.details.destination",
        "Destination path",
    ));
    dest.set_text(initial);
    attach_path_picker(ctx, &dest, crate::picker::FilePickerConfig::folders());
    let ok = gtk::Button::with_label(&ctx.t_or("common.ok", "OK"));
    ok.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(&ctx.t("common.cancel"));
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let dialog = dialog.clone();
        let dest = dest.clone();
        ok.connect_clicked(move |_| {
            on_ok(dest.text().to_string());
            dialog.close();
        });
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(8);
    buttons.append(&cancel);
    buttons.append(&ok);
    let group = adw::PreferencesGroup::new();
    group.add(&dest);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.append(&group);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn prompt(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    title: &str,
    label: &str,
    initial: &str,
    on_ok: impl Fn(String) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(label));
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", &ctx.t("common.cancel"));
    dialog.add_response("ok", &ctx.t_or("common.ok", "OK"));
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response == "ok" {
            on_ok(entry.text().to_string());
        }
    });
    dialog.present(Some(parent));
}

pub fn confirm_shutdown(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_quit: impl Fn() + 'static) {
    if ctx.shutdown_prompt_open.get() {
        return;
    }
    let summary = {
        let snap = ctx.snapshot.borrow();
        crate::jobs::shutdown_summary(&snap.jobs, snap.mounts.len(), snap.serves.len())
    };
    if !summary.active() {
        present_shutdown_overlay(parent, &ctx);
        perform_shutdown(&ctx);
        on_quit();
        return;
    }
    ctx.shutdown_prompt_open.set(true);
    let title = ctx.t_or("app.shutdown.confirmTitle", "Active Tasks in Progress");
    let jobs = summary.jobs.to_string();
    let mounts = summary.mounts.to_string();
    let serves = summary.serves.to_string();
    let message = ctx.tf(
        "app.shutdown.confirmMessage",
        &[("jobs", &jobs), ("mounts", &mounts), ("serves", &serves)],
    );
    let cancel = ctx.t_or("common.cancel", "Cancel");
    let quit = ctx.t_or("app.shutdown.stopAndQuit", "Stop & Quit");
    let dialog = adw::AlertDialog::new(Some(&title), Some(&message));
    dialog.add_response("cancel", &cancel);
    dialog.add_response("quit", &quit);
    dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let parent_for_overlay = parent.clone();
    dialog.connect_response(None, move |_, response| {
        ctx.shutdown_prompt_open.set(false);
        if response == "quit" {
            present_shutdown_overlay(&parent_for_overlay, &ctx);
            perform_shutdown(&ctx);
            on_quit();
        }
    });
    dialog.present(Some(parent));
}

fn present_shutdown_overlay(parent: &impl IsA<gtk::Widget>, ctx: &AppCtx) {
    let title = ctx.t_or("app.shutdown.title", "Shutting Down");
    let message = ctx.t_or(
        "app.shutdown.message",
        "Please wait while the application shuts down safely...",
    );
    let dialog = adw::AlertDialog::new(Some(&title), Some(&message));
    dialog.set_close_response("wait");
    dialog.present(Some(parent));
    while gtk::glib::MainContext::default().iteration(false) {}
}

pub fn perform_shutdown(ctx: &AppCtx) {
    if let Some(client) = ctx.client() {
        let snap = ctx.snapshot.borrow();
        crate::jobs::stop_all_runtime(&client, &snap.jobs, &snap.mounts);
    }
    ctx.inhibitor.borrow_mut().release();
    if let Some(mut engine) = ctx.engine.borrow_mut().take() {
        engine.shutdown();
    }
}

pub fn preferences(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    if try_spawn_standalone(&ctx, "preferences", serde_json::json!({})) {
        return;
    }
    super::preferences::present(parent, ctx);
}

pub fn about(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    if try_spawn_standalone(&ctx, "about", serde_json::json!({})) {
        return;
    }
    let version = ctx
        .engine
        .borrow()
        .as_ref()
        .map(|e| e.version.clone())
        .unwrap_or_default();
    let app_update = crate::updater::fetch_app_update(env!("CARGO_PKG_VERSION"))
        .ok()
        .filter(|u| u.available)
        .map(|u| format!("App update {} available", u.latest))
        .unwrap_or_else(|| ctx.t_or("modals.about.upToDate", "App is up to date"));
    let rclone_update = crate::updater::fetch_rclone_update(&version)
        .ok()
        .filter(|u| u.available)
        .map(|u| format!("rclone update {} available", u.latest))
        .unwrap_or_else(|| ctx.t_or("modals.about.rcloneUpToDate", "rclone is up to date"));
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("modals.about.title", "About"));
    dialog.set_content_width(560);
    dialog.set_content_height(560);
    let stack = adw::ViewStack::new();

    let details = gtk::Box::new(gtk::Orientation::Vertical, 12);
    details.set_margin_top(16);
    details.set_margin_start(16);
    details.set_margin_end(16);
    let title = gtk::Label::new(Some("Rclone Manager"));
    title.add_css_class("title-1");
    let comments = gtk::Label::new(Some(&format!(
        "GTK 4 + libadwaita · {} · rclone {version}\n{app_update}\n{rclone_update}",
        env!("CARGO_PKG_VERSION")
    )));
    comments.set_wrap(true);
    comments.set_justify(gtk::Justification::Center);
    details.append(&title);
    details.append(&comments);
    let site = gtk::LinkButton::with_label(
        "https://github.com/Zarestia-Dev/rclone-manager",
        &ctx.t_or("modals.about.website", "Website"),
    );
    let issues = gtk::LinkButton::with_label(
        "https://github.com/Zarestia-Dev/rclone-manager/issues",
        &ctx.t_or("modals.about.reportIssues", "Report Issues"),
    );
    details.append(&site);
    details.append(&issues);
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let notes = gtk::Button::with_label(&ctx.t_or("modals.about.whatsNew", "What's New"));
        notes.connect_clicked(move |_| whats_new(&parent, ctx.clone(), "app"));
        details.append(&notes);
    }
    stack.add_titled(
        &details,
        Some("details"),
        &ctx.t_or("modals.about.details", "Details"),
    );

    let credits = gtk::Box::new(gtk::Orientation::Vertical, 8);
    credits.set_margin_top(16);
    credits.set_margin_start(16);
    credits.set_margin_end(16);
    let team = adw::PreferencesGroup::new();
    team.set_title(&ctx.t_or("modals.about.devTeam", "Development Team"));
    let lead = adw::ActionRow::new();
    lead.set_title(&ctx.t_or("modals.about.leadDeveloper", "Lead Developer"));
    lead.set_subtitle("Zarestia Dev");
    team.add(&lead);
    let ack = adw::PreferencesGroup::new();
    ack.set_title(&ctx.t_or("modals.about.acknowledgments", "Acknowledgments"));
    let ack_row = adw::ActionRow::new();
    ack_row.set_title(&ctx.t_or(
        "modals.about.ackText",
        "This application relies on the excellent Rclone project for cloud storage management.",
    ));
    ack_row.set_subtitle_lines(4);
    ack.add(&ack_row);
    credits.append(&team);
    credits.append(&ack);
    stack.add_titled(
        &credits,
        Some("credits"),
        &ctx.t_or("modals.about.credits", "Credits"),
    );

    let legal = gtk::Box::new(gtk::Orientation::Vertical, 8);
    legal.set_margin_top(16);
    legal.set_margin_start(16);
    legal.set_margin_end(16);
    let license = adw::PreferencesGroup::new();
    license.set_title(&ctx.t_or("modals.about.license", "License"));
    let license_row = adw::ActionRow::new();
    license_row.set_title("GPL-3.0-or-later");
    license_row.set_subtitle(&format!(
        "{} GNU GPL v3 {} {}",
        ctx.t_or(
            "modals.about.licenseText1",
            "This application is free and open source software distributed under the"
        ),
        ctx.t_or("modals.about.orLater", "or later."),
        ctx.t_or(
            "modals.about.licenseText2",
            "This program comes with no warranty."
        )
    ));
    license_row.set_subtitle_lines(6);
    license.add(&license_row);
    let third = adw::PreferencesGroup::new();
    third.set_title(&ctx.t_or("modals.about.thirdParty", "Third-Party Software"));
    let third_row = adw::ActionRow::new();
    third_row.set_title(&ctx.t_or(
        "modals.about.thirdPartyText",
        "This application includes third-party libraries. See the project repository for a complete list.",
    ));
    third_row.set_subtitle_lines(4);
    third.add(&third_row);
    let gpl =
        gtk::LinkButton::with_label("https://www.gnu.org/licenses/gpl-3.0.html", "GNU GPL v3");
    legal.append(&license);
    legal.append(&third);
    legal.append(&gpl);
    stack.add_titled(
        &legal,
        Some("legal"),
        &ctx.t_or("modals.about.legal", "Legal"),
    );

    let system = gtk::Box::new(gtk::Orientation::Vertical, 8);
    system.set_margin_top(16);
    system.set_margin_start(16);
    system.set_margin_end(16);
    let engine_group = adw::PreferencesGroup::new();
    engine_group.set_title(&ctx.t_or("modals.about.engine", "Backend Infrastructure"));
    let pid_row = adw::ActionRow::new();
    pid_row.set_title(&ctx.t_or("dashboard.system.pid", "rclone PID"));
    let pid = ctx
        .client()
        .and_then(|c| c.pid().ok())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "—".into());
    pid_row.set_subtitle(&pid);
    engine_group.add(&pid_row);
    let toast = adw::ToastOverlay::new();
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let quit =
            gtk::Button::with_label(&ctx.t_or("modals.about.killRclone", "Quit rclone engine"));
        quit.add_css_class("destructive-action");
        quit.set_halign(gtk::Align::Start);
        quit.connect_clicked(move |_| {
            let local = ctx.settings.borrow().core.active_backend.is_empty()
                || ctx.settings.borrow().core.active_backend == "local";
            if local {
                if ctx.engine.borrow().is_none() {
                    toast.add_toast(adw::Toast::new(
                        &ctx.t_or("modals.about.noProcessToKill", "No rclone process to kill"),
                    ));
                    return;
                }
                *ctx.engine.borrow_mut() = None;
                ctx.refresh_runtime();
                toast.add_toast(adw::Toast::new(&ctx.t_or(
                    "modals.about.killSuccess",
                    "Rclone process killed successfully",
                )));
            } else if let Some(client) = ctx.client() {
                match client.quit() {
                    Ok(_) => toast.add_toast(adw::Toast::new(&ctx.t_or(
                        "modals.about.killSuccess",
                        "Rclone process killed successfully",
                    ))),
                    Err(_) => toast.add_toast(adw::Toast::new(
                        &ctx.t_or("modals.about.killFailed", "Failed to kill rclone process"),
                    )),
                }
            } else {
                toast.add_toast(adw::Toast::new(
                    &ctx.t_or("modals.about.noProcessToKill", "No rclone process to kill"),
                ));
            }
        });
        system.append(&engine_group);
        system.append(&quit);
    }
    let cache_group = adw::PreferencesGroup::new();
    cache_group.set_title(&ctx.t_or("modals.about.backendCache", "Backend Cache"));
    let cache_row = adw::ActionRow::new();
    cache_row.set_title(&ctx.t_or("modals.about.entries", "Active Entries"));
    let cache_count = ctx
        .client()
        .and_then(|c| c.fscache_entries().ok())
        .map(|v| crate::rclone::parse_fscache_entry_count(&v).to_string())
        .unwrap_or_else(|| "—".into());
    cache_row.set_subtitle(&cache_count);
    cache_group.add(&cache_row);
    system.append(&cache_group);
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let cache_row = cache_row.clone();
        let clear = gtk::Button::with_label(&ctx.t_or("modals.about.clear", "Clear"));
        clear.set_halign(gtk::Align::Start);
        clear.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                match client.fscache_clear() {
                    Ok(_) => {
                        let count = client
                            .fscache_entries()
                            .ok()
                            .map(|v| crate::rclone::parse_fscache_entry_count(&v).to_string())
                            .unwrap_or_else(|| "0".into());
                        cache_row.set_subtitle(&count);
                        toast.add_toast(adw::Toast::new(
                            &ctx.t_or("modals.about.cacheCleared", "Cache cleared successfully"),
                        ));
                    }
                    Err(_) => toast.add_toast(adw::Toast::new(
                        &ctx.t_or("modals.about.cacheClearFailed", "Failed to clear cache"),
                    )),
                }
            }
        });
        system.append(&clear);
    }
    let build = crate::platform::managed_build();
    if let Some(command) = crate::platform::update_command(build) {
        let cmd_row = adw::ActionRow::new();
        cmd_row.set_title(&ctx.t_or(
            "modals.about.managedBuildNotice",
            "This build cannot be updated by the app updater.",
        ));
        cmd_row.set_subtitle(command);
        system.append(&cmd_row);
    }
    if let Some(url) = crate::platform::update_page_url(build) {
        let label = if matches!(build, crate::platform::ManagedBuild::Flatpak) {
            ctx.t_or("modals.about.openFlathub", "Open Flathub")
        } else {
            ctx.t_or("modals.about.downloadPage", "Download Page")
        };
        system.append(&gtk::LinkButton::with_label(url, &label));
    }
    stack.add_titled(
        &system,
        Some("system"),
        &ctx.t_or("generalOverview.panels.system", "System"),
    );

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    toast.set_child(Some(&toolbar));
    dialog.set_child(Some(&toast));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn whats_new(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, kind: &str) {
    let dialog = adw::Dialog::new();
    let app_title = ctx.t_or("modals.about.whatsNew", "What's New");
    let rclone_title = ctx.t_or("modals.about.whatsNewRclone", "What's New in rclone");
    dialog.set_title(if kind == "rclone" {
        &rclone_title
    } else {
        &app_title
    });
    dialog.set_content_width(640);
    dialog.set_content_height(520);
    let pending = ctx.updates.borrow().clone();
    let (notes, url) = if kind == "rclone" {
        (
            pending
                .rclone
                .as_ref()
                .and_then(|u| u.notes.clone())
                .or_else(|| crate::updater::fetch_rclone_release_notes().ok()),
            pending
                .rclone
                .as_ref()
                .map(|u| u.url.clone())
                .unwrap_or_else(|| "https://rclone.org/changelog/".into()),
        )
    } else {
        (
            pending
                .app
                .as_ref()
                .and_then(|u| u.notes.clone())
                .or_else(|| crate::updater::fetch_app_release_notes().ok()),
            pending
                .app
                .as_ref()
                .map(|u| u.url.clone())
                .unwrap_or_else(|| {
                    "https://github.com/Zarestia-Dev/rclone-manager/releases".into()
                }),
        )
    };
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.buffer().set_text(&notes.unwrap_or_else(|| {
        ctx.t_or(
            "modals.logs.noLogsFound",
            "No release notes are available yet.",
        )
    }));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&view));
    let open = gtk::LinkButton::with_label(&url, &ctx.t_or("common.open", "Open in browser"));
    open.set_halign(gtk::Align::Start);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_bottom(12);
    box_.append(&scroll);
    box_.append(&open);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn memory_stats(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("modals.about.memory", "rclone memory"));
    dialog.set_content_width(420);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    if let Some(mem) = ctx.client().and_then(|c| c.memstats().ok()) {
        for (key, i18n_key, fallback) in [
            ("Alloc", "modals.about.memAlloc", "Allocated"),
            ("Sys", "modals.about.memSys", "System"),
            ("HeapAlloc", "modals.about.memHeapAlloc", "Heap"),
            ("NumGC", "modals.about.gc", "GC cycles"),
        ] {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or(i18n_key, fallback));
            let value = mem.get(key).cloned().unwrap_or(serde_json::json!(0));
            row.set_subtitle(&if key == "NumGC" {
                value.to_string()
            } else {
                crate::rclone::format_bytes(value.as_i64().unwrap_or(0))
            });
            list.append(&row);
        }
    } else {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(
            "titlebar.menu.memoryUnavailable",
            "Memory stats unavailable",
        ));
        list.append(&row);
    }
    let gc = gtk::Button::with_label(&ctx.t_or("titlebar.menu.runGc", "Run GC"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let dialog = dialog.clone();
        gc.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.gc();
            }
            dialog.close();
            memory_stats(&parent, ctx.clone());
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&scrolled_list(&list));
    box_.append(&gc);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn updates(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("modals.updates.title", "Updates"));
    dialog.set_content_width(520);
    dialog.set_content_height(420);
    let pending = ctx.updates.borrow().clone();
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("modals.updates.title", "Updates"));

    let app_row = adw::ActionRow::new();
    app_row.set_title(&ctx.t_or("titlebar.updates.app", "Application update"));
    if let Some(info) = pending.app.clone() {
        app_row.set_subtitle(&format!("{} → {}", info.current, info.latest));
        let notes = gtk::Button::with_label(&ctx.t_or("modals.about.whatsNew", "What's New"));
        notes.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            notes.connect_clicked(move |_| {
                whats_new(&parent, ctx.clone(), "app");
            });
        }
        app_row.add_suffix(&notes);
        if info.download_url.is_some() {
            let install = gtk::Button::with_label(&ctx.t_or("common.install", "Install"));
            install.add_css_class("suggested-action");
            install.set_valign(gtk::Align::Center);
            {
                let ctx = ctx.clone();
                let parent = parent.clone();
                let toast = toast.clone();
                let dialog = dialog.clone();
                install.connect_clicked(move |_| {
                    start_app_update(&parent, ctx.clone(), toast.clone());
                    dialog.close();
                });
            }
            app_row.add_suffix(&install);
        }
        let skip = gtk::Button::with_label(&ctx.t_or("modals.about.skipVersion", "Skip"));
        skip.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let latest = info.latest.clone();
            skip.connect_clicked(move |_| {
                ctx.settings
                    .borrow_mut()
                    .runtime
                    .app_skipped_updates
                    .push(latest.clone());
                ctx.persist();
                ctx.refresh_updates();
            });
        }
        app_row.add_suffix(&skip);
    } else {
        app_row.set_subtitle(&ctx.t_or("modals.about.upToDate", "Up to date"));
    }
    group.add(&app_row);

    let rclone_row = adw::ActionRow::new();
    rclone_row.set_title(&ctx.t_or("titlebar.updates.rclone", "Rclone update"));
    if let Some(info) = pending.rclone.clone() {
        rclone_row.set_subtitle(&format!("{} → {}", info.current, info.latest));
        let notes = gtk::Button::with_label(&ctx.t_or("modals.about.whatsNewRclone", "What's New"));
        notes.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            notes.connect_clicked(move |_| {
                whats_new(&parent, ctx.clone(), "rclone");
            });
        }
        rclone_row.add_suffix(&notes);
        let install = gtk::Button::with_label(&ctx.t_or("titlebar.menu.installRclone", "Install"));
        install.add_css_class("suggested-action");
        install.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let toast = toast.clone();
            let dialog = dialog.clone();
            install.connect_clicked(move |_| {
                start_rclone_update(&parent, ctx.clone(), toast.clone());
                dialog.close();
            });
        }
        rclone_row.add_suffix(&install);
        let skip = gtk::Button::with_label(&ctx.t_or("modals.about.skipVersion", "Skip"));
        skip.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let latest = info.latest.clone();
            skip.connect_clicked(move |_| {
                ctx.settings
                    .borrow_mut()
                    .runtime
                    .rclone_skipped_updates
                    .push(latest.clone());
                ctx.persist();
                ctx.refresh_updates();
            });
        }
        rclone_row.add_suffix(&skip);
    } else {
        rclone_row.set_subtitle(&ctx.t_or("modals.about.upToDate", "Up to date"));
    }
    group.add(&rclone_row);

    let refresh = gtk::Button::with_label(&ctx.t_or("common.refresh", "Check again"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let toast = toast.clone();
        let dialog = dialog.clone();
        refresh.connect_clicked(move |_| {
            ctx.refresh_updates();
            dialog.close();
            updates(&parent, ctx.clone(), toast.clone());
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.append(&group);
    box_.append(&refresh);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn start_app_update(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let Some(url) = ctx
        .updates
        .borrow()
        .app
        .as_ref()
        .and_then(|u| u.download_url.clone())
    else {
        toast.add_toast(adw::Toast::new(
            "No download is available for this platform",
        ));
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        toast.add_toast(adw::Toast::new(&ctx.t_or(
            "modals.about.killFailed",
            "Cannot locate the running binary",
        )));
        return;
    };
    run_download_job(
        parent,
        ctx,
        toast,
        "Installing application update",
        move |cancel, progress| crate::updater::install_app_update(&url, &exe, cancel, progress),
        |ctx, path, toast| {
            toast.add_toast(adw::Toast::new(&format!("Installed {}", path.display())));
            if let Err(e) = crate::platform::relaunch() {
                ctx.notify("Relaunch failed", &e);
            } else {
                std::process::exit(0);
            }
        },
    );
}

fn start_rclone_update(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dest = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".local/bin");
    run_download_job(
        parent,
        ctx,
        toast,
        "Installing rclone",
        move |cancel, progress| crate::updater::install_rclone_binary_ex(&dest, cancel, progress),
        |ctx, path, toast| {
            ctx.settings.borrow_mut().core.rclone_binary = path.to_string_lossy().into_owned();
            ctx.persist();
            toast.add_toast(adw::Toast::new(&format!(
                "Installed rclone to {}",
                path.display()
            )));
        },
    );
}

fn run_download_job<F, OnOk>(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    title: &str,
    work: F,
    on_ok: OnOk,
) where
    F: FnOnce(
            Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
            Option<std::sync::Arc<std::sync::Mutex<crate::updater::DownloadProgress>>>,
        ) -> Result<std::path::PathBuf, String>
        + Send
        + 'static,
    OnOk: FnOnce(AppCtx, std::path::PathBuf, adw::ToastOverlay) + 'static,
{
    run_progress_job(parent, ctx, toast, title, work, on_ok);
}

fn run_progress_job<T, F, OnOk>(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    title: &str,
    work: F,
    on_ok: OnOk,
) where
    T: Send + 'static,
    F: FnOnce(
            Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
            Option<std::sync::Arc<std::sync::Mutex<crate::updater::DownloadProgress>>>,
        ) -> Result<T, String>
        + Send
        + 'static,
    OnOk: FnOnce(AppCtx, T, adw::ToastOverlay) + 'static,
{
    let downloading = ctx.t_or("modals.about.downloading", "Downloading…");
    let dialog = adw::AlertDialog::new(Some(title), Some(&downloading));
    dialog.add_response("cancel", &ctx.t_or("common.cancel", "Cancel"));
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    dialog.set_extra_child(Some(&bar));
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let progress = std::sync::Arc::new(std::sync::Mutex::new(
        crate::updater::DownloadProgress::default(),
    ));
    let result: std::sync::Arc<std::sync::Mutex<Option<Result<T, String>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    {
        let cancel = cancel.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "cancel" {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
    dialog.present(Some(parent));
    {
        let cancel = cancel.clone();
        let progress = progress.clone();
        let result = result.clone();
        std::thread::spawn(move || {
            let outcome = work(Some(cancel), Some(progress));
            if let Ok(mut slot) = result.lock() {
                *slot = Some(outcome);
            }
        });
    }
    let ctx_done = ctx.clone();
    let toast_done = toast.clone();
    let bar_poll = bar.clone();
    let dialog_poll = dialog.clone();
    let on_ok = std::rc::Rc::new(std::cell::RefCell::new(Some(on_ok)));
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        if let Ok(guard) = progress.lock() {
            bar_poll.set_fraction(guard.fraction());
            bar_poll.set_text(Some(&guard.label()));
        }
        let finished = result.lock().ok().and_then(|mut slot| slot.take());
        if let Some(outcome) = finished {
            dialog_poll.close();
            match outcome {
                Ok(value) => {
                    if let Some(cb) = on_ok.borrow_mut().take() {
                        cb(ctx_done.clone(), value, toast_done.clone());
                    }
                }
                Err(e) if e == "cancelled" => {
                    toast_done.add_toast(adw::Toast::new(
                        &ctx_done.t_or("rcloneUpdate.cancelled", "Update cancelled"),
                    ));
                }
                Err(e) => {
                    toast_done.add_toast(adw::Toast::new(&e));
                }
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

pub fn shortcuts(parent: &impl IsA<gtk::Widget>, ctx: &AppCtx) {
    if try_spawn_standalone(ctx, "keyboard-shortcuts", serde_json::json!({})) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("shortcuts.title", "Keyboard Shortcuts"));
    dialog.set_content_width(560);
    dialog.set_content_height(560);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&ctx.t_or(
        "shortcuts.searchPlaceholder",
        "Search shortcuts or descriptions...",
    )));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let entries = [
        (
            "global",
            "shortcuts.categories.global",
            "Global",
            &[
                ("Ctrl+Q", "shortcuts.actions.quit", "Quit Application"),
                (
                    "Ctrl+Shift+?",
                    "shortcuts.actions.showShortcuts",
                    "Show Keyboard Shortcuts",
                ),
                (
                    "Ctrl+,",
                    "shortcuts.actions.openPreferences",
                    "Open Preferences",
                ),
                ("Ctrl+.", "shortcuts.actions.openFlags", "Open Rclone Flags"),
            ][..],
        ),
        (
            "remote",
            "shortcuts.categories.remoteManagement",
            "Remote Management",
            &[
                (
                    "Ctrl+N",
                    "shortcuts.actions.newRemoteDetailed",
                    "Create New Remote (Detailed)",
                ),
                (
                    "Ctrl+R",
                    "shortcuts.actions.newRemoteQuick",
                    "Create New Remote (Quick)",
                ),
                (
                    "Ctrl+I",
                    "shortcuts.actions.loadConfig",
                    "Load Configuration",
                ),
                (
                    "Ctrl+E",
                    "shortcuts.actions.exportConfig",
                    "Export Configuration",
                ),
                (
                    "Ctrl+Shift+M",
                    "shortcuts.actions.forceCheck",
                    "Force Check Mounts",
                ),
                (
                    "Ctrl+Shift+S",
                    "shortcuts.actions.forceCheckServes",
                    "Force Check Served Remotes",
                ),
            ][..],
        ),
        (
            "files",
            "shortcuts.categories.fileBrowser",
            "File Browser",
            &[
                (
                    "Ctrl+B",
                    "shortcuts.actions.toggleBrowser",
                    "Toggle File Browser",
                ),
                (
                    "Ctrl+Alt+F",
                    "shortcuts.actions.openFlowOverlay",
                    "Toggle Flow Workspace",
                ),
                (
                    "Ctrl+T / Ctrl+W",
                    "nautilus.tabs.newClose",
                    "New / close file tab",
                ),
                (
                    "Ctrl+Shift+D",
                    "titlebar.detach",
                    "Detach current workspace",
                ),
                ("F5", "nautilus.actions.reload", "Reload listing"),
                ("F2", "nautilus.contextMenu.rename", "Rename"),
                (
                    "Space",
                    "fileBrowser.fileViewer.title",
                    "Preview file or folder",
                ),
                ("Delete", "nautilus.contextMenu.delete", "Delete"),
                (
                    "Ctrl+C / X / V",
                    "nautilus.actions.clipboard",
                    "Copy / Cut / Paste",
                ),
                (
                    "Ctrl+Shift+N",
                    "nautilus.contextMenu.newFolder",
                    "New folder",
                ),
                ("Ctrl+H", "nautilus.view.hidden", "Toggle hidden files"),
                ("Backspace", "nautilus.actions.parent", "Parent folder"),
                ("Ctrl+Tab", "nautilus.contextMenu.nextTab", "Next file tab"),
                (
                    "Ctrl+Shift+Tab",
                    "nautilus.contextMenu.previousTab",
                    "Previous file tab",
                ),
                (
                    "Ctrl+I",
                    "nautilus.contextMenu.switchPane",
                    "Switch split pane",
                ),
                ("Alt+Enter", "nautilus.contextMenu.properties", "Properties"),
                ("Ctrl+A", "nautilus.contextMenu.selectAll", "Select all"),
                ("Ctrl+L", "nautilus.titles.pathPlaceholder", "Edit path"),
                (
                    "Ctrl+F",
                    "nautilus.titles.searchPlaceholder",
                    "Search listing",
                ),
                (
                    "Alt+← / Alt+→",
                    "nautilus.actions.history",
                    "Back / forward",
                ),
            ][..],
        ),
    ];
    for (_, cat_key, cat_fallback, items) in entries {
        let header = adw::ActionRow::new();
        header.set_title(&ctx.t_or(cat_key, cat_fallback));
        header.set_sensitive(false);
        list.append(&header);
        for (keys, desc_key, desc_fallback) in items {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or(desc_key, desc_fallback));
            row.set_subtitle(keys);
            row.set_widget_name(&format!("{keys} {desc_fallback}"));
            list.append(&row);
        }
    }
    {
        let list = list.clone();
        search.connect_search_changed(move |entry| {
            let query = entry.text().to_lowercase();
            let mut child = list.first_child();
            while let Some(widget) = child {
                let next = widget.next_sibling();
                if let Ok(row) = widget.downcast::<adw::ActionRow>() {
                    if row.is_sensitive() {
                        let hay = format!(
                            "{} {} {}",
                            row.title(),
                            row.subtitle().unwrap_or_default(),
                            row.widget_name()
                        )
                        .to_lowercase();
                        row.set_visible(query.is_empty() || hay.contains(&query));
                    }
                }
                child = next;
            }
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(8);
    box_.append(&search);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    box_.append(&scroll);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn debug_info(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let info = crate::platform::debug_info();
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("developerTools.debugInfo", "Debug Info"));
    dialog.set_content_width(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (title, value) in [
        (
            ctx.t_or("modals.about.version", "Version"),
            info.app_version.clone(),
        ),
        (
            ctx.t_or("generalOverview.system.platform", "Platform"),
            format!("{} / {}", info.platform, info.arch),
        ),
        (ctx.t_or("developerTools.mode", "Mode"), info.mode.clone()),
        (
            ctx.t_or("titlebar.menu.openConfig", "Config folder"),
            info.config_dir.clone(),
        ),
        (
            ctx.t_or("titlebar.menu.openCache", "Cache folder"),
            info.cache_dir.clone(),
        ),
        (
            ctx.t_or("titlebar.menu.openLog", "Logs folder"),
            info.logs_dir.clone(),
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&value);
        row.set_subtitle_lines(3);
        list.append(&row);
    }
    if let Some(client) = ctx.client() {
        if let Ok(paths) = client.config_paths() {
            for key in ["config", "cache", "temp"] {
                if let Some(value) = paths.get(key).and_then(|v| v.as_str()) {
                    let row = adw::ActionRow::new();
                    row.set_title(&format!("rclone {key}"));
                    row.set_subtitle(value);
                    row.set_subtitle_lines(3);
                    list.append(&row);
                }
            }
        }
        if let Ok(encrypted) = client.config_is_encrypted() {
            let row = adw::ActionRow::new();
            row.set_title("rclone.conf encrypted");
            row.set_subtitle(if encrypted { "yes" } else { "no" });
            list.append(&row);
        }
    }
    let copy = gtk::Button::with_label(&ctx.t_or("common.copy", "Copy"));
    let summary = format!(
        "version={}\nplatform={}/{}\nmode={}\nconfig={}\ncache={}\nlogs={}",
        info.app_version,
        info.platform,
        info.arch,
        info.mode,
        info.config_dir,
        info.cache_dir,
        info.logs_dir
    );
    copy.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&summary);
        }
    });
    let open_cfg = gtk::Button::with_label(&ctx.t_or("titlebar.menu.openConfig", "Open config"));
    let open_cache = gtk::Button::with_label(&ctx.t_or("titlebar.menu.openCache", "Open cache"));
    let open_logs = gtk::Button::with_label(&ctx.t_or("titlebar.menu.openLog", "Open logs"));
    let cfg = info.config_dir.clone();
    let cache = info.cache_dir.clone();
    let logs = info.logs_dir.clone();
    open_cfg.connect_clicked(move |_| {
        let _ = open::that(&cfg);
    });
    open_cache.connect_clicked(move |_| {
        let _ = open::that(&cache);
    });
    open_logs.connect_clicked(move |_| {
        let _ = open::that(&logs);
    });
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_top(8);
    actions.append(&copy);
    actions.append(&open_cfg);
    actions.append(&open_cache);
    actions.append(&open_logs);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_bottom(12);
    box_.append(&list);
    box_.append(&actions);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn vfs_control(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    toast: adw::ToastOverlay,
) {
    if try_spawn_standalone(&ctx, "vfs", serde_json::json!({ "remote": remote })) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!(
        "{} · {remote}",
        ctx.t_or("remoteConfig.vfs", "VFS")
    ));
    dialog.set_content_width(560);
    dialog.set_content_height(640);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let panel = super::vfs_panel::vfs_panel(ctx.clone(), remote, toast);
    panel.set_margin_top(12);
    panel.set_margin_bottom(12);
    panel.set_margin_start(12);
    panel.set_margin_end(12);
    scroll.set_child(Some(&panel));
    dialog.set_child(Some(&scroll));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn logs(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: Option<String>) {
    if try_spawn_standalone(
        &ctx,
        "logs",
        serde_json::json!({ "remote": remote.clone().unwrap_or_default() }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    let title = if let Some(name) = remote.as_ref().filter(|r| *r != "_engine") {
        ctx.tf("modals.logs.remoteLogs", &[("name", name)])
    } else {
        ctx.t_or("modals.logs.terminalOutput", "Logs")
    };
    dialog.set_title(&title);
    dialog.set_content_width(760);
    dialog.set_content_height(520);
    let key = remote.clone().unwrap_or_else(|| "_engine".into());
    let locked = remote.clone().filter(|r| r != "_engine");
    let entries = Rc::new(RefCell::new(crate::logs::collect_entries(
        &ctx.store.borrow().logs,
        &crate::logs::read_log_file_tail(400),
        locked.as_deref(),
    )));
    let level_filter = Rc::new(RefCell::new(String::new()));
    let remote_filter = Rc::new(RefCell::new(locked.clone()));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let empty = adw::StatusPage::new();
    empty.set_icon_name(Some("utilities-system-monitor-symbolic"));
    empty.set_title(&ctx.t_or("modals.logs.noLogsFound", "No logs found"));
    empty.set_description(Some(
        &ctx.t_or("modals.logs.adjustFilters", "Try adjusting your filters"),
    ));
    let stack = gtk::Stack::new();
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    stack.add_named(&scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));
    let search = gtk::Entry::new();
    search.set_placeholder_text(Some(
        &ctx.t_or("modals.logs.searchPlaceholder", "Search logs..."),
    ));
    let copy_label = ctx.t_or("common.copy", "Copy");
    let context_label = ctx.t_or("modals.logs.logContext", "Log Context");
    let apply = {
        let ctx = ctx.clone();
        let list = list.clone();
        let stack = stack.clone();
        let entries = entries.clone();
        let level_filter = level_filter.clone();
        let remote_filter = remote_filter.clone();
        let copy_label = copy_label.clone();
        let context_label = context_label.clone();
        move |query: &str| {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let owned = entries.borrow();
            let filtered = crate::logs::filter_entries(
                &owned,
                query,
                &level_filter.borrow(),
                remote_filter.borrow().as_deref(),
            );
            if filtered.is_empty() {
                stack.set_visible_child_name("empty");
                return;
            }
            stack.set_visible_child_name("list");
            for entry in filtered {
                let subtitle = match &entry.remote_name {
                    Some(remote) if !remote.is_empty() => {
                        format!("{} · {} · {remote}", entry.timestamp, entry.level.as_str())
                    }
                    _ => format!("{} · {}", entry.timestamp, entry.level.as_str()),
                };
                let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
                copy.set_valign(gtk::Align::Center);
                copy.set_tooltip_text(Some(&copy_label));
                let formatted = entry.formatted();
                copy.connect_clicked(move |_| {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&formatted);
                    }
                });
                let message = gtk::Label::new(None);
                let translated = ctx.translate_error(&entry.message);
                if crate::ansi::has_ansi(&translated) {
                    message.set_markup(&crate::ansi::ansi_to_pango(&translated));
                } else {
                    message.set_text(&translated);
                }
                message.set_wrap(true);
                message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                message.set_xalign(0.0);
                message.set_hexpand(true);
                message.set_selectable(true);
                message.add_css_class("heading");
                let sub = gtk::Label::new(Some(&subtitle));
                sub.add_css_class("dim-label");
                sub.set_xalign(0.0);
                let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
                text.set_hexpand(true);
                text.append(&message);
                text.append(&sub);
                let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row_box.set_margin_start(12);
                row_box.set_margin_end(12);
                row_box.set_margin_top(8);
                row_box.set_margin_bottom(8);
                row_box.append(&text);
                row_box.append(&copy);
                if let Some(details) = &entry.context {
                    let expander = gtk::Expander::new(Some(&context_label));
                    let label = gtk::Label::new(Some(details));
                    label.set_wrap(true);
                    label.set_xalign(0.0);
                    label.set_selectable(true);
                    label.add_css_class("monospace");
                    label.set_margin_start(12);
                    label.set_margin_end(12);
                    label.set_margin_top(4);
                    label.set_margin_bottom(8);
                    expander.set_child(Some(&label));
                    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    col.append(&row_box);
                    col.append(&expander);
                    let row = gtk::ListBoxRow::new();
                    row.set_child(Some(&col));
                    list.append(&row);
                } else {
                    let row = gtk::ListBoxRow::new();
                    row.set_child(Some(&row_box));
                    list.append(&row);
                }
            }
        }
    };
    apply("");
    {
        let apply = apply.clone();
        search.connect_changed(move |entry| apply(&entry.text()));
    }
    let levels = gtk::StringList::new(&["", "ERROR", "WARN", "NOTICE", "INFO", "DEBUG"]);
    let level = gtk::DropDown::new(Some(levels), gtk::Expression::NONE);
    level.set_tooltip_text(Some(&ctx.t_or("modals.logs.logLevel", "Log Level")));
    {
        let apply = apply.clone();
        let search = search.clone();
        let level_filter = level_filter.clone();
        level.connect_selected_notify(move |drop| {
            let selected = ["", "ERROR", "WARN", "NOTICE", "INFO", "DEBUG"]
                .get(drop.selected() as usize)
                .copied()
                .unwrap_or_default();
            *level_filter.borrow_mut() = selected.to_string();
            apply(&search.text());
        });
    }
    let reload = {
        let ctx = ctx.clone();
        let entries = entries.clone();
        let locked = locked.clone();
        move || {
            *entries.borrow_mut() = crate::logs::collect_entries(
                &ctx.store.borrow().logs,
                &crate::logs::read_log_file_tail(400),
                locked.as_deref(),
            );
        }
    };
    let clear = gtk::Button::from_icon_name("edit-clear-symbolic");
    clear.set_tooltip_text(Some(&ctx.t_or("modals.logs.clearAll", "Clear")));
    {
        let ctx = ctx.clone();
        let key = key.clone();
        let reload = reload.clone();
        let apply = apply.clone();
        let search = search.clone();
        clear.connect_clicked(move |_| {
            if key == "_engine" {
                ctx.store.borrow_mut().logs.clear();
                crate::logs::clear_log_file();
            } else {
                ctx.store.borrow_mut().logs.remove(&key);
            }
            ctx.persist();
            reload();
            apply(&search.text());
        });
    }
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some(&ctx.t("common.refresh")));
    {
        let reload = reload.clone();
        let apply = apply.clone();
        let search = search.clone();
        refresh.connect_clicked(move |_| {
            reload();
            apply(&search.text());
        });
    }
    let copy_all = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy_all.set_tooltip_text(Some(&ctx.t_or("common.copy", "Copy")));
    {
        let entries = entries.clone();
        let level_filter = level_filter.clone();
        let remote_filter = remote_filter.clone();
        let search = search.clone();
        copy_all.connect_clicked(move |_| {
            let owned = entries.borrow();
            let filtered = crate::logs::filter_entries(
                &owned,
                &search.text(),
                &level_filter.borrow(),
                remote_filter.borrow().as_deref(),
            );
            if let Some(display) = gtk::gdk::Display::default() {
                display
                    .clipboard()
                    .set_text(&crate::logs::export_text(&filtered));
            }
        });
    }
    let export = gtk::Button::from_icon_name("document-save-symbolic");
    export.set_tooltip_text(Some(&ctx.t_or("titlebar.menu.export", "Export")));
    {
        let parent = parent.clone();
        let entries = entries.clone();
        let level_filter = level_filter.clone();
        let remote_filter = remote_filter.clone();
        let search = search.clone();
        export.connect_clicked(move |_| {
            let text = {
                let owned = entries.borrow();
                let filtered = crate::logs::filter_entries(
                    &owned,
                    &search.text(),
                    &level_filter.borrow(),
                    remote_filter.borrow().as_deref(),
                );
                crate::logs::export_text(&filtered)
            };
            let file_dialog = gtk::FileDialog::new();
            file_dialog.set_initial_name(Some("rclone-manager-logs.txt"));
            let window = parent.root().and_downcast::<gtk::Window>();
            file_dialog.save(
                window.as_ref(),
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let _ = std::fs::write(path, text);
                        }
                    }
                },
            );
        });
    }
    let top = gtk::Button::from_icon_name("go-top-symbolic");
    top.set_tooltip_text(Some(&ctx.t_or("modals.logs.scrollTop", "Scroll Top")));
    {
        let scroll = scroll.clone();
        top.connect_clicked(move |_| {
            scroll.vadjustment().set_value(0.0);
        });
    }
    let bottom = gtk::Button::from_icon_name("go-bottom-symbolic");
    bottom.set_tooltip_text(Some(&ctx.t_or("modals.logs.scrollBottom", "Scroll Bottom")));
    {
        let scroll = scroll.clone();
        bottom.connect_clicked(move |_| {
            let adj = scroll.vadjustment();
            adj.set_value(adj.upper() - adj.page_size());
        });
    }
    {
        let reload = reload.clone();
        let apply = apply.clone();
        let search = search.clone();
        let list = list.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
            if !list.is_mapped() {
                return glib::ControlFlow::Break;
            }
            reload();
            apply(&search.text());
            glib::ControlFlow::Continue
        });
    }
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.set_margin_start(8);
    toolbar.set_margin_end(8);
    toolbar.set_margin_top(8);
    search.set_hexpand(true);
    toolbar.append(&level);
    toolbar.append(&search);
    toolbar.append(&refresh);
    toolbar.append(&copy_all);
    toolbar.append(&export);
    toolbar.append(&top);
    toolbar.append(&bottom);
    toolbar.append(&clear);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&toolbar);
    box_.append(&stack);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn push_flag_edit(
    edits: &Rc<RefCell<Vec<(String, String, serde_json::Value)>>>,
    block: &str,
    field: &str,
    value: serde_json::Value,
) {
    edits
        .borrow_mut()
        .push((block.to_string(), field.to_string(), value));
}

fn add_flag_option_row(
    ctx: &AppCtx,
    group: &adw::PreferencesGroup,
    edits: &Rc<RefCell<Vec<(String, String, serde_json::Value)>>>,
    block: &str,
    option: &crate::flags::FlagOption,
) {
    let title = ctx.option_label(&option.name, "title", &option.name);
    let help = ctx.option_label(&option.name, "help", &option.help);
    let current_text = crate::value_mapper::machine_to_human(
        &option.value,
        &option.type_name,
        &option.default_str,
    );
    let kind = crate::value_mapper::control_kind(
        &option.type_name,
        option.exclusive,
        option.examples.len(),
    );
    match kind {
        crate::value_mapper::ControlKind::Bool => {
            let row = adw::SwitchRow::new();
            row.set_title(&title);
            row.set_subtitle(&help);
            row.set_active(current_text.eq_ignore_ascii_case("true"));
            let edits = edits.clone();
            let block = block.to_string();
            let field = option.field_name.clone();
            row.connect_active_notify(move |row| {
                push_flag_edit(&edits, &block, &field, serde_json::json!(row.is_active()));
            });
            group.add(&row);
        }
        crate::value_mapper::ControlKind::Tristate => {
            let values = ["unset", "true", "false"];
            let row = adw::ComboRow::new();
            row.set_title(&title);
            row.set_subtitle(&help);
            row.set_model(Some(&gtk::StringList::new(&values)));
            if let Some(idx) = values
                .iter()
                .position(|v| v.eq_ignore_ascii_case(&current_text))
            {
                row.set_selected(idx as u32);
            }
            let edits = edits.clone();
            let block = block.to_string();
            let field = option.field_name.clone();
            let type_name = option.type_name.clone();
            row.connect_selected_notify(move |row| {
                let text = values
                    .get(row.selected() as usize)
                    .copied()
                    .unwrap_or("unset");
                push_flag_edit(
                    &edits,
                    &block,
                    &field,
                    crate::flags::parse_flag_value(&type_name, text),
                );
            });
            group.add(&row);
        }
        crate::value_mapper::ControlKind::Select => {
            let labels: Vec<String> = option
                .examples
                .iter()
                .map(|(value, help)| {
                    if help.is_empty() {
                        value.clone()
                    } else {
                        format!("{value} — {help}")
                    }
                })
                .collect();
            let values: Vec<String> = option.examples.iter().map(|(v, _)| v.clone()).collect();
            let row = adw::ComboRow::new();
            row.set_title(&title);
            row.set_subtitle(&help);
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            row.set_model(Some(&gtk::StringList::new(&refs)));
            if let Some(idx) = values.iter().position(|v| v == &current_text) {
                row.set_selected(idx as u32);
            }
            let edits = edits.clone();
            let block = block.to_string();
            let field = option.field_name.clone();
            let type_name = option.type_name.clone();
            row.connect_selected_notify(move |row| {
                if let Some(text) = values.get(row.selected() as usize) {
                    push_flag_edit(
                        &edits,
                        &block,
                        &field,
                        crate::flags::parse_flag_value(&type_name, text),
                    );
                }
            });
            group.add(&row);
        }
        crate::value_mapper::ControlKind::Numeric => {
            let row = adw::SpinRow::with_range(-1_000_000_000.0, 1_000_000_000.0, 1.0);
            row.set_title(&title);
            row.set_subtitle(&help);
            if let Ok(v) = current_text.parse::<f64>() {
                row.set_value(v);
            }
            row.set_digits(if crate::value_mapper::is_float_type(&option.type_name) {
                3
            } else {
                0
            });
            let edits = edits.clone();
            let block = block.to_string();
            let field = option.field_name.clone();
            let type_name = option.type_name.clone();
            row.connect_changed(move |row| {
                let text = if row.value().fract() == 0.0 {
                    format!("{}", row.value() as i64)
                } else {
                    row.value().to_string()
                };
                push_flag_edit(
                    &edits,
                    &block,
                    &field,
                    crate::flags::parse_flag_value(&type_name, &text),
                );
            });
            group.add(&row);
        }
        crate::value_mapper::ControlKind::Input => {
            let row = adw::EntryRow::new();
            row.set_title(&title);
            row.set_text(&current_text);
            if !help.is_empty() {
                row.set_tooltip_text(Some(&help));
            }
            if option.type_name == "Duration" {
                row.set_title(&format!("{title} (1h / 30s / 500ms)"));
            } else if option.type_name == "SizeSuffix" {
                row.set_title(&format!("{title} (1Gi / 512Mi / off)"));
            }
            let edits = edits.clone();
            let block = block.to_string();
            let field = option.field_name.clone();
            let type_name = option.type_name.clone();
            row.connect_changed(move |row| {
                push_flag_edit(
                    &edits,
                    &block,
                    &field,
                    crate::flags::parse_flag_value(&type_name, &row.text()),
                );
            });
            group.add(&row);
        }
    }
}

pub fn rclone_flags(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    if try_spawn_standalone(&ctx, "rclone-flags", serde_json::json!({})) {
        return;
    }
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title(&ctx.t_or("titlebar.menu.flags", "Rclone Flags"));
    dialog.set_search_enabled(true);
    let Some(client) = ctx.client() else {
        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("common.engineOffline", "Engine offline"));
        group.add(&row);
        page.add(&group);
        dialog.add(&page);
        dialog.present(Some(parent));
        return;
    };
    let info = client.options_info().unwrap_or(serde_json::json!({}));
    let current = client.options_get().unwrap_or(serde_json::json!({}));
    let mut blocks = crate::flags::parse_options_info(&info);
    crate::flags::merge_current_values(&mut blocks, &current);
    let edits: Rc<RefCell<Vec<(String, String, serde_json::Value)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for category in [
        "backend", "filter", "vfs", "mount", "copy", "sync", "check", "network", "other",
    ] {
        let page = adw::PreferencesPage::new();
        page.set_title(&category.to_ascii_uppercase());
        let group = adw::PreferencesGroup::new();
        group.set_title(category);
        let options = crate::flags::options_for_category(&blocks, category);
        if options.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("modals.flags.emptyCategory", "No flags in this category"));
            group.add(&row);
        }
        for (block, option) in options {
            add_flag_option_row(&ctx, &group, &edits, block, option);
        }
        page.add(&group);
        dialog.add(&page);
    }
    let apply = gtk::Button::with_label(&ctx.t_or("common.apply", "Apply changes"));
    apply.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let edits = edits.clone();
        apply.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let payload = crate::flags::collect_edits(&edits.borrow());
                match client.options_set(payload.clone()) {
                    Ok(_) => {
                        if let Err(e) =
                            crate::backend_options::merge_and_save(&ctx.backend_key(), &payload)
                        {
                            log::warn!("failed to persist flags: {e}");
                        } else {
                            log::info!("rclone flags applied");
                        }
                    }
                    Err(e) => log::warn!("failed to apply flags: {e}"),
                }
            }
        });
    }
    let extra = adw::PreferencesPage::new();
    extra.set_title(&ctx.t_or("common.apply", "Apply"));
    let g = adw::PreferencesGroup::new();
    g.set_description(Some(&ctx.t_or(
        "remoteConfig.flagsApplyHelp",
        "Edits are sent to rclone via options/set. Categories match the Angular flag panels.",
    )));
    let row = adw::ActionRow::new();
    row.set_title(&ctx.t_or(
        "remoteConfig.flagsWriteEngine",
        "Write flags to the running engine",
    ));
    row.add_suffix(&apply);
    g.add(&row);
    extra.add(&g);
    dialog.add(&extra);

    let json_page = adw::PreferencesPage::new();
    json_page.set_title(&ctx.t_or("remoteConfig.jsonMode", "JSON"));
    let json_group = adw::PreferencesGroup::new();
    json_group.set_title(&ctx.t_or("remoteConfig.jsonPayload", "Raw options/set payload"));
    json_group.set_description(Some(&ctx.t_or(
        "remoteConfig.jsonPayloadHelp",
        "Edit the current rclone options as JSON. Apply writes the object via options/set.",
    )));
    let json_toggle = adw::SwitchRow::new();
    json_toggle.set_title(&ctx.t_or(
        "settings.runtime.show_json_mode.label",
        "Remember JSON mode",
    ));
    json_toggle.set_active(ctx.settings.borrow().runtime.show_json_mode);
    {
        let ctx = ctx.clone();
        json_toggle.connect_active_notify(move |row| {
            ctx.settings.borrow_mut().runtime.show_json_mode = row.is_active();
            ctx.persist();
        });
    }
    json_group.add(&json_toggle);
    let json_view = gtk::TextView::new();
    json_view.set_monospace(true);
    json_view.set_wrap_mode(gtk::WrapMode::WordChar);
    let pretty = serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".into());
    json_view.buffer().set_text(&pretty);
    let json_scroll = gtk::ScrolledWindow::new();
    json_scroll.set_min_content_height(280);
    json_scroll.set_child(Some(&json_view));
    let json_apply = gtk::Button::with_label(&ctx.t_or("remoteConfig.applyJson", "Apply JSON"));
    json_apply.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let json_view = json_view.clone();
        json_apply.connect_clicked(move |_| {
            let buffer = json_view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            match crate::flags::parse_json_object(&text) {
                Ok(map) => {
                    if let Some(client) = ctx.client() {
                        let payload = serde_json::Value::Object(map);
                        match client.options_set(payload.clone()) {
                            Ok(_) => {
                                if let Err(e) = crate::backend_options::merge_and_save(
                                    &ctx.backend_key(),
                                    &payload,
                                ) {
                                    log::warn!("failed to persist JSON flags: {e}");
                                } else {
                                    log::info!("rclone flags applied from JSON");
                                }
                            }
                            Err(e) => log::warn!("failed to apply JSON flags: {e}"),
                        }
                    }
                }
                Err(e) => log::warn!("invalid flags JSON: {e}"),
            }
        });
    }
    let json_row = adw::ActionRow::new();
    json_row.set_title(&ctx.t_or(
        "remoteConfig.flagsWriteJson",
        "Write JSON to the running engine",
    ));
    json_row.add_suffix(&json_apply);
    json_group.add(&json_row);
    json_page.add(&json_group);
    let json_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    json_box.set_margin_start(12);
    json_box.set_margin_end(12);
    json_box.append(&json_scroll);
    let json_holder = adw::PreferencesGroup::new();
    let holder_row = adw::ActionRow::new();
    holder_row.set_title("Document");
    holder_row.set_activatable(false);
    json_holder.add(&holder_row);
    json_page.add(&json_holder);
    dialog.add(&json_page);
    // Attach the editor below the last page content via a toast-less overlay:
    // PreferencesDialog pages can't host arbitrary boxes easily, so present the
    // JSON editor as the ActionRow child suffix expansion.
    holder_row.set_child(Some(&json_box));
    dialog.present(Some(parent));
}

pub fn action_order(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    title: &str,
    catalog: &[&str],
    current: &[String],
    max_visible: Option<usize>,
    on_save: impl Fn(Vec<String>) + 'static,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(480);
    dialog.set_content_height(560);
    let items = Rc::new(RefCell::new({
        let mut built = crate::action_order::build_items(current, catalog);
        crate::action_order::cap_visible(&mut built, max_visible);
        built
    }));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let rebuild: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let visible_l = ctx.t_or("common.show", "Visible");
    let hidden_l = ctx.t_or("common.hide", "Hidden");
    let up_tip = ctx.t_or("modals.itemOrderVisibility.showItem", "Move up");
    let down_tip = ctx.t_or("modals.itemOrderVisibility.hideItem", "Move down");
    let rebuild_fn = {
        let list = list.clone();
        let items = items.clone();
        let rebuild = rebuild.clone();
        let visible_l = visible_l.clone();
        let hidden_l = hidden_l.clone();
        let up_tip = up_tip.clone();
        let down_tip = down_tip.clone();
        let ctx = ctx.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let snapshot = items.borrow().clone();
            let can_show_more = crate::action_order::can_show_more(&snapshot, max_visible);
            for (idx, item) in snapshot.iter().enumerate() {
                let row = adw::ActionRow::new();
                let title = OperationType::parse(&item.id)
                    .map(|op| ctx.t_or(op.action_label_key(), op.api_label()))
                    .unwrap_or_else(|| item.id.clone());
                row.set_title(&title);
                row.set_subtitle(if item.visible { &visible_l } else { &hidden_l });
                let visible = gtk::Switch::new();
                visible.set_active(item.visible);
                visible.set_valign(gtk::Align::Center);
                if !item.visible && !can_show_more {
                    visible.set_sensitive(false);
                }
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    let id = item.id.clone();
                    visible.connect_active_notify(move |sw| {
                        if sw.is_active()
                            && !crate::action_order::can_show_more(&items.borrow(), max_visible)
                            && !items
                                .borrow()
                                .iter()
                                .any(|item| item.id == id && item.visible)
                        {
                            sw.set_active(false);
                            return;
                        }
                        crate::action_order::apply_visibility(
                            &mut items.borrow_mut(),
                            &id,
                            sw.is_active(),
                        );
                        rebuild.borrow()();
                    });
                }
                let up = gtk::Button::from_icon_name("go-up-symbolic");
                up.set_valign(gtk::Align::Center);
                up.set_tooltip_text(Some(&up_tip));
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    up.connect_clicked(move |_| {
                        crate::action_order::move_item(&mut items.borrow_mut(), idx, -1);
                        rebuild.borrow()();
                    });
                }
                let down = gtk::Button::from_icon_name("go-down-symbolic");
                down.set_valign(gtk::Align::Center);
                down.set_tooltip_text(Some(&down_tip));
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    down.connect_clicked(move |_| {
                        crate::action_order::move_item(&mut items.borrow_mut(), idx, 1);
                        rebuild.borrow()();
                    });
                }
                row.add_suffix(&up);
                row.add_suffix(&down);
                row.add_suffix(&visible);
                list.append(&row);
            }
        })
    };
    *rebuild.borrow_mut() = Box::new({
        let rebuild_fn = rebuild_fn.clone();
        move || rebuild_fn()
    });
    rebuild_fn();

    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        let items = items.clone();
        save.connect_clicked(move |_| {
            let mut snapshot = items.borrow().clone();
            crate::action_order::cap_visible(&mut snapshot, max_visible);
            on_save(crate::action_order::visible_ids(&snapshot));
            dialog.close();
        });
    }
    let cancel = gtk::Button::with_label(&ctx.t("common.cancel"));
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    let reset = gtk::Button::with_label(&ctx.t("common.reset"));
    {
        let items = items.clone();
        let rebuild = rebuild.clone();
        let catalog: Vec<String> = catalog.iter().map(|s| (*s).to_string()).collect();
        reset.connect_clicked(move |_| {
            let refs: Vec<&str> = catalog.iter().map(|s| s.as_str()).collect();
            let mut built = crate::action_order::build_items(&[], &refs);
            crate::action_order::cap_visible(&mut built, max_visible);
            *items.borrow_mut() = built;
            rebuild.borrow()();
        });
    }
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_margin_top(8);
    bar.append(&reset);
    bar.append(&cancel);
    bar.append(&save);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    let hint = gtk::Label::new(Some(&if let Some(max) = max_visible {
        ctx.tf(
            "modals.actionSelection.description",
            &[("name", title), ("max", &max.to_string())],
        )
    } else {
        ctx.t_or(
            "modals.actionSelection.description",
            "Toggle visibility and reorder. Hidden actions stay available in Configure.",
        )
    }));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    box_.append(&hint);
    box_.append(&scroll);
    box_.append(&bar);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn backends(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    if try_spawn_standalone(&ctx, "backend", serde_json::json!({})) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("titlebar.menu.backends", "Backends"));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let ready = ctx.engine_ready();
    let port = ctx.engine.borrow().as_ref().map(|e| e.port).unwrap_or(0);
    let active = ctx.settings.borrow().core.active_backend.clone();
    let local = adw::ActionRow::new();
    local.set_title(&ctx.t_or("modals.backend.local", "Local rclone RC"));
    let local_id = ctx
        .client()
        .filter(|_| active.is_empty() || active == "local")
        .and_then(|c| c.version_info().ok())
        .map(|v| crate::rclone::backend_identity(&v).summary())
        .unwrap_or_default();
    local.set_subtitle(&format!(
        "127.0.0.1:{port} · ready={ready}{}{}",
        if active.is_empty() || active == "local" {
            " · active"
        } else {
            ""
        },
        if local_id.is_empty() {
            String::new()
        } else {
            format!(" · {local_id}")
        }
    ));
    let use_local = gtk::Button::with_label(&ctx.t_or("common.use", "Use"));
    use_local.set_valign(gtk::Align::Center);
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        use_local.connect_clicked(move |_| {
            ctx.switch_backend("local");
            backends(&parent, ctx.clone());
        });
    }
    local.add_suffix(&use_local);
    list.append(&local);
    let config_path = adw::EntryRow::new();
    config_path.set_title(&ctx.t_or(
        "modals.backend.fields.configPath.label",
        "Local rclone.conf",
    ));
    let current_config =
        crate::repair::config_path_from_flags(&ctx.settings.borrow().core.rclone_additional_flags)
            .unwrap_or_else(|| {
                crate::repair::default_rclone_config_path()
                    .display()
                    .to_string()
            });
    config_path.set_text(&current_config);
    let browse_config = gtk::Button::with_label(&ctx.t_or("common.browse", "Browse"));
    browse_config.set_valign(gtk::Align::Center);
    {
        let config_path = config_path.clone();
        browse_config.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            let config_path = config_path.clone();
            dialog.open(
                None::<&gtk::Window>,
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(picked) = file.path() {
                            config_path.set_text(&picked.display().to_string());
                        }
                    }
                },
            );
        });
    }
    config_path.add_suffix(&browse_config);
    {
        let ctx = ctx.clone();
        config_path.connect_changed(move |row| {
            crate::repair::set_config_path_flag(
                &mut ctx.settings.borrow_mut().core.rclone_additional_flags,
                &row.text(),
            );
            ctx.persist();
            if let Some(client) = ctx.client() {
                crate::rclone::apply_backend_rc_config(&client, Some(row.text().as_str()), None);
            }
        });
    }
    list.append(&config_path);
    for backend in ctx.settings.borrow().core.extra_backends.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&backend.name);
        let marker = if active == backend.name {
            format!(" · {}", ctx.t_or("modals.backend.active", "active"))
        } else {
            String::new()
        };
        let identity = {
            let user = if backend.user.is_empty() {
                None
            } else {
                Some(backend.user.clone())
            };
            let pass = if backend.pass.is_empty() {
                None
            } else {
                Some(backend.pass.clone())
            };
            let client =
                crate::rclone::RcClient::new(&backend.host, backend.port).with_auth(user, pass);
            client
                .version_info()
                .ok()
                .map(|v| crate::rclone::backend_identity(&v).summary())
        };
        row.set_subtitle(&format!(
            "{}:{}{marker}{}",
            backend.host,
            backend.port,
            identity
                .as_deref()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default()
        ));
        let test = gtk::Button::from_icon_name("network-transmit-receive-symbolic");
        test.set_valign(gtk::Align::Center);
        test.set_tooltip_text(Some(
            &ctx.t_or("modals.backend.testConnection", "Test connection"),
        ));
        let use_btn = gtk::Button::with_label(&ctx.t_or("common.continue", "Use"));
        use_btn.set_valign(gtk::Align::Center);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_valign(gtk::Align::Center);
        {
            let backend = backend.clone();
            let parent = parent.clone();
            let ctx = ctx.clone();
            test.connect_clicked(move |_| {
                let user = if backend.user.is_empty() {
                    None
                } else {
                    Some(backend.user.clone())
                };
                let pass = if backend.pass.is_empty() {
                    None
                } else {
                    Some(backend.pass.clone())
                };
                let client =
                    crate::rclone::RcClient::new(&backend.host, backend.port).with_auth(user, pass);
                let msg = if client.ping() {
                    format!(
                        "{} — {}",
                        ctx.t_or("modals.backend.status.connected", "Connected"),
                        client
                            .version()
                            .unwrap_or_else(|_| ctx.t_or("backup.restore.unknown", "unknown"))
                    )
                } else {
                    ctx.tf(
                        "modals.backend.status.error",
                        &[("message", &format!("{}:{}", backend.host, backend.port))],
                    )
                };
                let alert = adw::AlertDialog::new(
                    Some(&ctx.t_or("modals.backend.testConnection", "Backend test")),
                    Some(&msg),
                );
                alert.add_response("ok", &ctx.t("common.ok"));
                alert.present(Some(&parent));
            });
        }
        {
            let ctx = ctx.clone();
            let name = backend.name.clone();
            let parent = parent.clone();
            use_btn.connect_clicked(move |_| {
                ctx.switch_backend(&name);
                backends(&parent, ctx.clone());
            });
        }
        {
            let ctx = ctx.clone();
            let name = backend.name.clone();
            let parent = parent.clone();
            remove.connect_clicked(move |_| {
                ctx.settings
                    .borrow_mut()
                    .core
                    .extra_backends
                    .retain(|b| b.name != name);
                if ctx.settings.borrow().core.active_backend == name {
                    ctx.settings.borrow_mut().core.active_backend.clear();
                }
                ctx.persist();
                backends(&parent, ctx.clone());
            });
        }
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.set_valign(gtk::Align::Center);
        edit.set_tooltip_text(Some(
            &ctx.t_or("modals.backend.editBackend", "Edit backend"),
        ));
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let backend = backend.clone();
            edit.connect_clicked(move |_| {
                backend_editor(&parent, ctx.clone(), Some(backend.clone()));
            });
        }
        let clone = gtk::Button::with_label(&ctx.t("common.duplicate"));
        clone.set_valign(gtk::Align::Center);
        clone.set_tooltip_text(Some(&ctx.t_or(
            "modals.backend.copyOptions.description",
            "Duplicate connection settings into a new backend",
        )));
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let mut entry = backend.clone();
            entry.name = format!("{}-copy", backend.name);
            clone.connect_clicked(move |_| {
                backend_editor(&parent, ctx.clone(), Some(entry.clone()));
            });
        }
        let copy = gtk::Button::with_label(
            &ctx.t_or("modals.backend.copyOptions.remotesConfig", "Copy remotes"),
        );
        copy.set_valign(gtk::Align::Center);
        copy.set_tooltip_text(Some(&ctx.t_or(
            "modals.backend.copyOptions.description",
            "Copy remotes from the active backend to this one",
        )));
        {
            let backend = backend.clone();
            let ctx = ctx.clone();
            let parent = parent.clone();
            copy.connect_clicked(move |_| {
                let Some(source) = ctx.client() else {
                    return;
                };
                let dest = crate::rclone::RcClient::new(&backend.host, backend.port).with_auth(
                    if backend.user.is_empty() {
                        None
                    } else {
                        Some(backend.user.clone())
                    },
                    if backend.pass.is_empty() {
                        None
                    } else {
                        Some(backend.pass.clone())
                    },
                );
                let msg = match dest.copy_remotes_from(&source) {
                    Ok(n) => format!("{}: {n}", ctx.t_or("common.copied", "Copied remotes")),
                    Err(e) => e.to_string(),
                };
                let alert = adw::AlertDialog::new(
                    Some(&ctx.t_or("modals.backend.copyOptions.remotesConfig", "Copy remotes")),
                    Some(&msg),
                );
                alert.add_response("ok", &ctx.t("common.ok"));
                alert.present(Some(&parent));
            });
        }
        row.add_suffix(&test);
        row.add_suffix(&use_btn);
        row.add_suffix(&edit);
        row.add_suffix(&clone);
        row.add_suffix(&copy);
        row.add_suffix(&remove);
        list.append(&row);
    }
    let add =
        gtk::Button::with_label(&ctx.t_or("modals.backend.addBackend", "Add remote RC backend"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add.connect_clicked(move |_| {
            backend_editor(&parent, ctx.clone(), None);
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&scrolled_list(&list));
    box_.append(&add);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn rc_client_for(ctx: &AppCtx, name: &str) -> Option<crate::rclone::RcClient> {
    if name.is_empty() || name.eq_ignore_ascii_case("local") {
        return ctx.client();
    }
    ctx.settings
        .borrow()
        .core
        .extra_backends
        .iter()
        .find(|b| b.name == name)
        .map(rc_client_for_entry)
}

fn rc_client_for_entry(entry: &crate::settings::BackendEntry) -> crate::rclone::RcClient {
    crate::rclone::RcClient::new(&entry.host, entry.port).with_auth(
        if entry.user.is_empty() {
            None
        } else {
            Some(entry.user.clone())
        },
        if entry.pass.is_empty() {
            None
        } else {
            Some(entry.pass.clone())
        },
    )
}

fn backend_editor(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<crate::settings::BackendEntry>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("modals.backend.addTitle", "Remote RC backend"));
    dialog.set_content_width(480);
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("modals.backend.name", "Name"));
    let host = adw::EntryRow::new();
    host.set_title(&ctx.t_or("modals.backend.fields.host.label", "Host"));
    host.set_text("127.0.0.1");
    let port = adw::EntryRow::new();
    port.set_title(&ctx.t_or("modals.backend.fields.port.label", "Port"));
    port.set_text("5573");
    let user = adw::EntryRow::new();
    user.set_title(&ctx.t_or("modals.backend.fields.username.label", "Username"));
    let pass = adw::PasswordEntryRow::new();
    pass.set_title(&ctx.t_or("modals.backend.fields.password.label", "Password"));
    let config_path = adw::EntryRow::new();
    config_path.set_title(&ctx.t_or("modals.backend.fields.configPath.label", "rclone.conf path"));
    let config_pass = adw::PasswordEntryRow::new();
    config_pass.set_title(&ctx.t_or(
        "modals.backend.fields.configPassword.label",
        "Config password",
    ));
    if let Some(entry) = &existing {
        name.set_text(&entry.name);
        host.set_text(&entry.host);
        port.set_text(&entry.port.to_string());
        user.set_text(&entry.user);
        pass.set_text(&entry.pass);
        config_path.set_text(&entry.config_path);
        config_pass.set_text(&entry.config_password);
    }
    let mut copy_labels = vec![
        ctx.t_or("modals.backend.copyOptions.fromZero", "Don't copy remotes"),
        ctx.t_or("modals.backend.local", "Local rclone RC"),
    ];
    let mut copy_ids = vec![String::new(), "local".to_string()];
    for backend in ctx.settings.borrow().core.extra_backends.clone() {
        if existing
            .as_ref()
            .map(|e| e.name == backend.name)
            .unwrap_or(false)
        {
            continue;
        }
        copy_labels.push(backend.name.clone());
        copy_ids.push(backend.name);
    }
    let copy_refs: Vec<&str> = copy_labels.iter().map(|s| s.as_str()).collect();
    let copy_from = gtk::DropDown::from_strings(&copy_refs);
    copy_from.set_selected(0);
    let copy_row = adw::ActionRow::new();
    copy_row.set_title(&ctx.t_or(
        "modals.backend.copyOptions.remotesConfig",
        "Copy remotes from",
    ));
    copy_row.set_subtitle(&ctx.t_or(
        "modals.backend.copyOptions.description",
        "Optional: clone remotes from another RC backend after saving",
    ));
    copy_row.add_suffix(&copy_from);
    let test =
        gtk::Button::with_label(&ctx.t_or("modals.backend.testConnection", "Test connection"));
    let save = gtk::Button::with_label(&ctx.t_or("modals.backend.save", "Save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let host = host.clone();
        let port = port.clone();
        let user = user.clone();
        let pass = pass.clone();
        test.connect_clicked(move |_| {
            let port_n = port.text().parse::<u16>().unwrap_or(5573);
            let entry = crate::settings::BackendEntry {
                name: "test".into(),
                host: host.text().to_string(),
                port: port_n,
                user: user.text().to_string(),
                pass: pass.text().to_string(),
                ..Default::default()
            };
            let client = rc_client_for_entry(&entry);
            let msg = if client.ping() {
                format!(
                    "{} — {}",
                    ctx.t_or("modals.backend.status.connected", "Connected"),
                    client
                        .version()
                        .unwrap_or_else(|_| ctx.t_or("backup.restore.unknown", "unknown"))
                )
            } else {
                ctx.t_or("modals.backend.status.failed", "Connection failed")
            };
            let alert = adw::AlertDialog::new(
                Some(&ctx.t_or("modals.backend.testConnection", "Test connection")),
                Some(&msg),
            );
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&dialog));
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let name = name.clone();
        let host = host.clone();
        let port = port.clone();
        let user = user.clone();
        let pass = pass.clone();
        let config_path = config_path.clone();
        let config_pass = config_pass.clone();
        let copy_from = copy_from.clone();
        save.connect_clicked(move |_| {
            let port_n = port.text().parse::<u16>().unwrap_or(5573);
            let entry = crate::settings::BackendEntry {
                name: name.text().to_string(),
                host: host.text().to_string(),
                port: port_n,
                user: user.text().to_string(),
                pass: pass.text().to_string(),
                config_path: config_path.text().to_string(),
                config_password: config_pass.text().to_string(),
            };
            if entry.name.is_empty() || entry.host.is_empty() {
                return;
            }
            let source_idx = copy_from.selected() as usize;
            let source_id = copy_ids.get(source_idx).cloned().unwrap_or_default();
            let mut settings = ctx.settings.borrow_mut();
            if let Some(idx) = settings
                .core
                .extra_backends
                .iter()
                .position(|b| b.name == entry.name)
            {
                settings.core.extra_backends[idx] = entry.clone();
            } else {
                settings.core.extra_backends.push(entry.clone());
            }
            drop(settings);
            ctx.persist();
            let options_from = if source_id.is_empty() {
                ctx.backend_key()
            } else {
                source_id.clone()
            };
            if options_from != entry.name {
                let dest_empty = crate::backend_options::load_for(&entry.name)
                    .as_object()
                    .is_none_or(|o| o.is_empty());
                if dest_empty {
                    if let Err(e) = crate::backend_options::copy_for(&options_from, &entry.name) {
                        log::warn!("copy backend.json failed: {e}");
                    }
                }
            }
            crate::rclone::apply_backend_rc_config(
                &rc_client_for_entry(&entry),
                Some(entry.config_path.as_str()),
                Some(entry.config_password.as_str()),
            );
            if !source_id.is_empty() {
                if let (Some(source), dest) =
                    (rc_client_for(&ctx, &source_id), rc_client_for_entry(&entry))
                {
                    if let Err(e) = dest.copy_remotes_from(&source) {
                        let alert =
                            adw::AlertDialog::new(
                                Some(&ctx.t_or(
                                    "modals.backend.copyOptions.remotesConfig",
                                    "Copy remotes",
                                )),
                                Some(&e.to_string()),
                            );
                        alert.add_response("ok", &ctx.t_or("common.ok", "OK"));
                        alert.present(Some(&parent));
                    }
                }
            }
            dialog.close();
            backends(&parent, ctx.clone());
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&host);
    group.add(&port);
    group.add(&user);
    group.add(&pass);
    group.add(&config_path);
    group.add(&config_pass);
    group.add(&copy_row);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.append(&test);
    buttons.append(&save);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn alerts(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    if try_spawn_standalone(&ctx, "alerts", serde_json::json!({})) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("alerts.title", "Alerts"));
    dialog.set_content_width(720);
    dialog.set_content_height(560);
    let stack = adw::ViewStack::new();
    let history = gtk::ListBox::new();
    history.add_css_class("boxed-list");
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&ctx.t_or("alerts.search", "Search history")));
    let severity =
        gtk::DropDown::from_strings(&["All", "Critical", "High", "Average", "Warning", "Info"]);
    let event_kind = gtk::DropDown::from_strings(&[
        "All kinds",
        "Job",
        "Serve",
        "Mount",
        "Engine",
        "Update",
        "Automation",
        "System",
    ]);
    let ack_state = gtk::DropDown::from_strings(&["All", "Open", "Acknowledged"]);
    let (remote_vals, profile_vals, backend_vals) = ctx.store.borrow().alert_filter_values();
    let mut remote_labels = vec!["All remotes".to_string()];
    remote_labels.extend(remote_vals);
    let mut profile_labels = vec!["All profiles".to_string()];
    profile_labels.extend(profile_vals);
    let mut backend_labels = vec!["All backends".to_string()];
    backend_labels.extend(backend_vals);
    let remote_refs: Vec<&str> = remote_labels.iter().map(|s| s.as_str()).collect();
    let profile_refs: Vec<&str> = profile_labels.iter().map(|s| s.as_str()).collect();
    let backend_refs: Vec<&str> = backend_labels.iter().map(|s| s.as_str()).collect();
    let remote_dd = gtk::DropDown::from_strings(&remote_refs);
    let profile_dd = gtk::DropDown::from_strings(&profile_refs);
    let backend_dd = gtk::DropDown::from_strings(&backend_refs);
    let mut rule_labels = vec!["All rules".to_string()];
    rule_labels.extend(
        ctx.store
            .borrow()
            .alert_rules
            .iter()
            .map(|rule| rule.id.clone()),
    );
    let mut origin_labels = vec!["All origins".to_string()];
    for event in &ctx.store.borrow().alert_history {
        if !event.origin.is_empty() && !origin_labels.iter().any(|item| item == &event.origin) {
            origin_labels.push(event.origin.clone());
        }
    }
    let rule_refs: Vec<&str> = rule_labels.iter().map(|s| s.as_str()).collect();
    let origin_refs: Vec<&str> = origin_labels.iter().map(|s| s.as_str()).collect();
    let rule_dd = gtk::DropDown::from_strings(&rule_refs);
    let origin_dd = gtk::DropDown::from_strings(&origin_refs);
    let fill_cell: Rc<RefCell<Rc<dyn Fn()>>> = Rc::new(RefCell::new(Rc::new(|| {})));
    let fill: Rc<dyn Fn()> = {
        let history = history.clone();
        let search = search.clone();
        let severity = severity.clone();
        let event_kind = event_kind.clone();
        let ack_state = ack_state.clone();
        let remote_dd = remote_dd.clone();
        let profile_dd = profile_dd.clone();
        let backend_dd = backend_dd.clone();
        let rule_dd = rule_dd.clone();
        let origin_dd = origin_dd.clone();
        let remote_labels = remote_labels.clone();
        let profile_labels = profile_labels.clone();
        let backend_labels = backend_labels.clone();
        let rule_labels = rule_labels.clone();
        let origin_labels = origin_labels.clone();
        let ctx = ctx.clone();
        let fill_cell = fill_cell.clone();
        Rc::new(move || {
            while let Some(child) = history.first_child() {
                history.remove(&child);
            }
            let stats = ctx.store.borrow().alert_stats();
            let stats_row = adw::ActionRow::new();
            stats_row.set_title(&ctx.t_or("alerts.stats", "History stats"));
            let last = stats
                .last_at
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "—".into());
            stats_row.set_subtitle(&format!(
                "{} total · {} ack · {} open · {} delivered · last {last}",
                stats.total, stats.acknowledged, stats.unacknowledged, stats.delivered
            ));
            history.append(&stats_row);
            let sev = match severity.selected() {
                1 => Some("critical"),
                2 => Some("high"),
                3 => Some("average"),
                4 => Some("warning"),
                5 => Some("info"),
                _ => None,
            };
            let kind = match event_kind.selected() {
                1 => Some("job"),
                2 => Some("serve"),
                3 => Some("mount"),
                4 => Some("engine"),
                5 => Some("update"),
                6 => Some("automation"),
                7 => Some("system"),
                _ => None,
            };
            let ack = match ack_state.selected() {
                1 => Some(false),
                2 => Some(true),
                _ => None,
            };
            let pick = |dd: &gtk::DropDown, labels: &[String]| {
                let idx = dd.selected() as usize;
                if idx == 0 {
                    None
                } else {
                    labels.get(idx).cloned()
                }
            };
            let events = ctx
                .store
                .borrow()
                .filter_alerts(&crate::store::AlertHistoryFilter {
                    query: search.text().to_string(),
                    severity: sev.map(|s| s.to_string()),
                    event_kind: kind.map(|s| s.to_string()),
                    remote: pick(&remote_dd, &remote_labels),
                    profile: pick(&profile_dd, &profile_labels),
                    backend: pick(&backend_dd, &backend_labels),
                    acknowledged: ack,
                    rule_id: pick(&rule_dd, &rule_labels),
                    origins: pick(&origin_dd, &origin_labels).into_iter().collect(),
                    ..crate::store::AlertHistoryFilter::default()
                });
            let mut shown = 0;
            for event in events.into_iter().take(80) {
                let row = adw::ActionRow::new();
                row.set_title(&event.title);
                row.set_subtitle(&format!(
                    "{} · {} · {}",
                    event.severity.as_str(),
                    event.kind.as_str(),
                    event.body
                ));
                if !event.acknowledged {
                    let ack_one = gtk::Button::from_icon_name("object-select-symbolic");
                    ack_one.set_tooltip_text(Some(&ctx.t_or("alerts.acknowledge", "Acknowledge")));
                    ack_one.set_valign(gtk::Align::Center);
                    let ctx = ctx.clone();
                    let id = event.id.clone();
                    let fill_cell = fill_cell.clone();
                    ack_one.connect_clicked(move |_| {
                        ctx.store.borrow_mut().acknowledge_alert(&id);
                        ctx.persist();
                        let fill = fill_cell.borrow().clone();
                        fill();
                    });
                    row.add_suffix(&ack_one);
                }
                history.append(&row);
                shown += 1;
            }
            if shown == 0 {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("alerts.noHistory", "No alert history"));
                history.append(&row);
            }
        })
    };
    *fill_cell.borrow_mut() = fill.clone();
    fill();
    {
        let fill = fill.clone();
        search.connect_search_changed(move |_| fill());
    }
    {
        let fill = fill.clone();
        severity.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        event_kind.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        ack_state.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        remote_dd.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        profile_dd.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        backend_dd.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        rule_dd.connect_selected_notify(move |_| fill());
    }
    {
        let fill = fill.clone();
        origin_dd.connect_selected_notify(move |_| fill());
    }
    let ack = gtk::Button::with_label(&ctx.t_or("alerts.acknowledgeAll", "Acknowledge all"));
    {
        let ctx = ctx.clone();
        let fill = fill.clone();
        ack.connect_clicked(move |_| {
            ctx.store.borrow_mut().acknowledge_all();
            ctx.persist();
            fill();
        });
    }
    let clear = gtk::Button::with_label(&ctx.t_or("alerts.clearHistory", "Clear history"));
    clear.add_css_class("destructive-action");
    {
        let ctx = ctx.clone();
        let fill = fill.clone();
        clear.connect_clicked(move |_| {
            ctx.store.borrow_mut().clear_alert_history();
            ctx.persist();
            fill();
        });
    }
    let filters = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let filter_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    search.set_hexpand(true);
    filter_row.append(&search);
    filter_row.append(&severity);
    filter_row.append(&event_kind);
    filter_row.append(&ack_state);
    let scope_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    scope_row.append(&remote_dd);
    scope_row.append(&profile_dd);
    scope_row.append(&backend_dd);
    scope_row.append(&rule_dd);
    scope_row.append(&origin_dd);
    filters.append(&filter_row);
    filters.append(&scope_row);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&ack);
    buttons.append(&clear);
    let history_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    history_box.append(&filters);
    history_box.append(&scrolled_list(&history));
    history_box.append(&buttons);
    let history_title = ctx.t_or("alerts.history", "History");
    stack.add_titled(&history_box, Some("history"), &history_title);

    let rules = gtk::ListBox::new();
    rules.add_css_class("boxed-list");
    for rule in ctx.store.borrow().alert_rules.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&rule.name);
        row.set_subtitle(&format!(
            "min {} · {} actions · {}",
            rule.severity_min.as_str(),
            rule.action_ids.len(),
            if rule.enabled { "on" } else { "off" }
        ));
        let enabled = gtk::Switch::new();
        enabled.set_valign(gtk::Align::Center);
        enabled.set_active(rule.enabled);
        {
            let ctx = ctx.clone();
            let id = rule.id.clone();
            enabled.connect_active_notify(move |sw| {
                if let Some(rule) = ctx
                    .store
                    .borrow_mut()
                    .alert_rules
                    .iter_mut()
                    .find(|r| r.id == id)
                {
                    if rule.enabled != sw.is_active() {
                        rule.enabled = sw.is_active();
                    }
                }
                ctx.persist();
            });
        }
        row.add_suffix(&enabled);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let id = rule.id.clone();
            row.connect_activated(move |_| {
                alert_rule_editor(&parent, ctx.clone(), Some(id.clone()));
            });
        }
        rules.append(&row);
    }
    let add_rule = gtk::Button::with_label(&ctx.t_or("alerts.addRule", "Add rule"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add_rule.connect_clicked(move |_| {
            alert_rule_editor(&parent, ctx.clone(), None);
        });
    }
    let rules_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    rules_box.append(&scrolled_list(&rules));
    rules_box.append(&add_rule);
    let rules_title = ctx.t_or("alerts.rules", "Rules");
    stack.add_titled(&rules_box, Some("rules"), &rules_title);

    let actions = gtk::ListBox::new();
    actions.add_css_class("boxed-list");
    for action in ctx.store.borrow().alert_actions.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&action.name);
        row.set_subtitle(&action.kind);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let id = action.id.clone();
            row.connect_activated(move |_| {
                alert_action_editor(&parent, ctx.clone(), Some(id.clone()));
            });
        }
        actions.append(&row);
    }
    let add_action = gtk::Button::with_label(&ctx.t_or("alerts.addAction", "Add action"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add_action.connect_clicked(move |_| {
            alert_action_editor(&parent, ctx.clone(), None);
        });
    }
    let actions_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    actions_box.append(&scrolled_list(&actions));
    actions_box.append(&add_action);
    let actions_title = ctx.t_or("alerts.actions", "Actions");
    stack.add_titled(&actions_box, Some("actions"), &actions_title);

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&switcher);
    box_.append(&stack);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
    let _ = AlertSeverity::Info;
    let _ = AlertEvent::new(
        AlertEventKind::System,
        AlertSeverity::Info,
        String::new(),
        String::new(),
    );
}

pub fn quick_add_remote(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    if try_spawn_standalone(&ctx, "quick-add-remote", serde_json::json!({})) {
        return;
    }
    super::wizard::present_quick_add(parent, ctx, on_done);
}

pub fn remote_config(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    on_done: Rc<dyn Fn()>,
) {
    remote_config_open(
        parent,
        ctx,
        existing,
        super::remote_config::RemoteConfigOpen::default(),
        on_done,
    );
}

pub fn remote_config_open(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    open: super::remote_config::RemoteConfigOpen,
    on_done: Rc<dyn Fn()>,
) {
    if try_spawn_standalone(
        &ctx,
        "remote-config",
        serde_json::json!({
            "remote": existing.clone().unwrap_or_default(),
            "initial": open.initial.clone().unwrap_or_default(),
            "profile": open.profile.clone().unwrap_or_default(),
            "autoAdd": open.auto_add
        }),
    ) {
        return;
    }
    if let Some(name) = existing {
        super::remote_config::present_with(parent, ctx, name, open, on_done);
    } else {
        super::wizard::present(parent, ctx, None, on_done);
    }
}

fn remote_editor(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    quick: bool,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    let title = if quick {
        ctx.t_or("wizards.quickAdd.title", "Quick Add Remote")
    } else {
        ctx.t_or("general.remoteConfig.title.add", "Remote Configuration")
    };
    dialog.set_title(&title);
    dialog.set_content_width(520);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("wizards.remoteConfig.remoteName", "Remote name"));
    if let Some(existing) = &existing {
        name.set_text(existing);
        name.set_sensitive(false);
    }
    let providers = ctx
        .client()
        .and_then(|c| c.providers().ok())
        .and_then(|v| {
            v.get("providers").and_then(|p| p.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.get("Name")
                            .or_else(|| item.get("Prefix"))
                            .or_else(|| item.get("name"))
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_else(|| {
            [
                "drive", "s3", "dropbox", "onedrive", "sftp", "ftp", "webdav", "local", "crypt",
                "alias", "union", "smb", "b2", "box", "mega", "pcloud", "seafile",
            ]
            .into_iter()
            .map(String::from)
            .collect()
        });
    let type_labels: Vec<&str> = providers.iter().map(|s| s.as_str()).collect();
    let type_row = adw::ComboRow::new();
    type_row.set_title(&ctx.t_or("wizards.remoteConfig.remoteType", "Provider type"));
    type_row.set_model(Some(&gtk::StringList::new(&type_labels)));
    if let Some(idx) = providers.iter().position(|p| p == "drive") {
        type_row.set_selected(idx as u32);
    }
    let extra = adw::EntryRow::new();
    extra.set_title(&ctx.t_or(
        "wizards.remoteConfig.additionalConfig",
        "Parameters (key=value;key=value)",
    ));
    group.add(&name);
    group.add(&type_row);
    group.add(&extra);
    page.add(&group);

    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let on_done = on_done.clone();
        save.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            let r#type = providers
                .get(type_row.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "drive".into());
            if remote_name.is_empty() || r#type.is_empty() {
                return;
            }
            let mut params = serde_json::Map::new();
            for pair in extra.text().split(';') {
                if let Some((k, v)) = pair.split_once('=') {
                    params.insert(k.trim().into(), serde_json::Value::String(v.trim().into()));
                }
            }
            if let Some(client) = ctx.client() {
                let result = if existing.is_some() {
                    client.update_remote(&remote_name, serde_json::Value::Object(params), None)
                } else {
                    client.create_remote(
                        &remote_name,
                        &r#type,
                        serde_json::Value::Object(params),
                        None,
                    )
                };
                match result {
                    Ok(_) => {
                        ctx.store
                            .borrow_mut()
                            .push_log(&remote_name, "Remote saved".into());
                        ctx.persist();
                        on_done();
                        dialog.close();
                    }
                    Err(e) => {
                        let toast = adw::AlertDialog::new(Some("Error"), Some(&e.to_string()));
                        toast.add_response("ok", &ctx.t_or("common.ok", "OK"));
                        toast.present(Some(&dialog));
                    }
                }
            }
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.append(&page);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn start_operation(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    op: OperationType,
    toast: adw::ToastOverlay,
    on_done: Rc<dyn Fn()>,
) {
    if try_spawn_standalone(
        &ctx,
        "start-operation",
        serde_json::json!({ "remote": remote, "operation": op.as_str() }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("{} — {remote}", op.api_label()));
    dialog.set_content_width(560);
    dialog.set_content_height(640);

    let profiles = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .and_then(|m| m.profiles.get(op.as_str()).cloned())
        .unwrap_or_default();
    let mut names: Vec<String> = profiles.keys().cloned().collect();
    if names.is_empty() {
        names.push("default".into());
    }
    names.sort();
    let profile_row = adw::ComboRow::new();
    profile_row.set_title(&ctx.t_or("modals.jobDetail.fields.profile", "Profile"));
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    profile_row.set_model(Some(&gtk::StringList::new(&refs)));

    let initial = profiles
        .get(&names[0])
        .cloned()
        .unwrap_or_else(ProfileConfig::default);
    let rclone = flatten_rclone(&initial.rclone);
    let src = adw::EntryRow::new();
    src.set_title(&if op == OperationType::Copyurl {
        ctx.t_or("wizards.appOperation.urlLabel", "URL")
    } else {
        ctx.t_or("remoteConfig.source", "Source")
    });
    src.set_text(&default_source(remote, &rclone));
    let src_kind = if op != OperationType::Copyurl {
        attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
        Some(attach_path_kind(&ctx, &src, remote))
    } else {
        None
    };
    let extra_sources: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    let more_src = crate::jobs::path_list(&rclone, crate::jobs::SOURCE_KEYS);
    if more_src.len() > 1 {
        for extra in more_src.iter().skip(1) {
            let row = adw::EntryRow::new();
            row.set_title(&ctx.t_or("wizards.appOperation.addSource", "Additional source"));
            row.set_text(extra);
            attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
            extra_sources.borrow_mut().push(row);
        }
    }
    let dst = adw::EntryRow::new();
    dst.set_title(&match op {
        OperationType::Mount => ctx.t_or("remoteConfig.mountPoint", "Mount point"),
        OperationType::Serve => ctx.t_or("remoteConfig.listenAddress", "Listen address"),
        OperationType::Copyurl => ctx.t_or("remoteConfig.destinationFs", "Destination fs"),
        _ => ctx.t_or("remoteConfig.dest", "Destination"),
    });
    dst.set_text(&{
        let template = if op == OperationType::Bisync {
            ctx.settings.borrow().core.default_bisync_directory.clone()
        } else {
            ctx.settings.borrow().core.default_mount_directory.clone()
        };
        if op == OperationType::Mount || op == OperationType::Bisync {
            crate::path_inspection::suggest_default_op_path(
                remote,
                op,
                &ctx.store.borrow(),
                &template,
            )
        } else {
            default_dest(remote, &rclone, op)
        }
    });
    let dst_kind = if op == OperationType::Mount {
        attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::local_folders());
        None
    } else if op == OperationType::Serve {
        None
    } else {
        attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
        Some(attach_path_kind(&ctx, &dst, remote))
    };
    let dest_status = gtk::Label::new(None);
    dest_status.add_css_class("dim-label");
    dest_status.set_xalign(0.0);
    dest_status.set_wrap(true);
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
            dest_status.set_text(&crate::path_inspection::describe_status(&status));
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
    if let Some(t) = rclone.get("type").and_then(|x| x.as_str()) {
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
    let vfs_title = ctx.t_or("remoteConfig.vfsProfile", "VFS profile");
    let filter_title = ctx.t_or("remoteConfig.filterProfile", "Filter profile");
    let backend_title = ctx.t_or("remoteConfig.backendProfile", "Backend profile");
    let vfs_row = helper_combo(&vfs_title, &vfs_names, &initial.app.vfs_profile);
    let filter_row = helper_combo(&filter_title, &filter_names, &initial.app.filter_profile);
    let backend_row = helper_combo(&backend_title, &backend_names, &initial.app.backend_profile);
    let dry = adw::SwitchRow::new();
    dry.set_title(&ctx.t_or("remoteConfig.dryRun", "Dry run"));
    dry.set_active(crate::jobs::is_dry_run(&rclone));
    let resync = adw::SwitchRow::new();
    resync.set_title(&ctx.t_or("dashboard.appDetail.resync", "Resync"));
    resync.set_subtitle(&ctx.t_or(
        "dashboard.appDetail.resyncActive",
        "Force a bisync resync on this start",
    ));
    resync.set_visible(op == OperationType::Bisync);
    resync.set_active(
        rclone
            .get("Resync")
            .or_else(|| rclone.get("resync"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    );

    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&ctx.t_or("remoteConfig.flags", "Operation flags"));
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let live_blocks = operation_flag_blocks(&ctx);
    for flag in crate::flags::merged_flags_for(op, &live_blocks) {
        if op == OperationType::Serve && flag.field_name == "type" {
            continue;
        }
        let row = flag_value_row(&flag, &rclone);
        flags_group.add(&row);
        flag_rows
            .borrow_mut()
            .push((flag.field_name, row, flag.type_name));
    }
    let serve_flag_rows: Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    if op == OperationType::Serve {
        if let Some(client) = ctx.client() {
            if let Ok(info) = client.options_info() {
                let blocks = crate::flags::parse_options_info(&info);
                for serve_type in serve_types.iter() {
                    for flag in crate::flags::collect_serve_flags(&blocks, serve_type) {
                        let row = adw::EntryRow::new();
                        row.set_title(&format!("{serve_type} · {}", flag.name));
                        if !flag.help.is_empty() {
                            row.set_tooltip_text(Some(&flag.help));
                        }
                        let current = rclone
                            .get(&flag.field_name)
                            .map(|v| v.to_string().trim_matches('"').to_string())
                            .unwrap_or_else(|| flag.default_str.clone());
                        row.set_text(&current);
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
    attach_cli_import(&ctx, &flags_group, flag_rows.clone());

    {
        let profiles = profiles.clone();
        let names = names.clone();
        let src = src.clone();
        let dst = dst.clone();
        let remote = remote.to_string();
        profile_row.connect_selected_notify(move |row| {
            let Some(name) = names.get(row.selected() as usize) else {
                return;
            };
            let Some(profile) = profiles.get(name) else {
                return;
            };
            let rclone = flatten_rclone(&profile.rclone);
            src.set_text(&default_source(&remote, &rclone));
            dst.set_text(&default_dest(&remote, &rclone, op));
        });
    }

    let start = gtk::Button::with_label(&ctx.t_or("operations.start", "Start"));
    start.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let mount_types = mount_types.clone();
        let mount_type = mount_type.clone();
        let dry = dry.clone();
        let resync = resync.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let extra_sources = extra_sources.clone();
        let vfs_row = vfs_row.clone();
        let filter_row = filter_row.clone();
        let backend_row = backend_row.clone();
        let vfs_names = vfs_names.clone();
        let filter_names = filter_names.clone();
        let backend_names = backend_names.clone();
        start.connect_clicked(move |_| {
            let Some(client) = ctx.client() else {
                toast.add_toast(adw::Toast::new(&ctx.t_or(
                    "notification.title.engineConnectionFailed",
                    "Engine Connection Error",
                )));
                return;
            };
            let mut rclone = serde_json::Map::new();
            if op == OperationType::Serve {
                rclone.insert(
                    "type".into(),
                    serde_json::json!(crate::operations::selected_or(
                        &serve_types,
                        serve.selected(),
                        "webdav"
                    )),
                );
            }
            if op == OperationType::Mount {
                rclone.insert(
                    "mountType".into(),
                    serde_json::json!(crate::operations::selected_or(
                        &mount_types,
                        mount_type.selected(),
                        "mount"
                    )),
                );
            }
            if dry.is_active() {
                rclone.insert("dryRun".into(), serde_json::json!(true));
            }
            if resync.is_active() {
                rclone.insert("Resync".into(), serde_json::json!(true));
            }
            for (field, row, type_name) in flag_rows.borrow().iter() {
                let text = row.text().to_string();
                if !text.is_empty() {
                    rclone.insert(
                        field.clone(),
                        crate::flags::parse_flag_value(type_name, &text),
                    );
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
                    rclone.insert(
                        field.clone(),
                        crate::flags::parse_flag_value(type_name, &text),
                    );
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
            let dest = if op == OperationType::Serve {
                dst.text().to_string()
            } else {
                crate::path_kind::resolve_job_path(&dst.text(), &remote)
            };
            let mut rclone = serde_json::Value::Object(rclone);
            if let Some(meta) = ctx.store.borrow().remotes.get(&remote) {
                let mut profile = crate::store::ProfileConfig::default();
                profile.app.vfs_profile = helper_selected(&vfs_row, &vfs_names);
                profile.app.filter_profile = helper_selected(&filter_row, &filter_names);
                profile.app.backend_profile = helper_selected(&backend_row, &backend_names);
                crate::jobs::apply_helper_options(&mut rclone, &profile, Some(meta));
            }
            let mut ids = Vec::new();
            let mut error = None;
            for source in sources {
                match build_job_params(op, &remote, &source, &dest, &rclone) {
                    Ok(req) => match start_request(&client, &req) {
                        Ok(id) => ids.push(id),
                        Err(e) => {
                            error = Some(e.to_string());
                            break;
                        }
                    },
                    Err(e) => {
                        error = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = error {
                let err = adw::AlertDialog::new(
                    Some(&ctx.t_or("remoteConfig.startFailed", "Start failed")),
                    Some(&e),
                );
                err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                err.present(Some(&dialog));
            } else {
                ctx.store
                    .borrow_mut()
                    .push_log(&remote, format!("started {op} {}", ids.join(", ")));
                ctx.refresh_runtime();
                toast.add_toast(adw::Toast::new(&ctx.tf(
                    "operations.successStart",
                    &[
                        ("type", op.api_label()),
                        ("remote", &remote),
                        ("profile", "start"),
                    ],
                )));
                on_done();
                dialog.close();
            }
        });
    }

    let page = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    identity.add(&profile_row);
    if let Some(kind) = &src_kind {
        identity.add(kind);
    }
    identity.add(&src);
    for row in extra_sources.borrow().iter() {
        identity.add(row);
    }
    if op.supports_multi_source() {
        let add_src = gtk::Button::with_label(&ctx.t_or("remoteConfig.addSource", "Add source"));
        {
            let extra_sources = extra_sources.clone();
            let identity = identity.clone();
            let ctx = ctx.clone();
            add_src.connect_clicked(move |_| {
                let row = adw::EntryRow::new();
                row.set_title(&ctx.t_or("wizards.appOperation.addSource", "Additional source"));
                attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
                identity.add(&row);
                extra_sources.borrow_mut().push(row);
            });
        }
        let add_row = adw::ActionRow::new();
        add_row.set_title(&ctx.t_or("remoteConfig.multipleSources", "Multiple sources"));
        add_row.add_suffix(&add_src);
        identity.add(&add_row);
    }
    if let Some(kind) = &dst_kind {
        identity.add(kind);
    }
    identity.add(&dst);
    identity.add(&{
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("remoteConfig.pathStatusTitle", "Path check"));
        row.add_suffix(&dest_status);
        row
    });
    identity.add(&serve);
    identity.add(&mount_type);
    identity.add(&vfs_row);
    identity.add(&filter_row);
    identity.add(&backend_row);
    identity.add(&dry);
    identity.add(&resync);
    page.add(&identity);
    page.add(&flags_group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    box_.append(&scroll);
    box_.append(&start);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn delete_remote(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    name: &str,
    on_done: Rc<dyn Fn()>,
) {
    if try_spawn_standalone(&ctx, "delete-remote", serde_json::json!({ "remote": name })) {
        return;
    }
    let plan = crate::store::plan_delete_remote(name, &ctx.store.borrow(), &ctx.snapshot.borrow());
    let dialog = adw::AlertDialog::new(Some("Delete remote"), Some(&plan.summary()));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let name = name.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            if let Some(client) = ctx.client() {
                for mount in &plan.mounts {
                    let _ = client.unmount(mount);
                }
                for serve in &plan.serves {
                    let _ = client.serve_stop(serve);
                }
                for job in &plan.jobs {
                    let _ = client.job_stop(*job);
                }
                let _ = client.delete_remote(&name);
            }
            ctx.store.borrow_mut().apply_delete_remote(&name);
            ctx.persist();
            ctx.refresh_runtime();
            on_done();
        }
    });
    dialog.present(Some(parent));
}

pub fn clone_remote(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    name: &str,
    on_done: Rc<dyn Fn()>,
) {
    let existing = ctx.store.borrow().remote_names();
    let new_name = crate::store::unique_remote_name(&existing, name);
    let Some(meta) = crate::store::clone_remote_meta(&ctx.store.borrow(), name, &new_name) else {
        let toast = adw::AlertDialog::new(
            Some(&ctx.t_or("home.options.cloneFailed", "Clone failed")),
            Some(&ctx.t_or(
                "home.options.cloneNoSettings",
                "No saved settings were found for this remote.",
            )),
        );
        toast.add_response("ok", &ctx.t_or("common.ok", "OK"));
        toast.present(Some(parent));
        return;
    };
    if let Some(client) = ctx.client() {
        if let Err(e) = client.clone_remote_config(name, &new_name) {
            let toast = adw::AlertDialog::new(
                Some(&ctx.t_or("home.options.cloneFailed", "Clone failed")),
                Some(&e.to_string()),
            );
            toast.add_response("ok", &ctx.t_or("common.ok", "OK"));
            toast.present(Some(parent));
            return;
        }
    }
    {
        let mut store = ctx.store.borrow_mut();
        store.remotes.insert(new_name.clone(), meta);
        store.ensure_remote_order(&[new_name.clone()]);
    }
    ctx.persist();
    ctx.refresh_runtime();
    remote_config(parent, ctx, Some(new_name), on_done);
}

pub fn quick_run_editor(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<QuickRun>,
    on_done: Rc<dyn Fn()>,
) {
    let spawn_data = existing
        .as_ref()
        .and_then(|qr| serde_json::to_value(qr).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if try_spawn_standalone(&ctx, "quick-run-editor", spawn_data) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&if existing.is_some() {
        ctx.t_or("flow.quickRun.editor.editTitle", "Edit Quick Run")
    } else {
        ctx.t_or("flow.quickRun.editor.createTitle", "Create Quick Run")
    });
    dialog.set_content_width(520);
    let group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("flow.quickRun.editor.name", "Name"));
    let description = adw::EntryRow::new();
    description.set_title(&ctx.t_or("flow.quickRun.editor.description", "Description"));
    let remote = adw::EntryRow::new();
    remote.set_title(&ctx.t_or("flow.quickRun.editor.remote", "Remote"));
    let src = adw::EntryRow::new();
    src.set_title(&ctx.t_or("fileBrowser.operations.details.source", "Source"));
    let extra_sources: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    let extra_filenames: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    let url_filename = adw::EntryRow::new();
    url_filename.set_title(&ctx.t_or(
        "wizards.appOperation.copyUrlFilename",
        "Filename (optional)",
    ));
    let dst = adw::EntryRow::new();
    dst.set_title(&ctx.t_or(
        "fileBrowser.operations.details.destination",
        "Destination / mount point",
    ));
    attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
    let cron = adw::EntryRow::new();
    cron.set_title(&ctx.t_or("flow.quickRun.editor.cron", "Cron expression"));
    let cron_hint = gtk::Label::new(None);
    cron_hint.add_css_class("dim-label");
    cron_hint.set_xalign(0.0);
    cron_hint.set_wrap(true);
    {
        let cron_hint = cron_hint.clone();
        let ctx = ctx.clone();
        cron.connect_changed(move |row| {
            let text = row.text().to_string();
            if text.is_empty() {
                cron_hint.set_text("");
            } else if let Err(e) = validate_cron(&text) {
                cron_hint.set_text(&e);
            } else {
                cron_hint.set_text(&describe_cron_i18n(&text, &ctx.i18n.borrow()));
            }
        });
    }
    let cron_presets = attach_cron_builder(&cron, &ctx);
    let op_row = adw::ComboRow::new();
    op_row.set_title(&ctx.t_or("wizards.cliImport.operation", "Operation"));
    let labels: Vec<&str> = OperationType::ALL.iter().map(|o| o.as_str()).collect();
    op_row.set_model(Some(&gtk::StringList::new(&labels)));
    let auto = adw::SwitchRow::new();
    auto.set_title(&ctx.t_or("flow.quickRun.badges.autostart", "Auto start"));
    let watch = adw::SwitchRow::new();
    watch.set_title(&ctx.t_or("flow.quickRun.badges.watcher", "Watch enabled"));
    let watch_delay = adw::EntryRow::new();
    watch_delay.set_title(&ctx.t_or("wizards.appOperation.watchDelay", "Watch delay (seconds)"));
    let watch_changed = adw::SwitchRow::new();
    watch_changed.set_title(&ctx.t_or(
        "automation.monitoring.changedOnlyShort",
        "Changed files only",
    ));
    let tray = adw::SwitchRow::new();
    tray.set_title(&ctx.t_or("flow.quickRun.editor.showOnTray", "Show on tray"));
    let runtime_json = adw::EntryRow::new();
    runtime_json.set_title(&ctx.t_or(
        "flow.quickRun.editor.runtimeRemoteJson",
        "Runtime remote JSON",
    ));
    if let Some(qr) = &existing {
        name.set_text(&qr.name);
        description.set_text(&qr.description);
        remote.set_text(&qr.remote_name);
        let (s, d) = qr.paths();
        let copyurl = qr.operation_type == OperationType::Copyurl;
        let sources = if copyurl {
            path_list(&qr.config.rclone, &["url", "srcFs", "source"])
        } else {
            path_list(&qr.config.rclone, SOURCE_KEYS)
        };
        if let Some(first) = sources.first() {
            src.set_text(first);
        } else if let Some(s) = s {
            src.set_text(&s);
        }
        dst.set_text(&d.unwrap_or_default());
        let saved_filenames: Vec<String> = qr
            .config
            .rclone
            .get("filenames")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|v| v.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(first) = saved_filenames.first() {
            url_filename.set_text(first);
        }
        for extra in sources.iter().skip(1) {
            let row = adw::EntryRow::new();
            row.set_title(&ctx.t_or("wizards.appOperation.addSource", "Additional source"));
            if !copyurl {
                attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
            }
            row.set_text(extra);
            extra_sources.borrow_mut().push(row);
        }
        if copyurl {
            for name in saved_filenames.iter().skip(1) {
                extra_filenames
                    .borrow_mut()
                    .push(quick_run_filename_row(&ctx, name));
            }
            while extra_filenames.borrow().len() < extra_sources.borrow().len() {
                extra_filenames
                    .borrow_mut()
                    .push(quick_run_filename_row(&ctx, ""));
            }
        }
        cron.set_text(&qr.config.app.cron_expression);
        auto.set_active(qr.config.app.auto_start);
        watch.set_active(qr.config.app.watch_enabled);
        if qr.config.app.watch_delay > 0 {
            watch_delay.set_text(&qr.config.app.watch_delay.to_string());
        }
        watch_changed.set_active(qr.config.app.watch_changed_only);
        tray.set_active(qr.show_on_tray);
        if let Some(value) = qr.config.rclone.get("runtimeRemote") {
            runtime_json.set_text(&value.to_string());
        }
        if let Some(idx) = OperationType::ALL
            .iter()
            .position(|o| *o == qr.operation_type)
        {
            op_row.set_selected(idx as u32);
        }
    }
    let remote_name = existing
        .as_ref()
        .map(|qr| qr.remote_name.clone())
        .unwrap_or_else(|| remote.text().to_string());
    let vfs_names = Rc::new(RefCell::new(qr_helper_names(&ctx, &remote_name, "vfs")));
    let filter_names = Rc::new(RefCell::new(qr_helper_names(&ctx, &remote_name, "filter")));
    let backend_names = Rc::new(RefCell::new(qr_helper_names(&ctx, &remote_name, "backend")));
    let runtime_names = Rc::new(RefCell::new(qr_helper_names(&ctx, &remote_name, "runtime")));
    let vfs_sel = existing
        .as_ref()
        .map(|qr| qr.config.app.vfs_profile.clone())
        .unwrap_or_default();
    let filter_sel = existing
        .as_ref()
        .map(|qr| qr.config.app.filter_profile.clone())
        .unwrap_or_default();
    let backend_sel = existing
        .as_ref()
        .map(|qr| qr.config.app.backend_profile.clone())
        .unwrap_or_default();
    let runtime_sel = existing
        .as_ref()
        .map(|qr| qr.config.app.runtime_remote_profile.clone())
        .unwrap_or_default();
    let vfs_profile = helper_combo(
        &ctx.t_or("flow.quickRun.editor.tabVfs", "VFS profile"),
        &vfs_names.borrow(),
        &vfs_sel,
    );
    let filter_profile = helper_combo(
        &ctx.t_or("flow.quickRun.editor.tabFilter", "Filter profile"),
        &filter_names.borrow(),
        &filter_sel,
    );
    let backend_profile = helper_combo(
        &ctx.t_or("flow.quickRun.editor.tabBackend", "Backend profile"),
        &backend_names.borrow(),
        &backend_sel,
    );
    let runtime_profile = helper_combo(
        &ctx.t_or(
            "flow.quickRun.editor.tabRuntimeRemote",
            "Runtime remote profile",
        ),
        &runtime_names.borrow(),
        &runtime_sel,
    );
    let edit_helpers =
        gtk::Button::with_label(&ctx.t_or("remote.editHelpers", "Edit helper profiles"));
    {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let parent = parent.clone();
        edit_helpers.connect_clicked(move |_| {
            helper_profiles(&parent, ctx.clone(), &remote.text());
        });
    }
    let helpers_row = adw::ActionRow::new();
    helpers_row.set_title(&ctx.t_or(
        "general.remoteConfig.advancedProfiles.title",
        "Helper profiles",
    ));
    helpers_row.add_suffix(&edit_helpers);
    {
        let ctx = ctx.clone();
        let vfs_profile = vfs_profile.clone();
        let filter_profile = filter_profile.clone();
        let backend_profile = backend_profile.clone();
        let runtime_profile = runtime_profile.clone();
        let vfs_names = vfs_names.clone();
        let filter_names = filter_names.clone();
        let backend_names = backend_names.clone();
        let runtime_names = runtime_names.clone();
        remote.connect_changed(move |row| {
            let name = row.text().to_string();
            *vfs_names.borrow_mut() = qr_helper_names(&ctx, &name, "vfs");
            *filter_names.borrow_mut() = qr_helper_names(&ctx, &name, "filter");
            *backend_names.borrow_mut() = qr_helper_names(&ctx, &name, "backend");
            *runtime_names.borrow_mut() = qr_helper_names(&ctx, &name, "runtime");
            refresh_helper_combo(&vfs_profile, &vfs_names.borrow(), "");
            refresh_helper_combo(&filter_profile, &filter_names.borrow(), "");
            refresh_helper_combo(&backend_profile, &backend_names.borrow(), "");
            refresh_helper_combo(&runtime_profile, &runtime_names.borrow(), "");
        });
    }
    group.add(&name);
    group.add(&description);
    group.add(&remote);
    let initial_op = existing
        .as_ref()
        .map(|qr| qr.operation_type)
        .unwrap_or(OperationType::Sync);
    if initial_op != OperationType::Copyurl {
        attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
    }
    apply_quick_run_path_titles(&ctx, initial_op, &src, &dst);
    url_filename.set_visible(initial_op == OperationType::Copyurl);
    let src_kind = attach_path_kind(&ctx, &src, &remote.text());
    let dst_kind = attach_path_kind(&ctx, &dst, &remote.text());
    src_kind.set_visible(initial_op != OperationType::Copyurl);
    dst_kind.set_visible(!matches!(
        initial_op,
        OperationType::Mount | OperationType::Serve | OperationType::Delete
    ));
    let current_op = {
        let op_row = op_row.clone();
        Rc::new(move || {
            OperationType::ALL
                .get(op_row.selected() as usize)
                .copied()
                .unwrap_or(OperationType::Sync)
        }) as Rc<dyn Fn() -> OperationType>
    };
    let (guidance, refresh_guidance) = attach_operation_guidance(
        &ctx,
        false,
        &watch,
        &src,
        &extra_sources,
        &dst,
        current_op.clone(),
    );
    group.add(&op_row);
    group.add(&src_kind);
    group.add(&src);
    group.add(&url_filename);
    for (idx, row) in extra_sources.borrow().iter().enumerate() {
        group.add(row);
        if let Some(name) = extra_filenames.borrow().get(idx) {
            group.add(name);
        }
    }
    let add_src = gtk::Button::with_label(&ctx.t_or("remoteConfig.addSource", "Add source"));
    {
        let extra_sources = extra_sources.clone();
        let extra_filenames = extra_filenames.clone();
        let group = group.clone();
        let ctx = ctx.clone();
        let refresh_guidance = refresh_guidance.clone();
        let current_op = current_op.clone();
        add_src.connect_clicked(move |_| {
            let op = current_op();
            let row = adw::EntryRow::new();
            row.set_title(&ctx.t_or("wizards.appOperation.addSource", "Additional source"));
            if op != OperationType::Copyurl {
                attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
            }
            {
                let refresh_guidance = refresh_guidance.clone();
                row.connect_changed(move |_| refresh_guidance());
            }
            group.add(&row);
            extra_sources.borrow_mut().push(row);
            if op == OperationType::Copyurl {
                let name = quick_run_filename_row(&ctx, "");
                group.add(&name);
                extra_filenames.borrow_mut().push(name);
            }
            refresh_guidance();
        });
    }
    let add_src_row = adw::ActionRow::new();
    add_src_row.set_title(&ctx.t_or("remoteConfig.multipleSources", "Multiple sources"));
    add_src_row.add_suffix(&add_src);
    add_src_row.set_visible(initial_op.supports_multi_source());
    group.add(&add_src_row);
    group.add(&dst_kind);
    group.add(&dst);
    group.add(&cron);
    let cron_preset_row = adw::ActionRow::new();
    cron_preset_row.set_title(&ctx.t_or("flow.quickRun.editor.cron", "Cron schedule"));
    cron_preset_row.add_suffix(&cron_presets);
    group.add(&cron_preset_row);
    group.add(&auto);
    group.add(&watch);
    group.add(&watch_delay);
    group.add(&watch_changed);
    group.add(&tray);
    vfs_profile.set_visible(initial_op.supports_vfs());
    group.add(&vfs_profile);
    group.add(&filter_profile);
    group.add(&backend_profile);
    group.add(&runtime_profile);
    group.add(&helpers_row);
    let dry = adw::SwitchRow::new();
    dry.set_title(&ctx.t_or("detailShared.jobs.dryRun", "Dry run"));
    dry.set_active(
        existing
            .as_ref()
            .is_some_and(|qr| crate::jobs::is_dry_run(&qr.config.rclone)),
    );
    group.add(&dry);
    let resync = adw::SwitchRow::new();
    resync.set_title(&ctx.t_or("dashboard.appDetail.resync", "Resync"));
    resync.set_subtitle(&ctx.t_or(
        "dashboard.appDetail.resyncActive",
        "Force a bisync resync on the next start",
    ));
    resync.set_active(
        existing
            .as_ref()
            .is_some_and(|qr| crate::jobs::is_resync(&qr.config.rclone)),
    );
    resync.set_visible(initial_op == OperationType::Bisync);
    group.add(&resync);
    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&ctx.t_or("flow.quickRun.editor.flags", "Operation flags"));
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let serve_flag_rows: Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let initial_rclone = existing
        .as_ref()
        .map(|qr| qr.config.rclone.clone())
        .unwrap_or(serde_json::json!({}));
    let live_blocks = operation_flag_blocks(&ctx);
    populate_flag_rows(
        &flags_group,
        &flag_rows,
        initial_op,
        &initial_rclone,
        &live_blocks,
    );
    let serve_types = Rc::new(ctx.serve_types());
    let mount_types = Rc::new(ctx.mount_types());
    let serve = adw::ComboRow::new();
    serve.set_title(&ctx.t_or("operation.serve.type", "Serve type"));
    serve.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&serve_types),
    )));
    serve.set_visible(initial_op == OperationType::Serve);
    if let Some(t) = initial_rclone.get("type").and_then(|x| x.as_str()) {
        if let Some(idx) = serve_types.iter().position(|s| s == t) {
            serve.set_selected(idx as u32);
        }
    }
    let mount_type = adw::ComboRow::new();
    mount_type.set_title(&ctx.t_or("remoteConfig.mountType", "Mount type"));
    mount_type.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&mount_types),
    )));
    mount_type.set_visible(initial_op == OperationType::Mount);
    if let Some(t) = initial_rclone.get("mountType").and_then(|x| x.as_str()) {
        if let Some(idx) = mount_types.iter().position(|s| s == t) {
            mount_type.set_selected(idx as u32);
        }
    }
    flags_group.add(&serve);
    flags_group.add(&mount_type);
    if initial_op == OperationType::Serve {
        populate_serve_flag_rows(
            &flags_group,
            &serve_flag_rows,
            &live_blocks,
            &initial_rclone,
            &serve,
            &serve_types,
        );
    }
    attach_cli_import(&ctx, &flags_group, flag_rows.clone());
    {
        let flags_group = flags_group.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let mount_type = mount_type.clone();
        let resync = resync.clone();
        let rclone = initial_rclone.clone();
        let blocks = live_blocks.clone();
        let add_src_row = add_src_row.clone();
        let refresh_guidance = refresh_guidance.clone();
        let src = src.clone();
        let dst = dst.clone();
        let url_filename = url_filename.clone();
        let extra_filenames = extra_filenames.clone();
        let extra_sources = extra_sources.clone();
        let src_kind = src_kind.clone();
        let dst_kind = dst_kind.clone();
        let ctx_titles = ctx.clone();
        let group = group.clone();
        let vfs_profile = vfs_profile.clone();
        op_row.connect_selected_notify(move |row| {
            let op = OperationType::ALL
                .get(row.selected() as usize)
                .copied()
                .unwrap_or(OperationType::Sync);
            serve.set_visible(op == OperationType::Serve);
            mount_type.set_visible(op == OperationType::Mount);
            resync.set_visible(op == OperationType::Bisync);
            add_src_row.set_visible(op.supports_multi_source());
            apply_quick_run_path_titles(&ctx_titles, op, &src, &dst);
            url_filename.set_visible(op == OperationType::Copyurl);
            src_kind.set_visible(op != OperationType::Copyurl);
            dst_kind.set_visible(!matches!(
                op,
                OperationType::Mount | OperationType::Serve | OperationType::Delete
            ));
            if op == OperationType::Copyurl {
                while extra_filenames.borrow().len() < extra_sources.borrow().len() {
                    let name = quick_run_filename_row(&ctx_titles, "");
                    group.add(&name);
                    extra_filenames.borrow_mut().push(name);
                }
            }
            for row in extra_filenames.borrow().iter() {
                row.set_visible(op == OperationType::Copyurl);
            }
            clear_flag_rows(&flags_group, &flag_rows);
            clear_serve_flag_rows(&flags_group, &serve_flag_rows);
            populate_flag_rows(&flags_group, &flag_rows, op, &rclone, &blocks);
            if op == OperationType::Serve {
                populate_serve_flag_rows(
                    &flags_group,
                    &serve_flag_rows,
                    &blocks,
                    &rclone,
                    &serve,
                    &serve_types,
                );
            }
            vfs_profile.set_visible(op.supports_vfs());
            refresh_guidance();
        });
    }
    let vfs_flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let filter_flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let backend_flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let vfs_flags = adw::PreferencesGroup::new();
    vfs_flags.set_title(&ctx.t_or("flow.quickRun.editor.tabVfs", "VFS"));
    let filter_flags = adw::PreferencesGroup::new();
    filter_flags.set_title(&ctx.t_or("flow.quickRun.editor.tabFilter", "Filter"));
    let backend_flags = adw::PreferencesGroup::new();
    backend_flags.set_title(&ctx.t_or("flow.quickRun.editor.tabBackend", "Backend"));
    let runtime_flags = adw::PreferencesGroup::new();
    runtime_flags.set_title(&ctx.t_or("flow.quickRun.editor.tabRuntimeRemote", "Runtime Remote"));
    runtime_flags.add(&runtime_json);
    let flag_stack = adw::ViewStack::new();
    flag_stack.set_vhomogeneous(false);
    flag_stack.add_titled(
        &flags_group,
        Some("operation"),
        &ctx.t_or("flow.quickRun.editor.flags", "Operation flags"),
    );
    let vfs_page = flag_stack.add_titled(
        &vfs_flags,
        Some("vfs"),
        &ctx.t_or("flow.quickRun.editor.tabVfs", "VFS"),
    );
    flag_stack.add_titled(
        &filter_flags,
        Some("filter"),
        &ctx.t_or("flow.quickRun.editor.tabFilter", "Filter"),
    );
    flag_stack.add_titled(
        &backend_flags,
        Some("backend"),
        &ctx.t_or("flow.quickRun.editor.tabBackend", "Backend"),
    );
    flag_stack.add_titled(
        &runtime_flags,
        Some("runtime"),
        &ctx.t_or("flow.quickRun.editor.tabRuntimeRemote", "Runtime Remote"),
    );
    vfs_page.set_visible(initial_op.supports_vfs());
    let flag_switcher = adw::ViewSwitcher::new();
    flag_switcher.set_stack(Some(&flag_stack));
    flag_switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    populate_helper_flag_rows(
        &ctx,
        &vfs_flags,
        &vfs_flag_rows,
        "vfs",
        &live_blocks,
        &remote.text(),
        &helper_selected(&vfs_profile, &vfs_names.borrow()),
    );
    populate_helper_flag_rows(
        &ctx,
        &filter_flags,
        &filter_flag_rows,
        "filter",
        &live_blocks,
        &remote.text(),
        &helper_selected(&filter_profile, &filter_names.borrow()),
    );
    populate_helper_flag_rows(
        &ctx,
        &backend_flags,
        &backend_flag_rows,
        "backend",
        &live_blocks,
        &remote.text(),
        &helper_selected(&backend_profile, &backend_names.borrow()),
    );
    {
        let ctx = ctx.clone();
        let vfs_flags = vfs_flags.clone();
        let vfs_flag_rows = vfs_flag_rows.clone();
        let blocks = live_blocks.clone();
        let remote = remote.clone();
        let vfs_names = vfs_names.clone();
        vfs_profile.connect_selected_notify(move |row| {
            populate_helper_flag_rows(
                &ctx,
                &vfs_flags,
                &vfs_flag_rows,
                "vfs",
                &blocks,
                &remote.text(),
                &helper_selected(row, &vfs_names.borrow()),
            );
        });
    }
    {
        let ctx = ctx.clone();
        let filter_flags = filter_flags.clone();
        let filter_flag_rows = filter_flag_rows.clone();
        let blocks = live_blocks.clone();
        let remote = remote.clone();
        let filter_names = filter_names.clone();
        filter_profile.connect_selected_notify(move |row| {
            populate_helper_flag_rows(
                &ctx,
                &filter_flags,
                &filter_flag_rows,
                "filter",
                &blocks,
                &remote.text(),
                &helper_selected(row, &filter_names.borrow()),
            );
        });
    }
    {
        let ctx = ctx.clone();
        let backend_flags = backend_flags.clone();
        let backend_flag_rows = backend_flag_rows.clone();
        let blocks = live_blocks.clone();
        let remote = remote.clone();
        let backend_names = backend_names.clone();
        backend_profile.connect_selected_notify(move |row| {
            populate_helper_flag_rows(
                &ctx,
                &backend_flags,
                &backend_flag_rows,
                "backend",
                &blocks,
                &remote.text(),
                &helper_selected(row, &backend_names.borrow()),
            );
        });
    }
    {
        let vfs_profile = vfs_profile.clone();
        let vfs_page = vfs_page.clone();
        let flag_stack = flag_stack.clone();
        op_row.connect_selected_notify(move |row| {
            let op = OperationType::ALL
                .get(row.selected() as usize)
                .copied()
                .unwrap_or(OperationType::Sync);
            vfs_profile.set_visible(op.supports_vfs());
            vfs_page.set_visible(op.supports_vfs());
            if !op.supports_vfs() && flag_stack.visible_child_name().as_deref() == Some("vfs") {
                flag_stack.set_visible_child_name("operation");
            }
        });
    }
    let save = gtk::Button::with_label(&ctx.t_or("common.save", "Save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let existing_id = existing.as_ref().map(|q| q.id.clone());
        let vfs_profile = vfs_profile.clone();
        let filter_profile = filter_profile.clone();
        let backend_profile = backend_profile.clone();
        let runtime_profile = runtime_profile.clone();
        let vfs_names = vfs_names.clone();
        let filter_names = filter_names.clone();
        let backend_names = backend_names.clone();
        let runtime_names = runtime_names.clone();
        let vfs_flag_rows = vfs_flag_rows.clone();
        let filter_flag_rows = filter_flag_rows.clone();
        let backend_flag_rows = backend_flag_rows.clone();
        let runtime_json = runtime_json.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let mount_types = mount_types.clone();
        let mount_type = mount_type.clone();
        let dry = dry.clone();
        let resync = resync.clone();
        let description = description.clone();
        let watch_delay = watch_delay.clone();
        let watch_changed = watch_changed.clone();
        let extra_sources = extra_sources.clone();
        let extra_filenames = extra_filenames.clone();
        let url_filename = url_filename.clone();
        save.connect_clicked(move |_| {
            let expr = cron.text().to_string();
            if !expr.is_empty() {
                if let Err(e) = validate_cron(&expr) {
                    let err = adw::AlertDialog::new(
                        Some(&ctx.t_or("flow.quickRun.editor.cron", "Cron schedule")),
                        Some(&e),
                    );
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                    return;
                }
            }
            let op = OperationType::ALL
                .get(op_row.selected() as usize)
                .copied()
                .unwrap_or(OperationType::Sync);
            let mut qr = existing_id
                .as_ref()
                .and_then(|id| {
                    ctx.store
                        .borrow()
                        .quick_runs
                        .iter()
                        .find(|q| &q.id == id)
                        .cloned()
                })
                .unwrap_or_else(|| {
                    QuickRun::new(name.text().to_string(), op, remote.text().to_string())
                });
            qr.name = name.text().to_string();
            qr.description = description.text().to_string();
            qr.remote_name = remote.text().to_string();
            qr.operation_type = op;
            qr.config.app.auto_start = auto.is_active();
            qr.config.app.watch_enabled = watch.is_active();
            qr.config.app.watch_delay = watch_delay.text().parse().unwrap_or(0);
            qr.config.app.watch_changed_only = watch_changed.is_active();
            qr.config.app.cron_enabled = !expr.is_empty();
            qr.config.app.cron_expression = expr;
            qr.config.app.vfs_profile = helper_selected(&vfs_profile, &vfs_names.borrow());
            qr.config.app.filter_profile = helper_selected(&filter_profile, &filter_names.borrow());
            qr.config.app.backend_profile =
                helper_selected(&backend_profile, &backend_names.borrow());
            qr.config.app.runtime_remote_profile =
                helper_selected(&runtime_profile, &runtime_names.borrow());
            save_helper_from_rows(
                &ctx,
                &remote.text(),
                "vfs",
                &qr.config.app.vfs_profile,
                &vfs_flag_rows.borrow(),
            );
            save_helper_from_rows(
                &ctx,
                &remote.text(),
                "filter",
                &qr.config.app.filter_profile,
                &filter_flag_rows.borrow(),
            );
            save_helper_from_rows(
                &ctx,
                &remote.text(),
                "backend",
                &qr.config.app.backend_profile,
                &backend_flag_rows.borrow(),
            );
            qr.show_on_tray = tray.is_active();
            let mut sources = vec![src.text().to_string()];
            for row in extra_sources.borrow().iter() {
                let text = row.text().to_string();
                if !text.is_empty() {
                    sources.push(text);
                }
            }
            sources.retain(|s| !s.is_empty());
            let remote_name = remote.text().to_string();
            if op != OperationType::Copyurl {
                sources = sources
                    .into_iter()
                    .map(|s| crate::path_kind::resolve_job_path(&s, &remote_name))
                    .collect();
            }
            let mut flags = serde_json::Map::new();
            if dry.is_active() {
                flags.insert("dryRun".into(), serde_json::json!(true));
            }
            if op == OperationType::Bisync && resync.is_active() {
                flags.insert("Resync".into(), serde_json::json!(true));
            }
            if op == OperationType::Serve {
                flags.insert(
                    "type".into(),
                    serde_json::json!(crate::operations::selected_or(
                        &serve_types,
                        serve.selected(),
                        "http"
                    )),
                );
            }
            if op == OperationType::Mount {
                flags.insert(
                    "mountType".into(),
                    serde_json::json!(crate::operations::selected_or(
                        &mount_types,
                        mount_type.selected(),
                        "mount"
                    )),
                );
            }
            let inline = runtime_json.text().to_string();
            if !inline.trim().is_empty() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(inline.trim()) {
                    flags.insert("runtimeRemote".into(), value);
                }
            }
            for (field, row, type_name) in flag_rows.borrow().iter() {
                let text = row.text().to_string();
                if text.is_empty() {
                    continue;
                }
                flags.insert(
                    field.clone(),
                    crate::flags::parse_flag_value(type_name, &text),
                );
            }
            let selected_serve =
                crate::operations::selected_or(&serve_types, serve.selected(), "http");
            for (serve_type, field, row, type_name) in serve_flag_rows.borrow().iter() {
                if serve_type != selected_serve {
                    continue;
                }
                let text = row.text().to_string();
                if text.is_empty() {
                    continue;
                }
                flags.insert(
                    field.clone(),
                    crate::flags::parse_flag_value(type_name, &text),
                );
            }
            if op == OperationType::Copyurl {
                let mut names = vec![url_filename.text().to_string()];
                for row in extra_filenames.borrow().iter() {
                    names.push(row.text().to_string());
                }
                if names.iter().any(|s| !s.is_empty()) {
                    flags.insert("filenames".into(), serde_json::json!(names));
                }
            }
            let dest = if matches!(op, OperationType::Serve | OperationType::Delete) {
                dst.text().to_string()
            } else {
                crate::path_kind::resolve_job_path(&dst.text(), &remote_name)
            };
            qr.config.rclone = assemble_rclone(op, &sources, &dest, flags);
            {
                let mut store = ctx.store.borrow_mut();
                if let Some(idx) = store.quick_runs.iter().position(|q| q.id == qr.id) {
                    store.quick_runs[idx] = qr;
                } else {
                    store.quick_runs.push(qr);
                }
            }
            ctx.persist();
            on_done();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.append(&group);
    box_.append(&guidance);
    box_.append(&cron_hint);
    box_.append(&flag_switcher);
    box_.append(&flag_stack);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn export_backup(
    parent: &impl IsA<gtk::Window>,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    remote: Option<&str>,
) {
    if try_spawn_standalone(
        &ctx,
        "export",
        serde_json::json!({ "remote": remote.unwrap_or_default() }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("modals.export.title", "Export backup"));
    dialog.set_content_width(480);
    let categories = backup::export_categories();
    let labels: Vec<String> = categories
        .iter()
        .map(|(id, _)| backup::export_category_label(id, &ctx.i18n.borrow()))
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let type_row = adw::ComboRow::new();
    type_row.set_title(&ctx.t_or("modals.export.selectType", "What to export"));
    type_row.set_model(Some(&gtk::StringList::new(&label_refs)));
    let remotes: Vec<String> = ctx
        .snapshot
        .borrow()
        .remotes
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let remote_row = adw::ComboRow::new();
    remote_row.set_title(&ctx.t_or("modals.export.selectRemote", "Specific remote"));
    let remote_refs: Vec<&str> = remotes.iter().map(|s| s.as_str()).collect();
    if remote_refs.is_empty() {
        remote_row.set_model(Some(&gtk::StringList::new(&["—"])));
        remote_row.set_sensitive(false);
    } else {
        remote_row.set_model(Some(&gtk::StringList::new(&remote_refs)));
    }
    let specific = adw::SwitchRow::new();
    specific.set_title(&ctx.t_or("modals.export.selectRemote", "Export only one remote"));
    if let Some(name) = remote {
        specific.set_active(true);
        if let Some(idx) = remotes.iter().position(|r| r == name) {
            remote_row.set_selected(idx as u32);
        }
    }
    let note = adw::EntryRow::new();
    note.set_title(&ctx.t_or("modals.export.noteLabel", "Note"));
    let password = adw::PasswordEntryRow::new();
    password.set_title(&ctx.t_or(
        "modals.export.passwordLabel",
        "Zip password (optional, 4+ chars)",
    ));
    let secrets = adw::SwitchRow::new();
    secrets.set_title(&ctx.t_or(
        "modals.export.includeSecrets",
        "Include secrets in rclone dump",
    ));
    secrets.set_active(true);
    let extra_backends = ctx.settings.borrow().core.extra_backends.clone();
    let mut backend_labels = vec!["local".to_string()];
    backend_labels.extend(
        extra_backends
            .iter()
            .map(|b| format!("{} ({}:{})", b.name, b.host, b.port)),
    );
    let backend_refs: Vec<&str> = backend_labels.iter().map(|s| s.as_str()).collect();
    let backend_row = adw::ComboRow::new();
    backend_row.set_title(&ctx.t_or("modals.export.backend", "Rclone backend"));
    backend_row.set_subtitle(&ctx.t_or(
        "modals.export.categories.backend.description",
        "Dump remotes from this RC instance",
    ));
    backend_row.set_model(Some(&gtk::StringList::new(&backend_refs)));
    let save = gtk::Button::with_label(&ctx.t_or("modals.export.exportNow", "Choose file…"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let toast = toast.clone();
        let dialog = dialog.clone();
        let remotes = remotes.clone();
        let type_row = type_row.clone();
        let specific = specific.clone();
        let remote_row = remote_row.clone();
        let note = note.clone();
        let password = password.clone();
        let secrets = secrets.clone();
        let backend_row = backend_row.clone();
        let extra_backends = extra_backends.clone();
        save.connect_clicked(move |_| {
            let mut export_type = categories
                .get(type_row.selected() as usize)
                .map(|(id, _)| (*id).to_string())
                .unwrap_or_else(|| "FullBackup".into());
            if specific.is_active() {
                if let Some(name) = remotes.get(remote_row.selected() as usize) {
                    export_type = format!("remote:{name}");
                }
            }
            let note_text = note.text().to_string();
            let zip_pass = password.text().to_string();
            let include_secrets = secrets.is_active();
            let file_dialog = gtk::FileDialog::new();
            file_dialog.set_initial_name(Some("rclone-manager-backup.zip"));
            let ctx = ctx.clone();
            let toast = toast.clone();
            let backend_row = backend_row.clone();
            let extra_backends = extra_backends.clone();
            file_dialog.save(
                Some(&parent),
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let selected = backend_row.selected() as usize;
                            let dump_client = if selected == 0 {
                                ctx.client()
                            } else {
                                extra_backends.get(selected.saturating_sub(1)).map(|entry| {
                                    let user = if entry.user.is_empty() {
                                        None
                                    } else {
                                        Some(entry.user.clone())
                                    };
                                    let pass = if entry.pass.is_empty() {
                                        None
                                    } else {
                                        Some(entry.pass.clone())
                                    };
                                    crate::rclone::RcClient::new(&entry.host, entry.port)
                                        .with_auth(user, pass)
                                })
                            };
                            let mut dump = dump_client
                                .and_then(|c| c.dump_config().ok())
                                .unwrap_or(serde_json::json!({}));
                            if !include_secrets {
                                if let Some(obj) = dump.as_object_mut() {
                                    for cfg in obj.values_mut() {
                                        if let Some(map) = cfg.as_object_mut() {
                                            for key in [
                                                "token",
                                                "secret",
                                                "password",
                                                "pass",
                                                "client_secret",
                                                "key",
                                            ] {
                                                map.remove(key);
                                            }
                                        }
                                    }
                                }
                            }
                            let pw = if zip_pass.trim().len() >= 4 {
                                Some(zip_pass.as_str())
                            } else {
                                None
                            };
                            match backup::create_backup(
                                &path,
                                &ctx.settings.borrow(),
                                &ctx.store.borrow(),
                                &dump,
                                &export_type,
                                &note_text,
                                pw,
                            ) {
                                Ok(_) => {
                                    toast.add_toast(adw::Toast::new(&ctx.t_or(
                                        "backup.backupSuccess",
                                        "Backup created successfully",
                                    )))
                                }
                                Err(e) => toast.add_toast(adw::Toast::new(&e)),
                            }
                        }
                    }
                },
            );
            dialog.close();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&type_row);
    group.add(&specific);
    group.add(&remote_row);
    group.add(&note);
    group.add(&password);
    group.add(&secrets);
    group.add(&backend_row);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent.upcast_ref(), &ctx, &dialog);
}

pub fn import_backup(
    parent: &(impl IsA<gtk::Window> + Clone),
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = gtk::FileDialog::new();
    let parent_widget = parent.clone();
    dialog.open(
        Some(parent),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    restore_preview(&parent_widget, ctx.clone(), toast.clone(), path, on_done);
                }
            }
        },
    );
}

pub fn restore_preview(
    parent: &(impl IsA<gtk::Window> + Clone),
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    path: std::path::PathBuf,
    on_done: Rc<dyn Fn()>,
) {
    if try_spawn_standalone(
        &ctx,
        "restore-preview",
        serde_json::json!({ "path": path.to_string_lossy() }),
    ) {
        return;
    }
    let analysis = backup::analyze_backup(&path).ok();
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("backup.restore.title", "Restore Backup"));
    dialog.set_content_width(520);
    dialog.set_content_height(640);
    let page = adw::PreferencesPage::new();
    let info = adw::PreferencesGroup::new();
    info.set_title(&ctx.t_or("backup.restore.info", "Backup Information"));
    if let Some(analysis) = &analysis {
        let created = adw::ActionRow::new();
        created.set_title(&ctx.t_or("backup.restore.created", "Created"));
        let created_text = if analysis.manifest.created_at.is_empty() {
            ctx.t_or("backup.restore.unknown", "Unknown")
        } else {
            analysis.manifest.created_at.clone()
        };
        created.set_subtitle(&created_text);
        let kind = adw::ActionRow::new();
        kind.set_title(&ctx.t_or("backup.restore.type", "Type"));
        kind.set_subtitle(&analysis.manifest.export_type);
        let security = adw::ActionRow::new();
        security.set_title(&ctx.t_or("backup.restore.security", "Security"));
        security.set_subtitle(&if analysis.manifest.encrypted {
            ctx.t_or("backup.restore.encrypted", "Encrypted")
        } else {
            ctx.t_or("backup.restore.notEncrypted", "Not Encrypted")
        });
        let remotes = adw::ActionRow::new();
        remotes.set_title(&ctx.t_or(
            "backup.restore.remoteConfigs.title",
            "Remote Configurations",
        ));
        remotes.set_subtitle(&if analysis.manifest.remotes.is_empty() {
            ctx.t_or("backup.restore.unknown", "Unknown")
        } else {
            analysis.manifest.remotes.join(", ")
        });
        info.add(&created);
        info.add(&kind);
        info.add(&security);
        info.add(&remotes);
    } else {
        let missing = adw::ActionRow::new();
        missing.set_title(&ctx.t_or("backup.analyzeFailed", "Failed to analyze backup file"));
        missing.set_subtitle(&ctx.t_or(
            "backup.restore.errors.requiresPassword",
            "This backup requires a password to decrypt.",
        ));
        info.add(&missing);
    }
    page.add(&info);
    if let Some(analysis) = &analysis {
        let contents = adw::PreferencesGroup::new();
        contents.set_title(&ctx.t_or("backup.restore.contents", "Backup Contents"));
        for (key, fallback) in analysis.content_rows() {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or(key, fallback));
            row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
            contents.add(&row);
        }
        if !analysis.manifest.note.is_empty() {
            let note = adw::ActionRow::new();
            note.set_title(&ctx.t_or("backup.restore.note", "Backup Note"));
            note.set_subtitle(&analysis.manifest.note);
            contents.add(&note);
        }
        page.add(&contents);
    }
    let options = adw::PreferencesGroup::new();
    options.set_title(&ctx.t_or("backup.restore.options", "Restore Options"));
    let password = adw::PasswordEntryRow::new();
    password.set_title(&ctx.t_or(
        "backup.restore.passwordPlaceholder",
        "Enter your backup password",
    ));
    let mut scope_labels = vec![ctx.t_or("backup.restore.scope.all", "Restore Everything")];
    if let Some(analysis) = &analysis {
        scope_labels.extend(analysis.manifest.remotes.iter().cloned());
    }
    let scope_refs: Vec<&str> = scope_labels.iter().map(|s| s.as_str()).collect();
    let scope = adw::ComboRow::new();
    scope.set_title(&ctx.t_or("backup.restore.scope.profile", "Restore Specific Profile"));
    scope.set_model(Some(&gtk::StringList::new(&scope_refs)));
    let as_name = adw::EntryRow::new();
    as_name.set_title(&ctx.t_or(
        "backup.restore.selectProfile",
        "Restore as (optional rename)",
    ));
    options.add(&password);
    options.add(&scope);
    options.add(&as_name);
    page.add(&options);
    let restore =
        gtk::Button::with_label(&ctx.t_or("backup.restore.restoreAction", "Restore Backup"));
    restore.add_css_class("destructive-action");
    let cancel = gtk::Button::with_label(&ctx.t("common.cancel"));
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(8);
    buttons.set_margin_end(12);
    buttons.set_margin_bottom(12);
    buttons.append(&cancel);
    buttons.append(&restore);
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let dialog = dialog.clone();
        let path = path.clone();
        let on_done = on_done.clone();
        let scope_labels = scope_labels.clone();
        restore.connect_clicked(move |_| {
            let pw = password.text().to_string();
            let pw = if pw.is_empty() {
                None
            } else {
                Some(pw.as_str())
            };
            let selected = scope.selected() as usize;
            let profile = if selected == 0 {
                None
            } else {
                scope_labels.get(selected).map(|s| s.as_str())
            };
            let restore_as = as_name.text().to_string();
            let restore_as = if restore_as.trim().is_empty() {
                None
            } else {
                Some(restore_as.as_str())
            };
            if analysis.is_none() && pw.is_some() {
                if backup::analyze_backup_with_password(&path, pw).is_err() {
                    toast.add_toast(adw::Toast::new(&ctx.t_or(
                        "backup.restore.errors.wrongPassword",
                        "Incorrect password. Please try again.",
                    )));
                    return;
                }
            }
            match backup::restore_backup_scoped(&path, pw, profile, restore_as) {
                Ok((settings, store, rclone)) => {
                    if let Some(settings) = settings {
                        *ctx.settings.borrow_mut() = settings;
                    }
                    if let Some(store) = store {
                        *ctx.store.borrow_mut() = store;
                    }
                    if let (Some(dump), Some(client)) = (rclone, ctx.client()) {
                        if let Some(obj) = dump.as_object() {
                            for (name, cfg) in obj {
                                if let Some(t) = cfg.get("type").and_then(|x| x.as_str()) {
                                    let _ = client.create_remote(name, t, cfg.clone(), None);
                                }
                            }
                        }
                    }
                    ctx.persist();
                    ctx.restart_engine();
                    ctx.reload_automations();
                    ctx.refresh_runtime();
                    toast.add_toast(adw::Toast::new(
                        &ctx.t_or("backup.restoreSuccess", "Backup restored successfully"),
                    ));
                    dialog.close();
                    on_done();
                }
                Err(e) => toast.add_toast(adw::Toast::new(
                    &ctx.tf("backup.restoreFailed", &[("error", &e)]),
                )),
            }
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    box_.append(&scroll);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    let parent = parent.clone().upcast::<gtk::Window>();
    dialog.present(Some(&parent));
}

pub fn properties(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
) {
    if try_spawn_standalone(
        &ctx,
        "properties",
        serde_json::json!({ "remote": remote, "path": path, "name": name }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("fileBrowser.properties.title", "Properties"));
    dialog.set_content_width(520);
    dialog.set_content_height(640);
    let location = if remote == "local" {
        path.to_string()
    } else if path.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{path}")
    };
    let starred =
        crate::settings::collection_contains(&ctx.settings.borrow().nautilus.starred, &location);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let heading = gtk::Label::new(Some(name));
    heading.add_css_class("title-3");
    heading.set_hexpand(true);
    heading.set_xalign(0.0);
    let star = gtk::Button::from_icon_name(if starred {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    star.set_tooltip_text(Some(&if starred {
        ctx.t_or("fileBrowser.properties.unstar", "Unstar")
    } else {
        ctx.t_or("fileBrowser.properties.star", "Star")
    }));
    {
        let ctx = ctx.clone();
        let location = location.clone();
        let name = name.to_string();
        star.connect_clicked(move |btn| {
            let now = {
                let mut settings = ctx.settings.borrow_mut();
                crate::settings::toggle_collection(&mut settings.nautilus.starred, &location, &name)
            };
            ctx.persist();
            btn.set_icon_name(if now {
                "starred-symbolic"
            } else {
                "non-starred-symbolic"
            });
            btn.set_tooltip_text(Some(&if now {
                ctx.t_or("fileBrowser.properties.unstar", "Unstar")
            } else {
                ctx.t_or("fileBrowser.properties.star", "Star")
            }));
        });
    }
    header.append(&heading);
    header.append(&star);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (title, value) in [
        (ctx.t_or("common.name", "Name"), name.to_string()),
        (ctx.t_or("sidebar.remotes", "Remote"), remote.to_string()),
        (
            ctx.t_or("fileBrowser.properties.location", "Location"),
            path.to_string(),
        ),
        (ctx.t_or("modals.jobDetail.fields.type", "Type"), {
            let cat = crate::operations::FileTypeCategory::from_name(name, false);
            ctx.t_or(
                &format!("fileBrowser.fileTypes.{cat:?}"),
                &format!("{cat:?}"),
            )
        }),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&value);
        list.append(&row);
    }
    let fs = if remote == "local" {
        "/".into()
    } else {
        remote_fs(remote, "")
    };
    let info = ctx.fs_info(remote);
    let mut is_dir = path.is_empty() || path.ends_with('/') || name.ends_with('/');
    if let Some(client) = ctx.client() {
        if let Ok(Some(stat)) = client.stat(&fs, path) {
            is_dir = stat.is_dir;
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("fileBrowser.properties.kind", "Kind"));
            row.set_subtitle(&format!(
                "{}{}",
                if stat.mime.is_empty() {
                    if stat.is_dir {
                        ctx.t_or("fileBrowser.properties.directory", "Directory")
                    } else {
                        ctx.t_or("fileBrowser.properties.file", "File")
                    }
                } else {
                    stat.mime.clone()
                },
                if stat.is_dir {
                    String::new()
                } else {
                    format!(" · {}", crate::rclone::format_bytes(stat.size))
                }
            ));
            list.append(&row);
            if !stat.mod_time.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.properties.modified", "Modified"));
                row.set_subtitle(&crate::rclone::format_mod_time(&stat.mod_time));
                list.append(&row);
            }
        }
        if let Ok(about) = client.about(&fs) {
            for (key, label, fallback) in [
                ("used", "fileBrowser.properties.used", "Used"),
                ("free", "fileBrowser.properties.free", "Free"),
                ("total", "fileBrowser.properties.totalCapacity", "Total"),
            ] {
                if let Some(value) = about.get(key).and_then(|x| x.as_i64()) {
                    let row = adw::ActionRow::new();
                    row.set_title(&ctx.t_or(label, fallback));
                    row.set_subtitle(&crate::rclone::format_bytes(value));
                    list.append(&row);
                }
            }
        }
        if remote == "local" {
            if let Ok(du) = client.du(Some(path)) {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.properties.localDisk", "Local disk"));
                row.set_subtitle(&du.dir);
                list.append(&row);
                for (label, fallback, value) in [
                    ("fileBrowser.properties.used", "Used", du.used),
                    ("fileBrowser.properties.free", "Free", du.free),
                    ("fileBrowser.properties.totalCapacity", "Total", du.total),
                ] {
                    let row = adw::ActionRow::new();
                    row.set_title(&ctx.t_or(label, fallback));
                    row.set_subtitle(&crate::rclone::format_bytes(value));
                    list.append(&row);
                }
            }
        }
        if let Ok(size) = client.size(&fs, path) {
            let (count, bytes) = crate::rclone::parse_object_size(&size);
            let count_row = adw::ActionRow::new();
            count_row
                .set_title(&ctx.t_or("fileBrowser.properties.containedFiles", "Contained files"));
            count_row.set_subtitle(&count.to_string());
            list.append(&count_row);
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("fileBrowser.properties.totalSize", "Total size"));
            row.set_subtitle(&crate::rclone::format_bytes(bytes));
            list.append(&row);
        }
        let hashes = info
            .as_ref()
            .map(|i| i.hashes.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or_default();
        if hashes.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("fileBrowser.properties.checksums", "Hashes"));
            row.set_subtitle(&ctx.t_or(
                "fileBrowser.properties.noHashTypes",
                "This remote does not advertise hash support",
            ));
            list.append(&row);
        } else if is_dir {
            let instr = adw::ActionRow::new();
            instr.set_title(&ctx.t_or("fileBrowser.properties.checksums", "Checksums"));
            instr.set_subtitle(&ctx.t_or(
                "fileBrowser.properties.bulkHashInstruction",
                "Select a hash type to calculate checksums for all files in this directory.",
            ));
            list.append(&instr);
            let result_view = gtk::TextView::new();
            result_view.set_editable(false);
            result_view.set_monospace(true);
            result_view.set_wrap_mode(gtk::WrapMode::Char);
            result_view.set_left_margin(12);
            result_view.set_right_margin(12);
            result_view.set_top_margin(8);
            result_view.set_bottom_margin(8);
            let result_scroll = gtk::ScrolledWindow::new();
            result_scroll.set_min_content_height(160);
            result_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
            result_scroll.set_child(Some(&result_view));
            let result_row = gtk::ListBoxRow::new();
            result_row.set_activatable(false);
            result_row.set_child(Some(&result_scroll));
            result_row.set_visible(false);
            list.append(&result_row);
            let result_actions = adw::ActionRow::new();
            result_actions.set_title(&ctx.t_or(
                "fileBrowser.properties.generatedChecksums",
                "Generated Checksums",
            ));
            result_actions.set_visible(false);
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.set_valign(gtk::Align::Center);
            copy.set_tooltip_text(Some(&ctx.t_or(
                "fileBrowser.properties.copyToClipboard",
                "Copy to clipboard",
            )));
            {
                let result_view = result_view.clone();
                copy.connect_clicked(move |_| {
                    let buffer = result_view.buffer();
                    let start = buffer.start_iter();
                    let end = buffer.end_iter();
                    let text = buffer.text(&start, &end, false);
                    if !text.is_empty() {
                        if let Some(display) = gtk::gdk::Display::default() {
                            display.clipboard().set_text(&text);
                        }
                    }
                });
            }
            let another = gtk::Button::with_label(&ctx.t_or(
                "fileBrowser.properties.calculateAnother",
                "Calculate Another",
            ));
            another.set_valign(gtk::Align::Center);
            {
                let result_view = result_view.clone();
                let result_row = result_row.clone();
                let result_actions = result_actions.clone();
                another.connect_clicked(move |_| {
                    result_view.buffer().set_text("");
                    result_row.set_visible(false);
                    result_actions.set_visible(false);
                });
            }
            result_actions.add_suffix(&copy);
            result_actions.add_suffix(&another);
            list.append(&result_actions);
            for hash_type in hashes.iter() {
                let row = adw::ActionRow::new();
                row.set_title(&hash_type.to_ascii_uppercase());
                row.set_subtitle(&ctx.t_or("fileBrowser.properties.calculating", "Not calculated"));
                let calc = gtk::Button::with_label(
                    &ctx.t_or("fileBrowser.properties.calculateMore", "Calculate"),
                );
                calc.set_valign(gtk::Align::Center);
                {
                    let row = row.clone();
                    let ctx = ctx.clone();
                    let fs = fs.clone();
                    let path = path.to_string();
                    let hash_type = hash_type.clone();
                    let result_view = result_view.clone();
                    let result_row = result_row.clone();
                    let result_actions = result_actions.clone();
                    calc.connect_clicked(move |_| {
                        if let Some(client) = ctx.client() {
                            match client.hashsum(&fs, &path, &hash_type) {
                                Ok(value) => match parse_hashsum_list(&value) {
                                    Some(text) => {
                                        result_view.buffer().set_text(&text);
                                        result_row.set_visible(true);
                                        result_actions.set_visible(true);
                                        result_actions
                                            .set_subtitle(&hash_type.to_ascii_uppercase());
                                        row.set_subtitle(&ctx.t_or(
                                            "fileBrowser.properties.generatedChecksums",
                                            "Generated Checksums",
                                        ));
                                    }
                                    None => row.set_subtitle(&ctx.t_or(
                                        "fileBrowser.properties.noHashesFound",
                                        "No hashes returned",
                                    )),
                                },
                                Err(e) => row.set_subtitle(&e.to_string()),
                            }
                        }
                    });
                }
                row.add_suffix(&calc);
                list.append(&row);
            }
        } else {
            for (idx, hash_type) in hashes.iter().enumerate() {
                let row = adw::ActionRow::new();
                row.set_title(&hash_type.to_ascii_uppercase());
                row.set_subtitle(&ctx.t_or("fileBrowser.properties.calculating", "Not calculated"));
                let calc = gtk::Button::with_label(
                    &ctx.t_or("fileBrowser.properties.calculateMore", "Calculate"),
                );
                calc.set_valign(gtk::Align::Center);
                {
                    let row = row.clone();
                    let calc_btn = calc.clone();
                    let ctx = ctx.clone();
                    let fs = fs.clone();
                    let path = path.to_string();
                    let hash_type = hash_type.clone();
                    calc.connect_clicked(move |_| {
                        if let Some(client) = ctx.client() {
                            let value = client
                                .hashsum_file(&fs, &path, &hash_type)
                                .or_else(|_| client.hashsum(&fs, &path, &hash_type));
                            match value {
                                Ok(value) => {
                                    row.set_subtitle(
                                        &parse_hashsum(&value).unwrap_or_else(|| value.to_string()),
                                    );
                                    calc_btn.set_sensitive(false);
                                }
                                Err(e) => row.set_subtitle(&e.to_string()),
                            }
                        }
                    });
                    if idx == 0 {
                        calc.emit_clicked();
                    }
                }
                let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
                copy.set_valign(gtk::Align::Center);
                copy.set_tooltip_text(Some(&ctx.t_or(
                    "fileBrowser.properties.copyToClipboard",
                    "Copy to clipboard",
                )));
                {
                    let row = row.clone();
                    copy.connect_clicked(move |_| {
                        if let Some(text) = row.subtitle() {
                            if let Some(display) = gtk::gdk::Display::default() {
                                display.clipboard().set_text(&text);
                            }
                        }
                    });
                }
                row.add_suffix(&calc);
                row.add_suffix(&copy);
                list.append(&row);
            }
        }
        if remote != "local" && info.as_ref().is_none_or(|i| i.has_feature("PublicLink")) {
            let link_row = adw::ActionRow::new();
            link_row.set_title(&ctx.t_or("fileBrowser.properties.publicLink", "Public link"));
            link_row.set_subtitle(&ctx.t_or("fileBrowser.properties.creatingLink", "Not created"));
            list.append(&link_row);
            let exp_never = ctx.t_or("fileBrowser.properties.expiry.never", "Never");
            let exp_1h = ctx.t_or("fileBrowser.properties.expiry.1h", "1 Hour");
            let exp_1d = ctx.t_or("fileBrowser.properties.expiry.1d", "1 Day");
            let exp_7d = ctx.t_or("fileBrowser.properties.expiry.7d", "7 Days");
            let exp_30d = ctx.t_or("fileBrowser.properties.expiry.30d", "30 Days");
            let expire = adw::ComboRow::new();
            expire.set_title(&ctx.t_or("fileBrowser.properties.expires", "Expires"));
            expire.set_model(Some(&gtk::StringList::new(&[
                exp_never.as_str(),
                exp_1h.as_str(),
                exp_1d.as_str(),
                exp_7d.as_str(),
                exp_30d.as_str(),
            ])));
            expire.set_selected(0);
            list.append(&expire);
            let get_link =
                gtk::Button::with_label(&ctx.t_or("fileBrowser.properties.getLink", "Get Link"));
            let unlink = gtk::Button::with_label(
                &ctx.t_or("fileBrowser.properties.removeLink", "Remove Link"),
            );
            {
                let ctx = ctx.clone();
                let fs = fs.clone();
                let path = path.to_string();
                let link_row = link_row.clone();
                let expire = expire.clone();
                get_link.connect_clicked(move |_| {
                    if let Some(client) = ctx.client() {
                        let exp = public_link_expiry_value(expire.selected()).unwrap_or("");
                        match client.public_link_ex(
                            &fs,
                            &path,
                            (!exp.is_empty()).then_some(exp),
                            false,
                        ) {
                            Ok(url) if !url.is_empty() => {
                                link_row.set_subtitle(&url);
                                if let Some(display) = gtk::gdk::Display::default() {
                                    display.clipboard().set_text(&url);
                                }
                            }
                            Ok(_) => link_row.set_subtitle(&ctx.t_or(
                                "fileBrowser.properties.noPublicLink",
                                "Remote did not return a public link",
                            )),
                            Err(e) => link_row.set_subtitle(&e.to_string()),
                        }
                    }
                });
            }
            {
                let ctx = ctx.clone();
                let fs = fs.clone();
                let path = path.to_string();
                let link_row = link_row.clone();
                unlink.connect_clicked(move |_| {
                    if let Some(client) = ctx.client() {
                        match client.public_link_ex(&fs, &path, None, true) {
                            Ok(_) => link_row.set_subtitle(
                                &ctx.t_or("fileBrowser.properties.linkRemoved", "Link removed"),
                            ),
                            Err(e) => link_row.set_subtitle(&e.to_string()),
                        }
                    }
                });
            }
            let link_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            link_actions.append(&get_link);
            link_actions.append(&unlink);
            list.append(&{
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.properties.publicLink", "Link actions"));
                row.add_suffix(&link_actions);
                row
            });
        }
    }
    let copy_path =
        gtk::Button::with_label(&ctx.t_or("nautilus.contextMenu.copyPath", "Copy path"));
    {
        let text = if remote == "local" {
            path.to_string()
        } else {
            format!("{remote}:{path}")
        };
        copy_path.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
            }
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&header);
    box_.append(&list);
    box_.append(&copy_path);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&box_));
    dialog.set_child(Some(&scroll));
    dialog.present(Some(parent));
}

pub fn job_detail(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, job_id: u64) {
    if try_spawn_standalone(&ctx, "job-detail", serde_json::json!({ "jobid": job_id })) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.tf("modals.jobDetail.title", &[("id", &job_id.to_string())]));
    dialog.set_content_width(640);
    dialog.set_content_height(620);
    let meta = gtk::ListBox::new();
    meta.add_css_class("boxed-list");
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    let transfers = gtk::ListBox::new();
    transfers.add_css_class("boxed-list");
    let completed = gtk::ListBox::new();
    completed.add_css_class("boxed-list");
    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some(
        &ctx.t_or("modals.jobDetail.filterTransfers", "Filter transfers"),
    ));
    let stop = gtk::Button::with_label(&ctx.t_or("flow.quickRun.actions.stop", "Stop job"));
    stop.add_css_class("destructive-action");
    {
        let ctx = ctx.clone();
        stop.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.job_stop(job_id);
                ctx.refresh_runtime();
            }
        });
    }
    let stop_group = gtk::Button::with_label(&ctx.t_or("modals.jobDetail.stopGroup", "Stop group"));
    {
        let ctx = ctx.clone();
        stop_group.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let group = ctx
                    .snapshot
                    .borrow()
                    .jobs
                    .iter()
                    .find(|j| j.id == job_id)
                    .map(|j| j.group.clone())
                    .unwrap_or_else(|| format!("job/{job_id}"));
                let _ = client.job_stop_group(&group);
                ctx.refresh_runtime();
            }
        });
    }
    let reset = gtk::Button::with_label(&ctx.t_or("modals.jobDetail.resetStats", "Reset stats"));
    {
        let ctx = ctx.clone();
        reset.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.reset_stats(Some(&format!("job/{job_id}")));
            }
        });
    }
    let delete = gtk::Button::with_label(
        &ctx.t_or("fileBrowser.operations.removeJob", "Delete from history"),
    );
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        delete.connect_clicked(move |_| {
            ctx.store.borrow_mut().dismiss_job(job_id);
            ctx.persist();
            ctx.refresh_runtime();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&progress);
    box_.append(&scrolled_list(&meta));
    box_.append(&filter);
    let xfer_label = gtk::Label::new(Some(
        &ctx.t_or("generalOverview.jobs.transfers", "Active transfers"),
    ));
    xfer_label.add_css_class("heading");
    xfer_label.set_xalign(0.0);
    box_.append(&xfer_label);
    box_.append(&scrolled_list(&transfers));
    let done_label = gtk::Label::new(Some(
        &ctx.t_or("fileBrowser.operations.completed", "Completed transfers"),
    ));
    done_label.add_css_class("heading");
    done_label.set_xalign(0.0);
    box_.append(&done_label);
    box_.append(&scrolled_list(&completed));
    let open_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let populate_opens: Rc<dyn Fn(&str, &str)> = Rc::new({
        let open_box = open_box.clone();
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        move |src: &str, dst: &str| {
            while let Some(child) = open_box.first_child() {
                open_box.remove(&child);
            }
            append_open_job_paths(
                &open_box,
                &ctx,
                &dialog,
                &ctx.t_or("modals.jobDetail.openSource", "Open source in Files"),
                src,
            );
            append_open_job_paths(
                &open_box,
                &ctx,
                &dialog,
                &ctx.t_or(
                    "modals.jobDetail.openDestination",
                    "Open destination in Files",
                ),
                dst,
            );
        }
    });
    {
        let (src, dst) = ctx
            .snapshot
            .borrow()
            .jobs
            .iter()
            .find(|j| j.id == job_id)
            .map(|j| (j.src.clone(), j.dst.clone()))
            .or_else(|| {
                ctx.store
                    .borrow()
                    .job_history
                    .iter()
                    .find(|j| j.id == job_id)
                    .map(|j| (j.src.clone(), j.dst.clone()))
            })
            .unwrap_or_default();
        populate_opens(&src, &dst);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&stop);
    actions.append(&stop_group);
    actions.append(&reset);
    actions.append(&delete);
    actions.append(&open_box);
    box_.append(&actions);
    dialog.set_child(Some(&box_));

    let fill = {
        let ctx = ctx.clone();
        let meta = meta.clone();
        let transfers = transfers.clone();
        let completed = completed.clone();
        let progress = progress.clone();
        let filter = filter.clone();
        let dialog = dialog.clone();
        let populate_opens = populate_opens.clone();
        move || {
            while let Some(child) = meta.first_child() {
                meta.remove(&child);
            }
            while let Some(child) = transfers.first_child() {
                transfers.remove(&child);
            }
            while let Some(child) = completed.first_child() {
                completed.remove(&child);
            }
            let Some(client) = ctx.client() else {
                return;
            };
            let Ok(status) = client.job_status(job_id) else {
                return;
            };
            let group = status
                .get("group")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("job/{job_id}"));
            let stats = client.stats(Some(&group)).ok();
            let job = job_from_status(job_id, &status, stats.as_ref());
            populate_opens(&job.src, &job.dst);
            progress.set_fraction(job.progress);
            progress.set_text(Some(&format!(
                "{}{} · {:.0}% · {:.1}s",
                job.status,
                if job.dry_run {
                    format!(" · {}", ctx.t_or("detailShared.jobs.dryRun", "Dry Run"))
                } else {
                    String::new()
                },
                job.progress * 100.0,
                job.duration
            )));
            let speed = crate::jobs::stats_f64(&job.stats, &["speed"]);
            let speed_avg = crate::jobs::stats_f64(&job.stats, &["speedAvg", "speedAverage"]);
            let bytes = crate::jobs::stats_i64(&job.stats, &["bytes"]);
            let total_bytes = crate::jobs::stats_i64(&job.stats, &["totalBytes", "total_bytes"]);
            let eta = job
                .stats
                .get("eta")
                .cloned()
                .unwrap_or(serde_json::json!("—"));
            let end_time = crate::jobs::parse_job_end_time(&status)
                .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "—".into());
            let execute_id = ctx
                .store
                .borrow()
                .job_meta
                .get(&job_id)
                .map(|m| m.execute_id.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            let yes = ctx.t_or("common.yes", "Yes");
            let no = ctx.t_or("common.no", "No");
            let transfer_count = crate::jobs::stats_i64(&job.stats, &["transfers"]);
            let check_count = crate::jobs::stats_i64(&job.stats, &["checks"]);
            let delete_count = crate::jobs::stats_i64(&job.stats, &["deletes", "deleted"]);
            let rename_count = crate::jobs::stats_i64(&job.stats, &["renames"]);
            let listed_count = crate::jobs::stats_i64(&job.stats, &["listed"]);
            let ss_copies = crate::jobs::stats_i64(&job.stats, &["serverSideCopies"]);
            let ss_copy_bytes = crate::jobs::stats_i64(&job.stats, &["serverSideCopyBytes"]);
            let ss_moves = crate::jobs::stats_i64(&job.stats, &["serverSideMoves"]);
            let ss_move_bytes = crate::jobs::stats_i64(&job.stats, &["serverSideMoveBytes"]);
            let error_count = crate::jobs::stats_i64(&job.stats, &["errors"]);
            let fatal = crate::jobs::stats_bool(&job.stats, &["fatalError", "fatal_error"]);
            for (title, value) in [
                (
                    ctx.t_or("modals.jobDetail.fields.type", "Operation"),
                    job.operation.clone(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.status", "Status"),
                    job.status.clone(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.started", "Started"),
                    job.start_time.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.finished", "Finished"),
                    end_time,
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.duration", "Duration"),
                    format!("{:.1}s", job.duration),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.executeId", "Execute ID"),
                    execute_id,
                ),
                (ctx.t_or("sidebar.remotes", "Remote"), job.remote.clone()),
                (
                    ctx.t_or("modals.jobDetail.fields.profile", "Profile"),
                    job.profile.clone(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.origin", "Origin"),
                    job.origin.clone(),
                ),
                (ctx.t_or("modals.jobDetail.fields.backend", "Backend"), {
                    ctx.store
                        .borrow()
                        .job_meta
                        .get(&job.id)
                        .map(|m| m.backend.clone())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| {
                            let backend = ctx.settings.borrow().core.active_backend.clone();
                            if backend.is_empty() {
                                "local".into()
                            } else {
                                backend
                            }
                        })
                }),
                (
                    ctx.t_or("fileBrowser.operations.details.source", "Source"),
                    job.src.clone(),
                ),
                (
                    ctx.t_or("fileBrowser.operations.details.destination", "Destination"),
                    job.dst.clone(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.group", "Group"),
                    job.group.clone(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.transferred", "Transferred"),
                    format!(
                        "{} / {}",
                        crate::rclone::format_bytes(bytes),
                        crate::rclone::format_bytes(total_bytes)
                    ),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.speed", "Speed"),
                    format!("{:.1} KiB/s", speed / 1024.0),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.speedAvg", "Average speed"),
                    format!("{:.1} KiB/s", speed_avg / 1024.0),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.eta", "ETA"),
                    eta.to_string(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.files", "Files"),
                    format!(
                        "{} · {} · {} · {} · {}",
                        transfer_count, check_count, delete_count, rename_count, listed_count
                    ),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.checks", "Checks"),
                    check_count.to_string(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.deleted", "Deleted"),
                    delete_count.to_string(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.renamed", "Renamed"),
                    rename_count.to_string(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.listed", "Listed"),
                    listed_count.to_string(),
                ),
                (
                    ctx.t_or(
                        "modals.jobDetail.fields.serverSideCopies",
                        "Server-side copies",
                    ),
                    format!(
                        "{} ({})",
                        ss_copies,
                        crate::rclone::format_bytes(ss_copy_bytes)
                    ),
                ),
                (
                    ctx.t_or(
                        "modals.jobDetail.fields.serverSideMoves",
                        "Server-side moves",
                    ),
                    format!(
                        "{} ({})",
                        ss_moves,
                        crate::rclone::format_bytes(ss_move_bytes)
                    ),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.errorsCount", "Errors"),
                    error_count.to_string(),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.fatalError", "Fatal error"),
                    if fatal { yes.clone() } else { no.clone() },
                ),
                (
                    ctx.t_or("modals.jobDetail.sections.errors", "Error"),
                    job.error
                        .as_deref()
                        .map(|e| ctx.translate_error(e))
                        .unwrap_or_else(|| "—".into()),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.retryError", "Retry error"),
                    if job
                        .stats
                        .get("retryError")
                        .or_else(|| job.stats.get("retry_error"))
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false)
                    {
                        ctx.t_or("common.yes", "Yes")
                    } else {
                        ctx.t_or("common.no", "No")
                    },
                ),
            ] {
                let row = adw::ActionRow::new();
                row.set_title(&title);
                row.set_subtitle(&value);
                meta.append(&row);
            }
            let query = filter.text().to_lowercase();
            append_transfer_rows(
                &transfers,
                job.transferring.as_array(),
                &query,
                true,
                &ctx,
                &dialog,
                &job.operation,
            );
            append_transfer_rows(
                &completed,
                job.completed.as_array(),
                &query,
                false,
                &ctx,
                &dialog,
                &job.operation,
            );
            let check_source = crate::checks::check_source_from_job(&job.stats, &job.output);
            let results = crate::checks::parse_check_items(&check_source, &job.src, &job.dst);
            if !results.is_empty() {
                let heading = adw::ActionRow::new();
                heading.set_title(&format!("{} check results", results.len()));
                heading.set_subtitle(
                    "Resolve copies the missing/changed file; delete uses rclone deletefile",
                );
                completed.append(&heading);
                for item in results.into_iter().take(40) {
                    completed.append(&check_result_row(&ctx, &item, &dialog));
                }
            }
        }
    };
    fill();
    {
        let fill = fill.clone();
        filter.connect_search_changed(move |_| fill());
    }
    let alive = Rc::new(Cell::new(true));
    {
        let alive = alive.clone();
        let fill = fill.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            if !alive.get() {
                return glib::ControlFlow::Break;
            }
            fill();
            glib::ControlFlow::Continue
        });
    }
    dialog.connect_closed(move |_| {
        alive.set(false);
    });
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn append_open_job_paths(
    box_: &gtk::Box,
    ctx: &AppCtx,
    dialog: &adw::Dialog,
    label: &str,
    raw: &str,
) {
    let paths = crate::jobs::split_job_paths(raw);
    for (index, path) in paths.iter().enumerate() {
        let text = if paths.len() == 1 {
            label.to_string()
        } else {
            format!("{label} {}", index + 1)
        };
        let btn = gtk::Button::with_label(&text);
        btn.set_tooltip_text(Some(path));
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let path = path.clone();
        btn.connect_clicked(move |_| {
            if let Some((remote, folder)) = browse_target(&path) {
                super::window::present_files_at(&dialog, &ctx, &remote, &folder);
                dialog.close();
            }
        });
        box_.append(&btn);
    }
}

fn append_transfer_rows(
    list: &gtk::ListBox,
    items: Option<&Vec<serde_json::Value>>,
    query: &str,
    active: bool,
    ctx: &AppCtx,
    parent: &adw::Dialog,
    job_type: &str,
) {
    let empty_title = if active {
        ctx.t_or(
            "shared.transferActivity.empty.noActive",
            "No active transfers",
        )
    } else {
        ctx.t_or(
            "shared.transferActivity.empty.noRecent",
            "No completed transfers",
        )
    };
    let Some(arr) = items else {
        let row = adw::ActionRow::new();
        row.set_title(&empty_title);
        list.append(&row);
        return;
    };
    let mut shown = 0;
    for item in arr {
        let parsed = crate::transfers::parse_transfer_row(item);
        if !query.is_empty() && !parsed.name.to_lowercase().contains(query) {
            continue;
        }
        let row = adw::ActionRow::new();
        row.set_title(&parsed.name);
        let src = if parsed.src.is_empty() {
            "—".into()
        } else {
            parsed.src.clone()
        };
        let dst = if parsed.dst.is_empty() {
            "—".into()
        } else {
            parsed.dst.clone()
        };
        row.set_subtitle(&format!(
            "{}% · {} · {src} → {dst}",
            parsed.percentage,
            crate::rclone::format_bytes(parsed.size)
        ));
        row.add_suffix(&transfer_row_actions(
            ctx, parent, &parsed, job_type, !active,
        ));
        list.append(&row);
        shown += 1;
    }
    if shown == 0 {
        let row = adw::ActionRow::new();
        row.set_title(&empty_title);
        list.append(&row);
    }
}

pub(crate) fn transfer_row_actions(
    ctx: &AppCtx,
    parent: &impl IsA<gtk::Widget>,
    parsed: &crate::transfers::TransferRow,
    job_type: &str,
    completed: bool,
) -> gtk::Box {
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_valign(gtk::Align::Center);
    if !parsed.src.is_empty() {
        actions.append(&transfer_side_actions(
            ctx,
            parent,
            &parsed.src,
            job_type,
            true,
            true,
        ));
    }
    if !parsed.dst.is_empty() {
        actions.append(&transfer_side_actions(
            ctx,
            parent,
            &parsed.dst,
            job_type,
            completed,
            false,
        ));
    }
    actions
}

fn transfer_side_actions(
    ctx: &AppCtx,
    parent: &impl IsA<gtk::Widget>,
    path: &str,
    job_type: &str,
    allow_dest_ops: bool,
    is_source: bool,
) -> gtk::Box {
    let side = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    side.add_css_class("linked");
    side.set_valign(gtk::Align::Center);
    let copy_url = if is_source {
        crate::transfers::can_copy_url_source(path, job_type, path_fs_info(ctx, path).as_ref())
    } else {
        crate::transfers::can_copy_url_dest(
            path,
            job_type,
            allow_dest_ops,
            path_fs_info(ctx, path).as_ref(),
        )
    };
    let can_download = if is_source {
        crate::transfers::can_download_source(path, job_type)
    } else {
        crate::transfers::can_download_dest(path, job_type, allow_dest_ops)
    };
    let can_delete = if is_source {
        crate::transfers::can_delete_source(job_type) && !path.is_empty()
    } else {
        crate::transfers::can_delete_dest(job_type, allow_dest_ops) && !path.is_empty()
    };
    if let Some((remote, rest)) = crate::transfers::browse_for(path) {
        let open = gtk::Button::from_icon_name("folder-open-symbolic");
        open.set_tooltip_text(Some(&ctx.t_or("common.browse", "Open in Files")));
        open.set_valign(gtk::Align::Center);
        let ctx = ctx.clone();
        open.connect_clicked(move |_| {
            ctx.request_browse(&remote, &rest);
        });
        side.append(&open);
    }
    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some(&ctx.t_or("common.copy", "Copy path")));
    copy.set_valign(gtk::Align::Center);
    let copy_text = path.to_string();
    copy.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&copy_text);
        }
    });
    side.append(&copy);
    if copy_url {
        if let Some((remote, rest)) = crate::transfers::browse_for(path) {
            let link = gtk::Button::from_icon_name("emblem-shared-symbolic");
            link.set_tooltip_text(Some(
                &ctx.t_or("shared.transferActivity.actions.copyUrl", "Copy URL"),
            ));
            link.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            link.connect_clicked(move |_| {
                let Some(client) = ctx.client() else {
                    return;
                };
                let full = if rest.is_empty() {
                    format!("{remote}:")
                } else if remote == "local" {
                    rest.clone()
                } else {
                    format!("{remote}:{rest}")
                };
                let (fs, remote_path) = crate::transfers::fs_and_remote(&full);
                if let Ok(url) = client.public_link(&fs, &remote_path) {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&url);
                    }
                }
            });
            side.append(&link);
        }
    }
    if can_download {
        if let Some((remote, rest, name)) = crate::transfers::download_target(path) {
            let dl = gtk::Button::from_icon_name("folder-download-symbolic");
            dl.set_tooltip_text(Some(
                &ctx.t_or("shared.transferActivity.actions.download", "Download File"),
            ));
            dl.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            let parent = parent.clone();
            dl.connect_clicked(move |_| {
                download_file(&parent, ctx.clone(), &remote, &rest, &name);
            });
            side.append(&dl);
        }
    }
    if can_delete {
        let del = gtk::Button::from_icon_name(if is_source {
            "user-trash-symbolic"
        } else {
            "edit-delete-symbolic"
        });
        del.set_tooltip_text(Some(&if is_source {
            ctx.t_or("nautilus.modals.delete.title", "Delete source")
        } else {
            ctx.t_or("nautilus.modals.delete.title", "Delete destination")
        }));
        del.set_valign(gtk::Align::Center);
        let ctx = ctx.clone();
        let parent = parent.clone();
        let path = path.to_string();
        let title = if is_source {
            ctx.t_or("nautilus.modals.delete.title", "Delete source file?")
        } else {
            ctx.t_or("nautilus.modals.delete.title", "Delete destination file?")
        };
        del.connect_clicked(move |_| {
            confirm_delete_path(&parent, ctx.clone(), &path, &title);
        });
        side.append(&del);
    }
    side
}

fn path_fs_info(ctx: &AppCtx, path: &str) -> Option<crate::rclone::FsInfo> {
    crate::transfers::remote_name_from_path(path).and_then(|remote| ctx.fs_info(&remote))
}

pub(crate) fn confirm_delete_path(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    path: &str,
    title: &str,
) {
    let alert = adw::AlertDialog::new(Some(title), Some(path));
    alert.add_response("cancel", "Cancel");
    alert.add_response("delete", "Delete");
    alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let path = path.to_string();
    alert.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }
        let Some(client) = ctx.client() else {
            return;
        };
        let (fs, remote) = crate::transfers::fs_and_remote(&path);
        let _ = client
            .purge(&fs, &remote)
            .or_else(|_| client.delete_file(&fs, &remote));
        ctx.refresh_runtime();
    });
    alert.present(Some(parent));
}

fn pdf_panel(path: Option<std::path::PathBuf>, name: &str, ctx: &AppCtx) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_start(16);
    box_.set_margin_end(16);
    let size = path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| crate::rclone::format_bytes(m.len() as i64))
        .unwrap_or_else(|| "—".into());
    let pages = path
        .as_ref()
        .and_then(|p| crate::media::pdf_page_count(p))
        .unwrap_or(0);
    let label = gtk::Label::new(Some(&format!(
        "{name}\nSize: {size}{}",
        if pages > 0 {
            format!("\n{pages} pages")
        } else {
            String::new()
        }
    )));
    label.set_wrap(true);
    label.set_xalign(0.0);
    box_.append(&label);
    if let Some(path) = path {
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_vexpand(true);
        let page = Rc::new(Cell::new(1u32));
        let page_label = gtk::Label::new(None);
        let show: Rc<dyn Fn()> = {
            let path = path.clone();
            let picture = picture.clone();
            let page = page.clone();
            let page_label = page_label.clone();
            Rc::new(move || {
                let current = page.get().max(1);
                let total = crate::media::pdf_page_count(&path).unwrap_or(pages.max(1));
                let current = current.min(total.max(1));
                page.set(current);
                page_label.set_text(&format!("{current} / {total}"));
                if let Some(preview) = crate::media::render_pdf_page(&path, current) {
                    picture.set_filename(Some(&preview));
                    picture.set_visible(true);
                } else {
                    picture.set_visible(false);
                }
            })
        };
        show();
        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        nav.set_halign(gtk::Align::Center);
        let prev = gtk::Button::from_icon_name("go-previous-symbolic");
        prev.set_tooltip_text(Some("Previous page"));
        let next = gtk::Button::from_icon_name("go-next-symbolic");
        next.set_tooltip_text(Some("Next page"));
        {
            let show = show.clone();
            let page = page.clone();
            prev.connect_clicked(move |_| {
                page.set(page.get().saturating_sub(1).max(1));
                show();
            });
        }
        {
            let show = show.clone();
            let page = page.clone();
            next.connect_clicked(move |_| {
                page.set(page.get().saturating_add(1));
                show();
            });
        }
        nav.append(&prev);
        nav.append(&page_label);
        nav.append(&next);
        box_.append(&picture);
        if pages > 1 || crate::media::pdf_page_count(&path).unwrap_or(0) > 1 {
            box_.append(&nav);
        }
        let open =
            gtk::Button::with_label(&ctx.t_or("fileBrowser.fileViewer.openNative", "Open native"));
        open.add_css_class("suggested-action");
        let p = path.clone();
        open.connect_clicked(move |_| {
            let _ = open::that(&p);
        });
        box_.append(&open);
    }
    box_
}

fn attach_text_preview(
    parent: &gtk::Box,
    name: &str,
    text: &str,
    editable: bool,
    save_path: Option<&str>,
    remote_save: Option<(AppCtx, String, String)>,
) {
    let shown = if text.len() > 200_000 {
        format!("{}\n\n… truncated …", &text[..200_000])
    } else {
        text.to_string()
    };
    let view = gtk::TextView::new();
    view.set_monospace(true);
    view.set_editable(editable);
    apply_syntax_highlight(&view, name, &shown);
    attach_live_syntax(&view, name);
    let source_scroll = gtk::ScrolledWindow::new();
    source_scroll.set_vexpand(true);
    source_scroll.set_child(Some(&view));
    if crate::markdown::is_markdown(name) {
        let preview = gtk::TextView::new();
        preview.set_editable(false);
        preview.set_wrap_mode(gtk::WrapMode::WordChar);
        preview
            .buffer()
            .set_text(&crate::markdown::to_preview(&shown));
        let preview_scroll = gtk::ScrolledWindow::new();
        preview_scroll.set_vexpand(true);
        preview_scroll.set_child(Some(&preview));
        let stack = gtk::Stack::new();
        stack.add_named(&source_scroll, Some("source"));
        stack.add_named(&preview_scroll, Some("preview"));
        let toggle = gtk::ToggleButton::with_label(
            &remote_save
                .as_ref()
                .map(|(ctx, _, _)| ctx.t_or("fileBrowser.fileViewer.showPreview", "Preview"))
                .unwrap_or_else(|| "Preview".into()),
        );
        // Label is set by caller context; keep short so the toggle fits the toolbar.
        {
            let stack = stack.clone();
            toggle.connect_toggled(move |btn| {
                stack.set_visible_child_name(if btn.is_active() { "preview" } else { "source" });
            });
        }
        parent.append(&toggle);
        parent.append(&stack);
    } else {
        parent.append(&source_scroll);
    }
    if editable && (save_path.is_some() || remote_save.is_some()) {
        view.set_editable(false);
        let edit = gtk::Button::with_label(
            &remote_save
                .as_ref()
                .map(|(ctx, _, _)| ctx.t_or("fileBrowser.fileViewer.edit", "Edit"))
                .unwrap_or_else(|| "Edit".into()),
        );
        let save = gtk::Button::with_label(
            &remote_save
                .as_ref()
                .map(|(ctx, _, _)| ctx.t_or("fileBrowser.fileViewer.save", "Save"))
                .unwrap_or_else(|| "Save".into()),
        );
        save.add_css_class("suggested-action");
        save.set_sensitive(false);
        let cancel = gtk::Button::with_label(
            &remote_save
                .as_ref()
                .map(|(ctx, _, _)| ctx.t_or("fileBrowser.fileViewer.cancel", "Cancel"))
                .unwrap_or_else(|| "Cancel".into()),
        );
        cancel.set_sensitive(false);
        {
            let view = view.clone();
            let save = save.clone();
            let cancel = cancel.clone();
            edit.connect_clicked(move |_| {
                view.set_editable(true);
                save.set_sensitive(true);
                cancel.set_sensitive(true);
            });
        }
        {
            let view = view.clone();
            let save = save.clone();
            let cancel_btn = cancel.clone();
            let original = shown.clone();
            cancel.connect_clicked(move |_| {
                view.buffer().set_text(&original);
                view.set_editable(false);
                save.set_sensitive(false);
                cancel_btn.set_sensitive(false);
            });
        }
        let view = view.clone();
        let local = save_path.map(|p| p.to_string());
        save.connect_clicked(move |btn| {
            let buffer = view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let mut ok = true;
            if let Some(path) = &local {
                if let Err(e) = std::fs::write(path, text.as_str()) {
                    log::warn!("failed to save {path}: {e}");
                    ok = false;
                }
            }
            if let Some((ctx, fs, remote)) = remote_save.clone() {
                let Some(client) = ctx.client() else {
                    return;
                };
                let dir = crate::rclone::parent_remote_path(&remote);
                let name = remote
                    .rsplit_once('/')
                    .map(|(_, n)| n)
                    .filter(|n| !n.is_empty())
                    .unwrap_or(remote.as_str());
                if let Err(e) = client.upload_file(&fs, &dir, name, text.as_bytes()) {
                    log::warn!("remote save failed: {e}");
                    ok = false;
                }
            }
            view.set_editable(false);
            btn.set_sensitive(false);
            if ok {
                let label = remote_save
                    .as_ref()
                    .map(|(ctx, _, _)| ctx.t_or("common.ok", "OK"))
                    .unwrap_or_else(|| "OK".into());
                btn.set_label(&label);
            }
        });
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.append(&edit);
        bar.append(&cancel);
        bar.append(&save);
        parent.append(&bar);
    }
}

fn attach_lnk_preview(parent: &gtk::Box, ctx: &AppCtx, content: &str) -> bool {
    let targets = crate::textfix::extract_lnk_targets(content);
    if targets.is_empty() {
        return false;
    }
    let title = gtk::Label::new(Some(
        &ctx.t_or("fileBrowser.fileViewer.shortcutTargets", "Shortcut Targets"),
    ));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    parent.append(&title);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for target in targets {
        let row = adw::ActionRow::new();
        row.set_title(&target);
        list.append(&row);
    }
    parent.append(&list);
    true
}

fn append_markdown_targets(
    host: &gtk::Box,
    name: &str,
    text: &str,
    remote: &str,
    file_path: &str,
    ctx: &AppCtx,
    dialog_parent: &impl IsA<gtk::Widget>,
) {
    if !crate::markdown::is_markdown(name) {
        return;
    }
    let targets = crate::markdown::relative_targets(text);
    if targets.is_empty() {
        return;
    }
    let title = gtk::Label::new(Some(
        &ctx.t_or("fileBrowser.fileViewer.shortcutTargets", "Shortcut Targets"),
    ));
    title.set_xalign(0.0);
    title.add_css_class("heading");
    host.append(&title);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (label, rel) in targets {
        let resolved = crate::markdown::resolve_relative_path(file_path, &rel);
        let row = adw::ActionRow::new();
        row.set_title(&label);
        row.set_subtitle(&resolved);
        if resolved.starts_with("http://") || resolved.starts_with("https://") {
            let link = gtk::LinkButton::with_label(&resolved, "");
            link.set_uri(&resolved);
            row.add_suffix(&link);
        } else {
            let open = gtk::Button::from_icon_name("document-open-symbolic");
            open.set_valign(gtk::Align::Center);
            open.set_tooltip_text(Some(
                &ctx.t_or("fileBrowser.fileViewer.openNative", "Open in External App"),
            ));
            let ctx = ctx.clone();
            let parent = dialog_parent.clone();
            let open_remote = if crate::markdown::is_passthrough_ref(&resolved)
                && resolved.contains(':')
                && !resolved.starts_with('/')
            {
                crate::rclone::split_remote_path(&resolved).0
            } else {
                remote.to_string()
            };
            let path = if open_remote == "local" && crate::markdown::is_passthrough_ref(&resolved) {
                resolved.clone()
            } else if crate::markdown::is_passthrough_ref(&resolved)
                && resolved.contains(':')
                && !resolved.starts_with('/')
            {
                crate::rclone::split_remote_path(&resolved).1
            } else {
                resolved.clone()
            };
            let file_name = path
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(path.as_str())
                .to_string();
            open.connect_clicked(move |_| {
                file_viewer(
                    &parent,
                    ctx.clone(),
                    &open_remote,
                    &path,
                    &file_name,
                    false,
                    &[],
                );
            });
            row.add_suffix(&open);
        }
        list.append(&row);
    }
    host.append(&list);
}

fn attach_remote_stream_preview(
    parent: &gtk::Box,
    url: &str,
    category: crate::operations::FileTypeCategory,
) {
    let file = gio::File::for_uri(url);
    if matches!(category, crate::operations::FileTypeCategory::Image) {
        let picture = gtk::Picture::for_file(&file);
        picture.set_vexpand(true);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        parent.append(&picture);
    }
    if matches!(
        category,
        crate::operations::FileTypeCategory::Video | crate::operations::FileTypeCategory::Audio
    ) {
        let video = gtk::Video::for_file(Some(&file));
        video.set_vexpand(true);
        video.set_autoplay(true);
        parent.append(&video);
    }
}

fn append_download_preview_button(
    actions: &gtk::Box,
    ctx: &AppCtx,
    box_: &gtk::Box,
    fs: &str,
    path: &str,
    name: &str,
    category: crate::operations::FileTypeCategory,
) {
    let label = ctx.t_or("fileBrowser.fileViewer.downloadPreview", "Download preview");
    let fetch = gtk::Button::with_label(&label);
    fetch.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let box_ = box_.clone();
        let dest = std::env::temp_dir().join(name);
        let fs = fs.to_string();
        let path = path.to_string();
        let name = name.to_string();
        fetch.connect_clicked(move |btn| {
            let Some(client) = ctx.client() else {
                return;
            };
            if client
                .copy_file(&fs, &path, "/", &dest.to_string_lossy())
                .is_ok()
            {
                attach_local_media_preview(&box_, &dest, category, &name, &ctx);
                btn.set_sensitive(false);
            }
        });
    }
    actions.append(&fetch);
}

fn attach_local_media_preview(
    parent: &gtk::Box,
    local: &std::path::Path,
    category: crate::operations::FileTypeCategory,
    name: &str,
    ctx: &AppCtx,
) {
    let local_s = local.to_string_lossy();
    if matches!(category, crate::operations::FileTypeCategory::Image) {
        let picture = gtk::Picture::for_filename(local);
        picture.set_vexpand(true);
        parent.append(&picture);
    }
    if matches!(
        category,
        crate::operations::FileTypeCategory::Video | crate::operations::FileTypeCategory::Audio
    ) {
        let video = gtk::Video::for_filename(Some(local_s.as_ref()));
        video.set_vexpand(true);
        video.set_autoplay(true);
        parent.append(&video);
        if matches!(category, crate::operations::FileTypeCategory::Audio) {
            attach_audio_cover(parent, Some(local), None);
        }
    }
    if matches!(category, crate::operations::FileTypeCategory::Pdf) {
        parent.append(&pdf_panel(Some(local.to_path_buf()), name, ctx));
    }
}

fn attach_audio_cover(
    parent: &gtk::Box,
    local: Option<&std::path::Path>,
    remote: Option<(&AppCtx, &str, &str, &str)>,
) {
    let cover = local
        .and_then(|path| crate::media::best_audio_cover(path))
        .or_else(|| {
            let (ctx, remote, path, name) = remote?;
            let src = format!("{remote}:{path}");
            let settings = ctx.settings.borrow();
            let binary = crate::rclone::engine::resolve_rclone_binary(&settings.core.rclone_binary);
            let extra = settings.core.rclone_additional_flags.clone();
            let config = crate::repair::config_path_from_flags(&extra);
            drop(settings);
            let bytes = crate::media::read_remote_prefix(
                &binary,
                &extra,
                config.as_deref(),
                &src,
                crate::media::COVER_PROBE_BYTES,
            )?;
            let ext = std::path::Path::new(name)
                .extension()
                .and_then(|e| e.to_str());
            crate::media::extract_picture_from_bytes(&bytes, ext)
                .and_then(|pic| crate::media::write_temp_picture(&pic))
        });
    let Some(cover) = cover else {
        return;
    };
    let picture = gtk::Picture::for_filename(&cover);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_size_request(-1, 180);
    parent.append(&picture);
}

pub fn file_viewer(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
    is_dir: bool,
    siblings: &[(String, bool)],
) {
    if try_spawn_standalone(
        &ctx,
        "file-viewer",
        serde_json::json!({
            "remote": remote,
            "path": path,
            "name": name,
            "isDir": is_dir,
        }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(name);
    dialog.set_content_width(720);
    dialog.set_content_height(520);
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    nav.set_margin_start(12);
    nav.set_margin_end(12);
    nav.set_margin_top(8);
    let index = siblings.iter().position(|(n, _)| n == name);
    let prev = gtk::Button::from_icon_name("go-previous-symbolic");
    prev.set_tooltip_text(Some(
        &ctx.t_or("fileBrowser.fileViewer.previousFile", "Previous file"),
    ));
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some(
        &ctx.t_or("fileBrowser.fileViewer.nextFile", "Next file"),
    ));
    let remote_size = if remote == "local" {
        std::fs::metadata(path).ok().map(|m| m.len() as i64)
    } else {
        ctx.client().and_then(|c| {
            let fs = if remote == "local" {
                "/".into()
            } else {
                remote_fs(remote, "")
            };
            c.stat(&fs, path).ok().flatten().map(|s| s.size)
        })
    }
    .filter(|n| *n > 0);
    let size_hint = remote_size.map(crate::rclone::format_bytes);
    let pos = gtk::Label::new(Some(&{
        let base = match index {
            Some(i) if !siblings.is_empty() => format!("{} / {}", i + 1, siblings.len()),
            _ => name.to_string(),
        };
        match &size_hint {
            Some(size) => format!("{base} · {size}"),
            None => base,
        }
    }));
    pos.set_hexpand(true);
    pos.set_xalign(0.5);
    prev.set_sensitive(index.is_some_and(|i| i > 0));
    next.set_sensitive(index.is_some_and(|i| i + 1 < siblings.len()));
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        let siblings = siblings.to_vec();
        let dialog = dialog.clone();
        prev.connect_clicked(move |_| {
            let Some(i) = siblings.iter().position(|(n, _)| n == name.as_str()) else {
                return;
            };
            if i == 0 {
                return;
            }
            let (prev_name, prev_dir) = siblings[i - 1].clone();
            let parent_path = crate::rclone::parent_remote_path(&path);
            let next_path = crate::rclone::join_remote_path(&parent_path, &prev_name);
            dialog.close();
            file_viewer(
                &parent,
                ctx.clone(),
                &remote,
                &next_path,
                &prev_name,
                prev_dir,
                &siblings,
            );
        });
    }
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        let siblings = siblings.to_vec();
        let dialog = dialog.clone();
        next.connect_clicked(move |_| {
            let Some(i) = siblings.iter().position(|(n, _)| n == name.as_str()) else {
                return;
            };
            if i + 1 >= siblings.len() {
                return;
            }
            let (next_name, next_dir) = siblings[i + 1].clone();
            let parent_path = crate::rclone::parent_remote_path(&path);
            let next_path = crate::rclone::join_remote_path(&parent_path, &next_name);
            dialog.close();
            file_viewer(
                &parent,
                ctx.clone(),
                &remote,
                &next_path,
                &next_name,
                next_dir,
                &siblings,
            );
        });
    }
    nav.append(&prev);
    nav.append(&pos);
    nav.append(&next);
    {
        let keys = gtk::EventControllerKey::new();
        let prev = prev.clone();
        let next = next.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Left || key == gtk::gdk::Key::KP_Left {
                prev.emit_clicked();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Right || key == gtk::gdk::Key::KP_Right {
                next.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        dialog.add_controller(keys);
    }
    let category = crate::operations::FileTypeCategory::from_name(name, is_dir);
    let info = gtk::Label::new(Some(&format!("{remote}:{path}")));
    info.set_wrap(true);
    info.set_margin_top(16);
    info.set_margin_start(16);
    info.set_margin_end(16);
    let open = gtk::Button::with_label(
        &ctx.t_or("fileBrowser.fileViewer.openNative", "Open in External App"),
    );
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        open.connect_clicked(move |_| {
            ctx.notify(
                &ctx.t_or("fileBrowser.fileViewer.openNative", "Open in External App"),
                &ctx.tf("fileBrowser.fileViewer.openingNative", &[("name", &name)]),
            );
            let client = ctx.client();
            if let Err(err) =
                crate::fileops::open_file_natively(client.as_ref(), &remote, &path, &name)
            {
                let body = if err == crate::fileops::ENGINE_OFFLINE {
                    ctx.t_or(
                        "notification.title.engineConnectionFailed",
                        "Engine Connection Error",
                    )
                } else {
                    err
                };
                let alert = adw::AlertDialog::new(
                    Some(&ctx.t_or(
                        "fileBrowser.fileViewer.errorOpenNative",
                        "Could not open file",
                    )),
                    Some(&body),
                );
                alert.add_response("ok", &ctx.t_or("common.ok", "OK"));
                alert.present(Some(&parent));
            }
        });
    }
    let download =
        gtk::Button::with_label(&ctx.t_or("fileBrowser.fileViewer.download", "Download"));
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        download.connect_clicked(move |_| {
            download_file(&parent, ctx.clone(), &remote, &path, &name);
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.append(&nav);
    box_.append(&info);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&open);
    actions.append(&download);
    box_.append(&actions);
    if is_dir {
        let size_label = gtk::Label::new(Some(&ctx.t_or(
            "fileBrowser.fileViewer.calculatingFolderSize",
            "Calculating folder size...",
        )));
        size_label.add_css_class("dim-label");
        size_label.set_wrap(true);
        size_label.set_xalign(0.0);
        size_label.set_margin_start(16);
        size_label.set_margin_end(16);
        let fs = if remote == "local" {
            "/".into()
        } else {
            remote_fs(remote, "")
        };
        if let Some(client) = ctx.client() {
            match client.size(&fs, path) {
                Ok(size) => {
                    let (count, bytes) = crate::rclone::parse_object_size(&size);
                    let count_s = count.to_string();
                    let bytes_s = crate::rclone::format_bytes(bytes);
                    let contains = ctx.tf(
                        "fileBrowser.fileViewer.containsFiles",
                        &[("count", &count_s)],
                    );
                    let contains = if contains.contains("{{") {
                        format!("Contains {count} files")
                    } else {
                        contains
                    };
                    let total = ctx.tf("fileBrowser.fileViewer.totalSize", &[("size", &bytes_s)]);
                    let total = if total.contains("{{") {
                        format!("Total size: {bytes_s}")
                    } else {
                        total
                    };
                    size_label.set_text(&format!("{contains}\n{total}"));
                }
                Err(_) => {
                    size_label.set_text(&ctx.t_or(
                        "fileBrowser.fileViewer.errorCalculateSize",
                        "Failed to calculate folder size",
                    ));
                }
            }
        }
        box_.append(&size_label);
    }
    if !is_dir && matches!(category, crate::operations::FileTypeCategory::Archive) {
        let heading = gtk::Label::new(Some(
            &ctx.t_or("fileBrowser.fileViewer.archiveContents", "Archive Contents"),
        ));
        heading.add_css_class("heading");
        heading.set_xalign(0.0);
        box_.append(&heading);
        let archive_list = gtk::ListBox::new();
        archive_list.add_css_class("boxed-list");
        let src = if remote == "local" {
            path.to_string()
        } else {
            format!("{remote}:{path}")
        };
        match ctx.client().and_then(|c| c.archive_list(&src, true).ok()) {
            Some(items) if !items.is_empty() => {
                for item in items {
                    let row = adw::ActionRow::new();
                    row.set_title(&item.path);
                    row.set_subtitle(&item.subtitle());
                    row.add_prefix(&gtk::Image::from_icon_name(if item.is_dir {
                        "folder-symbolic"
                    } else {
                        "package-x-generic-symbolic"
                    }));
                    archive_list.append(&row);
                }
            }
            Some(_) => {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("nautilus.empty.noFiles", "Archive is empty"));
                archive_list.append(&row);
            }
            None => {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or(
                    "fileBrowser.fileViewer.errorListArchive",
                    "Failed to list archive contents",
                ));
                archive_list.append(&row);
            }
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_min_content_height(220);
        scroll.set_child(Some(&archive_list));
        box_.append(&scroll);
        let extract =
            gtk::Button::with_label(&ctx.t_or("fileBrowser.fileViewer.extract", "Extract"));
        extract.add_css_class("suggested-action");
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let src = src.clone();
            let dest = crate::rclone::parent_remote_path(path);
            let dest = if remote == "local" {
                dest
            } else if dest.is_empty() {
                format!("{remote}:")
            } else {
                format!("{remote}:{dest}")
            };
            extract.connect_clicked(move |_| {
                let ctx = ctx.clone();
                let parent = parent.clone();
                let src = src.clone();
                let fallback = dest.clone();
                let initial = fallback.clone();
                pick_destination(
                    &parent,
                    &ctx,
                    &ctx.t_or("fileBrowser.fileViewer.extract", "Extract archive"),
                    &initial,
                    {
                        let ctx = ctx.clone();
                        let parent = parent.clone();
                        move |chosen| {
                            let Some(client) = ctx.client() else {
                                return;
                            };
                            let dest = if chosen.is_empty() {
                                fallback.clone()
                            } else {
                                chosen
                            };
                            match client.archive_extract(&src, &dest) {
                                Ok(_) => {
                                    let toast = adw::AlertDialog::new(
                                        Some(
                                            &ctx.t_or("fileBrowser.fileViewer.extract", "Extract"),
                                        ),
                                        Some(&ctx.t_or("common.ok", "Extract started")),
                                    );
                                    toast.add_response("ok", &ctx.t("common.ok"));
                                    toast.present(Some(&parent));
                                }
                                Err(e) => {
                                    let toast = adw::AlertDialog::new(
                                        Some(&ctx.t_or(
                                            "fileBrowser.fileViewer.errorExtract",
                                            "Failed to extract archive",
                                        )),
                                        Some(&e.to_string()),
                                    );
                                    toast.add_response("ok", &ctx.t("common.ok"));
                                    toast.present(Some(&parent));
                                }
                            }
                        }
                    },
                );
            });
        }
        actions.append(&extract);
    }
    let mut preview_path = if !is_dir && remote == "local" {
        Some(std::path::PathBuf::from(path))
    } else {
        None
    };
    let mut shortcut_shown = false;
    if !is_dir && crate::textfix::is_lnk_name(name) {
        let raw = if remote == "local" {
            std::fs::read(path)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        } else {
            ctx.client()
                .and_then(|c| c.cat(&remote_fs(remote, ""), path, Some(65_536)).ok())
        };
        if let Some(raw) = raw {
            shortcut_shown = attach_lnk_preview(&box_, &ctx, &raw);
        }
    }
    if !is_dir && remote != "local" {
        if let Some(client) = ctx.client() {
            let fs = remote_fs(remote, "");
            if matches!(category, crate::operations::FileTypeCategory::Text) {
                if let Ok(text) = client.cat(&fs, path, Some(crate::rclone::CAT_PREVIEW_BYTES)) {
                    let text = crate::textfix::repair_text(&text);
                    if crate::textfix::looks_like_binary(&text) {
                        info.set_text(
                            &ctx.t_or("fileBrowser.fileViewer.binaryFile", "This is a binary file"),
                        );
                    } else {
                        info.set_text(&ctx.t_or(
                            "fileBrowser.fileViewer.remotePreview",
                            "Remote preview via operations/cat",
                        ));
                        attach_text_preview(
                            &box_,
                            name,
                            &text,
                            true,
                            None,
                            Some((ctx.clone(), fs, path.to_string())),
                        );
                        append_markdown_targets(&box_, name, &text, remote, path, &ctx, parent);
                    }
                }
            } else if category.can_stream_preview()
                && client.probe_rc_serve(&client.rc_serve_url(remote, path))
            {
                let url = client.rc_serve_url(remote, path);
                info.set_text(&ctx.t_or(
                    "fileBrowser.fileViewer.remoteStream",
                    "Streaming remote preview",
                ));
                attach_remote_stream_preview(&box_, &url, category);
                if matches!(category, crate::operations::FileTypeCategory::Audio) {
                    attach_audio_cover(&box_, None, Some((&ctx, remote, path, name)));
                }
            } else if crate::media::should_warn_remote_preview(remote_size) {
                let size_label = remote_size
                    .map(crate::rclone::format_bytes)
                    .unwrap_or_else(|| "?".into());
                info.set_text(&ctx.tf(
                    "fileBrowser.fileViewer.largeRemotePreview",
                    &[("size", &size_label)],
                ));
                if info.text().contains("{{") {
                    info.set_text(&format!(
                        "This remote file is {size_label}. Downloading the whole file for preview may take a while."
                    ));
                }
                if matches!(category, crate::operations::FileTypeCategory::Audio) {
                    attach_audio_cover(&box_, None, Some((&ctx, remote, path, name)));
                }
                append_download_preview_button(&actions, &ctx, &box_, &fs, path, name, category);
            } else {
                let dest = std::env::temp_dir().join(name);
                if client
                    .copy_file(&fs, path, "/", &dest.to_string_lossy())
                    .is_ok()
                {
                    info.set_text(&ctx.tf(
                        "fileBrowser.fileViewer.downloadedPreview",
                        &[("path", &dest.display().to_string())],
                    ));
                    if info.text().contains("{{") {
                        info.set_text(&format!("Downloaded preview to {}", dest.display()));
                    }
                    preview_path = Some(dest);
                }
            }
        }
    }
    if let Some(local) = preview_path.as_ref() {
        attach_local_media_preview(&box_, local, category, name, &ctx);
        if remote == "local" && matches!(category, crate::operations::FileTypeCategory::Text) {
            let text = std::fs::read(local)
                .ok()
                .map(|bytes| crate::textfix::decode_preview_bytes(&bytes))
                .unwrap_or_default();
            if crate::textfix::looks_like_binary(&text) {
                info.set_text(
                    &ctx.t_or("fileBrowser.fileViewer.binaryFile", "This is a binary file"),
                );
            } else {
                attach_text_preview(
                    &box_,
                    name,
                    &text,
                    true,
                    Some(&local.to_string_lossy()),
                    None,
                );
                append_markdown_targets(&box_, name, &text, remote, path, &ctx, parent);
            }
        }
    } else if remote == "local" && matches!(category, crate::operations::FileTypeCategory::Audio) {
        attach_audio_cover(&box_, Some(std::path::Path::new(path)), None);
    }
    let previewed = is_dir
        || matches!(
            category,
            crate::operations::FileTypeCategory::Archive
                | crate::operations::FileTypeCategory::Image
                | crate::operations::FileTypeCategory::Video
                | crate::operations::FileTypeCategory::Audio
                | crate::operations::FileTypeCategory::Pdf
                | crate::operations::FileTypeCategory::Text
        );
    if !previewed && !shortcut_shown {
        let page = adw::StatusPage::new();
        page.set_icon_name(Some(
            if matches!(category, crate::operations::FileTypeCategory::Binary) {
                "application-x-executable-symbolic"
            } else {
                "dialog-information-symbolic"
            },
        ));
        page.set_title(
            &if matches!(category, crate::operations::FileTypeCategory::Binary) {
                ctx.t_or("fileBrowser.fileViewer.binaryFile", "This is a binary file")
            } else {
                ctx.t_or(
                    "fileBrowser.fileViewer.previewNotAvailable",
                    "Preview not available",
                )
            },
        );
        page.set_description(Some(name));
        box_.append(&page);
    }
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn metadata_item_row(ctx: &AppCtx, key: &str, meta: &serde_json::Value) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(key);
    let typ = meta
        .get("Type")
        .or_else(|| meta.get("type"))
        .and_then(|x| x.as_str());
    let help = meta
        .get("Help")
        .or_else(|| meta.get("help"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let example = meta
        .get("Example")
        .or_else(|| meta.get("example"))
        .and_then(|x| x.as_str());
    let readonly = meta
        .get("ReadOnly")
        .or_else(|| meta.get("readOnly"))
        .and_then(|x| x.as_bool());
    let mut parts = Vec::new();
    if let Some(typ) = typ.filter(|s| !s.is_empty()) {
        parts.push(typ.to_string());
    } else {
        parts.push(ctx.t_or("fileBrowser.remoteAbout.unknown", "Unknown"));
    }
    if !help.is_empty() {
        parts.push(help.to_string());
    }
    if let Some(ex) = example.filter(|s| !s.is_empty()) {
        parts.push(format!(
            "{}: {ex}",
            ctx.t_or("fileBrowser.remoteAbout.example", "Example")
        ));
    }
    if let Some(ro) = readonly {
        let yes = ctx.t_or("fileBrowser.remoteAbout.yes", "Yes");
        let no = ctx.t_or("fileBrowser.remoteAbout.no", "No");
        parts.push(format!(
            "{}: {}",
            ctx.t_or("fileBrowser.remoteAbout.readOnly", "Read Only"),
            if ro { yes } else { no }
        ));
    }
    row.set_subtitle(&parts.join(" · "));
    row
}

pub fn remote_about(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
    if try_spawn_standalone(
        &ctx,
        "remote-about",
        serde_json::json!({ "remote": remote }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!(
        "{} {remote}",
        ctx.t_or("nautilus.contextMenu.about", "About")
    ));
    dialog.set_content_width(560);
    dialog.set_content_height(640);
    let page = adw::PreferencesPage::new();
    let usage = adw::PreferencesGroup::new();
    usage.set_title(&ctx.t_or("fileBrowser.remoteAbout.tabs.overview", "Usage"));
    let features = adw::PreferencesGroup::new();
    features.set_title(&ctx.t_or("fileBrowser.remoteAbout.tabs.features", "Features"));
    let hashes = adw::PreferencesGroup::new();
    hashes.set_title(&ctx.t_or("fileBrowser.remoteAbout.supportedHashes", "Hashes"));
    let metadata = adw::PreferencesGroup::new();
    metadata.set_title(&ctx.t_or("fileBrowser.remoteAbout.tabs.metadata", "Metadata"));
    let fs = remote_fs(remote, "");
    let group = format!(
        "gtk/remote-about/{remote}-{}",
        chrono::Utc::now().timestamp_millis()
    );
    if let Some(client) = ctx.client() {
        match client.about(&fs) {
            Ok(about) => {
                for (key, label, fallback) in [
                    ("used", "fileBrowser.remoteAbout.usedSpace", "Used Space"),
                    ("free", "fileBrowser.remoteAbout.freeSpace", "Free Space"),
                    ("total", "fileBrowser.remoteAbout.totalSpace", "Total Space"),
                    ("trashed", "fileBrowser.remoteAbout.trashed", "Trashed"),
                ] {
                    if let Some(value) = about.get(key).and_then(|x| x.as_i64()) {
                        let row = adw::ActionRow::new();
                        row.set_title(&ctx.t_or(label, fallback));
                        row.set_subtitle(&crate::rclone::format_bytes(value));
                        usage.add(&row);
                    }
                }
                for (key, value) in about.as_object().cloned().unwrap_or_default() {
                    if matches!(key.as_str(), "used" | "free" | "total" | "trashed") {
                        continue;
                    }
                    let row = adw::ActionRow::new();
                    row.set_title(&key);
                    row.set_subtitle(&value.to_string());
                    usage.add(&row);
                }
            }
            Err(e) => {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or(
                    "fileBrowser.remoteAbout.usageUnavailable",
                    "Unable to query disk usage",
                ));
                row.set_subtitle(&e.to_string());
                usage.add(&row);
            }
        }
        match client.size(&fs, "") {
            Ok(size) => {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.remoteAbout.objects", "Objects"));
                let count = size.get("count").and_then(|x| x.as_i64()).unwrap_or(0);
                let bytes = size.get("bytes").and_then(|x| x.as_i64()).unwrap_or(0);
                row.set_subtitle(&format!("{count} · {}", crate::rclone::format_bytes(bytes)));
                usage.add(&row);
            }
            Err(e) => {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.properties.size", "Size"));
                row.set_subtitle(&e.to_string());
                usage.add(&row);
            }
        }
        let info = ctx.fs_info(remote).or_else(|| client.fs_info(&fs).ok());
        if let Some(info) = info {
            let name = adw::ActionRow::new();
            name.set_title(&ctx.t("common.name"));
            name.set_subtitle(&info.name);
            usage.add(&name);
            let root = adw::ActionRow::new();
            root.set_title(&ctx.t_or("fileBrowser.remoteAbout.root", "Root"));
            let root_text = if info.root.is_empty() {
                "/".to_string()
            } else {
                info.root.clone()
            };
            root.set_subtitle(&root_text);
            usage.add(&root);
            let precision = adw::ActionRow::new();
            precision.set_title(&ctx.t_or(
                "fileBrowser.remoteAbout.timePrecision",
                "Timestamp precision",
            ));
            let precision_text = nanoseconds_to_duration(info.precision);
            precision.set_subtitle(&precision_text);
            usage.add(&precision);
            if info.hashes.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.remoteAbout.noneSupported", "None"));
                hashes.add(&row);
            } else {
                for hash in &info.hashes {
                    let row = adw::ActionRow::new();
                    row.set_title(&hash.to_ascii_uppercase());
                    hashes.add(&row);
                }
            }
            for (key, value) in &info.features {
                if key == "IsLocal" {
                    continue;
                }
                let row = adw::ActionRow::new();
                row.set_title(key);
                let yes = ctx.t_or("fileBrowser.remoteAbout.yes", "Yes");
                let no = ctx.t_or("fileBrowser.remoteAbout.no", "No");
                row.set_subtitle(if *value { &yes } else { &no });
                features.add(&row);
            }
            let (system_items, standard_items) = group_metadata_info(&info.metadata);
            let metadata_system = adw::PreferencesGroup::new();
            metadata_system
                .set_title(&ctx.t_or("fileBrowser.remoteAbout.metadata.system", "System Metadata"));
            let metadata_standard = adw::PreferencesGroup::new();
            metadata_standard.set_title(&ctx.t_or(
                "fileBrowser.remoteAbout.metadata.standard",
                "Standard Metadata",
            ));
            for (key, meta) in &system_items {
                metadata_system.add(&metadata_item_row(&ctx, key, meta));
            }
            for (key, meta) in &standard_items {
                metadata_standard.add(&metadata_item_row(&ctx, key, meta));
            }
            if system_items.is_empty() && standard_items.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or(
                    "fileBrowser.remoteAbout.noMetadata",
                    "No metadata specifications available.",
                ));
                metadata.add(&row);
            }
            page.add(&usage);
            page.add(&hashes);
            page.add(&features);
            if !system_items.is_empty() {
                page.add(&metadata_system);
            }
            if !standard_items.is_empty() {
                page.add(&metadata_standard);
            }
            if system_items.is_empty() && standard_items.is_empty() {
                page.add(&metadata);
            }
        } else {
            page.add(&usage);
            page.add(&hashes);
            page.add(&features);
            page.add(&metadata);
        }
    } else {
        page.add(&usage);
        page.add(&hashes);
        page.add(&features);
        page.add(&metadata);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    dialog.set_child(Some(&scroll));
    {
        let ctx = ctx.clone();
        dialog.connect_closed(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.job_stop_group(&group);
            }
        });
    }
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub(crate) fn download_file(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let dialog = gtk::FileDialog::new();
    dialog.set_initial_name(Some(name));
    let remote = remote.to_string();
    let path = path.to_string();
    dialog.save(
        Some(&win),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(dest) = file.path() else {
                return;
            };
            let Some(client) = ctx.client() else {
                return;
            };
            let fs = if remote == "local" {
                "/".into()
            } else {
                remote_fs(&remote, "")
            };
            let src_remote = if remote == "local" {
                path.trim_start_matches('/').to_string()
            } else {
                path
            };
            let dest_path = dest.to_string_lossy().into_owned();
            match client.start_job(
                "operations/copyfile",
                serde_json::json!({
                    "srcFs": fs,
                    "srcRemote": src_remote,
                    "dstFs": "/",
                    "dstRemote": dest_path,
                }),
            ) {
                Ok(id) => {
                    crate::jobs::remember_grouped(
                        &mut ctx.store.borrow_mut().job_meta,
                        &[id],
                        crate::store::JobMeta {
                            origin: "filemanager".into(),
                            profile: "default".into(),
                            remote: remote.clone(),
                            backend: ctx.backend_key(),
                            target: dest_path,
                            ..Default::default()
                        },
                    );
                    ctx.persist();
                    ctx.refresh_runtime();
                }
                Err(e) => log::warn!("download failed: {e}"),
            }
        },
    );
}

pub fn templates(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    if try_spawn_standalone(&ctx, "template-manager", serde_json::json!({})) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("titlebar.menu.templates", "Templates"));
    dialog.set_content_width(560);
    dialog.set_content_height(480);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for template in ctx.store.borrow().templates.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&template.name);
        row.set_subtitle(&template.description);
        let apply = gtk::Button::with_label(&ctx.t_or("remoteConfig.applyTemplate", "Apply"));
        apply.set_valign(gtk::Align::Center);
        apply.add_css_class("suggested-action");
        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let values = template.values.clone();
            apply.connect_clicked(move |_| {
                if crate::user_templates::is_categorized(&values) {
                    if let Some(name) = ctx.selected_remote.borrow().clone() {
                        let applied = crate::user_templates::apply_if_categorized(
                            ctx.store.borrow_mut().remotes.get_mut(&name),
                            &values,
                            true,
                        );
                        if applied > 0 {
                            ctx.persist();
                        }
                    }
                    let toast = adw::AlertDialog::new(
                        Some(&ctx.t_or("templates.applySuccess", "Template applied")),
                        Some(&ctx.t_or(
                            "templates.applySuccess",
                            "Helper and operation profiles were updated from this template.",
                        )),
                    );
                    toast.add_response("ok", &ctx.t("common.ok"));
                    toast.present(Some(&parent));
                    return;
                }
                if let Some(client) = ctx.client() {
                    match client.options_set(values.clone()) {
                        Ok(_) => {
                            let toast = adw::AlertDialog::new(
                                Some(&ctx.t_or("templates.applySuccess", "Template applied")),
                                Some(&ctx.t_or(
                                    "templates.applySuccess",
                                    "Current rclone options were updated from this template.",
                                )),
                            );
                            toast.add_response("ok", &ctx.t("common.ok"));
                            toast.present(Some(&parent));
                        }
                        Err(e) => {
                            let err = adw::AlertDialog::new(
                                Some(&ctx.t_or("common.error", "Apply failed")),
                                Some(&e.to_string()),
                            );
                            err.add_response("ok", &ctx.t("common.ok"));
                            err.present(Some(&parent));
                        }
                    }
                } else if let Some(name) = ctx.selected_remote.borrow().clone() {
                    if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&name) {
                        for profiles in meta.profiles.values_mut() {
                            for profile in profiles.values_mut() {
                                merge_template_into(&mut profile.rclone, &values);
                            }
                        }
                    }
                    ctx.persist();
                }
            });
        }
        {
            let ctx = ctx.clone();
            let id = template.id.clone();
            let parent = parent.clone();
            delete.connect_clicked(move |_| {
                ctx.store.borrow_mut().templates.retain(|t| t.id != id);
                ctx.persist();
                templates(&parent, ctx.clone());
            });
        }
        row.add_suffix(&apply);
        row.add_suffix(&delete);
        list.append(&row);
    }
    if list.first_child().is_none() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("templates.noUserTemplates", "No saved templates"));
        list.append(&row);
    }
    let add = gtk::Button::with_label(
        &ctx.t_or("templates.saveAsTemplate", "Save current flags as template"),
    );
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add.connect_clicked(move |_| {
            capture_template(&parent, ctx.clone());
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&scrolled_list(&list));
    box_.append(&add);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn archive_create(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    names: &[String],
) {
    if try_spawn_standalone(
        &ctx,
        "archive-create",
        serde_json::json!({ "remote": remote, "path": path, "name": names.join(",") }),
    ) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("nautilus.modals.archiveCreate.title", "Create archive"));
    let default_name = if names.len() == 1 {
        format!("{}.zip", names[0])
    } else {
        "archive.zip".into()
    };
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or(
        "nautilus.modals.archiveCreate.filenameLabel",
        "Archive name",
    ));
    name.set_text(&default_name);
    let formats = ["zip", "tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst"];
    let format = adw::ComboRow::new();
    format.set_title(&ctx.t_or("remoteConfig.archiveFormat", "Format"));
    format.set_model(Some(&gtk::StringList::new(&formats)));
    let prefix = adw::EntryRow::new();
    prefix.set_title(&ctx.t_or("remoteConfig.archivePrefix", "Prefix"));
    let full_path = adw::SwitchRow::new();
    full_path.set_title(&ctx.t_or(
        "remoteConfig.archiveFullPath",
        "Use full paths in the archive",
    ));
    let start =
        gtk::Button::with_label(&ctx.t_or("nautilus.modals.archiveCreate.compress", "Create"));
    start.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let folder = path.to_string();
        let names = names.to_vec();
        let name = name.clone();
        let format = format.clone();
        let prefix = prefix.clone();
        let full_path = full_path.clone();
        start.connect_clicked(move |_| {
            let mut dest_name = name.text().to_string();
            if dest_name.is_empty() {
                dest_name = "archive.zip".into();
            }
            let format_name = formats
                .get(format.selected() as usize)
                .copied()
                .unwrap_or("zip");
            if !crate::jobs::has_archive_extension(&dest_name) {
                dest_name = format!("{dest_name}.{format_name}");
            }
            let source = if names.len() == 1 {
                rclone_item_path(&remote, &folder, Some(&names[0]))
            } else {
                rclone_item_path(&remote, &folder, None)
            };
            let destination = rclone_item_path(&remote, &folder, Some(&dest_name));
            let include = if names.len() > 1 {
                names.clone()
            } else {
                Vec::new()
            };
            let opts = crate::rclone::ArchiveCreateOpts {
                src: source,
                dst: destination,
                format: Some(format_name.to_string()),
                prefix: {
                    let text = prefix.text().to_string();
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text)
                    }
                },
                full_path: full_path.is_active(),
                include,
            };
            let Some(client) = ctx.client() else {
                return;
            };
            match client.archive_create(&opts) {
                Ok(id) => {
                    ctx.store.borrow_mut().job_meta.insert(
                        id,
                        crate::jobs::job_meta_for(
                            &remote,
                            &crate::store::ProfileConfig::default(),
                            "file-manager",
                            &ctx.backend_key(),
                            "",
                        ),
                    );
                    ctx.store
                        .borrow_mut()
                        .push_log(&remote, format!("archive job #{id} queued"));
                    ctx.refresh_runtime();
                    dialog.close();
                }
                Err(e) => {
                    let err = adw::AlertDialog::new(
                        Some(&ctx.t_or("nautilus.errors.archiveCreateFailed", "Archive failed")),
                        Some(&e.to_string()),
                    );
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                }
            }
        });
    }
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&format);
    group.add(&prefix);
    group.add(&full_path);
    page.add(&group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&page);
    box_.append(&start);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn copy_url_into(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("nautilus.modals.copyUrl.title", "Copy URL"));
    let url = adw::EntryRow::new();
    url.set_title(&ctx.t_or("nautilus.modals.copyUrl.urlLabel", "URL"));
    url.set_text("https://");
    let filename = adw::EntryRow::new();
    filename.set_title(&ctx.t_or("nautilus.modals.copyUrl.fileLabel", "Filename (optional)"));
    filename.set_tooltip_text(Some(&ctx.t_or(
        "nautilus.modals.copyUrl.filePlaceholder",
        "Leave empty to use the automatic name",
    )));
    let start = gtk::Button::with_label(&ctx.t_or("nautilus.modals.copyUrl.confirm", "Download"));
    start.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let folder = path.to_string();
        let url = url.clone();
        let filename = filename.clone();
        start.connect_clicked(move |_| {
            let url_text = url.text().to_string();
            if url_text.trim().is_empty() {
                return;
            }
            let file = filename.text().to_string();
            let auto_filename = file.trim().is_empty();
            let dest_remote = if auto_filename {
                folder.clone()
            } else {
                crate::rclone::join_remote_path(&folder, file.trim())
            };
            let dest_remote = if remote == "local" {
                dest_remote.trim_start_matches('/').to_string()
            } else {
                dest_remote
            };
            let Some(client) = ctx.client() else {
                return;
            };
            let (fs, _) = if remote == "local" {
                ("/".to_string(), dest_remote.clone())
            } else {
                (remote_fs(&remote, ""), dest_remote.clone())
            };
            match client.copy_url(&url_text, &fs, &dest_remote, auto_filename) {
                Ok(id) => {
                    ctx.store.borrow_mut().job_meta.insert(
                        id,
                        crate::jobs::job_meta_for(
                            &remote,
                            &crate::store::ProfileConfig::default(),
                            "file-manager",
                            &ctx.backend_key(),
                            "",
                        ),
                    );
                    ctx.store
                        .borrow_mut()
                        .push_log(&remote, format!("copyurl job #{id} queued"));
                    ctx.refresh_runtime();
                    on_done();
                    dialog.close();
                }
                Err(e) => {
                    let err = adw::AlertDialog::new(
                        Some(&ctx.t_or("nautilus.errors.copyUrlFailed", "Copy URL failed")),
                        Some(&e.to_string()),
                    );
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                }
            }
        });
    }
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.add(&url);
    group.add(&filename);
    page.add(&group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&page);
    box_.append(&start);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn rclone_item_path(remote: &str, folder: &str, name: Option<&str>) -> String {
    let path = match name {
        Some(name) if !name.is_empty() => crate::rclone::join_remote_path(folder, name),
        _ => folder.to_string(),
    };
    if remote == "local" {
        if path.is_empty() {
            "/".into()
        } else {
            path
        }
    } else if path.is_empty() {
        format!("{remote}:")
    } else {
        format!("{remote}:{path}")
    }
}

pub fn install_rclone_update(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
) {
    start_rclone_update(parent, ctx, toast);
}

pub fn helper_profiles(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!(
        "{} — {remote}",
        ctx.t_or(
            "general.remoteConfig.advancedProfiles.title",
            "Helper profiles"
        )
    ));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let kind = adw::ComboRow::new();
    kind.set_title(&ctx.t_or("common.category", "Category"));
    kind.set_model(Some(&gtk::StringList::new(&[
        "vfs", "filter", "backend", "runtime",
    ])));
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("wizards.cliImport.profileName", "Profile name"));
    name.set_text("default");
    let json_view = gtk::TextView::new();
    json_view.set_monospace(true);
    json_view.set_wrap_mode(gtk::WrapMode::Word);
    let load = {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let name = name.clone();
        let json_view = json_view.clone();
        let kind = kind.clone();
        move || {
            let kind_name = ["vfs", "filter", "backend", "runtime"]
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("vfs");
            let text = ctx
                .store
                .borrow()
                .remotes
                .get(&remote)
                .and_then(|m| m.helper_profile(kind_name, &name.text()))
                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()))
                .unwrap_or_else(|| "{}".into());
            json_view.buffer().set_text(&text);
        }
    };
    load();
    {
        let load = load.clone();
        kind.connect_selected_notify(move |_| load());
    }
    let save =
        gtk::Button::with_label(&ctx.t_or("general.remoteConfig.saveProfile", "Save profile"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let name = name.clone();
        let json_view = json_view.clone();
        let kind = kind.clone();
        save.connect_clicked(move |_| {
            let kind_name = ["vfs", "filter", "backend", "runtime"]
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("vfs");
            let buffer = json_view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let value = serde_json::from_str::<serde_json::Value>(text.as_str())
                .unwrap_or(serde_json::json!({}));
            let key = name.text().to_string();
            if key.is_empty() {
                return;
            }
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                match kind_name {
                    "filter" => {
                        meta.filter_configs.insert(key, value);
                    }
                    "backend" => {
                        meta.backend_configs.insert(key, value);
                    }
                    "runtime" => {
                        meta.runtime_remote_configs.insert(key, value);
                    }
                    _ => {
                        meta.vfs_configs.insert(key, value);
                    }
                }
            }
            ctx.persist();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&kind);
    group.add(&name);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_min_content_height(240);
    scroll.set_child(Some(&json_view));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&scroll);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn item_order(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("titlebar.menu.remoteOrder", "Remote order and visibility"));
    dialog.set_content_width(480);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let all_names: Vec<String> = ctx
        .snapshot
        .borrow()
        .remotes
        .iter()
        .map(|r| r.name.clone())
        .collect();
    ctx.store.borrow_mut().ensure_remote_order(&all_names);
    let names = Rc::new(RefCell::new(ctx.store.borrow().remote_order.clone()));
    let hidden = Rc::new(RefCell::new(ctx.store.borrow().hidden_remotes.clone()));
    fn refill(
        list: &gtk::ListBox,
        names: &Rc<RefCell<Vec<String>>>,
        hidden: &Rc<RefCell<Vec<String>>>,
    ) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let current = names.borrow().clone();
        for (idx, name) in current.iter().enumerate() {
            let row = adw::SwitchRow::new();
            row.set_title(name);
            row.set_subtitle("Visible in sidebar and overview");
            row.set_active(!hidden.borrow().iter().any(|n| n == name));
            {
                let hidden = hidden.clone();
                let name = name.clone();
                row.connect_active_notify(move |row| {
                    let mut hidden = hidden.borrow_mut();
                    hidden.retain(|n| n != &name);
                    if !row.is_active() {
                        hidden.push(name.clone());
                    }
                });
            }
            let up = gtk::Button::from_icon_name("go-up-symbolic");
            up.set_valign(gtk::Align::Center);
            up.set_sensitive(idx > 0);
            let down = gtk::Button::from_icon_name("go-down-symbolic");
            down.set_valign(gtk::Align::Center);
            down.set_sensitive(idx + 1 < current.len());
            {
                let names = names.clone();
                let hidden = hidden.clone();
                let list = list.clone();
                let idx = idx;
                up.connect_clicked(move |_| {
                    {
                        let mut names = names.borrow_mut();
                        if idx > 0 {
                            names.swap(idx, idx - 1);
                        }
                    }
                    refill(&list, &names, &hidden);
                });
            }
            {
                let names = names.clone();
                let hidden = hidden.clone();
                let list = list.clone();
                let idx = idx;
                down.connect_clicked(move |_| {
                    {
                        let mut names = names.borrow_mut();
                        if idx + 1 < names.len() {
                            names.swap(idx, idx + 1);
                        }
                    }
                    refill(&list, &names, &hidden);
                });
            }
            row.add_suffix(&up);
            row.add_suffix(&down);
            list.append(&row);
        }
    }
    refill(&list, &names, &hidden);
    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let names = names.clone();
        let hidden = hidden.clone();
        save.connect_clicked(move |_| {
            ctx.store.borrow_mut().remote_order = names.borrow().clone();
            ctx.store.borrow_mut().hidden_remotes = hidden.borrow().clone();
            ctx.persist();
            ctx.refresh_runtime();
            on_done();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&scrolled_list(&list));
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn configure_sidebar(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("nautilus.sidebar.configureTitle", "Configure Sidebar Items"));
    dialog.set_content_width(480);
    dialog.set_content_height(520);
    let hint = gtk::Label::new(Some(&ctx.t_or(
        "nautilus.sidebar.configureDescription",
        "Drag items to reorder them, or toggle the eye icon to hide or show drives and cloud remotes in the sidebar.",
    )));
    hint.set_wrap(true);
    hint.add_css_class("dim-label");
    hint.set_xalign(0.0);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let mut ids: Vec<String> = ctx.snapshot.borrow().local_disks.clone();
    ids.extend(
        ctx.snapshot
            .borrow()
            .remotes
            .iter()
            .map(|r| format!("{}:", r.name)),
    );
    let order = ctx.settings.borrow().nautilus.sidebar_drive_order.clone();
    ids = crate::settings::sort_sidebar_ids(ids, &order);
    let names = Rc::new(RefCell::new(ids));
    let hidden = Rc::new(RefCell::new(
        ctx.settings.borrow().nautilus.sidebar_hidden_drives.clone(),
    ));
    fn refill(
        list: &gtk::ListBox,
        names: &Rc<RefCell<Vec<String>>>,
        hidden: &Rc<RefCell<Vec<String>>>,
    ) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let current = names.borrow().clone();
        for (idx, name) in current.iter().enumerate() {
            let row = adw::SwitchRow::new();
            row.set_title(name);
            row.set_active(!hidden.borrow().iter().any(|n| n == name));
            {
                let hidden = hidden.clone();
                let name = name.clone();
                row.connect_active_notify(move |row| {
                    let mut hidden = hidden.borrow_mut();
                    hidden.retain(|n| n != &name);
                    if !row.is_active() {
                        hidden.push(name.clone());
                    }
                });
            }
            let up = gtk::Button::from_icon_name("go-up-symbolic");
            up.set_valign(gtk::Align::Center);
            up.set_sensitive(idx > 0);
            let down = gtk::Button::from_icon_name("go-down-symbolic");
            down.set_valign(gtk::Align::Center);
            down.set_sensitive(idx + 1 < current.len());
            {
                let names = names.clone();
                let hidden = hidden.clone();
                let list = list.clone();
                up.connect_clicked(move |_| {
                    {
                        let mut names = names.borrow_mut();
                        if idx > 0 {
                            names.swap(idx, idx - 1);
                        }
                    }
                    refill(&list, &names, &hidden);
                });
            }
            {
                let names = names.clone();
                let hidden = hidden.clone();
                let list = list.clone();
                down.connect_clicked(move |_| {
                    {
                        let mut names = names.borrow_mut();
                        if idx + 1 < names.len() {
                            names.swap(idx, idx + 1);
                        }
                    }
                    refill(&list, &names, &hidden);
                });
            }
            row.add_suffix(&up);
            row.add_suffix(&down);
            list.append(&row);
        }
    }
    refill(&list, &names, &hidden);
    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        save.connect_clicked(move |_| {
            ctx.settings.borrow_mut().nautilus.sidebar_drive_order = names.borrow().clone();
            ctx.settings.borrow_mut().nautilus.sidebar_hidden_drives = hidden.borrow().clone();
            ctx.persist();
            on_done();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_top(8);
    box_.set_margin_bottom(12);
    box_.append(&hint);
    box_.append(&scrolled_list(&list));
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn scrolled_list(list: &gtk::ListBox) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(list));
    scroll
}

fn operation_flag_blocks(ctx: &AppCtx) -> Vec<crate::flags::FlagBlock> {
    ctx.client()
        .and_then(|c| c.options_info().ok())
        .map(|info| crate::flags::parse_options_info(&info))
        .unwrap_or_default()
}

fn flag_value_row(flag: &crate::flags::FlagOption, rclone: &serde_json::Value) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&flag.name);
    if !flag.help.is_empty() {
        row.set_tooltip_text(Some(&flag.help));
    }
    let current = rclone
        .get(&flag.field_name)
        .map(|v| v.to_string().trim_matches('"').to_string())
        .unwrap_or_else(|| flag.default_str.clone());
    row.set_text(&current);
    row
}

fn attach_cli_import(
    ctx: &AppCtx,
    flags_group: &adw::PreferencesGroup,
    flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>>,
) {
    let cli = adw::EntryRow::new();
    cli.set_title(&ctx.t_or("wizards.cliImport.placeholder", "Import rclone CLI flags"));
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
                    row.set_text(&value.to_string().trim_matches('"').to_string());
                }
            }
        });
    }
    let cli_row = adw::ActionRow::new();
    cli_row.set_title(&ctx.t_or("wizards.cliImport.title", "CLI import"));
    cli_row.add_suffix(&apply_cli);
    flags_group.add(&cli);
    flags_group.add(&cli_row);
}

fn clear_flag_rows(
    flags_group: &adw::PreferencesGroup,
    flag_rows: &Rc<RefCell<Vec<(String, adw::EntryRow, String)>>>,
) {
    for (_, row, _) in flag_rows.borrow().iter() {
        flags_group.remove(row);
    }
    flag_rows.borrow_mut().clear();
}

fn clear_serve_flag_rows(
    flags_group: &adw::PreferencesGroup,
    serve_flag_rows: &Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>>,
) {
    for (_, _, row, _) in serve_flag_rows.borrow().iter() {
        flags_group.remove(row);
    }
    serve_flag_rows.borrow_mut().clear();
}

fn populate_flag_rows(
    flags_group: &adw::PreferencesGroup,
    flag_rows: &Rc<RefCell<Vec<(String, adw::EntryRow, String)>>>,
    op: OperationType,
    rclone: &serde_json::Value,
    blocks: &[crate::flags::FlagBlock],
) {
    for flag in crate::flags::merged_flags_for(op, blocks) {
        if op == OperationType::Serve && flag.field_name == "type" {
            continue;
        }
        let row = flag_value_row(&flag, rclone);
        flags_group.add(&row);
        flag_rows
            .borrow_mut()
            .push((flag.field_name, row, flag.type_name));
    }
}

fn populate_serve_flag_rows(
    flags_group: &adw::PreferencesGroup,
    serve_flag_rows: &Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>>,
    blocks: &[crate::flags::FlagBlock],
    rclone: &serde_json::Value,
    serve: &adw::ComboRow,
    serve_types: &[String],
) {
    for serve_type in serve_types {
        for flag in crate::flags::collect_serve_flags(blocks, serve_type) {
            let row = flag_value_row(&flag, rclone);
            row.set_title(&format!("{serve_type} · {}", flag.name));
            let selected = crate::operations::selected_or(serve_types, serve.selected(), "http");
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
    let serve_flag_rows = serve_flag_rows.clone();
    let serve_types = serve_types.to_vec();
    serve.connect_selected_notify(move |row| {
        let selected = crate::operations::selected_or(&serve_types, row.selected(), "http");
        for (serve_type, _, widget, _) in serve_flag_rows.borrow().iter() {
            widget.set_visible(serve_type == selected);
        }
    });
}

pub(super) fn check_result_row(
    ctx: &AppCtx,
    item: &crate::checks::CheckResult,
    parent: &impl IsA<gtk::Widget>,
) -> gtk::Box {
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let row = adw::ActionRow::new();
    row.set_title(&item.name);
    row.set_subtitle(&item.status);
    if let Some(kind) = item.resolve_kind() {
        let resolve_label = if item.needs_overwrite_confirm() {
            ctx.t_or(
                "shared.transferActivity.actions.overwriteDst",
                "Overwrite Destination",
            )
        } else if kind == "copy_dst_to_src" {
            ctx.t_or(
                "shared.transferActivity.actions.resolveToSrc",
                "Copy to Source",
            )
        } else {
            ctx.t_or(
                "shared.transferActivity.actions.resolveToDst",
                "Copy to Destination",
            )
        };
        let resolve = gtk::Button::with_label(&resolve_label);
        resolve.set_valign(gtk::Align::Center);
        let ctx = ctx.clone();
        let item = item.clone();
        let parent = parent.clone();
        resolve.connect_clicked(move |_| {
            let go = || resolve_check_item(&ctx, &item, kind);
            if item.needs_overwrite_confirm() {
                let title = ctx.t_or(
                    "shared.transferActivity.actions.resolveOverwriteTitle",
                    "Resolve File Mismatch",
                );
                let message = ctx.tf(
                    "shared.transferActivity.actions.resolveOverwriteMessage",
                    &[("name", &item.name)],
                );
                let confirm = adw::AlertDialog::new(Some(&title), Some(&message));
                confirm.add_response("cancel", &ctx.t_or("common.cancel", "Cancel"));
                confirm.add_response(
                    "ok",
                    &ctx.t_or(
                        "shared.transferActivity.actions.overwriteDst",
                        "Overwrite Destination",
                    ),
                );
                confirm.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
                let ctx = ctx.clone();
                let item = item.clone();
                confirm.connect_response(None, move |_, response| {
                    if response == "ok" {
                        resolve_check_item(&ctx, &item, kind);
                    }
                });
                confirm.present(Some(&parent));
            } else {
                go();
            }
        });
        row.add_suffix(&resolve);
    }
    let parsed = crate::transfers::TransferRow {
        name: item.name.clone(),
        src: crate::transfers::join_fs_name(&item.src_fs, &item.name),
        dst: crate::transfers::join_fs_name(&item.dst_fs, &item.name),
        percentage: 0,
        size: 0,
    };
    row.add_suffix(&transfer_row_actions(ctx, parent, &parsed, "copy", true));
    wrap.append(&row);
    let resolve_job = {
        let snap = ctx.snapshot.borrow();
        let meta = ctx.store.borrow();
        crate::jobs::find_resolve_job(&snap.jobs, &meta.job_meta, &item.name)
            .or_else(|| {
                crate::jobs::find_resolve_job(&meta.job_history, &meta.job_meta, &item.name)
            })
            .cloned()
    };
    if let Some(job) = resolve_job {
        if crate::jobs::job_is_running(&job) {
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(job.progress.clamp(0.0, 1.0));
            bar.set_show_text(true);
            let speed = crate::jobs::stats_f64(&job.stats, &["speed"]);
            bar.set_text(Some(&format!(
                "{:.0}% · {}/s",
                job.progress * 100.0,
                crate::rclone::format_bytes(speed as i64)
            )));
            wrap.append(&bar);
        } else if job.status == "failed" || job.error.is_some() {
            let err = gtk::Label::new(Some(&job.error.unwrap_or_else(|| job.status.clone())));
            err.add_css_class("error");
            err.set_xalign(0.0);
            err.set_wrap(true);
            wrap.append(&err);
        }
    }
    wrap
}

fn resolve_check_item(ctx: &AppCtx, item: &crate::checks::CheckResult, kind: &str) {
    let Some(client) = ctx.client() else {
        return;
    };
    let (src_fs, dst_fs) = if kind == "copy_dst_to_src" {
        (item.dst_fs.as_str(), item.src_fs.as_str())
    } else {
        (item.src_fs.as_str(), item.dst_fs.as_str())
    };
    let payload = serde_json::json!({
        "srcFs": src_fs,
        "srcRemote": item.name,
        "dstFs": dst_fs,
        "dstRemote": item.name,
    });
    match client.start_job("operations/copyfile", payload) {
        Ok(id) => {
            let (remote, _) = crate::rclone::split_remote_path(src_fs);
            crate::jobs::remember_started(
                &mut ctx.store.borrow_mut().job_meta,
                &format!("#{id}"),
                crate::store::JobMeta {
                    origin: "check-resolve".into(),
                    profile: "default".into(),
                    remote,
                    backend: ctx.backend_key(),
                    quick_run_id: String::new(),
                    execute_id: uuid::Uuid::new_v4().to_string(),
                    parent_job_id: None,
                    target: item.name.clone(),
                },
            );
            ctx.persist();
            ctx.refresh_runtime();
        }
        Err(e) => {
            if let Err(fallback) = client.copy_file(src_fs, &item.name, dst_fs, &item.name) {
                log::warn!("check resolve failed: {e}; fallback {fallback}");
            } else {
                ctx.refresh_runtime();
            }
        }
    }
}

fn capture_template(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("templates.saveAsTemplate", "Capture template"));
    dialog.set_content_width(420);
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("templates.templateName", "Name"));
    name.set_text(&format!(
        "Template {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));
    let categories = [
        (
            "backend",
            ctx.t_or("general.remoteConfig.steps.backend", "Backend / main"),
        ),
        (
            "filter",
            ctx.t_or("general.remoteConfig.steps.filter", "Filter"),
        ),
        ("vfs", ctx.t_or("general.remoteConfig.steps.vfs", "VFS")),
        (
            "mount",
            ctx.t_or("general.remoteConfig.steps.mount", "Mount"),
        ),
        ("copy", ctx.t_or("general.remoteConfig.steps.copy", "Copy")),
        ("sync", ctx.t_or("general.remoteConfig.steps.sync", "Sync")),
        (
            "check",
            ctx.t_or("general.remoteConfig.steps.check", "Check"),
        ),
        (
            "network",
            ctx.t_or(
                "settings.rcloneFlags.mainCategories.network.title",
                "Network",
            ),
        ),
        ("other", ctx.t_or("common.other", "Other")),
    ];
    let switches: Vec<(&'static str, adw::SwitchRow)> = categories
        .iter()
        .map(|(id, label)| {
            let row = adw::SwitchRow::new();
            row.set_title(label);
            row.set_active(true);
            (*id, row)
        })
        .collect();
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("templates.selectAllKeys", "Categories to include"));
    group.add(&name);
    for (_, row) in &switches {
        group.add(row);
    }
    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let dialog = dialog.clone();
        let name = name.clone();
        let switches = switches.clone();
        save.connect_clicked(move |_| {
            let selected: Vec<&str> = switches
                .iter()
                .filter(|(_, row)| row.is_active())
                .map(|(id, _)| *id)
                .collect();
            let current = ctx
                .client()
                .and_then(|c| c.options_get().ok())
                .unwrap_or(serde_json::json!({}));
            let blocks = operation_flag_blocks(&ctx);
            let values = crate::flags::filter_options_for_categories(&current, &blocks, &selected);
            ctx.store
                .borrow_mut()
                .templates
                .push(crate::store::UserTemplate {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.text().to_string(),
                    description: format!("Captured {} categories", selected.len()),
                    icon: "emblem-ok-symbolic".into(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    values,
                });
            ctx.persist();
            dialog.close();
            templates(&parent, ctx.clone());
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub(crate) fn present_window_or_dialog(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    dialog: &adw::Dialog,
) {
    if ctx.settings.borrow().general.standalone_dialogs {
        let win = adw::Window::new();
        let title = dialog.title();
        win.set_title(Some(title.as_str()));
        let width = dialog.content_width();
        let height = dialog.content_height();
        if width > 0 {
            win.set_default_width(width);
        }
        if height > 0 {
            win.set_default_height(height);
        }
        if let Some(child) = dialog.child() {
            dialog.set_child(gtk::Widget::NONE);
            win.set_content(Some(&child));
        }
        if let Some(p) = parent.root().and_downcast::<gtk::Window>() {
            win.set_transient_for(Some(&p));
        }
        win.present();
    } else {
        dialog.present(Some(parent));
    }
}

fn qr_helper_names(ctx: &AppCtx, remote: &str, kind: &str) -> Vec<String> {
    let mut names = vec!["—".into()];
    if let Some(meta) = ctx.store.borrow().remotes.get(remote) {
        names.extend(meta.helper_names(kind));
    }
    names
}

fn refresh_helper_combo(row: &adw::ComboRow, names: &[String], selected: &str) {
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    row.set_model(Some(&gtk::StringList::new(&refs)));
    if let Some(idx) = names.iter().position(|n| n == selected && !n.is_empty()) {
        row.set_selected(idx as u32);
    } else {
        row.set_selected(0);
    }
}

fn populate_helper_flag_rows(
    ctx: &AppCtx,
    group: &adw::PreferencesGroup,
    rows: &Rc<RefCell<Vec<(String, adw::EntryRow, String)>>>,
    kind: &str,
    blocks: &[crate::flags::FlagBlock],
    remote: &str,
    selected: &str,
) {
    clear_flag_rows(group, rows);
    if selected.is_empty() {
        return;
    }
    let current = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .and_then(|meta| meta.helper_profile(kind, selected))
        .unwrap_or_else(|| serde_json::json!({}));
    for (_, option) in crate::flags::options_for_category(blocks, kind) {
        let row = flag_value_row(option, &current);
        group.add(&row);
        rows.borrow_mut()
            .push((option.field_name.clone(), row, option.type_name.clone()));
    }
}

fn save_helper_from_rows(
    ctx: &AppCtx,
    remote: &str,
    kind: &str,
    name: &str,
    rows: &[(String, adw::EntryRow, String)],
) {
    if remote.is_empty() || name.is_empty() {
        return;
    }
    let mut map = serde_json::Map::new();
    for (field, row, type_name) in rows {
        let text = row.text().to_string();
        if text.is_empty() {
            continue;
        }
        map.insert(
            field.clone(),
            crate::flags::parse_flag_value(type_name, &text),
        );
    }
    if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(remote) {
        meta.upsert_helper(kind, name, serde_json::Value::Object(map));
    }
}

pub(crate) fn helper_combo(title: &str, names: &[String], selected: &str) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    row.set_model(Some(&gtk::StringList::new(&refs)));
    if let Some(idx) = names.iter().position(|n| n == selected) {
        row.set_selected(idx as u32);
    }
    row
}

pub(crate) fn helper_selected(row: &adw::ComboRow, names: &[String]) -> String {
    names
        .get(row.selected() as usize)
        .cloned()
        .filter(|s| s != "—")
        .unwrap_or_default()
}

fn quick_run_filename_row(ctx: &AppCtx, value: &str) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    row.set_title(&ctx.t_or(
        "wizards.appOperation.copyUrlFilename",
        "Filename (optional)",
    ));
    row.set_text(value);
    row
}

fn apply_quick_run_path_titles(
    ctx: &AppCtx,
    op: OperationType,
    src: &adw::EntryRow,
    dst: &adw::EntryRow,
) {
    src.set_title(&if op == OperationType::Copyurl {
        ctx.t_or("remoteConfig.url", "URL")
    } else {
        ctx.t_or("fileBrowser.operations.details.source", "Source")
    });
    dst.set_title(&match op {
        OperationType::Mount => ctx.t_or("remoteConfig.mountPoint", "Mount point"),
        OperationType::Serve => ctx.t_or("remoteConfig.listenAddress", "Listen address"),
        OperationType::Copyurl => ctx.t_or("remoteConfig.destinationFs", "Destination fs"),
        OperationType::Delete => ctx.t_or("remoteConfig.unusedDestination", "Unused destination"),
        _ => ctx.t_or(
            "fileBrowser.operations.details.destination",
            "Destination / mount point",
        ),
    });
    dst.set_visible(op != OperationType::Delete);
}

pub(crate) fn attach_path_kind(
    ctx: &AppCtx,
    row: &adw::EntryRow,
    current_remote: &str,
) -> adw::ComboRow {
    let combo = adw::ComboRow::new();
    combo.set_title(&ctx.t_or("fileBrowser.pathKind.title", "Path type"));
    let local = ctx.t_or("fileBrowser.pathKind.local", "Local");
    let current = ctx.t_or("fileBrowser.pathKind.currentRemote", "Current remote");
    let other = ctx.t_or("fileBrowser.pathKind.otherRemote", "Other remote");
    combo.set_model(Some(&gtk::StringList::new(&[
        local.as_str(),
        current.as_str(),
        other.as_str(),
    ])));
    let remote = current_remote.to_string();
    let kind = crate::path_kind::infer_path_kind(&row.text(), &remote);
    combo.set_selected(crate::path_kind::kind_index(kind));
    {
        let row = row.clone();
        let remote = remote.clone();
        combo.connect_selected_notify(move |combo| {
            let kind = crate::path_kind::kind_from_index(combo.selected());
            let rewritten = crate::path_kind::rewrite_path_for_kind(&row.text(), &remote, kind);
            if rewritten != row.text().as_str() {
                row.set_text(&rewritten);
            }
        });
    }
    {
        let combo = combo.clone();
        let remote = remote.clone();
        row.connect_changed(move |row| {
            let kind = crate::path_kind::infer_path_kind(&row.text(), &remote);
            let idx = crate::path_kind::kind_index(kind);
            if combo.selected() != idx {
                combo.set_selected(idx);
            }
        });
    }
    combo
}

pub(crate) fn attach_cron_presets(cron: &adw::EntryRow, ctx: &AppCtx) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    box_.add_css_class("linked");
    box_.set_hexpand(true);
    for preset in crate::cron::PRESETS {
        let key = format!("flow.cronPresets.{}.title", preset.key);
        let label = ctx.t_or(&key, preset.label);
        let btn = gtk::Button::with_label(&label);
        btn.set_tooltip_text(Some(preset.cron));
        let cron = cron.clone();
        btn.connect_clicked(move |_| {
            cron.set_text(preset.cron);
        });
        box_.append(&btn);
    }
    box_
}

pub(crate) fn attach_cron_builder(cron: &adw::EntryRow, ctx: &AppCtx) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&attach_cron_presets(cron, ctx));

    let simple = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let daily = ctx.t_or("remoteConfig.cronParts.daily", "Daily");
    let weekly = ctx.t_or("remoteConfig.cronParts.weekly", "Weekly");
    let monthly = ctx.t_or("remoteConfig.cronParts.monthly", "Monthly");
    let interval_l = ctx.t_or("remoteConfig.cronParts.interval", "Interval");
    let freq_labels = [
        daily.as_str(),
        weekly.as_str(),
        monthly.as_str(),
        interval_l.as_str(),
    ];
    let freq = gtk::DropDown::from_strings(&freq_labels);
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    hour.set_value(9.0);
    hour.set_tooltip_text(Some(&ctx.t_or("remoteConfig.cronParts.hour", "Hour")));
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    minute.set_tooltip_text(Some(&ctx.t_or("remoteConfig.cronParts.minute", "Minute")));
    let dow = gtk::Entry::new();
    dow.set_placeholder_text(Some("1-5"));
    dow.set_text("1");
    dow.set_width_chars(5);
    dow.set_tooltip_text(Some(&ctx.t_or("remoteConfig.cronParts.dow", "Day of week")));
    let dom = gtk::SpinButton::with_range(1.0, 31.0, 1.0);
    dom.set_tooltip_text(Some(
        &ctx.t_or("remoteConfig.cronParts.dom", "Day of month"),
    ));
    let interval = gtk::SpinButton::with_range(1.0, 24.0, 1.0);
    interval.set_value(6.0);
    interval.set_tooltip_text(Some(
        &ctx.t_or("remoteConfig.cronParts.everyHours", "Every N hours"),
    ));
    if let Some(parsed) = crate::cron::parse_simple(&cron.text()) {
        freq.set_selected(match parsed.frequency {
            crate::cron::SimpleFrequency::Weekly => 1,
            crate::cron::SimpleFrequency::Monthly => 2,
            crate::cron::SimpleFrequency::Interval => 3,
            crate::cron::SimpleFrequency::Daily => 0,
        });
        hour.set_value(parsed.hour as f64);
        minute.set_value(parsed.minute as f64);
        dow.set_text(&parsed.day_of_week);
        dom.set_value(parsed.day_of_month as f64);
        interval.set_value(parsed.interval_hours as f64);
    }
    let apply_simple = {
        let cron = cron.clone();
        let freq = freq.clone();
        let hour = hour.clone();
        let minute = minute.clone();
        let dow = dow.clone();
        let dom = dom.clone();
        let interval = interval.clone();
        Rc::new(move || {
            let simple = crate::cron::SimpleCron {
                frequency: match freq.selected() {
                    1 => crate::cron::SimpleFrequency::Weekly,
                    2 => crate::cron::SimpleFrequency::Monthly,
                    3 => crate::cron::SimpleFrequency::Interval,
                    _ => crate::cron::SimpleFrequency::Daily,
                },
                minute: minute.value() as u32,
                hour: hour.value() as u32,
                day_of_week: dow.text().to_string(),
                day_of_month: dom.value() as u32,
                interval_hours: interval.value().max(1.0) as u32,
            };
            cron.set_text(&crate::cron::build_simple(&simple));
        })
    };
    {
        let apply = apply_simple.clone();
        freq.connect_selected_notify(move |_| apply());
    }
    {
        let apply = apply_simple.clone();
        hour.connect_value_changed(move |_| apply());
    }
    {
        let apply = apply_simple.clone();
        minute.connect_value_changed(move |_| apply());
    }
    {
        let apply = apply_simple.clone();
        dow.connect_changed(move |_| apply());
    }
    {
        let apply = apply_simple.clone();
        dom.connect_value_changed(move |_| apply());
    }
    {
        let apply = apply_simple.clone();
        interval.connect_value_changed(move |_| apply());
    }
    {
        let dow = dow.clone();
        let dom = dom.clone();
        let interval = interval.clone();
        let hour = hour.clone();
        let minute = minute.clone();
        freq.connect_selected_notify(move |freq| {
            let weekly = freq.selected() == 1;
            let monthly = freq.selected() == 2;
            let every = freq.selected() == 3;
            dow.set_visible(weekly);
            dom.set_visible(monthly);
            interval.set_visible(every);
            hour.set_visible(!every);
            minute.set_visible(!every);
        });
    }
    dow.set_visible(false);
    dom.set_visible(false);
    interval.set_visible(false);
    simple.append(&freq);
    simple.append(&hour);
    simple.append(&minute);
    simple.append(&dow);
    simple.append(&dom);
    simple.append(&interval);
    outer.append(&simple);

    let advanced = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let minute_f = gtk::Entry::new();
    minute_f.set_placeholder_text(Some("min"));
    minute_f.set_text("0");
    minute_f.set_width_chars(4);
    let hour_f = gtk::Entry::new();
    hour_f.set_placeholder_text(Some("hour"));
    hour_f.set_text("*");
    hour_f.set_width_chars(4);
    let dom_f = gtk::Entry::new();
    dom_f.set_placeholder_text(Some("dom"));
    dom_f.set_text("*");
    dom_f.set_width_chars(4);
    let month_f = gtk::Entry::new();
    month_f.set_placeholder_text(Some("mon"));
    month_f.set_text("*");
    month_f.set_width_chars(4);
    let dow_f = gtk::Entry::new();
    dow_f.set_placeholder_text(Some("dow"));
    dow_f.set_text("*");
    dow_f.set_width_chars(4);
    if let Some([min, hr, d, mon, dw]) = crate::cron::split_cron(&cron.text()) {
        minute_f.set_text(&min);
        hour_f.set_text(&hr);
        dom_f.set_text(&d);
        month_f.set_text(&mon);
        dow_f.set_text(&dw);
    }
    let apply_adv = {
        let cron = cron.clone();
        let minute_f = minute_f.clone();
        let hour_f = hour_f.clone();
        let dom_f = dom_f.clone();
        let month_f = month_f.clone();
        let dow_f = dow_f.clone();
        Rc::new(move || {
            cron.set_text(&crate::cron::build_advanced(
                &minute_f.text(),
                &hour_f.text(),
                &dom_f.text(),
                &month_f.text(),
                &dow_f.text(),
            ));
        })
    };
    for field in [&minute_f, &hour_f, &dom_f, &month_f, &dow_f] {
        let apply = apply_adv.clone();
        field.connect_changed(move |_| apply());
        advanced.append(field);
    }
    outer.append(&advanced);
    let _ = apply_simple;
    outer
}

pub(crate) fn attach_path_picker(
    ctx: &AppCtx,
    row: &adw::EntryRow,
    config: crate::picker::FilePickerConfig,
) {
    let btn = gtk::Button::from_icon_name("folder-open-symbolic");
    btn.set_valign(gtk::Align::Center);
    btn.set_tooltip_text(Some("Browse"));
    let ctx = ctx.clone();
    let picked = row.clone();
    btn.connect_clicked(move |_| {
        let mut config = config.clone();
        if config.initial_location.is_none() && !picked.text().is_empty() {
            config.initial_location = Some(picked.text().to_string());
        }
        if config.mode == crate::picker::PickerMode::Local {
            let Some(win) = picked.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let dialog = gtk::FileDialog::new();
            let row = picked.clone();
            if config.selection == crate::picker::PickerSelection::Files {
                dialog.open(
                    Some(&win),
                    None::<gio::Cancellable>.as_ref(),
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                row.set_text(&path.to_string_lossy());
                            }
                        }
                    },
                );
            } else {
                dialog.select_folder(
                    Some(&win),
                    None::<gio::Cancellable>.as_ref(),
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                row.set_text(&path.to_string_lossy());
                            }
                        }
                    },
                );
            }
            return;
        }
        let row = picked.clone();
        ctx.request_picker(
            config.clone(),
            Rc::new(move |result| {
                if !result.cancelled {
                    row.set_text(&result.formatted_path());
                }
            }),
        );
    });
    row.add_suffix(&btn);
}

fn ensure_syntax_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    for kind in [
        crate::syntax::TokenKind::Keyword,
        crate::syntax::TokenKind::String,
        crate::syntax::TokenKind::Comment,
        crate::syntax::TokenKind::Number,
    ] {
        if table.lookup(kind.tag_name()).is_none() {
            let tag = gtk::TextTag::builder()
                .name(kind.tag_name())
                .foreground(kind.color())
                .build();
            table.add(&tag);
        }
    }
}

fn paint_syntax(buffer: &gtk::TextBuffer, name: &str, text: &str) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_all_tags(&start, &end);
    let Some(lang) = crate::syntax::language_from_name(name) else {
        return;
    };
    for span in crate::syntax::highlight(text, lang) {
        let start = buffer.iter_at_offset(span.start as i32);
        let end = buffer.iter_at_offset(span.end as i32);
        buffer.apply_tag_by_name(span.kind.tag_name(), &start, &end);
    }
}

fn apply_syntax_highlight(view: &gtk::TextView, name: &str, text: &str) {
    let buffer = view.buffer();
    ensure_syntax_tags(&buffer);
    buffer.set_text(text);
    paint_syntax(&buffer, name, text);
}

fn attach_live_syntax(view: &gtk::TextView, name: &str) {
    let name = name.to_string();
    view.buffer().connect_changed(move |buffer| {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        paint_syntax(buffer, &name, text.as_str());
    });
}

const ACTION_KINDS: &[&str] = &[
    "os_toast", "webhook", "telegram", "whatsapp", "script", "email", "mqtt",
];
const EVENT_KINDS: &[AlertEventKind] = &[
    AlertEventKind::Job,
    AlertEventKind::Serve,
    AlertEventKind::Mount,
    AlertEventKind::Engine,
    AlertEventKind::Update,
    AlertEventKind::Automation,
    AlertEventKind::System,
];
const SEVERITIES: &[&str] = &["info", "warning", "average", "high", "critical"];
const ORIGINS: &[&str] = &[
    "dashboard",
    "automation",
    "filemanager",
    "startup",
    "update",
    "internal",
    "flow",
    "quickrun",
];

fn alert_rule_editor(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, existing_id: Option<String>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("alerts.rule.editorTitle", "Alert rule"));
    dialog.set_content_width(520);
    dialog.set_content_height(640);
    let existing = existing_id.as_ref().and_then(|id| {
        ctx.store
            .borrow()
            .alert_rules
            .iter()
            .find(|r| &r.id == id)
            .cloned()
    });
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("alerts.ruleName", "Name"));
    let default_rule_name = ctx.t_or("alerts.createRule", "New rule");
    name.set_text(
        existing
            .as_ref()
            .map(|r| r.name.as_str())
            .unwrap_or(&default_rule_name),
    );
    let enabled = adw::SwitchRow::new();
    enabled.set_title(&ctx.t("alerts.enabled"));
    enabled.set_active(existing.as_ref().map(|r| r.enabled).unwrap_or(true));
    let auto_ack = adw::SwitchRow::new();
    auto_ack.set_title(&ctx.t_or("alerts.rule.autoAcknowledge", "Auto-acknowledge"));
    auto_ack.set_active(
        existing
            .as_ref()
            .map(|r| r.auto_acknowledge)
            .unwrap_or(false),
    );
    let severity = adw::ComboRow::new();
    severity.set_title(&ctx.t_or("alerts.rule.severityMin", "Minimum severity"));
    severity.set_model(Some(&gtk::StringList::new(SEVERITIES)));
    if let Some(rule) = &existing {
        if let Some(idx) = SEVERITIES
            .iter()
            .position(|s| *s == rule.severity_min.as_str())
        {
            severity.set_selected(idx as u32);
        }
    }
    let cooldown = adw::EntryRow::new();
    cooldown.set_title(&ctx.t_or("alerts.rule.cooldown", "Cooldown (seconds)"));
    cooldown.set_text(
        &existing
            .as_ref()
            .map(|r| r.cooldown_secs.to_string())
            .unwrap_or_else(|| "0".into()),
    );
    let remote_names = {
        let mut names: Vec<String> = ctx
            .snapshot
            .borrow()
            .remotes
            .iter()
            .map(|r| r.name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    };
    let backend_names = {
        let mut names = vec!["local".to_string()];
        names.extend(
            ctx.settings
                .borrow()
                .core
                .extra_backends
                .iter()
                .map(|b| b.name.clone()),
        );
        names
    };
    let profile_names = {
        let mut names: Vec<String> = ctx
            .store
            .borrow()
            .remotes
            .values()
            .flat_map(|meta| {
                meta.profiles
                    .values()
                    .flat_map(|map| map.keys().cloned())
                    .collect::<Vec<_>>()
            })
            .collect();
        if !names.iter().any(|n| n == "default") {
            names.push("default".into());
        }
        names.sort();
        names.dedup();
        names
    };
    let remote_switches = filter_switch_rows(
        &remote_names,
        existing.as_ref().map(|r| r.remote_filter.as_slice()),
    );
    let backend_switches = filter_switch_rows(
        &backend_names,
        existing.as_ref().map(|r| r.backend_filter.as_slice()),
    );
    let profile_switches = filter_switch_rows(
        &profile_names,
        existing.as_ref().map(|r| r.profile_filter.as_slice()),
    );
    let origin_switches = filter_switch_rows(
        &ORIGINS.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
        existing.as_ref().map(|r| r.origin_filter.as_slice()),
    );
    let event_switches: Vec<(AlertEventKind, adw::SwitchRow)> = EVENT_KINDS
        .iter()
        .map(|kind| {
            let row = adw::SwitchRow::new();
            row.set_title(&ctx.t_or(&format!("alerts.events.{}", kind.as_str()), kind.as_str()));
            row.set_active(
                existing
                    .as_ref()
                    .map(|r| r.event_filter.is_empty() || r.event_filter.contains(kind))
                    .unwrap_or(*kind == AlertEventKind::Job),
            );
            (kind.clone(), row)
        })
        .collect();
    let action_switches: Vec<(String, adw::SwitchRow)> = ctx
        .store
        .borrow()
        .alert_actions
        .iter()
        .map(|action| {
            let row = adw::SwitchRow::new();
            row.set_title(&action.name);
            row.set_subtitle(&action.kind);
            row.set_active(
                existing
                    .as_ref()
                    .map(|r| r.action_ids.contains(&action.id))
                    .unwrap_or(false),
            );
            (action.id.clone(), row)
        })
        .collect();

    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label(&ctx.t("common.delete"));
    delete.add_css_class("destructive-action");
    delete.set_visible(existing_id.is_some());
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let event_switches = event_switches.clone();
        let action_switches = action_switches.clone();
        let existing_id = existing_id.clone();
        let name = name.clone();
        let enabled = enabled.clone();
        let auto_ack = auto_ack.clone();
        let severity = severity.clone();
        let cooldown = cooldown.clone();
        let remote_switches = remote_switches.clone();
        let backend_switches = backend_switches.clone();
        let profile_switches = profile_switches.clone();
        let origin_switches = origin_switches.clone();
        save.connect_clicked(move |_| {
            let mut rule = existing_id
                .as_ref()
                .and_then(|id| {
                    ctx.store
                        .borrow()
                        .alert_rules
                        .iter()
                        .find(|r| &r.id == id)
                        .cloned()
                })
                .unwrap_or_else(|| AlertRule::new(name.text().to_string()));
            rule.name = name.text().to_string();
            rule.enabled = enabled.is_active();
            rule.auto_acknowledge = auto_ack.is_active();
            rule.severity_min = AlertSeverity::parse(
                SEVERITIES
                    .get(severity.selected() as usize)
                    .copied()
                    .unwrap_or("info"),
            );
            rule.cooldown_secs = cooldown.text().parse().unwrap_or(0);
            rule.remote_filter = selected_or_all(&remote_switches);
            rule.backend_filter = selected_or_all(&backend_switches);
            rule.profile_filter = selected_or_all(&profile_switches);
            rule.origin_filter = selected_or_all(&origin_switches);
            rule.event_filter = event_switches
                .iter()
                .filter(|(_, row)| row.is_active())
                .map(|(kind, _)| *kind)
                .collect();
            rule.action_ids = action_switches
                .iter()
                .filter(|(_, row)| row.is_active())
                .map(|(id, _)| id.clone())
                .collect();
            {
                let mut store = ctx.store.borrow_mut();
                if let Some(idx) = store.alert_rules.iter().position(|r| r.id == rule.id) {
                    store.alert_rules[idx] = rule;
                } else {
                    store.alert_rules.push(rule);
                }
            }
            ctx.persist();
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        delete.connect_clicked(move |_| {
            if let Some(id) = &existing_id {
                ctx.store.borrow_mut().alert_rules.retain(|r| &r.id != id);
                ctx.persist();
            }
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }

    let page = adw::PreferencesPage::new();
    let general = adw::PreferencesGroup::new();
    general.set_title(&ctx.t_or("alerts.ruleLabel", "Rule"));
    general.add(&name);
    general.add(&enabled);
    general.add(&auto_ack);
    general.add(&severity);
    general.add(&cooldown);
    page.add(&general);
    let remotes_group = adw::PreferencesGroup::new();
    remotes_group.set_title(&ctx.t_or("alerts.rule.remoteFilter", "Remotes"));
    remotes_group.set_description(Some(&ctx.t_or(
        "alerts.rule.filtersEmptyHint",
        "Leave every switch on to match all remotes.",
    )));
    for (_, row) in &remote_switches {
        remotes_group.add(row);
    }
    page.add(&remotes_group);
    let backends_group = adw::PreferencesGroup::new();
    backends_group.set_title(&ctx.t_or("alerts.rule.backendFilter", "Backends"));
    for (_, row) in &backend_switches {
        backends_group.add(row);
    }
    page.add(&backends_group);
    let profiles_group = adw::PreferencesGroup::new();
    profiles_group.set_title(&ctx.t_or("alerts.rule.profileFilter", "Profiles"));
    for (_, row) in &profile_switches {
        profiles_group.add(row);
    }
    page.add(&profiles_group);
    let origins_group = adw::PreferencesGroup::new();
    origins_group.set_title(&ctx.t_or("alerts.origins.title", "Origins"));
    for (_, row) in &origin_switches {
        origins_group.add(row);
    }
    page.add(&origins_group);
    let events = adw::PreferencesGroup::new();
    events.set_title(&ctx.t_or("alerts.rule.eventFilter", "Events"));
    for (_, row) in &event_switches {
        events.add(row);
    }
    page.add(&events);
    let actions = adw::PreferencesGroup::new();
    actions.set_title(&ctx.t("alerts.actions"));
    for (_, row) in &action_switches {
        actions.add(row);
    }
    page.add(&actions);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&save);
    buttons.append(&delete);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    box_.append(&scroll);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn alert_action_editor(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, existing_id: Option<String>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("alerts.action.editorTitle", "Alert action"));
    dialog.set_content_width(520);
    dialog.set_content_height(620);
    let existing = existing_id.as_ref().and_then(|id| {
        ctx.store
            .borrow()
            .alert_actions
            .iter()
            .find(|a| &a.id == id)
            .cloned()
    });
    let name = adw::EntryRow::new();
    name.set_title(&ctx.t_or("alerts.action.placeholderName", "Name"));
    let default_action_name = ctx.t_or("alerts.createAction", "New action");
    name.set_text(
        existing
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or(&default_action_name),
    );
    let enabled = adw::SwitchRow::new();
    enabled.set_title(&ctx.t("alerts.enabled"));
    enabled.set_active(existing.as_ref().map(|a| a.enabled).unwrap_or(true));
    let kind = adw::ComboRow::new();
    kind.set_title(&ctx.t_or("alerts.action.kind", "Kind"));
    kind.set_model(Some(&gtk::StringList::new(ACTION_KINDS)));
    if let Some(action) = &existing {
        if let Some(idx) = ACTION_KINDS.iter().position(|k| *k == action.kind) {
            kind.set_selected(idx as u32);
        }
    }
    let url = adw::EntryRow::new();
    url.set_title(&ctx.t_or("alerts.action.url", "URL"));
    let method = adw::EntryRow::new();
    method.set_title(&ctx.t_or("alerts.action.method", "Method"));
    let token = adw::PasswordEntryRow::new();
    token.set_title(&ctx.t_or("alerts.action.botToken", "Token"));
    let extra = adw::EntryRow::new();
    extra.set_title(&ctx.t_or("common.moreActions", "Extra"));
    let extra2 = adw::EntryRow::new();
    extra2.set_title(&ctx.t_or("alerts.action.from", "From"));
    let headers = adw::EntryRow::new();
    headers.set_title(&ctx.t_or("alerts.action.headers", "Headers (Key: Value)"));
    let telegram_mode = adw::ComboRow::new();
    telegram_mode.set_title(&ctx.t_or("alerts.action.telegramMode", "Telegram mode"));
    let telegram_bot = ctx.t_or("alerts.action.telegram_bot", "Bot API");
    let telegram_botless = ctx.t_or("alerts.action.telegram_botless", "Bot-less (CallMeBot)");
    telegram_mode.set_model(Some(&gtk::StringList::new(&[
        telegram_bot.as_str(),
        telegram_botless.as_str(),
    ])));
    if existing
        .as_ref()
        .and_then(|a| a.config.get("mode"))
        .and_then(|x| x.as_str())
        == Some("botless")
    {
        telegram_mode.set_selected(1);
    }
    let timeout = adw::SpinRow::with_range(1.0, 120.0, 1.0);
    timeout.set_title(&ctx.t_or("alerts.action.timeout", "Timeout (seconds)"));
    timeout.set_value(
        existing
            .as_ref()
            .and_then(|a| a.config.get("timeout_secs"))
            .and_then(|x| x.as_f64())
            .unwrap_or(8.0),
    );
    let tls_verify = adw::SwitchRow::new();
    tls_verify.set_title(&ctx.t_or("alerts.action.tlsVerify", "Verify TLS"));
    tls_verify.set_active(
        existing
            .as_ref()
            .and_then(|a| a.config.get("tls_verify"))
            .and_then(|x| x.as_bool())
            .unwrap_or(true),
    );
    let subject = adw::EntryRow::new();
    subject.set_title(&ctx.t_or("alerts.action.subjectTemplate", "Subject template"));
    subject.set_text(
        &existing
            .as_ref()
            .and_then(|a| a.config.get("subject_template"))
            .and_then(|x| x.as_str())
            .unwrap_or("{{title}}"),
    );
    let qos = adw::SpinRow::with_range(0.0, 2.0, 1.0);
    qos.set_title(&ctx.t_or("alerts.action.qos", "MQTT QoS"));
    qos.set_value(
        existing
            .as_ref()
            .and_then(|a| a.config.get("qos"))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
    );
    let retain = adw::SwitchRow::new();
    retain.set_title(&ctx.t_or("alerts.action.retain", "MQTT retain"));
    retain.set_active(
        existing
            .as_ref()
            .and_then(|a| a.config.get("retain"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    );
    let encryption = adw::ComboRow::new();
    encryption.set_title(&ctx.t_or("alerts.action.encryption", "Encryption"));
    encryption.set_model(Some(&gtk::StringList::new(&[
        "None",
        "TLS (Port 465)",
        "StartTLS (Port 587)",
    ])));
    encryption.set_selected(
        match existing
            .as_ref()
            .and_then(|a| a.config.get("encryption"))
            .and_then(|x| x.as_str())
        {
            Some("none") => 0,
            Some("tls") => 1,
            _ => 2,
        },
    );
    let env_vars = adw::EntryRow::new();
    env_vars.set_title(&ctx.t_or("alerts.action.envVars", "Environment variables (KEY=value)"));
    let username = adw::EntryRow::new();
    username.set_title(&ctx.t_or("common.username", "Username"));
    let mqtt_tls = adw::SwitchRow::new();
    mqtt_tls.set_title(&ctx.t_or("alerts.action.useTls", "Use TLS"));
    mqtt_tls.set_active(
        existing
            .as_ref()
            .and_then(|a| a.config.get("use_tls"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    );
    let browse_lbl = ctx.t_or("common.browse", "Browse");
    let browse = gtk::Button::from_icon_name("folder-open-symbolic");
    browse.set_tooltip_text(Some(&browse_lbl));
    browse.set_valign(gtk::Align::Center);
    extra.add_suffix(&browse);
    {
        let extra = extra.clone();
        browse.connect_clicked(move |btn| {
            let Some(win) = btn.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let dialog = gtk::FileDialog::new();
            let extra = extra.clone();
            dialog.open(
                Some(&win),
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            extra.set_text(&path.to_string_lossy());
                        }
                    }
                },
            );
        });
    }
    let discord_lbl = ctx.t_or("alerts.action.discord", "Discord");
    let slack_lbl = ctx.t_or("alerts.action.slack", "Slack");
    let discord_btn = gtk::Button::with_label(&discord_lbl);
    let slack_btn = gtk::Button::with_label(&slack_lbl);
    let presets_label = gtk::Label::new(Some(&ctx.t_or("alerts.action.presets", "Presets")));
    presets_label.add_css_class("dim-label");
    presets_label.set_xalign(0.0);
    let presets = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    presets.append(&presets_label);
    presets.append(&discord_btn);
    presets.append(&slack_btn);
    let wa_provider = adw::ComboRow::new();
    wa_provider.set_title(&ctx.t_or("alerts.action.whatsappProvider", "WhatsApp provider"));
    wa_provider.set_model(Some(&gtk::StringList::new(&[
        "CallMeBot",
        "Custom gateway",
    ])));
    if existing
        .as_ref()
        .and_then(|a| a.config.get("provider"))
        .and_then(|x| x.as_str())
        == Some("custom_gateway")
    {
        wa_provider.set_selected(1);
    }
    let body = adw::EntryRow::new();
    body.set_title(&ctx.t_or("alerts.action.bodyTemplate", "Body template"));
    if let Some(action) = &existing {
        url.set_text(&action_cfg(action, &["url", "broker_url", "smtp_server"]));
        method.set_text(&action_cfg(action, &["method", "topic", "smtp_port"]));
        token.set_text(&action_cfg(action, &["bot_token", "password", "apikey"]));
        username.set_text(&action_cfg(action, &["username"]));
        extra.set_text(&action_cfg(action, &["chat_id", "phone", "to", "command"]));
        extra2.set_text(&action_cfg(action, &["from"]));
        if extra2.text().is_empty() {
            if let Some(args) = action.config.get("args").and_then(|x| x.as_array()) {
                let joined = args
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                extra2.set_text(&joined);
            }
        }
        headers.set_text(&crate::store::headers_to_text(&action.config));
        env_vars.set_text(&crate::store::env_vars_to_text(&action.config));
        body.set_text(
            action
                .config
                .get("body_template")
                .and_then(|x| x.as_str())
                .unwrap_or("{{title}}: {{body}}"),
        );
    } else {
        body.set_text("{{title}}: {{body}}");
    }
    let retries = adw::SpinRow::with_range(0.0, 5.0, 1.0);
    retries.set_title(&ctx.t_or("alerts.action.retryCount", "Retry count"));
    retries.set_subtitle(&ctx.t_or(
        "alerts.action.bodyTemplateHint",
        "Extra delivery attempts after a failure",
    ));
    retries.set_value(
        existing
            .as_ref()
            .map(crate::store::alert_retry_count)
            .unwrap_or(0) as f64,
    );
    let keys = adw::ComboRow::new();
    keys.set_title(&ctx.t_or("alerts.templateKeys", "Insert template key"));
    keys.set_model(Some(&gtk::StringList::new(
        crate::store::ALERT_TEMPLATE_KEYS,
    )));
    {
        let body = body.clone();
        let primed = Rc::new(Cell::new(false));
        keys.connect_selected_notify(move |row| {
            if !primed.get() {
                primed.set(true);
                return;
            }
            if let Some(key) = crate::store::ALERT_TEMPLATE_KEYS.get(row.selected() as usize) {
                let current = body.text().to_string();
                if !current.contains(key) {
                    if current.is_empty() {
                        body.set_text(key);
                    } else {
                        body.set_text(&format!("{current} {key}"));
                    }
                }
            }
        });
    }
    let test = gtk::Button::with_label(&ctx.t_or("modals.backend.testConnection", "Test"));
    let save = gtk::Button::with_label(&ctx.t("common.save"));
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label(&ctx.t("common.delete"));
    delete.add_css_class("destructive-action");
    delete.set_visible(existing_id.is_some());
    let collect = {
        let name = name.clone();
        let enabled = enabled.clone();
        let kind = kind.clone();
        let url = url.clone();
        let method = method.clone();
        let token = token.clone();
        let extra = extra.clone();
        let extra2 = extra2.clone();
        let headers = headers.clone();
        let body = body.clone();
        let retries = retries.clone();
        let timeout = timeout.clone();
        let tls_verify = tls_verify.clone();
        let subject = subject.clone();
        let qos = qos.clone();
        let retain = retain.clone();
        let encryption = encryption.clone();
        let env_vars = env_vars.clone();
        let username = username.clone();
        let mqtt_tls = mqtt_tls.clone();
        let telegram_mode = telegram_mode.clone();
        let wa_provider = wa_provider.clone();
        let existing_id = existing_id.clone();
        let ctx = ctx.clone();
        move || {
            let selected = ACTION_KINDS
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("os_toast");
            let mut action = existing_id
                .as_ref()
                .and_then(|id| {
                    ctx.store
                        .borrow()
                        .alert_actions
                        .iter()
                        .find(|a| &a.id == id)
                        .cloned()
                })
                .unwrap_or_else(|| AlertAction::new(name.text().to_string(), selected.into()));
            action.name = name.text().to_string();
            action.enabled = enabled.is_active();
            action.kind = selected.into();
            action.config = crate::store::alert_action_config(
                selected,
                &crate::store::AlertActionDraft {
                    url: url.text().to_string(),
                    method: method.text().to_string(),
                    token: token.text().to_string(),
                    extra: extra.text().to_string(),
                    extra2: extra2.text().to_string(),
                    headers: headers.text().to_string(),
                    body: body.text().to_string(),
                    retry_count: retries.value() as u32,
                    timeout_secs: timeout.value() as u32,
                    tls_verify: tls_verify.is_active(),
                    subject: subject.text().to_string(),
                    qos: qos.value() as u32,
                    retain: retain.is_active(),
                    encryption: match encryption.selected() {
                        0 => "none".into(),
                        1 => "tls".into(),
                        _ => "starttls".into(),
                    },
                    env_vars: env_vars.text().to_string(),
                    username: username.text().to_string(),
                    use_tls: mqtt_tls.is_active(),
                    telegram_mode: if telegram_mode.selected() == 1 {
                        "botless".into()
                    } else {
                        "bot".into()
                    },
                    provider: if wa_provider.selected() == 1 {
                        "custom_gateway".into()
                    } else {
                        "callmebot".into()
                    },
                },
            );
            action
        }
    };
    {
        let collect = collect.clone();
        let parent = parent.clone();
        let ctx = ctx.clone();
        test.connect_clicked(move |_| {
            let action = collect();
            let mut event = AlertEvent::new(
                AlertEventKind::System,
                AlertSeverity::Info,
                "Test alert".into(),
                format!("Testing action {}", action.name),
            );
            event.remote = ctx
                .selected_remote
                .borrow()
                .clone()
                .unwrap_or_else(|| "drive".into());
            event.profile = "default".into();
            event.origin = "test".into();
            crate::store::dispatch_action(&action, &event);
            let toast = adw::AlertDialog::new(
                Some("Test sent"),
                Some("The action was invoked with a sample event."),
            );
            toast.add_response("ok", &ctx.t_or("common.ok", "OK"));
            toast.present(Some(&parent));
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let collect = collect.clone();
        save.connect_clicked(move |_| {
            let action = collect();
            {
                let mut store = ctx.store.borrow_mut();
                if let Some(idx) = store.alert_actions.iter().position(|a| a.id == action.id) {
                    store.alert_actions[idx] = action;
                } else {
                    store.alert_actions.push(action);
                }
            }
            ctx.persist();
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        delete.connect_clicked(move |_| {
            if let Some(id) = &existing_id {
                ctx.store.borrow_mut().alert_actions.retain(|a| &a.id != id);
                ctx.persist();
            }
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }
    {
        let method = method.clone();
        let body = body.clone();
        let headers = headers.clone();
        discord_btn.connect_clicked(move |_| {
            method.set_text("POST");
            body.set_text(&crate::store::webhook_preset_body("discord"));
            headers.set_text(&crate::store::ensure_content_type_json(&headers.text()));
        });
    }
    {
        let method = method.clone();
        let body = body.clone();
        let headers = headers.clone();
        slack_btn.connect_clicked(move |_| {
            method.set_text("POST");
            body.set_text(&crate::store::webhook_preset_body("slack"));
            headers.set_text(&crate::store::ensure_content_type_json(&headers.text()));
        });
    }
    let sync_fields = {
        let url = url.clone();
        let method = method.clone();
        let token = token.clone();
        let extra = extra.clone();
        let extra2 = extra2.clone();
        let headers = headers.clone();
        let timeout = timeout.clone();
        let tls_verify = tls_verify.clone();
        let subject = subject.clone();
        let qos = qos.clone();
        let retain = retain.clone();
        let encryption = encryption.clone();
        let env_vars = env_vars.clone();
        let username = username.clone();
        let mqtt_tls = mqtt_tls.clone();
        let browse = browse.clone();
        let presets = presets.clone();
        let telegram_mode = telegram_mode.clone();
        let body = body.clone();
        let wa_provider = wa_provider.clone();
        let ctx = ctx.clone();
        move |selected: &str| {
            subject.set_visible(selected == "email");
            qos.set_visible(selected == "mqtt");
            retain.set_visible(selected == "mqtt");
            encryption.set_visible(selected == "email");
            env_vars.set_visible(selected == "script");
            username.set_visible(selected == "email" || selected == "mqtt");
            mqtt_tls.set_visible(selected == "mqtt");
            browse.set_visible(selected == "script");
            presets.set_visible(selected == "webhook");
            match selected {
                "os_toast" => {
                    wa_provider.set_visible(false);
                    telegram_mode.set_visible(false);
                    url.set_visible(false);
                    method.set_visible(false);
                    token.set_visible(false);
                    extra.set_visible(false);
                    extra2.set_visible(false);
                    headers.set_visible(false);
                    timeout.set_visible(false);
                    tls_verify.set_visible(false);
                    body.set_visible(true);
                    body.set_title(
                        &ctx.t_or("alerts.action.messageTemplate", "Toast body template"),
                    );
                }
                "webhook" => {
                    wa_provider.set_visible(false);
                    telegram_mode.set_visible(false);
                    url.set_visible(true);
                    url.set_title(&ctx.t_or("alerts.action.url", "Webhook URL"));
                    method.set_visible(true);
                    method.set_title(&ctx.t_or("alerts.action.method", "HTTP method"));
                    token.set_visible(false);
                    extra.set_visible(false);
                    extra2.set_visible(false);
                    headers.set_visible(true);
                    timeout.set_visible(true);
                    tls_verify.set_visible(true);
                    body.set_visible(true);
                    body.set_title(&ctx.t_or("alerts.action.bodyTemplate", "JSON / body template"));
                }
                "telegram" => {
                    wa_provider.set_visible(false);
                    telegram_mode.set_visible(true);
                    url.set_visible(false);
                    method.set_visible(false);
                    let botless = telegram_mode.selected() == 1;
                    token.set_visible(!botless);
                    token.set_title(&ctx.t_or("alerts.action.botToken", "Bot token"));
                    extra.set_visible(true);
                    extra.set_title(&ctx.t_or("alerts.action.chatId", "Chat ID"));
                    extra2.set_visible(false);
                    headers.set_visible(false);
                    timeout.set_visible(true);
                    tls_verify.set_visible(false);
                    body.set_visible(true);
                    body.set_title(&ctx.t_or("alerts.action.messageTemplate", "Message template"));
                }
                "whatsapp" => {
                    wa_provider.set_visible(true);
                    telegram_mode.set_visible(false);
                    let custom = wa_provider.selected() == 1;
                    url.set_visible(custom);
                    url.set_title(&ctx.t_or("alerts.action.gatewayUrl", "Gateway URL"));
                    method.set_visible(false);
                    token.set_visible(!custom);
                    token.set_title(&ctx.t_or("alerts.action.apiKey", "API key"));
                    extra.set_visible(true);
                    extra.set_title(&ctx.t_or("alerts.action.phone", "Phone number"));
                    extra2.set_visible(false);
                    headers.set_visible(false);
                    timeout.set_visible(true);
                    tls_verify.set_visible(false);
                    body.set_visible(true);
                    body.set_title(&ctx.t_or("alerts.action.messageTemplate", "Message template"));
                }
                "script" => {
                    wa_provider.set_visible(false);
                    telegram_mode.set_visible(false);
                    url.set_visible(false);
                    method.set_visible(false);
                    token.set_visible(false);
                    extra.set_visible(true);
                    extra.set_title(&ctx.t_or("alerts.action.command", "Command"));
                    extra2.set_visible(true);
                    extra2.set_title(&ctx.t_or("alerts.action.args", "Arguments"));
                    headers.set_visible(false);
                    timeout.set_visible(true);
                    tls_verify.set_visible(false);
                    body.set_visible(true);
                    body.set_title(&ctx.t_or("alerts.action.bodyTemplate", "Stdin template"));
                }
                "email" => {
                    wa_provider.set_visible(false);
                    telegram_mode.set_visible(false);
                    url.set_visible(true);
                    url.set_title(&ctx.t_or("alerts.action.smtpHost", "SMTP host"));
                    method.set_visible(true);
                    method.set_title(&ctx.t_or("alerts.action.smtpPort", "SMTP port"));
                    token.set_visible(true);
                    token.set_title(&ctx.t("common.password"));
                    extra.set_visible(true);
                    extra.set_title(&ctx.t_or("alerts.action.to", "To"));
                    extra2.set_visible(true);
                    extra2.set_title(&ctx.t_or("alerts.action.from", "From"));
                    headers.set_visible(false);
                    timeout.set_visible(true);
                    tls_verify.set_visible(false);
                    body.set_visible(true);
                    body.set_title(&ctx.t_or("alerts.action.bodyTemplate", "Body template"));
                }
                "mqtt" => {
                    wa_provider.set_visible(false);
                    telegram_mode.set_visible(false);
                    url.set_visible(true);
                    url.set_title(&ctx.t_or("alerts.action.brokerUrl", "Broker URL"));
                    method.set_visible(true);
                    method.set_title(&ctx.t_or("alerts.action.topic", "Topic"));
                    token.set_visible(true);
                    token.set_title(&ctx.t("common.password"));
                    extra.set_visible(false);
                    extra2.set_visible(false);
                    headers.set_visible(false);
                    timeout.set_visible(true);
                    tls_verify.set_visible(false);
                    body.set_visible(true);
                    body.set_title(&ctx.t_or("alerts.action.bodyTemplate", "Payload template"));
                }
                _ => {}
            }
        }
    };
    {
        let sync_fields = sync_fields.clone();
        let initial = ACTION_KINDS
            .get(kind.selected() as usize)
            .copied()
            .unwrap_or("os_toast");
        sync_fields(initial);
        kind.connect_selected_notify(move |row| {
            let selected = ACTION_KINDS
                .get(row.selected() as usize)
                .copied()
                .unwrap_or("os_toast");
            sync_fields(selected);
        });
    }
    {
        let sync_fields = sync_fields.clone();
        let kind = kind.clone();
        wa_provider.connect_selected_notify(move |_| {
            let selected = ACTION_KINDS
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("os_toast");
            sync_fields(selected);
        });
    }
    {
        let sync_fields = sync_fields.clone();
        let kind = kind.clone();
        telegram_mode.connect_selected_notify(move |_| {
            let selected = ACTION_KINDS
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("os_toast");
            sync_fields(selected);
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&enabled);
    group.add(&kind);
    group.add(&wa_provider);
    group.add(&telegram_mode);
    group.add(&url);
    group.add(&method);
    group.add(&token);
    group.add(&username);
    group.add(&extra);
    group.add(&extra2);
    group.add(&headers);
    group.add(&timeout);
    group.add(&tls_verify);
    group.add(&subject);
    group.add(&encryption);
    group.add(&qos);
    group.add(&mqtt_tls);
    group.add(&retain);
    group.add(&env_vars);
    group.add(&body);
    group.add(&retries);
    group.add(&keys);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&test);
    buttons.append(&save);
    buttons.append(&delete);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&presets);
    box_.append(&group);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn action_cfg(action: &AlertAction, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = action.config.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            } else if let Some(n) = v.as_u64() {
                return n.to_string();
            }
        }
    }
    String::new()
}

fn filter_switch_rows(
    options: &[String],
    selected: Option<&[String]>,
) -> Vec<(String, adw::SwitchRow)> {
    options
        .iter()
        .map(|option| {
            let row = adw::SwitchRow::new();
            row.set_title(option);
            row.set_active(match selected {
                Some(list) if !list.is_empty() => list.iter().any(|s| s == option),
                _ => true,
            });
            (option.clone(), row)
        })
        .collect()
}

fn selected_or_all(rows: &[(String, adw::SwitchRow)]) -> Vec<String> {
    let selected: Vec<String> = rows
        .iter()
        .filter(|(_, row)| row.is_active())
        .map(|(id, _)| id.clone())
        .collect();
    if selected.len() == rows.len() {
        Vec::new()
    } else {
        selected
    }
}

pub fn password_prompt(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dialog = adw::AlertDialog::new(
        Some(&ctx.t_or(
            "modals.backend.security.configPassword",
            "rclone.conf password",
        )),
        Some(&ctx.t_or(
            "repair.passwordPrompt",
            "Enter the rclone.conf password to unlock the engine.",
        )),
    );
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    let remember_label = ctx.t_or(
        "modals.backend.security.systemKeychain",
        "Remember in the system keyring",
    );
    let remember = gtk::CheckButton::with_label(&remember_label);
    remember.set_active(true);
    box_.append(&entry);
    box_.append(&remember);
    dialog.set_extra_child(Some(&box_));
    dialog.add_response("cancel", &ctx.t("common.cancel"));
    let unlock = ctx.t_or("repair.unlock", "Unlock");
    dialog.add_response("unlock", &unlock);
    dialog.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let entry = entry.clone();
        let remember = remember.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "unlock" {
                return;
            }
            let password = entry.text().to_string();
            if password.is_empty() {
                toast.add_toast(adw::Toast::new(
                    &ctx.t_or("repair.passwordEmpty", "Password is empty"),
                ));
                return;
            }
            if remember.is_active() {
                crate::keyring::persist_password_setting(
                    &mut ctx.settings.borrow_mut().core.config_password,
                    &password,
                );
                ctx.persist();
            }
            if let Some(client) = ctx.client() {
                match client.config_unlock(&password) {
                    Ok(_) => toast.add_toast(adw::Toast::new(
                        &ctx.t_or("repair.passwordUnlocked", "Config unlocked"),
                    )),
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
            ctx.restart_engine();
        });
    }
    dialog.present(Some(parent));
}

pub fn repair(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
    if try_spawn_standalone(&ctx, "repair", serde_json::json!({})) {
        return;
    }
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("repair.title", "Repair rclone"));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let version = ctx
        .client()
        .and_then(|c| c.version().ok())
        .or_else(|| ctx.engine.borrow().as_ref().map(|e| e.version.clone()));
    let issues = crate::repair::diagnose(
        &ctx.settings.borrow(),
        ctx.engine_ready(),
        ctx.client().as_ref(),
        version.as_deref(),
    );
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    if issues.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("repair.noIssues", "No issues detected"));
        row.set_subtitle(&ctx.t_or(
            "repair.healthy",
            "rclone, FUSE, config, and the RC engine look healthy.",
        ));
        list.append(&row);
    }
    for issue in issues {
        let (title_key, detail_key, action_key) = crate::repair::issue_i18n_keys(issue.kind);
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(title_key, &issue.title));
        let subtitle = if issue.kind == crate::repair::RepairKind::VersionTooOld {
            ctx.tf(
                detail_key,
                &[("required", crate::repair::MIN_RCLONE_VERSION)],
            )
        } else if issue.detail.is_empty() {
            ctx.t_or(detail_key, &issue.detail)
        } else {
            issue.detail.clone()
        };
        row.set_subtitle(&subtitle);
        let btn = gtk::Button::with_label(&ctx.t_or(action_key, &issue.action));
        btn.set_valign(gtk::Align::Center);
        btn.add_css_class("suggested-action");
        {
            let ctx = ctx.clone();
            let toast = toast.clone();
            let parent = parent.clone();
            let kind = issue.kind.clone();
            btn.connect_clicked(move |_| match kind {
                crate::repair::RepairKind::MissingBinary
                | crate::repair::RepairKind::VersionTooOld => {
                    install_rclone_update(&parent, ctx.clone(), toast.clone());
                    ctx.restart_engine();
                }
                crate::repair::RepairKind::FuseMissing => {
                    let title = ctx.t_or(
                        "repairSheet.progress.installingPlugin",
                        "Installing Plugin...",
                    );
                    run_progress_job(
                        &parent,
                        ctx.clone(),
                        toast.clone(),
                        &title,
                        |cancel, progress| crate::mount_plugin::install_ex(cancel, progress),
                        |_ctx, msg, toast| {
                            toast.add_toast(adw::Toast::new(&msg));
                        },
                    );
                }
                crate::repair::RepairKind::PasswordRequired => {
                    password_prompt(&parent, ctx.clone(), toast.clone());
                }
                crate::repair::RepairKind::AuthFailed => {
                    ctx.restart_engine();
                    toast.add_toast(adw::Toast::new(&ctx.t_or(
                        "repairSheet.progress.restartingEngine",
                        "Clearing auth error and retrying…",
                    )));
                    backends(&parent, ctx.clone());
                }
                crate::repair::RepairKind::ConfigUnreadable => {
                    let ask = adw::AlertDialog::new(
                        Some(&ctx.t_or(
                            "repairSheet.titles.corruptConfig",
                            "Configuration Corrupt",
                        )),
                        Some(&ctx.t_or(
                            "repairSheet.messages.corruptConfig",
                            "The rclone configuration file appears to be corrupt. Restore from backup?",
                        )),
                    );
                    ask.add_response(
                        "restore",
                        &ctx.t_or("repairSheet.actions.restoreBackup", "Restore Backup"),
                    );
                    ask.add_response(
                        "pick",
                        &ctx.t_or("repair.chooseConfig", "Choose rclone.conf…"),
                    );
                    ask.add_response("cancel", &ctx.t_or("common.cancel", "Cancel"));
                    ask.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
                    {
                        let ctx = ctx.clone();
                        let toast = toast.clone();
                        let parent = parent.clone();
                        ask.connect_response(None, move |_, response| {
                            if response == "restore" {
                                if let Some(win) = parent.root().and_downcast::<gtk::Window>() {
                                    import_backup(
                                        &win,
                                        ctx.clone(),
                                        toast.clone(),
                                        Rc::new(|| {}),
                                    );
                                }
                            } else if response == "pick" {
                                restore_or_pick_config(&parent, ctx.clone());
                            }
                        });
                    }
                    ask.present(Some(&parent));
                }
                crate::repair::RepairKind::EngineUnreachable => {
                    ctx.restart_engine();
                    toast.add_toast(adw::Toast::new(
                        &ctx.t_or("repair.engineRestarted", "Engine restart requested"),
                    ));
                }
            });
        }
        row.add_suffix(&btn);
        list.append(&row);
    }
    let install = gtk::Button::with_label(&ctx.t_or("repair.installRclone", "Install rclone"));
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let parent = parent.clone();
        install.connect_clicked(move |_| {
            install_rclone_update(&parent, ctx.clone(), toast.clone());
            ctx.restart_engine();
        });
    }
    let browse = gtk::Button::with_label(&ctx.t_or("repair.chooseBinary", "Choose binary…"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        browse.connect_clicked(move |_| pick_rclone_binary(&parent, ctx.clone()));
    }
    let config_btn =
        gtk::Button::with_label(&ctx.t_or("repair.chooseConfig", "Choose rclone.conf…"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        config_btn.connect_clicked(move |_| restore_or_pick_config(&parent, ctx.clone()));
    }
    let restart = gtk::Button::with_label(&ctx.t_or("repair.restartEngine", "Restart engine"));
    {
        let ctx = ctx.clone();
        restart.connect_clicked(move |_| {
            ctx.restart_engine();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(16);
    box_.set_margin_start(16);
    box_.set_margin_end(16);
    box_.set_margin_bottom(16);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_min_content_height(220);
    scroll.set_child(Some(&list));
    box_.append(&scroll);
    box_.append(&install);
    box_.append(&browse);
    box_.append(&config_btn);
    box_.append(&restart);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn pick_rclone_binary(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let picker = gtk::FileDialog::new();
    picker.open(
        Some(&win),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    ctx.settings.borrow_mut().core.rclone_binary =
                        path.to_string_lossy().into_owned();
                    ctx.persist();
                    ctx.restart_engine();
                }
            }
        },
    );
}

fn restore_or_pick_config(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let picker = gtk::FileDialog::new();
    picker.open(
        Some(&win),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let flag = format!("--config={}", path.display());
                    let flags = &mut ctx.settings.borrow_mut().core.rclone_additional_flags;
                    flags.retain(|f| !f.starts_with("--config"));
                    flags.push(flag);
                    ctx.persist();
                    ctx.restart_engine();
                }
            }
        },
    );
}

pub fn multi_rename(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    names: Vec<String>,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("nautilus.contextMenu.renameMultiple", "Rename items"));
    dialog.set_content_width(560);
    dialog.set_content_height(620);
    let mode = adw::ComboRow::new();
    mode.set_title(&ctx.t_or("nautilus.modals.multiRename.templateMode", "Mode"));
    let template_l = ctx.t_or("nautilus.modals.multiRename.templateMode", "Template");
    let replace_l = ctx.t_or(
        "nautilus.modals.multiRename.replaceMode",
        "Find and replace",
    );
    mode.set_model(Some(&gtk::StringList::new(&[&template_l, &replace_l])));
    let template = adw::EntryRow::new();
    template.set_title(&ctx.t_or("nautilus.modals.multiRename.templateInput", "Template"));
    template.set_text("[Original file name]");
    let find = adw::EntryRow::new();
    find.set_title(&ctx.t_or("nautilus.modals.multiRename.findLabel", "Find"));
    let replace = adw::EntryRow::new();
    replace.set_title(&ctx.t_or("nautilus.modals.multiRename.replaceLabel", "Replace with"));
    let start = adw::EntryRow::new();
    start.set_title(&ctx.t_or("nautilus.modals.multiRename.counterStart", "Counter start"));
    start.set_text("1");
    let step = adw::EntryRow::new();
    step.set_title(&ctx.t_or("nautilus.modals.multiRename.counterStep", "Counter step"));
    step.set_text("1");
    let pad = adw::EntryRow::new();
    pad.set_title(&ctx.t_or(
        "nautilus.modals.multiRename.counterPadding",
        "Counter padding",
    ));
    pad.set_text("2");
    let case_sensitive = adw::SwitchRow::new();
    case_sensitive.set_title(&ctx.t_or(
        "nautilus.modals.multiRename.caseSensitive",
        "Case sensitive",
    ));
    let preview = gtk::ListBox::new();
    preview.add_css_class("boxed-list");
    let plan_slots = Rc::new(RefCell::new((
        template.clone(),
        find.clone(),
        replace.clone(),
        start.clone(),
        step.clone(),
        pad.clone(),
        mode.clone(),
        case_sensitive.clone(),
    )));
    let refresh_preview = {
        let names = names.clone();
        let preview = preview.clone();
        let plan_slots = plan_slots.clone();
        move || {
            while let Some(child) = preview.first_child() {
                preview.remove(&child);
            }
            let slots = plan_slots.borrow();
            let plan = RenamePlan {
                mode: if slots.6.selected() == 0 {
                    RenameMode::Template
                } else {
                    RenameMode::Replace
                },
                template: slots.0.text().to_string(),
                find_text: slots.1.text().to_string(),
                replace_with: slots.2.text().to_string(),
                counter_start: slots.3.text().parse().unwrap_or(1),
                counter_step: slots.4.text().parse().unwrap_or(1),
                counter_padding: slots.5.text().parse().unwrap_or(2),
                case_sensitive: slots.7.is_active(),
            };
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            for row in rename_preview(&names, &plan, &date) {
                let item = adw::ActionRow::new();
                item.set_title(&row.original);
                item.set_subtitle(&if row.has_error {
                    format!("{} (invalid)", row.new_name)
                } else {
                    row.new_name
                });
                preview.append(&item);
            }
        }
    };
    for entry in [&template, &find, &replace, &start, &step, &pad] {
        let refresh_preview = refresh_preview.clone();
        entry.connect_changed(move |_| refresh_preview());
    }
    {
        let refresh_preview = refresh_preview.clone();
        mode.connect_selected_notify(move |_| refresh_preview());
    }
    {
        let refresh_preview = refresh_preview.clone();
        case_sensitive.connect_active_notify(move |_| refresh_preview());
    }
    refresh_preview();
    let tokens = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    tokens.set_homogeneous(false);
    for (label, token) in [
        (
            ctx.t_or("nautilus.modals.multiRename.originalName", "Original name"),
            "[Original file name]",
        ),
        (
            ctx.t_or("nautilus.modals.multiRename.counter", "Counter"),
            "[Counter]",
        ),
        (
            ctx.t_or("nautilus.modals.multiRename.date", "Date"),
            "[Date]",
        ),
        (
            ctx.t_or("nautilus.modals.multiRename.extension", "Extension"),
            "[Extension]",
        ),
    ] {
        let chip = gtk::Button::with_label(&label);
        chip.add_css_class("pill");
        let template = template.clone();
        let refresh_preview = refresh_preview.clone();
        chip.connect_clicked(move |_| {
            let current = template.text().to_string();
            if current.contains(token) {
                return;
            }
            if current.is_empty() {
                template.set_text(token);
            } else {
                template.set_text(&format!("{current}{token}"));
            }
            refresh_preview();
        });
        tokens.append(&chip);
    }
    let apply = gtk::Button::with_label(&ctx.t_or("nautilus.modals.multiRename.rename", "Rename"));
    apply.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let folder = path.to_string();
        let names = names.clone();
        let plan_slots = plan_slots.clone();
        apply.connect_clicked(move |_| {
            let Some(client) = ctx.client() else {
                return;
            };
            let slots = plan_slots.borrow();
            let plan = RenamePlan {
                mode: if slots.6.selected() == 0 {
                    RenameMode::Template
                } else {
                    RenameMode::Replace
                },
                template: slots.0.text().to_string(),
                find_text: slots.1.text().to_string(),
                replace_with: slots.2.text().to_string(),
                counter_start: slots.3.text().parse().unwrap_or(1),
                counter_step: slots.4.text().parse().unwrap_or(1),
                counter_padding: slots.5.text().parse().unwrap_or(2),
                case_sensitive: slots.7.is_active(),
            };
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            let rows = rename_preview(&names, &plan, &date);
            if crate::rename::has_errors(&rows) || !crate::rename::has_changes(&rows) {
                return;
            }
            let fs = if remote == "local" {
                "/".into()
            } else {
                remote_fs(&remote, "")
            };
            for row in rows {
                if row.new_name == row.original {
                    continue;
                }
                let src = crate::rclone::join_remote_path(&folder, &row.original);
                let dst = crate::rclone::join_remote_path(&folder, &row.new_name);
                let src = if remote == "local" {
                    src.trim_start_matches('/').to_string()
                } else {
                    src
                };
                let dst = if remote == "local" {
                    dst.trim_start_matches('/').to_string()
                } else {
                    dst
                };
                if let Err(e) = client.move_file(&fs, &src, &fs, &dst) {
                    let err = adw::AlertDialog::new(Some("Rename failed"), Some(&e.to_string()));
                    err.add_response("ok", &ctx.t_or("common.ok", "OK"));
                    err.present(Some(&dialog));
                    return;
                }
            }
            on_done();
            dialog.close();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&mode);
    group.add(&template);
    let token_row = adw::ActionRow::new();
    token_row.set_title(&ctx.t_or(
        "nautilus.modals.multiRename.insertPlaceholder",
        "Insert placeholder",
    ));
    token_row.add_suffix(&tokens);
    group.add(&token_row);
    group.add(&find);
    group.add(&replace);
    group.add(&start);
    group.add(&step);
    group.add(&pad);
    group.add(&case_sensitive);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&preview));
    box_.append(&group);
    box_.append(&scroll);
    box_.append(&apply);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}
