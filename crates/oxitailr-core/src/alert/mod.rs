//! Alert system module for desktop notifications, webhook alerts, and visual/sound alerts.

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
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};

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
    rules: RwLock<Vec<AlertRule>>,
    cooldowns: RwLock<HashMap<String, Instant>>,
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
                rules: RwLock::new(Vec::new()),
                cooldowns: RwLock::new(HashMap::new()),
                visual: Arc::new(VisualAlert::new()),
                sound: Arc::new(SoundAlert::new()),
                desktop: Arc::new(DesktopNotifier::new()),
                webhook: Arc::new(WebhookAlert::new()),
                alert_tx,
            },
            alert_rx,
        )
    }

    pub async fn add_rule(&self, rule: AlertRule) {
        let mut rules = self.rules.write().await;
        rules.push(rule);
    }

    pub async fn clear_rules(&self) {
        let mut rules = self.rules.write().await;
        rules.clear();
        let mut cooldowns = self.cooldowns.write().await;
        cooldowns.clear();
    }

    #[allow(dead_code)]
    pub async fn set_rules(&self, new_rules: Vec<AlertRule>) {
        let mut rules = self.rules.write().await;
        *rules = new_rules;
    }

    #[allow(dead_code)]
    pub async fn get_rules(&self) -> Vec<AlertRule> {
        let rules = self.rules.read().await;
        rules.clone()
    }

    async fn check_cooldown(&self, rule_name: &str, cooldown_secs: Option<u64>) -> bool {
        let Some(cooldown) = cooldown_secs else {
            return true; // No cooldown, always fire
        };

        let mut cooldowns = self.cooldowns.write().await;
        let now = Instant::now();

        if let Some(last_fired) = cooldowns.get(rule_name) {
            if now.duration_since(*last_fired).as_secs() < cooldown {
                return false; // Still in cooldown
            }
        }

        cooldowns.insert(rule_name.to_string(), now);
        true
    }

    pub async fn check_and_alert(&self, entry: &LogEntry) -> Result<()> {
        use crate::filter::Filter;

        let rules = self.rules.read().await;

        for rule in rules.iter() {
            if rule.pattern.matches(entry) {
                // Check cooldown
                if !self.check_cooldown(&rule.name, rule.cooldown_seconds).await {
                    continue;
                }

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
                            let _ = self.visual.alert(entry, rule).await;
                        }
                        AlertAction::Sound => {
                            let _ = self.sound.alert(entry, rule).await;
                        }
                        AlertAction::Desktop { .. } => {
                            let _ = self.desktop.alert(entry, rule).await;
                        }
                        AlertAction::Webhook { url } => {
                            let _ = self.webhook.send(entry, rule, url).await;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Default for AlertDispatcher {
    fn default() -> Self {
        Self::new().0
    }
}
