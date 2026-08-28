use super::window;
use super::AppCtx;
use crate::onboarding::{
    next_card, prev_card, rclone_install_dest, visible_cards, InstallLocation, OnboardingCard,
    MAIN_UI_OPTIONS,
};
use crate::rclone::engine::{rclone_exists, RcloneEngine};
use adw::prelude::*;
use gtk::gio;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub fn present(app: &adw::Application, ctx: AppCtx) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(&ctx.t_or(
            "onboarding.cards.welcome.title",
            "Welcome to Rclone Manager",
        ))
        .default_width(760)
        .default_height(620)
        .build();

    let config_path =
        crate::repair::config_path_from_flags(&ctx.settings.borrow().core.rclone_additional_flags)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(crate::repair::default_rclone_config_path);
    let cards = Rc::new(visible_cards(
        rclone_exists(&ctx.settings.borrow().core.rclone_binary),
        crate::mount_plugin::is_installed(),
        crate::repair::config_file_encrypted(&config_path),
    ));

    let nav = adw::NavigationView::new();
    let toast = adw::ToastOverlay::new();
    for card in cards.iter() {
        let page = match card {
            OnboardingCard::Welcome => page_welcome(&ctx, &nav, &cards, &window, &toast),
            OnboardingCard::Features => page_features(&ctx, &nav, &cards),
            OnboardingCard::InstallRclone => page_install(&ctx, &nav, &cards, &window),
            OnboardingCard::InstallPlugin => page_mount(&ctx, &nav, &cards),
            OnboardingCard::SelectConfig => page_config(&ctx, &nav, &cards, &window),
            OnboardingCard::PasswordRequired => page_password(&ctx, &nav, &cards),
            OnboardingCard::SelectMainUi => page_view(&ctx, &nav, &cards),
            OnboardingCard::Ready => page_ready(app, &ctx, &window, &nav, &cards),
        };
        nav.add(&page);
    }
    toast.set_child(Some(&nav));
    window.set_content(Some(&toast));
    {
        let nav = nav.clone();
        let cards = cards.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            let current = current_card(&nav);
            if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                push_next(&nav, &cards, current);
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Right || key == gtk::gdk::Key::KP_Right {
                push_next(&nav, &cards, current);
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Left || key == gtk::gdk::Key::KP_Left {
                if let Some(prev) = prev_card(&cards, current) {
                    nav.push_by_tag(prev.tag());
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }
    window.present();
}

fn current_card(nav: &adw::NavigationView) -> OnboardingCard {
    nav.visible_page()
        .and_then(|page| page.tag().map(|tag| tag.to_string()))
        .and_then(|tag| OnboardingCard::from_tag(&tag))
        .unwrap_or(OnboardingCard::Welcome)
}

fn push_next(nav: &adw::NavigationView, cards: &[OnboardingCard], current: OnboardingCard) {
    if let Some(next) = next_card(cards, current) {
        nav.push_by_tag(next.tag());
    }
}

fn indicators(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
    current: OnboardingCard,
) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.set_halign(gtk::Align::Center);
    bar.set_margin_top(12);
    bar.set_margin_bottom(8);
    for card in cards.iter() {
        let btn = gtk::Button::new();
        btn.add_css_class("circular");
        btn.add_css_class("flat");
        btn.set_tooltip_text(Some(&ctx.t_or(card.title_key(), card.tag())));
        if *card == current {
            btn.add_css_class("suggested-action");
        }
        let nav = nav.clone();
        let tag = card.tag();
        btn.connect_clicked(move |_| {
            if !nav.pop_to_tag(tag) {
                nav.push_by_tag(tag);
            }
        });
        bar.append(&btn);
    }
    bar
}

fn page_box(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
    current: OnboardingCard,
) -> gtk::Box {
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_margin_start(24);
    box_.set_margin_end(24);
    box_.set_margin_bottom(24);
    box_.append(&indicators(ctx, nav, cards, current));
    box_
}

fn footer_next(
    _ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
    current: OnboardingCard,
    label: &str,
) -> gtk::Button {
    let next = gtk::Button::with_label(label);
    next.add_css_class("suggested-action");
    let nav = nav.clone();
    let cards = cards.clone();
    next.connect_clicked(move |_| push_next(&nav, &cards, current));
    next
}

fn page_welcome(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
    window: &adw::ApplicationWindow,
    toast: &adw::ToastOverlay,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::Welcome);
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("folder-remote-symbolic"));
    status.set_title(&ctx.t_or(
        OnboardingCard::Welcome.title_key(),
        "Welcome to Rclone Manager",
    ));
    status.set_description(Some(&ctx.t_or(
        OnboardingCard::Welcome.content_key(),
        "A GTK 4 + libadwaita desktop client for managing rclone remotes, mounts, jobs, and files.",
    )));
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let import = gtk::Button::with_label(&ctx.t_or("titlebar.menu.import", "Import"));
    {
        let ctx = ctx.clone();
        let window = window.clone();
        let toast = toast.clone();
        import.connect_clicked(move |_| {
            super::dialogs::import_backup(&window, ctx.clone(), toast.clone(), Rc::new(|| {}));
        });
    }
    actions.append(&import);
    actions.append(&footer_next(
        ctx,
        nav,
        cards,
        OnboardingCard::Welcome,
        &ctx.t_or("common.continue", "Continue"),
    ));
    status.set_child(Some(&actions));
    box_.append(&status);
    adw::NavigationPage::builder()
        .tag(OnboardingCard::Welcome.tag())
        .title(&ctx.t_or(OnboardingCard::Welcome.title_key(), "Welcome"))
        .child(&box_)
        .build()
}

