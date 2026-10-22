use super::{AlertRule, Alerter};
use crate::models::LogEntry;
use anyhow::Result;
use async_trait::async_trait;

pub struct SoundAlert;

impl SoundAlert {
    pub fn new() -> Self {
        Self
    }

    pub fn bell() {
        print!("\x07");
    }
}

impl Default for SoundAlert {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Alerter for SoundAlert {
    async fn alert(&self, _entry: &LogEntry, _rule: &AlertRule) -> Result<()> {
        Self::bell();
        Ok(())
    }
}
