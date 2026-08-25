use super::AppCtx;
use crate::backup;
use crate::jobs::{
    build_job_params, default_dest, default_source, flatten_rclone, job_from_status,
    merge_template_into, start_request,
};
use crate::operations::OperationType;
use crate::rclone::{
    browse_target, describe_cron, nanoseconds_to_duration, parse_hashsum, remote_fs, validate_cron,
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
        "export" => export_backup(&window, ctx.clone(), toast),
        "backend" => backends(&window, ctx.clone()),
        "rclone-flags" => rclone_flags(&window, ctx.clone()),
        "job-detail" => job_detail(&window, ctx.clone(), jobid),
        "properties" => properties(&window, ctx.clone(), &remote, &path, &name),
        "remote-about" => remote_about(&window, ctx.clone(), &remote),
        "keyboard-shortcuts" => shortcuts(&window, &ctx),
        "alerts" => alerts(&window, ctx.clone()),
        "archive-create" => archive_create(&window, ctx.clone(), &remote, &path),
        "quick-run-editor" => quick_run_editor(&window, ctx.clone(), None, noop),
        "template-manager" => templates(&window, ctx.clone()),
        "delete-remote" => delete_remote(&window, ctx.clone(), &remote, noop),
        "remote-config" => remote_config(
            &window,
            ctx.clone(),
            if remote.is_empty() {
                None
            } else {
                Some(remote)
            },
            noop,
        ),
        "quick-add-remote" => quick_add_remote(&window, ctx.clone(), noop),
        "restore-preview" => {}
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

pub fn prompt(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    label: &str,
    initial: &str,
    on_ok: impl Fn(String) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(label));
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", "OK");
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

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));
    dialog.set_child(Some(&toolbar));
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
        for (key, label) in [
            ("Alloc", "Allocated"),
            ("Sys", "System"),
            ("HeapAlloc", "Heap"),
            ("NumGC", "GC cycles"),
        ] {
            let row = adw::ActionRow::new();
            row.set_title(label);
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
        row.set_title("Memory stats unavailable");
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
        toast.add_toast(adw::Toast::new("Cannot locate the running binary"));
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
    let dialog = adw::AlertDialog::new(Some(title), Some("Downloading…"));
    dialog.add_response("cancel", &ctx.t_or("common.cancel", "Cancel"));
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    dialog.set_extra_child(Some(&bar));
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let progress = std::sync::Arc::new(std::sync::Mutex::new(
        crate::updater::DownloadProgress::default(),
    ));
    let result: std::sync::Arc<std::sync::Mutex<Option<Result<std::path::PathBuf, String>>>> =
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
                Ok(path) => {
                    if let Some(cb) = on_ok.borrow_mut().take() {
                        cb(ctx_done.clone(), path, toast_done.clone());
                    }
                }
                Err(e) if e == "cancelled" => {
                    toast_done.add_toast(adw::Toast::new("Update cancelled"));
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
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("titlebar.menu.shortcuts", "Keyboard Shortcuts"));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (keys, desc) in [
        ("Ctrl+Q", "Quit"),
        ("Ctrl+B", "File browser"),
        ("Ctrl+N", "Detailed remote"),
        ("Ctrl+R", "Quick add remote"),
        ("Ctrl+I", "Import settings"),
        ("Ctrl+E", "Export settings"),
        ("Ctrl+,", "Preferences"),
        ("Ctrl+.", "Rclone flags"),
        ("Ctrl+Alt+A", "Alerts"),
        ("Ctrl+Alt+F", "Flow"),
        ("Ctrl+Shift+?", "Shortcuts"),
        ("Ctrl+Shift+M", "Refresh mounts"),
        ("Ctrl+Shift+S", "Refresh serves"),
        ("Ctrl+T / Ctrl+W", "New / close file tab"),
        ("Ctrl+Shift+D", "Detach current file tab"),
        ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo file action"),
        ("Ctrl+/", "Toggle split view"),
        ("Ctrl+L / Ctrl+F", "Focus path / search (Files)"),
        ("F5", "Reload listing"),
        ("F2", "Rename"),
        ("Delete", "Delete"),
        ("Ctrl+C / X / V", "Copy / Cut / Paste"),
        ("Ctrl+Shift+N", "New folder"),
        ("Ctrl+H", "Toggle hidden files"),
        ("Backspace", "Parent folder"),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(desc);
        row.set_subtitle(keys);
        list.append(&row);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    dialog.set_child(Some(&scroll));
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
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("VFS · {remote}"));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let fs = remote_fs(remote, "");
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let queue_list = gtk::ListBox::new();
    queue_list.add_css_class("boxed-list");
    let refresh_ui = {
        let ctx = ctx.clone();
        let fs = fs.clone();
        let list = list.clone();
        let queue_list = queue_list.clone();
        move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            while let Some(child) = queue_list.first_child() {
                queue_list.remove(&child);
            }
            let Some(client) = ctx.client() else {
                let row = adw::ActionRow::new();
                row.set_title("Engine offline");
                list.append(&row);
                return;
            };
            match client.vfs_stats(&fs) {
                Ok(value) => {
                    let stats = crate::vfs::parse_vfs_stats(&value);
                    for (title, value) in [
                        ("Metadata dirs", stats.metadata_dirs.to_string()),
                        ("Metadata files", stats.metadata_files.to_string()),
                        ("Uploads", stats.uploads_in_progress.to_string()),
                        ("Errors", stats.errored.to_string()),
                        ("Disk cache", stats.disk_path),
                    ] {
                        let row = adw::ActionRow::new();
                        row.set_title(title);
                        row.set_subtitle(&value);
                        list.append(&row);
                    }
                }
                Err(e) => {
                    let row = adw::ActionRow::new();
                    row.set_title("VFS stats unavailable");
                    row.set_subtitle(&e.to_string());
                    list.append(&row);
                }
            }
            match client.vfs_queue(&fs) {
                Ok(value) => {
                    let items = crate::vfs::parse_vfs_queue(&value);
                    if items.is_empty() {
                        let row = adw::ActionRow::new();
                        row.set_title("Queue is empty");
                        queue_list.append(&row);
                    }
                    for item in items {
                        let row = adw::ActionRow::new();
                        row.set_title(&if item.name.is_empty() {
                            format!("#{}", item.id)
                        } else {
                            item.name.clone()
                        });
                        row.set_subtitle(&format!(
                            "id {} · {} · expiry {}",
                            item.id,
                            crate::rclone::format_bytes(item.size),
                            item.expiry
                        ));
                        queue_list.append(&row);
                    }
                }
                Err(e) => {
                    let row = adw::ActionRow::new();
                    row.set_title("Queue unavailable");
                    row.set_subtitle(&e.to_string());
                    queue_list.append(&row);
                }
            }
        }
    };
    refresh_ui();
    let file = adw::EntryRow::new();
    file.set_title("Forget file / refresh dir");
    let forget = gtk::Button::with_label("Forget file");
    {
        let ctx = ctx.clone();
        let fs = fs.clone();
        let file = file.clone();
        let toast = toast.clone();
        let refresh_ui = refresh_ui.clone();
        forget.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                match client.vfs_forget_ex(&fs, Some(&file.text())) {
                    Ok(_) => {
                        toast.add_toast(adw::Toast::new("Forgot cached file"));
                        refresh_ui();
                    }
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
        });
    }
    let refresh_dir = gtk::Button::with_label("Refresh dir");
    {
        let ctx = ctx.clone();
        let fs = fs.clone();
        let file = file.clone();
        let toast = toast.clone();
        let refresh_ui = refresh_ui.clone();
        refresh_dir.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let dir = file.text();
                match client.vfs_refresh_ex(&fs, Some(&dir), true) {
                    Ok(_) => {
                        toast.add_toast(adw::Toast::new("Refreshed directory"));
                        refresh_ui();
                    }
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
        });
    }
    let poll = adw::EntryRow::new();
    poll.set_title("Poll interval");
    poll.set_text("1m");
    let apply_poll = gtk::Button::with_label("Set interval");
    {
        let ctx = ctx.clone();
        let fs = fs.clone();
        let poll = poll.clone();
        let toast = toast.clone();
        apply_poll.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                match client.vfs_poll_interval(&fs, Some(&poll.text())) {
                    Ok(_) => toast.add_toast(adw::Toast::new("Poll interval updated")),
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
        });
    }
    let expiry = adw::EntryRow::new();
    expiry.set_title("Queue id / expiry");
    expiry.set_text("id=1 expiry=1m");
    let apply_exp = gtk::Button::with_label("Set expiry");
    {
        let ctx = ctx.clone();
        let fs = fs.clone();
        let expiry = expiry.clone();
        let toast = toast.clone();
        let refresh_ui = refresh_ui.clone();
        apply_exp.connect_clicked(move |_| {
            let (id, exp) = crate::vfs::parse_expiry_pair(&expiry.text());
            if let Some(client) = ctx.client() {
                match client.vfs_queue_set_expiry_ex(&fs, &id, &exp, true) {
                    Ok(_) => {
                        toast.add_toast(adw::Toast::new("Queue expiry updated"));
                        refresh_ui();
                    }
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
        });
    }
    let reload = gtk::Button::with_label(&ctx.t("common.refresh"));
    {
        let refresh_ui = refresh_ui.clone();
        reload.connect_clicked(move |_| refresh_ui());
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&forget);
    actions.append(&refresh_dir);
    actions.append(&apply_poll);
    actions.append(&apply_exp);
    actions.append(&reload);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    box_.set_margin_bottom(12);
    let stats_label = gtk::Label::new(Some("Stats"));
    stats_label.add_css_class("heading");
    stats_label.set_xalign(0.0);
    let queue_label = gtk::Label::new(Some("Queue"));
    queue_label.add_css_class("heading");
    queue_label.set_xalign(0.0);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inner.append(&stats_label);
    inner.append(&list);
    inner.append(&queue_label);
    inner.append(&queue_list);
    scroll.set_child(Some(&inner));
    box_.append(&scroll);
    box_.append(&file);
    box_.append(&poll);
    box_.append(&expiry);
    box_.append(&actions);
    dialog.set_child(Some(&box_));
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
                if let Some(details) = &entry.context {
                    let row = adw::ExpanderRow::new();
                    row.set_title(&entry.message);
                    row.set_subtitle(&subtitle);
                    row.add_suffix(&copy);
                    let label = gtk::Label::new(Some(&format!("{context_label}\n{details}")));
                    label.set_wrap(true);
                    label.set_xalign(0.0);
                    label.set_selectable(true);
                    label.add_css_class("monospace");
                    label.set_margin_start(12);
                    label.set_margin_end(12);
                    label.set_margin_top(8);
                    label.set_margin_bottom(8);
                    row.add_row(&label);
                    list.append(&row);
                } else {
                    let row = adw::ActionRow::new();
                    row.set_title(&entry.message);
                    row.set_subtitle(&subtitle);
                    row.add_suffix(&copy);
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
        row.set_title("Engine offline");
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
    extra.set_title("Apply");
    let g = adw::PreferencesGroup::new();
    g.set_description(Some(
        "Edits are sent to rclone via options/set. Categories match the Angular flag panels.",
    ));
    let row = adw::ActionRow::new();
    row.set_title("Write flags to the running engine");
    row.add_suffix(&apply);
    g.add(&row);
    extra.add(&g);
    dialog.add(&extra);

    let json_page = adw::PreferencesPage::new();
    json_page.set_title("JSON");
    let json_group = adw::PreferencesGroup::new();
    json_group.set_title("Raw options/set payload");
    json_group.set_description(Some(
        "Edit the current rclone options as JSON. Apply writes the object via options/set.",
    ));
    let json_toggle = adw::SwitchRow::new();
    json_toggle.set_title("Remember JSON mode");
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
    let json_apply = gtk::Button::with_label("Apply JSON");
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
    json_row.set_title("Write JSON to the running engine");
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
    title: &str,
    catalog: &[&str],
    current: &[String],
    on_save: impl Fn(Vec<String>) + 'static,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(480);
    dialog.set_content_height(560);
    let items = Rc::new(RefCell::new(crate::action_order::build_items(
        current, catalog,
    )));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let rebuild: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let rebuild_fn = {
        let list = list.clone();
        let items = items.clone();
        let rebuild = rebuild.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let snapshot = items.borrow().clone();
            for (idx, item) in snapshot.iter().enumerate() {
                let row = adw::ActionRow::new();
                row.set_title(&item.id);
                row.set_subtitle(if item.visible { "Visible" } else { "Hidden" });
                let visible = gtk::Switch::new();
                visible.set_active(item.visible);
                visible.set_valign(gtk::Align::Center);
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    let id = item.id.clone();
                    visible.connect_active_notify(move |sw| {
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
                up.set_tooltip_text(Some("Move up"));
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
                down.set_tooltip_text(Some("Move down"));
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

    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        let items = items.clone();
        save.connect_clicked(move |_| {
            on_save(crate::action_order::visible_ids(&items.borrow()));
            dialog.close();
        });
    }
    let cancel = gtk::Button::with_label("Cancel");
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    let reset = gtk::Button::with_label("Reset");
    {
        let items = items.clone();
        let rebuild = rebuild.clone();
        let catalog: Vec<String> = catalog.iter().map(|s| (*s).to_string()).collect();
        reset.connect_clicked(move |_| {
            let refs: Vec<&str> = catalog.iter().map(|s| s.as_str()).collect();
            *items.borrow_mut() = crate::action_order::build_items(&[], &refs);
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
    let hint = gtk::Label::new(Some(
        "Toggle visibility and reorder. Hidden actions stay available in Configure.",
    ));
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
    let use_local = gtk::Button::with_label("Use");
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
    for backend in ctx.settings.borrow().core.extra_backends.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&backend.name);
        let marker = if active == backend.name {
            " · active"
        } else {
            ""
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
        test.set_tooltip_text(Some("Test connection"));
        let use_btn = gtk::Button::with_label("Use");
        use_btn.set_valign(gtk::Align::Center);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_valign(gtk::Align::Center);
        {
            let backend = backend.clone();
            let parent = parent.clone();
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
                        "Connected — {}",
                        client.version().unwrap_or_else(|_| "unknown".into())
                    )
                } else {
                    format!("Unreachable at {}:{}", backend.host, backend.port)
                };
                let alert = adw::AlertDialog::new(Some("Backend test"), Some(&msg));
                alert.add_response("ok", "OK");
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
        edit.set_tooltip_text(Some("Edit backend"));
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let backend = backend.clone();
            edit.connect_clicked(move |_| {
                backend_editor(&parent, ctx.clone(), Some(backend.clone()));
            });
        }
        let clone = gtk::Button::with_label("Clone");
        clone.set_valign(gtk::Align::Center);
        clone.set_tooltip_text(Some("Duplicate connection settings into a new backend"));
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let mut entry = backend.clone();
            entry.name = format!("{}-copy", backend.name);
            clone.connect_clicked(move |_| {
                backend_editor(&parent, ctx.clone(), Some(entry.clone()));
            });
        }
        let copy = gtk::Button::with_label("Copy remotes");
        copy.set_valign(gtk::Align::Center);
        copy.set_tooltip_text(Some("Copy remotes from the active backend to this one"));
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
                    Ok(n) => format!("Copied {n} remotes"),
                    Err(e) => e.to_string(),
                };
                let alert = adw::AlertDialog::new(Some("Copy remotes"), Some(&msg));
                alert.add_response("ok", "OK");
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
    let add = gtk::Button::with_label("Add remote RC backend");
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
    name.set_title("Name");
    let host = adw::EntryRow::new();
    host.set_title("Host");
    host.set_text("127.0.0.1");
    let port = adw::EntryRow::new();
    port.set_title("Port");
    port.set_text("5573");
    let user = adw::EntryRow::new();
    user.set_title("Username");
    let pass = adw::PasswordEntryRow::new();
    pass.set_title("Password");
    if let Some(entry) = &existing {
        name.set_text(&entry.name);
        host.set_text(&entry.host);
        port.set_text(&entry.port.to_string());
        user.set_text(&entry.user);
        pass.set_text(&entry.pass);
    }
    let mut copy_labels = vec![
        "Don't copy remotes".to_string(),
        "Local rclone RC".to_string(),
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
    copy_row.set_title("Copy remotes from");
    copy_row.set_subtitle("Optional: clone remotes from another RC backend after saving");
    copy_row.add_suffix(&copy_from);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let name = name.clone();
        let host = host.clone();
        let port = port.clone();
        let user = user.clone();
        let pass = pass.clone();
        let copy_from = copy_from.clone();
        save.connect_clicked(move |_| {
            let port_n = port.text().parse::<u16>().unwrap_or(5573);
            let entry = crate::settings::BackendEntry {
                name: name.text().to_string(),
                host: host.text().to_string(),
                port: port_n,
                user: user.text().to_string(),
                pass: pass.text().to_string(),
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
            if !source_id.is_empty() {
                if let (Some(source), dest) =
                    (rc_client_for(&ctx, &source_id), rc_client_for_entry(&entry))
                {
                    if let Err(e) = dest.copy_remotes_from(&source) {
                        let alert =
                            adw::AlertDialog::new(Some("Copy remotes"), Some(&e.to_string()));
                        alert.add_response("ok", "OK");
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
    group.add(&copy_row);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&save);
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
    let fill_cell: Rc<RefCell<Rc<dyn Fn()>>> = Rc::new(RefCell::new(Rc::new(|| {})));
    let fill: Rc<dyn Fn()> = {
        let history = history.clone();
        let search = search.clone();
        let severity = severity.clone();
        let remote_dd = remote_dd.clone();
        let profile_dd = profile_dd.clone();
        let backend_dd = backend_dd.clone();
        let remote_labels = remote_labels.clone();
        let profile_labels = profile_labels.clone();
        let backend_labels = backend_labels.clone();
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
                    remote: pick(&remote_dd, &remote_labels),
                    profile: pick(&profile_dd, &profile_labels),
                    backend: pick(&backend_dd, &backend_labels),
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
    let filters = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    search.set_hexpand(true);
    filters.append(&search);
    filters.append(&severity);
    filters.append(&remote_dd);
    filters.append(&profile_dd);
    filters.append(&backend_dd);
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
    super::wizard::present(parent, ctx, None, on_done);
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
    dialog.set_title(if quick {
        "Quick Add Remote"
    } else {
        "Remote Configuration"
    });
    dialog.set_content_width(520);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::new();
    name.set_title("Remote name");
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
    type_row.set_title("Provider type");
    type_row.set_model(Some(&gtk::StringList::new(&type_labels)));
    if let Some(idx) = providers.iter().position(|p| p == "drive") {
        type_row.set_selected(idx as u32);
    }
    let extra = adw::EntryRow::new();
    extra.set_title("Parameters (key=value;key=value)");
    group.add(&name);
    group.add(&type_row);
    group.add(&extra);
    page.add(&group);

    let save = gtk::Button::with_label("Save");
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
                    client.update_remote(&remote_name, serde_json::Value::Object(params))
                } else {
                    client.create_remote(&remote_name, &r#type, serde_json::Value::Object(params))
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
                        toast.add_response("ok", "OK");
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
    profile_row.set_title("Profile");
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    profile_row.set_model(Some(&gtk::StringList::new(&refs)));

    let initial = profiles
        .get(&names[0])
        .cloned()
        .unwrap_or_else(ProfileConfig::default);
    let rclone = flatten_rclone(&initial.rclone);
    let src = adw::EntryRow::new();
    src.set_title(if op == OperationType::Copyurl {
        "URL"
    } else {
        "Source"
    });
    src.set_text(&default_source(remote, &rclone));
    let src_kind = if op != OperationType::Copyurl {
        attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
        Some(attach_path_kind(&src, remote))
    } else {
        None
    };
    let extra_sources: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    let more_src = crate::jobs::path_list(&rclone, crate::jobs::SOURCE_KEYS);
    if more_src.len() > 1 {
        for extra in more_src.iter().skip(1) {
            let row = adw::EntryRow::new();
            row.set_title("Additional source");
            row.set_text(extra);
            attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
            extra_sources.borrow_mut().push(row);
        }
    }
    let dst = adw::EntryRow::new();
    dst.set_title(match op {
        OperationType::Mount => "Mount point",
        OperationType::Serve => "Listen address",
        OperationType::Copyurl => "Destination fs",
        _ => "Destination",
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
        Some(attach_path_kind(&dst, remote))
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
    serve.set_title("Serve type");
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
    mount_type.set_title("Mount type");
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
    let vfs_row = helper_combo("VFS profile", &vfs_names, &initial.app.vfs_profile);
    let filter_row = helper_combo("Filter profile", &filter_names, &initial.app.filter_profile);
    let backend_row = helper_combo(
        "Backend profile",
        &backend_names,
        &initial.app.backend_profile,
    );
    let dry = adw::SwitchRow::new();
    dry.set_title("Dry run");
    dry.set_active(crate::jobs::is_dry_run(&rclone));

    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title("Operation flags");
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
    attach_cli_import(&flags_group, flag_rows.clone());

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

    let start = gtk::Button::with_label("Start");
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
                toast.add_toast(adw::Toast::new("Engine offline"));
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
                let err = adw::AlertDialog::new(Some("Start failed"), Some(&e));
                err.add_response("ok", "OK");
                err.present(Some(&dialog));
            } else {
                ctx.store
                    .borrow_mut()
                    .push_log(&remote, format!("started {op} {}", ids.join(", ")));
                ctx.refresh_runtime();
                toast.add_toast(adw::Toast::new(&format!("Started {op} {}", ids.join(", "))));
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
        let add_src = gtk::Button::with_label("Add source");
        {
            let extra_sources = extra_sources.clone();
            let identity = identity.clone();
            let ctx = ctx.clone();
            add_src.connect_clicked(move |_| {
                let row = adw::EntryRow::new();
                row.set_title("Additional source");
                attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
                identity.add(&row);
                extra_sources.borrow_mut().push(row);
            });
        }
        let add_row = adw::ActionRow::new();
        add_row.set_title("Multiple sources");
        add_row.add_suffix(&add_src);
        identity.add(&add_row);
    }
    if let Some(kind) = &dst_kind {
        identity.add(kind);
    }
    identity.add(&dst);
    identity.add(&{
        let row = adw::ActionRow::new();
        row.set_title("Path check");
        row.add_suffix(&dest_status);
        row
    });
    identity.add(&serve);
    identity.add(&mount_type);
    identity.add(&vfs_row);
    identity.add(&filter_row);
    identity.add(&backend_row);
    identity.add(&dry);
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
            Some("Clone failed"),
            Some("No saved settings were found for this remote."),
        );
        toast.add_response("ok", "OK");
        toast.present(Some(parent));
        return;
    };
    if let Some(client) = ctx.client() {
        if let Err(e) = client.clone_remote_config(name, &new_name) {
            let toast = adw::AlertDialog::new(Some("Clone failed"), Some(&e.to_string()));
            toast.add_response("ok", "OK");
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
    let remote = adw::EntryRow::new();
    remote.set_title(&ctx.t_or("flow.quickRun.editor.remote", "Remote"));
    let src = adw::EntryRow::new();
    src.set_title(&ctx.t_or("fileBrowser.operations.details.source", "Source"));
    attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
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
        cron.connect_changed(move |row| {
            let text = row.text().to_string();
            if text.is_empty() {
                cron_hint.set_text("");
            } else if let Err(e) = validate_cron(&text) {
                cron_hint.set_text(&e);
            } else {
                cron_hint.set_text(&describe_cron(&text));
            }
        });
    }
    let cron_presets = attach_cron_builder(&cron);
    let op_row = adw::ComboRow::new();
    op_row.set_title(&ctx.t_or("wizards.cliImport.operation", "Operation"));
    let labels: Vec<&str> = OperationType::ALL.iter().map(|o| o.as_str()).collect();
    op_row.set_model(Some(&gtk::StringList::new(&labels)));
    let auto = adw::SwitchRow::new();
    auto.set_title(&ctx.t_or("flow.quickRun.badges.autostart", "Auto start"));
    let watch = adw::SwitchRow::new();
    watch.set_title(&ctx.t_or("flow.quickRun.badges.watcher", "Watch enabled"));
    let tray = adw::SwitchRow::new();
    tray.set_title(&ctx.t_or("flow.quickRun.editor.showOnTray", "Show on tray"));
    let vfs_profile = adw::EntryRow::new();
    vfs_profile.set_title(&ctx.t_or("flow.quickRun.editor.tabVfs", "VFS profile name"));
    let filter_profile = adw::EntryRow::new();
    filter_profile.set_title(&ctx.t_or("flow.quickRun.editor.tabFilter", "Filter profile name"));
    let backend_profile = adw::EntryRow::new();
    backend_profile.set_title(&ctx.t_or("flow.quickRun.editor.tabBackend", "Backend profile name"));
    if let Some(qr) = &existing {
        name.set_text(&qr.name);
        remote.set_text(&qr.remote_name);
        let (s, d) = qr.paths();
        src.set_text(&s.unwrap_or_default());
        dst.set_text(&d.unwrap_or_default());
        cron.set_text(&qr.config.app.cron_expression);
        auto.set_active(qr.config.app.auto_start);
        watch.set_active(qr.config.app.watch_enabled);
        tray.set_active(qr.show_on_tray);
        vfs_profile.set_text(&qr.config.app.vfs_profile);
        filter_profile.set_text(&qr.config.app.filter_profile);
        backend_profile.set_text(&qr.config.app.backend_profile);
        if let Some(idx) = OperationType::ALL
            .iter()
            .position(|o| *o == qr.operation_type)
        {
            op_row.set_selected(idx as u32);
        }
    }
    group.add(&name);
    group.add(&remote);
    group.add(&op_row);
    group.add(&src);
    group.add(&dst);
    group.add(&cron);
    let cron_preset_row = adw::ActionRow::new();
    cron_preset_row.set_title("Cron schedule");
    cron_preset_row.add_suffix(&cron_presets);
    group.add(&cron_preset_row);
    group.add(&auto);
    group.add(&watch);
    group.add(&tray);
    group.add(&vfs_profile);
    group.add(&filter_profile);
    group.add(&backend_profile);
    let dry = adw::SwitchRow::new();
    dry.set_title(&ctx.t_or("detailShared.jobs.dryRun", "Dry run"));
    dry.set_active(
        existing
            .as_ref()
            .is_some_and(|qr| crate::jobs::is_dry_run(&qr.config.rclone)),
    );
    group.add(&dry);
    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title(&ctx.t_or("flow.quickRun.editor.flags", "Operation flags"));
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let serve_flag_rows: Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let initial_op = existing
        .as_ref()
        .map(|qr| qr.operation_type)
        .unwrap_or(OperationType::Sync);
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
    mount_type.set_title("Mount type");
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
    attach_cli_import(&flags_group, flag_rows.clone());
    {
        let flags_group = flags_group.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let mount_type = mount_type.clone();
        let rclone = initial_rclone.clone();
        let blocks = live_blocks.clone();
        op_row.connect_selected_notify(move |row| {
            let op = OperationType::ALL
                .get(row.selected() as usize)
                .copied()
                .unwrap_or(OperationType::Sync);
            serve.set_visible(op == OperationType::Serve);
            mount_type.set_visible(op == OperationType::Mount);
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
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let serve = serve.clone();
        let serve_types = serve_types.clone();
        let mount_types = mount_types.clone();
        let mount_type = mount_type.clone();
        let dry = dry.clone();
        save.connect_clicked(move |_| {
            let expr = cron.text().to_string();
            if !expr.is_empty() {
                if let Err(e) = validate_cron(&expr) {
                    let err = adw::AlertDialog::new(Some("Invalid cron"), Some(&e));
                    err.add_response("ok", "OK");
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
            qr.remote_name = remote.text().to_string();
            qr.operation_type = op;
            qr.config.app.auto_start = auto.is_active();
            qr.config.app.watch_enabled = watch.is_active();
            qr.config.app.cron_enabled = !expr.is_empty();
            qr.config.app.cron_expression = expr;
            qr.config.app.vfs_profile = vfs_profile.text().to_string();
            qr.config.app.filter_profile = filter_profile.text().to_string();
            qr.config.app.backend_profile = backend_profile.text().to_string();
            qr.show_on_tray = tray.is_active();
            let mut rclone = serde_json::json!({
                "srcFs": src.text().to_string(),
                "dstFs": dst.text().to_string(),
                "mountPoint": dst.text().to_string(),
                "fs": src.text().to_string(),
            });
            if dry.is_active() {
                rclone["dryRun"] = serde_json::json!(true);
            }
            if op == OperationType::Serve {
                rclone["type"] = serde_json::json!(crate::operations::selected_or(
                    &serve_types,
                    serve.selected(),
                    "http"
                ));
            }
            if op == OperationType::Mount {
                rclone["mountType"] = serde_json::json!(crate::operations::selected_or(
                    &mount_types,
                    mount_type.selected(),
                    "mount"
                ));
            }
            if let Some(obj) = rclone.as_object_mut() {
                for (field, row, type_name) in flag_rows.borrow().iter() {
                    let text = row.text().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    obj.insert(
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
                    obj.insert(
                        field.clone(),
                        crate::flags::parse_flag_value(type_name, &text),
                    );
                }
            }
            qr.config.rclone = rclone;
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
    box_.append(&cron_hint);
    box_.append(&flags_group);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn export_backup(parent: &impl IsA<gtk::Window>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("modals.export.title", "Export backup"));
    dialog.set_content_width(480);
    let categories = backup::export_categories();
    let labels: Vec<&str> = categories.iter().map(|(_, label)| *label).collect();
    let type_row = adw::ComboRow::new();
    type_row.set_title(&ctx.t_or("modals.export.selectType", "What to export"));
    type_row.set_model(Some(&gtk::StringList::new(&labels)));
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
    let note = adw::EntryRow::new();
    note.set_title(&ctx.t_or("modals.export.noteLabel", "Note"));
    let password = adw::PasswordEntryRow::new();
    password.set_title("Zip password (optional, 4+ chars)");
    let secrets = adw::SwitchRow::new();
    secrets.set_title("Include secrets in rclone dump");
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
    backend_row.set_subtitle("Dump remotes from this RC instance");
    backend_row.set_model(Some(&gtk::StringList::new(&backend_refs)));
    let save = gtk::Button::with_label("Choose file…");
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
                                Ok(_) => toast.add_toast(adw::Toast::new("Backup exported")),
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
    let analysis = backup::analyze_backup(&path).ok();
    let summary = analysis
        .as_ref()
        .map(|a| {
            format!(
                "Version {} · {} · remotes: {}\nsettings={} store={} rclone={} encrypted={}",
                a.manifest.version,
                a.manifest.export_type,
                a.manifest.remotes.join(", "),
                a.has_settings,
                a.has_store,
                a.has_rclone_config,
                a.manifest.encrypted
            )
        })
        .unwrap_or_else(|| {
            "Could not read the zip without a password. Enter it below to restore.".into()
        });
    let dialog = adw::AlertDialog::new(Some("Restore backup"), Some(&summary));
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_placeholder_text(Some("Zip password (if encrypted)"));
    let mut scope_labels = vec!["All remotes".to_string()];
    if let Some(analysis) = &analysis {
        scope_labels.extend(analysis.manifest.remotes.iter().cloned());
    }
    let scope_refs: Vec<&str> = scope_labels.iter().map(|s| s.as_str()).collect();
    let scope = gtk::DropDown::from_strings(&scope_refs);
    let as_name = adw::EntryRow::new();
    as_name.set_title("Restore as (optional rename)");
    let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
    extra.append(&password);
    extra.append(&scope);
    extra.append(&as_name);
    dialog.set_extra_child(Some(&extra));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("restore", "Restore");
    dialog.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, response| {
        if response != "restore" {
            return;
        }
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
                                let _ = client.create_remote(name, t, cfg.clone());
                            }
                        }
                    }
                }
                ctx.persist();
                toast.add_toast(adw::Toast::new("Backup restored"));
                on_done();
            }
            Err(e) => toast.add_toast(adw::Toast::new(&e)),
        }
    });
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
        (
            ctx.t_or("modals.jobDetail.fields.type", "Type"),
            format!(
                "{:?}",
                crate::operations::FileTypeCategory::from_name(name, false)
            ),
        ),
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
    if let Some(client) = ctx.client() {
        if let Ok(Some(stat)) = client.stat(&fs, path) {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("fileBrowser.properties.kind", "Kind"));
            row.set_subtitle(&format!(
                "{} · {}{}",
                if stat.is_dir { "Directory" } else { "File" },
                crate::rclone::format_bytes(stat.size),
                if stat.mime.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", stat.mime)
                }
            ));
            list.append(&row);
        }
        if let Ok(about) = client.about(&fs) {
            let used = about.get("used").and_then(|x| x.as_i64()).unwrap_or(-1);
            let total = about.get("total").and_then(|x| x.as_i64()).unwrap_or(-1);
            let free = about.get("free").and_then(|x| x.as_i64()).unwrap_or(-1);
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("fileBrowser.properties.storage", "Disk usage"));
            row.set_subtitle(&format!(
                "used {} · free {} · total {}",
                crate::rclone::format_bytes(used),
                crate::rclone::format_bytes(free),
                crate::rclone::format_bytes(total)
            ));
            list.append(&row);
        }
        if remote == "local" {
            if let Ok(du) = client.du(Some(path)) {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or("fileBrowser.properties.localDisk", "Local disk"));
                row.set_subtitle(&format!(
                    "{} · used {} · free {} · total {}",
                    du.dir,
                    crate::rclone::format_bytes(du.used),
                    crate::rclone::format_bytes(du.free),
                    crate::rclone::format_bytes(du.total)
                ));
                list.append(&row);
            }
        }
        if let Ok(size) = client.size(&fs, path) {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or("fileBrowser.properties.size", "Size"));
            let count = size.get("count").and_then(|x| x.as_i64());
            let bytes = size.get("bytes").and_then(|x| x.as_i64()).unwrap_or(-1);
            row.set_subtitle(&match count {
                Some(n) => format!("{} · {n} objects", crate::rclone::format_bytes(bytes)),
                None => crate::rclone::format_bytes(bytes),
            });
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
                row.add_suffix(&calc);
                list.append(&row);
            }
        }
        if remote != "local" && info.as_ref().is_none_or(|i| i.has_feature("PublicLink")) {
            let link_row = adw::ActionRow::new();
            link_row.set_title(&ctx.t_or("fileBrowser.properties.publicLink", "Public link"));
            link_row.set_subtitle(&ctx.t_or("fileBrowser.properties.creatingLink", "Not created"));
            list.append(&link_row);
            let expire = adw::EntryRow::new();
            expire.set_title(&ctx.t_or(
                "fileBrowser.properties.expires",
                "Link expiry (e.g. 1d, 7d, 1M)",
            ));
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
                        let exp = expire.text().to_string();
                        match client.public_link_ex(
                            &fs,
                            &path,
                            (!exp.is_empty()).then_some(exp.as_str()),
                            false,
                        ) {
                            Ok(url) if !url.is_empty() => {
                                link_row.set_subtitle(&url);
                                if let Some(display) = gtk::gdk::Display::default() {
                                    display.clipboard().set_text(&url);
                                }
                            }
                            Ok(_) => link_row.set_subtitle("Remote did not return a public link"),
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
                            Ok(_) => link_row.set_subtitle("Link removed"),
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
    let open_src =
        gtk::Button::with_label(&ctx.t_or("modals.jobDetail.openSource", "Open source in Files"));
    let open_dst = gtk::Button::with_label(&ctx.t_or(
        "modals.jobDetail.openDestination",
        "Open destination in Files",
    ));
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        open_src.connect_clicked(move |_| {
            if let Some(job) = ctx.snapshot.borrow().jobs.iter().find(|j| j.id == job_id) {
                if let Some((remote, path)) = browse_target(&job.src) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                    return;
                }
            }
            if let Some(job) = ctx
                .store
                .borrow()
                .job_history
                .iter()
                .find(|j| j.id == job_id)
            {
                if let Some((remote, path)) = browse_target(&job.src) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                }
            }
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        open_dst.connect_clicked(move |_| {
            if let Some(job) = ctx.snapshot.borrow().jobs.iter().find(|j| j.id == job_id) {
                if let Some((remote, path)) = browse_target(&job.dst) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                    return;
                }
            }
            if let Some(job) = ctx
                .store
                .borrow()
                .job_history
                .iter()
                .find(|j| j.id == job_id)
            {
                if let Some((remote, path)) = browse_target(&job.dst) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                }
            }
        });
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&stop);
    actions.append(&stop_group);
    actions.append(&reset);
    actions.append(&delete);
    actions.append(&open_src);
    actions.append(&open_dst);
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
            let speed = job
                .stats
                .get("speed")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let bytes = job.stats.get("bytes").and_then(|x| x.as_i64()).unwrap_or(0);
            let eta = job
                .stats
                .get("eta")
                .cloned()
                .unwrap_or(serde_json::json!("—"));
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
                    ctx.t_or("modals.jobDetail.fields.duration", "Duration"),
                    format!("{:.1}s", job.duration),
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
                    crate::rclone::format_bytes(bytes),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.speed", "Speed"),
                    format!("{:.1} KiB/s", speed / 1024.0),
                ),
                (
                    ctx.t_or("modals.jobDetail.fields.eta", "ETA"),
                    eta.to_string(),
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
            let check_source = job
                .stats
                .get("checks")
                .or_else(|| job.output.get("results"))
                .or_else(|| job.output.get("cryptcheck").and_then(|v| v.get("results")))
                .cloned()
                .unwrap_or(serde_json::json!([]));
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
        "No active transfers"
    } else {
        "No completed transfers"
    };
    let Some(arr) = items else {
        let row = adw::ActionRow::new();
        row.set_title(empty_title);
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
        row.set_subtitle(&format!(
            "{}% · {}",
            parsed.percentage,
            crate::rclone::format_bytes(parsed.size)
        ));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        if let Some((remote, path)) = crate::transfers::browse_for(&parsed.src)
            .or_else(|| crate::transfers::browse_for(&parsed.dst))
        {
            let open = gtk::Button::from_icon_name("folder-open-symbolic");
            open.set_tooltip_text(Some("Open in Files"));
            open.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            let parent = parent.clone();
            open.connect_clicked(move |_| {
                ctx.request_browse(&remote, &path);
                parent.close();
            });
            actions.append(&open);
        }
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some("Copy path"));
        copy.set_valign(gtk::Align::Center);
        let copy_text = if !parsed.dst.is_empty() {
            parsed.dst.clone()
        } else {
            parsed.src.clone()
        };
        copy.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&copy_text);
            }
        });
        actions.append(&copy);
        if let Some((remote, path, name)) = crate::transfers::download_target(&parsed.src)
            .or_else(|| crate::transfers::download_target(&parsed.dst))
        {
            let dl = gtk::Button::from_icon_name("folder-download-symbolic");
            dl.set_tooltip_text(Some("Download"));
            dl.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            let parent = parent.clone();
            dl.connect_clicked(move |_| {
                download_file(&parent, ctx.clone(), &remote, &path, &name);
            });
            actions.append(&dl);
        }
        if let Some((remote, path)) = crate::transfers::browse_for(&parsed.src)
            .or_else(|| crate::transfers::browse_for(&parsed.dst))
        {
            let info = ctx.fs_info(&remote);
            if crate::transfers::can_public_link(&remote, info.as_ref()) {
                let link = gtk::Button::from_icon_name("emblem-shared-symbolic");
                link.set_tooltip_text(Some("Copy public URL"));
                link.set_valign(gtk::Align::Center);
                let ctx = ctx.clone();
                link.connect_clicked(move |_| {
                    let Some(client) = ctx.client() else {
                        return;
                    };
                    let (fs, remote_path) = crate::transfers::fs_and_remote(&if path.is_empty() {
                        format!("{remote}:")
                    } else if remote == "local" {
                        path.clone()
                    } else {
                        format!("{remote}:{path}")
                    });
                    if let Ok(url) = client.public_link(&fs, &remote_path) {
                        if let Some(display) = gtk::gdk::Display::default() {
                            display.clipboard().set_text(&url);
                        }
                    }
                });
                actions.append(&link);
            }
        }
        if crate::transfers::can_delete_source(job_type) && !parsed.src.is_empty() {
            let del = gtk::Button::from_icon_name("user-trash-symbolic");
            del.set_tooltip_text(Some("Delete source"));
            del.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            let parent = parent.clone();
            let src = parsed.src.clone();
            del.connect_clicked(move |_| {
                confirm_delete_path(&parent, ctx.clone(), &src, "Delete source file?");
            });
            actions.append(&del);
        }
        if crate::transfers::can_delete_dest(job_type, !active) && !parsed.dst.is_empty() {
            let del = gtk::Button::from_icon_name("edit-delete-symbolic");
            del.set_tooltip_text(Some("Delete destination"));
            del.set_valign(gtk::Align::Center);
            let ctx = ctx.clone();
            let parent = parent.clone();
            let dst = parsed.dst.clone();
            del.connect_clicked(move |_| {
                confirm_delete_path(&parent, ctx.clone(), &dst, "Delete destination file?");
            });
            actions.append(&del);
        }
        row.add_suffix(&actions);
        list.append(&row);
        shown += 1;
    }
    if shown == 0 {
        let row = adw::ActionRow::new();
        row.set_title(empty_title);
        list.append(&row);
    }
}

fn confirm_delete_path(parent: &adw::Dialog, ctx: AppCtx, path: &str, title: &str) {
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

fn pdf_panel(path: Option<std::path::PathBuf>, name: &str) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_start(16);
    box_.set_margin_end(16);
    let size = path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| crate::rclone::format_bytes(m.len() as i64))
        .unwrap_or_else(|| "—".into());
    let magic = path.as_ref().and_then(|p| {
        let mut buf = [0u8; 5];
        std::fs::File::open(p)
            .ok()
            .and_then(|mut f| {
                use std::io::Read;
                f.read_exact(&mut buf).ok().map(|_| buf)
            })
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    });
    let label = gtk::Label::new(Some(&format!(
        "{name}\nSize: {size}\n{}",
        if magic.as_deref() == Some("%PDF-") {
            "PDF document — open with the system viewer for pages and search."
        } else {
            "PDF preview uses the system viewer. Open native to display pages."
        }
    )));
    label.set_wrap(true);
    label.set_xalign(0.0);
    box_.append(&label);
    if let Some(path) = path {
        if let Some(preview) = crate::media::render_pdf_preview(&path) {
            let picture = gtk::Picture::for_filename(&preview);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_vexpand(true);
            box_.append(&picture);
        }
        let open = gtk::Button::with_label("Open native");
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
        let toggle = gtk::ToggleButton::with_label("Preview");
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
        let save = gtk::Button::with_label("Save");
        save.add_css_class("suggested-action");
        let view = view.clone();
        let local = save_path.map(|p| p.to_string());
        save.connect_clicked(move |_| {
            let buffer = view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            if let Some(path) = &local {
                if let Err(e) = std::fs::write(path, text.as_str()) {
                    log::warn!("failed to save {path}: {e}");
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
                }
            }
        });
        parent.append(&save);
    }
}

