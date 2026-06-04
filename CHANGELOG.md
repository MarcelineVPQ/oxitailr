# Changelog

All notable changes to Oxitailr will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_Nothing yet._

## [0.3.0] - 2026-06-03

### Changed
- **Rewrote the frontend as a terminal UI (TUI).** Replaced the egui/eframe GUI — which had a high CPU/memory floor (≈50% CPU and ≈500 MB even idle on a focused window) — with an event-driven Ratatui terminal interface. The entire async backend (file/SSH tailing, parsers, filters, alerts) is unchanged. Measured: **idle ≈1.6% CPU / ≈10 MB**, and a 5000 lines/sec flood ≈3.5% CPU / ≈16 MB.
- **File tailing** now uses bounded polling with batched line events instead of a filesystem-notify watcher, which fired thousands of times a second on busy logs and stormed the CPU.

### Phase 1 (this release) includes
Multi-source tabs, follow/scroll (`j`/`k`, `Ctrl+d`/`u`, `PgUp`/`Dn`, `g`/`G`, Space), per-level toggles (`1`–`6`), live filter (`f`), search with highlight (`/`, `n`/`N`), ANSI colors, status bar, `?` help, and CLI/glob file arguments.

### Not yet ported (backend is in place; coming in follow-up releases)
SSH sources, bookmarks, the alerts UI, the settings view, and custom highlight rules.

### Notes
- Oxitailr is now a terminal application — run it from a terminal: `oxitailr /var/log/syslog`.

## [0.2.20] - 2026-06-03

### Changed
- **Disabled the accessibility (AccessKit) integration** in the GUI. eframe builds an accessibility tree every frame and talks to the OS accessibility service (UI Automation on Windows, AT-SPI/D-Bus on Linux), a known cause of high CPU on a focused window. This is an experimental CPU-reduction change; the screen-reader integration is removed.

## [0.2.19] - 2026-06-03

### Changed
- **Lower CPU on busy logs** - The filtered view is now stored as reference-counted lines (`Arc`), so the per-frame rebuild during active tailing is a refcount bump instead of deep-copying the whole buffer. Benchmarked at ~4.7 ms/frame → ~0.1 ms/frame for a 10k-line buffer (≈28% of a core saved at 60fps). Also removed a per-line parser allocation, stopped spawning a task (and cloning the entry) per line when no alert rules are set, and memoized compiled filter/alert regexes.
- **Lower memory + ingest cost** - ANSI color spans are now parsed lazily (only for the visible lines that contain escape codes) instead of eagerly for every ingested line, removing a per-line allocation and shrinking the in-memory footprint of the line buffer.

## [0.2.18] - 2026-06-03

### Changed
- **Much lower CPU usage** - The window no longer repaints continuously (it now follows activity and goes idle when nothing is happening), and the system-theme check — which queried the OS on *every* frame (~8 ms each) — is cached and refreshed at most once every few seconds. This cuts idle CPU to near zero and dramatically reduces active CPU, most noticeably on Windows.

## [0.2.17] - 2026-06-03

### Fixed
- **Live tailing** - New log lines now appear automatically (within ~250ms) instead of requiring a manual Reload. Added a polling fallback so appends are picked up even when filesystem watch events are missed or coalesced.
- **Split lines** - A log line written in two flushes is no longer shown as two separate entries.

### Changed
- **Performance** - The view no longer re-filters and copies the entire buffer every frame, and wrapped-line mode is now virtualized, so large buffers stay responsive.
- **Responsiveness** - Opening, removing, and reloading sources no longer briefly freezes the window (source operations moved off the UI thread).
- **Build** - Switched TLS to rustls, removing the system OpenSSL/pkg-config build dependency.

### Internal
- Extracted the log view and source panel out of `main.rs` into `src/ui/panels/`.
- Added a tagged-release CI workflow that builds the Linux, Windows, and AppImage binaries.

## [0.2.16] - 2026-01-28

### Fixed
- **Auto-scroll shaking** - Fixed UI rapidly shaking when auto-scroll is enabled and viewing the last line
- **Unicode rendering** - Fixed arrow buttons and symbols showing as boxes by using ASCII equivalents

## [0.2.15] - 2026-01-26

### Added
- **Continuous integration** - GitHub Actions pipeline running tests, `cargo fmt`, `clippy`, and a code-duplication (jscpd) check, plus integration tests.
- Small UX improvements and an updated in-app help dialog.

### Changed
- Bookmark dropdown now expands to fit more entries.
- Webhook alert client is created lazily (only when first used).

