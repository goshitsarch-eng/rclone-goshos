//! Preferences dialog — Angular `preferences-modal` parity.
//!
//! Engine-restart settings (`rclone_binary`, extra flags, env vars) are queued
//! and only written when the user clicks Save & Restart. Every setting row has
//! a reset-to-default control.

use super::AppCtx;
use crate::settings::{
    apply_path_values, default_for_path, display_setting, requires_engine_restart, values_equal,
};
use adw::prelude::*;
use gtk::prelude::IsA;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
struct PrefsSession {
    ctx: AppCtx,
    pending: Rc<RefCell<HashMap<String, Value>>>,
    restorers: Rc<RefCell<HashMap<String, Rc<dyn Fn()>>>>,
    suppress: Rc<Cell<bool>>,
    banner: adw::PreferencesGroup,
    banner_row: adw::ActionRow,
}

impl PrefsSession {
    fn new(ctx: AppCtx) -> Self {
        let banner = adw::PreferencesGroup::new();
        banner.set_title(&ctx.t_or(
            "modals.preferences.pendingRestart",
            "Pending engine restart",
        ));
        banner.set_description(Some(&ctx.t_or(
            "modals.preferences.pendingChangesTooltip",
            "These rclone engine settings are not applied until you save and restart.",
        )));
        banner.set_visible(false);
        let banner_row = adw::ActionRow::new();
        banner_row.set_title(&glib::markup_escape_text(
            &ctx.t_or("modals.preferences.saveAndRestart", "Save & Restart Engine"),
        ));
        banner.add(&banner_row);
        Self {
            ctx,
            pending: Rc::new(RefCell::new(HashMap::new())),
            restorers: Rc::new(RefCell::new(HashMap::new())),
            suppress: Rc::new(Cell::new(false)),
            banner,
            banner_row,
        }
    }

    fn commit(&self, path: &str, value: Value) {
        if self.suppress.get() {
            return;
        }
        if requires_engine_restart(path) {
            let current = self
                .ctx
                .settings
                .borrow()
                .get_by_path(path)
                .unwrap_or(Value::Null);
            if values_equal(&current, &value) {
                self.pending.borrow_mut().remove(path);
            } else {
                self.pending.borrow_mut().insert(path.to_string(), value);
            }
            self.refresh_banner();
            return;
        }
        if let Err(e) = self.ctx.settings.borrow_mut().set_by_path(path, value) {
            log::warn!("failed to save {path}: {e}");
            return;
        }
        self.ctx.persist();
    }

    fn refresh_banner(&self) {
        let count = self.pending.borrow().len();
        self.banner.set_visible(count > 0);
        self.banner_row.set_subtitle(&if count == 1 {
            self.ctx.t_or(
                "modals.preferences.onePendingChange",
                "1 setting waiting for Save & Restart",
            )
        } else {
            self.ctx.tf(
                "modals.preferences.pendingChangesTooltip",
                &[("count", &count.to_string())],
            )
        });
    }

    fn save_and_restart(&self) {
        let pending: Vec<(String, Value)> = self
            .pending
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if pending.is_empty() {
            self.ctx.restart_engine();
            return;
        }
        if let Err(e) = apply_path_values(&mut self.ctx.settings.borrow_mut(), &pending) {
            log::warn!("failed to apply pending settings: {e}");
            return;
        }
        self.ctx.persist();
        self.pending.borrow_mut().clear();
        self.refresh_banner();
        self.ctx.restart_engine();
    }

    fn discard(&self) {
        self.suppress.set(true);
        for restorer in self.restorers.borrow().values() {
            restorer();
        }
        self.pending.borrow_mut().clear();
        self.suppress.set(false);
        self.refresh_banner();
    }

    fn reset_to_default(&self, path: &str, apply_widget: &dyn Fn(&Value)) {
        let Some(default) = default_for_path(path) else {
            return;
        };
        self.suppress.set(true);
        apply_widget(&default);
        self.suppress.set(false);
        self.commit(path, default);
    }

    fn reset_button(
        &self,
        path: &'static str,
        apply_widget: impl Fn(&Value) + 'static,
    ) -> gtk::Button {
        let button = gtk::Button::from_icon_name("edit-undo-symbolic");
        button.set_valign(gtk::Align::Center);
        button.add_css_class("flat");
        button.set_tooltip_text(Some(
            &self
                .ctx
                .t_or("modals.preferences.resetToDefault", "Reset to default"),
        ));
        let session = self.clone();
        button.connect_clicked(move |_| {
            session.reset_to_default(path, &apply_widget);
        });
        button
    }

    fn remember_restorer(&self, path: &str, restorer: impl Fn() + 'static) {
        self.restorers
            .borrow_mut()
            .insert(path.to_string(), Rc::new(restorer));
    }
}

pub fn present(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    present_page(parent, ctx, None);
}

