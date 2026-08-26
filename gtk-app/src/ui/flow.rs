use super::automation_card;
use super::dialogs;
use super::job_panels;
use super::operation_control;
use super::quick_run_card;
use super::AppCtx;
use crate::navigation::NavTarget;
use crate::store::QuickRun;
use adw::prelude::*;
use gtk::glib;
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
    content_scroll: gtk::ScrolledWindow,
    editing_layout: Rc<RefCell<bool>>,
    remote_filter: Rc<RefCell<Option<String>>>,
    selected_flow_remote: Rc<RefCell<Option<String>>>,
    detail_page: Rc<RefCell<String>>,
    origin_filter: Rc<RefCell<String>>,
    transfer_query: Rc<RefCell<String>>,
    transfer_tab: Rc<RefCell<String>>,
    split: adw::OverlaySplitView,
}

impl FlowView {
    pub fn new(ctx: AppCtx, toast: adw::ToastOverlay) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let split = adw::OverlaySplitView::new();
        split.set_min_sidebar_width(260.0);
        split.set_show_sidebar(ctx.settings.borrow().runtime.flow_sidebar_open);

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
        let content_scroll = scrolled(&content);
        let side_toggle = gtk::Button::from_icon_name("sidebar-show-symbolic");
        side_toggle.set_tooltip_text(Some(&ctx.t_or("sidebar.toggleSidebar", "Toggle Sidebar")));
        side_toggle.set_halign(gtk::Align::Start);
        let content_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        content_header.set_margin_top(6);
        content_header.set_margin_start(8);
        content_header.set_margin_end(8);
        content_header.append(&side_toggle);
        let content_wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_wrap.append(&content_header);
        content_wrap.append(&content_scroll);
        split.set_content(Some(&content_wrap));
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
            content_scroll,
            editing_layout: Rc::new(RefCell::new(false)),
            remote_filter: Rc::new(RefCell::new(None)),
            selected_flow_remote: Rc::new(RefCell::new(None)),
            detail_page: Rc::new(RefCell::new("monitoring".into())),
            origin_filter: Rc::new(RefCell::new("all".into())),
            transfer_query: Rc::new(RefCell::new(String::new())),
            transfer_tab: Rc::new(RefCell::new("active".into())),
            split,
        };
        {
            let view = view.clone();
            side_toggle.connect_clicked(move |_| view.toggle_sidebar());
        }

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

    fn toggle_sidebar(&self) {
        let next = !self.split.shows_sidebar();
        self.split.set_show_sidebar(next);
        self.ctx.settings.borrow_mut().runtime.flow_sidebar_open = next;
        self.ctx.persist();
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
        let scroll_y = self.content_scroll.vadjustment().value();
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

        if let Some(id) = self.ctx.selected_quick_run.borrow().clone() {
            if let Some(qr) = self
                .ctx
                .store
                .borrow()
                .quick_runs
                .iter()
                .find(|q| q.id == id)
                .cloned()
            {
                for item in &filtered {
                    self.append_sidebar_quick_run(item);
                }
                if filtered.iter().all(|item| item.id != qr.id) {
                    self.append_sidebar_quick_run(&qr);
                }
                self.fill_detail(&qr);
                self.restore_content_scroll(scroll_y);
                return;
            }
        }
        if let Some(remote) = self.selected_flow_remote.borrow().clone() {
            for item in &filtered {
                self.append_sidebar_quick_run(item);
            }
            self.fill_remote_detail(&remote);
            self.restore_content_scroll(scroll_y);
            return;
        }
        if filtered.is_empty() && remote_filter.is_none() {
            self.sidebar.append(&adw::ActionRow::new());
            self.fill_overview(&[]);
            self.restore_content_scroll(scroll_y);
            return;
        }

        for qr in &filtered {
            self.append_sidebar_quick_run(qr);
        }
        self.fill_overview(&filtered);
        self.restore_content_scroll(scroll_y);
    }

    fn restore_content_scroll(&self, y: f64) {
        let scroll = self.content_scroll.clone();
        glib::idle_add_local_once(move || {
            let adj = scroll.vadjustment();
            let max = (adj.upper() - adj.page_size()).max(0.0);
            adj.set_value(y.clamp(0.0, max));
        });
    }

    fn append_sidebar_quick_run(&self, qr: &QuickRun) {
        let row = adw::ActionRow::new();
        row.set_title(&qr.name);
        let mut badges = vec![qr.operation_type.as_str().to_string()];
        let running = qr.status == "running"
            || qr.status == "starting"
            || self
                .ctx
                .is_busy(&qr.remote_name, qr.operation_type.as_str(), &qr.id);
        if running {
            badges.push(self.ctx.t_or("flow.quickRun.status.running", "running"));
            let dot = gtk::Image::from_icon_name("media-playback-start-symbolic");
            dot.set_pixel_size(12);
            dot.set_tooltip_text(Some(
                &self.ctx.t_or("flow.quickRun.status.running", "running"),
            ));
            row.add_prefix(&dot);
        }
        if qr.config.app.cron_enabled {
            badges.push(self.ctx.t_or("flow.quickRun.badges.cron", "cron"));
        }
        if qr.config.app.watch_enabled {
            badges.push(self.ctx.t_or("flow.quickRun.badges.watcher", "watch"));
        }
        if qr.config.app.auto_start {
            badges.push(self.ctx.t_or("flow.quickRun.badges.autostart", "autostart"));
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
            names.extend(
                snap.remotes
                    .iter()
                    .map(|remote| remote.name.clone())
                    .filter(|name| !name.is_empty()),
            );
            names.sort();
            names.dedup();
            names
        };
        if !remotes.is_empty() {
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
                        let has_runs = view
                            .ctx
                            .store
                            .borrow()
                            .quick_runs
                            .iter()
                            .any(|qr| qr.remote_name == remote);
                        if has_runs {
                            *view.remote_filter.borrow_mut() = Some(remote.clone());
                            view.refresh();
                        } else {
                            view.open_remote_detail(&remote);
                        }
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
        layout_bar.append(&dialogs::backend_switch_button(&self.ctx));
        layout_bar.append(&edit_btn);
        layout_bar.append(&reset);
        self.content.append(&layout_bar);
        let view = self.clone();
        self.content
            .append(&super::dashboard::Dashboard::origin_filter_bar(
                &self.ctx,
                &self.origin_filter.borrow(),
                move |id| {
                    *view.origin_filter.borrow_mut() = id;
                    view.refresh();
                },
            ));
        for (id, visible) in layout.resolve(crate::layout::QUICK_RUN_PANELS) {
            if !visible && !editing {
                continue;
            }
            match id.as_str() {
                "quickRuns" => {
                    let cards = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    if runs.is_empty() {
                        let list = gtk::ListBox::new();
                        list.add_css_class("boxed-list");
                        let row = adw::ActionRow::new();
                        row.set_title(&self.ctx.t_or("dashboard.quickRuns.empty", "No quick runs"));
                        row.set_subtitle(&self.ctx.t_or(
                            "flow.empty.description",
                            "Create a reusable rclone operation with cron, watcher, or autostart.",
                        ));
                        list.append(&row);
                        cards.append(&list);
                    }
                    for qr in runs {
                        cards.append(&self.quick_run_overview_card(&qr));
                    }
                    self.append_expandable(
                        "quickRuns",
                        &self.ctx.t_or("flow.quickRun.title", "Quick Runs"),
                        &cards,
                    );
                }
                "jobs" => {
                    let filter = self.origin_filter.borrow().clone();
                    let filtered: Vec<_> = snap
                        .jobs
                        .iter()
                        .filter(|job| {
                            crate::jobs::is_overview_job(job)
                                && crate::jobs::origin_matches(&job.origin, &filter)
                        })
                        .cloned()
                        .collect();
                    let view = self.clone();
                    let panel = super::job_panels::overview_jobs_panel(
                        &self.ctx,
                        &filtered,
                        &snap.stats,
                        move || view.refresh(),
                    );
                    self.append_expandable(
                        "jobs",
                        &self
                            .ctx
                            .t_or("generalOverview.panels.jobs", "Job Information"),
                        &panel,
                    );
                }
                "serves" => {
                    let serves = gtk::ListBox::new();
                    serves.add_css_class("boxed-list");
                    let filter = self.origin_filter.borrow().clone();
                    let filtered: Vec<_> = snap
                        .serves
                        .iter()
                        .filter(|serve| crate::jobs::origin_matches(&serve.origin, &filter))
                        .cloned()
                        .collect();
                    if filtered.is_empty() {
                        let row = adw::ActionRow::new();
                        row.set_title(
                            &self
                                .ctx
                                .t_or("generalOverview.serves.noActive", "No active serves"),
                        );
                        serves.append(&row);
                    } else {
                        for serve in &filtered {
                            let view = self.clone();
                            serves.append(&super::dashboard::serve_card_row(
                                &self.ctx,
                                serve,
                                move || view.refresh(),
                            ));
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
                    let filter = self.origin_filter.borrow().clone();
                    let records: Vec<_> = crate::automation::collect(&self.ctx.store.borrow())
                        .into_iter()
                        .filter(|record| {
                            crate::jobs::automation_matches_filter(&record.id, &filter)
                        })
                        .collect();
                    let host = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    if records.is_empty() {
                        let row = adw::ActionRow::new();
                        row.set_title(
                            &self
                                .ctx
                                .t_or("generalOverview.automations.noScheduled", "No automations"),
                        );
                        let list = gtk::ListBox::new();
                        list.add_css_class("boxed-list");
                        list.append(&row);
                        host.append(&list);
                    } else {
                        for record in records {
                            let ctx = self.ctx.clone();
                            let id = record.id.clone();
                            host.append(&automation_card::compact_card(
                                &self.ctx,
                                &self.toast,
                                &record,
                                Some(Rc::new(move || {
                                    ctx.request_nav(NavTarget::Automation { id: id.clone() });
                                })),
                            ));
                        }
                    }
                    self.append_expandable(
                        "automations",
                        &self
                            .ctx
                            .t_or("generalOverview.panels.automations", "Automations"),
                        &host,
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
            let view = self.clone();
            dialogs::attach_id_drag_drop(
                &expander,
                id.to_string(),
                Rc::new(move |from, to| {
                    let mut layout = view.flow_layout();
                    if layout.move_panel_to(&from, &to, crate::layout::QUICK_RUN_PANELS) {
                        view.write_flow_layout(layout);
                    }
                }),
            );
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

    fn heading(&self, text: &str) -> gtk::Label {
        let label = gtk::Label::new(Some(text));
        label.add_css_class("heading");
        label.set_xalign(0.0);
        label
    }

    fn quick_run_overview_card(&self, qr: &QuickRun) -> gtk::Widget {
        let running =
            crate::jobs::find_active_quick_run(&self.ctx.snapshot.borrow().jobs, qr).is_some();
        let busy = self
            .ctx
            .is_busy(&qr.remote_name, qr.operation_type.as_str(), &qr.id);
        let view = self.clone();
        let run = qr.clone();
        quick_run_card::overview_card(
            &self.ctx,
            qr,
            running,
            busy,
            quick_run_card::OverviewHandlers {
                on_start: Rc::new({
                    let view = view.clone();
                    let run = run.clone();
                    move || view.start_run(&run)
                }),
                on_stop: Rc::new({
                    let view = view.clone();
                    let run = run.clone();
                    move || view.stop_run(&run)
                }),
                on_edit: Rc::new({
                    let view = view.clone();
                    let run = run.clone();
                    move || {
                        if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                            dialogs::quick_run_editor(&win, view.ctx.clone(), Some(run.clone()), {
                                let view = view.clone();
                                Rc::new(move || view.refresh())
                            });
                        }
                    }
                }),
                on_open_remote: Rc::new({
                    let view = view.clone();
                    let remote = run.remote_name.clone();
                    move || view.open_remote_detail(&remote)
                }),
                on_open_path: Rc::new({
                    let ctx = self.ctx.clone();
                    let remote = run.remote_name.clone();
                    move |path: &str| ctx.open_typed_path(&remote, path)
                }),
                on_select: Some(Rc::new({
                    let view = view.clone();
                    let id = run.id.clone();
                    move || view.select_quick_run(Some(&id))
                })),
            },
        )
    }

    fn open_remote_config_step(&self, name: &str, step: &str) {
        if let Some(win) = self.root.root().and_downcast::<gtk::Window>() {
            super::remote_config::present_with(
                &win,
                self.ctx.clone(),
                name.to_string(),
                super::remote_config::RemoteConfigOpen {
                    initial: Some(step.to_string()),
                    ..Default::default()
                },
                {
                    let view = self.clone();
                    Rc::new(move || view.refresh())
                },
            );
        }
    }

    fn append_disk_usage(&self, name: &str) {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let usage = adw::ActionRow::new();
        usage.set_title(&self.ctx.t_or("remote.diskUsage", "Disk usage"));
        let retry = gtk::Button::from_icon_name("view-refresh-symbolic");
        retry.set_valign(gtk::Align::Center);
        retry.set_tooltip_text(Some(&self.ctx.t_or("common.retry", "Retry")));
        {
            let view = self.clone();
            retry.connect_clicked(move |_| view.refresh());
        }
        usage.add_suffix(&retry);
        if let Some(client) = self.ctx.client() {
            match client.about(&crate::rclone::remote_fs(name, "")) {
                Ok(about) => {
                    usage.set_subtitle(&crate::store::disk_label_from_about(&about));
                    box_.append(&usage);
                    if let Some(ratio) = crate::store::disk_usage_ratio(&about) {
                        let bar = gtk::LevelBar::new();
                        bar.set_min_value(0.0);
                        bar.set_max_value(1.0);
                        bar.set_value(ratio);
                        bar.set_hexpand(true);
                        box_.append(&bar);
                    }
                }
                Err(err) => {
                    usage.set_subtitle(&err.to_string());
                    box_.append(&usage);
                }
            }
        } else {
            usage.set_subtitle(&self.ctx.t_or(
                "notification.title.engineConnectionFailed",
                "Engine Connection Error",
            ));
            box_.append(&usage);
        }
        self.content.append(&box_);
    }

    fn append_configuration_links(&self, name: &str) {
        self.content.append(
            &self.heading(
                &self
                    .ctx
                    .t_or("dashboard.appDetail.configuration", "Configuration"),
            ),
        );
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("linked");
        for (step, key, fallback) in [
            ("remote", "modals.remoteConfig.steps.remote", "Remote"),
            ("vfs", "general.remoteConfig.steps.vfs", "VFS"),
            ("filter", "general.remoteConfig.steps.filter", "Filter"),
            ("backend", "general.remoteConfig.steps.backend", "Backend"),
            (
                "runtime",
                "modals.remoteConfig.steps.runtimeRemote",
                "Runtime",
            ),
        ] {
            let btn = gtk::Button::with_label(&self.ctx.t_or(key, fallback));
            {
                let view = self.clone();
                let remote = name.to_string();
                btn.connect_clicked(move |_| view.open_remote_config_step(&remote, step));
            }
            row.append(&btn);
        }
        self.content.append(&row);
    }

    fn append_transfer_activity(&self, name: &str, snap: &crate::store::RuntimeSnapshot) {
        let jobs = {
            let store = self.ctx.store.borrow();
            crate::jobs::merge_overview_jobs(
                &snap.jobs,
                &crate::jobs::history_with_meta(&store.job_history, &store.job_meta),
                name,
                None,
                None,
            )
        };
        self.append_transfers(&self.content, &jobs);
    }

    fn append_transfers(&self, host: &gtk::Box, jobs: &[crate::store::JobInfo]) {
        let mut rows = Vec::new();
        for job in jobs {
            if let Some(arr) = job.transferring.as_array() {
                for item in arr {
                    rows.push((
                        job.operation.clone(),
                        job.remote.clone(),
                        crate::transfers::parse_transfer_row(item),
                        false,
                    ));
                }
            }
            for source in [
                job.completed.as_array(),
                job.stats.get("completed").and_then(|v| v.as_array()),
            ]
            .into_iter()
            .flatten()
            {
                for item in source {
                    rows.push((
                        job.operation.clone(),
                        job.remote.clone(),
                        crate::transfers::parse_completed_transfer_row(item),
                        true,
                    ));
                }
            }
        }
        let mut check_items = Vec::new();
        for job in jobs {
            if crate::checks::is_check_operation(&job.operation) {
                let source = crate::checks::check_source_from_job(&job.stats, &job.output);
                check_items.extend(
                    crate::checks::parse_check_items(&source, &job.src, &job.dst)
                        .into_iter()
                        .map(|item| crate::checks::with_job(item, job)),
                );
            }
        }
        check_items.truncate(40);
        let has_live_job = jobs
            .iter()
            .any(|job| crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job));
        if rows.is_empty() && check_items.is_empty() && !has_live_job {
            return;
        }
        let title = if jobs
            .iter()
            .any(|job| crate::checks::is_check_operation(&job.operation))
        {
            self.ctx
                .t_or("shared.transferActivity.titleCheck", "Check Results")
        } else {
            self.ctx
                .t_or("shared.transferActivity.title", "Transfer Activity")
        };
        host.append(&self.heading(&title));
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(
            &self.ctx.t_or("shared.search.toggle", "Search transfers"),
        ));
        search.set_text(&self.transfer_query.borrow());
        {
            let view = self.clone();
            search.connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                if *view.transfer_query.borrow() == text {
                    return;
                }
                *view.transfer_query.borrow_mut() = text;
                view.refresh();
            });
        }
        host.append(&search);
        let active_count = rows
            .iter()
            .filter(|(_, _, _, completed)| !*completed)
            .count();
        let done_count = rows
            .iter()
            .filter(|(_, _, _, completed)| *completed)
            .count();
        if active_count > 0 && done_count > 0 {
            let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            tabs.add_css_class("linked");
            let current = self.transfer_tab.borrow().clone();
            for (id, key, fallback, count) in [
                (
                    "active",
                    "shared.transferActivity.tabs.active",
                    "Active",
                    active_count,
                ),
                (
                    "recent",
                    "shared.transferActivity.tabs.recent",
                    "Recent",
                    done_count,
                ),
            ] {
                let template = self.ctx.t_or(key, fallback);
                let count_s = count.to_string();
                let label = if template.contains("{{count}}") || template.contains("{count}") {
                    template
                        .replace("{{count}}", &count_s)
                        .replace("{count}", &count_s)
                } else {
                    format!("{template} ({count})")
                };
                let btn = gtk::ToggleButton::with_label(&label);
                btn.set_active(current == id);
                {
                    let view = self.clone();
                    let id = id.to_string();
                    btn.connect_clicked(move |_| {
                        *view.transfer_tab.borrow_mut() = id.clone();
                        view.refresh();
                    });
                }
                tabs.append(&btn);
            }
            host.append(&tabs);
        }
        if let Some(job) = jobs.first() {
            let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let running = crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job);
            if running {
                let reset_label = self
                    .ctx
                    .t_or("shared.transferActivity.resetStats", "Reset stats");
                let reset = gtk::Button::with_label(&reset_label);
                reset.set_tooltip_text(Some(&reset_label));
                {
                    let ctx = self.ctx.clone();
                    let group = if job.group.is_empty() {
                        format!("job/{}", job.id)
                    } else {
                        job.group.clone()
                    };
                    let view = self.clone();
                    reset.connect_clicked(move |_| {
                        if let Some(client) = ctx.client() {
                            let _ = client.reset_stats(Some(&group));
                            ctx.refresh_runtime();
                            view.refresh();
                        }
                    });
                }
                toolbar.append(&reset);
            } else {
                let delete_label = self
                    .ctx
                    .t_or("detailShared.jobs.actions.delete", "Delete from history");
                let delete = gtk::Button::with_label(&delete_label);
                delete.add_css_class("destructive-action");
                delete.set_tooltip_text(Some(&delete_label));
                {
                    let ctx = self.ctx.clone();
                    let id = job.id;
                    let view = self.clone();
                    delete.connect_clicked(move |_| {
                        ctx.store.borrow_mut().dismiss_job(id);
                        ctx.persist();
                        ctx.refresh_runtime();
                        view.refresh();
                    });
                }
                toolbar.append(&delete);
            }
            host.append(&toolbar);
        }
        if !check_items.is_empty() {
            let query = self.transfer_query.borrow().clone();
            let check_items = crate::checks::visible_check_items(
                check_items,
                &self.ctx.hidden_check_ids.borrow(),
                &self.ctx.check_status_overrides.borrow(),
                &query,
            );
            if !check_items.is_empty() {
                let list = gtk::ListBox::new();
                list.add_css_class("boxed-list");
                for item in check_items {
                    list.append(&dialogs::check_result_row(&self.ctx, &item, &self.root));
                }
                host.append(&list);
            }
        }
        if rows.is_empty() {
            return;
        }
        let query = self.transfer_query.borrow().to_ascii_lowercase();
        let tab = self.transfer_tab.borrow().clone();
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for (operation, remote, row, completed) in rows {
            if tab == "active" && completed {
                continue;
            }
            if tab == "recent" && !completed {
                continue;
            }
            if !query.is_empty() {
                let hay = format!("{} {} {}", row.name, row.src, row.dst).to_ascii_lowercase();
                if !hay.contains(&query) {
                    continue;
                }
            }
            list.append(&dialogs::transfer_activity_row(
                &self.ctx,
                &row,
                completed,
                &operation,
                &remote,
                &self.toast,
            ));
        }
        host.append(&list);
    }

    fn append_remote_automations(&self, name: &str) {
        let records: Vec<_> = crate::automation::collect(&self.ctx.store.borrow())
            .into_iter()
            .filter(|record| record.remote == name)
            .collect();
        if records.is_empty() {
            return;
        }
        self.content.append(
            &self.heading(
                &self
                    .ctx
                    .t_or("dashboard.generalDetail.automations", "Automations"),
            ),
        );
        let selected = self.ctx.selected_automation.borrow().clone();
        self.content.append(&automation_card::detailed_carousel(
            &self.ctx,
            &self.toast,
            &records,
            selected.as_deref(),
        ));
    }

    fn append_settings_panel(&self, name: &str) {
        let dump = self
            .ctx
            .client()
            .and_then(|client| client.dump_config().ok())
            .unwrap_or(serde_json::json!({}));
        let params =
            crate::providers::dump_remote_params(&dump, name).unwrap_or(serde_json::json!({}));
        let view = self.clone();
        let remote = name.to_string();
        self.content.append(&dialogs::settings_panel(
            &self.ctx,
            &self.ctx.t_or(
                "dashboard.generalDetail.remoteConfiguration",
                "Remote Configuration",
            ),
            &params,
            Some(Rc::new(move || {
                view.open_remote_config_step(&remote, "remote")
            })),
        ));
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
            browse.connect_clicked(move |_| ctx.browse_remote_home(&remote));
        }
        let configure = gtk::Button::with_label(&self.ctx.t_or(
            "dashboard.generalDetail.editConfiguration",
            "Edit Configuration",
        ));
        {
            let view = self.clone();
            let remote = name.to_string();
            configure.connect_clicked(move |_| view.open_remote_config_step(&remote, "remote"));
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
        self.append_configuration_links(name);
        self.append_disk_usage(name);

        let qrs: Vec<_> = self
            .ctx
            .store
            .borrow()
            .quick_runs
            .iter()
            .filter(|q| q.remote_name == name)
            .cloned()
            .collect();
        let qr_title = if qrs.is_empty() {
            self.ctx
                .t_or("remote.quickRuns", "Quick Runs for this remote")
        } else {
            format!(
                "{} ({})",
                self.ctx.t_or("flow.quickRun.title", "Quick Runs"),
                qrs.len()
            )
        };
        self.content.append(&self.heading(&qr_title));
        if qrs.is_empty() {
            let qlist = gtk::ListBox::new();
            qlist.add_css_class("boxed-list");
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.tf(
                "flow.quickRun.overview.noRunsForRemote",
                &[("remote", name)],
            ));
            qlist.append(&row);
            self.content.append(&qlist);
        } else {
            let cards = gtk::Box::new(gtk::Orientation::Vertical, 8);
            for qr in &qrs {
                cards.append(&self.quick_run_overview_card(qr));
            }
            self.content.append(&cards);
        }
        let add_qr = gtk::Button::with_label(&self.ctx.tf(
            "flow.quickRun.overview.createForRemote",
            &[("remote", name)],
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

        let jobs: Vec<_> = {
            let store = self.ctx.store.borrow();
            crate::jobs::merge_overview_jobs(
                &snap.jobs,
                &crate::jobs::history_with_meta(&store.job_history, &store.job_meta),
                name,
                None,
                None,
            )
        };
        self.content
            .append(&job_panels::detail_jobs_panel(&self.ctx, &jobs, {
                let view = self.clone();
                move || view.refresh()
            }));
        self.append_remote_automations(name);
        self.append_transfer_activity(name, &snap);
        self.append_settings_panel(name);
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

        let monitoring = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let configuration = gtk::Box::new(gtk::Orientation::Vertical, 12);

        let snap = self.ctx.snapshot.borrow().clone();
        let live = crate::jobs::find_active_quick_run(&snap.jobs, qr);
        let default_addr = self.ctx.t_or("dashboard.appDetail.default", "Default");
        let (cfg_src, cfg_dst) = crate::jobs::operation_control_configured_paths(
            qr.operation_type,
            &qr.config.rclone,
            &qr.remote_name,
            &default_addr,
        );
        let paths = crate::jobs::operation_control_paths(qr.operation_type, cfg_src, cfg_dst, live);
        let active = live.is_some();
        let busy = self
            .ctx
            .is_busy(&qr.remote_name, qr.operation_type.as_str(), &qr.id);
        let spec = operation_control::OperationControlSpec {
            title: qr.name.clone(),
            operation: qr.operation_type,
            remote_name: qr.remote_name.clone(),
            source: paths.source,
            destination: paths.destination,
            hide_destination: paths.hide_destination,
            dest_browseable: paths.dest_browseable,
            dry_run: crate::jobs::is_dry_run(&qr.config.rclone),
            resync: crate::jobs::is_resync(&qr.config.rclone),
            active,
            busy,
            mount_usage: operation_control::mount_usage_pairs(&self.ctx, &qr.remote_name, &snap),
        };
        {
            let view = self.clone();
            let qr = qr.clone();
            let id = qr.id.clone();
            monitoring.append(&operation_control::operation_control(
                &self.ctx,
                &spec,
                operation_control::OperationControlHandlers {
                    on_start: {
                        let view = view.clone();
                        let qr = qr.clone();
                        Rc::new(move || view.start_run(&qr))
                    },
                    on_stop: {
                        let view = view.clone();
                        let qr = qr.clone();
                        Rc::new(move || view.stop_run(&qr))
                    },
                    on_dry_run: Some({
                        let ctx = self.ctx.clone();
                        let id = id.clone();
                        Rc::new(move |on| {
                            if let Some(run) = ctx
                                .store
                                .borrow_mut()
                                .quick_runs
                                .iter_mut()
                                .find(|q| q.id == id)
                            {
                                crate::jobs::apply_session_flags(&mut run.config.rclone, on, false);
                                if !on {
                                    if let Some(obj) = run.config.rclone.as_object_mut() {
                                        obj.remove("DryRun");
                                        obj.remove("dryRun");
                                    }
                                }
                            }
                            ctx.persist();
                        })
                    }),
                    on_resync: Some({
                        let ctx = self.ctx.clone();
                        Rc::new(move |on| {
                            if let Some(run) = ctx
                                .store
                                .borrow_mut()
                                .quick_runs
                                .iter_mut()
                                .find(|q| q.id == id)
                            {
                                let dry = crate::jobs::is_dry_run(&run.config.rclone);
                                crate::jobs::apply_session_flags(&mut run.config.rclone, dry, on);
                            }
                            ctx.persist();
                        })
                    }),
                },
            ));
        }

        if qr.config.app.cron_enabled && !qr.config.app.cron_expression.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or("flow.quickRun.badges.scheduled", "Scheduled"));
            row.set_subtitle(&crate::rclone::describe_cron_i18n(
                &qr.config.app.cron_expression,
                &self.ctx.i18n.borrow(),
            ));
            monitoring.append(&row);
        }
        if qr.config.app.watch_enabled {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or(
                "automation.monitoring.realtimeSchedule",
                "Real-time File Watcher",
            ));
            let mut subtitle = format!(
                "{}: {} {}",
                self.ctx
                    .t_or("automation.monitoring.debounce", "Debounce Delay"),
                qr.config.app.watch_delay,
                self.ctx.t_or("automation.monitoring.seconds", "seconds")
            );
            if qr.config.app.watch_changed_only {
                subtitle.push_str(" · ");
                subtitle.push_str(&self.ctx.t_or(
                    "automation.monitoring.changedOnlyShort",
                    "Changed files only",
                ));
            }
            row.set_subtitle(&subtitle);
            monitoring.append(&row);
        }

        if let Some(job) = {
            let live = self.ctx.snapshot.borrow().jobs.clone();
            let history = self.ctx.store.borrow().job_history.clone();
            crate::jobs::find_quick_run_job(&live, &history, qr)
        } {
            monitoring.append(&super::job_panels::job_info_group(&self.ctx, &job));
            monitoring.append(&super::job_panels::job_stats_group(&self.ctx, &job));
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
            monitoring.append(&buttons);
            self.append_transfers(&monitoring, std::slice::from_ref(&job));
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
            monitoring.append(&logs);
        }

        if qr.operation_type.supports_vfs() {
            monitoring.append(&super::vfs_panel::vfs_panel(
                self.ctx.clone(),
                &qr.remote_name,
                self.toast.clone(),
            ));
        }

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
        configuration.append(&tray);
        let config_value = serde_json::to_value(&qr.config).unwrap_or(serde_json::json!({}));
        {
            let view = self.clone();
            let qr = qr.clone();
            configuration.append(&dialogs::settings_panel(
                &self.ctx,
                &self
                    .ctx
                    .t_or("flow.quickRun.detail.configuration", "Configuration"),
                &config_value,
                Some(Rc::new(move || {
                    if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                        dialogs::quick_run_editor(&win, view.ctx.clone(), Some(qr.clone()), {
                            let view = view.clone();
                            Rc::new(move || view.refresh())
                        });
                    }
                })),
            ));
        }

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let edit = gtk::Button::with_label(&self.ctx.t_or("common.edit", "Edit"));
        let dup = gtk::Button::with_label(&self.ctx.t_or("common.duplicate", "Duplicate"));
        let delete = gtk::Button::with_label(&self.ctx.t_or("common.delete", "Delete"));
        delete.add_css_class("destructive-action");
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
        actions.append(&edit);
        actions.append(&dup);
        actions.append(&delete);
        configuration.append(&actions);

        let detail_key = format!("quick:{}", qr.id);
        if let Some(saved) = self
            .ctx
            .settings
            .borrow()
            .runtime
            .selected_detail_pages
            .get(&detail_key)
        {
            if saved == "monitoring" || saved == "configuration" {
                *self.detail_page.borrow_mut() = saved.clone();
            }
        }
        let stack = adw::ViewStack::new();
        stack.set_vhomogeneous(false);
        stack.add_titled(
            &monitoring,
            Some("monitoring"),
            &self
                .ctx
                .t_or("dashboard.appDetail.monitoring", "Monitoring"),
        );
        stack.add_titled(
            &configuration,
            Some("configuration"),
            &self
                .ctx
                .t_or("dashboard.appDetail.configuration", "Configuration"),
        );
        let switcher = super::detail_page_switcher(
            &stack,
            &self.detail_page,
            &[
                (
                    "monitoring",
                    self.ctx
                        .t_or("dashboard.appDetail.monitoring", "Monitoring"),
                ),
                (
                    "configuration",
                    self.ctx
                        .t_or("dashboard.appDetail.configuration", "Configuration"),
                ),
            ],
            {
                let ctx = self.ctx.clone();
                let key = self
                    .ctx
                    .selected_quick_run
                    .borrow()
                    .clone()
                    .map(|id| format!("quick:{id}"))
                    .or_else(|| {
                        self.selected_flow_remote
                            .borrow()
                            .clone()
                            .map(|remote| format!("remote:{remote}"))
                    });
                key.map(|key| {
                    Rc::new(move |page: &str| {
                        ctx.settings
                            .borrow_mut()
                            .runtime
                            .selected_detail_pages
                            .insert(key.clone(), page.to_string());
                        ctx.persist();
                    }) as Rc<dyn Fn(&str)>
                })
            },
        );
        self.content.append(&switcher);
        self.content.append(&stack);
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
                let rclone = crate::jobs::flatten_rclone(&qr.config.rclone);
                self.ctx.record_started_job(
                    &id,
                    &qr.remote_name,
                    &qr.config,
                    "flow",
                    qr.operation_type.as_str(),
                    &crate::jobs::default_source(&qr.remote_name, &rclone),
                    &crate::jobs::default_dest(&qr.remote_name, &rclone, qr.operation_type),
                    &qr.id,
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
