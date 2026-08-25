use super::dashboard::Dashboard;
use super::dialogs;
use super::flow::FlowView;
use super::nautilus::NautilusView;
use super::onboarding;
use super::AppCtx;
use crate::operations::MainView;
use crate::rclone::engine::RcloneEngine;
use adw::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::Rc;

pub fn activate(app: &adw::Application) {
    let ctx = AppCtx::new();
    ctx.apply_theme();

    let settings = ctx.settings.borrow().clone();
    if settings.core.active_backend.is_empty() || settings.core.active_backend == "local" {
        *ctx.engine.borrow_mut() = Some(RcloneEngine::start(&settings));
    }
    if settings.general.start_on_startup {
        let _ = crate::platform::set_autostart(true);
    }
    ctx.refresh_runtime();

    if !ctx.settings.borrow().core.completed_onboarding {
        onboarding::present(app, ctx.clone());
        return;
    }

    present_main(app, ctx);
}

pub fn present_main(app: &adw::Application, ctx: AppCtx) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Rclone Manager")
        .default_width(1280)
        .default_height(820)
        .build();

    let toast = adw::ToastOverlay::new();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();

    let view_stack = adw::ViewStack::new();
    let dashboard = Dashboard::new(ctx.clone(), toast.clone());
    let nautilus = NautilusView::new(ctx.clone(), toast.clone());
    let flow = FlowView::new(ctx.clone(), toast.clone());

    view_stack.add_titled_with_icon(
        &dashboard.root,
        Some("main_menu"),
        &ctx.t("sidebar.remotes"),
        "view-grid-symbolic",
    );
    view_stack.add_titled_with_icon(&nautilus.root, Some("nautilus"), "Files", "folder-symbolic");
    view_stack.add_titled_with_icon(
        &flow.root,
        Some("flow"),
        "Flow",
        "media-playlist-consecutive-symbolic",
    );

    let switcher = adw::ViewSwitcher::new();
    switcher.set_stack(Some(&view_stack));
    switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
    header.set_title_widget(Some(&switcher));

    let add_btn = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Add remote or quick run")
        .build();
    let add_menu = gio::Menu::new();
    add_menu.append(Some("Quick Add Remote"), Some("win.quick-add"));
    add_menu.append(Some("Detailed Remote"), Some("win.remote-config"));
    add_menu.append(Some("New Quick Run"), Some("win.quick-run-new"));
    add_btn.set_menu_model(Some(&add_menu));
    header.pack_start(&add_btn);

    let menu_btn = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Application menu")
        .build();
    menu_btn.set_menu_model(Some(&app_menu()));
    header.pack_end(&menu_btn);

    let banner = adw::Banner::new("");
    banner.set_button_label(Some("Repair"));
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        banner.connect_button_clicked(move |_| {
            dialogs::repair(&window, ctx.clone(), toast.clone());
        });
    }
    update_banner(&ctx, &banner);

    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&banner);
    toolbar.set_content(Some(&view_stack));
    toast.set_child(Some(&toolbar));
    window.set_content(Some(&toast));

    install_actions(
        app,
        &window,
        &ctx,
        &toast,
        &view_stack,
        &dashboard,
        &nautilus,
        &flow,
        &banner,
    );
    install_shortcuts(&window);

    let default_view = ctx.settings.borrow().default_view();
    view_stack.set_visible_child_name(default_view.as_str());

    let tray = super::tray::start(&ctx);
    let ctx_poll = ctx.clone();
    let dash_poll = dashboard.clone();
    let flow_poll = flow.clone();
    let banner_poll = banner.clone();
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        ctx_poll.tick_automations();
        ctx_poll.refresh_runtime();
        dash_poll.refresh();
        flow_poll.refresh();
        update_banner(&ctx_poll, &banner_poll);
        if let Some(tray) = &tray {
            tray.drain(&ctx_poll);
        }
        glib::ControlFlow::Continue
    });

    ctx.start_autostarts();
    window.present();
}

