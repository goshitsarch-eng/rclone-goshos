mod dashboard;
mod dialogs;
mod flow;
mod nautilus;
mod onboarding;
mod tray;
mod window;
mod wizard;

use crate::i18n::I18n;
use crate::platform::PowerInhibitor;
use crate::rclone::RcloneEngine;
use crate::settings::AppSettings;
use crate::store::{AppStore, RuntimeSnapshot};
use std::cell::RefCell;
use std::rc::Rc;

pub use window::activate;

#[derive(Clone)]
pub struct AppCtx {
    pub settings: Rc<RefCell<AppSettings>>,
    pub store: Rc<RefCell<AppStore>>,
    pub i18n: Rc<RefCell<I18n>>,
    pub engine: Rc<RefCell<Option<RcloneEngine>>>,
    pub snapshot: Rc<RefCell<RuntimeSnapshot>>,
    pub selected_remote: Rc<RefCell<Option<String>>>,
    pub selected_quick_run: Rc<RefCell<Option<String>>>,
    pub inhibitor: Rc<RefCell<PowerInhibitor>>,
}

impl AppCtx {
    pub fn new() -> Self {
        let settings = AppSettings::load();
        let store = AppStore::load();
        let i18n = I18n::load(&settings.general.language);
        Self {
            settings: Rc::new(RefCell::new(settings)),
            store: Rc::new(RefCell::new(store)),
            i18n: Rc::new(RefCell::new(i18n)),
            engine: Rc::new(RefCell::new(None)),
            snapshot: Rc::new(RefCell::new(RuntimeSnapshot::default())),
            selected_remote: Rc::new(RefCell::new(None)),
            selected_quick_run: Rc::new(RefCell::new(None)),
            inhibitor: Rc::new(RefCell::new(PowerInhibitor::new())),
        }
    }

    pub fn t(&self, key: &str) -> String {
        self.i18n.borrow().t(key)
    }

    pub fn persist(&self) {
        let _ = self.settings.borrow().save();
        let _ = self.store.borrow().save();
    }

    pub fn client(&self) -> Option<crate::rclone::RcClient> {
        self.engine.borrow().as_ref().map(|e| e.client.clone())
    }

    pub fn engine_ready(&self) -> bool {
        self.engine
            .borrow()
            .as_ref()
            .map(|e| e.available)
            .unwrap_or(false)
    }

    pub fn refresh_runtime(&self) {
        let Some(client) = self.client() else {
            return;
        };
        let dump = client.dump_config().unwrap_or(serde_json::json!({}));
        let mounts = client.list_mounts().unwrap_or_default();
        let serves = client.serve_list().unwrap_or_default();
        let stats = client.stats(None).unwrap_or(serde_json::json!({}));
        let disks = client
            .local_disks()
            .unwrap_or_else(|_| local_fallback_disks());
        let hidden = self.store.borrow().hidden_remotes.clone();
        let jobs = collect_jobs(&client);
        let remotes = crate::store::build_remote_infos(&dump, &mounts, &serves, &jobs, &hidden);
        let previous = self.snapshot.borrow().jobs.clone();
        notify_job_changes(self, &previous, &jobs);
        let mut snap = self.snapshot.borrow_mut();
        snap.remotes = remotes;
        snap.mounts = mounts;
        snap.serves = serves;
        snap.stats = stats;
        snap.local_disks = disks;
        snap.jobs = jobs;
        drop(snap);
        self.update_power_inhibit();
    }

    pub fn update_power_inhibit(&self) {
        if !self.settings.borrow().general.prevent_sleep {
            self.inhibitor.borrow_mut().release();
            return;
        }
        let snap = self.snapshot.borrow();
        let running = snap.jobs.iter().any(|j| j.status == "running")
            || !snap.mounts.is_empty()
            || !snap.serves.is_empty();
        let reason = format!(
            "{} jobs, {} mounts, {} serves",
            snap.jobs.len(),
            snap.mounts.len(),
            snap.serves.len()
        );
        drop(snap);
        self.inhibitor.borrow_mut().update(running, &reason);
    }

    pub fn toast(&self, overlay: &adw::ToastOverlay, message: impl AsRef<str>) {
        overlay.add_toast(adw::Toast::new(message.as_ref()));
    }

    pub fn notify(&self, title: &str, body: &str) {
        if self.settings.borrow().general.notifications {
            let _ = notify_rust::Notification::new()
                .summary(title)
                .body(body)
                .show();
        }
    }

    pub fn apply_theme(&self) {
        let theme = self.settings.borrow().runtime.theme.clone();
        let style = adw::StyleManager::default();
        match theme.as_str() {
            "light" => style.set_color_scheme(adw::ColorScheme::ForceLight),
            "dark" => style.set_color_scheme(adw::ColorScheme::ForceDark),
            _ => style.set_color_scheme(adw::ColorScheme::Default),
        }
    }
}

fn collect_jobs(client: &crate::rclone::RcClient) -> Vec<crate::store::JobInfo> {
    let Ok(list) = client.job_list() else {
        return Vec::new();
    };
    let ids = list
        .get("jobids")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let mut jobs = Vec::new();
    for id in ids {
        let Some(jobid) = id.as_u64() else { continue };
        let Ok(status) = client.job_status(jobid) else {
            continue;
        };
        let finished = status
            .get("finished")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let success = status
            .get("success")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let error = status
            .get("error")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        jobs.push(crate::store::JobInfo {
            id: jobid,
            operation: status
                .get("output")
                .and_then(|o| o.get("operation"))
                .and_then(|x| x.as_str())
                .unwrap_or("job")
                .to_string(),
            remote: String::new(),
            profile: "default".into(),
            status: if !finished {
                "running".into()
            } else if success {
                "completed".into()
            } else {
                "failed".into()
            },
            origin: "dashboard".into(),
            start_time: chrono::Utc::now(),
            error,
            dry_run: false,
            src: String::new(),
            dst: String::new(),
            group: format!("job/{jobid}"),
        });
    }
    jobs
}

fn notify_job_changes(
    ctx: &AppCtx,
    previous: &[crate::store::JobInfo],
    current: &[crate::store::JobInfo],
) {
    for job in current {
        let was = previous.iter().find(|j| j.id == job.id);
        if job.status == "failed" && was.map(|j| j.status.as_str()) != Some("failed") {
            let mut event = crate::store::AlertEvent::new(
                crate::store::AlertEventKind::Job,
                crate::store::AlertSeverity::High,
                format!("Job #{} failed", job.id),
                job.error
                    .clone()
                    .unwrap_or_else(|| "rclone job failed".into()),
            );
            event.origin = job.origin.clone();
            ctx.store.borrow_mut().record_event(event);
            ctx.persist();
        }
    }
}

fn local_fallback_disks() -> Vec<String> {
    let mut disks = vec!["/".to_string()];
    if let Some(home) = dirs::home_dir() {
        disks.push(home.to_string_lossy().into_owned());
    }
    disks
}