pub fn file_viewer(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
    siblings: &[String],
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(name);
    dialog.set_content_width(720);
    dialog.set_content_height(520);
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    nav.set_margin_start(12);
    nav.set_margin_end(12);
    nav.set_margin_top(8);
    let index = siblings.iter().position(|n| n == name);
    let prev = gtk::Button::from_icon_name("go-previous-symbolic");
    prev.set_tooltip_text(Some("Previous file"));
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some("Next file"));
    let pos = gtk::Label::new(Some(&match index {
        Some(i) if !siblings.is_empty() => format!("{} / {}", i + 1, siblings.len()),
        _ => name.to_string(),
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
            let Some(i) = siblings.iter().position(|n| n == name.as_str()) else {
                return;
            };
            if i == 0 {
                return;
            }
            let prev_name = siblings[i - 1].clone();
            let parent_path = crate::rclone::parent_remote_path(&path);
            let next_path = crate::rclone::join_remote_path(&parent_path, &prev_name);
            dialog.close();
            file_viewer(
                &parent,
                ctx.clone(),
                &remote,
                &next_path,
                &prev_name,
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
            let Some(i) = siblings.iter().position(|n| n == name.as_str()) else {
                return;
            };
            if i + 1 >= siblings.len() {
                return;
            }
            let next_name = siblings[i + 1].clone();
            let parent_path = crate::rclone::parent_remote_path(&path);
            let next_path = crate::rclone::join_remote_path(&parent_path, &next_name);
            dialog.close();
            file_viewer(
                &parent,
                ctx.clone(),
                &remote,
                &next_path,
                &next_name,
                &siblings,
            );
        });
    }
    nav.append(&prev);
    nav.append(&pos);
    nav.append(&next);
    let category = crate::operations::FileTypeCategory::from_name(name, false);
    let info = gtk::Label::new(Some(&format!(
        "{remote}:{path}\nType: {:?}\nUse Download to open with a system app when needed.",
        category
    )));
    info.set_wrap(true);
    info.set_margin_top(16);
    info.set_margin_start(16);
    info.set_margin_end(16);
    let open = gtk::Button::with_label("Open native");
    {
        let path = if remote == "local" {
            path.to_string()
        } else {
            remote_fs(remote, path)
        };
        open.connect_clicked(move |_| {
            let _ = open::that(&path);
        });
    }
    let download = gtk::Button::with_label("Download…");
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
    if matches!(category, crate::operations::FileTypeCategory::Archive) {
        let heading = gtk::Label::new(Some("Archive contents"));
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
                row.set_title("Archive is empty");
                archive_list.append(&row);
            }
            None => {
                let row = adw::ActionRow::new();
                row.set_title("Unable to list archive contents");
                row.set_subtitle("rclone operations/archive list failed for this file.");
                archive_list.append(&row);
            }
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_min_content_height(220);
        scroll.set_child(Some(&archive_list));
        box_.append(&scroll);
    }
    let mut preview_path = if remote == "local" {
        Some(std::path::PathBuf::from(path))
    } else {
        None
    };
    if remote != "local" {
        if let Some(client) = ctx.client() {
            let fs = remote_fs(remote, "");
            if matches!(category, crate::operations::FileTypeCategory::Text) {
                if let Ok(text) = client.cat(&fs, path, Some(crate::rclone::CAT_PREVIEW_BYTES)) {
                    info.set_text("Remote preview via operations/cat");
                    attach_text_preview(
                        &box_,
                        name,
                        &text,
                        true,
                        None,
                        Some((ctx.clone(), fs, path.to_string())),
                    );
                }
            } else {
                let dest = std::env::temp_dir().join(name);
                if client
                    .copy_file(&fs, path, "/", &dest.to_string_lossy())
                    .is_ok()
                {
                    info.set_text(&format!("Downloaded preview to {}", dest.display()));
                    preview_path = Some(dest);
                }
            }
        }
    }
    if let Some(local) = preview_path.as_ref() {
        let local_s = local.to_string_lossy();
        if matches!(category, crate::operations::FileTypeCategory::Image) {
            let picture = gtk::Picture::for_filename(local);
            picture.set_vexpand(true);
            box_.append(&picture);
        }
        if matches!(
            category,
            crate::operations::FileTypeCategory::Video | crate::operations::FileTypeCategory::Audio
        ) {
            let video = gtk::Video::for_filename(Some(local_s.as_ref()));
            video.set_vexpand(true);
            video.set_autoplay(true);
            box_.append(&video);
            if matches!(category, crate::operations::FileTypeCategory::Audio) {
                if let Some(cover) = crate::media::sibling_cover(local) {
                    let picture = gtk::Picture::for_filename(&cover);
                    picture.set_can_shrink(true);
                    picture.set_content_fit(gtk::ContentFit::Contain);
                    picture.set_size_request(-1, 180);
                    box_.append(&picture);
                }
            }
        }
        if matches!(category, crate::operations::FileTypeCategory::Pdf) {
            box_.append(&pdf_panel(Some(local.clone()), name));
        }
        if remote == "local" && matches!(category, crate::operations::FileTypeCategory::Text) {
            let text = std::fs::read_to_string(local).unwrap_or_default();
            attach_text_preview(&box_, name, &text, true, Some(&local_s), None);
        }
    }
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn remote_about(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
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
                for key in ["used", "free", "total", "trashed"] {
                    if let Some(value) = about.get(key).and_then(|x| x.as_i64()) {
                        let row = adw::ActionRow::new();
                        row.set_title(key);
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
                row.set_title("Unable to query disk usage");
                row.set_subtitle(&e.to_string());
                usage.add(&row);
            }
        }
        match client.size(&fs, "") {
            Ok(size) => {
                let row = adw::ActionRow::new();
                row.set_title("Objects");
                let count = size.get("count").and_then(|x| x.as_i64()).unwrap_or(0);
                let bytes = size.get("bytes").and_then(|x| x.as_i64()).unwrap_or(0);
                row.set_subtitle(&format!("{count} · {}", crate::rclone::format_bytes(bytes)));
                usage.add(&row);
            }
            Err(e) => {
                let row = adw::ActionRow::new();
                row.set_title("Size");
                row.set_subtitle(&e.to_string());
                usage.add(&row);
            }
        }
        let info = ctx.fs_info(remote).or_else(|| client.fs_info(&fs).ok());
        if let Some(info) = info {
            let name = adw::ActionRow::new();
            name.set_title("Name");
            name.set_subtitle(&info.name);
            usage.add(&name);
            let root = adw::ActionRow::new();
            root.set_title("Root");
            let root_text = if info.root.is_empty() {
                "/".to_string()
            } else {
                info.root.clone()
            };
            root.set_subtitle(&root_text);
            usage.add(&root);
            let precision = adw::ActionRow::new();
            precision.set_title("Timestamp precision");
            let precision_text = nanoseconds_to_duration(info.precision);
            precision.set_subtitle(&precision_text);
            usage.add(&precision);
            if info.hashes.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title("None");
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
                row.set_subtitle(if *value { "yes" } else { "no" });
                features.add(&row);
            }
            if let Some(obj) = info.metadata.as_object() {
                for (group_name, data) in obj {
                    if let Some(items) = data.as_object() {
                        for (key, meta) in items {
                            let row = adw::ActionRow::new();
                            row.set_title(&format!("{group_name}.{key}"));
                            let help = meta
                                .get("Help")
                                .or_else(|| meta.get("help"))
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            row.set_subtitle(help);
                            metadata.add(&row);
                        }
                    }
                }
            }
        }
    }
    page.add(&usage);
    page.add(&hashes);
    page.add(&features);
    page.add(&metadata);
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
            if let Err(e) = client.copy_file(&fs, &src_remote, "/", &dest.to_string_lossy()) {
                log::warn!("download failed: {e}");
            }
        },
    );
}

