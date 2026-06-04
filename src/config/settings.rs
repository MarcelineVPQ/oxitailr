use crate::alert::AlertAction;
use crate::filter::FilterRule;
use crate::Theme;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(default)]
    pub filters: HashMap<String, FilterConfig>,
    #[serde(default)]
    pub alerts: Vec<AlertConfig>,
    #[serde(default)]
    pub webhook: Option<WebhookConfig>,
    /// Rules that colorize matching text in the log view.
    #[serde(default)]
    pub highlights: Vec<HighlightConfig>,
}

/// A rule that paints matching text in a custom color.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightConfig {
    /// Substring (case-insensitive) or regex to match.
    pub pattern: String,
    /// Treat `pattern` as a regular expression instead of a literal substring.
    #[serde(default)]
    pub regex: bool,
    /// Foreground color as [R, G, B]. Defaults to a warm yellow.
    #[serde(default = "default_highlight_color")]
    pub color: [u8; 3],
}

fn default_highlight_color() -> [u8; 3] {
    [255, 214, 0]
}

/// Custom colors for log levels (stored as [R, G, B])
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLevelColors {
    #[serde(default = "default_trace_color")]
    pub trace: [u8; 3],
    #[serde(default = "default_debug_color")]
    pub debug: [u8; 3],
    #[serde(default = "default_info_color")]
    pub info: [u8; 3],
    #[serde(default = "default_warn_color")]
    pub warn: [u8; 3],
    #[serde(default = "default_error_color")]
    pub error: [u8; 3],
    #[serde(default = "default_fatal_color")]
    pub fatal: [u8; 3],
}

fn default_trace_color() -> [u8; 3] {
    [100, 100, 100]
}
fn default_debug_color() -> [u8; 3] {
    [140, 140, 140]
}
fn default_info_color() -> [u8; 3] {
    [80, 180, 220]
}
fn default_warn_color() -> [u8; 3] {
    [220, 180, 50]
}
fn default_error_color() -> [u8; 3] {
    [220, 80, 80]
}
fn default_fatal_color() -> [u8; 3] {
    [255, 50, 150]
}

impl Default for LogLevelColors {
    fn default() -> Self {
        Self {
            trace: default_trace_color(),
            debug: default_debug_color(),
            info: default_info_color(),
            warn: default_warn_color(),
            error: default_error_color(),
            fatal: default_fatal_color(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    #[serde(default = "default_true")]
    pub follow: bool,
    #[serde(default)]
    pub wrap_lines: bool,
    #[serde(default = "default_true")]
    pub show_timestamps: bool,
    #[serde(default = "default_true")]
    pub show_source: bool,
    #[serde(default = "default_true")]
    pub color_enabled: bool,
    #[serde(default = "default_true")]
    pub remember_last_session: bool,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_line_spacing")]
    pub line_spacing: f32,
    #[serde(default = "default_tab_width")]
    pub tab_width: usize,
    #[serde(default = "default_update_interval")]
    pub update_interval_ms: u64,
    #[serde(default = "default_true")]
    pub auto_parse_json: bool,
    #[serde(default)]
    pub log_level_colors: LogLevelColors,
}

fn default_buffer_size() -> usize {
    10000
}

fn default_true() -> bool {
    true
}

fn default_font_size() -> f32 {
    13.0
}

fn default_line_spacing() -> f32 {
    1.0
}

fn default_tab_width() -> usize {
    4
}

fn default_update_interval() -> u64 {
    100
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            buffer_size: default_buffer_size(),
            follow: true,
            wrap_lines: false,
            show_timestamps: true,
            show_source: true,
            color_enabled: true,
            remember_last_session: true,
            theme: Theme::default(),
            font_size: default_font_size(),
            line_spacing: default_line_spacing(),
            tab_width: default_tab_width(),
            update_interval_ms: default_update_interval(),
            auto_parse_json: true,
            log_level_colors: LogLevelColors::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceConfig {
    Local {
        name: String,
        path: String,
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        auto_open: bool,
    },
    Ssh {
        name: String,
        host: String,
        #[serde(default = "default_ssh_port")]
        port: u16,
        user: String,
        path: String,
        #[serde(default)]
        key_path: Option<PathBuf>,
        #[serde(default)]
        enabled: bool,
        #[serde(default)]
        auto_open: bool,
    },
}

fn default_ssh_port() -> u16 {
    22
}

impl SourceConfig {
    pub fn is_enabled(&self) -> bool {
        match self {
            SourceConfig::Local { enabled, .. } => *enabled,
            SourceConfig::Ssh { enabled, .. } => *enabled,
        }
    }

    /// Whether this source should be opened automatically on startup.
    pub fn auto_open(&self) -> bool {
        match self {
            SourceConfig::Local { auto_open, .. } => *auto_open,
            SourceConfig::Ssh { auto_open, .. } => *auto_open,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub rules: Vec<FilterRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    pub name: String,
    pub pattern: String,
    #[serde(default)]
    pub actions: Vec<AlertAction>,
    #[serde(default)]
    pub cooldown_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}
