//! Compact and detailed automation cards matching Angular `app-automation-card`.

use super::AppCtx;
use crate::automation::{AutomationRecord, AutomationStatus};
use adw::prelude::*;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AutomationCardKind {
    Compact,
    Detailed,
}

pub fn compact_card(
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    record: &AutomationRecord,
    on_activate: Option<Rc<dyn Fn()>>,
) -> gtk::Widget {
    build_card(ctx, toast, record, AutomationCardKind::Compact, on_activate)
}

pub fn detailed_card(
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    record: &AutomationRecord,
) -> gtk::Widget {
    build_card(ctx, toast, record, AutomationCardKind::Detailed, None)
}

pub fn detailed_carousel(
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    records: &[AutomationRecord],
    selected: Option<&str>,
) -> gtk::Widget {
    if records.len() <= 1 {
        return match records.first() {
            Some(record) => detailed_card(ctx, toast, record),
            None => gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
        };
    }
    let ids: Vec<String> = records.iter().map(|r| r.id.clone()).collect();
    let index = Rc::new(Cell::new(crate::automation::carousel_index(&ids, selected)));
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    for record in records {
        stack.add_named(&detailed_card(ctx, toast, record), Some(&record.id));
    }
    stack.set_visible_child_name(&ids[index.get().min(ids.len() - 1)]);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_margin_bottom(6);
    let prev = gtk::Button::from_icon_name("go-previous-symbolic");
    prev.set_tooltip_text(Some(&ctx.t_or(
        "dashboard.generalDetail.previousAutomationTooltip",
        "Previous automation",
    )));
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some(&ctx.t_or(
        "dashboard.generalDetail.nextAutomationTooltip",
        "Next automation",
    )));
    let label = gtk::Label::new(None);
    label.set_hexpand(true);
    label.add_css_class("dim-label");
    let sync = {
        let stack = stack.clone();
        let ids = ids.clone();
        let index = index.clone();
        let label = label.clone();
        let ctx = ctx.clone();
        Rc::new(move || {
            let i = index.get().min(ids.len().saturating_sub(1));
            stack.set_visible_child_name(&ids[i]);
            *ctx.selected_automation.borrow_mut() = Some(ids[i].clone());
            label.set_text(&ctx.tf(
                "dashboard.generalDetail.goToAutomation",
                &[("index", &(i + 1).to_string())],
            ));
        })
    };
    sync();
    {
        let sync = sync.clone();
        let index = index.clone();
        let len = ids.len();
        prev.connect_clicked(move |_| {
            let current = index.get();
            index.set(if current == 0 { len - 1 } else { current - 1 });
            sync();
        });
    }
    {
        let len = ids.len();
        next.connect_clicked(move |_| {
            let current = index.get();
            index.set(if current + 1 >= len { 0 } else { current + 1 });
            sync();
        });
    }
    header.append(&prev);
    header.append(&label);
    header.append(&next);

    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&header);
    wrap.append(&stack);
    wrap.upcast()
}