pub fn present_page(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, page: Option<&str>) {
    let session = PrefsSession::new(ctx.clone());
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title(&ctx.t_or("titlebar.menu.preferences", "Preferences"));
    dialog.set_search_enabled(true);

    let save = gtk::Button::with_label(
        &ctx.t_or("modals.preferences.saveAndRestart", "Save & Restart Engine"),
    );
    save.add_css_class("suggested-action");
    {
        let session = session.clone();
        save.connect_clicked(move |_| session.save_and_restart());
    }
    let discard =
        gtk::Button::with_label(&ctx.t_or("modals.preferences.discardChanges", "Discard Changes"));
    {
        let session = session.clone();
        discard.connect_clicked(move |_| session.discard());
    }
    session.banner_row.add_suffix(&discard);
    session.banner_row.add_suffix(&save);

    let general = adw::PreferencesPage::new();
    general.set_name(Some("general"));
    general.set_title(&ctx.t_or("modals.preferences.tabs.general", "General"));
    general.set_icon_name(Some("preferences-system-symbolic"));
    let g1 = adw::PreferencesGroup::new();
    g1.set_title(&ctx.t_or("settings.general.language.label", "Appearance & language"));

    add_language_row(&session, &g1);
    add_combo(
        &session,
        &g1,
        "settings.general.default_view.label",
        "Default view",
        "general.default_view",
        &["main_menu", "nautilus", "flow"],
        None,
        |_| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.general.tray_enabled.label",
        "Enable tray",
        "general.tray_enabled",
        |_, _| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.general.start_on_startup.label",
        "Start on startup",
        "general.start_on_startup",
        |_, value| {
            if let Some(on) = value.as_bool() {
                let _ = crate::platform::set_autostart(on);
            }
        },
    );
    add_switch(
        &session,
        &g1,
        "settings.general.notifications.label",
        "Notifications",
        "general.notifications",
        |ctx, value| {
            if let Some(on) = value.as_bool() {
                ctx.store.borrow_mut().notifications_enabled = on;
            }
        },
    );
    add_switch(
        &session,
        &g1,
        "settings.general.restrict.label",
        "Restrict sensitive values",
        "general.restrict",
        |_, _| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.general.prevent_sleep.label",
        "Prevent sleep during jobs",
        "general.prevent_sleep",
        |_, _| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.general.standalone_dialogs.label",
        "Standalone dialog windows",
        "general.standalone_dialogs",
        |_, _| {},
    );
    add_combo(
        &session,
        &g1,
        "settings.runtime.dashboard_card_variant.label",
        "Dashboard cards",
        "runtime.dashboard_card_variant",
        &["compact", "detailed"],
        None,
        |_| {},
    );
    add_combo(
        &session,
        &g1,
        "titlebar.menu.theme",
        "Theme",
        "runtime.theme",
        &["system", "light", "dark"],
        None,
        |ctx| ctx.apply_theme(),
    );
    add_combo(
        &session,
        &g1,
        "settings.general.tray_icon_theme.label",
        "Tray icon theme",
        "general.tray_icon_theme",
        &[
            "system",
            "color",
            "monochrome_light",
            "monochrome_dark",
            "symbolic",
        ],
        None,
        |_| {},
    );
    add_combo(
        &session,
        &g1,
        "nautilus.sort.label",
        "Files sort",
        "nautilus.sort_by",
        &["name", "size", "modified"],
        Some(("nautilus.sort.defaultHint", "Default Nautilus listing sort")),
        |_| {},
    );
    add_switch(
        &session,
        &g1,
        "nautilus.sort.descending",
        "Sort files descending",
        "nautilus.sort_desc",
        |_, _| {},
    );
    add_combo(
        &session,
        &g1,
        "nautilus.view.toggleLayout",
        "Files layout",
        "nautilus.layout",
        &["list", "grid"],
        None,
        |_| {},
    );
    add_switch(
        &session,
        &g1,
        "nautilus.view.showHidden",
        "Show Hidden Files",
        "nautilus.show_hidden",
        |_, _| {},
    );
    add_int_combo(
        &session,
        &g1,
        "nautilus.view.iconSize",
        "List icon size",
        "nautilus.icon_size",
        &[16, 24, 32, 48],
    );
    add_int_combo(
        &session,
        &g1,
        "nautilus.view.larger",
        "Grid icon size",
        "nautilus.grid_icon_size",
        &[48, 64, 96, 128, 192, 256],
    );
    add_switch(
        &session,
        &g1,
        "settings.runtime.flatpak_warn.label",
        "Flatpak Warning Shown",
        "runtime.flatpak_warn",
        |_, _| {},
    );
    add_combo(
        &session,
        &g1,
        "settings.runtime.app_update_channel.label",
        "App update channel",
        "runtime.app_update_channel",
        &["stable", "beta"],
        None,
        |_| {},
    );
    add_combo(
        &session,
        &g1,
        "settings.runtime.rclone_update_channel.label",
        "rclone update channel",
        "runtime.rclone_update_channel",
        &["stable", "beta"],
        None,
        |_| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.runtime.app_auto_check_updates.label",
        "Check for app updates",
        "runtime.app_auto_check_updates",
        |_, _| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.runtime.rclone_auto_check_updates.label",
        "Check for rclone updates",
        "runtime.rclone_auto_check_updates",
        |_, _| {},
    );
    add_switch(
        &session,
        &g1,
        "settings.runtime.show_json_mode.label",
        "JSON mode for flag editors",
        "runtime.show_json_mode",
        |_, _| {},
    );
    add_reset_all(&session, &g1, parent);
    add_skip_updates(&session, &g1);
    general.add(&g1);

    let core = adw::PreferencesPage::new();
    core.set_name(Some("core"));
    core.set_title(&ctx.t_or("modals.preferences.tabs.core", "Core"));
    core.set_icon_name(Some("application-x-executable-symbolic"));
    let c1 = adw::PreferencesGroup::new();
    c1.set_title(&ctx.t_or("titlebar.menu.installRclone", "Rclone"));
    core.add(&session.banner);
    let restart = gtk::Button::with_label(&ctx.t_or(
        "modals.preferences.aria.saveAndRestart",
        "Restart rclone engine",
    ));
    restart.set_tooltip_text(Some(&ctx.t_or(
        "settings.core.rclone_flags.description",
        "Required after changing the binary, extra flags, or environment",
    )));
    {
        let session = session.clone();
        restart.connect_clicked(move |_| session.save_and_restart());
    }
    c1.add(&{
        let row = adw::ActionRow::new();
        row.set_title(&ctx.t_or(
            "modals.preferences.aria.saveAndRestart",
            "Restart rclone engine",
        ));
        row.add_suffix(&restart);
        row
    });
    add_entry(
        &session,
        &c1,
        "settings.core.rclone_binary.label",
        "Rclone binary",
        "core.rclone_binary",
        " ",
        |text| json!(text),
        |_| {},
        None,
    );
    add_entry(
        &session,
        &c1,
        "settings.core.bandwidth_limit.label",
        "Bandwidth limit",
        "core.bandwidth_limit",
        " ",
        |text| json!(text),
        |ctx| ctx.apply_effective_bandwidth(),
        Some(validate_bandwidth_row),
    );
    add_entry(
        &session,
        &c1,
        "settings.core.metered_bandwidth_limit.label",
        "Bandwidth limit on metered networks",
        "core.metered_bandwidth_limit",
        " ",
        |text| json!(text),
        |ctx| ctx.apply_effective_bandwidth(),
        Some(validate_bandwidth_row),
    );
    add_entry(
        &session,
        &c1,
        "settings.core.default_mount_directory.label",
        "Default mount directory ({home}/rclone-manager/{remote})",
        "core.default_mount_directory",
        " ",
        |text| json!(text),
        |_| {},
        None,
    );
    add_entry(
        &session,
        &c1,
        "settings.core.default_bisync_directory.label",
        "Default bisync directory ({home}/rclone-manager/{remote}-bisync)",
        "core.default_bisync_directory",
        " ",
        |text| json!(text),
        |_| {},
        None,
    );
    add_spin(
        &session,
        &c1,
        "settings.core.max_tray_items.label",
        "Max tray items",
        "core.max_tray_items",
        1.0,
        40.0,
        1.0,
    );
    add_entry(
        &session,
        &c1,
        "settings.core.rclone_flags.label",
        "Additional rclone flags (space-separated)",
        "core.rclone_additional_flags",
        " ",
        |text| {
            json!(text
                .split_whitespace()
                .map(|s| s.to_string())
                .collect::<Vec<_>>())
        },
        |_| {},
        None,
    );
    add_entry(
        &session,
        &c1,
        "settings.core.rclone_env_vars.label",
        "Rclone environment (KEY=value;KEY=value)",
        "core.rclone_env_vars",
        ";",
        |text| {
            json!(text
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>())
        },
        |_| {},
        None,
    );
    add_entry(
        &session,
        &c1,
        "settings.core.connection_check_urls.label",
        "Connectivity check URLs (comma-separated)",
        "core.connection_check_urls",
        ", ",
        |text| {
            json!(text
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>())
        },
        |_| {},
        None,
    );
    core.add(&c1);

    let security = adw::PreferencesPage::new();
    security.set_name(Some("security"));
    security.set_title(&ctx.t_or("modals.backend.security.encrypted", "Security"));
    security.set_icon_name(Some("security-high-symbolic"));
    add_security_page(&session, &security, parent);

    let dev = adw::PreferencesPage::new();
    dev.set_name(Some("developer"));
    dev.set_title(&ctx.t_or("modals.preferences.tabs.developer", "Developer"));
    dev.set_icon_name(Some("applications-engineering-symbolic"));
    add_developer_page(&session, &dev, parent);

    dialog.add(&general);
    dialog.add(&core);
    dialog.add(&security);
    dialog.add(&dev);
    dialog.add(&search_page(&ctx, &dialog));
    if let Some(name) = page.filter(|name| !name.is_empty()) {
        dialog.set_visible_page_name(name);
    }
    dialog.present(Some(parent));
}

