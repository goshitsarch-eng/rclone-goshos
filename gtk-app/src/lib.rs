//! GTK 4 + libadwaita rewrite of Rclone Manager.

pub mod automation;
pub mod backup;
pub mod connection;
pub mod flags;
pub mod i18n;
pub mod interactive;
pub mod jobs;
pub mod mqtt;
pub mod operations;
pub mod platform;
pub mod providers;
pub mod rclone;
pub mod rename;
pub mod security;
pub mod settings;
pub mod smtp;
pub mod store;
pub mod syntax;
pub mod updater;

pub const APP_ID: &str = "io.github.zarestia_dev.rclone-manager";
pub const APP_NAME: &str = "Rclone Manager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
