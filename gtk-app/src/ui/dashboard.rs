use super::dialogs;
use super::AppCtx;
use crate::operations::{AppTab, OperationType};
use crate::rclone::{format_bytes, remote_fs};
use adw::prelude::*;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct Dashboard {
    pub root: gtk::Box,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    tab: Rc<RefCell<AppTab>>,
    sidebar_list: gtk::ListBox,
    search: gtk::SearchEntry,
    content: gtk::Stack,
    overview: gtk::Box,
    detail: gtk::Box,
}

impl Dashboard {
    pub fn new(ctx: AppCtx, toast: adw::ToastOverlay) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let split = adw::OverlaySplitView::new();
        split.set_min_sidebar_width(260.0);
        split.set_max_sidebar_width(360.0);

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 8);
        sidebar.set_margin_top(8);
        sidebar.set_margin_bottom(8);
        sidebar.set_margin_start(8);
        sidebar.set_margin_end(8);
        sidebar.add_css_class("sidebar");

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search remotes"));
        let sidebar_list = gtk::ListBox::new();
        sidebar_list.add_css_class("navigation-sidebar");
        sidebar_list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&sidebar_list));

        let add_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let quick = gtk::Button::from_icon_name("list-add-symbolic");
        quick.set_tooltip_text(Some("Quick add remote"));
        let detailed = gtk::Button::from_icon_name("document-edit-symbolic");
        detailed.set_tooltip_text(Some("Detailed remote config"));
        add_box.append(&quick);
        add_box.append(&detailed);

        sidebar.append(&search);
        sidebar.append(&scroll);
        sidebar.append(&add_box);
        split.set_sidebar(Some(&sidebar));

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let content = gtk::Stack::new();
        content.set_vexpand(true);
        let overview = gtk::Box::new(gtk::Orientation::Vertical, 12);
        overview.set_margin_top(16);
        overview.set_margin_bottom(16);
        overview.set_margin_start(16);
        overview.set_margin_end(16);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 12);
        detail.set_margin_top(16);
        detail.set_margin_bottom(16);
        detail.set_margin_start(16);
        detail.set_margin_end(16);
        content.add_named(&scrolled(&overview), Some("overview"));
        content.add_named(&scrolled(&detail), Some("detail"));

        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tabs.add_css_class("linked");
        tabs.set_halign(gtk::Align::Center);
        tabs.set_margin_bottom(10);
        tabs.set_margin_top(6);

        content_box.append(&content);
        content_box.append(&tabs);
        split.set_content(Some(&content_box));
        root.append(&split);

        let dash = Self {
            root,
            ctx: ctx.clone(),
            toast,
            tab: Rc::new(RefCell::new(AppTab::General)),
            sidebar_list,
            search,
            content,
            overview,
            detail,
        };

        for tab in AppTab::ALL {
            let btn = gtk::ToggleButton::new();
            btn.set_label(&ctx.t(tab.label_key()));
            btn.set_tooltip_text(Some(tab.as_str()));
            if tab == AppTab::General {
                btn.set_active(true);
            }
            let dash_c = dash.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    *dash_c.tab.borrow_mut() = tab;
                    dash_c.refresh();
                }
            });
            tabs.append(&btn);
        }

        {
            let dash = dash.clone();
            dash.search
                .clone()
                .connect_search_changed(move |_| dash.refresh());
        }
        {
            let ctx = ctx.clone();
            let dash = dash.clone();
            quick.connect_clicked(move |_| {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::quick_add_remote(&win, ctx.clone(), {
                        let dash = dash.clone();
                        let ctx = ctx.clone();
                        Rc::new(move || {
                            ctx.refresh_runtime();
                            dash.refresh();
                        })
                    });
                }
            });
        }
        {
            let ctx = ctx.clone();
            let dash = dash.clone();
            detailed.connect_clicked(move |_| {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::remote_config(&win, ctx.clone(), None, {
                        let dash = dash.clone();
                        let ctx = ctx.clone();
                        Rc::new(move || {
                            ctx.refresh_runtime();
                            dash.refresh();
                        })
                    });
                }
            });
        }
        {
            let dash = dash.clone();
            dash.sidebar_list
                .clone()
                .connect_row_activated(move |_, row| {
                    let name = row
                        .child()
                        .and_then(|c| c.downcast::<gtk::Box>().ok())
                        .and_then(|b| b.first_child())
                        .and_then(|c| c.downcast::<gtk::Label>().ok())
                        .map(|l| l.label().to_string());
                    if let Some(name) = name {
                        *dash.ctx.selected_remote.borrow_mut() = Some(name);
                        dash.refresh();
                    }
                });
        }

        dash.refresh();
        dash
    }

    pub fn refresh(&self) {
        self.fill_sidebar();
        if self.ctx.selected_remote.borrow().is_some() {
            self.content.set_visible_child_name("detail");
            self.fill_detail();
        } else {
            self.content.set_visible_child_name("overview");
            self.fill_overview();
        }
    }

    fn fill_sidebar(&self) {
        while let Some(child) = self.sidebar_list.first_child() {
            self.sidebar_list.remove(&child);
        }
        let query = self.search.text().to_lowercase();
        let snap = self.ctx.snapshot.borrow();
        for remote in &snap.remotes {
            if remote.hidden {
                continue;
            }
            if !query.is_empty()
                && !remote.name.to_lowercase().contains(&query)
                && !remote.r#type.to_lowercase().contains(&query)
            {
                continue;
            }
            let row = gtk::ListBoxRow::new();
            let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            box_.set_margin_top(6);
            box_.set_margin_bottom(6);
            box_.set_margin_start(8);
            box_.set_margin_end(8);
            let name = gtk::Label::new(Some(&remote.name));
            name.set_xalign(0.0);
            name.set_hexpand(true);
            let badge = gtk::Label::new(Some(&status_dot(
                remote.mounted,
                remote.serving,
                remote.job_active,
            )));
            box_.append(&name);
            box_.append(&badge);
            row.set_child(Some(&box_));
            if self.ctx.selected_remote.borrow().as_deref() == Some(remote.name.as_str()) {
                row.activate();
            }
            self.sidebar_list.append(&row);
        }
        if snap.remotes.is_empty() {
            let empty = adw::ActionRow::new();
            empty.set_title("No remotes configured");
            empty.set_subtitle("Use Quick Add or Detailed Config");
            self.sidebar_list.append(&empty);
        }
    }

    fn fill_overview(&self) {
        clear_box(&self.overview);
        let tab = *self.tab.borrow();
        let snap = self.ctx.snapshot.borrow();
        let title = adw::StatusPage::new();
        title.set_icon_name(Some(tab.icon_name()));
        title.set_title(&self.ctx.t(tab.label_key()));
        title.set_description(Some(&format!(
            "{} remotes · {} mounts · {} serves · {} jobs",
            snap.remotes.len(),
            snap.mounts.len(),
            snap.serves.len(),
            snap.jobs.len()
        )));
        self.overview.append(&title);

        self.overview.append(&section_label("Remotes"));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for remote in snap.remotes.iter().filter(|r| match tab {
            AppTab::Mount => true,
            AppTab::Serve => true,
            AppTab::Operations => true,
            AppTab::General => true,
        }) {
            if remote.hidden {
                continue;
            }
            if tab == AppTab::Mount && !remote.mounted && false {
                continue;
            }
            let row = adw::ActionRow::new();
            row.set_title(&remote.name);
            row.set_subtitle(&format!(
                "{} · {}",
                remote.r#type,
                remote_state_label(remote.mounted, remote.serving, remote.job_active)
            ));
            let browse = gtk::Button::from_icon_name("folder-symbolic");
            browse.set_valign(gtk::Align::Center);
            browse.set_tooltip_text(Some("Browse"));
            let mount = gtk::Button::from_icon_name("drive-harddisk-symbolic");
            mount.set_valign(gtk::Align::Center);
            mount.set_tooltip_text(Some("Mount / Unmount"));
            {
                let ctx = self.ctx.clone();
                let name = remote.name.clone();
                let dash = self.clone();
                browse.connect_clicked(move |_| {
                    *ctx.selected_remote.borrow_mut() = Some(name.clone());
                    dash.refresh();
                });
            }
            {
                let ctx = self.ctx.clone();
                let name = remote.name.clone();
                let mounted = remote.mounted;
                let toast = self.toast.clone();
                let dash = self.clone();
                mount.connect_clicked(move |_| {
                    toggle_mount(&ctx, &name, mounted, &toast);
                    dash.refresh();
                });
            }
            row.add_suffix(&browse);
            row.add_suffix(&mount);
            {
                let ctx = self.ctx.clone();
                let name = remote.name.clone();
                let dash = self.clone();
                row.connect_activated(move |_| {
                    *ctx.selected_remote.borrow_mut() = Some(name.clone());
                    dash.refresh();
                });
            }
            list.append(&row);
        }
        self.overview.append(&list);

        self.overview.append(&section_label("Jobs"));
        let jobs = gtk::ListBox::new();
        jobs.add_css_class("boxed-list");
        if snap.jobs.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("No active jobs");
            jobs.append(&row);
        } else {
            for job in &snap.jobs {
                let row = adw::ActionRow::new();
                row.set_title(&format!("{} · {}", job.operation, job.remote));
                row.set_subtitle(&format!("{} · {}", job.status, job.profile));
                let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
                stop.set_valign(gtk::Align::Center);
                {
                    let ctx = self.ctx.clone();
                    let id = job.id;
                    let dash = self.clone();
                    stop.connect_clicked(move |_| {
                        if let Some(c) = ctx.client() {
                            let _ = c.job_stop(id);
                            ctx.refresh_runtime();
                            dash.refresh();
                        }
                    });
                }
                row.add_suffix(&stop);
                {
                    let job = job.clone();
                    let dash = self.clone();
                    row.connect_activated(move |_| {
                        if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                            dialogs::job_detail(&win, &job);
                        }
                    });
                }
                jobs.append(&row);
            }
        }
        self.overview.append(&jobs);

        self.overview.append(&section_label("Serves"));
        let serves = gtk::ListBox::new();
        serves.add_css_class("boxed-list");
        if snap.serves.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("No running serves");
            serves.append(&row);
        } else {
            for serve in &snap.serves {
                let row = adw::ActionRow::new();
                row.set_title(&format!("{} · {}", serve.serve_type, serve.fs));
                row.set_subtitle(&serve.addr);
                serves.append(&row);
            }
        }
        self.overview.append(&serves);

        self.overview.append(&section_label("System"));
        let sys = adw::ActionRow::new();
        let bytes = snap
            .stats
            .get("bytes")
            .and_then(|x| x.as_i64())
            .unwrap_or(0);
        let speed = snap
            .stats
            .get("speed")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        sys.set_title("Bandwidth");
        sys.set_subtitle(&format!(
            "{} transferred · {:.1} KiB/s",
            format_bytes(bytes),
            speed / 1024.0
        ));
        self.overview.append(&sys);
    }

    fn fill_detail(&self) {
        clear_box(&self.detail);
        let Some(name) = self.ctx.selected_remote.borrow().clone() else {
            return;
        };
        let snap = self.ctx.snapshot.borrow();
        let remote = snap.remotes.iter().find(|r| r.name == name).cloned();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some("Back to overview"));
        {
            let ctx = self.ctx.clone();
            let dash = self.clone();
            back.connect_clicked(move |_| {
                *ctx.selected_remote.borrow_mut() = None;
                dash.refresh();
            });
        }
        let title = gtk::Label::new(Some(&name));
        title.add_css_class("title-2");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        header.append(&back);
        header.append(&title);
        self.detail.append(&header);

        if let Some(remote) = remote {
            let subtitle = gtk::Label::new(Some(&format!(
                "{} · {}",
                remote.r#type,
                remote_state_label(remote.mounted, remote.serving, remote.job_active)
            )));
            subtitle.add_css_class("dim-label");
            subtitle.set_xalign(0.0);
            self.detail.append(&subtitle);
        }

        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_max_children_per_line(6);
        for op in OperationType::ALL {
            let btn = gtk::Button::new();
            btn.set_label(op.api_label());
            btn.set_tooltip_text(Some(op.as_str()));
            {
                let ctx = self.ctx.clone();
                let name = name.clone();
                let toast = self.toast.clone();
                let dash = self.clone();
                btn.connect_clicked(move |_| {
                    start_operation(&ctx, &name, op, &toast);
                    dash.refresh();
                });
            }
            chips.append(&btn);
        }
        self.detail.append(&chips);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for (label, icon, kind) in [
            ("Browse", "folder-symbolic", "browse"),
            ("Logs", "utilities-terminal-symbolic", "logs"),
            ("Export", "document-save-symbolic", "export"),
            ("Clone", "edit-copy-symbolic", "clone"),
            ("Configure", "emblem-system-symbolic", "config"),
            ("Delete", "user-trash-symbolic", "delete"),
        ] {
            let btn = gtk::Button::from_icon_name(icon);
            btn.set_tooltip_text(Some(label));
            let ctx = self.ctx.clone();
            let name = name.clone();
            let toast = self.toast.clone();
            let dash = self.clone();
            btn.connect_clicked(move |_| match kind {
                "browse" => {
                    *ctx.selected_remote.borrow_mut() = Some(name.clone());
                }
                "logs" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::logs(&win, ctx.clone(), Some(name.clone()));
                    }
                }
                "export" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::export_backup(&win, ctx.clone(), toast.clone());
                    }
                }
                "clone" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::remote_config(&win, ctx.clone(), Some(name.clone()), {
                            let dash = dash.clone();
                            let ctx = ctx.clone();
                            Rc::new(move || {
                                ctx.refresh_runtime();
                                dash.refresh();
                            })
                        });
                    }
                }
                "config" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::remote_config(&win, ctx.clone(), Some(name.clone()), {
                            let dash = dash.clone();
                            let ctx = ctx.clone();
                            Rc::new(move || {
                                ctx.refresh_runtime();
                                dash.refresh();
                            })
                        });
                    }
                }
                "delete" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::delete_remote(&win, ctx.clone(), &name, {
                            let dash = dash.clone();
                            let ctx = ctx.clone();
                            Rc::new(move || {
                                *ctx.selected_remote.borrow_mut() = None;
                                ctx.refresh_runtime();
                                dash.refresh();
                            })
                        });
                    }
                }
                _ => {}
            });
            actions.append(&btn);
        }
        self.detail.append(&actions);

        let usage = adw::ActionRow::new();
        usage.set_title("Disk usage");
        if let Some(client) = self.ctx.client() {
            match client.about(&remote_fs(&name, "")) {
                Ok(about) => usage.set_subtitle(&crate::store::disk_label_from_about(&about)),
                Err(err) => usage.set_subtitle(&err.to_string()),
            }
        } else {
            usage.set_subtitle("Engine offline");
        }
        self.detail.append(&usage);

        self.detail
            .append(&section_label("Quick Runs for this remote"));
        let qlist = gtk::ListBox::new();
        qlist.add_css_class("boxed-list");
        for qr in self
            .ctx
            .store
            .borrow()
            .quick_runs
            .iter()
            .filter(|q| q.remote_name == name)
        {
            let row = adw::ActionRow::new();
            row.set_title(&qr.name);
            row.set_subtitle(&format!("{} · {}", qr.operation_type, qr.status));
            qlist.append(&row);
        }
        if qlist.first_child().is_none() {
            let row = adw::ActionRow::new();
            row.set_title("No quick runs");
            qlist.append(&row);
        }
        self.detail.append(&qlist);
    }
}

