use super::AppCtx;
use crate::jobs::{
    format_seconds, job_panel_row, job_status_key, job_transfer_caption, overview_job_stats,
    stats_f64, stats_i64,
};
use crate::navigation::NavTarget;
use crate::rclone::format_bytes;
use crate::store::JobInfo;
use adw::prelude::*;
use chrono::{Local, Utc};
use gtk::prelude::*;
use serde_json::Value;

pub fn job_info_group(ctx: &AppCtx, job: &JobInfo) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("detailShared.jobInfo.title", "Job Information"));
    if job.dry_run {
        group.set_description(Some(&ctx.t_or("dashboard.appDetail.dryRun", "Dry Run")));
    }
    let started = job
        .start_time
        .with_timezone(&Local)
        .format("%b %d, %Y %H:%M:%S")
        .to_string();
    let duration = if job.duration > 0.0 {
        format_seconds(job.duration)
    } else {
        format_seconds((Utc::now() - job.start_time).num_seconds() as f64)
    };
    let finished = if crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job) {
        String::new()
    } else {
        job.start_time
            .checked_add_signed(chrono::TimeDelta::seconds(job.duration.round() as i64))
            .unwrap_or(job.start_time)
            .with_timezone(&Local)
            .format("%b %d, %Y %H:%M:%S")
            .to_string()
    };
    for (title, value) in [
        (
            ctx.t_or("detailShared.jobInfo.type", "Job Type"),
            job.operation.clone(),
        ),
        (
            ctx.t_or("detailShared.jobInfo.id", "Job ID"),
            format!("# {}", job.id),
        ),
        (
            ctx.t_or("detailShared.jobInfo.status", "Job Status"),
            job.status.clone(),
        ),
        (ctx.t_or("detailShared.jobInfo.started", "Started"), started),
        (
            ctx.t_or("detailShared.jobInfo.finished", "Finished"),
            finished,
        ),
        (
            ctx.t_or("detailShared.jobInfo.duration", "Duration"),
            duration,
        ),
    ] {
        if value.is_empty() {
            continue;
        }
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&value);
        group.add(&row);
    }
    group
}