fn build_card(
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    record: &AutomationRecord,
    kind: AutomationCardKind,
    on_activate: Option<Rc<dyn Fn()>>,
) -> gtk::Widget {
    let paused = ctx.store.borrow().is_automation_paused(&record.id);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.set_margin_top(2);
    card.set_margin_bottom(2);
    card.set_margin_start(2);
    card.set_margin_end(2);

    let header = adw::ActionRow::new();
    header.set_title(&record.name);
    header.set_subtitle(&format!(
        "{} · {} · {}",
        record.remote,
        record.operation,
        ctx.t_or(
            crate::automation::origin_label_key(&record.id),
            if crate::automation::is_quick_run(&record.id) {
                "Quick Run"
            } else {
                "Dashboard"
            }
        )
    ));
    let status = gtk::Label::new(Some(&ctx.t_or(
        crate::automation::status_key(record.status),
        status_fallback(record.status),
    )));
    status.add_css_class("caption");
    if record.status == AutomationStatus::Failed {
        status.add_css_class("error");
    }
    status.set_valign(gtk::Align::Center);
    header.add_suffix(&status);

    let enabled = gtk::Switch::new();
    enabled.set_valign(gtk::Align::Center);
    enabled.set_tooltip_text(Some(&ctx.t_or(
        "generalOverview.automations.pauseResume",
        "Pause or resume this automation",
    )));
    enabled.set_active(!paused);
    enabled.set_sensitive(record.status != AutomationStatus::Stopping);
    {
        let ctx = ctx.clone();
        let id = record.id.clone();
        enabled.connect_active_notify(move |switch| {
            let mut store = ctx.store.borrow_mut();
            let paused = store.is_automation_paused(&id);
            if switch.is_active() == paused {
                store.toggle_automation_paused(&id);
                drop(store);
                ctx.persist();
            }
        });
    }
    header.add_suffix(&enabled);

    if kind == AutomationCardKind::Compact {
        let run = gtk::Button::from_icon_name("media-playback-start-symbolic");
        run.set_valign(gtk::Align::Center);
        run.set_tooltip_text(Some(
            &ctx.t_or("generalOverview.automations.runNow", "Run now"),
        ));
        bind_run(ctx, toast, record, &run);
        header.add_suffix(&run);
        if let Some(on_activate) = on_activate {
            // `AdwActionRow::activated` only fires for a row that is a direct
            // child of a GtkListBox; this one lives inside a card Box, so the
            // signal never reached the caller and clicking the card did
            // nothing. Drive it from a click gesture instead.
            header.set_activatable(true);
            let click = gtk::GestureClick::new();
            click.set_button(gtk::gdk::BUTTON_PRIMARY);
            click.connect_released(move |gesture, n_press, _, _| {
                if n_press != 1 {
                    return;
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
                on_activate();
            });
            header.add_controller(click);
        }
    }

    card.append(&header);

    if kind == AutomationCardKind::Compact {
        card.append(&compact_footer(ctx, record));
    } else {
        card.append(&detailed_body(ctx, record));
    }
    card.upcast()
}

fn compact_footer(ctx: &AppCtx, record: &AutomationRecord) -> gtk::Widget {
    let (ok, fail, stop, runs) = crate::automation::stat_counts(record);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer.set_margin_start(12);
    footer.set_margin_end(12);
    footer.set_margin_bottom(8);
    footer.append(&stat_chip(
        ctx,
        "emblem-ok-symbolic",
        ok,
        "dashboard.generalDetail.successful",
        "Successful",
    ));
    footer.append(&stat_chip(
        ctx,
        "dialog-warning-symbolic",
        fail,
        "dashboard.generalDetail.failedCount",
        "Failed",
    ));
    footer.append(&stat_chip(
        ctx,
        "media-playback-stop-symbolic",
        stop,
        "dashboard.generalDetail.stoppedCount",
        "Stopped",
    ));
    footer.append(&stat_chip(
        ctx,
        "document-open-recent-symbolic",
        runs,
        "dashboard.generalDetail.totalRuns",
        "Total Runs",
    ));
    let next = gtk::Label::new(Some(&format!(
        "{} {}",
        ctx.t_or("generalOverview.automations.nextRun", "Next Run:"),
        next_run_display(ctx, record)
    )));
    next.add_css_class("dim-label");
    next.set_hexpand(true);
    next.set_xalign(1.0);
    footer.append(&next);
    footer.upcast()
}

fn detailed_body(ctx: &AppCtx, record: &AutomationRecord) -> gtk::Widget {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_start(8);
    body.set_margin_end(8);
    body.set_margin_bottom(8);

    let schedule = adw::PreferencesGroup::new();
    schedule.set_title(&ctx.t_or("dashboard.generalDetail.schedule", "Schedule"));
    if record.cron_enabled && !record.cron.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("dashboard.generalDetail.cronExpression", "Cron Expression:"));
        row.set_subtitle(&record.cron);
        row.set_tooltip_text(Some(&crate::rclone::describe_cron_i18n(
            &record.cron,
            &ctx.i18n.borrow(),
        )));
        schedule.add(&row);
    }
    schedule.add(&labeled_row(
        ctx,
        "dashboard.generalDetail.nextRunLabel",
        "Next Run:",
        &next_run_display(ctx, record),
    ));
    schedule.add(&labeled_row(
        ctx,
        "dashboard.generalDetail.lastRunLabel",
        "Last Run:",
        &crate::automation::last_run_text(record)
            .unwrap_or_else(|| ctx.t_or("automation.lastRun.never", "Never")),
    ));
    if record.watch_enabled {
        schedule.add(&labeled_row(
            ctx,
            "automation.monitoring.realtime",
            "Real-time Monitoring",
            &ctx.t_or("automation.monitoring.watcherActive", "Watcher active"),
        ));
        schedule.add(&labeled_row(
            ctx,
            "automation.monitoring.debounce",
            "Debounce Delay",
            &format!(
                "{} {}",
                record.watch_delay,
                ctx.t_or("automation.monitoring.seconds", "seconds")
            ),
        ));
        if record.watch_changed_only {
            schedule.add(&labeled_row(
                ctx,
                "automation.monitoring.syncScope",
                "Sync scope",
                &ctx.t_or(
                    "automation.monitoring.changedOnlyActive",
                    "Changed files only",
                ),
            ));
        }
    }
    body.append(&schedule);

    let stats = adw::PreferencesGroup::new();
    stats.set_title(&ctx.t_or("dashboard.generalDetail.statistics", "Statistics"));
    let (ok, fail, stop, runs) = crate::automation::stat_counts(record);
    stats.add(&labeled_row(
        ctx,
        "dashboard.generalDetail.successful",
        "Successful",
        &ok.to_string(),
    ));
    stats.add(&labeled_row(
        ctx,
        "dashboard.generalDetail.failedCount",
        "Failed",
        &fail.to_string(),
    ));
    stats.add(&labeled_row(
        ctx,
        "dashboard.generalDetail.stoppedCount",
        "Stopped",
        &stop.to_string(),
    ));
    stats.add(&labeled_row(
        ctx,
        "dashboard.generalDetail.totalRuns",
        "Total Runs",
        &runs.to_string(),
    ));
    body.append(&stats);

    let paths = crate::automation::path_rows(record);
    if !paths.is_empty() {
        let group = adw::PreferencesGroup::new();
        group.set_title(&ctx.t_or("dashboard.generalDetail.paths", "Paths"));
        for (key, path) in paths {
            let row = adw::ActionRow::new();
            row.set_title(&ctx.t_or(
                key,
                if key.contains("source") {
                    "Source:"
                } else {
                    "Destination:"
                },
            ));
            row.set_subtitle(&path);
            let open = gtk::Button::from_icon_name("folder-open-symbolic");
            open.set_valign(gtk::Align::Center);
            open.set_tooltip_text(Some(
                &ctx.t_or("overviews.remoteCard.browse", "Open in Files"),
            ));
            let ctx_open = ctx.clone();
            let remote = record.remote.clone();
            let raw = path.clone();
            open.connect_clicked(move |_| ctx_open.open_typed_path(&remote, &raw));
            row.add_suffix(&open);
            group.add(&row);
        }
        body.append(&group);
    }

    if let Some(error) = record.last_error.as_deref().filter(|e| !e.is_empty()) {
        let group = adw::PreferencesGroup::new();
        group.set_title(&ctx.t_or("dashboard.generalDetail.lastError", "Last Error"));
        let row = adw::ActionRow::new();
        row.set_subtitle(error);
        row.set_title(&ctx.t_or("dashboard.generalDetail.lastError", "Last Error"));
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_valign(gtk::Align::Center);
        copy.set_tooltip_text(Some(&ctx.t_or("common.copy", "Copy")));
        let error = error.to_string();
        copy.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&error);
            }
        });
        row.add_suffix(&copy);
        group.add(&row);
        body.append(&group);
    }

    if let Some(job) = record.current_job_id.as_deref().filter(|id| !id.is_empty()) {
        let group = adw::PreferencesGroup::new();
        group.set_title(&ctx.t_or(
            "dashboard.generalDetail.currentlyRunning",
            "Currently Running",
        ));
        group.add(&labeled_row(
            ctx,
            "dashboard.generalDetail.jobIdLabel",
            "Job ID:",
            job,
        ));
        body.append(&group);
    }

    body.upcast()
}

