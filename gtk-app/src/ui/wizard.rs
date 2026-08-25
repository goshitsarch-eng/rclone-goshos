use super::AppCtx;
use crate::operations::OperationType;
use crate::providers::{parse_config_step, parse_providers, Provider, ProviderOption};
use crate::rclone::remote_fs;
use crate::store::{AppConfig, ProfileConfig, RemoteMeta};
use adw::prelude::*;
use gtk::prelude::*;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub fn present(
    parent: &impl IsA<gtk::Widget>,
    ctx: AppCtx,
    existing: Option<String>,
    on_done: Rc<dyn Fn()>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(if existing.is_some() {
        "Remote Configuration"
    } else {
        "Add Remote"
    });
    dialog.set_content_width(640);
    dialog.set_content_height(720);

    let providers = ctx
        .client()
        .and_then(|c| c.providers().ok())
        .map(|v| parse_providers(&v))
        .unwrap_or_default();

    let name = adw::EntryRow::new();
    name.set_title("Remote name");
    if let Some(existing) = &existing {
        name.set_text(existing);
        name.set_sensitive(false);
    }

    let type_row = adw::ComboRow::new();
    type_row.set_title("Provider");
    let labels: Vec<String> = if providers.is_empty() {
        [
            "drive", "s3", "dropbox", "onedrive", "sftp", "webdav", "local", "crypt",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    } else {
        providers
            .iter()
            .map(|p| format!("{} — {}", p.name, p.description))
            .collect()
    };
    let label_refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    type_row.set_model(Some(&gtk::StringList::new(&label_refs)));

    let fields_group = adw::PreferencesGroup::new();
    fields_group.set_title("Provider options");
    let fields: Rc<RefCell<HashMap<String, adw::EntryRow>>> = Rc::new(RefCell::new(HashMap::new()));
    rebuild_fields(&fields_group, &fields, providers.first());

    {
        let fields_group = fields_group.clone();
        let fields = fields.clone();
        let providers = providers.clone();
        type_row.connect_selected_notify(move |row| {
            let provider = providers.get(row.selected() as usize);
            rebuild_fields(&fields_group, &fields, provider);
        });
    }

    let mount = adw::EntryRow::new();
    mount.set_title("Mount point");
    let src = adw::EntryRow::new();
    src.set_title("Default source path");
    let dst = adw::EntryRow::new();
    dst.set_title("Default destination path");
    let serve = adw::ComboRow::new();
    serve.set_title("Default serve type");
    serve.set_model(Some(&gtk::StringList::new(&OperationType::SERVE_TYPES)));
    let cron = adw::EntryRow::new();
    cron.set_title("Default cron");
    let tray = adw::SwitchRow::new();
    tray.set_title("Show in tray");
    tray.set_active(true);
    let oauth = gtk::Button::with_label("Authorize (OAuth)");
    oauth.add_css_class("pill");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let name = name.clone();
        let type_row = type_row.clone();
        let providers = providers.clone();
        let fields = fields.clone();
        oauth.connect_clicked(move |_| {
            run_oauth(
                &dialog,
                &ctx,
                &name.text(),
                provider_type(&providers, type_row.selected()),
                collect_params(&fields),
            );
        });
    }

    let save = gtk::Button::with_label("Save remote");
    save.add_css_class("suggested-action");
    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let existing = existing.clone();
        let providers = providers.clone();
        let fields = fields.clone();
        let name = name.clone();
        let type_row = type_row.clone();
        let mount = mount.clone();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let cron = cron.clone();
        let tray = tray.clone();
        save.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            if remote_name.is_empty() {
                return;
            }
            let r#type = provider_type(&providers, type_row.selected());
            let mut params = collect_params(&fields);
            if let Some(client) = ctx.client() {
                for (key, value) in params
                    .clone()
                    .as_object()
                    .unwrap_or(&serde_json::Map::new())
                {
                    if let Some(s) = value.as_str() {
                        if looks_secret(key) && !s.is_empty() {
                            if let Ok(obscured) = client.obscure(s) {
                                params[key] = json!(obscured);
                            }
                        }
                    }
                }
                let result = if existing.is_some() {
                    client.update_remote(&remote_name, params)
                } else {
                    client.create_remote(&remote_name, &r#type, params)
                };
                match result {
                    Ok(_) => {
                        let mut meta = RemoteMeta {
                            show_on_tray: tray.is_active(),
                            ..RemoteMeta::default()
                        };
                        let mut profile = ProfileConfig {
                            name: "default".into(),
                            app: AppConfig {
                                cron_enabled: !cron.text().is_empty(),
                                cron_expression: cron.text().to_string(),
                                ..AppConfig::default()
                            },
                            rclone: json!({
                                "srcFs": src.text().to_string(),
                                "dstFs": dst.text().to_string(),
                                "mountPoint": mount.text().to_string(),
                                "fs": remote_fs(&remote_name, ""),
                                "type": OperationType::SERVE_TYPES
                                    .get(serve.selected() as usize)
                                    .unwrap_or(&"webdav")
                            }),
                        };
                        if profile
                            .rclone
                            .get("srcFs")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .is_empty()
                        {
                            profile.rclone["srcFs"] = json!(remote_fs(&remote_name, ""));
                        }
                        meta.profiles
                            .entry("mount".into())
                            .or_default()
                            .insert("default".into(), profile.clone());
                        meta.profiles
                            .entry("sync".into())
                            .or_default()
                            .insert("default".into(), profile);
                        ctx.store.borrow_mut().remotes.insert(remote_name, meta);
                        ctx.persist();
                        on_done();
                        dialog.close();
                    }
                    Err(e) => {
                        let err = adw::AlertDialog::new(
                            Some("Could not save remote"),
                            Some(&e.to_string()),
                        );
                        err.add_response("ok", "OK");
                        err.present(Some(&dialog));
                    }
                }
            }
        });
    }

    let page = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    identity.set_title("Identity");
    identity.add(&name);
    identity.add(&type_row);
    page.add(&identity);
    page.add(&fields_group);
    let profiles = adw::PreferencesGroup::new();
    profiles.set_title("Default profiles");
    profiles.add(&mount);
    profiles.add(&src);
    profiles.add(&dst);
    profiles.add(&serve);
    profiles.add(&cron);
    profiles.add(&tray);
    page.add(&profiles);

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(12);
    box_.set_margin_bottom(12);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&page));
    box_.append(&scroll);
    box_.append(&oauth);
    box_.append(&save);
    dialog.set_child(Some(&box_));
    dialog.present(Some(parent));
}

