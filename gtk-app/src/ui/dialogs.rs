use super::AppCtx;
use crate::backup;
use crate::jobs::{
    build_job_params, default_dest, default_source, flatten_rclone, job_from_status,
    merge_template_into, start_request,
};
use crate::operations::OperationType;
use crate::rclone::{
    browse_target, describe_cron, nanoseconds_to_duration, parse_hashsum, remote_fs, validate_cron,
};
use crate::rename::{preview as rename_preview, RenameMode, RenamePlan};
use crate::store::{
    AlertAction, AlertEvent, AlertEventKind, AlertRule, AlertSeverity, ProfileConfig, QuickRun,
};
use adw::prelude::*;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub fn prompt(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    label: &str,
    initial: &str,
    on_ok: impl Fn(String) + 'static,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(label));
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", "OK");
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response == "ok" {
            on_ok(entry.text().to_string());
        }
    });
    dialog.present(Some(parent));
}

pub fn preferences(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");
    dialog.set_search_enabled(true);

    let general = adw::PreferencesPage::new();
    general.set_title("General");
    general.set_icon_name(Some("preferences-system-symbolic"));
    let g1 = adw::PreferencesGroup::new();
    g1.set_title("Appearance & language");

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
    let lang_model = gtk::StringList::new(&lang_labels);
    let lang = adw::ComboRow::new();
    lang.set_title(&ctx.t_or("settings.general.language.label", "Language"));
    lang.set_subtitle(&ctx.t_or(
        "settings.general.language.description",
        "Application language",
    ));
    lang.set_model(Some(&lang_model));
    if let Some(idx) = langs
        .iter()
        .position(|l| *l == ctx.settings.borrow().general.language)
    {
        lang.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        lang.connect_selected_notify(move |row| {
            let idx = row.selected() as usize;
            if let Some(code) = langs.get(idx) {
                ctx.settings.borrow_mut().general.language = (*code).to_string();
                *ctx.i18n.borrow_mut() = crate::i18n::I18n::load(code);
                ctx.persist();
            }
        });
    }
    g1.add(&lang);

    let views = ["main_menu", "nautilus", "flow"];
    let view_model = gtk::StringList::new(&views);
    let view = adw::ComboRow::new();
    view.set_title("Default view");
    view.set_model(Some(&view_model));
    if let Some(idx) = views
        .iter()
        .position(|v| *v == ctx.settings.borrow().general.default_view)
    {
        view.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        view.connect_selected_notify(move |row| {
            if let Some(v) = views.get(row.selected() as usize) {
                ctx.settings.borrow_mut().general.default_view = (*v).to_string();
                ctx.persist();
            }
        });
    }
    g1.add(&view);

    g1.add(&switch_row(
        "Enable tray",
        ctx.settings.borrow().general.tray_enabled,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().general.tray_enabled = v
        },
    ));
    g1.add(&switch_row(
        "Start on startup",
        ctx.settings.borrow().general.start_on_startup,
        {
            let ctx = ctx.clone();
            move |v| {
                ctx.settings.borrow_mut().general.start_on_startup = v;
                let _ = crate::platform::set_autostart(v);
            }
        },
    ));
    g1.add(&switch_row(
        "Notifications",
        ctx.settings.borrow().general.notifications,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().general.notifications = v
        },
    ));
    g1.add(&switch_row(
        "Restrict sensitive values",
        ctx.settings.borrow().general.restrict,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().general.restrict = v
        },
    ));
    g1.add(&switch_row(
        "Prevent sleep during jobs",
        ctx.settings.borrow().general.prevent_sleep,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().general.prevent_sleep = v
        },
    ));
    g1.add(&switch_row(
        "Standalone dialog windows",
        ctx.settings.borrow().general.standalone_dialogs,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().general.standalone_dialogs = v
        },
    ));
    let cards = ["compact", "detailed"];
    let card_row = adw::ComboRow::new();
    card_row.set_title("Dashboard cards");
    card_row.set_model(Some(&gtk::StringList::new(&cards)));
    if let Some(idx) = cards
        .iter()
        .position(|c| *c == ctx.settings.borrow().runtime.dashboard_card_variant)
    {
        card_row.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        card_row.connect_selected_notify(move |row| {
            if let Some(v) = cards.get(row.selected() as usize) {
                ctx.settings.borrow_mut().runtime.dashboard_card_variant = (*v).to_string();
                ctx.persist();
            }
        });
    }
    g1.add(&card_row);
    let themes = ["system", "light", "dark"];
    let theme_row = adw::ComboRow::new();
    theme_row.set_title("Theme");
    theme_row.set_model(Some(&gtk::StringList::new(&themes)));
    if let Some(idx) = themes
        .iter()
        .position(|t| *t == ctx.settings.borrow().runtime.theme)
    {
        theme_row.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        theme_row.connect_selected_notify(move |row| {
            if let Some(v) = themes.get(row.selected() as usize) {
                ctx.settings.borrow_mut().runtime.theme = (*v).to_string();
                ctx.persist();
                ctx.apply_theme();
            }
        });
    }
    g1.add(&theme_row);
    let tray_themes = [
        "system",
        "color",
        "monochrome_light",
        "monochrome_dark",
        "symbolic",
    ];
    let tray_theme = adw::ComboRow::new();
    tray_theme.set_title(&ctx.t_or("settings.general.tray_icon_theme.label", "Tray icon theme"));
    tray_theme.set_model(Some(&gtk::StringList::new(&tray_themes)));
    if let Some(idx) = tray_themes
        .iter()
        .position(|t| *t == ctx.settings.borrow().general.tray_icon_theme)
    {
        tray_theme.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        tray_theme.connect_selected_notify(move |row| {
            if let Some(v) = tray_themes.get(row.selected() as usize) {
                ctx.settings.borrow_mut().general.tray_icon_theme = (*v).to_string();
                ctx.persist();
            }
        });
    }
    g1.add(&tray_theme);
    let sorts = ["name", "size", "modified"];
    let sort_row = adw::ComboRow::new();
    sort_row.set_title("Files sort");
    sort_row.set_subtitle("Default Nautilus listing sort");
    sort_row.set_model(Some(&gtk::StringList::new(&sorts)));
    if let Some(idx) = sorts
        .iter()
        .position(|s| *s == ctx.settings.borrow().nautilus.sort_by)
    {
        sort_row.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        sort_row.connect_selected_notify(move |row| {
            if let Some(v) = sorts.get(row.selected() as usize) {
                ctx.settings.borrow_mut().nautilus.sort_by = (*v).to_string();
                ctx.persist();
            }
        });
    }
    g1.add(&sort_row);
    g1.add(&switch_row(
        "Sort files descending",
        ctx.settings.borrow().nautilus.sort_desc,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().nautilus.sort_desc = v
        },
    ));
    let channels = ["stable", "beta"];
    let app_ch = adw::ComboRow::new();
    app_ch.set_title("App update channel");
    app_ch.set_model(Some(&gtk::StringList::new(&channels)));
    if let Some(idx) = channels
        .iter()
        .position(|c| *c == ctx.settings.borrow().runtime.app_update_channel)
    {
        app_ch.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        app_ch.connect_selected_notify(move |row| {
            if let Some(v) = channels.get(row.selected() as usize) {
                ctx.settings.borrow_mut().runtime.app_update_channel = (*v).to_string();
                ctx.persist();
            }
        });
    }
    g1.add(&app_ch);
    let rclone_ch = adw::ComboRow::new();
    rclone_ch.set_title("rclone update channel");
    rclone_ch.set_model(Some(&gtk::StringList::new(&channels)));
    if let Some(idx) = channels
        .iter()
        .position(|c| *c == ctx.settings.borrow().runtime.rclone_update_channel)
    {
        rclone_ch.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        rclone_ch.connect_selected_notify(move |row| {
            if let Some(v) = channels.get(row.selected() as usize) {
                ctx.settings.borrow_mut().runtime.rclone_update_channel = (*v).to_string();
                ctx.persist();
            }
        });
    }
    g1.add(&rclone_ch);
    g1.add(&switch_row(
        &ctx.t_or(
            "settings.runtime.app_auto_check_updates.label",
            "Check for app updates",
        ),
        ctx.settings.borrow().runtime.app_auto_check_updates,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().runtime.app_auto_check_updates = v
        },
    ));
    g1.add(&switch_row(
        &ctx.t_or(
            "settings.runtime.rclone_auto_check_updates.label",
            "Check for rclone updates",
        ),
        ctx.settings.borrow().runtime.rclone_auto_check_updates,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().runtime.rclone_auto_check_updates = v
        },
    ));
    g1.add(&switch_row(
        "JSON mode for flag editors",
        ctx.settings.borrow().runtime.show_json_mode,
        {
            let ctx = ctx.clone();
            move |v| {
                ctx.settings.borrow_mut().runtime.show_json_mode = v;
                ctx.persist();
            }
        },
    ));
    general.add(&g1);

    let core = adw::PreferencesPage::new();
    core.set_title("Core");
    core.set_icon_name(Some("application-x-executable-symbolic"));
    let c1 = adw::PreferencesGroup::new();
    c1.set_title("Rclone");
    let binary = adw::EntryRow::new();
    binary.set_title("Rclone binary");
    binary.set_text(&ctx.settings.borrow().core.rclone_binary);
    {
        let ctx = ctx.clone();
        binary.connect_changed(move |row| {
            ctx.settings.borrow_mut().core.rclone_binary = row.text().to_string();
            ctx.persist();
        });
    }
    let restart = gtk::Button::with_label("Restart rclone engine");
    restart.set_tooltip_text(Some(
        "Required after changing the binary, extra flags, or environment",
    ));
    {
        let ctx = ctx.clone();
        restart.connect_clicked(move |_| {
            ctx.restart_engine();
        });
    }
    c1.add(&restart);
    c1.add(&binary);
    let bw = adw::EntryRow::new();
    bw.set_title("Bandwidth limit");
    bw.set_text(&ctx.settings.borrow().core.bandwidth_limit);
    {
        let ctx = ctx.clone();
        bw.connect_changed(move |row| {
            let rate = row.text().to_string();
            ctx.settings.borrow_mut().core.bandwidth_limit = rate;
            ctx.persist();
            ctx.apply_effective_bandwidth();
        });
    }
    c1.add(&bw);
    let metered_bw = adw::EntryRow::new();
    metered_bw.set_title("Bandwidth limit on metered networks");
    metered_bw.set_text(&ctx.settings.borrow().core.metered_bandwidth_limit);
    {
        let ctx = ctx.clone();
        metered_bw.connect_changed(move |row| {
            ctx.settings.borrow_mut().core.metered_bandwidth_limit = row.text().to_string();
            ctx.persist();
            ctx.apply_effective_bandwidth();
        });
    }
    c1.add(&metered_bw);
    let tray_items = adw::SpinRow::with_range(1.0, 40.0, 1.0);
    tray_items.set_title("Max tray items");
    tray_items.set_value(ctx.settings.borrow().core.max_tray_items as f64);
    {
        let ctx = ctx.clone();
        tray_items.connect_changed(move |row| {
            ctx.settings.borrow_mut().core.max_tray_items = row.value() as usize;
            ctx.persist();
        });
    }
    c1.add(&tray_items);
    let flags = adw::EntryRow::new();
    flags.set_title("Additional rclone flags (space-separated)");
    flags.set_text(&ctx.settings.borrow().core.rclone_additional_flags.join(" "));
    {
        let ctx = ctx.clone();
        flags.connect_changed(move |row| {
            ctx.settings.borrow_mut().core.rclone_additional_flags = row
                .text()
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            ctx.persist();
        });
    }
    c1.add(&flags);
    let env = adw::EntryRow::new();
    env.set_title("Rclone environment (KEY=value;KEY=value)");
    env.set_text(&ctx.settings.borrow().core.rclone_env_vars.join(";"));
    {
        let ctx = ctx.clone();
        env.connect_changed(move |row| {
            ctx.settings.borrow_mut().core.rclone_env_vars = row
                .text()
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            ctx.persist();
        });
    }
    c1.add(&env);
    let urls = adw::EntryRow::new();
    urls.set_title("Connectivity check URLs (comma-separated)");
    urls.set_text(&ctx.settings.borrow().core.connection_check_urls.join(", "));
    {
        let ctx = ctx.clone();
        urls.connect_changed(move |row| {
            ctx.settings.borrow_mut().core.connection_check_urls = row
                .text()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            ctx.persist();
        });
    }
    c1.add(&urls);
    core.add(&c1);

    let dev = adw::PreferencesPage::new();
    dev.set_title("Developer");
    dev.set_icon_name(Some("applications-engineering-symbolic"));
    let d1 = adw::PreferencesGroup::new();
    let levels = ["error", "warn", "info", "debug", "trace"];
    let level_model = gtk::StringList::new(&levels);
    let level = adw::ComboRow::new();
    level.set_title("Log level");
    level.set_model(Some(&level_model));
    if let Some(idx) = levels
        .iter()
        .position(|l| *l == ctx.settings.borrow().developer.log_level)
    {
        level.set_selected(idx as u32);
    }
    {
        let ctx = ctx.clone();
        level.connect_selected_notify(move |row| {
            if let Some(v) = levels.get(row.selected() as usize) {
                ctx.settings.borrow_mut().developer.log_level = (*v).to_string();
                ctx.persist();
            }
        });
    }
    d1.add(&level);
    d1.add(&switch_row(
        "Destroy window on close",
        ctx.settings.borrow().developer.destroy_window_on_close,
        {
            let ctx = ctx.clone();
            move |v| ctx.settings.borrow_mut().developer.destroy_window_on_close = v
        },
    ));
    let open_cfg = gtk::Button::with_label("Open config folder");
    {
        open_cfg.connect_clicked(|_| {
            let _ = open::that(crate::settings::AppSettings::config_dir());
        });
    }
    let open_cache = gtk::Button::with_label("Open cache folder");
    {
        open_cache.connect_clicked(|_| {
            let dir = crate::settings::AppSettings::cache_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = open::that(dir);
        });
    }
    let open_log = gtk::Button::with_label("Open rclone log");
    {
        open_log.connect_clicked(|_| {
            let path = crate::settings::AppSettings::log_path();
            let _ = open::that(path);
        });
    }
    let cfg_row = adw::ActionRow::new();
    cfg_row.set_title("Folders");
    cfg_row.add_suffix(&open_cfg);
    cfg_row.add_suffix(&open_cache);
    cfg_row.add_suffix(&open_log);
    d1.add(&cfg_row);
    let gc = gtk::Button::with_label("Run GC");
    {
        let ctx = ctx.clone();
        gc.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.gc();
            }
        });
    }
    let fscache = gtk::Button::with_label("Clear FS cache");
    {
        let ctx = ctx.clone();
        fscache.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.fscache_clear();
            }
        });
    }
    let ping = gtk::Button::with_label("Check connectivity");
    {
        let ctx = ctx.clone();
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
            alert.add_response("ok", "OK");
            alert.present(Some(&parent));
        });
    }
    let maint = adw::ActionRow::new();
    maint.set_title("Maintenance");
    maint.add_suffix(&gc);
    maint.add_suffix(&fscache);
    maint.add_suffix(&ping);
    d1.add(&maint);
    dev.add(&d1);

    let security = adw::PreferencesPage::new();
    security.set_title("Security");
    security.set_icon_name(Some("security-high-symbolic"));
    let s1 = adw::PreferencesGroup::new();
    s1.set_title("rclone.conf password");
    let stored = adw::PasswordEntryRow::new();
    stored.set_title("Stored password");
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
    keyring_row.set_title("OS keyring");
    keyring_row.set_subtitle(if crate::keyring::load_password().is_some() {
        "rclone.conf password is stored in the system keyring"
    } else if ctx.settings.borrow().core.config_password.is_empty() {
        "No password stored. Saving will prefer the system keyring when available."
    } else {
        "Password is stored in settings.json because the keyring is unavailable"
    });
    s1.add(&keyring_row);
    let validate = gtk::Button::with_label("Validate");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        validate.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let msg = match crate::security::validate_password(&binary, &stored.text()) {
                Ok(()) => "Password accepted".into(),
                Err(e) => e,
            };
            let alert = adw::AlertDialog::new(Some("Config password"), Some(&msg));
            alert.add_response("ok", "OK");
            alert.present(Some(&parent));
        });
    }
    let encrypt = gtk::Button::with_label("Encrypt config");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        encrypt.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let msg = match crate::security::encrypt_config(&binary, &stored.text()) {
                Ok(()) => {
                    ctx.restart_engine();
                    "rclone.conf encrypted".into()
                }
                Err(e) => e,
            };
            let alert = adw::AlertDialog::new(Some("Encrypt"), Some(&msg));
            alert.add_response("ok", "OK");
            alert.present(Some(&parent));
        });
    }
    let new_pass = adw::PasswordEntryRow::new();
    new_pass.set_title("New password (change)");
    s1.add(&new_pass);
    let change = gtk::Button::with_label("Change password");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        let new_pass = new_pass.clone();
        change.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let msg =
                match crate::security::change_password(&binary, &stored.text(), &new_pass.text()) {
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
            let alert = adw::AlertDialog::new(Some("Change password"), Some(&msg));
            alert.add_response("ok", "OK");
            alert.present(Some(&parent));
        });
    }
    let unencrypt = gtk::Button::with_label("Remove encryption");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let stored = stored.clone();
        unencrypt.connect_clicked(move |_| {
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            let msg = match crate::security::unencrypt_config(&binary, &stored.text()) {
                Ok(()) => {
                    let _ = crate::keyring::delete_password();
                    ctx.settings.borrow_mut().core.config_password.clear();
                    ctx.persist();
                    ctx.restart_engine();
                    "rclone.conf encryption removed".into()
                }
                Err(e) => e,
            };
            let alert = adw::AlertDialog::new(Some("Unencrypt"), Some(&msg));
            alert.add_response("ok", "OK");
            alert.present(Some(&parent));
        });
    }
    let sec_row = adw::ActionRow::new();
    sec_row.set_title("Actions");
    sec_row.add_suffix(&validate);
    sec_row.add_suffix(&encrypt);
    sec_row.add_suffix(&change);
    sec_row.add_suffix(&unencrypt);
    s1.add(&sec_row);
    security.add(&s1);

    dialog.add(&general);
    dialog.add(&core);
    dialog.add(&security);
    dialog.add(&dev);
    dialog.present(Some(parent));
}

