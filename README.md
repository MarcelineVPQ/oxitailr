# Oxitailr

**ox·i·tail·r** | \ ˌäk-sē-ˈtā-lər \ | *OK-see-TAY-ler*

A fast, low-resource log viewer / live tailer built in Rust, for Linux and Windows.

![Rust](https://img.shields.io/badge/rust-1.82%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Real-time tailing** of local files and remote SSH sources. Local files are watched for
  changes and polled as a fallback, so tailing never stalls; log rotation and truncation
  are followed.
- **One tab per source**, or all of them together. Open files with the Open dialog or by
  dropping them on the window; add an SSH source from its form.
- **SSH passwords** saved in the OS password vault (Windows Credential Manager, macOS
  Keychain, or the Secret Service on Linux), or an encrypted file where there's no vault.
  Saved passwords are listed in Settings, shown only on a click, and removable. Key
  authentication works too.
- **JSON and ANSI:** JSON log lines are parsed; ANSI color codes are stripped.
- **Filtering and presets:** a live regex or text filter, a switch per level (Trace…Fatal),
  and filter presets from the config.
- **Search and highlight rules:** matches are highlighted as you type and you jump between
  them; highlight rules from the config paint matching text in a color.
- **Bookmarks and copy:** bookmark lines and jump between them; copy a line.
- **Alerts** from the config's rules: a bell with the recent hits, a badge on the rail, a
  desktop notification while the window isn't focused, and sound or webhook actions.
- **System tray:** a click shows or hides the window. Close to tray, Start minimized, Launch
  at login and alert notifications are switched in the tray menu. A second launch brings
  the open window forward.
- **Light, dark or system theme.**
- **Lean:** it only fetches the lines on screen, updates at most 10 times a second, and
  does nothing while hidden in the tray.

Filter presets, highlight rules and alerts are defined in the config file; there are no
in-app editors for them yet.

## Installation

Each [release](https://github.com/MarcelineVPQ/oxitailr/releases/latest) ships two x86-64 files.

### Linux — AppImage
```bash
chmod +x Oxitailr-<version>-x86_64.AppImage
./Oxitailr-<version>-x86_64.AppImage                  # or with files: ... /var/log/syslog
```

### Windows — installer
Run `Oxitailr-<version>-x86_64-setup.exe`. Oxitailr is then in the Start menu.

### From source
Prerequisites: a Rust toolchain (1.82+) and **Node 26**. On Linux, also the WebKitGTK build
dependencies:
```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libssl-dev build-essential pkg-config file
```

Then:
```bash
git clone https://github.com/MarcelineVPQ/oxitailr.git
cd oxitailr/desktop
npm ci
npm run tauri dev                          # run it
npx tauri build --bundles appimage         # Linux AppImage (under target/release/bundle/)
npx tauri build --bundles nsis             # Windows installer (on Windows)
```

## Usage

Open log files with **Open**, by dropping them on the window, or by naming them on the
command line (`oxitailr-desktop /var/log/syslog`). It follows the newest line live; scroll up
to read back, and **Live** (or `G` / `End`) returns to the newest line.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `k`, `↓` / `↑` | Move the cursor one line |
| `PgDn` / `PgUp` | Page down / up |
| `g` / `Home` | Jump to the top |
| `G` / `End` / `Space` | Jump to the newest line (follow live) |
| `1`–`6` | Show or hide a level (Trace…Fatal) |
| `f` | Filter (regex or text) |
| `/`, then `Enter` or `n` / `N` | Search, next / previous match |
| `b` · `]` / `[` | Bookmark the cursor line · next / previous bookmark |
| `y` | Copy the cursor line |
| `o` | Open a file |
| `Ctrl+B` | Keep the navigation rail open / collapse it |

## Configuration

Config lives at `~/.config/oxitailr/config.toml` (on Windows `%APPDATA%\oxitailr\config.toml`; see `config.example.toml` for the full schema). App settings shows where it is and what was read from it; edits apply the next time Oxitailr starts. It reads: `general.{buffer_size, show_timestamps, auto_parse_json, remember_last_session}`, the `[[sources]]` list (with `enabled` / `auto_open`), `[filters.<name>]` presets, `[[highlights]]` rules, and `[[alerts]]`. Keys from older builds — `show_source`, theme, fonts, line spacing, `wrap_lines` — are accepted but ignored.

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

# Filter preset — pick it from the preset menu on the Logs page
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

| File | Purpose |
|------|---------|
| `config.toml` | Configuration |
| `desktop-session.json` | Last-open files (reopened on launch) |
| `desktop-settings.json` | Close to tray, Start minimized, alert notifications |
| `secrets-index.json` | The ids of saved passwords (never the passwords) |
| `secrets.vault`, `vault.key` | Only where there's no OS password vault: the encrypted passwords and their key |

## Repository layout

- `crates/oxitailr-core/` — the engine, a library: sources, parsers, filters, alerts, config,
  sessions and the password store. Other programs can use it too.
- `desktop/` — the app (Tauri 2, React front end) on that engine.

Releases are built by GitHub Actions when a `v*` tag is pushed (see CLAUDE.md, Release
Process). v0.3.0 replaced the original egui GUI with a terminal app; v0.4 added this app,
which has since replaced the terminal app. See [CHANGELOG.md](CHANGELOG.md).

## License

MIT License — see LICENSE file for details.

## Acknowledgments

Built with:
- [Tauri](https://tauri.app/) + [React](https://react.dev/) — the app
- [tokio](https://tokio.rs/) — async runtime
- [russh](https://github.com/warp-tech/russh) — SSH client
- [keyring-core](https://crates.io/crates/keyring-core) — OS password vaults
