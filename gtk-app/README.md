# Rclone Manager GTK

Native **GTK 4 + libadwaita** desktop client for [Rclone Manager](https://github.com/Zarestia-Dev/rclone-manager). This is the application UI. The Rust backend lives in `src-tauri/`.

## Features

- **Main menu** — remotes sidebar, general / mount / operations / serve tabs, jobs, serves, disk usage
- **Nautilus file browser** — local + cloud listings, bookmarks, starred, copy/cut/paste, mkdir, rename, delete, viewer
- **Flow / Quick Runs** — reusable operations with cron, watcher, autostart, tray flag
- **Onboarding** — welcome, features, rclone detection, default view
- **Preferences** — language, theme, tray, notifications, rclone binary, bandwidth, log level
- **Alerts** — history, rules, actions (desktop toast, webhook, Telegram, WhatsApp, script)
- **Backup / restore** — zip export of settings, store, and rclone remotes
- **i18n** — loads `resources/i18n/{lang}/` (en, tr, es, zh, fr, uk, ru, pt, ja)

## Build

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config
cd gtk-app
cargo build --release
cargo test
```

Run:

```bash
cargo run --release
```

The app starts a local `rclone rcd` instance (localhost only) and talks to rclone’s RC API. The
daemon is given a random username and password on every launch, passed through the environment so
they stay out of the world-readable process arguments — the RC API can dump every remote’s
credentials, so a loopback bind alone is not a boundary on a shared machine.

## Tests

```bash
cargo test
```

Unit tests cover the operation registry, i18n flattening, settings paths, rclone path helpers, cron validation, alert matching, backup zip round-trip, and file-type detection. They do not require a display server.
