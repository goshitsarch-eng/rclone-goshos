use super::AppCtx;
use crate::jobs::{find_active_quick_run, start_profile, stop_profile};
use crate::operations::OperationType;
use crate::rclone::remote_fs;
use crate::tray_menu::{plan_tray, TrayAction, TrayCaption, TrayMenuItem};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct TrayBus {
    pub tx: Sender<TrayAction>,
    rx: Arc<Mutex<Receiver<TrayAction>>>,
}

impl TrayBus {
    pub fn drain(&self, ctx: &AppCtx) {
        let Ok(rx) = self.rx.lock() else {
            return;
        };
        while let Ok(cmd) = rx.try_recv() {
            handle(ctx, cmd);
        }
    }
}

struct StatusIcon {
    tx: Sender<TrayAction>,
    items: Vec<TrayMenuItem>,
    icon_name: String,
    tooltip_title: String,
    tooltip_description: String,
}

impl ksni::Tray for StatusIcon {
    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn title(&self) -> String {
        self.tooltip_title.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.tooltip_title.clone(),
            description: self.tooltip_description.clone(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.items.iter().map(|item| to_ksni(item)).collect()
    }
}

fn to_ksni(item: &TrayMenuItem) -> ksni::MenuItem<StatusIcon> {
    use ksni::menu::*;
    if !item.children.is_empty() {
        return SubMenu {
            label: item.label.clone(),
            submenu: item.children.iter().map(to_ksni).collect(),
            ..Default::default()
        }
        .into();
    }
    if item.action.is_none() {
        return MenuItem::Separator;
    }
    let action = item.action.clone();
    StandardItem {
        label: item.label.clone(),
        enabled: item.enabled,
        activate: Box::new(move |this: &mut StatusIcon| {
            if let Some(action) = action.clone() {
                let _ = this.tx.send(action);
            }
        }),
        ..Default::default()
    }
    .into()
}

fn tray_icon_name(theme: &str) -> &'static str {
    match theme {
        "symbolic" | "monochrome_light" | "monochrome_dark" => "folder-remote-symbolic",
        _ => "folder-remote",
    }
}

fn remote_mounts(ctx: &AppCtx, name: &str) -> Vec<String> {
    let prefix = format!("{name}:");
    ctx.snapshot
        .borrow()
        .mounts
        .iter()
        .filter(|m| m.fs == name || m.fs.starts_with(&prefix))
        .map(|m| m.mount_point.clone())
        .collect()
}

fn localize_plan(ctx: &AppCtx, items: &mut [TrayMenuItem]) {
    for item in items {
        match &item.caption {
            TrayCaption::Jobs { active, total } => {
                item.label = if *total == 0 {
                    ctx.t_or("tray.jobsNone", "Jobs [—]")
                } else {
                    ctx.tf(
                        "tray.jobsCount",
                        &[
                            ("active", &active.to_string()),
                            ("total", &total.to_string()),
                        ],
                    )
                };
            }
            TrayCaption::OpCount { op, active, total } => {
                let key = format!("tray.{op}Count");
                item.label = ctx.tf(
                    &key,
                    &[
                        ("active", &active.to_string()),
                        ("total", &total.to_string()),
                    ],
                );
                if item.label == key {
                    item.label = format!("{op} [{active}/{total}]");
                }
            }
            TrayCaption::QuickRuns { active, total } => {
                item.label = ctx.tf(
                    "tray.quickRunsCount",
                    &[
                        ("active", &active.to_string()),
                        ("total", &total.to_string()),
                    ],
                );
            }
            TrayCaption::Literal => {}
        }
        if let Some(action) = &item.action {
            item.label = match action {
                TrayAction::ShowWindow => ctx.t_or("tray.showApp", "Show Window"),
                TrayAction::OpenFiles => ctx.t_or("tray.openFileBrowser", "Open Files"),
                TrayAction::Quit => ctx.t_or("tray.quit", "Quit"),
                TrayAction::UnmountAll => ctx.t_or("tray.unmountAll", "Unmount All"),
                TrayAction::StopJobs => ctx.t_or("tray.stopAllJobs", "Stop All Jobs"),
                TrayAction::StopServes => ctx.t_or("tray.stopAllServes", "Stop All Serves"),
                TrayAction::Status => item.label.clone(),
                TrayAction::MountRemote { profile, .. } => {
                    format!("{} · {profile}", ctx.t_or("tray.mount", "Mount"))
                }
                TrayAction::UnmountRemote { profile, .. } => {
                    format!("{} · {profile}", ctx.t_or("tray.unmount", "Unmount"))
                }
                TrayAction::BrowseRemote(_) => ctx.t_or("tray.browse", "Browse"),
                TrayAction::BrowseInApp(_) => ctx.t_or("tray.browseInApp", "Browse (In App)"),
                TrayAction::StartProfile { op, profile, .. } => {
                    format!("{} {op} · {profile}", ctx.t_or("tray.start", "Start"))
                }
                TrayAction::StopProfile { op, profile, .. } => {
                    format!("{} {op} · {profile}", ctx.t_or("tray.stop", "Stop"))
                }
                TrayAction::StartQuickRun(id) | TrayAction::StopQuickRun(id) => {
                    let name = ctx
                        .store
                        .borrow()
                        .quick_runs
                        .iter()
                        .find(|qr| qr.id == *id)
                        .map(|qr| qr.name.clone())
                        .unwrap_or_else(|| id.clone());
                    crate::tray_menu::quick_run_action_label(
                        matches!(action, TrayAction::StartQuickRun(_)),
                        &name,
                        &ctx.t_or("tray.start", "Start"),
                        &ctx.t_or("tray.stop", "Stop"),
                    )
                }
            };
        }
        localize_plan(ctx, &mut item.children);
    }
}

