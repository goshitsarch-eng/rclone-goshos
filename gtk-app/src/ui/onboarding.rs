use super::window;
use super::AppCtx;
use crate::rclone::engine::{rclone_exists, RcloneEngine};
use adw::prelude::*;

pub fn present(app: &adw::Application, ctx: AppCtx) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Welcome to Rclone Manager")
        .default_width(720)
        .default_height(560)
        .build();

    let nav = adw::NavigationView::new();
    nav.add(&page_welcome(&ctx, &nav));
    nav.add(&page_features(&nav));
    nav.add(&page_install(&ctx, &nav));
    nav.add(&page_mount(&nav));
    nav.add(&page_view(&ctx, &nav));
    nav.add(&page_ready(app, &ctx, &window));
    window.set_content(Some(&nav));
    window.present();
}

fn page_welcome(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("folder-remote-symbolic"));
    status.set_title("Welcome to Rclone Manager");
    status.set_description(Some(
        "A GTK 4 + libadwaita desktop client for managing rclone remotes, mounts, jobs, and files.",
    ));
    let next = gtk::Button::with_label("Continue");
    next.add_css_class("suggested-action");
    next.add_css_class("pill");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("features");
    });
    status.set_child(Some(&next));
    let _ = ctx;
    adw::NavigationPage::builder()
        .tag("welcome")
        .title("Welcome")
        .child(&status)
        .build()
}