### Fixed
- Right-click context-menu handling.
- Removed excessive debug logging.

## [0.2.14] - 2026-01-26

### Fixed
- **Per-source bookmarks** - Fixed bookmarks from one tab appearing in another tab's dropdown
  - Sort source names for consistent tab ordering (HashMap iteration was non-deterministic)
  - Validate selected source exists before bookmark lookup
  - Prevents bookmark operations on closed/invalid sources

## [0.2.13] - 2026-01-25

### Fixed
- **Bookmark navigation** - Fixed bookmark jumps scrolling to wrong position (clicking bookmark for line 3112 would scroll to ~1950)
  - Now uses egui's native `scroll_to_me()` API instead of manual pixel calculations
  - Bookmark target now correctly appears at top of viewport
  - Works correctly in both wrap_lines and non-wrap modes

## [0.2.12] - 2026-01-25

### Added
- **Search navigation** - Jump between search matches with F3 / Shift+F3 or the ▲/▼ buttons.
- **Copy / export** - Right-click context menu: Copy Line, Copy with Timestamp, Copy Raw.
- **Log bookmarks** - Click ☆ to bookmark a line and jump back via the dropdown; persists across sessions.
- **Glob patterns** - Open multiple files at once, e.g. `oxitailr /var/log/*.log`.
- **Vim keybindings** - Optional vim-style navigation (enable in Settings).

## [0.2.11] - 2026-01-25

### Added
- **Alert system** - Rule dialog with desktop notifications, webhooks, and a visual indicator when matching lines arrive.

## [0.2.10] - 2026-01-25

### Fixed
- **Page Up/Down/Home/End scrolling** - Complete fix for all scroll navigation keys
  - Uses pixel-based scrolling instead of row-based calculations
  - Works correctly with wrap_lines mode where lines have variable heights
  - End key now properly scrolls to actual bottom
  - Initial scroll to bottom now works correctly with wrapped lines

## [0.2.9] - 2026-01-25

### Fixed
- **Page Up/Down scrolling** - Partial fix (still had issues with End key and initial scroll)

## [0.2.8] - 2026-01-25

### Fixed
- **Multiple file selection** - File dialog now allows selecting multiple log files at once (Ctrl+click or Shift+click)
- **Multiple CLI files** - Command line now accepts multiple file arguments: `oxitailr file1.log file2.log file3.log`

## [0.2.7] - 2026-01-25