fn switch_row(title: &str, active: bool, on_change: impl Fn(bool) + 'static) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_active(active);
    row.connect_active_notify(move |row| on_change(row.is_active()));
    row
}

pub fn about(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let version = ctx
        .engine
        .borrow()
        .as_ref()
        .map(|e| e.version.clone())
        .unwrap_or_default();
    let app_update = crate::updater::fetch_app_update(env!("CARGO_PKG_VERSION"))
        .ok()
        .filter(|u| u.available)
        .map(|u| format!("App update {} available", u.latest))
        .unwrap_or_else(|| "App is up to date (or update check failed)".into());
    let rclone_update = crate::updater::fetch_rclone_update(&version)
        .ok()
        .filter(|u| u.available)
        .map(|u| format!("rclone update {} available", u.latest))
        .unwrap_or_else(|| "rclone update check finished".into());
    let dialog = adw::AboutDialog::builder()
        .application_name("Rclone Manager")
        .application_icon("folder-remote-symbolic")
        .developer_name("Zarestia Dev")
        .version(env!("CARGO_PKG_VERSION"))
        .website("https://github.com/Zarestia-Dev/rclone-manager")
        .issue_url("https://github.com/Zarestia-Dev/rclone-manager/issues")
        .license_type(gtk::License::Gpl30)
        .comments(format!(
            "GTK 4 + libadwaita desktop client\nrclone {version}\n{app_update}\n{rclone_update}"
        ))
        .build();
    dialog.present(Some(parent));
}

pub fn shortcuts(parent: &impl IsA<gtk::Widget>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Keyboard Shortcuts");
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (keys, desc) in [
        ("Ctrl+Q", "Quit"),
        ("Ctrl+B", "File browser"),
        ("Ctrl+N", "Detailed remote"),
        ("Ctrl+R", "Quick add remote"),
        ("Ctrl+I", "Import settings"),
        ("Ctrl+E", "Export settings"),
        ("Ctrl+,", "Preferences"),
        ("Ctrl+.", "Rclone flags"),
        ("Ctrl+Alt+A", "Alerts"),
        ("Ctrl+Alt+F", "Flow"),
        ("Ctrl+Shift+?", "Shortcuts"),
        ("Ctrl+Shift+M", "Refresh mounts"),
        ("Ctrl+Shift+S", "Refresh serves"),
        ("Ctrl+T / Ctrl+W", "New / close file tab"),
        ("Ctrl+Shift+D", "Detach current file tab"),
        ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo file action"),
        ("Ctrl+/", "Toggle split view"),
        ("Ctrl+L / Ctrl+F", "Focus path / search (Files)"),
        ("F5", "Reload listing"),
        ("F2", "Rename"),
        ("Delete", "Delete"),
        ("Ctrl+C / X / V", "Copy / Cut / Paste"),
        ("Ctrl+Shift+N", "New folder"),
        ("Ctrl+H", "Toggle hidden files"),
        ("Backspace", "Parent folder"),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(desc);
        row.set_subtitle(keys);
        list.append(&row);
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    dialog.set_child(Some(&scroll));
    dialog.present(Some(parent));
}

pub fn logs(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: Option<String>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Logs");
    dialog.set_content_width(720);
    dialog.set_content_height(480);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_monospace(true);
    let key = remote.clone().unwrap_or_else(|| "_engine".into());
    let mut lines = ctx
        .store
        .borrow()
        .logs
        .get(&key)
        .cloned()
        .unwrap_or_default();
    if let Ok(file) = std::fs::read_to_string(crate::settings::AppSettings::log_path()) {
        let tail: Vec<String> = file
            .lines()
            .rev()
            .take(400)
            .map(|s| s.to_string())
            .collect();
        let mut chron = tail;
        chron.reverse();
        if !chron.is_empty() {
            lines.extend(chron);
        }
    }
    let search = gtk::Entry::new();
    search.set_placeholder_text(Some("Filter logs"));
    let apply = {
        let view = view.clone();
        let lines = lines.clone();
        move |query: &str| {
            let q = query.to_ascii_lowercase();
            let text = if q.is_empty() {
                if lines.is_empty() {
                    "No logs yet.".into()
                } else {
                    lines.join("\n")
                }
            } else {
                lines
                    .iter()
                    .filter(|line| line.to_ascii_lowercase().contains(&q))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            view.buffer().set_text(&text);
        }
    };
    apply("");
    {
        let apply = apply.clone();
        search.connect_changed(move |entry| apply(&entry.text()));
    }
    let clear = gtk::Button::with_label("Clear");
    {
        let ctx = ctx.clone();
        let key = key.clone();
        let view = view.clone();
        clear.connect_clicked(move |_| {
            ctx.store.borrow_mut().logs.remove(&key);
            ctx.persist();
            view.buffer().set_text("No logs yet.");
        });
    }
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.set_margin_start(8);
    toolbar.set_margin_end(8);
    toolbar.set_margin_top(8);
    search.set_hexpand(true);
    toolbar.append(&search);
    toolbar.append(&clear);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&view));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&toolbar);
    box_.append(&scroll);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn rclone_flags(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Rclone Flags");
    dialog.set_search_enabled(true);
    let Some(client) = ctx.client() else {
        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        let row = adw::ActionRow::new();
        row.set_title("Engine offline");
        group.add(&row);
        page.add(&group);
        dialog.add(&page);
        dialog.present(Some(parent));
        return;
    };
    let info = client.options_info().unwrap_or(serde_json::json!({}));
    let current = client.options_get().unwrap_or(serde_json::json!({}));
    let mut blocks = crate::flags::parse_options_info(&info);
    crate::flags::merge_current_values(&mut blocks, &current);
    let edits: Rc<RefCell<Vec<(String, String, serde_json::Value)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for category in ["backend", "filter", "vfs", "mount", "copy", "sync", "check"] {
        let page = adw::PreferencesPage::new();
        page.set_title(&category.to_ascii_uppercase());
        let group = adw::PreferencesGroup::new();
        group.set_title(category);
        let options = crate::flags::options_for_category(&blocks, category);
        if options.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("No flags in this category");
            group.add(&row);
        }
        for (block, option) in options.into_iter().take(40) {
            let current_text = match &option.value {
                serde_json::Value::Null => option.default_str.clone(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string().trim_matches('"').to_string(),
            };
            if option.type_name == "bool" {
                let row = adw::SwitchRow::new();
                row.set_title(&option.name);
                row.set_subtitle(&option.help);
                row.set_active(current_text.eq_ignore_ascii_case("true"));
                let edits = edits.clone();
                let block = block.to_string();
                let field = option.field_name.clone();
                row.connect_active_notify(move |row| {
                    edits.borrow_mut().push((
                        block.clone(),
                        field.clone(),
                        serde_json::json!(row.is_active()),
                    ));
                });
                group.add(&row);
            } else {
                let row = adw::EntryRow::new();
                row.set_title(&option.name);
                row.set_text(&current_text);
                if !option.help.is_empty() {
                    row.set_tooltip_text(Some(&option.help));
                }
                let edits = edits.clone();
                let block = block.to_string();
                let field = option.field_name.clone();
                let type_name = option.type_name.clone();
                row.connect_changed(move |row| {
                    edits.borrow_mut().push((
                        block.clone(),
                        field.clone(),
                        crate::flags::parse_flag_value(&type_name, &row.text()),
                    ));
                });
                group.add(&row);
            }
        }
        page.add(&group);
        dialog.add(&page);
    }
    let apply = gtk::Button::with_label("Apply changes");
    apply.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let edits = edits.clone();
        apply.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let payload = crate::flags::collect_edits(&edits.borrow());
                match client.options_set(payload) {
                    Ok(_) => log::info!("rclone flags applied"),
                    Err(e) => log::warn!("failed to apply flags: {e}"),
                }
            }
        });
    }
    let extra = adw::PreferencesPage::new();
    extra.set_title("Apply");
    let g = adw::PreferencesGroup::new();
    g.set_description(Some(
        "Edits are sent to rclone via options/set. Categories match the Angular flag panels.",
    ));
    let row = adw::ActionRow::new();
    row.set_title("Write flags to the running engine");
    row.add_suffix(&apply);
    g.add(&row);
    extra.add(&g);
    dialog.add(&extra);

    let json_page = adw::PreferencesPage::new();
    json_page.set_title("JSON");
    let json_group = adw::PreferencesGroup::new();
    json_group.set_title("Raw options/set payload");
    json_group.set_description(Some(
        "Edit the current rclone options as JSON. Apply writes the object via options/set.",
    ));
    let json_toggle = adw::SwitchRow::new();
    json_toggle.set_title("Remember JSON mode");
    json_toggle.set_active(ctx.settings.borrow().runtime.show_json_mode);
    {
        let ctx = ctx.clone();
        json_toggle.connect_active_notify(move |row| {
            ctx.settings.borrow_mut().runtime.show_json_mode = row.is_active();
            ctx.persist();
        });
    }
    json_group.add(&json_toggle);
    let json_view = gtk::TextView::new();
    json_view.set_monospace(true);
    json_view.set_wrap_mode(gtk::WrapMode::WordChar);
    let pretty = serde_json::to_string_pretty(&current).unwrap_or_else(|_| "{}".into());
    json_view.buffer().set_text(&pretty);
    let json_scroll = gtk::ScrolledWindow::new();
    json_scroll.set_min_content_height(280);
    json_scroll.set_child(Some(&json_view));
    let json_apply = gtk::Button::with_label("Apply JSON");
    json_apply.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let json_view = json_view.clone();
        json_apply.connect_clicked(move |_| {
            let buffer = json_view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            match crate::flags::parse_json_object(&text) {
                Ok(map) => {
                    if let Some(client) = ctx.client() {
                        match client.options_set(serde_json::Value::Object(map)) {
                            Ok(_) => log::info!("rclone flags applied from JSON"),
                            Err(e) => log::warn!("failed to apply JSON flags: {e}"),
                        }
                    }
                }
                Err(e) => log::warn!("invalid flags JSON: {e}"),
            }
        });
    }
    let json_row = adw::ActionRow::new();
    json_row.set_title("Write JSON to the running engine");
    json_row.add_suffix(&json_apply);
    json_group.add(&json_row);
    json_page.add(&json_group);
    let json_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    json_box.set_margin_start(12);
    json_box.set_margin_end(12);
    json_box.append(&json_scroll);
    let json_holder = adw::PreferencesGroup::new();
    let holder_row = adw::ActionRow::new();
    holder_row.set_title("Document");
    holder_row.set_activatable(false);
    json_holder.add(&holder_row);
    json_page.add(&json_holder);
    dialog.add(&json_page);
    // Attach the editor below the last page content via a toast-less overlay:
    // PreferencesDialog pages can't host arbitrary boxes easily, so present the
    // JSON editor as the ActionRow child suffix expansion.
    holder_row.set_child(Some(&json_box));
    dialog.present(Some(parent));
}