pub fn job_stats_group(ctx: &AppCtx, job: &JobInfo) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let heading = gtk::Label::new(Some(&ctx.tf(
        "dashboard.appDetail.transferStatistics",
        &[("op", &job.operation)],
    )));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    box_.append(&heading);

    let bytes = stats_i64(&job.stats, &["bytes"]);
    let total_bytes = stats_i64(&job.stats, &["totalBytes"]);
    let progress = if job.progress > 0.0 {
        job.progress
    } else if total_bytes > 0 {
        (bytes as f64 / total_bytes as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let eta = stats_f64(&job.stats, &["eta"]);
    let elapsed = stats_f64(&job.stats, &["elapsedTime"]).max(job.duration);
    let eta_progress = if elapsed > 0.0 && eta > 0.0 {
        elapsed / (elapsed + eta)
    } else {
        0.0
    };
    let speed = crate::rclone::format_bytes(stats_f64(&job.stats, &["speed"]).round() as i64);
    let errors = stats_i64(&job.stats, &["errors"]);
    let last_error = job
        .error
        .clone()
        .or_else(|| {
            job.stats
                .get("lastError")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    box_.append(&progress_block(
        &ctx.t_or("dashboard.appDetail.progress", "Progress"),
        &format!("{:.0}%", progress * 100.0),
        progress,
    ));
    box_.append(&progress_block(
        &ctx.t_or("dashboard.appDetail.eta", "ETA"),
        &format_seconds(eta),
        eta_progress,
    ));

    let group = adw::PreferencesGroup::new();
    for (title, value) in [
        (
            ctx.t_or("dashboard.appDetail.speed", "Speed"),
            format!("{speed}/s"),
        ),
        (
            ctx.t_or("dashboard.appDetail.files", "Files"),
            format!(
                "{}/{}",
                stats_i64(&job.stats, &["transfers"]),
                stats_i64(&job.stats, &["totalTransfers"])
            ),
        ),
        (
            ctx.t_or("dashboard.appDetail.checks", "Checks"),
            format!(
                "{}/{}",
                stats_i64(&job.stats, &["checks"]),
                stats_i64(&job.stats, &["totalChecks"])
            ),
        ),
        (
            ctx.t_or("dashboard.appDetail.errors", "Errors"),
            errors.to_string(),
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&value);
        if title == ctx.t_or("dashboard.appDetail.errors", "Errors") && errors > 0 {
            row.add_css_class("error");
            if !last_error.is_empty() {
                row.set_tooltip_text(Some(&last_error));
            }
        }
        group.add(&row);
    }
    box_.append(&group);
    box_
}

pub fn empty_stats_group(ctx: &AppCtx) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("detailShared.stats.emptyTitle", "No statistics available"));
    group.set_description(Some(&ctx.t_or(
        "detailShared.stats.emptyMessage",
        "Statistics will be shown when an operation is running",
    )));
    group
}

pub fn overview_jobs_panel(
    ctx: &AppCtx,
    jobs: &[JobInfo],
    core_stats: &Value,
    on_changed: impl Fn() + Clone + 'static,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let stats = overview_job_stats(jobs, core_stats);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let count = if stats.active > 0 {
        ctx.tf(
            "generalOverview.jobs.activeCount",
            &[
                ("count", &stats.active.to_string()),
                ("s", if stats.active == 1 { "" } else { "s" }),
            ],
        )
    } else {
        ctx.t_or("generalOverview.jobs.noActive", "No active jobs")
    };
    let badge = gtk::Label::new(Some(&count));
    badge.add_css_class("dim-label");
    badge.set_xalign(0.0);
    badge.set_hexpand(true);
    header.append(&badge);
    root.append(&header);

    if stats.total_bytes > 0 || stats.bytes > 0 {
        root.append(&progress_block(
            &ctx.t_or("generalOverview.jobs.progress", "Progress"),
            &format!(
                "{:.0}% · {} {} {}",
                stats.completion_pct(),
                format_bytes(stats.bytes),
                ctx.t_or("generalOverview.jobs.of", "of"),
                format_bytes(stats.total_bytes)
            ),
            stats.completion_pct() / 100.0,
        ));
        root.append(&progress_block(
            &ctx.t_or("generalOverview.jobs.eta", "ETA"),
            &format_seconds(stats.eta),
            if stats.eta > 0.0 {
                (stats.completion_pct() / 100.0).clamp(0.0, 1.0)
            } else {
                0.0
            },
        ));
    }

    let grid = adw::PreferencesGroup::new();
    for (title, value, error) in [
        (
            ctx.t_or("generalOverview.jobs.speed", "Transfer Speed"),
            format!("{}/s", format_bytes(stats.speed.round() as i64)),
            false,
        ),
        (
            ctx.t_or("generalOverview.jobs.transfers", "Transfers"),
            format!("{}/{}", stats.transfers, stats.total_transfers),
            false,
        ),
        (
            ctx.t_or("generalOverview.jobs.checks", "Checks"),
            format!("{}/{}", stats.checks, stats.total_checks),
            false,
        ),
        (
            ctx.t_or("generalOverview.jobs.errors", "Errors"),
            stats.errors.to_string(),
            stats.errors > 0,
        ),
        (
            ctx.t_or("generalOverview.jobs.deletes", "Deletes"),
            stats.deletes.to_string(),
            false,
        ),
        (
            ctx.t_or("generalOverview.jobs.renames", "Renames"),
            stats.renames.to_string(),
            false,
        ),
        (
            ctx.t_or("generalOverview.jobs.serverCopies", "Server Copies"),
            stats.server_side_copies.to_string(),
            false,
        ),
        (
            ctx.t_or("generalOverview.jobs.serverMoves", "Server Moves"),
            stats.server_side_moves.to_string(),
            false,
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&value);
        if error {
            row.add_css_class("error");
        }
        grid.add(&row);
    }
    if !stats.last_error.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("generalOverview.jobs.lastError", "Last Error"));
        row.set_subtitle(&stats.last_error);
        row.add_css_class("error");
        grid.add(&row);
    }
    root.append(&grid);

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let running: Vec<_> = jobs
        .iter()
        .filter(|job| crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job))
        .cloned()
        .collect();
    if running.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(
            "generalOverview.jobs.noRunning",
            "No active jobs are currently running.",
        ));
        list.append(&row);
    } else {
        let heading = gtk::Label::new(Some(
            &ctx.t_or("generalOverview.jobs.runningJobs", "Running Jobs"),
        ));
        heading.add_css_class("heading");
        heading.set_xalign(0.0);
        root.append(&heading);
        for job in &running {
            let row = adw::ActionRow::new();
            row.set_title(&format!("{} · {}", job.operation, job.remote));
            let origin = ctx.t_or(
                crate::jobs::origin_label_key(&job.origin),
                if job.origin.is_empty() {
                    "Dashboard"
                } else {
                    job.origin.as_str()
                },
            );
            let caption = job_transfer_caption(job);
            let subtitle = if caption.is_empty() {
                format!("{} · {} · {}", job.status, job.profile, origin)
            } else {
                format!(
                    "{} · {} · {} · {}",
                    job.status, job.profile, origin, caption
                )
            };
            row.set_subtitle(&subtitle);
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(job.progress.clamp(0.0, 1.0));
            bar.set_valign(gtk::Align::Center);
            bar.set_hexpand(true);
            bar.set_width_request(96);
            row.add_suffix(&bar);
            let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
            stop.set_valign(gtk::Align::Center);
            stop.set_tooltip_text(Some(&ctx.t_or("detailShared.jobs.actions.stop", "Stop")));
            {
                let ctx = ctx.clone();
                let id = job.id;
                let on_changed = on_changed.clone();
                stop.connect_clicked(move |_| {
                    if let Some(c) = ctx.client() {
                        let _ = c.job_stop(id);
                        ctx.refresh_runtime();
                        on_changed();
                    }
                });
            }
            row.add_suffix(&stop);
            {
                let job = job.clone();
                let ctx = ctx.clone();
                row.connect_activated(move |_| {
                    ctx.request_nav(NavTarget::for_job(&job));
                });
            }
            list.append(&row);
        }
    }
    root.append(&list);
    root
}

