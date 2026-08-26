mod dashboard;
mod dialogs;
mod flow;
mod interactive;
mod job_panels;
mod nautilus;
mod onboarding;
mod preferences;
mod remote_config;
mod tray;
mod vfs_panel;
mod window;
mod wizard;

use crate::i18n::I18n;
use crate::platform::PowerInhibitor;
use crate::rclone::RcloneEngine;
use crate::settings::AppSettings;
use crate::store::{AppStore, RuntimeSnapshot};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
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
    pub pending_nav: Rc<RefCell<Option<crate::navigation::NavTarget>>>,
    pub pending_show: Rc<Cell<bool>>,
    pub pending_quit: Rc<Cell<bool>>,
    pub pending_reload: Rc<Cell<bool>>,
    pub pending_reopen_prefs: Rc<Cell<bool>>,
    pub reload_destroy: Rc<Cell<bool>>,
    pub ui_generation: Rc<Cell<u64>>,
    pub active_workspace: Rc<RefCell<String>>,
    pub shutdown_prompt_open: Rc<Cell<bool>>,
    pub inhibitor: Rc<RefCell<PowerInhibitor>>,
    pub watch_mtimes: Rc<RefCell<HashMap<String, u64>>>,
    pub pending_watch: Rc<RefCell<HashMap<String, crate::automation::WatchPending>>>,
    pub watch_hub: Rc<RefCell<crate::watch::WatchHub>>,
    pub fsinfo_cache: Rc<RefCell<HashMap<String, crate::rclone::FsInfo>>>,
    pub pending_picker: Rc<RefCell<Option<crate::picker::PickerRequest>>>,
    pub pending_config_import: Rc<RefCell<Vec<std::path::PathBuf>>>,
    pub config_import_open: Rc<Cell<bool>>,
    pub connection: Rc<RefCell<crate::connection::ConnectionStatus>>,
    pub connection_detail: Rc<RefCell<String>>,
    pub last_connection_check: Rc<RefCell<Option<std::time::Instant>>>,
    pub updates: Rc<RefCell<crate::updater::PendingUpdates>>,
    pub last_update_check: Rc<RefCell<Option<std::time::Instant>>>,
    pub action_busy: Rc<RefCell<HashSet<String>>>,
    pub check_status_overrides: Rc<RefCell<HashMap<String, String>>>,
    pub hidden_check_ids: Rc<RefCell<HashSet<String>>>,
    dump_cache: Rc<RefCell<Option<(std::time::Instant, serde_json::Value)>>>,
}

impl AppCtx {
    pub fn new() -> Self {
        let mut settings = AppSettings::load();
        let mut store = AppStore::load();
        let config_dir = AppSettings::config_dir();
        if crate::migrate::detect_rcman_layout(&config_dir) {
            let report = crate::migrate::import_rcman(&config_dir, &mut store, &mut settings);
            if report.changed() {
                let _ = store.save();
                let _ = settings.save();
            }
        }
        let i18n = I18n::load(&settings.general.language);
        let ctx = Self {
            settings: Rc::new(RefCell::new(settings)),
            store: Rc::new(RefCell::new(store)),
            i18n: Rc::new(RefCell::new(i18n)),
            engine: Rc::new(RefCell::new(None)),
            snapshot: Rc::new(RefCell::new(RuntimeSnapshot::default())),
            selected_remote: Rc::new(RefCell::new(None)),
            selected_quick_run: Rc::new(RefCell::new(None)),
            pending_browse: Rc::new(RefCell::new(None)),
            pending_nav: Rc::new(RefCell::new(None)),
            pending_show: Rc::new(Cell::new(false)),
            pending_quit: Rc::new(Cell::new(false)),
            pending_reload: Rc::new(Cell::new(false)),
            pending_reopen_prefs: Rc::new(Cell::new(false)),
            reload_destroy: Rc::new(Cell::new(false)),
            ui_generation: Rc::new(Cell::new(0)),
            active_workspace: Rc::new(RefCell::new(String::new())),
            shutdown_prompt_open: Rc::new(Cell::new(false)),
            inhibitor: Rc::new(RefCell::new(PowerInhibitor::new())),
            watch_mtimes: Rc::new(RefCell::new(HashMap::new())),
            pending_watch: Rc::new(RefCell::new(HashMap::new())),
            watch_hub: Rc::new(RefCell::new(crate::watch::WatchHub::new())),
            fsinfo_cache: Rc::new(RefCell::new(HashMap::new())),
            pending_picker: Rc::new(RefCell::new(None)),
            pending_config_import: Rc::new(RefCell::new(Vec::new())),
            config_import_open: Rc::new(Cell::new(false)),
            connection: Rc::new(RefCell::new(crate::connection::ConnectionStatus::Checking)),
            connection_detail: Rc::new(RefCell::new(String::new())),
            last_connection_check: Rc::new(RefCell::new(Some(std::time::Instant::now()))),
            updates: Rc::new(RefCell::new(crate::updater::PendingUpdates::default())),
            last_update_check: Rc::new(RefCell::new(Some(std::time::Instant::now()))),
            action_busy: Rc::new(RefCell::new(HashSet::new())),
            check_status_overrides: Rc::new(RefCell::new(HashMap::new())),
            hidden_check_ids: Rc::new(RefCell::new(HashSet::new())),
            dump_cache: Rc::new(RefCell::new(None)),
        };
        ctx.apply_remote_layout();
        {
            let notifications = ctx.settings.borrow().general.notifications;
            {
                let mut store = ctx.store.borrow_mut();
                store.seed_alert_defaults(notifications);
                store.notifications_enabled = notifications;
            }
        }
        ctx.watch_hub
            .borrow_mut()
            .ensure_paths(&[crate::settings::AppSettings::log_path()
                .to_string_lossy()
                .into_owned()]);
        ctx
    }

