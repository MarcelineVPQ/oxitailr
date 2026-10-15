//! Saved passwords, for the window: is one saved, reveal it (on an explicit
//! click only), forget it, list them. Values live in the engine's secret store
//! (the OS credential vault, or the encrypted file where there is none); the
//! window never receives one except through `reveal_secret`.
//!
//! The store can block (an OS vault may ask to be unlocked), so every call
//! runs on a blocking thread, never on the window's thread, and the engine's
//! lock is never held while the store is used.

use oxitailr_core::secrets::{ssh_password_id, ExposeSecret, SecretStore, SecretString};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

pub const DEFAULT_SSH_PORT: u16 = 22;

/// The engine's store, taken out from under the lock.
pub fn store(shared: &crate::Shared) -> Result<Arc<dyn SecretStore>, String> {
    shared
        .lock()
        .core
        .secret_store()
        .ok_or_else(|| "Saved passwords are turned off".to_string())
}

/// Run `f` against the store on a blocking thread.
pub async fn with_store<T: Send + 'static>(
    store: Arc<dyn SecretStore>,
    f: impl FnOnce(&dyn SecretStore) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(store.as_ref()))
        .await
        .map_err(|_| "The password store stopped unexpectedly".to_string())?
        // Store errors are already free of secret values (see oxitailr-core).
        .map_err(|e| format!("Password store: {e}"))
}

/// Save an SSH password for `user@host:port`.
pub async fn save_ssh_password(
    store: Arc<dyn SecretStore>,
    user: &str,
    host: &str,
    port: Option<u16>,
    password: SecretString,
) -> Result<(), String> {
    let id = ssh_password_id(user, host, port.unwrap_or(DEFAULT_SSH_PORT));
    with_store(store, move |s| s.set(&id, &password)).await
}

#[derive(Serialize)]
pub struct SavedSecret {
    id: String,
    /// What it's for, in words: "ssh  user@host:port", or "old  name".
    label: String,
    kind: &'static str,
}

fn describe(id: &str) -> SavedSecret {
    let (kind, label) = if let Some(rest) = id.strip_prefix("ssh:") {
        ("ssh", rest.to_string())
    } else if let Some(rest) = id.strip_prefix("legacy:") {
        ("legacy", format!("{rest} (from an older version)"))
    } else {
        ("other", id.to_string())
    };
    SavedSecret {
        id: id.to_string(),
        label,
        kind,
    }
}

/// Which backend holds the passwords: shown in Settings.
#[tauri::command]
pub async fn secret_backend(shared: State<'_, crate::Shared>) -> Result<String, String> {
    let store = store(&shared)?;
    Ok(store.backend().to_string())
}

/// Saved passwords, without their values. The store's internal vault key is
/// not listed.
#[tauri::command]
pub async fn saved_secrets(shared: State<'_, crate::Shared>) -> Result<Vec<SavedSecret>, String> {
    let store = store(&shared)?;
    let mut ids = with_store(store, |s| s.ids()).await?;
    ids.retain(|id| id != oxitailr_core::secrets::VAULT_KEY_ID);
    ids.sort();
    Ok(ids.iter().map(|id| describe(id)).collect())
}

/// Whether a password is saved for `user@host:port`.
#[tauri::command]
pub async fn ssh_password_saved(
    shared: State<'_, crate::Shared>,
    user: String,
    host: String,
    port: Option<u16>,
) -> Result<bool, String> {
    if user.trim().is_empty() || host.trim().is_empty() {
        return Ok(false);
    }
    let store = store(&shared)?;
    let id = ssh_password_id(user.trim(), host.trim(), port.unwrap_or(DEFAULT_SSH_PORT));
    with_store(store, move |s| Ok(s.get(&id)?.is_some())).await
}

/// The saved value, for the reveal (eye) button only. Each reveal is logged
/// by id, never by value.
#[tauri::command]
pub async fn reveal_secret(
    shared: State<'_, crate::Shared>,
    id: String,
) -> Result<Option<String>, String> {
    let store = store(&shared)?;
    tracing::info!("revealed saved password {id}");
    with_store(store, move |s| {
        Ok(s.get(&id)?.map(|v| v.expose_secret().to_string()))
    })
    .await
}

#[tauri::command]
pub async fn forget_secret(shared: State<'_, crate::Shared>, id: String) -> Result<(), String> {
    let store = store(&shared)?;
    with_store(store, move |s| s.delete(&id)).await
}

/// The id the window uses for an SSH password (so reveal/forget can target it).
#[tauri::command]
pub fn ssh_secret_id(user: String, host: String, port: Option<u16>) -> String {
    ssh_password_id(user.trim(), host.trim(), port.unwrap_or(DEFAULT_SSH_PORT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_ids_without_values() {
        let d = describe("ssh:me@host:22");
        assert_eq!((d.kind, d.label.as_str()), ("ssh", "me@host:22"));
        let d = describe("legacy:prod");
        assert_eq!(d.kind, "legacy");
        assert!(d.label.starts_with("prod"));
    }
}
