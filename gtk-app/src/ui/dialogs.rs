use super::AppCtx;
use crate::backup;
use crate::operations::OperationType;
use crate::rclone::{remote_fs, validate_cron};
use crate::store::{AlertAction, AlertEvent, AlertEventKind, AlertRule, AlertSeverity, QuickRun};
use adw::prelude::*;
use gtk::gio;
use gtk::prelude::*;
use std::cell::RefCell;
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
    dialog.set_content_height(480);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let ready = ctx.engine_ready();
    let port = ctx.engine.borrow().as_ref().map(|e| e.port).unwrap_or(0);
    let local = adw::ActionRow::new();
    local.set_title("Local rclone RC");
    local.set_subtitle(&format!("127.0.0.1:{port} · ready={ready}"));
    list.append(&local);
    for backend in &ctx.settings.borrow().core.extra_backends {
        let row = adw::ActionRow::new();
        row.set_title(&backend.name);
        row.set_subtitle(&format!("{}:{}", backend.host, backend.port));
        list.append(&row);
    }
    let add = gtk::Button::with_label("Add remote RC backend");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add.connect_clicked(move |_| {
            prompt(
                &parent,
                "Add backend",
                "name|host|port",
                "extra|127.0.0.1|5573",
                {
                    let ctx = ctx.clone();
                    move |text| {
                        let parts: Vec<&str> = text.split('|').collect();
                        if parts.len() >= 3 {
                            if let Ok(port) = parts[2].trim().parse::<u16>() {
                                ctx.settings.borrow_mut().core.extra_backends.push(
                                    crate::settings::BackendEntry {
                                        name: parts[0].trim().into(),
                                        host: parts[1].trim().into(),
                                        port,
                                        user: String::new(),
                                        pass: String::new(),
                                    },
                                );
                                ctx.persist();
                            }
                        }
                    }
                },
            );
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_margin_top(12);
    box_.append(&scrolled_list(&list));
    box_.append(&add);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn alerts(parent: &impl IsA<gtk::Widget>, ctx: AppCtx) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Alerts");
    dialog.set_content_width(720);
    dialog.set_content_height(520);
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
    stack.add_titled(&scrolled_list(&history), Some("history"), "History");

    let rules = gtk::ListBox::new();
    rules.add_css_class("boxed-list");
    for rule in &ctx.store.borrow().alert_rules {
        let row = adw::ActionRow::new();
        row.set_title(&rule.name);
        row.set_subtitle(&format!(
            "min {} · {} actions",
            rule.severity_min.as_str(),
            rule.action_ids.len()
        ));
        rules.append(&row);
    }
    let add_rule = gtk::Button::with_label("Add rule");
    {
        let ctx = ctx.clone();
        let parent = parent.clone();
        add_rule.connect_clicked(move |_| {
            let mut rule = AlertRule::new("New rule".into());
            rule.event_filter = vec![AlertEventKind::Job];
            ctx.store.borrow_mut().alert_rules.push(rule);
            ctx.persist();
            alerts(&parent, ctx.clone());
        });
    }
    let rules_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    rules_box.append(&scrolled_list(&rules));
    rules_box.append(&add_rule);
    stack.add_titled(&rules_box, Some("rules"), "Rules");

    let actions = gtk::ListBox::new();
    actions.add_css_class("boxed-list");
    for action in &ctx.store.borrow().alert_actions {
        let row = adw::ActionRow::new();
        row.set_title(&action.name);
        row.set_subtitle(&action.kind);
        actions.append(&row);
    }
    let add_action = gtk::Button::with_label("Add action");
    {
        let ctx = ctx.clone();
        add_action.connect_clicked(move |_| {
            ctx.store.borrow_mut().alert_actions.push(AlertAction::new(
                "Desktop notification".into(),
                "os_toast".into(),
            ));
            ctx.persist();
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
    dialog.present(Some(parent));
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
    let dst = adw::EntryRow::new();
    dst.set_title("Destination / mount point");
    let cron = adw::EntryRow::new();
    cron.set_title("Cron expression");
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
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

pub fn export_backup(parent: &impl IsA<gtk::Window>, ctx: AppCtx, toast: adw::ToastOverlay) {
    let dialog = gtk::FileDialog::new();
    dialog.set_initial_name(Some("rclone-manager-backup.zip"));
    let dest_default = dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    dialog.save(
        Some(parent),
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
                        "FullBackup",
                        "",
                        None,
                    ) {
                        Ok(_) => toast.add_toast(adw::Toast::new("Backup exported")),
                        Err(e) => toast.add_toast(adw::Toast::new(&e)),
                    }
                }
            }
            let _ = dest_default;
        },
    );
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

pub fn job_detail(parent: &impl IsA<gtk::Widget>, job: &crate::store::JobInfo) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&format!("Job #{}", job.id));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    for (title, value) in [
        ("Operation", job.operation.clone()),
        ("Status", job.status.clone()),
        ("Remote", job.remote.clone()),
        ("Profile", job.profile.clone()),
        ("Origin", job.origin.clone()),
        ("Source", job.src.clone()),
        ("Destination", job.dst.clone()),
        ("Error", job.error.clone().unwrap_or_else(|| "—".into())),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_subtitle(&value);
        list.append(&row);
    }
    dialog.set_child(Some(&list));
    dialog.present(Some(parent));
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
        view.set_editable(false);
        if let Ok(text) = std::fs::read_to_string(path) {
            let shown = if text.len() > 200_000 {
                format!("{}\n\n… truncated …", &text[..200_000])
            } else {
                text
            };
            view.buffer().set_text(&shown);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&view));
        box_.append(&scroll);
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
    for template in &ctx.store.borrow().templates {
        let row = adw::ActionRow::new();
        row.set_title(&template.name);
        row.set_subtitle(&template.description);
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
        });
    }
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.append(&scrolled_list(&list));
    box_.append(&add);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
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

fn scrolled_list(list: &gtk::ListBox) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(list));
    scroll
}
