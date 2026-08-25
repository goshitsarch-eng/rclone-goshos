use super::dialogs;
use super::AppCtx;
use crate::operations::FileTypeCategory;
use crate::rclone::{
    format_bytes, format_relative_mod_time, join_remote_path, parent_remote_path, remote_fs,
    split_remote_path, DirEntry,
};
use crate::store::{apply_sort_option, sort_entries, sort_option_key, Bookmark};
use adw::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SideKind {
    Star,
    Bookmark,
    Local,
    Remote,
}

#[derive(Clone, PartialEq, Eq)]
enum InternalDrop {
    CurrentPane,
    SecondaryPane,
    Location { location: String, secondary: bool },
    Starred,
    Tab(u32),
}

#[derive(Clone)]
struct TabState {
    id: u32,
    title: String,
    remote: String,
    path: String,
    starred: bool,
}

#[derive(Clone)]
pub struct NautilusView {
    pub root: gtk::Box,
    ctx: AppCtx,
    toast: adw::ToastOverlay,
    sidebar: gtk::ListBox,
    list: gtk::ListBox,
    list_right: gtk::ListBox,
    grid: gtk::FlowBox,
    grid_right: gtk::FlowBox,
    left_stack: gtk::Stack,
    right_stack: gtk::Stack,
    layout_btn: gtk::Button,
    path_entry: gtk::Entry,
    path_stack: gtk::Stack,
    crumbs: gtk::Box,
    search_entry: gtk::SearchEntry,
    search_filter: Rc<RefCell<String>>,
    status: gtk::Label,
    tabs: Rc<RefCell<Vec<TabState>>>,
    current: Rc<RefCell<TabState>>,
    secondary: Rc<RefCell<TabState>>,
    history: Rc<RefCell<Vec<(String, String)>>>,
    future: Rc<RefCell<Vec<(String, String)>>>,
    clipboard: Rc<RefCell<Vec<(String, String, bool)>>>,
    pending_drag: Rc<RefCell<Option<Vec<crate::dnd::DragItem>>>>,
    skip_lasso: Rc<Cell<bool>>,
    hover_open: Rc<RefCell<Option<String>>>,
    undo: Rc<RefCell<Vec<String>>>,
    redo: Rc<RefCell<Vec<String>>>,
    tab_bar: gtk::Box,
    next_tab_id: Rc<RefCell<u32>>,
    split_enabled: Rc<RefCell<bool>>,
    paned: gtk::Paned,
    right_scroll: gtk::ScrolledWindow,
    ops: gtk::ListBox,
    last_listing: Rc<RefCell<Vec<DirEntry>>>,
    picker_bar: gtk::Box,
    picker_label: gtk::Label,
    share_bar: gtk::Box,
    share_label: gtk::Label,
    filter_bar: gtk::Box,
    icon_btn: gtk::Button,
}