fn page_features(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::Features);
    let title = gtk::Label::new(Some(
        &ctx.t_or(OnboardingCard::Features.title_key(), "What you can do"),
    ));
    title.add_css_class("title-1");
    title.set_wrap(true);
    title.set_xalign(0.0);
    box_.append(&title);
    let desc = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::Features.content_key(),
        "Sync, mount, browse, and automate rclone from one desktop client.",
    )));
    desc.add_css_class("dim-label");
    desc.set_wrap(true);
    desc.set_xalign(0.0);
    box_.append(&desc);
    for option in MAIN_UI_OPTIONS {
        let row = crate::ui::rows::action_row();
        row.set_title(&ctx.t_or(option.title_key, option.id));
        row.set_subtitle(&ctx.t_or(option.desc_key, ""));
        row.add_prefix(&gtk::Image::from_icon_name(option.icon));
        box_.append(&row);
    }
    box_.append(&footer_next(
        ctx,
        nav,
        cards,
        OnboardingCard::Features,
        &ctx.t_or("common.continue", "Continue"),
    ));
    adw::NavigationPage::builder()
        .tag(OnboardingCard::Features.tag())
        .title(&ctx.t_or(OnboardingCard::Features.title_key(), "Features"))
        .child(&scrolled(&box_))
        .build()
}

