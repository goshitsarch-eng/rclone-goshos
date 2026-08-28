# Known Issues & Platform Notes

Behaviour that is expected, worked around, or waiting on upstream. If your problem
is not listed here, check the [Troubleshooting
Wiki](https://hakanismail.info/zarestia/rclone-manager/docs/troubleshooting) and
then [open an issue](https://github.com/Zarestia-Dev/rclone-manager/issues).

---

## rclone version requirements

Two different floors apply, on purpose:

| Component                         | Minimum rclone | Defined in                             |
| :-------------------------------- | :------------- | :------------------------------------- |
| GTK 4 desktop client (`gtk-app/`) | **1.65.0**     | `gtk-app/src/repair.rs`                |
| Tauri / headless backend          | **1.75.0**     | `src-tauri/src/core/check_binaries.rs` |

The GTK client keeps a lower floor because it degrades gracefully: it caches which
RC methods a given rclone lacks (`job/batch`, `serve/list`, `core/disks`, …) and
falls back — for example, `serve/start` is replaced by spawning a local
`rclone serve` process on rclone < 1.64, and `core/command serve` is deliberately
**not** used because it can panic `rcd` on 1.60.

Native OAuth (`config/create` with the rclone-hosted callback) needs **1.75+** on
any target. On older versions the wizard falls back to opening the authorization
URL in your browser.

If rclone is older than the floor, the Repair sheet shows a **Version too old**
banner with an "Update rclone" action.

---

## Linux

### FUSE / mounting

- Mounting needs FUSE 3. The app looks for `/dev/fuse`, `fusermount3`, or
  `fusermount`.
- rclone 1.60 `mount` execs `fusermount`, but Debian and Ubuntu's `fuse3` package
  only ships `fusermount3`. The Repair sheet can install a small shim script into
  a private directory and prepend it to the engine's `PATH`
  (`repair::fusermount_shim_script`). This is a workaround, not a bug in your
  system.
- Leftover mounts from a previous `rcd` that exited uncleanly are cleaned up with
  `fusermount3 -u` before a new mount on the same point is attempted.
- Mounting inside a Flatpak sandbox requires the `--device=all` permission; without
  it `/dev/fuse` is not visible.

### Send-to / file-manager integration

- "Add to File Manager Menu" writes four integrations: a Nautilus script, a
  `nautilus-python` `MenuProvider` extension, a KIO service menu (Dolphin), and a
  Nemo action. Your file manager has to be restarted before a new entry appears —
  `nautilus -q` for GNOME Files.
- The `nautilus-python` extension requires the `python3-nautilus` package
  (`nautilus-python` on Fedora). Without it, only the Scripts submenu entry works.

### System tray

- The tray uses the StatusNotifierItem D-Bus protocol (`ksni`). GNOME does **not**
  implement StatusNotifierItem out of the box — install the
  _AppIndicator and KStatusNotifierItem Support_ extension, or the tray icon will
  not appear. KDE, XFCE, Cinnamon, and Budgie work without an extension.
- Icons are published as ARGB32 pixmaps as well as by theme name, so the panel can
  draw them even when the icon theme has no matching entry.

### Prevent sleep

- The inhibitor is taken over logind's D-Bus `Inhibit` API, with a
  `systemd-inhibit` CLI fallback. On a system without systemd neither is available
  and the setting has no effect.

### Secrets

- The rclone config password is stored in the OS keyring via libsecret. On a system
  with no running secret service (many minimal WM setups), it falls back to
  `settings.json` in `~/.config/rclone-manager/`. That file is written with
  owner-only permissions, but it is still plaintext — prefer running a keyring.

---

## Windows

- Mounting requires [WinFsp](https://winfsp.dev/). It is not installed
  automatically.
- Unsigned builds trigger SmartScreen on first run. Choose _More info → Run anyway_.
- Stale WinFsp mounts and disconnected network shares used to hang drive
  enumeration; disk enumeration is now non-blocking with a timeout and a root
  fallback (fixed in 0.3.2, [#263](https://github.com/Zarestia-Dev/rclone-manager/issues/263)).
- The GTK client is **Linux only**. On Windows use the Tauri build.

## macOS

- Mounting requires [macFUSE](https://osxfuse.github.io/).
- The Send-to integration installs an Automator workflow under
  `~/Library/Services/`. macOS caches the Services menu; log out and back in if a
  new entry does not appear.
- The GTK client is **Linux only**. On macOS use the Tauri build.

## Android (Beta)

- Runs rclone in-process through `librclone` (Go FFI) rather than spawning a
  binary, so features that shell out to the `rclone` executable are unavailable.
- Local directory access goes through the Storage Access Framework; a folder must
  be authorized with the system tree picker before it can be used as a source or
  destination.
- iOS is unverified — no test devices.

---

## rclone RC API limitations

Several rough edges come from the RC API itself rather than from this app. They are
tracked, with proposed upstream fixes, in
[rclone-rc-limitations.md](rclone-rc-limitations.md). The ones users notice most:

- **Encrypted config detection** — there is no non-blocking way to ask whether the
  config is encrypted. If an encrypted config is locked and any RC command touches
  it, rclone waits on `stdin` and the whole RC server freezes. The app starts the
  engine with `RCLONE_CONFIG_PASS` already set to avoid this.
- **No `core/restart`** — settings that need an engine restart are applied by
  stopping and respawning `rcd`, which briefly interrupts running jobs.
- **VFS instance addressing** — with several mounts of the same remote, `vfs/list`
  returns suffixed IDs (`remote:[0]`) that `vfs/stats` / `vfs/refresh` /
  `vfs/forget` often reject, so per-instance VFS actions can fail.
- **Cache path reporting** — `config/paths` keeps reporting the startup cache
  directory even after `options/set` changes it.

---

## Not implemented yet

- **Workflow builder** — the Flow workspace's _Workflow_ tab is a placeholder. A
  visual canvas for multi-step pipelines and conditional job chaining is tracked in
  [#232](https://github.com/Zarestia-Dev/rclone-manager/issues/232). Quick Runs,
  cron schedules, and filesystem-watch automations cover the scheduled/triggered
  cases today.
- **Windows NotifyIcon / macOS NSStatusItem trays in the GTK client** — out of
  scope; the GTK client targets Linux.

---

See [CONTRIBUTING.md](CONTRIBUTING.md) to report or fix any of these, and
[LINTING.md](LINTING.md) for the checks a fix has to pass.