    fn cached_dump(&self, client: &crate::rclone::RcClient) -> serde_json::Value {
        if let Some((at, value)) = self.dump_cache.borrow().clone() {
            if at.elapsed() < std::time::Duration::from_secs(5) {
                return value;
            }
        }
        let dump = client.dump_config().unwrap_or(serde_json::json!({}));
        *self.dump_cache.borrow_mut() = Some((std::time::Instant::now(), dump.clone()));
        dump
    }

    pub fn runtime_busy(&self) -> bool {
        let snap = self.snapshot.borrow();
        crate::refresh::runtime_busy(
            snap.jobs
                .iter()
                .any(|j| j.status == "running" || j.status == "starting"),
            snap.mounts.len(),
            snap.serves.len(),
        )
    }

    pub fn set_busy(&self, remote: &str, op: &str, profile: &str, busy: bool) {
        let key = crate::jobs::action_busy_key(remote, op, profile);
        if busy {
            self.action_busy.borrow_mut().insert(key);
        } else {
            self.action_busy.borrow_mut().remove(&key);
        }
    }

    pub fn is_busy(&self, remote: &str, op: &str, profile: &str) -> bool {
        self.action_busy
            .borrow()
            .contains(&crate::jobs::action_busy_key(remote, op, profile))
    }

    pub fn busy_guard(&self, remote: &str, op: &str, profile: &str) -> Option<ActionBusyGuard> {
        if self.is_busy(remote, op, profile) {
            return None;
        }
        self.set_busy(remote, op, profile, true);
        Some(ActionBusyGuard {
            ctx: self.clone(),
            remote: remote.to_string(),
            op: op.to_string(),
            profile: profile.to_string(),
        })
    }

    pub fn open_typed_path(&self, current_remote: &str, raw: &str) {
        let typed = crate::path_kind::parse_typed_path(raw, current_remote);
        if typed.kind == crate::path_kind::PathKind::Local {
            if self.engine_os().eq_ignore_ascii_case(std::env::consts::OS) {
                let _ = open::that(&typed.path);
                return;
            }
            self.request_browse("local", &typed.path);
            return;
        }
        self.request_browse(&typed.remote, &typed.path);
    }

    pub fn browse_remote_home(&self, name: &str) {
        let mount = self
            .snapshot
            .borrow()
            .mounts
            .iter()
            .find(|mount| {
                mount.fs == name
                    || mount.fs == format!("{name}:")
                    || mount.fs.starts_with(&format!("{name}:"))
            })
            .map(|mount| mount.mount_point.clone());
        if let Some(point) = mount.filter(|p| !p.is_empty()) {
            self.open_typed_path(name, &point);
            return;
        }
        self.request_browse(name, "");
    }

    pub fn browse_quick_run(&self, qr: &crate::store::QuickRun) {
        let (src, dst) = crate::store::quick_run_paths(&qr.config.rclone, qr.operation_type);
        if let Some(path) = src.or(dst).filter(|p| !p.is_empty()) {
            self.open_typed_path(&qr.remote_name, &path);
            return;
        }
        self.browse_remote_home(&qr.remote_name);
    }

    pub fn fs_info(&self, remote: &str) -> Option<crate::rclone::FsInfo> {
        if let Some(cached) = self.fsinfo_cache.borrow().get(remote).cloned() {
            return Some(cached);
        }
        let client = self.client()?;
        let fs = if remote == "local" {
            "/".into()
        } else {
            crate::rclone::remote_fs(remote, "")
        };
        let info = client.fs_info(&fs).ok()?;
        self.fsinfo_cache
            .borrow_mut()
            .insert(remote.to_string(), info.clone());
        Some(info)
    }

    pub fn t(&self, key: &str) -> String {
        self.i18n.borrow().t(key)
    }

