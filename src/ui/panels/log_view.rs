//! Log view rendering panel.

use crate::config::LogLevelColors;
use crate::models::LogLevel;
use crate::ui::ansi::{parse_ansi_line, ColoredSpan};
use eframe::egui;

/// Color for a log level, with optional custom colors
pub fn log_level_color(
    level: Option<&LogLevel>,
    custom_colors: Option<&LogLevelColors>,
) -> egui::Color32 {
    match (level, custom_colors) {
        (Some(LogLevel::Trace), Some(c)) => {
            egui::Color32::from_rgb(c.trace[0], c.trace[1], c.trace[2])
        }
        (Some(LogLevel::Debug), Some(c)) => {
            egui::Color32::from_rgb(c.debug[0], c.debug[1], c.debug[2])
        }
        (Some(LogLevel::Info), Some(c)) => egui::Color32::from_rgb(c.info[0], c.info[1], c.info[2]),
        (Some(LogLevel::Warn), Some(c)) => egui::Color32::from_rgb(c.warn[0], c.warn[1], c.warn[2]),
        (Some(LogLevel::Error), Some(c)) => {
            egui::Color32::from_rgb(c.error[0], c.error[1], c.error[2])
        }
        (Some(LogLevel::Fatal), Some(c)) => {
            egui::Color32::from_rgb(c.fatal[0], c.fatal[1], c.fatal[2])
        }
        (Some(LogLevel::Trace), None) => egui::Color32::from_rgb(100, 100, 100),
        (Some(LogLevel::Debug), None) => egui::Color32::from_rgb(140, 140, 140),
        (Some(LogLevel::Info), None) => egui::Color32::from_rgb(80, 180, 220),
        (Some(LogLevel::Warn), None) => egui::Color32::from_rgb(220, 180, 50),
        (Some(LogLevel::Error), None) => egui::Color32::from_rgb(220, 80, 80),
        (Some(LogLevel::Fatal), None) => egui::Color32::from_rgb(255, 50, 150),
        (None, _) => egui::Color32::from_rgb(200, 200, 200),
    }
}

/// A display line with pre-parsed ANSI colors
#[derive(Clone)]
pub struct DisplayLine {
    pub entry: crate::models::LogEntry,
    pub spans: Vec<ColoredSpan>,
    pub has_ansi: bool,
    pub line_num: usize,
}

impl DisplayLine {
    pub fn from_entry(entry: crate::models::LogEntry, line_num: usize) -> Self {
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
