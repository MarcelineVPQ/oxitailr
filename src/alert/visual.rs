use super::{AlertRule, Alerter};
use crate::models::LogEntry;
use anyhow::Result;
use async_trait::async_trait;

pub struct VisualAlert;

impl VisualAlert {
    pub fn new() -> Self {
        Self
    }
}

impl Default for VisualAlert {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Alerter for VisualAlert {
    async fn alert(&self, _entry: &LogEntry, _rule: &AlertRule) -> Result<()> {
        // Visual alerts are handled by the TUI through AlertEvent
        // This is a no-op here, the TUI will highlight matching entries
        Ok(())
    }
}
