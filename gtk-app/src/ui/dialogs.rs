use super::AppCtx;
use crate::backup;
use crate::jobs::{
    build_job_params, default_dest, default_source, flatten_rclone, job_from_status,
    merge_template_into, start_request,
};
use crate::operations::OperationType;
use crate::rclone::{describe_cron, remote_fs, validate_cron};
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
    let lang_model = gtk::StringList::new(langs);
    let lang = adw::ComboRow::new();
    lang.set_title("Language");
    lang.set_subtitle("Application language");
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
    c1.add(&binary);
    let bw = adw::EntryRow::new();
    bw.set_title("Bandwidth limit");
    bw.set_text(&ctx.settings.borrow().core.bandwidth_limit);
    {
        let ctx = ctx.clone();
        bw.connect_changed(move |row| {
            let rate = row.text().to_string();
            ctx.settings.borrow_mut().core.bandwidth_limit = rate.clone();
            ctx.persist();
            if let Some(client) = ctx.client() {
                let _ = client.bwlimit(if rate.is_empty() { None } else { Some(&rate) });
            }
        });
    }
    c1.add(&bw);
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
    dev.add(&d1);

    dialog.add(&general);
    dialog.add(&core);
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
    let key = remote.unwrap_or_else(|| "_engine".into());
    let text = ctx
        .store
        .borrow()
        .logs
        .get(&key)
        .map(|lines| lines.join("\n"))
        .unwrap_or_else(|| "No logs yet.".into());
    view.buffer().set_text(&text);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&view));
    dialog.set_child(Some(&scroll));
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
    local.set_subtitle(&format!(
        "127.0.0.1:{port} · ready={ready}{}",
        if active.is_empty() || active == "local" {
            " · active"
        } else {
            ""
        }
    ));
    let use_local = gtk::Button::with_label("Use");
    use_local.set_valign(gtk::Align::Center);
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        use_local.connect_clicked(move |_| {
            ctx.settings.borrow_mut().core.active_backend = String::new();
            ctx.persist();
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
        row.set_subtitle(&format!("{}:{}{marker}", backend.host, backend.port));
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
                ctx.settings.borrow_mut().core.active_backend = name.clone();
                ctx.persist();
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
        row.add_suffix(&test);
        row.add_suffix(&use_btn);
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
            let mut settings = ctx.settings.borrow_mut();
            if let Some(idx) = settings
                .core
                .extra_backends
                .iter()
                .position(|b| b.name == entry.name)
            {
                settings.core.extra_backends[idx] = entry;
            } else {
                settings.core.extra_backends.push(entry);
            }
            drop(settings);
            ctx.persist();
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
    super::wizard::present(parent, ctx, existing, on_done);
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
    attach_folder_picker(parent, &src);
    let dst = adw::EntryRow::new();
    dst.set_title(match op {
        OperationType::Mount => "Mount point",
        OperationType::Serve => "Listen address",
        OperationType::Copyurl => "Destination fs",
        _ => "Destination",
    });
    dst.set_text(&default_dest(remote, &rclone, op));
    attach_folder_picker(parent, &dst);
    let serve = adw::ComboRow::new();
    serve.set_title("Serve type");
    serve.set_model(Some(&gtk::StringList::new(&OperationType::SERVE_TYPES)));
    serve.set_visible(op == OperationType::Serve);
    if let Some(t) = rclone.get("type").and_then(|x| x.as_str()) {
        if let Some(idx) = OperationType::SERVE_TYPES.iter().position(|s| *s == t) {
            serve.set_selected(idx as u32);
        }
    }
    let dry = adw::SwitchRow::new();
    dry.set_title("Dry run");
    dry.set_active(crate::jobs::is_dry_run(&rclone));

    let flags_group = adw::PreferencesGroup::new();
    flags_group.set_title("Operation flags");
    let flag_rows: Rc<RefCell<Vec<(String, adw::EntryRow, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for flag in crate::flags::static_flags_for(op) {
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
            match build_job_params(
                op,
                &remote,
                &src.text(),
                &dst.text(),
                &serde_json::Value::Object(rclone),
            ) {
                Ok(req) => match start_request(&client, &req) {
                    Ok(id) => {
                        ctx.store
                            .borrow_mut()
                            .push_log(&remote, format!("started {op} {id}"));
                        ctx.refresh_runtime();
                        toast.add_toast(adw::Toast::new(&format!("Started {op} {id}")));
                        on_done();
                        dialog.close();
                    }
                    Err(e) => {
                        let err = adw::AlertDialog::new(Some("Start failed"), Some(&e.to_string()));
                        err.add_response("ok", "OK");
                        err.present(Some(&dialog));
                    }
                },
                Err(e) => {
                    let err = adw::AlertDialog::new(Some("Incomplete configuration"), Some(&e));
                    err.add_response("ok", "OK");
                    err.present(Some(&dialog));
                }
            }
        });
    }

    let page = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    identity.add(&profile_row);
    identity.add(&src);
    identity.add(&dst);
    identity.add(&serve);
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
    let dialog = adw::AlertDialog::new(
        Some("Delete remote"),
        Some(&format!(
            "Delete {name}? Active mounts, serves, and jobs will be stopped."
        )),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("delete", "Delete");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let name = name.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            if let Some(client) = ctx.client() {
                let _ = client.delete_remote(&name);
            }
            ctx.store.borrow_mut().remotes.remove(&name);
            ctx.persist();
            on_done();
        }
    });
    dialog.present(Some(parent));
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
    attach_folder_picker(parent, &src);
    let dst = adw::EntryRow::new();
    dst.set_title("Destination / mount point");
    attach_folder_picker(parent, &dst);
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
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let existing_id = existing.as_ref().map(|q| q.id.clone());
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
                            let dump = ctx
                                .client()
                                .and_then(|c| c.dump_config().ok())
                                .unwrap_or(serde_json::json!({}));
                            match backup::create_backup(
                                &path,
                                &ctx.settings.borrow(),
                                &ctx.store.borrow(),
                                &dump,
                                &export_type,
                                &note_text,
                                None,
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
    let analysis = match backup::analyze_backup(&path) {
        Ok(a) => a,
        Err(e) => {
            toast.add_toast(adw::Toast::new(&e));
            return;
        }
    };
    let dialog = adw::AlertDialog::new(
        Some("Restore backup"),
        Some(&format!(
            "Version {} · {} · remotes: {}\nsettings={} store={} rclone={}",
            analysis.manifest.version,
            analysis.manifest.export_type,
            analysis.manifest.remotes.join(", "),
            analysis.has_settings,
            analysis.has_store,
            analysis.has_rclone_config
        )),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("restore", "Restore");
    dialog.set_response_appearance("restore", adw::ResponseAppearance::Destructive);
    dialog.connect_response(None, move |_, response| {
        if response != "restore" {
            return;
        }
        match backup::restore_backup(&path) {
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
    dialog.set_content_width(480);
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
    if let Some(client) = ctx.client() {
        let fs = if remote == "local" {
            "/".into()
        } else {
            remote_fs(remote, "")
        };
        if let Ok(size) = client.size(&fs, path) {
            let row = adw::ActionRow::new();
            row.set_title("Size");
            row.set_subtitle(&size.to_string());
            list.append(&row);
        }
        if let Ok(hash) = client.hashsum(&fs, path, "MD5") {
            let row = adw::ActionRow::new();
            row.set_title("MD5");
            row.set_subtitle(&hash.to_string());
            list.append(&row);
        }
    }
    dialog.set_child(Some(&list));
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
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&progress);
    box_.append(&scrolled_list(&meta));
    let xfer_label = gtk::Label::new(Some("Transfers"));
    xfer_label.add_css_class("heading");
    xfer_label.set_xalign(0.0);
    box_.append(&xfer_label);
    box_.append(&scrolled_list(&transfers));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&stop);
    actions.append(&reset);
    box_.append(&actions);
    dialog.set_child(Some(&box_));

    let fill = {
        let ctx = ctx.clone();
        let meta = meta.clone();
        let transfers = transfers.clone();
        let progress = progress.clone();
        move || {
            while let Some(child) = meta.first_child() {
                meta.remove(&child);
            }
            while let Some(child) = transfers.first_child() {
                transfers.remove(&child);
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
                ("Error", job.error.clone().unwrap_or_else(|| "—".into())),
            ] {
                let row = adw::ActionRow::new();
                row.set_title(title);
                row.set_subtitle(&value);
                meta.append(&row);
            }
            if let Some(arr) = job.transferring.as_array() {
                if arr.is_empty() {
                    let row = adw::ActionRow::new();
                    row.set_title("No active transfers");
                    transfers.append(&row);
                }
                for item in arr {
                    let row = adw::ActionRow::new();
                    row.set_title(
                        item.get("name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("transfer"),
                    );
                    let pct = item
                        .get("percentage")
                        .or_else(|| item.get("percentageComplete"))
                        .cloned()
                        .unwrap_or(serde_json::json!(0));
                    let size = item.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
                    row.set_subtitle(&format!("{pct}% · {}", crate::rclone::format_bytes(size)));
                    transfers.append(&row);
                }
            }
        }
    };
    fill();
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

pub fn file_viewer(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    remote: &str,
    path: &str,
    name: &str,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(name);
    dialog.set_content_width(720);
    dialog.set_content_height(520);
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
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.append(&info);
    box_.append(&open);
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
            }
        }
    }
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn remote_about(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, remote: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("About {remote}"));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    if let Some(client) = ctx.client() {
        match client.about(&remote_fs(remote, "")) {
            Ok(about) => {
                for (key, value) in about.as_object().cloned().unwrap_or_default() {
                    let row = adw::ActionRow::new();
                    row.set_title(&key);
                    row.set_subtitle(&value.to_string());
                    list.append(&row);
                }
            }
            Err(e) => {
                let row = adw::ActionRow::new();
                row.set_title("Unable to query remote");
                row.set_subtitle(&e.to_string());
                list.append(&row);
            }
        }
    }
    dialog.set_child(Some(&list));
    dialog.present(Some(parent));
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

pub fn item_order(parent: &impl IsA<gtk::Widget>, ctx: AppCtx, on_done: Rc<dyn Fn()>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Remote order and visibility");
    dialog.set_content_width(480);
    dialog.set_content_height(520);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let mut names = ctx.store.borrow().remote_order.clone();
    if names.is_empty() {
        names = ctx
            .snapshot
            .borrow()
            .remotes
            .iter()
            .map(|r| r.name.clone())
            .collect();
    }
    let hidden = ctx.store.borrow().hidden_remotes.clone();
    let rows: Rc<RefCell<Vec<(String, adw::SwitchRow)>>> = Rc::new(RefCell::new(Vec::new()));
    for name in names {
        let row = adw::SwitchRow::new();
        row.set_title(&name);
        row.set_subtitle("Visible in sidebar");
        row.set_active(!hidden.contains(&name));
        list.append(&row);
        rows.borrow_mut().push((name, row));
    }
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        save.connect_clicked(move |_| {
            let mut order = Vec::new();
            let mut hidden = Vec::new();
            for (name, row) in rows.borrow().iter() {
                order.push(name.clone());
                if !row.is_active() {
                    hidden.push(name.clone());
                }
            }
            ctx.store.borrow_mut().remote_order = order;
            ctx.store.borrow_mut().hidden_remotes = hidden;
            ctx.persist();
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

fn present_window_or_dialog(parent: &impl IsA<gtk::Widget>, ctx: &AppCtx, dialog: &adw::Dialog) {
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

fn attach_folder_picker(parent: &impl IsA<gtk::Widget>, row: &adw::EntryRow) {
    let btn = gtk::Button::from_icon_name("folder-open-symbolic");
    btn.set_valign(gtk::Align::Center);
    btn.set_tooltip_text(Some("Browse"));
    let parent = parent.clone();
    {
        let row = row.clone();
        btn.connect_clicked(move |_| {
            let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let dialog = gtk::FileDialog::new();
            let row = row.clone();
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
        });
    }
    row.add_suffix(&btn);
}

fn apply_syntax_highlight(view: &gtk::TextView, name: &str, text: &str) {
    let buffer = view.buffer();
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
    buffer.set_text(text);
    if let Some(lang) = crate::syntax::language_from_name(name) {
        for span in crate::syntax::highlight(text, lang) {
            let start = buffer.iter_at_offset(span.start as i32);
            let end = buffer.iter_at_offset(span.end as i32);
            buffer.apply_tag_by_name(span.kind.tag_name(), &start, &end);
        }
    }
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
    url.set_title("URL / broker / SMTP host");
    let method = adw::EntryRow::new();
    method.set_title("Method / topic / port");
    let token = adw::PasswordEntryRow::new();
    token.set_title("Token / password / API key");
    let extra = adw::EntryRow::new();
    extra.set_title("Chat ID / phone / from / command");
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
            action.config = serde_json::json!({
                "url": url.text().to_string(),
                "broker_url": url.text().to_string(),
                "smtp_server": url.text().to_string(),
                "method": method.text().to_string(),
                "topic": method.text().to_string(),
                "smtp_port": method.text().parse::<u16>().unwrap_or(587),
                "bot_token": token.text().to_string(),
                "password": token.text().to_string(),
                "apikey": token.text().to_string(),
                "chat_id": extra.text().to_string(),
                "phone": extra.text().to_string(),
                "from": extra.text().to_string(),
                "to": extra.text().to_string(),
                "command": extra.text().to_string(),
                "body_template": body.text().to_string(),
            });
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
    dialog.set_content_width(520);
    let binary = ctx.settings.borrow().core.rclone_binary.clone();
    let found = crate::rclone::rclone_exists(&binary);
    let status = gtk::Label::new(Some(&if found {
        format!(
            "rclone was found{}.",
            if binary.is_empty() {
                String::new()
            } else {
                format!(" at {binary}")
            }
        )
    } else {
        "rclone is missing. Install it or pick a custom binary.".into()
    }));
    status.set_wrap(true);
    status.set_xalign(0.0);
    let install = gtk::Button::with_label("Install rclone");
    install.add_css_class("suggested-action");
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
        browse.connect_clicked(move |_| {
            let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let picker = gtk::FileDialog::new();
            let ctx = ctx.clone();
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
        });
    }
    let config_btn = gtk::Button::with_label("Choose rclone.conf…");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        config_btn.connect_clicked(move |_| {
            let Some(win) = parent.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let picker = gtk::FileDialog::new();
            let ctx = ctx.clone();
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
        });
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
    box_.append(&status);
    box_.append(&install);
    box_.append(&browse);
    box_.append(&config_btn);
    box_.append(&restart);
    dialog.set_child(Some(&box_));
    present_window_or_dialog(parent, &ctx, &dialog);
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
