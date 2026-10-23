# Oxitailr

**ox·i·tail·r** | \ ˌäk-sē-ˈtā-lər \ | *OK-see-TAY-ler*

A fast, low-resource log viewer / live tailer built in Rust, in two forms on one engine:

- **The terminal app** (`oxitailr`): run it from a terminal with a file path
  (`oxitailr /var/log/syslog`). About 2% CPU and 10 MB of memory on a busy log.
- **The desktop app** (preview, since 0.4): the same engine in a window, with a
  system tray. See [Desktop app](#desktop-app-preview).

Both read the same config file. The engine is the `oxitailr-core` library
(`crates/oxitailr-core`), which other programs can use too. See [CHANGELOG.md](CHANGELOG.md)
for the history, including v0.3.0's move from the original egui GUI to the terminal UI.

![Rust](https://img.shields.io/badge/rust-1.82%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Real-time tailing** — local files and remote SSH sources, live. Local files are
  watched for changes and polled as a fallback, so tailing never stalls.
- **Multiple sources as tabs** — `Tab` / `Shift+Tab` to switch. Open a local file with `O` (or **drag one onto the terminal window**); add an SSH source with `o`.
- **JSON & ANSI** — auto-detects and parses JSON log lines; renders ANSI color codes.
- **Filtering & presets** — live regex/substring filter (`f`), per-level toggles (`1`–`6`, Trace…Fatal), and saved filter presets from the config (`p` to pick one).
- **Search & highlight rules** — live search highlight as you type (`/`), jump between matches (`n` / `N`), plus config-defined highlight rules that paint matching text in a custom color.
- **Cursor, bookmarks & copy** — move a selection cursor through the log; bookmark lines (`b`, jump with `]` / `[`) — bookmarks persist across runs; copy a line to the clipboard (`y`, via OSC 52, so it works over SSH).
- **Alerts** — rules defined in the config fire desktop / sound / webhook actions; an in-app indicator (`⚠ N`) and an `a` popup show recent hits.
- **Sessions** — reopens the files you had open last time (`remember_last_session`), and auto-opens sources marked `auto_open` in the config.
- **Log-rotation detection** — keeps tailing across `logrotate` / truncation.
- **Settings** (`S`) — toggle timestamp display and JSON auto-parsing.
- **Tiny footprint** — fully event-driven: ~0–2% CPU and ~10 MB RAM, even on busy logs.

The key list above is the terminal app's; the desktop app has a control for each (see
below). Filter presets, highlight rules and alerts are defined in the config file; there
are no in-app editors for them yet. SSH in the terminal app uses key authentication; the
desktop app can also use a password and save it in the OS password vault.

## Installation

Each [release](https://github.com/MarcelineVPQ/oxitailr/releases) ships three x86-64 builds of the terminal app — **launch them from a terminal with a file path** — plus a Windows installer for the [desktop app](#desktop-app-preview).

### Linux — AppImage
```bash
wget https://github.com/MarcelineVPQ/oxitailr/releases/download/v0.4.1/Oxitailr-0.4.1-x86_64.AppImage
chmod +x Oxitailr-0.4.1-x86_64.AppImage
./Oxitailr-0.4.1-x86_64.AppImage /var/log/syslog
```

### Linux — plain binary
Download `oxitailr-0.4.1-x86_64`, `chmod +x`, then `./oxitailr-0.4.1-x86_64 /var/log/syslog`.

### Windows
Download `oxitailr-0.4.1-x86_64.exe`. In **PowerShell / Windows Terminal**:
```powershell
.\oxitailr-0.4.1-x86_64.exe C:\path\to\your.log
```
(or drag a log file onto the `.exe`). A plain double-click won't work — it needs a console.

### From source
```bash
git clone https://github.com/MarcelineVPQ/oxitailr.git
cd oxitailr
cargo build --release
./target/release/oxitailr /var/log/syslog
```

### Build dependencies
None special for the terminal app — the terminal UI (crossterm/ratatui) and TLS (rustls) are
pure Rust. A Rust toolchain (1.82+) is all you need. The repository is a Cargo workspace:
`cargo build` at the root builds the engine and the terminal app; the desktop app is built
separately (below).

## Usage

It's a terminal application: run it in a terminal (it cannot be double-clicked).

```bash
oxitailr /var/log/syslog                  # tail a file
oxitailr '/var/log/*.log'                 # globs expand to one source per match
oxitailr -c custom.toml /var/log/app.log  # custom config file
oxitailr -m 50000 /var/log/large.log      # buffer size (max lines kept)
```

Inside the app: press **`?`** for the full key list, **`q`** to quit. It tails live by default — scroll up to read history, and **`G`** / **`End`** jumps back to the newest line.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `k`, `↓` / `↑` | Move cursor one line |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `PgDn` / `PgUp` | Page down / up |
| `g` | Jump to top (scroll back through history) |
| `G` / `End` / `Space` | Jump to the newest line (resume live tail) |
| `Tab` / `Shift+Tab` | Switch source |
| `1`–`6` | Toggle level visibility (Trace…Fatal) |
| `f` | Filter (regex / substring) |
| `p` | Pick a filter preset (from config) |
| `/`, then `n` / `N` | Search, next / previous match |
| `b` · `]` / `[` | Bookmark cursor line · jump to next / prev bookmark |
| `y` | Copy the cursor line to the clipboard |
| `O` | Open a local file (or drag one onto the window) |
| `o` | Add an SSH source |
| `S` | Settings (timestamps, JSON auto-parse) |
| `a` | Show recent alerts |
| `r` / `c` | Reload / clear |
| `?` | Help |
| `q` / `Esc` | Quit |

## Desktop app (preview)

The desktop app is the **same engine, in a window** (Tauri 2, React front end):

- A left navigation rail; one tab per source; level chips, filter, search, follow (Live),
  bookmarks, filter presets, reload and clear; drag-and-drop or an Open dialog.
- **SSH sources** from a form, with the password saved in the OS password vault (Windows
  Credential Manager, macOS Keychain, or the Secret Service on Linux), or an encrypted file
  where there's no vault. Saved passwords are listed in Settings: shown only on a click,
  and removable.
- **Alerts** from the config's `[[alerts]]` rules: a bell with the recent hits, a badge on
  the rail, and a desktop notification while the window isn't focused.
- **System tray:** a click shows or hides the window; Close to tray, Start minimized,
  Launch at login and alert notifications are switched in the tray menu. A second launch
  brings the open window forward.
- Light, dark or system theme.
- Lean: it only fetches the rows on screen, updates at most 10 times a second, and does
  nothing while hidden in the tray.
- The terminal app's keys work too: `/` search, `f` filter, `1`–`6` levels, `G` / `End`
  live, `n` / `N`, `b`, `]` / `[`, `y` copy, `o` open, and `Ctrl+B` keeps the rail open.

**Status:** preview. Windows: download `Oxitailr-Desktop-0.4.1-x86_64-setup.exe` from the
[release](https://github.com/MarcelineVPQ/oxitailr/releases/latest) and run it. Linux: build
it from source.

Prerequisites: a Rust toolchain and **Node 26**. On Linux, also the WebKitGTK build dependencies:
```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libssl-dev build-essential pkg-config
```

Then:
```bash
cd desktop
npm ci
npm run tauri dev      # run it
npm run tauri build    # release build (installers under target/release/bundle/)
```

## Configuration

Config lives at `~/.config/oxitailr/config.toml` (on Windows `%APPDATA%\oxitailr\config.toml`; see `config.example.toml` for the full schema). Both apps read it; the desktop app's Settings page shows where. It reads: `general.{buffer_size, show_timestamps, auto_parse_json, show_source, remember_last_session}`, the `[[sources]]` list (with `enabled` / `auto_open`), `[filters.<name>]` presets, `[[highlights]]` rules, and `[[alerts]]`. Old GUI-only keys — theme, fonts, line spacing, `wrap_lines` — are accepted but ignored.

```toml
[general]
buffer_size = 10000          # max lines kept in memory
show_timestamps = true
auto_parse_json = true       # parse brace-wrapped lines as JSON
remember_last_session = true # reopen last session's files when none are given

# A source opens on startup only when enabled AND auto_open.
[[sources]]
type = "local"
name = "app"
path = "/var/log/myapp/app.log"
enabled = true
auto_open = true

[[sources]]
type = "ssh"
name = "remote"
host = "server.example.com"
port = 22
user = "admin"
path = "/var/log/syslog"
enabled = true
auto_open = true

# Filter preset — pick it in-app with `p`
[filters.production]
include = ["ERROR", "FATAL"]   # OR logic; regex patterns
exclude = ["DEBUG", "TRACE"]

# Highlight rule — paints matching text in [R, G, B]
[[highlights]]
pattern = "ERROR"
color = [255, 64, 64]
# regex = true               # treat `pattern` as a regular expression

# Alert rule — fires its actions and shows in the in-app indicator
[[alerts]]
name = "errors"
pattern = "ERROR|CRITICAL|FATAL"
actions = [{ type = "visual" }, { type = "desktop", title = "Error!" }]
cooldown_seconds = 30
```

SSH key authentication tries your specified key, then `~/.ssh/id_ed25519` / `id_rsa` / `id_ecdsa`.
Passwords are never written to the config file.

## Data Files

Oxitailr stores data under `~/.config/oxitailr/` (on Windows `%APPDATA%\oxitailr\`):

| File | Used by | Purpose |
|------|---------|---------|
| `config.toml` | both | Configuration |
| `session.json` | terminal | Bookmarks + last-open files (restored on launch) |
| `desktop-session.json` | desktop | Last-open files (kept apart so the two apps never overwrite each other) |
| `desktop-settings.json` | desktop | Close to tray, Start minimized, alert notifications |
| `secrets-index.json` | desktop | The ids of saved passwords (never the passwords) |
| `secrets.vault`, `vault.key` | desktop | Only where there's no OS password vault: the encrypted passwords and their key |

## Building for Distribution

```bash
cargo build --release            # -> target/release/oxitailr
./appimage/build-appimage.sh     # optional AppImage
cd desktop && npm ci && npx tauri build --bundles nsis   # desktop installer (on Windows)
```

Releases are built by GitHub Actions when a `v*` tag is pushed (see CLAUDE.md, Release
Process): the Linux binary, the Windows `.exe`, the AppImage and the desktop installer.

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for the full version history. The latest release is **v0.4.1**.

## License

MIT License — see LICENSE file for details.

## Contributing

Contributions are welcome — issues and pull requests appreciated.

## Acknowledgments

Built with:
- [ratatui](https://github.com/ratatui/ratatui) + [crossterm](https://github.com/crossterm-rs/crossterm) — terminal UI
- [Tauri](https://tauri.app/) + [React](https://react.dev/) — desktop app
- [keyring-core](https://crates.io/crates/keyring-core) — OS password vaults
- [tokio](https://tokio.rs/) — async runtime
- [russh](https://github.com/warp-tech/russh) — SSH client
