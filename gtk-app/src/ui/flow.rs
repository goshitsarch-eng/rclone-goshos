use super::dialogs;
use super::AppCtx;
use crate::navigation::NavTarget;
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
        search.set_placeholder_text(Some(&ctx.t_or("flow.search", "Search quick runs")));
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&sidebar));
        let add = gtk::Button::with_label(&ctx.t_or("flow.quickRun.new", "New Quick Run"));
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
        let stack = adw::ViewStack::new();
        stack.add_titled_with_icon(
            &split,
            Some("quick_run"),
            &ctx.t_or("flow.tabs.quickRun", "Quick Run"),
            "media-playlist-consecutive-symbolic",
        );
        let builder = adw::StatusPage::new();
        builder.set_icon_name(Some("applications-engineering-symbolic"));
        builder.set_title(&ctx.t_or(
            "flow.builder.placeholderTitle",
            "Workflow Builder in Progress",
        ));
        builder.set_description(Some(&ctx.t_or(
            "flow.builder.placeholderMessage",
            "A visual canvas for multi-step rclone pipelines and conditional job chaining is coming soon.",
        )));
        let roadmap = gtk::LinkButton::with_label(
            "https://github.com/Zarestia-Dev/rclone-manager/issues/232",
            &ctx.t_or("flow.builder.viewRoadmap", "View Roadmap on GitHub"),
        );
        builder.set_child(Some(&roadmap));
        stack.add_titled_with_icon(
            &builder,
            Some("builder"),
            &ctx.t_or("flow.tabs.workflow", "Workflow"),
            "media-playlist-shuffle-symbolic",
        );
        let switcher = adw::ViewSwitcherBar::new();
        switcher.set_stack(Some(&stack));
        switcher.set_reveal(true);
        root.append(&stack);
        root.append(&switcher);

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

    pub fn select_quick_run(&self, id: Option<&str>) {
        *self.ctx.selected_quick_run.borrow_mut() =
            id.filter(|s| !s.is_empty()).map(|s| s.to_string());
        self.refresh();
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
            empty.set_title(&self.ctx.t_or("flow.quickRun.title", "No quick runs"));
            empty.set_description(Some(&self.ctx.t_or(
                "flow.empty.description",
                "Create a reusable rclone operation with cron, watcher, or autostart.",
            )));
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
        let layout = crate::layout::PanelLayout::from_value(
            &self.ctx.settings.borrow().runtime.quick_run_layout,
        );
        let visible = |id: &str| {
            layout
                .resolve(crate::layout::QUICK_RUN_PANELS)
                .into_iter()
                .any(|(panel, vis)| panel == id && vis)
        };
        let snap = self.ctx.snapshot.borrow().clone();
        let title = gtk::Label::new(Some(&self.ctx.t_or("flow.quickRun.title", "Quick Runs")));
        title.add_css_class("title-1");
        title.set_xalign(0.0);
        self.content.append(&title);
        if visible("quickRuns") {
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
        }

        if visible("jobs") {
            self.content.append(&heading(
                &self
                    .ctx
                    .t_or("generalOverview.panels.jobs", "Job Information"),
            ));
            let jobs = gtk::ListBox::new();
            jobs.add_css_class("boxed-list");
            if snap.jobs.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(
                    &self
                        .ctx
                        .t_or("generalOverview.jobs.noActive", "No active jobs"),
                );
                jobs.append(&row);
            } else {
                for job in &snap.jobs {
                    let row = adw::ActionRow::new();
                    row.set_title(&format!("{} · {}", job.operation, job.remote));
                    row.set_subtitle(&format!("{} · {:.0}%", job.status, job.progress * 100.0));
                    let ctx = self.ctx.clone();
                    let id = job.id;
                    row.connect_activated(move |_| {
                        ctx.request_nav(NavTarget::Job { id });
                    });
                    jobs.append(&row);
                }
            }
            self.content.append(&jobs);
        }

        if visible("serves") {
            self.content.append(&heading(
                &self
                    .ctx
                    .t_or("generalOverview.panels.serves", "Running Serves"),
            ));
            let serves = gtk::ListBox::new();
            serves.add_css_class("boxed-list");
            if snap.serves.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(
                    &self
                        .ctx
                        .t_or("generalOverview.serves.noActive", "No active serves"),
                );
                serves.append(&row);
            } else {
                for serve in &snap.serves {
                    let row = adw::ActionRow::new();
                    row.set_title(&format!("{} · {}", serve.serve_type, serve.fs));
                    row.set_subtitle(&serve.addr);
                    {
                        let ctx = self.ctx.clone();
                        let id = serve.id.clone();
                        row.connect_activated(move |_| {
                            ctx.request_nav(NavTarget::Serve { id: id.clone() });
                        });
                    }
                    serves.append(&row);
                }
            }
            self.content.append(&serves);
        }

        if visible("automations") {
            self.content.append(&heading(
                &self
                    .ctx
                    .t_or("generalOverview.panels.automations", "Automations"),
            ));
            let autos = gtk::ListBox::new();
            autos.add_css_class("boxed-list");
            let records = crate::automation::collect(&self.ctx.store.borrow());
            if records.is_empty() {
                let row = adw::ActionRow::new();
                row.set_title(
                    &self
                        .ctx
                        .t_or("generalOverview.automations.noScheduled", "No automations"),
                );
                autos.append(&row);
            } else {
                for record in records.into_iter().take(8) {
                    let paused = self.ctx.store.borrow().is_automation_paused(&record.id);
                    let row = adw::ActionRow::new();
                    row.set_title(&record.name);
                    row.set_subtitle(&if paused {
                        self.ctx.t_or("flow.quickRun.status.paused", "paused")
                    } else {
                        self.ctx.t_or("flow.quickRun.badges.scheduled", "scheduled")
                    });
                    {
                        let ctx = self.ctx.clone();
                        let id = record.id.clone();
                        row.connect_activated(move |_| {
                            ctx.request_nav(NavTarget::Automation { id: id.clone() });
                        });
                    }
                    autos.append(&row);
                }
            }
            self.content.append(&autos);
        }
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
        paths.set_title(&self.ctx.t_or("modals.jobDetail.sections.paths", "Paths"));
        paths.set_subtitle(&format!(
            "{} → {}",
            src.unwrap_or_else(|| "—".into()),
            dst.unwrap_or_else(|| "—".into())
        ));
        self.content.append(&paths);

        let dry = adw::SwitchRow::new();
        dry.set_title(&self.ctx.t_or("dashboard.appDetail.dryRun", "Dry run"));
        dry.set_active(crate::jobs::is_dry_run(&qr.config.rclone));
        {
            let ctx = self.ctx.clone();
            let id = qr.id.clone();
            dry.connect_active_notify(move |row| {
                if let Some(run) = ctx
                    .store
                    .borrow_mut()
                    .quick_runs
                    .iter_mut()
                    .find(|q| q.id == id)
                {
                    crate::jobs::apply_session_flags(
                        &mut run.config.rclone,
                        row.is_active(),
                        false,
                    );
                    if !row.is_active() {
                        if let Some(obj) = run.config.rclone.as_object_mut() {
                            obj.remove("DryRun");
                            obj.remove("dryRun");
                        }
                    }
                }
                ctx.persist();
            });
        }
        self.content.append(&dry);

        let tray = adw::SwitchRow::new();
        tray.set_title(&self.ctx.t_or("flow.quickRun.showInTray", "Show in tray"));
        tray.set_active(qr.show_on_tray);
        {
            let ctx = self.ctx.clone();
            let id = qr.id.clone();
            tray.connect_active_notify(move |row| {
                if let Some(run) = ctx
                    .store
                    .borrow_mut()
                    .quick_runs
                    .iter_mut()
                    .find(|q| q.id == id)
                {
                    run.show_on_tray = row.is_active();
                }
                ctx.persist();
            });
        }
        self.content.append(&tray);

        if let Some(job) = qr.last_job_id.and_then(|id| {
            self.ctx
                .snapshot
                .borrow()
                .jobs
                .iter()
                .find(|j| j.id == id)
                .cloned()
                .or_else(|| {
                    self.ctx
                        .store
                        .borrow()
                        .job_history
                        .iter()
                        .find(|j| j.id == id)
                        .cloned()
                })
        }) {
            let stats = adw::PreferencesGroup::new();
            stats.set_title(
                &self
                    .ctx
                    .t_or("modals.jobDetail.sections.overview", "Last job"),
            );
            for (title, value) in [
                (
                    self.ctx.t_or("modals.jobDetail.fields.status", "Status"),
                    job.status.clone(),
                ),
                (
                    self.ctx.t_or("modals.jobDetail.fields.speed", "Speed"),
                    format!(
                        "{:.1} KiB/s",
                        crate::jobs::stats_f64(&job.stats, &["speed"]) / 1024.0
                    ),
                ),
                (
                    self.ctx
                        .t_or("modals.jobDetail.fields.transferred", "Transferred"),
                    crate::rclone::format_bytes(crate::jobs::stats_i64(&job.stats, &["bytes"])),
                ),
                (
                    self.ctx.t_or("modals.jobDetail.fields.files", "Files"),
                    crate::jobs::stats_i64(&job.stats, &["transfers"]).to_string(),
                ),
            ] {
                let row = adw::ActionRow::new();
                row.set_title(&title);
                row.set_subtitle(&value);
                stats.add(&row);
            }
            let open =
                gtk::Button::with_label(&self.ctx.t_or("modals.jobDetail.title", "Job detail"));
            let logs = gtk::Button::with_label(&self.ctx.t_or("logs.title", "View logs"));
            {
                let ctx = self.ctx.clone();
                let id = job.id;
                open.connect_clicked(move |_| {
                    ctx.request_nav(NavTarget::Job { id });
                });
            }
            {
                let view = self.clone();
                let remote = qr.remote_name.clone();
                logs.connect_clicked(move |_| {
                    if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                        dialogs::logs(&win, view.ctx.clone(), Some(remote.clone()));
                    }
                });
            }
            let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            buttons.append(&open);
            buttons.append(&logs);
            self.content.append(&stats);
            self.content.append(&buttons);
        } else {
            let logs = gtk::Button::with_label(&self.ctx.t_or("logs.title", "View logs"));
            {
                let view = self.clone();
                let remote = qr.remote_name.clone();
                logs.connect_clicked(move |_| {
                    if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                        dialogs::logs(&win, view.ctx.clone(), Some(remote.clone()));
                    }
                });
            }
            self.content.append(&logs);
        }

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let start = gtk::Button::with_label(&self.ctx.t_or("flow.quickRun.actions.start", "Start"));
        let stop = gtk::Button::with_label(&self.ctx.t_or("flow.quickRun.actions.stop", "Stop"));
        let edit = gtk::Button::with_label(&self.ctx.t_or("common.edit", "Edit"));
        let dup = gtk::Button::with_label(&self.ctx.t_or("common.duplicate", "Duplicate"));
        let delete = gtk::Button::with_label(&self.ctx.t_or("common.delete", "Delete"));
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
            self.toast.add_toast(adw::Toast::new(
                &self.ctx.t_or("home.errors.engineOffline", "Engine offline"),
            ));
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
                    Err(e) => self
                        .toast
                        .add_toast(adw::Toast::new(&self.ctx.translate_error(&e.to_string()))),
                }
            }
            OperationType::Serve => {
                match client.serve_start(
                    "webdav",
                    &crate::rclone::remote_fs(&qr.remote_name, ""),
                    "127.0.0.1:0",
                ) {
                    Ok(_) => self.set_status(&qr.id, "running"),
                    Err(e) => self
                        .toast
                        .add_toast(adw::Toast::new(&self.ctx.translate_error(&e.to_string()))),
                }
            }
            other => {
                let meta = self
                    .ctx
                    .store
                    .borrow()
                    .remotes
                    .get(&qr.remote_name)
                    .cloned();
                match crate::jobs::start_profile(
                    &client,
                    &qr.remote_name,
                    other,
                    &qr.config,
                    meta.as_ref(),
                    "flow",
                ) {
                    Ok(id) => {
                        crate::jobs::remember_started(
                            &mut self.ctx.store.borrow_mut().job_meta,
                            &id,
                            crate::jobs::job_meta_for(
                                &qr.remote_name,
                                &qr.config,
                                "flow",
                                &self.ctx.backend_key(),
                                &qr.id,
                            ),
                        );
                        if let Some(run) = self
                            .ctx
                            .store
                            .borrow_mut()
                            .quick_runs
                            .iter_mut()
                            .find(|q| q.id == qr.id)
                        {
                            run.status = "running".into();
                            run.run_count += 1;
                            if let Some(num) = id.trim_start_matches('#').split(',').next() {
                                run.last_job_id = num.trim().parse().ok();
                            }
                        }
                        self.ctx.persist();
                        self.refresh();
                    }
                    Err(e) => self
                        .toast
                        .add_toast(adw::Toast::new(&self.ctx.translate_error(&e))),
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

fn heading(text: &str) -> gtk::Label {
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
