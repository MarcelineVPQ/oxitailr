mod alert;
mod config;
mod filter;
mod models;
mod parser;
mod source;

use anyhow::Result;
use clap::Parser as ClapParser;
use config::{AppConfig, SourceConfig};
use eframe::egui;
use egui::IconData;
use filter::{FilterEngine, FilterRule};
use models::{LogEntry, LogLevel, SourceInfo, SourceStatus};
use parser::{auto_detect_parser, Parser, PlainParser};
use regex::Regex;
use source::{SourceEvent, SourceManager};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

// Embed the icon at compile time
const ICON_BYTES: &[u8] = include_bytes!("../assets/icon_64x64.png");

// Keyring service name for secure password storage
const KEYRING_SERVICE: &str = "oxitailr";

/// Store an SSH password securely in the OS keychain
fn store_ssh_password(source_name: &str, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, source_name)
        .map_err(|e| anyhow::anyhow!("Failed to create keyring entry: {}", e))?;
    entry
        .set_password(password)
        .map_err(|e| anyhow::anyhow!("Failed to store password: {}", e))?;
    Ok(())
}

/// Retrieve an SSH password from the OS keychain
fn get_ssh_password(source_name: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, source_name).ok()?;
    entry.get_password().ok()
}

/// Delete an SSH password from the OS keychain
fn delete_ssh_password(source_name: &str) {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, source_name) {
        let _ = entry.delete_credential();
    }
}

fn load_icon() -> Option<IconData> {
    let image = image::load_from_memory(ICON_BYTES).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

#[derive(ClapParser)]
#[command(name = "oxitailr")]
#[command(author, version, about = "A modern log viewer with GUI")]
struct Cli {
    /// Path to log file to view (optional - can select via file picker)
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Path to config file
    #[arg(short, long, value_name = "CONFIG")]
    config: Option<PathBuf>,

    /// Maximum lines to keep in buffer (overrides config)
    #[arg(short, long)]
    max_lines: Option<usize>,
}

fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oxitailr")
        .join("config.toml")
}

fn load_or_default_config(path: Option<PathBuf>) -> AppConfig {
    let config_path = path.unwrap_or_else(default_config_path);

    if config_path.exists() {
        match config::load_config(&config_path) {
            Ok(cfg) => {
                tracing::info!("Loaded config from {}", config_path.display());
                return cfg;
            }
            Err(e) => {
                tracing::warn!("Failed to load config: {}, using defaults", e);
            }
        }
    }

    AppConfig::default()
}

// ANSI color code parsing
static ANSI_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[([0-9;]*)m").unwrap()
});

#[derive(Clone)]
struct ColoredSpan {
    text: String,
    color: egui::Color32,
    bold: bool,
}

fn ansi_to_color(code: u8) -> egui::Color32 {
    match code {
        30 => egui::Color32::from_rgb(0, 0, 0),        // Black
        31 => egui::Color32::from_rgb(205, 49, 49),    // Red
        32 => egui::Color32::from_rgb(13, 188, 121),   // Green
        33 => egui::Color32::from_rgb(229, 229, 16),   // Yellow
        34 => egui::Color32::from_rgb(36, 114, 200),   // Blue
        35 => egui::Color32::from_rgb(188, 63, 188),   // Magenta
        36 => egui::Color32::from_rgb(17, 168, 205),   // Cyan
        37 => egui::Color32::from_rgb(229, 229, 229),  // White
        // Bright colors
        90 => egui::Color32::from_rgb(102, 102, 102),  // Bright Black
        91 => egui::Color32::from_rgb(241, 76, 76),    // Bright Red
        92 => egui::Color32::from_rgb(35, 209, 139),   // Bright Green
        93 => egui::Color32::from_rgb(245, 245, 67),   // Bright Yellow
        94 => egui::Color32::from_rgb(59, 142, 234),   // Bright Blue
        95 => egui::Color32::from_rgb(214, 112, 214),  // Bright Magenta
        96 => egui::Color32::from_rgb(41, 184, 219),   // Bright Cyan
        97 => egui::Color32::from_rgb(255, 255, 255),  // Bright White
        _ => egui::Color32::from_rgb(200, 200, 200),   // Default
    }
}

fn parse_ansi_line(text: &str) -> Vec<ColoredSpan> {
    let mut spans = Vec::new();
    let mut current_color = egui::Color32::from_rgb(200, 200, 200);
    let mut current_bold = false;
    let mut last_end = 0;

    for cap in ANSI_REGEX.captures_iter(text) {
        let full_match = cap.get(0).unwrap();

        // Add text before this escape sequence
        if full_match.start() > last_end {
            let segment = &text[last_end..full_match.start()];
            if !segment.is_empty() {
                spans.push(ColoredSpan {
                    text: segment.to_string(),
                    color: current_color,
                    bold: current_bold,
                });
            }
        }

        // Parse the escape codes
        let codes_str = cap.get(1).map_or("", |m| m.as_str());
        for code_str in codes_str.split(';') {
            if let Ok(code) = code_str.parse::<u8>() {
                match code {
                    0 => {
                        current_color = egui::Color32::from_rgb(200, 200, 200);
                        current_bold = false;
                    }
                    1 => current_bold = true,
                    22 => current_bold = false,
                    30..=37 | 90..=97 => current_color = ansi_to_color(code),
                    39 => current_color = egui::Color32::from_rgb(200, 200, 200),
                    _ => {}
                }
            }
        }

        last_end = full_match.end();
    }

    // Add remaining text
    if last_end < text.len() {
        let segment = &text[last_end..];
        if !segment.is_empty() {
            spans.push(ColoredSpan {
                text: segment.to_string(),
                color: current_color,
                bold: current_bold,
            });
        }
    }

    // If no spans were created, return the whole text as default
    if spans.is_empty() {
        spans.push(ColoredSpan {
            text: text.to_string(),
            color: egui::Color32::from_rgb(200, 200, 200),
            bold: false,
        });
    }

    spans
}

fn strip_ansi(text: &str) -> String {
    ANSI_REGEX.replace_all(text, "").to_string()
}

fn log_level_color(level: Option<&LogLevel>) -> egui::Color32 {
    match level {
        Some(LogLevel::Trace) => egui::Color32::from_rgb(100, 100, 100),
        Some(LogLevel::Debug) => egui::Color32::from_rgb(140, 140, 140),
        Some(LogLevel::Info) => egui::Color32::from_rgb(80, 180, 220),
        Some(LogLevel::Warn) => egui::Color32::from_rgb(220, 180, 50),
        Some(LogLevel::Error) => egui::Color32::from_rgb(220, 80, 80),
        Some(LogLevel::Fatal) => egui::Color32::from_rgb(255, 50, 150),
        None => egui::Color32::from_rgb(200, 200, 200),
    }
}

#[derive(Clone)]
struct DisplayLine {
    entry: LogEntry,
    spans: Vec<ColoredSpan>,
    has_ansi: bool,
    line_num: usize,
}

impl DisplayLine {
    fn from_entry(entry: LogEntry, line_num: usize) -> Self {
        let has_ansi = entry.raw.contains("\x1b[");
        let spans = parse_ansi_line(&entry.raw);
        Self {
            entry,
            spans,
            has_ansi,
            line_num,
        }
    }
}

struct LogState {
    lines: VecDeque<DisplayLine>,
    max_lines: usize,
    total_lines_read: usize,
}

impl LogState {
    fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_lines),
            max_lines,
            total_lines_read: 0,
        }
    }

    fn add_entry(&mut self, entry: LogEntry) {
        self.total_lines_read += 1;
        let display_line = DisplayLine::from_entry(entry, self.total_lines_read);
        if self.lines.len() >= self.max_lines {
            self.lines.pop_front();
        }
        self.lines.push_back(display_line);
    }
}

// Saved SSH source for JSON persistence
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SavedSshSource {
    name: String,
    host: String,
    port: u16,
    user: String,
    remote_path: String,
    key_path: Option<String>,
    /// Legacy field - passwords are now stored in OS keychain
    /// This field is only used for migration and should not be written
    #[serde(default, skip_serializing)]
    password: Option<String>,
    #[serde(default)]
    auto_open: bool,
}

// Session state for persistence
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SessionState {
    open_local_files: Vec<String>,
    open_ssh_sources: Vec<String>,
    timestamp: String,
}

// Saved local file source for auto_open feature
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SavedLocalSource {
    name: String,
    path: String,
    #[serde(default)]
    auto_open: bool,
}

// SSH Dialog state
#[derive(Default)]
struct SshDialogState {
    open: bool,
    editing: Option<String>, // Some(name) if editing existing source
    name: String,
    host: String,
    port: String,
    user: String,
    remote_path: String,
    key_path: String,
    password: String,
    error: Option<String>,
    auto_open: bool,
}

impl SshDialogState {
    fn reset(&mut self) {
        self.editing = None;
        self.name.clear();
        self.host.clear();
        self.port = "22".to_string();
        self.user.clear();
        self.remote_path.clear();
        self.key_path.clear();
        self.password.clear();
        self.error = None;
        self.auto_open = false;
    }

