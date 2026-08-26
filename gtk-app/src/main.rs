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
mod config_import;
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
mod pref_search;
mod presets;
mod providers;
mod rclone;
mod refresh;
mod rename;
mod repair;
mod restrict;
mod security;
mod settings;
mod shortcuts;
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
    if let Err(err) = platform::install_user_desktop_entry() {
        log::debug!("desktop entry not installed: {err}");
    }
    if let Err(err) = platform::install_user_mime_package() {
        log::debug!("mime package not installed: {err}");
    }
    if let Err(err) = platform::install_user_metainfo() {
        log::debug!("metainfo not installed: {err}");
    }
    if let Some(files) = platform::parse_share_intake_args(&args) {
        platform::enqueue_share_intake(&files);
    }
    let open_configs = config_import::parse_open_config_args(&args);
    if !open_configs.is_empty() {
        config_import::enqueue_open_configs(&open_configs);
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
        cli::merge_option_flags(&mut args, &command_line_option_flags(cmdline));
        if !cmdline.is_remote() {
            let env_args: Vec<String> = std::env::args().collect();
            if env_args.len() > 1 {
                args = env_args;
            }
        }
        cli::set_launch_args(args.clone());
        if let Some(files) = platform::parse_share_intake_args(&args) {
            platform::enqueue_share_intake(&files);
        }
        let open_configs = config_import::parse_open_config_args(&args);
        if !open_configs.is_empty() {
            config_import::enqueue_open_configs(&open_configs);
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
        "tray-action",
        OptionArg::String,
        "Dispatch a tray menu action to the running instance",
        Some("ACTION"),
    );
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
        "preferences",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open Preferences, optionally a page",
        Some("PAGE"),
    );
    app.add_main_option(
        "settings",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Alias for --preferences",
        Some("PAGE"),
    );
    add(
        "onboarding",
        OptionArg::None,
        "Re-open the first-run onboarding window",
        None,
    );
    add("about", OptionArg::None, "Open the About dialog", None);
    app.add_main_option(
        "logs",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open the Logs dialog, optionally scoped to a remote",
        Some("REMOTE"),
    );
    add(
        "auto-add",
        OptionArg::None,
        "Prompt for a new profile in remote-config",
        None,
    );
    add(
        "shortcuts",
        OptionArg::None,
        "Open the keyboard shortcuts dialog",
        None,
    );
    add(
        "backends",
        OptionArg::None,
        "Open the Backends dialog",
        None,
    );
    add(
        "rclone-flags",
        OptionArg::None,
        "Open the Rclone Flags dialog",
        None,
    );
    add("flags", OptionArg::None, "Alias for --rclone-flags", None);
    app.add_main_option(
        "templates",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open the Templates dialog, optionally the Save tab",
        Some("MODE"),
    );
    add(
        "quick-add",
        OptionArg::None,
        "Open the Quick Add remote wizard",
        None,
    );
    add("quickadd", OptionArg::None, "Alias for --quick-add", None);
    app.add_main_option(
        "whats-new",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open What's New (app or rclone)",
        Some("KIND"),
    );
    add(
        "whats-new-app",
        OptionArg::None,
        "Open app release notes",
        None,
    );
    add(
        "whats-new-rclone",
        OptionArg::None,
        "Open rclone release notes",
        None,
    );
    add(
        "properties",
        OptionArg::String,
        "Open file properties for remote:path",
        Some("REMOTE:PATH"),
    );
    add(
        "clone-from",
        OptionArg::String,
        "Clone a remote into the configuration wizard",
        Some("REMOTE"),
    );
    add(
        "file-viewer",
        OptionArg::String,
        "Open the file viewer for remote:path",
        Some("REMOTE:PATH"),
    );
    add(
        "start-operation",
        OptionArg::String,
        "Open Start Operation (remote or remote:operation)",
        Some("REMOTE[:OP]"),
    );
    add(
        "operation",
        OptionArg::String,
        "Operation type for --start-operation",
        Some("OP"),
    );
    add(
        "vfs",
        OptionArg::String,
        "Open VFS controls for a remote",
        Some("REMOTE"),
    );
    add(
        "delete-remote",
        OptionArg::String,
        "Open Delete Remote",
        Some("REMOTE"),
    );
    add(
        "remote-about",
        OptionArg::String,
        "Open Remote About",
        Some("REMOTE"),
    );
    add(
        "restore-preview",
        OptionArg::Filename,
        "Open Restore Backup for a zip file",
        Some("FILE"),
    );
    add(
        "archive-create",
        OptionArg::String,
        "Open Create Archive for remote:path",
        Some("REMOTE:PATH"),
    );
    app.add_main_option(
        "quick-run-editor",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open the Quick Run editor, optionally for an id",
        Some("ID"),
    );
    app.add_main_option(
        "export",
        0.into(),
        OptionFlags::OPTIONAL_ARG,
        OptionArg::String,
        "Open Export, optionally scoped to a remote",
        Some("REMOTE"),
    );
    add("repair", OptionArg::None, "Open the Repair sheet", None);
    add(
        "remote-config",
        OptionArg::String,
        "Open remote configuration",
        Some("REMOTE"),
    );
    add(
        "step",
        OptionArg::String,
        "Remote-config step (with --remote-config)",
        Some("STEP"),
    );
    add(
        "profile",
        OptionArg::String,
        "Remote-config profile (with --remote-config)",
        Some("NAME"),
    );
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
        "import-config",
        OptionArg::Filename,
        "Import remotes from an rclone.conf / rclone.json file",
        Some("FILE"),
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

fn command_line_option_flags(
    cmdline: &gio::ApplicationCommandLine,
) -> Vec<(String, Option<String>)> {
    const NAMES: &[&str] = &[
        "preferences",
        "settings",
        "onboarding",
        "about",
        "logs",
        "auto-add",
        "shortcuts",
        "backends",
        "rclone-flags",
        "flags",
        "templates",
        "quick-add",
        "quickadd",
        "whats-new",
        "whats-new-app",
        "whats-new-rclone",
        "properties",
        "clone-from",
        "file-viewer",
        "start-operation",
        "operation",
        "vfs",
        "delete-remote",
        "remote-about",
        "restore-preview",
        "archive-create",
        "quick-run-editor",
        "export",
        "repair",
        "remote-config",
        "step",
        "profile",
        "updates",
        "alerts",
        "dashboard",
        "tab",
        "remote",
        "flow",
        "quick-run",
        "job",
        "serve",
        "automation",
        "browse",
        "browse-path",
        "standalone",
        "import-config",
        "tray",
        "tray-action",
        "hidden",
        "dialog",
        "dialog-data",
        "dialog-result",
    ];
    let dict = cmdline.options_dict();
    let mut flags = Vec::new();
    for name in NAMES {
        let Some(value) = dict.lookup_value(name, None) else {
            continue;
        };
        if let Some(flag) = cli::option_flag_from_variant(name, &value) {
            flags.push(flag);
        }
    }
    flags
}
