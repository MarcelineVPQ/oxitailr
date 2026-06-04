# Oxitailr

**ox·i·tail·r** | \ ˌäk-sē-ˈtā-lər \ | *OK-see-TAY-ler*

A fast, low-resource **terminal** log viewer / live tailer built in Rust.

> **It's a terminal program — run it from a terminal** with a file path
> (`oxitailr /var/log/syslog`). It can't be double-clicked like a GUI app.
> v0.3.0 replaced the original egui GUI with a Ratatui terminal UI, dropping
> idle CPU/memory from ~50% / ~500 MB to ~2% / ~10 MB. See [CHANGELOG.md](CHANGELOG.md).

![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

- **Real-time tailing** — local files and remote SSH sources, live.
- **Multiple sources as tabs** — `Tab` / `Shift+Tab` to switch. Open a local file with `O` (or **drag one onto the terminal window**); add an SSH source with `o`.
- **JSON & ANSI** — auto-detects and parses JSON log lines; renders ANSI color codes.
- **Filtering** — live regex/substring filter (`f`) and per-level toggles (`1`–`6`, Trace…Fatal).
- **Search** — live highlight as you type (`/`), jump between matches (`n` / `N`).
- **Cursor, bookmarks & copy** — move a selection cursor through the log; bookmark lines (`b`, jump with `]` / `[`) — bookmarks persist across runs; copy a line to the clipboard (`y`, via OSC 52, so it works over SSH).
- **Alerts** — rules defined in the config fire desktop / sound / webhook actions; an in-app indicator (`⚠ N`) and an `a` popup show recent hits.
- **Log-rotation detection** — keeps tailing across `logrotate` / truncation.
- **Settings** (`S`) — toggle timestamp display and JSON auto-parsing.
- **Tiny footprint** — fully event-driven: ~0–2% CPU and ~10 MB RAM, even on busy logs.

Not yet ported from the old GUI (tracked in the changelog): a custom highlight-rule editor, filter-preset selection, and restoring previously-open files on launch (bookmarks *do* persist).

## Installation

Each [release](https://github.com/MarcelineVPQ/oxitailr/releases) ships three x86-64 builds. **All are terminal programs — launch them from a terminal with a file path.**

### Linux — AppImage
```bash
wget https://github.com/MarcelineVPQ/oxitailr/releases/download/v0.3.1/Oxitailr-0.3.1-x86_64.AppImage
chmod +x Oxitailr-0.3.1-x86_64.AppImage
./Oxitailr-0.3.1-x86_64.AppImage /var/log/syslog
```

### Linux — plain binary
Download `oxitailr-0.3.1-x86_64`, `chmod +x`, then `./oxitailr-0.3.1-x86_64 /var/log/syslog`.

### Windows
Download `oxitailr-0.3.1-x86_64.exe`. In **PowerShell / Windows Terminal**:
```powershell
.\oxitailr-0.3.1-x86_64.exe C:\path\to\your.log
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
None special — the terminal UI (crossterm/ratatui) and TLS (rustls) are pure Rust. A Rust toolchain (1.70+) is all you need.

## Usage

It's a terminal application: run it in a terminal (it cannot be double-clicked).

```bash
oxitailr /var/log/syslog                  # tail a file
oxitailr '/var/log/*.log'                 # globs expand to one source per match
oxitailr -c custom.toml /var/log/app.log  # custom config file
oxitailr -m 50000 /var/log/large.log      # buffer size (max lines kept)
```

Inside the app: press **`?`** for the full key list, **`q`** to quit, **`Space`** / **`G`** to follow new lines.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `j` / `k`, `↓` / `↑` | Move cursor one line |
| `Ctrl+d` / `Ctrl+u` | Half page down / up |
| `PgDn` / `PgUp` | Page down / up |
| `g` / `G` | Jump to top / bottom (`G` resumes follow) |
| `Space` | Toggle follow (auto-scroll) |
| `Tab` / `Shift+Tab` | Switch source |
| `1`–`6` | Toggle level visibility (Trace…Fatal) |
| `f` | Filter (regex / substring) |
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

## Configuration

Config lives at `~/.config/oxitailr/config.toml` (see `config.example.toml` for the full schema). The TUI currently reads: `general.buffer_size`, `general.show_timestamps`, `general.auto_parse_json`, the `[[sources]]` list, and `[[alerts]]`. Other keys from earlier (GUI-era) versions — theme, fonts, line spacing, `wrap_lines`, `auto_open`, filter presets — are ignored.

```toml
[general]
buffer_size = 10000      # max lines kept in memory
show_timestamps = true
auto_parse_json = true   # parse brace-wrapped lines as JSON

# Sources opened on startup
[[sources]]
type = "local"
name = "app"
path = "/var/log/myapp/app.log"
enabled = true

[[sources]]
type = "ssh"
name = "remote"
host = "server.example.com"
port = 22
user = "admin"
path = "/var/log/syslog"
enabled = true

# Alert rule — fires its actions and shows in the in-app indicator
[[alerts]]
name = "errors"
pattern = "ERROR|CRITICAL|FATAL"
actions = [{ type = "visual" }, { type = "desktop", title = "Error!" }]
cooldown_seconds = 30
```

SSH key authentication tries your specified key, then `~/.ssh/id_ed25519` / `id_rsa` / `id_ecdsa`.

## Data Files

Oxitailr stores data under `~/.config/oxitailr/`:

| File | Purpose |
|------|---------|
| `config.toml` | Configuration |
| `session.json` | Persisted bookmarks (restored on launch) |

## Building for Distribution

```bash
cargo build --release            # -> target/release/oxitailr
./appimage/build-appimage.sh     # optional AppImage
```

## Changelog

See [CHANGELOG.md](CHANGELOG.md) for the full version history. The latest release is **v0.3.1**.

## License

MIT License — see LICENSE file for details.

## Contributing

Contributions are welcome — issues and pull requests appreciated.

## Acknowledgments

Built with:
- [ratatui](https://github.com/ratatui/ratatui) + [crossterm](https://github.com/crossterm-rs/crossterm) — terminal UI
- [tokio](https://tokio.rs/) — async runtime
- [russh](https://github.com/warp-tech/russh) — SSH client