    fn load_from_saved(&mut self, source: &SavedSshSource) {
        self.editing = Some(source.name.clone());
        self.name = source.name.clone();
        self.host = source.host.clone();
        self.port = source.port.to_string();
        self.user = source.user.clone();
        self.remote_path = source.remote_path.clone();
        self.key_path = source.key_path.clone().unwrap_or_default();
        // Try to load password from keychain
        self.password = get_ssh_password(&source.name).unwrap_or_default();
        self.error = None;
        self.auto_open = source.auto_open;
    }

    fn to_saved(&self) -> SavedSshSource {
        SavedSshSource {
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port.parse().unwrap_or(22),
            user: self.user.clone(),
            remote_path: self.remote_path.clone(),
            key_path: if self.key_path.is_empty() { None } else { Some(self.key_path.clone()) },
            password: None, // Passwords are stored in keychain, not JSON
            auto_open: self.auto_open,
        }
    }
}

// Settings Dialog state
struct SettingsDialogState {
    open: bool,
    buffer_size: String,
    font_size: f32,
    show_timestamps: bool,
    show_source: bool,
    wrap_lines: bool,
    auto_scroll: bool,
    use_auto_parser: bool,
    line_spacing: f32,
    tab_width: String,
    update_interval_ms: String,
    always_on_top: bool,
    remember_last_session: bool,
}

impl Default for SettingsDialogState {
    fn default() -> Self {
        Self {
            open: false,
            buffer_size: "10000".to_string(),
            font_size: 13.0,
            show_timestamps: true,
            show_source: true,
            wrap_lines: false,
            auto_scroll: true,
            use_auto_parser: true,
            line_spacing: 1.0,
            tab_width: "4".to_string(),
            update_interval_ms: "100".to_string(),
            always_on_top: false,
            remember_last_session: true,
        }
    }
}

// Highlight rule for configurable highlighting
#[derive(Clone)]
struct HighlightRule {
    pattern: String,
    foreground: egui::Color32,
    background: egui::Color32,
    bold: bool,
    italic: bool,
    ignore_case: bool,
    enabled: bool,
}

impl Default for HighlightRule {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            foreground: egui::Color32::WHITE,
            background: egui::Color32::from_rgb(200, 50, 50),
            bold: false,
            italic: false,
            ignore_case: true,
            enabled: true,
        }
    }
}

impl HighlightRule {
    fn matches(&self, text: &str) -> bool {
        if self.pattern.is_empty() || !self.enabled {
            return false;
        }
        if self.ignore_case {
            text.to_lowercase().contains(&self.pattern.to_lowercase())
        } else {
            text.contains(&self.pattern)
        }
    }
}

// Highlight dialog state
#[derive(Default)]
struct HighlightDialogState {
    open: bool,
    editing_index: Option<usize>,
    current_rule: HighlightRule,
}

// Filter preset UI
#[derive(Clone)]
struct FilterPreset {
    name: String,
    rules: Vec<FilterRule>,
}

struct TailLoggerApp {
    // Core state
    log_state: Arc<Mutex<LogState>>,
    config: AppConfig,
    config_path: PathBuf,

    // Source management
    source_manager: Arc<tokio::sync::Mutex<SourceManager>>,
    source_infos: HashMap<String, SourceInfo>,
    event_rx: Option<mpsc::Receiver<SourceEvent>>,
    runtime: Arc<Runtime>,
    selected_source: Option<String>, // None = "All", Some(name) = specific source

    // Parsers
    plain_parser: PlainParser,
    use_auto_parser: bool,

    // Filter state
    filter_engine: FilterEngine,
    filter_text: String,
    filter_error: Option<String>,
    show_trace: bool,
    show_debug: bool,
    show_info: bool,
    show_warning: bool,
    show_error: bool,
    show_fatal: bool,
    filter_presets: Vec<FilterPreset>,
    selected_preset: Option<usize>,

    // Advanced filter builder
    show_filter_builder: bool,
    builder_rule_type: usize,
    builder_pattern: String,
    builder_field_name: String,
    builder_is_exclude: bool,

    // UI state
    auto_scroll: bool,
    search_text: String,
    font_size: f32,
    new_lines_received: bool,
    scroll_to_row: Option<usize>,
    current_scroll_row: usize,
    initial_scroll_pending: bool,
    show_timestamps: bool,
    show_source: bool,
    wrap_lines: bool,
    line_spacing: f32,
    tab_width: usize,
    update_interval_ms: u64,
    always_on_top: bool,

    // Highlighting
    highlight_rules: Vec<HighlightRule>,
    highlight_dialog: HighlightDialogState,

    // Saved SSH sources
    saved_ssh_sources: Vec<SavedSshSource>,
    ssh_sources_path: PathBuf,

    // Saved local sources (for auto_open)
    saved_local_sources: Vec<SavedLocalSource>,
    local_sources_path: PathBuf,

    // Session persistence
    session_path: PathBuf,
    cli_file_provided: bool,

    // Dialogs
    ssh_dialog: SshDialogState,
    settings_dialog: SettingsDialogState,
    show_source_panel: bool,
    show_about_dialog: bool,
}

