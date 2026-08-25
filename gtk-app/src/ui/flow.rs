use super::dialogs;
use super::AppCtx;
use crate::navigation::NavTarget;
use crate::store::QuickRun;
use adw::prelude::*;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct FlowView {
    pub root: gtk::Box,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    sidebar: gtk::ListBox,
    search: gtk::SearchEntry,
    content: gtk::Box,
    editing_layout: Rc<RefCell<bool>>,
    remote_filter: Rc<RefCell<Option<String>>>,
    selected_flow_remote: Rc<RefCell<Option<String>>>,
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
            editing_layout: Rc::new(RefCell::new(false)),
            remote_filter: Rc::new(RefCell::new(None)),
            selected_flow_remote: Rc::new(RefCell::new(None)),
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
        if id.is_some() {
            *self.selected_flow_remote.borrow_mut() = None;
        }
        self.refresh();
    }

    fn open_remote_detail(&self, remote: &str) {
        let clean = remote.trim_end_matches(':');
        if clean.is_empty() {
            return;
        }
        *self.ctx.selected_quick_run.borrow_mut() = None;
        *self.selected_flow_remote.borrow_mut() = Some(clean.to_string());
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
        let remote_filter = self.remote_filter.borrow().clone();
        let runs = self.ctx.store.borrow().quick_runs.clone();
        let filtered: Vec<QuickRun> = runs
            .into_iter()
            .filter(|qr| {
                if let Some(remote) = &remote_filter {
                    if qr.remote_name != *remote {
                        return false;
                    }
                }
                if query.is_empty() {
                    return true;
                }
                qr.name.to_lowercase().contains(&query)
                    || qr.remote_name.to_lowercase().contains(&query)
                    || qr.operation_type.as_str().contains(&query)
            })
            .collect();

        if filtered.is_empty() && remote_filter.is_none() {
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
                *view.selected_flow_remote.borrow_mut() = None;
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
        if let Some(remote) = self.selected_flow_remote.borrow().clone() {
            self.fill_remote_detail(&remote);
            return;
        }
        self.fill_overview(&filtered);
    }

    fn fill_overview(&self, runs: &[QuickRun]) {
        let editing = *self.editing_layout.borrow();
        let layout = crate::layout::PanelLayout::from_value(
            &self.ctx.settings.borrow().runtime.quick_run_layout,
        );
        let snap = self.ctx.snapshot.borrow().clone();
        let title = gtk::Label::new(Some(&self.ctx.t_or("flow.quickRun.title", "Quick Runs")));
        title.add_css_class("title-1");
        title.set_xalign(0.0);
        self.content.append(&title);
        let remotes: Vec<String> = {
            let mut names: Vec<String> = self
                .ctx
                .store
                .borrow()
                .quick_runs
                .iter()
                .map(|qr| qr.remote_name.clone())
                .filter(|name| !name.is_empty())
                .collect();
            names.sort();
            names.dedup();
            names
        };
        if remotes.len() > 1 {
            let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            chips.add_css_class("linked");
            let selected = self.remote_filter.borrow().clone();
            let all = gtk::ToggleButton::with_label(&self.ctx.t_or("common.all", "All"));
            all.set_active(selected.is_none());
            {
                let view = self.clone();
                all.connect_clicked(move |_| {
                    *view.remote_filter.borrow_mut() = None;
                    view.refresh();
                });
            }
            chips.append(&all);
            let mut group_anchor = all.clone();
            for remote in remotes {
                let btn = gtk::ToggleButton::with_label(&remote);
                btn.set_group(Some(&group_anchor));
                group_anchor = btn.clone();
                btn.set_active(selected.as_deref() == Some(remote.as_str()));
                {
                    let view = self.clone();
                    let remote = remote.clone();
                    btn.connect_clicked(move |_| {
                        *view.remote_filter.borrow_mut() = Some(remote.clone());
                        view.refresh();
                    });
                }
                chips.append(&btn);
            }
            self.content.append(&chips);
        }
        let layout_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let edit_label = if editing {
            self.ctx.t_or("common.done", "Done")
        } else {
            self.ctx.t_or("generalOverview.editLayout", "Edit layout")
        };
        let edit_btn = gtk::Button::with_label(&edit_label);
        edit_btn.set_tooltip_text(Some(&self.ctx.t_or(
            "generalOverview.editLayout",
            "Hide or reorder overview panels",
        )));
        {
            let view = self.clone();
            edit_btn.connect_clicked(move |_| {
                let next = !*view.editing_layout.borrow();
                *view.editing_layout.borrow_mut() = next;
                view.refresh();
            });
        }
        let reset =
            gtk::Button::with_label(&self.ctx.t_or("generalOverview.resetPanels", "Reset panels"));
        reset.set_tooltip_text(Some(&self.ctx.t_or(
            "generalOverview.resetPanels",
            "Restore the default overview panel order",
        )));
        {
            let view = self.clone();
            reset.connect_clicked(move |_| {
                view.ctx.settings.borrow_mut().runtime.quick_run_layout =
                    serde_json::json!({ "order": [], "hidden": [] });
                view.ctx.persist();
                view.refresh();
            });
        }
        layout_bar.append(&edit_btn);
        layout_bar.append(&reset);
        self.content.append(&layout_bar);
        for (id, visible) in layout.resolve(crate::layout::QUICK_RUN_PANELS) {
            if !visible && !editing {
                continue;
            }
            match id.as_str() {
                "quickRuns" => {
                    let list = gtk::ListBox::new();
                    list.add_css_class("boxed-list");
                    for qr in runs {
                        let row = adw::ActionRow::new();
                        row.set_title(&qr.name);
                        let mut badges =
                            vec![qr.operation_type.as_str().to_string(), qr.status.clone()];
                        if qr.config.app.cron_enabled {
                            badges.push(self.ctx.t_or("flow.quickRun.badges.cron", "cron"));
                        }
                        if qr.config.app.watch_enabled {
                            badges.push(self.ctx.t_or("flow.quickRun.badges.watcher", "watch"));
                        }
                        if qr.config.app.auto_start {
                            badges
                                .push(self.ctx.t_or("flow.quickRun.badges.autostart", "autostart"));
                        }
                        row.set_subtitle(&format!("{} · {}", qr.remote_name, badges.join(" · ")));
                        let remote_btn = gtk::Button::with_label(&qr.remote_name);
                        remote_btn.set_valign(gtk::Align::Center);
                        remote_btn.set_tooltip_text(Some(
                            &self
                                .ctx
                                .t_or("flow.quickRun.openRemote", "Open remote detail"),
                        ));
                        {
                            let view = self.clone();
                            let remote = qr.remote_name.clone();
                            remote_btn.connect_clicked(move |_| view.open_remote_detail(&remote));
                        }
                        row.add_suffix(&remote_btn);
                        let start = gtk::Button::from_icon_name("media-playback-start-symbolic");
                        start.set_valign(gtk::Align::Center);
                        start.set_tooltip_text(Some(
                            &self.ctx.t_or("flow.quickRun.actions.start", "Start"),
                        ));
                        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
                        stop.set_valign(gtk::Align::Center);
                        stop.set_tooltip_text(Some(
                            &self.ctx.t_or("flow.quickRun.actions.stop", "Stop"),
                        ));
                        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
                        edit.set_valign(gtk::Align::Center);
                        edit.set_tooltip_text(Some(&self.ctx.t_or("common.edit", "Edit")));
                        let busy =
                            self.ctx
                                .is_busy(&qr.remote_name, qr.operation_type.as_str(), &qr.id);
                        start.set_sensitive(!busy);
                        stop.set_sensitive(!busy);
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
                                    dialogs::quick_run_editor(
                                        &win,
                                        view.ctx.clone(),
                                        Some(qr.clone()),
                                        {
                                            let view = view.clone();
                                            Rc::new(move || view.refresh())
                                        },
                                    );
                                }
                            });
                        }
                        row.add_suffix(&start);
                        row.add_suffix(&stop);
                        row.add_suffix(&edit);
                        let (src, dst) = qr.paths();
                        let mut open_paths = Vec::new();
                        if let Some(src) = src {
                            open_paths.extend(crate::jobs::split_job_paths(&src));
                        }
                        if let Some(dst) = dst {
                            open_paths.extend(crate::jobs::split_job_paths(&dst));
                        }
                        for path in open_paths {
                            let folder = gtk::Button::from_icon_name("folder-open-symbolic");
                            folder.set_valign(gtk::Align::Center);
                            folder.set_tooltip_text(Some(&path));
                            let ctx = self.ctx.clone();
                            let remote = qr.remote_name.clone();
                            folder.connect_clicked(move |_| ctx.open_typed_path(&remote, &path));
                            row.add_suffix(&folder);
                        }
                        list.append(&row);
                    }
                    self.append_expandable(
                        "quickRuns",
                        &self.ctx.t_or("flow.quickRun.title", "Quick Runs"),
                        &list,
                    );
                }
                "jobs" => {
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
                            row.set_subtitle(&format!(
                                "{} · {:.0}%",
                                job.status,
                                job.progress * 100.0
                            ));
                            let ctx = self.ctx.clone();
                            let id = job.id;
                            row.connect_activated(move |_| {
                                ctx.request_nav(NavTarget::Job { id });
                            });
                            jobs.append(&row);
                        }
                    }
                    self.append_expandable(
                        "jobs",
                        &self
                            .ctx
                            .t_or("generalOverview.panels.jobs", "Job Information"),
                        &jobs,
                    );
                }
                "serves" => {
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
                    self.append_expandable(
                        "serves",
                        &self
                            .ctx
                            .t_or("generalOverview.panels.serves", "Running Serves"),
                        &serves,
                    );
                }
                "automations" => {
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
                    self.append_expandable(
                        "automations",
                        &self
                            .ctx
                            .t_or("generalOverview.panels.automations", "Automations"),
                        &autos,
                    );
                }
                "bandwidth" => self.append_bandwidth_panel(),
                "system" => self.append_system_panel(&snap),
                _ => {}
            }
        }
    }

    fn append_bandwidth_panel(&self) {
        let group = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let limit = self.ctx.settings.borrow().core.bandwidth_limit.clone();
        let current = adw::ActionRow::new();
        current.set_title(
            &self
                .ctx
                .t_or("dashboard.bandwidth.savedLimit", "Saved limit"),
        );
        current.set_subtitle(&if limit.is_empty() || limit == "off" {
            self.ctx.t_or("dashboard.bandwidth.unlimited", "Unlimited")
        } else {
            limit.clone()
        });
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.append(&current);
        if let Some(live) = self
            .ctx
            .client()
            .and_then(|c| c.bwlimit(None).ok())
            .map(|v| crate::jobs::parse_bwlimit(&v))
        {
            let live_row = adw::ActionRow::new();
            live_row.set_title(&self.ctx.t_or("dashboard.bandwidth.liveLimit", "Live limit"));
            live_row.set_subtitle(&format!(
                "{} · tx {}/s · rx {}/s",
                if live.rate == "off" {
                    self.ctx.t_or("dashboard.bandwidth.unlimited", "Unlimited")
                } else {
                    live.rate.clone()
                },
                crate::rclone::format_bytes(live.bytes_per_sec_tx),
                crate::rclone::format_bytes(live.bytes_per_sec_rx)
            ));
            list.append(&live_row);
        }
        group.append(&list);
        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        presets.add_css_class("linked");
        for (value, label) in crate::jobs::BANDWIDTH_PRESETS {
            let btn = gtk::Button::with_label(label);
            let ctx = self.ctx.clone();
            let view = self.clone();
            let value = (*value).to_string();
            btn.connect_clicked(move |_| {
                apply_bandwidth(&ctx, &value);
                view.refresh();
            });
            presets.append(&btn);
        }
        group.append(&presets);
        let custom = adw::EntryRow::new();
        custom.set_title(&self.ctx.t_or(
            "dashboard.bandwidth.customLimit",
            "Custom limit (e.g. 2M or 1M:10M)",
        ));
        custom.set_text(&limit);
        let apply = gtk::Button::with_label(&self.ctx.t_or("common.apply", "Apply"));
        apply.set_valign(gtk::Align::Center);
        {
            let ctx = self.ctx.clone();
            let view = self.clone();
            let custom = custom.clone();
            apply.connect_clicked(move |_| {
                apply_bandwidth(&ctx, &custom.text());
                view.refresh();
            });
        }
        custom.add_suffix(&apply);
        group.append(&custom);
        self.append_expandable(
            "bandwidth",
            &self
                .ctx
                .t_or("generalOverview.panels.bandwidth", "Bandwidth"),
            &group,
        );
    }

    fn append_system_panel(&self, snap: &crate::store::RuntimeSnapshot) {
        let sys = gtk::ListBox::new();
        sys.add_css_class("boxed-list");
        let version = self
            .ctx
            .engine
            .borrow()
            .as_ref()
            .map(|e| e.version.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| self.ctx.client().and_then(|c| c.version().ok()))
            .unwrap_or_else(|| self.ctx.t_or("generalOverview.system.unknown", "unknown"));
        let ver_row = adw::ActionRow::new();
        ver_row.set_title(&self.ctx.t_or("generalOverview.system.version", "rclone"));
        ver_row.set_subtitle(&version);
        sys.append(&ver_row);
        if let Some(client) = self.ctx.client() {
            if let Ok(pid) = client.pid() {
                let row = adw::ActionRow::new();
                row.set_title(&self.ctx.t_or("dashboard.system.pid", "rclone PID"));
                row.set_subtitle(&pid.to_string());
                sys.append(&row);
            }
            if let Ok(mem) = client.memstats() {
                let alloc = mem.get("Alloc").and_then(|x| x.as_i64()).unwrap_or(0);
                let sys_bytes = mem.get("Sys").and_then(|x| x.as_i64()).unwrap_or(0);
                let row = adw::ActionRow::new();
                row.set_title(&self.ctx.t_or("dashboard.system.memory", "Memory"));
                row.set_subtitle(&format!(
                    "{} alloc · {} sys",
                    crate::rclone::format_bytes(alloc),
                    crate::rclone::format_bytes(sys_bytes)
                ));
                sys.append(&row);
            }
        }
        let activity = adw::ActionRow::new();
        activity.set_title(&self.ctx.t_or("dashboard.system.activity", "Activity"));
        activity.set_subtitle(&format!(
            "{} running jobs · {} mounts · {} serves",
            snap.jobs
                .iter()
                .filter(|j| crate::jobs::is_overview_job(j) && j.status == "running")
                .count(),
            snap.mounts.len(),
            snap.serves.len()
        ));
        sys.append(&activity);
        self.append_expandable(
            "system",
            &self.ctx.t_or("generalOverview.panels.system", "System"),
            &sys,
        );
    }

    fn append_expandable(&self, id: &str, title: &str, child: &impl IsA<gtk::Widget>) {
        let expander = gtk::Expander::new(None);
        let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("heading");
        label.append(&heading);
        if *self.editing_layout.borrow() {
            let hide = gtk::Button::from_icon_name("view-conceal-symbolic");
            hide.set_tooltip_text(Some(
                &self
                    .ctx
                    .t_or("generalOverview.hidePanel", "Hide or show this panel"),
            ));
            let up = gtk::Button::from_icon_name("go-up-symbolic");
            up.set_tooltip_text(Some(
                &self.ctx.t_or("generalOverview.moveUp", "Move panel up"),
            ));
            let down = gtk::Button::from_icon_name("go-down-symbolic");
            down.set_tooltip_text(Some(
                &self.ctx.t_or("generalOverview.moveDown", "Move panel down"),
            ));
            {
                let view = self.clone();
                let id = id.to_string();
                hide.connect_clicked(move |_| view.toggle_panel(&id));
            }
            {
                let view = self.clone();
                let id = id.to_string();
                up.connect_clicked(move |_| view.move_panel(&id, -1));
            }
            {
                let view = self.clone();
                let id = id.to_string();
                down.connect_clicked(move |_| view.move_panel(&id, 1));
            }
            label.append(&hide);
            label.append(&up);
            label.append(&down);
        }
        expander.set_label_widget(Some(&label));
        expander.set_expanded(crate::settings::panel_is_open(
            &self.ctx.settings.borrow().runtime.panel_open_states,
            "flow",
            id,
        ));
        expander.set_child(Some(child));
        {
            let ctx = self.ctx.clone();
            let id = id.to_string();
            expander.connect_expanded_notify(move |exp| {
                if crate::settings::set_panel_open(
                    &mut ctx.settings.borrow_mut().runtime.panel_open_states,
                    "flow",
                    &id,
                    exp.is_expanded(),
                ) {
                    ctx.persist();
                }
            });
        }
        self.content.append(&expander);
    }

    fn flow_layout(&self) -> crate::layout::PanelLayout {
        crate::layout::PanelLayout::from_value(&self.ctx.settings.borrow().runtime.quick_run_layout)
    }

    fn write_flow_layout(&self, layout: crate::layout::PanelLayout) {
        self.ctx.settings.borrow_mut().runtime.quick_run_layout = layout.to_value();
        self.ctx.persist();
        self.refresh();
    }

    fn toggle_panel(&self, id: &str) {
        let mut layout = self.flow_layout();
        layout.toggle_hidden(id);
        self.write_flow_layout(layout);
    }

    fn move_panel(&self, id: &str, delta: isize) {
        let mut layout = self.flow_layout();
        layout.move_panel(id, delta, crate::layout::QUICK_RUN_PANELS);
        self.write_flow_layout(layout);
    }

    fn fill_remote_detail(&self, name: &str) {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some(&self.ctx.t_or("common.back", "Back to overview")));
        {
            let view = self.clone();
            back.connect_clicked(move |_| {
                *view.selected_flow_remote.borrow_mut() = None;
                view.refresh();
            });
        }
        let title = gtk::Label::new(Some(name));
        title.add_css_class("title-2");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        header.append(&back);
        let snap = self.ctx.snapshot.borrow().clone();
        let remote = snap.remotes.iter().find(|r| r.name == name).cloned();
        if let Some(info) = remote.as_ref() {
            header.append(&gtk::Image::from_icon_name(
                crate::providers::provider_icon(&info.r#type),
            ));
        }
        header.append(&title);
        self.content.append(&header);
        if let Some(info) = remote.as_ref() {
            let subtitle = gtk::Label::new(Some(&format!(
                "{} · {}",
                info.r#type,
                if info.mounted {
                    self.ctx.t_or("overviews.status.labels.mounted", "Mounted")
                } else if info.serving {
                    self.ctx.t_or("overviews.status.labels.serving", "Serving")
                } else if info.job_active {
                    self.ctx.t_or("overviews.status.labels.active", "Active")
                } else {
                    self.ctx
                        .t_or("overviews.status.labels.inactive", "Inactive")
                }
            )));
            subtitle.add_css_class("dim-label");
            subtitle.set_xalign(0.0);
            self.content.append(&subtitle);
        }

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let browse = gtk::Button::with_label(&self.ctx.t_or("common.browse", "Browse"));
        {
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            browse.connect_clicked(move |_| ctx.request_browse(&remote, ""));
        }
        let configure = gtk::Button::with_label(&self.ctx.t_or("common.edit", "Edit remote"));
        {
            let view = self.clone();
            let remote = name.to_string();
            configure.connect_clicked(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::remote_config(&win, view.ctx.clone(), Some(remote.clone()), {
                        let view = view.clone();
                        Rc::new(move || view.refresh())
                    });
                }
            });
        }
        let helpers =
            gtk::Button::with_label(&self.ctx.t_or("remote.editHelpers", "Edit helper profiles"));
        {
            let view = self.clone();
            let remote = name.to_string();
            helpers.connect_clicked(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::helper_profiles(&win, view.ctx.clone(), &remote);
                }
            });
        }
        actions.append(&browse);
        actions.append(&configure);
        actions.append(&helpers);
        self.content.append(&actions);

        let usage = adw::ActionRow::new();
        usage.set_title(&self.ctx.t_or("remote.diskUsage", "Disk usage"));
        if let Some(client) = self.ctx.client() {
            match client.about(&crate::rclone::remote_fs(name, "")) {
                Ok(about) => usage.set_subtitle(&crate::store::disk_label_from_about(&about)),
                Err(err) => usage.set_subtitle(&err.to_string()),
            }
        } else {
            usage.set_subtitle(&self.ctx.t_or(
                "notification.title.engineConnectionFailed",
                "Engine Connection Error",
            ));
        }
        self.content.append(&usage);

        self.content.append(&{
            let label =
                gtk::Label::new(Some(&self.ctx.t_or("generalOverview.activity", "Activity")));
            label.add_css_class("heading");
            label.set_xalign(0.0);
            label
        });
        let activity = gtk::ListBox::new();
        activity.add_css_class("boxed-list");
        let jobs: Vec<_> = snap
            .jobs
            .iter()
            .filter(|j| j.remote == name)
            .cloned()
            .collect();
        if jobs.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or(
                "generalOverview.jobs.noRunning",
                "No active jobs for this remote",
            ));
            activity.append(&row);
        } else {
            for job in jobs {
                let row = adw::ActionRow::new();
                row.set_title(&format!("{} · {}", job.operation, job.status));
                row.set_subtitle(&format!("{:.0}% · {}", job.progress * 100.0, job.profile));
                let id = job.id;
                {
                    let ctx = self.ctx.clone();
                    row.connect_activated(move |_| {
                        ctx.request_nav(NavTarget::Job { id });
                    });
                }
                if crate::jobs::job_is_running(&job) || crate::jobs::job_is_pending(&job) {
                    let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
                    stop.set_valign(gtk::Align::Center);
                    let ctx = self.ctx.clone();
                    let view = self.clone();
                    stop.connect_clicked(move |_| {
                        if let Some(c) = ctx.client() {
                            let _ = c.job_stop(id);
                            ctx.refresh_runtime();
                            view.refresh();
                        }
                    });
                    row.add_suffix(&stop);
                }
                activity.append(&row);
            }
        }
        self.content.append(&activity);

        self.content.append(&{
            let label = gtk::Label::new(Some(
                &self
                    .ctx
                    .t_or("remote.quickRuns", "Quick Runs for this remote"),
            ));
            label.add_css_class("heading");
            label.set_xalign(0.0);
            label
        });
        let qlist = gtk::ListBox::new();
        qlist.add_css_class("boxed-list");
        for qr in self
            .ctx
            .store
            .borrow()
            .quick_runs
            .iter()
            .filter(|q| q.remote_name == name)
            .cloned()
        {
            let row = adw::ActionRow::new();
            row.set_title(&qr.name);
            row.set_subtitle(&format!("{} · {}", qr.operation_type.as_str(), qr.status));
            {
                let view = self.clone();
                let id = qr.id.clone();
                row.connect_activated(move |_| view.select_quick_run(Some(&id)));
            }
            qlist.append(&row);
        }
        if qlist.first_child().is_none() {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or("dashboard.quickRuns.empty", "No quick runs"));
            qlist.append(&row);
        }
        self.content.append(&qlist);
        let add_qr = gtk::Button::with_label(&self.ctx.t_or(
            "dashboard.quickRuns.createForRemote",
            "Create quick run for this remote",
        ));
        {
            let view = self.clone();
            let remote = name.to_string();
            add_qr.connect_clicked(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    let draft = QuickRun::new(
                        String::new(),
                        crate::operations::OperationType::Sync,
                        remote.clone(),
                    );
                    dialogs::quick_run_editor(&win, view.ctx.clone(), Some(draft), {
                        let view = view.clone();
                        Rc::new(move || view.refresh())
                    });
                }
            });
        }
        self.content.append(&add_qr);
    }

    fn fill_detail(&self, qr: &QuickRun) {
        let title = gtk::Label::new(Some(&qr.name));
        title.add_css_class("title-1");
        title.set_xalign(0.0);
        self.content.append(&title);
        if !qr.description.is_empty() {
            let desc = gtk::Label::new(Some(&qr.description));
            desc.add_css_class("dim-label");
            desc.set_xalign(0.0);
            desc.set_wrap(true);
            self.content.append(&desc);
        }
        let sub = gtk::Label::new(Some(&format!(
            "{} · {}",
            qr.operation_type.api_label(),
            qr.status
        )));
        sub.add_css_class("dim-label");
        sub.set_xalign(0.0);
        self.content.append(&sub);
        let remote_btn = gtk::Button::with_label(&qr.remote_name);
        remote_btn.set_halign(gtk::Align::Start);
        remote_btn.set_tooltip_text(Some(
            &self
                .ctx
                .t_or("flow.quickRun.openRemote", "Open remote detail"),
        ));
        {
            let view = self.clone();
            let remote = qr.remote_name.clone();
            remote_btn.connect_clicked(move |_| view.open_remote_detail(&remote));
        }
        self.content.append(&remote_btn);

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
                clone.last_job_id = None;
                clone.run_count = 0;
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::quick_run_editor(&win, view.ctx.clone(), Some(clone), {
                        let view = view.clone();
                        Rc::new(move || view.refresh())
                    });
                }
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
        let Some(_guard) = self
            .ctx
            .busy_guard(&qr.remote_name, qr.operation_type.as_str(), &qr.id)
        else {
            self.toast.add_toast(adw::Toast::new(
                &self
                    .ctx
                    .t_or("remote.actionInProgress", "Action already in progress"),
            ));
            return;
        };
        let Some(client) = self.ctx.client() else {
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "notification.title.engineConnectionFailed",
                "Engine Connection Error",
            )));
            return;
        };
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
            qr.operation_type,
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
                self.ctx.store.borrow_mut().log_operation(
                    &qr.remote_name,
                    qr.operation_type.as_str(),
                    &format!("started quick run {id}"),
                    Some(&crate::restrict::redact_value(&qr.config.rclone)),
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

    fn stop_run(&self, qr: &QuickRun) {
        let Some(_guard) = self
            .ctx
            .busy_guard(&qr.remote_name, qr.operation_type.as_str(), &qr.id)
        else {
            self.toast.add_toast(adw::Toast::new(
                &self
                    .ctx
                    .t_or("remote.actionInProgress", "Action already in progress"),
            ));
            return;
        };
        if let Some(run) = self
            .ctx
            .store
            .borrow_mut()
            .quick_runs
            .iter_mut()
            .find(|q| q.id == qr.id)
        {
            run.status = "stopping".into();
        }
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

fn apply_bandwidth(ctx: &AppCtx, value: &str) {
    let rate = crate::jobs::normalize_bandwidth(value);
    ctx.settings.borrow_mut().core.bandwidth_limit = if rate == "off" {
        String::new()
    } else {
        rate.clone()
    };
    ctx.persist();
    ctx.apply_effective_bandwidth();
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(child));
    scroll
}