fn page_features(nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let title = gtk::Label::new(Some("What you can do"));
    title.add_css_class("title-1");
    box_.append(&title);
    for (icon, title, subtitle) in [
        (
            "folder-symbolic",
            "Nautilus file browser",
            "Browse, copy, move, rename, and preview remote files",
        ),
        (
            "drive-harddisk-symbolic",
            "Mount & serve",
            "Mount remotes and serve WebDAV, SFTP, HTTP, FTP, and more",
        ),
        (
            "media-playlist-consecutive-symbolic",
            "Jobs & Flow",
            "Watch transfers and save reusable quick runs",
        ),
        (
            "dialog-warning-symbolic",
            "Alerts",
            "Webhook, Telegram, email, MQTT, script, and desktop notifications",
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(title);
        row.set_subtitle(subtitle);
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        box_.append(&row);
    }
    let next = gtk::Button::with_label("Continue");
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("install");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("features")
        .title("Features")
        .child(&box_)
        .build()
}

fn page_install(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let status = adw::StatusPage::new();
    let exists = rclone_exists(&ctx.settings.borrow().core.rclone_binary);
    if exists {
        status.set_icon_name(Some("emblem-ok-symbolic"));
        status.set_title("rclone is installed");
        status.set_description(Some("The engine will start automatically."));
    } else {
        status.set_icon_name(Some("dialog-warning-symbolic"));
        status.set_title("rclone was not found");
        status.set_description(Some(
            "Install rclone from rclone.org or download a binary into ~/.local/bin.",
        ));
        let install = gtk::Button::with_label("Download rclone");
        install.add_css_class("suggested-action");
        let open = gtk::LinkButton::builder()
            .uri("https://rclone.org/install/")
            .label("Open install guide")
            .build();
        let status_btn = status.clone();
        let ctx_install = ctx.clone();
        install.connect_clicked(move |_| {
            let dest = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".local/bin");
            match crate::updater::install_rclone_binary(&dest) {
                Ok(path) => {
                    ctx_install.settings.borrow_mut().core.rclone_binary =
                        path.to_string_lossy().into_owned();
                    ctx_install.persist();
                    status_btn.set_icon_name(Some("emblem-ok-symbolic"));
                    status_btn.set_title("rclone is installed");
                    status_btn.set_description(Some(&format!("Installed to {}", path.display())));
                }
                Err(e) => {
                    status_btn.set_description(Some(&e));
                }
            }
        });
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
        actions.append(&install);
        actions.append(&open);
        status.set_child(Some(&actions));
    }
    box_.append(&status);
    let password = adw::PasswordEntryRow::new();
    password.set_title("rclone.conf password (optional)");
    password.set_text(&crate::keyring::resolve_config_password(
        &ctx.settings.borrow().core.config_password,
    ));
    {
        let ctx = ctx.clone();
        password.connect_changed(move |row| {
            let mut settings = ctx.settings.borrow_mut();
            crate::keyring::persist_password_setting(
                &mut settings.core.config_password,
                &row.text(),
            );
            drop(settings);
            ctx.persist();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.set_title("Config password");
    group.add(&password);
    box_.append(&group);
    let next = gtk::Button::with_label("Continue");
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("mount");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("install")
        .title("Install rclone")
        .child(&box_)
        .build()
}

fn page_mount(nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let status = adw::StatusPage::new();
    let label = crate::mount_plugin::plugin_label();
    if crate::mount_plugin::is_installed() {
        status.set_icon_name(Some("emblem-ok-symbolic"));
        status.set_title(&format!("{label} is ready"));
        status.set_description(Some("Mounts can use the local filesystem helper."));
    } else {
        status.set_icon_name(Some("dialog-warning-symbolic"));
        status.set_title(&crate::mount_plugin::missing_title());
        status.set_description(Some(&crate::mount_plugin::missing_detail()));
        let install = gtk::Button::with_label(&format!("Install {label}"));
        install.add_css_class("suggested-action");
        let status_btn = status.clone();
        install.connect_clicked(move |_| match crate::mount_plugin::install() {
            Ok(msg) => {
                status_btn.set_icon_name(Some("emblem-ok-symbolic"));
                status_btn.set_title(&msg);
                status_btn.set_description(Some("You can continue."));
            }
            Err(e) => status_btn.set_description(Some(&e)),
        });
        status.set_child(Some(&install));
    }
    box_.append(&status);
    let next = gtk::Button::with_label("Continue");
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("view");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("mount")
        .title(label)
        .child(&box_)
        .build()
}

fn page_view(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let title = gtk::Label::new(Some("Choose your default view"));
    title.add_css_class("title-1");
    box_.append(&title);
    let group = gtk::CheckButton::new();
    for (id, label) in [
        ("main_menu", "Main menu — remotes, mounts, jobs"),
        ("nautilus", "File browser"),
        ("flow", "Flow / Quick Runs"),
    ] {
        let btn = gtk::CheckButton::with_label(label);
        btn.set_group(Some(&group));
        if ctx.settings.borrow().general.default_view == id {
            btn.set_active(true);
        }
        let ctx = ctx.clone();
        btn.connect_toggled(move |b| {
            if b.is_active() {
                ctx.settings.borrow_mut().general.default_view = id.to_string();
            }
        });
        box_.append(&btn);
    }
    let next = gtk::Button::with_label("Continue");
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("ready");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("view")
        .title("Default view")
        .child(&box_)
        .build()
}

fn page_ready(
    app: &adw::Application,
    ctx: &AppCtx,
    window: &adw::ApplicationWindow,
) -> adw::NavigationPage {
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("emblem-ok-symbolic"));
    status.set_title("You're ready");
    status.set_description(Some(
        "Start managing remotes with the GTK 4 + Adwaita desktop client.",
    ));
    let finish = gtk::Button::with_label("Get started");
    finish.add_css_class("suggested-action");
    finish.add_css_class("pill");
    let ctx = ctx.clone();
    let app = app.clone();
    let window = window.clone();
    finish.connect_clicked(move |_| {
        ctx.settings.borrow_mut().core.completed_onboarding = true;
        ctx.persist();
        if ctx.engine.borrow().is_none() {
            *ctx.engine.borrow_mut() = Some(RcloneEngine::start(&ctx.settings.borrow()));
        }
        ctx.refresh_runtime();
        window.close();
        window::present_main(&app, ctx.clone());
    });
    status.set_child(Some(&finish));
    adw::NavigationPage::builder()
        .tag("ready")
        .title("Ready")
        .child(&status)
        .build()
}
