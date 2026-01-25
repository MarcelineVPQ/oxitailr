// Prevent terminal window from appearing on Windows
#![windows_subsystem = "windows"]

mod alert;
mod config;
mod credentials;
mod filter;
mod models;
mod parser;
mod source;
mod state;
mod ui;

use anyhow::Result;
use clap::Parser as ClapParser;
use config::{AppConfig, SourceConfig};
use credentials::{delete_ssh_password, get_ssh_password, store_ssh_password};
use eframe::egui;
use egui::IconData;
use filter::{FilterEngine, FilterRule};
use models::{LogEntry, LogLevel, SourceInfo, SourceStatus};
use parser::{auto_detect_parser, Parser, PlainParser};
use regex::Regex;
use source::{SourceEvent, SourceManager};
use state::{load_local_sources, load_session, save_local_sources, save_session, SavedLocalSource};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use ui::{
    default_highlight_rules, find_matching_highlight, format_file_size, log_level_color,
    render_highlight_dialog, render_settings_dialog, render_ssh_dialog, DisplayLine,
    HighlightDialogState, HighlightRule, SavedSshSource, SettingsDialogResult,
    SettingsDialogState, SshDialogResult, SshDialogState,
};

// Embed the icon at compile time
const ICON_BYTES: &[u8] = include_bytes!("../assets/icon_64x64.png");