impl TailLoggerApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        config: AppConfig,
        config_path: PathBuf,
        initial_file: Option<PathBuf>,
        runtime: Arc<Runtime>,
    ) -> Self {
        let buffer_size = config.general.buffer_size;
        let log_state = Arc::new(Mutex::new(LogState::new(buffer_size)));

        // Create source manager
        let mut source_manager = SourceManager::new();
        let event_rx = source_manager.take_event_receiver();
        let source_manager = Arc::new(tokio::sync::Mutex::new(source_manager));

        // Load filter presets from config
        let filter_presets: Vec<FilterPreset> = config
            .filters
            .iter()
            .map(|(name, fc)| {
                FilterPreset {
                    name: name.clone(),
                    rules: fc.rules.clone(),
                }
            })
            .collect();

        let mut app = Self {
            log_state,
            config: config.clone(),
            config_path,
            source_manager,
            source_infos: HashMap::new(),
            event_rx,
            runtime,
            selected_source: None,
            plain_parser: PlainParser::new(),
            use_auto_parser: true,
            filter_engine: FilterEngine::new(),
            filter_text: String::new(),
            filter_error: None,
            show_trace: true,
            show_debug: true,
            show_info: true,
            show_warning: true,
            show_error: true,
            show_fatal: true,
            filter_presets,
            selected_preset: None,
            show_filter_builder: false,
            builder_rule_type: 0,
            builder_pattern: String::new(),
            builder_field_name: String::new(),
            builder_is_exclude: false,
            auto_scroll: config.general.follow,
            search_text: String::new(),
            font_size: 13.0,
            new_lines_received: false,
            scroll_to_row: None,
            current_scroll_row: 0,
            initial_scroll_pending: true,
            show_timestamps: config.general.show_timestamps,
            show_source: config.general.show_source,
            wrap_lines: config.general.wrap_lines,
            line_spacing: 1.0,
            tab_width: 4,
            update_interval_ms: 100,
            always_on_top: false,
            highlight_rules: vec![
                // Default highlight rules
                HighlightRule {
                    pattern: "ERROR".to_string(),
                    foreground: egui::Color32::WHITE,
                    background: egui::Color32::from_rgb(180, 40, 40),
                    bold: true,
                    ignore_case: true,
                    enabled: true,
                    ..Default::default()
                },
                HighlightRule {
                    pattern: "WARN".to_string(),
                    foreground: egui::Color32::BLACK,
                    background: egui::Color32::from_rgb(230, 180, 50),
                    bold: false,
                    ignore_case: true,
                    enabled: true,
                    ..Default::default()
                },
            ],
            highlight_dialog: HighlightDialogState::default(),
            saved_ssh_sources: Vec::new(),
            ssh_sources_path: dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("oxitailr")
                .join("ssh_sources.json"),
            saved_local_sources: Vec::new(),
            local_sources_path: dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("oxitailr")
                .join("local_sources.json"),
            session_path: dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("oxitailr")
                .join("session.json"),
            cli_file_provided: initial_file.is_some(),
            ssh_dialog: SshDialogState::default(),
            settings_dialog: SettingsDialogState::default(),
            show_source_panel: true,
            show_about_dialog: false,
        };

        // Load saved sources
        app.load_ssh_sources();
        app.load_local_sources();

        // Migrate any legacy passwords from JSON to keychain
        app.migrate_passwords_to_keychain();

        // Add initial file if provided via CLI
        if let Some(path) = initial_file {
            app.add_local_source_from_path(path.clone());
            // Also save it as a local source for future reference
            app.update_or_add_local_source(SavedLocalSource {
                name: path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "local".to_string()),
                path: path.display().to_string(),
                auto_open: false,
            });
        } else if config.general.remember_last_session {
            // Try to restore last session
            app.load_session();
        }

        // Load sources from config
        for source_config in &config.sources {
            if source_config.is_enabled() {
                app.add_source_from_config(source_config.clone());
            }
        }

        // Open all sources with auto_open enabled
        app.open_auto_open_sources();

        app
    }

    fn load_ssh_sources(&mut self) {
        if self.ssh_sources_path.exists() {
            match std::fs::read_to_string(&self.ssh_sources_path) {
                Ok(content) => {
                    match serde_json::from_str::<Vec<SavedSshSource>>(&content) {
                        Ok(sources) => {
                            self.saved_ssh_sources = sources;
                            tracing::info!("Loaded {} SSH sources from {}",
                                self.saved_ssh_sources.len(),
                                self.ssh_sources_path.display());
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse SSH sources: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read SSH sources file: {}", e);
                }
            }
        }
    }

    fn save_ssh_sources(&self) {
        if let Some(parent) = self.ssh_sources_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.saved_ssh_sources) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&self.ssh_sources_path, content) {
                    tracing::error!("Failed to save SSH sources: {}", e);
                } else {
                    tracing::info!("Saved {} SSH sources to {}",
                        self.saved_ssh_sources.len(),
                        self.ssh_sources_path.display());
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize SSH sources: {}", e);
            }
        }
    }

    fn load_local_sources(&mut self) {
        if self.local_sources_path.exists() {
            match std::fs::read_to_string(&self.local_sources_path) {
                Ok(content) => {
                    match serde_json::from_str::<Vec<SavedLocalSource>>(&content) {
                        Ok(sources) => {
                            self.saved_local_sources = sources;
                            tracing::info!("Loaded {} local sources from {}",
                                self.saved_local_sources.len(),
                                self.local_sources_path.display());
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse local sources: {}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read local sources file: {}", e);
                }
            }
        }
    }

    fn save_local_sources(&self) {
        if let Some(parent) = self.local_sources_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.saved_local_sources) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&self.local_sources_path, content) {
                    tracing::error!("Failed to save local sources: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize local sources: {}", e);
            }
        }
    }

    fn update_or_add_local_source(&mut self, source: SavedLocalSource) {
        if let Some(existing) = self.saved_local_sources.iter_mut().find(|s| s.path == source.path) {
            *existing = source;
        } else {
            self.saved_local_sources.push(source);
        }
        self.save_local_sources();
    }

    fn find_saved_local_source(&self, path: &str) -> Option<&SavedLocalSource> {
        self.saved_local_sources.iter().find(|s| s.path == path)
    }

    fn migrate_passwords_to_keychain(&mut self) {
        let mut migrated = false;
        for source in &self.saved_ssh_sources {
            if let Some(ref password) = source.password {
                if !password.is_empty() {
                    if let Err(e) = store_ssh_password(&source.name, password) {
                        tracing::warn!("Failed to migrate password for {}: {}", source.name, e);
                    } else {
                        tracing::info!("Migrated password for {} to keychain", source.name);
                        migrated = true;
                    }
                }
            }
        }

        // If we migrated any passwords, save the sources without passwords
        if migrated {
            // Clear passwords from in-memory sources (they'll be None when saved due to skip_serializing)
            for source in &mut self.saved_ssh_sources {
                source.password = None;
            }
            self.save_ssh_sources();
        }
    }

    fn load_session(&mut self) {
        if !self.session_path.exists() {
            return;
        }

        match std::fs::read_to_string(&self.session_path) {
            Ok(content) => {
                match serde_json::from_str::<SessionState>(&content) {
                    Ok(session) => {
                        tracing::info!("Restoring session from {}", session.timestamp);

                        // Restore local files
                        for path_str in session.open_local_files {
                            let path = PathBuf::from(&path_str);
                            if path.exists() {
                                self.add_local_source_from_path(path);
                            }
                        }

                        // Restore SSH sources
                        for name in session.open_ssh_sources {
                            if let Some(source) = self.find_saved_ssh_source(&name).cloned() {
                                let key_path = source.key_path.as_ref().map(PathBuf::from);
                                let password = get_ssh_password(&source.name);
                                self.add_ssh_source(
                                    source.name,
                                    source.host,
                                    source.port,
                                    source.user,
                                    source.remote_path,
                                    key_path,
                                    password,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse session file: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read session file: {}", e);
            }
        }
    }

    fn save_session(&self) {
        if !self.config.general.remember_last_session {
            return;
        }

        let mut open_local_files = Vec::new();
        let mut open_ssh_sources = Vec::new();

        for (name, info) in &self.source_infos {
            match info.source_type {
                models::SourceType::Local => {
                    open_local_files.push(info.path.clone());
                }
                models::SourceType::Ssh => {
                    open_ssh_sources.push(name.clone());
                }
            }
        }

        let session = SessionState {
            open_local_files,
            open_ssh_sources,
            timestamp: chrono::Local::now().to_rfc3339(),
        };

        if let Some(parent) = self.session_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match serde_json::to_string_pretty(&session) {
            Ok(content) => {
                if let Err(e) = std::fs::write(&self.session_path, content) {
                    tracing::error!("Failed to save session: {}", e);
                } else {
                    tracing::info!("Session saved with {} local files and {} SSH sources",
                        session.open_local_files.len(),
                        session.open_ssh_sources.len());
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize session: {}", e);
            }
        }
    }

    fn open_auto_open_sources(&mut self) {
        // Open local sources with auto_open
        let local_sources: Vec<SavedLocalSource> = self.saved_local_sources
            .iter()
            .filter(|s| s.auto_open)
            .cloned()
            .collect();

        for source in local_sources {
            let path = PathBuf::from(&source.path);
            if path.exists() && !self.source_infos.contains_key(&source.name) {
                self.add_local_source_from_path(path);
            }
        }

        // Open SSH sources with auto_open
        let ssh_sources: Vec<SavedSshSource> = self.saved_ssh_sources
            .iter()
            .filter(|s| s.auto_open)
            .cloned()
            .collect();

        for source in ssh_sources {
            if !self.source_infos.contains_key(&source.name) {
                let key_path = source.key_path.as_ref().map(PathBuf::from);
                let password = get_ssh_password(&source.name);
                self.add_ssh_source(
                    source.name,
                    source.host,
                    source.port,
                    source.user,
                    source.remote_path,
                    key_path,
                    password,
                );
            }
        }
    }

    fn toggle_local_source_auto_open(&mut self, path: &str) {
        if let Some(source) = self.saved_local_sources.iter_mut().find(|s| s.path == path) {
            source.auto_open = !source.auto_open;
            self.save_local_sources();
        }
    }

    fn toggle_ssh_source_auto_open(&mut self, name: &str) {
        if let Some(source) = self.saved_ssh_sources.iter_mut().find(|s| s.name == name) {
            source.auto_open = !source.auto_open;
            self.save_ssh_sources();
        }
    }

    fn is_local_source_auto_open(&self, path: &str) -> bool {
        self.saved_local_sources.iter().any(|s| s.path == path && s.auto_open)
    }

    fn is_ssh_source_auto_open(&self, name: &str) -> bool {
        self.saved_ssh_sources.iter().any(|s| s.name == name && s.auto_open)
    }

    fn find_saved_ssh_source(&self, name: &str) -> Option<&SavedSshSource> {
        self.saved_ssh_sources.iter().find(|s| s.name == name)
    }

    fn update_or_add_ssh_source(&mut self, source: SavedSshSource, password: Option<String>) {
        // Store password in keychain if provided
        if let Some(ref pwd) = password {
            if !pwd.is_empty() {
                if let Err(e) = store_ssh_password(&source.name, pwd) {
                    tracing::warn!("Failed to store password in keychain: {}", e);
                }
            }
        }

        if let Some(existing) = self.saved_ssh_sources.iter_mut().find(|s| s.name == source.name) {
            *existing = source;
        } else {
            self.saved_ssh_sources.push(source);
        }
        self.save_ssh_sources();
    }

    fn remove_saved_ssh_source(&mut self, name: &str) {
        // Also remove password from keychain
        delete_ssh_password(name);
        self.saved_ssh_sources.retain(|s| s.name != name);
        self.save_ssh_sources();
    }

    fn add_local_source_from_path(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "local".to_string());

        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            sm.add_local_source(name.clone(), path);
            let _ = sm.start_source(&name).await;
        });
    }

    fn add_source_from_config(&mut self, source_config: SourceConfig) {
        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            match source_config {
                SourceConfig::Local { name, path, .. } => {
                    sm.add_local_source(name.clone(), PathBuf::from(path));
                    let _ = sm.start_source(&name).await;
                }
                SourceConfig::Ssh {
                    name,
                    host,
                    port,
                    user,
                    path,
                    key_path,
                    ..
                } => {
                    sm.add_ssh_source(name.clone(), host, user, path, Some(port), key_path, None);
                    let _ = sm.start_source(&name).await;
                }
            }
        });
    }

    fn add_ssh_source(&mut self, name: String, host: String, port: u16, user: String, path: String, key_path: Option<PathBuf>, password: Option<String>) {
        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            sm.add_ssh_source(name.clone(), host, user, path, Some(port), key_path, password);
            let _ = sm.start_source(&name).await;
        });
    }

    fn remove_source(&mut self, name: &str) {
        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();
        let name = name.to_string();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            let _ = sm.stop_source(&name).await;
        });

        self.source_infos.remove(&name);
    }

    fn open_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Log files", &["log", "txt"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            self.add_local_source_from_path(path);
        }
    }

    fn reload_sources(&mut self) {
        // Clear log state
        {
            let mut state = self.log_state.lock().unwrap();
            state.lines.clear();
            state.total_lines_read = 0;
        }

        // Scroll to bottom when data loads
        self.initial_scroll_pending = true;

        // Restart all sources
        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();
        let source_names: Vec<String> = self.source_infos.keys().cloned().collect();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            // Stop all sources
            for name in &source_names {
                let _ = sm.stop_source(name).await;
            }
            // Start all sources again
            for name in &source_names {
                let _ = sm.start_source(name).await;
            }
        });
    }

    fn open_settings_dialog(&mut self) {
        // Sync current state to dialog
        self.settings_dialog.buffer_size = self.config.general.buffer_size.to_string();
        self.settings_dialog.font_size = self.font_size;
        self.settings_dialog.show_timestamps = self.show_timestamps;
        self.settings_dialog.show_source = self.show_source;
        self.settings_dialog.wrap_lines = self.wrap_lines;
        self.settings_dialog.auto_scroll = self.auto_scroll;
        self.settings_dialog.use_auto_parser = self.use_auto_parser;
        self.settings_dialog.line_spacing = self.line_spacing;
        self.settings_dialog.tab_width = self.tab_width.to_string();
        self.settings_dialog.update_interval_ms = self.update_interval_ms.to_string();
        self.settings_dialog.always_on_top = self.always_on_top;
        self.settings_dialog.remember_last_session = self.config.general.remember_last_session;
        self.settings_dialog.open = true;
    }

    fn render_settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings_dialog.open {
            return;
        }

        let mut open_highlight_dialog = false;

        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(5.0);

                egui::Grid::new("settings_grid")
                    .num_columns(2)
                    .spacing([20.0, 8.0])
                    .show(ui, |ui| {
                        // Display settings
                        ui.label(egui::RichText::new("Display").strong());
                        ui.end_row();

                        ui.label("Font size:");
                        ui.add(egui::Slider::new(&mut self.settings_dialog.font_size, 8.0..=24.0));
                        ui.end_row();

                        ui.label("Line spacing:");
                        ui.add(egui::Slider::new(&mut self.settings_dialog.line_spacing, 0.5..=3.0).step_by(0.1));
                        ui.end_row();

                        ui.label("Tab width:");
                        ui.add(egui::TextEdit::singleline(&mut self.settings_dialog.tab_width)
                            .desired_width(60.0));
                        ui.end_row();

                        ui.label("Show timestamps:");
                        ui.checkbox(&mut self.settings_dialog.show_timestamps, "");
                        ui.end_row();

                        ui.label("Show source:");
                        ui.checkbox(&mut self.settings_dialog.show_source, "");
                        ui.end_row();

                        ui.label("Wrap lines:");
                        ui.checkbox(&mut self.settings_dialog.wrap_lines, "");
                        ui.end_row();

                        ui.label("");
                        ui.end_row();

                        // Behavior settings
                        ui.label(egui::RichText::new("Behavior").strong());
                        ui.end_row();

                        ui.label("Auto-scroll:");
                        ui.checkbox(&mut self.settings_dialog.auto_scroll, "");
                        ui.end_row();

                        ui.label("Auto-parse JSON:");
                        ui.checkbox(&mut self.settings_dialog.use_auto_parser, "");
                        ui.end_row();

                        ui.label("Buffer size:");
                        ui.add(egui::TextEdit::singleline(&mut self.settings_dialog.buffer_size)
                            .desired_width(100.0));
                        ui.end_row();

                        ui.label("Update interval (ms):");
                        ui.add(egui::TextEdit::singleline(&mut self.settings_dialog.update_interval_ms)
                            .desired_width(80.0));
                        ui.end_row();

                        ui.label("Remember last session:");
                        ui.checkbox(&mut self.settings_dialog.remember_last_session, "");
                        ui.end_row();

                        ui.label("");
                        ui.end_row();

                        // Window settings
                        ui.label(egui::RichText::new("Window").strong());
                        ui.end_row();

                        ui.label("Always on top:");
                        ui.checkbox(&mut self.settings_dialog.always_on_top, "");
                        ui.end_row();

                        ui.label("");
                        ui.end_row();

                        // Highlighting
                        ui.label(egui::RichText::new("Highlighting").strong());
                        ui.end_row();

                        ui.label("Highlight rules:");
                        if ui.button(format!("Configure ({} rules)", self.highlight_rules.len())).clicked() {
                            open_highlight_dialog = true;
                        }
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Config: {}", self.config_path.display()))
                            .small()
                            .color(egui::Color32::from_rgb(120, 120, 120))
                    );
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.settings_dialog.open = false;
                    }

                    if ui.button("Apply").clicked() {
                        self.apply_settings_from_dialog();
                        self.settings_dialog.open = false;
                    }

                    if ui.button("Save to File").clicked() {
                        self.apply_settings_from_dialog();

                        // Save to file
                        if let Some(parent) = self.config_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = config::save_config(&self.config, &self.config_path) {
                            tracing::error!("Failed to save config: {}", e);
                        }

                        self.settings_dialog.open = false;
                    }
                });
            });

        if open_highlight_dialog {
            self.highlight_dialog.open = true;
        }
    }

    fn apply_settings_from_dialog(&mut self) {
        self.font_size = self.settings_dialog.font_size;
        self.show_timestamps = self.settings_dialog.show_timestamps;
        self.show_source = self.settings_dialog.show_source;
        self.wrap_lines = self.settings_dialog.wrap_lines;
        self.auto_scroll = self.settings_dialog.auto_scroll;
        self.use_auto_parser = self.settings_dialog.use_auto_parser;
        self.line_spacing = self.settings_dialog.line_spacing;
        self.always_on_top = self.settings_dialog.always_on_top;

        if let Ok(tab) = self.settings_dialog.tab_width.parse::<usize>() {
            self.tab_width = tab.clamp(1, 16);
        }

        if let Ok(interval) = self.settings_dialog.update_interval_ms.parse::<u64>() {
            self.update_interval_ms = interval.clamp(10, 5000);
        }

        if let Ok(size) = self.settings_dialog.buffer_size.parse::<usize>() {
            self.config.general.buffer_size = size;
            let mut state = self.log_state.lock().unwrap();
            state.max_lines = size;
        }

        // Update config struct
        self.config.general.show_timestamps = self.show_timestamps;
        self.config.general.show_source = self.show_source;
        self.config.general.wrap_lines = self.wrap_lines;
        self.config.general.follow = self.auto_scroll;
        self.config.general.remember_last_session = self.settings_dialog.remember_last_session;
    }

    fn render_highlight_dialog(&mut self, ctx: &egui::Context) {
        if !self.highlight_dialog.open {
            return;
        }

        let mut close_dialog = false;
        let mut delete_rule: Option<usize> = None;
        let mut edit_rule: Option<usize> = None;

        egui::Window::new("Highlight Rules")
            .collapsible(false)
            .resizable(true)
            .default_size([500.0, 400.0])
            .show(ctx, |ui| {
                ui.heading("Configured Rules");
                ui.add_space(5.0);

                // List existing rules
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for (i, rule) in self.highlight_rules.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut rule.enabled, "");

                                // Preview color
                                let preview_size = egui::vec2(60.0, 18.0);
                                let (rect, _) = ui.allocate_exact_size(preview_size, egui::Sense::hover());
                                ui.painter().rect_filled(rect, 2.0, rule.background);
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Sample",
                                    egui::FontId::monospace(12.0),
                                    rule.foreground,
                                );

                                ui.label(&rule.pattern);

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("x").clicked() {
                                        delete_rule = Some(i);
                                    }
                                    if ui.small_button("Edit").clicked() {
                                        edit_rule = Some(i);
                                    }
                                });
                            });
                        }
                    });

                ui.separator();
                ui.add_space(5.0);

                // Rule editor
                let editing_text = if self.highlight_dialog.editing_index.is_some() {
                    "Edit Rule"
                } else {
                    "New Rule"
                };
                ui.heading(editing_text);
                ui.add_space(5.0);

                egui::Grid::new("highlight_editor")
                    .num_columns(2)
                    .spacing([10.0, 5.0])
                    .show(ui, |ui| {
                        ui.label("Pattern:");
                        ui.text_edit_singleline(&mut self.highlight_dialog.current_rule.pattern);
                        ui.end_row();

                        ui.label("Ignore case:");
                        ui.checkbox(&mut self.highlight_dialog.current_rule.ignore_case, "");
                        ui.end_row();

                        ui.label("Foreground:");
                        ui.color_edit_button_srgba(&mut self.highlight_dialog.current_rule.foreground);
                        ui.end_row();

                        ui.label("Background:");
                        ui.color_edit_button_srgba(&mut self.highlight_dialog.current_rule.background);
                        ui.end_row();

                        ui.label("Bold:");
                        ui.checkbox(&mut self.highlight_dialog.current_rule.bold, "");
                        ui.end_row();

                        ui.label("Italic:");
                        ui.checkbox(&mut self.highlight_dialog.current_rule.italic, "");
                        ui.end_row();
                    });

                ui.add_space(10.0);

                // Preview
                ui.horizontal(|ui| {
                    ui.label("Preview:");
                    let preview_size = egui::vec2(150.0, 20.0);
                    let (rect, _) = ui.allocate_exact_size(preview_size, egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, self.highlight_dialog.current_rule.background);

                    let font_id = egui::FontId::monospace(13.0);
                    let sample_text = if self.highlight_dialog.current_rule.pattern.is_empty() {
                        "Sample Text"
                    } else {
                        &self.highlight_dialog.current_rule.pattern
                    };
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        sample_text,
                        font_id,
                        self.highlight_dialog.current_rule.foreground,
                    );
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if self.highlight_dialog.editing_index.is_some() {
                        if ui.button("Update").clicked() {
                            if let Some(idx) = self.highlight_dialog.editing_index {
                                if idx < self.highlight_rules.len() {
                                    self.highlight_rules[idx] = self.highlight_dialog.current_rule.clone();
                                }
                            }
                            self.highlight_dialog.editing_index = None;
                            self.highlight_dialog.current_rule = HighlightRule::default();
                        }
                        if ui.button("Cancel Edit").clicked() {
                            self.highlight_dialog.editing_index = None;
                            self.highlight_dialog.current_rule = HighlightRule::default();
                        }
                    } else {
                        if ui.button("Add Rule").clicked() {
                            if !self.highlight_dialog.current_rule.pattern.is_empty() {
                                let new_rule = self.highlight_dialog.current_rule.clone();
                                self.highlight_rules.push(new_rule);
                                self.highlight_dialog.current_rule = HighlightRule::default();
                            }
                        }
                    }
                });

                ui.separator();
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close_dialog = true;
                    }
                });
            });

        // Handle deferred actions
        if let Some(idx) = delete_rule {
            if idx < self.highlight_rules.len() {
                self.highlight_rules.remove(idx);
            }
        }

        if let Some(idx) = edit_rule {
            if idx < self.highlight_rules.len() {
                self.highlight_dialog.editing_index = Some(idx);
                self.highlight_dialog.current_rule = self.highlight_rules[idx].clone();
            }
        }

        if close_dialog {
            self.highlight_dialog.open = false;
        }
    }

    fn get_highlight_for_line(&self, text: &str) -> Option<&HighlightRule> {
        find_matching_highlight(&self.highlight_rules, text)
    }

    fn update_filter(&mut self) {
        self.filter_engine.set_live_filter(
            if self.filter_text.is_empty() {
                None
            } else {
                Some(self.filter_text.clone())
            }
        );

        // Check if the pattern is valid regex
        if !self.filter_text.is_empty() {
            match Regex::new(&self.filter_text) {
                Ok(_) => self.filter_error = None,
                Err(e) => self.filter_error = Some(format!("Invalid regex: {}", e)),
            }
        } else {
            self.filter_error = None;
        }
    }

    fn apply_preset(&mut self, preset_idx: usize) {
        if let Some(preset) = self.filter_presets.get(preset_idx) {
            self.filter_engine.clear_rules();
            for rule in &preset.rules {
                self.filter_engine.add_include_rule(rule.clone());
            }
            self.selected_preset = Some(preset_idx);
        }
    }

    fn add_filter_rule(&mut self) {
        let rule = match self.builder_rule_type {
            0 => FilterRule::contains(&self.builder_pattern),
            1 => FilterRule::regex(&self.builder_pattern),
            2 => {
                if let Some(level) = LogLevel::from_str(&self.builder_pattern) {
                    FilterRule::min_level(level)
                } else {
                    return;
                }
            }
            3 => FilterRule::field(&self.builder_field_name, &self.builder_pattern),
            _ => return,
        };

        if self.builder_is_exclude {
            self.filter_engine.add_exclude_rule(rule);
        } else {
            self.filter_engine.add_include_rule(rule);
        }

        // Clear builder state
        self.builder_pattern.clear();
        self.builder_field_name.clear();
    }

    fn line_matches_filter(&self, line: &DisplayLine) -> bool {
        // Check level filter
        let level_ok = match line.entry.level {
            Some(LogLevel::Trace) => self.show_trace,
            Some(LogLevel::Debug) => self.show_debug,
            Some(LogLevel::Info) => self.show_info,
            Some(LogLevel::Warn) => self.show_warning,
            Some(LogLevel::Error) => self.show_error,
            Some(LogLevel::Fatal) => self.show_fatal,
            None => true,
        };

        if !level_ok {
            return false;
        }

        // Check filter engine
        self.filter_engine.matches(&line.entry)
    }

    fn process_events(&mut self) {
        self.new_lines_received = false;

        if let Some(ref mut rx) = self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    SourceEvent::Line { source, line } => {
                        // Parse the line using auto-detection or plain parser
                        let entry = if self.use_auto_parser {
                            let parser = auto_detect_parser(&line);
                            parser.parse(&source, &line)
                        } else {
                            self.plain_parser.parse(&source, &line)
                        };

                        self.log_state.lock().unwrap().add_entry(entry);
                        self.new_lines_received = true;
                    }
                    SourceEvent::StatusChange { source, info } => {
                        self.source_infos.insert(source, info);
                    }
                    SourceEvent::Error { source, error } => {
                        tracing::error!("Source {} error: {}", source, error);
                    }
                }
            }
        }
    }

    fn render_source_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sources");
        ui.separator();

        // Source list
        let source_names: Vec<String> = self.source_infos.keys().cloned().collect();
        let mut to_remove = Vec::new();
        let mut to_edit: Option<String> = None;
        let mut toggle_auto_open_local: Option<String> = None;
        let mut toggle_auto_open_ssh: Option<String> = None;

        for name in &source_names {
            if let Some(info) = self.source_infos.get(name) {
                ui.horizontal(|ui| {
                    // Auto-open star toggle
                    let is_auto_open = match info.source_type {
                        models::SourceType::Local => self.is_local_source_auto_open(&info.path),
                        models::SourceType::Ssh => self.is_ssh_source_auto_open(name),
                    };
                    let star = if is_auto_open { "★" } else { "☆" };
                    let star_color = if is_auto_open {
                        egui::Color32::from_rgb(255, 200, 50)
                    } else {
                        egui::Color32::from_rgb(100, 100, 100)
                    };
                    if ui.add(egui::Button::new(egui::RichText::new(star).color(star_color)).frame(false)).on_hover_text("Toggle auto-open on startup").clicked() {
                        match info.source_type {
                            models::SourceType::Local => toggle_auto_open_local = Some(info.path.clone()),
                            models::SourceType::Ssh => toggle_auto_open_ssh = Some(name.clone()),
                        }
                    }

                    // Status indicator
                    let status_color = match info.status {
                        SourceStatus::Connected => egui::Color32::from_rgb(50, 205, 50),
                        SourceStatus::Connecting => egui::Color32::from_rgb(255, 200, 0),
                        SourceStatus::Disconnected => egui::Color32::from_rgb(150, 150, 150),
                        SourceStatus::Error => egui::Color32::from_rgb(255, 50, 50),
                    };

                    ui.colored_label(status_color, info.status_symbol());
                    ui.label(&info.name);
                    ui.label(format!("({})", info.source_type));

                    // Edit button for SSH sources
                    if info.source_type == models::SourceType::Ssh {
                        if ui.small_button("Edit").clicked() {
                            to_edit = Some(name.clone());
                        }
                    }

                    if ui.small_button("x").clicked() {
                        to_remove.push(name.clone());
                    }
                });

                ui.label(
                    egui::RichText::new(format!("  {} lines", info.line_count))
                        .small()
                        .color(egui::Color32::from_rgb(150, 150, 150))
                );
            }
        }

        // Handle auto-open toggles
        if let Some(path) = toggle_auto_open_local {
            // Ensure the local source is saved first
            if self.find_saved_local_source(&path).is_none() {
                let name = PathBuf::from(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "local".to_string());
                self.update_or_add_local_source(SavedLocalSource {
                    name,
                    path: path.clone(),
                    auto_open: true,
                });
            } else {
                self.toggle_local_source_auto_open(&path);
            }
        }
        if let Some(name) = toggle_auto_open_ssh {
            self.toggle_ssh_source_auto_open(&name);
        }

        // Handle edit action
        if let Some(name) = to_edit {
            if let Some(saved) = self.find_saved_ssh_source(&name).cloned() {
                self.ssh_dialog.load_from_saved(&saved);
                self.ssh_dialog.open = true;
            }
        }

        for name in to_remove {
            self.remove_source(&name);
        }

        ui.separator();

        // Saved SSH sources section
        if !self.saved_ssh_sources.is_empty() {
            ui.label(egui::RichText::new("Saved SSH Sources").small().color(egui::Color32::from_rgb(150, 150, 150)));

            let saved_names: Vec<String> = self.saved_ssh_sources.iter().map(|s| s.name.clone()).collect();
            let active_names: std::collections::HashSet<String> = self.source_infos.keys().cloned().collect();

            let mut connect_source: Option<SavedSshSource> = None;
            let mut delete_source: Option<String> = None;
            let mut edit_saved: Option<SavedSshSource> = None;
            let mut toggle_saved_auto_open: Option<String> = None;

            for name in &saved_names {
                let is_active = active_names.contains(name);
                if !is_active {
                    ui.horizontal(|ui| {
                        // Auto-open star toggle for saved sources
                        let is_auto_open = self.is_ssh_source_auto_open(name);
                        let star = if is_auto_open { "★" } else { "☆" };
                        let star_color = if is_auto_open {
                            egui::Color32::from_rgb(255, 200, 50)
                        } else {
                            egui::Color32::from_rgb(100, 100, 100)
                        };
                        if ui.add(egui::Button::new(egui::RichText::new(star).color(star_color).small()).frame(false)).on_hover_text("Toggle auto-open on startup").clicked() {
                            toggle_saved_auto_open = Some(name.clone());
                        }

                        ui.label(
                            egui::RichText::new(name)
                                .small()
                                .color(egui::Color32::from_rgb(120, 120, 120))
                        );

                        if ui.small_button("Connect").clicked() {
                            if let Some(saved) = self.find_saved_ssh_source(name).cloned() {
                                connect_source = Some(saved);
                            }
                        }

                        if ui.small_button("Edit").clicked() {
                            if let Some(saved) = self.find_saved_ssh_source(name).cloned() {
                                edit_saved = Some(saved);
                            }
                        }

                        if ui.small_button("x").clicked() {
                            delete_source = Some(name.clone());
                        }
                    });
                }
            }

            // Handle toggle auto-open for saved sources
            if let Some(name) = toggle_saved_auto_open {
                self.toggle_ssh_source_auto_open(&name);
            }

            // Handle deferred actions
            if let Some(source) = connect_source {
                let key_path = source.key_path.as_ref().map(PathBuf::from);
                // Get password from keychain
                let password = get_ssh_password(&source.name);
                self.add_ssh_source(
                    source.name,
                    source.host,
                    source.port,
                    source.user,
                    source.remote_path,
                    key_path,
                    password,
                );
            }

            if let Some(source) = edit_saved {
                self.ssh_dialog.load_from_saved(&source);
                self.ssh_dialog.open = true;
            }

            if let Some(name) = delete_source {
                self.remove_saved_ssh_source(&name);
            }

            ui.separator();
        }

        // Add source buttons
        ui.horizontal(|ui| {
            if ui.button("+ Local File").clicked() {
                self.open_file_dialog();
            }

            if ui.button("+ SSH").clicked() {
                self.ssh_dialog.open = true;
                self.ssh_dialog.reset();
                self.ssh_dialog.port = "22".to_string();
            }
        });
    }

    fn render_ssh_dialog(&mut self, ctx: &egui::Context) {
        if !self.ssh_dialog.open {
            return;
        }

        let is_editing = self.ssh_dialog.editing.is_some();
        let title = if is_editing { "Edit SSH Source" } else { "Add SSH Source" };

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("ssh_dialog_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name:");
                        if is_editing {
                            // Name is read-only when editing
                            ui.label(&self.ssh_dialog.name);
                        } else {
                            ui.text_edit_singleline(&mut self.ssh_dialog.name);
                        }
                        ui.end_row();

                        ui.label("Host:");
                        ui.text_edit_singleline(&mut self.ssh_dialog.host);
                        ui.end_row();

                        ui.label("Port:");
                        ui.text_edit_singleline(&mut self.ssh_dialog.port);
                        ui.end_row();

                        ui.label("User:");
                        ui.text_edit_singleline(&mut self.ssh_dialog.user);
                        ui.end_row();

                        ui.label("Remote Path:");
                        ui.text_edit_singleline(&mut self.ssh_dialog.remote_path);
                        ui.end_row();

                        ui.label("Key Path (optional):");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.ssh_dialog.key_path);
                            if ui.button("Browse").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .set_directory(dirs::home_dir().unwrap_or_default().join(".ssh"))
                                    .pick_file()
                                {
                                    self.ssh_dialog.key_path = path.display().to_string();
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("Password (optional):");
                        ui.add(egui::TextEdit::singleline(&mut self.ssh_dialog.password).password(true));
                        ui.end_row();

                        ui.label("Auto-open on startup:");
                        ui.checkbox(&mut self.ssh_dialog.auto_open, "");
                        ui.end_row();
                    });

                ui.label(
                    egui::RichText::new("Note: Will try SSH keys first (id_ed25519, id_rsa), then password")
                        .small()
                        .color(egui::Color32::from_rgb(120, 120, 120))
                );
                ui.label(
                    egui::RichText::new("Password is stored securely in OS keychain")
                        .small()
                        .color(egui::Color32::from_rgb(100, 150, 100))
                );

                if let Some(ref err) = self.ssh_dialog.error {
                    ui.colored_label(egui::Color32::RED, err);
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.ssh_dialog.open = false;
                    }

                    // Save button (saves without connecting)
                    if ui.button("Save").clicked() {
                        if self.ssh_dialog.name.is_empty() {
                            self.ssh_dialog.error = Some("Name is required".to_string());
                        } else if self.ssh_dialog.host.is_empty() {
                            self.ssh_dialog.error = Some("Host is required".to_string());
                        } else if self.ssh_dialog.user.is_empty() {
                            self.ssh_dialog.error = Some("User is required".to_string());
                        } else if self.ssh_dialog.remote_path.is_empty() {
                            self.ssh_dialog.error = Some("Remote path is required".to_string());
                        } else {
                            let saved = self.ssh_dialog.to_saved();
                            let password = if self.ssh_dialog.password.is_empty() {
                                None
                            } else {
                                Some(self.ssh_dialog.password.clone())
                            };
                            self.update_or_add_ssh_source(saved, password);
                            self.ssh_dialog.open = false;
                        }
                    }

                    // Connect button (saves and connects)
                    let connect_label = if is_editing { "Save & Reconnect" } else { "Save & Connect" };
                    if ui.button(connect_label).clicked() {
                        // Validate inputs
                        if self.ssh_dialog.name.is_empty() {
                            self.ssh_dialog.error = Some("Name is required".to_string());
                        } else if self.ssh_dialog.host.is_empty() {
                            self.ssh_dialog.error = Some("Host is required".to_string());
                        } else if self.ssh_dialog.user.is_empty() {
                            self.ssh_dialog.error = Some("User is required".to_string());
                        } else if self.ssh_dialog.remote_path.is_empty() {
                            self.ssh_dialog.error = Some("Remote path is required".to_string());
                        } else {
                            let port: u16 = self.ssh_dialog.port.parse().unwrap_or(22);
                            let key_path = if self.ssh_dialog.key_path.is_empty() {
                                None
                            } else {
                                Some(PathBuf::from(&self.ssh_dialog.key_path))
                            };

                            // Get password before saving
                            let password = if self.ssh_dialog.password.is_empty() {
                                None
                            } else {
                                Some(self.ssh_dialog.password.clone())
                            };

                            // Save to JSON and keychain
                            let saved = self.ssh_dialog.to_saved();
                            self.update_or_add_ssh_source(saved, password.clone());

                            // If editing, stop the existing source first
                            if is_editing {
                                let name_to_remove = self.ssh_dialog.name.clone();
                                self.remove_source(&name_to_remove);
                            }

                            // Connect (use the password we already extracted above)
                            self.add_ssh_source(
                                self.ssh_dialog.name.clone(),
                                self.ssh_dialog.host.clone(),
                                port,
                                self.ssh_dialog.user.clone(),
                                self.ssh_dialog.remote_path.clone(),
                                key_path,
                                password,
                            );

                            self.ssh_dialog.open = false;
                        }
                    }
                });

                // Show file path where sources are saved
                ui.add_space(5.0);
                ui.label(
                    egui::RichText::new(format!("Saved to: {}", self.ssh_sources_path.display()))
                        .small()
                        .color(egui::Color32::from_rgb(100, 100, 100))
                );
            });
    }

    fn render_about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about_dialog {
            return;
        }

        egui::Window::new("About Oxitailr")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    ui.heading("Oxitailr");
                    ui.label("A modern log viewer");
                    ui.label("Developed by Marcy Kuhn");
                    ui.add_space(10.0);
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.label("Features:");
                    ui.label("• Multi-source log viewing (local & SSH)");
                    ui.label("• JSON log auto-parsing");
                    ui.label("• Advanced filtering with presets");
                    ui.label("• ANSI color support");
                    ui.label("• Real-time log tailing");
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.label("Built with Rust + egui");
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.show_about_dialog = false;
                    }
                    ui.add_space(10.0);
                });
            });
    }

    fn render_filter_builder(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Type:");
            egui::ComboBox::from_id_salt("rule_type")
                .selected_text(match self.builder_rule_type {
                    0 => "Contains",
                    1 => "Regex",
                    2 => "Min Level",
                    3 => "Field",
                    _ => "Unknown",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.builder_rule_type, 0, "Contains");
                    ui.selectable_value(&mut self.builder_rule_type, 1, "Regex");
                    ui.selectable_value(&mut self.builder_rule_type, 2, "Min Level");
                    ui.selectable_value(&mut self.builder_rule_type, 3, "Field");
                });
        });

        if self.builder_rule_type == 3 {
            ui.horizontal(|ui| {
                ui.label("Field name:");
                ui.text_edit_singleline(&mut self.builder_field_name);
            });
        }

        ui.horizontal(|ui| {
            ui.label(if self.builder_rule_type == 2 { "Level:" } else { "Pattern:" });
            if self.builder_rule_type == 2 {
                egui::ComboBox::from_id_salt("level_select")
                    .selected_text(&self.builder_pattern)
                    .show_ui(ui, |ui| {
                        for level in ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"] {
                            ui.selectable_value(&mut self.builder_pattern, level.to_string(), level);
                        }
                    });
            } else {
                ui.text_edit_singleline(&mut self.builder_pattern);
            }
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.builder_is_exclude, "Exclude");
            if ui.button("Add Rule").clicked() {
                self.add_filter_rule();
            }
            if ui.button("Clear All").clicked() {
                self.filter_engine.clear_rules();
                self.selected_preset = None;
            }
        });
    }
}

