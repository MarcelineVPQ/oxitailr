use super::{AlertRule, Alerter};
use crate::models::LogEntry;
use anyhow::Result;
use async_trait::async_trait;
use notify_rust::Notification;

pub struct DesktopNotifier {
    app_name: String,
}

impl DesktopNotifier {
    pub fn new() -> Self {
        Self {
            app_name: "Tail Logger".to_string(),
        }
    }

    pub fn with_app_name(mut self, name: String) -> Self {
        self.app_name = name;
        self
    }
}

impl Default for DesktopNotifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Alerter for DesktopNotifier {
    async fn alert(&self, entry: &LogEntry, rule: &AlertRule) -> Result<()> {
        let title = format!("[{}] Alert: {}", entry.source, rule.name);
        let body = if entry.message.len() > 200 {
            format!("{}...", &entry.message[..200])
        } else {
            entry.message.clone()
        };

        // Run notification in blocking task since notify-rust is sync
        let app_name = self.app_name.clone();
        tokio::task::spawn_blocking(move || {
            let _ = Notification::new()
                .appname(&app_name)
                .summary(&title)
                .body(&body)
                .timeout(5000)
                .show();
        })
        .await?;

        Ok(())
    }
}
