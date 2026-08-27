use super::dialogs;
use super::AppCtx;
use crate::listing::{self, ListJobState, ListStart};
use crate::operations::FileTypeCategory;
use crate::rclone::{
    format_bytes, format_relative_mod_time, join_remote_path, parent_remote_path, remote_fs,
    split_remote_path, DirEntry,
};
use crate::store::{apply_sort_option, sort_entries, sort_option_key, Bookmark, JobInfo};
use adw::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Copy)]
struct LassoDrag {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    grid: bool,
    primary: bool,
}

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
    history: Vec<(String, String)>,
    future: Vec<(String, String)>,
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
    clipboard: Rc<RefCell<Vec<(String, String, bool, bool)>>>,
    pending_drag: Rc<RefCell<Option<Vec<crate::dnd::DragItem>>>>,
    skip_lasso: Rc<Cell<bool>>,
    lasso_drag: Rc<RefCell<Option<LassoDrag>>>,
    lasso_tick: Rc<Cell<bool>>,
    lasso_pointer_y: Rc<Cell<Option<f64>>>,
    is_narrow: Rc<Cell<bool>>,
    ignore_activate: Rc<Cell<bool>>,
    listing_menu_open: Rc<Cell<bool>>,
    listing_popover: gtk::Popover,
    hover_open: Rc<RefCell<Option<String>>>,
    undo: Rc<RefCell<Vec<String>>>,
    redo: Rc<RefCell<Vec<String>>>,
    tab_bar: gtk::Box,
    next_tab_id: Rc<RefCell<u32>>,
    split_enabled: Rc<RefCell<bool>>,
    paned: gtk::Paned,
    ops: gtk::ListBox,
    ops_title: gtk::Label,
    last_listing: Rc<RefCell<Vec<DirEntry>>>,
    last_listing_right: Rc<RefCell<Vec<DirEntry>>>,
    listing_shown: Rc<Cell<usize>>,
    listing_shown_right: Rc<Cell<usize>>,
    files_scroll: gtk::ScrolledWindow,
    right_scroll: gtk::ScrolledWindow,
    picker_bar: gtk::Box,
    picker_label: gtk::Label,
    bottom_bar: gtk::Box,
    bottom_confirm: gtk::Button,
    share_bar: gtk::Box,
    share_label: gtk::Label,
    filter_bar: gtk::Box,
    icon_btn: gtk::Button,
    actions_btn: gtk::MenuButton,
    send_to_btn: gtk::Button,
    split: adw::OverlaySplitView,
    last_poll_jobs: Rc<RefCell<Vec<JobInfo>>>,
    ops_sig: Rc<RefCell<String>>,
    list_group_left: String,
    list_group_right: String,
    list_job_left: Rc<Cell<Option<u64>>>,
    list_job_right: Rc<Cell<Option<u64>>>,
    list_gen_left: Rc<Cell<u64>>,
    list_gen_right: Rc<Cell<u64>>,
    loading_left: gtk::Box,
    loading_right: gtk::Box,
    right_host: gtk::Overlay,
    pending_undo: Rc<RefCell<Vec<crate::fileops::PendingUndo>>>,
}

fn picker_result_from_selection(
    remote: &str,
    current_path: &str,
    listing: &[DirEntry],
    names: &[String],
    config: &crate::picker::FilePickerConfig,
) -> crate::picker::PickerResult {
    let is_dir_of = |name: &str| {
        listing
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.is_dir)
            .unwrap_or(true)
    };
    let chosen: Vec<(String, bool)> = names
        .iter()
        .filter(|name| crate::picker::is_entry_allowed(name, is_dir_of(name), config))
        .map(|name| (join_remote_path(current_path, name), is_dir_of(name)))
        .collect();
    if chosen.is_empty() {
        return crate::picker::PickerResult {
            remote: remote.to_string(),
            path: current_path.to_string(),
            is_dir: true,
            cancelled: false,
            extra_paths: vec![],
        };
    }
    let extra_paths = if config.multi {
        chosen
            .iter()
            .skip(1)
            .map(|(path, _)| path.clone())
            .collect()
    } else {
        vec![]
    };
    crate::picker::PickerResult {
        remote: remote.to_string(),
        path: chosen[0].0.clone(),
        is_dir: chosen[0].1,
        cancelled: false,
        extra_paths,
    }
}

