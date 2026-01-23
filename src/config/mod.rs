mod settings;

pub use settings::{AppConfig, SourceConfig};

use anyhow::{Context, Result};
use std::path::Path;

pub fn load_config(path: &Path) -> Result<AppConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    let config: AppConfig = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;

    Ok(config)
}

pub fn save_config(config: &AppConfig, path: &Path) -> Result<()> {
    let content = toml::to_string_pretty(config)
        .context("Failed to serialize config")?;

    std::fs::write(path, content)
        .with_context(|| format!("Failed to write config file: {}", path.display()))?;

    Ok(())
}
