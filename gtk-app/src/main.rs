mod backup;
mod i18n;
mod operations;
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
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(ui::activate);
    let code = app.run();
    std::process::exit(code.value());
}
