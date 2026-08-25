use super::dialogs;
use super::AppCtx;
use crate::operations::OperationType;
use crate::store::QuickRun;
use adw::prelude::*;
use gtk::prelude::*;
use std::rc::Rc;

#[derive(Clone)]
pub struct FlowView {
    pub root: gtk::Box,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    sidebar: gtk::ListBox,
    search: gtk::SearchEntry,
    content: gtk::Box,
}

impl FlowView {
    pub fn new(ctx: AppCtx, toast: adw::ToastOverlay) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let split = adw::OverlaySplitView::new();
        split.set_min_sidebar_width(260.0);

        let side = gtk::Box::new(gtk::Orientation::Vertical, 8);
        side.set_margin_top(8);
        side.set_margin_start(8);
        side.set_margin_end(8);
        side.set_margin_bottom(8);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search quick runs"));
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&sidebar));
        let add = gtk::Button::with_label("New Quick Run");
        side.append(&search);
        side.append(&scroll);
        side.append(&add);
        split.set_sidebar(Some(&side));

        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        content.set_margin_start(16);
        content.set_margin_end(16);
        split.set_content(Some(&scrolled(&content)));
        root.append(&split);

        let view = Self {
            root,
            ctx: ctx.clone(),
            toast,
            sidebar,
            search,
            content,
        };

        {
            let view = view.clone();
            view.search
                .clone()
                .connect_search_changed(move |_| view.refresh());
        }
        {
            let view = view.clone();
            let ctx = ctx.clone();
            add.connect_clicked(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::quick_run_editor(&win, ctx.clone(), None, {
                        let view = view.clone();
                        Rc::new(move || view.refresh())
                    });
                }
            });
        }
        view.refresh();
        view
    }

    pub fn refresh(&self) {
        while let Some(child) = self.sidebar.first_child() {
            self.sidebar.remove(&child);
        }
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }

        let query = self.search.text().to_lowercase();
        let runs = self.ctx.store.borrow().quick_runs.clone();
        let filtered: Vec<QuickRun> = runs
            .into_iter()
            .filter(|qr| {
                if query.is_empty() {
                    return true;
                }
                qr.name.to_lowercase().contains(&query)
                    || qr.remote_name.to_lowercase().contains(&query)
                    || qr.operation_type.as_str().contains(&query)
            })
            .collect();

        if filtered.is_empty() {
            let empty = adw::StatusPage::new();
            empty.set_icon_name(Some("media-playlist-consecutive-symbolic"));
            empty.set_title("No quick runs");
            empty.set_description(Some(
                "Create a reusable rclone operation with cron, watcher, or autostart.",
            ));
            self.sidebar.append(&adw::ActionRow::new());
            self.content.append(&empty);
            return;
        }

        for qr in &filtered {
            let row = adw::ActionRow::new();
            row.set_title(&qr.name);
            let mut badges = vec![qr.operation_type.as_str().to_string()];
            if qr.config.app.cron_enabled {
                badges.push("cron".into());
            }
            if qr.config.app.watch_enabled {
                badges.push("watch".into());
            }
            if qr.config.app.auto_start {
                badges.push("autostart".into());
            }
            row.set_subtitle(&format!("{} · {}", qr.remote_name, badges.join(" · ")));
            let view = self.clone();
            let id = qr.id.clone();
            row.connect_activated(move |_| {
                *view.ctx.selected_quick_run.borrow_mut() = Some(id.clone());
                view.refresh();
            });
            self.sidebar.append(&row);
        }

        if let Some(id) = self.ctx.selected_quick_run.borrow().clone() {
            if let Some(qr) = filtered.iter().find(|q| q.id == id).cloned() {
                self.fill_detail(&qr);
                return;
            }
        }
        self.fill_overview(&filtered);
    }

    fn fill_overview(&self, runs: &[QuickRun]) {
        let title = gtk::Label::new(Some("Quick Runs"));
        title.add_css_class("title-1");
        title.set_xalign(0.0);
        self.content.append(&title);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for qr in runs {
            let row = adw::ActionRow::new();
            row.set_title(&qr.name);
            row.set_subtitle(&format!(
                "{} · {} · {}",
                qr.operation_type, qr.remote_name, qr.status
            ));
            let start = gtk::Button::from_icon_name("media-playback-start-symbolic");
            start.set_valign(gtk::Align::Center);
            {
                let view = self.clone();
                let qr = qr.clone();
                start.connect_clicked(move |_| view.start_run(&qr));
            }
            row.add_suffix(&start);
            list.append(&row);
        }
        self.content.append(&list);

        let builder = adw::StatusPage::new();
        builder.set_icon_name(Some("applications-engineering-symbolic"));
        builder.set_title("Workflow builder");
        builder.set_description(Some(
            "The visual workflow builder is a placeholder, matching the current app (GitHub #232).",
        ));
        self.content.append(&builder);
    }

    fn fill_detail(&self, qr: &QuickRun) {
        let title = gtk::Label::new(Some(&qr.name));
        title.add_css_class("title-1");
        title.set_xalign(0.0);
        self.content.append(&title);
        let sub = gtk::Label::new(Some(&format!(
            "{} · {} · {}",
            qr.operation_type.api_label(),
            qr.remote_name,
            qr.status
        )));
        sub.add_css_class("dim-label");
        sub.set_xalign(0.0);
        self.content.append(&sub);

        let (src, dst) = qr.paths();
        let paths = adw::ActionRow::new();
        paths.set_title("Paths");
        paths.set_subtitle(&format!(
            "{} → {}",
            src.unwrap_or_else(|| "—".into()),
            dst.unwrap_or_else(|| "—".into())
        ));
        self.content.append(&paths);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let start = gtk::Button::with_label("Start");
        let stop = gtk::Button::with_label("Stop");
        let edit = gtk::Button::with_label("Edit");
        let dup = gtk::Button::with_label("Duplicate");
        let delete = gtk::Button::with_label("Delete");
        delete.add_css_class("destructive-action");
        {
            let view = self.clone();
            let qr = qr.clone();
            start.connect_clicked(move |_| view.start_run(&qr));
        }
        {
            let view = self.clone();
            let qr = qr.clone();
            stop.connect_clicked(move |_| view.stop_run(&qr));
        }
        {
            let view = self.clone();
            let qr = qr.clone();
            edit.connect_clicked(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::quick_run_editor(&win, view.ctx.clone(), Some(qr.clone()), {
                        let view = view.clone();
                        Rc::new(move || view.refresh())
                    });
                }
            });
        }
        {
            let view = self.clone();
            let qr = qr.clone();
            dup.connect_clicked(move |_| {
                let mut clone = qr.clone();
                clone.id = uuid::Uuid::new_v4().to_string();
                clone.name = format!("{} copy", qr.name);
                clone.status = "idle".into();
                view.ctx.store.borrow_mut().quick_runs.push(clone);
                view.ctx.persist();
                view.refresh();
            });
        }
        {
            let view = self.clone();
            let id = qr.id.clone();
            delete.connect_clicked(move |_| {
                view.ctx
                    .store
                    .borrow_mut()
                    .quick_runs
                    .retain(|q| q.id != id);
                *view.ctx.selected_quick_run.borrow_mut() = None;
                view.ctx.persist();
                view.refresh();
            });
        }
        actions.append(&start);
        actions.append(&stop);
        actions.append(&edit);
        actions.append(&dup);
        actions.append(&delete);
        self.content.append(&actions);
    }

    fn start_run(&self, qr: &QuickRun) {
        let Some(client) = self.ctx.client() else {
            self.toast.add_toast(adw::Toast::new("Engine offline"));
            return;
        };
        match qr.operation_type {
            OperationType::Mount => {
                let (_, dest) = qr.paths();
                let point = dest.unwrap_or_else(|| {
                    format!(
                        "{}/mnt/{}",
                        dirs::home_dir().unwrap_or_default().display(),
                        qr.remote_name
                    )
                });
                let _ = std::fs::create_dir_all(&point);
                match client.mount(
                    &crate::rclone::remote_fs(&qr.remote_name, ""),
                    &point,
                    "mount",
                ) {
                    Ok(_) => self.set_status(&qr.id, "running"),
                    Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
            OperationType::Serve => {
                match client.serve_start(
                    "webdav",
                    &crate::rclone::remote_fs(&qr.remote_name, ""),
                    "127.0.0.1:0",
                ) {
                    Ok(_) => self.set_status(&qr.id, "running"),
                    Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
            other => {
                if let Some(endpoint) = other.rc_job_endpoint() {
                    let (src, dst) = qr.paths();
                    let params = serde_json::json!({
                        "srcFs": src.unwrap_or_else(|| crate::rclone::remote_fs(&qr.remote_name, "")),
                        "dstFs": dst.unwrap_or_else(|| crate::rclone::remote_fs(&qr.remote_name, "")),
                    });
                    match client.start_job(endpoint, params) {
                        Ok(id) => {
                            if let Some(run) = self
                                .ctx
                                .store
                                .borrow_mut()
                                .quick_runs
                                .iter_mut()
                                .find(|q| q.id == qr.id)
                            {
                                run.last_job_id = Some(id);
                                run.status = "running".into();
                                run.run_count += 1;
                            }
                            self.ctx.persist();
                            self.refresh();
                        }
                        Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
                    }
                }
            }
        }
    }

    fn stop_run(&self, qr: &QuickRun) {
        if let (Some(client), Some(jobid)) = (self.ctx.client(), qr.last_job_id) {
            let _ = client.job_stop(jobid);
        }
        self.set_status(&qr.id, "stopped");
    }

    fn set_status(&self, id: &str, status: &str) {
        if let Some(run) = self
            .ctx
            .store
            .borrow_mut()
            .quick_runs
            .iter_mut()
            .find(|q| q.id == id)
        {
            run.status = status.into();
        }
        self.ctx.persist();
        self.refresh();
    }
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(child));
    scroll
}
