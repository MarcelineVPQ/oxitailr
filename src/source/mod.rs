mod local;
mod manager;
mod ssh;

pub use local::LocalFileSource;
pub use manager::SourceManager;
pub use ssh::SshSource;

use crate::models::{SourceInfo, SourceType};
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
#[allow(dead_code)]
pub trait Source: Send + Sync {
    fn name(&self) -> &str;
    fn source_type(&self) -> SourceType;
    fn info(&self) -> SourceInfo;
    async fn start(&mut self, sender: mpsc::Sender<SourceEvent>) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
}

#[derive(Debug, Clone)]
pub enum SourceEvent {
    Line { source: String, line: String },
    StatusChange { source: String, info: SourceInfo },
    Error { source: String, error: String },
}

/// Commands sent from the UI thread to the async task that owns the
/// [`SourceManager`]. This keeps all source start/stop work off the UI thread
/// (it previously called `runtime.block_on`, which froze the window).
#[derive(Debug)]
pub enum SourceCommand {
    AddLocal {
        name: String,
        path: std::path::PathBuf,
    },
    AddSsh {
        name: String,
        host: String,
        user: String,
        path: String,
        port: Option<u16>,
        key_path: Option<std::path::PathBuf>,
        password: Option<String>,
    },
    Remove {
        name: String,
    },
    /// Stop then restart the named sources (re-reads each file from the start).
    Reload {
        names: Vec<String>,
    },
    /// Stop the named sources (best-effort, used on shutdown).
    StopAll {
        names: Vec<String>,
    },
}
