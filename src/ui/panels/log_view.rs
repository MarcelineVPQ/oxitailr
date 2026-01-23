//! Log view rendering panel.

use crate::models::LogLevel;
use crate::ui::ansi::{parse_ansi_line, ColoredSpan};
use eframe::egui;

/// Color for a log level
pub fn log_level_color(level: Option<&LogLevel>) -> egui::Color32 {
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
