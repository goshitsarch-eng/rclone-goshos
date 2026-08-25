mod dashboard;
mod dialogs;
mod flow;
mod nautilus;
mod onboarding;
mod remote_config;
mod tray;
mod window;
mod wizard;

use crate::i18n::I18n;
use crate::platform::PowerInhibitor;
use crate::rclone::RcloneEngine;
use crate::settings::AppSettings;
use crate::store::{AppStore, RuntimeSnapshot};
use std::cell::RefCell;
use std::collections::HashMap;
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
    pub pending_browse: Rc<RefCell<Option<(String, String)>>>,
    pub inhibitor: Rc<RefCell<PowerInhibitor>>,
    pub watch_mtimes: Rc<RefCell<HashMap<String, u64>>>,
    pub watch_hub: Rc<RefCell<crate::watch::WatchHub>>,
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
            pending_browse: Rc::new(RefCell::new(None)),
            inhibitor: Rc::new(RefCell::new(PowerInhibitor::new())),
            watch_mtimes: Rc::new(RefCell::new(HashMap::new())),
            watch_hub: Rc::new(RefCell::new(crate::watch::WatchHub::new())),
        }
    }

    pub fn t(&self, key: &str) -> String {
        self.i18n.borrow().t(key)
    }

    pub fn translate_error(&self, message: &str) -> String {
        self.i18n.borrow().translate_backend(message)
    }

    pub fn request_browse(&self, remote: &str, path: &str) {
        *self.pending_browse.borrow_mut() = Some((remote.to_string(), path.to_string()));
    }

    pub fn persist(&self) {
        let _ = self.settings.borrow().save();
        let _ = self.store.borrow().save();
    }

    pub fn client(&self) -> Option<crate::rclone::RcClient> {
        let settings = self.settings.borrow();
        let active = settings.core.active_backend.clone();
        if !active.is_empty() && active != "local" {
            if let Some(entry) = settings
                .core
                .extra_backends
                .iter()
                .find(|b| b.name == active)
            {
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
                return Some(
                    crate::rclone::RcClient::new(&entry.host, entry.port).with_auth(user, pass),
                );
            }
        }
        drop(settings);
        self.engine.borrow().as_ref().map(|e| e.client.clone())
    }

    pub fn engine_ready(&self) -> bool {
        let settings = self.settings.borrow();
        if !settings.core.active_backend.is_empty() && settings.core.active_backend != "local" {
            return settings
                .core
                .extra_backends
                .iter()
                .any(|b| b.name == settings.core.active_backend);
        }
        drop(settings);
        self.engine
            .borrow()
            .as_ref()
            .map(|e| e.available)
            .unwrap_or(false)
    }

    pub fn restart_engine(&self) {
        let settings = self.settings.borrow().clone();
        if !settings.core.active_backend.is_empty() && settings.core.active_backend != "local" {
            *self.engine.borrow_mut() = None;
        } else {
            *self.engine.borrow_mut() = Some(crate::rclone::RcloneEngine::start(&settings));
        }
        if self.engine_ready() {
            self.start_autostarts();
        } else {
            self.refresh_runtime();
        }
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

    pub fn start_autostarts(&self) {
        let Some(client) = self.client() else {
            return;
        };
        let remotes = self.store.borrow().remotes.clone();
        for (name, meta) in remotes {
            for (op_key, profiles) in &meta.profiles {
                let Some(op) = crate::operations::OperationType::parse(op_key) else {
                    continue;
                };
                for profile in profiles.values() {
                    if profile.app.auto_start {
                        if let Err(e) =
                            crate::jobs::start_profile(&client, &name, op, profile, Some(&meta))
                        {
                            log::warn!("autostart {op} on {name} failed: {e}");
                        }
                    }
                }
            }
        }
        let quick_runs = self.store.borrow().quick_runs.clone();
        for qr in quick_runs {
            if qr.config.app.auto_start {
                let meta = self.store.borrow().remotes.get(&qr.remote_name).cloned();
                if let Err(e) = crate::jobs::start_profile(
                    &client,
                    &qr.remote_name,
                    qr.operation_type,
                    &qr.config,
                    meta.as_ref(),
                ) {
                    log::warn!("autostart quick run {} failed: {e}", qr.name);
                }
            }
        }
        self.refresh_runtime();
    }

    pub fn tick_automations(&self) {
        let Some(client) = self.client() else {
            return;
        };
        let records = crate::automation::collect(&self.store.borrow());
        let local_sources: Vec<String> = records
            .iter()
            .flat_map(|r| r.sources.iter().cloned())
            .filter(|p| crate::automation::is_local_watch_path(p))
            .collect();
        self.watch_hub.borrow_mut().ensure_paths(&local_sources);
        let dirty = self.watch_hub.borrow().consume_dirty();
        let now = chrono::Utc::now();
        let mut fired = false;
        let mut mtimes = self.watch_mtimes.borrow_mut();
        for record in records {
            if self.store.borrow().is_automation_paused(&record.id) {
                continue;
            }
            let due_cron = record.cron_enabled
                && crate::automation::cron_is_due(&record.cron, record.last_run, now);
            let mut due_watch = record.watch_enabled
                && (crate::automation::watch_triggered(
                    &record.sources,
                    &mut mtimes,
                    record.watch_changed_only,
                ) || crate::watch::dirty_matches(&record.sources, &dirty));
            if due_watch {
                if let Some(last) = record.last_run {
                    if record.watch_delay > 0
                        && (now - last).num_seconds() < record.watch_delay as i64
                    {
                        due_watch = false;
                    }
                }
            }
            if due_cron || due_watch {
                let mut store = self.store.borrow_mut();
                match crate::automation::fire(&client, &mut store, &record, now) {
                    Ok(id) => {
                        log::info!("automation {} started {id}", record.name);
                        fired = true;
                    }
                    Err(e) => log::warn!("automation {} failed: {e}", record.name),
                }
            }
        }
        drop(mtimes);
        if fired {
            self.persist();
            self.refresh_runtime();
        }
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
        let group = status
            .get("group")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("job/{jobid}"));
        let stats = client.stats(Some(&group)).ok();
        jobs.push(crate::jobs::job_from_status(jobid, &status, stats.as_ref()));
    }
    jobs
}

fn notify_job_changes(
    ctx: &AppCtx,
    previous: &[crate::store::JobInfo],
    current: &[crate::store::JobInfo],
) {
    let mut dirty = false;
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
            dirty = true;
        }
        if job.status != "running" && was.map(|j| j.status.as_str()) != Some(job.status.as_str()) {
            ctx.store.borrow_mut().remember_job(job.clone());
            dirty = true;
        }
    }
    for job in previous {
        if current.iter().all(|j| j.id != job.id) {
            let mut finished = job.clone();
            if finished.status == "running" {
                finished.status = "completed".into();
            }
            ctx.store.borrow_mut().remember_job(finished);
            dirty = true;
        }
    }
    if dirty {
        ctx.persist();
    }
}

fn local_fallback_disks() -> Vec<String> {
    let mut disks = vec!["/".to_string()];
    if let Some(home) = dirs::home_dir() {
        disks.push(home.to_string_lossy().into_owned());
    }
    disks
}