pub fn action_order(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    catalog: &[&str],
    current: &[String],
    on_save: impl Fn(Vec<String>) + 'static,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(480);
    dialog.set_content_height(560);
    let items = Rc::new(RefCell::new(crate::action_order::build_items(
        current, catalog,
    )));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let rebuild: Rc<RefCell<Box<dyn Fn()>>> = Rc::new(RefCell::new(Box::new(|| {})));
    let rebuild_fn = {
        let list = list.clone();
        let items = items.clone();
        let rebuild = rebuild.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let snapshot = items.borrow().clone();
            for (idx, item) in snapshot.iter().enumerate() {
                let row = adw::ActionRow::new();
                row.set_title(&item.id);
                row.set_subtitle(if item.visible { "Visible" } else { "Hidden" });
                let visible = gtk::Switch::new();
                visible.set_active(item.visible);
                visible.set_valign(gtk::Align::Center);
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    let id = item.id.clone();
                    visible.connect_active_notify(move |sw| {
                        crate::action_order::apply_visibility(
                            &mut items.borrow_mut(),
                            &id,
                            sw.is_active(),
                        );
                        rebuild.borrow()();
                    });
                }
                let up = gtk::Button::from_icon_name("go-up-symbolic");
                up.set_valign(gtk::Align::Center);
                up.set_tooltip_text(Some("Move up"));
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    up.connect_clicked(move |_| {
                        crate::action_order::move_item(&mut items.borrow_mut(), idx, -1);
                        rebuild.borrow()();
                    });
                }
                let down = gtk::Button::from_icon_name("go-down-symbolic");
                down.set_valign(gtk::Align::Center);
                down.set_tooltip_text(Some("Move down"));
                {
                    let items = items.clone();
                    let rebuild = rebuild.clone();
                    down.connect_clicked(move |_| {
                        crate::action_order::move_item(&mut items.borrow_mut(), idx, 1);
                        rebuild.borrow()();
                    });
                }
                row.add_suffix(&up);
                row.add_suffix(&down);
                row.add_suffix(&visible);
                list.append(&row);
            }
        })
    };
    *rebuild.borrow_mut() = Box::new({
        let rebuild_fn = rebuild_fn.clone();
        move || rebuild_fn()
    });
    rebuild_fn();

    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        let items = items.clone();
        save.connect_clicked(move |_| {
            on_save(crate::action_order::visible_ids(&items.borrow()));
            dialog.close();
        });
    }
    let cancel = gtk::Button::with_label("Cancel");
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    let reset = gtk::Button::with_label("Reset");
    {
        let items = items.clone();
        let rebuild = rebuild.clone();
        let catalog: Vec<String> = catalog.iter().map(|s| (*s).to_string()).collect();
        reset.connect_clicked(move |_| {
            let refs: Vec<&str> = catalog.iter().map(|s| s.as_str()).collect();
            *items.borrow_mut() = crate::action_order::build_items(&[], &refs);
            rebuild.borrow()();
        });
    }
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_margin_top(8);
    bar.append(&reset);
    bar.append(&cancel);
    bar.append(&save);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&list));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.set_margin_start(12);
    box_.set_margin_end(12);
    let hint = gtk::Label::new(Some(
        "Toggle visibility and reorder. Hidden actions stay available in Configure.",
    ));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    box_.append(&hint);
    box_.append(&scroll);
    box_.append(&bar);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn backends(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Backends");
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let ready = ctx.engine_ready();
    let port = ctx.engine.borrow().as_ref().map(|e| e.port).unwrap_or(0);
    let active = ctx.settings.borrow().core.active_backend.clone();
    let local = adw::ActionRow::new();
    local.set_title("Local rclone RC");
    let local_id = ctx
        .client()
        .filter(|_| active.is_empty() || active == "local")
        .and_then(|c| c.version_info().ok())
        .map(|v| crate::rclone::backend_identity(&v).summary())
        .unwrap_or_default();
    local.set_subtitle(&format!(
        "127.0.0.1:{port} · ready={ready}{}{}",
        if active.is_empty() || active == "local" {
            " · active"
        } else {
            ""
        },
        if local_id.is_empty() {
            String::new()
        } else {
            format!(" · {local_id}")
        }
    ));
    let use_local = gtk::Button::with_label("Use");
    use_local.set_valign(gtk::Align::Center);
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        use_local.connect_clicked(move |_| {
            ctx.switch_backend("local");
            backends(&parent, ctx.clone());
        });
    }
    local.add_suffix(&use_local);
    list.append(&local);
    for backend in ctx.settings.borrow().core.extra_backends.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&backend.name);
        let marker = if active == backend.name {
            " · active"
        } else {
            ""
        };
        let identity = {
            let user = if backend.user.is_empty() {
                None
            } else {
                Some(backend.user.clone())
            };
            let pass = if backend.pass.is_empty() {
                None
            } else {
                Some(backend.pass.clone())
            };
            let client =
                crate::rclone::RcClient::new(&backend.host, backend.port).with_auth(user, pass);
            client
                .version_info()
                .ok()
                .map(|v| crate::rclone::backend_identity(&v).summary())
        };
        row.set_subtitle(&format!(
            "{}:{}{marker}{}",
            backend.host,
            backend.port,
            identity
                .as_deref()
                .map(|s| format!(" · {s}"))
                .unwrap_or_default()
        ));
        let test = gtk::Button::from_icon_name("network-transmit-receive-symbolic");
        test.set_valign(gtk::Align::Center);
        test.set_tooltip_text(Some("Test connection"));
        let use_btn = gtk::Button::with_label("Use");
        use_btn.set_valign(gtk::Align::Center);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_valign(gtk::Align::Center);
        {
            let backend = backend.clone();
            let parent = parent.clone();
            test.connect_clicked(move |_| {
                let user = if backend.user.is_empty() {
                    None
                } else {
                    Some(backend.user.clone())
                };
                let pass = if backend.pass.is_empty() {
                    None
                } else {
                    Some(backend.pass.clone())
                };
                let client =
                    crate::rclone::RcClient::new(&backend.host, backend.port).with_auth(user, pass);
                let msg = if client.ping() {
                    format!(
                        "Connected — {}",
                        client.version().unwrap_or_else(|_| "unknown".into())
                    )
                } else {
                    format!("Unreachable at {}:{}", backend.host, backend.port)
                };
                let alert = adw::AlertDialog::new(Some("Backend test"), Some(&msg));
                alert.add_response("ok", "OK");
                alert.present(Some(&parent));
            });
        }
        {
            let ctx = ctx.clone();
            let name = backend.name.clone();
            let parent = parent.clone();
            use_btn.connect_clicked(move |_| {
                ctx.switch_backend(&name);
                backends(&parent, ctx.clone());
            });
        }
        {
            let ctx = ctx.clone();
            let name = backend.name.clone();
            let parent = parent.clone();
            remove.connect_clicked(move |_| {
                ctx.settings
                    .borrow_mut()
                    .core
                    .extra_backends
                    .retain(|b| b.name != name);
                if ctx.settings.borrow().core.active_backend == name {
                    ctx.settings.borrow_mut().core.active_backend.clear();
                }
                ctx.persist();
                backends(&parent, ctx.clone());
            });
        }
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.set_valign(gtk::Align::Center);
        edit.set_tooltip_text(Some("Edit backend"));
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let backend = backend.clone();
            edit.connect_clicked(move |_| {
                backend_editor(&parent, ctx.clone(), Some(backend.clone()));
            });
        }
        let clone = gtk::Button::with_label("Clone");
        clone.set_valign(gtk::Align::Center);
        clone.set_tooltip_text(Some("Duplicate connection settings into a new backend"));
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let mut entry = backend.clone();
            entry.name = format!("{}-copy", backend.name);
            clone.connect_clicked(move |_| {
                backend_editor(&parent, ctx.clone(), Some(entry.clone()));
            });
        }
        let copy = gtk::Button::with_label("Copy remotes");
        copy.set_valign(gtk::Align::Center);
        copy.set_tooltip_text(Some("Copy remotes from the active backend to this one"));
        {
            let backend = backend.clone();
            let ctx = ctx.clone();
            let parent = parent.clone();
            copy.connect_clicked(move |_| {
                let Some(source) = ctx.client() else {
                    return;
                };
                let dest = crate::rclone::RcClient::new(&backend.host, backend.port).with_auth(
                    if backend.user.is_empty() {
                        None
                    } else {
                        Some(backend.user.clone())
                    },
                    if backend.pass.is_empty() {
                        None
                    } else {
                        Some(backend.pass.clone())
                    },
                );
                let msg = match dest.copy_remotes_from(&source) {
                    Ok(n) => format!("Copied {n} remotes"),
                    Err(e) => e.to_string(),
                };
                let alert = adw::AlertDialog::new(Some("Copy remotes"), Some(&msg));
                alert.add_response("ok", "OK");
                alert.present(Some(&parent));
            });
        }
        row.add_suffix(&test);
        row.add_suffix(&use_btn);
        row.add_suffix(&edit);
        row.add_suffix(&clone);
        row.add_suffix(&copy);
        row.add_suffix(&remove);
        list.append(&row);
    }
    let add = gtk::Button::with_label("Add remote RC backend");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add.connect_clicked(move |_| {
            backend_editor(&parent, ctx.clone(), None);
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&scrolled_list(&list));
    box_.append(&add);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn rc_client_for(ctx: &AppCtx, name: &str) -> Option<crate::rclone::RcClient> {
    if name.is_empty() || name.eq_ignore_ascii_case("local") {
        return ctx.client();
    }
    ctx.settings
        .borrow()
        .core
        .extra_backends
        .iter()
        .find(|b| b.name == name)
        .map(rc_client_for_entry)
}

fn rc_client_for_entry(entry: &crate::settings::BackendEntry) -> crate::rclone::RcClient {
    crate::rclone::RcClient::new(&entry.host, entry.port).with_auth(
        if entry.user.is_empty() {
            None
        } else {
            Some(entry.user.clone())
        },
        if entry.pass.is_empty() {
            None
        } else {
            Some(entry.pass.clone())
        },
    )
}

fn backend_editor(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<crate::settings::BackendEntry>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Remote RC backend");
    dialog.set_content_width(480);
    let name = adw::EntryRow::new();
    name.set_title("Name");
    let host = adw::EntryRow::new();
    host.set_title("Host");
    host.set_text("127.0.0.1");
    let port = adw::EntryRow::new();
    port.set_title("Port");
    port.set_text("5573");
    let user = adw::EntryRow::new();
    user.set_title("Username");
    let pass = adw::PasswordEntryRow::new();
    pass.set_title("Password");
    if let Some(entry) = &existing {
        name.set_text(&entry.name);
        host.set_text(&entry.host);
        port.set_text(&entry.port.to_string());
        user.set_text(&entry.user);
        pass.set_text(&entry.pass);
    }
    let mut copy_labels = vec![
        "Don't copy remotes".to_string(),
        "Local rclone RC".to_string(),
    ];
    let mut copy_ids = vec![String::new(), "local".to_string()];
    for backend in ctx.settings.borrow().core.extra_backends.clone() {
        if existing
            .as_ref()
            .map(|e| e.name == backend.name)
            .unwrap_or(false)
        {
            continue;
        }
        copy_labels.push(backend.name.clone());
        copy_ids.push(backend.name);
    }
    let copy_refs: Vec<&str> = copy_labels.iter().map(|s| s.as_str()).collect();
    let copy_from = gtk::DropDown::from_strings(&copy_refs);
    copy_from.set_selected(0);
    let copy_row = adw::ActionRow::new();
    copy_row.set_title("Copy remotes from");
    copy_row.set_subtitle("Optional: clone remotes from another RC backend after saving");
    copy_row.add_suffix(&copy_from);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let name = name.clone();
        let host = host.clone();
        let port = port.clone();
        let user = user.clone();
        let pass = pass.clone();
        let copy_from = copy_from.clone();
        save.connect_clicked(move |_| {
            let port_n = port.text().parse::<u16>().unwrap_or(5573);
            let entry = crate::settings::BackendEntry {
                name: name.text().to_string(),
                host: host.text().to_string(),
                port: port_n,
                user: user.text().to_string(),
                pass: pass.text().to_string(),
            };
            if entry.name.is_empty() || entry.host.is_empty() {
                return;
            }
            let source_idx = copy_from.selected() as usize;
            let source_id = copy_ids.get(source_idx).cloned().unwrap_or_default();
            let mut settings = ctx.settings.borrow_mut();
            if let Some(idx) = settings
                .core
                .extra_backends
                .iter()
                .position(|b| b.name == entry.name)
            {
                settings.core.extra_backends[idx] = entry.clone();
            } else {
                settings.core.extra_backends.push(entry.clone());
            }
            drop(settings);
            ctx.persist();
            if !source_id.is_empty() {
                if let (Some(source), dest) =
                    (rc_client_for(&ctx, &source_id), rc_client_for_entry(&entry))
                {
                    if let Err(e) = dest.copy_remotes_from(&source) {
                        let alert =
                            adw::AlertDialog::new(Some("Copy remotes"), Some(&e.to_string()));
                        alert.add_response("ok", "OK");
                        alert.present(Some(&parent));
                    }
                }
            }
            dialog.close();
            backends(&parent, ctx.clone());
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&host);
    group.add(&port);
    group.add(&user);
    group.add(&pass);
    group.add(&copy_row);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn alerts(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Alerts");
    dialog.set_content_width(720);
    dialog.set_content_height(560);
    let stack = adw::ViewStack::new();
    let history = gtk::ListBox::new();
    history.add_css_class("boxed-list");
    for event in ctx.store.borrow().alert_history.iter().take(50) {
        let row = adw::ActionRow::new();
        row.set_title(&event.title);
        row.set_subtitle(&format!(
            "{} · {} · {}",
            event.severity.as_str(),
            event.kind.as_str(),
            event.body
        ));
        history.append(&row);
    }
    if history.first_child().is_none() {
        let row = adw::ActionRow::new();
        row.set_title("No alert history");
        history.append(&row);
    }
    let ack = gtk::Button::with_label("Acknowledge all");
    {
        let ctx = ctx.clone();
        ack.connect_clicked(move |_| {
            ctx.store.borrow_mut().acknowledge_all();
            ctx.persist();
        });
    }
    let history_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    history_box.append(&scrolled_list(&history));
    history_box.append(&ack);
    stack.add_titled(&history_box, Some("history"), "History");

    let rules = gtk::ListBox::new();
    rules.add_css_class("boxed-list");
    for rule in ctx.store.borrow().alert_rules.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&rule.name);
        row.set_subtitle(&format!(
            "min {} · {} actions · {}",
            rule.severity_min.as_str(),
            rule.action_ids.len(),
            if rule.enabled { "on" } else { "off" }
        ));
        let enabled = gtk::Switch::new();
        enabled.set_valign(gtk::Align::Center);
        enabled.set_active(rule.enabled);
        {
            let ctx = ctx.clone();
            let id = rule.id.clone();
            enabled.connect_active_notify(move |sw| {
                if let Some(rule) = ctx
                    .store
                    .borrow_mut()
                    .alert_rules
                    .iter_mut()
                    .find(|r| r.id == id)
                {
                    if rule.enabled != sw.is_active() {
                        rule.enabled = sw.is_active();
                    }
                }
                ctx.persist();
            });
        }
        row.add_suffix(&enabled);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let id = rule.id.clone();
            row.connect_activated(move |_| {
                alert_rule_editor(&parent, ctx.clone(), Some(id.clone()));
            });
        }
        rules.append(&row);
    }
    let add_rule = gtk::Button::with_label("Add rule");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add_rule.connect_clicked(move |_| {
            alert_rule_editor(&parent, ctx.clone(), None);
        });
    }
    let rules_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    rules_box.append(&scrolled_list(&rules));
    rules_box.append(&add_rule);
    stack.add_titled(&rules_box, Some("rules"), "Rules");

    let actions = gtk::ListBox::new();
    actions.add_css_class("boxed-list");
    for action in ctx.store.borrow().alert_actions.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&action.name);
        row.set_subtitle(&action.kind);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let id = action.id.clone();
            row.connect_activated(move |_| {
                alert_action_editor(&parent, ctx.clone(), Some(id.clone()));
            });
        }
        actions.append(&row);
    }
    let add_action = gtk::Button::with_label("Add action");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add_action.connect_clicked(move |_| {
            alert_action_editor(&parent, ctx.clone(), None);
        });
    }
    let actions_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    actions_box.append(&scrolled_list(&actions));
    actions_box.append(&add_action);
    stack.add_titled(&actions_box, Some("actions"), "Actions");

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&stack));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&switcher);
    box_.append(&stack);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
    let _ = AlertSeverity::Info;
    let _ = AlertEvent::new(
        AlertEventKind::System,
        AlertSeverity::Info,
        String::new(),
        String::new(),
    );
}

pub fn quick_add_remote(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    super::wizard::present(parent, ctx, None, on_done);
}

pub fn remote_config(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    on_done: Rc<dyn Fn()>,
) {
    if let Some(name) = existing {
        super::remote_config::present(parent, ctx, name, on_done);
    } else {
        super::wizard::present(parent, ctx, None, on_done);
    }
}