pub fn templates(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
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
        let apply = gtk::Button::with_label("Apply");
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
                        Some("Template applied"),
                        Some("Helper and operation profiles were updated from this template."),
                    );
                    toast.add_response("ok", "OK");
                    toast.present(Some(&parent));
                    return;
                }
                if let Some(client) = ctx.client() {
                    match client.options_set(values.clone()) {
                        Ok(_) => {
                            let toast = adw::AlertDialog::new(
                                Some("Template applied"),
                                Some("Current rclone options were updated from this template."),
                            );
                            toast.add_response("ok", "OK");
                            toast.present(Some(&parent));
                        }
                        Err(e) => {
                            let err =
                                adw::AlertDialog::new(Some("Apply failed"), Some(&e.to_string()));
                            err.add_response("ok", "OK");
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
        row.set_title("No saved templates");
        list.append(&row);
    }
    let add = gtk::Button::with_label("Save current flags as template");
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

pub fn archive_create(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str, path: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&ctx.t_or("nautilus.contextMenu.compress", "Create archive"));
    let name = adw::EntryRow::new();
    name.set_title("Archive name");
    name.set_text("archive.zip");
    let format = adw::ComboRow::new();
    format.set_title("Format");
    format.set_model(Some(&gtk::StringList::new(&[
        "zip", "tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst",
    ])));
    let start = gtk::Button::with_label("Create");
    start.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let src_path = path.to_string();
        let name = name.clone();
        start.connect_clicked(move |_| {
            let mut dest = name.text().to_string();
            if dest.is_empty() {
                dest = "archive.zip".into();
            }
            if let Some(client) = ctx.client() {
                let fs = if remote == "local" {
                    "/".into()
                } else {
                    remote_fs(&remote, "")
                };
                match client.start_job(
                    "operations/copyfile",
                    serde_json::json!({
                        "srcFs": fs,
                        "srcRemote": src_path,
                        "dstFs": fs,
                        "dstRemote": dest
                    }),
                ) {
                    Ok(id) => {
                        ctx.store
                            .borrow_mut()
                            .push_log(&remote, format!("archive job #{id} queued"));
                        dialog.close();
                    }
                    Err(e) => {
                        let err =
                            adw::AlertDialog::new(Some("Archive failed"), Some(&e.to_string()));
                        err.add_response("ok", "OK");
                        err.present(Some(&dialog));
                    }
                }
            }
        });
    }
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&format);
    page.add(&group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&page);
    box_.append(&start);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
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
    dialog.set_title(&format!("Helper profiles — {remote}"));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let kind = adw::ComboRow::new();
    kind.set_title("Category");
    kind.set_model(Some(&gtk::StringList::new(&["vfs", "filter", "backend"])));
    let name = adw::EntryRow::new();
    name.set_title("Profile name");
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
            let kind_name = ["vfs", "filter", "backend"]
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
    let save = gtk::Button::with_label("Save profile");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let name = name.clone();
        let json_view = json_view.clone();
        let kind = kind.clone();
        save.connect_clicked(move |_| {
            let kind_name = ["vfs", "filter", "backend"]
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
    let save = gtk::Button::with_label("Save");
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
    flags_group: &adw::PreferencesGroup,
    flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>>,
) {
    let cli = adw::EntryRow::new();
    cli.set_title("Import rclone CLI flags");
    let apply_cli = gtk::Button::with_label("Apply");
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
    cli_row.set_title("CLI import");
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

fn check_result_row(
    ctx: &AppCtx,
    item: &crate::checks::CheckResult,
    parent: &adw::Dialog,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&item.name);
    row.set_subtitle(&item.status);
    if let Some(kind) = item.resolve_kind() {
        let resolve = gtk::Button::with_label(if item.needs_overwrite_confirm() {
            "Overwrite"
        } else {
            "Resolve"
        });
        resolve.set_valign(gtk::Align::Center);
        let ctx = ctx.clone();
        let item = item.clone();
        let parent = parent.clone();
        resolve.connect_clicked(move |_| {
            let go = || resolve_check_item(&ctx, &item, kind);
            if item.needs_overwrite_confirm() {
                let confirm = adw::AlertDialog::new(
                    Some("Overwrite destination?"),
                    Some(&format!("Copy {} onto the destination copy.", item.name)),
                );
                confirm.add_response("cancel", "Cancel");
                confirm.add_response("ok", "Overwrite");
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
    let del_src = gtk::Button::from_icon_name("edit-delete-symbolic");
    del_src.set_tooltip_text(Some("Delete source"));
    del_src.set_valign(gtk::Align::Center);
    {
        let ctx = ctx.clone();
        let item = item.clone();
        del_src.connect_clicked(move |_| {
            delete_check_side(&ctx, &item.src_fs, &item.name);
        });
    }
    let del_dst = gtk::Button::from_icon_name("user-trash-symbolic");
    del_dst.set_tooltip_text(Some("Delete destination"));
    del_dst.set_valign(gtk::Align::Center);
    {
        let ctx = ctx.clone();
        let item = item.clone();
        del_dst.connect_clicked(move |_| {
            delete_check_side(&ctx, &item.dst_fs, &item.name);
        });
    }
    row.add_suffix(&del_src);
    row.add_suffix(&del_dst);
    row
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
    if let Err(e) = client.copy_file(src_fs, &item.name, dst_fs, &item.name) {
        log::warn!("check resolve failed: {e}");
    } else {
        ctx.refresh_runtime();
    }
}

fn delete_check_side(ctx: &AppCtx, fs: &str, path: &str) {
    let Some(client) = ctx.client() else {
        return;
    };
    if let Err(e) = client.delete_file(fs, path) {
        log::warn!("check delete failed: {e}");
    } else {
        ctx.refresh_runtime();
    }
}

fn capture_template(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Capture template");
    dialog.set_content_width(420);
    let name = adw::EntryRow::new();
    name.set_title("Name");
    name.set_text(&format!(
        "Template {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));
    let categories = [
        ("backend", "Backend / main"),
        ("filter", "Filter"),
        ("vfs", "VFS"),
        ("mount", "Mount"),
        ("copy", "Copy"),
        ("sync", "Sync"),
        ("check", "Check"),
        ("network", "Network"),
        ("other", "Other"),
    ];
    let switches: Vec<(&'static str, adw::SwitchRow)> = categories
        .into_iter()
        .map(|(id, label)| {
            let row = adw::SwitchRow::new();
            row.set_title(label);
            row.set_active(true);
            (id, row)
        })
        .collect();
    let group = adw::PreferencesGroup::new();
    group.set_title("Categories to include");
    group.add(&name);
    for (_, row) in &switches {
        group.add(row);
    }
    let save = gtk::Button::with_label("Save");
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

pub(crate) fn attach_path_kind(row: &adw::EntryRow, current_remote: &str) -> adw::ComboRow {
    let combo = adw::ComboRow::new();
    combo.set_title("Path type");
    combo.set_model(Some(&gtk::StringList::new(&[
        "Local",
        "Current remote",
        "Other remote",
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

pub(crate) fn attach_cron_presets(cron: &adw::EntryRow) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    box_.add_css_class("linked");
    box_.set_hexpand(true);
    for preset in crate::cron::PRESETS {
        let btn = gtk::Button::with_label(preset.label);
        btn.set_tooltip_text(Some(preset.cron));
        let cron = cron.clone();
        btn.connect_clicked(move |_| {
            cron.set_text(preset.cron);
        });
        box_.append(&btn);
    }
    box_
}

pub(crate) fn attach_cron_builder(cron: &adw::EntryRow) -> gtk::Box {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&attach_cron_presets(cron));

    let simple = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let freq = gtk::DropDown::from_strings(&["Daily", "Weekly", "Monthly", "Interval"]);
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    hour.set_value(9.0);
    hour.set_tooltip_text(Some("Hour"));
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    minute.set_tooltip_text(Some("Minute"));
    let dow = gtk::Entry::new();
    dow.set_placeholder_text(Some("1-5"));
    dow.set_text("1");
    dow.set_width_chars(5);
    dow.set_tooltip_text(Some("Day of week"));
    let dom = gtk::SpinButton::with_range(1.0, 31.0, 1.0);
    dom.set_tooltip_text(Some("Day of month"));
    let interval = gtk::SpinButton::with_range(1.0, 24.0, 1.0);
    interval.set_value(6.0);
    interval.set_tooltip_text(Some("Every N hours"));
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
    name.set_title("Name");
    name.set_text(
        existing
            .as_ref()
            .map(|r| r.name.as_str())
            .unwrap_or("New rule"),
    );
    let enabled = adw::SwitchRow::new();
    enabled.set_title("Enabled");
    enabled.set_active(existing.as_ref().map(|r| r.enabled).unwrap_or(true));
    let auto_ack = adw::SwitchRow::new();
    auto_ack.set_title("Auto-acknowledge");
    auto_ack.set_active(
        existing
            .as_ref()
            .map(|r| r.auto_acknowledge)
            .unwrap_or(false),
    );
    let severity = adw::ComboRow::new();
    severity.set_title("Minimum severity");
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
    cooldown.set_title("Cooldown (seconds)");
    cooldown.set_text(
        &existing
            .as_ref()
            .map(|r| r.cooldown_secs.to_string())
            .unwrap_or_else(|| "0".into()),
    );
    let remotes = adw::EntryRow::new();
    remotes.set_title("Remote filter (comma-separated)");
    remotes.set_text(
        &existing
            .as_ref()
            .map(|r| r.remote_filter.join(", "))
            .unwrap_or_default(),
    );
    let backends = adw::EntryRow::new();
    backends.set_title("Backend filter (comma-separated)");
    backends.set_text(
        &existing
            .as_ref()
            .map(|r| r.backend_filter.join(", "))
            .unwrap_or_default(),
    );
    let profiles = adw::EntryRow::new();
    profiles.set_title("Profile filter (comma-separated)");
    profiles.set_text(
        &existing
            .as_ref()
            .map(|r| r.profile_filter.join(", "))
            .unwrap_or_default(),
    );
    let origins = adw::EntryRow::new();
    origins.set_title("Origin filter (comma-separated)");
    origins.set_text(
        &existing
            .as_ref()
            .map(|r| r.origin_filter.join(", "))
            .unwrap_or_default(),
    );
    let event_switches: Vec<(AlertEventKind, adw::SwitchRow)> = EVENT_KINDS
        .iter()
        .map(|kind| {
            let row = adw::SwitchRow::new();
            row.set_title(&format!("Event: {}", kind.as_str()));
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

    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete");
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
        let remotes = remotes.clone();
        let backends = backends.clone();
        let profiles = profiles.clone();
        let origins = origins.clone();
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
            rule.remote_filter = split_csv(&remotes.text());
            rule.backend_filter = split_csv(&backends.text());
            rule.profile_filter = split_csv(&profiles.text());
            rule.origin_filter = split_csv(&origins.text());
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
    general.set_title("Rule");
    general.add(&name);
    general.add(&enabled);
    general.add(&auto_ack);
    general.add(&severity);
    general.add(&cooldown);
    general.add(&remotes);
    general.add(&backends);
    general.add(&profiles);
    general.add(&origins);
    page.add(&general);
    let events = adw::PreferencesGroup::new();
    events.set_title("Events");
    for (_, row) in &event_switches {
        events.add(row);
    }
    page.add(&events);
    let actions = adw::PreferencesGroup::new();
    actions.set_title("Actions");
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
    name.set_title("Name");
    name.set_text(
        existing
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or("New action"),
    );
    let enabled = adw::SwitchRow::new();
    enabled.set_title("Enabled");
    enabled.set_active(existing.as_ref().map(|a| a.enabled).unwrap_or(true));
    let kind = adw::ComboRow::new();
    kind.set_title("Kind");
    kind.set_model(Some(&gtk::StringList::new(ACTION_KINDS)));
    if let Some(action) = &existing {
        if let Some(idx) = ACTION_KINDS.iter().position(|k| *k == action.kind) {
            kind.set_selected(idx as u32);
        }
    }
    let url = adw::EntryRow::new();
    url.set_title("URL");
    let method = adw::EntryRow::new();
    method.set_title("Method");
    let token = adw::PasswordEntryRow::new();
    token.set_title("Token");
    let extra = adw::EntryRow::new();
    extra.set_title("Extra");
    let body = adw::EntryRow::new();
    body.set_title("Body template");
    if let Some(action) = &existing {
        url.set_text(&action_cfg(action, &["url", "broker_url", "smtp_server"]));
        method.set_text(&action_cfg(action, &["method", "topic", "smtp_port"]));
        token.set_text(&action_cfg(
            action,
            &["bot_token", "password", "apikey", "username"],
        ));
        extra.set_text(&action_cfg(
            action,
            &["chat_id", "phone", "from", "command", "to"],
        ));
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
    retries.set_title("Retry count");
    retries.set_subtitle("Extra delivery attempts after a failure");
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
    let test = gtk::Button::with_label("Test");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete");
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
        let body = body.clone();
        let retries = retries.clone();
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
                    body: body.text().to_string(),
                    retry_count: retries.value() as u32,
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
            toast.add_response("ok", "OK");
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
    let sync_fields = {
        let url = url.clone();
        let method = method.clone();
        let token = token.clone();
        let extra = extra.clone();
        let body = body.clone();
        move |selected: &str| match selected {
            "os_toast" => {
                url.set_visible(false);
                method.set_visible(false);
                token.set_visible(false);
                extra.set_visible(false);
                body.set_visible(true);
                body.set_title("Toast body template");
            }
            "webhook" => {
                url.set_visible(true);
                url.set_title("Webhook URL");
                method.set_visible(true);
                method.set_title("HTTP method");
                token.set_visible(false);
                extra.set_visible(false);
                body.set_visible(true);
                body.set_title("JSON / body template");
            }
            "telegram" => {
                url.set_visible(false);
                method.set_visible(false);
                token.set_visible(true);
                token.set_title("Bot token");
                extra.set_visible(true);
                extra.set_title("Chat ID");
                body.set_visible(true);
                body.set_title("Message template");
            }
            "whatsapp" => {
                url.set_visible(true);
                url.set_title("Gateway URL (optional)");
                method.set_visible(false);
                token.set_visible(true);
                token.set_title("API key");
                extra.set_visible(true);
                extra.set_title("Phone number");
                body.set_visible(true);
                body.set_title("Message template");
            }
            "script" => {
                url.set_visible(false);
                method.set_visible(false);
                token.set_visible(false);
                extra.set_visible(true);
                extra.set_title("Command");
                body.set_visible(true);
                body.set_title("Stdin template");
            }
            "email" => {
                url.set_visible(true);
                url.set_title("SMTP host");
                method.set_visible(true);
                method.set_title("SMTP port");
                token.set_visible(true);
                token.set_title("Password");
                extra.set_visible(true);
                extra.set_title("From / to address");
                body.set_visible(true);
                body.set_title("Body template");
            }
            "mqtt" => {
                url.set_visible(true);
                url.set_title("Broker URL");
                method.set_visible(true);
                method.set_title("Topic");
                token.set_visible(true);
                token.set_title("Password");
                extra.set_visible(false);
                body.set_visible(true);
                body.set_title("Payload template");
            }
            _ => {}
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
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&enabled);
    group.add(&kind);
    group.add(&url);
    group.add(&method);
    group.add(&token);
    group.add(&extra);
    group.add(&body);
    group.add(&retries);
    group.add(&keys);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&test);
    buttons.append(&save);
    buttons.append(&delete);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
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

fn split_csv(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn repair(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
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
        row.set_title("No issues detected");
        row.set_subtitle("rclone, FUSE, config, and the RC engine look healthy.");
        list.append(&row);
    }
    for issue in issues {
        let row = adw::ActionRow::new();
        row.set_title(&issue.title);
        row.set_subtitle(&issue.detail);
        let btn = gtk::Button::with_label(&issue.action);
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
                crate::repair::RepairKind::FuseMissing => match crate::mount_plugin::install() {
                    Ok(msg) => toast.add_toast(adw::Toast::new(&msg)),
                    Err(e) => {
                        let help = adw::AlertDialog::new(
                            Some(&format!("Install {}", crate::mount_plugin::plugin_label())),
                            Some(&e),
                        );
                        help.add_response("ok", "OK");
                        help.present(Some(&parent));
                    }
                },
                crate::repair::RepairKind::PasswordRequired => {
                    preferences(&parent, ctx.clone());
                }
                crate::repair::RepairKind::AuthFailed => {
                    backends(&parent, ctx.clone());
                }
                crate::repair::RepairKind::ConfigUnreadable => {
                    restore_or_pick_config(&parent, ctx.clone());
                }
                crate::repair::RepairKind::EngineUnreachable => {
                    ctx.restart_engine();
                    toast.add_toast(adw::Toast::new("Engine restart requested"));
                }
            });
        }
        row.add_suffix(&btn);
        list.append(&row);
    }
    let install = gtk::Button::with_label("Install rclone");
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let parent = parent.clone();
        install.connect_clicked(move |_| {
            install_rclone_update(&parent, ctx.clone(), toast.clone());
            ctx.restart_engine();
        });
    }
    let browse = gtk::Button::with_label("Choose binary…");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        browse.connect_clicked(move |_| pick_rclone_binary(&parent, ctx.clone()));
    }
    let config_btn = gtk::Button::with_label("Choose rclone.conf…");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        config_btn.connect_clicked(move |_| restore_or_pick_config(&parent, ctx.clone()));
    }
    let restart = gtk::Button::with_label("Restart engine");
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
    mode.set_title("Mode");
    mode.set_model(Some(&gtk::StringList::new(&[
        "Template",
        "Find and replace",
    ])));
    let template = adw::EntryRow::new();
    template.set_title("Template");
    template.set_text("[Original file name]");
    let find = adw::EntryRow::new();
    find.set_title("Find");
    let replace = adw::EntryRow::new();
    replace.set_title("Replace with");
    let start = adw::EntryRow::new();
    start.set_title("Counter start");
    start.set_text("1");
    let step = adw::EntryRow::new();
    step.set_title("Counter step");
    step.set_text("1");
    let pad = adw::EntryRow::new();
    pad.set_title("Counter padding");
    pad.set_text("2");
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
                case_sensitive: false,
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
    refresh_preview();
    let apply = gtk::Button::with_label("Rename");
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
                case_sensitive: false,
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
                    err.add_response("ok", "OK");
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
    group.add(&find);
    group.add(&replace);
    group.add(&start);
    group.add(&step);
    group.add(&pad);
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
