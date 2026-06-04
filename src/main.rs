//! Oxitailr — a terminal log viewer / live tailer.
//
// The backend (SSH credentials, session persistence, alert actions, the full
// filter rule set) is feature-complete but not all of it is wired into the TUI
// yet — that lands in later phases. Allow dead code crate-wide during the
// rewrite rather than deleting working backend modules.
#![allow(dead_code)]

mod alert;
mod app;
mod config;
mod credentials;
mod filter;
mod models;
mod parser;
mod source;
mod state;
mod tui;

use anyhow::Result;
use clap::Parser as ClapParser;
use config::AppConfig;
use glob::glob;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Application theme (kept for config compatibility; the TUI follows the
/// terminal's own colors).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(ClapParser)]
#[command(name = "oxitailr")]
#[command(author, version, about = "A terminal log viewer / live tailer")]
struct Cli {
    /// Log files to tail (globs like '/var/log/*.log' are expanded)
    #[arg(value_name = "FILE")]
    files: Vec<PathBuf>,

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
            Ok(cfg) => return cfg,
            Err(e) => eprintln!("Failed to load config: {e}; using defaults"),
        }
    }
    AppConfig::default()
}

fn is_glob_pattern(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains('*') || s.contains('?') || s.contains('[')
}

fn expand_glob_pattern(pattern: &Path) -> Vec<PathBuf> {
    match glob(&pattern.to_string_lossy()) {
        Ok(paths) => paths
            .filter_map(|e| e.ok())
            .filter(|p| p.is_file())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let mut config = load_or_default_config(Some(config_path));
    if let Some(max_lines) = cli.max_lines {
        config.general.buffer_size = max_lines;
    }

    let mut initial_files: Vec<PathBuf> = Vec::new();
    for file in &cli.files {
        if is_glob_pattern(file) {
            initial_files.extend(expand_glob_pattern(file));
        } else if file.exists() {
            initial_files.push(file.clone());
        } else {
            anyhow::bail!("File not found: {}", file.display());
        }
    }

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?,
    );

    let (core, channels) = app::AppCore::new(runtime.clone(), config, initial_files);
    runtime.block_on(tui::run(core, channels))
}