    pub fn t_or(&self, key: &str, fallback: &str) -> String {
        self.i18n.borrow().t_or(key, fallback)
    }

    pub fn tf(&self, key: &str, params: &[(&str, &str)]) -> String {
        self.i18n.borrow().tf(key, params)
    }

    pub fn tf_or(&self, key: &str, fallback: &str, params: &[(&str, &str)]) -> String {
        self.i18n.borrow().tf_or(key, fallback, params)
    }

    pub fn option_label(&self, name: &str, kind: &str, fallback: &str) -> String {
        self.i18n.borrow().option_label(name, kind, fallback, None)
    }

    pub fn refresh_connection(&self) {
        *self.connection.borrow_mut() = crate::connection::ConnectionStatus::Checking;
        let urls = self.settings.borrow().core.connection_check_urls.clone();
        let urls = if urls.is_empty() {
            vec!["https://www.google.com".into()]
        } else {
            urls
        };
        let results = crate::connection::check_links(&urls, 2);
        *self.connection.borrow_mut() = crate::connection::status_from_results(&results);
        *self.connection_detail.borrow_mut() = crate::connection::failed_services(&results);
        *self.last_connection_check.borrow_mut() = Some(std::time::Instant::now());
    }

    pub fn connection_stale(&self, max_age: std::time::Duration) -> bool {
        self.last_connection_check
            .borrow()
            .is_none_or(|t| t.elapsed() > max_age)
    }

    pub fn refresh_updates(&self) {
        let settings = self.settings.borrow().clone();
        let previous = self.updates.borrow().clone();
        let mut pending = crate::updater::PendingUpdates::default();
        if settings.runtime.app_auto_check_updates {
            pending.app = crate::updater::filter_skipped(
                crate::updater::fetch_app_update(env!("CARGO_PKG_VERSION")).ok(),
                &settings.runtime.app_skipped_updates,
            );
        }
        if settings.runtime.rclone_auto_check_updates {
            let version = self
                .engine
                .borrow()
                .as_ref()
                .map(|e| e.version.clone())
                .unwrap_or_default();
            if !version.is_empty() {
                pending.rclone = crate::updater::filter_skipped(
                    crate::updater::fetch_rclone_update(&version).ok(),
                    &settings.runtime.rclone_skipped_updates,
                );
            }
        }
        if !previous.has_updates() && pending.has_updates() {
            if let Some(app) = pending.app.as_ref().filter(|u| u.available) {
                self.store
                    .borrow_mut()
                    .record_event(crate::alerts::update_event(
                        "app",
                        &app.latest,
                        &app.url,
                        &|key, params| self.tf(key, params),
                    ));
            }
            if let Some(rclone) = pending.rclone.as_ref().filter(|u| u.available) {
                self.store
                    .borrow_mut()
                    .record_event(crate::alerts::update_event(
                        "rclone",
                        &rclone.latest,
                        &rclone.url,
                        &|key, params| self.tf(key, params),
                    ));
            }
            self.persist();
        }
        *self.updates.borrow_mut() = pending;
        *self.last_update_check.borrow_mut() = Some(std::time::Instant::now());
    }

    pub fn updates_stale(&self, max_age: std::time::Duration) -> bool {
        self.last_update_check
            .borrow()
            .is_none_or(|t| t.elapsed() > max_age)
    }

    pub fn translate_error(&self, message: &str) -> String {
        self.i18n.borrow().translate_backend(message)
    }

    pub fn request_browse(&self, remote: &str, path: &str) {
        self.request_nav(crate::navigation::NavTarget::Files {
            remote: remote.to_string(),
            path: path.to_string(),
        });
    }

    pub fn request_nav(&self, target: crate::navigation::NavTarget) {
        *self.pending_nav.borrow_mut() = Some(target);
    }

    pub fn take_nav(&self) -> Option<crate::navigation::NavTarget> {
        self.pending_nav.borrow_mut().take()
    }

    pub fn request_show(&self) {
        self.pending_show.set(true);
    }

    pub fn take_show(&self) -> bool {
        self.pending_show.replace(false)
    }

    pub fn request_quit(&self) {
        self.pending_quit.set(true);
    }

    pub fn take_quit(&self) -> bool {
        self.pending_quit.replace(false)
    }

    pub fn request_reload_ui(&self, reopen_prefs: bool) {
        self.pending_reopen_prefs.set(reopen_prefs);
        self.pending_reload.set(true);
    }

    pub fn take_reload(&self) -> bool {
        self.pending_reload.replace(false)
    }

    pub fn take_reopen_prefs(&self) -> bool {
        self.pending_reopen_prefs.replace(false)
    }

    pub fn take_reload_destroy(&self) -> bool {
        self.reload_destroy.replace(false)
    }

