use super::window;
use super::AppCtx;
use crate::rclone::engine::{rclone_exists, RcloneEngine};
use adw::prelude::*;
use gtk::gio;

pub fn present(app: &adw::Application, ctx: AppCtx) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&ctx.t_or(
            "onboarding.cards.welcome.title",
            "Welcome to Rclone Manager",
        ))
        .default_width(720)
        .default_height(560)
        .build();

    let nav = adw::NavigationView::new();
    nav.add(&page_welcome(&ctx, &nav));
    nav.add(&page_features(&ctx, &nav));
    nav.add(&page_install(&ctx, &nav));
    nav.add(&page_mount(&ctx, &nav));
    nav.add(&page_config(&ctx, &nav));
    nav.add(&page_password(&ctx, &nav));
    nav.add(&page_view(&ctx, &nav));
    nav.add(&page_ready(app, &ctx, &window));
    window.set_content(Some(&nav));
    window.present();
}

fn page_welcome(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("folder-remote-symbolic"));
    status.set_title(&ctx.t_or(
        "onboarding.cards.welcome.title",
        "Welcome to Rclone Manager",
    ));
    status.set_description(Some(&ctx.t_or(
        "onboarding.cards.welcome.content",
        "A GTK 4 + libadwaita desktop client for managing rclone remotes, mounts, jobs, and files.",
    )));
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
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
        .title(&ctx.t_or("onboarding.cards.welcome.title", "Welcome"))
        .child(&status)
        .build()
}