fn listing_loading_box(ctx: &AppCtx) -> (gtk::Box, gtk::Button) {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
    box_.set_widget_name("listing-loading");
    box_.add_css_class("osd");
    box_.set_halign(gtk::Align::Center);
    box_.set_valign(gtk::Align::Center);
    box_.set_margin_top(24);
    box_.set_margin_bottom(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    spinner.set_size_request(32, 32);
    let label = gtk::Label::new(Some(&ctx.t_or("common.loading", "Loading...")));
    let cancel = gtk::Button::from_icon_name("window-close-symbolic");
    cancel.set_tooltip_text(Some(&ctx.t("common.cancel")));
    cancel.add_css_class("circular");
    cancel.set_halign(gtk::Align::Center);
    box_.append(&spinner);
    box_.append(&label);
    box_.append(&cancel);
    box_.set_visible(false);
    (box_, cancel)
}

impl NautilusView {
    pub fn new(ctx: AppCtx, toast: adw::ToastOverlay) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.add_css_class("nautilus-toolbar");
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
        let crumbs_overlay = gtk::Overlay::new();
        crumbs_overlay.set_hexpand(true);
        crumbs_overlay.set_child(Some(&crumbs_scroll));
        let fade_start = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fade_start.add_css_class("path-fade-start");
        fade_start.set_halign(gtk::Align::Start);
        fade_start.set_valign(gtk::Align::Fill);
        fade_start.set_can_target(false);
        fade_start.set_visible(false);
        let fade_end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        fade_end.add_css_class("path-fade-end");
        fade_end.set_halign(gtk::Align::End);
        fade_end.set_valign(gtk::Align::Fill);
        fade_end.set_can_target(false);
        fade_end.set_visible(false);
        crumbs_overlay.add_overlay(&fade_start);
        crumbs_overlay.add_overlay(&fade_end);
        {
            let fade_start = fade_start.clone();
            let fade_end = fade_end.clone();
            let sync = Rc::new(move |adj: &gtk::Adjustment| {
                fade_start.set_visible(adj.value() > 1.0);
                fade_end.set_visible(adj.value() + adj.page_size() + 1.0 < adj.upper());
            });
            let adj = crumbs_scroll.hadjustment();
            adj.connect_value_changed({
                let sync = sync.clone();
                move |adj| sync(adj)
            });
            adj.connect_notify_local(Some("upper"), {
                let sync = sync.clone();
                move |adj, _| sync(adj)
            });
            adj.connect_notify_local(Some("page-size"), move |adj, _| sync(adj));
        }
        let search_entry = gtk::SearchEntry::new();
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some(
            &ctx.t_or("nautilus.titles.searchPlaceholder", "Search files..."),
        ));
        let path_stack = gtk::Stack::new();
        path_stack.set_hexpand(true);
        path_stack.set_hexpand_set(true);
        path_stack.set_width_request(80);
        path_stack.add_named(&crumbs_overlay, Some("crumbs"));
        path_stack.add_named(&path_entry, Some("entry"));
        path_stack.add_named(&search_entry, Some("search"));
        path_stack.set_visible_child_name("crumbs");
        let new_folder = gtk::Button::from_icon_name("folder-new-symbolic");
        new_folder.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.newFolder", "New folder"),
        ));
        let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy_btn.set_tooltip_text(Some(&ctx.t_or("nautilus.contextMenu.copy", "Copy")));
        let copy_to_btn = gtk::Button::from_icon_name("folder-copy-symbolic");
        copy_to_btn.set_tooltip_text(Some(&ctx.t_or("nautilus.contextMenu.copyTo", "Copy to…")));
        let move_to_btn = gtk::Button::from_icon_name("send-to-symbolic");
        move_to_btn.set_tooltip_text(Some(&ctx.t_or("nautilus.contextMenu.moveTo", "Move to…")));
        let paste_btn = gtk::Button::from_icon_name("edit-paste-symbolic");
        paste_btn.set_tooltip_text(Some(&ctx.t_or("nautilus.contextMenu.paste", "Paste")));
        let actions_btn = gtk::MenuButton::new();
        actions_btn.set_icon_name("view-list-bullet-symbolic");
        actions_btn.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.selectionActions", "Selection actions"),
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

        let sidebar_btn = gtk::Button::from_icon_name("sidebar-show-symbolic");
        sidebar_btn.set_tooltip_text(Some(&ctx.t_or("sidebar.toggleSidebar", "Toggle Sidebar")));
        toolbar.append(&sidebar_btn);
        toolbar.append(&back);
        toolbar.append(&forward);
        toolbar.append(&up);
        let path_menu = gtk::MenuButton::new();
        path_menu.set_icon_name("open-menu-symbolic");
        path_menu.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.pathOptions", "Path options"),
        ));
        toolbar.append(&reload);
        let send_to_btn = gtk::Button::from_icon_name("list-add-symbolic");
        send_to_btn.set_tooltip_text(Some(&ctx.t_or(
            "nautilus.contextMenu.addToSendTo",
            "Add to File Manager Menu",
        )));
        toolbar.append(&path_stack);
        toolbar.append(&path_menu);
        toolbar.append(&send_to_btn);
        toolbar.append(&new_folder);
        toolbar.append(&copy_btn);
        toolbar.append(&copy_to_btn);
        toolbar.append(&move_to_btn);
        toolbar.append(&paste_btn);
        toolbar.append(&actions_btn);
        toolbar.append(&upload);
        toolbar.append(&new_tab);
        toolbar.append(&split_btn);
        let detach_btn = gtk::Button::from_icon_name("view-restore-symbolic");
        detach_btn.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.detachTab", "Detach Tab"),
        ));
        toolbar.append(&detach_btn);
        toolbar.append(&star);
        toolbar.append(&layout);
        toolbar.append(&icon_btn);
        toolbar.append(&sort_btn);
        toolbar.append(&hidden_btn);

        let split = adw::OverlaySplitView::new();
        split.set_min_sidebar_width(220.0);
        split.set_show_sidebar(ctx.settings.borrow().nautilus.sidebar_visible);
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let side_scroll = gtk::ScrolledWindow::new();
        side_scroll.set_child(Some(&sidebar));
        split.set_sidebar(Some(&side_scroll));

        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Multiple);
        list.set_activate_on_single_click(false);
        let grid = make_flow();
        let left_stack = gtk::Stack::new();
        left_stack.add_named(&list, Some("list"));
        left_stack.add_named(&grid, Some("grid"));
        let files_scroll = gtk::ScrolledWindow::new();
        files_scroll.set_vexpand(true);
        files_scroll.set_child(Some(&left_stack));
        let (loading_left, cancel_left) = listing_loading_box(&ctx);
        let left_host = gtk::Overlay::new();
        left_host.set_hexpand(true);
        left_host.set_vexpand(true);
        left_host.set_child(Some(&files_scroll));
        left_host.add_overlay(&loading_left);
        let list_right = gtk::ListBox::new();
        list_right.add_css_class("boxed-list");
        list_right.set_selection_mode(gtk::SelectionMode::Multiple);
        list_right.set_activate_on_single_click(false);
        let grid_right = make_flow();
        let right_stack = gtk::Stack::new();
        right_stack.add_named(&list_right, Some("list"));
        right_stack.add_named(&grid_right, Some("grid"));
        let right_scroll = gtk::ScrolledWindow::new();
        right_scroll.set_vexpand(true);
        right_scroll.set_child(Some(&right_stack));
        let (loading_right, cancel_right) = listing_loading_box(&ctx);
        let right_host = gtk::Overlay::new();
        right_host.set_hexpand(true);
        right_host.set_vexpand(true);
        right_host.set_child(Some(&right_scroll));
        right_host.add_overlay(&loading_right);
        let split_on = ctx.settings.borrow().nautilus.split_enabled;
        right_host.set_visible(split_on);
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_start_child(Some(&left_host));
        paned.set_end_child(Some(&right_host));
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
        let tab_scroll = gtk::ScrolledWindow::new();
        tab_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        tab_scroll.set_propagate_natural_height(true);
        tab_scroll.set_child(Some(&tab_bar));
        let tab_overlay = gtk::Overlay::new();
        tab_overlay.set_child(Some(&tab_scroll));
        let tab_fade_start = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_fade_start.add_css_class("tab-fade-start");
        tab_fade_start.set_halign(gtk::Align::Start);
        tab_fade_start.set_valign(gtk::Align::Fill);
        tab_fade_start.set_can_target(false);
        tab_fade_start.set_visible(false);
        let tab_fade_end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_fade_end.add_css_class("tab-fade-end");
        tab_fade_end.set_halign(gtk::Align::End);
        tab_fade_end.set_valign(gtk::Align::Fill);
        tab_fade_end.set_can_target(false);
        tab_fade_end.set_visible(false);
        tab_overlay.add_overlay(&tab_fade_start);
        tab_overlay.add_overlay(&tab_fade_end);
        {
            let tab_fade_start = tab_fade_start.clone();
            let tab_fade_end = tab_fade_end.clone();
            let sync = Rc::new(move |adj: &gtk::Adjustment| {
                tab_fade_start.set_visible(adj.value() > 1.0);
                tab_fade_end.set_visible(adj.value() + adj.page_size() + 1.0 < adj.upper());
            });
            let adj = tab_scroll.hadjustment();
            adj.connect_value_changed({
                let sync = sync.clone();
                move |adj| sync(adj)
            });
            adj.connect_notify_local(Some("upper"), {
                let sync = sync.clone();
                move |adj, _| sync(adj)
            });
            adj.connect_notify_local(Some("page-size"), move |adj, _| sync(adj));
        }
        let files_col = gtk::Box::new(gtk::Orientation::Vertical, 4);
        files_col.append(&tab_overlay);
        files_col.append(&paned);
        split.set_content(Some(&files_col));

        let ops = gtk::ListBox::new();
        ops.add_css_class("boxed-list");
        let ops_scroll = gtk::ScrolledWindow::new();
        ops_scroll.set_min_content_height(180);
        ops_scroll.set_max_content_height(260);
        ops_scroll.set_child(Some(&ops));
        let ops_title = gtk::Label::new(Some(
            &ctx.t_or("fileBrowser.operations.title", "Operations"),
        ));
        ops_title.add_css_class("heading");
        ops_title.set_xalign(0.0);
        ops_title.set_hexpand(true);
        let ops_expander = gtk::Expander::new(None);
        ops_expander.set_label_widget(Some(&ops_title));
        ops_expander.set_expanded(ctx.settings.borrow().nautilus.ops_panel_expanded);
        ops_expander.set_child(Some(&ops_scroll));
        ops_expander.set_margin_start(8);
        ops_expander.set_margin_end(8);
        ops_expander.set_margin_top(4);

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

        let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bottom_bar.add_css_class("toolbar");
        bottom_bar.set_margin_start(8);
        bottom_bar.set_margin_end(8);
        bottom_bar.set_margin_top(4);
        bottom_bar.set_margin_bottom(4);
        bottom_bar.set_visible(false);
        let bottom_sidebar = gtk::Button::from_icon_name("sidebar-show-symbolic");
        bottom_sidebar.set_tooltip_text(Some(&ctx.t_or("sidebar.toggleSidebar", "Toggle Sidebar")));
        let bottom_confirm =
            gtk::Button::with_label(&ctx.t_or("nautilus.contextMenu.open", "Open"));
        bottom_confirm.add_css_class("suggested-action");
        bottom_confirm.set_hexpand(true);
        bottom_confirm.set_visible(false);
        let bottom_layout = gtk::Button::from_icon_name("view-list-symbolic");
        bottom_layout.set_tooltip_text(Some(
            &ctx.t_or("nautilus.view.toggleLayout", "Toggle list / grid"),
        ));
        let bottom_popout = gtk::Button::from_icon_name("window-new-symbolic");
        bottom_popout.set_tooltip_text(Some(
            &ctx.t_or("nautilus.contextMenu.openNewWindow", "Open in New Window"),
        ));
        let bottom_view = gtk::MenuButton::new();
        bottom_view.set_icon_name("view-more-symbolic");
        bottom_view.set_tooltip_text(Some(&ctx.t_or("nautilus.view.viewOptions", "View options")));
        bottom_bar.append(&bottom_sidebar);
        bottom_bar.append(&bottom_confirm);
        bottom_bar.append(&bottom_popout);
        bottom_bar.append(&bottom_layout);
        bottom_bar.append(&bottom_view);

        let toolbar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .overlay_scrolling(true)
            .propagate_natural_height(true)
            .propagate_natural_width(false)
            .hexpand(true)
            .build();
        toolbar_scroll.add_css_class("nautilus-toolbar-scroll");
        toolbar_scroll.set_child(Some(&toolbar));
        let filter_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .overlay_scrolling(true)
            .propagate_natural_height(true)
            .propagate_natural_width(false)
            .hexpand(true)
            .build();
        filter_scroll.add_css_class("nautilus-toolbar-scroll");
        filter_scroll.set_child(Some(&filter_bar));
        root.append(&toolbar_scroll);
        root.append(&filter_scroll);
        root.append(&picker_bar);
        root.append(&share_bar);
        root.append(&split);
        root.append(&ops_expander);
        root.append(&status);
        root.append(&bottom_bar);

        let initial = TabState {
            id: 1,
            title: "Home".into(),
            remote: "local".into(),
            path: dirs::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".into()),
            starred: false,
            history: Vec::new(),
            future: Vec::new(),
        };
        let mut secondary = initial.clone();
        {
            let nautilus = ctx.settings.borrow().nautilus.clone();
            if nautilus.split_enabled && !nautilus.split_secondary_remote.is_empty() {
                secondary.remote = nautilus.split_secondary_remote;
                secondary.path = nautilus.split_secondary_path;
                secondary.title = secondary.remote.clone();
            }
        }

        let listing_popover = gtk::Popover::new();
        listing_popover.set_autohide(true);
        listing_popover.set_has_arrow(true);
        listing_popover.set_parent(&root);

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
            secondary: Rc::new(RefCell::new(secondary)),
            history: Rc::new(RefCell::new(vec![])),
            future: Rc::new(RefCell::new(vec![])),
            clipboard: Rc::new(RefCell::new(Vec::new())),
            pending_drag: Rc::new(RefCell::new(None)),
            skip_lasso: Rc::new(Cell::new(false)),
            lasso_drag: Rc::new(RefCell::new(None)),
            lasso_tick: Rc::new(Cell::new(false)),
            lasso_pointer_y: Rc::new(Cell::new(None)),
            is_narrow: Rc::new(Cell::new(false)),
            ignore_activate: Rc::new(Cell::new(false)),
            listing_menu_open: Rc::new(Cell::new(false)),
            listing_popover,
            hover_open: Rc::new(RefCell::new(None)),
            undo: Rc::new(RefCell::new(vec![])),
            redo: Rc::new(RefCell::new(vec![])),
            tab_bar,
            next_tab_id: Rc::new(RefCell::new(2)),
            split_enabled: Rc::new(RefCell::new(split_on)),
            paned,
            ops,
            ops_title,
            last_listing: Rc::new(RefCell::new(Vec::new())),
            last_listing_right: Rc::new(RefCell::new(Vec::new())),
            listing_shown: Rc::new(Cell::new(0)),
            listing_shown_right: Rc::new(Cell::new(0)),
            files_scroll: files_scroll.clone(),
            right_scroll: right_scroll.clone(),
            picker_bar,
            picker_label,
            bottom_bar,
            bottom_confirm: bottom_confirm.clone(),
            share_bar,
            share_label,
            filter_bar,
            icon_btn: icon_btn.clone(),
            actions_btn: actions_btn.clone(),
            send_to_btn: send_to_btn.clone(),
            split: split.clone(),
            last_poll_jobs: Rc::new(RefCell::new(Vec::new())),
            ops_sig: Rc::new(RefCell::new(String::new())),
            list_group_left: listing::list_read_group(true),
            list_group_right: listing::list_read_group(false),
            list_job_left: Rc::new(Cell::new(None)),
            list_job_right: Rc::new(Cell::new(None)),
            list_gen_left: Rc::new(Cell::new(0)),
            list_gen_right: Rc::new(Cell::new(0)),
            loading_left,
            loading_right,
            right_host,
            pending_undo: Rc::new(RefCell::new(Vec::new())),
        };
        view.refresh_type_filters();
        {
            let view = view.clone();
            let motion = gtk::EventControllerMotion::new();
            motion.connect_motion({
                let view = view.clone();
                move |_, x, y| {
                    let primary = view
                        .lasso_drag
                        .borrow()
                        .map(|drag| drag.primary)
                        .unwrap_or(true);
                    let scroll = if primary {
                        &view.files_scroll
                    } else {
                        &view.right_scroll
                    };
                    if let Some(pt) = view
                        .root
                        .compute_point(scroll, &gtk::graphene::Point::new(x as f32, y as f32))
                    {
                        view.lasso_pointer_y.set(Some(f64::from(pt.y())));
                    }
                }
            });
            view.root.add_controller(motion);
        }
        {
            let view = view.clone();
            files_scroll
                .vadjustment()
                .connect_value_changed(move |adj| {
                    view.maybe_scroll_load(true, adj);
                });
        }
        {
            let view = view.clone();
            right_scroll
                .vadjustment()
                .connect_value_changed(move |adj| {
                    view.maybe_scroll_load(false, adj);
                });
        }
        {
            let view = view.clone();
            ops_expander.connect_notify_local(Some("expanded"), move |expander, _| {
                view.ctx.settings.borrow_mut().nautilus.ops_panel_expanded = expander.is_expanded();
                view.ctx.persist();
            });
        }
        {
            let view = view.clone();
            cancel_left.connect_clicked(move |_| view.cancel_listing(true));
        }
        {
            let view = view.clone();
            cancel_right.connect_clicked(move |_| view.cancel_listing(false));
        }
        {
            let view = view.clone();
            view.listing_popover.connect_closed(move |_| {
                view.listing_menu_open.set(false);
                view.ignore_activate.set(false);
                view.skip_lasso.set(false);
            });
        }

        {
            let view = view.clone();
            sidebar_btn.connect_clicked(move |_| view.toggle_sidebar());
        }
        {
            let view = view.clone();
            detach_btn.connect_clicked(move |_| view.detach_current_tab());
        }
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
            send_to_btn.connect_clicked(move |_| view.toggle_send_to());
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
                view.clear_search();
            });
        }
        {
            let view = view.clone();
            new_folder.connect_clicked(move |_| view.mkdir_prompt());
        }
        {
            let view = view.clone();
            copy_btn.connect_clicked(move |_| view.cut_or_copy(false));
        }
        {
            let view = view.clone();
            copy_to_btn.connect_clicked(move |_| view.copy_or_move_to(false));
        }
        {
            let view = view.clone();
            move_to_btn.connect_clicked(move |_| view.copy_or_move_to(true));
        }
        {
            let view = view.clone();
            paste_btn.connect_clicked(move |_| view.paste());
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
                if view.ignore_activate.get() {
                    return;
                }
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
                    if view.ignore_activate.get() {
                        return;
                    }
                    if let Some(name) = row_name(row) {
                        view.open_name_in_pane(&name, false);
                    }
                });
        }
        {
            let view = view.clone();
            view.grid.clone().connect_child_activated(move |_, child| {
                if view.ignore_activate.get() {
                    return;
                }
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
                    if view.ignore_activate.get() {
                        return;
                    }
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
        view.attach_selection_actions(&actions_btn);
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
            bottom_confirm.connect_clicked(move |_| view.finish_picker(false));
        }
        {
            let view = view.clone();
            bottom_sidebar.connect_clicked(move |_| view.toggle_sidebar());
        }
        {
            let view = view.clone();
            bottom_layout.connect_clicked(move |_| {
                let next = if view.is_grid() { "list" } else { "grid" };
                view.ctx.settings.borrow_mut().nautilus.layout = next.into();
                view.ctx.persist();
                view.sync_layout();
                view.reload();
            });
        }
        {
            let view = view.clone();
            bottom_popout.connect_clicked(move |_| view.detach_current_tab());
        }
        view.attach_view_options(&bottom_view);
        {
            let view = view.clone();
            view.root.clone().connect_map(move |widget| {
                view.hook_narrow_resize(widget);
                view.sync_narrow_from_widget(widget);
            });
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
            items.push((
                self.ctx.t_or("nautilus.contextMenu.copyTo", "Copy to…"),
                "copyto",
            ));
            items.push((
                self.ctx.t_or("nautilus.contextMenu.moveTo", "Move to…"),
                "moveto",
            ));
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
                "copyto" => view.copy_or_move_to(false),
                "moveto" => view.copy_or_move_to(true),
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

    fn attach_selection_actions(&self, button: &gtk::MenuButton) {
        let popover = gtk::Popover::new();
        button.set_popover(Some(&popover));
        let view = self.clone();
        popover.connect_show(move |popover| {
            popover.set_child(Some(&view.build_selection_actions()));
        });
    }

    fn build_selection_actions(&self) -> gtk::Box {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        box_.set_margin_start(8);
        box_.set_margin_end(8);
        let selected = self.selected_names();
        let rename_label = if selected.len() > 1 {
            self.ctx
                .t_or("nautilus.contextMenu.renameMultiple", "Rename Multiple...")
        } else {
            self.ctx.t_or("nautilus.contextMenu.rename", "Rename")
        };
        let items = [
            (self.ctx.t_or("nautilus.contextMenu.open", "Open"), "open"),
            (self.ctx.t_or("nautilus.contextMenu.copy", "Copy"), "copy"),
            (self.ctx.t_or("nautilus.contextMenu.cut", "Cut"), "cut"),
            (
                self.ctx.t_or("nautilus.contextMenu.copyTo", "Copy to…"),
                "copyto",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.moveTo", "Move to…"),
                "moveto",
            ),
            (
                self.ctx.t_or("nautilus.contextMenu.paste", "Paste"),
                "paste",
            ),
            (rename_label, "rename"),
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
        ];
        for (label, action) in items {
            let btn = gtk::Button::with_label(&label);
            btn.set_halign(gtk::Align::Fill);
            let view = self.clone();
            btn.connect_clicked(move |_| match action {
                "open" => {
                    if let Some(name) = view.selected_name() {
                        view.open_name(&name);
                    }
                }
                "copy" => view.cut_or_copy(false),
                "cut" => view.cut_or_copy(true),
                "copyto" => view.copy_or_move_to(false),
                "moveto" => view.copy_or_move_to(true),
                "paste" => view.paste(),
                "rename" => view.rename_selected(),
                "delete" => view.delete_selected(),
                "props" => view.properties_selected(),
                "download" => view.download_selected(),
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

    fn listing_progress_label(&self) -> String {
        let total = self.last_listing.borrow().len();
        let shown = self.listing_shown.get();
        if let Some((shown, total)) = crate::fileops::listing_progress_caption(shown, total) {
            return self.ctx.tf_or(
                "nautilus.progressiveRender",
                "Showing {{shown}} of {{total}}",
                &[("shown", &shown.to_string()), ("total", &total.to_string())],
            );
        }
        self.listing_count_label(total)
    }

    fn refresh_listing_status(&self) {
        let text = self.selection_status();
        if text.is_empty() {
            self.status.set_text(&self.listing_progress_label());
        } else {
            self.status.set_text(&text);
        }
        if let Some(prompt) = self.picker_prompt() {
            let label = crate::picker::picker_bar_label(&prompt, &text);
            if self.picker_label.text().as_str() != label {
                self.picker_label.set_text(&label);
            }
        }
    }

    fn maybe_scroll_load(&self, primary: bool, adj: &gtk::Adjustment) {
        if !crate::fileops::listing_near_bottom(
            adj.value(),
            adj.page_size(),
            adj.upper(),
            crate::fileops::LISTING_SCROLL_LOAD_PX,
        ) {
            return;
        }
        let (shown, total) = if primary {
            (self.listing_shown.get(), self.last_listing.borrow().len())
        } else {
            (
                self.listing_shown_right.get(),
                self.last_listing_right.borrow().len(),
            )
        };
        if shown >= total {
            return;
        }
        self.append_listing_batch(primary);
        if primary {
            self.refresh_listing_status();
        }
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

    fn picker_prompt(&self) -> Option<String> {
        self.ctx
            .pending_picker
            .borrow()
            .as_ref()
            .map(|req| match req.config.selection {
                crate::picker::PickerSelection::Folders => self
                    .ctx
                    .t_or("nautilus.titles.selectFolder", "Select a folder"),
                crate::picker::PickerSelection::Files => {
                    self.ctx.t_or("nautilus.titles.selectFile", "Select a file")
                }
                crate::picker::PickerSelection::Both => self
                    .ctx
                    .t_or("nautilus.titles.selectItems", "Select a file or folder"),
            })
    }

    fn refresh_selection_status(&self) {
        self.refresh_listing_status();
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

    fn starred_unstar_button(&self, path: &str, name: &str) -> gtk::Button {
        let btn = gtk::Button::from_icon_name("starred-symbolic");
        btn.add_css_class("flat");
        btn.add_css_class("star-button");
        btn.set_tooltip_text(Some(
            &self.ctx.t_or("fileBrowser.properties.unstar", "Unstar"),
        ));
        btn.set_can_focus(false);
        let view = self.clone();
        let path = path.to_string();
        let name = name.to_string();
        btn.connect_clicked(move |b| {
            view.toggle_star_path(&path, &name);
            b.set_state_flags(gtk::StateFlags::empty(), true);
        });
        btn
    }

    fn show_starred(&self) {
        self.close_sidebar_if_narrow();
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
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let view = self.clone();
            let host = widget.clone().upcast::<gtk::Widget>();
            gesture.connect_pressed(move |g, n_press, _, _| {
                if n_press != 1 {
                    return;
                }
                g.set_state(gtk::EventSequenceState::Claimed);
            });
            gesture.connect_released(move |_, n_press, x, y| {
                if n_press != 1 {
                    return;
                }
                let view = view.clone();
                let host = host.clone();
                view.schedule_listing_menu(host, x, y, move |view, host, x, y| {
                    if let Some(name) = view.hit_test_name(host, x, y, grid, primary) {
                        view.ensure_name_selected(&name, primary);
                    } else {
                        view.clear_listing_selection(primary);
                    }
                    view.popup_context_at(host, x, y);
                });
            });
        }
        widget.add_controller(gesture);

        let drop = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY,
        );
        {
            let view = self.clone();
            drop.connect_drop(move |_, value, _, _| {
                if let Ok(list) = value.get::<gtk::gdk::FileList>() {
                    let paths: Vec<std::path::PathBuf> = list
                        .files()
                        .into_iter()
                        .filter_map(|file| file.path())
                        .collect();
                    if !paths.is_empty() {
                        view.upload_local_paths(&paths);
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
        drag.set_button(1);
        {
            let view = self.clone();
            drag.connect_drag_update(move |g, ox, oy| {
                if view.skip_lasso.get() {
                    return;
                }
                let Some((x, y)) = g.start_point() else {
                    return;
                };
                view.update_lasso_drag(x, y, x + ox, y + oy, grid, primary);
            });
        }
        {
            let view = self.clone();
            drag.connect_drag_end(move |g, _, _| {
                view.lasso_tick.set(false);
                view.lasso_pointer_y.set(None);
                if view.skip_lasso.get() {
                    view.lasso_drag.borrow_mut().take();
                    return;
                }
                let Some((x, y)) = g.start_point() else {
                    view.lasso_drag.borrow_mut().take();
                    return;
                };
                let Some((ox, oy)) = g.offset() else {
                    view.lasso_drag.borrow_mut().take();
                    return;
                };
                view.apply_lasso(x, y, x + ox, y + oy, grid);
                view.lasso_drag.borrow_mut().take();
            });
        }
        widget.add_controller(drag);
    }

    fn item_menu_button(&self, name: &str, primary: bool) -> gtk::Button {
        let more = gtk::Button::from_icon_name("view-more-symbolic");
        more.add_css_class("flat");
        more.add_css_class("circular");
        more.add_css_class("file-item-menu");
        more.set_can_target(true);
        more.set_focus_on_click(true);
        more.set_size_request(
            crate::fileops::FILE_ITEM_MENU_HIT_PX,
            crate::fileops::FILE_ITEM_MENU_HIT_PX,
        );
        more.set_tooltip_text(Some(
            &self.ctx.t_or("nautilus.contextMenu.moreActions", "Actions"),
        ));
        more.set_widget_name(&crate::fileops::file_item_menu_widget_name(name));
        {
            let view = self.clone();
            let name = name.to_string();
            let target = more.clone();
            more.connect_clicked(move |_| {
                view.skip_lasso.set(true);
                view.ignore_activate.set(true);
                view.ensure_name_selected(&name, primary);
                let view = view.clone();
                let target = target.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
                    view.popup_context_at(
                        &target,
                        f64::from(crate::fileops::FILE_ITEM_MENU_HIT_PX) / 2.0,
                        f64::from(crate::fileops::FILE_ITEM_MENU_HIT_PX) / 2.0,
                    );
                });
            });
        }
        more
    }

    fn attach_item_context(&self, widget: &impl IsA<gtk::Widget>, name: &str, primary: bool) {
        let gesture = gtk::GestureClick::new();
        gesture.set_button(3);
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        let view = self.clone();
        let name = name.to_string();
        let target = widget.clone().upcast::<gtk::Widget>();
        gesture.connect_pressed(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
        });
        {
            let view = view.clone();
            let name = name.clone();
            let target = target.clone();
            gesture.connect_released(move |_, _, x, y| {
                let view = view.clone();
                let name = name.clone();
                let target = target.clone();
                view.schedule_listing_menu(target, x, y, move |view, target, x, y| {
                    view.ensure_name_selected(&name, primary);
                    view.popup_context_at(target, x, y);
                });
            });
        }
        widget.add_controller(gesture);
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
            source.connect_drag_begin(move |_, drag| {
                let items = view.pending_drag.borrow().clone().unwrap_or_default();
                let icon = gtk::DragIcon::for_drag(drag);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.add_css_class("card");
                row.set_margin_start(8);
                row.set_margin_end(8);
                row.set_margin_top(6);
                row.set_margin_bottom(6);
                row.append(&gtk::Image::from_icon_name(crate::dnd::drag_ghost_icon(
                    &items,
                )));
                row.append(&gtk::Label::new(Some(&crate::dnd::drag_ghost_label(
                    &items,
                ))));
                icon.set_child(Some(&row));
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
        let host = widget.clone().upcast::<gtk::Widget>();
        {
            let view = self.clone();
            let dest = dest.clone();
            let host = host.clone();
            drop.connect_drop(move |_, value, _, _| {
                host.remove_css_class("file-drag-over");
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
            let host = host.clone();
            drop.connect_enter(move |_, _, _| {
                if view.pending_drag.borrow().is_some() {
                    host.add_css_class("file-drag-over");
                }
                view.schedule_hover_open(&dest);
                gtk::gdk::DragAction::COPY
            });
        }
        {
            let view = self.clone();
            let host = host.clone();
            drop.connect_leave(move |_| {
                host.remove_css_class("file-drag-over");
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
        self.persist_split_location();
        if *self.split_enabled.borrow() {
            self.reload_pane(&self.secondary.borrow());
        }
    }

    fn persist_split_location(&self) {
        let secondary = self.secondary.borrow();
        let mut settings = self.ctx.settings.borrow_mut();
        settings.nautilus.split_secondary_remote = secondary.remote.clone();
        settings.nautilus.split_secondary_path = secondary.path.clone();
        drop(settings);
        self.ctx.persist();
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

    fn attach_tab_reorder(&self, widget: &impl IsA<gtk::Widget>, id: u32) {
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::MOVE);
        let start = Rc::new(Cell::new((0.0_f64, 0.0_f64)));
        {
            let start = start.clone();
            let host = widget.clone().upcast::<gtk::Widget>();
            source.connect_prepare(move |_, x, y| {
                start.set(widget_point_in_root(&host, x, y).unwrap_or((x, y)));
                Some(gtk::gdk::ContentProvider::for_value(
                    &crate::dnd::encode_tab_payload(id).to_value(),
                ))
            });
        }
        {
            let view = self.clone();
            source.connect_drag_begin(move |_, drag| {
                let title = view
                    .tabs
                    .borrow()
                    .iter()
                    .find(|tab| tab.id == id)
                    .map(|tab| tab.title.clone())
                    .unwrap_or_else(|| format!("tab {id}"));
                let icon = gtk::DragIcon::for_drag(drag);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.add_css_class("card");
                row.set_margin_start(8);
                row.set_margin_end(8);
                row.set_margin_top(6);
                row.set_margin_bottom(6);
                row.append(&gtk::Image::from_icon_name("folder-symbolic"));
                row.append(&gtk::Label::new(Some(&title)));
                icon.set_child(Some(&row));
            });
        }
        {
            let view = self.clone();
            let host = widget.clone().upcast::<gtk::Widget>();
            let start = start.clone();
            source.connect_drag_end(move |_, _, delete_data| {
                if delete_data {
                    return;
                }
                let end = pointer_in_root(&host).unwrap_or_else(|| start.get());
                let bounds = root_bounds(&host).unwrap_or((0.0, 0.0, 0.0, 0.0));
                if crate::dnd::should_detach_tab(false, start.get(), end, bounds) {
                    view.detach_tab(id);
                }
            });
        }
        widget.add_controller(source);
    }

    fn reorder_tab(&self, from_id: u32, to_id: u32) {
        let mut tabs = self.tabs.borrow_mut();
        let Some(from) = tabs.iter().position(|tab| tab.id == from_id) else {
            return;
        };
        let Some(to) = tabs.iter().position(|tab| tab.id == to_id) else {
            return;
        };
        crate::dnd::move_item_in_array(&mut tabs, from, to);
        drop(tabs);
        self.refresh_tabs();
    }

    fn handle_internal_drop(&self, text: &str, dest: &InternalDrop) -> bool {
        if let Some(from_id) = crate::dnd::decode_tab_payload(text) {
            if let InternalDrop::Tab(to_id) = dest {
                self.reorder_tab(from_id, *to_id);
                return true;
            }
            self.detach_tab(from_id);
            return true;
        }
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
                        self.remember_file_jobs_ex(
                            &ids,
                            "filemanager",
                            &group,
                            crate::jobs::transfer_snapshot_from_items(&transfers),
                        );
                        self.queue_job_undo(
                            &ids,
                            transfers.iter().map(|item| item.file_op()).collect(),
                        );
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

    fn update_lasso_drag(&self, x1: f64, y1: f64, x2: f64, y2: f64, grid: bool, primary: bool) {
        *self.lasso_drag.borrow_mut() = Some(LassoDrag {
            x1,
            y1,
            x2,
            y2,
            grid,
            primary,
        });
        self.apply_lasso(x1, y1, x2, y2, grid);
        self.maybe_lasso_scroll();
    }

    fn lasso_viewport_y(&self, scroll: &gtk::ScrolledWindow, fallback_content_y: f64) -> f64 {
        self.lasso_pointer_y
            .get()
            .unwrap_or_else(|| fallback_content_y - scroll.vadjustment().value())
    }

    fn maybe_lasso_scroll(&self) {
        let Some(drag) = *self.lasso_drag.borrow() else {
            return;
        };
        let scroll = if drag.primary {
            &self.files_scroll
        } else {
            &self.right_scroll
        };
        let adj = scroll.vadjustment();
        let height = if adj.page_size() > 0.0 {
            adj.page_size()
        } else {
            f64::from(scroll.height())
        };
        let viewport_y = self.lasso_viewport_y(scroll, drag.y2);
        let delta = crate::fileops::lasso_edge_scroll(
            viewport_y,
            0.0,
            height,
            crate::fileops::LASSO_EDGE_PX,
            crate::fileops::LASSO_SCROLL_STEP,
        );
        if delta == 0.0 || height <= 0.0 {
            return;
        }
        if self.lasso_tick.get() {
            return;
        }
        self.lasso_tick.set(true);
        let view = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            if !view.lasso_tick.get() {
                return glib::ControlFlow::Break;
            }
            let Some(mut drag) = view.lasso_drag.borrow().as_ref().copied() else {
                view.lasso_tick.set(false);
                return glib::ControlFlow::Break;
            };
            let scroll = if drag.primary {
                &view.files_scroll
            } else {
                &view.right_scroll
            };
            let adj = scroll.vadjustment();
            let height = if adj.page_size() > 0.0 {
                adj.page_size()
            } else {
                f64::from(scroll.height())
            };
            let viewport_y = view.lasso_viewport_y(scroll, drag.y2);
            let delta = crate::fileops::lasso_edge_scroll(
                viewport_y,
                0.0,
                height,
                crate::fileops::LASSO_EDGE_PX,
                crate::fileops::LASSO_SCROLL_STEP,
            );
            if delta == 0.0 {
                return glib::ControlFlow::Continue;
            }
            let adj = scroll.vadjustment();
            let next = crate::fileops::clamp_scroll_value(
                adj.value() + delta,
                adj.page_size(),
                adj.upper(),
            );
            if (next - adj.value()).abs() < f64::EPSILON {
                return glib::ControlFlow::Continue;
            }
            adj.set_value(next);
            drag.y2 += delta;
            *view.lasso_drag.borrow_mut() = Some(drag);
            view.apply_lasso(drag.x1, drag.y1, drag.x2, drag.y2, drag.grid);
            glib::ControlFlow::Continue
        });
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
            if files_keys_should_yield(&view, key, ctrl) {
                return glib::Propagation::Proceed;
            }
            if ctrl && key == gtk::gdk::Key::l {
                view.show_path_entry();
                return glib::Propagation::Stop;
            }
            if ctrl && key == gtk::gdk::Key::f {
                view.show_search();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Escape {
                view.clear_search();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::F5 || (ctrl && key == gtk::gdk::Key::r) {
                view.reload();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::F2 {
                view.rename_selected();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Menu || (shift && key == gtk::gdk::Key::F10) {
                view.popup_context();
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::space {
                if let Some(name) = view.selected_names().first().cloned() {
                    view.open_viewer(&name);
                    return glib::Propagation::Stop;
                }
            }
            if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                if modifier.contains(gtk::gdk::ModifierType::ALT_MASK) {
                    view.properties_selected();
                    return glib::Propagation::Stop;
                }
                if ctrl {
                    view.open_selected_in_new_tab();
                    return glib::Propagation::Stop;
                }
                if shift {
                    view.open_selected_in_new_window();
                    return glib::Propagation::Stop;
                }
                if let Some(name) = view.selected_name() {
                    view.open_name(&name);
                    return glib::Propagation::Stop;
                }
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
        let paths: Vec<std::path::PathBuf> =
            paths.into_iter().map(std::path::PathBuf::from).collect();
        self.upload_local_paths(&paths);
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
        let shortcuts = adw::ActionRow::new();
        shortcuts.set_title(&self.ctx.t_or("shortcuts.title", "Keyboard Shortcuts"));
        shortcuts.set_activatable(true);
        shortcuts.add_prefix(&gtk::Image::from_icon_name("input-keyboard-symbolic"));
        {
            let view = self.clone();
            shortcuts.connect_activated(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::shortcuts_open(&win, &view.ctx, true);
                }
            });
        }
        self.sidebar.append(&shortcuts);
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
        for drive in crate::fileops::collect_local_drives(&disks) {
            if allowed(&drive.path) && !crate::settings::sidebar_id_hidden(&hidden, &drive.path) {
                let title = drive.title(|key, fallback| self.ctx.t_or(key, fallback));
                let icon = if drive.is_removable {
                    "media-removable-symbolic"
                } else {
                    "drive-harddisk-symbolic"
                };
                let mut tooltip = drive.path.clone();
                if drive.total_space > 0 {
                    tooltip = format!(
                        "{tooltip} — {} / {}",
                        crate::rclone::format_bytes(drive.available_space as i64),
                        crate::rclone::format_bytes(drive.total_space as i64)
                    );
                }
                if !drive.file_system.is_empty() {
                    tooltip = format!("{tooltip} ({})", drive.file_system);
                }
                self.add_side_disk_row(
                    &title,
                    &drive.path,
                    icon,
                    SideKind::Local,
                    drive.subtitle().as_deref(),
                    Some(&tooltip),
                );
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
        self.add_side_disk_row(title, target, icon, kind, None, None);
    }

    fn add_side_disk_row(
        &self,
        title: &str,
        target: &str,
        icon: &str,
        kind: SideKind,
        subtitle: Option<&str>,
        tooltip: Option<&str>,
    ) {
        let row = adw::ActionRow::new();
        row.set_title(title);
        if let Some(subtitle) = subtitle {
            row.set_subtitle(subtitle);
        }
        if let Some(tooltip) = tooltip {
            row.set_tooltip_text(Some(tooltip));
        }
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
        let Some(app) = win.application().and_downcast::<adw::Application>() else {
            return;
        };
        let (remote, path) = split_remote_path(target);
        super::window::present_files_overlay(&app, &self.ctx, &remote, &path, None);
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
                self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                    "nautilus.notifications.locationNotAllowed",
                    "That location is not allowed for this picker",
                )));
                return;
            }
        }
        let current = self.current.borrow().clone();
        let (remote, path) = split_remote_path(input);
        if listing::same_nav_location(&current.remote, &current.path, &remote, &path) {
            self.current.borrow_mut().starred = false;
            self.sync_current_tab();
            self.reload();
            return;
        }
        self.record_nav(&current.remote, &current.path);
        self.current.borrow_mut().remote = remote;
        self.current.borrow_mut().path = path;
        self.current.borrow_mut().starred = false;
        self.sync_current_tab();
        self.close_sidebar_if_narrow();
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
        edit.set_tooltip_text(Some(
            &self.ctx.t_or("nautilus.titles.editPath", "Edit path"),
        ));
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
        let showing = self.picker_bar.is_visible() || self.bottom_confirm.is_visible();
        self.sync_narrow_chrome();
        if active != showing {
            self.reload_sidebar();
            self.reload();
        }
        if self.ctx.pending_picker.borrow().is_some() {
            self.refresh_selection_status();
        }
    }

    fn finish_picker(&self, cancelled: bool) {
        self.finish_picker_choice(cancelled, None);
    }

    fn finish_picker_choice(&self, cancelled: bool, forced: Option<(String, bool)>) {
        let Some(req) = self.ctx.pending_picker.borrow_mut().take() else {
            return;
        };
        self.sync_narrow_chrome();
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
                extra_paths: vec![],
            }
        } else {
            picker_result_from_selection(
                &current.remote,
                &current.path,
                &listing,
                &self.selected_names(),
                &req.config,
            )
        };
        let dirs = usize::from(result.is_dir);
        let files = usize::from(!result.is_dir);
        if !cancelled && !crate::picker::can_confirm_selection(dirs, files, &req.config) {
            *self.ctx.pending_picker.borrow_mut() = Some(req);
            self.picker_bar.set_visible(true);
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.selectValid",
                "Select a valid item to continue",
            )));
            return;
        }
        self.picker_bar.set_visible(false);
        self.reload_sidebar();
        (req.on_pick)(result);
    }

    fn search_is_active(&self) -> bool {
        self.path_stack.visible_child_name().as_deref() == Some("search")
            || !self.search_filter.borrow().is_empty()
    }

    fn clear_search(&self) {
        self.search_entry.set_text("");
        self.search_filter.borrow_mut().clear();
        self.show_crumbs();
        self.reload();
    }

    fn go_back(&self) {
        if self.search_is_active() {
            self.clear_search();
            return;
        }
        let current = self.current.borrow().clone();
        let dest = listing::pop_nav_back(
            &mut self.history.borrow_mut(),
            &mut self.future.borrow_mut(),
            &current.remote,
            &current.path,
        );
        if let Some((remote, path)) = dest {
            self.current.borrow_mut().remote = remote;
            self.current.borrow_mut().path = path;
            self.current.borrow_mut().starred = false;
            self.sync_current_tab();
            self.reload();
        }
    }

    fn go_forward(&self) {
        let current = self.current.borrow().clone();
        let dest = listing::pop_nav_forward(
            &mut self.history.borrow_mut(),
            &mut self.future.borrow_mut(),
            &current.remote,
            &current.path,
        );
        if let Some((remote, path)) = dest {
            self.current.borrow_mut().remote = remote;
            self.current.borrow_mut().path = path;
            self.current.borrow_mut().starred = false;
            self.sync_current_tab();
            self.reload();
        }
    }

    fn go_up(&self) {
        if self.current.borrow().starred {
            self.current.borrow_mut().starred = false;
            self.reload();
            return;
        }
        let current = self.current.borrow().clone();
        let parent = parent_remote_path(&current.path);
        if listing::same_nav_location(&current.remote, &current.path, &current.remote, &parent) {
            return;
        }
        self.record_nav(&current.remote, &current.path);
        self.current.borrow_mut().path = parent;
        self.reload();
    }

    fn record_nav(&self, remote: &str, path: &str) {
        listing::push_nav_history(
            &mut self.history.borrow_mut(),
            &mut self.future.borrow_mut(),
            remote,
            path,
        );
        self.sync_current_tab();
    }

    fn reload(&self) {
        if self.listing_menu_open.get() {
            return;
        }
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
        self.sync_send_to_button();
        self.reload_ops();

        if self.ctx.client().is_none() {
            self.status.set_text(
                &self
                    .ctx
                    .t_or("fileBrowser.errors.connectionFailed", "Connection Failed"),
            );
            return;
        }
        self.begin_listing(true);
        if *self.split_enabled.borrow() {
            self.begin_listing(false);
        }
    }

    fn populate_entries(&self, entries: &[DirEntry], primary: bool) {
        if primary {
            *self.last_listing.borrow_mut() = entries.to_vec();
            self.listing_shown.set(0);
        } else {
            *self.last_listing_right.borrow_mut() = entries.to_vec();
            self.listing_shown_right.set(0);
        }
        if !self.is_grid() {
            let list = if primary {
                &self.list
            } else {
                &self.list_right
            };
            list.append(&self.column_header_row());
        }
        self.append_listing_batch(primary);
    }

    fn append_listing_batch(&self, primary: bool) {
        let listing = if primary {
            self.last_listing.borrow().clone()
        } else {
            self.last_listing_right.borrow().clone()
        };
        let shown = if primary {
            self.listing_shown.get()
        } else {
            self.listing_shown_right.get()
        };
        let (start, end) = crate::fileops::next_listing_batch(listing.len(), shown);
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
            remove_named_flow_child(grid, "load-more");
            for entry in &listing[start..end] {
                grid.insert(&self.entry_tile(entry.clone(), &tab, primary), -1);
            }
            if end < listing.len() {
                grid.insert(&self.load_more_tile(listing.len() - end, primary), -1);
            }
        } else {
            let list = if primary {
                &self.list
            } else {
                &self.list_right
            };
            remove_named_list_child(list, "load-more");
            for entry in &listing[start..end] {
                list.append(&self.entry_row(entry.clone(), &tab, primary));
            }
            if end < listing.len() {
                list.append(&self.load_more_row(listing.len() - end, primary));
            }
        }
        if primary {
            self.listing_shown.set(end);
            self.refresh_listing_status();
        } else {
            self.listing_shown_right.set(end);
        }
    }

    fn load_more_label(&self, remaining: usize) -> String {
        self.ctx.tf_or(
            "nautilus.loadMore",
            "Show {{count}} more",
            &[("count", &remaining.to_string())],
        )
    }

    fn load_more_row(&self, remaining: usize, primary: bool) -> adw::ActionRow {
        let row = adw::ActionRow::new();
        row.set_title(&self.load_more_label(remaining));
        row.set_activatable(true);
        row.set_widget_name("load-more");
        {
            let view = self.clone();
            row.connect_activated(move |_| view.append_listing_batch(primary));
        }
        row
    }

    fn load_more_tile(&self, remaining: usize, primary: bool) -> gtk::Widget {
        let btn = gtk::Button::with_label(&self.load_more_label(remaining));
        btn.set_widget_name("load-more");
        btn.add_css_class("pill");
        btn.set_halign(gtk::Align::Center);
        btn.set_valign(gtk::Align::Center);
        {
            let view = self.clone();
            btn.connect_clicked(move |_| view.append_listing_batch(primary));
        }
        btn.upcast()
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
        let icon = self.entry_icon(&entry, tab);
        row.add_prefix(&icon);
        if tab.starred {
            row.add_prefix(&self.starred_unstar_button(&entry.path, &entry.name));
        }
        if self.entry_is_cut(tab, &entry.name) {
            row.add_css_class("cut-item");
            row.set_opacity(0.5);
        }
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
        row.add_suffix(&self.item_menu_button(&entry.name, primary));
        row.set_activatable(true);
        self.attach_item_dnd(&icon, &entry, tab, primary);
        self.attach_item_context(&row, &entry.name, primary);
        row
    }

    fn entry_tile(&self, entry: DirEntry, tab: &TabState, primary: bool) -> gtk::Widget {
        let tile = gtk::Box::new(gtk::Orientation::Vertical, 4);
        tile.set_halign(gtk::Align::Center);
        tile.set_valign(gtk::Align::Start);
        tile.set_margin_top(8);
        tile.set_margin_bottom(8);
        tile.set_margin_start(8);
        tile.set_margin_end(8);
        tile.set_widget_name(&entry.name);
        let icon = self.entry_icon(&entry, tab);
        icon.set_pixel_size(self.current_icon_size().max(48));
        if self.entry_is_cut(tab, &entry.name) {
            tile.add_css_class("cut-item");
            tile.set_opacity(0.5);
        }
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
        self.attach_item_dnd(&icon, &entry, tab, primary);
        self.attach_item_context(&tile, &entry.name, primary);
        let overlay = gtk::Overlay::new();
        overlay.set_widget_name(&entry.name);
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);
        overlay.set_overflow(gtk::Overflow::Visible);
        overlay.set_child(Some(&tile));
        if tab.starred {
            let star = self.starred_unstar_button(&entry.path, &entry.name);
            star.set_halign(gtk::Align::Start);
            star.set_valign(gtk::Align::Start);
            star.set_margin_start(4);
            star.set_margin_top(4);
            overlay.add_overlay(&star);
        }
        let hit = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hit.add_css_class("file-item-menu-hit");
        hit.set_halign(gtk::Align::End);
        hit.set_valign(gtk::Align::Start);
        hit.set_can_target(true);
        hit.set_size_request(
            crate::fileops::FILE_ITEM_MENU_HIT_PX,
            crate::fileops::FILE_ITEM_MENU_HIT_PX,
        );
        let more = self.item_menu_button(&entry.name, primary);
        more.set_hexpand(true);
        more.set_vexpand(true);
        hit.append(&more);
        overlay.add_overlay(&hit);
        overlay.upcast()
    }

    fn selected_name(&self) -> Option<String> {
        self.selected_names().into_iter().next()
    }

    fn hit_test_name(
        &self,
        container: &gtk::Widget,
        x: f64,
        y: f64,
        grid: bool,
        primary: bool,
    ) -> Option<String> {
        if grid {
            let flow = if primary {
                &self.grid
            } else {
                &self.grid_right
            };
            let mut child = flow.first_child();
            while let Some(widget) = child {
                if let Some(bounds) = widget.compute_bounds(container) {
                    if crate::fileops::point_in_rect(
                        x,
                        y,
                        bounds.x() as f64,
                        bounds.y() as f64,
                        bounds.width() as f64,
                        bounds.height() as f64,
                    ) {
                        if let Ok(flow_child) = widget.clone().downcast::<gtk::FlowBoxChild>() {
                            return flow_child_name(&flow_child);
                        }
                    }
                }
                child = widget.next_sibling();
            }
            return None;
        }
        let list = if primary {
            &self.list
        } else {
            &self.list_right
        };
        let mut child = list.first_child();
        while let Some(widget) = child {
            if let Some(bounds) = widget.compute_bounds(container) {
                if crate::fileops::point_in_rect(
                    x,
                    y,
                    bounds.x() as f64,
                    bounds.y() as f64,
                    bounds.width() as f64,
                    bounds.height() as f64,
                ) {
                    if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>() {
                        return row_name(&row);
                    }
                }
            }
            child = widget.next_sibling();
        }
        None
    }

    fn ensure_name_selected(&self, name: &str, primary: bool) {
        if self
            .selected_names()
            .iter()
            .any(|existing| existing == name)
        {
            return;
        }
        if self.is_grid() {
            let grid = if primary {
                &self.grid
            } else {
                &self.grid_right
            };
            grid.unselect_all();
            let mut child = grid.first_child();
            while let Some(widget) = child {
                if let Ok(flow) = widget.clone().downcast::<gtk::FlowBoxChild>() {
                    if flow_child_name(&flow).as_deref() == Some(name) {
                        grid.select_child(&flow);
                        break;
                    }
                }
                child = widget.next_sibling();
            }
            return;
        }
        let list = if primary {
            &self.list
        } else {
            &self.list_right
        };
        list.unselect_all();
        let mut child = list.first_child();
        while let Some(widget) = child {
            if let Ok(row) = widget.clone().downcast::<gtk::ListBoxRow>() {
                if row_name(&row).as_deref() == Some(name) {
                    list.select_row(Some(&row));
                    break;
                }
            }
            child = widget.next_sibling();
        }
    }

    fn clear_listing_selection(&self, primary: bool) {
        if self.is_grid() {
            if primary {
                self.grid.unselect_all();
            } else {
                self.grid_right.unselect_all();
            }
            return;
        }
        if primary {
            self.list.unselect_all();
        } else {
            self.list_right.unselect_all();
        }
    }

    fn selected_is_directory(&self, name: &str) -> bool {
        self.last_listing
            .borrow()
            .iter()
            .chain(self.last_listing_right.borrow().iter())
            .any(|entry| entry.name == name && entry.is_dir)
    }

    fn selected_names(&self) -> Vec<String> {
        if self.is_grid() {
            let mut names: Vec<String> = self
                .grid
                .selected_children()
                .into_iter()
                .filter_map(|child| flow_child_name(&child))
                .collect();
            if names.is_empty() && *self.split_enabled.borrow() {
                names = self
                    .grid_right
                    .selected_children()
                    .into_iter()
                    .filter_map(|child| flow_child_name(&child))
                    .collect();
            }
            names
        } else {
            let mut names = selected_list_names(&self.list);
            if names.is_empty() && *self.split_enabled.borrow() {
                names = selected_list_names(&self.list_right);
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
            let siblings = self.pane_siblings(false);
            let is_dir = siblings
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, d)| *d)
                .unwrap_or(false);
            let view = self.clone();
            dialogs::file_viewer(
                &win,
                self.ctx.clone(),
                &tab.remote,
                &next,
                name,
                is_dir,
                &siblings,
                Some(Rc::new(move |next_name: &str| {
                    view.ensure_name_selected(next_name, false);
                })),
            );
        }
    }

    fn pane_siblings(&self, primary: bool) -> Vec<(String, bool)> {
        let listing = if primary {
            self.last_listing.borrow()
        } else {
            self.last_listing_right.borrow()
        };
        listing::listing_siblings(&listing)
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
            if !listing::same_nav_location(&current.remote, &current.path, &current.remote, &next) {
                self.record_nav(&current.remote, &current.path);
            }
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
            let siblings = self.pane_siblings(true);
            let is_dir = siblings
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, d)| *d)
                .unwrap_or(false);
            let view = self.clone();
            dialogs::file_viewer(
                &win,
                self.ctx.clone(),
                &current.remote,
                &path,
                name,
                is_dir,
                &siblings,
                Some(Rc::new(move |next_name: &str| {
                    view.ensure_name_selected(next_name, true);
                })),
            );
        }
    }

    fn mkdir_with_selected(&self) {
        let names = self.selected_names();
        if names.is_empty() {
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.selectNewFolderItems",
                "Select items to move into a new folder",
            )));
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
                let mut ops = vec![crate::fileops::FileOp::Mkdir {
                    fs: fs.clone(),
                    path: folder_remote.clone(),
                }];
                for item in &names {
                    let src = join_remote_path(&current.path, item);
                    let src_remote = if current.remote == "local" {
                        src.trim_start_matches('/').to_string()
                    } else {
                        src
                    };
                    let dst_remote = join_remote_path(&folder_remote, item);
                    match client.move_file(&fs, &src_remote, &fs, &dst_remote) {
                        Ok(_) => ops.push(crate::fileops::FileOp::Move {
                            src_fs: fs.clone(),
                            src: src_remote,
                            dst_fs: fs.clone(),
                            dst: dst_remote,
                        }),
                        Err(e) => view.toast.add_toast(adw::Toast::new(&e.to_string())),
                    }
                }
                view.push_undo_ops(ops);
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
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.selectToDownload",
                "Select a file to download",
            )));
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
                    let mut paths = Vec::new();
                    for i in 0..files.n_items() {
                        if let Some(file) =
                            files.item(i).and_then(|o| o.downcast::<gio::File>().ok())
                        {
                            if let Some(path) = file.path() {
                                paths.push(path);
                            }
                        }
                    }
                    view.upload_local_paths(&paths);
                }
            },
        );
    }

    fn upload_local_paths(&self, paths: &[std::path::PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let Some(client) = self.ctx.client() else {
            return;
        };
        let current = self.current.borrow().clone();
        let (dest_fs, dest_dir) = fs_remote(&current.remote, &current.path);
        let items = match crate::fileops::collect_local_upload_items(paths, &dest_fs, &dest_dir) {
            Ok(items) => items,
            Err(e) => {
                self.toast.add_toast(adw::Toast::new(&e));
                return;
            }
        };
        if items.is_empty() {
            return;
        }
        for (fs, path) in crate::fileops::upload_dest_dirs(&items) {
            let _ = client.mkdir(&fs, &path);
        }
        match crate::fileops::start_grouped_transfers(&client, &items, "filemanager-upload") {
            Ok((group, ids)) => {
                self.remember_file_jobs_ex(
                    &ids,
                    "filemanager",
                    &group,
                    crate::jobs::transfer_snapshot_from_items(&items),
                );
                if let Some(id) = ids.first().copied() {
                    let bytes: u64 = items
                        .iter()
                        .map(|item| std::fs::metadata(&item.src).map(|m| m.len()).unwrap_or(0))
                        .sum();
                    let preparing = crate::jobs::preparing_job(
                        id,
                        &current.remote,
                        &items
                            .first()
                            .map(|item| item.src.clone())
                            .unwrap_or_default(),
                        &current.path,
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
                    self.ctx.persist();
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
                self.queue_job_undo(
                    &ids,
                    items
                        .iter()
                        .map(|item| crate::fileops::FileOp::Upload {
                            fs: item.dst_fs.clone(),
                            path: item.dst.clone(),
                            source: item.src.clone(),
                        })
                        .collect(),
                );
                self.ctx.refresh_runtime();
                self.reload();
                self.toast.add_toast(adw::Toast::new(&self.ctx.tf(
                    "nautilus.notifications.uploadStarted",
                    &[("count", &items.len().to_string())],
                )));
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
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

    fn toggle_sidebar(&self) {
        let next = !self.split.shows_sidebar();
        self.split.set_show_sidebar(next);
        if !self.split.is_collapsed() {
            self.ctx.settings.borrow_mut().nautilus.sidebar_visible = next;
            self.ctx.persist();
        }
    }

    fn close_sidebar_if_narrow(&self) {
        if self.is_narrow.get() {
            self.split.set_show_sidebar(false);
        }
    }

    fn hook_narrow_resize(&self, widget: &impl IsA<gtk::Widget>) {
        let Some(root) = widget.root() else {
            return;
        };
        {
            let view = self.clone();
            widget.add_tick_callback(move |widget, _| {
                view.sync_narrow_from_widget(widget);
                glib::ControlFlow::Continue
            });
        }
        if let Some(surface) = root.surface() {
            let view = self.clone();
            surface.connect_notify_local(Some("width"), move |surface, _| {
                view.sync_narrow_layout(surface.width());
            });
        }
        if let Ok(win) = root.downcast::<gtk::Window>() {
            let view = self.clone();
            win.connect_notify_local(Some("default-width"), move |win, _| {
                view.sync_narrow_layout(win.width());
            });
            let view = self.clone();
            win.connect_notify_local(Some("maximized"), move |win, _| {
                view.sync_narrow_layout(win.width());
            });
        }
    }

    fn sync_narrow_from_widget(&self, widget: &impl IsA<gtk::Widget>) {
        let width = widget
            .root()
            .map(|root| root.width())
            .filter(|w| *w > 0)
            .unwrap_or_else(|| widget.width());
        self.sync_narrow_layout(width);
    }

    fn sync_narrow_layout(&self, width: i32) {
        let narrow = crate::fileops::is_narrow_files_width(width);
        let overlay = crate::fileops::is_overlay_sidebar_width(width);
        if narrow == self.is_narrow.get()
            && self.bottom_bar.is_visible() == narrow
            && self.split.is_collapsed() == overlay
        {
            return;
        }
        self.is_narrow.set(narrow);
        self.split.set_collapsed(overlay);
        self.list.set_activate_on_single_click(narrow);
        self.list_right.set_activate_on_single_click(narrow);
        self.grid.set_activate_on_single_click(narrow);
        self.grid_right.set_activate_on_single_click(narrow);
        if overlay {
            self.split.set_show_sidebar(false);
        } else {
            self.split
                .set_show_sidebar(self.ctx.settings.borrow().nautilus.sidebar_visible);
        }
        self.sync_narrow_chrome();
    }

    fn sync_narrow_chrome(&self) {
        let picker = self.ctx.pending_picker.borrow().is_some();
        let narrow = self.is_narrow.get();
        self.bottom_bar.set_visible(narrow);
        self.bottom_confirm.set_visible(picker && narrow);
        self.layout_btn.set_visible(!narrow);
        self.icon_btn.set_visible(!narrow);
        self.actions_btn.set_visible(!narrow);
        if picker {
            self.picker_bar.set_visible(!narrow);
        } else {
            self.picker_bar.set_visible(false);
        }
        self.sync_send_to_button();
    }

    fn toggle_split(&self) {
        let next = !*self.split_enabled.borrow();
        *self.split_enabled.borrow_mut() = next;
        self.ctx.settings.borrow_mut().nautilus.split_enabled = next;
        self.ctx.persist();
        self.right_host.set_visible(next);
        if next {
            let saved_remote = self
                .ctx
                .settings
                .borrow()
                .nautilus
                .split_secondary_remote
                .clone();
            let saved_path = self
                .ctx
                .settings
                .borrow()
                .nautilus
                .split_secondary_path
                .clone();
            if !saved_remote.is_empty() {
                self.secondary.borrow_mut().remote = saved_remote;
                self.secondary.borrow_mut().path = saved_path;
            } else {
                *self.secondary.borrow_mut() = self.current.borrow().clone();
                self.persist_split_location();
            }
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

    fn reload_pane(&self, _tab: &TabState) {
        self.begin_listing(false);
    }

    fn list_group(&self, primary: bool) -> &str {
        if primary {
            &self.list_group_left
        } else {
            &self.list_group_right
        }
    }

    fn listing_generation(&self, primary: bool) -> u64 {
        if primary {
            self.list_gen_left.get()
        } else {
            self.list_gen_right.get()
        }
    }

    fn bump_listing_generation(&self, primary: bool) -> u64 {
        let cell = if primary {
            &self.list_gen_left
        } else {
            &self.list_gen_right
        };
        let next = cell.get().wrapping_add(1);
        cell.set(next);
        next
    }

    fn set_list_job(&self, primary: bool, jobid: Option<u64>) {
        if primary {
            self.list_job_left.set(jobid);
        } else {
            self.list_job_right.set(jobid);
        }
    }

    fn stop_list_group(&self, primary: bool) {
        self.bump_listing_generation(primary);
        self.set_list_job(primary, None);
        if let Some(client) = self.ctx.client() {
            let _ = client.job_stop_group(self.list_group(primary));
        }
    }

    fn set_listing_loading(&self, primary: bool, loading: bool) {
        let overlay = if primary {
            &self.loading_left
        } else {
            &self.loading_right
        };
        overlay.set_visible(loading);
        if let Some(spinner) = overlay
            .first_child()
            .and_then(|child| child.downcast::<gtk::Spinner>().ok())
        {
            spinner.set_spinning(loading);
        }
    }

    fn cancel_listing(&self, primary: bool) {
        self.stop_list_group(primary);
        self.set_listing_loading(primary, false);
        self.status.set_text(
            &self
                .ctx
                .t_or("provision.stages.cancelled", "Operation cancelled"),
        );
    }

    fn pane_list_target(&self, primary: bool) -> (String, String) {
        let tab = if primary {
            self.current.borrow().clone()
        } else {
            self.secondary.borrow().clone()
        };
        listing::list_target(&tab.remote, &tab.path)
    }

    fn begin_listing(&self, primary: bool) {
        if primary {
            clear_list(&self.list);
            clear_flow(&self.grid);
        } else {
            clear_list(&self.list_right);
            clear_flow(&self.grid_right);
        }
        let gen = {
            self.stop_list_group(primary);
            self.listing_generation(primary)
        };
        self.set_listing_loading(primary, true);
        let Some(client) = self.ctx.client() else {
            self.finish_listing_error(
                primary,
                &self
                    .ctx
                    .t_or("fileBrowser.errors.connectionFailed", "Connection Failed"),
            );
            return;
        };
        let (fs, remote_path) = self.pane_list_target(primary);
        match listing::start_list_dir(&client, &fs, &remote_path, self.list_group(primary)) {
            Ok(ListStart::Ready(entries)) => self.finish_listing_ok(primary, entries),
            Ok(ListStart::Job(jobid)) => {
                self.set_list_job(primary, Some(jobid));
                self.schedule_list_poll(primary, gen);
            }
            Err(err) => {
                self.toast.add_toast(adw::Toast::new(&err.to_string()));
                self.finish_listing_error(primary, &err.to_string());
            }
        }
    }

    fn schedule_list_poll(&self, primary: bool, gen: u64) {
        let view = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if view.listing_generation(primary) != gen {
                return glib::ControlFlow::Break;
            }
            let Some(jobid) = (if primary {
                view.list_job_left.get()
            } else {
                view.list_job_right.get()
            }) else {
                return glib::ControlFlow::Break;
            };
            let Some(client) = view.ctx.client() else {
                view.finish_listing_error(
                    primary,
                    &view
                        .ctx
                        .t_or("fileBrowser.errors.connectionFailed", "Connection Failed"),
                );
                return glib::ControlFlow::Break;
            };
            match listing::poll_list_job(&client, jobid) {
                ListJobState::Running => glib::ControlFlow::Continue,
                ListJobState::Finished(entries) => {
                    if view.listing_generation(primary) == gen {
                        view.finish_listing_ok(primary, entries);
                    }
                    glib::ControlFlow::Break
                }
                ListJobState::Cancelled => {
                    if view.listing_generation(primary) == gen {
                        view.set_listing_loading(primary, false);
                        view.set_list_job(primary, None);
                        view.status.set_text(
                            &view
                                .ctx
                                .t_or("provision.stages.cancelled", "Operation cancelled"),
                        );
                    }
                    glib::ControlFlow::Break
                }
                ListJobState::Failed(err) => {
                    if view.listing_generation(primary) == gen {
                        view.toast.add_toast(adw::Toast::new(&err));
                        view.finish_listing_error(primary, &err);
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn filter_listing_entries(&self, mut entries: Vec<DirEntry>, primary: bool) -> Vec<DirEntry> {
        if !self.ctx.settings.borrow().nautilus.show_hidden {
            entries.retain(|e| !e.name.starts_with('.'));
        }
        if primary {
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
        entries
    }

    fn finish_listing_ok(&self, primary: bool, entries: Vec<DirEntry>) {
        self.set_list_job(primary, None);
        self.set_listing_loading(primary, false);
        let entries = self.filter_listing_entries(entries, primary);
        self.populate_entries(&entries, primary);
        if primary {
            self.refresh_listing_status();
        }
    }

    fn finish_listing_error(&self, primary: bool, message: &str) {
        self.set_list_job(primary, None);
        self.set_listing_loading(primary, false);
        self.show_listing_error(message, primary);
    }

    fn show_listing_error(&self, message: &str, primary: bool) {
        let title = self
            .ctx
            .t_or("fileBrowser.errors.loadFailed", "Failed to load directory");
        let retry_label = self.ctx.t_or("common.retry", "Retry");
        let retry = gtk::Button::with_label(&retry_label);
        retry.add_css_class("suggested-action");
        retry.set_valign(gtk::Align::Center);
        {
            let view = self.clone();
            retry.connect_clicked(move |_| {
                if primary {
                    view.reload();
                } else {
                    let tab = view.secondary.borrow().clone();
                    view.reload_pane(&tab);
                }
            });
        }
        if self.is_grid() {
            let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
            box_.set_widget_name("listing-error");
            box_.set_margin_top(16);
            box_.set_margin_start(16);
            box_.set_margin_end(16);
            let label = gtk::Label::new(Some(&format!("{title}\n{message}")));
            label.set_wrap(true);
            label.set_xalign(0.0);
            box_.append(&label);
            box_.append(&retry);
            let grid = if primary {
                &self.grid
            } else {
                &self.grid_right
            };
            grid.insert(&box_, -1);
        } else {
            let row = adw::ActionRow::new();
            row.set_title(&title);
            row.set_subtitle(message);
            row.add_suffix(&retry);
            let list = if primary {
                &self.list
            } else {
                &self.list_right
            };
            list.append(&row);
        }
        self.status.set_text(message);
    }

    pub fn poll_refresh(&self) {
        if self.listing_menu_open.get() {
            return;
        }
        self.settle_pending_undos();
        let jobs = self.ctx.snapshot.borrow().jobs.clone();
        let history = {
            let store = self.ctx.store.borrow();
            crate::jobs::history_with_meta(&store.job_history, &store.job_meta)
        };
        let sig = crate::jobs::ops_panel_signature(&jobs, &history);
        if *self.ops_sig.borrow() != sig {
            *self.ops_sig.borrow_mut() = sig;
            self.reload_ops();
        }
        let previous = self.last_poll_jobs.borrow().clone();
        let finished = crate::jobs::terminal_job_transitions(&previous, &jobs);
        *self.last_poll_jobs.borrow_mut() = jobs;
        if finished.is_empty() {
            return;
        }
        let affected: Vec<crate::jobs::AffectedListing> = finished
            .iter()
            .flat_map(crate::jobs::job_affected_listings)
            .collect();
        if affected.is_empty() {
            return;
        }
        let current = self.current.borrow().clone();
        let mut open = vec![(current.remote.clone(), current.path.clone())];
        let split_on = *self.split_enabled.borrow();
        if split_on {
            let secondary = self.secondary.borrow().clone();
            open.push((secondary.remote, secondary.path));
        }
        let current_id = current.id;
        for tab in self.tabs.borrow().iter() {
            if tab.id != current_id {
                open.push((tab.remote.clone(), tab.path.clone()));
            }
        }
        let need = crate::jobs::open_listings_needing_refresh(&open, &affected);
        let refresh_primary = !current.starred && need.contains(&0);
        let refresh_secondary = split_on && need.contains(&1);
        if refresh_primary {
            self.reload();
        } else if refresh_secondary {
            let secondary = self.secondary.borrow().clone();
            self.reload_pane(&secondary);
        }
    }

    fn queue_job_undo(&self, ids: &[u64], ops: Vec<crate::fileops::FileOp>) {
        let Some(pending) = crate::fileops::pending_undo_from_ops(ids, &ops) else {
            return;
        };
        if ids.is_empty() {
            self.push_undo(pending.token);
            return;
        }
        self.pending_undo.borrow_mut().push(pending);
    }

    fn settle_pending_undos(&self) {
        let jobs = self.ctx.snapshot.borrow().jobs.clone();
        let history = self.ctx.store.borrow().job_history.clone();
        let statuses = crate::fileops::merge_job_statuses(
            jobs.iter().map(|job| (job.id, job.status.as_str())),
            history.iter().map(|job| (job.id, job.status.as_str())),
        );
        let pending = self.pending_undo.borrow().clone();
        let (commit, keep) = crate::fileops::settle_pending_undos(pending, &statuses);
        *self.pending_undo.borrow_mut() = keep;
        for token in commit {
            self.push_undo(token);
        }
    }

    fn push_undo(&self, op: String) {
        crate::fileops::push_capped(&mut self.undo.borrow_mut(), op);
        self.redo.borrow_mut().clear();
    }

    fn push_undo_ops(&self, ops: Vec<crate::fileops::FileOp>) {
        if let Some(token) = crate::fileops::encode_undo(&ops) {
            self.push_undo(token);
        }
    }

    fn undo_last(&self) {
        let Some(op) = self.undo.borrow_mut().pop() else {
            self.toast.add_toast(adw::Toast::new(
                &self
                    .ctx
                    .t_or("nautilus.notifications.nothingToUndo", "Nothing to undo"),
            ));
            return;
        };
        self.invert_file_op(&op);
        self.redo.borrow_mut().push(op);
        self.toast.add_toast(adw::Toast::new(
            &self
                .ctx
                .t_or("nautilus.notifications.undoComplete", "Undid last action"),
        ));
    }

    fn redo_last(&self) {
        let Some(op) = self.redo.borrow_mut().pop() else {
            self.toast.add_toast(adw::Toast::new(
                &self
                    .ctx
                    .t_or("nautilus.notifications.nothingToRedo", "Nothing to redo"),
            ));
            return;
        };
        self.replay_file_op(&op);
        self.undo.borrow_mut().push(op);
        self.toast.add_toast(adw::Toast::new(
            &self
                .ctx
                .t_or("nautilus.notifications.redoComplete", "Redid last action"),
        ));
    }

    fn invert_file_op(&self, op: &str) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        let Some(decoded) = crate::fileops::decode_undo(op) else {
            return;
        };
        match crate::fileops::invert_ops(&decoded) {
            Some(inv) => {
                if let Err(e) = crate::fileops::apply_ops(&client, &inv) {
                    self.toast.add_toast(adw::Toast::new(&e));
                }
            }
            None => self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.cannotUndo",
                "This action cannot be undone",
            ))),
        }
        self.reload();
    }

    fn replay_file_op(&self, op: &str) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        if let Some(decoded) = crate::fileops::decode_undo(op) {
            if let Err(e) = crate::fileops::apply_ops(&client, &decoded) {
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
                            let paths: Vec<std::path::PathBuf> = list
                                .files()
                                .into_iter()
                                .filter_map(|file| file.path())
                                .collect();
                            if !paths.is_empty() {
                                view.upload_local_paths(&paths);
                                return;
                            }
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
            if let Some(items) = crate::fileops::parse_manager_clipboard(&text) {
                *view.clipboard.borrow_mut() = items;
                view.paste();
                return;
            }
            let mut paths = Vec::new();
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
                if path.exists() {
                    paths.push(path);
                }
            }
            view.upload_local_paths(&paths);
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

    fn sync_send_to_button(&self) {
        let current = self.current.borrow().clone();
        let show = !self.is_narrow.get() && current.remote != "local" && !current.starred;
        self.send_to_btn.set_visible(show);
        if !show {
            return;
        }
        let registered =
            crate::platform::is_send_to_registered(&current.remote, Some(&current.path));
        let label = if registered {
            self.ctx.t_or(
                "nautilus.contextMenu.removeFromSendTo",
                "Remove from File Manager Menu",
            )
        } else {
            self.ctx.t_or(
                "nautilus.contextMenu.addToSendTo",
                "Add to File Manager Menu",
            )
        };
        self.send_to_btn.set_icon_name(if registered {
            "list-remove-symbolic"
        } else {
            "list-add-symbolic"
        });
        self.send_to_btn.set_tooltip_text(Some(&label));
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
        let path_param = crate::platform::send_to_path_param(Some(&current.path));
        let params = [
            ("remote", current.remote.as_str()),
            ("path", path_param.as_str()),
        ];
        match result {
            Ok(_) => self.toast.add_toast(adw::Toast::new(&if registered {
                self.ctx.tf_or(
                    "nautilus.notifications.sendToRemoved",
                    "Removed '{{remote}}{{path}}' from File Manager menu",
                    &params,
                )
            } else {
                self.ctx.tf_or(
                    "nautilus.notifications.sendToAdded",
                    "Added '{{remote}}{{path}}' to File Manager menu",
                    &params,
                )
            })),
            Err(e) => self.toast.add_toast(adw::Toast::new(&self.ctx.tf_or(
                "nautilus.errors.sendToFailed",
                "Failed to update File Manager menu: {{error}}",
                &[("error", &e)],
            ))),
        }
        self.sync_send_to_button();
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
                    let is_dir = view
                        .last_listing
                        .borrow()
                        .iter()
                        .any(|entry| entry.name == old && entry.is_dir);
                    let item = crate::fileops::RenameItem {
                        fs,
                        from: src,
                        to: dst,
                        is_dir,
                    };
                    match crate::fileops::start_grouped_renames(
                        &client,
                        &[item.clone()],
                        "filemanager",
                    ) {
                        Ok((_group, ids)) => {
                            view.remember_file_jobs(&ids, "filemanager");
                            view.queue_job_undo(&ids, vec![item.file_op()]);
                            view.ctx.refresh_runtime();
                            view.reload();
                        }
                        Err(e) => view.toast.add_toast(adw::Toast::new(&e)),
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
        let listing = self.last_listing.borrow().clone();
        let mut items = Vec::new();
        let mut undos = Vec::new();
        for name in names {
            let path = join_remote_path(&current.path, &name);
            let (fs, remote) = fs_remote(&current.remote, &path);
            let is_dir = listing
                .iter()
                .any(|entry| entry.name == name && entry.is_dir);
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
            let item = crate::fileops::DeleteItem {
                fs,
                path: remote,
                is_dir,
            };
            undos.push(item.file_op(trash));
            items.push(item);
        }
        match crate::fileops::start_grouped_deletes(&client, &items, "filemanager") {
            Ok((_group, ids)) => {
                self.remember_file_jobs(&ids, "filemanager");
                self.queue_job_undo(&ids, undos);
                self.ctx.refresh_runtime();
                self.reload();
            }
            Err(e) => self.toast.add_toast(adw::Toast::new(&e)),
        }
    }

    fn cut_or_copy(&self, cut: bool) {
        let items = self.selection_clipboard_items(cut);
        if items.is_empty() {
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.nothingSelected",
                "Select a file or folder first",
            )));
            return;
        }
        *self.clipboard.borrow_mut() = items.clone();
        if let Some(display) = gtk::gdk::Display::default() {
            display
                .clipboard()
                .set_text(&crate::fileops::encode_manager_clipboard(&items));
        }
        self.toast.add_toast(adw::Toast::new(&if cut {
            self.ctx.t_or("nautilus.contextMenu.cut", "Cut")
        } else {
            self.ctx.t_or("common.copy", "Copied")
        }));
        self.restyle_listings();
    }

    fn entry_is_cut(&self, tab: &TabState, name: &str) -> bool {
        crate::fileops::clipboard_marks_cut(
            &self.clipboard.borrow(),
            &tab.remote,
            &join_remote_path(&tab.path, name),
        )
    }

    fn entry_icon(&self, entry: &DirEntry, tab: &TabState) -> gtk::Image {
        let cut = self.entry_is_cut(tab, &entry.name);
        let icon = gtk::Image::from_icon_name(&if cut {
            "edit-cut-symbolic".to_string()
        } else {
            crate::mime::icon_for_entry(&entry.name, entry.is_dir, &entry.mime)
        });
        if cut {
            icon.add_css_class("cut-icon");
        }
        icon
    }

    fn restyle_listings(&self) {
        let left = self.last_listing.borrow().clone();
        clear_list(&self.list);
        clear_flow(&self.grid);
        self.listing_shown.set(0);
        self.populate_entries(&left, true);
        if *self.split_enabled.borrow() {
            let right = self.last_listing_right.borrow().clone();
            clear_list(&self.list_right);
            clear_flow(&self.grid_right);
            self.listing_shown_right.set(0);
            self.populate_entries(&right, false);
        }
    }

    fn paste(&self) {
        let items = self.clipboard.borrow().clone();
        if items.is_empty() {
            self.paste_system_clipboard();
            return;
        }
        let current = self.current.borrow().clone();
        let listing_dirs: Vec<String> = self
            .last_listing
            .borrow()
            .iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| entry.name.clone())
            .collect();
        let dest_dir = crate::fileops::paste_dest_dir(
            &current.remote,
            &current.path,
            &self.selected_names(),
            &listing_dirs,
            &items,
        );
        let transfers =
            crate::fileops::transfers_from_clipboard(&items, &current.remote, &dest_dir);
        self.start_grouped_file_transfers(transfers);
    }

    fn selection_from_secondary(&self) -> bool {
        crate::fileops::clipboard_uses_secondary(
            *self.split_enabled.borrow(),
            self.primary_has_selection(),
            self.secondary_has_selection(),
        )
    }

    fn primary_has_selection(&self) -> bool {
        if self.is_grid() {
            !self.grid.selected_children().is_empty()
        } else {
            !selected_list_names(&self.list).is_empty()
        }
    }

    fn secondary_has_selection(&self) -> bool {
        if self.is_grid() {
            !self.grid_right.selected_children().is_empty()
        } else {
            !selected_list_names(&self.list_right).is_empty()
        }
    }

    fn selection_clipboard_items(&self, cut: bool) -> Vec<(String, String, bool, bool)> {
        let secondary = self.selection_from_secondary();
        let tab = if secondary {
            self.secondary.borrow().clone()
        } else {
            self.current.borrow().clone()
        };
        let listing = if secondary {
            self.last_listing_right.borrow().clone()
        } else {
            self.last_listing.borrow().clone()
        };
        self.selected_names()
            .into_iter()
            .map(|name| {
                let is_dir = listing
                    .iter()
                    .find(|entry| entry.name == name)
                    .map(|entry| entry.is_dir)
                    .unwrap_or(false);
                (
                    tab.remote.clone(),
                    join_remote_path(&tab.path, &name),
                    cut,
                    is_dir,
                )
            })
            .collect()
    }

    fn copy_or_move_to(&self, cut: bool) {
        let items = self.selection_clipboard_items(cut);
        if items.is_empty() {
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.nothingSelected",
                "Select a file or folder first",
            )));
            return;
        }
        let current = self.current.borrow().clone();
        let mut config = crate::picker::FilePickerConfig::folders();
        config.initial_location = Some(if current.remote == "local" {
            if current.path.is_empty() {
                "/".into()
            } else {
                current.path.clone()
            }
        } else if current.path.is_empty() {
            format!("{}:", current.remote)
        } else {
            format!("{}:{}", current.remote, current.path)
        });
        let view = self.clone();
        self.ctx.request_picker(
            config,
            Rc::new(move |result| {
                if result.cancelled {
                    return;
                }
                view.start_transfers_to(&items, &result.remote, &result.path);
            }),
        );
    }

    fn start_transfers_to(
        &self,
        items: &[(String, String, bool, bool)],
        dest_remote: &str,
        dest_dir: &str,
    ) {
        let transfers = crate::fileops::transfers_from_clipboard(items, dest_remote, dest_dir);
        self.start_grouped_file_transfers(transfers);
    }

    fn start_grouped_file_transfers(&self, transfers: Vec<crate::fileops::TransferItem>) {
        let Some(client) = self.ctx.client() else {
            return;
        };
        match crate::fileops::start_grouped_transfers(&client, &transfers, "filemanager") {
            Ok((group, ids)) => {
                self.remember_file_jobs_ex(
                    &ids,
                    "filemanager",
                    &group,
                    crate::jobs::transfer_snapshot_from_items(&transfers),
                );
                self.queue_job_undo(&ids, transfers.iter().map(|item| item.file_op()).collect());
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
            tab.history = self.history.borrow().clone();
            tab.future = self.future.borrow().clone();
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
            let middle = gtk::GestureClick::new();
            middle.set_button(2);
            {
                let view = self.clone();
                let id = tab.id;
                middle.connect_pressed(move |g, _, _, _| {
                    view.close_tab(id);
                    g.set_state(gtk::EventSequenceState::Claimed);
                });
            }
            btn.add_controller(middle);
            self.attach_tab_reorder(&btn, id);
            self.attach_internal_drop(&btn, InternalDrop::Tab(id));
            self.tab_bar.append(&btn);
            if tab.id == current_id {
                let view = self.clone();
                let btn = btn.clone();
                glib::idle_add_local_once(move || view.scroll_tab_into_view(&btn));
            }
        }
    }

    fn scroll_tab_into_view(&self, button: &impl IsA<gtk::Widget>) {
        let Some(scroll) = self.tab_bar.parent().and_downcast::<gtk::ScrolledWindow>() else {
            return;
        };
        let Some(bounds) = button.compute_bounds(&self.tab_bar) else {
            return;
        };
        let adj = scroll.hadjustment();
        let next = crate::fileops::scroll_child_into_view(
            adj.value(),
            adj.page_size(),
            f64::from(bounds.x()),
            f64::from(bounds.x() + bounds.width()),
        );
        if (next - adj.value()).abs() > f64::EPSILON {
            adj.set_value(next);
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
        self.persist_nav_stacks();
        self.tabs.borrow_mut().push(copy.clone());
        self.adopt_tab(copy);
        self.reload();
    }

    fn close_other_tabs(&self, keep: u32) {
        self.persist_nav_stacks();
        self.tabs.borrow_mut().retain(|t| t.id == keep);
        if let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == keep).cloned() {
            self.adopt_tab(tab);
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
                self.adopt_tab(tab);
            }
        }
        self.reload();
    }

    fn persist_nav_stacks(&self) {
        let current = self.current.borrow().clone();
        if let Some(tab) = self
            .tabs
            .borrow_mut()
            .iter_mut()
            .find(|t| t.id == current.id)
        {
            tab.remote = current.remote;
            tab.path = current.path;
            tab.starred = current.starred;
            tab.history = self.history.borrow().clone();
            tab.future = self.future.borrow().clone();
        }
    }

    fn activate_tab(&self, id: u32) {
        if self.current.borrow().id == id {
            return;
        }
        self.persist_nav_stacks();
        if let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == id).cloned() {
            self.adopt_tab(tab);
            self.reload();
        }
    }

    fn adopt_tab(&self, tab: TabState) {
        *self.history.borrow_mut() = tab.history.clone();
        *self.future.borrow_mut() = tab.future.clone();
        *self.current.borrow_mut() = tab;
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
        self.persist_nav_stacks();
        let mut current = self.current.borrow().clone();
        let id = *self.next_tab_id.borrow();
        *self.next_tab_id.borrow_mut() = id + 1;
        current.id = id;
        current.history.clear();
        current.future.clear();
        current.title = format!("{}:{}", current.remote, current.path)
            .rsplit('/')
            .next()
            .unwrap_or("tab")
            .to_string();
        self.tabs.borrow_mut().push(current.clone());
        *self.history.borrow_mut() = Vec::new();
        *self.future.borrow_mut() = Vec::new();
        *self.current.borrow_mut() = current;
        self.reload();
    }

    fn close_current_tab(&self) {
        let id = self.current.borrow().id;
        self.close_tab(id);
    }

    fn close_tab(&self, id: u32) {
        self.tabs.borrow_mut().retain(|t| t.id != id);
        if self.tabs.borrow().is_empty() {
            self.open_new_tab();
            return;
        }
        if self.current.borrow().id == id {
            if let Some(next) = self.tabs.borrow().first().cloned() {
                self.adopt_tab(next);
            }
        }
        self.reload();
    }

    pub fn detach_current_tab(&self) {
        let id = self.current.borrow().id;
        self.detach_tab(id);
    }

    fn detach_tab(&self, id: u32) {
        let Some(tab) = self.tabs.borrow().iter().find(|t| t.id == id).cloned() else {
            return;
        };
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(app) = win.application().and_downcast::<adw::Application>() else {
            return;
        };
        let files = self.ctx.t_or("nautilus.titles.files", "Files");
        let title = self.ctx.tf_or(
            "nautilus.titles.detachedTab",
            "{app} — {title}",
            &[("app", &files), ("title", &tab.title)],
        );
        super::window::present_files_overlay(&app, &self.ctx, &tab.remote, &tab.path, Some(&title));
        if self.tabs.borrow().len() > 1 {
            self.close_tab(id);
        }
    }

    fn popup_context(&self) {
        let list: gtk::Widget = if self.is_grid() {
            self.grid.clone().upcast()
        } else {
            self.list.clone().upcast()
        };
        self.popup_context_at(&list, 8.0, 8.0);
    }

    /// Wait until the pointer click has finished so GtkPopover autohide
    /// does not treat that same click as an outside dismiss.
    fn schedule_listing_menu(
        &self,
        host: gtk::Widget,
        x: f64,
        y: f64,
        open: impl FnOnce(&Self, &gtk::Widget, f64, f64) + 'static,
    ) {
        let view = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            open(&view, &host, x, y);
        });
    }

    fn listing_popover_host(&self, widget: &gtk::Widget) -> gtk::Widget {
        widget
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .map(|window| window.upcast::<gtk::Widget>())
            .or_else(|| {
                widget
                    .ancestor(gtk::Window::static_type())
                    .and_then(|w| w.downcast::<gtk::Widget>().ok())
            })
            .unwrap_or_else(|| self.root.clone().upcast())
    }

    fn popup_context_at(&self, widget: &impl IsA<gtk::Widget>, x: f64, y: f64) {
        let widget = widget.upcast_ref::<gtk::Widget>();
        let host = self.listing_popover_host(widget);
        let (px, py) = widget
            .compute_bounds(&host)
            .map(|bounds| {
                crate::fileops::pointing_in_parent(
                    x,
                    y,
                    f64::from(bounds.x()),
                    f64::from(bounds.y()),
                )
            })
            .unwrap_or((x.round() as i32, y.round() as i32));
        if self.listing_popover.parent().as_ref() != Some(&host) {
            self.listing_popover.unparent();
            self.listing_popover.set_parent(&host);
        }
        self.listing_menu_open.set(true);
        self.listing_popover
            .set_child(Some(&self.build_context_menu(&self.listing_popover)));
        self.listing_popover
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(px, py, 1, 1)));
        self.listing_popover.popup();
    }

    fn listing_action_label(&self, action: &str, multi: bool, send_registered: bool) -> String {
        match action {
            "open" => self.ctx.t_or("nautilus.contextMenu.open", "Open"),
            "open_submenu" => self.ctx.t_or("nautilus.contextMenu.open", "Open"),
            "native" => self
                .ctx
                .t_or("nautilus.contextMenu.openNative", "Open native"),
            "tab" => self
                .ctx
                .t_or("nautilus.contextMenu.openNewTab", "Open in New Tab"),
            "window" => self
                .ctx
                .t_or("nautilus.contextMenu.openNewWindow", "Open in New Window"),
            "reload" => self.ctx.t_or("nautilus.contextMenu.refresh", "Refresh"),
            "copy" => self.ctx.t_or("nautilus.contextMenu.copy", "Copy"),
            "cut" => self.ctx.t_or("nautilus.contextMenu.cut", "Cut"),
            "copyto" => self.ctx.t_or("nautilus.contextMenu.copyTo", "Copy to…"),
            "moveto" => self.ctx.t_or("nautilus.contextMenu.moveTo", "Move to…"),
            "paste" => self.ctx.t_or("nautilus.contextMenu.paste", "Paste"),
            "copypath" => self.ctx.t_or("nautilus.contextMenu.copyPath", "Copy Path"),
            "public" => self
                .ctx
                .t_or("nautilus.contextMenu.copyPublicLink", "Copy Public Link"),
            "copyurl" => self
                .ctx
                .t_or("nautilus.contextMenu.copyUrl", "Copy URL into folder…"),
            "rename" if multi => self
                .ctx
                .t_or("nautilus.contextMenu.renameMultiple", "Rename Multiple..."),
            "rename" => self.ctx.t_or("nautilus.contextMenu.rename", "Rename"),
            "delete" => self.ctx.t_or("nautilus.contextMenu.delete", "Delete"),
            "props" => self
                .ctx
                .t_or("nautilus.contextMenu.properties", "Properties"),
            "download" => self.ctx.t_or("nautilus.contextMenu.download", "Download…"),
            "mkdir" => self
                .ctx
                .t_or("nautilus.contextMenu.newFolder", "New Folder"),
            "mkdirsel" => self.ctx.t_or(
                "nautilus.contextMenu.createFolderWithItems",
                "New Folder with Selection...",
            ),
            "upload" => self
                .ctx
                .t_or("nautilus.contextMenu.uploadFiles", "Upload Files"),
            "uploaddir" => self
                .ctx
                .t_or("nautilus.contextMenu.uploadFolder", "Upload Folder"),
            "star" => self.ctx.t_or("nautilus.contextMenu.star", "Star / Unstar"),
            "bookmark" => self.ctx.t_or("nautilus.contextMenu.bookmark", "Bookmark"),
            "archive" => self.ctx.t_or("nautilus.contextMenu.compress", "Compress"),
            "archivelist" => self.ctx.t_or(
                "nautilus.contextMenu.browseArchive",
                "Browse archive contents",
            ),
            "extract" => self
                .ctx
                .t_or("nautilus.contextMenu.extract", "Extract archive…"),
            "rmdirs" => self.ctx.t_or(
                "nautilus.contextMenu.removeEmptyDirs",
                "Remove empty folders",
            ),
            "cleanup" => self
                .ctx
                .t_or("nautilus.contextMenu.emptyTrash", "Empty Trash"),
            "sendto" if send_registered => self.ctx.t_or(
                "nautilus.contextMenu.removeFromSendTo",
                "Remove from File Manager Menu",
            ),
            "sendto" => self.ctx.t_or(
                "nautilus.contextMenu.addToSendTo",
                "Add to File Manager Menu",
            ),
            "share" => self.ctx.t_or("nautilus.contextMenu.share", "Share"),
            "undo" => self.ctx.t_or("nautilus.contextMenu.undo", "Undo"),
            "redo" => self.ctx.t_or("nautilus.contextMenu.redo", "Redo"),
            "syspaste" => self.ctx.t_or(
                "nautilus.contextMenu.pasteSystem",
                "Paste from system clipboard",
            ),
            "detach" => self
                .ctx
                .t_or("nautilus.contextMenu.detachTab", "Detach Tab"),
            "back" => self.ctx.t_or("common.back", "Back"),
            _ => action.to_string(),
        }
    }

    fn activate_listing_action(&self, action: &str) {
        match action {
            "open" => {
                if let Some(name) = self.selected_name() {
                    self.open_name(&name);
                }
            }
            "native" => self.open_native_selected(),
            "tab" => self.open_selected_in_new_tab(),
            "window" => self.open_selected_in_new_window(),
            "reload" => self.reload(),
            "copy" => self.cut_or_copy(false),
            "cut" => self.cut_or_copy(true),
            "copyto" => self.copy_or_move_to(false),
            "moveto" => self.copy_or_move_to(true),
            "paste" => self.paste(),
            "copypath" => self.copy_selected_path(),
            "public" => self.copy_public_link(),
            "copyurl" => self.copy_url_prompt(),
            "rename" => self.rename_selected(),
            "delete" => self.delete_selected(),
            "props" => self.properties_selected(),
            "mkdir" => self.mkdir_prompt(),
            "mkdirsel" => self.mkdir_with_selected(),
            "download" => self.download_selected(),
            "archivelist" => {
                if let Some(name) = self.selected_name() {
                    self.open_name(&name);
                }
            }
            "upload" => self.upload_prompt(),
            "uploaddir" => self.upload_folder_prompt(),
            "togglestar" | "star" => self.toggle_star_selected(),
            "bookmark" => self.add_bookmark(),
            "extract" => self.extract_selected(),
            "rmdirs" => self.remove_empty_dirs(),
            "cleanup" => self.cleanup_remote(),
            "archive" => {
                if let Some(win) = self.root.root().and_downcast::<gtk::Window>() {
                    let current = self.current.borrow().clone();
                    let names = self.selected_names();
                    if names.is_empty() {
                        self.toast.add_toast(adw::Toast::new(
                            &self
                                .ctx
                                .t_or("nautilus.errors.minSelection", "Select items to archive"),
                        ));
                    } else {
                        dialogs::archive_create(
                            &win,
                            self.ctx.clone(),
                            &current.remote,
                            &current.path,
                            &names,
                        );
                    }
                }
            }
            "share" => self.share_selected(),
            "sendto" => self.toggle_send_to(),
            "undo" => self.undo_last(),
            "redo" => self.redo_last(),
            "syspaste" => self.paste_system_clipboard(),
            "detach" => self.detach_current_tab(),
            _ => {}
        }
    }

    fn listing_menu_page(
        &self,
        popover: &gtk::Popover,
        actions: &[&str],
        stack: Option<&gtk::Stack>,
        multi: bool,
        send_registered: bool,
    ) -> gtk::Box {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 4);
        box_.set_margin_top(8);
        box_.set_margin_bottom(8);
        box_.set_margin_start(8);
        box_.set_margin_end(8);
        for action in actions {
            if *action == "sep" {
                box_.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                continue;
            }
            let label = self.listing_action_label(action, multi, send_registered);
            let btn = gtk::Button::with_label(&label);
            if *action == "open_submenu" {
                btn.set_tooltip_text(Some(&self.ctx.t_or(
                    "nautilus.contextMenu.openNewTab",
                    "Open in New Tab / Window",
                )));
            }
            let view = self.clone();
            let popover = popover.clone();
            let action = (*action).to_string();
            let stack = stack.cloned();
            btn.connect_clicked(move |_| {
                if action == "open_submenu" {
                    if let Some(stack) = &stack {
                        stack.set_visible_child_name("open");
                    }
                    return;
                }
                if action == "back" {
                    if let Some(stack) = &stack {
                        stack.set_visible_child_name("main");
                    }
                    return;
                }
                popover.popdown();
                view.activate_listing_action(&action);
            });
            box_.append(&btn);
        }
        box_
    }

    fn build_context_menu(&self, popover: &gtk::Popover) -> gtk::Widget {
        let selected = self.selected_names();
        let current = self.current.borrow().clone();
        let info = self.ctx.fs_info(&current.remote);
        let first_is_dir = selected
            .first()
            .is_some_and(|name| self.selected_is_directory(name));
        let kind = crate::fileops::ListingMenuKind::from_selection(selected.len(), first_is_dir);
        let flags = crate::fileops::ListingMenuFlags {
            has_clipboard: !self.clipboard.borrow().is_empty(),
            public_ok: info.as_ref().is_none_or(|i| i.has_feature("PublicLink"))
                && !selected.is_empty(),
            cleanup_ok: info.as_ref().is_none_or(|i| i.has_feature("CleanUp")),
            archive_selected: selected.iter().any(|name| {
                matches!(
                    crate::mime::category_for_entry(name, false, ""),
                    FileTypeCategory::Archive
                )
            }),
            can_undo: !self.undo.borrow().is_empty(),
            can_redo: !self.redo.borrow().is_empty(),
        };
        let actions = crate::fileops::listing_context_actions(kind, flags);
        let send_registered =
            crate::platform::is_send_to_registered(&current.remote, Some(&current.path));
        let multi = selected.len() > 1;
        let stack = gtk::Stack::new();
        let main = self.listing_menu_page(popover, &actions, Some(&stack), multi, send_registered);
        stack.add_named(&main, Some("main"));
        if kind == crate::fileops::ListingMenuKind::SingleFolder {
            let mut open_actions = vec!["back", "sep"];
            open_actions.extend(
                crate::fileops::listing_open_submenu_actions()
                    .iter()
                    .copied(),
            );
            let open = self.listing_menu_page(
                popover,
                &open_actions,
                Some(&stack),
                multi,
                send_registered,
            );
            stack.add_named(&open, Some("open"));
        }
        stack.set_visible_child_name("main");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_min_content_width(crate::fileops::FILE_CONTEXT_MENU_MIN_WIDTH_PX);
        scroll.set_max_content_height(crate::fileops::FILE_CONTEXT_MENU_MAX_HEIGHT_PX);
        scroll.set_propagate_natural_width(true);
        scroll.set_propagate_natural_height(true);
        let height = if kind == crate::fileops::ListingMenuKind::Empty {
            320
        } else {
            crate::fileops::FILE_CONTEXT_MENU_MAX_HEIGHT_PX
        };
        scroll.set_size_request(crate::fileops::FILE_CONTEXT_MENU_MIN_WIDTH_PX, height);
        scroll.set_child(Some(&stack));
        scroll.upcast()
    }

    fn selected_is_dir(&self) -> bool {
        let Some(name) = self.selected_name() else {
            return false;
        };
        self.last_listing
            .borrow()
            .iter()
            .any(|entry| entry.name == name && entry.is_dir)
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
            self.toast.add_toast(adw::Toast::new(
                &self.ctx.t_or("common.copied", "Copied to clipboard"),
            ));
        }
    }

    fn copy_selected_path(&self) {
        let text = self.formatted_path(self.selected_name().as_deref());
        self.copy_text(&text);
    }

    fn copy_public_link(&self) {
        let Some(name) = self.selected_name() else {
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.selectToShare",
                "Select a file to share",
            )));
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
            Ok(_) => self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.noPublicLink",
                "Remote did not return a public link",
            ))),
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
                        view.upload_local_paths(&[path]);
                    }
                }
            },
        );
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
                self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                    "nautilus.notifications.rmdirsDone",
                    "Removed empty directories",
                )));
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
            Ok(_) => self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.cleanupStarted",
                "Cleanup started for this remote",
            ))),
            Err(e) => self.toast.add_toast(adw::Toast::new(&e.to_string())),
        }
    }

    fn extract_selected(&self) {
        let Some(name) = self.selected_name() else {
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.selectArchive",
                "Select an archive to extract",
            )));
            return;
        };
        let Some(win) = self.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let current = self.current.borrow().clone();
        let src = self.formatted_path(Some(&name));
        let default_dst = current.path.clone();
        let view = self.clone();
        dialogs::pick_destination(
            &win,
            &self.ctx,
            &self
                .ctx
                .t_or("fileBrowser.fileViewer.extract", "Extract archive"),
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
            self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                "nautilus.notifications.selectToShare",
                "Select a file to share",
            )));
            return;
        };
        let current = self.current.borrow().clone();
        let path = join_remote_path(&current.path, &name);
        if current.remote == "local" {
            match crate::platform::share_file(std::path::Path::new(&path)) {
                Ok(()) => self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                    "nautilus.notifications.shareOpened",
                    "Opened system share for the file",
                ))),
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
                Ok(()) => self.toast.add_toast(adw::Toast::new(&self.ctx.t_or(
                    "nautilus.notifications.shareOpened",
                    "Opened system share for the file",
                ))),
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
        self.toast.add_toast(adw::Toast::new(
            &self
                .ctx
                .tf("fileBrowser.fileViewer.openingNative", &[("name", &name)]),
        ));
        let client = self.ctx.client();
        if let Err(e) =
            crate::fileops::open_file_natively(client.as_ref(), &current.remote, &path, &name)
        {
            let msg = if e == crate::fileops::ENGINE_OFFLINE {
                self.ctx.t_or(
                    "notification.title.engineConnectionFailed",
                    "Engine Connection Error",
                )
            } else {
                e
            };
            self.toast.add_toast(adw::Toast::new(&msg));
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

    fn remember_file_jobs(&self, ids: &[u64], origin: &str) {
        self.remember_file_jobs_ex(ids, origin, "", serde_json::json!([]));
    }

    fn remember_file_jobs_ex(
        &self,
        ids: &[u64],
        origin: &str,
        group: &str,
        snapshot: serde_json::Value,
    ) {
        let remote = self.current.borrow().remote.clone();
        let op = if group.contains("upload") || origin.contains("upload") {
            "upload"
        } else if group.contains("delete") {
            "delete"
        } else {
            "copy"
        };
        {
            let mut store = self.ctx.store.borrow_mut();
            crate::jobs::remember_grouped(
                &mut store.job_meta,
                ids,
                crate::store::JobMeta {
                    origin: origin.into(),
                    profile: "default".into(),
                    remote: remote.clone(),
                    backend: self.ctx.backend_key(),
                    group: group.into(),
                    transfer_snapshot: snapshot.clone(),
                    ..Default::default()
                },
            );
            for job in
                crate::jobs::jobs_from_transfer_start(ids, op, &remote, origin, group, &snapshot)
            {
                store.remember_job(job);
            }
        }
        self.ctx.persist();
    }

    fn reload_ops(&self) {
        while let Some(child) = self.ops.first_child() {
            self.ops.remove(&child);
        }
        let mut jobs: Vec<_> = self
            .ctx
            .snapshot
            .borrow()
            .jobs
            .iter()
            .filter(|job| {
                crate::jobs::is_overview_job(job)
                    && crate::jobs::origin_matches(&job.origin, "filemanager")
            })
            .cloned()
            .collect();
        let history: Vec<_> = {
            let store = self.ctx.store.borrow();
            crate::jobs::history_with_meta(&store.job_history, &store.job_meta)
                .into_iter()
                .filter(|job| {
                    crate::jobs::is_overview_job(job)
                        && crate::jobs::origin_matches(&job.origin, "filemanager")
                })
                .collect()
        };
        if jobs.is_empty() && history.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(
                &self
                    .ctx
                    .t_or("nautilus.noFileOperations", "No file operations"),
            );
            self.ops.append(&row);
            self.ops_title
                .set_text(&self.ctx.t_or("fileBrowser.operations.title", "Operations"));
            return;
        }
        let registry = self.ctx.store.borrow().job_meta.clone();
        let siblings = {
            let store = self.ctx.store.borrow();
            crate::jobs::merge_job_lists(
                &jobs,
                &crate::jobs::history_with_meta(&store.job_history, &store.job_meta),
            )
        };
        for job in &mut jobs {
            crate::jobs::decorate_job_transfers(job, &registry, &siblings);
        }
        for job in &jobs {
            self.ops.append(&self.ops_row(job, true));
        }
        let running_ids: std::collections::HashSet<u64> = jobs.iter().map(|j| j.id).collect();
        let active = crate::fileops::active_ops_count(jobs.iter().map(|job| job.status.as_str()));
        self.ops_title.set_text(&crate::fileops::ops_panel_title(
            &self.ctx.t_or("fileBrowser.operations.title", "Operations"),
            active,
        ));
        for mut job in history.into_iter().filter(|j| !running_ids.contains(&j.id)) {
            crate::jobs::decorate_job_transfers(&mut job, &registry, &siblings);
            self.ops.append(&self.ops_row(&job, false));
        }
    }

    fn ops_row(&self, job: &crate::store::JobInfo, live: bool) -> adw::ActionRow {
        let percent = if matches!(job.status.as_str(), "completed" | "failed" | "stopped")
            && job.progress <= 0.0
        {
            100
        } else {
            (job.progress * 100.0).round() as i32
        };
        let row = adw::ActionRow::new();
        row.set_activatable(true);
        row.set_title(&crate::fileops::ops_job_title(
            &job.operation,
            &self
                .ctx
                .t_or(crate::jobs::job_status_key(&job.status), &job.status),
        ));
        let src = if job.src.is_empty() {
            job.remote.clone()
        } else {
            job.src.clone()
        };
        let bytes = crate::jobs::stats_i64(&job.stats, &["bytes"]);
        let total = crate::jobs::stats_i64(&job.stats, &["totalBytes", "size"]);
        row.set_subtitle(&crate::fileops::ops_job_subtitle(
            job.id,
            percent,
            &src,
            &crate::rclone::format_bytes(bytes),
            &crate::rclone::format_bytes(total),
        ));
        if live && (job.status == "running" || job.status == "preparing") {
            let bar = gtk::ProgressBar::new();
            bar.set_valign(gtk::Align::Center);
            bar.set_width_request(96);
            bar.set_show_text(true);
            if crate::jobs::is_delete_like_operation(&job.operation) {
                bar.set_pulse_step(0.15);
                bar.pulse();
                bar.set_text(Some(
                    &self.ctx.t_or("fileBrowser.operations.progress", "Progress"),
                ));
                let weak = bar.downgrade();
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || match weak
                    .upgrade()
                {
                    Some(bar) => {
                        if bar.is_mapped() {
                            bar.pulse();
                        }
                        glib::ControlFlow::Continue
                    }
                    None => glib::ControlFlow::Break,
                });
            } else {
                bar.set_fraction(job.progress.clamp(0.0, 1.0));
                bar.set_text(Some(&format!("{percent}%")));
            }
            row.add_suffix(&bar);
        }
        if live && matches!(job.status.as_str(), "running" | "preparing" | "starting") {
            let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
            stop.set_valign(gtk::Align::Center);
            stop.set_tooltip_text(Some(&self.ctx.t_or("flow.quickRun.actions.stop", "Stop")));
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
            dismiss.set_tooltip_text(Some(
                &self
                    .ctx
                    .t_or("fileBrowser.operations.removeJob", "Remove from history"),
            ));
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
        let popover = gtk::Popover::new();
        popover.set_has_arrow(true);
        popover.set_position(gtk::PositionType::Top);
        popover.set_child(Some(&self.build_ops_details(job, live)));
        popover.set_parent(&row);
        {
            let popover = popover.clone();
            row.connect_activated(move |_| popover.popup());
        }
        row
    }

    fn build_ops_details(&self, job: &crate::store::JobInfo, live: bool) -> gtk::Box {
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
        box_.set_size_request(320, -1);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        let (speed, eta) = crate::jobs::job_speed_eta(job);
        let speed_row = adw::ActionRow::new();
        speed_row.set_title(&self.ctx.t_or("modals.jobDetail.fields.speed", "Speed"));
        speed_row.set_subtitle(&format!(
            "{speed} · {} {eta}",
            self.ctx.t_or("modals.jobDetail.fields.eta", "ETA")
        ));
        list.append(&speed_row);
        if !job.src.is_empty() {
            let source = adw::ActionRow::new();
            source.set_title(
                &self
                    .ctx
                    .t_or("fileBrowser.operations.details.source", "Source"),
            );
            source.set_subtitle(&job.src);
            list.append(&source);
        }
        if !job.dst.is_empty() {
            let dest = adw::ActionRow::new();
            dest.set_title(
                &self
                    .ctx
                    .t_or("fileBrowser.operations.details.destination", "Destination"),
            );
            dest.set_subtitle(&job.dst);
            list.append(&dest);
        }
        if crate::jobs::has_known_start_time(job) {
            let started = adw::ActionRow::new();
            started.set_title(
                &self
                    .ctx
                    .t_or("fileBrowser.operations.details.startTime", "Start time"),
            );
            started.set_subtitle(
                &job.start_time
                    .with_timezone(&chrono::Local)
                    .format("%b %d, %H:%M")
                    .to_string(),
            );
            list.append(&started);
        }
        if matches!(
            job.status.as_str(),
            "failed" | "running" | "Failed" | "Running"
        ) {
            if let Some(error) = crate::jobs::job_error_text(job) {
                let err_row = adw::ActionRow::new();
                err_row.set_title(&self.ctx.t_or("fileBrowser.operations.failed", "Failed"));
                err_row.set_subtitle(&error);
                err_row.add_css_class("error");
                let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
                copy.set_valign(gtk::Align::Center);
                copy.set_tooltip_text(Some(&self.ctx.t_or("common.copy", "Copy")));
                let error_text = error.clone();
                copy.connect_clicked(move |_| {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&error_text);
                    }
                });
                err_row.add_suffix(&copy);
                list.append(&err_row);
            }
        }
        let previews = crate::jobs::job_transfer_previews(job, 6);
        if !previews.is_empty() {
            let header = adw::ActionRow::new();
            header.set_title(&self.ctx.t_or(
                if previews.len() == 1 {
                    "fileBrowser.operations.currentFile"
                } else {
                    "fileBrowser.operations.currentFiles"
                },
                "Current files",
            ));
            list.append(&header);
            for (name, detail) in &previews {
                let child = adw::ActionRow::new();
                child.set_title(name);
                child.set_subtitle(detail);
                list.append(&child);
            }
        }
        let completed = crate::jobs::job_completed_previews(job, 12);
        if !completed.is_empty() {
            let header = adw::ActionRow::new();
            header.set_title(&self.ctx.t_or(
                crate::jobs::job_transferred_label_key(&job.operation),
                "Processed files",
            ));
            list.append(&header);
            for item in completed {
                let child = adw::ActionRow::new();
                child.set_title(&item.name);
                if !item.detail.is_empty() {
                    child.set_subtitle(&item.detail);
                    child.set_tooltip_text(Some(&item.detail));
                }
                if item.failed {
                    child.add_css_class("error");
                }
                list.append(&child);
            }
        } else if previews.is_empty() && live {
            let empty = adw::ActionRow::new();
            empty.set_title(&self.ctx.t_or(
                "shared.transferActivity.empty.noActive",
                "No active transfers",
            ));
            list.append(&empty);
        }
        let details = adw::ActionRow::new();
        details.set_title(
            &self
                .ctx
                .t_or("modals.jobDetail.sections.overview", "Job details"),
        );
        details.set_activatable(true);
        {
            let ctx = self.ctx.clone();
            let id = job.id;
            let view = self.clone();
            details.connect_activated(move |_| {
                if let Some(win) = view.root.root().and_downcast::<gtk::Window>() {
                    dialogs::job_detail(&win, ctx.clone(), id);
                }
            });
        }
        list.append(&details);
        box_.append(&list);
        box_
    }
}