fn page_install(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
    window: &adw::ApplicationWindow,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::InstallRclone);
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("software-update-available-symbolic"));
    status.set_title(&ctx.t_or(OnboardingCard::InstallRclone.title_key(), "Install Rclone"));
    status.set_description(Some(&ctx.t_or(
        OnboardingCard::InstallRclone.content_key(),
        "Choose a download location or point at an existing rclone binary.",
    )));
    box_.append(&status);

    let mode = crate::ui::rows::combo_row();
    mode.set_title(&ctx.t_or("onboarding.options.recommended", "Install location"));
    mode.set_model(Some(&gtk::StringList::new(&[
        &ctx.t_or("onboarding.options.default", "Default"),
        &ctx.t_or("onboarding.options.custom", "Custom"),
        &ctx.t_or("onboarding.options.existing", "Existing"),
    ])));
    let custom = crate::ui::rows::entry_row();
    custom.set_title(&ctx.t_or(
        "onboarding.installButton.selectPath",
        "Custom install directory",
    ));
    custom.set_visible(false);
    let browse = gtk::Button::with_label(&ctx.t_or("common.browse", "Browse"));
    browse.set_valign(gtk::Align::Center);
    {
        let custom = custom.clone();
        let window = window.clone();
        browse.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            let custom = custom.clone();
            dialog.select_folder(
                Some(&window),
                None::<gio::Cancellable>.as_ref(),
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            custom.set_text(&path.display().to_string());
                        }
                    }
                },
            );
        });
    }
    custom.add_suffix(&browse);
    let binary = crate::ui::rows::entry_row();
    binary.set_title(&ctx.t_or(
        "onboarding.installButton.selectBinary",
        "Existing rclone binary",
    ));
    binary.set_text(&ctx.settings.borrow().core.rclone_binary);
    binary.set_visible(false);
    {
        let custom = custom.clone();
        let binary = binary.clone();
        mode.connect_selected_notify(move |row| {
            let selected = match row.selected() {
                1 => InstallLocation::Custom,
                2 => InstallLocation::Existing,
                _ => InstallLocation::Default,
            };
            custom.set_visible(selected == InstallLocation::Custom);
            binary.set_visible(selected == InstallLocation::Existing);
        });
    }
    let group = adw::PreferencesGroup::new();
    group.add(&mode);
    group.add(&custom);
    group.add(&binary);
    box_.append(&group);

    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_visible(false);
    box_.append(&bar);
    let message = gtk::Label::new(None);
    message.add_css_class("dim-label");
    message.set_wrap(true);
    message.set_xalign(0.0);
    box_.append(&message);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let install =
        gtk::Button::with_label(&ctx.t_or("onboarding.installButton.install", "Install Rclone"));
    install.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(&ctx.t_or("common.cancel", "Cancel"));
    cancel.set_visible(false);
    let test =
        gtk::Button::with_label(&ctx.t_or("onboarding.installButton.testBinary", "Test binary"));
    actions.append(&install);
    actions.append(&cancel);
    actions.append(&test);
    box_.append(&actions);
    box_.append(&footer_next(
        ctx,
        nav,
        cards,
        OnboardingCard::InstallRclone,
        &ctx.t_or("common.continue", "Continue"),
    ));

    {
        let ctx = ctx.clone();
        let binary = binary.clone();
        let message = message.clone();
        test.connect_clicked(move |_| {
            let path = binary.text().to_string();
            ctx.settings.borrow_mut().core.rclone_binary = path.clone();
            ctx.persist();
            if crate::installation::test_rclone_binary(&path)
                == crate::installation::BinaryStatus::Valid
            {
                message
                    .set_text(&ctx.t_or("onboarding.installButton.useBinary", "Binary is valid"));
            } else {
                message.set_text(
                    &ctx.t_or("onboarding.installButton.invalidBinary", "Invalid binary"),
                );
            }
        });
    }
    let cancel_flag = Rc::new(RefCell::new(None::<Arc<AtomicBool>>));
    {
        let cancel_flag = cancel_flag.clone();
        cancel.connect_clicked(move |_| {
            if let Some(flag) = cancel_flag.borrow().as_ref() {
                flag.store(true, Ordering::Relaxed);
            }
        });
    }
    {
        let ctx = ctx.clone();
        let custom = custom.clone();
        let binary = binary.clone();
        let bar = bar.clone();
        let message = message.clone();
        let status = status.clone();
        let install = install.clone();
        let cancel = cancel.clone();
        let cancel_flag = cancel_flag.clone();
        install.clone().connect_clicked(move |_| {
            let selected = match mode.selected() {
                1 => InstallLocation::Custom,
                2 => InstallLocation::Existing,
                _ => InstallLocation::Default,
            };
            if selected == InstallLocation::Existing {
                let path = binary.text().to_string();
                ctx.settings.borrow_mut().core.rclone_binary = path.clone();
                ctx.persist();
                if crate::installation::test_rclone_binary(&path)
                    == crate::installation::BinaryStatus::Valid
                {
                    status.set_icon_name(Some("emblem-ok-symbolic"));
                    message.set_text(
                        &ctx.t_or("onboarding.installButton.useBinary", "Binary is valid"),
                    );
                } else {
                    message.set_text(
                        &ctx.t_or("onboarding.installButton.invalidBinary", "Invalid binary"),
                    );
                }
                return;
            }
            let Some(dest) = rclone_install_dest(selected, &custom.text()) else {
                message.set_text(&ctx.t_or(
                    "onboarding.validation.completeInstallation",
                    "Please complete the installation options above",
                ));
                return;
            };
            let flag = Arc::new(AtomicBool::new(false));
            *cancel_flag.borrow_mut() = Some(flag.clone());
            let progress = Arc::new(Mutex::new(crate::updater::DownloadProgress::default()));
            bar.set_visible(true);
            cancel.set_visible(true);
            install.set_sensitive(false);
            message.set_text(&ctx.t_or("onboarding.installButton.installing", "Installing..."));
            let result: Arc<Mutex<Option<Result<std::path::PathBuf, String>>>> =
                Arc::new(Mutex::new(None));
            {
                let result = result.clone();
                let progress = progress.clone();
                std::thread::spawn(move || {
                    let outcome =
                        crate::updater::install_rclone_binary_ex(&dest, Some(flag), Some(progress));
                    if let Ok(mut slot) = result.lock() {
                        *slot = Some(outcome);
                    }
                });
            }
            let ctx = ctx.clone();
            let bar = bar.clone();
            let message = message.clone();
            let status = status.clone();
            let install = install.clone();
            let cancel = cancel.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                if let Ok(guard) = progress.lock() {
                    if guard.total == 0 {
                        bar.pulse();
                    } else {
                        bar.set_fraction(guard.fraction());
                    }
                    bar.set_text(Some(&guard.label()));
                }
                let finished = result.lock().ok().and_then(|mut slot| slot.take());
                if let Some(outcome) = finished {
                    install.set_sensitive(true);
                    cancel.set_visible(false);
                    match outcome {
                        Ok(path) => {
                            ctx.settings.borrow_mut().core.rclone_binary =
                                path.to_string_lossy().into_owned();
                            ctx.persist();
                            status.set_icon_name(Some("emblem-ok-symbolic"));
                            status.set_title(&ctx.t_or(
                                "onboarding.cards.installRclone.title",
                                "rclone is installed",
                            ));
                            message.set_text(&format!("Installed to {}", path.display()));
                            bar.set_fraction(1.0);
                        }
                        Err(e) if e == "cancelled" => {
                            message.set_text(&ctx.t_or("common.cancel", "Cancelled"));
                        }
                        Err(e) => message.set_text(&e),
                    }
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        });
    }

    adw::NavigationPage::builder()
        .tag(OnboardingCard::InstallRclone.tag())
        .title(&ctx.t_or(OnboardingCard::InstallRclone.title_key(), "Install rclone"))
        .child(&scrolled(&box_))
        .build()
}