fn remote_editor(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    quick: bool,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(if quick {
        "Quick Add Remote"
    } else {
        "Remote Configuration"
    });
    dialog.set_content_width(520);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::new();
    name.set_title("Remote name");
    if let Some(existing) = &existing {
        name.set_text(existing);
        name.set_sensitive(false);
    }
    let providers = ctx
        .client()
        .and_then(|c| c.providers().ok())
        .and_then(|v| {
            v.get("providers").and_then(|p| p.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.get("Name")
                            .or_else(|| item.get("Prefix"))
                            .or_else(|| item.get("name"))
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_else(|| {
            [
                "drive", "s3", "dropbox", "onedrive", "sftp", "ftp", "webdav", "local", "crypt",
                "alias", "union", "smb", "b2", "box", "mega", "pcloud", "seafile",
            ]
            .into_iter()
            .map(String::from)
            .collect()
        });
    let type_labels: Vec<&str> = providers.iter().map(|s| s.as_str()).collect();
    let type_row = adw::ComboRow::new();
    type_row.set_title("Provider type");
    type_row.set_model(Some(&gtk::StringList::new(&type_labels)));
    if let Some(idx) = providers.iter().position(|p| p == "drive") {
        type_row.set_selected(idx as u32);
    }
    let extra = adw::EntryRow::new();
    extra.set_title("Parameters (key=value;key=value)");
    group.add(&name);
    group.add(&type_row);
    group.add(&extra);
    page.add(&group);

    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let on_done = on_done.clone();
        save.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            let r#type = providers
                .get(type_row.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "drive".into());
            if remote_name.is_empty() || r#type.is_empty() {
                return;
            }
            let mut params = serde_json::Map::new();
            for pair in extra.text().split(';') {
                if let Some((k, v)) = pair.split_once('=') {
                    params.insert(k.trim().into(), serde_json::Value::String(v.trim().into()));
                }
            }
            if let Some(client) = ctx.client() {
                let result = if existing.is_some() {
                    client.update_remote(&remote_name, serde_json::Value::Object(params))
                } else {
                    client.create_remote(&remote_name, &r#type, serde_json::Value::Object(params))
                };
                match result {
                    Ok(_) => {
                        ctx.store
                            .borrow_mut()
                            .push_log(&remote_name, "Remote saved".into());
                        ctx.persist();
                        on_done();
                        dialog.close();
                    }
                    Err(e) => {
                        let toast = adw::AlertDialog::new(Some("Error"), Some(&e.to_string()));
                        toast.add_response("ok", "OK");
                        toast.present(Some(&dialog));
                    }
                }
            }
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.append(&page);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn start_operation(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    op: OperationType,
    toast: adw::ToastOverlay,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("{} — {remote}", op.api_label()));
    dialog.set_content_width(560);
    dialog.set_content_height(640);

    let profiles = ctx
        .store
        .borrow()
        .remotes
        .get(remote)
        .and_then(|m| m.profiles.get(op.as_str()).cloned())
        .unwrap_or_default();
    let mut names: Vec<String> = profiles.keys().cloned().collect();
    if names.is_empty() {
        names.push("default".into());
    }
    names.sort();
    let profile_row = adw::ComboRow::new();
    profile_row.set_title("Profile");
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    profile_row.set_model(Some(&gtk::StringList::new(&refs)));

    let initial = profiles
        .get(&names[0])
        .cloned()
        .unwrap_or_else(ProfileConfig::default);
    let rclone = flatten_rclone(&initial.rclone);
    let src = adw::EntryRow::new();
    src.set_title(if op == OperationType::Copyurl {
        "URL"
    } else {
        "Source"
    });
    src.set_text(&default_source(remote, &rclone));
    let src_kind = if op != OperationType::Copyurl {
        attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
        Some(attach_path_kind(&src, remote))
    } else {
        None
    };
    let extra_sources: Rc<RefCell<Vec<adw::EntryRow>>> = Rc::new(RefCell::new(Vec::new()));
    let more_src = crate::jobs::path_list(&rclone, crate::jobs::SOURCE_KEYS);
    if more_src.len() > 1 {
        for extra in more_src.iter().skip(1) {
            let row = adw::EntryRow::new();
            row.set_title("Additional source");
            row.set_text(extra);
            attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
            extra_sources.borrow_mut().push(row);
        }
    }
    let dst = adw::EntryRow::new();
    dst.set_title(match op {
        OperationType::Mount => "Mount point",
        OperationType::Serve => "Listen address",
        OperationType::Copyurl => "Destination fs",
        _ => "Destination",
    });
    dst.set_text(&if op == OperationType::Mount {
        crate::path_inspection::suggest_default_mount_path(remote, &ctx.store.borrow())
    } else {
        default_dest(remote, &rclone, op)
    });
    let dst_kind = if op == OperationType::Mount {
        attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::local_folders());
        None
    } else if op == OperationType::Serve {
        None
    } else {
        attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
        Some(attach_path_kind(&dst, remote))
    };
    let dest_status = gtk::Label::new(None);
    dest_status.add_css_class("dim-label");
    dest_status.set_xalign(0.0);
    dest_status.set_wrap(true);
    dest_status.set_visible(matches!(
        op,
        OperationType::Mount | OperationType::Sync | OperationType::Copy | OperationType::Bisync
    ));
    {
        let dest_status = dest_status.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let refresh_status = move |path: &str| {
            let resolved = crate::path_kind::resolve_job_path(path, &remote);
            let status = crate::path_inspection::inspect_dest(
                &ctx.store.borrow(),
                &resolved,
                &remote,
                op,
                &ctx.snapshot.borrow().mounts,
            );
            dest_status.set_text(&crate::path_inspection::describe_status(&status));
        };
        refresh_status(&dst.text());
        dst.connect_changed(move |row| refresh_status(&row.text()));
    }
    let serve = adw::ComboRow::new();
    serve.set_title("Serve type");
    serve.set_model(Some(&gtk::StringList::new(&OperationType::SERVE_TYPES)));
    serve.set_visible(op == OperationType::Serve);
    if let Some(t) = rclone.get("type").and_then(|x| x.as_str()) {
        if let Some(idx) = OperationType::SERVE_TYPES.iter().position(|s| *s == t) {
            serve.set_selected(idx as u32);
        }
    }
    let helper_names = |kind: &str| -> Vec<String> {
        let mut names = vec!["—".into()];
        if let Some(meta) = ctx.store.borrow().remotes.get(remote) {
            names.extend(meta.helper_names(kind));
        }
        names
    };
    let vfs_names = helper_names("vfs");
    let filter_names = helper_names("filter");
    let backend_names = helper_names("backend");
    let vfs_row = helper_combo("VFS profile", &vfs_names, &initial.app.vfs_profile);
    let filter_row = helper_combo("Filter profile", &filter_names, &initial.app.filter_profile);
    let backend_row = helper_combo(
        "Backend profile",
        &backend_names,
        &initial.app.backend_profile,
    );
    let dry = adw::SwitchRow::new();
    dry.set_title("Dry run");
    dry.set_active(crate::jobs::is_dry_run(&rclone));

    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title("Operation flags");
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for flag in crate::flags::static_flags_for(op) {
        if op == OperationType::Serve && flag.field_name == "type" {
            continue;
        }
        let row = adw::EntryRow::new();
        row.set_title(&flag.name);
        if !flag.help.is_empty() {
            row.set_tooltip_text(Some(&flag.help));
        }
        let current = rclone
            .get(&flag.field_name)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_else(|| flag.default_str.clone());
        row.set_text(&current);
        flags_group.add(&row);
        flag_rows
            .borrow_mut()
            .push((flag.field_name, row, flag.type_name));
    }
    let serve_flag_rows: Rc<RefCell<Vec<(String, String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    if op == OperationType::Serve {
        if let Some(client) = ctx.client() {
            if let Ok(info) = client.options_info() {
                let blocks = crate::flags::parse_options_info(&info);
                for serve_type in OperationType::SERVE_TYPES {
                    for flag in crate::flags::collect_serve_flags(&blocks, serve_type) {
                        let row = adw::EntryRow::new();
                        row.set_title(&format!("{serve_type} · {}", flag.name));
                        if !flag.help.is_empty() {
                            row.set_tooltip_text(Some(&flag.help));
                        }
                        let current = rclone
                            .get(&flag.field_name)
                            .map(|v| v.to_string().trim_matches('"').to_string())
                            .unwrap_or_else(|| flag.default_str.clone());
                        row.set_text(&current);
                        let selected = OperationType::SERVE_TYPES
                            .get(serve.selected() as usize)
                            .copied()
                            .unwrap_or("http");
                        row.set_visible(serve_type == selected);
                        flags_group.add(&row);
                        serve_flag_rows.borrow_mut().push((
                            serve_type.to_string(),
                            flag.field_name,
                            row,
                            flag.type_name,
                        ));
                    }
                }
            }
        }
        {
            let serve_flag_rows = serve_flag_rows.clone();
            serve.connect_selected_notify(move |row| {
                let selected = OperationType::SERVE_TYPES
                    .get(row.selected() as usize)
                    .copied()
                    .unwrap_or("http");
                for (serve_type, _, widget, _) in serve_flag_rows.borrow().iter() {
                    widget.set_visible(serve_type == selected);
                }
            });
        }
    }

    {
        let profiles = profiles.clone();
        let names = names.clone();
        let src = src.clone();
        let dst = dst.clone();
        let remote = remote.to_string();
        profile_row.connect_selected_notify(move |row| {
            let Some(name) = names.get(row.selected() as usize) else {
                return;
            };
            let Some(profile) = profiles.get(name) else {
                return;
            };
            let rclone = flatten_rclone(&profile.rclone);
            src.set_text(&default_source(&remote, &rclone));
            dst.set_text(&default_dest(&remote, &rclone, op));
        });
    }

    let start = gtk::Button::with_label("Start");
    start.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let dry = dry.clone();
        let flag_rows = flag_rows.clone();
        let serve_flag_rows = serve_flag_rows.clone();
        let extra_sources = extra_sources.clone();
        let vfs_row = vfs_row.clone();
        let filter_row = filter_row.clone();
        let backend_row = backend_row.clone();
        let vfs_names = vfs_names.clone();
        let filter_names = filter_names.clone();
        let backend_names = backend_names.clone();
        start.connect_clicked(move |_| {
            let Some(client) = ctx.client() else {
                toast.add_toast(adw::Toast::new("Engine offline"));
                return;
            };
            let mut rclone = serde_json::Map::new();
            if op == OperationType::Serve {
                rclone.insert(
                    "type".into(),
                    serde_json::json!(OperationType::SERVE_TYPES
                        .get(serve.selected() as usize)
                        .copied()
                        .unwrap_or("webdav")),
                );
            }
            if dry.is_active() {
                rclone.insert("dryRun".into(), serde_json::json!(true));
            }
            for (field, row, type_name) in flag_rows.borrow().iter() {
                let text = row.text().to_string();
                if !text.is_empty() {
                    rclone.insert(
                        field.clone(),
                        crate::flags::parse_flag_value(type_name, &text),
                    );
                }
            }
            let selected_serve = OperationType::SERVE_TYPES
                .get(serve.selected() as usize)
                .copied()
                .unwrap_or("webdav");
            for (serve_type, field, row, type_name) in serve_flag_rows.borrow().iter() {
                if serve_type != selected_serve {
                    continue;
                }
                let text = row.text().to_string();
                if !text.is_empty() {
                    rclone.insert(
                        field.clone(),
                        crate::flags::parse_flag_value(type_name, &text),
                    );
                }
            }
            let mut sources = vec![src.text().to_string()];
            for row in extra_sources.borrow().iter() {
                let text = row.text().to_string();
                if !text.is_empty() {
                    sources.push(text);
                }
            }
            if op != OperationType::Copyurl {
                sources = sources
                    .into_iter()
                    .map(|s| crate::path_kind::resolve_job_path(&s, &remote))
                    .collect();
            }
            let dest = if op == OperationType::Serve {
                dst.text().to_string()
            } else {
                crate::path_kind::resolve_job_path(&dst.text(), &remote)
            };
            let mut rclone = serde_json::Value::Object(rclone);
            if let Some(meta) = ctx.store.borrow().remotes.get(&remote) {
                let mut profile = crate::store::ProfileConfig::default();
                profile.app.vfs_profile = helper_selected(&vfs_row, &vfs_names);
                profile.app.filter_profile = helper_selected(&filter_row, &filter_names);
                profile.app.backend_profile = helper_selected(&backend_row, &backend_names);
                crate::jobs::apply_helper_options(&mut rclone, &profile, Some(meta));
            }
            let mut ids = Vec::new();
            let mut error = None;
            for source in sources {
                match build_job_params(op, &remote, &source, &dest, &rclone) {
                    Ok(req) => match start_request(&client, &req) {
                        Ok(id) => ids.push(id),
                        Err(e) => {
                            error = Some(e.to_string());
                            break;
                        }
                    },
                    Err(e) => {
                        error = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = error {
                let err = adw::AlertDialog::new(Some("Start failed"), Some(&e));
                err.add_response("ok", "OK");
                err.present(Some(&dialog));
            } else {
                ctx.store
                    .borrow_mut()
                    .push_log(&remote, format!("started {op} {}", ids.join(", ")));
                ctx.refresh_runtime();
                toast.add_toast(adw::Toast::new(&format!("Started {op} {}", ids.join(", "))));
                on_done();
                dialog.close();
            }
        });
    }

    let page = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    identity.add(&profile_row);
    if let Some(kind) = &src_kind {
        identity.add(kind);
    }
    identity.add(&src);
    for row in extra_sources.borrow().iter() {
        identity.add(row);
    }
    if op.supports_multi_source() {
        let add_src = gtk::Button::with_label("Add source");
        {
            let extra_sources = extra_sources.clone();
            let identity = identity.clone();
            let ctx = ctx.clone();
            add_src.connect_clicked(move |_| {
                let row = adw::EntryRow::new();
                row.set_title("Additional source");
                attach_path_picker(&ctx, &row, crate::picker::FilePickerConfig::folders());
                identity.add(&row);
                extra_sources.borrow_mut().push(row);
            });
        }
        let add_row = adw::ActionRow::new();
        add_row.set_title("Multiple sources");
        add_row.add_suffix(&add_src);
        identity.add(&add_row);
    }
    if let Some(kind) = &dst_kind {
        identity.add(kind);
    }
    identity.add(&dst);
    identity.add(&{
        let row = adw::ActionRow::new();
        row.set_title("Path check");
        row.add_suffix(&dest_status);
        row
    });
    identity.add(&serve);
    identity.add(&vfs_row);
    identity.add(&filter_row);
    identity.add(&backend_row);
    identity.add(&dry);
    page.add(&identity);
    page.add(&flags_group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    box_.append(&scroll);
    box_.append(&start);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn delete_remote(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    name: &str,
    on_done: Rc<dyn Fn()>,
) {
    let plan = crate::store::plan_delete_remote(name, &ctx.store.borrow(), &ctx.snapshot.borrow());
    let dialog = adw::AlertDialog::new(Some("Delete remote"), Some(&plan.summary()));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let name = name.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            if let Some(client) = ctx.client() {
                for mount in &plan.mounts {
                    let _ = client.unmount(mount);
                }
                for serve in &plan.serves {
                    let _ = client.serve_stop(serve);
                }
                for job in &plan.jobs {
                    let _ = client.job_stop(*job);
                }
                let _ = client.delete_remote(&name);
            }
            ctx.store.borrow_mut().apply_delete_remote(&name);
            ctx.persist();
            ctx.refresh_runtime();
            on_done();
        }
    });
    dialog.present(Some(parent));
}

pub fn clone_remote(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    name: &str,
    on_done: Rc<dyn Fn()>,
) {
    let existing = ctx.store.borrow().remote_names();
    let new_name = crate::store::unique_remote_name(&existing, name);
    let Some(meta) = crate::store::clone_remote_meta(&ctx.store.borrow(), name, &new_name) else {
        let toast = adw::AlertDialog::new(
            Some("Clone failed"),
            Some("No saved settings were found for this remote."),
        );
        toast.add_response("ok", "OK");
        toast.present(Some(parent));
        return;
    };
    if let Some(client) = ctx.client() {
        if let Err(e) = client.clone_remote_config(name, &new_name) {
            let toast = adw::AlertDialog::new(Some("Clone failed"), Some(&e.to_string()));
            toast.add_response("ok", "OK");
            toast.present(Some(parent));
            return;
        }
    }
    {
        let mut store = ctx.store.borrow_mut();
        store.remotes.insert(new_name.clone(), meta);
        store.ensure_remote_order(&[new_name.clone()]);
    }
    ctx.persist();
    ctx.refresh_runtime();
    remote_config(parent, ctx, Some(new_name), on_done);
}

pub fn quick_run_editor(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<QuickRun>,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Quick Run");
    dialog.set_content_width(520);
    let group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::new();
    name.set_title("Name");
    let remote = adw::EntryRow::new();
    remote.set_title("Remote");
    let src = adw::EntryRow::new();
    src.set_title("Source");
    attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
    let dst = adw::EntryRow::new();
    dst.set_title("Destination / mount point");
    attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
    let cron = adw::EntryRow::new();
    cron.set_title("Cron expression");
    let cron_hint = gtk::Label::new(None);
    cron_hint.add_css_class("dim-label");
    cron_hint.set_xalign(0.0);
    cron_hint.set_wrap(true);
    {
        let cron_hint = cron_hint.clone();
        cron.connect_changed(move |row| {
            let text = row.text().to_string();
            if text.is_empty() {
                cron_hint.set_text("");
            } else if let Err(e) = validate_cron(&text) {
                cron_hint.set_text(&e);
            } else {
                cron_hint.set_text(&describe_cron(&text));
            }
        });
    }
    let op_row = adw::ComboRow::new();
    op_row.set_title("Operation");
    let labels: Vec<&str> = OperationType::ALL.iter().map(|o| o.as_str()).collect();
    op_row.set_model(Some(&gtk::StringList::new(&labels)));
    let auto = adw::SwitchRow::new();
    auto.set_title("Auto start");
    let watch = adw::SwitchRow::new();
    watch.set_title("Watch enabled");
    let tray = adw::SwitchRow::new();
    tray.set_title("Show on tray");
    let vfs_profile = adw::EntryRow::new();
    vfs_profile.set_title("VFS profile name");
    let filter_profile = adw::EntryRow::new();
    filter_profile.set_title("Filter profile name");
    let backend_profile = adw::EntryRow::new();
    backend_profile.set_title("Backend profile name");
    if let Some(qr) = &existing {
        name.set_text(&qr.name);
        remote.set_text(&qr.remote_name);
        let (s, d) = qr.paths();
        src.set_text(&s.unwrap_or_default());
        dst.set_text(&d.unwrap_or_default());
        cron.set_text(&qr.config.app.cron_expression);
        auto.set_active(qr.config.app.auto_start);
        watch.set_active(qr.config.app.watch_enabled);
        tray.set_active(qr.show_on_tray);
        vfs_profile.set_text(&qr.config.app.vfs_profile);
        filter_profile.set_text(&qr.config.app.filter_profile);
        backend_profile.set_text(&qr.config.app.backend_profile);
        if let Some(idx) = OperationType::ALL
            .iter()
            .position(|o| *o == qr.operation_type)
        {
            op_row.set_selected(idx as u32);
        }
    }
    group.add(&name);
    group.add(&remote);
    group.add(&op_row);
    group.add(&src);
    group.add(&dst);
    group.add(&cron);
    group.add(&auto);
    group.add(&watch);
    group.add(&tray);
    group.add(&vfs_profile);
    group.add(&filter_profile);
    group.add(&backend_profile);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let existing_id = existing.as_ref().map(|q| q.id.clone());
        let vfs_profile = vfs_profile.clone();
        let filter_profile = filter_profile.clone();
        let backend_profile = backend_profile.clone();
        save.connect_clicked(move |_| {
            let expr = cron.text().to_string();
            if !expr.is_empty() {
                if let Err(e) = validate_cron(&expr) {
                    let err = adw::AlertDialog::new(Some("Invalid cron"), Some(&e));
                    err.add_response("ok", "OK");
                    err.present(Some(&dialog));
                    return;
                }
            }
            let op = OperationType::ALL
                .get(op_row.selected() as usize)
                .copied()
                .unwrap_or(OperationType::Sync);
            let mut qr = existing_id
                .as_ref()
                .and_then(|id| {
                    ctx.store
                        .borrow()
                        .quick_runs
                        .iter()
                        .find(|q| &q.id == id)
                        .cloned()
                })
                .unwrap_or_else(|| {
                    QuickRun::new(name.text().to_string(), op, remote.text().to_string())
                });
            qr.name = name.text().to_string();
            qr.remote_name = remote.text().to_string();
            qr.operation_type = op;
            qr.config.app.auto_start = auto.is_active();
            qr.config.app.watch_enabled = watch.is_active();
            qr.config.app.cron_enabled = !expr.is_empty();
            qr.config.app.cron_expression = expr;
            qr.config.app.vfs_profile = vfs_profile.text().to_string();
            qr.config.app.filter_profile = filter_profile.text().to_string();
            qr.config.app.backend_profile = backend_profile.text().to_string();
            qr.show_on_tray = tray.is_active();
            qr.config.rclone = serde_json::json!({
                "srcFs": src.text().to_string(),
                "dstFs": dst.text().to_string(),
                "mountPoint": dst.text().to_string(),
                "fs": src.text().to_string(),
            });
            {
                let mut store = ctx.store.borrow_mut();
                if let Some(idx) = store.quick_runs.iter().position(|q| q.id == qr.id) {
                    store.quick_runs[idx] = qr;
                } else {
                    store.quick_runs.push(qr);
                }
            }
            ctx.persist();
            on_done();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    box_.append(&group);
    box_.append(&cron_hint);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn export_backup(parent: &impl IsA<gtk::Window>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Export backup");
    dialog.set_content_width(480);
    let categories = backup::export_categories();
    let labels: Vec<&str> = categories.iter().map(|(_, label)| *label).collect();
    let type_row = adw::ComboRow::new();
    type_row.set_title("What to export");
    type_row.set_model(Some(&gtk::StringList::new(&labels)));
    let remotes: Vec<String> = ctx
        .snapshot
        .borrow()
        .remotes
        .iter()
        .map(|r| r.name.clone())
        .collect();
    let remote_row = adw::ComboRow::new();
    remote_row.set_title("Specific remote");
    let remote_refs: Vec<&str> = remotes.iter().map(|s| s.as_str()).collect();
    if remote_refs.is_empty() {
        remote_row.set_model(Some(&gtk::StringList::new(&["—"])));
        remote_row.set_sensitive(false);
    } else {
        remote_row.set_model(Some(&gtk::StringList::new(&remote_refs)));
    }
    let specific = adw::SwitchRow::new();
    specific.set_title("Export only one remote");
    let note = adw::EntryRow::new();
    note.set_title("Note");
    let password = adw::PasswordEntryRow::new();
    password.set_title("Zip password (optional, 4+ chars)");
    let secrets = adw::SwitchRow::new();
    secrets.set_title("Include secrets in rclone dump");
    secrets.set_active(true);
    let save = gtk::Button::with_label("Choose file…");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        let toast = toast.clone();
        let dialog = dialog.clone();
        let remotes = remotes.clone();
        let type_row = type_row.clone();
        let specific = specific.clone();
        let remote_row = remote_row.clone();
        let note = note.clone();
        let password = password.clone();
        let secrets = secrets.clone();
        save.connect_clicked(move |_| {
            let mut export_type = categories
                .get(type_row.selected() as usize)
                .map(|(id, _)| (*id).to_string())
                .unwrap_or_else(|| "FullBackup".into());
            if specific.is_active() {
                if let Some(name) = remotes.get(remote_row.selected() as usize) {
                    export_type = format!("remote:{name}");
                }
            }
            let note_text = note.text().to_string();
            let zip_pass = password.text().to_string();
            let include_secrets = secrets.is_active();
            let file_dialog = gtk::FileDialog::new();
            file_dialog.set_initial_name(Some("rclone-manager-backup.zip"));
            let ctx = ctx.clone();
            let toast = toast.clone();
            file_dialog.save(
                Some(&parent),
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let mut dump = ctx
                                .client()
                                .and_then(|c| c.dump_config().ok())
                                .unwrap_or(serde_json::json!({}));
                            if !include_secrets {
                                if let Some(obj) = dump.as_object_mut() {
                                    for cfg in obj.values_mut() {
                                        if let Some(map) = cfg.as_object_mut() {
                                            for key in [
                                                "token",
                                                "secret",
                                                "password",
                                                "pass",
                                                "client_secret",
                                                "key",
                                            ] {
                                                map.remove(key);
                                            }
                                        }
                                    }
                                }
                            }
                            let pw = if zip_pass.trim().len() >= 4 {
                                Some(zip_pass.as_str())
                            } else {
                                None
                            };
                            match backup::create_backup(
                                &path,
                                &ctx.settings.borrow(),
                                &ctx.store.borrow(),
                                &dump,
                                &export_type,
                                &note_text,
                                pw,
                            ) {
                                Ok(_) => toast.add_toast(adw::Toast::new("Backup exported")),
                                Err(e) => toast.add_toast(adw::Toast::new(&e)),
                            }
                        }
                    }
                },
            );
            dialog.close();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&type_row);
    group.add(&specific);
    group.add(&remote_row);
    group.add(&note);
    group.add(&password);
    group.add(&secrets);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent.upcast_ref(), &ctx, &dialog);
}

pub fn import_backup(
    parent: &(impl IsA<gtk::Window> + Clone),
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = gtk::FileDialog::new();
    let parent_widget = parent.clone();
    dialog.open(
        Some(parent),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    restore_preview(&parent_widget, ctx.clone(), toast.clone(), path, on_done);
                }
            }
        },
    );
}

pub fn restore_preview(
    parent: &(impl IsA<gtk::Window> + Clone),
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    path: std::path::PathBuf,
    on_done: Rc<dyn Fn()>,
) {
    let analysis = backup::analyze_backup(&path).ok();
    let summary = analysis
        .as_ref()
        .map(|a| {
            format!(
                "Version {} · {} · remotes: {}\nsettings={} store={} rclone={} encrypted={}",
                a.manifest.version,
                a.manifest.export_type,
                a.manifest.remotes.join(", "),
                a.has_settings,
                a.has_store,
                a.has_rclone_config,
                a.manifest.encrypted
            )
        })
        .unwrap_or_else(|| {
            "Could not read the zip without a password. Enter it below to restore.".into()
        });
    let dialog = adw::AlertDialog::new(Some("Restore backup"), Some(&summary));
    let password = gtk::PasswordEntry::new();
    password.set_show_peek_icon(true);
    password.set_placeholder_text(Some("Zip password (if encrypted)"));
    dialog.set_extra_child(Some(&password));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("restore", "Restore");
    dialog.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, response| {
        if response != "restore" {
            return;
        }
        let pw = password.text().to_string();
        let pw = if pw.is_empty() {
            None
        } else {
            Some(pw.as_str())
        };
        match backup::restore_backup_with_password(&path, pw) {
            Ok((settings, store, rclone)) => {
                if let Some(settings) = settings {
                    *ctx.settings.borrow_mut() = settings;
                }
                if let Some(store) = store {
                    *ctx.store.borrow_mut() = store;
                }
                if let (Some(dump), Some(client)) = (rclone, ctx.client()) {
                    if let Some(obj) = dump.as_object() {
                        for (name, cfg) in obj {
                            if let Some(t) = cfg.get("type").and_then(|x| x.as_str()) {
                                let _ = client.create_remote(name, t, cfg.clone());
                            }
                        }
                    }
                }
                ctx.persist();
                toast.add_toast(adw::Toast::new("Backup restored"));
                on_done();
            }
            Err(e) => toast.add_toast(adw::Toast::new(&e)),
        }
    });
    let parent = parent.clone().upcast::<gtk::Window>();
    dialog.present(Some(&parent));
}