fn search_page(ctx: &AppCtx, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_name(Some("search"));
    page.set_title(&ctx.t_or("modals.preferences.searchResults", "Search Results"));
    page.set_icon_name(Some("edit-find-symbolic"));
    let query = adw::EntryRow::new();
    query.set_title(&ctx.t_or("modals.preferences.searchPlaceholder", "Search settings..."));
    let query_group = adw::PreferencesGroup::new();
    query_group.add(&query);
    let chips_row = adw::ActionRow::new();
    chips_row.set_title(&ctx.t_or("modals.preferences.trySearching", "Try searching for:"));
    for (id, key, fallback) in crate::pref_search::SUGGESTIONS {
        let label = ctx.t_or(key, fallback);
        let btn = gtk::Button::with_label(&label);
        btn.add_css_class("pill");
        btn.set_valign(gtk::Align::Center);
        btn.set_tooltip_text(Some(&ctx.tf(
            "modals.preferences.aria.searchFor",
            &[("suggestion", &label)],
        )));
        let query = query.clone();
        let text = if *id == "apiPort" {
            "port".to_string()
        } else {
            label.clone()
        };
        btn.connect_clicked(move |_| query.set_text(&text));
        chips_row.add_suffix(&btn);
    }
    let results = adw::PreferencesGroup::new();
    results.set_title(&ctx.t_or("modals.preferences.searchResults", "Search Results"));
    let empty = adw::ActionRow::new();
    empty.set_title(&ctx.t_or(
        "modals.preferences.noSettingsFound",
        "No settings found matching",
    ));
    empty.set_visible(false);
    results.add(&empty);
    let result_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let refresh = {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let results = results.clone();
        let empty = empty.clone();
        let chips_row = chips_row.clone();
        let result_rows = result_rows.clone();
        Rc::new(move |text: String| {
            for row in result_rows.borrow().iter() {
                results.remove(row);
            }
            result_rows.borrow_mut().clear();
            let query = text.trim().to_string();
            if query.is_empty() {
                empty.set_visible(false);
                chips_row.set_visible(true);
                return;
            }
            let hits = crate::pref_search::matching_items(&query, &ctx.i18n.borrow());
            chips_row.set_visible(hits.is_empty());
            empty.set_visible(hits.is_empty());
            empty.set_subtitle(&format!("\"{query}\""));
            for item in hits {
                let row = adw::ActionRow::new();
                row.set_title(&ctx.t_or(item.title_key, item.title_fallback));
                row.set_subtitle(&format!(
                    "{} · {}",
                    ctx.t_or(&format!("modals.preferences.tabs.{}", item.page), item.page),
                    ctx.t_or(item.help_key, item.help_fallback)
                ));
                row.set_activatable(true);
                let dialog = dialog.clone();
                let page = item.page.to_string();
                row.connect_activated(move |_| {
                    dialog.set_visible_page_name(&page);
                });
                results.add(&row);
                result_rows.borrow_mut().push(row);
            }
        })
    };
    {
        let refresh = refresh.clone();
        query.connect_changed(move |row| refresh(row.text().to_string()));
    }
    let chips_group = adw::PreferencesGroup::new();
    chips_group.add(&chips_row);
    page.add(&query_group);
    page.add(&chips_group);
    page.add(&results);
    page
}