fn page_mount(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::InstallPlugin);
    let status = adw::StatusPage::new();
    let label = crate::mount_plugin::plugin_label();
    status.set_title(&ctx.t_or(
        OnboardingCard::InstallPlugin.title_key(),
        "Install Mount Plugin",
    ));
    status.set_description(Some(&ctx.t_or(
        OnboardingCard::InstallPlugin.content_key(),
        "The mount plugin enables you to mount cloud storage as local drives.",
    )));
    if crate::mount_plugin::is_installed() {
        status.set_icon_name(Some("emblem-ok-symbolic"));
        status.set_title(&format!("{label} is ready"));
    } else {
        status.set_icon_name(Some("dialog-warning-symbolic"));
    }
    box_.append(&status);
    let hint = gtk::Label::new(Some(&crate::mount_plugin::missing_detail()));
    hint.add_css_class("dim-label");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    box_.append(&hint);
    let bar = gtk::ProgressBar::new();
    bar.set_show_text(true);
    bar.set_visible(false);
    box_.append(&bar);
    let message = gtk::Label::new(None);
    message.add_css_class("dim-label");
    message.set_wrap(true);
    message.set_xalign(0.0);
    box_.append(&message);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let install =
        gtk::Button::with_label(&ctx.t_or("onboarding.actions.installPlugin", "Install Plugin"));
    install.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(&ctx.t_or("common.cancel", "Cancel"));
    cancel.set_visible(false);
    actions.append(&install);
    actions.append(&cancel);
    box_.append(&actions);
    box_.append(&footer_next(
        ctx,
        nav,
        cards,
        OnboardingCard::InstallPlugin,
        &ctx.t_or("common.continue", "Continue"),
    ));

    let cancel_flag = Rc::new(RefCell::new(None::<Arc<AtomicBool>>));
    {
        let cancel_flag = cancel_flag.clone();
        cancel.connect_clicked(move |_| {
            if let Some(flag) = cancel_flag.borrow().as_ref() {
                flag.store(true, Ordering::Relaxed);
            }
        });
    }
    {
        let ctx = ctx.clone();
        let bar = bar.clone();
        let message = message.clone();
        let status = status.clone();
        let install = install.clone();
        let cancel = cancel.clone();
        let cancel_flag = cancel_flag.clone();
        install.clone().connect_clicked(move |_| {
            let flag = Arc::new(AtomicBool::new(false));
            *cancel_flag.borrow_mut() = Some(flag.clone());
            let progress = Arc::new(Mutex::new(crate::updater::DownloadProgress::default()));
            bar.set_visible(true);
            cancel.set_visible(true);
            install.set_sensitive(false);
            message.set_text(&ctx.t_or("onboarding.actions.installingPlugin", "Installing..."));
            let result: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
            {
                let result = result.clone();
                let progress = progress.clone();
                std::thread::spawn(move || {
                    let outcome = crate::mount_plugin::install_ex(Some(flag), Some(progress));
                    if let Ok(mut slot) = result.lock() {
                        *slot = Some(outcome);
                    }
                });
            }
            let ctx = ctx.clone();
            let bar = bar.clone();
            let message = message.clone();
            let status = status.clone();
            let install = install.clone();
            let cancel = cancel.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
                if let Ok(guard) = progress.lock() {
                    if guard.total == 0 {
                        bar.pulse();
                    } else {
                        bar.set_fraction(guard.fraction());
                    }
                    bar.set_text(Some(&guard.label()));
                }
                let finished = result.lock().ok().and_then(|mut slot| slot.take());
                if let Some(outcome) = finished {
                    install.set_sensitive(true);
                    cancel.set_visible(false);
                    match outcome {
                        Ok(msg) => {
                            status.set_icon_name(Some("emblem-ok-symbolic"));
                            status.set_title(&msg);
                            message.set_text(&ctx.t_or("common.continue", "You can continue."));
                            bar.set_fraction(1.0);
                        }
                        Err(e) if e == "cancelled" => {
                            message.set_text(&ctx.t_or("common.cancel", "Cancelled"));
                        }
                        Err(e) => message.set_text(&e),
                    }
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        });
    }

    adw::NavigationPage::builder()
        .tag(OnboardingCard::InstallPlugin.tag())
        .title(&ctx.t_or(OnboardingCard::InstallPlugin.title_key(), "Mount plugin"))
        .child(&scrolled(&box_))
        .build()
}

