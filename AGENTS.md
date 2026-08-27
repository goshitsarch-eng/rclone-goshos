# AGENTS.md

This file provides guidance to AI coding agents (e.g. Antigravity, Claude Code, Gemini CLI, Cursor, and similar tools) when working with code in this repository.

Rclone Manager welcomes AI-assisted contributions, but the expectation is that you, the human submitter, understand every line you propose and have compiled, linted, and tested it against real code — not just generated it. See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

---

## Project Overview

`rclone-manager` is a GUI and backend for managing [Rclone](https://rclone.org).

- **Desktop UI**: GTK 4 + libadwaita (`gtk-app/`).
- **Backend**: Rust, Tauri v2 (`src-tauri`), `librclone` C-FFI / Go integration.

There is no Angular (or other web-framework) application in this repository.

---

## General Notes & AI Guidelines

1. **Keep Changes Minimal & Elegant**: Work to make the smallest, most effective change possible. Avoid unneeded refactoring, re-ordering of imports, or restyling surrounding code.
2. **Backwards Compatibility**: PRs should preserve existing behavior across the GTK desktop client, the Rust backend, and headless web-server API targets.
3. **Verify Before Proposing**: AI agents must run build, lint, and formatting verification commands before declaring success.
4. **Automated Testing & Test Coverage (CRITICAL)**:
   - Whenever adding or refactoring business logic, services, utilities, parsers, or mappings, write or update accompanying automated unit tests.
   - **GTK client**: Rust unit tests (`#[cfg(test)]`) live in `gtk-app/src`.
   - **Backend**: Rust unit tests live inside `src-tauri` source files or test modules.
   - Cover both normal operation and edge cases (null inputs, empty values, special characters, error paths).

---

## Essential Code Quality & UI Rules

1. **Tooltip Usage**
   - Use native GTK/widget tooltips (`set_tooltip_text` / `title` attributes). Do not add a web tooltip toolkit.

2. **Menus**
   - Use GTK / libadwaita menus (`Gio.Menu`, `Gio.MenuButton`, popovers). Do not add Angular Material or CDK menus.

3. **Internationalization & Backend Error Architecture**
   - All translation files reside in `resources/i18n/{lang}/` (`main.json`, `rclone.json`, `rclone-providers.json`).
   - The GTK client loads those files directly.
   - The Rust backend (`src-tauri/src/utils/i18n.rs`) only reads `main.json` and caches keys needed for OS integrations (`tray`, `notification`, `powerInhibitor`, `alerts`, `backendErrors`).
   - Rust code must **never** pre-translate error messages. Always use `localized_error!("backendErrors.<category>.<key>", ...)` or `localized_success!("backendSuccess.<category>.<key>", ...)`.

4. **Platform abstraction**
   - GTK talks to a local `rclone rcd` (or an extra RC backend).
   - `src-tauri` remains the engine/backend for headless, tray, and packaged Tauri builds. It does not embed a web app UI.

5. **Automated Unit Testing**
   - New utility functions, data transformers, flag parsers, state machines, and business services should always have corresponding unit test suites.

---

## CI & Automated Workflows ([.github/workflows/](.github/workflows/))

- **[.github/workflows/ci.yml](.github/workflows/ci.yml)**: GTK unit tests plus Rust Clippy across desktop, web-server, and mobile backend features.
- **[.github/workflows/docker-build-push.yml](.github/workflows/docker-build-push.yml)**: Multi-architecture Docker container for the headless backend.
- **[.github/workflows/release-\*.yml](.github/workflows/)**: Cross-platform release workflows.

---

## Build, Test & Lint Commands

### 1. GTK 4 desktop client

```bash
cd gtk-app
cargo test --lib -- --test-threads=1
cargo build
cargo fmt -- --check
```

### 2. Backend (Rust / Tauri)

All Cargo commands are executed from the `src-tauri` directory.

> **Note on `LIBRCLONE_SKIP_LINK_CHECK`**:
> When `librclone.a` / Go toolchain is not compiled locally, set `export LIBRCLONE_SKIP_LINK_CHECK=1` so Clippy can type-check Rust without linking errors.

```bash
# 1. Clippy check - Desktop target
cd src-tauri && cargo clippy --features desktop --no-default-features -- -D warnings

# 2. Clippy check - Web Server target
cd src-tauri && TAURI_CONFIG=$(cat ./tauri.conf.headless.json) cargo clippy --features web-server --no-default-features -- -D warnings

# 3. Clippy check - Mobile target
cd src-tauri && cargo clippy --features mobile --no-default-features -- -D warnings

# 4. Rust formatting check
cd src-tauri && cargo fmt -- --check

# 5. Run backend unit tests
cd src-tauri && cargo test --features desktop --no-default-features
```

### 3. Local Development

```bash
# GTK desktop client
cd gtk-app && cargo run

# Tauri backend (no web app UI)
npm run tauri dev
npm run dev:headless
```

---

## Utility Scripts

- `npm run sync:endpoints`: Update endpoint definitions from rclone schemas.
- `npm run sync:providers`: Sync rclone provider configurations.
- `npm run sync:flags`: Sync rclone CLI flag definitions.
- `npm run audit:i18n`: Verify missing translation keys.
