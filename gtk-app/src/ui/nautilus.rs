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
    path_entry: gtk::Entry,
    status: gtk::Label,
    tabs: Rc<RefCell<Vec<TabState>>>,
    current: Rc<RefCell<TabState>>,
    secondary: Rc<RefCell<TabState>>,
    history: Rc<RefCell<Vec<(String, String)>>>,
    future: Rc<RefCell<Vec<(String, String)>>>,
    clipboard: Rc<RefCell<Option<(String, String, bool)>>>,
    undo: Rc<RefCell<Vec<String>>>,
    tab_bar: gtk::Box,
    next_tab_id: Rc<RefCell<u32>>,
    split_enabled: Rc<RefCell<bool>>,
    paned: gtk::Paned,
    right_scroll: gtk::ScrolledWindow,
    ops: gtk::ListBox,
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
        back.set_tooltip_text(Some("Back"));
        let forward = gtk::Button::from_icon_name("go-next-symbolic");
        forward.set_tooltip_text(Some("Forward"));
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.set_tooltip_text(Some("Parent folder"));
        let reload = gtk::Button::from_icon_name("view-refresh-symbolic");
        reload.set_tooltip_text(Some("Reload"));
        let path_entry = gtk::Entry::new();
        path_entry.set_hexpand(true);
        path_entry.set_placeholder_text(Some("remote:path or /local/path"));
        let new_folder = gtk::Button::from_icon_name("folder-new-symbolic");
        new_folder.set_tooltip_text(Some("New folder"));
        let upload = gtk::Button::from_icon_name("document-send-symbolic");
        upload.set_tooltip_text(Some("Upload files"));
        let layout = gtk::Button::from_icon_name("view-list-symbolic");
        layout.set_tooltip_text(Some("Toggle list / grid"));
        let hidden_btn = gtk::Button::from_icon_name("view-conceal-symbolic");
        hidden_btn.set_tooltip_text(Some("Toggle hidden files"));
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
        toolbar.append(&path_entry);
        toolbar.append(&new_folder);
        toolbar.append(&upload);
        toolbar.append(&new_tab);
        toolbar.append(&split_btn);
        toolbar.append(&star);
        toolbar.append(&layout);
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
        let files_scroll = gtk::ScrolledWindow::new();
        files_scroll.set_vexpand(true);
        files_scroll.set_child(Some(&list));
        let list_right = gtk::ListBox::new();
        list_right.add_css_class("boxed-list");
        list_right.set_selection_mode(gtk::SelectionMode::Multiple);
        let right_scroll = gtk::ScrolledWindow::new();
        right_scroll.set_vexpand(true);
        right_scroll.set_child(Some(&list_right));
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

        root.append(&toolbar);
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
            path_entry,
            status,
            tabs: Rc::new(RefCell::new(vec![initial.clone()])),
            current: Rc::new(RefCell::new(initial.clone())),
            secondary: Rc::new(RefCell::new(initial)),
            history: Rc::new(RefCell::new(vec![])),
            future: Rc::new(RefCell::new(vec![])),
            clipboard: Rc::new(RefCell::new(None)),
            undo: Rc::new(RefCell::new(vec![])),
            tab_bar,
            next_tab_id: Rc::new(RefCell::new(2)),
            split_enabled: Rc::new(RefCell::new(false)),
            paned,
            right_scroll,
            ops,
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
                let next = if view.ctx.settings.borrow().nautilus.layout == "grid" {
                    "list"
                } else {
                    "grid"
                };
                view.ctx.settings.borrow_mut().nautilus.layout = next.into();
                view.ctx.persist();
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
            let list = view.list.clone();
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            gesture.connect_pressed(move |_, _, _, _| view.popup_context());
            list.add_controller(gesture);
        }
        {
            let view = view.clone();
            let drop = gtk::DropTarget::new(gio::File::static_type(), gtk::gdk::DragAction::COPY);
            {
                let view = view.clone();
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
            view.list.add_controller(drop);
        }
        {
            let view = view.clone();
            let list = view.list.clone();
            let drag = gtk::GestureDrag::new();
            drag.connect_drag_end(move |g, _, _| {
                let Some((_, y)) = g.start_point() else {
                    return;
                };
                let Some((_, ey)) = g.offset() else {
                    return;
                };
                let y1 = y.min(y + ey);
                let y2 = y.max(y + ey);
                if (y2 - y1).abs() < 12.0 {
                    return;
                }
                view.list.unselect_all();
                let mut child = view.list.first_child();
                let mut acc = 0.0;
                while let Some(row) = child {
                    let h = row.height() as f64;
                    let top = acc;
                    let bottom = acc + h;
                    if bottom >= y1 && top <= y2 {
                        if let Ok(list_row) = row.clone().downcast::<gtk::ListBoxRow>() {
                            view.list.select_row(Some(&list_row));
                        }
                    }
                    acc = bottom;
                    child = row.next_sibling();
                }
            });
            list.add_controller(drag);
        }

        view.reload_sidebar();
        view.reload();
        view.refresh_tabs();
        view.reload_ops();
        view.install_keybinds();
        view
    }

    fn install_keybinds(&self) {
        let controller = gtk::EventControllerKey::new();
        let view = self.clone();
        controller.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            if ctrl && key == gtk::gdk::Key::l {
                view.path_entry.grab_focus();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::f {
                view.path_entry.grab_focus();
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
            if ctrl && key == gtk::gdk::Key::z {
                view.undo_last();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::a {
                view.list.select_all();
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
        self.add_side_header("Starred");
        for star in &self.ctx.settings.borrow().nautilus.starred {
            if let Some(path) = star.get("path").and_then(|x| x.as_str()) {
                self.add_side_row(path, path);
            }
        }
        self.add_side_header("Bookmarks");
        for mark in &self.ctx.settings.borrow().nautilus.bookmarks {
            if let (Some(name), Some(path)) = (
                mark.get("name").and_then(|x| x.as_str()),
                mark.get("path").and_then(|x| x.as_str()),
            ) {
                self.add_side_row(name, path);
            }
        }
        self.add_side_header("Local");
        for disk in &self.ctx.snapshot.borrow().local_disks {
            self.add_side_row(disk, disk);
        }
        self.add_side_header("Cloud remotes");
        for remote in &self.ctx.snapshot.borrow().remotes {
            self.add_side_row(&remote.name, &format!("{}:", remote.name));
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

    fn navigate_to(&self, input: &str) {
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
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let current = self.current.borrow().clone();
        let display = if current.remote == "local" {
            current.path.clone()
        } else {
            format!("{}:{}", current.remote, current.path)
        };
        self.path_entry.set_text(&display);
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
                sort_entries(
                    &mut entries,
                    &self.ctx.settings.borrow().nautilus.sort_by,
                    self.ctx.settings.borrow().nautilus.sort_desc,
                );
                self.status.set_text(&format!("{} items", entries.len()));
                for entry in entries {
                    self.list.append(&self.entry_row(entry));
                }
                if *self.split_enabled.borrow() {
                    self.reload_pane(&self.list_right, &self.secondary.borrow());
                }
            }
            Err(err) => {
                self.status.set_text(&err.to_string());
                self.toast.add_toast(adw::Toast::new(&err.to_string()));
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
        if self.ctx.settings.borrow().nautilus.layout == "grid" {
            icon.set_pixel_size(self.ctx.settings.borrow().nautilus.icon_size.max(32));
        }
        row.add_prefix(&icon);
        row.set_activatable(true);
        row
    }

    fn selected_name(&self) -> Option<String> {
        self.list.selected_row().and_then(|row| row_name(&row))
    }

    fn selected_names(&self) -> Vec<String> {
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

    fn open_name(&self, name: &str) {
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
            dialogs::file_viewer(&win, self.ctx.clone(), &current.remote, &path, name);
        }
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
                            view.undo.borrow_mut().push(format!("mkdir:{remote}"));
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
        match client.copy_file("/", &path.to_string_lossy(), &dst_fs, &dst_remote) {
            Ok(_) => {
                self.undo.borrow_mut().push(format!("upload:{dst_remote}"));
                self.reload();
                self.toast
                    .add_toast(adw::Toast::new(&format!("Uploaded {name}")));
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

    fn reload_pane(&self, list: &gtk::ListBox, tab: &TabState) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
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
            for entry in entries {
                list.append(&self.entry_row(entry));
            }
        }
    }

    fn undo_last(&self) {
        let Some(op) = self.undo.borrow_mut().pop() else {
            self.toast.add_toast(adw::Toast::new("Nothing to undo"));
            return;
        };
        let Some(client) = self.ctx.client() else {
            return;
        };
        if let Some(path) = op.strip_prefix("mkdir:") {
            let current = self.current.borrow().clone();
            let fs = if current.remote == "local" {
                "/".into()
            } else {
                remote_fs(&current.remote, "")
            };
            let _ = client.purge(&fs, path);
            self.reload();
        } else if let Some(path) = op.strip_prefix("upload:") {
            let current = self.current.borrow().clone();
            let fs = if current.remote == "local" {
                "/".into()
            } else {
                remote_fs(&current.remote, "")
            };
            let _ = client.delete_file(&fs, path);
            self.reload();
        }
        self.toast.add_toast(adw::Toast::new("Undid last action"));
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
                    Ok(_) => view.reload(),
                    Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                }
            }
        });
    }

    fn delete_selected(&self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
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
        match client.delete_file(&fs, &remote) {
            Ok(_) => self.reload(),
            Err(_) => {
                let _ = client.purge(&fs, &remote);
                self.reload();
            }
        }
    }

    fn cut_or_copy(&self, cut: bool) {
        let Some(name) = self.selected_name() else {
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        *self.clipboard.borrow_mut() = Some((current.remote, path, cut));
        self.toast
            .add_toast(adw::Toast::new(if cut { "Cut" } else { "Copied" }));
    }

    fn paste(&self) {
        let Some((src_remote, src_path, cut)) = self.clipboard.borrow().clone() else {
            return;
        };
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let name = src_path.rsplit('/').next().unwrap_or(&src_path).to_string();
        let dst_path = join_remote_path(&current.path, &name);
        let src_fs = if src_remote == "local" {
            "/".into()
        } else {
            remote_fs(&src_remote, "")
        };
        let dst_fs = if current.remote == "local" {
            "/".into()
        } else {
            remote_fs(&current.remote, "")
        };
        let src_remote_path = if src_remote == "local" {
            src_path.trim_start_matches('/').to_string()
        } else {
            src_path
        };
        let dst_remote_path = if current.remote == "local" {
            dst_path.trim_start_matches('/').to_string()
        } else {
            dst_path
        };
        let result = if cut {
            client.move_file(&src_fs, &src_remote_path, &dst_fs, &dst_remote_path)
        } else {
            client.copy_file(&src_fs, &src_remote_path, &dst_fs, &dst_remote_path)
        };
        match result {
            Ok(_) => self.reload(),
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
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

    fn popup_context(&self) {
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let popover = gtk::Popover::new();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        for (label, action) in [
            ("Open", "open"),
            ("Open in new tab", "tab"),
            ("Copy", "copy"),
            ("Cut", "cut"),
            ("Paste", "paste"),
            ("Rename", "rename"),
            ("Delete", "delete"),
            ("Properties", "props"),
            ("New folder", "mkdir"),
            ("Bookmark", "star"),
            ("Create archive", "archive"),
            ("Send to…", "sendto"),
            ("Undo", "undo"),
        ] {
            let btn = gtk::Button::with_label(label);
            let view = self.clone();
            btn.connect_clicked(move |_| match action {
                "open" => {
                    if let Some(name) = view.selected_name() {
                        view.open_name(&name);
                    }
                }
                "tab" => view.open_new_tab(),
                "copy" => view.cut_or_copy(false),
                "cut" => view.cut_or_copy(true),
                "paste" => view.paste(),
                "rename" => view.rename_selected(),
                "delete" => view.delete_selected(),
                "props" => view.properties_selected(),
                "mkdir" => view.mkdir_prompt(),
                "star" => view.add_bookmark(),
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
                _ => {}
            });
            box_.append(&btn);
        }
        popover.set_child(Some(&box_));
        popover.set_parent(&win);
        popover.popup();
    }

    fn reload_ops(&self) {
        while let Some(child) = self.ops.first_child() {
            self.ops.remove(&child);
        }
        let jobs = self.ctx.snapshot.borrow().jobs.clone();
        if jobs.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title("No file operations");
            self.ops.append(&row);
            return;
        }
        for job in jobs {
            let row = adw::ActionRow::new();
            row.set_title(&format!("{} · {}", job.operation, job.status));
            row.set_subtitle(&format!("#{} {}", job.id, job.remote));
            self.ops.append(&row);
        }
    }
}

fn row_name(row: &gtk::ListBoxRow) -> Option<String> {
    row.child()
        .and_then(|child| child.downcast::<adw::ActionRow>().ok())
        .map(|r| r.title().to_string())
}