fn page_config(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
    window: &adw::ApplicationWindow,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::SelectConfig);
    let title = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::SelectConfig.title_key(),
        "Choose rclone.conf",
    )));
    title.add_css_class("title-1");
    title.set_wrap(true);
    title.set_xalign(0.0);
    let desc = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::SelectConfig.content_key(),
        "Use the default rclone configuration or pick a custom rclone.conf for this client.",
    )));
    desc.add_css_class("dim-label");
    desc.set_wrap(true);
    desc.set_xalign(0.0);
    let path = crate::ui::rows::entry_row();
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
        let window = window.clone();
        browse.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::new();
            let path = path.clone();
            dialog.open(
                Some(&window),
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
    group.set_title(&crate::ui::rows::escape(
        ctx.t_or(OnboardingCard::SelectConfig.title_key(), "Configuration"),
    ));
    group.add(&path);
    box_.append(&title);
    box_.append(&desc);
    box_.append(&group);
    box_.append(&footer_next(
        ctx,
        nav,
        cards,
        OnboardingCard::SelectConfig,
        &ctx.t_or("common.continue", "Continue"),
    ));
    adw::NavigationPage::builder()
        .tag(OnboardingCard::SelectConfig.tag())
        .title(&ctx.t_or(OnboardingCard::SelectConfig.title_key(), "Select config"))
        .child(&scrolled(&box_))
        .build()
}

