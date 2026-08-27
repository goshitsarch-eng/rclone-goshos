//! Angular `app-operation-control` expander (paths, dry-run, start/stop).

use super::AppCtx;
use crate::jobs::{
    format_mount_usage_subtitle, mount_point_usage, mount_usage_ratio,
    operation_control_action_kind, operation_control_subtitle, operation_shows_mount_usage,
    operation_shows_session_flags,
};
use crate::operations::OperationType;
use crate::store::RuntimeSnapshot;
use adw::prelude::*;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

pub struct MountUsageView {
    pub title: String,
    pub subtitle: String,
    pub ratio: f64,
    pub path: String,
}

pub struct OperationControlSpec {
    pub title: String,
    pub operation: OperationType,
    pub remote_name: String,
    pub source: Option<String>,
    pub destination: Option<String>,
    pub hide_destination: bool,
    pub dest_browseable: bool,
    pub dry_run: bool,
    pub resync: bool,
    pub active: bool,
    pub busy: bool,
    pub mount_usage: Vec<MountUsageView>,
}

pub struct OperationControlHandlers {
    pub on_start: Rc<dyn Fn()>,
    pub on_stop: Rc<dyn Fn()>,
    pub on_dry_run: Option<Rc<dyn Fn(bool)>>,
    pub on_resync: Option<Rc<dyn Fn(bool)>>,
}

pub fn mount_usage_pairs(
    ctx: &AppCtx,
    name: &str,
    alias: &str,
    snap: &RuntimeSnapshot,
    destination: Option<&str>,
) -> Vec<MountUsageView> {
    let title_base = ctx.t_or("dashboard.appDetail.mountDiskUsage", "Mount point usage");
    crate::jobs::mount_usage_candidates(&snap.mounts, name, alias, destination)
        .into_iter()
        .filter_map(|(profile, path)| usage_view_for(&title_base, profile, path))
        .collect()
}

fn usage_view_for(title_base: &str, profile: &str, path: &str) -> Option<MountUsageView> {
    let (used, free, total) = mount_point_usage(path)?;
    let title = if profile.is_empty() {
        title_base.to_string()
    } else {
        format!("{title_base} · {profile}")
    };
    Some(MountUsageView {
        title,
        subtitle: format_mount_usage_subtitle(path, used, free, total),
        ratio: mount_usage_ratio(used, total),
        path: path.to_string(),
    })
}