### Fixed
- **Window size persistence on Wayland** - Use screen_rect fallback when outer_rect is unavailable (Wayland doesn't expose window position to apps, but size now works)

## [0.2.6] - 2026-01-25

### Fixed
- **Window state persistence** - Fixed window size/position not being restored (now uses manual persistence via session.json instead of eframe's built-in which wasn't working)

## [0.2.5] - 2026-01-25

### Added
- **Window state persistence** - Remember window size and position between sessions (broken - fixed in 0.2.6)

## [0.2.4] - 2026-01-25

### Fixed
- **Windows terminal window** - Added `windows_subsystem` attribute to prevent console window from appearing alongside GUI
- **Windows tab refresh bug** - Fixed issue where opening a second log file caused existing tabs to go blank and continuously refresh
  - Fixed Windows pseudo-inode calculation that incorrectly used file size, causing false rotation detection
  - File watcher now filters events to only the specific file being watched, preventing cross-file interference

## [0.2.3] - 2026-01-23

### Added
- **File size display** - Status bar now shows file size for current source (bytes received for SSH)

### Changed
- **Removed "All" tab** - Simplified interface, sources now displayed individually without combined view
- **Improved scroll behavior** - Home/End keys and auto-scroll to bottom work correctly

### Fixed
- **Initial scroll position** - Files now correctly scroll to bottom on load
- **Settings persistence** - All GUI settings properly saved to config file

## [0.2.2] - 2026-01-23

### Fixed
- **Settings persistence** - All GUI settings now properly saved to config file (theme, font size, line spacing, tab width, update interval, auto-parse JSON)

## [0.2.1] - 2026-01-22

### Added
- **Theme support** - Light, Dark, or System theme (follows OS preference via `dark-light` crate)
- **Help dialog** - Comprehensive in-app help with keyboard shortcuts, filtering guide, SSH authentication details, and settings explanations
- **GitHub link** - Added repository link in About dialog
- **SSH host key verification** - Verify SSH server keys against `~/.ssh/known_hosts`

### Changed
- **Improved encryption** - Replaced weak XOR encryption with Argon2 KDF for credential storage
- **Secure nonces** - Use cryptographically secure random nonces instead of time-based generation
- **Code organization** - Refactored into modules: `credentials`, `ui/dialogs`, `ui/panels`, `state`

### Fixed
- **SSH command injection** - Fixed potential command injection vulnerability using proper shell escaping

### Removed
- **Always-on-top** - Removed feature due to unreliable Wayland support
- Unused dependencies: `egui_extras`, `tokio-stream`
- Dead code: 15+ unused functions removed

### Security
- SSH paths now properly escaped to prevent command injection
- Credentials encrypted with Argon2-derived keys (OWASP recommended)
- Nonces generated with cryptographic randomness
- Credential files set to 0600 permissions on Unix
- Regex patterns limited in size to prevent DoS attacks
- SSH host keys verified against known_hosts file

## [0.2.0] - 2026-01-22

### Added

#### Log Rotation Detection
- Automatic detection of log file rotation (logrotate, truncation)
- Seamless continuation of tailing after rotation
- Visual notification when rotation is detected
- Platform-specific inode tracking (Unix/Windows)

#### Session Persistence
- Remember open files and SSH sources on exit
- Automatic session restore on startup
- New `remember_last_session` setting in config
- Session state saved to `~/.config/oxitailr/session.json`

#### Auto-Open Feature
- Mark sources to automatically open on startup
- Star toggle (★/☆) in source panel for quick access
- Auto-open checkbox in SSH source dialog
- Separate tracking for local files and SSH sources

#### Secure Password Storage
- SSH passwords now stored in OS keychain (libsecret on Linux, Keychain on macOS, Credential Manager on Windows)
- Automatic migration of existing passwords from JSON to keychain
- Passwords removed from JSON files for security
- Added `keyring` crate dependency

#### SSH Authentication Improvements
- Try multiple SSH keys in order: specified key, id_ed25519, id_rsa, id_ecdsa
- Fall back to password authentication if key auth fails
- Detailed error messages showing which authentication methods were tried
- Password field in SSH dialog with secure storage

### Changed
- SSH sources JSON no longer stores passwords (migrated to OS keychain)
- Settings dialog now includes "Remember last session" option
- Source panel shows auto-open status with star indicator

### Security
- Passwords no longer stored in plain text JSON files
- Uses OS-native secure credential storage

## [0.1.0] - 2026-01-20

### Added

#### Core Features
- Real-time log file tailing
- Local file source support
- SSH remote source support
- JSON log auto-parsing
- ANSI color code rendering

#### User Interface
- Modern GUI built with egui
- Tabbed source view (All / individual sources)
- Collapsible source panel
- Settings dialog with multiple configuration options
- About dialog

#### Filtering
- Regex-based live filtering
- Log level filtering (Trace, Debug, Info, Warn, Error, Fatal)
- Filter presets from config file
- Advanced filter builder UI

#### Display Options
- Configurable font size
- Line spacing adjustment
- Timestamp display toggle
- Source name display toggle
- Line wrapping option
- Theme support (Light, Dark, System)

#### Highlighting
- Customizable highlight rules
- Pattern matching with colors
- Bold/italic text support
- Default ERROR and WARN highlighting

#### Configuration
- TOML configuration file support
- Example configuration included
- CLI arguments for file and config path
- Buffer size configuration

#### SSH Features
- SSH key authentication
- Saved SSH sources (persisted to JSON)
- Edit and reconnect to saved sources
- Connection status indicators

### Technical
- Async runtime with Tokio
- File watching with notify crate
- Cross-platform support (Linux, macOS, Windows)

<!-- Compare links (only versions with release tags are linked; 0.2.11/0.2.12
     and pre-0.2.3 versions were never tagged and ship within adjacent tags). -->
[Unreleased]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.20...v0.3.0
[0.2.20]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.19...v0.2.20
[0.2.19]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.18...v0.2.19
[0.2.18]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.17...v0.2.18
[0.2.17]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.16...v0.2.17
[0.2.16]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.15...v0.2.16
[0.2.15]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.14...v0.2.15
[0.2.14]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.13...v0.2.14
[0.2.13]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.10...v0.2.13
[0.2.10]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.9...v0.2.10
[0.2.9]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.8...v0.2.9
[0.2.8]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.7...v0.2.8
[0.2.7]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.6...v0.2.7
[0.2.6]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.5...v0.2.6
[0.2.5]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.4...v0.2.5
[0.2.4]: https://github.com/MarcelineVPQ/oxitailr/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/MarcelineVPQ/oxitailr/releases/tag/v0.2.3
