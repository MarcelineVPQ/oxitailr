//! Session state persistence.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Session state for persistence across restarts
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub open_local_files: Vec<String>,
    pub open_ssh_sources: Vec<String>,
    pub timestamp: String,
}

/// Saved local file source for auto_open feature
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedLocalSource {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub auto_open: bool,
}

/// Load session state from file
pub fn load_session(session_path: &PathBuf) -> Option<SessionState> {
    if !session_path.exists() {
        return None;
    }

    match std::fs::read_to_string(session_path) {
        Ok(content) => match serde_json::from_str::<SessionState>(&content) {
            Ok(session) => {
                tracing::info!("Restoring session from {}", session.timestamp);
                Some(session)
            }
            Err(e) => {
                tracing::warn!("Failed to parse session file: {}", e);
                None
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read session file: {}", e);
            None
        }
    }
}

/// Save session state to file
pub fn save_session(
    session_path: &PathBuf,
    open_local_files: Vec<String>,
    open_ssh_sources: Vec<String>,
) {
    let session = SessionState {
        open_local_files,
        open_ssh_sources,
        timestamp: chrono::Local::now().to_rfc3339(),
    };

    if let Some(parent) = session_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match serde_json::to_string_pretty(&session) {
        Ok(content) => {
            if let Err(e) = std::fs::write(session_path, content) {
                tracing::error!("Failed to save session: {}", e);
            } else {
                tracing::info!(
                    "Session saved with {} local files and {} SSH sources",
                    session.open_local_files.len(),
                    session.open_ssh_sources.len()
                );
            }
        }
        Err(e) => {
            tracing::error!("Failed to serialize session: {}", e);
        }
    }
}

/// Load saved local sources from file
pub fn load_local_sources(path: &PathBuf) -> Vec<SavedLocalSource> {
    if !path.exists() {
        return Vec::new();
    }

    match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<Vec<SavedLocalSource>>(&content) {
            Ok(sources) => {
                tracing::info!("Loaded {} local sources from {}", sources.len(), path.display());
                sources
            }
            Err(e) => {
                tracing::warn!("Failed to parse local sources: {}", e);
                Vec::new()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read local sources file: {}", e);
            Vec::new()
        }
    }
}

/// Save local sources to file
pub fn save_local_sources(path: &PathBuf, sources: &[SavedLocalSource]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match serde_json::to_string_pretty(sources) {
        Ok(content) => {
            if let Err(e) = std::fs::write(path, content) {
                tracing::error!("Failed to save local sources: {}", e);
            }
        }
        Err(e) => {
            tracing::error!("Failed to serialize local sources: {}", e);
        }
    }
}
