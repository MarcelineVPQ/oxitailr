use super::{Source, SourceEvent};
use crate::models::{SourceInfo, SourceStatus, SourceType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch, Mutex};

pub struct LocalFileSource {
    name: String,
    path: PathBuf,
    info: Arc<Mutex<SourceInfo>>,
    stop_tx: Option<watch::Sender<bool>>,
}

impl LocalFileSource {
    pub fn new(name: String, path: PathBuf) -> Self {
        let info = SourceInfo::new(name.clone(), SourceType::Local, path.display().to_string());
        Self {
            name,
            path,
            info: Arc::new(Mutex::new(info)),
            stop_tx: None,
        }
    }
}

#[async_trait]
impl Source for LocalFileSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> SourceType {
        SourceType::Local
    }

    fn info(&self) -> SourceInfo {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async { self.info.lock().await.clone() })
        })
    }

    async fn start(&mut self, sender: mpsc::Sender<SourceEvent>) -> Result<()> {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        self.stop_tx = Some(stop_tx);

        let path = self.path.clone();
        let name = self.name.clone();
        let info = self.info.clone();

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
            if let Err(e) = run_local_tail(path, name.clone(), info.clone(), sender.clone(), &mut stop_rx).await {
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

async fn run_local_tail(
    path: PathBuf,
    name: String,
    info: Arc<Mutex<SourceInfo>>,
    sender: mpsc::Sender<SourceEvent>,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let file = File::open(&path)
        .await
        .with_context(|| format!("Failed to open file: {}", path.display()))?;

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

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_count: u64 = 0;

    // Read existing content from the file first
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // End of file
            Ok(_) => {
                let trimmed = line.trim_end().to_string();
                if !trimmed.is_empty() {
                    line_count += 1;

                    {
                        let mut info_guard = info.lock().await;
                        info_guard.line_count = line_count;
                    }

                    let _ = sender
                        .send(SourceEvent::Line {
                            source: name.clone(),
                            line: trimmed,
                        })
                        .await;
                }
            }
            Err(e) => {
                let _ = sender
                    .send(SourceEvent::Error {
                        source: name.clone(),
                        error: e.to_string(),
                    })
                    .await;
                break;
            }
        }
    }

    // Set up file watcher for new content
    let (notify_tx, mut notify_rx) = mpsc::channel(100);
    let path_for_watcher = path.clone();

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                if event.kind.is_modify() {
                    let _ = notify_tx.blocking_send(());
                }
            }
        },
        Config::default(),
    )?;

    watcher.watch(&path_for_watcher, RecursiveMode::NonRecursive)?;

    // Now tail for new lines
    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            _ = notify_rx.recv() => {
                // File was modified, read new lines
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break, // No more data
                        Ok(_) => {
                            let trimmed = line.trim_end().to_string();
                            if !trimmed.is_empty() {
                                line_count += 1;

                                {
                                    let mut info_guard = info.lock().await;
                                    info_guard.line_count = line_count;
                                }

                                let _ = sender.send(SourceEvent::Line {
                                    source: name.clone(),
                                    line: trimmed,
                                }).await;
                            }
                        }
                        Err(e) => {
                            let _ = sender.send(SourceEvent::Error {
                                source: name.clone(),
                                error: e.to_string(),
                            }).await;
                            break;
                        }
                    }
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
