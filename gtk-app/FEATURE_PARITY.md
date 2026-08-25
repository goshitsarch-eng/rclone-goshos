# GTK rewrite feature parity

This client is the GTK 4 + libadwaita rewrite of the Angular/Tauri desktop UI. It talks to a local `rclone rcd` instance and persists app state under `~/.config/rclone-manager/`.

## Implemented

- Workspaces: Main menu, Nautilus file browser, Flow
- Dashboard tabs: general, mount, operations, serve
- Remote sidebar search, status badges, overview + detail
- All 11 primary operations (mount, sync, copy, move, bisync, serve, check, delete, copyurl, archivecreate, cryptcheck)
- Serve types: http, webdav, ftp, sftp, nfs, dlna, restic, s3
- Quick add / detailed remote create-update-delete
- Mount / unmount, start/stop jobs, stop all serves
- File browser: path bar, history, parent, hidden files, mkdir, rename, delete, copy/cut/paste, upload picker, bookmarks, starred, local disks, remotes
- File viewer (image locally, open-native, type metadata)
- Quick runs: create/edit/duplicate/delete, cron validation, watcher, autostart, tray flag, start/stop
- Workflow builder placeholder (same as current Angular stub)
- Preferences: language, default view, tray, startup, notifications, restrict, prevent sleep, rclone binary, bandwidth, tray item cap, log level
- Rclone flags inspector, backends status, logs
- Alerts: history, rules, actions (os_toast, webhook, telegram, whatsapp, script, email/mqtt logged)
- Backup/restore zip (settings, store, rclone dump)
- Onboarding: welcome, features, rclone detect, default view, complete
- Keyboard shortcuts (global + nautilus)
- Theme: system / light / dark
- i18n: en-US, tr-TR, es-ES, zh-CN, fr-FR, uk-UA, ru-RU, pt-BR, ja-JP
- Tray action menu: unmount all, stop jobs, stop serves

## Still deepening toward pixel-level Angular parity

- Full provider OAuth/interactive config wizard and flag-category editors
- Nautilus split panes, multi-tabs, lasso, CodeMirror/PDF/media viewers
- Native StatusNotifier tray icon process
- App/rclone updater install flows
- Per-profile VFS/filter/backend editors identical to the Angular forms
