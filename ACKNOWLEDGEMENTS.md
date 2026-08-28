# 🙏 Acknowledgements

RClone Manager is a graphical front end. Almost everything it makes easy was made
possible by someone else's work first. This page records who, and under what terms.

---

## The projects this one is built on

### rclone — the engine

Every transfer, mount, serve, and remote definition in this application is
performed by **[rclone](https://rclone.org)**. This project does not reimplement
any of it: it drives a local `rclone rcd` instance over rclone's Remote Control
API and presents the result.

> **rclone** — © Nick Craig-Wood and the rclone contributors
> [rclone.org](https://rclone.org) · [github.com/rclone/rclone](https://github.com/rclone/rclone)
> MIT License

rclone is a genuinely remarkable piece of engineering — 70+ storage backends, a
VFS layer, bisync, crypt, and an RC API stable enough for a GUI to sit on. If this
app is useful to you, the credit for the hard part belongs upstream. Please
consider [sponsoring rclone](https://rclone.org/sponsor/).

The rough edges we work around in the RC API — and the fixes we would like to see
upstream — are written up in [rclone-rc-limitations.md](rclone-rc-limitations.md),
offered in the spirit of contributing back rather than complaining.

### RClone Manager — the upstream application

This repository is a derivative of **RClone Manager**, created and maintained by
**Hakan İSMAİL** ([@Hakanbaban53](https://github.com/Hakanbaban53)) and the
Zarestia Dev team. The application design, the Rust/Tauri backend, the operations
model, the i18n architecture, the alert and automation systems, and the great
majority of the codebase originate there.

> **RClone Manager** — © Hakan İSMAİL and contributors
> [github.com/Zarestia-Dev/rclone-manager](https://github.com/Zarestia-Dev/rclone-manager)
> GNU GPL v3.0-or-later

Because upstream is GPL-3.0-or-later, this fork is too. See [LICENSE](LICENSE).
Everyone who has contributed to upstream is listed in
[CONTRIBUTORS.md](CONTRIBUTORS.md); please add fixes there as well as here where
they apply.

### GNOME — the toolkit

The Linux desktop client is written directly against GNOME's toolkit, and follows
the GNOME Human Interface Guidelines.

> **GTK 4** — © The GNOME Project and the GTK team · [gtk.org](https://www.gtk.org) · LGPL-2.1-or-later
> **libadwaita** — © The GNOME Project · [gnome.pages.gitlab.gnome.org/libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/) · LGPL-2.1-or-later
> **gtk-rs** (`gtk4`, `libadwaita`, `glib`, `gio` Rust bindings) — © the gtk-rs developers · [gtk-rs.org](https://gtk-rs.org) · MIT

The file browser is named _Nautilus_ in tribute to
[GNOME Files](https://apps.gnome.org/Nautilus/), whose interaction model it
follows. It is a separate implementation, not GNOME Files code.

### Tauri — the packaged backend

> **Tauri** — © Tauri Programme within The Commons Conservancy
> [tauri.app](https://tauri.app) · MIT / Apache-2.0

Tauri hosts the cross-platform backend (`src-tauri/`) used for the Windows, macOS,
Android, and headless web-server builds.

### Rust

> **The Rust programming language** — © The Rust Project Developers
> [rust-lang.org](https://www.rust-lang.org) · MIT / Apache-2.0

---

## Rust crates

The GTK client (`gtk-app/`) depends directly on:

| Crate                                                                                                                                                                                                                                                                                                                                                   | License                 | What it does here                                |
| :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | :---------------------- | :----------------------------------------------- |
| [`gtk4`](https://crates.io/crates/gtk4)                                                                                                                                                                                                                                                                                                                 | MIT                     | GTK 4 bindings                                   |
| [`libadwaita`](https://crates.io/crates/libadwaita)                                                                                                                                                                                                                                                                                                     | MIT                     | Adwaita widgets                                  |
| [`glib`](https://crates.io/crates/glib) / [`gio`](https://crates.io/crates/gio)                                                                                                                                                                                                                                                                         | MIT                     | Main loop, actions, settings, file I/O           |
| [`serde`](https://crates.io/crates/serde) / [`serde_json`](https://crates.io/crates/serde_json)                                                                                                                                                                                                                                                         | MIT OR Apache-2.0       | Config, store, and RC payload serialization      |
| [`ureq`](https://crates.io/crates/ureq)                                                                                                                                                                                                                                                                                                                 | MIT OR Apache-2.0       | HTTP client for the rclone RC API                |
| [`chrono`](https://crates.io/crates/chrono)                                                                                                                                                                                                                                                                                                             | MIT OR Apache-2.0       | Timestamps and schedule maths                    |
| [`croner`](https://crates.io/crates/croner)                                                                                                                                                                                                                                                                                                             | MIT                     | Cron expression parsing for automations          |
| [`notify`](https://crates.io/crates/notify)                                                                                                                                                                                                                                                                                                             | CC0-1.0                 | Filesystem watch automations                     |
| [`notify-rust`](https://crates.io/crates/notify-rust)                                                                                                                                                                                                                                                                                                   | MIT OR Apache-2.0       | Desktop notifications                            |
| [`ksni`](https://crates.io/crates/ksni)                                                                                                                                                                                                                                                                                                                 | Unlicense               | StatusNotifierItem system tray                   |
| [`keyring`](https://crates.io/crates/keyring)                                                                                                                                                                                                                                                                                                           | MIT OR Apache-2.0       | rclone config password in the OS keyring         |
| [`dbus`](https://crates.io/crates/dbus)                                                                                                                                                                                                                                                                                                                 | Apache-2.0 / MIT        | logind sleep inhibitor                           |
| [`zip`](https://crates.io/crates/zip)                                                                                                                                                                                                                                                                                                                   | MIT                     | Backup and restore archives                      |
| [`sha2`](https://crates.io/crates/sha2)                                                                                                                                                                                                                                                                                                                 | MIT OR Apache-2.0       | File hashing                                     |
| [`native-tls`](https://crates.io/crates/native-tls)                                                                                                                                                                                                                                                                                                     | MIT OR Apache-2.0       | SMTP / MQTT alert transports                     |
| [`uuid`](https://crates.io/crates/uuid)                                                                                                                                                                                                                                                                                                                 | Apache-2.0 OR MIT       | Profile and job identifiers                      |
| [`which`](https://crates.io/crates/which)                                                                                                                                                                                                                                                                                                               | MIT                     | Locating the rclone binary                       |
| [`open`](https://crates.io/crates/open)                                                                                                                                                                                                                                                                                                                 | MIT                     | Opening URLs and files in the desktop handler    |
| [`dirs`](https://crates.io/crates/dirs)                                                                                                                                                                                                                                                                                                                 | MIT OR Apache-2.0       | XDG directory resolution                         |
| [`base64`](https://crates.io/crates/base64), [`urlencoding`](https://crates.io/crates/urlencoding), [`sys-locale`](https://crates.io/crates/sys-locale), [`thiserror`](https://crates.io/crates/thiserror), [`log`](https://crates.io/crates/log), [`env_logger`](https://crates.io/crates/env_logger), [`tempfile`](https://crates.io/crates/tempfile) | MIT / MIT OR Apache-2.0 | Encoding, locale, errors, logging, test fixtures |

The Tauri backend (`src-tauri/`) additionally uses
[`tokio`](https://crates.io/crates/tokio),
[`axum`](https://crates.io/crates/axum),
[`tower-http`](https://crates.io/crates/tower-http),
[`reqwest`](https://crates.io/crates/reqwest),
[`rustls`](https://crates.io/crates/rustls),
[`lettre`](https://crates.io/crates/lettre),
[`rumqttc`](https://crates.io/crates/rumqttc),
[`handlebars`](https://crates.io/crates/handlebars),
[`sysinfo`](https://crates.io/crates/sysinfo),
[`tokio-cron-scheduler`](https://crates.io/crates/tokio-cron-scheduler),
[`lofty`](https://crates.io/crates/lofty),
[`clap`](https://crates.io/crates/clap), and
[`rcman`](https://crates.io/crates/rcman), among others.

Full, authoritative version and license data lives in `gtk-app/Cargo.lock` and
`src-tauri/Cargo.lock`. To regenerate the list:

```bash
cargo install cargo-license
cd gtk-app && cargo license
```

---

## People

- **Contributors** to this application, upstream and here, are listed in
  [CONTRIBUTORS.md](CONTRIBUTORS.md).
- **Translators** work on [Crowdin](https://crowdin.com/project/rclone-manger).
  The app ships in English, Turkish, Spanish, French, Ukrainian, Russian,
  Brazilian Portuguese, Japanese, and Simplified Chinese entirely because
  volunteers put it there.
- **Bug reporters** — a reproducible report is a contribution. Several of the
  fixes in [CHANGELOG.md](CHANGELOG.md) exist only because someone took the time
  to describe exactly what broke.

---

## Conventions and tooling

- [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) — CHANGELOG format
- [Conventional Commits](https://www.conventionalcommits.org/) — commit messages
- [Semantic Versioning](https://semver.org/) — release numbering
- [GNOME Human Interface Guidelines](https://developer.gnome.org/hig/) — UI design
- [freedesktop.org](https://www.freedesktop.org/) specifications — desktop entries,
  MIME, autostart, StatusNotifierItem, and the XDG base directories

---

## License

This project is licensed under the [GNU GPL v3.0-or-later](LICENSE), inherited from
upstream RClone Manager. The dependencies above remain under their own licenses,
listed beside each entry.

Nothing here implies endorsement: rclone, GNOME, the Tauri project, and upstream
RClone Manager are independent projects and have not reviewed or approved this
fork.
