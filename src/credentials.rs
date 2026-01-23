//! Secure credential storage with proper encryption.
//!
//! Uses Argon2 for key derivation and AES-256-GCM for encryption.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::Result;
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

// Constants for encryption
pub const ENCRYPTION_KEY_SIZE: usize = 32;
pub const ENCRYPTION_NONCE_SIZE: usize = 12;

// Fixed application salt (combined with user-specific data for actual salt)
const APP_SALT_PREFIX: &[u8] = b"oxitailr-credential-storage-v2";

/// Global credentials cache
pub static CREDENTIALS_CACHE: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| {
    Mutex::new(load_credentials_from_file().unwrap_or_default())
});

/// Get the credentials file path
pub fn get_credentials_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oxitailr")
        .join("credentials.enc")
}

/// Derive encryption key from machine-specific data using Argon2
pub fn get_encryption_key() -> [u8; ENCRYPTION_KEY_SIZE] {
    // Collect machine-specific key material
    let mut key_material = String::new();

    // Add username
    if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        key_material.push_str(&user);
    }

    // Add home directory path
    if let Some(home) = dirs::home_dir() {
        key_material.push_str(&home.to_string_lossy());
    }

    // Combine app salt with key material to create the final salt
    let mut salt = Vec::with_capacity(APP_SALT_PREFIX.len() + key_material.len());
    salt.extend_from_slice(APP_SALT_PREFIX);
    salt.extend_from_slice(key_material.as_bytes());

    // Use Argon2 for key derivation
    let mut output_key = [0u8; ENCRYPTION_KEY_SIZE];
    let argon2 = Argon2::default();

    // Derive the key - using the combined material as both password and salt
    // This is acceptable since we're deriving from local machine data, not user passwords
    argon2
        .hash_password_into(key_material.as_bytes(), &salt, &mut output_key)
        .expect("Argon2 key derivation failed");

    output_key
}

/// Generate a cryptographically secure random nonce
pub fn generate_random_nonce() -> [u8; ENCRYPTION_NONCE_SIZE] {
    let mut nonce = [0u8; ENCRYPTION_NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce);
    nonce
}

/// Load credentials from encrypted file
pub fn load_credentials_from_file() -> Result<HashMap<String, String>> {
    let path = get_credentials_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let encrypted = std::fs::read_to_string(&path)?;
    let decoded = BASE64.decode(encrypted.trim())?;

    if decoded.len() < ENCRYPTION_NONCE_SIZE {
        return Ok(HashMap::new());
    }

    let (nonce_bytes, ciphertext) = decoded.split_at(ENCRYPTION_NONCE_SIZE);
    let key = get_encryption_key();
    let cipher = Aes256Gcm::new_from_slice(&key)?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed - credentials may be corrupted"))?;

    let json = String::from_utf8(plaintext)?;
    let creds: HashMap<String, String> = serde_json::from_str(&json)?;

    Ok(creds)
}

/// Save credentials to encrypted file with proper permissions
pub fn save_credentials_to_file(creds: &HashMap<String, String>) -> Result<()> {
    let path = get_credentials_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string(creds)?;
    let key = get_encryption_key();
    let cipher = Aes256Gcm::new_from_slice(&key)?;

    // Generate cryptographically secure random nonce
    let nonce_bytes = generate_random_nonce();
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, json.as_bytes())
        .map_err(|_| anyhow::anyhow!("Encryption failed"))?;

    // Prepend nonce to ciphertext
    let mut data = nonce_bytes.to_vec();
    data.extend(ciphertext);

    let encoded = BASE64.encode(&data);
    std::fs::write(&path, &encoded)?;

    // Set file permissions to 0600 on Unix systems
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, permissions)?;
    }

    Ok(())
}

/// Store an SSH password in encrypted local storage
pub fn store_ssh_password(source_name: &str, password: &str) -> Result<()> {
    let mut cache = CREDENTIALS_CACHE
        .lock()
        .map_err(|_| anyhow::anyhow!("Credential cache lock poisoned"))?;
    cache.insert(source_name.to_string(), password.to_string());
    save_credentials_to_file(&cache)?;
    Ok(())
}

/// Retrieve an SSH password from encrypted local storage
pub fn get_ssh_password(source_name: &str) -> Option<String> {
    let cache = CREDENTIALS_CACHE.lock().ok()?;
    cache.get(source_name).cloned()
}

/// Delete an SSH password from encrypted local storage
pub fn delete_ssh_password(source_name: &str) {
    if let Ok(mut cache) = CREDENTIALS_CACHE.lock() {
        cache.remove(source_name);
        let _ = save_credentials_to_file(&cache);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nonce_generation() {
        let nonce1 = generate_random_nonce();
        let nonce2 = generate_random_nonce();
        // Nonces should be different
        assert_ne!(nonce1, nonce2);
        // Nonces should be the correct size
        assert_eq!(nonce1.len(), ENCRYPTION_NONCE_SIZE);
    }

    #[test]
    fn test_key_derivation_deterministic() {
        let key1 = get_encryption_key();
        let key2 = get_encryption_key();
        // Same machine should produce same key
        assert_eq!(key1, key2);
    }
}