fn toggle_mount(ctx: &AppCtx, name: &str, mounted: bool, toast: &adw::ToastOverlay) {
    let Some(client) = ctx.client() else {
        toast.add_toast(adw::Toast::new("Rclone engine is offline"));
        return;
    };
    if mounted {
        if let Some(m) = ctx
            .snapshot
            .borrow()
            .mounts
            .iter()
            .find(|m| m.fs.starts_with(&format!("{name}:")))
        {
            match client.unmount(&m.mount_point) {
                Ok(_) => toast.add_toast(adw::Toast::new("Unmounted")),
                Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
            }
        }
    } else {
        let mount_point = default_mount_point(name);
        let _ = std::fs::create_dir_all(&mount_point);
        match client.mount(&remote_fs(name, ""), &mount_point, "mount") {
            Ok(_) => toast.add_toast(adw::Toast::new(&format!("Mounted at {mount_point}"))),
            Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }
    ctx.refresh_runtime();
}

fn start_operation(ctx: &AppCtx, name: &str, op: OperationType, toast: &adw::ToastOverlay) {
    let Some(client) = ctx.client() else {
        toast.add_toast(adw::Toast::new("Rclone engine is offline"));
        return;
    };
    match op {
        OperationType::Mount => {
            let mounted = ctx
                .snapshot
                .borrow()
                .mounts
                .iter()
                .any(|m| m.fs.starts_with(&format!("{name}:")));
            toggle_mount(ctx, name, mounted, toast);
        }
        OperationType::Serve => {
            match client.serve_start("webdav", &remote_fs(name, ""), "127.0.0.1:0") {
                Ok(v) => {
                    let addr = v.get("addr").and_then(|x| x.as_str()).unwrap_or("started");
                    toast.add_toast(adw::Toast::new(&format!("Serving at {addr}")));
                }
                Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
            }
        }
        other => {
            if let Some(endpoint) = other.rc_job_endpoint() {
                let params = serde_json::json!({
                    "srcFs": remote_fs(name, ""),
                    "dstFs": remote_fs(name, ""),
                });
                match client.start_job(endpoint, params) {
                    Ok(id) => {
                        toast.add_toast(adw::Toast::new(&format!("Started {} job #{id}", other)))
                    }
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
        }
    }
    ctx.refresh_runtime();
}

fn default_mount_point(name: &str) -> String {
    let base = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("mnt").join(name).to_string_lossy().into_owned()
}

fn status_dot(mounted: bool, serving: bool, job: bool) -> String {
    let mut parts = Vec::new();
    if mounted {
        parts.push("M");
    }
    if serving {
        parts.push("S");
    }
    if job {
        parts.push("J");
    }
    if parts.is_empty() {
        "·".into()
    } else {
        parts.join("")
    }
}

fn remote_state_label(mounted: bool, serving: bool, job: bool) -> String {
    let mut parts = Vec::new();
    if mounted {
        parts.push("mounted");
    }
    if serving {
        parts.push("serving");
    }
    if job {
        parts.push("job running");
    }
    if parts.is_empty() {
        "idle".into()
    } else {
        parts.join(" · ")
    }
}

fn section_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("title-4");
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(child));
    scroll
}

fn clear_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}