pub fn properties(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Properties");
    dialog.set_content_width(520);
    dialog.set_content_height(640);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (title, value) in [
        ("Name", name.to_string()),
        ("Remote", remote.to_string()),
        ("Path", path.to_string()),
        (
            "Type",
            format!(
                "{:?}",
                crate::operations::FileTypeCategory::from_name(name, false)
            ),
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_subtitle(&value);
        list.append(&row);
    }
    let fs = if remote == "local" {
        "/".into()
    } else {
        remote_fs(remote, "")
    };
    let info = ctx.fs_info(remote);
    if let Some(client) = ctx.client() {
        if let Ok(about) = client.about(&fs) {
            let used = about.get("used").and_then(|x| x.as_i64()).unwrap_or(-1);
            let total = about.get("total").and_then(|x| x.as_i64()).unwrap_or(-1);
            let free = about.get("free").and_then(|x| x.as_i64()).unwrap_or(-1);
            let row = adw::ActionRow::new();
            row.set_title("Disk usage");
            row.set_subtitle(&format!(
                "used {} · free {} · total {}",
                crate::rclone::format_bytes(used),
                crate::rclone::format_bytes(free),
                crate::rclone::format_bytes(total)
            ));
            list.append(&row);
        }
        if let Ok(size) = client.size(&fs, path) {
            let row = adw::ActionRow::new();
            row.set_title("Size");
            let count = size.get("count").and_then(|x| x.as_i64());
            let bytes = size.get("bytes").and_then(|x| x.as_i64()).unwrap_or(-1);
            row.set_subtitle(&match count {
                Some(n) => format!("{} · {n} objects", crate::rclone::format_bytes(bytes)),
                None => crate::rclone::format_bytes(bytes),
            });
            list.append(&row);
        }
        let hashes = info
            .as_ref()
            .map(|i| i.hashes.clone())
            .filter(|h| !h.is_empty())
            .unwrap_or_default();
        if hashes.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("Hashes");
            row.set_subtitle("This remote does not advertise hash support");
            list.append(&row);
        } else {
            for (idx, hash_type) in hashes.iter().enumerate() {
                let row = adw::ActionRow::new();
                row.set_title(&hash_type.to_ascii_uppercase());
                row.set_subtitle("Not calculated");
                let calc = gtk::Button::with_label("Calculate");
                calc.set_valign(gtk::Align::Center);
                {
                    let row = row.clone();
                    let calc_btn = calc.clone();
                    let ctx = ctx.clone();
                    let fs = fs.clone();
                    let path = path.to_string();
                    let hash_type = hash_type.clone();
                    calc.connect_clicked(move |_| {
                        if let Some(client) = ctx.client() {
                            match client.hashsum(&fs, &path, &hash_type) {
                                Ok(value) => {
                                    row.set_subtitle(
                                        &parse_hashsum(&value).unwrap_or_else(|| value.to_string()),
                                    );
                                    calc_btn.set_sensitive(false);
                                }
                                Err(e) => row.set_subtitle(&e.to_string()),
                            }
                        }
                    });
                    if idx == 0 {
                        calc.emit_clicked();
                    }
                }
                row.add_suffix(&calc);
                list.append(&row);
            }
        }
        if remote != "local" && info.as_ref().is_none_or(|i| i.has_feature("PublicLink")) {
            let link_row = adw::ActionRow::new();
            link_row.set_title("Public link");
            link_row.set_subtitle("Not created");
            list.append(&link_row);
            let expire = adw::EntryRow::new();
            expire.set_title("Link expiry (e.g. 1d, 7d, 1M)");
            list.append(&expire);
            let get_link = gtk::Button::with_label("Get public link");
            let unlink = gtk::Button::with_label("Remove public link");
            {
                let ctx = ctx.clone();
                let fs = fs.clone();
                let path = path.to_string();
                let link_row = link_row.clone();
                let expire = expire.clone();
                get_link.connect_clicked(move |_| {
                    if let Some(client) = ctx.client() {
                        let exp = expire.text().to_string();
                        match client.public_link_ex(
                            &fs,
                            &path,
                            (!exp.is_empty()).then_some(exp.as_str()),
                            false,
                        ) {
                            Ok(url) if !url.is_empty() => {
                                link_row.set_subtitle(&url);
                                if let Some(display) = gtk::gdk::Display::default() {
                                    display.clipboard().set_text(&url);
                                }
                            }
                            Ok(_) => link_row.set_subtitle("Remote did not return a public link"),
                            Err(e) => link_row.set_subtitle(&e.to_string()),
                        }
                    }
                });
            }
            {
                let ctx = ctx.clone();
                let fs = fs.clone();
                let path = path.to_string();
                let link_row = link_row.clone();
                unlink.connect_clicked(move |_| {
                    if let Some(client) = ctx.client() {
                        match client.public_link_ex(&fs, &path, None, true) {
                            Ok(_) => link_row.set_subtitle("Link removed"),
                            Err(e) => link_row.set_subtitle(&e.to_string()),
                        }
                    }
                });
            }
            let link_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            link_actions.append(&get_link);
            link_actions.append(&unlink);
            list.append(&{
                let row = adw::ActionRow::new();
                row.set_title("Link actions");
                row.add_suffix(&link_actions);
                row
            });
        }
    }
    let copy_path = gtk::Button::with_label("Copy path");
    {
        let text = if remote == "local" {
            path.to_string()
        } else {
            format!("{remote}:{path}")
        };
        copy_path.connect_clicked(move |_| {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
            }
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&list);
    box_.append(&copy_path);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&box_));
    dialog.set_child(Some(&scroll));
    dialog.present(Some(parent));
}

pub fn job_detail(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, job_id: u64) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("Job #{job_id}"));
    dialog.set_content_width(640);
    dialog.set_content_height(620);
    let meta = gtk::ListBox::new();
    meta.add_css_class("boxed-list");
    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    let transfers = gtk::ListBox::new();
    transfers.add_css_class("boxed-list");
    let completed = gtk::ListBox::new();
    completed.add_css_class("boxed-list");
    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some("Filter transfers"));
    let stop = gtk::Button::with_label("Stop job");
    stop.add_css_class("destructive-action");
    {
        let ctx = ctx.clone();
        stop.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.job_stop(job_id);
                ctx.refresh_runtime();
            }
        });
    }
    let reset = gtk::Button::with_label("Reset stats");
    {
        let ctx = ctx.clone();
        reset.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.reset_stats(Some(&format!("job/{job_id}")));
            }
        });
    }
    let delete = gtk::Button::with_label("Delete from history");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        delete.connect_clicked(move |_| {
            ctx.store.borrow_mut().dismiss_job(job_id);
            ctx.persist();
            ctx.refresh_runtime();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&progress);
    box_.append(&scrolled_list(&meta));
    box_.append(&filter);
    let xfer_label = gtk::Label::new(Some("Active transfers"));
    xfer_label.add_css_class("heading");
    xfer_label.set_xalign(0.0);
    box_.append(&xfer_label);
    box_.append(&scrolled_list(&transfers));
    let done_label = gtk::Label::new(Some("Completed transfers"));
    done_label.add_css_class("heading");
    done_label.set_xalign(0.0);
    box_.append(&done_label);
    box_.append(&scrolled_list(&completed));
    let open_src = gtk::Button::with_label("Open source in Files");
    let open_dst = gtk::Button::with_label("Open destination in Files");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        open_src.connect_clicked(move |_| {
            if let Some(job) = ctx.snapshot.borrow().jobs.iter().find(|j| j.id == job_id) {
                if let Some((remote, path)) = browse_target(&job.src) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                    return;
                }
            }
            if let Some(job) = ctx
                .store
                .borrow()
                .job_history
                .iter()
                .find(|j| j.id == job_id)
            {
                if let Some((remote, path)) = browse_target(&job.src) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                }
            }
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        open_dst.connect_clicked(move |_| {
            if let Some(job) = ctx.snapshot.borrow().jobs.iter().find(|j| j.id == job_id) {
                if let Some((remote, path)) = browse_target(&job.dst) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                    return;
                }
            }
            if let Some(job) = ctx
                .store
                .borrow()
                .job_history
                .iter()
                .find(|j| j.id == job_id)
            {
                if let Some((remote, path)) = browse_target(&job.dst) {
                    ctx.request_browse(&remote, &path);
                    dialog.close();
                }
            }
        });
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&stop);
    actions.append(&reset);
    actions.append(&delete);
    actions.append(&open_src);
    actions.append(&open_dst);
    box_.append(&actions);
    dialog.set_child(Some(&box_));

    let fill = {
        let ctx = ctx.clone();
        let meta = meta.clone();
        let transfers = transfers.clone();
        let completed = completed.clone();
        let progress = progress.clone();
        let filter = filter.clone();
        move || {
            while let Some(child) = meta.first_child() {
                meta.remove(&child);
            }
            while let Some(child) = transfers.first_child() {
                transfers.remove(&child);
            }
            while let Some(child) = completed.first_child() {
                completed.remove(&child);
            }
            let Some(client) = ctx.client() else {
                return;
            };
            let Ok(status) = client.job_status(job_id) else {
                return;
            };
            let group = status
                .get("group")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("job/{job_id}"));
            let stats = client.stats(Some(&group)).ok();
            let job = job_from_status(job_id, &status, stats.as_ref());
            progress.set_fraction(job.progress);
            progress.set_text(Some(&format!(
                "{} · {:.0}% · {:.1}s",
                job.status,
                job.progress * 100.0,
                job.duration
            )));
            let speed = job
                .stats
                .get("speed")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let bytes = job.stats.get("bytes").and_then(|x| x.as_i64()).unwrap_or(0);
            let eta = job
                .stats
                .get("eta")
                .cloned()
                .unwrap_or(serde_json::json!("—"));
            for (title, value) in [
                ("Operation", job.operation.clone()),
                ("Status", job.status.clone()),
                ("Remote", job.remote.clone()),
                ("Profile", job.profile.clone()),
                ("Source", job.src.clone()),
                ("Destination", job.dst.clone()),
                ("Group", job.group.clone()),
                ("Transferred", crate::rclone::format_bytes(bytes)),
                ("Speed", format!("{:.1} KiB/s", speed / 1024.0)),
                ("ETA", eta.to_string()),
                (
                    "Error",
                    job.error
                        .as_deref()
                        .map(|e| ctx.translate_error(e))
                        .unwrap_or_else(|| "—".into()),
                ),
            ] {
                let row = adw::ActionRow::new();
                row.set_title(title);
                row.set_subtitle(&value);
                meta.append(&row);
            }
            let query = filter.text().to_lowercase();
            append_transfer_rows(&transfers, job.transferring.as_array(), &query, true);
            append_transfer_rows(&completed, job.completed.as_array(), &query, false);
            let checks = job
                .stats
                .get("checks")
                .or_else(|| job.output.get("results"))
                .and_then(|v| v.as_array());
            if let Some(arr) = checks {
                if !arr.is_empty() {
                    let row = adw::ActionRow::new();
                    row.set_title(&format!("{} check results", arr.len()));
                    let preview = arr
                        .iter()
                        .filter_map(|item| item.get("name").and_then(|x| x.as_str()))
                        .take(3)
                        .collect::<Vec<_>>()
                        .join(", ");
                    row.set_subtitle(&preview);
                    completed.append(&row);
                }
            }
            if let Some(cc) = job.output.get("cryptcheck").and_then(|v| v.get("results")) {
                if let Some(first) = cc.as_array().and_then(|a| a.first()) {
                    let row = adw::ActionRow::new();
                    row.set_title("Cryptcheck");
                    let status = first.get("status").and_then(|x| x.as_str()).unwrap_or("OK");
                    row.set_subtitle(status);
                    completed.append(&row);
                }
            }
        }
    };
    fill();
    {
        let fill = fill.clone();
        filter.connect_search_changed(move |_| fill());
    }
    let alive = Rc::new(Cell::new(true));
    {
        let alive = alive.clone();
        let fill = fill.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            if !alive.get() {
                return glib::ControlFlow::Break;
            }
            fill();
            glib::ControlFlow::Continue
        });
    }
    dialog.connect_closed(move |_| {
        alive.set(false);
    });
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn append_transfer_rows(
    list: &gtk::ListBox,
    items: Option<&Vec<serde_json::Value>>,
    query: &str,
    active: bool,
) {
    let empty_title = if active {
        "No active transfers"
    } else {
        "No completed transfers"
    };
    let Some(arr) = items else {
        let row = adw::ActionRow::new();
        row.set_title(empty_title);
        list.append(&row);
        return;
    };
    let mut shown = 0;
    for item in arr {
        let name = item
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("transfer");
        if !query.is_empty() && !name.to_lowercase().contains(query) {
            continue;
        }
        let row = adw::ActionRow::new();
        row.set_title(name);
        let pct = item
            .get("percentage")
            .or_else(|| item.get("percentageComplete"))
            .cloned()
            .unwrap_or(serde_json::json!(0));
        let size = item.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
        row.set_subtitle(&format!("{pct}% · {}", crate::rclone::format_bytes(size)));
        list.append(&row);
        shown += 1;
    }
    if shown == 0 {
        let row = adw::ActionRow::new();
        row.set_title(empty_title);
        list.append(&row);
    }
}

