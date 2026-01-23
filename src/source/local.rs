use super::{Source, SourceEvent};
use crate::models::{SourceInfo, SourceStatus, SourceType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch, Mutex};

/// Tracks file state for rotation detection
struct FileState {
    inode: u64,
    last_size: u64,
    last_position: u64,
}

/// Get the inode of a file (Unix)
#[cfg(unix)]
fn get_file_inode(path: &Path) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path)?.ino())
}

/// Get a pseudo-inode for Windows (using file size + creation time as a proxy)
/// Note: This is a best-effort approach since Windows doesn't have true inodes
#[cfg(windows)]
fn get_file_inode(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::fs::MetadataExt;
    let meta = std::fs::metadata(path)?;
    // Combine creation time and file size as a pseudo-inode
    // This will detect most rotation scenarios where a new file is created
    Ok(meta.creation_time() ^ (meta.file_size() << 32))
}

/// Get the current size of a file
fn get_file_size(path: &Path) -> std::io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

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

    // Initialize file state for rotation detection
    let initial_inode = get_file_inode(&path).unwrap_or(0);
    let mut file_state = FileState {
        inode: initial_inode,
        last_size: get_file_size(&path).unwrap_or(0),
        last_position: 0,
    };

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut line_count: u64 = 0;

    // Read existing content from the file first
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // End of file
            Ok(n) => {
                file_state.last_position += n as u64;
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
                if event.kind.is_modify() || event.kind.is_create() {
                    let _ = notify_tx.blocking_send(());
                }
            }
        },
        Config::default(),
    )?;

    // Watch the parent directory to detect file recreation
    if let Some(parent) = path_for_watcher.parent() {
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
    } else {
        watcher.watch(&path_for_watcher, RecursiveMode::NonRecursive)?;
    }

    // Now tail for new lines
    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            _ = notify_rx.recv() => {
                // Check for log rotation
                let rotation_detected = if let Ok(current_inode) = get_file_inode(&path) {
                    if current_inode != file_state.inode {
                        // Inode changed - file was replaced (e.g., by logrotate)
                        true
                    } else if let Ok(current_size) = get_file_size(&path) {
                        // Check if file was truncated (size < last position)
                        if current_size < file_state.last_position {
                            true
                        } else {
                            file_state.last_size = current_size;
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    // File might not exist yet after rotation, wait for it
                    false
                };

                if rotation_detected {
                    // Wait a bit for the new file to be ready
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                    // Try to open the new file
                    if let Ok(new_file) = File::open(&path).await {
                        // Send rotation notification
                        let _ = sender
                            .send(SourceEvent::Line {
                                source: name.clone(),
                                line: "--- Log rotation detected, continuing with new file ---".to_string(),
                            })
                            .await;

                        // Update file state
                        file_state.inode = get_file_inode(&path).unwrap_or(0);
                        file_state.last_size = get_file_size(&path).unwrap_or(0);
                        file_state.last_position = 0;

                        // Create new reader
                        reader = BufReader::new(new_file);

                        tracing::info!("Log rotation detected for {}, reopened file", path.display());
                    }
                }

                // Read new lines
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break, // No more data
                        Ok(n) => {
                            file_state.last_position += n as u64;
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
