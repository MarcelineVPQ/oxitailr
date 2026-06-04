use super::{Source, SourceEvent};
use crate::models::{SourceInfo, SourceStatus, SourceType};
use anyhow::{Context, Result};
use async_trait::async_trait;
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
    /// Bytes read past the last newline. We hold an unterminated trailing line
    /// here until its newline arrives, so a line written in two flushes isn't
    /// emitted as two separate entries.
    partial: String,
}

/// How often each source polls its file for new content. Bounds the read rate
/// regardless of how fast the file is written; 100ms is imperceptible latency
/// for a log viewer.
const POLL_INTERVAL_MS: u64 = 100;

/// Get the inode of a file (Unix)
#[cfg(unix)]
fn get_file_inode(path: &Path) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    Ok(std::fs::metadata(path)?.ino())
}

/// Get a pseudo-inode for Windows (using creation time only)
/// Note: This is a best-effort approach since Windows doesn't have true inodes
/// We only use creation_time because file_size changes as the file grows,
/// which would incorrectly trigger rotation detection
#[cfg(windows)]
fn get_file_inode(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::fs::MetadataExt;
    let meta = std::fs::metadata(path)?;
    // Use only creation time as pseudo-inode - this changes when file is replaced
    Ok(meta.creation_time())
}

/// Get the current size of a file
fn get_file_size(path: &Path) -> std::io::Result<u64> {
    Ok(std::fs::metadata(path)?.len())
}

/// Read from the reader's current position to EOF, emitting one `Line` event per
/// complete (newline-terminated) line. A trailing chunk with no newline is kept
/// in `file_state.partial` and prepended to the next read, so a line flushed in
/// two writes is not split into two entries.
async fn read_lines_to_eof(
    name: &str,
    info: &Arc<Mutex<SourceInfo>>,
    sender: &mpsc::Sender<SourceEvent>,
    reader: &mut BufReader<File>,
    file_state: &mut FileState,
    line_count: &mut u64,
) {
    // Collect all currently-available complete lines, then emit them as a
    // single batch — one channel send and one info lock for the whole read,
    // instead of per line.
    let mut batch: Vec<String> = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(n) => {
                file_state.last_position += n as u64;
                if line.ends_with('\n') {
                    let mut full = std::mem::take(&mut file_state.partial);
                    full.push_str(&line);
                    let trimmed = full.trim_end();
                    if !trimmed.is_empty() {
                        batch.push(trimmed.to_string());
                    }
                } else {
                    // Hit EOF mid-line; stash and wait for the rest to be written.
                    file_state.partial.push_str(&line);
                    break;
                }
            }
            Err(e) => {
                let _ = sender
                    .send(SourceEvent::Error {
                        source: name.to_string(),
                        error: e.to_string(),
                    })
                    .await;
                break;
            }
        }
    }

    if !batch.is_empty() {
        *line_count += batch.len() as u64;
        {
            let mut info_guard = info.lock().await;
            info_guard.line_count = *line_count;
        }
        let _ = sender
            .send(SourceEvent::Lines {
                source: name.to_string(),
                lines: batch,
            })
            .await;
    }
}

/// Emit any held partial line as a final entry and clear it. Used after the
/// initial historical read so files whose last line lacks a newline still show.
async fn flush_partial(
    name: &str,
    info: &Arc<Mutex<SourceInfo>>,
    sender: &mpsc::Sender<SourceEvent>,
    file_state: &mut FileState,
    line_count: &mut u64,
) {
    if file_state.partial.is_empty() {
        return;
    }
    let trimmed = std::mem::take(&mut file_state.partial)
        .trim_end()
        .to_string();
    if !trimmed.is_empty() {
        *line_count += 1;
        {
            let mut info_guard = info.lock().await;
            info_guard.line_count = *line_count;
        }
        let _ = sender
            .send(SourceEvent::Line {
                source: name.to_string(),
                line: trimmed,
            })
            .await;
    }
}