pub fn start(ctx: &AppCtx) -> Option<TrayBus> {
    if !ctx.settings.borrow().general.tray_enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let snap = ctx.snapshot.borrow();
    let store = ctx.store.borrow();
    let mut items = plan_tray(
        &snap.remotes,
        &store,
        &snap.jobs,
        &snap.mounts,
        &snap.serves,
        ctx.settings.borrow().core.max_tray_items.max(1),
    );
    drop(snap);
    drop(store);
    localize_plan(ctx, &mut items);
    let remotes = items
        .iter()
        .filter(|i| {
            !i.children.is_empty()
                && !i.children.iter().any(|child| {
                    matches!(
                        child.action,
                        Some(TrayAction::StartQuickRun(_)) | Some(TrayAction::StopQuickRun(_))
                    )
                })
        })
        .count();
    let bus = TrayBus {
        tx: tx.clone(),
        rx: Arc::new(Mutex::new(rx)),
    };
    let icon = StatusIcon {
        tx: tx.clone(),
        items,
        icon_name: tray_icon_name(&ctx.settings.borrow().general.tray_icon_theme).into(),
        tooltip_title: ctx.t_or("tray.tooltipDefault", "RClone Manager"),
        tooltip_description: ctx.t_or("tray.tooltipSubtitle", "Remotes, mounts, and transfers"),
    };
    std::thread::Builder::new()
        .name("rclone-manager-sni".into())
        .spawn(move || {
            let service = ksni::TrayService::new(icon);
            let _ = service.run();
        })
        .ok();
    log::info!("StatusNotifier tray started ({remotes} remotes)");
    Some(bus)
}