// Standalone helper function to avoid borrow conflicts
fn find_matching_highlight<'a>(rules: &'a [HighlightRule], text: &str) -> Option<&'a HighlightRule> {
    for rule in rules {
        if rule.matches(text) {
            return Some(rule);
        }
    }
    None
}

impl eframe::App for TailLoggerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process events from source manager
        self.process_events();

        // Request repaint for live updates (use configured interval)
        ctx.request_repaint_after(std::time::Duration::from_millis(self.update_interval_ms));

        // Handle always-on-top
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            if self.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }
        ));

        // Render dialogs
        self.render_ssh_dialog(ctx);
        self.render_about_dialog(ctx);
        self.render_settings_dialog(ctx);
        self.render_highlight_dialog(ctx);

        // Top panel with controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Hamburger menu
                ui.menu_button("☰", |ui| {
                    ui.set_min_width(150.0);

                    if ui.button("📂  Open File...").clicked() {
                        self.open_file_dialog();
                        ui.close_menu();
                    }

                    if ui.button("🔗  Add SSH Source...").clicked() {
                        self.ssh_dialog.open = true;
                        self.ssh_dialog.reset();
                        self.ssh_dialog.port = "22".to_string();
                        ui.close_menu();
                    }

                    ui.separator();

                    if ui.button("⚙  Settings").clicked() {
                        self.open_settings_dialog();
                        ui.close_menu();
                    }

                    ui.separator();

                    if ui.button("❓  Help").clicked() {
                        // TODO: Open help documentation
                        ui.close_menu();
                    }

                    if ui.button("ℹ  About").clicked() {
                        self.show_about_dialog = true;
                        ui.close_menu();
                    }
                });

                ui.separator();

                let state = self.log_state.lock().unwrap();
                ui.label(format!("Lines: {}", state.lines.len()));
                ui.label(format!("Total: {}", state.total_lines_read));
                drop(state);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear").clicked() {
                        let mut state = self.log_state.lock().unwrap();
                        state.lines.clear();
                        state.total_lines_read = 0;
                        drop(state);
                        self.initial_scroll_pending = true;
                    }

                    if ui.button("Reload").clicked() {
                        self.reload_sources();
                    }

                    ui.checkbox(&mut self.show_source_panel, "Sources");
                });
            });

            ui.horizontal(|ui| {
                ui.label("Filter:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.filter_text)
                        .hint_text("Regex filter...")
                        .desired_width(200.0),
                );
                if response.changed() {
                    self.update_filter();
                }

                if let Some(ref err) = self.filter_error {
                    ui.colored_label(egui::Color32::RED, err);
                }

                ui.separator();

                // Filter presets dropdown
                if !self.filter_presets.is_empty() {
                    let preset_text = self.selected_preset
                        .map(|i| self.filter_presets.get(i).map(|p| p.name.as_str()).unwrap_or(""))
                        .unwrap_or("Preset...");

                    let mut preset_to_apply: Option<usize> = None;
                    let mut clear_preset = false;

                    egui::ComboBox::from_id_salt("filter_preset")
                        .selected_text(preset_text)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.selected_preset.is_none(), "None").clicked() {
                                clear_preset = true;
                            }
                            for (i, preset) in self.filter_presets.iter().enumerate() {
                                if ui.selectable_label(self.selected_preset == Some(i), &preset.name).clicked() {
                                    preset_to_apply = Some(i);
                                }
                            }
                        });

                    if clear_preset {
                        self.filter_engine.clear_rules();
                        self.selected_preset = None;
                    } else if let Some(idx) = preset_to_apply {
                        self.apply_preset(idx);
                    }
                }

                if ui.button("Advanced").clicked() {
                    self.show_filter_builder = !self.show_filter_builder;
                }

                ui.separator();

                ui.label("Levels:");
                ui.checkbox(&mut self.show_trace, "Trace");
                ui.checkbox(&mut self.show_debug, "Debug");
                ui.checkbox(&mut self.show_info, "Info");
                ui.checkbox(&mut self.show_warning, "Warn");
                ui.checkbox(&mut self.show_error, "Error");
                ui.checkbox(&mut self.show_fatal, "Fatal");
            });

            // Advanced filter builder
            if self.show_filter_builder {
                ui.separator();
                self.render_filter_builder(ui);
            }

            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.search_text)
                        .hint_text("Highlight text...")
                        .desired_width(200.0),
                );
            });
        });

        // Bottom status bar - must be before CentralPanel
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let state = self.log_state.lock().unwrap();
                let filtered_count = state
                    .lines
                    .iter()
                    .filter(|line| self.line_matches_filter(line))
                    .count();
                ui.label(format!(
                    "Showing {} of {} lines",
                    filtered_count,
                    state.lines.len()
                ));

                // Show active sources count
                let connected_count = self.source_infos
                    .values()
                    .filter(|i| i.status == SourceStatus::Connected)
                    .count();
                ui.separator();
                ui.label(format!("Sources: {} connected", connected_count));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("Scroll: Mouse wheel | Home/End | Page Up/Down");
                });
            });
        });

        // Source panel (left side)
        if self.show_source_panel {
            egui::SidePanel::left("source_panel")
                .default_width(200.0)
                .show(ctx, |ui| {
                    self.render_source_panel(ui);
                });
        }

        // Main log view
        egui::CentralPanel::default().show(ctx, |ui| {
            // Show welcome screen if no sources
            if self.source_infos.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.heading("Welcome to Oxitailr");
                    ui.add_space(20.0);
                    ui.label("Add a log source to get started");
                    ui.add_space(20.0);
                    ui.horizontal(|ui| {
                        if ui.button("📂 Open Local File").clicked() {
                            self.open_file_dialog();
                        }
                        if ui.button("🔗 Add SSH Source").clicked() {
                            self.ssh_dialog.open = true;
                            self.ssh_dialog.reset();
                            self.ssh_dialog.port = "22".to_string();
                        }
                    });
                    ui.add_space(40.0);
                    ui.label("Or run from command line:");
                    ui.code("oxitailr /path/to/file.log");
                    ui.add_space(20.0);
                    ui.label("Config file location:");
                    ui.code(self.config_path.display().to_string());
                });
                return;
            }

            // Source tabs
            ui.horizontal(|ui| {
                // "All" tab
                let all_selected = self.selected_source.is_none();
                if ui.selectable_label(all_selected, "All").clicked() {
                    self.selected_source = None;
                }

                ui.separator();

                // Individual source tabs
                let source_names: Vec<String> = self.source_infos.keys().cloned().collect();
                for name in &source_names {
                    let is_selected = self.selected_source.as_ref() == Some(name);
                    let info = self.source_infos.get(name);

                    // Show status indicator in tab
                    let status_symbol = info.map(|i| i.status_symbol()).unwrap_or("?");
                    let tab_text = format!("{} {}", status_symbol, name);

                    if ui.selectable_label(is_selected, tab_text).clicked() {
                        self.selected_source = Some(name.clone());
                    }
                }
            });

            ui.separator();

            let text_style = egui::TextStyle::Monospace;
            let row_height = ui.text_style_height(&text_style) * (self.font_size / 13.0) * self.line_spacing;

            // Extract values needed in closures to avoid borrow conflicts
            let font_size = self.font_size;
            let line_spacing = self.line_spacing;
            let show_source = self.show_source;
            let show_timestamps = self.show_timestamps;
            let source_count = self.source_infos.len();
            let highlight_rules = self.highlight_rules.clone();

            let state = self.log_state.lock().unwrap();
            let filtered_lines: Vec<&DisplayLine> = state
                .lines
                .iter()
                .filter(|line| {
                    // Filter by selected source
                    if let Some(ref selected) = self.selected_source {
                        if &line.entry.source != selected {
                            return false;
                        }
                    }
                    // Apply other filters
                    self.line_matches_filter(line)
                })
                .collect();

            let total_rows = filtered_lines.len();
            let search_lower = self.search_text.to_lowercase();
            let mut local_scroll_row = self.current_scroll_row;

            // Calculate visible rows for page up/down
            let visible_rows = (ui.available_height() / row_height) as usize;

            // Handle keyboard input
            let mut scroll_request: Option<usize> = None;
            ctx.input(|i| {
                if i.key_pressed(egui::Key::PageDown) {
                    let new_row = (self.current_scroll_row + visible_rows).min(total_rows.saturating_sub(1));
                    scroll_request = Some(new_row);
                    self.auto_scroll = false;
                }
                if i.key_pressed(egui::Key::PageUp) {
                    let new_row = self.current_scroll_row.saturating_sub(visible_rows);
                    scroll_request = Some(new_row);
                    self.auto_scroll = false;
                }
                if i.key_pressed(egui::Key::Home) {
                    scroll_request = Some(0);
                    self.auto_scroll = false;
                }
                if i.key_pressed(egui::Key::End) {
                    scroll_request = Some(total_rows.saturating_sub(1));
                    self.auto_scroll = true;
                }
            });

            if let Some(row) = scroll_request {
                self.scroll_to_row = Some(row);
            }

            // Handle initial scroll to bottom
            if self.initial_scroll_pending && total_rows > 0 {
                self.scroll_to_row = Some(total_rows.saturating_sub(1));
                self.initial_scroll_pending = false;
            }

            // Only stick to bottom when new lines arrive and auto-scroll is on
            let should_stick = self.auto_scroll && self.new_lines_received;

            let mut scroll_area = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(should_stick);

            // Handle scroll to specific row
            if let Some(row) = self.scroll_to_row.take() {
                scroll_area = scroll_area.vertical_scroll_offset(row as f32 * row_height);
            }

            // Render log lines - use different approach based on wrap setting
            if self.wrap_lines {
                // Wrapped mode: can't use show_rows with fixed heights
                scroll_area.show(ui, |ui| {
                    // Apply line spacing
                    ui.spacing_mut().item_spacing.y = 4.0 * line_spacing;

                    for (row, line) in filtered_lines.iter().enumerate() {
                        if row == 0 {
                            local_scroll_row = 0;
                        }

                        ui.horizontal_wrapped(|ui| {
                            // Line number
                            ui.label(
                                egui::RichText::new(format!("{:6} ", line.line_num))
                                    .monospace()
                                    .size(font_size)
                                    .color(egui::Color32::from_rgb(100, 100, 100)),
                            );

                            // Source indicator
                            if show_source && source_count > 1 {
                                let source_color = egui::Color32::from_rgb(100, 150, 200);
                                ui.label(
                                    egui::RichText::new(format!("[{}] ", line.entry.source))
                                        .monospace()
                                        .size(font_size)
                                        .color(source_color),
                                );
                            }

                            // Timestamp
                            if show_timestamps {
                                if let Some(ts) = &line.entry.timestamp {
                                    ui.label(
                                        egui::RichText::new(format!("{} ", ts.format("%H:%M:%S%.3f")))
                                            .monospace()
                                            .size(font_size)
                                            .color(egui::Color32::from_rgb(120, 120, 120)),
                                    );
                                }
                            }

                            // Level badge
                            if let Some(level) = &line.entry.level {
                                let level_color = log_level_color(Some(level));
                                ui.label(
                                    egui::RichText::new(format!("[{:5}] ", level.as_str()))
                                        .monospace()
                                        .size(font_size)
                                        .color(level_color),
                                );
                            }

                            // Render message with wrapping
                            let has_search_match = !search_lower.is_empty()
                                && line.entry.message.to_lowercase().contains(&search_lower);

                            // Check for highlight rule match
                            let highlight = find_matching_highlight(&highlight_rules, &line.entry.raw);

                            let (fg_color, bg_color, is_bold, is_italic) = if let Some(rule) = highlight {
                                (rule.foreground, Some(rule.background), rule.bold, rule.italic)
                            } else {
                                (log_level_color(line.entry.level.as_ref()), None, false, false)
                            };

                            let mut text = egui::RichText::new(&line.entry.message)
                                .monospace()
                                .size(font_size)
                                .color(fg_color);

                            if is_bold {
                                text = text.strong();
                            }
                            if is_italic {
                                text = text.italics();
                            }
                            if let Some(bg) = bg_color {
                                text = text.background_color(bg);
                            }
                            if has_search_match && bg_color.is_none() {
                                text = text.background_color(egui::Color32::from_rgb(100, 100, 0));
                            }

                            ui.label(text);

                            // Show JSON fields indicator
                            if !line.entry.fields.is_empty() {
                                ui.label(
                                    egui::RichText::new(" {...}")
                                        .monospace()
                                        .size(font_size * 0.9)
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                ).on_hover_ui(|ui| {
                                    ui.label("JSON Fields:");
                                    for (key, value) in &line.entry.fields {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{}:", key))
                                                    .color(egui::Color32::from_rgb(150, 200, 150))
                                            );
                                            ui.label(format!("{}", value));
                                        });
                                    }
                                });
                            }
                        });
                    }
                });
            } else {
                // Non-wrapped mode: use show_rows for performance
                scroll_area.show_rows(ui, row_height, total_rows, |ui, row_range| {
                    local_scroll_row = row_range.start;

                    for row in row_range {
                        if let Some(line) = filtered_lines.get(row) {
                            ui.horizontal(|ui| {
                                // Line number
                                ui.label(
                                    egui::RichText::new(format!("{:6} ", line.line_num))
                                        .monospace()
                                        .size(font_size)
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                );

                                // Source indicator
                                if show_source && source_count > 1 {
                                    let source_color = egui::Color32::from_rgb(100, 150, 200);
                                    ui.label(
                                        egui::RichText::new(format!("[{}] ", line.entry.source))
                                            .monospace()
                                            .size(font_size)
                                            .color(source_color),
                                    );
                                }

                                // Timestamp
                                if show_timestamps {
                                    if let Some(ts) = &line.entry.timestamp {
                                        ui.label(
                                            egui::RichText::new(format!("{} ", ts.format("%H:%M:%S%.3f")))
                                                .monospace()
                                                .size(font_size)
                                                .color(egui::Color32::from_rgb(120, 120, 120)),
                                        );
                                    }
                                }

                                // Level badge
                                if let Some(level) = &line.entry.level {
                                    let level_color = log_level_color(Some(level));
                                    ui.label(
                                        egui::RichText::new(format!("[{:5}] ", level.as_str()))
                                            .monospace()
                                            .size(font_size)
                                            .color(level_color),
                                    );
                                }

                                // Render message
                                let has_search_match = !search_lower.is_empty()
                                    && line.entry.message.to_lowercase().contains(&search_lower);

                                let display_text = &line.entry.message;

                                // Check for highlight rule match
                                let highlight = find_matching_highlight(&highlight_rules, &line.entry.raw);

                                if let Some(rule) = highlight {
                                    // Apply highlight rule styling
                                    let mut text = egui::RichText::new(display_text)
                                        .monospace()
                                        .size(font_size)
                                        .color(rule.foreground)
                                        .background_color(rule.background);

                                    if rule.bold {
                                        text = text.strong();
                                    }
                                    if rule.italic {
                                        text = text.italics();
                                    }

                                    ui.label(text);
                                } else if line.has_ansi {
                                    // Render ANSI colored spans
                                    for span in &line.spans {
                                        let mut text = egui::RichText::new(&span.text)
                                            .monospace()
                                            .size(font_size)
                                            .color(span.color);

                                        if span.bold {
                                            text = text.strong();
                                        }

                                        if has_search_match && span.text.to_lowercase().contains(&search_lower) {
                                            text = text.background_color(egui::Color32::from_rgb(100, 100, 0));
                                        }

                                        ui.label(text);
                                    }
                                } else {
                                    // Render with level-based color
                                    let color = log_level_color(line.entry.level.as_ref());
                                    let mut text = egui::RichText::new(display_text)
                                        .monospace()
                                        .size(font_size)
                                        .color(color);

                                    if has_search_match {
                                        text = text.background_color(egui::Color32::from_rgb(100, 100, 0));
                                    }

                                    ui.label(text);
                                }

                                // Show JSON fields on hover if present
                                if !line.entry.fields.is_empty() {
                                    ui.label(
                                        egui::RichText::new(" {...}")
                                            .monospace()
                                            .size(font_size * 0.9)
                                            .color(egui::Color32::from_rgb(100, 100, 100)),
                                    ).on_hover_ui(|ui| {
                                        ui.label("JSON Fields:");
                                        for (key, value) in &line.entry.fields {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(format!("{}:", key))
                                                        .color(egui::Color32::from_rgb(150, 200, 150))
                                                );
                                                ui.label(format!("{}", value));
                                            });
                                        }
                                    });
                                }
                            });
                        }
                    }
                });
            }

            // Update current scroll row from local variable
            self.current_scroll_row = local_scroll_row;
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save session on exit
        self.save_session();

        // Stop all sources cleanly
        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();
        let source_names: Vec<String> = self.source_infos.keys().cloned().collect();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            for name in &source_names {
                let _ = sm.stop_source(name).await;
            }
        });

        tracing::info!("Application exiting, session saved");
    }
}

fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Load config
    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let mut config = load_or_default_config(Some(config_path.clone()));

    // Override buffer size from CLI if provided
    if let Some(max_lines) = cli.max_lines {
        config.general.buffer_size = max_lines;
    }

    // Validate file if provided
    if let Some(ref file) = cli.file {
        if !file.exists() {
            anyhow::bail!("File not found: {}", file.display());
        }
    }

    // Create tokio runtime for async operations
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
    );

    let title = match &cli.file {
        Some(path) => format!("Oxitailr - {}", path.display()),
        None => "Oxitailr".to_string(),
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 800.0])
        .with_title(title)
        .with_app_id("oxitailr");

    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let file_path = cli.file.clone();

    eframe::run_native(
        "Oxitailr",
        options,
        Box::new(move |cc| {
            Ok(Box::new(TailLoggerApp::new(cc, config, config_path, file_path, runtime)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run GUI: {}", e))?;

    Ok(())
}