/// Detect rotation/truncation (reopening the file if needed), then read any new
/// content to EOF. Shared by both the notify-event path and the polling fallback.
async fn read_new_content(
    path: &Path,
    name: &str,
    info: &Arc<Mutex<SourceInfo>>,
    sender: &mpsc::Sender<SourceEvent>,
    reader: &mut BufReader<File>,
    file_state: &mut FileState,
    line_count: &mut u64,
) {
    // Check for log rotation.
    let rotation_detected = if let Ok(current_inode) = get_file_inode(path) {
        if current_inode != file_state.inode {
            // Inode changed - file was replaced (e.g., by logrotate).
            true
        } else if let Ok(current_size) = get_file_size(path) {
            // File truncated (size shrank below where we last read).
            if current_size < file_state.last_position {
                true
            } else {
                file_state.last_size = current_size;
                let mut info_guard = info.lock().await;
                info_guard.file_size = Some(current_size);
                false
            }
        } else {
            false
        }
    } else {
        // File might not exist yet after rotation; try again next tick.
        false
    };

    if rotation_detected {
        // Give the new file a moment to be ready.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        if let Ok(new_file) = File::open(path).await {
            let _ = sender
                .send(SourceEvent::Line {
                    source: name.to_string(),
                    line: "--- Log rotation detected, continuing with new file ---".to_string(),
                })
                .await;

            file_state.inode = get_file_inode(path).unwrap_or(0);
            file_state.last_size = get_file_size(path).unwrap_or(0);
            file_state.last_position = 0;
            file_state.partial.clear();

            *reader = BufReader::new(new_file);

            tracing::info!(
                "Log rotation detected for {}, reopened file",
                path.display()
            );
        }
    }

    read_lines_to_eof(name, info, sender, reader, file_state, line_count).await;
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
        // Synchronous snapshot; callers are off the runtime. Status updates also
        // reach the UI via `SourceEvent::StatusChange`.
        self.info.blocking_lock().clone()
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
            if let Err(e) = run_local_tail(
                path,
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

    // Initialize file state for rotation detection
    let initial_inode = get_file_inode(&path).unwrap_or(0);
    let initial_size = get_file_size(&path).unwrap_or(0);
    let mut file_state = FileState {
        inode: initial_inode,
        last_size: initial_size,
        last_position: 0,
        partial: String::new(),
    };

    {
        let mut info_guard = info.lock().await;
        info_guard.status = SourceStatus::Connected;
        info_guard.file_size = Some(initial_size);
        let _ = sender
            .send(SourceEvent::StatusChange {
                source: name.clone(),
                info: info_guard.clone(),
            })
            .await;
    }

    let mut reader = BufReader::new(file);
    let mut line_count: u64 = 0;

    // Read existing content from the file first.
    read_lines_to_eof(
        &name,
        &info,
        &sender,
        &mut reader,
        &mut file_state,
        &mut line_count,
    )
    .await;

    // Flush a trailing line with no terminating newline so static files (and any
    // historical content) display in full. During live tailing below we instead
    // hold partials until their newline arrives, to avoid splitting one logical
    // line into two entries.
    flush_partial(&name, &info, &sender, &mut file_state, &mut line_count).await;

    // Tail by polling at a fixed cadence. Each poll batch-reads everything that
    // accumulated since the last one, so the read rate is bounded regardless of
    // how fast the file is written (a filesystem-notify watcher fires once per
    // write — thousands of times a second on a busy log — which storms the CPU
    // for no benefit, since we coalesce reads anyway). Rotation/truncation is
    // detected by the inode/size checks in read_new_content.
    let mut poll_interval =
        tokio::time::interval(std::time::Duration::from_millis(POLL_INTERVAL_MS));
    poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    break;
                }
            }
            _ = poll_interval.tick() => {
                read_new_content(&path, &name, &info, &sender, &mut reader, &mut file_state, &mut line_count).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;
    use tokio::time::timeout;

    /// Wait for the next emitted line, flattening `Lines` batches and skipping
    /// status/error events. `pending` carries leftover batch lines between calls.
    async fn next_line(
        rx: &mut mpsc::Receiver<SourceEvent>,
        pending: &mut std::collections::VecDeque<String>,
        dur: Duration,
    ) -> Option<String> {
        loop {
            if let Some(line) = pending.pop_front() {
                return Some(line);
            }
            match timeout(dur, rx.recv()).await {
                Ok(Some(SourceEvent::Line { line, .. })) => return Some(line),
                Ok(Some(SourceEvent::Lines { lines, .. })) => {
                    pending.extend(lines);
                }
                Ok(Some(_)) => continue, // ignore StatusChange / Error
                Ok(None) | Err(_) => return None,
            }
        }
    }

    /// The polling fallback must surface appended lines even if no notify event
    /// fires — this is the core "behaves like a tail logger" guarantee.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn polling_picks_up_appended_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "first line\n").unwrap();

        let mut source = LocalFileSource::new("test".to_string(), path.clone());
        let (tx, mut rx) = mpsc::channel(100);
        let mut pending = std::collections::VecDeque::new();
        source.start(tx).await.unwrap();

        assert_eq!(
            next_line(&mut rx, &mut pending, Duration::from_secs(2))
                .await
                .as_deref(),
            Some("first line")
        );

        // Append after the source is running.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "second line").unwrap();
        }

        assert_eq!(
            next_line(&mut rx, &mut pending, Duration::from_secs(2))
                .await
                .as_deref(),
            Some("second line")
        );

        source.stop().await.unwrap();
    }

    /// A line flushed in two writes (no intervening newline) must arrive as one
    /// entry, not be split across the partial-read boundary.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn partial_line_is_not_split() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "").unwrap();

        let mut source = LocalFileSource::new("test".to_string(), path.clone());
        let (tx, mut rx) = mpsc::channel(100);
        let mut pending = std::collections::VecDeque::new();
        source.start(tx).await.unwrap();

        // Let the initial (empty) read complete before writing, so the fragment
        // is seen by the live-tail path rather than the historical read (which
        // intentionally flushes a trailing unterminated line).
        tokio::time::sleep(Duration::from_millis(400)).await;

        // First flush: a fragment with no newline.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            write!(f, "hello ").unwrap();
            f.flush().unwrap();
        }
        // Let at least one poll observe the fragment.
        tokio::time::sleep(Duration::from_millis(400)).await;
        // Second flush completes the line.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "world").unwrap();
        }

        assert_eq!(
            next_line(&mut rx, &mut pending, Duration::from_secs(2))
                .await
                .as_deref(),
            Some("hello world")
        );

        source.stop().await.unwrap();
    }
}