fn page_password(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::PasswordRequired);
    let title = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::PasswordRequired.title_key(),
        "Config password",
    )));
    title.add_css_class("title-1");
    title.set_wrap(true);
    title.set_xalign(0.0);
    let desc = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::PasswordRequired.content_key(),
        "This rclone.conf is encrypted. Enter the password to unlock it before continuing.",
    )));
    desc.add_css_class("dim-label");
    desc.set_wrap(true);
    desc.set_xalign(0.0);
    let password = crate::ui::rows::password_entry_row();
    password.set_title(&ctx.t_or(
        OnboardingCard::PasswordRequired.title_key(),
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
    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_wrap(true);
    error.set_xalign(0.0);
    let group = adw::PreferencesGroup::new();
    group.add(&password);
    let unlock = gtk::Button::with_label(&ctx.t_or("onboarding.actions.unlock", "Unlock"));
    unlock.add_css_class("suggested-action");
    unlock.set_sensitive(!password.text().is_empty());
    {
        let unlock = unlock.clone();
        password.connect_changed(move |row| {
            unlock.set_sensitive(!row.text().is_empty());
        });
    }
    {
        let ctx = ctx.clone();
        let password = password.clone();
        let error = error.clone();
        let nav = nav.clone();
        let cards = cards.clone();
        unlock.connect_clicked(move |_| {
            let secret = password.text().to_string();
            let binary = ctx.settings.borrow().core.rclone_binary.clone();
            match crate::security::validate_password_for(ctx.client().as_ref(), &binary, &secret) {
                Ok(()) => {
                    error.set_text("");
                    push_next(&nav, &cards, OnboardingCard::PasswordRequired);
                }
                Err(_) => {
                    error.set_text(&ctx.t_or(
                        "onboarding.validation.wrongPassword",
                        "Wrong password. Please try again.",
                    ));
                }
            }
        });
    }
    box_.append(&title);
    box_.append(&desc);
    box_.append(&group);
    box_.append(&error);
    box_.append(&unlock);
    adw::NavigationPage::builder()
        .tag(OnboardingCard::PasswordRequired.tag())
        .title(&ctx.t_or(OnboardingCard::PasswordRequired.title_key(), "Password"))
        .child(&scrolled(&box_))
        .build()
}

fn page_view(
    ctx: &AppCtx,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::SelectMainUi);
    let title = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::SelectMainUi.title_key(),
        "Choose Primary UI",
    )));
    title.add_css_class("title-1");
    title.set_wrap(true);
    title.set_xalign(0.0);
    let desc = gtk::Label::new(Some(&ctx.t_or(
        OnboardingCard::SelectMainUi.content_key(),
        "Select your preferred workspace layout.",
    )));
    desc.add_css_class("dim-label");
    desc.set_wrap(true);
    desc.set_xalign(0.0);
    box_.append(&title);
    box_.append(&desc);
    let group = gtk::CheckButton::new();
    let selected = ctx.settings.borrow().general.default_view.clone();
    for option in MAIN_UI_OPTIONS {
        let row = crate::ui::rows::action_row();
        row.set_activatable(true);
        row.set_title(&ctx.t_or(option.title_key, option.id));
        row.set_subtitle(&format!(
            "{} · {}",
            ctx.t_or(option.badge_key, ""),
            ctx.t_or(option.desc_key, "")
        ));
        row.add_prefix(&gtk::Image::from_icon_name(option.icon));
        let radio = gtk::CheckButton::new();
        radio.set_group(Some(&group));
        radio.set_valign(gtk::Align::Center);
        if selected == option.id {
            radio.set_active(true);
        }
        {
            let ctx = ctx.clone();
            let id = option.id;
            radio.connect_toggled(move |btn| {
                if btn.is_active() {
                    ctx.settings.borrow_mut().general.default_view = id.to_string();
                    ctx.persist();
                }
            });
        }
        {
            let radio = radio.clone();
            row.connect_activated(move |_| {
                radio.set_active(true);
            });
        }
        row.add_suffix(&radio);
        box_.append(&row);
    }
    box_.append(&footer_next(
        ctx,
        nav,
        cards,
        OnboardingCard::SelectMainUi,
        &ctx.t_or("common.continue", "Continue"),
    ));
    adw::NavigationPage::builder()
        .tag(OnboardingCard::SelectMainUi.tag())
        .title(&ctx.t_or(OnboardingCard::SelectMainUi.title_key(), "Default view"))
        .child(&scrolled(&box_))
        .build()
}

fn page_ready(
    app: &adw::Application,
    ctx: &AppCtx,
    window: &adw::ApplicationWindow,
    nav: &adw::NavigationView,
    cards: &Rc<Vec<OnboardingCard>>,
) -> adw::NavigationPage {
    let box_ = page_box(ctx, nav, cards, OnboardingCard::Ready);
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("emblem-ok-symbolic"));
    status.set_title(&ctx.t_or(OnboardingCard::Ready.title_key(), "Ready to Go!"));
    status.set_description(Some(&ctx.t_or(
        OnboardingCard::Ready.content_key(),
        "Start managing remotes with the GTK 4 + Adwaita desktop client.",
    )));
    let finish = gtk::Button::with_label(&ctx.t_or("onboarding.actions.getStarted", "Get Started"));
    finish.add_css_class("suggested-action");
    finish.add_css_class("pill");
    let title = ctx.t_or(OnboardingCard::Ready.title_key(), "Ready");
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
    box_.append(&status);
    adw::NavigationPage::builder()
        .tag(OnboardingCard::Ready.tag())
        .title(&title)
        .child(&box_)
        .build()
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(child));
    scroll
}
