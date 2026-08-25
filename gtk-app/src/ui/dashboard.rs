use super::dialogs;
use super::AppCtx;
use crate::navigation::NavTarget;
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
    tab_buttons: Rc<RefCell<Vec<(AppTab, gtk::ToggleButton)>>>,
    sidebar_list: gtk::ListBox,
    search: gtk::SearchEntry,
    content: gtk::Stack,
    overview: gtk::Box,
    detail: gtk::Box,
    editing_layout: Rc<RefCell<bool>>,
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
        search.set_placeholder_text(Some(
            &ctx.t_or("sidebar.searchPlaceholder", "Search remotes..."),
        ));
        let sidebar_list = gtk::ListBox::new();
        sidebar_list.add_css_class("navigation-sidebar");
        sidebar_list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&sidebar_list));

        let add_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let quick = gtk::Button::from_icon_name("list-add-symbolic");
        quick.set_tooltip_text(Some(&ctx.t_or("sidebar.quickAdd", "Quick add remote")));
        let detailed = gtk::Button::from_icon_name("document-edit-symbolic");
        detailed.set_tooltip_text(Some(
            &ctx.t_or("sidebar.detailedConfig", "Detailed remote config"),
        ));
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
            tab_buttons: Rc::new(RefCell::new(Vec::new())),
            sidebar_list,
            search,
            content,
            overview,
            detail,
            editing_layout: Rc::new(RefCell::new(false)),
        };

        let mut group_anchor: Option<gtk::ToggleButton> = None;
        for tab in AppTab::ALL {
            let btn = gtk::ToggleButton::new();
            btn.set_label(&ctx.t(tab.label_key()));
            btn.set_tooltip_text(Some(tab.as_str()));
            if let Some(anchor) = &group_anchor {
                btn.set_group(Some(anchor));
            } else {
                group_anchor = Some(btn.clone());
            }
            if tab == AppTab::General {
                btn.set_active(true);
            }
            dash.tab_buttons.borrow_mut().push((tab, btn.clone()));
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

    pub fn navigate(&self, tab: AppTab, remote: Option<&str>) {
        *self.ctx.selected_remote.borrow_mut() =
            remote.filter(|s| !s.is_empty()).map(|s| s.to_string());
        let already = *self.tab.borrow() == tab;
        *self.tab.borrow_mut() = tab;
        for (candidate, btn) in self.tab_buttons.borrow().iter() {
            if *candidate == tab {
                if !btn.is_active() {
                    btn.set_active(true);
                }
                break;
            }
        }
        if already {
            self.refresh();
        }
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
        let order = self.ctx.store.borrow().remote_order.clone();
        let mut remotes: Vec<_> = snap.remotes.iter().collect();
        if !order.is_empty() {
            remotes.sort_by_key(|r| {
                order
                    .iter()
                    .position(|n| n == &r.name)
                    .unwrap_or(usize::MAX)
            });
        }
        let editing = *self.editing_layout.borrow();
        for remote in remotes {
            if remote.hidden && !editing {
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
            let icon = gtk::Image::from_icon_name(crate::providers::provider_icon(&remote.r#type));
            icon.set_pixel_size(16);
            icon.set_tooltip_text(Some(&remote.r#type));
            box_.append(&icon);
            let name = gtk::Label::new(Some(&if remote.hidden {
                format!("{} (hidden)", remote.name)
            } else {
                remote.name.clone()
            }));
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
            empty.set_title(
                &self
                    .ctx
                    .t_or("sidebar.noRemotesConfigured", "No remotes configured"),
            );
            empty.set_subtitle(&self.ctx.t_or(
                "flow.quickRun.editor.noRemotes",
                "Use Quick Add or Detailed Config",
            ));
            self.sidebar_list.append(&empty);
        }
    }

    fn remotes_for_display(&self) -> Vec<crate::store::RemoteInfo> {
        let snap = self.ctx.snapshot.borrow();
        let mut remotes = snap.remotes.clone();
        let order = self.ctx.store.borrow().remote_order.clone();
        if !order.is_empty() {
            remotes.sort_by_key(|r| {
                order
                    .iter()
                    .position(|n| n == &r.name)
                    .unwrap_or(usize::MAX)
            });
        }
        if !*self.editing_layout.borrow() {
            remotes.retain(|r| !r.hidden);
        }
        remotes
    }

    fn move_remote(&self, name: &str, delta: isize) {
        let names: Vec<String> = self
            .ctx
            .snapshot
            .borrow()
            .remotes
            .iter()
            .map(|r| r.name.clone())
            .collect();
        {
            let mut store = self.ctx.store.borrow_mut();
            store.ensure_remote_order(&names);
            store.move_remote(name, delta);
        }
        self.ctx.persist();
        self.refresh();
    }

    fn toggle_remote_hidden(&self, name: &str) {
        self.ctx.store.borrow_mut().toggle_remote_hidden(name);
        self.ctx.persist();
        self.ctx.refresh_runtime();
        self.refresh();
    }

    fn fill_overview(&self) {
        clear_box(&self.overview);
        let tab = *self.tab.borrow();
        let remotes = self.remotes_for_display();
        let snap = self.ctx.snapshot.borrow().clone();
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

        let editing = *self.editing_layout.borrow();
        let layout_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let edit_label = if editing {
            self.ctx.t_or("common.done", "Done")
        } else {
            self.ctx.t_or("generalOverview.editLayout", "Edit layout")
        };
        let edit_btn = gtk::Button::with_label(&edit_label);
        edit_btn.set_tooltip_text(Some("Hide or reorder remotes and overview panels"));
        {
            let dash = self.clone();
            edit_btn.connect_clicked(move |_| {
                let next = !*dash.editing_layout.borrow();
                *dash.editing_layout.borrow_mut() = next;
                dash.refresh();
            });
        }
        let order_btn = gtk::Button::with_label(
            &self
                .ctx
                .t_or("generalOverview.reorderRemotes", "Reorder remotes…"),
        );
        order_btn.set_tooltip_text(Some("Open the remote order and visibility editor"));
        {
            let dash = self.clone();
            order_btn.connect_clicked(move |_| {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::item_order(&win, dash.ctx.clone(), {
                        let dash = dash.clone();
                        Rc::new(move || dash.refresh())
                    });
                }
            });
        }
        let reset =
            gtk::Button::with_label(&self.ctx.t_or("generalOverview.resetPanels", "Reset panels"));
        reset.set_tooltip_text(Some("Restore the default overview panel order"));
        {
            let dash = self.clone();
            reset.connect_clicked(move |_| {
                dash.ctx.settings.borrow_mut().runtime.dashboard_layout =
                    serde_json::json!({ "order": [], "hidden": [] });
                dash.ctx.persist();
                dash.refresh();
            });
        }
        layout_bar.append(&edit_btn);
        layout_bar.append(&order_btn);
        layout_bar.append(&reset);
        self.overview.append(&layout_bar);
        let layout = crate::layout::PanelLayout::from_value(
            &self.ctx.settings.borrow().runtime.dashboard_layout,
        );
        for (id, visible) in layout.resolve(crate::layout::DASHBOARD_PANELS) {
            if !visible && !editing {
                continue;
            }
            self.append_panel_chrome(&id);
            match id.as_str() {
                "remotes" => self.render_remotes_panel(&remotes, tab, editing),
                "jobs" => self.render_jobs_panel(&snap),
                "serves" => self.render_serves_panel(&snap),
                "bandwidth" => self.render_bandwidth_panel(&snap),
                "system" => self.render_system_panel(&snap),
                "automations" => self.render_automations_panel(),
                _ => {}
            }
        }
    }

    fn append_panel_chrome(&self, id: &str) {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.append(&section_label(crate::layout::panel_title(id)));
        if *self.editing_layout.borrow() {
            let hide = gtk::Button::from_icon_name("view-conceal-symbolic");
            hide.set_tooltip_text(Some("Hide or show this panel"));
            let up = gtk::Button::from_icon_name("go-up-symbolic");
            up.set_tooltip_text(Some("Move panel up"));
            let down = gtk::Button::from_icon_name("go-down-symbolic");
            down.set_tooltip_text(Some("Move panel down"));
            {
                let dash = self.clone();
                let id = id.to_string();
                hide.connect_clicked(move |_| dash.toggle_panel(&id));
            }
            {
                let dash = self.clone();
                let id = id.to_string();
                up.connect_clicked(move |_| dash.move_panel(&id, -1));
            }
            {
                let dash = self.clone();
                let id = id.to_string();
                down.connect_clicked(move |_| dash.move_panel(&id, 1));
            }
            header.append(&hide);
            header.append(&up);
            header.append(&down);
        }
        self.overview.append(&header);
    }

    fn dashboard_layout(&self) -> crate::layout::PanelLayout {
        crate::layout::PanelLayout::from_value(&self.ctx.settings.borrow().runtime.dashboard_layout)
    }

    fn write_dashboard_layout(&self, layout: crate::layout::PanelLayout) {
        self.ctx.settings.borrow_mut().runtime.dashboard_layout = layout.to_value();
        self.ctx.persist();
        self.refresh();
    }

    fn toggle_panel(&self, id: &str) {
        let mut layout = self.dashboard_layout();
        layout.toggle_hidden(id);
        self.write_dashboard_layout(layout);
    }

    fn move_panel(&self, id: &str, delta: isize) {
        let mut layout = self.dashboard_layout();
        layout.move_panel(id, delta, crate::layout::DASHBOARD_PANELS);
        self.write_dashboard_layout(layout);
    }

    fn render_remotes_panel(
        &self,
        remotes: &[crate::store::RemoteInfo],
        tab: AppTab,
        editing: bool,
    ) {
        let (active, idle): (Vec<_>, Vec<_>) = remotes.iter().cloned().partition(|remote| {
            tab.remote_is_active(remote.mounted, remote.serving, remote.job_active)
        });
        if !active.is_empty() {
            self.overview.append(&section_label(
                &self
                    .ctx
                    .t_or(tab.active_section_key(), tab.active_section_fallback()),
            ));
            self.append_remote_list(&active, tab, editing);
        }
        if !idle.is_empty() {
            self.overview.append(&section_label(
                &self
                    .ctx
                    .t_or(tab.idle_section_key(), tab.idle_section_fallback()),
            ));
            self.append_remote_list(&idle, tab, editing);
        }
    }

    fn append_remote_list(&self, remotes: &[crate::store::RemoteInfo], tab: AppTab, editing: bool) {
        let detailed = self.ctx.settings.borrow().runtime.dashboard_card_variant == "detailed";
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for remote in remotes {
            let row = adw::ActionRow::new();
            row.set_title(&remote.name);
            row.set_subtitle(&format!(
                "{} · {}{}",
                remote.r#type,
                remote_state_label(remote.mounted, remote.serving, remote.job_active),
                if remote.hidden { " · hidden" } else { "" }
            ));
            row.add_prefix(&gtk::Image::from_icon_name(
                crate::providers::provider_icon(&remote.r#type),
            ));
            if remote.hidden {
                row.add_css_class("dim-label");
            }
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
                    ctx.request_browse(&name, "");
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
            if editing {
                let hide = gtk::Button::from_icon_name(if remote.hidden {
                    "view-reveal-symbolic"
                } else {
                    "view-conceal-symbolic"
                });
                hide.set_valign(gtk::Align::Center);
                hide.set_tooltip_text(Some(if remote.hidden {
                    "Show on overview"
                } else {
                    "Hide from overview"
                }));
                let up = gtk::Button::from_icon_name("go-up-symbolic");
                up.set_valign(gtk::Align::Center);
                up.set_tooltip_text(Some("Move up"));
                let down = gtk::Button::from_icon_name("go-down-symbolic");
                down.set_valign(gtk::Align::Center);
                down.set_tooltip_text(Some("Move down"));
                {
                    let dash = self.clone();
                    let name = remote.name.clone();
                    hide.connect_clicked(move |_| dash.toggle_remote_hidden(&name));
                }
                {
                    let dash = self.clone();
                    let name = remote.name.clone();
                    up.connect_clicked(move |_| dash.move_remote(&name, -1));
                }
                {
                    let dash = self.clone();
                    let name = remote.name.clone();
                    down.connect_clicked(move |_| dash.move_remote(&name, 1));
                }
                row.add_suffix(&hide);
                row.add_suffix(&up);
                row.add_suffix(&down);
            }
            {
                let ctx = self.ctx.clone();
                let name = remote.name.clone();
                let dash = self.clone();
                row.connect_activated(move |_| {
                    *ctx.selected_remote.borrow_mut() = Some(name.clone());
                    dash.refresh();
                });
            }
            if detailed {
                let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
                card.set_margin_top(4);
                card.set_margin_bottom(8);
                card.set_margin_start(8);
                card.set_margin_end(8);
                card.append(&row);
                let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                chips.add_css_class("linked");
                chips.set_hexpand(true);
                if let Some(meta) = self.ctx.store.borrow().remotes.get(&remote.name) {
                    for op in meta.visible_operations() {
                        if !tab.includes_operation(op) {
                            continue;
                        }
                        for pname in meta.profile_names(op) {
                            let snap = self.ctx.snapshot.borrow();
                            let active = crate::jobs::profile_is_active(
                                &remote.name,
                                op,
                                &pname,
                                &snap.jobs,
                                &snap.mounts,
                                &snap.serves,
                            );
                            drop(snap);
                            let chip = gtk::Button::with_label(&if active {
                                format!("Stop {} · {pname}", op.api_label())
                            } else {
                                format!("{} · {pname}", op.api_label())
                            });
                            chip.set_tooltip_text(Some(if active {
                                "Stop this profile"
                            } else {
                                "Start this profile"
                            }));
                            if active {
                                chip.add_css_class("destructive-action");
                            }
                            let ctx = self.ctx.clone();
                            let name = remote.name.clone();
                            let toast = self.toast.clone();
                            let dash = self.clone();
                            let pname = pname.clone();
                            chip.connect_clicked(move |_| {
                                toggle_profile(&ctx, &name, op, &pname, &toast);
                                dash.refresh();
                            });
                            chips.append(&chip);
                        }
                    }
                }
                card.append(&chips);
                let wrap = gtk::ListBoxRow::new();
                wrap.set_activatable(false);
                wrap.set_child(Some(&card));
                list.append(&wrap);
            } else {
                list.append(&row);
            }
        }
        self.overview.append(&list);
    }

    fn render_jobs_panel(&self, snap: &crate::store::RuntimeSnapshot) {
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
                    let ctx = self.ctx.clone();
                    row.connect_activated(move |_| {
                        ctx.request_nav(NavTarget::Job { id: job.id });
                    });
                }
                jobs.append(&row);
            }
        }
        self.overview.append(&jobs);
    }

    fn render_serves_panel(&self, snap: &crate::store::RuntimeSnapshot) {
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
                let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
                stop.set_valign(gtk::Align::Center);
                stop.set_tooltip_text(Some("Stop this serve"));
                {
                    let ctx = self.ctx.clone();
                    let id = serve.id.clone();
                    let dash = self.clone();
                    stop.connect_clicked(move |_| {
                        if let Some(c) = ctx.client() {
                            let _ = c.serve_stop(&id);
                            ctx.refresh_runtime();
                            dash.refresh();
                        }
                    });
                }
                if !serve.addr.is_empty() {
                    let open = gtk::LinkButton::new(&format!("http://{}", serve.addr));
                    open.set_label(&self.ctx.t_or("common.open", "Open"));
                    open.set_valign(gtk::Align::Center);
                    row.add_suffix(&open);
                }
                row.add_suffix(&stop);
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
        self.overview.append(&serves);
    }

    fn render_bandwidth_panel(&self, snap: &crate::store::RuntimeSnapshot) {
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
        let bw_group = gtk::ListBox::new();
        bw_group.add_css_class("boxed-list");
        let current = adw::ActionRow::new();
        current.set_title(
            &self
                .ctx
                .t_or("bandwidth.currentTransfer", "Current transfer"),
        );
        current.set_subtitle(&format!(
            "{} transferred · {:.1} KiB/s",
            format_bytes(bytes),
            speed / 1024.0
        ));
        bw_group.append(&current);
        let limit = ctx_settings_bandwidth(&self.ctx);
        let limit_row = adw::ActionRow::new();
        limit_row.set_title(
            &self
                .ctx
                .t_or("dashboard.bandwidth.savedLimit", "Saved limit"),
        );
        limit_row.set_subtitle(&if limit.is_empty() || limit == "off" {
            self.ctx.t_or("dashboard.bandwidth.unlimited", "Unlimited")
        } else {
            limit.clone()
        });
        bw_group.append(&limit_row);
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
                format_bytes(live.bytes_per_sec_tx),
                format_bytes(live.bytes_per_sec_rx)
            ));
            bw_group.append(&live_row);
        }
        self.overview.append(&bw_group);
        let presets = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        presets.set_margin_top(8);
        presets.add_css_class("linked");
        for (value, label) in crate::jobs::BANDWIDTH_PRESETS {
            let btn = gtk::Button::with_label(label);
            let ctx = self.ctx.clone();
            let dash = self.clone();
            let value = (*value).to_string();
            btn.connect_clicked(move |_| {
                apply_bandwidth(&ctx, &value);
                dash.refresh();
            });
            presets.append(&btn);
        }
        self.overview.append(&presets);
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
            let dash = self.clone();
            let custom = custom.clone();
            apply.connect_clicked(move |_| {
                apply_bandwidth(&ctx, &custom.text());
                dash.refresh();
            });
        }
        custom.add_suffix(&apply);
        self.overview.append(&custom);
    }

    fn render_system_panel(&self, snap: &crate::store::RuntimeSnapshot) {
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
            .unwrap_or_else(|| "unknown".into());
        let ver_row = adw::ActionRow::new();
        ver_row.set_title("rclone");
        ver_row.set_subtitle(&version);
        sys.append(&ver_row);
        if let Some(mem) = self.ctx.client().and_then(|c| c.memstats().ok()) {
            let alloc = mem.get("Alloc").and_then(|x| x.as_i64()).unwrap_or(0);
            let sys_bytes = mem.get("Sys").and_then(|x| x.as_i64()).unwrap_or(0);
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or("dashboard.system.memory", "Memory"));
            row.set_subtitle(&format!(
                "{} alloc · {} sys",
                format_bytes(alloc),
                format_bytes(sys_bytes)
            ));
            sys.append(&row);
        }
        let jobs_row = adw::ActionRow::new();
        jobs_row.set_title(&self.ctx.t_or("dashboard.system.activity", "Activity"));
        jobs_row.set_subtitle(&format!(
            "{} running jobs · {} mounts · {} serves",
            snap.jobs.iter().filter(|j| j.status == "running").count(),
            snap.mounts.len(),
            snap.serves.len()
        ));
        sys.append(&jobs_row);
        self.overview.append(&sys);
    }

    fn render_automations_panel(&self) {
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
            row.set_subtitle(&self.ctx.t_or(
                "generalOverview.automations.noConfigured",
                "Enable cron or watch on a profile or quick run",
            ));
            autos.append(&row);
        } else {
            for record in records {
                let paused = self.ctx.store.borrow().is_automation_paused(&record.id);
                let row = adw::ActionRow::new();
                row.set_title(&record.name);
                let next = record
                    .next_run
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "—".into());
                row.set_subtitle(&format!(
                    "{} · cron={} · watch={} · next {next}{}",
                    record.operation,
                    if record.cron_enabled {
                        record.cron.as_str()
                    } else {
                        "off"
                    },
                    if record.watch_enabled { "on" } else { "off" },
                    if paused { " · paused" } else { "" }
                ));
                let enabled = gtk::Switch::new();
                enabled.set_valign(gtk::Align::Center);
                enabled.set_tooltip_text(Some("Pause or resume this automation"));
                enabled.set_active(!paused);
                {
                    let ctx = self.ctx.clone();
                    let id = record.id.clone();
                    let dash = self.clone();
                    enabled.connect_active_notify(move |row| {
                        let mut store = ctx.store.borrow_mut();
                        let paused = store.is_automation_paused(&id);
                        if row.is_active() == paused {
                            store.toggle_automation_paused(&id);
                            drop(store);
                            ctx.persist();
                            dash.refresh();
                        }
                    });
                }
                let run = gtk::Button::from_icon_name("media-playback-start-symbolic");
                run.set_valign(gtk::Align::Center);
                run.set_tooltip_text(Some("Run now"));
                let nav_id = record.id.clone();
                {
                    let ctx = self.ctx.clone();
                    let toast = self.toast.clone();
                    run.connect_clicked(move |_| {
                        if let Some(client) = ctx.client() {
                            let mut store = ctx.store.borrow_mut();
                            match crate::automation::fire(
                                &client,
                                &mut store,
                                &record,
                                chrono::Utc::now(),
                            ) {
                                Ok(id) => {
                                    toast.add_toast(adw::Toast::new(&format!("Started {id}")))
                                }
                                Err(e) => {
                                    toast.add_toast(adw::Toast::new(&ctx.translate_error(&e)))
                                }
                            }
                        }
                        ctx.persist();
                        ctx.refresh_runtime();
                    });
                }
                row.add_suffix(&enabled);
                row.add_suffix(&run);
                {
                    let ctx = self.ctx.clone();
                    row.connect_activated(move |_| {
                        ctx.request_nav(NavTarget::Automation { id: nav_id.clone() });
                    });
                }
                autos.append(&row);
            }
        }
        self.overview.append(&autos);
    }

    fn fill_detail(&self) {
        clear_box(&self.detail);
        let Some(name) = self.ctx.selected_remote.borrow().clone() else {
            return;
        };
        let snap = self.ctx.snapshot.borrow().clone();
        let remote = snap.remotes.iter().find(|r| r.name == name).cloned();
        let tab = *self.tab.borrow();
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
        if let Some(remote) = remote.as_ref() {
            let icon = gtk::Image::from_icon_name(crate::providers::provider_icon(&remote.r#type));
            icon.set_pixel_size(28);
            header.append(&icon);
        }
        header.append(&title);
        self.detail.append(&header);

        if let Some(remote) = remote.as_ref() {
            let subtitle = gtk::Label::new(Some(&format!(
                "{} · {}",
                remote.r#type,
                remote_state_label(remote.mounted, remote.serving, remote.job_active)
            )));
            subtitle.add_css_class("dim-label");
            subtitle.set_xalign(0.0);
            self.detail.append(&subtitle);
        }

        self.append_status_chips(&name, remote.as_ref());
        if tab == AppTab::Operations {
            self.append_sync_op_picker(&name);
        }

        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_max_children_per_line(6);
        let ops = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(&name)
            .map(|m| m.visible_operations())
            .unwrap_or_else(|| OperationType::ALL.to_vec())
            .into_iter()
            .filter(|op| tab.includes_operation(*op))
            .collect::<Vec<_>>();
        for op in ops {
            let btn = gtk::Button::new();
            btn.set_label(op.api_label());
            btn.set_tooltip_text(Some(op.as_str()));
            {
                let ctx = self.ctx.clone();
                let name = name.clone();
                let toast = self.toast.clone();
                let dash = self.clone();
                btn.connect_clicked(move |_| {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::start_operation(&win, ctx.clone(), &name, op, toast.clone(), {
                            let dash = dash.clone();
                            Rc::new(move || dash.refresh())
                        });
                    } else {
                        start_operation(&ctx, &name, op, &toast);
                        dash.refresh();
                    }
                });
            }
            chips.append(&btn);
        }
        self.detail.append(&chips);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let tray_on = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(&name)
            .map(|m| m.show_on_tray)
            .unwrap_or(false);
        let tray = gtk::ToggleButton::new();
        tray.set_icon_name("view-pin-symbolic");
        tray.set_active(tray_on);
        tray.set_tooltip_text(Some(&self.ctx.t_or("remote.showInTray", "Show in tray")));
        {
            let ctx = self.ctx.clone();
            let name = name.clone();
            tray.connect_toggled(move |btn| {
                ctx.store
                    .borrow_mut()
                    .remotes
                    .entry(name.clone())
                    .or_default()
                    .show_on_tray = btn.is_active();
                ctx.persist();
            });
        }
        actions.append(&tray);
        for (label, icon, kind) in [
            ("Browse", "folder-symbolic", "browse"),
            ("About", "dialog-information-symbolic", "about"),
            ("Logs", "utilities-terminal-symbolic", "logs"),
            ("Export", "document-save-symbolic", "export"),
            ("Clone", "edit-copy-symbolic", "clone"),
            ("Configure", "emblem-system-symbolic", "config"),
            ("Reset", "view-refresh-symbolic", "reset"),
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
                    ctx.request_browse(&name, "");
                }
                "about" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::remote_about(&win, ctx.clone(), &name);
                    }
                }
                "logs" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::logs(&win, ctx.clone(), Some(name.clone()));
                    }
                }
                "export" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::export_backup(&win, ctx.clone(), toast.clone(), Some(&name));
                    }
                }
                "reset" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        let dialog = adw::AlertDialog::new(
                            Some(&ctx.t_or("remote.resetSettings", "Reset remote settings?")),
                            Some(&ctx.t_or(
                                "remote.resetSettingsBody",
                                "Profiles, helpers, and automations for this remote will be removed. The rclone remote stays.",
                            )),
                        );
                        dialog.add_response("cancel", &ctx.t("common.cancel"));
                        dialog.add_response(
                            "reset",
                            &ctx.t_or("remote.resetSettingsConfirm", "Reset"),
                        );
                        dialog.set_response_appearance(
                            "reset",
                            adw::ResponseAppearance::Destructive,
                        );
                        let ctx = ctx.clone();
                        let name = name.clone();
                        let dash = dash.clone();
                        dialog.connect_response(None, move |_, response| {
                            if response != "reset" {
                                return;
                            }
                            ctx.store.borrow_mut().reset_remote_settings(&name);
                            ctx.persist();
                            ctx.refresh_runtime();
                            dash.refresh();
                        });
                        dialog.present(Some(&win));
                    }
                }
                "clone" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::clone_remote(&win, ctx.clone(), &name, {
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

        self.append_disk_usage(&name);

        self.detail.append(&section_label(
            &self.ctx.t_or("generalOverview.activity", "Activity"),
        ));
        let activity = gtk::ListBox::new();
        activity.add_css_class("boxed-list");
        let remote_jobs: Vec<_> = snap
            .jobs
            .iter()
            .filter(|j| j.remote == name)
            .cloned()
            .collect();
        if remote_jobs.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or(
                "generalOverview.jobs.noRunning",
                "No active jobs for this remote",
            ));
            activity.append(&row);
        } else {
            for job in remote_jobs {
                let row = adw::ActionRow::new();
                row.set_title(&format!("{} · {}", job.operation, job.status));
                row.set_subtitle(&format!(
                    "{:.0}% · {} · {}",
                    job.progress * 100.0,
                    crate::rclone::format_bytes(
                        job.stats.get("bytes").and_then(|x| x.as_i64()).unwrap_or(0)
                    ),
                    job.profile
                ));
                let ctx = self.ctx.clone();
                let id = job.id;
                row.connect_activated(move |_| {
                    ctx.request_nav(NavTarget::Job { id });
                });
                activity.append(&row);
            }
        }
        let remote_serves: Vec<_> = snap
            .serves
            .iter()
            .filter(|s| s.fs.contains(&name))
            .cloned()
            .collect();
        for serve in remote_serves {
            let row = adw::ActionRow::new();
            row.set_title(&format!("Serve · {}", serve.serve_type));
            row.set_subtitle(&serve.addr);
            {
                let ctx = self.ctx.clone();
                let id = serve.id.clone();
                row.connect_activated(move |_| {
                    ctx.request_nav(NavTarget::Serve { id: id.clone() });
                });
            }
            activity.append(&row);
        }
        self.detail.append(&activity);
        self.append_transfer_activity(&name, &snap);
        if tab == AppTab::General {
            self.append_remote_automations(&name);
        }

        self.detail
            .append(&section_label(&self.ctx.t_or("remote.vfs", "VFS")));
        let vfs = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let open_vfs = gtk::Button::with_label(&self.ctx.t_or("remote.vfsPanel", "VFS panel"));
        open_vfs.add_css_class("suggested-action");
        {
            let ctx = self.ctx.clone();
            let name = name.clone();
            let toast = self.toast.clone();
            let root = self.root.clone();
            open_vfs.connect_clicked(move |_| {
                if let Some(win) = root.root().and_downcast::<gtk::Window>() {
                    dialogs::vfs_control(&win, ctx.clone(), &name, toast.clone());
                }
            });
        }
        vfs.append(&open_vfs);
        for (label, action) in [("Refresh", "refresh"), ("Forget cache", "forget")] {
            let btn = gtk::Button::with_label(label);
            let ctx = self.ctx.clone();
            let name = name.clone();
            let toast = self.toast.clone();
            btn.connect_clicked(move |_| {
                let Some(client) = ctx.client() else {
                    toast.add_toast(adw::Toast::new("Engine offline"));
                    return;
                };
                let fs = remote_fs(&name, "");
                let result = match action {
                    "refresh" => client.vfs_refresh_ex(&fs, None, true),
                    _ => client.vfs_forget(&fs),
                };
                match result {
                    Ok(_) => toast.add_toast(adw::Toast::new(&format!("{action} finished"))),
                    Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            });
            vfs.append(&btn);
        }
        self.detail.append(&vfs);

        self.detail.append(&section_label(
            &self.ctx.t_or("remote.profiles", "Profiles"),
        ));
        let plist = gtk::ListBox::new();
        plist.add_css_class("boxed-list");
        if let Some(meta) = self.ctx.store.borrow().remotes.get(&name) {
            for (op, profiles) in &meta.profiles {
                let Some(op_ty) = crate::operations::OperationType::parse(op) else {
                    continue;
                };
                if !tab.includes_operation(op_ty) {
                    continue;
                }
                for (pname, profile) in profiles {
                    let row = adw::ActionRow::new();
                    row.set_activatable(true);
                    row.set_title(&format!("{op} / {pname}"));
                    row.set_subtitle(&crate::jobs::profile_summary(op_ty, profile));
                    let snap = self.ctx.snapshot.borrow();
                    let active = crate::jobs::profile_is_active(
                        &name,
                        op_ty,
                        pname,
                        &snap.jobs,
                        &snap.mounts,
                        &snap.serves,
                    );
                    drop(snap);
                    let start = gtk::Button::from_icon_name(if active {
                        "media-playback-stop-symbolic"
                    } else {
                        "media-playback-start-symbolic"
                    });
                    start.set_valign(gtk::Align::Center);
                    start.set_tooltip_text(Some(&if active {
                        self.ctx.t_or("remote.stopProfile", "Stop profile")
                    } else {
                        self.ctx.t_or("remote.startProfile", "Start profile")
                    }));
                    {
                        let ctx = self.ctx.clone();
                        let toast = self.toast.clone();
                        let remote = name.clone();
                        let pname = pname.clone();
                        start.connect_clicked(move |_| {
                            toggle_profile(&ctx, &remote, op_ty, &pname, &toast);
                        });
                    }
                    row.add_suffix(&start);
                    {
                        let ctx = self.ctx.clone();
                        let dash = self.clone();
                        let remote = name.clone();
                        let op_key = op.clone();
                        let pname = pname.clone();
                        row.connect_activated(move |_| {
                            if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                                dialogs::remote_config_open(
                                    &win,
                                    ctx.clone(),
                                    Some(remote.clone()),
                                    super::remote_config::RemoteConfigOpen {
                                        initial: Some(op_key.clone()),
                                        profile: Some(pname.clone()),
                                        auto_add: false,
                                    },
                                    {
                                        let dash = dash.clone();
                                        let ctx = ctx.clone();
                                        Rc::new(move || {
                                            ctx.refresh_runtime();
                                            dash.refresh();
                                        })
                                    },
                                );
                            }
                        });
                    }
                    plist.append(&row);
                }
            }
        }
        if plist.first_child().is_none() {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or(
                "remote.noProfiles",
                "No saved profiles — configure the remote to add them",
            ));
            plist.append(&row);
        }
        self.detail.append(&plist);
        let add_profile =
            gtk::Button::with_label(&self.ctx.t_or("remote.addProfile", "Add profile"));
        {
            let ctx = self.ctx.clone();
            let remote = name.clone();
            let dash = self.clone();
            add_profile.connect_clicked(move |_| {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::remote_config_open(
                        &win,
                        ctx.clone(),
                        Some(remote.clone()),
                        super::remote_config::RemoteConfigOpen {
                            initial: Some(
                                if tab == AppTab::Operations {
                                    dash.selected_sync_op(&remote)
                                } else {
                                    tab.default_operation()
                                }
                                .as_str()
                                .to_string(),
                            ),
                            profile: None,
                            auto_add: true,
                        },
                        {
                            let dash = dash.clone();
                            let ctx = ctx.clone();
                            Rc::new(move || {
                                ctx.refresh_runtime();
                                dash.refresh();
                            })
                        },
                    );
                }
            });
        }
        self.detail.append(&add_profile);
        let helpers =
            gtk::Button::with_label(&self.ctx.t_or("remote.editHelpers", "Edit helper profiles"));
        {
            let ctx = self.ctx.clone();
            let remote = name.clone();
            let dash = self.clone();
            helpers.connect_clicked(move |_| {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::helper_profiles(&win, ctx.clone(), &remote);
                }
            });
        }
        self.detail.append(&helpers);

        self.detail.append(&section_label(
            &self
                .ctx
                .t_or("remote.quickRuns", "Quick Runs for this remote"),
        ));
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
            row.set_title(&self.ctx.t_or("dashboard.quickRuns.empty", "No quick runs"));
            qlist.append(&row);
        }
        self.detail.append(&qlist);

        self.detail.append(&section_label("Settings"));
        let slist = gtk::ListBox::new();
        slist.add_css_class("boxed-list");
        let restrict = self.ctx.settings.borrow().general.restrict;
        let mut rows = Vec::new();
        if let Some(meta) = self.ctx.store.borrow().remotes.get(&name) {
            for (op, profiles) in &meta.profiles {
                for (pname, profile) in profiles {
                    let value = serde_json::to_value(profile).unwrap_or(serde_json::json!({}));
                    for (key, display) in crate::restrict::flatten_settings(
                        &format!("{op}.{pname}"),
                        &value,
                        restrict,
                    ) {
                        rows.push((key, display));
                    }
                }
            }
        }
        rows.truncate(24);
        if rows.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(
                &self
                    .ctx
                    .t_or("dashboard.settings.empty", "No saved profile settings"),
            );
            slist.append(&row);
        } else {
            for (key, display) in rows {
                let row = adw::ActionRow::new();
                row.set_title(&key);
                row.set_subtitle(&display);
                slist.append(&row);
            }
        }
        self.detail.append(&slist);
    }

    fn selected_sync_op(&self, name: &str) -> OperationType {
        self.ctx
            .settings
            .borrow()
            .runtime
            .selected_sync_ops
            .get(name)
            .and_then(|value| OperationType::parse(value))
            .filter(|op| AppTab::Operations.includes_operation(*op))
            .unwrap_or(OperationType::Sync)
    }

    fn append_sync_op_picker(&self, name: &str) {
        let selected = self.selected_sync_op(name);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("linked");
        for op in OperationType::PRIMARY_SYNC {
            let btn = gtk::ToggleButton::with_label(op.api_label());
            btn.set_active(op == selected);
            btn.set_tooltip_text(Some(op.as_str()));
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let dash = self.clone();
            btn.connect_clicked(move |_| {
                ctx.settings
                    .borrow_mut()
                    .runtime
                    .selected_sync_ops
                    .insert(remote.clone(), op.as_str().to_string());
                ctx.persist();
                dash.refresh();
            });
            row.append(&btn);
        }
        self.detail.append(&row);
    }

    fn append_status_chips(&self, name: &str, remote: Option<&crate::store::RemoteInfo>) {
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("linked");
        let mounted = remote.map(|r| r.mounted).unwrap_or(false);
        let serving = remote.map(|r| r.serving).unwrap_or(false);
        let job_active = remote.map(|r| r.job_active).unwrap_or(false);
        for (label, active, kind) in [
            (self.ctx.t_or("tabs.mount", "Mount"), mounted, "mount"),
            (
                self.ctx.t_or("tabs.operations", "Operations"),
                job_active,
                "ops",
            ),
            (self.ctx.t_or("tabs.serve", "Serve"), serving, "serve"),
        ] {
            let btn = gtk::Button::with_label(&format!(
                "{} {}",
                if active { "Stop" } else { "Start" },
                label
            ));
            if active {
                btn.add_css_class("destructive-action");
            }
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let toast = self.toast.clone();
            let dash = self.clone();
            let mounted = mounted;
            btn.connect_clicked(move |_| match kind {
                "mount" => {
                    toggle_mount(&ctx, &remote, mounted, &toast);
                    dash.refresh();
                }
                "ops" => {
                    let op = dash.selected_sync_op(&remote);
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::start_operation(&win, ctx.clone(), &remote, op, toast.clone(), {
                            let dash = dash.clone();
                            Rc::new(move || dash.refresh())
                        });
                    }
                }
                "serve" => {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        dialogs::start_operation(
                            &win,
                            ctx.clone(),
                            &remote,
                            OperationType::Serve,
                            toast.clone(),
                            {
                                let dash = dash.clone();
                                Rc::new(move || dash.refresh())
                            },
                        );
                    }
                }
                _ => {}
            });
            chips.append(&btn);
        }
        self.detail.append(&chips);
    }

    fn append_disk_usage(&self, name: &str) {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let usage = adw::ActionRow::new();
        usage.set_title(&self.ctx.t_or("remote.diskUsage", "Disk usage"));
        let retry = gtk::Button::from_icon_name("view-refresh-symbolic");
        retry.set_valign(gtk::Align::Center);
        retry.set_tooltip_text(Some(&self.ctx.t_or("common.retry", "Retry")));
        {
            let dash = self.clone();
            retry.connect_clicked(move |_| dash.refresh());
        }
        usage.add_suffix(&retry);
        if let Some(client) = self.ctx.client() {
            match client.about(&remote_fs(name, "")) {
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
            usage.set_subtitle(&self.ctx.t_or("remote.engineOffline", "Engine offline"));
            box_.append(&usage);
        }
        self.detail.append(&box_);
    }

    fn append_transfer_activity(&self, name: &str, snap: &crate::store::RuntimeSnapshot) {
        let rows: Vec<_> = snap
            .jobs
            .iter()
            .filter(|job| job.remote == name)
            .flat_map(|job| {
                job.transferring
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|item| crate::transfers::parse_transfer_row(&item))
            })
            .take(8)
            .collect();
        if rows.is_empty() {
            return;
        }
        self.detail.append(&section_label(
            &self.ctx.t_or("jobDetail.transfers", "Current transfers"),
        ));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for row in rows {
            let item = adw::ActionRow::new();
            item.set_title(&row.name);
            item.set_subtitle(&format!("{}% · {}", row.percentage, row.src));
            list.append(&item);
        }
        self.detail.append(&list);
    }

    fn append_remote_automations(&self, name: &str) {
        let records: Vec<_> = crate::automation::collect(&self.ctx.store.borrow())
            .into_iter()
            .filter(|record| record.remote == name)
            .collect();
        if records.is_empty() {
            return;
        }
        self.detail.append(&section_label(
            &self
                .ctx
                .t_or("generalOverview.automations.title", "Automations"),
        ));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for record in records {
            let paused = self.ctx.store.borrow().is_automation_paused(&record.id);
            let row = adw::ActionRow::new();
            row.set_title(&record.name);
            row.set_subtitle(&format!(
                "{} · {}{}",
                record.operation,
                if record.cron_enabled {
                    record.cron.as_str()
                } else if record.watch_enabled {
                    "watch"
                } else {
                    "off"
                },
                if paused { " · paused" } else { "" }
            ));
            let enabled = gtk::Switch::new();
            enabled.set_valign(gtk::Align::Center);
            enabled.set_active(!paused);
            {
                let ctx = self.ctx.clone();
                let id = record.id.clone();
                let dash = self.clone();
                enabled.connect_active_notify(move |switch| {
                    let mut store = ctx.store.borrow_mut();
                    let paused = store.is_automation_paused(&id);
                    if switch.is_active() == paused {
                        store.toggle_automation_paused(&id);
                        drop(store);
                        ctx.persist();
                        dash.refresh();
                    }
                });
            }
            row.add_suffix(&enabled);
            list.append(&row);
        }
        self.detail.append(&list);
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

fn toggle_profile(
    ctx: &AppCtx,
    name: &str,
    op: OperationType,
    profile_name: &str,
    toast: &adw::ToastOverlay,
) {
    let Some(client) = ctx.client() else {
        toast.add_toast(adw::Toast::new("Rclone engine is offline"));
        return;
    };
    let snap = ctx.snapshot.borrow().clone();
    if crate::jobs::profile_is_active(
        name,
        op,
        profile_name,
        &snap.jobs,
        &snap.mounts,
        &snap.serves,
    ) {
        match crate::jobs::stop_profile(
            &client,
            name,
            op,
            profile_name,
            &snap.jobs,
            &snap.mounts,
            &snap.serves,
        ) {
            Ok(msg) => toast.add_toast(adw::Toast::new(&msg)),
            Err(e) => toast.add_toast(adw::Toast::new(&e)),
        }
        ctx.refresh_runtime();
        return;
    }
    let meta = ctx.store.borrow().remotes.get(name).cloned();
    let profile = meta
        .as_ref()
        .and_then(|m| m.get_profile(op, profile_name))
        .unwrap_or_default();
    match crate::jobs::start_profile(&client, name, op, &profile, meta.as_ref(), "dashboard") {
        Ok(id) => {
            crate::jobs::remember_started(
                &mut ctx.store.borrow_mut().job_meta,
                &id,
                crate::jobs::job_meta_for(name, &profile, "dashboard", &ctx.backend_key(), ""),
            );
            toast.add_toast(adw::Toast::new(&format!("Started {op} {id}")));
        }
        Err(e) => toast.add_toast(adw::Toast::new(&e)),
    }
    ctx.refresh_runtime();
}

fn start_operation(ctx: &AppCtx, name: &str, op: OperationType, toast: &adw::ToastOverlay) {
    let profile = ctx
        .store
        .borrow()
        .remotes
        .get(name)
        .and_then(|m| {
            m.profile_names(op)
                .into_iter()
                .next()
                .or_else(|| Some("default".into()))
        })
        .unwrap_or_else(|| "default".into());
    toggle_profile(ctx, name, op, &profile, toast);
}

fn default_mount_point(name: &str) -> String {
    crate::path_inspection::suggest_default_mount_path(name, &crate::store::AppStore::default())
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

fn ctx_settings_bandwidth(ctx: &AppCtx) -> String {
    ctx.settings.borrow().core.bandwidth_limit.clone()
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
