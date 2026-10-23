# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Oxitailr is a log viewer / live tailer built in Rust, with local-file and SSH log sources, in
two forms on one engine: a **terminal app** (`ratatui`/`crossterm`) and a **desktop app**
(Tauri 2 + React), both on the `oxitailr-core` library. v0.3.0 replaced the original egui GUI
with the terminal app; v0.4 split the engine into `oxitailr-core` and added the desktop app.
Nothing from the egui era is left (no `eframe`, `TailLoggerApp` or `src/ui/`).

## Workflow: straight to `main`, no PRs

Marcy works alone on this repo: **build, commit to `main`, push, repeat.** No feature
branches, no pull requests, no stacked PRs; they are extra steps with nothing to gain for one
person. CI runs on every push to `main` and is the check: if it goes red, fix it in the next
commit. Ask before committing and pushing.

## Workspace layout

The repo is a Cargo workspace (since the 0.4 cycle):

- `crates/oxitailr-core/` — the engine, a library: `source/`, `parser/`, `filter/`, `alert/`,
  `config/`, `state/`, `models/`, `secrets.rs`, and `app.rs` (`AppCore`). Its tests,
  including `tests/integration_tests.rs`, live here.
- `crates/oxitailr/` — the terminal app: `main.rs` (CLI) and `tui.rs`. It depends on
  `oxitailr-core` and builds the `oxitailr` binary (still `target/release/oxitailr`).
- `desktop/` — the desktop app (Tauri 2). The front end (React + Vite + TypeScript) is in
  `desktop/src`; the Tauri crate `oxitailr-desktop` is in `desktop/src-tauri`, with the
  commands in `src-tauri/src/main.rs` and the view logic in `src-tauri/src/view.rs`.
  `desktop/src/api.ts` is the **only** place the front end calls `invoke`. The crate is a
  workspace member but not a default member, so plain `cargo build/test/clippy` at the root
  skip it (it needs WebKitGTK / WebView2); `cargo fmt` at the root does format it.

Desktop commands:

```bash
cd desktop && npm ci && npm run tauri dev       # run the desktop app
cd desktop && npm run typecheck                 # tsc --noEmit
cd desktop && npm run build                     # front end into desktop/dist (needed before cargo)
cargo clippy -p oxitailr-desktop -- -D warnings
cargo test -p oxitailr-desktop
```

Desktop design rules:

- Only the rows on screen are fetched from the back end; never ship the whole buffer.
- The back end notifies the front end "changed" at most 10 times a second, and nothing
  ticks while idle (no polling timers).
- No literal colors outside the palette files `desktop/src/tokens.css` and `desktop/src/levels.css`; everything else uses their tokens (config highlight colors are the one data-driven exception).
- Dependencies are pinned exactly (`desktop/.npmrc` has `save-exact=true`) and kept on the
  latest releases.

Shared dependency versions live in the root `Cargo.toml` under `[workspace.dependencies]`;
each crate opts in with `name.workspace = true`. Paths in the Architecture section below are
relative to `crates/oxitailr-core/src/` unless they name `main.rs`/`tui.rs`.

## Commands

```bash
# Build
cargo build                  # debug
cargo build --release        # release (what CI ships)

# Run (files optional; can also open via file picker in the UI)
cargo run -- /var/log/syslog
cargo run -- '/var/log/*.log'        # glob patterns are expanded into one source per match
cargo run -- FILE -c config.toml -m 50000   # -c custom config, -m buffer size (max lines)

# Test
cargo test                                   # all tests
cargo test --test integration_tests          # the integration test binary
cargo test <name_substring>                  # a single test by name

# Lint / format — CI enforces both and fails on any warning
cargo fmt -- --check
cargo clippy -- -D warnings
```

CI (`.github/workflows/ci.yml`) runs on every push to `main`. It runs `cargo test`, `cargo fmt -- --check`, and `cargo clippy -- -D warnings`
for the default members, plus a `desktop` job (npm typecheck + build, then fmt/clippy/test for
`-p oxitailr-desktop`) and a `desktop-windows` job (`npx tauri build --bundles nsis` on
`windows-latest`, uploading the installer as the `oxitailr-desktop-windows-setup` artifact).
Match formatting and keep clippy clean before pushing.

## Architecture

One engine, two front ends. The engine never draws anything; each front end owns its event
loop and talks to the engine's I/O tasks **only over channels**.