fn page_features(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let title = gtk::Label::new(Some(
        &ctx.t_or("onboarding.cards.features.title", "What you can do"),
    ));
    title.add_css_class("title-1");
    box_.append(&title);
    for (icon, title, subtitle) in [
        (
            "folder-symbolic",
            ctx.t_or(
                "onboarding.uiOptions.nautilus.title",
                "Nautilus file browser",
            ),
            ctx.t_or(
                "onboarding.uiOptions.nautilus.description",
                "Browse, copy, move, rename, and preview remote files",
            ),
        ),
        (
            "drive-harddisk-symbolic",
            ctx.t_or("onboarding.cards.installPlugin.title", "Mount & serve"),
            ctx.t_or(
                "onboarding.cards.installPlugin.content",
                "Mount remotes and serve WebDAV, SFTP, HTTP, FTP, and more",
            ),
        ),
        (
            "media-playlist-consecutive-symbolic",
            ctx.t_or("onboarding.uiOptions.flow.title", "Jobs & Flow"),
            ctx.t_or(
                "onboarding.uiOptions.flow.description",
                "Watch transfers and save reusable quick runs",
            ),
        ),
        (
            "dialog-warning-symbolic",
            ctx.t_or("onboarding.cards.features.title", "Alerts"),
            ctx.t_or(
                "onboarding.cards.features.content",
                "Webhook, Telegram, email, MQTT, script, and desktop notifications",
            ),
        ),
    ] {
        let row = adw::ActionRow::new();
        row.set_title(&title);
        row.set_subtitle(&subtitle);
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        box_.append(&row);
    }
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("install");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("features")
        .title(&ctx.t_or("onboarding.cards.features.title", "Features"))
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
        status.set_title(&ctx.t_or(
            "onboarding.cards.installRclone.title",
            "rclone is installed",
        ));
        status.set_description(Some(&ctx.t_or(
            "onboarding.cards.installRclone.content",
            "The engine will start automatically.",
        )));
    } else {
        status.set_icon_name(Some("dialog-warning-symbolic"));
        status.set_title(&ctx.t_or(
            "onboarding.cards.installRclone.title",
            "rclone was not found",
        ));
        status.set_description(Some(&ctx.t_or(
            "onboarding.cards.installRclone.content",
            "Install rclone from rclone.org or download a binary into ~/.local/bin.",
        )));
        let install = gtk::Button::with_label(
            &ctx.t_or("onboarding.installButton.install", "Download rclone"),
        );
        install.add_css_class("suggested-action");
        let open = gtk::LinkButton::builder()
            .uri("https://rclone.org/install/")
            .label(&ctx.t_or("onboarding.installGuide", "Open install guide"))
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
    password.set_title(&ctx.t_or(
        "onboarding.cards.passwordRequired.title",
        "rclone.conf password (optional)",
    ));
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
    group.set_title(&ctx.t_or("onboarding.cards.passwordRequired.title", "Config password"));
    group.add(&password);
    let binary = adw::EntryRow::new();
    binary.set_title(&ctx.t_or(
        "onboarding.installButton.selectBinary",
        "Existing rclone binary",
    ));
    binary.set_text(&ctx.settings.borrow().core.rclone_binary);
    let test =
        gtk::Button::with_label(&ctx.t_or("onboarding.installButton.testBinary", "Test binary"));
    test.set_valign(gtk::Align::Center);
    let test_status = gtk::Label::new(None);
    test_status.add_css_class("dim-label");
    test_status.set_xalign(0.0);
    {
        let binary = binary.clone();
        let ctx = ctx.clone();
        let test_status = test_status.clone();
        test.connect_clicked(move |_| {
            let path = binary.text().to_string();
            ctx.settings.borrow_mut().core.rclone_binary = path.clone();
            ctx.persist();
            if rclone_exists(&path) {
                test_status
                    .set_text(&ctx.t_or("onboarding.installButton.useBinary", "Binary is valid"));
            } else {
                test_status.set_text(
                    &ctx.t_or("onboarding.installButton.invalidBinary", "Invalid binary"),
                );
            }
        });
    }
    let binary_row = adw::ActionRow::new();
    binary_row.set_title(&ctx.t_or("onboarding.options.existing", "Existing"));
    binary_row.add_suffix(&test);
    group.add(&binary);
    group.add(&binary_row);
    box_.append(&group);
    box_.append(&test_status);
    let import = gtk::Button::with_label(&ctx.t_or("backup.restore", "Import backup…"));
    {
        let ctx = ctx.clone();
        import.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            let ctx = ctx.clone();
            dialog.open(
                None::<&gtk::Window>,
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            match crate::backup::restore_backup(&path) {
                                Ok((settings, store, _)) => {
                                    if let Some(settings) = settings {
                                        *ctx.settings.borrow_mut() = settings;
                                    }
                                    if let Some(store) = store {
                                        *ctx.store.borrow_mut() = store;
                                    }
                                    ctx.persist();
                                }
                                Err(e) => log::warn!("onboarding restore failed: {e}"),
                            }
                        }
                    }
                },
            );
        });
    }
    box_.append(&import);
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("mount");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("install")
        .title(&ctx.t_or("onboarding.cards.installRclone.title", "Install rclone"))
        .child(&box_)
        .build()
}

fn page_mount(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
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
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("config");
    });
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("mount")
        .title(label)
        .child(&box_)
        .build()
}

fn page_config(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let title = gtk::Label::new(Some(
        &ctx.t_or("onboarding.cards.selectConfig.title", "Choose rclone.conf"),
    ));
    title.add_css_class("title-1");
    title.set_wrap(true);
    title.set_xalign(0.0);
    let desc = gtk::Label::new(Some(&ctx.t_or(
        "onboarding.cards.selectConfig.content",
        "Use the default rclone configuration or pick a custom rclone.conf for this client.",
    )));
    desc.add_css_class("dim-label");
    desc.set_wrap(true);
    desc.set_xalign(0.0);
    let path = adw::EntryRow::new();
    path.set_title(&ctx.t_or("modals.backend.selectConfigFile", "rclone.conf path"));
    let current =
        crate::repair::config_path_from_flags(&ctx.settings.borrow().core.rclone_additional_flags)
            .unwrap_or_else(|| {
                crate::repair::default_rclone_config_path()
                    .display()
                    .to_string()
            });
    path.set_text(&current);
    let browse = gtk::Button::with_label(&ctx.t_or("common.browse", "Browse"));
    browse.set_valign(gtk::Align::Center);
    {
        let path = path.clone();
        browse.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            let path = path.clone();
            dialog.open(
                None::<&gtk::Window>,
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(picked) = file.path() {
                            path.set_text(&picked.display().to_string());
                        }
                    }
                },
            );
        });
    }
    path.add_suffix(&browse);
    {
        let ctx = ctx.clone();
        path.connect_changed(move |row| {
            crate::repair::set_config_path_flag(
                &mut ctx.settings.borrow_mut().core.rclone_additional_flags,
                &row.text(),
            );
            ctx.persist();
        });
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&ctx.t_or("onboarding.cards.selectConfig.title", "Configuration"));
    group.add(&path);
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("password");
    });
    box_.append(&title);
    box_.append(&desc);
    box_.append(&group);
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("config")
        .title(&ctx.t_or("onboarding.cards.selectConfig.title", "Select config"))
        .child(&box_)
        .build()
}

