use super::AppCtx;
use crate::rclone::remote_fs;
use crate::vfs::{
    filter_vfs_names, is_indexed_vfs, parse_vfs_list, parse_vfs_queue, parse_vfs_stats,
    queue_item_status, QueueStatus, VfsQueueItem, DELAY_EXPIRY, DELAY_SLIDER_DEFAULT,
    PRIORITY_EXPIRY,
};
use adw::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub fn vfs_panel(ctx: AppCtx, remote: &str, toast: adw::ToastOverlay) -> gtk::Widget {
    let remote = remote.to_string();
    let selected = Rc::new(RefCell::new(remote_fs(&remote, "")));
    let known_names = Rc::new(RefCell::new(Vec::<String>::new()));
    let delay_id = Rc::new(RefCell::new(None::<String>));
    let suppressing = Rc::new(Cell::new(false));
    let refresh_slot: Rc<RefCell<Rc<dyn Fn()>>> = Rc::new(RefCell::new(Rc::new(|| {})));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let heading = gtk::Label::new(Some(&ctx.t_or("shared.vfsControl.title", "VFS Control")));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    root.append(&heading);

    let empty = gtk::Label::new(None);
    empty.add_css_class("dim-label");
    empty.set_wrap(true);
    empty.set_xalign(0.0);
    root.append(&empty);

    let indexed = gtk::Box::new(gtk::Orientation::Vertical, 4);
    indexed.set_visible(false);
    let indexed_title = gtk::Label::new(Some(&ctx.t_or(
        crate::vfs::vfs_indexed_banner_title_key(),
        "VFS Controls Unavailable",
    )));
    indexed_title.add_css_class("heading");
    indexed_title.set_xalign(0.0);
    let indexed_label = gtk::Label::new(None);
    indexed_label.add_css_class("warning");
    indexed_label.set_wrap(true);
    indexed_label.set_xalign(0.0);
    let indexed_link =
        gtk::LinkButton::with_label("https://github.com/rclone/rclone/issues/9120", "#9120");
    indexed_link.set_halign(gtk::Align::Start);
    indexed.append(&indexed_title);
    indexed.append(&indexed_label);
    indexed.append(&indexed_link);
    root.append(&indexed);

    let combo = adw::ComboRow::new();
    combo.set_title(&ctx.t_or("shared.vfsControl.vfsInstance", "VFS Instance"));
    combo.set_visible(false);
    let combo_group = adw::PreferencesGroup::new();
    combo_group.add(&combo);
    root.append(&combo_group);

    let stats_label = gtk::Label::new(Some(&ctx.t_or("shared.vfsControl.stats.metadata", "Stats")));
    stats_label.add_css_class("heading");
    stats_label.set_xalign(0.0);
    let stats = gtk::ListBox::new();
    stats.add_css_class("boxed-list");
    root.append(&stats_label);
    root.append(&stats);

    let queue_label = gtk::Label::new(Some(
        &ctx.t_or("shared.vfsControl.queue.title", "Upload Queue"),
    ));
    queue_label.add_css_class("heading");
    queue_label.set_xalign(0.0);
    let queue_info = gtk::Label::new(None);
    queue_info.add_css_class("dim-label");
    queue_info.set_xalign(0.0);
    let queue_list = gtk::ListBox::new();
    queue_list.add_css_class("boxed-list");
    root.append(&queue_label);
    root.append(&queue_info);
    root.append(&queue_list);

    let delay_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    delay_box.set_visible(false);
    let delay_label = gtk::Label::new(Some(&ctx.t_or(
        "shared.vfsControl.queue.slider.title",
        "Set custom delay time",
    )));
    delay_label.set_xalign(0.0);
    let delay_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 10.0, 86_400.0, 10.0);
    delay_scale.set_value(DELAY_SLIDER_DEFAULT);
    delay_scale.set_hexpand(true);
    delay_scale.set_draw_value(true);
    delay_scale.set_value_pos(gtk::PositionType::Right);
    let delay_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let delay_cancel =
        gtk::Button::with_label(&ctx.t_or("shared.vfsControl.queue.slider.cancel", "Cancel"));
    let delay_apply =
        gtk::Button::with_label(&ctx.t_or("shared.vfsControl.queue.slider.apply", "Apply Delay"));
    delay_apply.add_css_class("suggested-action");
    delay_actions.append(&delay_cancel);
    delay_actions.append(&delay_apply);
    delay_box.append(&delay_label);
    delay_box.append(&delay_scale);
    delay_box.append(&delay_actions);
    root.append(&delay_box);

    let cache_label = gtk::Label::new(Some(
        &ctx.t_or("shared.vfsControl.cacheInfo.title", "Cache Information"),
    ));
    cache_label.add_css_class("heading");
    cache_label.set_xalign(0.0);
    let cache = gtk::ListBox::new();
    cache.add_css_class("boxed-list");
    root.append(&cache_label);
    root.append(&cache);

    let advanced = adw::ExpanderRow::new();
    advanced.set_title(&ctx.t_or(
        "shared.vfsControl.advancedConfig.title",
        "Advanced Configuration",
    ));
    let advanced_search = gtk::SearchEntry::new();
    advanced_search.set_placeholder_text(Some(&ctx.t_or(
        "shared.vfsControl.advancedConfig.search",
        "Search VFS options",
    )));
    advanced_search.set_hexpand(true);
    let search_row = adw::ActionRow::new();
    search_row.set_title(&ctx.t_or(
        "shared.vfsControl.advancedConfig.search",
        "Search VFS options",
    ));
    search_row.add_suffix(&advanced_search);
    let advanced_list = gtk::ListBox::new();
    advanced_list.add_css_class("boxed-list");
    advanced.add_row(&search_row);
    advanced.add_row(&advanced_list);
    let advanced_group = adw::PreferencesGroup::new();
    advanced_group.add(&advanced);
    root.append(&advanced_group);

    let poll = adw::EntryRow::new();
    poll.set_title(&ctx.t_or("shared.vfsControl.actions.pollInterval", "Poll Interval"));
    poll.set_text("1m");
    let apply_poll = gtk::Button::with_label(&ctx.t_or("common.apply", "Set interval"));
    apply_poll.set_valign(gtk::Align::Center);
    poll.add_suffix(&apply_poll);
    let poll_group = adw::PreferencesGroup::new();
    poll_group.add(&poll);
    let poll_unsupported = adw::ActionRow::new();
    poll_unsupported.set_title(&ctx.t_or(
        "shared.vfsControl.actions.pollIntervalNotSupported",
        "Poll interval not supported",
    ));
    poll_unsupported.set_visible(false);
    poll_group.add(&poll_unsupported);
    root.append(&poll_group);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_homogeneous(true);
    let refresh_meta = gtk::Button::with_label(&ctx.t_or(
        "shared.vfsControl.actions.refreshMetadata",
        "Refresh Metadata",
    ));
    let clear_cache = gtk::Button::with_label(&ctx.t_or(
        "shared.vfsControl.actions.clearCache",
        "Clear Metadata Cache",
    ));
    let reload = gtk::Button::with_label(
        &ctx.t_or("shared.vfsControl.actions.refreshAll", "Refresh All Data"),
    );
    actions.append(&refresh_meta);
    actions.append(&clear_cache);
    actions.append(&reload);
    root.append(&actions);

    let refresh_ui = {
        let ctx = ctx.clone();
        let remote = remote.clone();
        let selected = selected.clone();
        let known_names = known_names.clone();
        let delay_id = delay_id.clone();
        let refresh_slot = refresh_slot.clone();
        let suppressing = suppressing.clone();
        let empty = empty.clone();
        let indexed = indexed.clone();
        let indexed_label = indexed_label.clone();
        let combo = combo.clone();
        let combo_group = combo_group.clone();
        let stats = stats.clone();
        let queue_list = queue_list.clone();
        let queue_info = queue_info.clone();
        let delay_box = delay_box.clone();
        let cache = cache.clone();
        let advanced_list = advanced_list.clone();
        let advanced_search = advanced_search.clone();
        let poll = poll.clone();
        let apply_poll = apply_poll.clone();
        let poll_unsupported = poll_unsupported.clone();
        let toast = toast.clone();
        Rc::new(move || {
            clear_list(&stats);
            clear_list(&queue_list);
            clear_list(&cache);
            clear_list(&advanced_list);
            queue_info.set_text("");

            let Some(client) = ctx.client() else {
                empty.set_text(&ctx.t_or(
                    "notification.title.engineConnectionFailed",
                    "Engine Connection Error",
                ));
                empty.set_visible(true);
                combo_group.set_visible(false);
                return;
            };

            let listed = client
                .vfs_list()
                .ok()
                .map(|value| filter_vfs_names(&parse_vfs_list(&value), &remote))
                .unwrap_or_default();
            if listed != *known_names.borrow() {
                *known_names.borrow_mut() = listed.clone();
                suppressing.set(true);
                let refs: Vec<&str> = listed.iter().map(String::as_str).collect();
                combo.set_model(Some(&gtk::StringList::new(&refs)));
                if let Some(idx) = listed.iter().position(|n| n == selected.borrow().as_str()) {
                    combo.set_selected(idx as u32);
                } else if let Some(first) = listed.first() {
                    *selected.borrow_mut() = first.clone();
                    combo.set_selected(0);
                }
                suppressing.set(false);
            }
            combo.set_visible(listed.len() > 1);
            combo_group.set_visible(listed.len() > 1);

            if listed.is_empty() {
                empty.set_markup(&format!(
                    "<b>{}</b>\n{}",
                    glib::markup_escape_text(
                        &ctx.t_or("shared.vfsControl.noVfsFound", "No VFS Found")
                    ),
                    glib::markup_escape_text(&ctx.t_or(
                        "shared.vfsControl.noVfsRunning",
                        "No VFS instances are running for this remote."
                    ))
                ));
                empty.set_visible(true);
                indexed.set_visible(false);
                delay_box.set_visible(false);
                return;
            }
            empty.set_visible(false);

            let fs = selected.borrow().clone();
            let indexed_fs = is_indexed_vfs(&fs);
            indexed_label
                .set_text(&ctx.tf("shared.vfsControl.indexedWarning", &[("suffix", ":[0]")]));
            indexed.set_visible(indexed_fs);
            if indexed_fs {
                delay_box.set_visible(false);
                return;
            }

            match client.vfs_stats(&fs) {
                Ok(value) => {
                    let parsed = parse_vfs_stats(&value);
                    add_stat(
                        &stats,
                        &ctx.t_or("shared.vfsControl.stats.metadata", "Metadata"),
                        &format!(
                            "{} {}",
                            parsed.metadata_items(),
                            ctx.t_or("shared.vfsControl.stats.items", "items")
                        ),
                    );
                    if parsed.disk_cache_enabled() {
                        add_stat(
                            &stats,
                            &ctx.t_or("shared.vfsControl.stats.diskCache", "Disk Cache"),
                            &crate::rclone::format_bytes(parsed.disk_bytes),
                        );
                        add_stat(
                            &stats,
                            &ctx.t_or("shared.vfsControl.stats.cachedFiles", "Cached Files"),
                            &parsed.disk_files.to_string(),
                        );
                        add_stat(
                            &stats,
                            &ctx.t_or("shared.vfsControl.stats.pendingUploads", "Pending Uploads"),
                            &parsed.uploads_queued.to_string(),
                        );
                        add_stat(
                            &stats,
                            &ctx.t_or("shared.vfsControl.stats.inProgress", "In Progress"),
                            &parsed.uploads_in_progress.to_string(),
                        );
                        if parsed.errored > 0 {
                            add_stat(
                                &stats,
                                &ctx.t_or("shared.vfsControl.stats.errors", "Errors"),
                                &parsed.errored.to_string(),
                            );
                        }
                    } else {
                        add_stat(
                            &stats,
                            &ctx.t_or("shared.vfsControl.stats.diskCache", "Disk Cache"),
                            &ctx.t_or("shared.vfsControl.stats.disabled", "Disabled"),
                        );
                    }
                    add_stat(
                        &stats,
                        &ctx.t_or("shared.vfsControl.stats.inUse", "In Use"),
                        &parsed.in_use.to_string(),
                    );
                    if !parsed.disk_path.is_empty() {
                        cache.append(&path_row(
                            &ctx,
                            &ctx.t_or("shared.vfsControl.cacheInfo.location", "Cache Location:"),
                            &parsed.disk_path,
                            &ctx.t_or(
                                "shared.vfsControl.cacheInfo.openFolder",
                                "Open cache folder",
                            ),
                        ));
                    }
                    if !parsed.disk_path_meta.is_empty() {
                        cache.append(&path_row(
                            &ctx,
                            &ctx.t_or(
                                "shared.vfsControl.cacheInfo.metaLocation",
                                "Metadata Location:",
                            ),
                            &parsed.disk_path_meta,
                            &ctx.t_or(
                                "shared.vfsControl.cacheInfo.openMetaFolder",
                                "Open metadata folder",
                            ),
                        ));
                    }
                    if parsed.disk_cache_enabled() {
                        let note = adw::ActionRow::new();
                        note.set_title(&ctx.t_or("shared.vfsControl.cacheInfo.note", "Note:"));
                        note.set_subtitle(&ctx.t_or(
                            "shared.vfsControl.cacheInfo.noteText",
                            "Clear Metadata only removes directory listings cache.",
                        ));
                        cache.append(&note);
                    }
                    if let Some(map) = parsed.opt.as_object() {
                        let query = advanced_search.text().to_string();
                        fill_advanced_opts(&ctx, &advanced_list, map, &query);
                    }
                    match client.vfs_poll_interval(&fs, None) {
                        Ok(_) => {
                            poll.set_sensitive(true);
                            apply_poll.set_sensitive(true);
                            poll_unsupported.set_visible(false);
                        }
                        Err(_) => {
                            poll.set_sensitive(false);
                            apply_poll.set_sensitive(false);
                            poll_unsupported.set_visible(true);
                        }
                    }
                }
                Err(e) => {
                    add_stat(
                        &stats,
                        &ctx.t_or("remoteConfig.vfsStatsUnavailable", "VFS stats unavailable"),
                        &e.to_string(),
                    );
                }
            }

            match client.vfs_queue(&fs) {
                Ok(value) => {
                    let items = parse_vfs_queue(&value);
                    if items.is_empty() {
                        let row = adw::ActionRow::new();
                        row.set_title(&ctx.t_or(
                            "shared.vfsControl.queue.empty",
                            "No files queued for upload",
                        ));
                        row.set_subtitle(&ctx.t_or(
                            "shared.vfsControl.queue.emptyHint",
                            "Files are uploaded automatically based on VFS cache mode",
                        ));
                        queue_list.append(&row);
                    } else {
                        let uploading = items.iter().filter(|i| i.uploading).count();
                        let total: i64 = items.iter().map(|i| i.size).sum();
                        queue_info.set_text(&ctx.tf(
                            "shared.vfsControl.queue.uploadingCount",
                            &[
                                ("uploading", &uploading.to_string()),
                                ("total", &crate::rclone::format_bytes(total)),
                            ],
                        ));
                        for item in items {
                            queue_list.append(&queue_row(
                                &ctx,
                                &item,
                                &toast,
                                selected.clone(),
                                delay_id.clone(),
                                delay_box.clone(),
                                refresh_slot.clone(),
                            ));
                        }
                    }
                }
                Err(e) => {
                    let row = adw::ActionRow::new();
                    row.set_title(
                        &ctx.t_or("remoteConfig.vfsQueueUnavailable", "Queue unavailable"),
                    );
                    row.set_subtitle(&e.to_string());
                    queue_list.append(&row);
                }
            }

            let open_delay = delay_id
                .borrow()
                .as_ref()
                .map(|id| !id.is_empty())
                .unwrap_or(false);
            delay_box.set_visible(open_delay);
        })
    };
    *refresh_slot.borrow_mut() = refresh_ui.clone();
    {
        let advanced_list = advanced_list.clone();
        advanced_search.connect_search_changed(move |entry| {
            apply_advanced_filter(&advanced_list, &entry.text());
        });
    }
    refresh_ui();

    {
        let selected = selected.clone();
        let known_names = known_names.clone();
        let suppressing = suppressing.clone();
        let refresh_ui = refresh_ui.clone();
        combo.connect_selected_notify(move |row| {
            if suppressing.get() {
                return;
            }
            let idx = row.selected() as usize;
            // `refresh_ui` rebuilds the dropdown and takes
            // `known_names.borrow_mut()`, so the read must end first.
            let name = known_names.borrow().get(idx).cloned();
            if let Some(name) = name {
                *selected.borrow_mut() = name;
                refresh_ui();
            }
        });
    }
    {
        let delay_id = delay_id.clone();
        let delay_box = delay_box.clone();
        delay_cancel.connect_clicked(move |_| {
            *delay_id.borrow_mut() = None;
            delay_box.set_visible(false);
        });
    }
    {
        let ctx = ctx.clone();
        let selected = selected.clone();
        let delay_id = delay_id.clone();
        let delay_scale = delay_scale.clone();
        let delay_box = delay_box.clone();
        let toast = toast.clone();
        let refresh_ui = refresh_ui.clone();
        delay_apply.connect_clicked(move |_| {
            let Some(id) = delay_id.borrow().clone() else {
                return;
            };
            let secs = delay_scale.value().round() as i64;
            set_expiry(
                &ctx,
                &selected.borrow(),
                &id,
                &secs.to_string(),
                false,
                &ctx.tf(
                    "shared.vfsControl.actions.messages.delayedSeconds",
                    &[("seconds", &secs.to_string())],
                ),
                &toast,
                || refresh_ui(),
            );
            *delay_id.borrow_mut() = None;
            delay_box.set_visible(false);
        });
    }
    {
        let ctx = ctx.clone();
        let selected = selected.clone();
        let poll = poll.clone();
        let toast = toast.clone();
        apply_poll.connect_clicked(move |_| {
            let Some(client) = ctx.client() else {
                return;
            };
            match client.vfs_poll_interval(&selected.borrow(), Some(&poll.text())) {
                Ok(_) => toast.add_toast(adw::Toast::new(&ctx.tf(
                    "shared.vfsControl.actions.messages.intervalSet",
                    &[("val", &poll.text())],
                ))),
                Err(e) => ctx.toast_error(&toast, &e.to_string()),
            }
        });
    }
    {
        let ctx = ctx.clone();
        let selected = selected.clone();
        let toast = toast.clone();
        let refresh_ui = refresh_ui.clone();
        refresh_meta.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                // The `Ref` from a match scrutinee lives for the whole match,
                // and `refresh_ui` reassigns `selected` when the VFS list
                // changes — take an owned copy first.
                let target = selected.borrow().clone();
                match client.vfs_refresh_ex(&target, None, true) {
                    Ok(_) => {
                        toast.add_toast(adw::Toast::new(&ctx.t_or(
                            "shared.vfsControl.actions.messages.directoryRefreshed",
                            "Directory refreshed",
                        )));
                        refresh_ui();
                    }
                    Err(e) => ctx.toast_error(&toast, &e.to_string()),
                }
            }
        });
    }
    {
        let ctx = ctx.clone();
        let selected = selected.clone();
        let toast = toast.clone();
        let refresh_ui = refresh_ui.clone();
        clear_cache.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let target = selected.borrow().clone();
                match client.vfs_forget(&target) {
                    Ok(value) => {
                        let count = value
                            .get("forgotten")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        toast.add_toast(adw::Toast::new(&ctx.tf(
                            "shared.vfsControl.actions.messages.cleared",
                            &[("count", &count.to_string())],
                        )));
                        refresh_ui();
                    }
                    Err(e) => ctx.toast_error(&toast, &e.to_string()),
                }
            }
        });
    }
    {
        let refresh_ui = refresh_ui.clone();
        reload.connect_clicked(move |_| refresh_ui());
    }

    let root_weak = root.downgrade();
    glib::timeout_add_seconds_local(5, move || {
        let Some(root) = root_weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if root.is_mapped() {
            refresh_ui();
        }
        glib::ControlFlow::Continue
    });

    root.upcast()
}

