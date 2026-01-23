//! UI module containing all user interface components.

pub mod ansi;
pub mod dialogs;
pub mod panels;

pub use dialogs::{
    default_highlight_rules, find_matching_highlight, render_highlight_dialog,
    render_settings_dialog, render_ssh_dialog, HighlightDialogState, HighlightRule,
    SavedSshSource, SettingsDialogResult, SettingsDialogState, SshDialogResult, SshDialogState,
};
pub use panels::{log_level_color, DisplayLine};

/// Format file size in human-readable format
pub fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