fn page_password(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let title = gtk::Label::new(Some(
        &ctx.t_or("onboarding.cards.passwordRequired.title", "Config password"),
    ));
    title.add_css_class("title-1");
    title.set_wrap(true);
    title.set_xalign(0.0);
    let config_path =
        crate::repair::config_path_from_flags(&ctx.settings.borrow().core.rclone_additional_flags)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(crate::repair::default_rclone_config_path);
    let encrypted = crate::repair::config_file_encrypted(&config_path);
    let desc = gtk::Label::new(Some(&if encrypted {
        ctx.t_or(
            "onboarding.cards.passwordRequired.content",
            "This rclone.conf is encrypted. Enter the password to unlock it before continuing.",
        )
    } else {
        ctx.t_or(
            "onboarding.cards.passwordRequired.optional",
            "Optional. Store a password if you encrypt rclone.conf later.",
        )
    }));
    desc.add_css_class("dim-label");
    desc.set_wrap(true);
    desc.set_xalign(0.0);
    let password = adw::PasswordEntryRow::new();
    password.set_title(&ctx.t_or(
        "onboarding.cards.passwordRequired.title",
        "rclone.conf password",
    ));
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
    group.add(&password);
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
    next.add_css_class("suggested-action");
    if encrypted && password.text().is_empty() {
        next.set_sensitive(false);
    }
    {
        let next = next.clone();
        next.set_sensitive(!(encrypted && password.text().is_empty()));
        password.connect_changed(move |row| {
            if encrypted {
                next.set_sensitive(!row.text().is_empty());
            }
        });
    }
    let nav = nav.clone();
    next.connect_clicked(move |_| {
        nav.push_by_tag("view");
    });
    box_.append(&title);
    box_.append(&desc);
    box_.append(&group);
    box_.append(&next);
    adw::NavigationPage::builder()
        .tag("password")
        .title(&ctx.t_or("onboarding.cards.passwordRequired.title", "Password"))
        .child(&box_)
        .build()
}

fn page_view(ctx: &AppCtx, nav: &adw::NavigationView) -> adw::NavigationPage {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_top(24);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    let title = gtk::Label::new(Some(
        &ctx.t_or("onboarding.uiOptions.title", "Choose your default view"),
    ));
    title.add_css_class("title-1");
    box_.append(&title);
    let group = gtk::CheckButton::new();
    for (id, label) in [
        (
            "main_menu",
            ctx.t_or(
                "onboarding.uiOptions.classic.title",
                "Main menu — remotes, mounts, jobs",
            ),
        ),
        (
            "nautilus",
            ctx.t_or("onboarding.uiOptions.nautilus.title", "File browser"),
        ),
        (
            "flow",
            ctx.t_or("onboarding.uiOptions.flow.title", "Flow / Quick Runs"),
        ),
    ] {
        let btn = gtk::CheckButton::with_label(&label);
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
    let next = gtk::Button::with_label(&ctx.t_or("common.continue", "Continue"));
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
    status.set_title(&ctx.t_or("onboarding.ready.title", "You're ready"));
    status.set_description(Some(&ctx.t_or(
        "onboarding.ready.body",
        "Start managing remotes with the GTK 4 + Adwaita desktop client.",
    )));
    let finish = gtk::Button::with_label(&ctx.t_or("onboarding.getStarted", "Get started"));
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
