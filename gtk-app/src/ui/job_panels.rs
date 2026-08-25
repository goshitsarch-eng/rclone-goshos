use super::AppCtx;
use crate::jobs::{format_seconds, stats_f64, stats_i64};
use crate::store::JobInfo;
use adw::prelude::*;
use chrono::{Local, Utc};

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
