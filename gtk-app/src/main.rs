mod backup;
mod flags;
mod i18n;
mod interactive;
mod jobs;
mod operations;
mod platform;
mod providers;
mod rclone;
mod settings;
mod store;
mod ui;
mod updater;

use gtk::prelude::*;

const APP_ID: &str = "io.github.zarestia_dev.rclone-manager";

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().collect();
    if let Some(send) = platform::parse_send_to_args(&args) {
        handle_send_to(send);
        return;
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(ui::activate);
    let code = app.run();
    std::process::exit(code.value());
}

fn handle_send_to(send: platform::SendToArgs) {
    let settings = settings::AppSettings::load();
    let engine = rclone::engine::RcloneEngine::start(&settings);
    if !engine.available {
        eprintln!("rclone engine is not available");
        std::process::exit(1);
    }
    let dest_fs = rclone::remote_fs(&send.remote, "");
    for file in &send.files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("upload");
        let dest = if send.path.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", send.path.trim_end_matches('/'), name)
        };
        match engine
            .client
            .copy_file("/", &file.to_string_lossy(), &dest_fs, &dest)
        {
            Ok(_) => println!("uploaded {} -> {}:{}", file.display(), send.remote, dest),
            Err(e) => eprintln!("failed to upload {}: {e}", file.display()),
        }
    }
}