fn apply_help_subtitle(
    session: &PrefsSession,
    row: &impl adw::prelude::ActionRowExt,
    label_key: &str,
) {
    if row.subtitle().is_some_and(|text| !text.is_empty()) {
        return;
    }
    if let Some(help) = help_text(session, label_key) {
        row.set_subtitle(&help);
    }
}

fn apply_help_tooltip(session: &PrefsSession, widget: &impl IsA<gtk::Widget>, label_key: &str) {
    if let Some(help) = help_text(session, label_key) {
        widget.set_tooltip_text(Some(&help));
    }
}

fn help_text(session: &PrefsSession, label_key: &str) -> Option<String> {
    let key = crate::pref_search::help_key_from_label(label_key)?;
    session
        .ctx
        .i18n
        .borrow()
        .has(&key)
        .then(|| session.ctx.t(&key))
}

fn add_language_row(session: &PrefsSession, group: &adw::PreferencesGroup) {
    let langs = crate::i18n::SUPPORTED_LANGUAGES;
    let lang_labels = [
        "English (US)",
        "Türkçe (Türkiye)",
        "Español (España)",
        "中文 (简体)",
        "Français (France)",
        "Українська (Україна)",
        "Русский (Россия)",
        "Português (Brasil)",
        "日本語 (日本)",
    ];
    let row = adw::ComboRow::new();
    row.set_title(
        &session
            .ctx
            .t_or("settings.general.language.label", "Language"),
    );
    row.set_subtitle(&session.ctx.t_or(
        "settings.general.language.description",
        "Application language",
    ));
    row.set_model(Some(&gtk::StringList::new(&lang_labels)));
    if let Some(idx) = langs
        .iter()
        .position(|l| *l == session.ctx.settings.borrow().general.language)
    {
        row.set_selected(idx as u32);
    }
    {
        let session = session.clone();
        row.connect_selected_notify(move |row| {
            if session.suppress.get() {
                return;
            }
            let idx = row.selected() as usize;
            if let Some(code) = langs.get(idx) {
                session.commit("general.language", json!(code));
                *session.ctx.i18n.borrow_mut() = crate::i18n::I18n::load(code);
            }
        });
    }
    let row_reset = row.clone();
    let session_reset = session.clone();
    row.add_suffix(&session.reset_button("general.language", move |value| {
        let code = value.as_str().unwrap_or("en-US");
        if let Some(idx) = langs.iter().position(|l| *l == code) {
            row_reset.set_selected(idx as u32);
        }
        *session_reset.ctx.i18n.borrow_mut() = crate::i18n::I18n::load(code);
    }));
    {
        let row = row.clone();
        let ctx = session.ctx.clone();
        session.remember_restorer("general.language", move || {
            if let Some(idx) = langs
                .iter()
                .position(|l| *l == ctx.settings.borrow().general.language)
            {
                row.set_selected(idx as u32);
            }
        });
    }
    group.add(&row);
}