fn widget_point_in_root(widget: &gtk::Widget, x: f64, y: f64) -> Option<(f64, f64)> {
    let root = widget.root()?;
    let point = widget.compute_point(&root, &gtk::graphene::Point::new(x as f32, y as f32))?;
    Some((f64::from(point.x()), f64::from(point.y())))
}

fn pointer_in_root(widget: &gtk::Widget) -> Option<(f64, f64)> {
    let native = widget.native()?;
    let device = widget.display().default_seat()?.pointer()?;
    let (sx, sy, _) = native.surface()?.device_position(&device)?;
    let (tx, ty) = native.surface_transform();
    Some((sx - tx, sy - ty))
}

fn root_bounds(widget: &gtk::Widget) -> Option<(f64, f64, f64, f64)> {
    let root = widget.root()?;
    Some((0.0, 0.0, root.width() as f64, root.height() as f64))
}

fn fs_remote(remote: &str, path: &str) -> (String, String) {
    if remote == "local" {
        ("/".into(), path.trim_start_matches('/').to_string())
    } else {
        (remote_fs(remote, ""), path.to_string())
    }
}

fn selected_list_names(list: &gtk::ListBox) -> Vec<String> {
    list.selected_rows()
        .into_iter()
        .filter_map(|row| row_name(&row))
        .collect()
}