pub fn operation_control(
    ctx: &AppCtx,
    spec: &OperationControlSpec,
    handlers: OperationControlHandlers,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_widget_name("operation-control");
    row.set_title(&if spec.title.is_empty() {
        "default".to_string()
    } else {
        spec.title.clone()
    });
    row.set_icon_name(Some(spec.operation.icon_name()));
    let dry_label = ctx.t_or("dashboard.appDetail.dryRun", "Dry run");
    row.set_subtitle(&operation_control_subtitle(
        spec.operation.api_label(),
        spec.dry_run,
        &dry_label,
    ));

    let start = gtk::Button::with_label(&ctx.t_or("actions.start", "Start"));
    start.add_css_class("suggested-action");
    start.set_valign(gtk::Align::Center);
    start.set_tooltip_text(Some(&ctx.t_or("actions.start", "Start")));
    let stop = gtk::Button::with_label(&if spec.operation == OperationType::Mount {
        ctx.t_or("actions.unmount", "Unmount")
    } else {
        ctx.t_or("actions.stop", "Stop")
    });
    stop.add_css_class("destructive-action");
    stop.set_valign(gtk::Align::Center);
    apply_busy(&start, spec.busy && !spec.active, ctx);
    apply_busy(&stop, spec.busy && spec.active, ctx);
    start.set_sensitive(!spec.active && !spec.busy);
    stop.set_sensitive(spec.active && !spec.busy);
    {
        let on_start = handlers.on_start.clone();
        start.connect_clicked(move |_| on_start());
    }
    {
        let on_stop = handlers.on_stop.clone();
        stop.connect_clicked(move |_| on_stop());
    }
    row.add_suffix(&start);
    row.add_suffix(&stop);
    {
        let start = start.clone();
        let stop = stop.clone();
        row.connect_expanded_notify(move |row| {
            let show = !row.is_expanded();
            start.set_visible(show);
            stop.set_visible(show);
        });
    }

    for path_row in path_rows(ctx, spec) {
        row.add_row(&path_row);
    }
    if operation_shows_mount_usage(
        spec.operation,
        spec.active,
        spec.destination.as_deref().unwrap_or(""),
    ) {
        for view in &spec.mount_usage {
            let usage = adw::ActionRow::new();
            usage.set_title(&view.title);
            usage.set_subtitle(&view.subtitle);
            row.add_row(&usage);
            let bar = gtk::LevelBar::new();
            bar.set_min_value(0.0);
            bar.set_max_value(1.0);
            bar.set_value(view.ratio);
            bar.set_hexpand(true);
            bar.add_css_class(crate::store::disk_usage_severity_css(view.ratio));
            let bar_row = gtk::ListBoxRow::new();
            bar_row.set_activatable(false);
            bar_row.set_child(Some(&bar));
            row.add_row(&bar_row);
            if spec.active && !view.path.is_empty() {
                let path = view.path.clone();
                let usage = usage.clone();
                let bar = bar.clone();
                glib::timeout_add_seconds_local(5, move || {
                    if !usage.is_mapped() {
                        return glib::ControlFlow::Break;
                    }
                    if let Some((used, free, total)) = mount_point_usage(&path) {
                        usage.set_subtitle(&format_mount_usage_subtitle(&path, used, free, total));
                        let ratio = mount_usage_ratio(used, total);
                        bar.set_value(ratio);
                        for class in [
                            "disk-usage-critical",
                            "disk-usage-high",
                            "disk-usage-warning",
                            "disk-usage-healthy",
                        ] {
                            bar.remove_css_class(class);
                        }
                        bar.add_css_class(crate::store::disk_usage_severity_css(ratio));
                    }
                    glib::ControlFlow::Continue
                });
            }
        }
    }

    if operation_shows_session_flags(spec.operation) {
        let dry = adw::SwitchRow::new();
        dry.set_title(&dry_label);
        if spec.dry_run {
            dry.set_subtitle(&ctx.t_or(
                "dashboard.appDetail.dryRunActive",
                "Start the next operation without writing changes",
            ));
        }
        dry.set_active(spec.dry_run);
        dry.set_sensitive(!spec.active && !spec.busy);
        if let Some(on_dry_run) = handlers.on_dry_run.clone() {
            dry.connect_active_notify(move |row| on_dry_run(row.is_active()));
        }
        row.add_row(&dry);
        if spec.operation == OperationType::Bisync {
            let resync = adw::SwitchRow::new();
            resync.set_title(&ctx.t_or("dashboard.appDetail.resync", "Resync"));
            if spec.resync {
                resync.set_subtitle(&ctx.t_or(
                    "dashboard.appDetail.resyncActive",
                    "Force a bisync resync on the next start",
                ));
            }
            resync.set_active(spec.resync);
            resync.set_sensitive(!spec.active && !spec.busy);
            if let Some(on_resync) = handlers.on_resync.clone() {
                resync.connect_active_notify(move |row| on_resync(row.is_active()));
            }
            row.add_row(&resync);
        }
    }

    let kind = operation_control_action_kind(spec.operation, spec.active);
    let full_label = match kind {
        "mount" => ctx.t_or("dashboard.appDetail.mount", "Mount"),
        "unmount" => ctx.t_or("dashboard.appDetail.unmount", "Unmount"),
        "stop" => ctx.tf(
            "dashboard.appDetail.stop",
            &[("op", spec.operation.api_label())],
        ),
        _ => ctx.tf(
            "dashboard.appDetail.start",
            &[("op", spec.operation.api_label())],
        ),
    };
    let full = gtk::Button::with_label(&full_label);
    full.set_widget_name("operation-control-action");
    full.set_hexpand(true);
    if spec.active {
        full.add_css_class("destructive-action");
    } else {
        full.add_css_class("suggested-action");
    }
    apply_busy(&full, spec.busy, ctx);
    full.set_sensitive(!spec.busy);
    {
        let on_start = handlers.on_start.clone();
        let on_stop = handlers.on_stop;
        let active = spec.active;
        full.connect_clicked(move |_| {
            if active {
                on_stop();
            } else {
                on_start();
            }
        });
    }
    let action_row = gtk::ListBoxRow::new();
    action_row.set_activatable(false);
    let pad = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pad.set_margin_top(8);
    pad.set_margin_bottom(8);
    pad.set_margin_start(8);
    pad.set_margin_end(8);
    pad.append(&full);
    action_row.set_child(Some(&pad));
    row.add_row(&action_row);
    row
}