fn queue_row(
    ctx: &AppCtx,
    item: &VfsQueueItem,
    toast: &adw::ToastOverlay,
    selected: Rc<RefCell<String>>,
    delay_id: Rc<RefCell<Option<String>>>,
    delay_box: gtk::Box,
    refresh_slot: Rc<RefCell<Rc<dyn Fn()>>>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    let name = if item.name.is_empty() {
        format!("#{}", item.id)
    } else {
        item.name.clone()
    };
    row.set_title(&name);
    row.set_tooltip_text(Some(&format!(
        "{}: {name}",
        ctx.t_or("shared.vfsControl.queue.name", "Name")
    )));
    let status = match queue_item_status(item) {
        QueueStatus::Uploading => ctx.t_or(
            "shared.vfsControl.queue.statusText.uploading",
            "Uploading now",
        ),
        QueueStatus::Delayed => ctx.t_or("shared.vfsControl.queue.statusText.delayed", "Delayed"),
        QueueStatus::Ready => ctx.tf(
            "shared.vfsControl.queue.statusText.ready",
            &[("seconds", &item.expiry_secs.abs().round().to_string())],
        ),
        QueueStatus::Waiting => ctx.tf(
            "shared.vfsControl.queue.statusText.waiting",
            &[("seconds", &format!("{:.1}", item.expiry_secs))],
        ),
    };
    let size_label = format!(
        "{} {}",
        ctx.t_or("shared.vfsControl.queue.size", "Size"),
        crate::rclone::format_bytes(item.size)
    );
    row.set_subtitle(&crate::vfs::vfs_queue_subtitle(
        &size_label,
        &format!(
            "{} {}",
            ctx.t_or("shared.vfsControl.queue.status", "Status"),
            status
        ),
        item.tries,
        &ctx.t_or("shared.vfsControl.queue.tries", "try"),
    ));

    if !item.uploading {
        if item.expiry_secs > 5.0 {
            let prioritize = gtk::Button::from_icon_name("go-up-symbolic");
            prioritize.set_valign(gtk::Align::Center);
            prioritize.set_tooltip_text(Some(&ctx.t_or(
                "shared.vfsControl.queue.tooltips.uploadNext",
                "Upload this file next",
            )));
            let ctx = ctx.clone();
            let toast = toast.clone();
            let selected = selected.clone();
            let refresh_slot = refresh_slot.clone();
            let id = item.id.clone();
            let name = item.name.clone();
            prioritize.connect_clicked(move |_| {
                let refresh = refresh_slot.borrow().clone();
                set_expiry(
                    &ctx,
                    &selected.borrow(),
                    &id,
                    &PRIORITY_EXPIRY.to_string(),
                    false,
                    &ctx.tf(
                        "shared.vfsControl.actions.messages.prioritized",
                        &[("name", &name)],
                    ),
                    &toast,
                    || refresh(),
                );
            });
            row.add_suffix(&prioritize);
        }
        let delay = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        delay.set_valign(gtk::Align::Center);
        delay.set_tooltip_text(Some(&ctx.t_or(
            "shared.vfsControl.queue.tooltips.delayLong",
            "Delay upload by 11.5 days",
        )));
        {
            let ctx = ctx.clone();
            let toast = toast.clone();
            let selected = selected.clone();
            let refresh_slot = refresh_slot.clone();
            let id = item.id.clone();
            delay.connect_clicked(move |_| {
                let refresh = refresh_slot.borrow().clone();
                set_expiry(
                    &ctx,
                    &selected.borrow(),
                    &id,
                    &DELAY_EXPIRY.to_string(),
                    false,
                    &ctx.tf(
                        "shared.vfsControl.actions.messages.delayedSeconds",
                        &[("seconds", &DELAY_EXPIRY.to_string())],
                    ),
                    &toast,
                    || refresh(),
                );
            });
        }
        row.add_suffix(&delay);
        let custom = gtk::Button::from_icon_name("alarm-symbolic");
        custom.set_valign(gtk::Align::Center);
        custom.set_tooltip_text(Some(&ctx.t_or(
            "shared.vfsControl.queue.tooltips.setDelay",
            "Set custom delay",
        )));
        let id = item.id.clone();
        custom.connect_clicked(move |_| {
            *delay_id.borrow_mut() = Some(id.clone());
            delay_box.set_visible(true);
        });
        row.add_suffix(&custom);
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn set_expiry(
    ctx: &AppCtx,
    fs: &str,
    id: &str,
    expiry: &str,
    relative: bool,
    ok: &str,
    toast: &adw::ToastOverlay,
    refresh: impl Fn(),
) {
    let Some(client) = ctx.client() else {
        return;
    };
    match client.vfs_queue_set_expiry_ex(fs, id, expiry, relative) {
        Ok(_) => {
            toast.add_toast(adw::Toast::new(ok));
            refresh();
        }
        Err(e) => toast.add_toast(adw::Toast::new(&ctx.tf(
            "shared.vfsControl.actions.messages.actionFailed",
            &[("error", &ctx.translate_error(&e.to_string()))],
        ))),
    }
}

fn add_stat(list: &gtk::ListBox, title: &str, value: &str) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(value);
    list.append(&row);
}

fn path_row(ctx: &AppCtx, title: &str, path: &str, tooltip: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(path);
    let open = gtk::Button::from_icon_name("folder-open-symbolic");
    open.set_valign(gtk::Align::Center);
    open.set_tooltip_text(Some(tooltip));
    let path = path.to_string();
    let toast_title = ctx.t_or("shared.vfsControl.cacheInfo.openFolder", "Open folder");
    open.connect_clicked(move |_| {
        if let Err(e) = open::that(&path) {
            eprintln!("{toast_title}: {e}");
        }
    });
    row.add_suffix(&open);
    row
}

fn format_opt_value(ctx: &AppCtx, value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(true) => ctx.t_or(
            "shared.vfsControl.advancedConfig.booleanEnabled",
            "✓ Enabled",
        ),
        serde_json::Value::Bool(false) => ctx.t_or(
            "shared.vfsControl.advancedConfig.booleanDisabled",
            "✗ Disabled",
        ),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn fill_advanced_opts(
    ctx: &AppCtx,
    list: &gtk::ListBox,
    map: &serde_json::Map<String, serde_json::Value>,
    query: &str,
) {
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    let mut keys: Vec<_> = map.keys().cloned().collect();
    keys.sort();
    for key in keys {
        let group = crate::vfs::vfs_opt_group(&key);
        if let Some((_, items)) = groups.iter_mut().find(|(name, _)| *name == group) {
            items.push(key);
        } else {
            groups.push((group, vec![key]));
        }
    }
    for (group, items) in groups {
        let header = adw::ActionRow::new();
        header.set_title(&ctx.t_or(&crate::vfs::vfs_opt_group_i18n_key(group), group));
        header.set_sensitive(false);
        header.set_widget_name(&format!("group:{group}"));
        header.add_css_class("heading");
        list.append(&header);
        for key in items {
            let display = format_opt_value(ctx, &map[&key]);
            let row = adw::ActionRow::new();
            row.set_title(&key);
            row.set_subtitle(&display);
            row.set_widget_name(&key);
            list.append(&row);
        }
    }
    let empty = adw::ActionRow::new();
    empty.set_title(&ctx.t_or(
        "shared.vfsControl.advancedConfig.noResults",
        "No options match your search",
    ));
    empty.set_widget_name("no-results");
    empty.set_sensitive(false);
    list.append(&empty);
    apply_advanced_filter(list, query);
}

fn apply_advanced_filter(list: &gtk::ListBox, query: &str) {
    let mut child = list.first_child();
    let mut any_option = false;
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(row) = widget.clone().downcast::<adw::ActionRow>() {
            let name = row.widget_name().to_string();
            if name.starts_with("group:") || name == "no-results" {
                child = next;
                continue;
            }
            let value = row.subtitle().unwrap_or_default().to_string();
            let visible = crate::vfs::vfs_opt_matches(&name, &value, query);
            row.set_visible(visible);
            any_option |= visible;
        }
        child = next;
    }
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if let Ok(row) = widget.clone().downcast::<adw::ActionRow>() {
            let name = row.widget_name().to_string();
            if name.starts_with("group:") {
                let mut show = false;
                let mut sibling = next.clone();
                while let Some(next_row) = sibling {
                    if let Ok(opt) = next_row.clone().downcast::<adw::ActionRow>() {
                        let opt_name = opt.widget_name().to_string();
                        if opt_name.starts_with("group:") || opt_name == "no-results" {
                            break;
                        }
                        show |= opt.is_visible();
                    }
                    sibling = next_row.next_sibling();
                }
                row.set_visible(show);
            } else if name == "no-results" {
                row.set_visible(!any_option);
            }
        }
        child = next;
    }
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}
