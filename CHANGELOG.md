# Changelog

All notable changes to Oxitailr will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

### Security
- SSH paths now properly escaped to prevent command injection
- Credentials encrypted with Argon2-derived keys (OWASP recommended)
- Nonces generated with cryptographic randomness
- Credential files set to 0600 permissions on Unix
- Regex patterns limited in size to prevent DoS attacks
- SSH host keys verified against known_hosts file

### Removed
- Unused dependencies: `egui_extras`, `tokio-stream`
- Dead code: 15+ unused functions removed

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
