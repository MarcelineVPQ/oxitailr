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
