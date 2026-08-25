mod dashboard;
mod dialogs;
mod flow;
mod nautilus;
mod onboarding;
mod window;

use crate::i18n::I18n;
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
        let jobs = self.snapshot.borrow().jobs.clone();
        let remotes = crate::store::build_remote_infos(&dump, &mounts, &serves, &jobs, &hidden);
        let mut snap = self.snapshot.borrow_mut();
        snap.remotes = remotes;
        snap.mounts = mounts;
        snap.serves = serves;
        snap.stats = stats;
        snap.local_disks = disks;
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

fn local_fallback_disks() -> Vec<String> {
    let mut disks = vec!["/".to_string()];
    if let Some(home) = dirs::home_dir() {
        disks.push(home.to_string_lossy().into_owned());
    }
    disks
}