fn add_switch(
    session: &PrefsSession,
    group: &adw::PreferencesGroup,
    key: &str,
    fallback: &str,
    path: &'static str,
    extra: impl Fn(&AppCtx, &Value) + 'static,
) {
    let active = session
        .ctx
        .settings
        .borrow()
        .get_by_path(path)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let row = adw::SwitchRow::new();
    row.set_title(&session.ctx.t_or(key, fallback));
    apply_help_subtitle(session, &row, key);
    row.set_active(active);
    let extra = Rc::new(extra);
    {
        let session = session.clone();
        let extra = extra.clone();
        row.connect_active_notify(move |row| {
            if session.suppress.get() {
                return;
            }
            let value = json!(row.is_active());
            extra(&session.ctx, &value);
            session.commit(path, value);
        });
    }
    let row_reset = row.clone();
    let session_extra = session.clone();
    row.add_suffix(&session.reset_button(path, {
        let extra = extra.clone();
        move |value| {
            if let Some(on) = value.as_bool() {
                row_reset.set_active(on);
            }
            extra(&session_extra.ctx, value);
        }
    }));
    {
        let row = row.clone();
        let ctx = session.ctx.clone();
        session.remember_restorer(path, move || {
            let on = ctx
                .settings
                .borrow()
                .get_by_path(path)
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            row.set_active(on);
        });
    }
    group.add(&row);
}

fn add_combo(
    session: &PrefsSession,
    group: &adw::PreferencesGroup,
    key: &str,
    fallback: &str,
    path: &'static str,
    options: &'static [&'static str],
    subtitle: Option<(&str, &str)>,
    extra: impl Fn(&AppCtx) + 'static,
) {
    let current = session
        .ctx
        .settings
        .borrow()
        .get_by_path(path)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let row = adw::ComboRow::new();
    row.set_title(&session.ctx.t_or(key, fallback));
    apply_help_subtitle(session, &row, key);
    if let Some((sub_key, sub_fallback)) = subtitle {
        row.set_subtitle(&session.ctx.t_or(sub_key, sub_fallback));
    }
    let labels: Vec<String> = options
        .iter()
        .map(|id| {
            session
                .ctx
                .t_or(&format!("settings.{path}.options.{id}"), id)
        })
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    row.set_model(Some(&gtk::StringList::new(&label_refs)));
    if let Some(idx) = options.iter().position(|item| *item == current) {
        row.set_selected(idx as u32);
    }
    let extra = Rc::new(extra);
    {
        let session = session.clone();
        let extra = extra.clone();
        row.connect_selected_notify(move |row| {
            if session.suppress.get() {
                return;
            }
            if let Some(value) = options.get(row.selected() as usize) {
                session.commit(path, json!(value));
                extra(&session.ctx);
            }
        });
    }
    let row_reset = row.clone();
    let extra_reset = extra.clone();
    let session_extra = session.clone();
    row.add_suffix(&session.reset_button(path, move |value| {
        let text = value.as_str().unwrap_or("");
        if let Some(idx) = options.iter().position(|item| *item == text) {
            row_reset.set_selected(idx as u32);
        }
        extra_reset(&session_extra.ctx);
    }));
    {
        let row = row.clone();
        let ctx = session.ctx.clone();
        session.remember_restorer(path, move || {
            let text = ctx
                .settings
                .borrow()
                .get_by_path(path)
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default();
            if let Some(idx) = options.iter().position(|item| *item == text) {
                row.set_selected(idx as u32);
            }
        });
    }
    group.add(&row);
}

fn add_int_combo(
    session: &PrefsSession,
    group: &adw::PreferencesGroup,
    key: &str,
    fallback: &str,
    path: &'static str,
    options: &'static [i64],
) {
    let labels: Vec<String> = options.iter().map(|n| n.to_string()).collect();
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let current = session
        .ctx
        .settings
        .borrow()
        .get_by_path(path)
        .and_then(|v| v.as_i64())
        .unwrap_or(options.first().copied().unwrap_or(0));
    let row = adw::ComboRow::new();
    row.set_title(&session.ctx.t_or(key, fallback));
    apply_help_subtitle(session, &row, key);
    row.set_model(Some(&gtk::StringList::new(&refs)));
    if let Some(idx) = options.iter().position(|item| *item == current) {
        row.set_selected(idx as u32);
    }
    {
        let session = session.clone();
        row.connect_selected_notify(move |row| {
            if session.suppress.get() {
                return;
            }
            if let Some(value) = options.get(row.selected() as usize) {
                session.commit(path, json!(value));
            }
        });
    }
    let row_reset = row.clone();
    row.add_suffix(&session.reset_button(path, move |value| {
        let n = value.as_i64().unwrap_or(0);
        if let Some(idx) = options.iter().position(|item| *item == n) {
            row_reset.set_selected(idx as u32);
        }
    }));
    {
        let row = row.clone();
        let ctx = session.ctx.clone();
        session.remember_restorer(path, move || {
            let n = ctx
                .settings
                .borrow()
                .get_by_path(path)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if let Some(idx) = options.iter().position(|item| *item == n) {
                row.set_selected(idx as u32);
            }
        });
    }
    group.add(&row);
}

