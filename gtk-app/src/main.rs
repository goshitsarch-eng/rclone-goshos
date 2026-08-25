mod action_order;
mod alerts;
mod ansi;
mod automation;
mod backend_options;
mod backup;
mod checks;
mod cli;
mod cli_import;
mod command_options;
mod connection;
mod cron;
mod dnd;
mod fileops;
mod flags;
mod guidance;
mod i18n;
mod interactive;
mod jobs;
mod keyring;
mod layout;
mod logs;
mod markdown;
mod media;
mod migrate;
mod mime;
mod mount_plugin;
mod mqtt;
mod navigation;
mod onboarding;
mod operations;
mod path_inspection;
mod path_kind;
mod picker;
mod platform;
mod presets;
mod providers;
mod rclone;
mod refresh;
mod rename;
mod repair;
mod restrict;
mod security;
mod settings;
mod smtp;
mod store;
mod syntax;
mod textfix;
mod transfers;
mod tray_menu;
mod ui;
mod updater;
mod user_templates;
mod validators;
mod value_mapper;
mod vfs;
mod watch;

use gtk::gio;
use gtk::gio::prelude::*;
use gtk::glib::{OptionArg, OptionFlags};
use gtk::prelude::*;

const APP_ID: &str = "io.github.zarestia_dev.rclone-manager";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();
    cli::apply(&cli::parse_cli_args(&args));
    if let Some(files) = platform::parse_share_intake_args(&args) {
        platform::enqueue_share_intake(&files);
    }
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    register_application_options(&app);
    app.connect_command_line(|app, cmdline| {
        let mut args: Vec<String> = cmdline
            .arguments()
            .iter()
            .map(|s| s.to_str().unwrap_or_default().to_string())
            .collect();
        let env_args: Vec<String> = std::env::args().collect();
        if !args.iter().any(|arg| arg.starts_with("--"))
            && env_args.iter().any(|arg| arg.starts_with("--"))
        {
            args = env_args;
        }
        cli::set_launch_args(args.clone());
        if let Some(files) = platform::parse_share_intake_args(&args) {
            platform::enqueue_share_intake(&files);
        }
        app.activate();
        0
    });
    app.connect_activate(ui::activate);
    let code = app.run();
    std::process::exit(code.value());
}

fn register_application_options(app: &adw::Application) {
    let add = |name: &str, arg: OptionArg, desc: &str, hint: Option<&str>| {
        app.add_main_option(name, 0.into(), OptionFlags::NONE, arg, desc, hint);
    };
    add(
        "data-dir",
        OptionArg::Filename,
        "Override the application data directory",
        Some("DIR"),
    );
    add(
        "cache-dir",
        OptionArg::Filename,
        "Override the application cache directory",
        Some("DIR"),
    );
    add(
        "logs-dir",
        OptionArg::Filename,
        "Override the application logs directory",
        Some("DIR"),
    );
    add("tray", OptionArg::None, "Start hidden in the tray", None);
    add(
        "hidden",
        OptionArg::None,
        "Start with the window hidden",
        None,
    );
    add(
        "browse",
        OptionArg::String,
        "Open Files at remote[:path]",
        Some("REMOTE"),
    );
    add(
        "browse-path",
        OptionArg::String,
        "Path for --browse",
        Some("PATH"),
    );
    add("dashboard", OptionArg::None, "Open the dashboard", None);
    add("tab", OptionArg::String, "Dashboard tab", Some("TAB"));
    add(
        "remote",
        OptionArg::String,
        "Dashboard remote",
        Some("NAME"),
    );
    add("flow", OptionArg::None, "Open the Flow workspace", None);
    add(
        "quick-run",
        OptionArg::String,
        "Select a Quick Run",
        Some("ID"),
    );
    add("job", OptionArg::String, "Open a job", Some("ID"));
    add("serve", OptionArg::String, "Open a serve", Some("ID"));
    add(
        "automation",
        OptionArg::String,
        "Open an automation",
        Some("ID"),
    );
    add("updates", OptionArg::None, "Open the Updates dialog", None);
    add("alerts", OptionArg::None, "Open the Alerts dialog", None);
    app.add_main_option(
        "standalone",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open a standalone workspace (nautilus|flow|main)",
        Some("KIND"),
    );
    add(
        "send-to-remote",
        OptionArg::String,
        "Upload files to this remote",
        Some("REMOTE"),
    );
    add(
        "send-to-path",
        OptionArg::String,
        "Destination path for Send-to",
        Some("PATH"),
    );
    add(
        "share-intake",
        OptionArg::None,
        "Queue files for Files Upload here",
        None,
    );
    add(
        "dialog",
        OptionArg::String,
        "Open a standalone dialog",
        Some("TYPE"),
    );
    add(
        "dialog-data",
        OptionArg::String,
        "JSON payload for --dialog",
        Some("JSON"),
    );
    add(
        "dialog-result",
        OptionArg::Filename,
        "Result file for --dialog",
        Some("PATH"),
    );
    app.set_option_context_parameter_string(Some("[URL or FILE…]"));
}