pub fn file_viewer(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
    siblings: &[String],
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(name);
    dialog.set_content_width(720);
    dialog.set_content_height(520);
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    nav.set_margin_start(12);
    nav.set_margin_end(12);
    nav.set_margin_top(8);
    let index = siblings.iter().position(|n| n == name);
    let prev = gtk::Button::from_icon_name("go-previous-symbolic");
    prev.set_tooltip_text(Some("Previous file"));
    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some("Next file"));
    let pos = gtk::Label::new(Some(&match index {
        Some(i) if !siblings.is_empty() => format!("{} / {}", i + 1, siblings.len()),
        _ => name.to_string(),
    }));
    pos.set_hexpand(true);
    pos.set_xalign(0.5);
    prev.set_sensitive(index.is_some_and(|i| i > 0));
    next.set_sensitive(index.is_some_and(|i| i + 1 < siblings.len()));
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        let siblings = siblings.to_vec();
        let dialog = dialog.clone();
        prev.connect_clicked(move |_| {
            let Some(i) = siblings.iter().position(|n| n == name.as_str()) else {
                return;
            };
            if i == 0 {
                return;
            }
            let prev_name = siblings[i - 1].clone();
            let parent_path = crate::rclone::parent_remote_path(&path);
            let next_path = crate::rclone::join_remote_path(&parent_path, &prev_name);
            dialog.close();
            file_viewer(
                &parent,
                ctx.clone(),
                &remote,
                &next_path,
                &prev_name,
                &siblings,
            );
        });
    }
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        let siblings = siblings.to_vec();
        let dialog = dialog.clone();
        next.connect_clicked(move |_| {
            let Some(i) = siblings.iter().position(|n| n == name.as_str()) else {
                return;
            };
            if i + 1 >= siblings.len() {
                return;
            }
            let next_name = siblings[i + 1].clone();
            let parent_path = crate::rclone::parent_remote_path(&path);
            let next_path = crate::rclone::join_remote_path(&parent_path, &next_name);
            dialog.close();
            file_viewer(
                &parent,
                ctx.clone(),
                &remote,
                &next_path,
                &next_name,
                &siblings,
            );
        });
    }
    nav.append(&prev);
    nav.append(&pos);
    nav.append(&next);
    let category = crate::operations::FileTypeCategory::from_name(name, false);
    let info = gtk::Label::new(Some(&format!(
        "{remote}:{path}\nType: {:?}\nUse Download to open with a system app when needed.",
        category
    )));
    info.set_wrap(true);
    info.set_margin_top(16);
    info.set_margin_start(16);
    info.set_margin_end(16);
    let open = gtk::Button::with_label("Open native");
    {
        let path = if remote == "local" {
            path.to_string()
        } else {
            remote_fs(remote, path)
        };
        open.connect_clicked(move |_| {
            let _ = open::that(&path);
        });
    }
    let download = gtk::Button::with_label("Download…");
    {
        let parent = parent.clone();
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.to_string();
        let name = name.to_string();
        download.connect_clicked(move |_| {
            download_file(&parent, ctx.clone(), &remote, &path, &name);
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.append(&nav);
    box_.append(&info);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&open);
    actions.append(&download);
    box_.append(&actions);
    if matches!(category, crate::operations::FileTypeCategory::Archive) {
        let heading = gtk::Label::new(Some("Archive contents"));
        heading.add_css_class("heading");
        heading.set_xalign(0.0);
        box_.append(&heading);
        let archive_list = gtk::ListBox::new();
        archive_list.add_css_class("boxed-list");
        let src = if remote == "local" {
            path.to_string()
        } else {
            format!("{remote}:{path}")
        };
        match ctx.client().and_then(|c| c.archive_list(&src, true).ok()) {
            Some(items) if !items.is_empty() => {
                for item in items {
                    let row = adw::ActionRow::new();
                    row.set_title(&item.path);
                    row.set_subtitle(&item.subtitle());
                    row.add_prefix(&gtk::Image::from_icon_name(if item.is_dir {
                        "folder-symbolic"
                    } else {
                        "package-x-generic-symbolic"
                    }));
                    archive_list.append(&row);
                }
            }
            Some(_) => {
                let row = adw::ActionRow::new();
                row.set_title("Archive is empty");
                archive_list.append(&row);
            }
            None => {
                let row = adw::ActionRow::new();
                row.set_title("Unable to list archive contents");
                row.set_subtitle("rclone operations/archive list failed for this file.");
                archive_list.append(&row);
            }
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_min_content_height(220);
        scroll.set_child(Some(&archive_list));
        box_.append(&scroll);
    }
    if remote == "local" && matches!(category, crate::operations::FileTypeCategory::Image) {
        let picture = gtk::Picture::for_filename(path);
        picture.set_vexpand(true);
        box_.append(&picture);
    }
    if remote == "local"
        && matches!(
            category,
            crate::operations::FileTypeCategory::Video | crate::operations::FileTypeCategory::Audio
        )
    {
        let video = gtk::Video::for_filename(Some(path));
        video.set_vexpand(true);
        video.set_autoplay(true);
        box_.append(&video);
        if matches!(category, crate::operations::FileTypeCategory::Audio) {
            if let Some(cover) = crate::media::sibling_cover(std::path::Path::new(path)) {
                let picture = gtk::Picture::for_filename(&cover);
                picture.set_can_shrink(true);
                picture.set_content_fit(gtk::ContentFit::Contain);
                picture.set_size_request(-1, 180);
                box_.append(&picture);
            }
        }
    }
    if remote == "local" && matches!(category, crate::operations::FileTypeCategory::Pdf) {
        let note = gtk::Label::new(Some(
            "PDF preview uses the system viewer. Click Open native to display it.",
        ));
        note.set_wrap(true);
        box_.append(&note);
    }
    if remote == "local" && matches!(category, crate::operations::FileTypeCategory::Text) {
        let view = gtk::TextView::new();
        view.set_monospace(true);
        view.set_editable(true);
        if let Ok(text) = std::fs::read_to_string(path) {
            let shown = if text.len() > 200_000 {
                format!("{}\n\n… truncated …", &text[..200_000])
            } else {
                text
            };
            apply_syntax_highlight(&view, name, &shown);
        }
        attach_live_syntax(&view, name);
        let save = gtk::Button::with_label("Save");
        save.add_css_class("suggested-action");
        {
            let view = view.clone();
            let path = path.to_string();
            save.connect_clicked(move |_| {
                let buffer = view.buffer();
                let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                if let Err(e) = std::fs::write(&path, text.as_str()) {
                    log::warn!("failed to save {path}: {e}");
                }
            });
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&view));
        box_.append(&scroll);
        box_.append(&save);
    }
    if remote != "local" {
        if let Some(client) = ctx.client() {
            let fs = remote_fs(remote, "");
            let dest = std::env::temp_dir().join(name);
            if client
                .copy_file(&fs, path, "/", &dest.to_string_lossy())
                .is_ok()
            {
                info.set_text(&format!("Downloaded preview to {}", dest.display()));
                if matches!(category, crate::operations::FileTypeCategory::Text) {
                    if let Ok(text) = std::fs::read_to_string(&dest) {
                        let view = gtk::TextView::new();
                        view.set_monospace(true);
                        view.set_editable(false);
                        let shown = if text.len() > 200_000 {
                            format!("{}\n\n… truncated …", &text[..200_000])
                        } else {
                            text
                        };
                        apply_syntax_highlight(&view, name, &shown);
                        attach_live_syntax(&view, name);
                        let scroll = gtk::ScrolledWindow::new();
                        scroll.set_vexpand(true);
                        scroll.set_child(Some(&view));
                        box_.append(&scroll);
                    }
                }
            }
        }
    }
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn remote_about(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("About {remote}"));
    dialog.set_content_width(560);
    dialog.set_content_height(640);
    let page = adw::PreferencesPage::new();
    let usage = adw::PreferencesGroup::new();
    usage.set_title("Usage");
    let features = adw::PreferencesGroup::new();
    features.set_title("Features");
    let hashes = adw::PreferencesGroup::new();
    hashes.set_title("Hashes");
    let metadata = adw::PreferencesGroup::new();
    metadata.set_title("Metadata");
    let fs = remote_fs(remote, "");
    let group = format!(
        "gtk/remote-about/{remote}-{}",
        chrono::Utc::now().timestamp_millis()
    );
    if let Some(client) = ctx.client() {
        match client.about(&fs) {
            Ok(about) => {
                for key in ["used", "free", "total", "trashed"] {
                    if let Some(value) = about.get(key).and_then(|x| x.as_i64()) {
                        let row = adw::ActionRow::new();
                        row.set_title(key);
                        row.set_subtitle(&crate::rclone::format_bytes(value));
                        usage.add(&row);
                    }
                }
                for (key, value) in about.as_object().cloned().unwrap_or_default() {
                    if matches!(key.as_str(), "used" | "free" | "total" | "trashed") {
                        continue;
                    }
                    let row = adw::ActionRow::new();
                    row.set_title(&key);
                    row.set_subtitle(&value.to_string());
                    usage.add(&row);
                }
            }
            Err(e) => {
                let row = adw::ActionRow::new();
                row.set_title("Unable to query disk usage");
                row.set_subtitle(&e.to_string());
                usage.add(&row);
            }
        }
        match client.size(&fs, "") {
            Ok(size) => {
                let row = adw::ActionRow::new();
                row.set_title("Objects");
                let count = size.get("count").and_then(|x| x.as_i64()).unwrap_or(0);
                let bytes = size.get("bytes").and_then(|x| x.as_i64()).unwrap_or(0);
                row.set_subtitle(&format!("{count} · {}", crate::rclone::format_bytes(bytes)));
                usage.add(&row);
            }
            Err(e) => {
                let row = adw::ActionRow::new();
                row.set_title("Size");
                row.set_subtitle(&e.to_string());
                usage.add(&row);
            }
        }
        let info = ctx.fs_info(remote).or_else(|| client.fs_info(&fs).ok());
        if let Some(info) = info {
            let name = adw::ActionRow::new();
            name.set_title("Name");
            name.set_subtitle(&info.name);
            usage.add(&name);
            let root = adw::ActionRow::new();
            root.set_title("Root");
            let root_text = if info.root.is_empty() {
                "/".to_string()
            } else {
                info.root.clone()
            };
            root.set_subtitle(&root_text);
            usage.add(&root);
            let precision = adw::ActionRow::new();
            precision.set_title("Timestamp precision");
            let precision_text = nanoseconds_to_duration(info.precision);
            precision.set_subtitle(&precision_text);
            usage.add(&precision);
            if info.hashes.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title("None");
                hashes.add(&row);
            } else {
                for hash in &info.hashes {
                    let row = adw::ActionRow::new();
                    row.set_title(&hash.to_ascii_uppercase());
                    hashes.add(&row);
                }
            }
            for (key, value) in &info.features {
                if key == "IsLocal" {
                    continue;
                }
                let row = adw::ActionRow::new();
                row.set_title(key);
                row.set_subtitle(if *value { "yes" } else { "no" });
                features.add(&row);
            }
            if let Some(obj) = info.metadata.as_object() {
                for (group_name, data) in obj {
                    if let Some(items) = data.as_object() {
                        for (key, meta) in items {
                            let row = adw::ActionRow::new();
                            row.set_title(&format!("{group_name}.{key}"));
                            let help = meta
                                .get("Help")
                                .or_else(|| meta.get("help"))
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            row.set_subtitle(help);
                            metadata.add(&row);
                        }
                    }
                }
            }
        }
    }
    page.add(&usage);
    page.add(&hashes);
    page.add(&features);
    page.add(&metadata);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    dialog.set_child(Some(&scroll));
    {
        let ctx = ctx.clone();
        dialog.connect_closed(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.job_stop_group(&group);
            }
        });
    }
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub(crate) fn download_file(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let dialog = gtk::FileDialog::new();
    dialog.set_initial_name(Some(name));
    let remote = remote.to_string();
    let path = path.to_string();
    dialog.save(
        Some(&win),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(dest) = file.path() else {
                return;
            };
            let Some(client) = ctx.client() else {
                return;
            };
            let fs = if remote == "local" {
                "/".into()
            } else {
                remote_fs(&remote, "")
            };
            let src_remote = if remote == "local" {
                path.trim_start_matches('/').to_string()
            } else {
                path
            };
            if let Err(e) = client.copy_file(&fs, &src_remote, "/", &dest.to_string_lossy()) {
                log::warn!("download failed: {e}");
            }
        },
    );
}

pub fn templates(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Templates");
    dialog.set_content_width(560);
    dialog.set_content_height(480);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for template in ctx.store.borrow().templates.clone() {
        let row = adw::ActionRow::new();
        row.set_title(&template.name);
        row.set_subtitle(&template.description);
        let apply = gtk::Button::with_label("Apply");
        apply.set_valign(gtk::Align::Center);
        apply.add_css_class("suggested-action");
        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.set_valign(gtk::Align::Center);
        {
            let ctx = ctx.clone();
            let parent = parent.clone();
            let values = template.values.clone();
            apply.connect_clicked(move |_| {
                if let Some(client) = ctx.client() {
                    match client.options_set(values.clone()) {
                        Ok(_) => {
                            let toast = adw::AlertDialog::new(
                                Some("Template applied"),
                                Some("Current rclone options were updated from this template."),
                            );
                            toast.add_response("ok", "OK");
                            toast.present(Some(&parent));
                        }
                        Err(e) => {
                            let err =
                                adw::AlertDialog::new(Some("Apply failed"), Some(&e.to_string()));
                            err.add_response("ok", "OK");
                            err.present(Some(&parent));
                        }
                    }
                } else if let Some(name) = ctx.selected_remote.borrow().clone() {
                    if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&name) {
                        for profiles in meta.profiles.values_mut() {
                            for profile in profiles.values_mut() {
                                merge_template_into(&mut profile.rclone, &values);
                            }
                        }
                    }
                    ctx.persist();
                }
            });
        }
        {
            let ctx = ctx.clone();
            let id = template.id.clone();
            let parent = parent.clone();
            delete.connect_clicked(move |_| {
                ctx.store.borrow_mut().templates.retain(|t| t.id != id);
                ctx.persist();
                templates(&parent, ctx.clone());
            });
        }
        row.add_suffix(&apply);
        row.add_suffix(&delete);
        list.append(&row);
    }
    if list.first_child().is_none() {
        let row = adw::ActionRow::new();
        row.set_title("No saved templates");
        list.append(&row);
    }
    let add = gtk::Button::with_label("Save current flags as template");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add.connect_clicked(move |_| {
            let values = ctx
                .client()
                .and_then(|c| c.options_get().ok())
                .unwrap_or(serde_json::json!({}));
            ctx.store
                .borrow_mut()
                .templates
                .push(crate::store::UserTemplate {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: format!("Template {}", chrono::Local::now().format("%Y-%m-%d %H:%M")),
                    description: "Captured rclone options".into(),
                    icon: "emblem-ok-symbolic".into(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    values,
                });
            ctx.persist();
            templates(&parent, ctx.clone());
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&scrolled_list(&list));
    box_.append(&add);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn archive_create(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str, path: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Create archive");
    let name = adw::EntryRow::new();
    name.set_title("Archive name");
    name.set_text("archive.zip");
    let format = adw::ComboRow::new();
    format.set_title("Format");
    format.set_model(Some(&gtk::StringList::new(&[
        "zip", "tar", "tar.gz", "tar.bz2", "tar.xz", "tar.zst",
    ])));
    let start = gtk::Button::with_label("Create");
    start.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let src_path = path.to_string();
        let name = name.clone();
        start.connect_clicked(move |_| {
            let mut dest = name.text().to_string();
            if dest.is_empty() {
                dest = "archive.zip".into();
            }
            if let Some(client) = ctx.client() {
                let fs = if remote == "local" {
                    "/".into()
                } else {
                    remote_fs(&remote, "")
                };
                match client.start_job(
                    "operations/copyfile",
                    serde_json::json!({
                        "srcFs": fs,
                        "srcRemote": src_path,
                        "dstFs": fs,
                        "dstRemote": dest
                    }),
                ) {
                    Ok(id) => {
                        ctx.store
                            .borrow_mut()
                            .push_log(&remote, format!("archive job #{id} queued"));
                        dialog.close();
                    }
                    Err(e) => {
                        let err =
                            adw::AlertDialog::new(Some("Archive failed"), Some(&e.to_string()));
                        err.add_response("ok", "OK");
                        err.present(Some(&dialog));
                    }
                }
            }
        });
    }
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&format);
    page.add(&group);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&page);
    box_.append(&start);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn install_rclone_update(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
) {
    let dest = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".local/bin");
    match crate::updater::install_rclone_binary(&dest) {
        Ok(path) => {
            ctx.settings.borrow_mut().core.rclone_binary = path.to_string_lossy().into_owned();
            ctx.persist();
            toast.add_toast(adw::Toast::new(&format!(
                "Installed rclone to {}",
                path.display()
            )));
        }
        Err(e) => {
            let err = adw::AlertDialog::new(Some("Update failed"), Some(&e));
            err.add_response("ok", "OK");
            err.present(Some(parent));
        }
    }
}

