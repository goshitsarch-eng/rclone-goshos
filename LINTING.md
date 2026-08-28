# Linting & Code Style

All code must pass linting, formatting, and the unit-test suite before it is merged.
The same commands run in [CI](.github/workflows/ci.yml), so running them locally is
the fastest way to get a green pull request.

---

## Quick reference

| Target                          | Format                      | Lint                                                                                   | Test                                                                  |
| :------------------------------ | :-------------------------- | :------------------------------------------------------------------------------------- | :-------------------------------------------------------------------- |
| GTK desktop client (`gtk-app/`) | `cd gtk-app && cargo fmt`   | `cd gtk-app && cargo clippy --all-targets`                                             | `cd gtk-app && cargo test --lib -- --test-threads=1`                  |
| Backend (`src-tauri/`)          | `cd src-tauri && cargo fmt` | `cd src-tauri && cargo clippy --features desktop --no-default-features -- -D warnings` | `cd src-tauri && cargo test --features desktop --no-default-features` |
| Translations                    | —                           | `npm run audit:i18n`                                                                   | —                                                                     |
| Markdown / JSON / YAML          | `npx prettier --write .`    | `npx prettier --check .`                                                               | —                                                                     |

Run everything before pushing:

```bash
cd gtk-app   && cargo fmt -- --check && cargo clippy --all-targets && cargo test --lib -- --test-threads=1
cd ../src-tauri && cargo fmt -- --check
cd ..        && npm run audit:i18n
```

---

## Build prerequisites

The GTK client needs GTK 4, libadwaita, D-Bus, and libsecret development headers:

```bash
# Debian / Ubuntu
sudo apt install build-essential pkg-config \
  libgtk-4-dev libadwaita-1-dev libdbus-1-dev libsecret-1-dev libssl-dev

# Fedora
sudo dnf install gcc pkgconf-pkg-config \
  gtk4-devel libadwaita-devel dbus-devel libsecret-devel openssl-devel
```

### `LIBRCLONE_SKIP_LINK_CHECK`

`src-tauri/build.rs` links against `librclone.a`, which requires a Go toolchain and
a checked-out rclone source tree. When you only want to type-check Rust, set:

```bash
export LIBRCLONE_SKIP_LINK_CHECK=1
```

`build.rs` then emits a `cargo:warning=…` instead of panicking. Clippy still
type-checks the FFI module — it just does not link. **Release builds must not set
this variable.**

---

## Backend feature matrix

`src-tauri` compiles for three mutually exclusive targets. CI lints all three, so
check the one you touched:

```bash
cd src-tauri

# Desktop (Tauri app)
cargo clippy --features desktop --no-default-features -- -D warnings

# Headless web server
TAURI_CONFIG=$(cat ./tauri.conf.headless.json) \
  cargo clippy --features web-server --no-default-features -- -D warnings

# Mobile (Android / librclone FFI)
cargo clippy --features mobile --no-default-features -- -D warnings
```

A helper that is used by only one target still has to compile cleanly for the
others. Prefer `#[cfg_attr(not(target_os = "linux"), allow(dead_code))]` over an
unconditional `#[allow(dead_code)]` so the warning still fires where the code is
actually reachable.

---

## Style rules

These are the project-specific rules that Clippy cannot enforce for you. They are
the same rules listed in [AGENTS.md](AGENTS.md).

### 1. Keep changes minimal

Make the smallest effective change. Do not reorder imports, restyle surrounding
code, or refactor adjacent functions in a PR that is about something else — it
buries the real diff.

### 2. Never pre-translate backend errors

Rust code must not build user-facing English strings. Always go through the
localization macros so the GTK client can render the message in the user's
language:

```rust
// Good
return Err(localized_error!("backendErrors.mount.notMounted", "remote" => remote));

// Bad — untranslatable, and duplicated in every locale
return Err(format!("Remote {remote} is not mounted"));
```

Translation files live in `resources/i18n/{lang}/` (`main.json`, `rclone.json`,
`rclone-providers.json`). `src-tauri/src/utils/i18n.rs` reads only `main.json` and
caches the keys needed for OS integrations (`tray`, `notification`,
`powerInhibitor`, `alerts`, `backendErrors`).

In the GTK client, use `ctx.t_or("key.path", "English fallback")` — never a bare
string literal in a user-visible widget.

### 3. Native widgets only

- **Tooltips**: `widget.set_tooltip_text(Some(…))`. Do not add a tooltip toolkit.
- **Menus**: `gio::Menu`, `gtk::MenuButton`, `gtk::PopoverMenu`.
- **Empty states**: `adw::StatusPage`, not a hand-rolled label stack.
- **Lists**: `adw::PreferencesGroup` / the `.boxed-list` style class.

### 4. Colors come from libadwaita

Use named colors (`@accent_bg_color`, `@card_bg_color`, `@window_fg_color`,
`@success_color`, `@warning_color`, `@destructive_bg_color`) or `alpha()` blends
of them. A hardcoded hex value breaks dark mode and user accent colors.

### 5. Escape anything you interpolate into another language

Values that reach a shell command, a `.desktop` `Exec=` line, a generated Python
extension, or an XML document must be escaped for that target — remote paths are
user data and can contain quotes, `$`, and backticks. See
`platform::escape_double_quoted` and `platform::escape_python_string`.

### 6. `RefCell` borrows must not outlive a re-entrant call

This compiles, and panics at runtime:

```rust
// BAD: the RefMut is lifetime-extended to the end of the block,
// so ctx.persist() -> settings.borrow() panics.
let flags = &mut ctx.settings.borrow_mut().core.rclone_additional_flags;
flags.push(flag);
ctx.persist();
```

Scope the borrow instead:

```rust
{
    let mut settings = ctx.settings.borrow_mut();
    settings.core.rclone_additional_flags.push(flag);
}
ctx.persist();
```

The same trap applies to `if let Some(x) = cell.borrow_mut()…` in edition 2021 —
the temporary lives until the end of the whole `if let`, including the `else`.

---

## Tests

Whenever you add or refactor business logic, parsers, mappings, state machines, or
utilities, add unit tests alongside them.

- **GTK client**: `#[cfg(test)] mod tests` inside `gtk-app/src/**`.
- **Backend**: `#[cfg(test)] mod tests` inside `src-tauri/src/**`.

Cover the error paths too — empty values, special characters, and malformed input
are where the real bugs live. Tests that assert nothing meaningful (`assert!(true)`,
or re-asserting a literal you just constructed) are worse than no test, because they
make coverage look better than it is.

GTK tests must run single-threaded (`-- --test-threads=1`); several of them touch
process-global state.

---

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add bisync profile to the remote wizard
fix: escape remote paths in generated Send-to scripts
docs: document the backend feature matrix
refactor: extract job progress formatting
style: run cargo fmt
test: cover cron weekday parsing
chore: bump dependencies
```

---

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contribution workflow and
[ISSUES.md](ISSUES.md) for known platform-specific limitations.