fn provider_type(providers: &[Provider], index: u32) -> String {
    providers
        .get(index as usize)
        .map(|p| p.prefix.clone())
        .unwrap_or_else(|| "drive".into())
}

fn rebuild_fields(
    group: &adw::PreferencesGroup,
    fields: &Rc<RefCell<HashMap<String, adw::EntryRow>>>,
    provider: Option<&Provider>,
) {
    for row in fields.borrow().values() {
        group.remove(row);
    }
    fields.borrow_mut().clear();
    let Some(provider) = provider else {
        return;
    };
    for option in provider
        .basic_options()
        .chain(provider.advanced_options().take(8))
    {
        let row = option_row(option);
        group.add(&row);
        fields.borrow_mut().insert(option.name.clone(), row);
    }
}

fn option_row(option: &ProviderOption) -> adw::EntryRow {
    let row = adw::EntryRow::new();
    let title = if option.required {
        format!("{} *", option.name)
    } else if option.advanced {
        format!("{} (advanced)", option.name)
    } else {
        option.name.clone()
    };
    row.set_title(&title);
    if !option.help.is_empty() {
        row.set_tooltip_text(Some(&option.help));
    }
    if option.is_password {
        if let Some(child) = row.first_child() {
            if let Ok(editable) = child.downcast::<gtk::Text>() {
                editable.set_visibility(false);
            }
        }
    }
    if let Some((example, _)) = option.examples.first() {
        row.set_text(example);
    }
    row
}

fn collect_params(fields: &Rc<RefCell<HashMap<String, adw::EntryRow>>>) -> Value {
    let mut map = serde_json::Map::new();
    for (key, row) in fields.borrow().iter() {
        let text = row.text().to_string();
        if !text.is_empty() {
            map.insert(key.clone(), json!(text));
        }
    }
    Value::Object(map)
}

fn looks_secret(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("pass") || key.contains("secret") || key.contains("token") || key.contains("key")
}

fn run_oauth(
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    name: &str,
    r#type: String,
    parameters: Value,
) {
    let Some(client) = ctx.client() else {
        return;
    };
    match client.create_remote_interactive(name, &r#type, parameters, None, None) {
        Ok(value) => {
            let step = parse_config_step(&value);
            if let Ok((_, Some(url))) = client.oauth_status() {
                let _ = open::that(&url);
            }
            let message = step
                .error
                .or_else(|| step.option.map(|o| o.help))
                .unwrap_or_else(|| {
                    "Authorization started. Complete it in the browser, then save the remote."
                        .into()
                });
            let info = adw::AlertDialog::new(Some("OAuth"), Some(&message));
            info.add_response("ok", "OK");
            info.present(Some(parent));
        }
        Err(e) => {
            let err = adw::AlertDialog::new(Some("OAuth failed"), Some(&e.to_string()));
            err.add_response("ok", "OK");
            err.present(Some(parent));
        }
    }
}
