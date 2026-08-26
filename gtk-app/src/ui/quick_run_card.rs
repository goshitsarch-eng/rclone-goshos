//! Angular `app-quick-run-card` overview variant.

use super::AppCtx;
use crate::jobs::{quick_run_card_badges, quick_run_openable_folders};
use crate::store::QuickRun;
use adw::prelude::*;
use gtk::prelude::*;
use std::rc::Rc;

pub struct OverviewHandlers {
    pub on_start: Rc<dyn Fn()>,
    pub on_stop: Rc<dyn Fn()>,
    pub on_edit: Rc<dyn Fn()>,
    pub on_open_remote: Rc<dyn Fn()>,
    pub on_open_path: Rc<dyn Fn(&str)>,
    pub on_select: Option<Rc<dyn Fn()>>,
}

pub fn overview_card(
    ctx: &AppCtx,
    qr: &QuickRun,
    running: bool,
    busy: bool,
    handlers: OverviewHandlers,
) -> gtk::Widget {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.set_margin_top(4);
    card.set_margin_bottom(4);
    card.set_margin_start(2);
    card.set_margin_end(2);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
    inner.set_margin_top(10);
    inner.set_margin_bottom(10);
    inner.set_margin_start(12);
    inner.set_margin_end(12);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.set_hexpand(true);
    let title = gtk::Label::new(Some(&qr.name));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    names.append(&title);
    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let op = gtk::Label::new(Some(&ctx.t_or(
        &format!("actions.{}", qr.operation_type.as_str()),
        qr.operation_type.api_label(),
    )));
    op.add_css_class("dim-label");
    meta.append(&op);
    let remote = gtk::Button::with_label(&qr.remote_name);
    remote.add_css_class("flat");
    remote.set_tooltip_text(Some(&format!(
        "{}: {}",
        ctx.t_or("dashboard.appDetail.remoteSettings", "Remote settings"),
        qr.remote_name
    )));
    {
        let on_open_remote = handlers.on_open_remote.clone();
        remote.connect_clicked(move |_| on_open_remote());
    }
    meta.append(&remote);
    if running {
        let live = gtk::Label::new(Some(&ctx.t_or("flow.quickRun.status.running", "Running")));
        live.add_css_class("accent");
        meta.append(&live);
    }
    names.append(&meta);
    header.append(&names);

    let badges = quick_run_card_badges(qr);
    let pills = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    pills.set_valign(gtk::Align::Start);
    if badges.cron {
        pills.append(&badge_icon(
            "document-open-recent-symbolic",
            &format!(
                "{}: {}",
                ctx.t_or("flow.quickRun.badges.scheduled", "Scheduled"),
                badges.cron_expression
            ),
        ));
    }
    if badges.watcher {
        let tip = if badges.watcher_changed_only {
            ctx.t_or(
                "flow.quickRun.badges.watcherChangedOnly",
                "Watch changed files only",
            )
        } else {
            ctx.t_or("flow.quickRun.badges.watcher", "watch")
        };
        pills.append(&badge_icon("view-reveal-symbolic", &tip));
    }
    if badges.autostart {
        pills.append(&badge_icon(
            "system-run-symbolic",
            &ctx.t_or("flow.quickRun.badges.autostart", "autostart"),
        ));
    }
    header.append(&pills);
    inner.append(&header);

    if !qr.description.is_empty() {
        let desc = gtk::Label::new(Some(&qr.description));
        desc.add_css_class("dim-label");
        desc.set_xalign(0.0);
        desc.set_wrap(true);
        inner.append(&desc);
    }

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let start = if running {
        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop.add_css_class("destructive-action");
        stop.set_tooltip_text(Some(&ctx.t_or("flow.quickRun.actions.stop", "Stop")));
        stop.set_sensitive(!busy);
        let on_stop = handlers.on_stop.clone();
        stop.connect_clicked(move |_| on_stop());
        stop
    } else {
        let start = gtk::Button::from_icon_name("media-playback-start-symbolic");
        start.add_css_class("suggested-action");
        start.set_tooltip_text(Some(&ctx.t_or("flow.quickRun.actions.start", "Start")));
        start.set_sensitive(!busy);
        let on_start = handlers.on_start.clone();
        start.connect_clicked(move |_| on_start());
        start
    };
    actions.append(&start);
    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.set_tooltip_text(Some(&ctx.t_or("common.edit", "Edit")));
    {
        let on_edit = handlers.on_edit.clone();
        edit.connect_clicked(move |_| on_edit());
    }
    actions.append(&edit);

    let folders = quick_run_openable_folders(qr);
    if folders.len() == 1 {
        let folder = gtk::Button::from_icon_name("folder-open-symbolic");
        folder.set_tooltip_text(Some(&folders[0].path));
        let path = folders[0].path.clone();
        let on_open_path = handlers.on_open_path.clone();
        folder.connect_clicked(move |_| on_open_path(&path));
        actions.append(&folder);
    } else if folders.len() > 1 {
        let btn = gtk::MenuButton::new();
        btn.set_icon_name("folder-open-symbolic");
        btn.set_tooltip_text(Some(
            &ctx.t_or("overviews.remoteCard.browse", "Browse active folders"),
        ));
        let popover = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.set_margin_top(6);
        list.set_margin_bottom(6);
        list.set_margin_start(6);
        list.set_margin_end(6);
        for folder in folders {
            let label = format!(
                "{} ({})",
                if folder.kind == "source" {
                    ctx.t_or("detailShared.pathDisplay.source", "Source")
                } else {
                    ctx.t_or("detailShared.pathDisplay.destination", "Destination")
                },
                folder.path
            );
            let item = gtk::Button::with_label(&label);
            let popover = popover.clone();
            let path = folder.path.clone();
            let on_open_path = handlers.on_open_path.clone();
            item.connect_clicked(move |_| {
                on_open_path(&path);
                popover.popdown();
            });
            list.append(&item);
        }
        popover.set_child(Some(&list));
        btn.set_popover(Some(&popover));
        actions.append(&btn);
    }
    inner.append(&actions);
    card.append(&inner);

    if let Some(on_select) = handlers.on_select {
        title.set_cursor_from_name(Some("pointer"));
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.connect_released(move |_, _, _, _| on_select());
        title.add_controller(click);
    }
    card.upcast()
}

fn badge_icon(icon: &str, tooltip: &str) -> gtk::Image {
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(16);
    image.set_tooltip_text(Some(tooltip));
    image
}