    pub fn bump_generation(&self) {
        self.ui_generation
            .set(self.ui_generation.get().wrapping_add(1));
    }

    pub fn apply_persisted_options(&self) {
        if let Some(client) = self.client() {
            crate::backend_options::apply(&client, &self.backend_key());
        }
    }

    pub fn request_picker(
        &self,
        config: crate::picker::FilePickerConfig,
        on_pick: Rc<dyn Fn(crate::picker::PickerResult)>,
    ) {
        let loc = config
            .initial_location
            .clone()
            .unwrap_or_else(|| match config.mode {
                crate::picker::PickerMode::Remote => String::new(),
                _ => {
                    if !self.engine_os().eq_ignore_ascii_case(std::env::consts::OS) {
                        crate::path_kind::default_local_root(&self.engine_os())
                    } else {
                        dirs::home_dir()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| {
                                crate::path_kind::default_local_root(&self.engine_os())
                            })
                    }
                }
            });
        let (remote, path) = if loc.is_empty() {
            ("local".into(), String::new())
        } else {
            crate::rclone::split_remote_path(&loc)
        };
        *self.pending_picker.borrow_mut() = Some(crate::picker::PickerRequest { config, on_pick });
        self.request_browse(&remote, &path);
    }

    pub fn enqueue_config_import(&self, path: std::path::PathBuf) {
        if path.as_os_str().is_empty() {
            return;
        }
        let mut pending = self.pending_config_import.borrow_mut();
        if pending.iter().any(|p| p == &path) {
            return;
        }
        pending.push(path);
    }

    pub fn take_config_import(&self) -> Option<std::path::PathBuf> {
        if self.config_import_open.get() {
            return None;
        }
        let mut pending = self.pending_config_import.borrow_mut();
        if pending.is_empty() {
            None
        } else {
            Some(pending.remove(0))
        }
    }

    pub fn persist(&self) {
        self.save_remote_layout();
        let _ = self.settings.borrow().save();
        let _ = self.store.borrow().save();
    }

    pub fn record_started_job(
        &self,
        result: &str,
        remote: &str,
        profile: &crate::store::ProfileConfig,
        origin: &str,
        op: &str,
        src: &str,
        dst: &str,
        quick_run_id: &str,
    ) {
        let meta =
            crate::jobs::job_meta_for(remote, profile, origin, &self.backend_key(), quick_run_id);
        let profile_name = meta.profile.clone();
        {
            let mut store = self.store.borrow_mut();
            crate::jobs::remember_started(&mut store.job_meta, result, meta);
            for id in crate::jobs::parse_started_ids(result) {
                store.remember_job(crate::jobs::started_operation_job(
                    id,
                    op,
                    remote,
                    &profile_name,
                    origin,
                    src,
                    dst,
                ));
            }
        }
        self.persist();
    }

    pub fn stamp_mount(&self, fs: &str, mount_point: &str, profile: &str, origin: &str) {
        if mount_point.is_empty() {
            return;
        }
        let mut snap = self.snapshot.borrow_mut();
        if let Some(mount) = snap
            .mounts
            .iter_mut()
            .find(|item| item.mount_point == mount_point)
        {
            if !profile.is_empty() {
                mount.profile = profile.to_string();
            }
            if !origin.is_empty() {
                mount.origin = origin.to_string();
            }
            if !fs.is_empty() {
                mount.fs = fs.to_string();
            }
            return;
        }
        let mut mount = crate::rclone::MountedRemote::new(fs, mount_point);
        mount.profile = profile.to_string();
        mount.origin = origin.to_string();
        snap.mounts.push(mount);
    }

    pub fn backend_key(&self) -> String {
        crate::layout::backend_key(&self.settings.borrow().core.active_backend)
    }

    /// OS of the active extra RC backend, or this process (`linux` / `windows` / `macos`).
    pub fn engine_os(&self) -> String {
        let active = self.settings.borrow().core.active_backend.clone();
        if !active.is_empty() {
            if let Some(os) = self
                .settings
                .borrow()
                .core
                .extra_backends
                .iter()
                .find(|backend| backend.name == active)
                .map(|backend| backend.os.clone())
            {
                if !os.is_empty() && os != "unknown" {
                    return os;
                }
            }
        }
        std::env::consts::OS.to_string()
    }

    pub fn remember_backend_identity(&self, name: &str, identity: &crate::rclone::BackendIdentity) {
        if let Some(entry) = self
            .settings
            .borrow_mut()
            .core
            .extra_backends
            .iter_mut()
            .find(|backend| backend.name == name)
        {
            entry.os = identity.os.clone();
            entry.arch = identity.arch.clone();
        }
        self.persist();
    }

    pub fn apply_remote_layout(&self) {
        let key = self.backend_key();
        let layout =
            crate::layout::load_remote_layout(&self.settings.borrow().runtime.remote_layouts, &key);
        if layout.order.is_empty() && layout.hidden.is_empty() {
            return;
        }
        let mut store = self.store.borrow_mut();
        store.remote_order = layout.order;
        store.hidden_remotes = layout.hidden;
    }

    pub fn save_remote_layout(&self) {
        let key = self.backend_key();
        let store = self.store.borrow();
        let layout = crate::layout::RemotesLayout {
            order: store.remote_order.clone(),
            hidden: store.hidden_remotes.clone(),
        };
        drop(store);
        crate::layout::store_remote_layout(
            &mut self.settings.borrow_mut().runtime.remote_layouts,
            &key,
            &layout,
        );
    }

    pub fn switch_backend(&self, name: &str) {
        let from = self.settings.borrow().core.active_backend.clone();
        let from_key = crate::layout::backend_key(&from);
        let to_key = crate::layout::backend_key(name);
        self.fsinfo_cache.borrow_mut().clear();
        self.save_remote_layout();
        if from_key != to_key {
            let (from_mounts, from_serves) = {
                let mut snap = self.snapshot.borrow_mut();
                snap.jobs.clear();
                snap.remotes.clear();
                (
                    std::mem::take(&mut snap.mounts),
                    std::mem::take(&mut snap.serves),
                )
            };
            let (mounts, serves) =
                self.store
                    .borrow_mut()
                    .swap_backend_state(&from, name, from_mounts, from_serves);
            let mut snap = self.snapshot.borrow_mut();
            snap.mounts = mounts;
            snap.serves = serves;
        }
        self.settings.borrow_mut().core.active_backend = if name == "local" {
            String::new()
        } else {
            name.to_string()
        };
        self.apply_remote_layout();
        self.persist();
        self.apply_persisted_options();
        self.apply_active_backend_config();
        self.refresh_runtime();
    }

    pub fn serve_types(&self) -> Vec<String> {
        let live = self
            .client()
            .and_then(|client| client.serve_types().ok())
            .unwrap_or_default();
        crate::operations::serve_types_or_default(&live)
    }

    pub fn mount_types(&self) -> Vec<String> {
        let live = self
            .client()
            .and_then(|client| client.mount_types().ok())
            .unwrap_or_default();
        crate::operations::mount_types_or_default(&live)
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
            let binary = crate::rclone::engine::resolve_rclone_binary(&settings.core.rclone_binary);
            let password = crate::keyring::resolve_config_password(&settings.core.config_password);
            let config_path =
                crate::repair::config_path_from_flags(&settings.core.rclone_additional_flags)
                    .map(std::path::PathBuf::from);
            crate::rclone::serve::install_spawn_context(
                crate::rclone::serve::spawn_context_from_settings(
                    binary,
                    config_path,
                    &settings.core.rclone_additional_flags,
                    &settings.core.rclone_env_vars,
                    &password,
                ),
            );
        } else {
            *self.engine.borrow_mut() = Some(crate::rclone::RcloneEngine::start(&settings));
        }
        if self.engine_ready() {
            self.apply_persisted_options();
            self.apply_active_backend_config();
            self.start_autostarts();
        } else {
            self.refresh_runtime();
        }
    }

    pub fn apply_active_backend_config(&self) {
        let settings = self.settings.borrow().clone();
        let Some(client) = self.client() else {
            return;
        };
        let active = settings.core.active_backend.clone();
        if !active.is_empty() && active != "local" {
            if let Some(entry) = settings
                .core
                .extra_backends
                .iter()
                .find(|b| b.name == active)
            {
                crate::rclone::apply_backend_rc_config(
                    &client,
                    Some(entry.config_path.as_str()),
                    Some(entry.config_password.as_str()),
                );
            }
            return;
        }
        let path = crate::repair::config_path_from_flags(&settings.core.rclone_additional_flags);
        let password = crate::keyring::load_password().or_else(|| {
            let stored = settings.core.config_password.clone();
            if stored.is_empty() {
                None
            } else {
                Some(stored)
            }
        });
        crate::rclone::apply_backend_rc_config(&client, path.as_deref(), password.as_deref());
    }

    pub fn apply_effective_bandwidth(&self) {
        let metered = crate::platform::is_network_metered();
        let settings = self.settings.borrow();
        let rate = if metered && !settings.core.metered_bandwidth_limit.is_empty() {
            settings.core.metered_bandwidth_limit.clone()
        } else {
            settings.core.bandwidth_limit.clone()
        };
        drop(settings);
        let normalized = crate::jobs::normalize_bandwidth(&rate);
        if let Some(client) = self.client() {
            let _ = client.bwlimit(if normalized == "off" {
                None
            } else {
                Some(&normalized)
            });
        }
    }

    pub fn refresh_runtime(&self) {
        let Some(client) = self.client() else {
            let was_online = self.snapshot.borrow().engine_online;
            self.snapshot.borrow_mut().engine_online = false;
            if was_online {
                let event = crate::alerts::engine_event(
                    false,
                    "rclone engine is offline",
                    &|key, params| self.tf(key, params),
                );
                self.store.borrow_mut().record_event(event);
                self.persist();
            }
            return;
        };
        let dump = self.cached_dump(&client);
        let previous_mounts = self.snapshot.borrow().mounts.clone();
        let mounts = crate::rclone::merge_mount_context(
            client.list_mounts().unwrap_or_default(),
            &previous_mounts,
        );
        let serves = client.serve_list().unwrap_or_default();
        let stats = client.stats(None).unwrap_or(serde_json::json!({}));
        let disks = client
            .local_disks()
            .unwrap_or_else(|_| local_fallback_disks(&self.engine_os()));
        let hidden = self.store.borrow().hidden_remotes.clone();
        let known: Vec<u64> = self.store.borrow().job_meta.keys().copied().collect();
        let mut jobs = crate::jobs::merge_preparing_jobs(
            collect_jobs(&client, &known),
            &self.store.borrow().job_history,
        );
        {
            let registry = self.store.borrow().job_meta.clone();
            for job in &mut jobs {
                crate::jobs::apply_job_meta(job, registry.get(&job.id));
            }
            crate::jobs::hydrate_grouped_transfers(&mut jobs, &registry);
        }
        let remotes = crate::store::build_remote_infos(&dump, &mounts, &serves, &jobs, &hidden);
        let previous = self.snapshot.borrow().jobs.clone();
        let previous_serves = self.snapshot.borrow().serves.clone();
        let was_online = self.snapshot.borrow().engine_online;
        notify_job_changes(self, &previous, &jobs);
        emit_runtime_alerts(self, &previous_mounts, &mounts, &previous_serves, &serves);
        if !was_online {
            self.store
                .borrow_mut()
                .record_event(crate::alerts::engine_event(
                    true,
                    "rclone engine is online",
                    &|key, params| self.tf(key, params),
                ));
        }
        let mut snap = self.snapshot.borrow_mut();
        snap.remotes = remotes;
        snap.mounts = mounts;
        snap.serves = serves;
        snap.stats = stats;
        snap.local_disks = disks;
        snap.jobs = jobs;
        snap.engine_online = true;
        drop(snap);
        self.apply_effective_bandwidth();
        self.update_power_inhibit();
    }

    pub fn force_check_mounts(&self) -> Result<usize, String> {
        let client = self
            .client()
            .ok_or_else(|| self.t_or("errors.engineOffline", "rclone engine is offline"))?;
        let previous_mounts = self.snapshot.borrow().mounts.clone();
        let mounts = crate::rclone::merge_mount_context(
            client.list_mounts().map_err(|e| e.to_string())?,
            &previous_mounts,
        );
        let dump = self.cached_dump(&client);
        let serves = self.snapshot.borrow().serves.clone();
        let jobs = self.snapshot.borrow().jobs.clone();
        let hidden = self.store.borrow().hidden_remotes.clone();
        let remotes = crate::store::build_remote_infos(&dump, &mounts, &serves, &jobs, &hidden);
        emit_runtime_alerts(self, &previous_mounts, &mounts, &serves, &serves);
        let count = mounts.len();
        let mut snap = self.snapshot.borrow_mut();
        snap.mounts = mounts;
        snap.remotes = remotes;
        drop(snap);
        self.update_power_inhibit();
        Ok(count)
    }

    pub fn force_check_serves(&self) -> Result<usize, String> {
        let client = self
            .client()
            .ok_or_else(|| self.t_or("errors.engineOffline", "rclone engine is offline"))?;
        let serves = client.serve_list().map_err(|e| e.to_string())?;
        let dump = self.cached_dump(&client);
        let mounts = self.snapshot.borrow().mounts.clone();
        let jobs = self.snapshot.borrow().jobs.clone();
        let hidden = self.store.borrow().hidden_remotes.clone();
        let remotes = crate::store::build_remote_infos(&dump, &mounts, &serves, &jobs, &hidden);
        let previous_serves = self.snapshot.borrow().serves.clone();
        emit_runtime_alerts(self, &mounts, &mounts, &previous_serves, &serves);
        let count = serves.len();
        let mut snap = self.snapshot.borrow_mut();
        snap.serves = serves;
        snap.remotes = remotes;
        drop(snap);
        self.update_power_inhibit();
        Ok(count)
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
                        if let Err(e) = crate::jobs::start_profile(
                            &client,
                            &name,
                            op,
                            profile,
                            Some(&meta),
                            "autostart",
                        ) {
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
                    "quick-run",
                ) {
                    log::warn!("autostart quick run {} failed: {e}", qr.name);
                }
            }
        }
        self.refresh_runtime();
    }

    pub fn reload_automations(&self) {
        let sources = crate::automation::local_watch_sources(&self.store.borrow());
        self.watch_hub.borrow_mut().ensure_paths(&sources);
    }

    pub fn tick_automations(&self) {
        let Some(client) = self.client() else {
            return;
        };
        self.reload_automations();
        let records = crate::automation::collect(&self.store.borrow());
        let dirty = self.watch_hub.borrow().consume_dirty();
        let now = chrono::Utc::now();
        let tick_now = std::time::Instant::now();
        let mut fired = false;
        let mut mtimes = self.watch_mtimes.borrow_mut();
        let mut pending_watch = self.pending_watch.borrow_mut();
        for record in records {
            if self.store.borrow().is_automation_paused(&record.id) {
                continue;
            }
            let due_cron = record.cron_enabled
                && crate::automation::cron_is_due(&record.cron, record.last_run, now);
            if record.watch_enabled {
                let matching = crate::watch::dirty_for_sources(&record.sources, &dirty);
                let mtime_hit = crate::automation::watch_triggered(
                    &record.sources,
                    &mut mtimes,
                    record.watch_changed_only,
                );
                if !matching.is_empty() || mtime_hit {
                    let paths = if matching.is_empty() {
                        record.sources.clone()
                    } else {
                        matching.into_iter().collect()
                    };
                    crate::automation::note_watch_change(
                        pending_watch.entry(record.id.clone()).or_default(),
                        paths,
                    );
                }
            }
            let due_watch = record.watch_enabled
                && pending_watch.get(&record.id).is_some_and(|pending| {
                    crate::automation::watch_ready(pending, record.watch_delay, tick_now)
                });
            if due_cron || due_watch {
                let scoped = if due_watch
                    && record.watch_changed_only
                    && record.operation != crate::operations::OperationType::Bisync
                {
                    pending_watch.get(&record.id).map(|pending| {
                        crate::automation::compute_scoped_targets(
                            &record.sources,
                            &record.destinations,
                            &pending.paths,
                        )
                    })
                } else {
                    None
                };
                let scoped_ref = scoped.as_deref().filter(|pairs| !pairs.is_empty());
                let mut store = self.store.borrow_mut();
                match crate::automation::fire(&client, &mut store, &record, now, scoped_ref) {
                    Ok(id) => {
                        log::info!("automation {} started {id}", record.name);
                        store.record_event(crate::alerts::automation_event(
                            &record.name,
                            &record.remote,
                            true,
                            &id,
                            &|key, params| self.tf(key, params),
                        ));
                        fired = true;
                    }
                    Err(e) => {
                        log::warn!("automation {} failed: {e}", record.name);
                        store.record_event(crate::alerts::automation_event(
                            &record.name,
                            &record.remote,
                            false,
                            &e,
                            &|key, params| self.tf(key, params),
                        ));
                        fired = true;
                    }
                }
                if due_watch {
                    pending_watch.remove(&record.id);
                }
            }
        }
        drop(pending_watch);
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
            let _ = crate::platform::show_os_notification(title, body);
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

pub struct ActionBusyGuard {
    ctx: AppCtx,
    remote: String,
    op: String,
    profile: String,
}

impl Drop for ActionBusyGuard {
    fn drop(&mut self) {
        self.ctx
            .set_busy(&self.remote, &self.op, &self.profile, false);
    }
}

fn collect_jobs(client: &crate::rclone::RcClient, known: &[u64]) -> Vec<crate::store::JobInfo> {
    let Ok(list) = client.job_list() else {
        return Vec::new();
    };
    let listed: Vec<u64> = list
        .get("jobids")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| id.as_u64())
        .collect();
    let ids = crate::jobs::select_job_ids(&listed, known, crate::jobs::MAX_JOB_STATUS_FETCH);
    let inputs: Vec<serde_json::Value> = ids
        .iter()
        .map(|jobid| {
            crate::rclone::batch_input("job/status", serde_json::json!({ "jobid": jobid }))
        })
        .collect();
    let batched = client
        .batch(&inputs)
        .ok()
        .map(|v| crate::rclone::parse_batch_results(&v));
    let mut jobs = Vec::new();
    for (idx, jobid) in ids.into_iter().enumerate() {
        let status = batched
            .as_ref()
            .and_then(|rows| rows.get(idx))
            .and_then(|row| {
                row.get("output")
                    .cloned()
                    .or_else(|| row.get("result").cloned())
                    .or_else(|| Some(row.clone()))
            })
            .or_else(|| client.job_status(jobid).ok());
        let Some(status) = status else {
            continue;
        };
        let group = status
            .get("group")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("job/{jobid}"));
        let mut stats = client.stats(Some(&group)).unwrap_or(serde_json::json!({}));
        let finished = status
            .get("finished")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        if finished {
            if let Ok(transferred) = client.transferred(Some(&group)) {
                crate::jobs::merge_completed_transfers(&mut stats, &transferred);
            }
        }
        jobs.push(crate::jobs::job_from_status(jobid, &status, Some(&stats)));
    }
    jobs.retain(crate::jobs::is_managed_job);
    jobs
}

