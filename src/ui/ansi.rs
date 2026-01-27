//! ANSI color code parsing for terminal output rendering.

use eframe::egui;
use regex::Regex;
use std::sync::LazyLock;

// Constants for default colors
pub const DEFAULT_TEXT_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 200, 200);

/// Regex for matching ANSI escape sequences
pub static ANSI_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[([0-9;]*)m").unwrap());

/// A span of text with styling information
#[derive(Clone)]
pub struct ColoredSpan {
    pub text: String,
    pub color: egui::Color32,
    pub bold: bool,
}

/// Convert ANSI color code to egui Color32
pub fn ansi_to_color(code: u8) -> egui::Color32 {
    match code {
        30 => egui::Color32::from_rgb(0, 0, 0),       // Black
        31 => egui::Color32::from_rgb(205, 49, 49),   // Red
        32 => egui::Color32::from_rgb(13, 188, 121),  // Green
        33 => egui::Color32::from_rgb(229, 229, 16),  // Yellow
        34 => egui::Color32::from_rgb(36, 114, 200),  // Blue
        35 => egui::Color32::from_rgb(188, 63, 188),  // Magenta
        36 => egui::Color32::from_rgb(17, 168, 205),  // Cyan
        37 => egui::Color32::from_rgb(229, 229, 229), // White
        // Bright colors
        90 => egui::Color32::from_rgb(102, 102, 102), // Bright Black
        91 => egui::Color32::from_rgb(241, 76, 76),   // Bright Red
        92 => egui::Color32::from_rgb(35, 209, 139),  // Bright Green
        93 => egui::Color32::from_rgb(245, 245, 67),  // Bright Yellow
        94 => egui::Color32::from_rgb(59, 142, 234),  // Bright Blue
        95 => egui::Color32::from_rgb(214, 112, 214), // Bright Magenta
        96 => egui::Color32::from_rgb(41, 184, 219),  // Bright Cyan
        97 => egui::Color32::from_rgb(255, 255, 255), // Bright White
        _ => DEFAULT_TEXT_COLOR,
    }
}

/// Parse an ANSI-encoded line into colored spans
pub fn parse_ansi_line(text: &str) -> Vec<ColoredSpan> {
    let mut spans = Vec::new();
    let mut current_color = DEFAULT_TEXT_COLOR;
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
                        current_color = DEFAULT_TEXT_COLOR;
                        current_bold = false;
                    }
                    1 => current_bold = true,
                    22 => current_bold = false,
                    30..=37 | 90..=97 => current_color = ansi_to_color(code),
                    39 => current_color = DEFAULT_TEXT_COLOR,
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
            color: DEFAULT_TEXT_COLOR,
            bold: false,
        });
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plain_text() {
        let spans = parse_ansi_line("Hello World");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "Hello World");
    }

    #[test]
    fn test_parse_colored_text() {
        let spans = parse_ansi_line("\x1b[31mRed Text\x1b[0m Normal");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].text, "Red Text");
        assert_eq!(spans[0].color, egui::Color32::from_rgb(205, 49, 49));
        assert_eq!(spans[1].text, " Normal");
    }
}
