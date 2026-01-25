# Oxitailr

**ox·i·tail·r** | \ ˌäk-sē-ˈtā-lər \ | *OK-see-TAY-ler*

A modern, feature-rich log viewer with GUI interface built in Rust.

![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)
![License](https://img.shields.io/badge/license-MIT-blue)

## Features

### Core Features
- **Real-time log tailing** - Watch log files update live as new entries are added
- **Multi-source support** - View logs from local files and remote SSH servers simultaneously
- **JSON log parsing** - Automatic detection and parsing of JSON-formatted logs
- **ANSI color support** - Full support for colored terminal output in logs

### Filtering & Search
- **Regex filtering** - Filter log entries with regular expressions
- **Log level filtering** - Toggle visibility by level (Trace, Debug, Info, Warn, Error, Fatal)
- **Filter presets** - Save and quickly apply common filter configurations
- **Live search highlighting** - Highlight matching text as you type
- **Search navigation** - Jump between matches with F3/Shift+F3 or navigation buttons
- **Glob patterns** - Open multiple files with wildcards (e.g., `/var/log/*.log`)

### Advanced Features
- **Log rotation detection** - Automatically detects when log files are rotated and continues tailing
- **Session persistence** - Remember open files and restore them on restart
- **Auto-open sources** - Mark sources to automatically open on startup
- **Secure password storage** - SSH passwords stored in OS keychain (not in config files)
- **Customizable highlighting** - Create rules to highlight specific patterns with custom colors
- **Desktop notifications** - Get notified when specific patterns appear in logs

### User Interface
- **Tabbed source view** - Switch between multiple open sources
- **File size display** - Status bar shows current file size
- **Source panel** - Manage local files and SSH connections
- **Theme support** - Light, Dark, or System theme (follows OS preference)
- **Configurable display** - Adjust font size, line spacing, timestamps, and more
- **Log bookmarks** - Mark important lines and jump back to them later (persisted across sessions)
- **Context menu** - Right-click to copy log lines (with/without timestamp, or raw)
- **Vim keybindings** - Optional vim-style navigation (j/k, G/gg, Ctrl+d/u, n/N)

## Installation

### AppImage (Recommended for Linux)

Download the latest AppImage from the [Releases](https://github.com/MarcelineVPQ/oxitailr/releases) page:

```bash
# Download (replace version as needed)
wget https://github.com/MarcelineVPQ/oxitailr/releases/download/v0.2.13/Oxitailr-0.2.13-x86_64.AppImage

# Make executable
chmod +x Oxitailr-0.2.13-x86_64.AppImage

# Run
./Oxitailr-0.2.13-x86_64.AppImage
```

No dependencies required - works on most Linux distributions.

### From Source

```bash
# Clone the repository
git clone https://github.com/MarcelineVPQ/oxitailr.git
cd oxitailr

# Build release binary
cargo build --release

# The binary will be at target/release/oxitailr

# Optional: Build AppImage
./appimage/build-appimage.sh
```

### Build Dependencies

On Linux, you may need to install some system dependencies to build from source:

```bash
# Ubuntu/Debian
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libsecret-1-dev

# Fedora
sudo dnf install libxcb-devel libxkbcommon-devel openssl-devel libsecret-devel
```

## Usage

### Command Line

```bash
# Open a specific log file
oxitailr /var/log/syslog

# Open with a custom config
oxitailr --config ~/.config/oxitailr/config.toml /var/log/app.log

# Override buffer size
oxitailr --max-lines 50000 /var/log/large.log
```

### GUI

Launch without arguments to use the graphical interface:

```bash
oxitailr
```

From the GUI you can:
- **Open local files** - Click "+ Local File" or use the menu
- **Add SSH sources** - Click "+ SSH" to connect to remote servers
- **Manage sources** - Use the source panel to connect, edit, or remove sources
- **Configure settings** - Access via the menu (gear icon)

## Configuration

Configuration is stored at `~/.config/oxitailr/config.toml`. See `config.example.toml` for all options.

### Example Configuration

```toml
[general]
buffer_size = 10000
follow = true
wrap_lines = false
show_timestamps = true
show_source = true
remember_last_session = true

# Local file source
[[sources]]
type = "local"
name = "app-logs"
path = "/var/log/myapp/app.log"
enabled = true
auto_open = true

# SSH remote source
[[sources]]
type = "ssh"
name = "remote-server"
host = "server.example.com"
port = 22
user = "admin"
path = "/var/log/syslog"
enabled = true

# Filter presets
[filters.errors]
include = ["ERROR", "CRITICAL", "FATAL"]
exclude = ["healthcheck"]
```

## Data Files

Oxitailr stores its data in `~/.config/oxitailr/`:

| File | Purpose |
|------|---------|
| `config.toml` | Main configuration |
| `ssh_sources.json` | Saved SSH connection details |
| `local_sources.json` | Saved local file sources |
| `session.json` | Last session state (for restore) |

**Note:** SSH passwords are stored encrypted locally using Argon2 key derivation and AES-256-GCM encryption. Credential files are set to mode 0600 on Unix systems.

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Ctrl+F` | Focus filter field |
| `Ctrl+G` | Focus search field |
| `Ctrl+L` | Clear log view |
| `Ctrl+O` | Open local file |
| `F3` | Jump to next search match |
| `Shift+F3` | Jump to previous search match |
| `Page Up/Down` | Scroll by page |
| `Home` | Jump to beginning |
| `End` | Jump to end (enables auto-scroll) |
| `Mouse wheel` | Scroll through logs |

### Vim Mode (enable in Settings)

| Key | Action |
|-----|--------|
| `j` / `k` | Scroll down/up one line |
| `G` | Jump to end (enable auto-scroll) |
| `gg` | Jump to beginning |
| `Ctrl+d` / `Ctrl+u` | Half-page down/up |
| `Ctrl+f` / `Ctrl+b` | Full-page down/up |
| `/` | Focus search field |
| `n` / `N` | Next/previous search match |

**Tip:** Access the in-app Help dialog from the hamburger menu (☰) for more details.

## Building for Distribution

```bash
# Build optimized release
cargo build --release

# Binary location
ls -la target/release/oxitailr
```

## Changelog

### v0.2.13
- Fix bookmark navigation scrolling to wrong position

### v0.2.12
- **Search Navigation** - Jump between search matches with F3/Shift+F3 or ▲/▼ buttons
- **Copy/Export Log Lines** - Right-click context menu with Copy Line, Copy with Timestamp, Copy Raw
- **Log Bookmarks** - Click ☆ to bookmark lines, jump back via dropdown, persists across sessions
- **Glob Pattern Support** - Open multiple files with `oxitailr /var/log/*.log`
- **Vim Keybindings** - Optional vim-style navigation (enable in Settings)

### v0.2.11
- Alert System with dialog, desktop notifications, webhooks, and visual indicator

### v0.2.10
- Fix End key and initial scroll to bottom
- Fix incorrect AppImage install instructions

### v0.2.9
- Fix Page Up/Down scrolling

### v0.2.8
- Add multiple file selection support

## License

MIT License - see LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Acknowledgments

Built with:
- [egui](https://github.com/emilk/egui) - Immediate mode GUI
- [tokio](https://tokio.rs/) - Async runtime
- [russh](https://github.com/warp-tech/russh) - SSH client
- [notify](https://github.com/notify-rs/notify) - File system notifications