fn notify_job_changes(
    ctx: &AppCtx,
    previous: &[crate::store::JobInfo],
    current: &[crate::store::JobInfo],
) {
    let mut dirty = false;
    for event in crate::alerts::job_events(previous, current, &|key, params| ctx.tf(key, params)) {
        ctx.store.borrow_mut().record_event(event);
        dirty = true;
    }
    for job in current {
        let was = previous.iter().find(|j| j.id == job.id);
        if job.status != "running" && was.map(|j| j.status.as_str()) != Some(job.status.as_str()) {
            let level = if job.status == "failed" || job.error.is_some() {
                crate::logs::LogLevel::Error
            } else {
                crate::logs::LogLevel::Info
            };
            let context = serde_json::json!({
                "job_id": job.id,
                "operation": job.operation,
                "status": job.status,
                "error": job.error,
                "src": job.src,
                "dst": job.dst,
            });
            ctx.store.borrow_mut().push_log(
                &job.remote,
                crate::logs::format_now(
                    level,
                    Some(&job.remote),
                    &format!("job {} {}", job.id, job.status),
                    Some(&context.to_string()),
                ),
            );
            ctx.store.borrow_mut().remember_job(job.clone());
            dirty = true;
        }
    }
    for job in previous {
        if current.iter().all(|j| j.id != job.id) {
            ctx.store
                .borrow_mut()
                .remember_job(crate::jobs::finalize_dropped_job(job));
            dirty = true;
        }
    }
    if dirty {
        ctx.persist();
    }
}