fn app_menu() -> gio::Menu {
    let menu = gio::Menu::new();

    let theme = gio::Menu::new();
    theme.append(Some("System theme"), Some("win.theme::system"));
    theme.append(Some("Light"), Some("win.theme::light"));
    theme.append(Some("Dark"), Some("win.theme::dark"));
    menu.append_submenu(Some("Theme"), &theme);

    let file = gio::Menu::new();
    file.append(Some("Import settings"), Some("win.import"));
    file.append(Some("Export settings"), Some("win.export"));
    menu.append_section(None, &file);

    let prefs = gio::Menu::new();
    prefs.append(Some("Preferences"), Some("win.preferences"));
    prefs.append(Some("Rclone Flags"), Some("win.rclone-flags"));
    prefs.append(Some("Backends"), Some("win.backends"));
    prefs.append(Some("Alerts"), Some("win.alerts"));
    prefs.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
    prefs.append(Some("Templates"), Some("win.templates"));
    prefs.append(Some("Install rclone"), Some("win.install-rclone"));
    prefs.append(Some("Remote order"), Some("win.item-order"));
    menu.append_section(None, &prefs);

    let views = gio::Menu::new();
    views.append(Some("Main Menu"), Some("win.view::main_menu"));
    views.append(Some("File Browser"), Some("win.view::nautilus"));
    views.append(Some("Flow"), Some("win.view::flow"));
    menu.append_section(None, &views);

    let tray = gio::Menu::new();
    tray.append(Some("Unmount All"), Some("win.unmount-all"));
    tray.append(Some("Stop All Jobs"), Some("win.stop-jobs"));
    tray.append(Some("Stop All Serves"), Some("win.stop-serves"));
    menu.append_submenu(Some("Tray actions"), &tray);

    let about = gio::Menu::new();
    about.append(Some("About"), Some("win.about"));
    about.append(Some("Quit"), Some("win.quit"));
    menu.append_section(None, &about);
    menu
}

