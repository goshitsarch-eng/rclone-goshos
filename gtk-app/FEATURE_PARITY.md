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
- File viewer (image/text/video/audio locally, PDF via system app, remote download preview)
- Quick runs: create/edit/duplicate/delete, cron validation, watcher, autostart, tray flag, start/stop
- Workflow builder placeholder (same as current Angular stub)
- Preferences: language, default view, tray, startup (XDG autostart), notifications, restrict, prevent sleep (systemd-inhibit), rclone binary, bandwidth, tray item cap, log level
- Rclone flags editor: category pages (backend/filter/vfs/mount/copy/sync/check) writing `options/set`
- Backends: local rcd plus extra RC backends
- Alerts: history, rules, actions (os_toast, webhook, telegram, whatsapp, script, email/mqtt logged)
- Backup/restore zip with restore preview
- Template manager (capture current rclone options)
- Archive create job dialog
- Onboarding: welcome, features, rclone detect, default view, complete
- Keyboard shortcuts (global + nautilus)
- Theme: system / light / dark
- i18n: en-US, tr-TR, es-ES, zh-CN, fr-FR, uk-UA, ru-RU, pt-BR, ja-JP
- Native StatusNotifier tray (ksni) with remotes and quick runs
- Live job polling from rclone `job/list` + `job/status`, with failure alerts
- Provider list from `config/providers` when adding remotes
- Job detail, remote about, file properties; local media viewer
- Remote wizard: live provider fields (basic + advanced), secret obscure, default profiles, multi-step interactive `config/create` + `config/update` continue, OAuth URL open
- Nautilus tabs (Ctrl+T/W), real dual-pane split, drag-drop upload, undo, right-click context menu (archive / Send to), operations panel
- VFS stats/refresh/forget/queue on remote detail
- App + rclone update checks in About; rclone binary install to `~/.local/bin`
- Linux Send-to (Nautilus/Dolphin/Nemo) and `--send-to-remote` CLI upload path
- Autostart + prevent-sleep inhibitor while jobs/mounts/serves are active

- Profile-aware operation start dialog (src/dst/URL/mount/serve type, dry-run, static flags)
- Per-operation profiles in the remote wizard, CLI flag import, obscure tool
- Autostart of saved profiles and quick runs
- Remote order / visibility editor
- Editable local text files in the viewer
- Nautilus drag-lasso multi-select

## Still deepening toward pixel-level Angular parity

- CodeMirror-class syntax highlighting in the text viewer
- Standalone dialog windows (Tauri multi-window) and Windows/macOS send-to
- Live GUI session against rclone rcd (no display in this environment)