fn load_icon() -> Option<IconData> {
    let image = image::load_from_memory(ICON_BYTES).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// Application theme
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Light => "Light",
            Theme::Dark => "Dark",
            Theme::System => "System",
        }
    }
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
    selected_source: Option<String>,

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
    theme: Theme,

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

    // Dialogs
    ssh_dialog: SshDialogState,
    settings_dialog: SettingsDialogState,
    show_source_panel: bool,
    show_about_dialog: bool,
    show_help_dialog: bool,
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
            .map(|(name, fc)| FilterPreset {
                name: name.clone(),
                rules: fc.rules.clone(),
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
            use_auto_parser: config.general.auto_parse_json,
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
            font_size: config.general.font_size,
            new_lines_received: false,
            scroll_to_row: None,
            current_scroll_row: 0,
            initial_scroll_pending: true,
            show_timestamps: config.general.show_timestamps,
            show_source: config.general.show_source,
            wrap_lines: config.general.wrap_lines,
            line_spacing: config.general.line_spacing,
            tab_width: config.general.tab_width,
            update_interval_ms: config.general.update_interval_ms,
            theme: config.general.theme,
            highlight_rules: default_highlight_rules(),
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
            ssh_dialog: SshDialogState::default(),
            settings_dialog: SettingsDialogState::default(),
            show_source_panel: true,
            show_about_dialog: false,
            show_help_dialog: false,
        };

        // Load saved sources
        app.load_ssh_sources();
        app.saved_local_sources = load_local_sources(&app.local_sources_path);

        // Add initial file if provided via CLI
        if let Some(path) = initial_file {
            app.add_local_source_from_path(path.clone());
            app.update_or_add_local_source(SavedLocalSource {
                name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "local".to_string()),
                path: path.display().to_string(),
                auto_open: false,
            });
        } else if config.general.remember_last_session {
            // Try to restore last session
            app.restore_session();
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
                Ok(content) => match serde_json::from_str::<Vec<SavedSshSource>>(&content) {
                    Ok(sources) => {
                        self.saved_ssh_sources = sources;
                        tracing::info!(
                            "Loaded {} SSH sources from {}",
                            self.saved_ssh_sources.len(),
                            self.ssh_sources_path.display()
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse SSH sources: {}", e);
                    }
                },
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
                    tracing::info!(
                        "Saved {} SSH sources to {}",
                        self.saved_ssh_sources.len(),
                        self.ssh_sources_path.display()
                    );
                }
            }
            Err(e) => {
                tracing::error!("Failed to serialize SSH sources: {}", e);
            }
        }
    }

    fn update_or_add_local_source(&mut self, source: SavedLocalSource) {
        if let Some(existing) = self
            .saved_local_sources
            .iter_mut()
            .find(|s| s.path == source.path)
        {
            *existing = source;
        } else {
            self.saved_local_sources.push(source);
        }
        save_local_sources(&self.local_sources_path, &self.saved_local_sources);
    }

    fn find_saved_local_source(&self, path: &str) -> Option<&SavedLocalSource> {
        self.saved_local_sources.iter().find(|s| s.path == path)
    }

    fn restore_session(&mut self) {
        if let Some(session) = load_session(&self.session_path) {
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
    }

    fn persist_session(&self) {
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

        save_session(&self.session_path, open_local_files, open_ssh_sources);
    }

    fn open_auto_open_sources(&mut self) {
        // Open local sources with auto_open
        let local_sources: Vec<SavedLocalSource> = self
            .saved_local_sources
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
        let ssh_sources: Vec<SavedSshSource> = self
            .saved_ssh_sources
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
        if let Some(source) = self
            .saved_local_sources
            .iter_mut()
            .find(|s| s.path == path)
        {
            source.auto_open = !source.auto_open;
            save_local_sources(&self.local_sources_path, &self.saved_local_sources);
        }
    }

    fn toggle_ssh_source_auto_open(&mut self, name: &str) {
        if let Some(source) = self
            .saved_ssh_sources
            .iter_mut()
            .find(|s| s.name == name)
        {
            source.auto_open = !source.auto_open;
            self.save_ssh_sources();
        }
    }

    fn is_local_source_auto_open(&self, path: &str) -> bool {
        self.saved_local_sources
            .iter()
            .any(|s| s.path == path && s.auto_open)
    }

    fn is_ssh_source_auto_open(&self, name: &str) -> bool {
        self.saved_ssh_sources
            .iter()
            .any(|s| s.name == name && s.auto_open)
    }

    fn find_saved_ssh_source(&self, name: &str) -> Option<&SavedSshSource> {
        self.saved_ssh_sources.iter().find(|s| s.name == name)
    }

    fn update_or_add_ssh_source(&mut self, source: SavedSshSource, password: Option<String>) {
        // Store password in keychain if provided
        if let Some(ref pwd) = password {
            if !pwd.is_empty() {
                if let Err(e) = store_ssh_password(&source.name, pwd) {
                    tracing::warn!("Failed to store password: {}", e);
                }
            }
        }

        if let Some(existing) = self
            .saved_ssh_sources
            .iter_mut()
            .find(|s| s.name == source.name)
        {
            *existing = source;
        } else {
            self.saved_ssh_sources.push(source);
        }
        self.save_ssh_sources();
    }

    fn remove_saved_ssh_source(&mut self, name: &str) {
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

    #[allow(clippy::too_many_arguments)]
    fn add_ssh_source(
        &mut self,
        name: String,
        host: String,
        port: u16,
        user: String,
        path: String,
        key_path: Option<PathBuf>,
        password: Option<String>,
    ) {
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
        if let Ok(mut state) = self.log_state.lock() {
            state.lines.clear();
            state.total_lines_read = 0;
        }

        self.initial_scroll_pending = true;

        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();
        let source_names: Vec<String> = self.source_infos.keys().cloned().collect();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            for name in &source_names {
                let _ = sm.stop_source(name).await;
            }
            for name in &source_names {
                let _ = sm.start_source(name).await;
            }
        });
    }

    fn open_settings_dialog(&mut self) {
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
        self.settings_dialog.theme = self.theme;
        self.settings_dialog.remember_last_session = self.config.general.remember_last_session;
        self.settings_dialog.open = true;
    }

    fn apply_settings_from_dialog(&mut self) {
        self.font_size = self.settings_dialog.font_size;
        self.show_timestamps = self.settings_dialog.show_timestamps;
        self.show_source = self.settings_dialog.show_source;
        self.wrap_lines = self.settings_dialog.wrap_lines;
        self.auto_scroll = self.settings_dialog.auto_scroll;
        self.use_auto_parser = self.settings_dialog.use_auto_parser;
        self.line_spacing = self.settings_dialog.line_spacing;
        self.theme = self.settings_dialog.theme;

        if let Ok(tab) = self.settings_dialog.tab_width.parse::<usize>() {
            self.tab_width = tab.clamp(1, 16);
        }

        if let Ok(interval) = self.settings_dialog.update_interval_ms.parse::<u64>() {
            self.update_interval_ms = interval.clamp(10, 5000);
        }

        if let Ok(size) = self.settings_dialog.buffer_size.parse::<usize>() {
            self.config.general.buffer_size = size;
            if let Ok(mut state) = self.log_state.lock() {
                state.max_lines = size;
            }
        }

        self.config.general.show_timestamps = self.show_timestamps;
        self.config.general.show_source = self.show_source;
        self.config.general.wrap_lines = self.wrap_lines;
        self.config.general.follow = self.auto_scroll;
        self.config.general.remember_last_session = self.settings_dialog.remember_last_session;
        self.config.general.theme = self.theme;
        self.config.general.font_size = self.font_size;
        self.config.general.line_spacing = self.line_spacing;
        self.config.general.tab_width = self.tab_width;
        self.config.general.update_interval_ms = self.update_interval_ms;
        self.config.general.auto_parse_json = self.use_auto_parser;
    }

    fn update_filter(&mut self) {
        self.filter_engine.set_live_filter(if self.filter_text.is_empty() {
            None
        } else {
            Some(self.filter_text.clone())
        });

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

        self.builder_pattern.clear();
        self.builder_field_name.clear();
    }

    fn line_matches_filter(&self, line: &DisplayLine) -> bool {
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

        self.filter_engine.matches(&line.entry)
    }

    fn process_events(&mut self) {
        self.new_lines_received = false;

        if let Some(ref mut rx) = self.event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    SourceEvent::Line { source, line } => {
                        let entry = if self.use_auto_parser {
                            let parser = auto_detect_parser(&line);
                            parser.parse(&source, &line)
                        } else {
                            self.plain_parser.parse(&source, &line)
                        };

                        if let Ok(mut state) = self.log_state.lock() {
                            state.add_entry(entry);
                        }
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

        let source_names: Vec<String> = self.source_infos.keys().cloned().collect();
        let mut to_remove = Vec::new();
        let mut to_edit: Option<String> = None;
        let mut toggle_auto_open_local: Option<String> = None;
        let mut toggle_auto_open_ssh: Option<String> = None;

        for name in &source_names {
            if let Some(info) = self.source_infos.get(name) {
                ui.horizontal(|ui| {
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
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(star).color(star_color))
                                .frame(false),
                        )
                        .on_hover_text("Toggle auto-open on startup")
                        .clicked()
                    {
                        match info.source_type {
                            models::SourceType::Local => {
                                toggle_auto_open_local = Some(info.path.clone())
                            }
                            models::SourceType::Ssh => toggle_auto_open_ssh = Some(name.clone()),
                        }
                    }

                    let status_color = match info.status {
                        SourceStatus::Connected => egui::Color32::from_rgb(50, 205, 50),
                        SourceStatus::Connecting => egui::Color32::from_rgb(255, 200, 0),
                        SourceStatus::Disconnected => egui::Color32::from_rgb(150, 150, 150),
                        SourceStatus::Error => egui::Color32::from_rgb(255, 50, 50),
                    };

                    ui.colored_label(status_color, info.status_symbol());
                    ui.label(&info.name);
                    ui.label(format!("({})", info.source_type));

                    if info.source_type == models::SourceType::Ssh && ui.small_button("Edit").clicked() {
                        to_edit = Some(name.clone());
                    }

                    if ui.small_button("x").clicked() {
                        to_remove.push(name.clone());
                    }
                });

                ui.label(
                    egui::RichText::new(format!("  {} lines", info.line_count))
                        .small()
                        .color(egui::Color32::from_rgb(150, 150, 150)),
                );
            }
        }

        if let Some(path) = toggle_auto_open_local {
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
            ui.label(
                egui::RichText::new("Saved SSH Sources")
                    .small()
                    .color(egui::Color32::from_rgb(150, 150, 150)),
            );

            let saved_names: Vec<String> =
                self.saved_ssh_sources.iter().map(|s| s.name.clone()).collect();
            let active_names: std::collections::HashSet<String> =
                self.source_infos.keys().cloned().collect();

            let mut connect_source: Option<SavedSshSource> = None;
            let mut delete_source: Option<String> = None;
            let mut edit_saved: Option<SavedSshSource> = None;
            let mut toggle_saved_auto_open: Option<String> = None;

            for name in &saved_names {
                let is_active = active_names.contains(name);
                if !is_active {
                    ui.horizontal(|ui| {
                        let is_auto_open = self.is_ssh_source_auto_open(name);
                        let star = if is_auto_open { "★" } else { "☆" };
                        let star_color = if is_auto_open {
                            egui::Color32::from_rgb(255, 200, 50)
                        } else {
                            egui::Color32::from_rgb(100, 100, 100)
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(star).color(star_color).small(),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Toggle auto-open on startup")
                            .clicked()
                        {
                            toggle_saved_auto_open = Some(name.clone());
                        }

                        ui.label(
                            egui::RichText::new(name)
                                .small()
                                .color(egui::Color32::from_rgb(120, 120, 120)),
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

            if let Some(name) = toggle_saved_auto_open {
                self.toggle_ssh_source_auto_open(&name);
            }

            if let Some(source) = connect_source {
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

            if let Some(source) = edit_saved {
                self.ssh_dialog.load_from_saved(&source);
                self.ssh_dialog.open = true;
            }

            if let Some(name) = delete_source {
                self.remove_saved_ssh_source(&name);
            }

            ui.separator();
        }

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
                    ui.add_space(5.0);
                    ui.hyperlink_to(
                        "GitHub: MarcelineVPQ/oxitailr",
                        "https://github.com/MarcelineVPQ/oxitailr",
                    );
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() {
                        self.show_about_dialog = false;
                    }
                    ui.add_space(10.0);
                });
            });
    }

    fn render_help_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_help_dialog {
            return;
        }

        egui::Window::new("Help")
            .collapsible(false)
            .resizable(true)
            .default_size([500.0, 450.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Getting Started");
                    ui.add_space(5.0);
                    ui.label("Oxitailr is a modern log viewer that supports both local files and remote SSH sources.");
                    ui.add_space(10.0);

                    ui.heading("Adding Sources");
                    ui.add_space(5.0);
                    ui.label("• Local File: Click '+ Local File' to browse and open a local log file");
                    ui.label("• SSH Source: Click '+ SSH' to connect to a remote server via SSH");
                    ui.add_space(10.0);

                    ui.heading("Filtering");
                    ui.add_space(5.0);
                    ui.label("• Filter bar: Type a regex pattern to filter log lines");
                    ui.label("• Level filters: Use checkboxes to show/hide specific log levels");
                    ui.label("• Advanced: Click 'Advanced' to build complex filter rules");
                    ui.add_space(10.0);

                    ui.heading("Search");
                    ui.add_space(5.0);
                    ui.label("• Use the 'Search' field to highlight matching text");
                    ui.label("• Matching lines will have a yellow background");
                    ui.add_space(10.0);

                    ui.heading("Keyboard Shortcuts");
                    ui.add_space(5.0);
                    ui.label("• Ctrl+F: Focus filter field");
                    ui.label("• Ctrl+G: Focus search field");
                    ui.label("• Ctrl+L: Clear log view");
                    ui.label("• Ctrl+O: Open local file");
                    ui.label("• Home/End: Jump to first/last line");
                    ui.label("• Page Up/Down: Scroll by page");
                    ui.add_space(10.0);

                    ui.heading("Settings");
                    ui.add_space(5.0);
                    ui.label("• Theme: Choose Light, Dark, or System (follows OS preference)");
                    ui.label("• Font size and line spacing: Adjust display preferences");
                    ui.label("• Auto-scroll: Keep view at the bottom as new lines arrive");
                    ui.label("• Buffer size: Maximum number of lines to keep in memory");
                    ui.label("• Highlight rules: Configure custom highlighting for patterns");
                    ui.add_space(10.0);

                    ui.heading("SSH Authentication");
                    ui.add_space(5.0);
                    ui.label("SSH connections try the following methods in order:");
                    ui.label("1. Specified key file (if provided)");
                    ui.label("2. Default keys: ~/.ssh/id_ed25519, id_rsa, id_ecdsa");
                    ui.label("3. Password (if provided)");
                    ui.label("Passwords are stored encrypted locally.");
                    ui.add_space(15.0);

                    ui.separator();
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        if ui.button("Close").clicked() {
                            self.show_help_dialog = false;
                        }
                    });
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
            ui.label(if self.builder_rule_type == 2 {
                "Level:"
            } else {
                "Pattern:"
            });
            if self.builder_rule_type == 2 {
                egui::ComboBox::from_id_salt("level_select")
                    .selected_text(&self.builder_pattern)
                    .show_ui(ui, |ui| {
                        for level in ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "FATAL"] {
                            ui.selectable_value(
                                &mut self.builder_pattern,
                                level.to_string(),
                                level,
                            );
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

impl eframe::App for TailLoggerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_events();

        ctx.request_repaint_after(std::time::Duration::from_millis(self.update_interval_ms));

        // Handle SSH dialog
        match render_ssh_dialog(ctx, &mut self.ssh_dialog, &self.ssh_sources_path) {
            SshDialogResult::None => {}
            SshDialogResult::Save(saved, password) => {
                self.update_or_add_ssh_source(saved, password);
            }
            SshDialogResult::Connect(saved, password) => {
                let is_editing = self.ssh_dialog.editing.is_some();
                if is_editing {
                    self.remove_source(&saved.name);
                }
                self.update_or_add_ssh_source(saved.clone(), password.clone());
                let key_path = saved.key_path.as_ref().map(PathBuf::from);
                self.add_ssh_source(
                    saved.name,
                    saved.host,
                    saved.port,
                    saved.user,
                    saved.remote_path,
                    key_path,
                    password,
                );
            }
        }

        self.render_about_dialog(ctx);
        self.render_help_dialog(ctx);

        // Handle settings dialog
        match render_settings_dialog(
            ctx,
            &mut self.settings_dialog,
            &self.config_path,
            self.highlight_rules.len(),
        ) {
            SettingsDialogResult::None => {}
            SettingsDialogResult::Apply => {
                self.apply_settings_from_dialog();
            }
            SettingsDialogResult::Save => {
                self.apply_settings_from_dialog();
                if let Some(parent) = self.config_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = config::save_config(&self.config, &self.config_path) {
                    tracing::error!("Failed to save config: {}", e);
                }
            }
            SettingsDialogResult::OpenHighlightDialog => {
                self.highlight_dialog.open = true;
            }
        }

        render_highlight_dialog(ctx, &mut self.highlight_dialog, &mut self.highlight_rules);

        // Apply theme
        match self.theme {
            Theme::Light => ctx.set_visuals(egui::Visuals::light()),
            Theme::Dark => ctx.set_visuals(egui::Visuals::dark()),
            Theme::System => {
                // Use system preference if available, otherwise default to dark
                let visuals = if dark_light::detect() == dark_light::Mode::Light {
                    egui::Visuals::light()
                } else {
                    egui::Visuals::dark()
                };
                ctx.set_visuals(visuals);
            }
        }

        // Top panel with controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
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
                        self.show_help_dialog = true;
                        ui.close_menu();
                    }

                    if ui.button("ℹ  About").clicked() {
                        self.show_about_dialog = true;
                        ui.close_menu();
                    }
                });

                ui.separator();

                if let Ok(state) = self.log_state.lock() {
                    ui.label(format!("Lines: {}", state.lines.len()));
                    ui.label(format!("Total: {}", state.total_lines_read));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear").clicked() {
                        if let Ok(mut state) = self.log_state.lock() {
                            state.lines.clear();
                            state.total_lines_read = 0;
                        }
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

                if !self.filter_presets.is_empty() {
                    let preset_text = self
                        .selected_preset
                        .map(|i| {
                            self.filter_presets
                                .get(i)
                                .map(|p| p.name.as_str())
                                .unwrap_or("")
                        })
                        .unwrap_or("Preset...");

                    let mut preset_to_apply: Option<usize> = None;
                    let mut clear_preset = false;

                    egui::ComboBox::from_id_salt("filter_preset")
                        .selected_text(preset_text)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.selected_preset.is_none(), "None")
                                .clicked()
                            {
                                clear_preset = true;
                            }
                            for (i, preset) in self.filter_presets.iter().enumerate() {
                                if ui
                                    .selectable_label(self.selected_preset == Some(i), &preset.name)
                                    .clicked()
                                {
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

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Ok(state) = self.log_state.lock() {
                    let filtered_count = state
                        .lines
                        .iter()
                        .filter(|line| self.line_matches_filter(line))
                        .count();
                    ui.label(format!("Showing {} of {} lines", filtered_count, state.lines.len()));
                }

                let connected_count = self
                    .source_infos
                    .values()
                    .filter(|i| i.status == SourceStatus::Connected)
                    .count();
                ui.separator();
                ui.label(format!("Sources: {} connected", connected_count));

                // Show file size for current source
                if let Some(ref source_name) = self.selected_source {
                    if let Some(info) = self.source_infos.get(source_name) {
                        if let Some(size) = info.file_size {
                            ui.separator();
                            ui.label(format!("Size: {}", format_file_size(size)));
                        }
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label("Scroll: Mouse wheel | Home/End | Page Up/Down");
                });
            });
        });

        if self.show_source_panel {
            egui::SidePanel::left("source_panel")
                .default_width(200.0)
                .show(ctx, |ui| {
                    self.render_source_panel(ui);
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
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

            ui.horizontal(|ui| {
                let source_names: Vec<String> = self.source_infos.keys().cloned().collect();

                // Auto-select first source if none selected
                if self.selected_source.is_none() && !source_names.is_empty() {
                    self.selected_source = Some(source_names[0].clone());
                }

                for name in &source_names {
                    let is_selected = self.selected_source.as_ref() == Some(name);
                    let info = self.source_infos.get(name);

                    let status_symbol = info.map(|i| i.status_symbol()).unwrap_or("?");
                    let tab_text = format!("{} {}", status_symbol, name);

                    if ui.selectable_label(is_selected, tab_text).clicked() {
                        self.selected_source = Some(name.clone());
                    }
                }
            });

            ui.separator();

            let text_style = egui::TextStyle::Monospace;
            let row_height =
                ui.text_style_height(&text_style) * (self.font_size / 13.0) * self.line_spacing;

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
                    if let Some(ref selected) = self.selected_source {
                        if &line.entry.source != selected {
                            return false;
                        }
                    }
                    self.line_matches_filter(line)
                })
                .collect();

            let total_rows = filtered_lines.len();
            let search_lower = self.search_text.to_lowercase();
            let mut local_scroll_row = self.current_scroll_row;

            // Calculate row height including spacing (what show_rows uses internally)
            let spacing_y = ui.spacing().item_spacing.y;
            let row_height_with_spacing = row_height + spacing_y;
            let visible_rows = (ui.available_height() / row_height) as usize;
            let visible_height = ui.available_height();
            let content_height = total_rows as f32 * row_height_with_spacing;
            let max_offset = (content_height - visible_height).max(0.0);

            let mut scroll_request: Option<f32> = None;
            let mut scroll_to_end = false;
            ctx.input(|i| {
                if i.key_pressed(egui::Key::PageDown) {
                    let new_row = (self.current_scroll_row + visible_rows).min(total_rows.saturating_sub(1));
                    scroll_request = Some(new_row as f32 * row_height_with_spacing);
                    self.auto_scroll = false;
                }
                if i.key_pressed(egui::Key::PageUp) {
                    let new_row = self.current_scroll_row.saturating_sub(visible_rows);
                    scroll_request = Some(new_row as f32 * row_height_with_spacing);
                    self.auto_scroll = false;
                }
                if i.key_pressed(egui::Key::Home) {
                    scroll_request = Some(0.0);
                    self.auto_scroll = false;
                }
                if i.key_pressed(egui::Key::End) {
                    scroll_to_end = true;
                    self.auto_scroll = true;
                }
            });

            // Determine scroll offset
            let scroll_offset: Option<f32> = if let Some(offset) = scroll_request {
                self.scroll_to_row = None;
                Some(offset)
            } else if scroll_to_end {
                self.scroll_to_row = None;
                Some(max_offset)
            } else if let Some(row) = self.scroll_to_row.take() {
                Some(row as f32 * row_height_with_spacing)
            } else if total_rows > 0 && (self.initial_scroll_pending || (self.auto_scroll && self.new_lines_received)) {
                self.initial_scroll_pending = false;
                Some(max_offset)
            } else {
                None
            };

            let mut scroll_area = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(self.auto_scroll);

            if let Some(offset) = scroll_offset {
                scroll_area = scroll_area.vertical_scroll_offset(offset);
            }

            if self.wrap_lines {
                scroll_area.show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0 * line_spacing;

                    for (row, line) in filtered_lines.iter().enumerate() {
                        if row == 0 {
                            local_scroll_row = 0;
                        }

                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{:6} ", line.line_num))
                                    .monospace()
                                    .size(font_size)
                                    .color(egui::Color32::from_rgb(100, 100, 100)),
                            );

                            if show_source && source_count > 1 {
                                let source_color = egui::Color32::from_rgb(100, 150, 200);
                                ui.label(
                                    egui::RichText::new(format!("[{}] ", line.entry.source))
                                        .monospace()
                                        .size(font_size)
                                        .color(source_color),
                                );
                            }

                            if show_timestamps {
                                if let Some(ts) = &line.entry.timestamp {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} ",
                                            ts.format("%H:%M:%S%.3f")
                                        ))
                                        .monospace()
                                        .size(font_size)
                                        .color(egui::Color32::from_rgb(120, 120, 120)),
                                    );
                                }
                            }

                            if let Some(level) = &line.entry.level {
                                let level_color = log_level_color(Some(level));
                                ui.label(
                                    egui::RichText::new(format!("[{:5}] ", level.as_str()))
                                        .monospace()
                                        .size(font_size)
                                        .color(level_color),
                                );
                            }

                            let has_search_match = !search_lower.is_empty()
                                && line.entry.message.to_lowercase().contains(&search_lower);

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

                            if !line.entry.fields.is_empty() {
                                ui.label(
                                    egui::RichText::new(" {...}")
                                        .monospace()
                                        .size(font_size * 0.9)
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                )
                                .on_hover_ui(|ui| {
                                    ui.label("JSON Fields:");
                                    for (key, value) in &line.entry.fields {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{}:", key))
                                                    .color(egui::Color32::from_rgb(150, 200, 150)),
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
                scroll_area.show_rows(ui, row_height, total_rows, |ui, row_range| {
                    local_scroll_row = row_range.start;

                    for row in row_range {
                        if let Some(line) = filtered_lines.get(row) {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{:6} ", line.line_num))
                                        .monospace()
                                        .size(font_size)
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                );

                                if show_source && source_count > 1 {
                                    let source_color = egui::Color32::from_rgb(100, 150, 200);
                                    ui.label(
                                        egui::RichText::new(format!("[{}] ", line.entry.source))
                                            .monospace()
                                            .size(font_size)
                                            .color(source_color),
                                    );
                                }

                                if show_timestamps {
                                    if let Some(ts) = &line.entry.timestamp {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{} ",
                                                ts.format("%H:%M:%S%.3f")
                                            ))
                                            .monospace()
                                            .size(font_size)
                                            .color(egui::Color32::from_rgb(120, 120, 120)),
                                        );
                                    }
                                }

                                if let Some(level) = &line.entry.level {
                                    let level_color = log_level_color(Some(level));
                                    ui.label(
                                        egui::RichText::new(format!("[{:5}] ", level.as_str()))
                                            .monospace()
                                            .size(font_size)
                                            .color(level_color),
                                    );
                                }

                                let has_search_match = !search_lower.is_empty()
                                    && line.entry.message.to_lowercase().contains(&search_lower);

                                let display_text = &line.entry.message;
                                let highlight =
                                    find_matching_highlight(&highlight_rules, &line.entry.raw);

                                if let Some(rule) = highlight {
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
                                    for span in &line.spans {
                                        let mut text = egui::RichText::new(&span.text)
                                            .monospace()
                                            .size(font_size)
                                            .color(span.color);

                                        if span.bold {
                                            text = text.strong();
                                        }

                                        if has_search_match
                                            && span.text.to_lowercase().contains(&search_lower)
                                        {
                                            text = text.background_color(egui::Color32::from_rgb(
                                                100, 100, 0,
                                            ));
                                        }

                                        ui.label(text);
                                    }
                                } else {
                                    let color = log_level_color(line.entry.level.as_ref());
                                    let mut text = egui::RichText::new(display_text)
                                        .monospace()
                                        .size(font_size)
                                        .color(color);

                                    if has_search_match {
                                        text = text
                                            .background_color(egui::Color32::from_rgb(100, 100, 0));
                                    }

                                    ui.label(text);
                                }

                                if !line.entry.fields.is_empty() {
                                    ui.label(
                                        egui::RichText::new(" {...}")
                                            .monospace()
                                            .size(font_size * 0.9)
                                            .color(egui::Color32::from_rgb(100, 100, 100)),
                                    )
                                    .on_hover_ui(|ui| {
                                        ui.label("JSON Fields:");
                                        for (key, value) in &line.entry.fields {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(format!("{}:", key))
                                                        .color(egui::Color32::from_rgb(
                                                            150, 200, 150,
                                                        )),
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

            self.current_scroll_row = local_scroll_row;
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_session();

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
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let mut config = load_or_default_config(Some(config_path.clone()));

    if let Some(max_lines) = cli.max_lines {
        config.general.buffer_size = max_lines;
    }

    if let Some(ref file) = cli.file {
        if !file.exists() {
            anyhow::bail!("File not found: {}", file.display());
        }
    }

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
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
            Ok(Box::new(TailLoggerApp::new(
                cc, config, config_path, file_path, runtime,
            )))
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run GUI: {}", e))?;

    Ok(())
}
