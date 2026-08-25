//! GTK 4 + libadwaita rewrite of Rclone Manager.

pub mod action_order;
pub mod automation;
pub mod backup;
pub mod connection;
pub mod fileops;
pub mod flags;
pub mod i18n;
pub mod interactive;
pub mod jobs;
pub mod keyring;
pub mod layout;
pub mod media;
pub mod mqtt;
pub mod operations;
pub mod path_inspection;
pub mod platform;
pub mod presets;
pub mod providers;
pub mod rclone;
pub mod rename;
pub mod repair;
pub mod restrict;
pub mod security;
pub mod settings;
pub mod smtp;
pub mod store;
pub mod syntax;
pub mod updater;
pub mod watch;

pub const APP_ID: &str = "io.github.zarestia_dev.rclone-manager";
pub const APP_NAME: &str = "Rclone Manager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