fn emit_runtime_alerts(
    ctx: &AppCtx,
    previous_mounts: &[crate::rclone::MountedRemote],
    mounts: &[crate::rclone::MountedRemote],
    previous_serves: &[crate::rclone::ServeItem],
    serves: &[crate::rclone::ServeItem],
) {
    let mut dirty = false;
    for event in
        crate::alerts::mount_events(previous_mounts, mounts, &|key, params| ctx.tf(key, params))
    {
        ctx.store.borrow_mut().push_log(
            &event.remote,
            crate::logs::format_now(
                crate::logs::LogLevel::Notice,
                Some(&event.remote),
                &format!("mount {}", event.title),
                Some(&event.body),
            ),
        );
        ctx.store.borrow_mut().record_event(event);
        dirty = true;
    }
    for event in
        crate::alerts::serve_events(previous_serves, serves, &|key, params| ctx.tf(key, params))
    {
        ctx.store.borrow_mut().push_log(
            &event.remote,
            crate::logs::format_now(
                crate::logs::LogLevel::Notice,
                Some(&event.remote),
                &format!("serve {}", event.title),
                Some(&event.body),
            ),
        );
        ctx.store.borrow_mut().record_event(event);
        dirty = true;
    }
    if dirty {
        ctx.persist();
    }
}