/// Angular `app-jobs-panel`: type, `#id`, profile, status, progress, dry-run, time.
pub fn detail_jobs_panel(
    ctx: &AppCtx,
    jobs: &[JobInfo],
    on_changed: impl Fn() + Clone + 'static,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some(&ctx.t_or("detailShared.jobs.title", "Jobs")));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    let count = gtk::Label::new(Some(&jobs.len().to_string()));
    count.add_css_class("dim-label");
    header.append(&count);
    root.append(&header);

    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_activate_on_single_click(true);
    list.set_selection_mode(gtk::SelectionMode::None);
    if jobs.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or("detailShared.jobs.empty", "No jobs found"));
        list.append(&row);
        root.append(&list);
        return root;
    }
    let now = Utc::now();
    for job in jobs {
        let view = job_panel_row(job, now);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.set_margin_top(8);
        card.set_margin_bottom(8);
        card.set_margin_start(12);
        card.set_margin_end(12);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let kind = gtk::Label::new(Some(&view.operation));
        kind.add_css_class("heading");
        kind.set_xalign(0.0);
        top.append(&kind);
        let id = gtk::Label::new(Some(&view.id_label));
        id.add_css_class("dim-label");
        top.append(&id);
        if !view.profile.is_empty() {
            let profile = gtk::Label::new(Some(&view.profile));
            profile.add_css_class("dim-label");
            top.append(&profile);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        top.append(&spacer);
        let status = gtk::Label::new(Some(&ctx.t_or(job_status_key(&view.status), &view.status)));
        status.add_css_class("dim-label");
        if !view.error.is_empty() && view.status.eq_ignore_ascii_case("failed") {
            status.set_tooltip_text(Some(&view.error));
        }
        top.append(&status);
        let action = if view.can_stop {
            let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
            stop.set_valign(gtk::Align::Center);
            stop.set_tooltip_text(Some(
                &ctx.t_or("detailShared.jobs.actions.stop", "Stop Job"),
            ));
            let ctx = ctx.clone();
            let id = job.id;
            let on_changed = on_changed.clone();
            stop.connect_clicked(move |_| {
                if let Some(client) = ctx.client() {
                    let _ = client.job_stop(id);
                    ctx.refresh_runtime();
                    on_changed();
                }
            });
            stop
        } else {
            let delete = gtk::Button::from_icon_name("user-trash-symbolic");
            delete.set_valign(gtk::Align::Center);
            delete.set_tooltip_text(Some(
                &ctx.t_or("detailShared.jobs.actions.delete", "Delete Job"),
            ));
            let ctx = ctx.clone();
            let id = job.id;
            let on_changed = on_changed.clone();
            delete.connect_clicked(move |_| {
                ctx.store.borrow_mut().dismiss_job(id);
                ctx.persist();
                on_changed();
            });
            delete
        };
        top.append(&action);
        card.append(&top);

        if let Some(pct) = view.progress_pct {
            let progress = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let bar = gtk::ProgressBar::new();
            bar.set_fraction((pct as f64 / 100.0).clamp(0.0, 1.0));
            bar.set_hexpand(true);
            bar.set_valign(gtk::Align::Center);
            progress.append(&bar);
            progress.append(&gtk::Label::new(Some(&format!("{pct}%"))));
            card.append(&progress);
        }

        if view.has_footer {
            let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            if view.progress_pct.is_some() {
                let size = gtk::Label::new(Some(&format!(
                    "{} / {}",
                    format_bytes(view.bytes),
                    format_bytes(view.total_bytes)
                )));
                size.add_css_class("dim-label");
                size.set_xalign(0.0);
                size.set_hexpand(true);
                footer.append(&size);
            } else {
                let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                spacer.set_hexpand(true);
                footer.append(&spacer);
            }
            if view.dry_run {
                let dry = gtk::Label::new(Some(&ctx.t_or("detailShared.jobs.dryRun", "Dry Run")));
                dry.add_css_class("dim-label");
                footer.append(&dry);
            }
            if view.duration_secs > 0 {
                footer.append(&gtk::Label::new(Some(&format_seconds(
                    view.duration_secs as f64,
                ))));
            }
            if let Some((key, count)) = view.relative {
                let text = if count <= 0 {
                    ctx.t_or(key, "Just now")
                } else {
                    ctx.tf(key, &[("count", &count.to_string())])
                };
                let rel = gtk::Label::new(Some(&text));
                rel.add_css_class("dim-label");
                footer.append(&rel);
            }
            card.append(&footer);
        }

        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&card));
        row.set_activatable(true);
        list.append(&row);
    }
    {
        let ctx = ctx.clone();
        let jobs = jobs.to_vec();
        list.connect_row_activated(move |_, row| {
            let index = row.index();
            if index < 0 {
                return;
            }
            if let Some(job) = jobs.get(index as usize) {
                ctx.request_nav(NavTarget::Job { id: job.id });
            }
        });
    }
    root.append(&list);
    root
}

fn progress_block(title: &str, value: &str, fraction: f64) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(title));
    label.add_css_class("dim-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let amount = gtk::Label::new(Some(value));
    amount.add_css_class("heading");
    header.append(&label);
    header.append(&amount);
    let bar = gtk::ProgressBar::new();
    bar.set_fraction(fraction.clamp(0.0, 1.0));
    bar.set_hexpand(true);
    box_.append(&header);
    box_.append(&bar);
    box_
}