impl NautilusView {
    pub fn new(ctx: AppCtx, toast: adw::ToastOverlay) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_margin_start(8);
        toolbar.set_margin_end(8);

        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.set_tooltip_text(Some(&ctx.t_or("common.back", "Back")));
        let forward = gtk::Button::from_icon_name("go-next-symbolic");
        forward.set_tooltip_text(Some(&ctx.t_or("common.continue", "Forward")));
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.goUp", "Parent folder"),
        ));
        let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
        reload.set_tooltip_text(Some(&ctx.t_or("common.refresh", "Reload")));
        let path_entry = gtk::Entry::new();
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some(&ctx.t_or(
            "nautilus.titles.pathPlaceholder",
            "remote:path or /local/path",
        )));
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        crumbs.add_css_class("linked");
        let crumbs_scroll = gtk::ScrolledWindow::new();
        crumbs_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        crumbs_scroll.set_child(Some(&crumbs));
        crumbs_scroll.set_hexpand(true);
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some(
            &ctx.t_or("nautilus.titles.searchPlaceholder", "Search files..."),
        ));
        let path_stack = gtk::Stack::new();
        path_stack.set_hexpand(true);
        path_stack.add_named(&crumbs_scroll, Some("crumbs"));
        path_stack.add_named(&path_entry, Some("entry"));
        path_stack.add_named(&search_entry, Some("search"));
        path_stack.set_visible_child_name("crumbs");
        let new_folder = gtk::Button::from_icon_name("folder-new-symbolic");
        new_folder.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.newFolder", "New folder"),
        ));
        let upload = gtk::Button::from_icon_name("document-send-symbolic");
        upload.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.uploadFiles", "Upload files"),
        ));
        let layout = gtk::Button::from_icon_name("view-list-symbolic");
        layout.set_tooltip_text(Some(
            &ctx.t_or("nautilus.view.toggleLayout", "Toggle list / grid"),
        ));
        let hidden_btn = gtk::Button::from_icon_name("view-conceal-symbolic");
        hidden_btn.set_tooltip_text(Some(
            &ctx.t_or("nautilus.view.showHidden", "Toggle hidden files"),
        ));
        let sort_btn = gtk::MenuButton::new();
        sort_btn.set_icon_name("view-more-symbolic");
        sort_btn.set_tooltip_text(Some(&ctx.t_or("nautilus.view.viewOptions", "View options")));
        let new_tab = gtk::Button::from_icon_name("tab-new-symbolic");
        new_tab.set_tooltip_text(Some(&ctx.t_or("nautilus.contextMenu.newTab", "New tab")));
        let split_btn = gtk::Button::from_icon_name("view-dual-symbolic");
        split_btn.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.toggleSplit", "Toggle split view"),
        ));
        let star = gtk::Button::from_icon_name("non-starred-symbolic");
        star.set_tooltip_text(Some(&ctx.t_or("nautilus.view.star", "Star this folder")));
        let icon_btn = gtk::Button::from_icon_name("zoom-in-symbolic");
        icon_btn.set_tooltip_text(Some(&ctx.t_or("nautilus.view.iconSize", "Icon size")));

        toolbar.append(&back);
        toolbar.append(&forward);
        toolbar.append(&up);
        let path_menu = gtk::MenuButton::new();
        path_menu.set_icon_name("open-menu-symbolic");
        path_menu.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.pathOptions", "Path options"),
        ));
        toolbar.append(&reload);
        toolbar.append(&path_stack);
        toolbar.append(&path_menu);
        toolbar.append(&new_folder);
        toolbar.append(&upload);
        toolbar.append(&new_tab);
        toolbar.append(&split_btn);
        toolbar.append(&star);
        toolbar.append(&layout);
        toolbar.append(&icon_btn);
        toolbar.append(&sort_btn);
        toolbar.append(&hidden_btn);

        let split = adw::OverlaySplitView::new();
        split.set_min_sidebar_width(220.0);
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let side_scroll = gtk::ScrolledWindow::new();
        side_scroll.set_child(Some(&sidebar));
        split.set_sidebar(Some(&side_scroll));

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Multiple);
        let grid = make_flow();
        let left_stack = gtk::Stack::new();
        left_stack.add_named(&list, Some("list"));
        left_stack.add_named(&grid, Some("grid"));
        let files_scroll = gtk::ScrolledWindow::new();
        files_scroll.set_vexpand(true);
        files_scroll.set_child(Some(&left_stack));
        let list_right = gtk::ListBox::new();
        list_right.add_css_class("boxed-list");
        list_right.set_selection_mode(gtk::SelectionMode::Multiple);
        let grid_right = make_flow();
        let right_stack = gtk::Stack::new();
        right_stack.add_named(&list_right, Some("list"));
        right_stack.add_named(&grid_right, Some("grid"));
        let right_scroll = gtk::ScrolledWindow::new();
        right_scroll.set_vexpand(true);
        right_scroll.set_child(Some(&right_stack));
        let split_on = ctx.settings.borrow().nautilus.split_enabled;
        right_scroll.set_visible(split_on);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&files_scroll));
        paned.set_end_child(Some(&right_scroll));
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        paned.set_wide_handle(true);
        let divider = ctx.settings.borrow().nautilus.split_divider_pos;
        if divider > 0 {
            paned.set_position(divider);
        }
        let tab_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tab_bar.add_css_class("linked");
        tab_bar.set_margin_start(8);
        tab_bar.set_margin_end(8);
        let files_col = gtk::Box::new(gtk::Orientation::Vertical, 4);
        files_col.append(&tab_bar);
        files_col.append(&paned);
        split.set_content(Some(&files_col));

        let ops = gtk::ListBox::new();
        ops.add_css_class("boxed-list");
        let ops_scroll = gtk::ScrolledWindow::new();
        ops_scroll.set_min_content_height(90);
        ops_scroll.set_child(Some(&ops));

        let status = gtk::Label::new(Some(&ctx.t_or("common.ready", "Ready")));
        status.add_css_class("dim-label");
        status.set_xalign(0.0);
        status.set_margin_start(10);
        status.set_margin_bottom(6);

        let picker_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        picker_bar.add_css_class("toolbar");
        picker_bar.set_margin_start(8);
        picker_bar.set_margin_end(8);
        picker_bar.set_margin_bottom(4);
        picker_bar.set_visible(false);
        let picker_label = gtk::Label::new(Some(
            &ctx.t_or("nautilus.titles.selectItems", "Select a location"),
        ));
        picker_label.set_hexpand(true);
        picker_label.set_xalign(0.0);
        picker_bar.append(&picker_label);
        let picker_cancel = gtk::Button::with_label(&ctx.t("common.cancel"));
        picker_bar.append(&picker_cancel);
        let picker_select = gtk::Button::with_label(&ctx.t_or("common.ok", "Select"));
        picker_select.add_css_class("suggested-action");
        picker_bar.append(&picker_select);

        let share_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        share_bar.add_css_class("toolbar");
        share_bar.set_margin_start(8);
        share_bar.set_margin_end(8);
        share_bar.set_margin_bottom(4);
        share_bar.set_visible(false);
        let share_label = gtk::Label::new(None);
        share_label.set_hexpand(true);
        share_label.set_xalign(0.0);
        share_bar.append(&share_label);
        let share_cancel = gtk::Button::with_label(&ctx.t("common.cancel"));
        share_bar.append(&share_cancel);
        let share_upload =
            gtk::Button::with_label(&ctx.t_or("nautilus.androidShare.confirm", "Upload here"));
        share_upload.add_css_class("suggested-action");
        share_bar.append(&share_upload);

        let filter_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        filter_bar.add_css_class("linked");
        filter_bar.set_margin_start(8);
        filter_bar.set_margin_end(8);
        filter_bar.set_margin_bottom(4);
        filter_bar.set_halign(gtk::Align::Start);

        root.append(&toolbar);
        root.append(&filter_bar);
        root.append(&picker_bar);
        root.append(&share_bar);
        root.append(&split);
        root.append(&ops_scroll);
        root.append(&status);

        let initial = TabState {
            id: 1,
            title: "Home".into(),
            remote: "local".into(),
            path: dirs::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".into()),
            starred: false,
        };

        let view = Self {
            root,
            ctx,
            toast,
            sidebar,
            list,
            list_right,
            grid,
            grid_right,
            left_stack,
            right_stack,
            layout_btn: layout.clone(),
            path_entry,
            path_stack,
            crumbs,
            search_entry,
            search_filter: Rc::new(RefCell::new(String::new())),
            status,
            tabs: Rc::new(RefCell::new(vec![initial.clone()])),
            current: Rc::new(RefCell::new(initial.clone())),
            secondary: Rc::new(RefCell::new(initial)),
            history: Rc::new(RefCell::new(vec![])),
            future: Rc::new(RefCell::new(vec![])),
            clipboard: Rc::new(RefCell::new(Vec::new())),
            pending_drag: Rc::new(RefCell::new(None)),
            skip_lasso: Rc::new(Cell::new(false)),
            hover_open: Rc::new(RefCell::new(None)),
            undo: Rc::new(RefCell::new(vec![])),
            redo: Rc::new(RefCell::new(vec![])),
            tab_bar,
            next_tab_id: Rc::new(RefCell::new(2)),
            split_enabled: Rc::new(RefCell::new(split_on)),
            paned,
            right_scroll,
            ops,
            last_listing: Rc::new(RefCell::new(Vec::new())),
            picker_bar,
            picker_label,
            share_bar,
            share_label,
            filter_bar,
            icon_btn: icon_btn.clone(),
        };
        view.refresh_type_filters();

        {
            let view = view.clone();
            back.connect_clicked(move |_| view.go_back());
        }
        {
            let view = view.clone();
            forward.connect_clicked(move |_| view.go_forward());
        }
        {
            let view = view.clone();
            up.connect_clicked(move |_| view.go_up());
        }
        {
            let view = view.clone();
            reload.connect_clicked(move |_| view.reload());
        }
        {
            let view = view.clone();
            view.path_entry.clone().connect_activate(move |entry| {
                view.navigate_to(&entry.text());
                view.show_crumbs();
            });
        }
        {
            let view = view.clone();
            view.search_entry
                .clone()
                .connect_search_changed(move |entry| {
                    *view.search_filter.borrow_mut() = entry.text().to_string();
                    view.reload();
                });
        }
        {
            let view = view.clone();
            view.search_entry.clone().connect_stop_search(move |_| {
                view.search_entry.set_text("");
                view.search_filter.borrow_mut().clear();
                view.show_crumbs();
                view.reload();
            });
        }
        {
            let view = view.clone();
            new_folder.connect_clicked(move |_| view.mkdir_prompt());
        }
        {
            let view = view.clone();
            upload.connect_clicked(move |_| view.upload_prompt());
        }
        {
            let view = view.clone();
            layout.connect_clicked(move |_| {
                let next = if view.is_grid() { "list" } else { "grid" };
                view.ctx.settings.borrow_mut().nautilus.layout = next.into();
                view.ctx.persist();
                view.sync_layout();
                view.reload();
            });
        }
        {
            let view = view.clone();
            hidden_btn.connect_clicked(move |_| {
                let hidden = !view.ctx.settings.borrow().nautilus.show_hidden;
                view.ctx.settings.borrow_mut().nautilus.show_hidden = hidden;
                view.ctx.persist();
                view.reload();
            });
        }
        {
            let view = view.clone();
            view.list.clone().connect_row_activated(move |_, row| {
                if let Some(name) = row_name(row) {
                    view.open_name(&name);
                }
            });
        }
        {
            let view = view.clone();
            view.list_right
                .clone()
                .connect_row_activated(move |_, row| {
                    if let Some(name) = row_name(row) {
                        view.open_name_in_pane(&name, false);
                    }
                });
        }
        {
            let view = view.clone();
            view.grid.clone().connect_child_activated(move |_, child| {
                if let Some(name) = flow_child_name(child) {
                    view.open_name(&name);
                }
            });
        }
        {
            let view = view.clone();
            view.grid_right
                .clone()
                .connect_child_activated(move |_, child| {
                    if let Some(name) = flow_child_name(child) {
                        view.open_name_in_pane(&name, false);
                    }
                });
        }
        {
            let view = view.clone();
            new_tab.connect_clicked(move |_| view.open_new_tab());
        }
        {
            let view = view.clone();
            split_btn.connect_clicked(move |_| view.toggle_split());
        }
        {
            let view = view.clone();
            star.connect_clicked(move |_| view.toggle_star_current());
        }
        {
            let view = view.clone();
            icon_btn.connect_clicked(move |_| view.cycle_icon_size());
        }
        {
            let ctx = view.ctx.clone();
            let pending = Rc::new(Cell::new(false));
            view.paned
                .connect_notify_local(Some("position"), move |p, _| {
                    let pos = p.position();
                    if pos > 0 {
                        ctx.settings.borrow_mut().nautilus.split_divider_pos = pos;
                    }
                    if pending.get() {
                        return;
                    }
                    pending.set(true);
                    let ctx = ctx.clone();
                    let pending = pending.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_millis(400),
                        move || {
                            pending.set(false);
                            let _ = ctx.settings.borrow().save();
                        },
                    );
                });
        }
        view.attach_view_options(&sort_btn);
        view.attach_path_options(&path_menu);
        {
            let view = view.clone();
            view.list
                .clone()
                .connect_selected_rows_changed(move |_| view.refresh_selection_status());
        }
        {
            let view = view.clone();
            view.list_right
                .clone()
                .connect_selected_rows_changed(move |_| view.refresh_selection_status());
        }
        {
            let view = view.clone();
            view.grid
                .clone()
                .connect_selected_children_changed(move |_| view.refresh_selection_status());
        }
        {
            let view = view.clone();
            view.grid_right
                .clone()
                .connect_selected_children_changed(move |_| view.refresh_selection_status());
        }
        view.attach_file_controllers(&view.list, false, true);
        view.attach_file_controllers(&view.list_right, false, false);
        view.attach_file_controllers(&view.grid, true, true);
        view.attach_file_controllers(&view.grid_right, true, false);

        {
            let view = view.clone();
            picker_cancel.connect_clicked(move |_| view.finish_picker(true));
        }
        {
            let view = view.clone();
            picker_select.connect_clicked(move |_| view.finish_picker(false));
        }
        {
            let view = view.clone();
            share_cancel.connect_clicked(move |_| view.cancel_share_intake());
        }
        {
            let view = view.clone();
            share_upload.connect_clicked(move |_| view.confirm_share_intake());
        }
        view.refresh_share_banner();
        view.sync_layout();
        view.reload_sidebar();
        view.reload();
        view.refresh_tabs();
        view.reload_ops();
        view.install_keybinds();
        view
    }

    fn is_grid(&self) -> bool {
        self.ctx.settings.borrow().nautilus.layout == "grid"
    }

    fn current_sort_key(&self) -> &'static str {
        let nautilus = self.ctx.settings.borrow().nautilus.clone();
        sort_option_key(&nautilus.sort_by, nautilus.sort_desc)
    }

    fn apply_sort_key(&self, key: &str) {
        {
            let mut settings = self.ctx.settings.borrow_mut();
            let mut by = settings.nautilus.sort_by.clone();
            let mut desc = settings.nautilus.sort_desc;
            apply_sort_option(&mut by, &mut desc, key);
            settings.nautilus.sort_by = by;
            settings.nautilus.sort_desc = desc;
        }
        self.ctx.persist();
        self.reload();
    }

    fn icon_size_steps(&self) -> &'static [i32] {
        if self.is_grid() {
            &[48, 64, 96, 128, 160, 256]
        } else {
            &[16, 24, 32, 48]
        }
    }

    fn current_icon_size(&self) -> i32 {
        let nautilus = &self.ctx.settings.borrow().nautilus;
        if self.is_grid() {
            nautilus.grid_icon_px()
        } else {
            nautilus.list_icon_px()
        }
    }

    fn bump_icon_size(&self, dir: i32) {
        let steps = self.icon_size_steps();
        let cur = self.current_icon_size();
        let idx = steps
            .iter()
            .position(|&s| s == cur)
            .or_else(|| steps.iter().position(|&s| s > cur))
            .unwrap_or(0);
        let next_idx = (idx as i32 + dir).clamp(0, steps.len() as i32 - 1) as usize;
        let next = steps[next_idx];
        if self.is_grid() {
            self.ctx.settings.borrow_mut().nautilus.grid_icon_size = next;
        } else {
            self.ctx.settings.borrow_mut().nautilus.icon_size = next;
        }
        self.ctx.persist();
        self.icon_btn.set_tooltip_text(Some(&format!(
            "{}: {next}px",
            self.ctx.t_or("nautilus.view.iconSize", "Icon size")
        )));
        self.reload();
    }

    fn cycle_icon_size(&self) {
        let steps = self.icon_size_steps();
        let cur = self.current_icon_size();
        let idx = steps.iter().position(|&s| s == cur).unwrap_or(0);
        let next = steps[(idx + 1) % steps.len()];
        if self.is_grid() {
            self.ctx.settings.borrow_mut().nautilus.grid_icon_size = next;
        } else {
            self.ctx.settings.borrow_mut().nautilus.icon_size = next;
        }
        self.ctx.persist();
        self.icon_btn.set_tooltip_text(Some(&format!(
            "{}: {next}px",
            self.ctx.t_or("nautilus.view.iconSize", "Icon size")
        )));
        self.reload();
    }

    fn attach_path_options(&self, button: &gtk::MenuButton) {
        let popover = gtk::Popover::new();
        button.set_popover(Some(&popover));
        let view = self.clone();
        popover.connect_show(move |popover| {
            popover.set_child(Some(&view.build_path_options()));
        });
    }

    fn build_path_options(&self) -> gtk::Box {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        box_.set_margin_start(8);
        box_.set_margin_end(8);
        let starred = self.current.borrow().starred;
        let mut items: Vec<(String, &str)> = Vec::new();
        if !starred {
            items.extend([
                (
                    self.ctx
                        .t_or("nautilus.contextMenu.newFolder", "New folder"),
                    "mkdir",
                ),
                (
                    self.ctx
                        .t_or("nautilus.contextMenu.uploadFiles", "Upload files"),
                    "upload",
                ),
                (
                    self.ctx
                        .t_or("nautilus.contextMenu.uploadFolder", "Upload folder"),
                    "uploaddir",
                ),
                (
                    self.ctx.t_or("nautilus.modals.copyUrl.title", "Copy URL"),
                    "copyurl",
                ),
            ]);
            if !self.clipboard.borrow().is_empty() {
                items.push((
                    self.ctx.t_or("nautilus.contextMenu.paste", "Paste"),
                    "paste",
                ));
            }
        }
        items.push((
            self.ctx.t_or("nautilus.contextMenu.reload", "Reload"),
            "reload",
        ));
        if !starred {
            items.push((
                self.ctx
                    .t_or("nautilus.contextMenu.copyLocation", "Copy location"),
                "location",
            ));
        }
        items.push((
            self.ctx
                .t_or("nautilus.contextMenu.selectAll", "Select all"),
            "selectall",
        ));
        if !starred {
            let current = self.current.borrow().clone();
            if current.remote != "local" {
                let registered =
                    crate::platform::is_send_to_registered(&current.remote, Some(&current.path));
                items.push((
                    if registered {
                        self.ctx.t_or(
                            "nautilus.contextMenu.removeFromSendTo",
                            "Remove from File Manager Menu",
                        )
                    } else {
                        self.ctx.t_or(
                            "nautilus.contextMenu.addToSendTo",
                            "Add to File Manager Menu",
                        )
                    },
                    "sendto",
                ));
            }
            items.push((
                self.ctx
                    .t_or("nautilus.contextMenu.properties", "Properties"),
                "props",
            ));
        }
        for (label, action) in items {
            let btn = gtk::Button::with_label(&label);
            btn.set_halign(gtk::Align::Fill);
            let view = self.clone();
            btn.connect_clicked(move |_| match action {
                "mkdir" => view.mkdir_prompt(),
                "upload" => view.upload_prompt(),
                "uploaddir" => view.upload_folder_prompt(),
                "copyurl" => view.copy_url_prompt(),
                "paste" => view.paste(),
                "reload" => view.reload(),
                "location" => view.copy_current_location(),
                "selectall" => view.select_all(),
                "sendto" => view.toggle_send_to(),
                "props" => view.properties_selected(),
                _ => {}
            });
            box_.append(&btn);
        }
        box_
    }

    fn attach_view_options(&self, button: &gtk::MenuButton) {
        let popover = gtk::Popover::new();
        button.set_popover(Some(&popover));
        let view = self.clone();
        popover.connect_show(move |popover| {
            popover.set_child(Some(&view.build_view_options()));
        });
    }

    fn build_view_options(&self) -> gtk::Box {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
        box_.set_margin_top(10);
        box_.set_margin_bottom(10);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        let size_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let size_label =
            gtk::Label::new(Some(&self.ctx.t_or("nautilus.view.iconSize", "Icon size")));
        size_label.set_hexpand(true);
        size_label.set_xalign(0.0);
        let minus = gtk::Button::from_icon_name("list-remove-symbolic");
        minus.set_tooltip_text(Some(&self.ctx.t_or("nautilus.view.smaller", "Smaller")));
        let plus = gtk::Button::from_icon_name("list-add-symbolic");
        plus.set_tooltip_text(Some(&self.ctx.t_or("nautilus.view.larger", "Larger")));
        {
            let view = self.clone();
            minus.connect_clicked(move |_| view.bump_icon_size(-1));
        }
        {
            let view = self.clone();
            plus.connect_clicked(move |_| view.bump_icon_size(1));
        }
        size_row.append(&size_label);
        size_row.append(&minus);
        size_row.append(&plus);
        box_.append(&size_row);
        let sort_heading = gtk::Label::new(Some(&self.ctx.t_or("nautilus.sort.label", "Sort")));
        sort_heading.add_css_class("heading");
        sort_heading.set_xalign(0.0);
        sort_heading.set_margin_top(6);
        box_.append(&sort_heading);
        let current = self.current_sort_key();
        let options = [
            ("name-asc", "nautilus.sort.az", "A-Z"),
            ("name-desc", "nautilus.sort.za", "Z-A"),
            (
                "modified-desc",
                "nautilus.sort.lastModified",
                "Last Modified",
            ),
            (
                "modified-asc",
                "nautilus.sort.firstModified",
                "First Modified",
            ),
            (
                "size-desc",
                "nautilus.sort.sizeLargest",
                "Size (Largest First)",
            ),
            (
                "size-asc",
                "nautilus.sort.sizeSmallest",
                "Size (Smallest First)",
            ),
        ];
        let mut group: Option<gtk::CheckButton> = None;
        for (key, i18n_key, fallback) in options {
            let btn = gtk::CheckButton::with_label(&self.ctx.t_or(i18n_key, fallback));
            if let Some(leader) = &group {
                btn.set_group(Some(leader));
            } else {
                group = Some(btn.clone());
            }
            btn.set_active(current == key);
            {
                let view = self.clone();
                btn.connect_toggled(move |btn| {
                    if btn.is_active() {
                        view.apply_sort_key(key);
                    }
                });
            }
            box_.append(&btn);
        }
        let split = gtk::CheckButton::with_label(
            &self
                .ctx
                .t_or("nautilus.contextMenu.toggleSplit", "Split view"),
        );
        split.set_active(*self.split_enabled.borrow());
        {
            let view = self.clone();
            split.connect_toggled(move |btn| {
                if btn.is_active() != *view.split_enabled.borrow() {
                    view.toggle_split();
                }
            });
        }
        box_.append(&split);
        let hidden = gtk::CheckButton::with_label(
            &self
                .ctx
                .t_or("nautilus.view.showHidden", "Show hidden files"),
        );
        hidden.set_active(self.ctx.settings.borrow().nautilus.show_hidden);
        {
            let view = self.clone();
            hidden.connect_toggled(move |btn| {
                let current = view.ctx.settings.borrow().nautilus.show_hidden;
                if btn.is_active() != current {
                    view.ctx.settings.borrow_mut().nautilus.show_hidden = btn.is_active();
                    view.ctx.persist();
                    view.reload();
                }
            });
        }
        box_.append(&hidden);
        box_
    }

    fn listing_count_label(&self, count: usize) -> String {
        if count == 0 {
            return self
                .ctx
                .t_or("nautilus.empty.folderEmpty", "Folder is Empty");
        }
        let label = if count == 1 {
            self.ctx.t_or("nautilus.selection.item", "item")
        } else {
            self.ctx.t_or("nautilus.selection.items", "items")
        };
        format!("{count} {label}")
    }

    fn selection_status(&self) -> String {
        let names = self.selected_names();
        if names.is_empty() {
            return String::new();
        }
        let listing = self.last_listing.borrow();
        let selected: Vec<&DirEntry> = listing
            .iter()
            .filter(|entry| names.iter().any(|name| name == &entry.name))
            .collect();
        if selected.is_empty() {
            return String::new();
        }
        let sel = self.ctx.t_or("nautilus.selection.selected", "selected");
        if selected.len() == 1 {
            let item = selected[0];
            return if item.is_dir {
                format!("\"{}\" {sel}", item.name)
            } else {
                format!("\"{}\" {sel} ({})", item.name, format_bytes(item.size))
            };
        }
        let folders = selected.iter().filter(|entry| entry.is_dir).count();
        let files: Vec<&&DirEntry> = selected.iter().filter(|entry| !entry.is_dir).collect();
        let mut parts = Vec::new();
        if folders > 0 {
            let label = if folders == 1 {
                self.ctx.t_or("nautilus.selection.folder", "folder")
            } else {
                self.ctx.t_or("nautilus.selection.folders", "folders")
            };
            parts.push(format!("{folders} {label} {sel}"));
        }
        if !files.is_empty() {
            let total: i64 = files.iter().map(|entry| entry.size).sum();
            let label = if files.len() == 1 {
                self.ctx.t_or("nautilus.selection.item", "item")
            } else {
                self.ctx.t_or("nautilus.selection.items", "items")
            };
            parts.push(format!(
                "{} {label} {sel} ({})",
                files.len(),
                format_bytes(total)
            ));
        }
        parts.join(", ")
    }

    fn refresh_selection_status(&self) {
        let text = self.selection_status();
        if text.is_empty() {
            self.status
                .set_text(&self.listing_count_label(self.last_listing.borrow().len()));
        } else {
            self.status.set_text(&text);
        }
    }

    fn current_location(&self) -> String {
        let current = self.current.borrow().clone();
        if current.remote == "local" {
            current.path
        } else if current.path.is_empty() {
            format!("{}:", current.remote)
        } else {
            format!("{}:{}", current.remote, current.path)
        }
    }

    fn toggle_star_path(&self, path: &str, name: &str) {
        let now = {
            let mut settings = self.ctx.settings.borrow_mut();
            crate::settings::toggle_collection(&mut settings.nautilus.starred, path, name)
        };
        self.ctx.persist();
        self.reload_sidebar();
        self.toast.add_toast(adw::Toast::new(&if now {
            self.ctx.t_or("nautilus.view.starred", "Starred")
        } else {
            self.ctx
                .t_or("nautilus.view.unstarred", "Removed from Starred")
        }));
        if self.current.borrow().starred {
            self.reload();
        }
    }

    fn toggle_star_current(&self) {
        let path = self.current_location();
        let name = self.current.borrow().title.clone();
        self.toggle_star_path(&path, &name);
    }

    fn toggle_star_selected(&self) {
        if let Some(name) = self.selected_name() {
            if self.current.borrow().starred {
                if let Some(entry) = self.last_listing.borrow().iter().find(|e| e.name == name) {
                    self.toggle_star_path(&entry.path, &entry.name);
                    return;
                }
            }
            let path = self.formatted_path(Some(&name));
            self.toggle_star_path(&path, &name);
        } else {
            self.toggle_star_current();
        }
    }

    fn show_starred(&self) {
        let current = self.current.borrow().clone();
        if !current.starred {
            self.history
                .borrow_mut()
                .push((current.remote, current.path));
            self.future.borrow_mut().clear();
        }
        self.current.borrow_mut().starred = true;
        self.current.borrow_mut().title = self.ctx.t_or("nautilus.titles.starred", "Starred");
        self.sync_current_tab();
        self.reload();
    }

    fn populate_starred(&self) {
        clear_list(&self.list);
        clear_flow(&self.grid);
        let items = self.ctx.settings.borrow().nautilus.starred.clone();
        let mut entries = Vec::new();
        for item in items {
            let Some(path) = crate::settings::collection_path(&item) else {
                continue;
            };
            let name = item
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or_else(|| path.rsplit(['/', ':']).next().unwrap_or(&path))
                .to_string();
            entries.push(DirEntry {
                name,
                path,
                is_dir: true,
                size: 0,
                mime: String::new(),
                mod_time: String::new(),
            });
        }
        if entries.is_empty() {
            self.status.set_text(
                &self
                    .ctx
                    .t_or("nautilus.empty.noStarred", "No Starred Files"),
            );
        } else {
            self.status
                .set_text(&self.listing_count_label(entries.len()));
        }
        self.path_entry.set_text("");
        self.refresh_crumbs();
        self.sync_current_tab();
        self.refresh_tabs();
        self.reload_ops();
        self.populate_entries(&entries, true);
    }

    fn sync_layout(&self) {
        let name = if self.is_grid() { "grid" } else { "list" };
        self.left_stack.set_visible_child_name(name);
        self.right_stack.set_visible_child_name(name);
        self.layout_btn.set_icon_name(if self.is_grid() {
            "view-grid-symbolic"
        } else {
            "view-list-symbolic"
        });
    }

    fn attach_file_controllers(&self, widget: &impl IsA<gtk::Widget>, grid: bool, primary: bool) {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        {
            let view = self.clone();
            gesture.connect_pressed(move |_, _, _, _| view.popup_context());
        }
        widget.add_controller(gesture);

        let drop = gtk::DropTarget::new(gio::File::static_type(), gtk::gdk::DragAction::COPY);
        {
            let view = self.clone();
            drop.connect_drop(move |_, value, _, _| {
                if let Ok(file) = value.get::<gio::File>() {
                    if let Some(path) = file.path() {
                        view.upload_local_path(&path);
                        return true;
                    }
                }
                false
            });
        }
        widget.add_controller(drop);
        self.attach_internal_drop(
            widget,
            if primary {
                InternalDrop::CurrentPane
            } else {
                InternalDrop::SecondaryPane
            },
        );

        let drag = gtk::GestureDrag::new();
        {
            let view = self.clone();
            drag.connect_drag_end(move |g, _, _| {
                if view.skip_lasso.get() {
                    return;
                }
                let Some((x, y)) = g.start_point() else {
                    return;
                };
                let Some((ox, oy)) = g.offset() else {
                    return;
                };
                view.apply_lasso(x, y, x + ox, y + oy, grid);
            });
        }
        widget.add_controller(drag);
    }

    fn attach_item_dnd(
        &self,
        widget: &impl IsA<gtk::Widget>,
        entry: &DirEntry,
        tab: &TabState,
        primary: bool,
    ) {
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE);
        {
            let view = self.clone();
            let entry = entry.clone();
            let tab = tab.clone();
            source.connect_prepare(move |_, _, _| {
                let items = view.drag_items_for(&entry, &tab, primary);
                if items.is_empty() {
                    return None;
                }
                view.skip_lasso.set(true);
                *view.pending_drag.borrow_mut() = Some(items.clone());
                Some(gtk::gdk::ContentProvider::for_value(
                    &crate::dnd::encode_payload(&items).to_value(),
                ))
            });
        }
        {
            let view = self.clone();
            source.connect_drag_end(move |_, _, _| {
                view.pending_drag.borrow_mut().take();
                view.clear_hover_open();
                let view = view.clone();
                glib::idle_add_local_once(move || view.skip_lasso.set(false));
            });
        }
        widget.add_controller(source);
        if entry.is_dir {
            let dest_path = if tab.starred {
                entry.path.clone()
            } else {
                join_remote_path(&tab.path, &entry.name)
            };
            let location = if tab.starred {
                dest_path
            } else {
                crate::dnd::location_string(&tab.remote, &dest_path)
            };
            self.attach_internal_drop(
                widget,
                InternalDrop::Location {
                    location,
                    secondary: !primary,
                },
            );
        }
    }

    fn attach_internal_drop(&self, widget: &impl IsA<gtk::Widget>, dest: InternalDrop) {
        let drop = gtk::DropTarget::new(
            glib::Type::STRING,
            gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
        );
        {
            let view = self.clone();
            let dest = dest.clone();
            drop.connect_drop(move |_, value, _, _| {
                view.clear_hover_open();
                let Ok(text) = value.get::<String>() else {
                    return false;
                };
                view.handle_internal_drop(&text, &dest)
            });
        }
        {
            let view = self.clone();
            let dest = dest.clone();
            drop.connect_enter(move |_, _, _| {
                view.schedule_hover_open(&dest);
                gtk::gdk::DragAction::COPY
            });
        }
        {
            let view = self.clone();
            drop.connect_leave(move |_| {
                view.clear_hover_open();
            });
        }
        widget.add_controller(drop);
    }

    fn hover_key(dest: &InternalDrop) -> String {
        match dest {
            InternalDrop::CurrentPane => "pane:current".into(),
            InternalDrop::SecondaryPane => "pane:secondary".into(),
            InternalDrop::Location {
                location,
                secondary,
            } => {
                format!("loc:{secondary}:{location}")
            }
            InternalDrop::Starred => "starred".into(),
            InternalDrop::Tab(id) => format!("tab:{id}"),
        }
    }

    fn schedule_hover_open(&self, dest: &InternalDrop) {
        let key = Self::hover_key(dest);
        if self.hover_open.borrow().as_deref() == Some(key.as_str()) {
            return;
        }
        *self.hover_open.borrow_mut() = Some(key.clone());
        let view = self.clone();
        let dest = dest.clone();
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(crate::dnd::HOVER_OPEN_MS),
            move || {
                if view.hover_open.borrow().as_deref() != Some(key.as_str()) {
                    return;
                }
                if view.pending_drag.borrow().is_none() {
                    return;
                }
                view.apply_hover_open(&dest);
            },
        );
    }

    fn clear_hover_open(&self) {
        self.hover_open.borrow_mut().take();
    }

    fn apply_hover_open(&self, dest: &InternalDrop) {
        match dest {
            InternalDrop::Location {
                location,
                secondary,
            } => {
                let items = self.pending_drag.borrow().clone().unwrap_or_default();
                let drop_dest = crate::dnd::dest_from_location(location);
                let current = if *secondary {
                    self.secondary.borrow().clone()
                } else {
                    self.current.borrow().clone()
                };
                let current_dest = crate::dnd::DropDest {
                    remote: current.remote,
                    path: current.path,
                };
                if crate::dnd::should_hover_navigate(&items, &drop_dest, &current_dest) {
                    self.navigate_location(location, *secondary);
                }
            }
            InternalDrop::Tab(id) => {
                if self.current.borrow().id != *id {
                    self.activate_tab(*id);
                }
            }
            InternalDrop::Starred => {
                if !self.current.borrow().starred {
                    self.show_starred();
                }
            }
            InternalDrop::CurrentPane | InternalDrop::SecondaryPane => {}
        }
    }

    fn navigate_location(&self, input: &str, secondary: bool) {
        if secondary {
            self.navigate_secondary(input);
        } else {
            self.navigate_to(input);
        }
    }

    fn navigate_secondary(&self, input: &str) {
        if input == "starred:" || input == "starred" {
            return;
        }
        let (remote, path) = split_remote_path(input);
        self.secondary.borrow_mut().remote = remote;
        self.secondary.borrow_mut().path = path;
        self.secondary.borrow_mut().starred = false;
        if *self.split_enabled.borrow() {
            self.reload_pane(&self.secondary.borrow());
        }
    }

    fn drag_items_for(
        &self,
        clicked: &DirEntry,
        tab: &TabState,
        primary: bool,
    ) -> Vec<crate::dnd::DragItem> {
        if !primary {
            return vec![self.drag_item_from_entry(clicked, tab)];
        }
        let selected = self.selected_names();
        let names = if selected.iter().any(|name| name == &clicked.name) {
            selected
        } else {
            vec![clicked.name.clone()]
        };
        let listing = self.last_listing.borrow().clone();
        names
            .into_iter()
            .map(|name| {
                listing
                    .iter()
                    .find(|entry| entry.name == name)
                    .cloned()
                    .unwrap_or_else(|| DirEntry {
                        name: name.clone(),
                        path: join_remote_path(&tab.path, &name),
                        is_dir: false,
                        size: 0,
                        mime: String::new(),
                        mod_time: String::new(),
                    })
            })
            .map(|entry| self.drag_item_from_entry(&entry, tab))
            .collect()
    }

    fn drag_item_from_entry(&self, entry: &DirEntry, tab: &TabState) -> crate::dnd::DragItem {
        if tab.starred {
            let (remote, path) = split_remote_path(&entry.path);
            return crate::dnd::DragItem {
                remote,
                path,
                name: entry.name.clone(),
                is_dir: entry.is_dir,
            };
        }
        crate::dnd::DragItem {
            remote: tab.remote.clone(),
            path: join_remote_path(&tab.path, &entry.name),
            name: entry.name.clone(),
            is_dir: entry.is_dir,
        }
    }

    fn handle_internal_drop(&self, text: &str, dest: &InternalDrop) -> bool {
        let Some(items) =
            crate::dnd::decode_payload(text).or_else(|| self.pending_drag.borrow().clone())
        else {
            return false;
        };
        match dest {
            InternalDrop::Starred => {
                self.star_drag_items(&items);
                true
            }
            InternalDrop::CurrentPane => {
                let current = self.current.borrow().clone();
                self.apply_transfer_drop(
                    &items,
                    &crate::dnd::DropDest {
                        remote: current.remote,
                        path: current.path,
                    },
                )
            }
            InternalDrop::SecondaryPane => {
                let secondary = self.secondary.borrow().clone();
                self.apply_transfer_drop(
                    &items,
                    &crate::dnd::DropDest {
                        remote: secondary.remote,
                        path: secondary.path,
                    },
                )
            }
            InternalDrop::Location { location, .. } => {
                self.apply_transfer_drop(&items, &crate::dnd::dest_from_location(location))
            }
            InternalDrop::Tab(id) => {
                let tab = self.tabs.borrow().iter().find(|tab| tab.id == *id).cloned();
                let Some(tab) = tab else {
                    return false;
                };
                if tab.starred {
                    self.star_drag_items(&items);
                    return true;
                }
                self.apply_transfer_drop(
                    &items,
                    &crate::dnd::DropDest {
                        remote: tab.remote,
                        path: tab.path,
                    },
                )
            }
        }
    }

    fn star_drag_items(&self, items: &[crate::dnd::DragItem]) {
        for item in items {
            let location = crate::dnd::location_string(&item.remote, &item.path);
            if crate::settings::collection_contains(
                &self.ctx.settings.borrow().nautilus.starred,
                &location,
            ) {
                continue;
            }
            self.toggle_star_path(&location, &item.name);
        }
    }

    fn apply_transfer_drop(
        &self,
        items: &[crate::dnd::DragItem],
        dest: &crate::dnd::DropDest,
    ) -> bool {
        match crate::dnd::resolve_transfer(items, dest) {
            crate::dnd::DropPlan::Ignore => false,
            crate::dnd::DropPlan::Star => {
                self.star_drag_items(items);
                true
            }
            crate::dnd::DropPlan::Transfer { dest, move_items } => {
                let Some(client) = self.ctx.client() else {
                    return false;
                };
                let transfers = crate::dnd::transfer_items(items, &dest, move_items);
                match crate::fileops::start_grouped_transfers(&client, &transfers, "filemanager") {
                    Ok((group, ids)) => {
                        for item in &transfers {
                            self.push_undo(item.file_op().encode());
                        }
                        self.ctx.refresh_runtime();
                        self.reload();
                        self.toast.add_toast(adw::Toast::new(&format!(
                            "{} · {group} · {} job(s)",
                            if move_items {
                                self.ctx.t_or("nautilus.contextMenu.move", "Move")
                            } else {
                                self.ctx.t_or("nautilus.contextMenu.copy", "Copy")
                            },
                            ids.len()
                        )));
                        true
                    }
                    Err(e) => {
                        self.toast.add_toast(adw::Toast::new(&e));
                        true
                    }
                }
            }
        }
    }

    fn apply_lasso(&self, x1: f64, y1: f64, x2: f64, y2: f64, grid: bool) {
        let left = x1.min(x2);
        let right = x1.max(x2);
        let top = y1.min(y2);
        let bottom = y1.max(y2);
        if (bottom - top).abs() < 12.0 && (!grid || (right - left).abs() < 12.0) {
            return;
        }
        if grid {
            select_flow_in_rect(&self.grid, left, top, right, bottom);
            if *self.split_enabled.borrow() {
                select_flow_in_rect(&self.grid_right, left, top, right, bottom);
            }
            return;
        }
        self.list.unselect_all();
        let mut child = self.list.first_child();
        let mut acc = 0.0;
        while let Some(row) = child {
            let h = row.height() as f64;
            let row_top = acc;
            let row_bottom = acc + h;
            if row_bottom >= top && row_top <= bottom {
                if let Ok(list_row) = row.clone().downcast::<gtk::ListBoxRow>() {
                    self.list.select_row(Some(&list_row));
                }
            }
            acc = row_bottom;
            child = row.next_sibling();
        }
    }

    fn install_keybinds(&self) {
        let controller = gtk::EventControllerKey::new();
        let view = self.clone();
        controller.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            if ctrl && key == gtk::gdk::Key::l {
                view.show_path_entry();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::f {
                view.show_search();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Escape {
                view.search_entry.set_text("");
                view.search_filter.borrow_mut().clear();
                view.show_crumbs();
                view.reload();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::F5 {
                view.reload();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::F2 {
                view.rename_selected();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::space {
                if let Some(name) = view.selected_names().first().cloned() {
                    view.open_viewer(&name);
                    return glib::Propagation::Stop;
                }
            }
            if modifier.contains(gtk::gdk::ModifierType::ALT_MASK)
                && (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter)
            {
                view.properties_selected();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Delete {
                view.delete_selected();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::c {
                view.cut_or_copy(false);
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::x {
                view.cut_or_copy(true);
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::v {
                view.paste();
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key == gtk::gdk::Key::N {
                view.mkdir_prompt();
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key == gtk::gdk::Key::d {
                view.detach_current_tab();
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key == gtk::gdk::Key::T {
                let id = view.current.borrow().id;
                view.duplicate_tab(id);
                return glib::Propagation::Stop;
            }
            if modifier.contains(gtk::gdk::ModifierType::ALT_MASK)
                && (key == gtk::gdk::Key::Left || key == gtk::gdk::Key::KP_Left)
            {
                view.go_back();
                return glib::Propagation::Stop;
            }
            if modifier.contains(gtk::gdk::ModifierType::ALT_MASK)
                && (key == gtk::gdk::Key::Right || key == gtk::gdk::Key::KP_Right)
            {
                view.go_forward();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::t {
                view.open_new_tab();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::w {
                view.close_current_tab();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::slash {
                view.toggle_split();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::i {
                view.switch_pane();
                return glib::Propagation::Stop;
            }
            if ctrl && shift && key == gtk::gdk::Key::z {
                view.redo_last();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::z {
                view.undo_last();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::y {
                view.paste_system_clipboard();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::a {
                if view.is_grid() {
                    view.grid.select_all();
                    if *view.split_enabled.borrow() {
                        view.grid_right.select_all();
                    }
                } else {
                    view.list.select_all();
                    if *view.split_enabled.borrow() {
                        view.list_right.select_all();
                    }
                }
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::h {
                let hidden = !view.ctx.settings.borrow().nautilus.show_hidden;
                view.ctx.settings.borrow_mut().nautilus.show_hidden = hidden;
                view.ctx.persist();
                view.reload();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::BackSpace {
                view.go_up();
                return glib::Propagation::Stop;
            }
            if modifier.contains(gtk::gdk::ModifierType::ALT_MASK)
                && (key == gtk::gdk::Key::Up || key == gtk::gdk::Key::KP_Up)
            {
                view.go_up();
                return glib::Propagation::Stop;
            }
            if ctrl
                && (key == gtk::gdk::Key::Tab
                    || key == gtk::gdk::Key::ISO_Left_Tab
                    || key == gtk::gdk::Key::Page_Down
                    || key == gtk::gdk::Key::Page_Up)
            {
                view.cycle_tab(
                    shift || key == gtk::gdk::Key::ISO_Left_Tab || key == gtk::gdk::Key::Page_Up,
                );
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.root.add_controller(controller);
    }

    fn refresh_share_banner(&self) {
        let count = self.ctx.store.borrow().pending_share_paths.len();
        self.share_bar.set_visible(count > 0);
        if count == 0 {
            return;
        }
        let count_s = count.to_string();
        self.share_label.set_text(
            &self
                .ctx
                .tf("nautilus.androidShare.uploadHere", &[("count", &count_s)]),
        );
    }

    fn cancel_share_intake(&self) {
        self.ctx.store.borrow_mut().pending_share_paths.clear();
        self.ctx.persist();
        self.refresh_share_banner();
    }

    fn confirm_share_intake(&self) {
        let paths = {
            let mut store = self.ctx.store.borrow_mut();
            std::mem::take(&mut store.pending_share_paths)
        };
        self.ctx.persist();
        for path in paths {
            self.upload_local_path(std::path::Path::new(&path));
        }
        self.refresh_share_banner();
    }

    fn refresh_type_filters(&self) {
        while let Some(child) = self.filter_bar.first_child() {
            self.filter_bar.remove(&child);
        }
        let current = self.ctx.settings.borrow().nautilus.file_type_filter.clone();
        let chips = [
            ("all", self.ctx.t("common.all")),
            (
                "folders",
                self.ctx.t_or("nautilus.filters.folders", "Folders"),
            ),
            ("images", self.ctx.t_or("nautilus.filters.images", "Images")),
            ("videos", self.ctx.t_or("nautilus.filters.videos", "Videos")),
            ("audio", self.ctx.t_or("nautilus.filters.audio", "Audio")),
            (
                "documents",
                self.ctx.t_or("nautilus.filters.documents", "Documents"),
            ),
            (
                "archives",
                self.ctx.t_or("nautilus.filters.archives", "Archives"),
            ),
        ];
        let mut group: Option<gtk::ToggleButton> = None;
        for (id, label) in chips {
            let btn = gtk::ToggleButton::with_label(&label);
            if let Some(first) = &group {
                btn.set_group(Some(first));
            } else {
                group = Some(btn.clone());
            }
            let active = (current.is_empty() && id == "all") || current == id;
            btn.set_active(active);
            let view = self.clone();
            let id = id.to_string();
            btn.connect_toggled(move |btn| {
                if !btn.is_active() {
                    return;
                }
                view.ctx.settings.borrow_mut().nautilus.file_type_filter = if id == "all" {
                    String::new()
                } else {
                    id.clone()
                };
                view.ctx.persist();
                view.reload();
            });
            self.filter_bar.append(&btn);
        }
    }

    fn reload_sidebar(&self) {
        while let Some(child) = self.sidebar.first_child() {
            self.sidebar.remove(&child);
        }
        let picker_cfg = self
            .ctx
            .pending_picker
            .borrow()
            .as_ref()
            .map(|r| r.config.clone());
        let allowed = |loc: &str| {
            picker_cfg
                .as_ref()
                .is_none_or(|cfg| crate::picker::is_location_allowed(loc, cfg))
        };
        let hidden = self
            .ctx
            .settings
            .borrow()
            .nautilus
            .sidebar_hidden_drives
            .clone();
        let order = self
            .ctx
            .settings
            .borrow()
            .nautilus
            .sidebar_drive_order
            .clone();
        let configure = adw::ActionRow::new();
        configure.set_title(
            &self
                .ctx
                .t_or("nautilus.sidebar.configureTitle", "Configure Sidebar Items"),
        );
        configure.set_activatable(true);
        configure.add_prefix(&gtk::Image::from_icon_name("emblem-system-symbolic"));
        {
            let view = self.clone();
            configure.connect_activated(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    let view = view.clone();
                    dialogs::configure_sidebar(
                        &win,
                        view.ctx.clone(),
                        Rc::new(move || view.reload_sidebar()),
                    );
                }
            });
        }
        self.sidebar.append(&configure);
        let starred_row = adw::ActionRow::new();
        starred_row.set_title(&self.ctx.t_or("nautilus.titles.starred", "Starred"));
        starred_row.set_activatable(true);
        starred_row.add_prefix(&gtk::Image::from_icon_name("starred-symbolic"));
        {
            let view = self.clone();
            starred_row.connect_activated(move |_| view.show_starred());
        }
        self.attach_internal_drop(&starred_row, InternalDrop::Starred);
        self.sidebar.append(&starred_row);
        self.add_side_header(&self.ctx.t_or("nautilus.titles.starred", "Starred"));
        for star in &self.ctx.settings.borrow().nautilus.starred {
            if let Some(path) = star.get("path").and_then(|x| x.as_str()) {
                if allowed(path) && !crate::settings::sidebar_id_hidden(&hidden, path) {
                    let title = star
                        .get("name")
                        .and_then(|x| x.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(path);
                    self.add_side_row(title, path, "starred-symbolic", SideKind::Star);
                }
            }
        }
        self.add_side_header(&self.ctx.t_or("nautilus.titles.bookmarks", "Bookmarks"));
        for mark in &self.ctx.settings.borrow().nautilus.bookmarks {
            if let (Some(name), Some(path)) = (
                mark.get("name").and_then(|x| x.as_str()),
                mark.get("path").and_then(|x| x.as_str()),
            ) {
                if allowed(path) && !crate::settings::sidebar_id_hidden(&hidden, path) {
                    self.add_side_row(name, path, "user-bookmarks-symbolic", SideKind::Bookmark);
                }
            }
        }
        self.add_side_header(&self.ctx.t_or("nautilus.titles.local", "Local"));
        let disks = crate::settings::sort_sidebar_ids(
            self.ctx.snapshot.borrow().local_disks.clone(),
            &order,
        );
        for disk in disks {
            if allowed(&disk) && !crate::settings::sidebar_id_hidden(&hidden, &disk) {
                self.add_side_row(&disk, &disk, "drive-harddisk-symbolic", SideKind::Local);
            }
        }
        self.add_side_header(&self.ctx.t_or("nautilus.titles.cloud", "Cloud remotes"));
        let mut remotes: Vec<_> = self.ctx.snapshot.borrow().remotes.clone();
        remotes.sort_by_key(|r| {
            let loc = format!("{}:", r.name);
            order
                .iter()
                .position(|n| n == &loc || n == &r.name)
                .unwrap_or(usize::MAX)
        });
        for remote in remotes {
            let loc = format!("{}:", remote.name);
            if allowed(&loc)
                && !crate::settings::sidebar_id_hidden(&hidden, &loc)
                && !crate::settings::sidebar_id_hidden(&hidden, &remote.name)
            {
                self.add_side_row(
                    &remote.name,
                    &loc,
                    crate::providers::provider_icon(&remote.r#type),
                    SideKind::Remote,
                );
            }
        }
    }

    fn add_side_header(&self, title: &str) {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_sensitive(false);
        self.sidebar.append(&row);
    }

    fn add_side_row(&self, title: &str, target: &str, icon: &str, kind: SideKind) {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_activatable(true);
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        let view = self.clone();
        let target_nav = target.to_string();
        row.connect_activated(move |_| view.navigate_to(&target_nav));
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        {
            let view = self.clone();
            let target = target.to_string();
            let title = title.to_string();
            let row_menu = row.clone();
            gesture.connect_pressed(move |g, _, _, _| {
                view.popup_side_menu(&row_menu, &title, &target, kind);
                g.set_state(gtk::EventSequenceState::Claimed);
            });
        }
        row.add_controller(gesture);
        self.attach_internal_drop(
            &row,
            InternalDrop::Location {
                location: target.to_string(),
                secondary: false,
            },
        );
        self.sidebar.append(&row);
    }

    fn popup_side_menu(&self, row: &adw::ActionRow, title: &str, target: &str, kind: SideKind) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let popover = gtk::Popover::new();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let mut items: Vec<(String, &'static str)> = vec![
            (self.ctx.t_or("nautilus.contextMenu.open", "Open"), "open"),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.openNewTab", "Open in New Tab"),
                "tab",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.openNewWindow", "Open in New Window"),
                "window",
            ),
        ];
        match kind {
            SideKind::Star => items.push((
                self.ctx.t_or("fileBrowser.properties.unstar", "Unstar"),
                "unstar",
            )),
            SideKind::Bookmark => {
                items.push((
                    self.ctx
                        .t_or("nautilus.contextMenu.properties", "Properties"),
                    "props",
                ));
                items.push((
                    self.ctx
                        .t_or("nautilus.contextMenu.removeBookmark", "Remove Bookmark"),
                    "unbookmark",
                ));
            }
            SideKind::Local => items.push((
                self.ctx
                    .t_or("nautilus.contextMenu.properties", "Properties"),
                "props",
            )),
            SideKind::Remote => {
                items.push((
                    self.ctx.t_or("nautilus.contextMenu.about", "About"),
                    "about",
                ));
                let (remote, _) = crate::rclone::split_remote_path(target);
                let cleanup_ok = self
                    .ctx
                    .fs_info(&remote)
                    .is_none_or(|i| i.has_feature("CleanUp"));
                if cleanup_ok {
                    items.push((
                        self.ctx
                            .t_or("nautilus.contextMenu.emptyTrash", "Empty Trash"),
                        "cleanup",
                    ));
                }
            }
        }
        for (label, action) in items {
            let btn = gtk::Button::with_label(&label);
            if matches!(action, "unstar" | "unbookmark" | "cleanup") {
                btn.add_css_class("destructive-action");
            }
            let view = self.clone();
            let popover = popover.clone();
            let target = target.to_string();
            let title = title.to_string();
            btn.connect_clicked(move |_| {
                match action {
                    "open" => view.navigate_to(&target),
                    "tab" => view.open_target_in_new_tab(&target),
                    "window" => view.open_target_in_new_window(&target),
                    "unstar" => view.toggle_star_path(&target, &title),
                    "unbookmark" => view.remove_bookmark_path(&target),
                    "props" => view.properties_for_target(&target),
                    "about" => {
                        let (remote, _) = crate::rclone::split_remote_path(&target);
                        if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                            dialogs::remote_about(&win, view.ctx.clone(), &remote);
                        }
                    }
                    "cleanup" => {
                        let (remote, _) = crate::rclone::split_remote_path(&target);
                        view.cleanup_named_remote(&remote);
                    }
                    _ => {}
                }
                popover.popdown();
            });
            box_.append(&btn);
        }
        popover.set_child(Some(&box_));
        popover.set_parent(row);
        popover.popup();
        let _ = win;
    }

    fn open_target_in_new_tab(&self, target: &str) {
        self.open_new_tab();
        self.navigate_to(target);
    }

    fn open_target_in_new_window(&self, target: &str) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(app) = win.application() else {
            return;
        };
        let toast = adw::ToastOverlay::new();
        let view = NautilusView::new(self.ctx.clone(), toast.clone());
        toast.set_child(Some(&view.root));
        let detached = adw::ApplicationWindow::new(&app);
        detached.set_title(Some(&self.ctx.t_or("titlebar.menu.fileBrowser", "Files")));
        detached.set_default_width(960);
        detached.set_default_height(640);
        detached.set_content(Some(&toast));
        view.navigate_to(target);
        detached.present();
    }

    fn remove_bookmark_path(&self, path: &str) {
        self.ctx
            .settings
            .borrow_mut()
            .nautilus
            .bookmarks
            .retain(|item| crate::settings::collection_path(item).as_deref() != Some(path));
        self.ctx.persist();
        self.reload_sidebar();
    }

    fn properties_for_target(&self, target: &str) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let (remote, path) = crate::rclone::split_remote_path(target);
        let name = path
            .rsplit(['/', ':'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&remote)
            .to_string();
        dialogs::properties(&win, self.ctx.clone(), &remote, &path, &name);
    }

    fn cleanup_named_remote(&self, remote: &str) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        let fs = if remote == "local" {
            "/".into()
        } else {
            crate::rclone::remote_fs(remote, "")
        };
        match client.cleanup(&fs, None) {
            Ok(_) => self.toast.add_toast(adw::Toast::new(
                &self
                    .ctx
                    .t_or("nautilus.notifications.trashEmptied", "Cleanup started"),
            )),
            Err(e) => self.toast.add_toast(adw::Toast::new(&self.ctx.tf(
                "nautilus.errors.emptyTrashFailed",
                &[("remote", remote), ("error", &e.to_string())],
            ))),
        }
    }

    pub fn navigate_to(&self, input: &str) {
        if input == "starred:" || input == "starred" {
            self.show_starred();
            return;
        }
        if let Some(req) = self.ctx.pending_picker.borrow().as_ref() {
            if !crate::picker::is_location_allowed(input, &req.config) {
                self.toast.add_toast(adw::Toast::new(
                    "That location is not allowed for this picker",
                ));
                return;
            }
        }
        let current = self.current.borrow().clone();
        self.history
            .borrow_mut()
            .push((current.remote, current.path));
        self.future.borrow_mut().clear();
        let (remote, path) = split_remote_path(input);
        self.current.borrow_mut().remote = remote;
        self.current.borrow_mut().path = path;
        self.current.borrow_mut().starred = false;
        self.sync_current_tab();
        self.reload();
    }

    fn show_path_entry(&self) {
        self.path_stack.set_visible_child_name("entry");
        self.path_entry.grab_focus();
    }

    fn show_search(&self) {
        self.path_stack.set_visible_child_name("search");
        self.search_entry.grab_focus();
    }

    fn show_crumbs(&self) {
        self.path_stack.set_visible_child_name("crumbs");
    }

    fn refresh_crumbs(&self) {
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        if self.current.borrow().starred {
            self.add_crumb(
                &self.ctx.t_or("nautilus.titles.starred", "Starred"),
                "starred:",
            );
            return;
        }
        let current = self.current.borrow().clone();
        for (idx, (label, target)) in
            crate::path_kind::breadcrumb_targets(&current.remote, &current.path)
                .into_iter()
                .enumerate()
        {
            if idx > 0 {
                let sep = gtk::Label::new(Some("›"));
                sep.add_css_class("dim-label");
                self.crumbs.append(&sep);
            }
            let label = if idx == 0 && current.remote == "local" {
                self.ctx.t_or("nautilus.titles.local", "Local")
            } else {
                label
            };
            self.add_crumb(&label, &target);
        }
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.set_tooltip_text(Some("Edit path"));
        edit.set_has_frame(false);
        let view = self.clone();
        edit.connect_clicked(move |_| view.show_path_entry());
        self.crumbs.append(&edit);
    }

    fn add_crumb(&self, label: &str, target: &str) {
        let btn = gtk::Button::with_label(label);
        btn.set_has_frame(false);
        btn.set_tooltip_text(Some(target));
        let view = self.clone();
        let target = target.to_string();
        {
            let view = view.clone();
            let target = target.clone();
            btn.connect_clicked(move |_| view.navigate_to(&target));
        }
        self.attach_internal_drop(
            &btn,
            InternalDrop::Location {
                location: target,
                secondary: false,
            },
        );
        self.crumbs.append(&btn);
    }

    pub fn apply_pending_picker(&self) {
        let active = self.ctx.pending_picker.borrow().is_some();
        if active != self.picker_bar.is_visible() {
            self.picker_bar.set_visible(active);
            self.reload_sidebar();
            self.reload();
        }
        if let Some(req) = self.ctx.pending_picker.borrow().as_ref() {
            let text = match req.config.selection {
                crate::picker::PickerSelection::Folders => self
                    .ctx
                    .t_or("nautilus.titles.selectFolder", "Select a folder"),
                crate::picker::PickerSelection::Files => {
                    self.ctx.t_or("nautilus.titles.selectFile", "Select a file")
                }
                crate::picker::PickerSelection::Both => self
                    .ctx
                    .t_or("nautilus.titles.selectItems", "Select a file or folder"),
            };
            if self.picker_label.text().as_str() != text {
                self.picker_label.set_text(&text);
            }
        }
    }

    fn finish_picker(&self, cancelled: bool) {
        self.finish_picker_choice(cancelled, None);
    }

    fn finish_picker_choice(&self, cancelled: bool, forced: Option<(String, bool)>) {
        let Some(req) = self.ctx.pending_picker.borrow_mut().take() else {
            return;
        };
        let current = self.current.borrow().clone();
        let listing = self.last_listing.borrow().clone();
        let result = if cancelled {
            crate::picker::PickerResult {
                cancelled: true,
                ..Default::default()
            }
        } else if let Some((name, is_dir)) = forced {
            crate::picker::PickerResult {
                remote: current.remote.clone(),
                path: join_remote_path(&current.path, &name),
                is_dir,
                cancelled: false,
            }
        } else if let Some(name) = self.selected_name() {
            let is_dir = listing
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.is_dir)
                .unwrap_or(true);
            crate::picker::PickerResult {
                remote: current.remote.clone(),
                path: join_remote_path(&current.path, &name),
                is_dir,
                cancelled: false,
            }
        } else {
            crate::picker::PickerResult {
                remote: current.remote.clone(),
                path: current.path.clone(),
                is_dir: true,
                cancelled: false,
            }
        };
        let dirs = usize::from(result.is_dir);
        let files = usize::from(!result.is_dir);
        if !cancelled && !crate::picker::can_confirm_selection(dirs, files, &req.config) {
            *self.ctx.pending_picker.borrow_mut() = Some(req);
            self.picker_bar.set_visible(true);
            self.toast
                .add_toast(adw::Toast::new("Select a valid item to continue"));
            return;
        }
        self.picker_bar.set_visible(false);
        self.reload_sidebar();
        (req.on_pick)(result);
    }

    fn go_back(&self) {
        if let Some((remote, path)) = self.history.borrow_mut().pop() {
            let current = self.current.borrow().clone();
            self.future
                .borrow_mut()
                .push((current.remote, current.path));
            self.current.borrow_mut().remote = remote;
            self.current.borrow_mut().path = path;
            self.current.borrow_mut().starred = false;
            self.reload();
        }
    }

    fn go_forward(&self) {
        if let Some((remote, path)) = self.future.borrow_mut().pop() {
            let current = self.current.borrow().clone();
            self.history
                .borrow_mut()
                .push((current.remote, current.path));
            self.current.borrow_mut().remote = remote;
            self.current.borrow_mut().path = path;
            self.current.borrow_mut().starred = false;
            self.reload();
        }
    }

    fn go_up(&self) {
        if self.current.borrow().starred {
            self.current.borrow_mut().starred = false;
            self.reload();
            return;
        }
        let path = self.current.borrow().path.clone();
        let parent = parent_remote_path(&path);
        self.current.borrow_mut().path = parent;
        self.reload();
    }

    fn reload(&self) {
        clear_list(&self.list);
        clear_flow(&self.grid);
        if self.current.borrow().starred {
            self.populate_starred();
            return;
        }
        let current = self.current.borrow().clone();
        let display = if current.remote == "local" {
            current.path.clone()
        } else {
            format!("{}:{}", current.remote, current.path)
        };
        self.path_entry.set_text(&display);
        self.refresh_crumbs();
        self.sync_current_tab();
        self.refresh_tabs();
        self.reload_ops();

        let Some(client) = self.ctx.client() else {
            self.status.set_text(&self.ctx.t_or(
                "nautilus.errors.connectionFailed",
                "Rclone engine offline — showing empty listing",
            ));
            return;
        };
        let fs = if current.remote == "local" {
            "/".to_string()
        } else {
            remote_fs(&current.remote, "")
        };
        let remote_path = if current.remote == "local" {
            current.path.trim_start_matches('/').to_string()
        } else {
            current.path.clone()
        };
        match client.list_dir(&fs, &remote_path) {
            Ok(mut entries) => {
                let show_hidden = self.ctx.settings.borrow().nautilus.show_hidden;
                if !show_hidden {
                    entries.retain(|e| !e.name.starts_with('.'));
                }
                if let Some(req) = self.ctx.pending_picker.borrow().as_ref() {
                    entries.retain(|e| {
                        e.is_dir || crate::picker::is_entry_allowed(&e.name, e.is_dir, &req.config)
                    });
                    if req.config.selection == crate::picker::PickerSelection::Folders {
                        entries.retain(|e| e.is_dir);
                    }
                }
                let query = self.search_filter.borrow().to_lowercase();
                if !query.is_empty() {
                    entries.retain(|e| e.name.to_lowercase().contains(&query));
                }
                let type_filter = self.ctx.settings.borrow().nautilus.file_type_filter.clone();
                if !type_filter.is_empty() && type_filter != "all" {
                    entries.retain(|e| {
                        crate::mime::category_for_entry(&e.name, e.is_dir, &e.mime)
                            .matches_filter(&type_filter)
                    });
                }
                sort_entries(
                    &mut entries,
                    &self.ctx.settings.borrow().nautilus.sort_by,
                    self.ctx.settings.borrow().nautilus.sort_desc,
                );
                self.status
                    .set_text(&self.listing_count_label(entries.len()));
                self.populate_entries(&entries, true);
                if *self.split_enabled.borrow() {
                    self.reload_pane(&self.secondary.borrow());
                }
            }
            Err(err) => {
                self.status.set_text(&err.to_string());
                self.toast.add_toast(adw::Toast::new(&err.to_string()));
            }
        }
    }

    fn populate_entries(&self, entries: &[DirEntry], primary: bool) {
        if primary {
            *self.last_listing.borrow_mut() = entries.to_vec();
        }
        let tab = if primary {
            self.current.borrow().clone()
        } else {
            self.secondary.borrow().clone()
        };
        if self.is_grid() {
            let grid = if primary {
                &self.grid
            } else {
                &self.grid_right
            };
            for entry in entries {
                grid.insert(&self.entry_tile(entry.clone(), &tab, primary), -1);
            }
        } else {
            let list = if primary {
                &self.list
            } else {
                &self.list_right
            };
            list.append(&self.column_header_row());
            for entry in entries {
                list.append(&self.entry_row(entry.clone(), &tab, primary));
            }
        }
    }

    fn column_header_row(&self) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(&self.ctx.t_or("nautilus.columns.name", "Name"));
        row.set_activatable(false);
        row.set_sensitive(false);
        row.set_widget_name("column-header");
        let size = gtk::Label::new(Some(&self.ctx.t_or("nautilus.columns.size", "Size")));
        size.add_css_class("dim-label");
        size.set_width_chars(10);
        size.set_xalign(1.0);
        let modified = gtk::Label::new(Some(
            &self.ctx.t_or("nautilus.columns.modified", "Modified"),
        ));
        modified.add_css_class("dim-label");
        modified.set_width_chars(18);
        modified.set_xalign(1.0);
        row.add_suffix(&size);
        row.add_suffix(&modified);
        row
    }

    fn entry_row(&self, entry: DirEntry, tab: &TabState, primary: bool) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(&entry.name);
        row.set_widget_name(&entry.name);
        row.set_subtitle(&if entry.is_dir {
            self.ctx.t_or("nautilus.selection.folder", "Folder")
        } else {
            format!(
                "{} · {}",
                format_bytes(entry.size),
                format_relative_mod_time(&entry.mod_time)
            )
        });
        let icon = gtk::Image::from_icon_name(&crate::mime::icon_for_entry(
            &entry.name,
            entry.is_dir,
            &entry.mime,
        ));
        row.add_prefix(&icon);
        let size = gtk::Label::new(Some(&if entry.is_dir {
            String::new()
        } else {
            format_bytes(entry.size)
        }));
        size.add_css_class("dim-label");
        size.set_width_chars(10);
        size.set_xalign(1.0);
        let modified = gtk::Label::new(Some(&format_relative_mod_time(&entry.mod_time)));
        modified.add_css_class("dim-label");
        modified.set_width_chars(18);
        modified.set_xalign(1.0);
        row.add_suffix(&size);
        row.add_suffix(&modified);
        row.set_activatable(true);
        self.attach_item_dnd(&row, &entry, tab, primary);
        row
    }

    fn entry_tile(&self, entry: DirEntry, tab: &TabState, primary: bool) -> gtk::Box {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
        tile.set_halign(gtk::Align::Center);
        tile.set_valign(gtk::Align::Start);
        tile.set_margin_top(8);
        tile.set_margin_bottom(8);
        tile.set_margin_start(8);
        tile.set_margin_end(8);
        tile.set_widget_name(&entry.name);
        let icon = gtk::Image::from_icon_name(&crate::mime::icon_for_entry(
            &entry.name,
            entry.is_dir,
            &entry.mime,
        ));
        icon.set_pixel_size(self.current_icon_size().max(48));
        let label = gtk::Label::new(Some(&entry.name));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(14);
        label.set_justify(gtk::Justification::Center);
        label.set_wrap(true);
        tile.append(&icon);
        tile.append(&label);
        let caption = crate::rclone::listing_caption(entry.is_dir, entry.size, &entry.mod_time);
        if !caption.is_empty() {
            let meta = gtk::Label::new(Some(&caption));
            meta.add_css_class("dim-label");
            meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
            meta.set_max_width_chars(14);
            meta.set_justify(gtk::Justification::Center);
            tile.append(&meta);
        }
        self.attach_item_dnd(&tile, &entry, tab, primary);
        tile
    }

    fn selected_name(&self) -> Option<String> {
        self.selected_names().into_iter().next()
    }

    fn selected_names(&self) -> Vec<String> {
        if self.is_grid() {
            self.grid
                .selected_children()
                .into_iter()
                .filter_map(|child| flow_child_name(&child))
                .collect()
        } else {
            let mut names = Vec::new();
            let mut child = self.list.first_child();
            while let Some(widget) = child {
                if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>() {
                    if row.is_selected() {
                        if let Some(name) = row_name(&row) {
                            names.push(name);
                        }
                    }
                }
                child = widget.next_sibling();
            }
            names
        }
    }

    fn open_name_in_pane(&self, name: &str, primary: bool) {
        if primary {
            self.open_name(name);
            return;
        }
        let tab = self.secondary.borrow().clone();
        let next = join_remote_path(&tab.path, name);
        let Some(client) = self.ctx.client() else {
            return;
        };
        let fs = if tab.remote == "local" {
            "/".into()
        } else {
            remote_fs(&tab.remote, "")
        };
        let list_path = if tab.remote == "local" {
            next.trim_start_matches('/').to_string()
        } else {
            next.clone()
        };
        if client.list_dir(&fs, &list_path).is_ok() {
            self.secondary.borrow_mut().path = next;
            self.reload_pane(&self.secondary.borrow());
            return;
        }
        if let Some(win) = self.root.root().and_downcast::<gtk::Window>() {
            let is_dir = self
                .last_listing
                .borrow()
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.is_dir)
                .unwrap_or(false);
            dialogs::file_viewer(
                &win,
                self.ctx.clone(),
                &tab.remote,
                &next,
                name,
                is_dir,
                &[],
            );
        }
    }

    fn open_name(&self, name: &str) {
        if self.current.borrow().starred {
            if let Some(entry) = self
                .last_listing
                .borrow()
                .iter()
                .find(|e| e.name == name)
                .cloned()
            {
                self.current.borrow_mut().starred = false;
                self.navigate_to(&entry.path);
                return;
            }
        }
        if self.ctx.pending_picker.borrow().is_some() {
            let listing = self.last_listing.borrow().clone();
            if let Some(entry) = listing.iter().find(|e| e.name == name) {
                if !entry.is_dir {
                    self.finish_picker_choice(false, Some((name.to_string(), false)));
                    return;
                }
            }
        }
        let current = self.current.borrow().clone();
        let next = join_remote_path(&current.path, name);
        let Some(client) = self.ctx.client() else {
            return;
        };
        let fs = if current.remote == "local" {
            "/".into()
        } else {
            remote_fs(&current.remote, "")
        };
        let list_path = if current.remote == "local" {
            next.trim_start_matches('/').to_string()
        } else {
            next.clone()
        };
        if client.list_dir(&fs, &list_path).is_ok() {
            self.current.borrow_mut().path = next;
            self.reload();
            return;
        }
        self.open_viewer(name);
    }

    fn open_viewer(&self, name: &str) {
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, name);
        if let Some(win) = self.root.root().and_downcast::<gtk::Window>() {
            let siblings: Vec<(String, bool)> = self
                .last_listing
                .borrow()
                .iter()
                .map(|e| (e.name.clone(), e.is_dir))
                .collect();
            let is_dir = siblings
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, d)| *d)
                .unwrap_or(false);
            dialogs::file_viewer(
                &win,
                self.ctx.clone(),
                &current.remote,
                &path,
                name,
                is_dir,
                &siblings,
            );
        }
    }

    fn mkdir_with_selected(&self) {
        let names = self.selected_names();
        if names.is_empty() {
            self.toast
                .add_toast(adw::Toast::new("Select items to move into a new folder"));
            return;
        }
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let view = self.clone();
        dialogs::prompt(
            &win,
            &self.ctx,
            &self
                .ctx
                .t_or("nautilus.contextMenu.newFolder", "New folder with selected"),
            &self.ctx.t_or("common.name", "Folder name"),
            "",
            move |name| {
                if name.is_empty() {
                    return;
                }
                let Some(client) = view.ctx.client() else {
                    return;
                };
                let current = view.current.borrow().clone();
                let fs = if current.remote == "local" {
                    "/".into()
                } else {
                    remote_fs(&current.remote, "")
                };
                let folder = join_remote_path(&current.path, &name);
                let folder_remote = if current.remote == "local" {
                    folder.trim_start_matches('/').to_string()
                } else {
                    folder
                };
                if let Err(e) = client.mkdir(&fs, &folder_remote) {
                    view.toast.add_toast(adw::Toast::new(&e.to_string()));
                    return;
                }
                view.push_undo(
                    crate::fileops::FileOp::Mkdir {
                        fs: fs.clone(),
                        path: folder_remote.clone(),
                    }
                    .encode(),
                );
                for item in &names {
                    let src = join_remote_path(&current.path, item);
                    let src_remote = if current.remote == "local" {
                        src.trim_start_matches('/').to_string()
                    } else {
                        src
                    };
                    let dst_remote = join_remote_path(&folder_remote, item);
                    match client.move_file(&fs, &src_remote, &fs, &dst_remote) {
                        Ok(_) => view.push_undo(
                            crate::fileops::FileOp::Move {
                                src_fs: fs.clone(),
                                src: src_remote,
                                dst_fs: fs.clone(),
                                dst: dst_remote,
                            }
                            .encode(),
                        ),
                        Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                    }
                }
                view.reload();
                view.toast.add_toast(adw::Toast::new(&format!(
                    "Moved {} items into {name}",
                    names.len()
                )));
            },
        );
    }

    fn download_selected(&self) {
        let Some(name) = self.selected_name() else {
            self.toast
                .add_toast(adw::Toast::new("Select a file to download"));
            return;
        };
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        dialogs::download_file(&win, self.ctx.clone(), &current.remote, &path, &name);
    }

    fn mkdir_prompt(&self) {
        if let Some(win) = self.root.root().and_downcast::<gtk::Window>() {
            let view = self.clone();
            dialogs::prompt(
                &win,
                &self.ctx,
                &self
                    .ctx
                    .t_or("nautilus.contextMenu.newFolder", "New folder"),
                &self.ctx.t_or("common.name", "Folder name"),
                "",
                move |name| {
                    if name.is_empty() {
                        return;
                    }
                    if let Some(client) = view.ctx.client() {
                        let current = view.current.borrow().clone();
                        let fs = if current.remote == "local" {
                            "/".into()
                        } else {
                            remote_fs(&current.remote, "")
                        };
                        let path = join_remote_path(&current.path, &name);
                        let remote = if current.remote == "local" {
                            path.trim_start_matches('/').to_string()
                        } else {
                            path
                        };
                        match client.mkdir(&fs, &remote) {
                            Ok(_) => {
                                view.push_undo(
                                    crate::fileops::FileOp::Mkdir { fs, path: remote }.encode(),
                                );
                                view.reload();
                            }
                            Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                        }
                    }
                },
            );
        }
    }

    fn upload_prompt(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::new();
        let view = self.clone();
        dialog.open_multiple(
            Some(&win),
            None::<gio::Cancellable>.as_ref(),
            move |result| {
                if let Ok(files) = result {
                    for i in 0..files.n_items() {
                        if let Some(file) =
                            files.item(i).and_then(|o| o.downcast::<gio::File>().ok())
                        {
                            if let Some(path) = file.path() {
                                view.upload_local_path(&path);
                            }
                        }
                    }
                }
            },
        );
    }

    fn upload_local_path(&self, path: &std::path::Path) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upload")
            .to_string();
        let dst = join_remote_path(&current.path, &name);
        let dst_fs = if current.remote == "local" {
            "/".into()
        } else {
            remote_fs(&current.remote, "")
        };
        let dst_remote = if current.remote == "local" {
            dst.trim_start_matches('/').to_string()
        } else {
            dst
        };
        match client.start_job(
            "operations/copyfile",
            serde_json::json!({
                "srcFs": "/",
                "srcRemote": path.to_string_lossy(),
                "dstFs": dst_fs,
                "dstRemote": dst_remote
            }),
        ) {
            Ok(id) => {
                let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                let preparing = crate::jobs::preparing_job(
                    id,
                    &current.remote,
                    &path.to_string_lossy(),
                    &dst_remote,
                    1,
                    bytes,
                );
                let transferring = serde_json::json!([{
                    "name": name,
                    "size": bytes,
                    "bytes": 0
                }]);
                self.ctx.store.borrow_mut().remember_job(preparing.clone());
                self.ctx.store.borrow_mut().update_job_stats(
                    id,
                    crate::jobs::preparing_progress_stats(0, bytes, 0, 1, transferring),
                );
                if let Some(job) = self
                    .ctx
                    .store
                    .borrow()
                    .job_history
                    .iter()
                    .find(|j| j.id == id)
                {
                    self.ctx.snapshot.borrow_mut().jobs.insert(0, job.clone());
                } else {
                    self.ctx.snapshot.borrow_mut().jobs.insert(0, preparing);
                }
                self.push_undo(
                    crate::fileops::FileOp::Upload {
                        fs: dst_fs,
                        path: dst_remote,
                    }
                    .encode(),
                );
                self.ctx.refresh_runtime();
                self.reload();
                self.toast
                    .add_toast(adw::Toast::new(&format!("Upload job #{id} · {name}")));
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn switch_pane(&self) {
        if !*self.split_enabled.borrow() {
            return;
        }
        let left = self.current.borrow().clone();
        let right = self.secondary.borrow().clone();
        *self.current.borrow_mut() = right;
        *self.secondary.borrow_mut() = left;
        self.reload();
        self.status.set_text(
            &self
                .ctx
                .t_or("nautilus.contextMenu.switchPane", "Switched split pane"),
        );
    }

    fn toggle_split(&self) {
        let next = !*self.split_enabled.borrow();
        *self.split_enabled.borrow_mut() = next;
        self.ctx.settings.borrow_mut().nautilus.split_enabled = next;
        self.ctx.persist();
        self.right_scroll.set_visible(next);
        if next {
            *self.secondary.borrow_mut() = self.current.borrow().clone();
            self.reload();
            self.status.set_text(
                &self
                    .ctx
                    .t_or("nautilus.splitOn", "Split view — two listings"),
            );
        } else {
            self.status
                .set_text(&self.ctx.t_or("nautilus.splitOff", "Split view off"));
        }
        let _ = &self.paned;
    }

    fn reload_pane(&self, tab: &TabState) {
        clear_list(&self.list_right);
        clear_flow(&self.grid_right);
        let Some(client) = self.ctx.client() else {
            return;
        };
        let fs = if tab.remote == "local" {
            "/".to_string()
        } else {
            remote_fs(&tab.remote, "")
        };
        let remote_path = if tab.remote == "local" {
            tab.path.trim_start_matches('/').to_string()
        } else {
            tab.path.clone()
        };
        if let Ok(mut entries) = client.list_dir(&fs, &remote_path) {
            if !self.ctx.settings.borrow().nautilus.show_hidden {
                entries.retain(|e| !e.name.starts_with('.'));
            }
            let type_filter = self.ctx.settings.borrow().nautilus.file_type_filter.clone();
            if !type_filter.is_empty() && type_filter != "all" {
                entries.retain(|e| {
                    crate::mime::category_for_entry(&e.name, e.is_dir, &e.mime)
                        .matches_filter(&type_filter)
                });
            }
            self.populate_entries(&entries, false);
        }
    }

    fn push_undo(&self, op: String) {
        self.undo.borrow_mut().push(op);
        self.redo.borrow_mut().clear();
    }

    fn undo_last(&self) {
        let Some(op) = self.undo.borrow_mut().pop() else {
            self.toast.add_toast(adw::Toast::new("Nothing to undo"));
            return;
        };
        self.invert_file_op(&op);
        self.redo.borrow_mut().push(op);
        self.toast.add_toast(adw::Toast::new("Undid last action"));
    }

    fn redo_last(&self) {
        let Some(op) = self.redo.borrow_mut().pop() else {
            self.toast.add_toast(adw::Toast::new("Nothing to redo"));
            return;
        };
        self.replay_file_op(&op);
        self.undo.borrow_mut().push(op);
        self.toast.add_toast(adw::Toast::new("Redid last action"));
    }

    fn invert_file_op(&self, op: &str) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        let Some(decoded) = crate::fileops::FileOp::decode(op) else {
            return;
        };
        match decoded.invert() {
            Some(inv) => {
                if let Err(e) = inv.apply(&client) {
                    self.toast.add_toast(adw::Toast::new(&e));
                }
            }
            None => self
                .toast
                .add_toast(adw::Toast::new("This action cannot be undone")),
        }
        self.reload();
    }

    fn replay_file_op(&self, op: &str) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        if let Some(decoded) = crate::fileops::FileOp::decode(op) {
            if let Err(e) = decoded.apply(&client) {
                self.toast.add_toast(adw::Toast::new(&e));
            }
            self.reload();
        }
    }

    fn paste_system_clipboard(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let clipboard = display.clipboard();
        let view = self.clone();
        clipboard.read_value_async(
            gtk::gdk::FileList::static_type(),
            glib::Priority::DEFAULT,
            None::<gio::Cancellable>.as_ref(),
            {
                let view = view.clone();
                move |result| {
                    if let Ok(value) = result {
                        if let Ok(list) = value.get::<gtk::gdk::FileList>() {
                            for file in list.files() {
                                if let Some(path) = file.path() {
                                    if path.is_dir() {
                                        view.upload_local_tree(&path);
                                    } else if path.is_file() {
                                        view.upload_local_path(&path);
                                    }
                                }
                            }
                            return;
                        }
                    }
                    view.paste_clipboard_text();
                }
            },
        );
    }

    fn paste_clipboard_text(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let clipboard = display.clipboard();
        let view = self.clone();
        clipboard.read_text_async(None::<gio::Cancellable>.as_ref(), move |result| {
            let Ok(Some(text)) = result else {
                return;
            };
            for line in text.lines() {
                let raw = line.trim();
                if raw.is_empty() {
                    continue;
                }
                let path = if let Some(rest) = raw.strip_prefix("file://") {
                    std::path::PathBuf::from(
                        urlencoding::decode(rest)
                            .unwrap_or(std::borrow::Cow::Borrowed(rest))
                            .as_ref(),
                    )
                } else {
                    std::path::PathBuf::from(raw)
                };
                if path.is_dir() {
                    view.upload_local_tree(&path);
                } else if path.is_file() {
                    view.upload_local_path(&path);
                }
            }
        });
    }

    fn properties_selected(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let current = self.current.borrow().clone();
        if let Some(name) = self.selected_name() {
            let path = join_remote_path(&current.path, &name);
            dialogs::properties(&win, self.ctx.clone(), &current.remote, &path, &name);
        } else {
            let name = current
                .path
                .rsplit(['/', ':'])
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(&current.remote)
                .to_string();
            dialogs::properties(
                &win,
                self.ctx.clone(),
                &current.remote,
                &current.path,
                &name,
            );
        }
    }

    fn select_all(&self) {
        if self.is_grid() {
            self.grid.select_all();
            if *self.split_enabled.borrow() {
                self.grid_right.select_all();
            }
        } else {
            self.list.select_all();
            if *self.split_enabled.borrow() {
                self.list_right.select_all();
            }
        }
    }

    fn copy_current_location(&self) {
        self.copy_text(&self.formatted_path(None));
    }

    fn toggle_send_to(&self) {
        let current = self.current.borrow().clone();
        let registered =
            crate::platform::is_send_to_registered(&current.remote, Some(&current.path));
        let result = if registered {
            crate::platform::unregister_send_to(&current.remote, Some(&current.path))
        } else {
            crate::platform::register_send_to(&current.remote, Some(&current.path))
        };
        match result {
            Ok(_) => self.toast.add_toast(adw::Toast::new(if registered {
                "Removed Send to shortcut"
            } else {
                "Added Send to shortcut"
            })),
            Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
        }
    }

    fn rename_selected(&self) {
        let names = self.selected_names();
        if names.is_empty() {
            return;
        }
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        if names.len() > 1 {
            let current = self.current.borrow().clone();
            let view = self.clone();
            dialogs::multi_rename(
                &win,
                self.ctx.clone(),
                &current.remote,
                &current.path,
                names,
                Rc::new(move || view.reload()),
            );
            return;
        }
        let old = names[0].clone();
        let view = self.clone();
        dialogs::prompt(
            &win,
            &self.ctx,
            &self.ctx.t_or("nautilus.contextMenu.rename", "Rename"),
            &self.ctx.t_or("modals.remoteConfig.newName", "New name"),
            &old.clone(),
            move |new_name| {
                if new_name.is_empty() || new_name == old {
                    return;
                }
                if let Some(client) = view.ctx.client() {
                    let current = view.current.borrow().clone();
                    let fs = if current.remote == "local" {
                        "/".into()
                    } else {
                        remote_fs(&current.remote, "")
                    };
                    let src = join_remote_path(&current.path, &old);
                    let dst = join_remote_path(&current.path, &new_name);
                    let src = if current.remote == "local" {
                        src.trim_start_matches('/').to_string()
                    } else {
                        src
                    };
                    let dst = if current.remote == "local" {
                        dst.trim_start_matches('/').to_string()
                    } else {
                        dst
                    };
                    match client.move_file(&fs, &src, &fs, &dst) {
                        Ok(_) => {
                            view.push_undo(
                                crate::fileops::FileOp::Rename {
                                    fs,
                                    from: src,
                                    to: dst,
                                }
                                .encode(),
                            );
                            view.reload();
                        }
                        Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                    }
                }
            },
        );
    }

    fn delete_selected(&self) {
        let names = self.selected_names();
        if names.is_empty() {
            return;
        }
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        for name in names {
            let path = join_remote_path(&current.path, &name);
            let (fs, remote) = fs_remote(&current.remote, &path);
            let trash = if current.remote == "local" {
                let local = if path.starts_with('/') {
                    path.clone()
                } else {
                    format!("/{remote}")
                };
                crate::fileops::stash_local_path(&local)
            } else {
                None
            };
            if client.delete_file(&fs, &remote).is_err() {
                let _ = client.purge(&fs, &remote);
            }
            self.push_undo(
                crate::fileops::FileOp::Delete {
                    fs,
                    path: remote,
                    trash,
                }
                .encode(),
            );
        }
        self.reload();
    }

    fn cut_or_copy(&self, cut: bool) {
        let names = self.selected_names();
        if names.is_empty() {
            return;
        }
        let current = self.current.borrow().clone();
        let items = names
            .into_iter()
            .map(|name| {
                (
                    current.remote.clone(),
                    join_remote_path(&current.path, &name),
                    cut,
                )
            })
            .collect();
        *self.clipboard.borrow_mut() = items;
        self.toast
            .add_toast(adw::Toast::new(if cut { "Cut" } else { "Copied" }));
    }

    fn paste(&self) {
        let items = self.clipboard.borrow().clone();
        if items.is_empty() {
            self.paste_system_clipboard();
            return;
        }
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let transfers: Vec<crate::fileops::TransferItem> = items
            .into_iter()
            .map(|(src_remote, src_path, cut)| {
                let name = src_path.rsplit('/').next().unwrap_or(&src_path).to_string();
                let dst_path = join_remote_path(&current.path, &name);
                let (src_fs, src_remote_path) = fs_remote(&src_remote, &src_path);
                let (dst_fs, dst_remote_path) = fs_remote(&current.remote, &dst_path);
                crate::fileops::TransferItem {
                    src_fs,
                    src: src_remote_path,
                    dst_fs,
                    dst: dst_remote_path,
                    cut,
                }
            })
            .collect();
        match crate::fileops::start_grouped_transfers(&client, &transfers, "filemanager") {
            Ok((group, ids)) => {
                for item in &transfers {
                    self.push_undo(item.file_op().encode());
                }
                self.ctx.refresh_runtime();
                self.reload();
                self.toast.add_toast(adw::Toast::new(&format!(
                    "Transfer group {group} · {} job(s)",
                    ids.len()
                )));
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
        }
    }

    pub fn add_bookmark(&self) {
        let current = self.current.borrow().clone();
        let path = if current.remote == "local" {
            current.path
        } else {
            format!("{}:{}", current.remote, current.path)
        };
        self.ctx
            .settings
            .borrow_mut()
            .nautilus
            .bookmarks
            .push(serde_json::json!({
                "name": path.rsplit('/').next().unwrap_or("bookmark"),
                "path": path
            }));
        self.ctx.persist();
        self.reload_sidebar();
        let _ = Bookmark::default();
    }

    fn sync_current_tab(&self) {
        let current = self.current.borrow().clone();
        if let Some(tab) = self
            .tabs
            .borrow_mut()
            .iter_mut()
            .find(|t| t.id == current.id)
        {
            tab.remote = current.remote.clone();
            tab.path = current.path.clone();
            tab.starred = current.starred;
            tab.title = if current.starred {
                self.ctx.t_or("nautilus.titles.starred", "Starred")
            } else if current.path.is_empty() {
                current.remote.clone()
            } else {
                current
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&current.remote)
                    .to_string()
            };
        }
    }

    fn refresh_tabs(&self) {
        while let Some(child) = self.tab_bar.first_child() {
            self.tab_bar.remove(&child);
        }
        let current_id = self.current.borrow().id;
        for tab in self.tabs.borrow().iter() {
            let btn = gtk::ToggleButton::with_label(&tab.title);
            btn.set_active(tab.id == current_id);
            let view = self.clone();
            let id = tab.id;
            btn.connect_clicked(move |_| view.activate_tab(id));
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            {
                let view = self.clone();
                let id = tab.id;
                gesture.connect_pressed(move |g, _, _, _| {
                    view.popup_tab_menu(id);
                    g.set_state(gtk::EventSequenceState::Claimed);
                });
            }
            btn.add_controller(gesture);
            self.attach_internal_drop(&btn, InternalDrop::Tab(id));
            self.tab_bar.append(&btn);
        }
    }

    fn popup_tab_menu(&self, id: u32) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        self.activate_tab(id);
        let popover = gtk::Popover::new();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let items = [
            (
                self.ctx
                    .t_or("nautilus.contextMenu.duplicateTab", "Duplicate Tab"),
                "dup",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.closeTab", "Close Tab"),
                "close",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.closeOtherTabs", "Close Other Tabs"),
                "others",
            ),
            (
                self.ctx.t_or(
                    "nautilus.contextMenu.closeTabsToRight",
                    "Close Tabs to the Right",
                ),
                "right",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.detachTab", "Detach Tab"),
                "detach",
            ),
        ];
        for (label, action) in items {
            let btn = gtk::Button::with_label(&label);
            let view = self.clone();
            let popover = popover.clone();
            btn.connect_clicked(move |_| {
                match action {
                    "dup" => view.duplicate_tab(id),
                    "close" => {
                        view.activate_tab(id);
                        view.close_current_tab();
                    }
                    "others" => view.close_other_tabs(id),
                    "right" => view.close_tabs_to_right(id),
                    "detach" => {
                        view.activate_tab(id);
                        view.detach_current_tab();
                    }
                    _ => {}
                }
                popover.popdown();
            });
            box_.append(&btn);
        }
        popover.set_child(Some(&box_));
        popover.set_parent(&win);
        popover.popup();
    }

    fn duplicate_tab(&self, id: u32) {
        let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == id).cloned() else {
            return;
        };
        let new_id = *self.next_tab_id.borrow();
        *self.next_tab_id.borrow_mut() = new_id + 1;
        let mut copy = tab;
        copy.id = new_id;
        self.tabs.borrow_mut().push(copy.clone());
        *self.current.borrow_mut() = copy;
        self.reload();
    }

    fn close_other_tabs(&self, keep: u32) {
        self.tabs.borrow_mut().retain(|t| t.id == keep);
        if let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == keep).cloned() {
            *self.current.borrow_mut() = tab;
        }
        if self.tabs.borrow().is_empty() {
            self.open_new_tab();
            return;
        }
        self.reload();
    }

    fn close_tabs_to_right(&self, id: u32) {
        let idx = self
            .tabs
            .borrow()
            .iter()
            .position(|t| t.id == id)
            .unwrap_or(0);
        self.tabs.borrow_mut().truncate(idx + 1);
        if self.current.borrow().id != id {
            if let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == id).cloned() {
                *self.current.borrow_mut() = tab;
            }
        }
        self.reload();
    }

    fn activate_tab(&self, id: u32) {
        if let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == id).cloned() {
            *self.current.borrow_mut() = tab;
            self.reload();
        }
    }

    fn cycle_tab(&self, reverse: bool) {
        let tabs = self.tabs.borrow();
        if tabs.len() < 2 {
            return;
        }
        let Some(idx) = tabs.iter().position(|t| t.id == self.current.borrow().id) else {
            return;
        };
        let next = if reverse {
            if idx == 0 {
                tabs.len() - 1
            } else {
                idx - 1
            }
        } else {
            (idx + 1) % tabs.len()
        };
        let id = tabs[next].id;
        drop(tabs);
        self.activate_tab(id);
    }

    fn open_new_tab(&self) {
        let mut current = self.current.borrow().clone();
        let id = *self.next_tab_id.borrow();
        *self.next_tab_id.borrow_mut() = id + 1;
        current.id = id;
        current.title = format!("{}:{}", current.remote, current.path)
            .rsplit('/')
            .next()
            .unwrap_or("tab")
            .to_string();
        self.tabs.borrow_mut().push(current.clone());
        *self.current.borrow_mut() = current;
        self.reload();
    }

    fn close_current_tab(&self) {
        let id = self.current.borrow().id;
        self.tabs.borrow_mut().retain(|t| t.id != id);
        if self.tabs.borrow().is_empty() {
            self.open_new_tab();
            return;
        }
        if let Some(next) = self.tabs.borrow().first().cloned() {
            *self.current.borrow_mut() = next;
        }
        self.reload();
    }

    pub fn detach_current_tab(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(app) = win.application() else {
            return;
        };
        let current = self.current.borrow().clone();
        let toast = adw::ToastOverlay::new();
        let view = NautilusView::new(self.ctx.clone(), toast.clone());
        toast.set_child(Some(&view.root));
        let detached = adw::ApplicationWindow::new(&app);
        detached.set_title(Some(&format!("Files — {}", current.title)));
        detached.set_default_width(960);
        detached.set_default_height(640);
        detached.set_content(Some(&toast));
        let target = if current.remote == "local" {
            current.path.clone()
        } else if current.path.is_empty() {
            format!("{}:", current.remote)
        } else {
            format!("{}:{}", current.remote, current.path)
        };
        view.navigate_to(&target);
        detached.present();
        if self.tabs.borrow().len() > 1 {
            self.close_current_tab();
        }
    }

    fn popup_context(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let popover = gtk::Popover::new();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let selected = self.selected_names();
        let current = self.current.borrow().clone();
        let info = self.ctx.fs_info(&current.remote);
        let public_ok =
            info.as_ref().is_none_or(|i| i.has_feature("PublicLink")) && !selected.is_empty();
        let cleanup_ok = info.as_ref().is_none_or(|i| i.has_feature("CleanUp"));
        let archive_selected = selected.iter().any(|name| {
            matches!(
                crate::mime::category_for_entry(name, false, ""),
                FileTypeCategory::Archive
            )
        });
        let rename_label = if selected.len() > 1 {
            self.ctx
                .t_or("nautilus.contextMenu.renameMultiple", "Rename Multiple...")
        } else {
            self.ctx.t_or("nautilus.contextMenu.rename", "Rename")
        };
        let send_label = {
            let registered =
                crate::platform::is_send_to_registered(&current.remote, Some(&current.path));
            if registered {
                self.ctx.t_or(
                    "nautilus.contextMenu.removeFromSendTo",
                    "Remove from File Manager Menu",
                )
            } else {
                self.ctx.t_or(
                    "nautilus.contextMenu.addToSendTo",
                    "Add to File Manager Menu",
                )
            }
        };
        let mut items: Vec<(String, &str)> = vec![
            (self.ctx.t_or("nautilus.contextMenu.open", "Open"), "open"),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.openNative", "Open native"),
                "native",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.openNewTab", "Open in New Tab"),
                "tab",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.openNewWindow", "Open in New Window"),
                "window",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.refresh", "Refresh"),
                "reload",
            ),
            (self.ctx.t_or("nautilus.contextMenu.copy", "Copy"), "copy"),
            (self.ctx.t_or("nautilus.contextMenu.cut", "Cut"), "cut"),
            (
                self.ctx.t_or("nautilus.contextMenu.paste", "Paste"),
                "paste",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.copyPath", "Copy Path"),
                "copypath",
            ),
        ];
        if public_ok {
            items.push((
                self.ctx
                    .t_or("nautilus.contextMenu.copyPublicLink", "Copy Public Link"),
                "public",
            ));
        }
        items.push((
            self.ctx
                .t_or("nautilus.contextMenu.copyUrl", "Copy URL into folder…"),
            "copyurl",
        ));
        items.push((rename_label, "rename"));
        items.extend([
            (
                self.ctx.t_or("nautilus.contextMenu.delete", "Delete"),
                "delete",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.properties", "Properties"),
                "props",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.download", "Download…"),
                "download",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.newFolder", "New Folder"),
                "mkdir",
            ),
        ]);
        if !selected.is_empty() {
            items.push((
                self.ctx.t_or(
                    "nautilus.contextMenu.createFolderWithItems",
                    "New Folder with Selection...",
                ),
                "mkdirsel",
            ));
        }
        items.extend([
            (
                self.ctx
                    .t_or("nautilus.contextMenu.uploadFolder", "Upload Folder"),
                "uploaddir",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.star", "Star / Unstar"),
                "togglestar",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.bookmark", "Bookmark"),
                "star",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.compress", "Compress"),
                "archive",
            ),
        ]);
        if archive_selected {
            items.push((
                self.ctx.t_or(
                    "nautilus.contextMenu.browseArchive",
                    "Browse archive contents",
                ),
                "archivelist",
            ));
            items.push((
                self.ctx
                    .t_or("nautilus.contextMenu.extract", "Extract archive…"),
                "extract",
            ));
        }
        items.push((
            self.ctx.t_or(
                "nautilus.contextMenu.removeEmptyDirs",
                "Remove empty folders",
            ),
            "rmdirs",
        ));
        if cleanup_ok {
            items.push((
                self.ctx
                    .t_or("nautilus.contextMenu.emptyTrash", "Empty Trash"),
                "cleanup",
            ));
        }
        items.extend([
            (send_label, "sendto"),
            (
                self.ctx.t_or("nautilus.contextMenu.share", "Share"),
                "share",
            ),
            (self.ctx.t_or("nautilus.contextMenu.undo", "Undo"), "undo"),
            (self.ctx.t_or("nautilus.contextMenu.redo", "Redo"), "redo"),
            (
                self.ctx.t_or(
                    "nautilus.contextMenu.pasteSystem",
                    "Paste from system clipboard",
                ),
                "syspaste",
            ),
            (
                self.ctx
                    .t_or("nautilus.contextMenu.detachTab", "Detach Tab"),
                "detach",
            ),
        ]);
        for (label, action) in items {
            let btn = gtk::Button::with_label(&label);
            let view = self.clone();
            btn.connect_clicked(move |_| match action {
                "open" => {
                    if let Some(name) = view.selected_name() {
                        view.open_name(&name);
                    }
                }
                "native" => view.open_native_selected(),
                "tab" => view.open_selected_in_new_tab(),
                "window" => view.open_selected_in_new_window(),
                "reload" => view.reload(),
                "copy" => view.cut_or_copy(false),
                "cut" => view.cut_or_copy(true),
                "paste" => view.paste(),
                "copypath" => view.copy_selected_path(),
                "public" => view.copy_public_link(),
                "copyurl" => view.copy_url_prompt(),
                "rename" => view.rename_selected(),
                "delete" => view.delete_selected(),
                "props" => view.properties_selected(),
                "mkdir" => view.mkdir_prompt(),
                "mkdirsel" => view.mkdir_with_selected(),
                "download" => view.download_selected(),
                "archivelist" => {
                    if let Some(name) = view.selected_name() {
                        view.open_name(&name);
                    }
                }
                "uploaddir" => view.upload_folder_prompt(),
                "togglestar" => view.toggle_star_selected(),
                "star" => view.add_bookmark(),
                "extract" => view.extract_selected(),
                "rmdirs" => view.remove_empty_dirs(),
                "cleanup" => view.cleanup_remote(),
                "archive" => {
                    if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                        let current = view.current.borrow().clone();
                        let names = view.selected_names();
                        if names.is_empty() {
                            view.toast.add_toast(adw::Toast::new(
                                &view.ctx.t_or(
                                    "nautilus.errors.minSelection",
                                    "Select items to archive",
                                ),
                            ));
                        } else {
                            dialogs::archive_create(
                                &win,
                                view.ctx.clone(),
                                &current.remote,
                                &current.path,
                                &names,
                            );
                        }
                    }
                }
                "share" => view.share_selected(),
                "sendto" => view.toggle_send_to(),
                "undo" => view.undo_last(),
                "redo" => view.redo_last(),
                "syspaste" => view.paste_system_clipboard(),
                "detach" => view.detach_current_tab(),
                _ => {}
            });
            box_.append(&btn);
        }
        popover.set_child(Some(&box_));
        popover.set_parent(&win);
        popover.popup();
    }

    fn formatted_path(&self, name: Option<&str>) -> String {
        let current = self.current.borrow().clone();
        let path = match name {
            Some(name) => join_remote_path(&current.path, name),
            None => current.path.clone(),
        };
        if current.remote == "local" {
            path
        } else if path.is_empty() {
            format!("{}:", current.remote)
        } else {
            format!("{}:{}", current.remote, path)
        }
    }

    fn copy_text(&self, text: &str) {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(text);
            self.toast.add_toast(adw::Toast::new("Copied to clipboard"));
        }
    }

    fn copy_selected_path(&self) {
        let text = self.formatted_path(self.selected_name().as_deref());
        self.copy_text(&text);
    }

    fn copy_public_link(&self) {
        let Some(name) = self.selected_name() else {
            self.toast
                .add_toast(adw::Toast::new("Select a file to share"));
            return;
        };
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        let (fs, remote) = fs_remote(&current.remote, &path);
        match client.public_link(&fs, &remote) {
            Ok(url) if !url.is_empty() => self.copy_text(&url),
            Ok(_) => self
                .toast
                .add_toast(adw::Toast::new("Remote did not return a public link")),
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn copy_url_prompt(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let current = self.current.borrow().clone();
        let view = self.clone();
        dialogs::copy_url_into(
            &win,
            self.ctx.clone(),
            &current.remote,
            &current.path,
            Rc::new(move || {
                view.reload();
                view.toast.add_toast(adw::Toast::new(
                    &view
                        .ctx
                        .t_or("nautilus.notifications.copyUrlStarted", "Started URL copy"),
                ));
            }),
        );
    }

    fn upload_folder_prompt(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let dialog = gtk::FileDialog::new();
        let view = self.clone();
        dialog.select_folder(
            Some(&win),
            None::<gio::Cancellable>.as_ref(),
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        view.upload_local_tree(&path);
                    }
                }
            },
        );
    }

    fn upload_local_tree(&self, root: &std::path::Path) {
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upload")
            .to_string();
        let current = self.current.borrow().clone();
        let dest = join_remote_path(&current.path, &name);
        match self.upload_tree_into(root, &dest) {
            Ok(count) => {
                self.reload();
                self.toast
                    .add_toast(adw::Toast::new(&format!("Uploaded {count} items")));
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
        }
    }

    fn upload_tree_into(&self, local: &std::path::Path, dest_dir: &str) -> Result<usize, String> {
        let Some(client) = self.ctx.client() else {
            return Err("Engine offline".into());
        };
        let current = self.current.borrow().clone();
        let mut items = Vec::new();
        self.collect_upload_items(&client, &current.remote, local, dest_dir, &mut items)?;
        if items.is_empty() {
            return Ok(0);
        }
        let (_, ids) =
            crate::fileops::start_grouped_transfers(&client, &items, "filemanager-upload")?;
        if let Some(id) = ids.first().copied() {
            let bytes: u64 = items
                .iter()
                .map(|item| std::fs::metadata(&item.src).map(|m| m.len()).unwrap_or(0))
                .sum();
            let preparing = crate::jobs::preparing_job(
                id,
                &current.remote,
                &local.to_string_lossy(),
                dest_dir,
                items.len() as u64,
                bytes,
            );
            let transferring = serde_json::json!(items
                .iter()
                .map(|item| {
                    let size = std::fs::metadata(&item.src).map(|m| m.len()).unwrap_or(0);
                    serde_json::json!({
                        "name": item.src,
                        "size": size,
                        "bytes": 0
                    })
                })
                .collect::<Vec<_>>());
            self.ctx.store.borrow_mut().remember_job(preparing.clone());
            self.ctx.store.borrow_mut().update_job_stats(
                id,
                crate::jobs::preparing_progress_stats(
                    0,
                    bytes,
                    0,
                    items.len() as u64,
                    transferring,
                ),
            );
            if let Some(job) = self
                .ctx
                .store
                .borrow()
                .job_history
                .iter()
                .find(|j| j.id == id)
            {
                self.ctx.snapshot.borrow_mut().jobs.insert(0, job.clone());
            } else {
                self.ctx.snapshot.borrow_mut().jobs.insert(0, preparing);
            }
        }
        self.ctx.refresh_runtime();
        Ok(ids.len())
    }

    fn collect_upload_items(
        &self,
        client: &crate::rclone::RcClient,
        remote: &str,
        local: &std::path::Path,
        dest_dir: &str,
        items: &mut Vec<crate::fileops::TransferItem>,
    ) -> Result<(), String> {
        let (fs, remote_path) = fs_remote(remote, dest_dir);
        let _ = client.mkdir(&fs, &remote_path);
        let entries = std::fs::read_dir(local).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("item")
                .to_string();
            let dest = join_remote_path(dest_dir, &name);
            if path.is_dir() {
                self.collect_upload_items(client, remote, &path, &dest, items)?;
            } else {
                let (dst_fs, dst_remote) = fs_remote(remote, &dest);
                items.push(crate::fileops::TransferItem {
                    src_fs: "/".into(),
                    src: path.to_string_lossy().into_owned(),
                    dst_fs,
                    dst: dst_remote,
                    cut: false,
                });
            }
        }
        Ok(())
    }

    fn remove_empty_dirs(&self) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let path = self
            .selected_name()
            .map(|n| join_remote_path(&current.path, &n))
            .unwrap_or_else(|| current.path.clone());
        let (fs, remote) = fs_remote(&current.remote, &path);
        match client.rmdirs(&fs, &remote) {
            Ok(_) => {
                self.reload();
                self.toast
                    .add_toast(adw::Toast::new("Removed empty directories"));
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn cleanup_remote(&self) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let (fs, remote) = fs_remote(&current.remote, &current.path);
        let remote_opt = if remote.is_empty() {
            None
        } else {
            Some(remote.as_str())
        };
        match client.cleanup(&fs, remote_opt) {
            Ok(_) => self
                .toast
                .add_toast(adw::Toast::new("Cleanup started for this remote")),
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn extract_selected(&self) {
        let Some(name) = self.selected_name() else {
            self.toast
                .add_toast(adw::Toast::new("Select an archive to extract"));
            return;
        };
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let current = self.current.borrow().clone();
        let src = self.formatted_path(Some(&name));
        let default_dst = current.path.clone();
        let view = self.clone();
        dialogs::prompt(
            &win,
            &self.ctx,
            &self
                .ctx
                .t_or("fileBrowser.fileViewer.extract", "Extract archive"),
            &self.ctx.t_or(
                "fileBrowser.operations.details.destination",
                "Destination path",
            ),
            &default_dst,
            move |dst| {
                let Some(client) = view.ctx.client() else {
                    return;
                };
                let dest = if dst.is_empty() {
                    view.formatted_path(None)
                } else if current.remote == "local" || dst.contains(':') {
                    dst
                } else {
                    format!("{}:{dst}", current.remote)
                };
                match client.archive_extract(&src, &dest) {
                    Ok(id) => {
                        view.ctx.refresh_runtime();
                        view.toast
                            .add_toast(adw::Toast::new(&format!("Extract job #{id}")));
                    }
                    Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            },
        );
    }

    fn share_selected(&self) {
        let Some(name) = self.selected_name() else {
            self.toast
                .add_toast(adw::Toast::new("Select a file to share"));
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        if current.remote == "local" {
            match crate::platform::share_file(std::path::Path::new(&path)) {
                Ok(()) => self
                    .toast
                    .add_toast(adw::Toast::new("Opened system share for the file")),
                Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
            }
            return;
        }
        let Some(client) = self.ctx.client() else {
            return;
        };
        let (fs, remote) = fs_remote(&current.remote, &path);
        let dest = std::env::temp_dir().join(&name);
        match client.copy_file(&fs, &remote, "/", &dest.to_string_lossy()) {
            Ok(_) => match crate::platform::share_file(&dest) {
                Ok(()) => self
                    .toast
                    .add_toast(adw::Toast::new("Opened system share for the file")),
                Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
            },
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn open_native_selected(&self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        if current.remote == "local" {
            let _ = open::that(&path);
            return;
        }
        let Some(client) = self.ctx.client() else {
            return;
        };
        let (fs, remote) = fs_remote(&current.remote, &path);
        let dest = std::env::temp_dir().join(&name);
        match client.copy_file(&fs, &remote, "/", &dest.to_string_lossy()) {
            Ok(_) => {
                let _ = open::that(&dest);
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn open_selected_in_new_tab(&self) {
        if let Some(name) = self.selected_name() {
            let current = self.current.borrow().clone();
            let next = join_remote_path(&current.path, &name);
            self.open_new_tab();
            self.current.borrow_mut().path = next;
            self.current.borrow_mut().remote = current.remote;
            self.reload();
            return;
        }
        self.open_new_tab();
    }

    fn open_selected_in_new_window(&self) {
        if let Some(name) = self.selected_name() {
            let listing = self.last_listing.borrow().clone();
            if listing.iter().any(|e| e.name == name && e.is_dir) {
                self.open_target_in_new_window(&self.formatted_path(Some(&name)));
                return;
            }
        }
        self.open_target_in_new_window(&self.formatted_path(None));
    }

    fn reload_ops(&self) {
        while let Some(child) = self.ops.first_child() {
            self.ops.remove(&child);
        }
        let jobs = self.ctx.snapshot.borrow().jobs.clone();
        let history = self.ctx.store.borrow().job_history.clone();
        if jobs.is_empty() && history.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(
                &self
                    .ctx
                    .t_or("nautilus.noFileOperations", "No file operations"),
            );
            self.ops.append(&row);
            return;
        }
        for job in &jobs {
            self.ops.append(&self.ops_row(job, true));
        }
        let running_ids: std::collections::HashSet<u64> = jobs.iter().map(|j| j.id).collect();
        for job in history
            .iter()
            .filter(|j| !running_ids.contains(&j.id))
            .take(12)
        {
            self.ops.append(&self.ops_row(job, false));
        }
    }

    fn ops_row(&self, job: &crate::store::JobInfo, live: bool) -> adw::ActionRow {
        let percent = (job.progress * 100.0).round() as i32;
        let row = adw::ActionRow::new();
        row.set_title(&format!("{} · {}", job.operation, job.status));
        let src = if job.src.is_empty() {
            job.remote.clone()
        } else {
            job.src.clone()
        };
        row.set_subtitle(&format!("#{id} · {percent}% · {src}", id = job.id));
        row.set_activatable(true);
        {
            let ctx = self.ctx.clone();
            let id = job.id;
            let view = self.clone();
            row.connect_activated(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::job_detail(&win, ctx.clone(), id);
                }
            });
        }
        if live && job.status == "running" {
            let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
            stop.set_valign(gtk::Align::Center);
            stop.set_tooltip_text(Some("Stop"));
            let ctx = self.ctx.clone();
            let id = job.id;
            let view = self.clone();
            stop.connect_clicked(move |_| {
                if let Some(client) = ctx.client() {
                    let _ = client.job_stop(id);
                    ctx.refresh_runtime();
                    view.reload_ops();
                }
            });
            row.add_suffix(&stop);
        } else {
            let dismiss = gtk::Button::from_icon_name("window-close-symbolic");
            dismiss.set_valign(gtk::Align::Center);
            dismiss.set_tooltip_text(Some("Remove from history"));
            let ctx = self.ctx.clone();
            let id = job.id;
            let view = self.clone();
            dismiss.connect_clicked(move |_| {
                ctx.store.borrow_mut().dismiss_job(id);
                ctx.persist();
                view.reload_ops();
            });
            row.add_suffix(&dismiss);
        }
        row
    }
}

fn fs_remote(remote: &str, path: &str) -> (String, String) {
    if remote == "local" {
        ("/".into(), path.trim_start_matches('/').to_string())
    } else {
        (remote_fs(remote, ""), path.to_string())
    }
}

fn row_name(row: &gtk::ListBoxRow) -> Option<String> {
    let child = row.child()?.downcast::<adw::ActionRow>().ok()?;
    if child.widget_name() == "column-header" {
        return None;
    }
    let named = child.widget_name().to_string();
    if !named.is_empty() && named != "AdwActionRow" {
        return Some(named);
    }
    Some(child.title().to_string())
}

fn make_flow() -> gtk::FlowBox {
    let grid = gtk::FlowBox::new();
    grid.set_selection_mode(gtk::SelectionMode::Multiple);
    grid.set_homogeneous(true);
    grid.set_min_children_per_line(2);
    grid.set_max_children_per_line(12);
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    grid.set_valign(gtk::Align::Start);
    grid.set_vexpand(true);
    grid
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn clear_flow(flow: &gtk::FlowBox) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
}

fn flow_child_name(child: &gtk::FlowBoxChild) -> Option<String> {
    child.child().and_then(|widget| {
        let name = widget.widget_name().to_string();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    })
}

fn select_flow_in_rect(flow: &gtk::FlowBox, x1: f64, y1: f64, x2: f64, y2: f64) {
    flow.unselect_all();
    let mut child = flow.first_child();
    while let Some(widget) = child {
        if let Some(bounds) = widget.compute_bounds(flow) {
            let left = f64::from(bounds.x());
            let top = f64::from(bounds.y());
            let right = left + f64::from(bounds.width());
            let bottom = top + f64::from(bounds.height());
            if right >= x1 && left <= x2 && bottom >= y1 && top <= y2 {
                if let Ok(item) = widget.clone().downcast::<gtk::FlowBoxChild>() {
                    flow.select_child(&item);
                }
            }
        }
        child = widget.next_sibling();
    }
}