- **`AppCore`** (`crates/oxitailr-core/src/app.rs`): the headless core. It holds the log
  buffer (`LogState`, a `VecDeque` ring capped at `general.buffer_size`, default 10,000), the
  filter (`FilterEngine`, the filter text, the per-level mask), the compiled highlight rules,
  the alert rules, the config, and `source_infos` (each source's status). `AppCore::new`
  returns it with a `CoreChannels` (the source-event and alert receivers), kept separate so a
  front end can `select!` on them without borrowing the core. `ingest(event)` parses a line,
  appends it, and runs the alert rules; `passes_filter(line)` is the view test both front ends
  use. Sources are added with `add_local_source` / `add_ssh_source` and removed by sending
  `SourceCommand::Remove` on `source_cmd_tx`.
- **Sources** (`source/`): the async `Source` trait; `start(sender)` spawns a Tokio task that
  sends `SourceEvent::{Line, Lines, StatusChange, Error}`. `SourceManager` owns them.
  - `LocalFileSource` (`local.rs`): reads the existing content, then tails with a `notify`
    watcher **plus a polling fallback** (`POLL_INTERVAL_MS`), so a missed event can't stall
    it; detects rotation and truncation (inode + size).
  - `SshSource` (`ssh.rs`): tails a remote file over `russh`; key auth, or a password given
    directly or found in the secret store.
- **Parsers** (`parser/`): `JsonParser` for brace-wrapped lines (when `auto_parse_json`),
  else `PlainParser` (timestamp patterns, ISO 8601 / `YYYY-MM-DD HH:MM:SS[.,fff]` / syslog,
  and the level word). Output is a `LogEntry` (`models/`).
- **Alerts** (`alert/`): `AlertDispatcher` matches rules and sends `AlertEvent`s on its own
  channel; actions in `desktop.rs` / `sound.rs` / `visual.rs` / `webhook.rs`.
- **Secrets** (`secrets.rs`): SSH passwords, recoverable (they're sent to the server), never
  in the config. The OS vault through `keyring-core` (Windows Credential Manager, macOS
  Keychain, the Secret Service over pure-Rust zbus), with the ids (never values) in
  `secrets-index.json`; or, with no vault, an AES-256-GCM file (`secrets.vault`) whose key is
  in the vault or in `vault.key`. Values travel as `SecretString`, never in a log or error.
  `OXITAILR_SECRET_BACKEND=file` forces the file backend (tests, services). It replaced the
  old `credentials.rs`; `credentials.enc` is migrated on first use.
- **Config & state** (`config/`, `state/`): TOML at `<config_dir>/oxitailr/config.toml`;
  the terminal app's session (open files, bookmarks) in `session.json`, the desktop app's in
  `desktop-session.json`.

**Terminal app** (`crates/oxitailr/src/`): `main.rs` parses the CLI (`clap`: FILEs with glob
expansion, `-c` config, `-m` buffer size), builds the Tokio runtime and the core, and calls
`tui::run`. `tui.rs` is one event-driven `tokio::select!` loop over crossterm input, the
source events and the alerts; it redraws only when something changed.

**Desktop app** (`desktop/`): `src-tauri/src/main.rs` keeps one `AppCore` behind a mutex and
exposes Tauri commands. A `pump` task drains the channels into the core and emits `changed`
at most every 100 ms (500 ms when another app is in front, nothing while hidden). The page
then asks for **only the rows on screen** (`rows(start, count)`) from `view.rs`, which keeps
the passing line numbers and extends them incrementally. `tray.rs` is the tray icon and menu
and the window behaviour (close to tray, start minimized, launch at login); `ssh.rs` and
`secrets_cmd.rs` are the SSH form and the saved-password commands. The front end is in
`desktop/src` (`api.ts` is its only `invoke` caller; `pages/LogsPage.tsx` is the log view).

### Threading rule
Source and alert tasks run on the Tokio runtime. A front end's loop must never block on
them: talk through the channels, and in the desktop app never hold the core's lock across an
`await` or for a large batch (`EVENTS_PER_LOCK`).

## Known gaps

- No in-app editors for filter presets, highlight rules or alerts (the config file only).
- The terminal app's SSH form has no password field (key auth only).
- The desktop app is a preview: not yet tried end to end on Windows (see
  `docs/HANDOFF-desktop-windows.md`), no Linux package, no self-update.

## Release Process

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed. Don't build or
upload release files by hand. The workflow runs the gates, builds all four files and publishes the
GitHub release with generated notes. The names must stay exactly:

1. **Linux binary**: `oxitailr-{VERSION}-x86_64`
2. **Windows binary**: `oxitailr-{VERSION}-x86_64.exe` (MinGW cross-compile on Ubuntu)
3. **AppImage**: `Oxitailr-{VERSION}-x86_64.AppImage`
4. **Desktop app, Windows installer**: `Oxitailr-Desktop-{VERSION}-x86_64-setup.exe` (NSIS, built on `windows-latest`)

**IMPORTANT**:
- Always include the VERSION in all file names
- Always include the `-x86_64` suffix (we only build 64-bit)
- The desktop installer is NSIS only: MSI refuses pre-release versions

### Checklist

- [ ] Bump `version` in the root `Cargo.toml` (`[workspace.package]`; the desktop crate inherits it), then `cargo update -w`
- [ ] Bump `version` in `desktop/package.json` and `desktop/package-lock.json`
- [ ] Move `[Unreleased]` in `CHANGELOG.md` to the new version and update the compare links at the bottom
- [ ] Update the version in the `README.md` download instructions and the desktop app section
- [ ] Commit the bump to `main`, push, and wait for CI to go green
- [ ] Tag `main` and push the tag:

```bash
git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin vX.Y.Z
```

- [ ] Check the release has all four files: `gh release view vX.Y.Z`
