use super::{Source, SourceEvent};
use crate::models::{SourceInfo, SourceStatus, SourceType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use russh::client;
use russh_keys::load_secret_key;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

pub struct SshSource {
    name: String,
    host: String,
    port: u16,
    user: String,
    path: String,
    key_path: Option<PathBuf>,
    info: Arc<Mutex<SourceInfo>>,
    stop_tx: Option<watch::Sender<bool>>,
}

impl SshSource {
    pub fn new(name: String, host: String, user: String, path: String) -> Self {
        let display_path = format!("{}@{}:{}", user, host, path);
        let info = SourceInfo::new(name.clone(), SourceType::Ssh, display_path);
        Self {
            name,
            host,
            port: 22,
            user,
            path,
            key_path: None,
            info: Arc::new(Mutex::new(info)),
            stop_tx: None,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn with_key_path(mut self, key_path: PathBuf) -> Self {
        self.key_path = Some(key_path);
        self
    }
}

struct SshHandler;

#[async_trait]
impl client::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[async_trait]
impl Source for SshSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::Ssh
    }

    fn info(&self) -> SourceInfo {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async { self.info.lock().await.clone() })
        })
    }

    async fn start(&mut self, sender: mpsc::Sender<SourceEvent>) -> Result<()> {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        self.stop_tx = Some(stop_tx);

        let host = self.host.clone();
        let port = self.port;
        let user = self.user.clone();
        let path = self.path.clone();
        let name = self.name.clone();
        let info = self.info.clone();
        let key_path = self.key_path.clone();

        {
            let mut info_guard = info.lock().await;
            info_guard.status = SourceStatus::Connecting;
            let _ = sender
                .send(SourceEvent::StatusChange {
                    source: name.clone(),
                    info: info_guard.clone(),
                })
                .await;
        }

        tokio::spawn(async move {
            if let Err(e) = run_ssh_tail(
                host,
                port,
                user,
                path,
                key_path,
                name.clone(),
                info.clone(),
                sender.clone(),
                &mut stop_rx,
            )
            .await
            {
                let _ = sender
                    .send(SourceEvent::Error {
                        source: name.clone(),
                        error: e.to_string(),
                    })
                    .await;

                let mut info_guard = info.lock().await;
                info_guard.status = SourceStatus::Error;
                let _ = sender
                    .send(SourceEvent::StatusChange {
                        source: name,
                        info: info_guard.clone(),
                    })
                    .await;
            }
        });

        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(true);
        }
        Ok(())
    }
}

async fn run_ssh_tail(
    host: String,
    port: u16,
    user: String,
    path: String,
    key_path: Option<PathBuf>,
    name: String,
    info: Arc<Mutex<SourceInfo>>,
    sender: mpsc::Sender<SourceEvent>,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let config = client::Config::default();
    let config = Arc::new(config);
    let handler = SshHandler;

    let mut session = client::connect(config, (host.as_str(), port), handler)
        .await
        .context("Failed to connect to SSH server")?;

    // Try to authenticate with key
    let key_path = key_path.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ssh")
            .join("id_rsa")
    });

    let authenticated = if key_path.exists() {
        if let Ok(key) = load_secret_key(&key_path, None) {
            session
                .authenticate_publickey(&user, Arc::new(key))
                .await
                .unwrap_or(false)
        } else {
            false
        }
    } else {
        false
    };

    if !authenticated {
        anyhow::bail!("SSH authentication failed");
    }

    {
        let mut info_guard = info.lock().await;
        info_guard.status = SourceStatus::Connected;
        let _ = sender
            .send(SourceEvent::StatusChange {
                source: name.clone(),
                info: info_guard.clone(),
            })
            .await;
    }

    let mut channel = session.channel_open_session().await?;
    let command = format!("tail -F {}", path);
    channel.exec(true, command).await?;

    let mut line_count: u64 = 0;
    let mut buffer = String::new();

    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(russh::ChannelMsg::Data { data }) => {
                        buffer.push_str(&String::from_utf8_lossy(&data));

                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].to_string();
                            buffer = buffer[pos + 1..].to_string();

                            if !line.trim().is_empty() {
                                line_count += 1;

                                {
                                    let mut info_guard = info.lock().await;
                                    info_guard.line_count = line_count;
                                }

                                let _ = sender.send(SourceEvent::Line {
                                    source: name.clone(),
                                    line: line.trim_end().to_string(),
                                }).await;
                            }
                        }
                    }
                    Some(russh::ChannelMsg::Eof) => {
                        break;
                    }
                    None => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let mut info_guard = info.lock().await;
    info_guard.status = SourceStatus::Disconnected;
    let _ = sender
        .send(SourceEvent::StatusChange {
            source: name,
            info: info_guard.clone(),
        })
        .await;

    Ok(())
}