fn local_fallback_disks(engine_os: &str) -> Vec<String> {
    let mut disks = if engine_os.eq_ignore_ascii_case("windows") || cfg!(windows) {
        vec!["C:\\".to_string()]
    } else {
        vec!["/".to_string()]
    };
    if let Some(home) = dirs::home_dir() {
        disks.push(home.to_string_lossy().into_owned());
    }
    disks
}

pub(super) fn detail_page_switcher(
    stack: &adw::ViewStack,
    page: &Rc<RefCell<String>>,
    pages: &[(&str, String)],
) -> gtk::Box {
    use gtk::prelude::*;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("linked");
    row.set_halign(gtk::Align::Center);
    if stack.child_by_name(page.borrow().as_str()).is_some() {
        stack.set_visible_child_name(page.borrow().as_str());
    }
    let mut group: Option<gtk::ToggleButton> = None;
    for (name, label) in pages {
        let btn = gtk::ToggleButton::with_label(label);
        btn.set_hexpand(true);
        if let Some(anchor) = &group {
            btn.set_group(Some(anchor));
        } else {
            group = Some(btn.clone());
        }
        if page.borrow().as_str() == *name {
            btn.set_active(true);
        }
        let stack = stack.clone();
        let page = page.clone();
        let name = (*name).to_string();
        btn.connect_toggled(move |button| {
            if button.is_active() {
                *page.borrow_mut() = name.clone();
                stack.set_visible_child_name(&name);
            }
        });
        row.append(&btn);
    }
    row
}
