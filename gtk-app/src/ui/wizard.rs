use super::AppCtx;
use crate::interactive::{
    apply_interactive_response, is_continue_disabled, update_interactive_answer, InteractiveAnswer,
    InteractiveFlowState,
};
use crate::jobs::{
    first_path, flatten_rclone, parse_cli_flags, profile_src_dst, DEST_KEYS, SOURCE_KEYS,
};
use crate::operations::OperationType;
use crate::providers::{parse_providers, Provider, ProviderOption};
use crate::rclone::remote_fs;
use crate::store::{AppConfig, ProfileConfig, RemoteMeta};
use adw::prelude::*;
use gtk::prelude::*;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct WizardState {
    providers: Vec<Provider>,
    fields: HashMap<String, adw::EntryRow>,
    flow: InteractiveFlowState,
    parameters: Value,
}

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
    dialog.set_content_width(680);
    dialog.set_content_height(780);

    let providers = ctx
        .client()
        .and_then(|c| c.providers().ok())
        .map(|v| parse_providers(&v))
        .unwrap_or_default();
    let existing_params = existing.as_ref().and_then(|name| {
        ctx.client()
            .and_then(|c| c.dump_config().ok())
            .and_then(|dump| crate::providers::dump_remote_params(&dump, name))
    });
    let existing_meta = existing
        .as_ref()
        .and_then(|name| ctx.store.borrow().remotes.get(name).cloned());

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
    let advanced_group = adw::PreferencesGroup::new();
    advanced_group.set_title("Advanced options");
    let state = Rc::new(RefCell::new(WizardState {
        providers: providers.clone(),
        fields: HashMap::new(),
        flow: InteractiveFlowState::default(),
        parameters: json!({}),
    }));
    rebuild_fields(
        parent,
        &ctx,
        &fields_group,
        &advanced_group,
        &state,
        providers.first(),
        true,
    );
    if let Some(ref params) = existing_params {
        if let Some(type_name) = crate::providers::dump_provider_type(params) {
            if let Some(idx) = crate::providers::provider_index_by_name(&providers, &type_name) {
                type_row.set_selected(idx as u32);
            }
        }
        apply_dump_to_wizard_fields(&state, params);
    }

    {
        let fields_group = fields_group.clone();
        let advanced_group = advanced_group.clone();
        let state = state.clone();
        let providers = providers.clone();
        let parent = parent.clone();
        let ctx = ctx.clone();
        let existing_params = existing_params.clone();
        type_row.connect_selected_notify(move |row| {
            let provider = providers.get(row.selected() as usize);
            rebuild_fields(
                &parent,
                &ctx,
                &fields_group,
                &advanced_group,
                &state,
                provider,
                true,
            );
            if let Some(params) = existing_params.as_ref() {
                let matches_type = crate::providers::dump_provider_type(params).is_some_and(|t| {
                    provider.is_some_and(|p| {
                        p.name.eq_ignore_ascii_case(&t) || p.prefix.eq_ignore_ascii_case(&t)
                    })
                });
                if matches_type {
                    apply_dump_to_wizard_fields(&state, params);
                }
            }
        });
    }

    let mount = adw::EntryRow::new();
    mount.set_title("Mount point");
    super::dialogs::attach_path_picker(
        &ctx,
        &mount,
        crate::picker::FilePickerConfig::local_folders(),
    );
    let src = adw::EntryRow::new();
    src.set_title("Default source path");
    super::dialogs::attach_path_picker(&ctx, &src, crate::picker::FilePickerConfig::folders());
    let dst = adw::EntryRow::new();
    dst.set_title("Default destination path");
    super::dialogs::attach_path_picker(&ctx, &dst, crate::picker::FilePickerConfig::folders());
    let serve_types = ctx.serve_types();
    let mount_types = ctx.mount_types();
    let serve = adw::ComboRow::new();
    serve.set_title("Default serve type");
    serve.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&serve_types),
    )));
    let mount_type = adw::ComboRow::new();
    mount_type.set_title("Default mount type");
    mount_type.set_model(Some(&gtk::StringList::new(
        &crate::operations::combo_names(&mount_types),
    )));
    let cron = adw::EntryRow::new();
    cron.set_title("Default cron");
    let tray = adw::SwitchRow::new();
    tray.set_title("Show in tray");
    tray.set_active(true);
    let autostart = adw::SwitchRow::new();
    autostart.set_title("Auto-start mount / jobs");
    let cli = adw::EntryRow::new();
    cli.set_title("Import CLI flags (--transfers 8 --vfs-cache-mode full)");
    let obscure_in = adw::EntryRow::new();
    obscure_in.set_title("Obscure a secret");
    let obscure_btn = gtk::Button::with_label("Obscure");
    {
        let ctx = ctx.clone();
        let obscure_in = obscure_in.clone();
        obscure_btn.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                if let Ok(out) = client.obscure(&obscure_in.text()) {
                    obscure_in.set_text(&out);
                }
            }
        });
    }
    let op_flags: Rc<RefCell<Vec<(OperationType, adw::SwitchRow, adw::EntryRow, adw::EntryRow)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let ops_group = adw::PreferencesGroup::new();
    ops_group.set_title("Per-operation profiles");
    for op in OperationType::ALL {
        let enable = adw::SwitchRow::new();
        enable.set_title(&format!("Configure {}", op.api_label()));
        enable.set_active(matches!(
            op,
            OperationType::Mount | OperationType::Sync | OperationType::Serve
        ));
        let osrc = adw::EntryRow::new();
        osrc.set_title(&format!("{} source", op.as_str()));
        let odst = adw::EntryRow::new();
        odst.set_title(&format!("{} destination / mount / addr", op.as_str()));
        ops_group.add(&enable);
        ops_group.add(&osrc);
        ops_group.add(&odst);
        op_flags.borrow_mut().push((op, enable, osrc, odst));
    }
    if let Some(ref meta) = existing_meta {
        apply_existing_meta(
            meta,
            &mount,
            &src,
            &dst,
            &serve,
            &serve_types,
            &mount_type,
            &mount_types,
            &cron,
            &tray,
            &autostart,
            &op_flags,
        );
    }

    let question_title = gtk::Label::new(Some("Interactive configuration"));
    question_title.add_css_class("title-4");
    question_title.set_xalign(0.0);
    let question_help = gtk::Label::new(Some(
        "Authorize the provider or answer rclone's configuration questions.",
    ));
    question_help.set_wrap(true);
    question_help.set_xalign(0.0);
    question_help.add_css_class("dim-label");
    let question_error = gtk::Label::new(None);
    question_error.add_css_class("error");
    question_error.set_wrap(true);
    question_error.set_xalign(0.0);
    let answer_row = adw::EntryRow::new();
    answer_row.set_title("Answer");
    let answer_switch = adw::SwitchRow::new();
    answer_switch.set_title("Yes / enabled");
    let example_row = adw::ComboRow::new();
    example_row.set_title("Choose an option");
    let oauth_status = gtk::Label::new(Some(""));
    oauth_status.add_css_class("dim-label");
    oauth_status.set_xalign(0.0);

    let nav = adw::ViewStack::new();
    let setup = adw::PreferencesPage::new();
    let identity = adw::PreferencesGroup::new();
    identity.set_title("Identity");
    identity.add(&name);
    identity.add(&type_row);
    setup.add(&identity);
    setup.add(&fields_group);
    setup.add(&advanced_group);
    nav.add_titled(&setup, Some("setup"), "Provider");

    let interactive_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
    interactive_box.set_margin_top(16);
    interactive_box.set_margin_start(16);
    interactive_box.set_margin_end(16);
    interactive_box.append(&question_title);
    interactive_box.append(&question_help);
    interactive_box.append(&question_error);
    interactive_box.append(&example_row);
    interactive_box.append(&answer_row);
    interactive_box.append(&answer_switch);
    interactive_box.append(&oauth_status);
    let cancel_oauth = gtk::Button::with_label("Cancel OAuth");
    {
        let ctx = ctx.clone();
        let oauth_status = oauth_status.clone();
        cancel_oauth.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                match client.oauth_stop() {
                    Ok(_) => oauth_status.set_text("OAuth cancelled"),
                    Err(e) => oauth_status.set_text(&e.to_string()),
                }
            }
        });
    }
    interactive_box.append(&cancel_oauth);
    nav.add_titled(&interactive_box, Some("interactive"), "Authorize");

    let profiles = adw::PreferencesPage::new();
    let pgroup = adw::PreferencesGroup::new();
    pgroup.set_title("Default profiles");
    pgroup.add(&mount);
    pgroup.add(&src);
    pgroup.add(&dst);
    pgroup.add(&serve);
    pgroup.add(&mount_type);
    pgroup.add(&cron);
    pgroup.add(&tray);
    pgroup.add(&autostart);
    pgroup.add(&cli);
    pgroup.add(&obscure_in);
    profiles.add(&pgroup);
    profiles.add(&ops_group);
    nav.add_titled(&profiles, Some("profiles"), "Profiles");

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&nav));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);

    let continue_btn = gtk::Button::with_label("Continue / Authorize");
    continue_btn.add_css_class("suggested-action");
    let save = gtk::Button::with_label("Save remote");
    save.add_css_class("pill");

    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let name = name.clone();
        let type_row = type_row.clone();
        let state = state.clone();
        let question_title = question_title.clone();
        let question_help = question_help.clone();
        let question_error = question_error.clone();
        let answer_row = answer_row.clone();
        let answer_switch = answer_switch.clone();
        let example_row = example_row.clone();
        let oauth_status = oauth_status.clone();
        let nav = nav.clone();
        continue_btn.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            if remote_name.is_empty() {
                return;
            }
            let r#type = provider_type(&state.borrow().providers, type_row.selected());
            let params = collect_params(&state);
            state.borrow_mut().parameters = params.clone();
            let Some(client) = ctx.client() else {
                return;
            };
            let result = {
                let flow = state.borrow().flow.clone();
                if !flow.is_active {
                    client.create_remote_interactive(&remote_name, &r#type, params, None)
                } else {
                    if is_continue_disabled(&flow) {
                        return;
                    }
                    let option_type = flow
                        .question
                        .as_ref()
                        .and_then(|q| q.option.as_ref())
                        .map(|o| o.type_name.as_str())
                        .unwrap_or("string");
                    let answer = current_answer(&flow, &answer_row, &answer_switch, &example_row);
                    let token = flow
                        .question
                        .as_ref()
                        .map(|q| q.state.clone())
                        .unwrap_or_default();
                    client.continue_create_remote(
                        &remote_name,
                        &token,
                        answer.as_rc_result(option_type),
                        params,
                        None,
                    )
                }
            };
            match result {
                Ok(value) => {
                    let next = apply_interactive_response(&value);
                    if let Ok((_, Some(url))) = client.oauth_status() {
                        let _ = open::that(&url);
                        oauth_status.set_text(&format!("Opened authorization URL: {url}"));
                    }
                    if !next.is_active {
                        question_title.set_text("Authorization complete");
                        question_help.set_text(
                            "rclone finished the interactive flow. Review profiles and save.",
                        );
                        question_error.set_text("");
                        nav.set_visible_child_name("profiles");
                    } else {
                        apply_question_widgets(
                            &next,
                            &question_title,
                            &question_help,
                            &question_error,
                            &answer_row,
                            &answer_switch,
                            &example_row,
                        );
                        nav.set_visible_child_name("interactive");
                    }
                    state.borrow_mut().flow = next;
                }
                Err(e) => {
                    let err =
                        adw::AlertDialog::new(Some("Configuration failed"), Some(&e.to_string()));
                    err.add_response("ok", "OK");
                    err.present(Some(&dialog));
                }
            }
        });
    }

    {
        let ctx = ctx.clone();
        let dialog = dialog.clone();
        let existing = existing.clone();
        let state = state.clone();
        let name = name.clone();
        let type_row = type_row.clone();
        let mount = mount.clone();
        let src = src.clone();
        let dst = dst.clone();
        let serve = serve.clone();
        let cron = cron.clone();
        let tray = tray.clone();
        let autostart = autostart.clone();
        let op_flags = op_flags.clone();
        let cli = cli.clone();
        save.connect_clicked(move |_| {
            let remote_name = name.text().to_string();
            if remote_name.is_empty() {
                return;
            }
            let r#type = provider_type(&state.borrow().providers, type_row.selected());
            let mut params = collect_params(&state);
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
                if existing.is_none() {
                    let vendor = params.get("vendor").and_then(|v| v.as_str());
                    let presets =
                        crate::presets::resolve_presets(&r#type, vendor, std::env::consts::OS);
                    crate::presets::merge_remote_params(&mut params, &presets);
                }
                let result = if existing.is_some() || state.borrow().flow.is_active {
                    client.update_remote(&remote_name, params)
                } else {
                    client.create_remote(&remote_name, &r#type, params)
                };
                match result {
                    Ok(_) => {
                        persist_meta(
                            &ctx,
                            &remote_name,
                            &r#type,
                            &mount.text(),
                            &src.text(),
                            &dst.text(),
                            crate::operations::selected_or(
                                &serve_types,
                                serve.selected(),
                                "webdav",
                            ),
                            crate::operations::selected_or(
                                &mount_types,
                                mount_type.selected(),
                                "mount",
                            ),
                            &cron.text(),
                            tray.is_active(),
                            autostart.is_active(),
                            &cli.text(),
                            &op_flags.borrow(),
                        );
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

    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(8);
    box_.set_margin_bottom(12);
    box_.append(&switcher);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&nav));
    box_.append(&scroll);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_end(12);
    buttons.append(&continue_btn);
    buttons.append(&save);
    box_.append(&buttons);
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
    parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    basic: &adw::PreferencesGroup,
    advanced: &adw::PreferencesGroup,
    state: &Rc<RefCell<WizardState>>,
    provider: Option<&Provider>,
    include_advanced: bool,
) {
    for row in state.borrow().fields.values() {
        basic.remove(row);
        advanced.remove(row);
    }
    state.borrow_mut().fields.clear();
    let Some(provider) = provider else {
        return;
    };
    for option in provider.basic_options() {
        let row = option_row(parent, ctx, option);
        basic.add(&row);
        state.borrow_mut().fields.insert(option.name.clone(), row);
    }
    if include_advanced {
        for option in provider.advanced_options() {
            let row = option_row(parent, ctx, option);
            advanced.add(&row);
            state.borrow_mut().fields.insert(option.name.clone(), row);
        }
    }
}

fn option_row(
    _parent: &impl IsA<gtk::Widget>,
    ctx: &AppCtx,
    option: &ProviderOption,
) -> adw::EntryRow {
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
    let restrict = ctx.settings.borrow().general.restrict;
    if option.is_password || (restrict && crate::restrict::is_sensitive_key(&option.name)) {
        if let Some(child) = row.first_child() {
            if let Ok(editable) = child.downcast::<gtk::Text>() {
                editable.set_visibility(false);
            }
        }
        if restrict {
            row.set_text("");
            return row;
        }
    }
    if !option.default_str.is_empty() {
        row.set_text(&option.default_str);
    } else if let Some((example, _)) = option.examples.first() {
        row.set_text(example);
    }
    if crate::media::is_path_field(&option.name, &option.help) {
        super::dialogs::attach_path_picker(ctx, &row, crate::picker::FilePickerConfig::folders());
        apply_path_usage(&row);
        row.connect_changed(|row| apply_path_usage(row));
    }
    row
}

fn apply_path_usage(row: &adw::EntryRow) {
    let text = row.text().to_string();
    if let Some(usage) = crate::media::local_path_usage(&text) {
        row.set_tooltip_text(Some(&format!("{usage} — {}", text)));
    }
}

fn apply_dump_to_wizard_fields(state: &Rc<RefCell<WizardState>>, params: &Value) {
    for (name, row) in state.borrow().fields.iter() {
        if let Some(text) = crate::providers::dump_field_text(params, name) {
            row.set_text(&text);
        }
    }
}

fn apply_existing_meta(
    meta: &RemoteMeta,
    mount: &adw::EntryRow,
    src: &adw::EntryRow,
    dst: &adw::EntryRow,
    serve: &adw::ComboRow,
    serve_types: &[String],
    mount_type: &adw::ComboRow,
    mount_types: &[String],
    cron: &adw::EntryRow,
    tray: &adw::SwitchRow,
    autostart: &adw::SwitchRow,
    op_flags: &Rc<RefCell<Vec<(OperationType, adw::SwitchRow, adw::EntryRow, adw::EntryRow)>>>,
) {
    tray.set_active(meta.show_on_tray);
    if let Some(dest) = profile_src_dst(meta, OperationType::Mount).1 {
        if !dest.is_empty() {
            mount.set_text(&dest);
        }
    }
    for op in [
        OperationType::Sync,
        OperationType::Copy,
        OperationType::Move,
        OperationType::Bisync,
    ] {
        let (psrc, pdst) = profile_src_dst(meta, op);
        if src.text().is_empty() {
            if let Some(value) = psrc {
                src.set_text(&value);
            }
        }
        if dst.text().is_empty() {
            if let Some(value) = pdst {
                dst.set_text(&value);
            }
        }
        if !src.text().is_empty() && !dst.text().is_empty() {
            break;
        }
    }
    if let Some(profile) = meta
        .get_profile(OperationType::Mount, "default")
        .or_else(|| meta.get_profile(OperationType::Sync, "default"))
        .or_else(|| meta.get_profile(OperationType::Serve, "default"))
    {
        if profile.app.auto_start {
            autostart.set_active(true);
        }
        if cron.text().is_empty() && !profile.app.cron_expression.is_empty() {
            cron.set_text(&profile.app.cron_expression);
        }
    }
    if let Some(profile) = meta.get_profile(OperationType::Serve, "default") {
        let rclone = flatten_rclone(&profile.rclone);
        if let Some(t) = rclone.get("type").and_then(|v| v.as_str()) {
            if let Some(idx) = serve_types.iter().position(|s| s == t) {
                serve.set_selected(idx as u32);
            }
        }
    }
    if let Some(profile) = meta.get_profile(OperationType::Mount, "default") {
        let rclone = flatten_rclone(&profile.rclone);
        if let Some(t) = rclone.get("mountType").and_then(|v| v.as_str()) {
            if let Some(idx) = mount_types.iter().position(|s| s == t) {
                mount_type.set_selected(idx as u32);
            }
        }
    }
    for (op, enable, osrc, odst) in op_flags.borrow().iter() {
        if let Some(profile) = meta.get_profile(*op, "default") {
            enable.set_active(true);
            let rclone = flatten_rclone(&profile.rclone);
            if let Some(s) = first_path(&rclone, SOURCE_KEYS) {
                osrc.set_text(&s);
            }
            if let Some(d) = first_path(&rclone, DEST_KEYS) {
                odst.set_text(&d);
            }
        }
    }
}

fn collect_params(state: &Rc<RefCell<WizardState>>) -> Value {
    let mut map = serde_json::Map::new();
    for (key, row) in state.borrow().fields.iter() {
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

fn current_answer(
    flow: &InteractiveFlowState,
    answer_row: &adw::EntryRow,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
) -> InteractiveAnswer {
    let option = flow.question.as_ref().and_then(|q| q.option.as_ref());
    if let Some(option) = option {
        if option.type_name == "bool" {
            return InteractiveAnswer::Bool(answer_switch.is_active());
        }
        if !option.examples.is_empty() && option.exclusive {
            if let Some((value, _)) = option.examples.get(example_row.selected() as usize) {
                return InteractiveAnswer::Text(value.clone());
            }
        }
    }
    let text = answer_row.text().to_string();
    if text.is_empty() {
        update_interactive_answer(flow.clone(), InteractiveAnswer::Empty).answer
    } else {
        InteractiveAnswer::Text(text)
    }
}

fn apply_question_widgets(
    flow: &InteractiveFlowState,
    title: &gtk::Label,
    help: &gtk::Label,
    error: &gtk::Label,
    answer_row: &adw::EntryRow,
    answer_switch: &adw::SwitchRow,
    example_row: &adw::ComboRow,
) {
    let Some(step) = &flow.question else {
        return;
    };
    if let Some(option) = &step.option {
        title.set_text(&option.name);
        help.set_text(&option.help);
        error.set_text(step.error.as_deref().unwrap_or(""));
        answer_switch.set_visible(option.type_name == "bool");
        answer_row.set_visible(option.type_name != "bool");
        if option.type_name == "bool" {
            answer_switch.set_active(matches!(flow.answer, InteractiveAnswer::Bool(true)));
        } else {
            answer_row.set_text(&flow.answer.as_string());
            if option.is_password {
                if let Some(child) = answer_row.first_child() {
                    if let Ok(editable) = child.downcast::<gtk::Text>() {
                        editable.set_visibility(false);
                    }
                }
            }
        }
        if option.examples.is_empty() {
            example_row.set_visible(false);
        } else {
            example_row.set_visible(true);
            let labels: Vec<String> = option
                .examples
                .iter()
                .map(|(v, h)| {
                    if h.is_empty() {
                        v.clone()
                    } else {
                        format!("{v} — {h}")
                    }
                })
                .collect();
            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
            example_row.set_model(Some(&gtk::StringList::new(&refs)));
        }
    } else {
        title.set_text("Continue authorization");
        help.set_text(step.error.as_deref().unwrap_or("Complete the next step."));
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_meta(
    ctx: &AppCtx,
    remote_name: &str,
    remote_type: &str,
    mount: &str,
    src: &str,
    dst: &str,
    serve_type: &str,
    mount_type: &str,
    cron: &str,
    tray: bool,
    autostart: bool,
    cli: &str,
    op_flags: &[(OperationType, adw::SwitchRow, adw::EntryRow, adw::EntryRow)],
) {
    let mut meta = ctx
        .store
        .borrow()
        .remotes
        .get(remote_name)
        .cloned()
        .unwrap_or_default();
    meta.show_on_tray = tray;
    let extra = parse_cli_flags(cli);
    for (op, enable, osrc, odst) in op_flags {
        if !enable.is_active() {
            continue;
        }
        let mut source = osrc.text().to_string();
        let mut dest = odst.text().to_string();
        if source.is_empty() {
            source = src.to_string();
        }
        if dest.is_empty() {
            dest = if *op == OperationType::Mount {
                mount.to_string()
            } else {
                dst.to_string()
            };
        }
        if source.is_empty() {
            source = remote_fs(remote_name, "");
        }
        let mut rclone = json!({
            "srcFs": source,
            "dstFs": dest,
            "mountPoint": if *op == OperationType::Mount { dest.clone() } else { mount.to_string() },
            "fs": remote_fs(remote_name, ""),
            "path1": source,
            "path2": dest,
            "type": serve_type,
            "mountType": mount_type
        });
        if let Some(obj) = rclone.as_object_mut() {
            obj.extend(extra.clone());
        }
        let profile = ProfileConfig {
            name: "default".into(),
            app: AppConfig {
                auto_start: autostart
                    && matches!(
                        *op,
                        OperationType::Mount | OperationType::Sync | OperationType::Serve
                    ),
                cron_enabled: !cron.is_empty(),
                cron_expression: cron.to_string(),
                ..AppConfig::default()
            },
            rclone,
        };
        meta.profiles
            .entry(op.as_str().into())
            .or_default()
            .insert("default".into(), profile);
    }
    crate::presets::apply_to_remote_meta(
        &mut meta,
        &crate::presets::resolve_presets(remote_type, None, std::env::consts::OS),
    );
    ctx.store
        .borrow_mut()
        .remotes
        .insert(remote_name.to_string(), meta);
    ctx.persist();
}