fn add_entry(
    session: &PrefsSession,
    group: &adw::PreferencesGroup,
    key: &str,
    fallback: &str,
    path: &'static str,
    display_sep: &'static str,
    encode: impl Fn(&str) -> Value + 'static,
    extra: impl Fn(&AppCtx) + 'static,
    validate: Option<fn(&adw::EntryRow) -> bool>,
) {
    let current = session
        .ctx
        .settings
        .borrow()
        .get_by_path(path)
        .map(|v| display_setting(&v, display_sep))
        .unwrap_or_default();
    let row = adw::EntryRow::new();
    row.set_title(&session.ctx.t_or(key, fallback));
    apply_help_tooltip(session, &row, key);
    row.set_text(&current);
    let encode = Rc::new(encode);
    let extra = Rc::new(extra);
    {
        let session = session.clone();
        let encode = encode.clone();
        let extra = extra.clone();
        row.connect_changed(move |row| {
            if session.suppress.get() {
                return;
            }
            if let Some(validate) = validate {
                if !validate(row) {
                    return;
                }
            }
            session.commit(path, encode(&row.text()));
            extra(&session.ctx);
        });
    }
    let row_reset = row.clone();
    row.add_suffix(&session.reset_button(path, {
        let extra = extra.clone();
        let session_extra = session.clone();
        move |value| {
            row_reset.set_text(&display_setting(value, display_sep));
            extra(&session_extra.ctx);
        }
    }));
    {
        let row = row.clone();
        let ctx = session.ctx.clone();
        session.remember_restorer(path, move || {
            let text = ctx
                .settings
                .borrow()
                .get_by_path(path)
                .map(|v| display_setting(&v, display_sep))
                .unwrap_or_default();
            row.set_text(&text);
        });
    }
    group.add(&row);
}

fn add_spin(
    session: &PrefsSession,
    group: &adw::PreferencesGroup,
    key: &str,
    fallback: &str,
    path: &'static str,
    min: f64,
    max: f64,
    step: f64,
) {
    let current = session
        .ctx
        .settings
        .borrow()
        .get_by_path(path)
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let row = adw::SpinRow::with_range(min, max, step);
    row.set_title(&session.ctx.t_or(key, fallback));
    apply_help_tooltip(session, &row, key);
    row.set_value(current as f64);
    {
        let session = session.clone();
        row.connect_changed(move |row| {
            if session.suppress.get() {
                return;
            }
            session.commit(path, json!(row.value() as u64));
        });
    }
    let row_reset = row.clone();
    row.add_suffix(&session.reset_button(path, move |value| {
        if let Some(n) = value.as_u64() {
            row_reset.set_value(n as f64);
        }
    }));
    {
        let row = row.clone();
        let ctx = session.ctx.clone();
        session.remember_restorer(path, move || {
            let n = ctx
                .settings
                .borrow()
                .get_by_path(path)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            row.set_value(n as f64);
        });
    }
    group.add(&row);
}

fn validate_bandwidth_row(row: &adw::EntryRow) -> bool {
    match crate::validators::validate_bandwidth(&row.text()) {
        Ok(()) => {
            row.remove_css_class("error");
            row.set_tooltip_text(None);
            true
        }
        Err(msg) => {
            row.add_css_class("error");
            row.set_tooltip_text(Some(&msg));
            false
        }
    }
}

fn add_reset_all(
    session: &PrefsSession,
    group: &adw::PreferencesGroup,
    parent: &impl IsA<gtk::Widget>,
) {
    let reset = gtk::Button::with_label(
        &session
            .ctx
            .t_or("modals.preferences.resetAll", "Reset all settings"),
    );
    reset.add_css_class("destructive-action");
    {
        let ctx = session.ctx.clone();
        let session = session.clone();
        let parent = parent.clone();
        reset.connect_clicked(move |_| {
            let alert = adw::AlertDialog::new(
                Some(&ctx.t_or("settings.resetAll.title", "Reset Settings")),
                Some(&ctx.t_or(
                    "settings.resetAll.message",
                    "Are you sure you want to reset all app settings? This cannot be undone.",
                )),
            );
            alert.add_response("cancel", &ctx.t("common.cancel"));
            alert.add_response("reset", &ctx.t_or("common.reset", "Reset"));
            alert.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
            let ctx = ctx.clone();
            let session = session.clone();
            alert.connect_response(None, move |_, response| {
                if response != "reset" {
                    return;
                }
                let mut next = crate::settings::AppSettings::default();
                next.core.completed_onboarding = true;
                let lang = next.general.language.clone();
                *ctx.settings.borrow_mut() = next;
                *ctx.i18n.borrow_mut() = crate::i18n::I18n::load(&lang);
                ctx.persist();
                ctx.apply_theme();
                session.pending.borrow_mut().clear();
                session.refresh_banner();
            });
            alert.present(Some(&parent));
        });
    }
    let row = adw::ActionRow::new();
    row.set_title(
        &session
            .ctx
            .t_or("modals.preferences.resetAll", "Reset all settings"),
    );
    row.add_suffix(&reset);
    group.add(&row);
}

