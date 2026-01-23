//! Alert system module - currently unused but planned for future integration.
//! Provides desktop notifications, webhook alerts, and visual/sound alerts.

#![allow(dead_code)]

mod desktop;
mod sound;
mod visual;
mod webhook;

pub use desktop::DesktopNotifier;
pub use sound::SoundAlert;
pub use visual::VisualAlert;
pub use webhook::WebhookAlert;

use crate::filter::FilterRule;
use crate::models::LogEntry;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub name: String,
    pub pattern: FilterRule,
    pub actions: Vec<AlertAction>,
    pub cooldown_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AlertAction {
    Visual,
    Sound,
    Desktop { title: Option<String> },
    Webhook { url: String },
}

#[async_trait]
pub trait Alerter: Send + Sync {
    async fn alert(&self, entry: &LogEntry, rule: &AlertRule) -> Result<()>;
}

pub struct AlertDispatcher {
    rules: Vec<AlertRule>,
    visual: Arc<VisualAlert>,
    sound: Arc<SoundAlert>,
    desktop: Arc<DesktopNotifier>,
    webhook: Arc<WebhookAlert>,
    alert_tx: mpsc::Sender<AlertEvent>,
}

#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub entry: LogEntry,
    pub rule_name: String,
}

impl AlertDispatcher {
    pub fn new() -> (Self, mpsc::Receiver<AlertEvent>) {
        let (alert_tx, alert_rx) = mpsc::channel(100);
        (
            Self {
                rules: Vec::new(),
                visual: Arc::new(VisualAlert::new()),
                sound: Arc::new(SoundAlert::new()),
                desktop: Arc::new(DesktopNotifier::new()),
                webhook: Arc::new(WebhookAlert::new()),
                alert_tx,
            },
            alert_rx,
        )
    }

    pub fn add_rule(&mut self, rule: AlertRule) {
        self.rules.push(rule);
    }

    pub fn clear_rules(&mut self) {
        self.rules.clear();
    }

    pub async fn check_and_alert(&self, entry: &LogEntry) -> Result<()> {
        use crate::filter::Filter;

        for rule in &self.rules {
            if rule.pattern.matches(entry) {
                let _ = self
                    .alert_tx
                    .send(AlertEvent {
                        entry: entry.clone(),
                        rule_name: rule.name.clone(),
                    })
                    .await;

                for action in &rule.actions {
                    match action {
                        AlertAction::Visual => {
                            self.visual.alert(entry, rule).await?;
                        }
                        AlertAction::Sound => {
                            self.sound.alert(entry, rule).await?;
                        }
                        AlertAction::Desktop { .. } => {
                            self.desktop.alert(entry, rule).await?;
                        }
                        AlertAction::Webhook { url } => {
                            self.webhook.send(entry, rule, url).await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get_matching_rules(&self, entry: &LogEntry) -> Vec<&AlertRule> {
        use crate::filter::Filter;
        self.rules
            .iter()
            .filter(|rule| rule.pattern.matches(entry))
            .collect()
    }
}

impl Default for AlertDispatcher {
    fn default() -> Self {
        Self::new().0
    }
}