pub fn helper_profiles(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("Helper profiles — {remote}"));
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let kind = adw::ComboRow::new();
    kind.set_title("Category");
    kind.set_model(Some(&gtk::StringList::new(&["vfs", "filter", "backend"])));
    let name = adw::EntryRow::new();
    name.set_title("Profile name");
    name.set_text("default");
    let json_view = gtk::TextView::new();
    json_view.set_monospace(true);
    json_view.set_wrap_mode(gtk::WrapMode::Word);
    let load = {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let name = name.clone();
        let json_view = json_view.clone();
        let kind = kind.clone();
        move || {
            let kind_name = ["vfs", "filter", "backend"]
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("vfs");
            let text = ctx
                .store
                .borrow()
                .remotes
                .get(&remote)
                .and_then(|m| m.helper_profile(kind_name, &name.text()))
                .map(|v| serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()))
                .unwrap_or_else(|| "{}".into());
            json_view.buffer().set_text(&text);
        }
    };
    load();
    {
        let load = load.clone();
        kind.connect_selected_notify(move |_| load());
    }
    let save = gtk::Button::with_label("Save profile");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let name = name.clone();
        let json_view = json_view.clone();
        let kind = kind.clone();
        save.connect_clicked(move |_| {
            let kind_name = ["vfs", "filter", "backend"]
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("vfs");
            let buffer = json_view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let value = serde_json::from_str::<serde_json::Value>(text.as_str())
                .unwrap_or(serde_json::json!({}));
            let key = name.text().to_string();
            if key.is_empty() {
                return;
            }
            if let Some(meta) = ctx.store.borrow_mut().remotes.get_mut(&remote) {
                match kind_name {
                    "filter" => {
                        meta.filter_configs.insert(key, value);
                    }
                    "backend" => {
                        meta.backend_configs.insert(key, value);
                    }
                    _ => {
                        meta.vfs_configs.insert(key, value);
                    }
                }
            }
            ctx.persist();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&kind);
    group.add(&name);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_min_content_height(240);
    scroll.set_child(Some(&json_view));
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&scroll);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

pub fn item_order(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Remote order and visibility");
    dialog.set_content_width(480);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let all_names: Vec<String> = ctx
        .snapshot
        .borrow()
        .remotes
        .iter()
        .map(|r| r.name.clone())
        .collect();
    ctx.store.borrow_mut().ensure_remote_order(&all_names);
    let names = Rc::new(RefCell::new(ctx.store.borrow().remote_order.clone()));
    let hidden = Rc::new(RefCell::new(ctx.store.borrow().hidden_remotes.clone()));
    fn refill(
        list: &gtk::ListBox,
        names: &Rc<RefCell<Vec<String>>>,
        hidden: &Rc<RefCell<Vec<String>>>,
    ) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        let current = names.borrow().clone();
        for (idx, name) in current.iter().enumerate() {
            let row = adw::SwitchRow::new();
            row.set_title(name);
            row.set_subtitle("Visible in sidebar and overview");
            row.set_active(!hidden.borrow().iter().any(|n| n == name));
            {
                let hidden = hidden.clone();
                let name = name.clone();
                row.connect_active_notify(move |row| {
                    let mut hidden = hidden.borrow_mut();
                    hidden.retain(|n| n != &name);
                    if !row.is_active() {
                        hidden.push(name.clone());
                    }
                });
            }
            let up = gtk::Button::from_icon_name("go-up-symbolic");
            up.set_valign(gtk::Align::Center);
            up.set_sensitive(idx > 0);
            let down = gtk::Button::from_icon_name("go-down-symbolic");
            down.set_valign(gtk::Align::Center);
            down.set_sensitive(idx + 1 < current.len());
            {
                let names = names.clone();
                let hidden = hidden.clone();
                let list = list.clone();
                let idx = idx;
                up.connect_clicked(move |_| {
                    {
                        let mut names = names.borrow_mut();
                        if idx > 0 {
                            names.swap(idx, idx - 1);
                        }
                    }
                    refill(&list, &names, &hidden);
                });
            }
            {
                let names = names.clone();
                let hidden = hidden.clone();
                let list = list.clone();
                let idx = idx;
                down.connect_clicked(move |_| {
                    {
                        let mut names = names.borrow_mut();
                        if idx + 1 < names.len() {
                            names.swap(idx, idx + 1);
                        }
                    }
                    refill(&list, &names, &hidden);
                });
            }
            row.add_suffix(&up);
            row.add_suffix(&down);
            list.append(&row);
        }
    }
    refill(&list, &names, &hidden);
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let names = names.clone();
        let hidden = hidden.clone();
        save.connect_clicked(move |_| {
            ctx.store.borrow_mut().remote_order = names.borrow().clone();
            ctx.store.borrow_mut().hidden_remotes = hidden.borrow().clone();
            ctx.persist();
            ctx.refresh_runtime();
            on_done();
            dialog.close();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&scrolled_list(&list));
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn scrolled_list(list: &gtk::ListBox) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(list));
    scroll
}

pub(crate) fn present_window_or_dialog(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    dialog: &adw::Dialog,
) {
    if ctx.settings.borrow().general.standalone_dialogs {
        let win = adw::Window::new();
        let title = dialog.title();
        win.set_title(Some(title.as_str()));
        let width = dialog.content_width();
        let height = dialog.content_height();
        if width > 0 {
            win.set_default_width(width);
        }
        if height > 0 {
            win.set_default_height(height);
        }
        if let Some(child) = dialog.child() {
            dialog.set_child(gtk::Widget::NONE);
            win.set_content(Some(&child));
        }
        if let Some(p) = parent.root().and_downcast::<gtk::Window>() {
            win.set_transient_for(Some(&p));
        }
        win.present();
    } else {
        dialog.present(Some(parent));
    }
}

pub(crate) fn helper_combo(title: &str, names: &[String], selected: &str) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    row.set_model(Some(&gtk::StringList::new(&refs)));
    if let Some(idx) = names.iter().position(|n| n == selected) {
        row.set_selected(idx as u32);
    }
    row
}

pub(crate) fn helper_selected(row: &adw::ComboRow, names: &[String]) -> String {
    names
        .get(row.selected() as usize)
        .cloned()
        .filter(|s| s != "—")
        .unwrap_or_default()
}

pub(crate) fn attach_path_kind(row: &adw::EntryRow, current_remote: &str) -> adw::ComboRow {
    let combo = adw::ComboRow::new();
    combo.set_title("Path type");
    combo.set_model(Some(&gtk::StringList::new(&[
        "Local",
        "Current remote",
        "Other remote",
    ])));
    let remote = current_remote.to_string();
    let kind = crate::path_kind::infer_path_kind(&row.text(), &remote);
    combo.set_selected(crate::path_kind::kind_index(kind));
    {
        let row = row.clone();
        let remote = remote.clone();
        combo.connect_selected_notify(move |combo| {
            let kind = crate::path_kind::kind_from_index(combo.selected());
            let rewritten = crate::path_kind::rewrite_path_for_kind(&row.text(), &remote, kind);
            if rewritten != row.text().as_str() {
                row.set_text(&rewritten);
            }
        });
    }
    {
        let combo = combo.clone();
        let remote = remote.clone();
        row.connect_changed(move |row| {
            let kind = crate::path_kind::infer_path_kind(&row.text(), &remote);
            let idx = crate::path_kind::kind_index(kind);
            if combo.selected() != idx {
                combo.set_selected(idx);
            }
        });
    }
    combo
}

pub(crate) fn attach_path_picker(
    ctx: &AppCtx,
    row: &adw::EntryRow,
    config: crate::picker::FilePickerConfig,
) {
    let btn = gtk::Button::from_icon_name("folder-open-symbolic");
    btn.set_valign(gtk::Align::Center);
    btn.set_tooltip_text(Some("Browse"));
    let ctx = ctx.clone();
    let picked = row.clone();
    btn.connect_clicked(move |_| {
        let mut config = config.clone();
        if config.initial_location.is_none() && !picked.text().is_empty() {
            config.initial_location = Some(picked.text().to_string());
        }
        if config.mode == crate::picker::PickerMode::Local {
            let Some(win) = picked.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let dialog = gtk::FileDialog::new();
            let row = picked.clone();
            if config.selection == crate::picker::PickerSelection::Files {
                dialog.open(
                    Some(&win),
                    None::<gio::Cancellable>.as_ref(),
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                row.set_text(&path.to_string_lossy());
                            }
                        }
                    },
                );
            } else {
                dialog.select_folder(
                    Some(&win),
                    None::<gio::Cancellable>.as_ref(),
                    move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                row.set_text(&path.to_string_lossy());
                            }
                        }
                    },
                );
            }
            return;
        }
        let row = picked.clone();
        ctx.request_picker(
            config.clone(),
            Rc::new(move |result| {
                if !result.cancelled {
                    row.set_text(&result.formatted_path());
                }
            }),
        );
    });
    row.add_suffix(&btn);
}

fn ensure_syntax_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    for kind in [
        crate::syntax::TokenKind::Keyword,
        crate::syntax::TokenKind::String,
        crate::syntax::TokenKind::Comment,
        crate::syntax::TokenKind::Number,
    ] {
        if table.lookup(kind.tag_name()).is_none() {
            let tag = gtk::TextTag::builder()
                .name(kind.tag_name())
                .foreground(kind.color())
                .build();
            table.add(&tag);
        }
    }
}

fn paint_syntax(buffer: &gtk::TextBuffer, name: &str, text: &str) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    buffer.remove_all_tags(&start, &end);
    let Some(lang) = crate::syntax::language_from_name(name) else {
        return;
    };
    for span in crate::syntax::highlight(text, lang) {
        let start = buffer.iter_at_offset(span.start as i32);
        let end = buffer.iter_at_offset(span.end as i32);
        buffer.apply_tag_by_name(span.kind.tag_name(), &start, &end);
    }
}

fn apply_syntax_highlight(view: &gtk::TextView, name: &str, text: &str) {
    let buffer = view.buffer();
    ensure_syntax_tags(&buffer);
    buffer.set_text(text);
    paint_syntax(&buffer, name, text);
}

fn attach_live_syntax(view: &gtk::TextView, name: &str) {
    let name = name.to_string();
    view.buffer().connect_changed(move |buffer| {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        paint_syntax(buffer, &name, text.as_str());
    });
}

const ACTION_KINDS: &[&str] = &[
    "os_toast", "webhook", "telegram", "whatsapp", "script", "email", "mqtt",
];
const EVENT_KINDS: &[AlertEventKind] = &[
    AlertEventKind::Job,
    AlertEventKind::Serve,
    AlertEventKind::Mount,
    AlertEventKind::Engine,
    AlertEventKind::Update,
    AlertEventKind::Automation,
    AlertEventKind::System,
];
const SEVERITIES: &[&str] = &["info", "warning", "average", "high", "critical"];

