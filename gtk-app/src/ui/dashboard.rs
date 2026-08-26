use super::dialogs;
use super::job_panels;
use super::vfs_panel;
use super::AppCtx;
use crate::navigation::NavTarget;
use crate::operations::{AppTab, OperationType};
use crate::rclone::{format_bytes, remote_fs};
use adw::prelude::*;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
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
    dry_run: Rc<Cell<bool>>,
    resync: Rc<Cell<bool>>,
    origin_filter: Rc<RefCell<String>>,
    transfer_query: Rc<RefCell<String>>,
    transfer_tab: Rc<RefCell<String>>,
    panel_host: Rc<RefCell<Option<gtk::Box>>>,
    detail_host: Rc<RefCell<Option<gtk::Box>>>,
    detail_page: Rc<RefCell<String>>,
    detail_sig: Rc<RefCell<String>>,
    sidebar_sig: Rc<RefCell<String>>,
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
            dry_run: Rc::new(Cell::new(false)),
            resync: Rc::new(Cell::new(false)),
            origin_filter: Rc::new(RefCell::new("all".into())),
            transfer_query: Rc::new(RefCell::new(String::new())),
            transfer_tab: Rc::new(RefCell::new("all".into())),
            panel_host: Rc::new(RefCell::new(None)),
            detail_host: Rc::new(RefCell::new(None)),
            detail_page: Rc::new(RefCell::new("monitoring".into())),
            detail_sig: Rc::new(RefCell::new(String::new())),
            sidebar_sig: Rc::new(RefCell::new(String::new())),
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
                    let name = row.widget_name().to_string();
                    if !name.is_empty() && name != "GtkListBoxRow" {
                        *dash.ctx.selected_remote.borrow_mut() = Some(name);
                        dash.refresh();
                    }
                });
        }

        dash.refresh();
        dash
    }

    pub fn navigate(&self, tab: AppTab, remote: Option<&str>) {
        self.detail_sig.borrow_mut().clear();
        *self.ctx.selected_remote.borrow_mut() =
            remote.filter(|s| !s.is_empty()).map(|s| s.to_string());
        *self.tab.borrow_mut() = tab;
        for (candidate, btn) in self.tab_buttons.borrow().iter() {
            if *candidate == tab {
                if !btn.is_active() {
                    btn.set_active(true);
                }
                break;
            }
        }
        self.refresh();
    }

    pub fn refresh(&self) {
        self.detail_sig.borrow_mut().clear();
        self.sidebar_sig.borrow_mut().clear();
        self.refresh_inner();
    }

    pub fn poll_refresh(&self) {
        self.refresh_inner();
    }

    fn refresh_inner(&self) {
        if self.should_rebuild_sidebar() {
            self.fill_sidebar();
        }
        if let Some(name) = self.ctx.selected_remote.borrow().clone() {
            self.content.set_visible_child_name("detail");
            if self.should_rebuild_detail(&name) {
                self.fill_detail();
            }
        } else {
            self.detail_sig.borrow_mut().clear();
            self.content.set_visible_child_name("overview");
            self.fill_overview();
        }
    }

    fn should_rebuild_sidebar(&self) -> bool {
        let query = self.search.text().to_lowercase();
        let snap = self.ctx.snapshot.borrow();
        let sig = snap
            .remotes
            .iter()
            .map(|remote| {
                format!(
                    "{}:{}:{}:{}:{}",
                    remote.name, remote.r#type, remote.mounted, remote.serving, remote.job_active
                )
            })
            .collect::<Vec<_>>()
            .join("|");
        let sig = format!("{query}:{sig}");
        drop(snap);
        let mut prev = self.sidebar_sig.borrow_mut();
        if *prev == sig {
            return false;
        }
        *prev = sig;
        true
    }

    fn should_rebuild_detail(&self, name: &str) -> bool {
        let tab = *self.tab.borrow();
        let snap = self.ctx.snapshot.borrow();
        let jobs: String = snap
            .jobs
            .iter()
            .filter(|job| {
                job.remote == name
                    && (crate::jobs::job_is_running(job) || crate::jobs::job_is_pending(job))
            })
            .map(|job| format!("{}:{}:{:.2}", job.id, job.status, job.progress))
            .collect::<Vec<_>>()
            .join(",");
        let mounts = snap
            .mounts
            .iter()
            .filter(|item| item.fs.contains(name))
            .count();
        let serves = snap
            .serves
            .iter()
            .filter(|item| item.fs.contains(name))
            .count();
        let sig = format!(
            "{name}:{:?}:{jobs}:m{mounts}:s{serves}:{}",
            tab,
            self.detail_page.borrow().as_str(),
        );
        drop(snap);
        let mut prev = self.detail_sig.borrow_mut();
        if *prev == sig {
            return false;
        }
        *prev = sig;
        true
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
            row.set_widget_name(&remote.name);
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
        self.host().append(&title);

        let editing = *self.editing_layout.borrow();
        let layout_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let edit_label = if editing {
            self.ctx.t_or("common.done", "Done")
        } else {
            self.ctx.t_or("generalOverview.editLayout", "Edit layout")
        };
        let edit_btn = gtk::Button::with_label(&edit_label);
        edit_btn.set_tooltip_text(Some(&self.ctx.t_or(
            "generalOverview.editLayout",
            "Hide or reorder remotes and overview panels",
        )));
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
        order_btn.set_tooltip_text(Some(&self.ctx.t_or(
            "generalOverview.reorderRemotes",
            "Open the remote order and visibility editor",
        )));
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
        reset.set_tooltip_text(Some(&self.ctx.t_or(
            "generalOverview.resetPanels",
            "Restore the default overview panel order",
        )));
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
        if editing {
            let detailed = self.ctx.settings.borrow().runtime.dashboard_card_variant == "detailed";
            let variant_label = if detailed {
                self.ctx
                    .t_or("generalOverview.layout.showCompact", "Show Compact Cards")
            } else {
                self.ctx
                    .t_or("generalOverview.layout.showDetailed", "Show Detailed Cards")
            };
            let variant_btn = gtk::Button::with_label(&variant_label);
            variant_btn.set_tooltip_text(Some(&variant_label));
            {
                let dash = self.clone();
                variant_btn.connect_clicked(move |_| {
                    let next = if dash.ctx.settings.borrow().runtime.dashboard_card_variant
                        == "detailed"
                    {
                        "compact"
                    } else {
                        "detailed"
                    };
                    dash.ctx
                        .settings
                        .borrow_mut()
                        .runtime
                        .dashboard_card_variant = next.into();
                    dash.ctx.persist();
                    dash.refresh();
                });
            }
            layout_bar.append(&variant_btn);
        }
        self.host().append(&layout_bar);
        self.host().append(&self.origin_filter_bar());
        let layout = crate::layout::PanelLayout::from_value(
            &self.ctx.settings.borrow().runtime.dashboard_layout,
        );
        for (id, visible) in layout.resolve(crate::layout::DASHBOARD_PANELS) {
            if !visible && !editing {
                continue;
            }
            self.append_panel(&id, |dash| match id.as_str() {
                "remotes" => dash.render_remotes_panel(&remotes, tab, editing),
                "jobs" => dash.render_jobs_panel(&snap),
                "serves" => dash.render_serves_panel(&snap),
                "bandwidth" => dash.render_bandwidth_panel(&snap),
                "system" => dash.render_system_panel(&snap),
                "automations" => dash.render_automations_panel(),
                _ => {}
            });
        }
    }

    fn origin_filter_bar(&self) -> gtk::Box {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("linked");
        bar.set_halign(gtk::Align::Start);
        bar.set_margin_bottom(4);
        let current = self.origin_filter.borrow().clone();
        for (id, key, fallback) in [
            ("all", "common.all", "All"),
            ("dashboard", "navigation.dashboard", "Dashboard"),
            ("quickrun", "flow.tabs.quickRun", "Quick Run"),
            ("filemanager", "navigation.files", "Files"),
            (
                "automation",
                "generalOverview.panels.automations",
                "Automations",
            ),
        ] {
            let btn = gtk::ToggleButton::with_label(&self.ctx.t_or(key, fallback));
            btn.set_active(current == id);
            {
                let dash = self.clone();
                let id = id.to_string();
                btn.connect_clicked(move |_| {
                    *dash.origin_filter.borrow_mut() = id.clone();
                    dash.refresh();
                });
            }
            bar.append(&btn);
        }
        bar
    }

    fn origin_filter(&self) -> String {
        self.origin_filter.borrow().clone()
    }

    fn host(&self) -> gtk::Box {
        self.panel_host
            .borrow()
            .clone()
            .unwrap_or_else(|| self.overview.clone())
    }

    fn detail_box(&self) -> gtk::Box {
        self.detail_host
            .borrow()
            .clone()
            .unwrap_or_else(|| self.detail.clone())
    }

    fn append_panel(&self, id: &str, build: impl FnOnce(&Self)) {
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let expander = gtk::Expander::new(None);
        let label = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        label.append(&section_label(crate::layout::panel_title(id)));
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
            label.append(&hide);
            label.append(&up);
            label.append(&down);
        }
        expander.set_label_widget(Some(&label));
        expander.set_expanded(crate::settings::panel_is_open(
            &self.ctx.settings.borrow().runtime.panel_open_states,
            "dashboard",
            id,
        ));
        {
            let ctx = self.ctx.clone();
            let id = id.to_string();
            expander.connect_expanded_notify(move |exp| {
                let changed = crate::settings::set_panel_open(
                    &mut ctx.settings.borrow_mut().runtime.panel_open_states,
                    "dashboard",
                    &id,
                    exp.is_expanded(),
                );
                if changed {
                    ctx.persist();
                }
            });
        }
        *self.panel_host.borrow_mut() = Some(inner.clone());
        build(self);
        *self.panel_host.borrow_mut() = None;
        expander.set_child(Some(&inner));
        self.host().append(&expander);
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
        let total = remotes.len();
        if total > 0 {
            let summary = adw::ActionRow::new();
            summary.set_title(
                &self
                    .ctx
                    .t_or("dashboard.statusOverview.title", "Status overview"),
            );
            let pct = (active.len() * 100) / total;
            summary.set_subtitle(&format!(
                "{} / {total} {} · {pct}%",
                active.len(),
                self.ctx.t_or("dashboard.statusOverview.active", "active")
            ));
            self.host().append(&summary);
        }
        if !active.is_empty() {
            self.host().append(&section_label(
                &self
                    .ctx
                    .t_or(tab.active_section_key(), tab.active_section_fallback()),
            ));
            self.append_remote_list(&active, tab, editing);
        }
        if !idle.is_empty() {
            self.host().append(&section_label(
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
                remote_state_label(&self.ctx, remote.mounted, remote.serving, remote.job_active,),
                if remote.hidden {
                    format!(
                        " · {}",
                        self.ctx.t_or("generalOverview.layout.hidden", "Hidden")
                    )
                } else {
                    String::new()
                }
            ));
            row.add_prefix(&gtk::Image::from_icon_name(
                crate::providers::provider_icon(&remote.r#type),
            ));
            if remote.hidden {
                row.add_css_class("dim-label");
            }
            let browse = gtk::Button::from_icon_name("folder-symbolic");
            browse.set_valign(gtk::Align::Center);
            browse.set_tooltip_text(Some(&self.ctx.t_or("common.browse", "Browse")));
            let mount = gtk::Button::from_icon_name("drive-harddisk-symbolic");
            mount.set_valign(gtk::Align::Center);
            mount.set_tooltip_text(Some(
                &self.ctx.t_or("remote.mountToggle", "Mount / Unmount"),
            ));
            {
                let ctx = self.ctx.clone();
                let name = remote.name.clone();
                let dash = self.clone();
                browse.connect_clicked(move |_| {
                    *ctx.selected_remote.borrow_mut() = Some(name.clone());
                    ctx.browse_remote_home(&name);
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
            if detailed {
                row.add_suffix(&mount);
            } else {
                let compact_ops = self
                    .ctx
                    .store
                    .borrow()
                    .remotes
                    .get(&remote.name)
                    .map(|meta| {
                        tab.compact_primary_ops(&meta.primary_actions, &meta.sync_actions, 3)
                    })
                    .unwrap_or_else(|| tab.compact_primary_ops(&[], &[], 3));
                let snap = self.ctx.snapshot.borrow().clone();
                let mut open_paths: Vec<String> = Vec::new();
                for op in compact_ops.iter().copied() {
                    let names = self
                        .ctx
                        .store
                        .borrow()
                        .remotes
                        .get(&remote.name)
                        .map(|meta| meta.profile_names(op))
                        .unwrap_or_default();
                    let active = match op {
                        OperationType::Mount => remote.mounted,
                        OperationType::Serve => remote.serving,
                        other => snap.jobs.iter().any(|job| {
                            crate::jobs::job_belongs_to_remote(job, &remote.name)
                                && crate::jobs::job_operation_matches(&job.operation, other)
                                && crate::jobs::job_is_running(job)
                        }),
                    };
                    if active {
                        match op {
                            OperationType::Mount => {
                                let point = snap
                                    .mounts
                                    .iter()
                                    .find(|m| {
                                        m.fs.trim_end_matches(':') == remote.name
                                            || m.fs == format!("{}:", remote.name)
                                    })
                                    .map(|m| m.mount_point.as_str());
                                open_paths
                                    .extend(crate::jobs::active_open_paths(op, "", "", point));
                            }
                            OperationType::Serve => {}
                            other => {
                                if let Some(job) = snap.jobs.iter().find(|job| {
                                    crate::jobs::job_belongs_to_remote(job, &remote.name)
                                        && crate::jobs::job_operation_matches(&job.operation, other)
                                        && crate::jobs::job_is_running(job)
                                }) {
                                    open_paths.extend(crate::jobs::active_open_paths(
                                        other, &job.src, &job.dst, None,
                                    ));
                                }
                            }
                        }
                    }
                    let label = self
                        .ctx
                        .t_or(&format!("actions.{}", op.as_str()), op.api_label());
                    let verb = if active {
                        self.ctx.t_or("actions.stop", "Stop")
                    } else {
                        self.ctx.t_or("actions.start", "Start")
                    };
                    if names.is_empty() && !crate::jobs::allows_unconfigured_start(op) {
                        let btn = gtk::Button::from_icon_name(op.icon_name());
                        btn.set_valign(gtk::Align::Center);
                        btn.set_sensitive(false);
                        btn.set_tooltip_text(Some(&self.ctx.t_or(
                            "modals.remoteConfig.profile.noProfiles",
                            "No profiles configured",
                        )));
                        row.add_suffix(&btn);
                    } else if names.len() > 1 {
                        let btn = gtk::MenuButton::new();
                        btn.set_icon_name(op.icon_name());
                        btn.set_valign(gtk::Align::Center);
                        btn.set_tooltip_text(Some(&format!("{verb} {label} · {}", names.len())));
                        if active {
                            btn.add_css_class("destructive-action");
                        }
                        if names.iter().any(|pname| {
                            crate::jobs::action_in_progress(
                                &remote.name,
                                op,
                                pname,
                                &snap.jobs,
                                self.ctx.is_busy(&remote.name, op.as_str(), pname),
                            )
                        }) {
                            btn.set_sensitive(false);
                            btn.set_icon_name("content-loading-symbolic");
                            btn.set_tooltip_text(Some(
                                &self
                                    .ctx
                                    .t_or("remote.actionInProgress", "Action already in progress"),
                            ));
                        }
                        let popover = gtk::Popover::new();
                        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
                        list.set_margin_top(6);
                        list.set_margin_bottom(6);
                        list.set_margin_start(6);
                        list.set_margin_end(6);
                        for pname in names {
                            let profile_active = crate::jobs::profile_is_active(
                                &remote.name,
                                op,
                                &pname,
                                &snap.jobs,
                                &snap.mounts,
                                &snap.serves,
                            );
                            let item = gtk::Button::with_label(&pname);
                            if profile_active {
                                item.add_css_class("destructive-action");
                            }
                            mark_action_busy(
                                &item,
                                crate::jobs::action_in_progress(
                                    &remote.name,
                                    op,
                                    &pname,
                                    &snap.jobs,
                                    self.ctx.is_busy(&remote.name, op.as_str(), &pname),
                                ),
                                &self.ctx,
                            );
                            let ctx = self.ctx.clone();
                            let name = remote.name.clone();
                            let toast = self.toast.clone();
                            let dash = self.clone();
                            let dry = self.dry_run.clone();
                            let resync = self.resync.clone();
                            let popover = popover.clone();
                            item.connect_clicked(move |_| {
                                toggle_profile(
                                    &ctx,
                                    &name,
                                    op,
                                    &pname,
                                    &toast,
                                    dry.get(),
                                    resync.get(),
                                );
                                popover.popdown();
                                dash.refresh();
                            });
                            list.append(&item);
                        }
                        popover.set_child(Some(&list));
                        btn.set_popover(Some(&popover));
                        row.add_suffix(&btn);
                    } else {
                        let btn = gtk::Button::from_icon_name(op.icon_name());
                        btn.set_valign(gtk::Align::Center);
                        btn.set_tooltip_text(Some(&format!("{verb} {label}")));
                        if active {
                            btn.add_css_class("destructive-action");
                        }
                        let ctx = self.ctx.clone();
                        let name = remote.name.clone();
                        let mounted = remote.mounted;
                        let toast = self.toast.clone();
                        let dash = self.clone();
                        let dry = self.dry_run.clone();
                        let resync = self.resync.clone();
                        let pname = names.into_iter().next().unwrap_or_else(|| "default".into());
                        mark_action_busy(
                            &btn,
                            crate::jobs::action_in_progress(
                                &remote.name,
                                op,
                                &pname,
                                &snap.jobs,
                                self.ctx.is_busy(&remote.name, op.as_str(), &pname),
                            ),
                            &self.ctx,
                        );
                        btn.connect_clicked(move |_| {
                            if op == OperationType::Mount {
                                toggle_mount(&ctx, &name, mounted, &toast);
                            } else {
                                toggle_profile(
                                    &ctx,
                                    &name,
                                    op,
                                    &pname,
                                    &toast,
                                    dry.get(),
                                    resync.get(),
                                );
                            }
                            dash.refresh();
                        });
                        row.add_suffix(&btn);
                    }
                }
                open_paths.sort();
                open_paths.dedup();
                append_open_folder_suffix(&row, &self.ctx, &remote.name, &open_paths);
                let overflow = crate::jobs::overflow_active_ops(
                    &compact_ops,
                    &crate::jobs::active_remote_ops(
                        &remote.name,
                        remote.mounted,
                        remote.serving,
                        &snap.jobs,
                    ),
                );
                for op in overflow {
                    let pill = gtk::Image::from_icon_name(op.icon_name());
                    pill.set_tooltip_text(Some(
                        &self
                            .ctx
                            .t_or(&format!("actions.{}", op.as_str()), op.api_label()),
                    ));
                    pill.add_css_class("accent");
                    row.add_suffix(&pill);
                }
            }
            if editing {
                let hide = gtk::Button::from_icon_name(if remote.hidden {
                    "view-reveal-symbolic"
                } else {
                    "view-conceal-symbolic"
                });
                hide.set_valign(gtk::Align::Center);
                hide.set_tooltip_text(Some(&if remote.hidden {
                    self.ctx
                        .t_or("generalOverview.showRemote", "Show on overview")
                } else {
                    self.ctx
                        .t_or("generalOverview.hideRemote", "Hide from overview")
                }));
                let up = gtk::Button::from_icon_name("go-up-symbolic");
                up.set_valign(gtk::Align::Center);
                up.set_tooltip_text(Some(&self.ctx.t_or("generalOverview.moveUp", "Move up")));
                let down = gtk::Button::from_icon_name("go-down-symbolic");
                down.set_valign(gtk::Align::Center);
                down.set_tooltip_text(Some(
                    &self.ctx.t_or("generalOverview.moveDown", "Move down"),
                ));
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
                let mut groups = 0usize;
                if let Some(meta) = self.ctx.store.borrow().remotes.get(&remote.name) {
                    for op in meta.visible_operations() {
                        if !tab.includes_operation(op) {
                            continue;
                        }
                        let names = meta.profile_names(op);
                        if names.is_empty() {
                            continue;
                        }
                        let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
                        let header = gtk::Label::new(Some(
                            &self
                                .ctx
                                .t_or(&format!("actions.{}", op.as_str()), op.api_label()),
                        ));
                        header.set_xalign(0.0);
                        header.add_css_class("heading");
                        group.append(&header);
                        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                        chips.add_css_class("linked");
                        chips.set_hexpand(true);
                        let mut group_paths = Vec::new();
                        for pname in names {
                            let snap = self.ctx.snapshot.borrow();
                            let active = crate::jobs::profile_is_active(
                                &remote.name,
                                op,
                                &pname,
                                &snap.jobs,
                                &snap.mounts,
                                &snap.serves,
                            );
                            let job = snap.jobs.iter().find(|job| {
                                crate::jobs::job_belongs_to_remote(job, &remote.name)
                                    && crate::jobs::job_operation_matches(&job.operation, op)
                                    && crate::jobs::job_is_running(job)
                                    && job.profile == pname
                            });
                            let mount_point = snap
                                .mounts
                                .iter()
                                .find(|m| {
                                    m.fs.trim_end_matches(':') == remote.name
                                        || m.fs == format!("{}:", remote.name)
                                })
                                .map(|m| m.mount_point.clone());
                            if active {
                                group_paths.extend(crate::jobs::active_open_paths(
                                    op,
                                    job.map(|j| j.src.as_str()).unwrap_or(""),
                                    job.map(|j| j.dst.as_str()).unwrap_or(""),
                                    mount_point.as_deref(),
                                ));
                            }
                            drop(snap);
                            let chip = gtk::Button::with_label(&if active {
                                format!("Stop {} · {pname}", op.api_label())
                            } else {
                                format!("{} · {pname}", op.api_label())
                            });
                            chip.set_tooltip_text(Some(&if active {
                                self.ctx.t_or("remote.stopProfile", "Stop this profile")
                            } else {
                                self.ctx.t_or("remote.startProfile", "Start this profile")
                            }));
                            if active {
                                chip.add_css_class("destructive-action");
                            }
                            mark_action_busy(
                                &chip,
                                crate::jobs::action_in_progress(
                                    &remote.name,
                                    op,
                                    &pname,
                                    &self.ctx.snapshot.borrow().jobs,
                                    self.ctx.is_busy(&remote.name, op.as_str(), &pname),
                                ),
                                &self.ctx,
                            );
                            let ctx = self.ctx.clone();
                            let name = remote.name.clone();
                            let toast = self.toast.clone();
                            let dash = self.clone();
                            let pname = pname.clone();
                            let dry = self.dry_run.clone();
                            let resync = self.resync.clone();
                            chip.connect_clicked(move |_| {
                                toggle_profile(
                                    &ctx,
                                    &name,
                                    op,
                                    &pname,
                                    &toast,
                                    dry.get(),
                                    resync.get(),
                                );
                                dash.refresh();
                            });
                            chips.append(&chip);
                        }
                        group_paths.sort();
                        group_paths.dedup();
                        for path in group_paths {
                            let open = gtk::Button::from_icon_name("folder-open-symbolic");
                            open.set_tooltip_text(Some(&path));
                            let ctx = self.ctx.clone();
                            let name = remote.name.clone();
                            open.connect_clicked(move |_| {
                                open_overview_path(&ctx, &name, &path);
                            });
                            chips.append(&open);
                        }
                        group.append(&chips);
                        card.append(&group);
                        groups += 1;
                    }
                }
                if groups == 0 {
                    let empty = gtk::Label::new(Some(&self.ctx.t_or(
                        "overviews.remoteCard.emptyState.message",
                        "This remote has no operation profiles to show.",
                    )));
                    empty.add_css_class("dim-label");
                    empty.set_wrap(true);
                    card.append(&empty);
                }
                let wrap = gtk::ListBoxRow::new();
                wrap.set_activatable(false);
                wrap.set_child(Some(&card));
                list.append(&wrap);
            } else {
                list.append(&row);
            }
        }
        self.host().append(&list);
    }

    fn render_jobs_panel(&self, snap: &crate::store::RuntimeSnapshot) {
        let filter = self.origin_filter();
        let filtered: Vec<_> = snap
            .jobs
            .iter()
            .filter(|job| {
                crate::jobs::is_overview_job(job)
                    && crate::jobs::origin_matches(&job.origin, &filter)
            })
            .cloned()
            .collect();
        let dash = self.clone();
        self.host().append(&job_panels::overview_jobs_panel(
            &self.ctx,
            &filtered,
            &snap.stats,
            move || dash.refresh(),
        ));
    }

    fn render_serves_panel(&self, snap: &crate::store::RuntimeSnapshot) {
        let serves = gtk::ListBox::new();
        serves.add_css_class("boxed-list");
        let filter = self.origin_filter();
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
                let dash = self.clone();
                serves.append(&serve_card_row(&self.ctx, serve, move || dash.refresh()));
            }
        }
        self.host().append(&serves);
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
        self.host().append(&bw_group);
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
        self.host().append(&presets);
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
        self.host().append(&custom);
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
        if let Some(client) = self.ctx.client() {
            if let Ok(pid) = client.pid() {
                let row = adw::ActionRow::new();
                row.set_title(&self.ctx.t_or("dashboard.system.pid", "rclone PID"));
                row.set_subtitle(&pid.to_string());
                sys.append(&row);
            }
            if let Ok(groups) = client.group_list() {
                let count = groups
                    .get("groups")
                    .or_else(|| groups.get("list"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if count > 0 {
                    let row = adw::ActionRow::new();
                    row.set_title(&self.ctx.t_or("dashboard.system.groups", "Job groups"));
                    row.set_subtitle(&count.to_string());
                    sys.append(&row);
                }
            }
        }
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
        jobs_row.set_subtitle(
            &self.ctx.tf(
                "dashboard.system.activitySummary",
                &[
                    (
                        "jobs",
                        &snap
                            .jobs
                            .iter()
                            .filter(|j| crate::jobs::is_overview_job(j) && j.status == "running")
                            .count()
                            .to_string(),
                    ),
                    ("mounts", &snap.mounts.len().to_string()),
                    ("serves", &snap.serves.len().to_string()),
                ],
            ),
        );
        sys.append(&jobs_row);
        self.host().append(&sys);
    }

    fn render_automations_panel(&self) {
        let autos = gtk::ListBox::new();
        autos.add_css_class("boxed-list");
        let filter = self.origin_filter();
        let records: Vec<_> = crate::automation::collect(&self.ctx.store.borrow())
            .into_iter()
            .filter(|record| {
                crate::jobs::origin_matches(crate::jobs::automation_origin(&record.id), &filter)
            })
            .collect();
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
                let cron = if record.cron_enabled {
                    crate::rclone::describe_cron_i18n(&record.cron, &self.ctx.i18n.borrow())
                } else {
                    self.ctx.t_or("common.off", "off")
                };
                let watch = if record.watch_enabled {
                    let delay = self.ctx.tf(
                        "generalOverview.automations.watchEvery",
                        &[("seconds", &record.watch_delay.to_string())],
                    );
                    if record.watch_changed_only {
                        format!(
                            "{delay} · {}",
                            self.ctx
                                .t_or("generalOverview.automations.changedOnly", "changed only")
                        )
                    } else {
                        delay
                    }
                } else {
                    self.ctx.t_or("common.off", "off")
                };
                let paused_suffix = if paused {
                    format!(
                        " · {}",
                        self.ctx
                            .t_or("generalOverview.automations.paused", "paused")
                    )
                } else {
                    String::new()
                };
                row.set_subtitle(&format!(
                    "{} · {cron} · {watch} · {} {next}{paused_suffix}",
                    record.operation,
                    self.ctx
                        .t_or("generalOverview.automations.nextRun", "Next Run:")
                ));
                let enabled = gtk::Switch::new();
                enabled.set_valign(gtk::Align::Center);
                enabled.set_tooltip_text(Some(&self.ctx.t_or(
                    "generalOverview.automations.pauseResume",
                    "Pause or resume this automation",
                )));
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
                run.set_tooltip_text(Some(
                    &self
                        .ctx
                        .t_or("generalOverview.automations.runNow", "Run now"),
                ));
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
                                Ok(_) => toast.add_toast(adw::Toast::new(&ctx.tf(
                                    "notification.title.jobStarted",
                                    &[("type", record.operation.api_label())],
                                ))),
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
        self.host().append(&autos);
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
        back.set_tooltip_text(Some(&self.ctx.t_or("common.back", "Back to overview")));
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
        header.append(&self.remote_quick_action(
            &name,
            "folder-symbolic",
            "common.browse",
            "Browse",
            "browse",
        ));
        header.append(&self.remote_quick_action(
            &name,
            "dialog-information-symbolic",
            "fileBrowser.about",
            "About",
            "about",
        ));
        header.append(&self.remote_options_menu(&name));
        self.detail_box().append(&header);

        if let Some(remote) = remote.as_ref() {
            let subtitle = gtk::Label::new(Some(&format!(
                "{} · {}",
                remote.r#type,
                remote_state_label(&self.ctx, remote.mounted, remote.serving, remote.job_active)
            )));
            subtitle.add_css_class("dim-label");
            subtitle.set_xalign(0.0);
            self.detail_box().append(&subtitle);
        }

        let use_tabs = tab != AppTab::General;
        let monitoring = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let configuration = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if tab == AppTab::Operations {
            self.append_sync_op_picker(&name);
        }
        if use_tabs {
            *self.detail_host.borrow_mut() = Some(monitoring.clone());
        } else {
            self.append_status_chips(&name, remote.as_ref());
            self.append_configuration_links(&name);
        }
        let detail_op = if tab == AppTab::Operations {
            self.selected_sync_op(&name)
        } else {
            tab.default_operation()
        };
        let profile_name = self.selected_profile_name(&name, detail_op);
        self.append_automation_banners(&name, detail_op, &profile_name);

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
            let names = self
                .ctx
                .store
                .borrow()
                .remotes
                .get(&name)
                .map(|m| m.profile_names(op))
                .unwrap_or_default();
            if names.is_empty() && !crate::jobs::allows_unconfigured_start(op) {
                btn.set_sensitive(false);
                btn.set_tooltip_text(Some(&self.ctx.t_or(
                    "modals.remoteConfig.profile.noProfiles",
                    "No profiles configured",
                )));
            }
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
                        start_operation(
                            &ctx,
                            &name,
                            op,
                            &toast,
                            dash.dry_run.get(),
                            dash.resync.get(),
                        );
                        dash.refresh();
                    }
                });
            }
            chips.append(&btn);
        }
        self.detail_box().append(&chips);
        self.append_operation_controls(&name, detail_op, remote.as_ref());

        let selected_profile = (tab == AppTab::Operations)
            .then(|| self.selected_profile_name(&name, self.selected_sync_op(&name)));
        if let Some(job) = remote_jobs_preview(&snap.jobs, &name, selected_profile.as_deref()) {
            self.detail_box()
                .append(&job_panels::job_info_group(&self.ctx, job));
            self.detail_box()
                .append(&job_panels::job_stats_group(&self.ctx, job));
        } else if tab == AppTab::Operations {
            self.detail_box()
                .append(&job_panels::empty_stats_group(&self.ctx));
        }

        self.append_disk_usage(&name);
        if tab == AppTab::Mount {
            self.append_mount_disk_usage(&name, &snap);
        }

        self.detail_box().append(&section_label(
            &self.ctx.t_or("generalOverview.activity", "Activity"),
        ));
        let activity = gtk::ListBox::new();
        activity.add_css_class("boxed-list");
        let remote_jobs = {
            let store = self.ctx.store.borrow();
            crate::jobs::merge_overview_jobs(
                &snap.jobs,
                &crate::jobs::history_with_meta(&store.job_history, &store.job_meta),
                &name,
                None,
            )
        };
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
                row.set_title(&format!(
                    "{} · {}",
                    job.operation,
                    self.ctx
                        .t_or(crate::jobs::job_status_key(&job.status), &job.status)
                ));
                row.set_subtitle(&format!(
                    "{:.0}% · {} · {}",
                    if matches!(job.status.as_str(), "completed" | "failed" | "stopped")
                        && job.progress <= 0.0
                    {
                        100.0
                    } else {
                        job.progress * 100.0
                    },
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
                if crate::jobs::job_is_running(&job) || crate::jobs::job_is_pending(&job) {
                    let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
                    stop.set_valign(gtk::Align::Center);
                    stop.set_tooltip_text(Some(
                        &self.ctx.t_or("flow.quickRun.actions.stop", "Stop"),
                    ));
                    let ctx = self.ctx.clone();
                    let dash = self.clone();
                    stop.connect_clicked(move |_| {
                        if let Some(c) = ctx.client() {
                            let _ = c.job_stop(id);
                            ctx.refresh_runtime();
                            dash.refresh();
                        }
                    });
                    row.add_suffix(&stop);
                } else {
                    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
                    delete.set_valign(gtk::Align::Center);
                    delete.set_tooltip_text(Some(
                        &self
                            .ctx
                            .t_or("fileBrowser.operations.removeJob", "Remove from history"),
                    ));
                    let ctx = self.ctx.clone();
                    let dash = self.clone();
                    delete.connect_clicked(move |_| {
                        ctx.store.borrow_mut().dismiss_job(id);
                        ctx.persist();
                        dash.refresh();
                    });
                    row.add_suffix(&delete);
                }
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
            let dash = self.clone();
            activity.append(&serve_card_row(&self.ctx, &serve, move || dash.refresh()));
        }
        self.detail_box().append(&activity);
        self.append_transfer_activity(&name, &snap, selected_profile.as_deref());
        if tab == AppTab::General {
            self.append_remote_automations(&name);
        }

        if tab == AppTab::General || detail_op.supports_vfs() {
            self.detail_box().append(&vfs_panel::vfs_panel(
                self.ctx.clone(),
                &name,
                self.toast.clone(),
            ));
        }

        if use_tabs {
            *self.detail_host.borrow_mut() = Some(configuration.clone());
            self.append_configuration_links(&name);
        }

        self.detail_box().append(&section_label(
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
                    mark_action_busy(
                        &start,
                        crate::jobs::action_in_progress(
                            &name,
                            op_ty,
                            pname,
                            &self.ctx.snapshot.borrow().jobs,
                            self.ctx.is_busy(&name, op_ty.as_str(), pname),
                        ),
                        &self.ctx,
                    );
                    {
                        let ctx = self.ctx.clone();
                        let toast = self.toast.clone();
                        let remote = name.clone();
                        let pname = pname.clone();
                        let dry = self.dry_run.clone();
                        let resync = self.resync.clone();
                        start.connect_clicked(move |_| {
                            toggle_profile(
                                &ctx,
                                &remote,
                                op_ty,
                                &pname,
                                &toast,
                                dry.get(),
                                resync.get(),
                            );
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
        self.detail_box().append(&plist);
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
        self.detail_box().append(&add_profile);
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
        self.detail_box().append(&helpers);

        if !use_tabs {
            self.detail_box().append(&section_label(
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
                .cloned()
                .collect::<Vec<_>>()
            {
                let row = adw::ActionRow::new();
                row.set_title(&qr.name);
                row.set_subtitle(&format!("{} · {}", qr.operation_type, qr.status));
                let start = gtk::Button::from_icon_name("media-playback-start-symbolic");
                start.set_valign(gtk::Align::Center);
                start
                    .set_tooltip_text(Some(&self.ctx.t_or("flow.quickRun.actions.start", "Start")));
                let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
                stop.set_valign(gtk::Align::Center);
                stop.set_tooltip_text(Some(&self.ctx.t_or("flow.quickRun.actions.stop", "Stop")));
                let edit = gtk::Button::from_icon_name("document-edit-symbolic");
                edit.set_valign(gtk::Align::Center);
                edit.set_tooltip_text(Some(&self.ctx.t_or("common.edit", "Edit")));
                let open = gtk::Button::from_icon_name("folder-symbolic");
                open.set_valign(gtk::Align::Center);
                open.set_tooltip_text(Some(&self.ctx.t_or("common.browse", "Browse")));
                {
                    let ctx = self.ctx.clone();
                    let toast = self.toast.clone();
                    let dash = self.clone();
                    let qr = qr.clone();
                    start.connect_clicked(move |_| {
                        start_quick_run(&ctx, &qr, &toast);
                        dash.refresh();
                    });
                }
                {
                    let ctx = self.ctx.clone();
                    let id = qr.last_job_id;
                    let qr_id = qr.id.clone();
                    let dash = self.clone();
                    stop.connect_clicked(move |_| {
                        if let (Some(client), Some(jobid)) = (ctx.client(), id) {
                            let _ = client.job_stop(jobid);
                        }
                        if let Some(run) = ctx
                            .store
                            .borrow_mut()
                            .quick_runs
                            .iter_mut()
                            .find(|q| q.id == qr_id)
                        {
                            run.status = "stopped".into();
                        }
                        ctx.persist();
                        dash.refresh();
                    });
                }
                {
                    let ctx = self.ctx.clone();
                    let dash = self.clone();
                    let qr = qr.clone();
                    edit.connect_clicked(move |_| {
                        if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                            dialogs::quick_run_editor(&win, ctx.clone(), Some(qr.clone()), {
                                let dash = dash.clone();
                                Rc::new(move || dash.refresh())
                            });
                        }
                    });
                }
                {
                    let ctx = self.ctx.clone();
                    let qr = qr.clone();
                    open.connect_clicked(move |_| {
                        ctx.browse_quick_run(&qr);
                    });
                }
                row.add_suffix(&open);
                row.add_suffix(&edit);
                row.add_suffix(&stop);
                row.add_suffix(&start);
                qlist.append(&row);
            }
            if qlist.first_child().is_none() {
                let row = adw::ActionRow::new();
                row.set_title(&self.ctx.t_or("dashboard.quickRuns.empty", "No quick runs"));
                qlist.append(&row);
            }
            self.detail_box().append(&qlist);
            let add_qr = gtk::Button::with_label(&self.ctx.t_or(
                "dashboard.quickRuns.createForRemote",
                "Create quick run for this remote",
            ));
            {
                let ctx = self.ctx.clone();
                let remote = name.clone();
                let dash = self.clone();
                add_qr.connect_clicked(move |_| {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        let draft = crate::store::QuickRun::new(
                            String::new(),
                            OperationType::Sync,
                            remote.clone(),
                        );
                        dialogs::quick_run_editor(&win, ctx.clone(), Some(draft), {
                            let dash = dash.clone();
                            Rc::new(move || dash.refresh())
                        });
                    }
                });
            }
            self.detail_box().append(&add_qr);
        }

        self.append_settings_groups(&name, detail_op, &profile_name);
        if use_tabs {
            *self.detail_host.borrow_mut() = None;
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
            );
            self.detail_box().append(&switcher);
            self.detail_box().append(&stack);
        }
    }

    fn remote_quick_action(
        &self,
        name: &str,
        icon: &str,
        key: &str,
        fallback: &str,
        kind: &'static str,
    ) -> gtk::Button {
        let btn = gtk::Button::from_icon_name(icon);
        btn.set_tooltip_text(Some(&self.ctx.t_or(key, fallback)));
        let ctx = self.ctx.clone();
        let name = name.to_string();
        let dash = self.clone();
        btn.connect_clicked(move |_| match kind {
            "browse" => {
                *ctx.selected_remote.borrow_mut() = Some(name.clone());
                ctx.browse_remote_home(&name);
            }
            "about" => {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::remote_about(&win, ctx.clone(), &name);
                }
            }
            _ => {}
        });
        btn
    }

    fn remote_options_menu(&self, name: &str) -> gtk::MenuButton {
        let btn = gtk::MenuButton::new();
        btn.set_icon_name("view-more-symbolic");
        btn.set_tooltip_text(Some(&self.ctx.t_or("common.options", "Options")));
        let popover = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.set_margin_top(8);
        list.set_margin_bottom(8);
        list.set_margin_start(8);
        list.set_margin_end(8);
        list.add_css_class("material-context-menu");

        let tray_on = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .map(|m| m.show_on_tray)
            .unwrap_or(false);
        let tray = gtk::CheckButton::with_label(
            &self
                .ctx
                .t_or("home.options.showInTray", "Show in Tray Menu"),
        );
        tray.set_active(tray_on);
        tray.add_css_class("menu-item");
        {
            let ctx = self.ctx.clone();
            let name = name.to_string();
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
        list.append(&tray);

        for (key, fallback, kind) in [
            ("home.options.viewLogs", "View Logs", "logs"),
            ("home.options.cloneRemote", "Clone Remote", "clone"),
            (
                "home.options.exportConfig",
                "Export Configuration",
                "export",
            ),
            ("home.options.resetSettings", "Reset Settings", "reset"),
            ("home.options.deleteRemote", "Delete Remote", "delete"),
        ] {
            let item = gtk::Button::with_label(&self.ctx.t_or(key, fallback));
            item.add_css_class("flat");
            item.add_css_class("menu-item");
            item.set_halign(gtk::Align::Fill);
            if kind == "delete" {
                item.add_css_class("destructive-action");
            }
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let toast = self.toast.clone();
            let dash = self.clone();
            let popover = popover.clone();
            item.connect_clicked(move |_| {
                popover.popdown();
                match kind {
                    "logs" => {
                        if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                            dialogs::logs(&win, ctx.clone(), Some(remote.clone()));
                        }
                    }
                    "clone" => {
                        if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                            dialogs::clone_remote(&win, ctx.clone(), &remote, {
                                let dash = dash.clone();
                                let ctx = ctx.clone();
                                Rc::new(move || {
                                    ctx.refresh_runtime();
                                    dash.refresh();
                                })
                            });
                        }
                    }
                    "export" => {
                        if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                            dialogs::export_backup(&win, ctx.clone(), toast.clone(), Some(&remote));
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
                            let remote = remote.clone();
                            let dash = dash.clone();
                            dialog.connect_response(None, move |_, response| {
                                if response != "reset" {
                                    return;
                                }
                                ctx.store.borrow_mut().reset_remote_settings(&remote);
                                ctx.persist();
                                ctx.refresh_runtime();
                                dash.refresh();
                            });
                            dialog.present(Some(&win));
                        }
                    }
                    "delete" => {
                        if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                            dialogs::delete_remote(&win, ctx.clone(), &remote, {
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
                }
            });
            list.append(&item);
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_min_content_height(280);
        scroll.set_max_content_height(360);
        scroll.set_child(Some(&list));
        popover.set_child(Some(&scroll));
        btn.set_popover(Some(&popover));
        btn
    }

    fn append_settings_groups(&self, name: &str, op: OperationType, profile: &str) {
        self.detail_box().append(&section_label(&self.ctx.tf(
            "dashboard.appDetail.settingsLabel",
            &[("op", op.api_label())],
        )));
        let desc_key = format!("dashboard.appDetail.{}Desc", op.as_str());
        let desc = self.ctx.t_or(
            &desc_key,
            "Adjust how this operation behaves. Multi-profile supported.",
        );
        if !desc.is_empty() {
            let label = gtk::Label::new(Some(&desc));
            label.add_css_class("dim-label");
            label.set_xalign(0.0);
            label.set_wrap(true);
            self.detail_box().append(&label);
        }
        if let Some(cfg) = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .and_then(|meta| meta.get_profile(op, profile))
        {
            let value = serde_json::to_value(&cfg).unwrap_or(serde_json::json!({}));
            self.detail_box()
                .append(&dialogs::settings_list(&self.ctx, &value, 16));
        } else {
            let dump = self
                .ctx
                .client()
                .and_then(|client| client.dump_config().ok())
                .unwrap_or(serde_json::json!({}));
            let params =
                crate::providers::dump_remote_params(&dump, name).unwrap_or(serde_json::json!({}));
            self.detail_box()
                .append(&dialogs::settings_list(&self.ctx, &params, 16));
        }
        let edit_op = gtk::Button::with_label(&self.ctx.t_or("common.edit", "Edit"));
        {
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let dash = self.clone();
            let op_key = op.as_str().to_string();
            let profile = profile.to_string();
            edit_op.connect_clicked(move |_| {
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::remote_config_open(
                        &win,
                        ctx.clone(),
                        Some(remote.clone()),
                        super::remote_config::RemoteConfigOpen {
                            initial: Some(op_key.clone()),
                            profile: Some(profile.clone()),
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
        self.detail_box().append(&edit_op);

        self.detail_box().append(&section_label(
            &self
                .ctx
                .t_or("dashboard.appDetail.sharedSettings", "Shared Settings"),
        ));
        let shared_desc = gtk::Label::new(Some(&self.ctx.t_or(
            "dashboard.appDetail.sharedSettingsDesc",
            "Applies to all operations, regardless of sync or mount mode.",
        )));
        shared_desc.add_css_class("dim-label");
        shared_desc.set_xalign(0.0);
        shared_desc.set_wrap(true);
        self.detail_box().append(&shared_desc);

        for (kind, title_key, fallback) in [
            ("vfs", "dashboard.appDetail.vfsOptions", "VFS Options"),
            (
                "filter",
                "dashboard.appDetail.filterOptions",
                "Filter Options",
            ),
            (
                "backend",
                "dashboard.appDetail.backendConfig",
                "Backend Config",
            ),
            (
                "runtime",
                "dashboard.appDetail.runtimeRemoteOptions",
                "Runtime Remote",
            ),
        ] {
            if kind == "vfs" && !op.supports_vfs() {
                continue;
            }
            self.detail_box()
                .append(&section_label(&self.ctx.t_or(title_key, fallback)));
            let value = self
                .ctx
                .store
                .borrow()
                .remotes
                .get(name)
                .and_then(|meta| {
                    let helper = meta.helper_names(kind).into_iter().next()?;
                    meta.helper_profile(kind, &helper)
                })
                .unwrap_or(serde_json::json!({}));
            self.detail_box()
                .append(&dialogs::settings_list(&self.ctx, &value, 12));
            let edit = gtk::Button::with_label(&self.ctx.t_or("common.edit", "Edit"));
            {
                let ctx = self.ctx.clone();
                let remote = name.to_string();
                let dash = self.clone();
                let step = kind.to_string();
                edit.connect_clicked(move |_| {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        super::remote_config::present_with(
                            &win,
                            ctx.clone(),
                            remote.clone(),
                            super::remote_config::RemoteConfigOpen {
                                initial: Some(step.clone()),
                                ..Default::default()
                            },
                            {
                                let dash = dash.clone();
                                Rc::new(move || dash.refresh())
                            },
                        );
                    }
                });
            }
            self.detail_box().append(&edit);
        }

        let dump = self
            .ctx
            .client()
            .and_then(|client| client.dump_config().ok())
            .unwrap_or(serde_json::json!({}));
        if let Some(params) = crate::providers::dump_remote_params(&dump, name) {
            self.detail_box().append(&section_label(
                &self
                    .ctx
                    .t_or("dashboard.appDetail.remoteSettings", "Remote settings"),
            ));
            self.detail_box()
                .append(&dialogs::settings_list(&self.ctx, &params, 12));
        }
    }

    fn selected_profile_name(&self, name: &str, op: OperationType) -> String {
        let key = crate::jobs::selected_profile_key(name, op);
        let stored = self
            .ctx
            .settings
            .borrow()
            .runtime
            .selected_profiles
            .get(&key)
            .cloned();
        let names = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .map(|meta| meta.profile_names(op))
            .unwrap_or_default();
        if let Some(name) = stored {
            if names.iter().any(|n| n == &name) {
                return name;
            }
        }
        names.into_iter().next().unwrap_or_else(|| "default".into())
    }

    fn append_profile_picker(&self, name: &str, op: OperationType) {
        let names = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .map(|meta| meta.profile_names(op))
            .unwrap_or_default();
        if names.is_empty() {
            return;
        }
        let selected = self.selected_profile_name(name, op);
        let row = adw::ComboRow::new();
        row.set_title(&self.ctx.t_or("remote.profiles", "Profile"));
        let model_names: Vec<&str> = names.iter().map(String::as_str).collect();
        row.set_model(Some(&gtk::StringList::new(&model_names)));
        if let Some(idx) = names.iter().position(|n| n == &selected) {
            row.set_selected(idx as u32);
        }
        {
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let names = names.clone();
            let dash = self.clone();
            row.connect_selected_notify(move |combo| {
                if let Some(profile) = names.get(combo.selected() as usize) {
                    ctx.settings.borrow_mut().runtime.selected_profiles.insert(
                        crate::jobs::selected_profile_key(&remote, op),
                        profile.clone(),
                    );
                    ctx.persist();
                    dash.refresh();
                }
            });
        }
        self.detail_box().append(&row);
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
        let toggles = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toggles.add_css_class("linked");
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
            toggles.append(&btn);
        }
        let gear = gtk::Button::from_icon_name("emblem-system-symbolic");
        gear.set_tooltip_text(Some(&self.ctx.t_or(
            "dashboard.appDetail.configureActions",
            "Configure sync actions",
        )));
        {
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let dash = self.clone();
            gear.connect_clicked(move |_| {
                let current = ctx
                    .store
                    .borrow()
                    .remotes
                    .get(&remote)
                    .map(|meta| meta.sync_actions.clone())
                    .unwrap_or_default();
                let catalog = crate::action_order::sync_catalog_ids();
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::action_order(
                        &win,
                        &ctx,
                        &ctx.t_or("remoteConfig.syncActions", "Sync actions"),
                        &catalog,
                        &current,
                        Some(3),
                        {
                            let ctx = ctx.clone();
                            let remote = remote.clone();
                            let dash = dash.clone();
                            move |ids| {
                                ctx.store
                                    .borrow_mut()
                                    .remotes
                                    .entry(remote.clone())
                                    .or_default()
                                    .sync_actions = ids;
                                ctx.persist();
                                dash.refresh();
                            }
                        },
                    );
                }
            });
        }
        row.append(&toggles);
        row.append(&gear);
        self.detail_box().append(&row);
    }

    fn append_status_chips(&self, name: &str, remote: Option<&crate::store::RemoteInfo>) {
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chips.add_css_class("linked");
        let mounted = remote.map(|r| r.mounted).unwrap_or(false);
        let serving = remote.map(|r| r.serving).unwrap_or(false);
        let snap = self.ctx.snapshot.borrow().clone();
        let primary: Vec<OperationType> = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .map(|meta| {
                meta.primary_actions
                    .iter()
                    .filter_map(|s| OperationType::parse(s))
                    .take(3)
                    .collect()
            })
            .unwrap_or_default();
        let kinds = if primary.is_empty() {
            vec![
                OperationType::Mount,
                OperationType::Sync,
                OperationType::Serve,
            ]
        } else {
            primary
        };
        let start = self.ctx.t_or("actions.start", "Start");
        let stop = self.ctx.t_or("actions.stop", "Stop");
        for op in kinds {
            let active = match op {
                OperationType::Mount => mounted,
                OperationType::Serve => serving,
                other => snap.jobs.iter().any(|job| {
                    crate::jobs::job_belongs_to_remote(job, name)
                        && crate::jobs::job_operation_matches(&job.operation, other)
                        && crate::jobs::job_is_running(job)
                }),
            };
            let label = self
                .ctx
                .t_or(&format!("actions.{}", op.as_str()), op.api_label());
            let verb = if active { &stop } else { &start };
            let btn = gtk::Button::with_label(&format!("{verb} {label}"));
            if active {
                btn.add_css_class("destructive-action");
            }
            let names = self
                .ctx
                .store
                .borrow()
                .remotes
                .get(name)
                .map(|m| m.profile_names(op))
                .unwrap_or_default();
            if names.is_empty() && !crate::jobs::allows_unconfigured_start(op) {
                btn.set_sensitive(false);
                btn.set_tooltip_text(Some(&self.ctx.t_or(
                    "modals.remoteConfig.profile.noProfiles",
                    "No profiles configured",
                )));
            }
            mark_action_busy(
                &btn,
                crate::jobs::action_in_progress(
                    name,
                    op,
                    "",
                    &snap.jobs,
                    self.ctx.is_busy(name, op.as_str(), "default"),
                ),
                &self.ctx,
            );
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let toast = self.toast.clone();
            let dash = self.clone();
            btn.connect_clicked(move |_| {
                if op == OperationType::Mount {
                    toggle_mount(&ctx, &remote, mounted, &toast);
                    dash.refresh();
                    return;
                }
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::start_operation(&win, ctx.clone(), &remote, op, toast.clone(), {
                        let dash = dash.clone();
                        Rc::new(move || dash.refresh())
                    });
                }
            });
            chips.append(&btn);
        }
        let gear = gtk::Button::from_icon_name("emblem-system-symbolic");
        gear.set_tooltip_text(Some(&self.ctx.t_or(
            "dashboard.appDetail.configureActions",
            "Configure primary actions",
        )));
        {
            let ctx = self.ctx.clone();
            let remote = name.to_string();
            let dash = self.clone();
            gear.connect_clicked(move |_| {
                let current = ctx
                    .store
                    .borrow()
                    .remotes
                    .get(&remote)
                    .map(|meta| meta.primary_actions.clone())
                    .unwrap_or_default();
                let catalog = crate::action_order::catalog_ids();
                if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                    dialogs::action_order(
                        &win,
                        &ctx,
                        &ctx.t_or(
                            "dashboard.appDetail.configureActions",
                            "Configure primary actions",
                        ),
                        &catalog,
                        &current,
                        Some(3),
                        {
                            let ctx = ctx.clone();
                            let remote = remote.clone();
                            let dash = dash.clone();
                            move |ids| {
                                ctx.store
                                    .borrow_mut()
                                    .remotes
                                    .entry(remote.clone())
                                    .or_default()
                                    .primary_actions = ids;
                                ctx.persist();
                                dash.refresh();
                            }
                        },
                    );
                }
            });
        }
        chips.append(&gear);
        self.detail_box().append(&chips);
    }

    fn append_automation_banners(&self, name: &str, op: OperationType, profile: &str) {
        let Some(cfg) = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .and_then(|meta| meta.get_profile(op, profile))
        else {
            return;
        };
        if cfg.app.cron_enabled && !cfg.app.cron_expression.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or("flow.quickRun.badges.scheduled", "Scheduled"));
            row.set_subtitle(&crate::rclone::describe_cron_i18n(
                &cfg.app.cron_expression,
                &self.ctx.i18n.borrow(),
            ));
            self.detail_box().append(&row);
        }
        if cfg.app.watch_enabled {
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or(
                "automation.monitoring.realtimeSchedule",
                "Real-time File Watcher",
            ));
            let mut subtitle = format!(
                "{}: {} {}",
                self.ctx
                    .t_or("automation.monitoring.debounce", "Debounce Delay"),
                cfg.app.watch_delay,
                self.ctx.t_or("automation.monitoring.seconds", "seconds")
            );
            if cfg.app.watch_changed_only {
                subtitle.push_str(" · ");
                subtitle.push_str(&self.ctx.t_or(
                    "automation.monitoring.changedOnlyShort",
                    "Changed files only",
                ));
            }
            row.set_subtitle(&subtitle);
            self.detail_box().append(&row);
        }
    }

    fn append_operation_controls(
        &self,
        name: &str,
        op: OperationType,
        remote: Option<&crate::store::RemoteInfo>,
    ) {
        let names = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .map(|meta| meta.profile_names(op))
            .unwrap_or_default();
        if names.len() > 1 {
            self.append_profile_picker(name, op);
            self.append_operation_control(name, op, &self.selected_profile_name(name, op), remote);
            return;
        }
        self.append_operation_control(
            name,
            op,
            names.first().map(String::as_str).unwrap_or("default"),
            remote,
        );
    }

    fn append_operation_control(
        &self,
        name: &str,
        op: OperationType,
        profile: &str,
        remote: Option<&crate::store::RemoteInfo>,
    ) {
        let snap = self.ctx.snapshot.borrow().clone();
        let active = crate::jobs::profile_is_active(
            name,
            op,
            profile,
            &snap.jobs,
            &snap.mounts,
            &snap.serves,
        );
        let cfg = self
            .ctx
            .store
            .borrow()
            .remotes
            .get(name)
            .and_then(|meta| meta.get_profile(op, profile));
        let dry_on = cfg
            .as_ref()
            .map(|cfg| crate::jobs::is_dry_run(&cfg.rclone))
            .unwrap_or(false);
        let resync_on = cfg
            .as_ref()
            .map(|cfg| crate::jobs::is_resync(&cfg.rclone))
            .unwrap_or(false);
        if profile == self.selected_profile_name(name, op) {
            self.dry_run.set(dry_on);
            self.resync.set(resync_on);
        }
        let row = adw::ExpanderRow::new();
        row.set_title(if profile.is_empty() {
            "default"
        } else {
            profile
        });
        row.set_subtitle(op.api_label());
        if dry_on {
            row.set_subtitle(&format!(
                "{} · {}",
                op.api_label(),
                self.ctx.t_or("dashboard.appDetail.dryRun", "Dry run")
            ));
        }
        let start = gtk::Button::with_label(&self.ctx.t_or("actions.start", "Start"));
        start.add_css_class("suggested-action");
        start.set_valign(gtk::Align::Center);
        let stop = gtk::Button::with_label(&self.ctx.t_or("actions.stop", "Stop"));
        stop.add_css_class("destructive-action");
        stop.set_valign(gtk::Align::Center);
        start.set_sensitive(!active);
        stop.set_sensitive(active);
        mark_action_busy(
            &start,
            crate::jobs::action_in_progress(
                name,
                op,
                profile,
                &snap.jobs,
                self.ctx.is_busy(name, op.as_str(), profile),
            ),
            &self.ctx,
        );
        {
            let ctx = self.ctx.clone();
            let toast = self.toast.clone();
            let remote = name.to_string();
            let profile = profile.to_string();
            let dry = self.dry_run.clone();
            let resync = self.resync.clone();
            let dash = self.clone();
            start.connect_clicked(move |_| {
                toggle_profile(&ctx, &remote, op, &profile, &toast, dry.get(), resync.get());
                dash.refresh();
            });
        }
        {
            let ctx = self.ctx.clone();
            let toast = self.toast.clone();
            let remote = name.to_string();
            let profile = profile.to_string();
            let dash = self.clone();
            stop.connect_clicked(move |_| {
                toggle_profile(&ctx, &remote, op, &profile, &toast, false, false);
                dash.refresh();
            });
        }
        row.add_suffix(&start);
        row.add_suffix(&stop);

        for path_row in self.operation_path_rows(name, op, profile, remote) {
            row.add_row(&path_row);
        }
        if op == OperationType::Mount {
            if let Some(usage) = self.mount_usage_row(name, &snap) {
                row.add_row(&usage);
            }
        }
        if matches!(
            op,
            OperationType::Sync
                | OperationType::Copy
                | OperationType::Move
                | OperationType::Bisync
                | OperationType::Check
        ) {
            let dry = adw::SwitchRow::new();
            dry.set_title(&self.ctx.t_or("dashboard.appDetail.dryRun", "Dry run"));
            if dry_on {
                dry.set_subtitle(&self.ctx.t_or(
                    "dashboard.appDetail.dryRunActive",
                    "Start the next operation without writing changes",
                ));
            }
            dry.set_active(dry_on);
            dry.set_sensitive(!active);
            {
                let flag = self.dry_run.clone();
                let ctx = self.ctx.clone();
                let remote = name.to_string();
                let profile = profile.to_string();
                dry.connect_active_notify(move |row| {
                    flag.set(row.is_active());
                    persist_profile_flag(&ctx, &remote, op, &profile, Some(row.is_active()), None);
                });
            }
            row.add_row(&dry);
            if op == OperationType::Bisync {
                let resync = adw::SwitchRow::new();
                resync.set_title(&self.ctx.t_or("dashboard.appDetail.resync", "Resync"));
                if resync_on {
                    resync.set_subtitle(&self.ctx.t_or(
                        "dashboard.appDetail.resyncActive",
                        "Force a bisync resync on the next start",
                    ));
                }
                resync.set_active(resync_on);
                resync.set_sensitive(!active);
                {
                    let flag = self.resync.clone();
                    let ctx = self.ctx.clone();
                    let remote = name.to_string();
                    let profile = profile.to_string();
                    resync.connect_active_notify(move |row| {
                        flag.set(row.is_active());
                        persist_profile_flag(
                            &ctx,
                            &remote,
                            op,
                            &profile,
                            None,
                            Some(row.is_active()),
                        );
                    });
                }
                row.add_row(&resync);
            }
        }
        let full = gtk::Button::with_label(&if active {
            self.ctx
                .tf("dashboard.appDetail.stop", &[("op", op.api_label())])
        } else {
            self.ctx
                .tf("dashboard.appDetail.start", &[("op", op.api_label())])
        });
        if active {
            full.add_css_class("destructive-action");
        } else {
            full.add_css_class("suggested-action");
        }
        full.set_hexpand(true);
        {
            let ctx = self.ctx.clone();
            let toast = self.toast.clone();
            let remote = name.to_string();
            let profile = profile.to_string();
            let dry = self.dry_run.clone();
            let resync = self.resync.clone();
            let dash = self.clone();
            full.connect_clicked(move |_| {
                toggle_profile(&ctx, &remote, op, &profile, &toast, dry.get(), resync.get());
                dash.refresh();
            });
        }
        let action_row = gtk::ListBoxRow::new();
        action_row.set_activatable(false);
        let pad = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pad.set_margin_top(8);
        pad.set_margin_bottom(8);
        pad.set_margin_start(8);
        pad.set_margin_end(8);
        pad.append(&full);
        action_row.set_child(Some(&pad));
        row.add_row(&action_row);
        self.detail_box().append(&row);
    }

    fn operation_path_rows(
        &self,
        name: &str,
        op: OperationType,
        profile: &str,
        remote: Option<&crate::store::RemoteInfo>,
    ) -> Vec<adw::ActionRow> {
        let snap = self.ctx.snapshot.borrow().clone();
        let live = remote
            .and_then(|remote| crate::jobs::find_active_job(&snap.jobs, &remote.name, op, profile));
        let (src, dst) = if let Some(job) = live {
            (
                (!job.src.is_empty()).then(|| job.src.clone()),
                (!job.dst.is_empty()).then(|| job.dst.clone()),
            )
        } else {
            self.ctx
                .store
                .borrow()
                .remotes
                .get(name)
                .and_then(|meta| meta.get_profile(op, profile))
                .map(|cfg| crate::store::quick_run_paths(&cfg.rclone, op))
                .unwrap_or((None, None))
        };
        let mut rows = Vec::new();
        for (title_key, fallback, path) in [
            (
                "fileBrowser.operations.details.source",
                "Source",
                src.unwrap_or_default(),
            ),
            (
                "fileBrowser.operations.details.destination",
                "Destination",
                dst.unwrap_or_default(),
            ),
        ] {
            if path.is_empty() {
                continue;
            }
            let row = adw::ActionRow::new();
            row.set_title(&self.ctx.t_or(title_key, fallback));
            row.set_subtitle(&path);
            if crate::transfers::browse_for(&path).is_some() {
                let open = gtk::Button::from_icon_name("folder-open-symbolic");
                open.set_tooltip_text(Some(&self.ctx.t_or("common.browse", "Open in Files")));
                open.set_valign(gtk::Align::Center);
                let ctx = self.ctx.clone();
                let current = name.to_string();
                let raw = path.clone();
                open.connect_clicked(move |_| {
                    ctx.open_typed_path(&current, &raw);
                });
                row.add_suffix(&open);
            }
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.set_tooltip_text(Some(&self.ctx.t_or("common.copy", "Copy path")));
            copy.set_valign(gtk::Align::Center);
            let copy_text = path.clone();
            copy.connect_clicked(move |_| {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&copy_text);
                }
            });
            row.add_suffix(&copy);
            rows.push(row);
        }
        rows
    }

    fn mount_usage_row(
        &self,
        name: &str,
        snap: &crate::store::RuntimeSnapshot,
    ) -> Option<adw::ActionRow> {
        let mount = snap
            .mounts
            .iter()
            .find(|item| item.fs.contains(name) || item.fs.starts_with(&format!("{name}:")))?;
        let client = self.ctx.client()?;
        let usage = client.du(Some(&mount.mount_point)).ok()?;
        let row = adw::ActionRow::new();
        row.set_title(
            &self
                .ctx
                .t_or("dashboard.appDetail.mountDiskUsage", "Mount point usage"),
        );
        row.set_subtitle(&format!(
            "{} · {} used / {} free · {}",
            mount.mount_point,
            crate::rclone::format_bytes(usage.used),
            crate::rclone::format_bytes(usage.free),
            crate::rclone::format_bytes(usage.total)
        ));
        Some(row)
    }

    fn append_configuration_links(&self, name: &str) {
        self.detail_box().append(&section_label(
            &self
                .ctx
                .t_or("dashboard.appDetail.configuration", "Configuration"),
        ));
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
                let ctx = self.ctx.clone();
                let name = name.to_string();
                let dash = self.clone();
                let step = step.to_string();
                btn.connect_clicked(move |_| {
                    if let Some(win) = dash.root.root().and_downcast::<gtk::Window>() {
                        super::remote_config::present_with(
                            &win,
                            ctx.clone(),
                            name.clone(),
                            super::remote_config::RemoteConfigOpen {
                                initial: Some(step.clone()),
                                ..Default::default()
                            },
                            {
                                let dash = dash.clone();
                                Rc::new(move || dash.refresh())
                            },
                        );
                    }
                });
            }
            row.append(&btn);
        }
        self.detail_box().append(&row);
    }

    fn append_mount_disk_usage(&self, name: &str, snap: &crate::store::RuntimeSnapshot) {
        let Some(mount) = snap
            .mounts
            .iter()
            .find(|item| item.fs.contains(name) || item.fs.starts_with(&format!("{name}:")))
        else {
            return;
        };
        let Some(client) = self.ctx.client() else {
            return;
        };
        let Ok(usage) = client.du(Some(&mount.mount_point)) else {
            return;
        };
        let row = adw::ActionRow::new();
        row.set_title(
            &self
                .ctx
                .t_or("dashboard.appDetail.mountDiskUsage", "Mount point usage"),
        );
        row.set_subtitle(&format!(
            "{} · {} used / {} free · {}",
            mount.mount_point,
            crate::rclone::format_bytes(usage.used),
            crate::rclone::format_bytes(usage.free),
            crate::rclone::format_bytes(usage.total)
        ));
        self.detail_box().append(&row);
        if usage.total > 0 {
            let bar = gtk::LevelBar::new();
            bar.set_min_value(0.0);
            bar.set_max_value(1.0);
            bar.set_value(usage.used as f64 / usage.total as f64);
            bar.set_hexpand(true);
            self.detail_box().append(&bar);
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
            usage.set_subtitle(&self.ctx.t_or(
                "notification.title.engineConnectionFailed",
                "Engine Connection Error",
            ));
            box_.append(&usage);
        }
        self.detail_box().append(&box_);
    }

    fn append_transfer_activity(
        &self,
        name: &str,
        snap: &crate::store::RuntimeSnapshot,
        profile: Option<&str>,
    ) {
        let jobs = {
            let store = self.ctx.store.borrow();
            crate::jobs::merge_overview_jobs(
                &snap.jobs,
                &crate::jobs::history_with_meta(&store.job_history, &store.job_meta),
                name,
                profile,
            )
        };
        let mut rows = Vec::new();
        for job in &jobs {
            if let Some(arr) = job.transferring.as_array() {
                for item in arr {
                    rows.push((
                        job.operation.clone(),
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
                        crate::transfers::parse_completed_transfer_row(item),
                        true,
                    ));
                }
            }
        }
        rows.truncate(12);
        let mut check_items = Vec::new();
        for job in &jobs {
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
        self.detail_box().append(&section_label(&title));
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(
            &self.ctx.t_or("shared.search.toggle", "Search transfers"),
        ));
        search.set_text(&self.transfer_query.borrow());
        {
            let dash = self.clone();
            search.connect_search_changed(move |entry| {
                let text = entry.text().to_string();
                if *dash.transfer_query.borrow() == text {
                    return;
                }
                *dash.transfer_query.borrow_mut() = text;
                dash.refresh();
            });
        }
        self.detail_box().append(&search);
        let active_count = rows.iter().filter(|(_, _, completed)| !*completed).count();
        let done_count = rows.iter().filter(|(_, _, completed)| *completed).count();
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
                    let dash = self.clone();
                    let id = id.to_string();
                    btn.connect_clicked(move |_| {
                        *dash.transfer_tab.borrow_mut() = id.clone();
                        dash.refresh();
                    });
                }
                tabs.append(&btn);
            }
            self.detail_box().append(&tabs);
        }
        if let Some(job) = jobs.first() {
            let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let running = job.status == "running" || job.status == "starting";
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
                    let dash = self.clone();
                    reset.connect_clicked(move |_| {
                        if let Some(client) = ctx.client() {
                            let _ = client.reset_stats(Some(&group));
                            ctx.refresh_runtime();
                            dash.refresh();
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
                    let dash = self.clone();
                    delete.connect_clicked(move |_| {
                        ctx.store.borrow_mut().dismiss_job(id);
                        ctx.persist();
                        ctx.refresh_runtime();
                        dash.refresh();
                    });
                }
                toolbar.append(&delete);
            }
            self.detail_box().append(&toolbar);
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
                self.detail_box().append(&list);
            }
        }
        if rows.is_empty() {
            return;
        }
        let query = self.transfer_query.borrow().to_ascii_lowercase();
        let tab = self.transfer_tab.borrow().clone();
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        for (operation, row, completed) in rows {
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
                &self.toast,
            ));
        }
        self.detail_box().append(&list);
    }

    fn append_remote_automations(&self, name: &str) {
        let records: Vec<_> = crate::automation::collect(&self.ctx.store.borrow())
            .into_iter()
            .filter(|record| record.remote == name)
            .collect();
        if records.is_empty() {
            return;
        }
        self.detail_box().append(&section_label(
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
            let schedule = if record.cron_enabled {
                crate::rclone::describe_cron_i18n(&record.cron, &self.ctx.i18n.borrow())
            } else if record.watch_enabled {
                let delay = self.ctx.tf(
                    "generalOverview.automations.watchEvery",
                    &[("seconds", &record.watch_delay.to_string())],
                );
                if record.watch_changed_only {
                    format!(
                        "{delay} · {}",
                        self.ctx
                            .t_or("generalOverview.automations.changedOnly", "changed only")
                    )
                } else {
                    delay
                }
            } else {
                self.ctx.t_or("common.off", "off")
            };
            row.set_subtitle(&format!(
                "{} · {schedule}{}",
                record.operation,
                if paused {
                    format!(
                        " · {}",
                        self.ctx
                            .t_or("generalOverview.automations.paused", "paused")
                    )
                } else {
                    String::new()
                }
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
        self.detail_box().append(&list);
    }
}

fn append_open_folder_suffix(row: &adw::ActionRow, ctx: &AppCtx, remote: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    if paths.len() == 1 {
        let folder = gtk::Button::from_icon_name("folder-open-symbolic");
        folder.set_valign(gtk::Align::Center);
        folder.set_tooltip_text(Some(&paths[0]));
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = paths[0].clone();
        folder.connect_clicked(move |_| open_overview_path(&ctx, &remote, &path));
        row.add_suffix(&folder);
        return;
    }
    let btn = gtk::MenuButton::new();
    btn.set_icon_name("folder-open-symbolic");
    btn.set_valign(gtk::Align::Center);
    btn.set_tooltip_text(Some(
        &ctx.t_or("overviews.remoteCard.browse", "Browse active folders"),
    ));
    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    list.set_margin_start(6);
    list.set_margin_end(6);
    for path in paths {
        let item = gtk::Button::with_label(path);
        item.set_hexpand(true);
        let ctx = ctx.clone();
        let remote = remote.to_string();
        let path = path.clone();
        let popover = popover.clone();
        item.connect_clicked(move |_| {
            open_overview_path(&ctx, &remote, &path);
            popover.popdown();
        });
        list.append(&item);
    }
    popover.set_child(Some(&list));
    btn.set_popover(Some(&popover));
    row.add_suffix(&btn);
}

fn open_overview_path(ctx: &AppCtx, current_remote: &str, raw: &str) {
    ctx.open_typed_path(current_remote, raw);
}

fn mark_action_busy(btn: &gtk::Button, busy: bool, ctx: &AppCtx) {
    btn.set_sensitive(!busy);
    if busy {
        let spinner = gtk::Spinner::new();
        spinner.set_spinning(true);
        btn.set_child(Some(&spinner));
        btn.set_tooltip_text(Some(
            &ctx.t_or("remote.actionInProgress", "Action already in progress"),
        ));
    }
}

fn toggle_mount(ctx: &AppCtx, name: &str, mounted: bool, toast: &adw::ToastOverlay) {
    let Some(_guard) = ctx.busy_guard(name, OperationType::Mount.as_str(), "default") else {
        toast.add_toast(adw::Toast::new(
            &ctx.t_or("remote.actionInProgress", "Action already in progress"),
        ));
        return;
    };
    let Some(client) = ctx.client() else {
        toast.add_toast(adw::Toast::new(&ctx.t_or(
            "notification.title.engineConnectionFailed",
            "Engine Connection Error",
        )));
        return;
    };
    let snap = ctx.snapshot.borrow().clone();
    let meta = ctx.store.borrow().remotes.get(name).cloned();
    let profile = crate::jobs::preferred_mount_profile(meta.as_ref());
    let pname = profile
        .as_ref()
        .map(|p| p.name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".into());
    if mounted {
        match crate::jobs::stop_profile(
            &client,
            name,
            OperationType::Mount,
            &pname,
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
    if let Some(profile) = profile {
        match crate::jobs::start_profile(
            &client,
            name,
            OperationType::Mount,
            &profile,
            meta.as_ref(),
            "dashboard",
        ) {
            Ok(id) => {
                ctx.stamp_mount(&remote_fs(name, ""), &id, &pname, "dashboard");
                crate::jobs::remember_started(
                    &mut ctx.store.borrow_mut().job_meta,
                    &id,
                    crate::jobs::job_meta_for(name, &profile, "dashboard", &ctx.backend_key(), ""),
                );
                ctx.store.borrow_mut().log_operation(
                    name,
                    "mount",
                    &format!("started mount {id}"),
                    Some(&crate::restrict::redact_value(&profile.rclone)),
                );
                toast.add_toast(adw::Toast::new(&ctx.tf(
                    "mount.successMount",
                    &[("remote", name), ("profile", pname.as_str())],
                )));
            }
            Err(e) => toast.add_toast(adw::Toast::new(&e)),
        }
        ctx.refresh_runtime();
        return;
    }
    let mount_point = default_mount_point(name);
    let _ = std::fs::create_dir_all(&mount_point);
    match client.mount(&remote_fs(name, ""), &mount_point, "mount") {
        Ok(_) => {
            ctx.stamp_mount(&remote_fs(name, ""), &mount_point, &pname, "dashboard");
            toast.add_toast(adw::Toast::new(&ctx.tf(
                "notification.body.mountSucceeded",
                &[
                    ("remote", name),
                    ("profile", pname.as_str()),
                    ("backend", "local"),
                    ("mountPoint", &mount_point),
                ],
            )));
        }
        Err(e) => toast.add_toast(adw::Toast::new(&e.to_string())),
    }
    ctx.refresh_runtime();
}

fn toggle_profile(
    ctx: &AppCtx,
    name: &str,
    op: OperationType,
    profile_name: &str,
    toast: &adw::ToastOverlay,
    dry_run: bool,
    resync: bool,
) {
    let Some(_guard) = ctx.busy_guard(name, op.as_str(), profile_name) else {
        toast.add_toast(adw::Toast::new(
            &ctx.t_or("remote.actionInProgress", "Action already in progress"),
        ));
        return;
    };
    let Some(client) = ctx.client() else {
        toast.add_toast(adw::Toast::new(&ctx.t_or(
            "notification.title.engineConnectionFailed",
            "Engine Connection Error",
        )));
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
    let mut profile = meta
        .as_ref()
        .and_then(|m| m.get_profile(op, profile_name))
        .unwrap_or_default();
    crate::jobs::apply_session_flags(&mut profile.rclone, dry_run, resync);
    match crate::jobs::start_profile(&client, name, op, &profile, meta.as_ref(), "dashboard") {
        Ok(id) => {
            let rclone = crate::jobs::flatten_rclone(&profile.rclone);
            ctx.record_started_job(
                &id,
                name,
                &profile,
                "dashboard",
                op.as_str(),
                &crate::jobs::default_source(name, &rclone),
                &crate::jobs::default_dest(name, &rclone, op),
                "",
            );
            ctx.store.borrow_mut().log_operation(
                name,
                op.as_str(),
                &format!("started {op} {id}"),
                Some(&crate::restrict::redact_value(&profile.rclone)),
            );
            toast.add_toast(adw::Toast::new(&ctx.tf(
                "operations.successStart",
                &[
                    ("type", op.api_label()),
                    ("remote", name),
                    ("profile", profile_name),
                ],
            )));
        }
        Err(e) => toast.add_toast(adw::Toast::new(&e)),
    }
    ctx.refresh_runtime();
}

fn persist_profile_flag(
    ctx: &AppCtx,
    remote: &str,
    op: OperationType,
    profile: &str,
    dry_run: Option<bool>,
    resync: Option<bool>,
) {
    let mut store = ctx.store.borrow_mut();
    let Some(meta) = store.remotes.get_mut(remote) else {
        return;
    };
    let Some(mut cfg) = meta.get_profile(op, profile) else {
        return;
    };
    if !cfg.rclone.is_object() {
        cfg.rclone = serde_json::json!({});
    }
    if let Some(obj) = cfg.rclone.as_object_mut() {
        if let Some(on) = dry_run {
            if on {
                obj.insert("DryRun".into(), serde_json::json!(true));
            } else {
                obj.remove("DryRun");
                obj.remove("dryRun");
            }
        }
        if let Some(on) = resync {
            if on {
                obj.insert("Resync".into(), serde_json::json!(true));
            } else {
                obj.remove("Resync");
                obj.remove("resync");
            }
        }
    }
    meta.upsert_profile(op, cfg);
    drop(store);
    ctx.persist();
}

fn start_quick_run(ctx: &AppCtx, qr: &crate::store::QuickRun, toast: &adw::ToastOverlay) {
    let Some(_guard) = ctx.busy_guard(&qr.remote_name, qr.operation_type.as_str(), &qr.id) else {
        toast.add_toast(adw::Toast::new(
            &ctx.t_or("remote.actionInProgress", "Action already in progress"),
        ));
        return;
    };
    let Some(client) = ctx.client() else {
        toast.add_toast(adw::Toast::new(&ctx.t_or(
            "notification.title.engineConnectionFailed",
            "Engine Connection Error",
        )));
        return;
    };
    let meta = ctx.store.borrow().remotes.get(&qr.remote_name).cloned();
    match crate::jobs::start_profile(
        &client,
        &qr.remote_name,
        qr.operation_type,
        &qr.config,
        meta.as_ref(),
        "quickrun",
    ) {
        Ok(id) => {
            let rclone = crate::jobs::flatten_rclone(&qr.config.rclone);
            ctx.record_started_job(
                &id,
                &qr.remote_name,
                &qr.config,
                "quickrun",
                qr.operation_type.as_str(),
                &crate::jobs::default_source(&qr.remote_name, &rclone),
                &crate::jobs::default_dest(&qr.remote_name, &rclone, qr.operation_type),
                &qr.id,
            );
            ctx.store.borrow_mut().log_operation(
                &qr.remote_name,
                qr.operation_type.as_str(),
                &format!("started quick run {id}"),
                Some(&crate::restrict::redact_value(&qr.config.rclone)),
            );
            if let Some(run) = ctx
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
            ctx.persist();
            toast.add_toast(adw::Toast::new(&ctx.tf(
                "operations.successStart",
                &[
                    ("type", qr.operation_type.api_label()),
                    ("remote", &qr.remote_name),
                    ("profile", &qr.id),
                ],
            )));
        }
        Err(e) => toast.add_toast(adw::Toast::new(&ctx.translate_error(&e))),
    }
    ctx.refresh_runtime();
}

fn start_operation(
    ctx: &AppCtx,
    name: &str,
    op: OperationType,
    toast: &adw::ToastOverlay,
    dry_run: bool,
    resync: bool,
) {
    let names = ctx
        .store
        .borrow()
        .remotes
        .get(name)
        .map(|m| m.profile_names(op))
        .unwrap_or_default();
    if names.is_empty() && !crate::jobs::allows_unconfigured_start(op) {
        toast.add_toast(adw::Toast::new(&ctx.t_or(
            "modals.remoteConfig.profile.noProfiles",
            "No profiles configured",
        )));
        return;
    }
    let profile = names.into_iter().next().unwrap_or_else(|| "default".into());
    toggle_profile(ctx, name, op, &profile, toast, dry_run, resync);
}

fn remote_jobs_preview<'a>(
    jobs: &'a [crate::store::JobInfo],
    name: &str,
    profile: Option<&str>,
) -> Option<&'a crate::store::JobInfo> {
    jobs.iter()
        .filter(|j| j.remote == name)
        .filter(|j| {
            profile.is_none_or(|wanted| {
                j.profile == wanted || j.profile.is_empty() || j.profile == "default"
            })
        })
        .max_by_key(|j| j.id)
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

fn remote_state_label(ctx: &AppCtx, mounted: bool, serving: bool, job: bool) -> String {
    let mut parts = Vec::new();
    if mounted {
        parts.push(ctx.t_or("overviews.status.labels.mounted", "Mounted"));
    }
    if serving {
        parts.push(ctx.t_or("serve.serving", "Serving"));
    }
    if job {
        parts.push(ctx.t_or("automation.status.running", "Running"));
    }
    if parts.is_empty() {
        ctx.t_or("overviews.status.labels.inactive", "Inactive")
    } else {
        parts.join(" · ")
    }
}

pub(super) fn serve_card_row(
    ctx: &AppCtx,
    serve: &crate::rclone::ServeItem,
    on_changed: impl Fn() + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&format!("{} · {}", serve.serve_type, serve.fs));
    let mut subtitle = serve.addr.clone();
    if !serve.profile.is_empty() {
        subtitle.push_str(&format!(" · {}", serve.profile));
    }
    if !serve.origin.is_empty() {
        subtitle.push_str(&format!(" · {}", serve.origin));
    }
    if serve.option_count > 0 {
        subtitle.push_str(&format!(
            " · {} {}",
            serve.option_count,
            ctx.t_or("shared.serveCard.labels.active", "active")
        ));
    }
    row.set_subtitle(&subtitle);
    let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
    stop.set_valign(gtk::Align::Center);
    stop.set_tooltip_text(Some(
        &ctx.t_or("shared.serveCard.tooltips.stop", "Stop this serve"),
    ));
    {
        let ctx = ctx.clone();
        let id = serve.id.clone();
        stop.connect_clicked(move |_| {
            if let Some(client) = ctx.client() {
                let _ = client.serve_stop(&id);
                ctx.refresh_runtime();
                on_changed();
            }
        });
    }
    let url = serve.url();
    if !url.is_empty() {
        let copy_url = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_url.set_valign(gtk::Align::Center);
        copy_url.set_tooltip_text(Some(
            &ctx.t_or("shared.serveCard.tooltips.copyUrl", "Click to copy URL"),
        ));
        {
            let url = url.clone();
            copy_url.connect_clicked(move |_| {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&url);
                }
            });
        }
        row.add_suffix(&copy_url);
        let open = gtk::LinkButton::new(&url);
        open.set_label(&ctx.t_or("common.open", "Open"));
        open.set_valign(gtk::Align::Center);
        row.add_suffix(&open);
    }
    if !serve.id.is_empty() {
        let copy_id = gtk::Button::from_icon_name("edit-select-all-symbolic");
        copy_id.set_valign(gtk::Align::Center);
        copy_id.set_tooltip_text(Some(
            &ctx.t_or("shared.serveCard.tooltips.copyId", "Click to copy ID"),
        ));
        {
            let id = serve.id.clone();
            copy_id.connect_clicked(move |_| {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&id);
                }
            });
        }
        row.add_suffix(&copy_id);
    }
    row.add_suffix(&stop);
    {
        let ctx = ctx.clone();
        let id = serve.id.clone();
        row.connect_activated(move |_| {
            ctx.request_nav(NavTarget::Serve { id: id.clone() });
        });
    }
    row
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
