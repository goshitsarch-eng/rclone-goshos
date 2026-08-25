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

struct StatusIcon {
    tx: Sender<TrayCommand>,
    remotes: Vec<String>,
    quick_runs: Vec<(String, String)>,
}

impl ksni::Tray for StatusIcon {
    fn icon_name(&self) -> String {
        "folder-remote".into()
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
                label: "Show Window".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::ShowWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Unmount All".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::UnmountAll);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop All Jobs".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::StopJobs);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Stop All Serves".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(TrayCommand::StopServes);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];
        for remote in &self.remotes {
            let name = remote.clone();
            items.push(
                StandardItem {
                    label: format!("Mount {name}"),
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.tx.send(TrayCommand::MountRemote(name.clone()));
                    }),
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

pub fn start(ctx: &AppCtx) -> Option<TrayBus> {
    if !ctx.settings.borrow().general.tray_enabled {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let remotes: Vec<String> = ctx
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
        .map(|r| r.name.clone())
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
        TrayCommand::ShowWindow => {}
    }
}