fn alert_rule_editor(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, existing_id: Option<String>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Alert rule");
    dialog.set_content_width(520);
    dialog.set_content_height(640);
    let existing = existing_id.as_ref().and_then(|id| {
        ctx.store
            .borrow()
            .alert_rules
            .iter()
            .find(|r| &r.id == id)
            .cloned()
    });
    let name = adw::EntryRow::new();
    name.set_title("Name");
    name.set_text(
        existing
            .as_ref()
            .map(|r| r.name.as_str())
            .unwrap_or("New rule"),
    );
    let enabled = adw::SwitchRow::new();
    enabled.set_title("Enabled");
    enabled.set_active(existing.as_ref().map(|r| r.enabled).unwrap_or(true));
    let auto_ack = adw::SwitchRow::new();
    auto_ack.set_title("Auto-acknowledge");
    auto_ack.set_active(
        existing
            .as_ref()
            .map(|r| r.auto_acknowledge)
            .unwrap_or(false),
    );
    let severity = adw::ComboRow::new();
    severity.set_title("Minimum severity");
    severity.set_model(Some(&gtk::StringList::new(SEVERITIES)));
    if let Some(rule) = &existing {
        if let Some(idx) = SEVERITIES
            .iter()
            .position(|s| *s == rule.severity_min.as_str())
        {
            severity.set_selected(idx as u32);
        }
    }
    let cooldown = adw::EntryRow::new();
    cooldown.set_title("Cooldown (seconds)");
    cooldown.set_text(
        &existing
            .as_ref()
            .map(|r| r.cooldown_secs.to_string())
            .unwrap_or_else(|| "0".into()),
    );
    let remotes = adw::EntryRow::new();
    remotes.set_title("Remote filter (comma-separated)");
    remotes.set_text(
        &existing
            .as_ref()
            .map(|r| r.remote_filter.join(", "))
            .unwrap_or_default(),
    );
    let backends = adw::EntryRow::new();
    backends.set_title("Backend filter (comma-separated)");
    backends.set_text(
        &existing
            .as_ref()
            .map(|r| r.backend_filter.join(", "))
            .unwrap_or_default(),
    );
    let profiles = adw::EntryRow::new();
    profiles.set_title("Profile filter (comma-separated)");
    profiles.set_text(
        &existing
            .as_ref()
            .map(|r| r.profile_filter.join(", "))
            .unwrap_or_default(),
    );
    let origins = adw::EntryRow::new();
    origins.set_title("Origin filter (comma-separated)");
    origins.set_text(
        &existing
            .as_ref()
            .map(|r| r.origin_filter.join(", "))
            .unwrap_or_default(),
    );
    let event_switches: Vec<(AlertEventKind, adw::SwitchRow)> = EVENT_KINDS
        .iter()
        .map(|kind| {
            let row = adw::SwitchRow::new();
            row.set_title(&format!("Event: {}", kind.as_str()));
            row.set_active(
                existing
                    .as_ref()
                    .map(|r| r.event_filter.is_empty() || r.event_filter.contains(kind))
                    .unwrap_or(*kind == AlertEventKind::Job),
            );
            (kind.clone(), row)
        })
        .collect();
    let action_switches: Vec<(String, adw::SwitchRow)> = ctx
        .store
        .borrow()
        .alert_actions
        .iter()
        .map(|action| {
            let row = adw::SwitchRow::new();
            row.set_title(&action.name);
            row.set_subtitle(&action.kind);
            row.set_active(
                existing
                    .as_ref()
                    .map(|r| r.action_ids.contains(&action.id))
                    .unwrap_or(false),
            );
            (action.id.clone(), row)
        })
        .collect();

    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    delete.set_visible(existing_id.is_some());
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let event_switches = event_switches.clone();
        let action_switches = action_switches.clone();
        let existing_id = existing_id.clone();
        let name = name.clone();
        let enabled = enabled.clone();
        let auto_ack = auto_ack.clone();
        let severity = severity.clone();
        let cooldown = cooldown.clone();
        let remotes = remotes.clone();
        let backends = backends.clone();
        let profiles = profiles.clone();
        let origins = origins.clone();
        save.connect_clicked(move |_| {
            let mut rule = existing_id
                .as_ref()
                .and_then(|id| {
                    ctx.store
                        .borrow()
                        .alert_rules
                        .iter()
                        .find(|r| &r.id == id)
                        .cloned()
                })
                .unwrap_or_else(|| AlertRule::new(name.text().to_string()));
            rule.name = name.text().to_string();
            rule.enabled = enabled.is_active();
            rule.auto_acknowledge = auto_ack.is_active();
            rule.severity_min = AlertSeverity::parse(
                SEVERITIES
                    .get(severity.selected() as usize)
                    .copied()
                    .unwrap_or("info"),
            );
            rule.cooldown_secs = cooldown.text().parse().unwrap_or(0);
            rule.remote_filter = split_csv(&remotes.text());
            rule.backend_filter = split_csv(&backends.text());
            rule.profile_filter = split_csv(&profiles.text());
            rule.origin_filter = split_csv(&origins.text());
            rule.event_filter = event_switches
                .iter()
                .filter(|(_, row)| row.is_active())
                .map(|(kind, _)| *kind)
                .collect();
            rule.action_ids = action_switches
                .iter()
                .filter(|(_, row)| row.is_active())
                .map(|(id, _)| id.clone())
                .collect();
            {
                let mut store = ctx.store.borrow_mut();
                if let Some(idx) = store.alert_rules.iter().position(|r| r.id == rule.id) {
                    store.alert_rules[idx] = rule;
                } else {
                    store.alert_rules.push(rule);
                }
            }
            ctx.persist();
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        delete.connect_clicked(move |_| {
            if let Some(id) = &existing_id {
                ctx.store.borrow_mut().alert_rules.retain(|r| &r.id != id);
                ctx.persist();
            }
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }

    let page = adw::PreferencesPage::new();
    let general = adw::PreferencesGroup::new();
    general.set_title("Rule");
    general.add(&name);
    general.add(&enabled);
    general.add(&auto_ack);
    general.add(&severity);
    general.add(&cooldown);
    general.add(&remotes);
    general.add(&backends);
    general.add(&profiles);
    general.add(&origins);
    page.add(&general);
    let events = adw::PreferencesGroup::new();
    events.set_title("Events");
    for (_, row) in &event_switches {
        events.add(row);
    }
    page.add(&events);
    let actions = adw::PreferencesGroup::new();
    actions.set_title("Actions");
    for (_, row) in &action_switches {
        actions.add(row);
    }
    page.add(&actions);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&save);
    buttons.append(&delete);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    box_.append(&scroll);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn alert_action_editor(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, existing_id: Option<String>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Alert action");
    dialog.set_content_width(520);
    dialog.set_content_height(620);
    let existing = existing_id.as_ref().and_then(|id| {
        ctx.store
            .borrow()
            .alert_actions
            .iter()
            .find(|a| &a.id == id)
            .cloned()
    });
    let name = adw::EntryRow::new();
    name.set_title("Name");
    name.set_text(
        existing
            .as_ref()
            .map(|a| a.name.as_str())
            .unwrap_or("New action"),
    );
    let enabled = adw::SwitchRow::new();
    enabled.set_title("Enabled");
    enabled.set_active(existing.as_ref().map(|a| a.enabled).unwrap_or(true));
    let kind = adw::ComboRow::new();
    kind.set_title("Kind");
    kind.set_model(Some(&gtk::StringList::new(ACTION_KINDS)));
    if let Some(action) = &existing {
        if let Some(idx) = ACTION_KINDS.iter().position(|k| *k == action.kind) {
            kind.set_selected(idx as u32);
        }
    }
    let url = adw::EntryRow::new();
    url.set_title("URL");
    let method = adw::EntryRow::new();
    method.set_title("Method");
    let token = adw::PasswordEntryRow::new();
    token.set_title("Token");
    let extra = adw::EntryRow::new();
    extra.set_title("Extra");
    let body = adw::EntryRow::new();
    body.set_title("Body template");
    if let Some(action) = &existing {
        url.set_text(&action_cfg(action, &["url", "broker_url", "smtp_server"]));
        method.set_text(&action_cfg(action, &["method", "topic", "smtp_port"]));
        token.set_text(&action_cfg(
            action,
            &["bot_token", "password", "apikey", "username"],
        ));
        extra.set_text(&action_cfg(
            action,
            &["chat_id", "phone", "from", "command", "to"],
        ));
        body.set_text(
            action
                .config
                .get("body_template")
                .and_then(|x| x.as_str())
                .unwrap_or("{{title}}: {{body}}"),
        );
    } else {
        body.set_text("{{title}}: {{body}}");
    }
    let test = gtk::Button::with_label("Test");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let delete = gtk::Button::with_label("Delete");
    delete.add_css_class("destructive-action");
    delete.set_visible(existing_id.is_some());
    let collect = {
        let name = name.clone();
        let enabled = enabled.clone();
        let kind = kind.clone();
        let url = url.clone();
        let method = method.clone();
        let token = token.clone();
        let extra = extra.clone();
        let body = body.clone();
        let existing_id = existing_id.clone();
        let ctx = ctx.clone();
        move || {
            let selected = ACTION_KINDS
                .get(kind.selected() as usize)
                .copied()
                .unwrap_or("os_toast");
            let mut action = existing_id
                .as_ref()
                .and_then(|id| {
                    ctx.store
                        .borrow()
                        .alert_actions
                        .iter()
                        .find(|a| &a.id == id)
                        .cloned()
                })
                .unwrap_or_else(|| AlertAction::new(name.text().to_string(), selected.into()));
            action.name = name.text().to_string();
            action.enabled = enabled.is_active();
            action.kind = selected.into();
            action.config = crate::store::alert_action_config(
                selected,
                &crate::store::AlertActionDraft {
                    url: url.text().to_string(),
                    method: method.text().to_string(),
                    token: token.text().to_string(),
                    extra: extra.text().to_string(),
                    body: body.text().to_string(),
                },
            );
            action
        }
    };
    {
        let collect = collect.clone();
        let parent = parent.clone();
        test.connect_clicked(move |_| {
            let action = collect();
            let event = AlertEvent::new(
                AlertEventKind::System,
                AlertSeverity::Info,
                "Test alert".into(),
                format!("Testing action {}", action.name),
            );
            crate::store::dispatch_action(&action, &event);
            let toast = adw::AlertDialog::new(
                Some("Test sent"),
                Some("The action was invoked with a sample event."),
            );
            toast.add_response("ok", "OK");
            toast.present(Some(&parent));
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        let collect = collect.clone();
        save.connect_clicked(move |_| {
            let action = collect();
            {
                let mut store = ctx.store.borrow_mut();
                if let Some(idx) = store.alert_actions.iter().position(|a| a.id == action.id) {
                    store.alert_actions[idx] = action;
                } else {
                    store.alert_actions.push(action);
                }
            }
            ctx.persist();
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let parent = parent.clone();
        delete.connect_clicked(move |_| {
            if let Some(id) = &existing_id {
                ctx.store.borrow_mut().alert_actions.retain(|a| &a.id != id);
                ctx.persist();
            }
            dialog.close();
            alerts(&parent, ctx.clone());
        });
    }
    let sync_fields = {
        let url = url.clone();
        let method = method.clone();
        let token = token.clone();
        let extra = extra.clone();
        let body = body.clone();
        move |selected: &str| match selected {
            "os_toast" => {
                url.set_visible(false);
                method.set_visible(false);
                token.set_visible(false);
                extra.set_visible(false);
                body.set_visible(true);
                body.set_title("Toast body template");
            }
            "webhook" => {
                url.set_visible(true);
                url.set_title("Webhook URL");
                method.set_visible(true);
                method.set_title("HTTP method");
                token.set_visible(false);
                extra.set_visible(false);
                body.set_visible(true);
                body.set_title("JSON / body template");
            }
            "telegram" => {
                url.set_visible(false);
                method.set_visible(false);
                token.set_visible(true);
                token.set_title("Bot token");
                extra.set_visible(true);
                extra.set_title("Chat ID");
                body.set_visible(true);
                body.set_title("Message template");
            }
            "whatsapp" => {
                url.set_visible(true);
                url.set_title("Gateway URL (optional)");
                method.set_visible(false);
                token.set_visible(true);
                token.set_title("API key");
                extra.set_visible(true);
                extra.set_title("Phone number");
                body.set_visible(true);
                body.set_title("Message template");
            }
            "script" => {
                url.set_visible(false);
                method.set_visible(false);
                token.set_visible(false);
                extra.set_visible(true);
                extra.set_title("Command");
                body.set_visible(true);
                body.set_title("Stdin template");
            }
            "email" => {
                url.set_visible(true);
                url.set_title("SMTP host");
                method.set_visible(true);
                method.set_title("SMTP port");
                token.set_visible(true);
                token.set_title("Password");
                extra.set_visible(true);
                extra.set_title("From / to address");
                body.set_visible(true);
                body.set_title("Body template");
            }
            "mqtt" => {
                url.set_visible(true);
                url.set_title("Broker URL");
                method.set_visible(true);
                method.set_title("Topic");
                token.set_visible(true);
                token.set_title("Password");
                extra.set_visible(false);
                body.set_visible(true);
                body.set_title("Payload template");
            }
            _ => {}
        }
    };
    {
        let sync_fields = sync_fields.clone();
        let initial = ACTION_KINDS
            .get(kind.selected() as usize)
            .copied()
            .unwrap_or("os_toast");
        sync_fields(initial);
        kind.connect_selected_notify(move |row| {
            let selected = ACTION_KINDS
                .get(row.selected() as usize)
                .copied()
                .unwrap_or("os_toast");
            sync_fields(selected);
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&enabled);
    group.add(&kind);
    group.add(&url);
    group.add(&method);
    group.add(&token);
    group.add(&extra);
    group.add(&body);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&test);
    buttons.append(&save);
    buttons.append(&delete);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&group);
    box_.append(&buttons);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn action_cfg(action: &AlertAction, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = action.config.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            } else if let Some(n) = v.as_u64() {
                return n.to_string();
            }
        }
    }
    String::new()
}

fn split_csv(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn repair(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Repair rclone");
    dialog.set_content_width(560);
    dialog.set_content_height(520);
    let version = ctx
        .client()
        .and_then(|c| c.version().ok())
        .or_else(|| ctx.engine.borrow().as_ref().map(|e| e.version.clone()));
    let issues = crate::repair::diagnose(
        &ctx.settings.borrow(),
        ctx.engine_ready(),
        ctx.client().as_ref(),
        version.as_deref(),
    );
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    if issues.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No issues detected");
        row.set_subtitle("rclone, FUSE, config, and the RC engine look healthy.");
        list.append(&row);
    }
    for issue in issues {
        let row = adw::ActionRow::new();
        row.set_title(&issue.title);
        row.set_subtitle(&issue.detail);
        let btn = gtk::Button::with_label(&issue.action);
        btn.set_valign(gtk::Align::Center);
        btn.add_css_class("suggested-action");
        {
            let ctx = ctx.clone();
            let toast = toast.clone();
            let parent = parent.clone();
            let kind = issue.kind.clone();
            btn.connect_clicked(move |_| match kind {
                crate::repair::RepairKind::MissingBinary | crate::repair::RepairKind::VersionTooOld => {
                    install_rclone_update(&parent, ctx.clone(), toast.clone());
                    ctx.restart_engine();
                }
                crate::repair::RepairKind::FuseMissing => {
                    let help = adw::AlertDialog::new(
                        Some("Install FUSE"),
                        Some("On Debian/Ubuntu: sudo apt install fuse3\nOn Fedora: sudo dnf install fuse3\nThen restart the session so /dev/fuse is available."),
                    );
                    help.add_response("ok", "OK");
                    help.present(Some(&parent));
                }
                crate::repair::RepairKind::PasswordRequired => {
                    preferences(&parent, ctx.clone());
                }
                crate::repair::RepairKind::ConfigUnreadable => {
                    restore_or_pick_config(&parent, ctx.clone());
                }
                crate::repair::RepairKind::EngineUnreachable => {
                    ctx.restart_engine();
                    toast.add_toast(adw::Toast::new("Engine restart requested"));
                }
            });
        }
        row.add_suffix(&btn);
        list.append(&row);
    }
    let install = gtk::Button::with_label("Install rclone");
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let parent = parent.clone();
        install.connect_clicked(move |_| {
            install_rclone_update(&parent, ctx.clone(), toast.clone());
            ctx.restart_engine();
        });
    }
    let browse = gtk::Button::with_label("Choose binary…");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        browse.connect_clicked(move |_| pick_rclone_binary(&parent, ctx.clone()));
    }
    let config_btn = gtk::Button::with_label("Choose rclone.conf…");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        config_btn.connect_clicked(move |_| restore_or_pick_config(&parent, ctx.clone()));
    }
    let restart = gtk::Button::with_label("Restart engine");
    {
        let ctx = ctx.clone();
        restart.connect_clicked(move |_| {
            ctx.restart_engine();
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(16);
    box_.set_margin_start(16);
    box_.set_margin_end(16);
    box_.set_margin_bottom(16);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_min_content_height(220);
    scroll.set_child(Some(&list));
    box_.append(&scroll);
    box_.append(&install);
    box_.append(&browse);
    box_.append(&config_btn);
    box_.append(&restart);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}

fn pick_rclone_binary(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let picker = gtk::FileDialog::new();
    picker.open(
        Some(&win),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    ctx.settings.borrow_mut().core.rclone_binary =
                        path.to_string_lossy().into_owned();
                    ctx.persist();
                    ctx.restart_engine();
                }
            }
        },
    );
}

fn restore_or_pick_config(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let picker = gtk::FileDialog::new();
    picker.open(
        Some(&win),
        None::<gio::Cancellable>.as_ref(),
        move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let flag = format!("--config={}", path.display());
                    let flags = &mut ctx.settings.borrow_mut().core.rclone_additional_flags;
                    flags.retain(|f| !f.starts_with("--config"));
                    flags.push(flag);
                    ctx.persist();
                    ctx.restart_engine();
                }
            }
        },
    );
}

pub fn multi_rename(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    names: Vec<String>,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Rename items");
    dialog.set_content_width(560);
    dialog.set_content_height(620);
    let mode = adw::ComboRow::new();
    mode.set_title("Mode");
    mode.set_model(Some(&gtk::StringList::new(&[
        "Template",
        "Find and replace",
    ])));
    let template = adw::EntryRow::new();
    template.set_title("Template");
    template.set_text("[Original file name]");
    let find = adw::EntryRow::new();
    find.set_title("Find");
    let replace = adw::EntryRow::new();
    replace.set_title("Replace with");
    let start = adw::EntryRow::new();
    start.set_title("Counter start");
    start.set_text("1");
    let step = adw::EntryRow::new();
    step.set_title("Counter step");
    step.set_text("1");
    let pad = adw::EntryRow::new();
    pad.set_title("Counter padding");
    pad.set_text("2");
    let preview = gtk::ListBox::new();
    preview.add_css_class("boxed-list");
    let plan_slots = Rc::new(RefCell::new((
        template.clone(),
        find.clone(),
        replace.clone(),
        start.clone(),
        step.clone(),
        pad.clone(),
        mode.clone(),
    )));
    let refresh_preview = {
        let names = names.clone();
        let preview = preview.clone();
        let plan_slots = plan_slots.clone();
        move || {
            while let Some(child) = preview.first_child() {
                preview.remove(&child);
            }
            let slots = plan_slots.borrow();
            let plan = RenamePlan {
                mode: if slots.6.selected() == 0 {
                    RenameMode::Template
                } else {
                    RenameMode::Replace
                },
                template: slots.0.text().to_string(),
                find_text: slots.1.text().to_string(),
                replace_with: slots.2.text().to_string(),
                counter_start: slots.3.text().parse().unwrap_or(1),
                counter_step: slots.4.text().parse().unwrap_or(1),
                counter_padding: slots.5.text().parse().unwrap_or(2),
                case_sensitive: false,
            };
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            for row in rename_preview(&names, &plan, &date) {
                let item = adw::ActionRow::new();
                item.set_title(&row.original);
                item.set_subtitle(&if row.has_error {
                    format!("{} (invalid)", row.new_name)
                } else {
                    row.new_name
                });
                preview.append(&item);
            }
        }
    };
    for entry in [&template, &find, &replace, &start, &step, &pad] {
        let refresh_preview = refresh_preview.clone();
        entry.connect_changed(move |_| refresh_preview());
    }
    {
        let refresh_preview = refresh_preview.clone();
        mode.connect_selected_notify(move |_| refresh_preview());
    }
    refresh_preview();
    let apply = gtk::Button::with_label("Rename");
    apply.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let remote = remote.to_string();
        let folder = path.to_string();
        let names = names.clone();
        let plan_slots = plan_slots.clone();
        apply.connect_clicked(move |_| {
            let Some(client) = ctx.client() else {
                return;
            };
            let slots = plan_slots.borrow();
            let plan = RenamePlan {
                mode: if slots.6.selected() == 0 {
                    RenameMode::Template
                } else {
                    RenameMode::Replace
                },
                template: slots.0.text().to_string(),
                find_text: slots.1.text().to_string(),
                replace_with: slots.2.text().to_string(),
                counter_start: slots.3.text().parse().unwrap_or(1),
                counter_step: slots.4.text().parse().unwrap_or(1),
                counter_padding: slots.5.text().parse().unwrap_or(2),
                case_sensitive: false,
            };
            let date = chrono::Local::now().format("%Y-%m-%d").to_string();
            let rows = rename_preview(&names, &plan, &date);
            if crate::rename::has_errors(&rows) || !crate::rename::has_changes(&rows) {
                return;
            }
            let fs = if remote == "local" {
                "/".into()
            } else {
                remote_fs(&remote, "")
            };
            for row in rows {
                if row.new_name == row.original {
                    continue;
                }
                let src = crate::rclone::join_remote_path(&folder, &row.original);
                let dst = crate::rclone::join_remote_path(&folder, &row.new_name);
                let src = if remote == "local" {
                    src.trim_start_matches('/').to_string()
                } else {
                    src
                };
                let dst = if remote == "local" {
                    dst.trim_start_matches('/').to_string()
                } else {
                    dst
                };
                if let Err(e) = client.move_file(&fs, &src, &fs, &dst) {
                    let err = adw::AlertDialog::new(Some("Rename failed"), Some(&e.to_string()));
                    err.add_response("ok", "OK");
                    err.present(Some(&dialog));
                    return;
                }
            }
            on_done();
            dialog.close();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&mode);
    group.add(&template);
    group.add(&find);
    group.add(&replace);
    group.add(&start);
    group.add(&step);
    group.add(&pad);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&preview));
    box_.append(&group);
    box_.append(&scroll);
    box_.append(&apply);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
}
