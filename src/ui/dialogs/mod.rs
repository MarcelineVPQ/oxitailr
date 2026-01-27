//! UI dialog modules.

pub mod alerts;
pub mod highlight;
pub mod settings;
pub mod ssh;

pub use alerts::{render_alert_dialog, AlertDialogResult, AlertDialogState};
pub use highlight::{
    default_highlight_rules, find_matching_highlight, render_highlight_dialog, HighlightDialogState,
    HighlightRule,
};
pub use settings::{render_settings_dialog, SettingsDialogResult, SettingsDialogState};
pub use ssh::{render_ssh_dialog, SavedSshSource, SshDialogResult, SshDialogState};
