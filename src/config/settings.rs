use crate::alert::AlertAction;
use crate::filter::FilterRule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            sources: Vec::new(),
            filters: HashMap::new(),
            alerts: Vec::new(),
            webhook: None,
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
}

fn default_buffer_size() -> usize {
    10000
}

fn default_true() -> bool {
    true
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
    pub fn name(&self) -> &str {
        match self {
            SourceConfig::Local { name, .. } => name,
            SourceConfig::Ssh { name, .. } => name,
        }
    }

    pub fn is_enabled(&self) -> bool {
        match self {
            SourceConfig::Local { enabled, .. } => *enabled,
            SourceConfig::Ssh { enabled, .. } => *enabled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub rules: Vec<FilterRule>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            rules: Vec::new(),
        }
    }
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