#[allow(clippy::too_many_arguments)]
fn install_actions(
    app: &adw::Application,
    window: &adw::ApplicationWindow,
    ctx: &AppCtx,
    toast: &adw::ToastOverlay,
    view_stack: &adw::ViewStack,
    dashboard: &Dashboard,
    nautilus: &NautilusView,
    flow: &FlowView,
    banner: &adw::Banner,
) {
    let add_action = |name: &str, cb: Box<dyn Fn() + 'static>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| cb());
        window.add_action(&action);
    };

    {
        let ctx = ctx.clone();
        let window = window.clone();
        let dash = dashboard.clone();
        add_action(
            "quick-add",
            Box::new(move || {
                dialogs::quick_add_remote(&window, ctx.clone(), {
                    let dash = dash.clone();
                    let ctx = ctx.clone();
                    Rc::new(move || {
                        ctx.refresh_runtime();
                        dash.refresh();
                    })
                });
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let dash = dashboard.clone();
        add_action(
            "remote-config",
            Box::new(move || {
                dialogs::remote_config(&window, ctx.clone(), None, {
                    let dash = dash.clone();
                    let ctx = ctx.clone();
                    Rc::new(move || {
                        ctx.refresh_runtime();
                        dash.refresh();
                    })
                });
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let flow = flow.clone();
        add_action(
            "quick-run-new",
            Box::new(move || {
                dialogs::quick_run_editor(&window, ctx.clone(), None, {
                    let flow = flow.clone();
                    Rc::new(move || flow.refresh())
                });
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "preferences",
            Box::new(move || dialogs::preferences(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "rclone-flags",
            Box::new(move || dialogs::rclone_flags(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "backends",
            Box::new(move || dialogs::backends(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "alerts",
            Box::new(move || dialogs::alerts(&window, ctx.clone())),
        );
    }
    {
        let window = window.clone();
        add_action("shortcuts", Box::new(move || dialogs::shortcuts(&window)));
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "templates",
            Box::new(move || dialogs::templates(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "install-rclone",
            Box::new(move || dialogs::install_rclone_update(&window, ctx.clone(), toast.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let dash = dashboard.clone();
        add_action(
            "item-order",
            Box::new(move || {
                dialogs::item_order(&window, ctx.clone(), {
                    let dash = dash.clone();
                    Rc::new(move || dash.refresh())
                })
            }),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        add_action(
            "about",
            Box::new(move || dialogs::about(&window, ctx.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        add_action(
            "export",
            Box::new(move || dialogs::export_backup(&window, ctx.clone(), toast.clone())),
        );
    }
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        add_action(
            "import",
            Box::new(move || {
                dialogs::import_backup(&window, ctx.clone(), toast.clone(), {
                    let dash = dash.clone();
                    Rc::new(move || dash.refresh())
                })
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "unmount-all",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let _ = c.unmount_all();
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "stop-jobs",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    if let Ok(list) = c.job_list() {
                        if let Some(arr) = list.get("jobids").and_then(|x| x.as_array()) {
                            for id in arr {
                                if let Some(jobid) = id.as_u64() {
                                    let _ = c.job_stop(jobid);
                                }
                            }
                        }
                    }
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let ctx = ctx.clone();
        add_action(
            "stop-serves",
            Box::new(move || {
                if let Some(c) = ctx.client() {
                    let _ = c.serve_stop_all();
                    ctx.refresh_runtime();
                }
            }),
        );
    }
    {
        let window = window.clone();
        add_action("quit", Box::new(move || window.close()));
    }

    let theme_action = gio::SimpleAction::new_stateful(
        "theme",
        Some(&glib::VariantTy::STRING),
        &glib::Variant::from(ctx.settings.borrow().runtime.theme.as_str()),
    );
    {
        let ctx = ctx.clone();
        theme_action.connect_activate(move |action, value| {
            if let Some(v) = value.and_then(|v| v.str().map(|s| s.to_string())) {
                ctx.settings.borrow_mut().runtime.theme = v.clone();
                ctx.persist();
                ctx.apply_theme();
                action.set_state(&glib::Variant::from(v.as_str()));
            }
        });
    }
    window.add_action(&theme_action);

    let view_action = gio::SimpleAction::new_stateful(
        "view",
        Some(&glib::VariantTy::STRING),
        &glib::Variant::from(ctx.settings.borrow().general.default_view.as_str()),
    );
    {
        let stack = view_stack.clone();
        view_action.connect_activate(move |action, value| {
            if let Some(v) = value.and_then(|v| v.str().map(|s| s.to_string())) {
                stack.set_visible_child_name(&v);
                action.set_state(&glib::Variant::from(v.as_str()));
            }
        });
    }
    window.add_action(&view_action);

    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        let action = gio::SimpleAction::new("refresh-mounts", None);
        action.connect_activate(move |_, _| {
            ctx.refresh_runtime();
            dash.refresh();
            toast.add_toast(adw::Toast::new("Mounts refreshed"));
        });
        window.add_action(&action);
    }
    {
        let ctx = ctx.clone();
        let toast = toast.clone();
        let dash = dashboard.clone();
        let action = gio::SimpleAction::new("refresh-serves", None);
        action.connect_activate(move |_, _| {
            ctx.refresh_runtime();
            dash.refresh();
            toast.add_toast(adw::Toast::new("Serves refreshed"));
        });
        window.add_action(&action);
    }

    let _ = (app, nautilus, banner);
}

fn install_shortcuts(window: &adw::ApplicationWindow) {
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Global);
    let add = |accel: &str, action: &str| {
        let trigger = gtk::ShortcutTrigger::parse_string(accel);
        let shortcut = gtk::Shortcut::new(trigger, Some(gtk::NamedAction::new(action)));
        controller.add_shortcut(shortcut);
    };
    add("<Control>q", "win.quit");
    add("<Control>b", "win.view::nautilus");
    add("<Control>n", "win.remote-config");
    add("<Control>r", "win.quick-add");
    add("<Control>i", "win.import");
    add("<Control>e", "win.export");
    add("<Control>comma", "win.preferences");
    add("<Control>period", "win.rclone-flags");
    add("<Control><Alt>a", "win.alerts");
    add("<Control><Alt>f", "win.view::flow");
    add("<Control><Shift>question", "win.shortcuts");
    add("<Control><Shift>m", "win.refresh-mounts");
    add("<Control><Shift>s", "win.refresh-serves");
    window.add_controller(controller);
}

fn update_banner(ctx: &AppCtx, banner: &adw::Banner) {
    if !ctx.engine_ready() {
        banner.set_title(
            "Rclone engine is not running. Install rclone or set a custom binary in Preferences.",
        );
        banner.set_revealed(true);
    } else {
        banner.set_revealed(false);
    }
}

#[allow(dead_code)]
pub fn open_view(stack: &adw::ViewStack, view: MainView) {
    stack.set_visible_child_name(view.as_str());
}