fn labeled_row(ctx: &AppCtx, key: &str, fallback: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&ctx.t_or(key, fallback));
    row.set_subtitle(value);
    row
}

fn stat_chip(ctx: &AppCtx, icon: &str, value: u64, key: &str, fallback: &str) -> gtk::Box {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(14);
    let label = gtk::Label::new(Some(&value.to_string()));
    chip.set_tooltip_text(Some(&ctx.t_or(key, fallback)));
    chip.append(&image);
    chip.append(&label);
    chip
}

fn next_run_display(ctx: &AppCtx, record: &AutomationRecord) -> String {
    match crate::automation::next_run_key(record) {
        Some(key) => ctx.t_or(
            key,
            match record.status {
                AutomationStatus::Disabled => "Automation is disabled",
                AutomationStatus::Stopping => "Disabling after current run",
                _ => "Not scheduled",
            },
        ),
        None => crate::automation::next_run_text(record),
    }
}

fn status_fallback(status: AutomationStatus) -> &'static str {
    match status {
        AutomationStatus::Enabled => "Enabled",
        AutomationStatus::Disabled => "Disabled",
        AutomationStatus::Running => "Running",
        AutomationStatus::Failed => "Failed",
        AutomationStatus::Stopping => "Stopping",
    }
}

fn bind_run(
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    record: &AutomationRecord,
    button: &gtk::Button,
) {
    let ctx = ctx.clone();
    let toast = toast.clone();
    let record = record.clone();
    button.connect_clicked(move |_| {
        if let Some(client) = ctx.client() {
            let mut store = ctx.store.borrow_mut();
            match crate::automation::fire(&client, &mut store, &record, chrono::Utc::now(), None) {
                Ok(_) => toast.add_toast(adw::Toast::new(&ctx.tf(
                    "notification.title.jobStarted",
                    &[("type", record.operation.api_label())],
                ))),
                Err(e) => toast.add_toast(adw::Toast::new(&ctx.translate_error(&e))),
            }
        }
        ctx.persist();
        ctx.refresh_runtime();
    });
}
