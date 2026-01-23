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

// SSH Dialog state
#[derive(Default)]
struct SshDialogState {
    open: bool,
    name: String,
    host: String,
    port: String,
    user: String,
    remote_path: String,
    key_path: String,
    error: Option<String>,
}

impl SshDialogState {
    fn reset(&mut self) {
        self.name.clear();
        self.host.clear();
        self.port = "22".to_string();
        self.user.clear();
        self.remote_path.clear();
        self.key_path.clear();
        self.error = None;
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
    show_timestamps: bool,
    show_source: bool,
    wrap_lines: bool,

    // Dialogs
    ssh_dialog: SshDialogState,
    show_source_panel: bool,
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
            show_timestamps: config.general.show_timestamps,
            show_source: config.general.show_source,
            wrap_lines: config.general.wrap_lines,
            ssh_dialog: SshDialogState::default(),
            show_source_panel: true,
        };

        // Add initial file if provided
        if let Some(path) = initial_file {
            app.add_local_source_from_path(path);
        }

        // Load sources from config
        for source_config in &config.sources {
            if source_config.is_enabled() {
                app.add_source_from_config(source_config.clone());
            }
        }

        app
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
                    sm.add_ssh_source(name.clone(), host, user, path, Some(port), key_path);
                    let _ = sm.start_source(&name).await;
                }
            }
        });
    }

    fn add_ssh_source(&mut self, name: String, host: String, port: u16, user: String, path: String, key_path: Option<PathBuf>) {
        let source_manager = self.source_manager.clone();
        let runtime = self.runtime.clone();

        runtime.block_on(async {
            let mut sm = source_manager.lock().await;
            sm.add_ssh_source(name.clone(), host, user, path, Some(port), key_path);
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

    fn open_settings_location(&self) {
        if let Some(parent) = self.config_path.parent() {
            // Create directory if it doesn't exist
            let _ = std::fs::create_dir_all(parent);

            // Create default config if it doesn't exist
            if !self.config_path.exists() {
                let _ = config::save_config(&self.config, &self.config_path);
            }

            // Open the directory in file manager
            #[cfg(target_os = "linux")]
            {
                let _ = std::process::Command::new("xdg-open")
                    .arg(parent)
                    .spawn();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = std::process::Command::new("open")
                    .arg(parent)
                    .spawn();
            }
            #[cfg(target_os = "windows")]
            {
                let _ = std::process::Command::new("explorer")
                    .arg(parent)
                    .spawn();
            }
        }
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

        for name in &source_names {
            if let Some(info) = self.source_infos.get(name) {
                ui.horizontal(|ui| {
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

                    if ui.small_button("✕").clicked() {
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

        for name in to_remove {
            self.remove_source(&name);
        }

        ui.separator();

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

        egui::Window::new("Add SSH Source")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("ssh_dialog_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.ssh_dialog.name);
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
                    });

                if let Some(ref err) = self.ssh_dialog.error {
                    ui.colored_label(egui::Color32::RED, err);
                }

                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.ssh_dialog.open = false;
                    }

                    if ui.button("Connect").clicked() {
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

                            self.add_ssh_source(
                                self.ssh_dialog.name.clone(),
                                self.ssh_dialog.host.clone(),
                                port,
                                self.ssh_dialog.user.clone(),
                                self.ssh_dialog.remote_path.clone(),
                                key_path,
                            );

                            self.ssh_dialog.open = false;
                        }
                    }
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

impl eframe::App for TailLoggerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process events from source manager
        self.process_events();

        // Request repaint for live updates
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // Render SSH dialog if open
        self.render_ssh_dialog(ctx);

        // Top panel with controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("📂 Open File").clicked() {
                    self.open_file_dialog();
                }

                if ui.button("⚙ Settings").clicked() {
                    self.open_settings_location();
                }

                ui.separator();

                let state = self.log_state.lock().unwrap();
                ui.label(format!("Lines: {}", state.lines.len()));
                ui.label(format!("Total: {}", state.total_lines_read));
                drop(state);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear").clicked() {
                        self.log_state.lock().unwrap().lines.clear();
                    }

                    ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                    ui.checkbox(&mut self.show_source_panel, "Sources");

                    ui.add(egui::Slider::new(&mut self.font_size, 8.0..=24.0).text("Font"));
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

                ui.separator();

                ui.checkbox(&mut self.show_timestamps, "Timestamps");
                ui.checkbox(&mut self.show_source, "Source");
                ui.checkbox(&mut self.wrap_lines, "Wrap");
                ui.checkbox(&mut self.use_auto_parser, "Auto-parse JSON");
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

            let text_style = egui::TextStyle::Monospace;
            let row_height = ui.text_style_height(&text_style) * (self.font_size / 13.0);

            let state = self.log_state.lock().unwrap();
            let filtered_lines: Vec<&DisplayLine> = state
                .lines
                .iter()
                .filter(|line| self.line_matches_filter(line))
                .collect();

            let total_rows = filtered_lines.len();
            let search_lower = self.search_text.to_lowercase();

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

            // Only stick to bottom when new lines arrive and auto-scroll is on
            let should_stick = self.auto_scroll && self.new_lines_received;

            let mut scroll_area = egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(should_stick);

            // Handle scroll to specific row
            if let Some(row) = self.scroll_to_row.take() {
                scroll_area = scroll_area.vertical_scroll_offset(row as f32 * row_height);
            }

            scroll_area.show_rows(ui, row_height, total_rows, |ui, row_range| {
                // Track current scroll position
                self.current_scroll_row = row_range.start;

                for row in row_range {
                    if let Some(line) = filtered_lines.get(row) {
                        ui.horizontal(|ui| {
                            // Line number
                            ui.label(
                                egui::RichText::new(format!("{:6} ", line.line_num))
                                    .monospace()
                                    .size(self.font_size)
                                    .color(egui::Color32::from_rgb(100, 100, 100)),
                            );

                            // Source indicator
                            if self.show_source && self.source_infos.len() > 1 {
                                let source_color = egui::Color32::from_rgb(100, 150, 200);
                                ui.label(
                                    egui::RichText::new(format!("[{}] ", line.entry.source))
                                        .monospace()
                                        .size(self.font_size)
                                        .color(source_color),
                                );
                            }

                            // Timestamp
                            if self.show_timestamps {
                                if let Some(ts) = &line.entry.timestamp {
                                    ui.label(
                                        egui::RichText::new(format!("{} ", ts.format("%H:%M:%S%.3f")))
                                            .monospace()
                                            .size(self.font_size)
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
                                        .size(self.font_size)
                                        .color(level_color),
                                );
                            }

                            // Render message
                            let has_search_match = !search_lower.is_empty()
                                && line.entry.message.to_lowercase().contains(&search_lower);

                            // Use level-based colors if no ANSI codes, or show parsed message for JSON
                            let display_text = if !line.entry.fields.is_empty() {
                                // JSON log - show parsed message
                                &line.entry.message
                            } else {
                                // Plain log - show raw (ANSI stripped is in spans)
                                &line.entry.message
                            };

                            if line.has_ansi {
                                // Render ANSI colored spans
                                for span in &line.spans {
                                    let mut text = egui::RichText::new(&span.text)
                                        .monospace()
                                        .size(self.font_size)
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
                                    .size(self.font_size)
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
                                        .size(self.font_size * 0.9)
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
        });

        // Bottom status bar
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
