use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    Local,
    Ssh,
}

impl std::fmt::Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceType::Local => write!(f, "local"),
            SourceType::Ssh => write!(f, "ssh"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceStatus {
    Connecting,
    Connected,
    Disconnected,
    Error,
}

impl std::fmt::Display for SourceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceStatus::Connecting => write!(f, "connecting"),
            SourceStatus::Connected => write!(f, "connected"),
            SourceStatus::Disconnected => write!(f, "disconnected"),
            SourceStatus::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub name: String,
    pub source_type: SourceType,
    pub path: String,
    pub status: SourceStatus,
    pub line_count: u64,
    pub enabled: bool,
    #[serde(default)]
    pub file_size: Option<u64>,
}

impl SourceInfo {
    pub fn new(name: String, source_type: SourceType, path: String) -> Self {
        Self {
            name,
            source_type,
            path,
            status: SourceStatus::Disconnected,
            line_count: 0,
            enabled: true,
            file_size: None,
        }
    }

    pub fn status_symbol(&self) -> &'static str {
        match self.status {
            SourceStatus::Connecting => "◐",
            SourceStatus::Connected => "●",
            SourceStatus::Disconnected => "○",
            SourceStatus::Error => "✗",
        }
    }
}
