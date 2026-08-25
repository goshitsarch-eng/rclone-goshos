use super::AppCtx;
use crate::rclone::remote_fs;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum TrayCommand {
    UnmountAll,
    StopJobs,
    StopServes,
    MountRemote(String),
    UnmountRemote(String),
    BrowseRemote(String),
    BrowseInApp(String),
    StartQuickRun(String),
    ShowWindow,
}

#[derive(Clone)]
pub struct TrayBus {
    pub tx: Sender<TrayCommand>,
    rx: Arc<Mutex<Receiver<TrayCommand>>>,
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

#[derive(Clone)]
struct TrayRemote {
    name: String,
    mounted: bool,
}

struct StatusIcon {
    tx: Sender<TrayCommand>,
    remotes: Vec<TrayRemote>,
    quick_runs: Vec<(String, String)>,
    icon_name: String,
    show_label: String,
    unmount_all: String,
    stop_jobs: String,
    stop_serves: String,
    mount_prefix: String,
    unmount_prefix: String,
    browse_label: String,
    browse_in_app: String,
}

impl ksni::Tray for StatusIcon {
    fn icon_name(&self) -> String {
        self.icon_name.clone()
    }

    fn title(&self) -> String {
        "Rclone Manager".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Rclone Manager".into(),
            description: "Remotes, mounts, and transfers".into(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let mut items = vec![
            StandardItem {
                label: self.show_label.clone(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: self.unmount_all.clone(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::UnmountAll);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.stop_jobs.clone(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::StopJobs);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: self.stop_serves.clone(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::StopServes);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];
        for remote in &self.remotes {
            let name = remote.name.clone();
            let mounted = remote.mounted;
            let mount_label = format!("{} {}", self.mount_prefix, name);
            let unmount_label = format!("{} {}", self.unmount_prefix, name);
            let browse_label = self.browse_label.clone();
            let browse_in_app = self.browse_in_app.clone();
            items.push(
                SubMenu {
                    label: if mounted {
                        format!("● {name}")
                    } else {
                        name.clone()
                    },
                    submenu: vec![
                        StandardItem {
                            label: mount_label,
                            enabled: !mounted,
                            activate: Box::new({
                                let name = name.clone();
                                move |this: &mut Self| {
                                    let _ = this.tx.send(TrayCommand::MountRemote(name.clone()));
                                }
                            }),
                            ..Default::default()
                        }
                        .into(),
                        StandardItem {
                            label: unmount_label,
                            enabled: mounted,
                            activate: Box::new({
                                let name = name.clone();
                                move |this: &mut Self| {
                                    let _ = this.tx.send(TrayCommand::UnmountRemote(name.clone()));
                                }
                            }),
                            ..Default::default()
                        }
                        .into(),
                        StandardItem {
                            label: browse_label,
                            activate: Box::new({
                                let name = name.clone();
                                move |this: &mut Self| {
                                    let _ = this.tx.send(TrayCommand::BrowseRemote(name.clone()));
                                }
                            }),
                            ..Default::default()
                        }
                        .into(),
                        StandardItem {
                            label: browse_in_app,
                            activate: Box::new({
                                let name = name.clone();
                                move |this: &mut Self| {
                                    let _ = this.tx.send(TrayCommand::BrowseInApp(name.clone()));
                                }
                            }),
                            ..Default::default()
                        }
                        .into(),
                    ],
                    ..Default::default()
                }
                .into(),
            );
        }
        if !self.quick_runs.is_empty() {
            items.push(MenuItem::Separator);
        }
        for (id, label) in &self.quick_runs {
            let id = id.clone();
            items.push(
                StandardItem {
                    label: format!("Run {label}"),
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.tx.send(TrayCommand::StartQuickRun(id.clone()));
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items
    }
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

pub fn start(ctx: &AppCtx) -> Option<TrayBus> {
    if !ctx.settings.borrow().general.tray_enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let remotes: Vec<TrayRemote> = ctx
        .snapshot
        .borrow()
        .remotes
        .iter()
        .filter(|r| {
            ctx.store
                .borrow()
                .remotes
                .get(&r.name)
                .map(|m| m.show_on_tray)
                .unwrap_or(true)
        })
        .take(ctx.settings.borrow().core.max_tray_items.max(1))
        .map(|r| TrayRemote {
            name: r.name.clone(),
            mounted: r.mounted,
        })
        .collect();
    let quick_runs: Vec<(String, String)> = ctx
        .store
        .borrow()
        .quick_runs
        .iter()
        .filter(|q| q.show_on_tray)
        .map(|q| (q.id.clone(), q.name.clone()))
        .collect();
    let bus = TrayBus {
        tx: tx.clone(),
        rx: Arc::new(Mutex::new(rx)),
    };
    let icon = StatusIcon {
        tx: tx.clone(),
        remotes: remotes.clone(),
        quick_runs: quick_runs.clone(),
        icon_name: tray_icon_name(&ctx.settings.borrow().general.tray_icon_theme).into(),
        show_label: ctx.t_or("tray.showApp", "Show Window"),
        unmount_all: ctx.t_or("tray.unmountAll", "Unmount All"),
        stop_jobs: ctx.t_or("tray.stopAllJobs", "Stop All Jobs"),
        stop_serves: ctx.t_or("tray.stopAllServes", "Stop All Serves"),
        mount_prefix: ctx.t_or("tray.mount", "Mount"),
        unmount_prefix: ctx.t_or("tray.unmount", "Unmount"),
        browse_label: ctx.t_or("tray.browse", "Browse"),
        browse_in_app: ctx.t_or("tray.browseInApp", "Browse in app"),
    };
    std::thread::Builder::new()
        .name("rclone-manager-sni".into())
        .spawn(move || {
            let service = ksni::TrayService::new(icon);
            let _ = service.run();
        })
        .ok();
    log::info!(
        "StatusNotifier tray started ({} remotes, {} quick runs)",
        remotes.len(),
        quick_runs.len()
    );
    Some(bus)
}

pub fn handle(ctx: &AppCtx, cmd: TrayCommand) {
    match cmd {
        TrayCommand::UnmountAll => {
            if let Some(c) = ctx.client() {
                let _ = c.unmount_all();
            }
            ctx.refresh_runtime();
        }
        TrayCommand::StopJobs => {
            if let Some(c) = ctx.client() {
                if let Ok(list) = c.job_list() {
                    if let Some(arr) = list.get("jobids").and_then(|x| x.as_array()) {
                        for id in arr {
                            if let Some(jobid) = id.as_u64() {
                                let _ = c.job_stop(jobid);
                            }
                        }
                    }
                }
            }
            ctx.refresh_runtime();
        }
        TrayCommand::StopServes => {
            if let Some(c) = ctx.client() {
                let _ = c.serve_stop_all();
            }
            ctx.refresh_runtime();
        }
        TrayCommand::MountRemote(name) => {
            if let Some(c) = ctx.client() {
                let point = dirs::home_dir().unwrap_or_default().join("mnt").join(&name);
                let _ = std::fs::create_dir_all(&point);
                let _ = c.mount(&remote_fs(&name, ""), &point.to_string_lossy(), "mount");
            }
            ctx.refresh_runtime();
        }
        TrayCommand::UnmountRemote(name) => {
            if let Some(c) = ctx.client() {
                for point in remote_mounts(ctx, &name) {
                    let _ = c.unmount(&point);
                }
            }
            ctx.refresh_runtime();
        }
        TrayCommand::BrowseRemote(name) => {
            if let Some(point) = remote_mounts(ctx, &name).into_iter().next() {
                let _ = open::that(point);
            } else {
                ctx.request_show();
                ctx.request_browse(&name, "");
            }
        }
        TrayCommand::BrowseInApp(name) => {
            ctx.request_show();
            ctx.request_browse(&name, "");
        }
        TrayCommand::StartQuickRun(id) => {
            let qr = ctx
                .store
                .borrow()
                .quick_runs
                .iter()
                .find(|q| q.id == id)
                .cloned();
            if let (Some(c), Some(qr)) = (ctx.client(), qr) {
                if let Some(endpoint) = qr.operation_type.rc_job_endpoint() {
                    let (src, dst) = qr.paths();
                    let _ = c.start_job(
                        endpoint,
                        serde_json::json!({
                            "srcFs": src.unwrap_or_else(|| remote_fs(&qr.remote_name, "")),
                            "dstFs": dst.unwrap_or_else(|| remote_fs(&qr.remote_name, "")),
                        }),
                    );
                }
            }
        }
        TrayCommand::ShowWindow => ctx.request_show(),
    }
}