fn add_skip_updates(session: &PrefsSession, group: &adw::PreferencesGroup) {
    let skip_updates = gtk::Button::with_label(
        &session
            .ctx
            .t_or("modals.about.skipVersion", "Skip pending updates"),
    );
    {
        let ctx = session.ctx.clone();
        skip_updates.connect_clicked(move |_| {
            let pending = ctx.updates.borrow().clone();
            let mut settings = ctx.settings.borrow_mut();
            if let Some(app) = &pending.app {
                if !settings.runtime.app_skipped_updates.contains(&app.latest) {
                    settings
                        .runtime
                        .app_skipped_updates
                        .push(app.latest.clone());
                }
            }
            if let Some(rclone) = &pending.rclone {
                if !settings
                    .runtime
                    .rclone_skipped_updates
                    .contains(&rclone.latest)
                {
                    settings
                        .runtime
                        .rclone_skipped_updates
                        .push(rclone.latest.clone());
                }
            }
            drop(settings);
            ctx.persist();
            *ctx.updates.borrow_mut() = crate::updater::PendingUpdates::default();
        });
    }
    let row = adw::ActionRow::new();
    row.set_title(
        &session
            .ctx
            .t_or("modals.about.skipVersion", "Skip pending updates"),
    );
    row.add_suffix(&skip_updates);
    group.add(&row);
}

fn add_security_page(
    session: &PrefsSession,
    page: &adw::PreferencesPage,
    parent: &impl IsA<gtk::Widget>,
) {
    let ctx = &session.ctx;
    let s1 = adw::PreferencesGroup::new();
    s1.set_title(&ctx.t_or(
        "modals.backend.security.configPassword",
        "rclone.conf password",
    ));
    let stored = adw::PasswordEntryRow::new();
    stored.set_title(&ctx.t_or("modals.backend.security.password", "Stored password"));
    stored.set_text(&crate::keyring::resolve_config_password(
        &ctx.settings.borrow().core.config_password,
    ));
    {
        let ctx = ctx.clone();
        stored.connect_changed(move |row| {
            let mut settings = ctx.settings.borrow_mut();
            crate::keyring::persist_password_setting(
                &mut settings.core.config_password,
                &row.text(),
            );
            drop(settings);
            ctx.persist();
        });
    }
    s1.add(&stored);
    let keyring_row = adw::ActionRow::new();
    keyring_row.set_title(&ctx.t_or("modals.backend.security.systemKeychain", "OS keyring"));
    keyring_row.set_subtitle(&if crate::keyring::load_password().is_some() {
        ctx.t_or(
            "modals.backend.security.passwordStoredInKeyring",
            "rclone.conf password is stored in the system keyring",
        )
    } else if ctx.settings.borrow().core.config_password.is_empty() {
        ctx.t_or(
            "modals.backend.security.protectCredentials",
            "No password stored. Saving will prefer the system keyring when available.",
        )
    } else {
        ctx.t_or(
            "modals.backend.security.credentialsPlainText",
            "Password is stored in settings.json because the keyring is unavailable",
        )
    });
    s1.add(&keyring_row);
    let validate = gtk::Button::with_label(&ctx.t_or("common.ok", "Validate"));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        validate.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let client = ctx.client();
            let msg = match crate::security::validate_password_for(
                client.as_ref(),
                &binary,
                &stored.text(),
            ) {
                Ok(()) => "Password accepted".into(),
                Err(e) => e,
            };
            let alert = adw::AlertDialog::new(
                Some(&ctx.t_or("modals.backend.security.configPassword", "Config password")),
                Some(&msg),
            );
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&parent));
        });
    }
    let encrypt = gtk::Button::with_label(
        &ctx.t_or("modals.backend.security.enableEncryption", "Encrypt config"),
    );
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        encrypt.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let client = ctx.client();
            let msg =
                match crate::security::encrypt_config_for(client.as_ref(), &binary, &stored.text())
                {
                    Ok(()) => {
                        ctx.restart_engine();
                        "rclone.conf encrypted".into()
                    }
                    Err(e) => e,
                };
            let alert = adw::AlertDialog::new(
                Some(&ctx.t_or("modals.backend.security.enableEncryption", "Encrypt")),
                Some(&msg),
            );
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&parent));
        });
    }
    let new_pass = adw::PasswordEntryRow::new();
    new_pass.set_title(&ctx.t_or(
        "modals.backend.security.newPassword",
        "New password (change)",
    ));
    s1.add(&new_pass);
    let change = gtk::Button::with_label(
        &ctx.t_or("modals.backend.security.changePassword", "Change password"),
    );
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        let new_pass = new_pass.clone();
        change.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let client = ctx.client();
            let msg = match crate::security::change_password_for(
                client.as_ref(),
                &binary,
                &stored.text(),
                &new_pass.text(),
            ) {
                Ok(()) => {
                    crate::keyring::persist_password_setting(
                        &mut ctx.settings.borrow_mut().core.config_password,
                        &new_pass.text(),
                    );
                    ctx.persist();
                    ctx.restart_engine();
                    "Password changed".into()
                }
                Err(e) => e,
            };
            let alert = adw::AlertDialog::new(
                Some(&ctx.t_or("modals.backend.security.changePassword", "Change password")),
                Some(&msg),
            );
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&parent));
        });
    }
    let unencrypt = gtk::Button::with_label(&ctx.t_or(
        "modals.backend.security.removeEncryption",
        "Remove encryption",
    ));
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        unencrypt.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let client = ctx.client();
            let msg = match crate::security::unencrypt_config_for(
                client.as_ref(),
                &binary,
                &stored.text(),
            ) {
                Ok(()) => {
                    let _ = crate::keyring::delete_password();
                    ctx.settings.borrow_mut().core.config_password.clear();
                    ctx.persist();
                    ctx.restart_engine();
                    "rclone.conf encryption removed".into()
                }
                Err(e) => e,
            };
            let alert = adw::AlertDialog::new(
                Some(&ctx.t_or("modals.backend.security.removeEncryption", "Unencrypt")),
                Some(&msg),
            );
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&parent));
        });
    }
    let sec_row = adw::ActionRow::new();
    sec_row.set_title(&ctx.t_or("common.moreActions", "Actions"));
    sec_row.add_suffix(&validate);
    sec_row.add_suffix(&encrypt);
    sec_row.add_suffix(&change);
    sec_row.add_suffix(&unencrypt);
    s1.add(&sec_row);
    page.add(&s1);
}

