# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Oxitailr is a GUI log viewer / live tailer built in Rust on `eframe`/`egui`, with local-file
and SSH log sources.

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

CI (`.github/workflows/ci.yml`) runs `cargo test`, `cargo fmt -- --check`, and
`cargo clippy -- -D warnings`. Match formatting and keep clippy clean before pushing.

## Architecture

Single-threaded egui UI + a multi-threaded Tokio runtime for I/O, communicating **only over
mpsc channels**. Understanding that boundary is the key to working here.

- **Entry / runtime** (`src/main.rs::main`, ~line 2655): parses CLI args (`clap`), loads the
  TOML config, builds a multi-threaded Tokio `Runtime` wrapped in `Arc`, constructs
  `TailLoggerApp`, and hands off to `eframe::run_native`.

- **`TailLoggerApp`** (`src/main.rs`, ~2,700 lines): the central app object. It holds nearly
  all state *and* all UI rendering, and implements `eframe::App::update` (~line 1473), which is
  the per-frame loop: drain events → handle dialogs/input → render panels →
  `request_repaint_after(update_interval_ms)`. This file is intentionally being broken up; see
  "Known issues" below.

- **Sources** (`src/source/`): the `Source` trait (`mod.rs`) is `async` with
  `start(sender)`/`stop()`. `start()` spawns a Tokio task that emits `SourceEvent::{Line,
  StatusChange, Error}` into a shared `mpsc::Sender`. `SourceManager` owns the
  `Box<dyn Source>` set and hands the UI a single `mpsc::Receiver` via `take_event_receiver()`.
  - `LocalFileSource` (`local.rs`): reads existing content, then tails via a `notify`
    filesystem watcher; includes log-rotation/truncation detection (inode + size).
  - `SshSource` (`ssh.rs`): tails a remote file over `russh`.

- **Ingestion** (`src/main.rs::process_events`, ~line 971): runs each frame, drains the source
  receiver, parses each line, appends to `LogState`, and dispatches alert checks. `LogState`
  (`src/main.rs:140`) is a `VecDeque` ring buffer capped at `buffer_size` (default 10,000).

- **Parsers** (`src/parser/`): `Parser` trait → `JsonParser` / `PlainParser`.
  `auto_detect_parser()` picks JSON when a line is brace-wrapped, else plain. Output is a
  `LogEntry` (`src/models/`, with `LogLevel`).

- **Filter & highlight** (`src/filter/`): `FilterEngine` + `FilterRule` (regex / level)
  decide which entries render; highlight rules colorize matches.

- **Alerts** (`src/alert/`): `AlertDispatcher` matches rules against entries asynchronously and
  emits `AlertEvent`s on its own mpsc channel (drained by the UI for the visual indicator).
  Actions live in `desktop.rs` / `sound.rs` / `visual.rs` / `webhook.rs`.

- **Config & persistence**: TOML config at `<config_dir>/oxitailr/config.toml`
  (`src/config/`); session state — open files, window geometry, bookmarks — at
  `<config_dir>/oxitailr/session.json` (`src/state/session.rs`). SSH passwords are encrypted
  via `src/credentials.rs` (argon2 + aes-gcm), never stored in plaintext config.

### Threading rule
Source/alert tasks run on the Tokio runtime; the UI runs on the main thread. They must talk
through channels. Do **not** add blocking work to the egui frame loop.

## Known issues / in-progress work

See `RESUME_NOTES.md` for an active diagnosis + plan. In short:
- `LocalFileSource` relies solely on `notify` with no polling fallback, so live tailing can
  stall until "Reload" forces a full re-read.
- Wrap-mode rendering isn't virtualized and the filtered list is re-cloned every frame.
- Some UI actions (`reload_sources`, add/remove source) currently call `runtime.block_on(...)`
  on the UI thread — this violates the threading rule above and freezes the window; it is slated
  to move behind a command channel.

## Release Process

When creating a GitHub release, **ALWAYS upload ALL THREE binaries** with consistent naming:

1. **Linux binary**: `oxitailr-{VERSION}-x86_64`
2. **Windows binary**: `oxitailr-{VERSION}-x86_64.exe`
3. **AppImage**: `Oxitailr-{VERSION}-x86_64.AppImage`

**IMPORTANT**:
- Always include the VERSION in all binary names
- Always include `-x86_64` suffix on all binaries (we only build 64-bit)

### Build Commands

```bash
# Build all three binaries
cargo build --release
cargo build --release --target x86_64-pc-windows-gnu
./appimage/build-appimage.sh
```

### Release Command

```bash
# Set version variable
VERSION="0.2.13"  # Update this!

# Copy binaries with correct names
cp target/release/oxitailr /tmp/oxitailr-${VERSION}-x86_64
cp target/x86_64-pc-windows-gnu/release/oxitailr.exe /tmp/oxitailr-${VERSION}-x86_64.exe

# Create release with all three binaries
gh release create v${VERSION} \
  --title "v${VERSION}" \
  --notes "Release notes here" \
  /tmp/oxitailr-${VERSION}-x86_64 \
  /tmp/oxitailr-${VERSION}-x86_64.exe \
  release/Oxitailr-${VERSION}-x86_64.AppImage
```

### Checklist Before Release

- [ ] Update version in `Cargo.toml`
- [ ] Update `CHANGELOG.md`
- [ ] Update version in `README.md` download instructions
- [ ] Build all three binaries (Linux, Windows, AppImage)
- [ ] Commit and push changes
- [ ] Create GitHub release with **ALL THREE** binaries:
  - [ ] `oxitailr-{VERSION}-x86_64` (Linux)
  - [ ] `oxitailr-{VERSION}-x86_64.exe` (Windows)
  - [ ] `Oxitailr-{VERSION}-x86_64.AppImage`
