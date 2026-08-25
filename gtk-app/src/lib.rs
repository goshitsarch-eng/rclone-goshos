//! GTK 4 + libadwaita rewrite of Rclone Manager.

pub mod backup;
pub mod i18n;
pub mod operations;
pub mod providers;
pub mod rclone;
pub mod settings;
pub mod store;
pub mod updater;

pub const APP_ID: &str = "io.github.zarestia_dev.rclone-manager";
pub const APP_NAME: &str = "Rclone Manager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