fn add_developer_page(
    session: &PrefsSession,
    page: &adw::PreferencesPage,
    parent: &impl IsA<gtk::Widget>,
) {
    let d1 = adw::PreferencesGroup::new();
    add_combo(
        session,
        &d1,
        "settings.developer.log_level.label",
        "Log level",
        "developer.log_level",
        &["error", "warn", "info", "debug", "trace"],
        None,
        |_| {},
    );
    add_switch(
        session,
        &d1,
        "settings.developer.destroy_window_on_close.label",
        "Destroy window on close",
        "developer.destroy_window_on_close",
        |_, _| {},
    );
    let open_cfg = gtk::Button::with_label(
        &session
            .ctx
            .t_or("titlebar.menu.openConfig", "Open config folder"),
    );
    open_cfg.connect_clicked(|_| {
        let _ = open::that(crate::settings::AppSettings::config_dir());
    });
    let open_cache = gtk::Button::with_label(
        &session
            .ctx
            .t_or("titlebar.menu.openCache", "Open cache folder"),
    );
    open_cache.connect_clicked(|_| {
        let dir = crate::settings::AppSettings::cache_dir();
        let _ = std::fs::create_dir_all(&dir);
        let _ = open::that(dir);
    });
    let open_log =
        gtk::Button::with_label(&session.ctx.t_or("titlebar.menu.openLog", "Open rclone log"));
    open_log.connect_clicked(|_| {
        let path = crate::settings::AppSettings::log_path();
        let _ = open::that(path);
    });
    let cfg_row = adw::ActionRow::new();
    cfg_row.set_title(&session.ctx.t_or("developerTools.folders", "Folders"));
    cfg_row.add_suffix(&open_cfg);
    cfg_row.add_suffix(&open_cache);
    cfg_row.add_suffix(&open_log);
    d1.add(&cfg_row);
    let gc = gtk::Button::with_label(&session.ctx.t_or("titlebar.menu.runGc", "Run GC"));
    {
        let ctx = session.ctx.clone();
        gc.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.gc();
            }
        });
    }
    let fscache = gtk::Button::with_label(
        &session
            .ctx
            .t_or("titlebar.menu.clearFsCache", "Clear FS cache"),
    );
    {
        let ctx = session.ctx.clone();
        fscache.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.fscache_clear();
            }
        });
    }
    let ping = gtk::Button::with_label(
        &session
            .ctx
            .t_or("titlebar.menu.checkConnectivity", "Check connectivity"),
    );
    {
        let ctx = session.ctx.clone();
        let parent = parent.clone();
        ping.connect_clicked(move |_| {
            let urls = ctx.settings.borrow().core.connection_check_urls.clone();
            let results = crate::connection::check_links(&urls, 4);
            let body = results
                .iter()
                .map(|r| {
                    format!(
                        "{} — {} ({})",
                        r.url,
                        if r.ok { "ok" } else { "fail" },
                        r.detail
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let alert =
                adw::AlertDialog::new(Some(&crate::connection::summarize(&results)), Some(&body));
            alert.add_response("ok", &ctx.t("common.ok"));
            alert.present(Some(&parent));
        });
    }
    let maint = adw::ActionRow::new();
    maint.set_title(
        &session
            .ctx
            .t_or("developerTools.maintenance", "Maintenance"),
    );
    maint.add_suffix(&gc);
    maint.add_suffix(&fscache);
    maint.add_suffix(&ping);
    d1.add(&maint);
    page.add(&d1);
}
