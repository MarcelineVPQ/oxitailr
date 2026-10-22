use super::{AlertRule, Alerter};
use crate::models::LogEntry;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;

pub struct WebhookAlert {
    client: std::sync::OnceLock<Client>,
}

#[derive(Serialize)]
struct WebhookPayload<'a> {
    alert_name: &'a str,
    source: &'a str,
    level: Option<String>,
    message: &'a str,
    timestamp: Option<String>,
    raw: &'a str,
}

impl WebhookAlert {
    pub fn new() -> Self {
        Self {
            client: std::sync::OnceLock::new(),
        }
    }

    fn get_client(&self) -> &Client {
        self.client.get_or_init(Client::new)
    }

    pub async fn send(&self, entry: &LogEntry, rule: &AlertRule, url: &str) -> Result<()> {
        let payload = WebhookPayload {
            alert_name: &rule.name,
            source: &entry.source,
            level: entry.level.map(|l| l.to_string()),
            message: &entry.message,
            timestamp: entry.timestamp.map(|t| t.to_rfc3339()),
            raw: &entry.raw,
        };

        self.get_client().post(url).json(&payload).send().await?;

        Ok(())
    }
}

impl Default for WebhookAlert {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Alerter for WebhookAlert {
    async fn alert(&self, entry: &LogEntry, rule: &AlertRule) -> Result<()> {
        // Find webhook URL from actions
        for action in &rule.actions {
            if let super::AlertAction::Webhook { url } = action {
                self.send(entry, rule, url).await?;
            }
        }
        Ok(())
    }
}
