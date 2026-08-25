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
    log::info!(
        "tray actions ready ({} remotes, {} quick runs)",
        remotes.len(),
        quick_runs.len()
    );
    let _ = (tx, remotes, quick_runs);
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
    }
}