fn row_name(row: &gtk::ListBoxRow) -> Option<String> {
    if let Ok(action) = row.clone().downcast::<adw::ActionRow>() {
        return crate::fileops::listing_row_name(&action.widget_name(), &action.title());
    }
    if let Some(child) = row.child() {
        if let Ok(action) = child.clone().downcast::<adw::ActionRow>() {
            return crate::fileops::listing_row_name(&action.widget_name(), &action.title());
        }
        return crate::fileops::listing_row_name(&child.widget_name(), "");
    }
    crate::fileops::listing_row_name(&row.widget_name(), "")
}

fn make_flow() -> gtk::FlowBox {
    let grid = gtk::FlowBox::new();
    grid.set_selection_mode(gtk::SelectionMode::Multiple);
    grid.set_activate_on_single_click(false);
    grid.set_homogeneous(true);
    grid.set_min_children_per_line(2);
    grid.set_max_children_per_line(12);
    grid.set_row_spacing(8);
    grid.set_column_spacing(8);
    grid.set_valign(gtk::Align::Start);
    grid.set_vexpand(true);
    grid
}

fn files_keys_should_yield(view: &NautilusView, key: gtk::gdk::Key, ctrl: bool) -> bool {
    if overlay_dialog_open(&view.root) {
        return true;
    }
    let editing = view.search_entry.has_focus() || view.path_entry.has_focus();
    if !editing {
        return false;
    }
    let allowed = key == gtk::gdk::Key::Escape
        || (ctrl && key == gtk::gdk::Key::f)
        || (ctrl && key == gtk::gdk::Key::l);
    !allowed
}

fn overlay_dialog_open(widget: &impl IsA<gtk::Widget>) -> bool {
    let Some(root) = widget.root() else {
        return false;
    };
    let mut stack = vec![root.upcast::<gtk::Widget>()];
    while let Some(node) = stack.pop() {
        if node.is::<adw::Dialog>() && node.is_visible() {
            return true;
        }
        let mut child = node.first_child();
        while let Some(next) = child {
            stack.push(next.clone());
            child = next.next_sibling();
        }
    }
    false
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

fn remove_named_list_child(list: &gtk::ListBox, name: &str) {
    let mut child = list.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        if widget.widget_name() == name {
            list.remove(&widget);
        }
        child = next;
    }
}

fn remove_named_flow_child(flow: &gtk::FlowBox, name: &str) {
    let mut child = flow.first_child();
    while let Some(widget) = child {
        let next = widget.next_sibling();
        let matches = widget.widget_name() == name
            || widget
                .downcast_ref::<gtk::FlowBoxChild>()
                .and_then(|item| item.child())
                .is_some_and(|inner| inner.widget_name() == name);
        if matches {
            flow.remove(&widget);
        }
        child = next;
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
