//! oxitailr-core — the engine behind oxitailr.
//!
//! Everything that isn't a screen lives here: log sources (local files with
//! rotation detection, SSH), parsers, the filter engine and highlight rules,
//! alerts, config and session persistence, and `AppCore`, which ties them
//! together behind channels. A front end (the oxitailr app in `desktop/`, or
//! any other program) drives an `AppCore` and draws what it emits.
//
// The backend (SSH credentials, session persistence, alert actions, the full
// filter rule set) is feature-complete but not all of it is wired into a front
// end yet. Allow dead code crate-wide rather than deleting working modules.
#![allow(dead_code)]

pub mod alert;
pub mod app;
pub mod config;
pub mod filter;
pub mod models;
pub mod parser;
pub mod secrets;
pub mod source;
pub mod state;

/// Application theme (kept for config compatibility; the app sets its theme
/// in its own settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}
