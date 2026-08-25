use super::dialogs;
use super::AppCtx;
use crate::operations::FileTypeCategory;
use crate::rclone::{
    format_bytes, join_remote_path, parent_remote_path, remote_fs, split_remote_path, DirEntry,
};
use crate::store::{sort_entries, Bookmark};
use adw::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct TabState {
    id: u32,
    title: String,
    remote: String,
    path: String,
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
        up.set_tooltip_text(Some("Parent folder"));
        let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
        reload.set_tooltip_text(Some(&ctx.t_or("common.refresh", "Reload")));
        let path_entry = gtk::Entry::new();
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some("remote:path or /local/path"));
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
        new_folder.set_tooltip_text(Some("New folder"));
        let upload = gtk::Button::from_icon_name("document-send-symbolic");
        upload.set_tooltip_text(Some("Upload files"));
        let layout = gtk::Button::from_icon_name("view-list-symbolic");
        layout.set_tooltip_text(Some("Toggle list / grid"));
        let hidden_btn = gtk::Button::from_icon_name("view-conceal-symbolic");
        hidden_btn.set_tooltip_text(Some("Toggle hidden files"));
        let sort_btn = gtk::Button::from_icon_name("view-sort-ascending-symbolic");
        sort_btn.set_tooltip_text(Some("Sort listing"));
        let new_tab = gtk::Button::from_icon_name("tab-new-symbolic");
        new_tab.set_tooltip_text(Some("New tab"));
        let split_btn = gtk::Button::from_icon_name("view-dual-symbolic");
        split_btn.set_tooltip_text(Some("Toggle split view"));
        let star = gtk::Button::from_icon_name("starred-symbolic");
        star.set_tooltip_text(Some("Bookmark this folder"));

        toolbar.append(&back);
        toolbar.append(&forward);
        toolbar.append(&up);
        toolbar.append(&reload);
        toolbar.append(&path_stack);
        toolbar.append(&new_folder);
        toolbar.append(&upload);
        toolbar.append(&new_tab);
        toolbar.append(&split_btn);
        toolbar.append(&star);
        toolbar.append(&layout);
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
        right_scroll.set_visible(false);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&files_scroll));
        paned.set_end_child(Some(&right_scroll));
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        paned.set_wide_handle(true);
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

        let status = gtk::Label::new(Some("Ready"));
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
        let picker_label = gtk::Label::new(Some("Select a location"));
        picker_label.set_hexpand(true);
        picker_label.set_xalign(0.0);
        picker_bar.append(&picker_label);
        let picker_cancel = gtk::Button::with_label("Cancel");
        picker_bar.append(&picker_cancel);
        let picker_select = gtk::Button::with_label("Select");
        picker_select.add_css_class("suggested-action");
        picker_bar.append(&picker_select);

        root.append(&toolbar);
        root.append(&picker_bar);
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
            undo: Rc::new(RefCell::new(vec![])),
            redo: Rc::new(RefCell::new(vec![])),
            tab_bar,
            next_tab_id: Rc::new(RefCell::new(2)),
            split_enabled: Rc::new(RefCell::new(false)),
            paned,
            right_scroll,
            ops,
            last_listing: Rc::new(RefCell::new(Vec::new())),
            picker_bar,
            picker_label,
        };

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
                        view.open_name(&name);
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
                        view.open_name(&name);
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
            star.connect_clicked(move |_| view.add_bookmark());
        }
        {
            let view = view.clone();
            sort_btn.connect_clicked(move |_| view.cycle_sort());
        }
        view.attach_file_controllers(&view.list, false);
        view.attach_file_controllers(&view.list_right, false);
        view.attach_file_controllers(&view.grid, true);
        view.attach_file_controllers(&view.grid_right, true);

        {
            let view = view.clone();
            picker_cancel.connect_clicked(move |_| view.finish_picker(true));
        }
        {
            let view = view.clone();
            picker_select.connect_clicked(move |_| view.finish_picker(false));
        }
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

    fn cycle_sort(&self) {
        let next = match self.ctx.settings.borrow().nautilus.sort_by.as_str() {
            "name" => "size",
            "size" => "modified",
            _ => "name",
        };
        if next == "name" {
            let desc = self.ctx.settings.borrow().nautilus.sort_desc;
            self.ctx.settings.borrow_mut().nautilus.sort_desc = !desc;
        }
        self.ctx.settings.borrow_mut().nautilus.sort_by = next.to_string();
        self.ctx.persist();
        let desc = self.ctx.settings.borrow().nautilus.sort_desc;
        self.status.set_text(&format!(
            "Sorted by {next}{}",
            if desc { " (desc)" } else { "" }
        ));
        self.reload();
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

    fn attach_file_controllers(&self, widget: &impl IsA<gtk::Widget>, grid: bool) {
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

        let drag = gtk::GestureDrag::new();
        {
            let view = self.clone();
            drag.connect_drag_end(move |g, _, _| {
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
            glib::Propagation::Proceed
        });
        self.root.add_controller(controller);
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
        self.add_side_header(&self.ctx.t_or("nautilus.titles.starred", "Starred"));
        for star in &self.ctx.settings.borrow().nautilus.starred {
            if let Some(path) = star.get("path").and_then(|x| x.as_str()) {
                if allowed(path) {
                    self.add_side_row(path, path);
                }
            }
        }
        self.add_side_header(&self.ctx.t_or("nautilus.titles.bookmarks", "Bookmarks"));
        for mark in &self.ctx.settings.borrow().nautilus.bookmarks {
            if let (Some(name), Some(path)) = (
                mark.get("name").and_then(|x| x.as_str()),
                mark.get("path").and_then(|x| x.as_str()),
            ) {
                if allowed(path) {
                    self.add_side_row(name, path);
                }
            }
        }
        self.add_side_header(&self.ctx.t_or("nautilus.titles.local", "Local"));
        for disk in &self.ctx.snapshot.borrow().local_disks {
            if allowed(disk) {
                self.add_side_row(disk, disk);
            }
        }
        self.add_side_header(&self.ctx.t_or("nautilus.titles.cloud", "Cloud remotes"));
        for remote in &self.ctx.snapshot.borrow().remotes {
            let loc = format!("{}:", remote.name);
            if allowed(&loc) {
                self.add_side_row(&remote.name, &loc);
            }
        }
    }

    fn add_side_header(&self, title: &str) {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_sensitive(false);
        self.sidebar.append(&row);
    }

    fn add_side_row(&self, title: &str, target: &str) {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_activatable(true);
        let view = self.clone();
        let target = target.to_string();
        row.connect_activated(move |_| view.navigate_to(&target));
        self.sidebar.append(&row);
    }

    pub fn navigate_to(&self, input: &str) {
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
        btn.connect_clicked(move |_| view.navigate_to(&target));
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
                crate::picker::PickerSelection::Folders => "Select a folder",
                crate::picker::PickerSelection::Files => "Select a file",
                crate::picker::PickerSelection::Both => "Select a file or folder",
            };
            if self.picker_label.text().as_str() != text {
                self.picker_label.set_text(text);
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
            self.reload();
        }
    }

    fn go_up(&self) {
        let path = self.current.borrow().path.clone();
        let parent = parent_remote_path(&path);
        self.current.borrow_mut().path = parent;
        self.reload();
    }

    fn reload(&self) {
        clear_list(&self.list);
        clear_flow(&self.grid);
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
            self.status
                .set_text("Rclone engine offline — showing empty listing");
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
                sort_entries(
                    &mut entries,
                    &self.ctx.settings.borrow().nautilus.sort_by,
                    self.ctx.settings.borrow().nautilus.sort_desc,
                );
                self.status.set_text(&format!("{} items", entries.len()));
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
        if self.is_grid() {
            let grid = if primary {
                &self.grid
            } else {
                &self.grid_right
            };
            for entry in entries {
                grid.insert(&self.entry_tile(entry.clone()), -1);
            }
        } else {
            let list = if primary {
                &self.list
            } else {
                &self.list_right
            };
            for entry in entries {
                list.append(&self.entry_row(entry.clone()));
            }
        }
    }

    fn entry_row(&self, entry: DirEntry) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(&entry.name);
        let category = FileTypeCategory::from_name(&entry.name, entry.is_dir);
        row.set_subtitle(&if entry.is_dir {
            "Folder".into()
        } else {
            format!("{} · {}", format_bytes(entry.size), entry.mod_time)
        });
        let icon = gtk::Image::from_icon_name(category.icon_name());
        row.add_prefix(&icon);
        row.set_activatable(true);
        row
    }

    fn entry_tile(&self, entry: DirEntry) -> gtk::Box {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
        tile.set_halign(gtk::Align::Center);
        tile.set_valign(gtk::Align::Start);
        tile.set_margin_top(8);
        tile.set_margin_bottom(8);
        tile.set_margin_start(8);
        tile.set_margin_end(8);
        tile.set_widget_name(&entry.name);
        let category = FileTypeCategory::from_name(&entry.name, entry.is_dir);
        let icon = gtk::Image::from_icon_name(category.icon_name());
        icon.set_pixel_size(self.ctx.settings.borrow().nautilus.icon_size.max(48));
        let label = gtk::Label::new(Some(&entry.name));
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(14);
        label.set_justify(gtk::Justification::Center);
        label.set_wrap(true);
        tile.append(&icon);
        tile.append(&label);
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

    fn open_name(&self, name: &str) {
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
            let siblings: Vec<String> = self
                .last_listing
                .borrow()
                .iter()
                .filter(|e| !e.is_dir)
                .map(|e| e.name.clone())
                .collect();
            dialogs::file_viewer(
                &win,
                self.ctx.clone(),
                &current.remote,
                &path,
                name,
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
            "New folder with selected",
            "Folder name",
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
            dialogs::prompt(&win, "New folder", "Folder name", "", move |name| {
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
            });
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

    fn toggle_split(&self) {
        let next = !*self.split_enabled.borrow();
        *self.split_enabled.borrow_mut() = next;
        self.right_scroll.set_visible(next);
        if next {
            *self.secondary.borrow_mut() = self.current.borrow().clone();
            self.reload();
            self.status.set_text("Split view — two listings");
        } else {
            self.status.set_text("Split view off");
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
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        dialogs::properties(&win, self.ctx.clone(), &current.remote, &path, &name);
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
        dialogs::prompt(&win, "Rename", "New name", &old.clone(), move |new_name| {
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
        });
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
        let mut error = None;
        for (src_remote, src_path, cut) in items {
            let name = src_path.rsplit('/').next().unwrap_or(&src_path).to_string();
            let dst_path = join_remote_path(&current.path, &name);
            let (src_fs, src_remote_path) = fs_remote(&src_remote, &src_path);
            let (dst_fs, dst_remote_path) = fs_remote(&current.remote, &dst_path);
            let result = if cut {
                client.move_file(&src_fs, &src_remote_path, &dst_fs, &dst_remote_path)
            } else {
                client.copy_file(&src_fs, &src_remote_path, &dst_fs, &dst_remote_path)
            };
            if let Err(e) = result {
                error = Some(e.to_string());
                break;
            }
            let op = if cut {
                crate::fileops::FileOp::Move {
                    src_fs,
                    src: src_remote_path,
                    dst_fs,
                    dst: dst_remote_path,
                }
            } else {
                crate::fileops::FileOp::Copy {
                    src_fs,
                    src: src_remote_path,
                    dst_fs,
                    dst: dst_remote_path,
                }
            };
            self.push_undo(op.encode());
        }
        if let Some(e) = error {
            self.toast.add_toast(adw::Toast::new(&e));
        } else {
            self.reload();
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
            tab.title = if current.path.is_empty() {
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
            self.tab_bar.append(&btn);
        }
    }

    fn activate_tab(&self, id: u32) {
        if let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == id).cloned() {
            *self.current.borrow_mut() = tab;
            self.reload();
        }
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

    fn detach_current_tab(&self) {
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
                FileTypeCategory::from_name(name, false),
                FileTypeCategory::Archive
            )
        });
        let mut items: Vec<(&str, &str)> = vec![
            ("Open", "open"),
            ("Open native", "native"),
            ("Open in new tab", "tab"),
            ("Refresh", "reload"),
            ("Copy", "copy"),
            ("Cut", "cut"),
            ("Paste", "paste"),
            ("Copy path", "copypath"),
        ];
        if public_ok {
            items.push(("Copy public link", "public"));
        }
        items.extend([
            ("Copy URL into folder…", "copyurl"),
            ("Rename", "rename"),
            ("Delete", "delete"),
            ("Properties", "props"),
            ("Download…", "download"),
            ("New folder", "mkdir"),
        ]);
        if !selected.is_empty() {
            items.push(("New folder with selected", "mkdirsel"));
        }
        items.extend([
            ("Upload folder…", "uploaddir"),
            ("Bookmark", "star"),
            ("Create archive", "archive"),
        ]);
        if archive_selected {
            items.push(("Browse archive contents", "archivelist"));
            items.push(("Extract archive…", "extract"));
        }
        items.push(("Remove empty folders", "rmdirs"));
        if cleanup_ok {
            items.push(("Empty trash / cleanup", "cleanup"));
        }
        items.extend([
            ("Send to…", "sendto"),
            ("Share…", "share"),
            ("Undo", "undo"),
            ("Redo", "redo"),
            ("Paste from system clipboard", "syspaste"),
            ("Detach tab", "detach"),
        ]);
        for (label, action) in items {
            let btn = gtk::Button::with_label(label);
            let view = self.clone();
            btn.connect_clicked(move |_| match action {
                "open" => {
                    if let Some(name) = view.selected_name() {
                        view.open_name(&name);
                    }
                }
                "native" => view.open_native_selected(),
                "tab" => view.open_selected_in_new_tab(),
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
                "star" => view.add_bookmark(),
                "extract" => view.extract_selected(),
                "rmdirs" => view.remove_empty_dirs(),
                "cleanup" => view.cleanup_remote(),
                "archive" => {
                    if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                        let current = view.current.borrow().clone();
                        dialogs::archive_create(
                            &win,
                            view.ctx.clone(),
                            &current.remote,
                            &current.path,
                        );
                    }
                }
                "share" => view.share_selected(),
                "sendto" => {
                    let current = view.current.borrow().clone();
                    let registered = crate::platform::is_send_to_registered(
                        &current.remote,
                        Some(&current.path),
                    );
                    let result = if registered {
                        crate::platform::unregister_send_to(&current.remote, Some(&current.path))
                    } else {
                        crate::platform::register_send_to(&current.remote, Some(&current.path))
                    };
                    match result {
                        Ok(_) => view.toast.add_toast(adw::Toast::new(if registered {
                            "Removed Send to shortcut"
                        } else {
                            "Added Send to shortcut"
                        })),
                        Err(e) => view.toast.add_toast(adw::Toast::new(&e)),
                    }
                }
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
        let view = self.clone();
        dialogs::prompt(
            &win,
            "Copy URL",
            "Download this URL into the current folder",
            "https://",
            move |url| {
                if url.is_empty() {
                    return;
                }
                let Some(client) = view.ctx.client() else {
                    return;
                };
                let current = view.current.borrow().clone();
                let (fs, remote) = fs_remote(&current.remote, &current.path);
                match client.copy_url(&url, &fs, &remote) {
                    Ok(_) => {
                        view.reload();
                        view.toast.add_toast(adw::Toast::new("Started URL copy"));
                    }
                    Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            },
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
        let (fs, remote) = fs_remote(&current.remote, dest_dir);
        let _ = client.mkdir(&fs, &remote);
        let mut count = 1;
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
                count += self.upload_tree_into(&path, &dest)?;
            } else {
                let (dst_fs, dst_remote) = fs_remote(&current.remote, &dest);
                client
                    .copy_file("/", &path.to_string_lossy(), &dst_fs, &dst_remote)
                    .map_err(|e| e.to_string())?;
                count += 1;
            }
        }
        Ok(count)
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
            "Extract archive",
            "Destination path",
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

    fn reload_ops(&self) {
        while let Some(child) = self.ops.first_child() {
            self.ops.remove(&child);
        }
        let jobs = self.ctx.snapshot.borrow().jobs.clone();
        let history = self.ctx.store.borrow().job_history.clone();
        if jobs.is_empty() && history.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("No file operations");
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
    row.child()
        .and_then(|child| child.downcast::<adw::ActionRow>().ok())
        .map(|r| r.title().to_string())
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
