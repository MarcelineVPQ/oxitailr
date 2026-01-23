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