pub fn handle(ctx: &AppCtx, cmd: TrayAction) {
    match cmd {
        TrayAction::UnmountAll => {
            if let Some(c) = ctx.client() {
                let _ = c.unmount_all();
            }
            ctx.refresh_runtime();
        }
        TrayAction::StopJobs => {
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
            }
            ctx.refresh_runtime();
        }
        TrayAction::StopServes => {
            if let Some(c) = ctx.client() {
                let _ = c.serve_stop_all();
            }
            ctx.refresh_runtime();
        }
        TrayAction::MountRemote { remote, profile } => {
            if let Some(c) = ctx.client() {
                let meta = ctx.store.borrow().remotes.get(&remote).cloned();
                let cfg = meta
                    .as_ref()
                    .and_then(|m| m.get_profile(OperationType::Mount, &profile))
                    .unwrap_or_default();
                if let Err(e) = start_profile(
                    &c,
                    &remote,
                    OperationType::Mount,
                    &cfg,
                    meta.as_ref(),
                    "tray",
                ) {
                    log::warn!("tray mount {remote} failed: {e}");
                    let point = dirs::home_dir()
                        .unwrap_or_default()
                        .join("mnt")
                        .join(&remote);
                    let _ = std::fs::create_dir_all(&point);
                    let _ = c.mount(&remote_fs(&remote, ""), &point.to_string_lossy(), "mount");
                }
            }
            ctx.refresh_runtime();
        }
        TrayAction::UnmountRemote { remote, profile } => {
            if let Some(c) = ctx.client() {
                let mounts = ctx.snapshot.borrow().mounts.clone();
                if let Err(e) = stop_profile(
                    &c,
                    &remote,
                    OperationType::Mount,
                    &profile,
                    &[],
                    &mounts,
                    &[],
                ) {
                    for point in remote_mounts(ctx, &remote) {
                        let _ = c.unmount(&point);
                    }
                    log::warn!("tray unmount {remote}: {e}");
                }
            }
            ctx.refresh_runtime();
        }
        TrayAction::BrowseRemote(name) => {
            if let Some(point) = remote_mounts(ctx, &name).into_iter().next() {
                let _ = open::that(point);
            } else {
                ctx.request_show();
                ctx.request_browse(&name, "");
            }
        }
        TrayAction::BrowseInApp(name) => {
            ctx.request_show();
            ctx.request_browse(&name, "");
        }
        TrayAction::StartProfile {
            remote,
            op,
            profile,
        } => {
            let Some(op) = OperationType::parse(&op) else {
                return;
            };
            if let Some(c) = ctx.client() {
                let meta = ctx.store.borrow().remotes.get(&remote).cloned();
                let cfg = meta
                    .as_ref()
                    .and_then(|m| m.get_profile(op, &profile))
                    .unwrap_or_default();
                if let Err(e) = start_profile(&c, &remote, op, &cfg, meta.as_ref(), "tray") {
                    log::warn!("tray start {op} {remote}/{profile} failed: {e}");
                }
            }
            ctx.refresh_runtime();
        }
        TrayAction::StopProfile {
            remote,
            op,
            profile,
        } => {
            let Some(op) = OperationType::parse(&op) else {
                return;
            };
            if let Some(c) = ctx.client() {
                let snap = ctx.snapshot.borrow().clone();
                if let Err(e) = stop_profile(
                    &c,
                    &remote,
                    op,
                    &profile,
                    &snap.jobs,
                    &snap.mounts,
                    &snap.serves,
                ) {
                    log::warn!("tray stop {op} {remote}/{profile} failed: {e}");
                }
            }
            ctx.refresh_runtime();
        }
        TrayAction::StartQuickRun(id) => {
            let qr = ctx
                .store
                .borrow()
                .quick_runs
                .iter()
                .find(|q| q.id == id)
                .cloned();
            if let (Some(c), Some(qr)) = (ctx.client(), qr) {
                let meta = ctx.store.borrow().remotes.get(&qr.remote_name).cloned();
                if let Err(e) = start_profile(
                    &c,
                    &qr.remote_name,
                    qr.operation_type,
                    &qr.config,
                    meta.as_ref(),
                    "quick-run",
                ) {
                    log::warn!("tray quick run {} failed: {e}", qr.name);
                }
            }
            ctx.refresh_runtime();
        }
        TrayAction::StopQuickRun(id) => {
            let qr = ctx
                .store
                .borrow()
                .quick_runs
                .iter()
                .find(|q| q.id == id)
                .cloned();
            if let (Some(c), Some(qr)) = (ctx.client(), qr) {
                let jobs = ctx.snapshot.borrow().jobs.clone();
                if let Some(job) = find_active_quick_run(&jobs, &qr) {
                    let _ = c.job_stop(job.id);
                }
            }
            ctx.refresh_runtime();
        }
        TrayAction::OpenFiles => {
            ctx.request_show();
            ctx.request_browse("local", "");
        }
        TrayAction::ShowWindow => ctx.request_show(),
        TrayAction::Quit => ctx.request_quit(),
        TrayAction::Status => {}
    }
}