fn path_rows(ctx: &AppCtx, spec: &OperationControlSpec) -> Vec<adw::ActionRow> {
    let serve = spec.operation == OperationType::Serve;
    let mut rows = Vec::new();
    for (title_key, fallback, path, hidden, is_source) in [
        (
            if serve {
                "dashboard.appDetail.serving"
            } else {
                "fileBrowser.operations.details.source"
            },
            if serve { "Serving" } else { "Source" },
            spec.source.clone().unwrap_or_default(),
            false,
            true,
        ),
        (
            if serve {
                "dashboard.appDetail.accessibleVia"
            } else {
                "fileBrowser.operations.details.destination"
            },
            if serve {
                "Accessible via"
            } else {
                "Destination"
            },
            spec.destination.clone().unwrap_or_default(),
            spec.hide_destination,
            false,
        ),
    ] {
        if hidden {
            continue;
        }
        let display = if path.is_empty() {
            ctx.t_or("dashboard.appDetail.notConfigured", "Not configured")
        } else {
            path.clone()
        };
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(title_key, fallback));
        row.set_subtitle(&display);
        let open_paths: Vec<String> = if path.is_empty() {
            Vec::new()
        } else if is_source {
            crate::jobs::split_job_paths(&path)
        } else {
            vec![path.clone()]
        };
        let can_browse = (is_source || spec.dest_browseable)
            && open_paths
                .iter()
                .any(|item| crate::transfers::browse_for(item).is_some());
        if can_browse {
            append_open_path_suffix(&row, ctx, &spec.remote_name, &open_paths);
        }
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some(&ctx.t_or("common.copy", "Copy path")));
        copy.set_valign(gtk::Align::Center);
        let copy_text = display.clone();
        copy.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&copy_text);
            }
        });
        row.add_suffix(&copy);
        rows.push(row);
    }
    rows
}

fn append_open_path_suffix(row: &adw::ActionRow, ctx: &AppCtx, remote: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let opening = ctx.is_folder_opening(remote);
    if paths.len() == 1 {
        let open = gtk::Button::from_icon_name("folder-open-symbolic");
        open.set_tooltip_text(Some(&ctx.t_or(
            "detailShared.pathDisplay.openInExplorer",
            "Open in file explorer",
        )));
        open.set_valign(gtk::Align::Center);
        open.add_css_class("flat");
        open.add_css_class("circular");
        open.set_sensitive(!opening);
        if opening {
            let spinner = gtk::Spinner::new();
            spinner.set_spinning(true);
            spinner.set_valign(gtk::Align::Center);
            row.add_suffix(&spinner);
        }
        let ctx = ctx.clone();
        let rem = remote.to_string();
        let path = paths[0].clone();
        open.connect_clicked(move |_| {
            ctx.open_typed_path(&rem, &path);
        });
        row.add_suffix(&open);
        return;
    }

    let button = gtk::MenuButton::new();
    button.set_icon_name("folder-open-symbolic");
    button.set_valign(gtk::Align::Center);
    button.set_has_frame(false);
    button.add_css_class("circular");
    button.set_tooltip_text(Some(
        &ctx.t_or("overviews.remoteCard.browse", "Browse active folders"),
    ));
    button.set_sensitive(!opening);
    if opening {
        let spinner = gtk::Spinner::new();
        spinner.set_spinning(true);
        spinner.set_valign(gtk::Align::Center);
        row.add_suffix(&spinner);
    }
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);
    for path in paths {
        let item = gtk::Button::with_label(path);
        item.set_hexpand(true);
        let ctx = ctx.clone();
        let rem = remote.to_string();
        let path = path.clone();
        let popover = popover.clone();
        item.connect_clicked(move |_| {
            ctx.open_typed_path(&rem, &path);
            popover.popdown();
        });
        list.append(&item);
    }
    popover.set_child(Some(&list));
    button.set_popover(Some(&popover));
    row.add_suffix(&button);
}

fn apply_busy(btn: &gtk::Button, busy: bool, ctx: &AppCtx) {
    if !busy {
        return;
    }
    btn.set_sensitive(false);
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    btn.set_child(Some(&spinner));
    btn.set_tooltip_text(Some(
        &ctx.t_or("remote.actionInProgress", "Action already in progress"),
    ));
}
