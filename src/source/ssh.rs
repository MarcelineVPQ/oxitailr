use super::{Source, SourceEvent};
use crate::models::{SourceInfo, SourceStatus, SourceType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use russh::client;
use russh_keys::key::PublicKey;
use russh_keys::load_secret_key;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

/// Store for known SSH host keys
#[derive(Debug, Clone, Default)]
pub struct KnownHostsStore {
    /// Map of "host:port" -> list of accepted public key fingerprints
    hosts: HashMap<String, Vec<String>>,
}

impl KnownHostsStore {
    /// Load known hosts from the default ~/.ssh/known_hosts file
    pub fn load_default() -> Self {
        let known_hosts_path = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".ssh")
            .join("known_hosts");

        Self::load_from_file(&known_hosts_path)
    }

    /// Load known hosts from a specific file
    pub fn load_from_file(path: &PathBuf) -> Self {
        let mut store = Self::default();

        if !path.exists() {
            tracing::warn!("known_hosts file not found at {}", path.display());
            return store;
        }

        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to open known_hosts: {}", e);
                return store;
            }
        };

        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            // Skip comments and empty lines
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse known_hosts format: hostname[,hostname]... key-type base64-key [comment]
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let hostnames = parts[0];
                let key_type = parts[1];
                let key_data = parts[2];

                // Create a fingerprint from key type and data
                let fingerprint = format!("{}:{}", key_type, key_data);

                // Handle multiple hostnames (comma-separated)
                for hostname in hostnames.split(',') {
                    // Handle hashed hostnames (|1|...) by skipping them
                    // We only support plain hostnames for now
                    if hostname.starts_with('|') {
                        continue;
                    }

                    // Normalize hostname (remove brackets for IPv6, handle port)
                    let normalized = normalize_hostname(hostname);
                    store
                        .hosts
                        .entry(normalized)
                        .or_default()
                        .push(fingerprint.clone());
                }
            }
        }

        tracing::info!(
            "Loaded {} known hosts from {}",
            store.hosts.len(),
            path.display()
        );
        store
    }

    /// Check if a host key is known
    pub fn is_known(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let host_key = format_host_key(host, port);
        let key_fingerprint = format_key_fingerprint(key);

        // Also check without port for default SSH port
        let host_keys = if port == 22 {
            vec![host_key.clone(), host.to_string()]
        } else {
            vec![host_key]
        };

        for hk in &host_keys {
            if let Some(known_keys) = self.hosts.get(hk) {
                for known_fp in known_keys {
                    if *known_fp == key_fingerprint {
                        return HostKeyStatus::Known;
                    }
                }
                // Host is known but key doesn't match - potential MITM!
                return HostKeyStatus::Changed;
            }
        }

        HostKeyStatus::Unknown
    }
}

/// Status of host key verification
#[derive(Debug, Clone, PartialEq)]
pub enum HostKeyStatus {
    /// Host key is known and matches
    Known,
    /// Host is known but key has changed (potential MITM attack)
    Changed,
    /// Host is not in known_hosts
    Unknown,
}

fn normalize_hostname(hostname: &str) -> String {
    // Remove brackets from [host]:port format
    hostname
        .trim_start_matches('[')
        .replace("]:", ":")
        .to_string()
}

fn format_host_key(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{}]:{}", host, port)
    }
}

fn format_key_fingerprint(key: &PublicKey) -> String {
    let key_type = match key {
        PublicKey::Ed25519(_) => "ssh-ed25519",
        PublicKey::RSA { .. } => "ssh-rsa",
        _ => "unknown",
    };

    // Get the key data as base64
    let key_data = match key {
        PublicKey::Ed25519(k) => {
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, k.as_bytes())
        }
        PublicKey::RSA { ref key, .. } => {
            // For RSA, we'd need to serialize the full key - simplified here
            format!("{:?}", key)
        }
        _ => String::new(),
    };

    format!("{}:{}", key_type, key_data)
}

pub struct SshSource {
    name: String,
    host: String,
    port: u16,
    user: String,
    path: String,
    key_path: Option<PathBuf>,
    password: Option<String>,
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
            password: None,
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

    pub fn with_password(mut self, password: String) -> Self {
        self.password = Some(password);
        self
    }
}

struct SshHandler {
    host: String,
    port: u16,
    known_hosts: KnownHostsStore,
}

impl SshHandler {
    fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            known_hosts: KnownHostsStore::load_default(),
        }
    }
}

#[async_trait]
impl client::Handler for SshHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match self
            .known_hosts
            .is_known(&self.host, self.port, server_public_key)
        {
            HostKeyStatus::Known => {
                tracing::info!("SSH host key verified for {}:{}", self.host, self.port);
                Ok(true)
            }
            HostKeyStatus::Changed => {
                tracing::error!(
                    "SSH HOST KEY CHANGED for {}:{}! This could indicate a man-in-the-middle attack.",
                    self.host,
                    self.port
                );
                Err(anyhow::anyhow!(
                    "SSH host key has changed for {}:{}. This could indicate a security threat. \
                     If you trust this host, remove the old key from ~/.ssh/known_hosts and reconnect.",
                    self.host,
                    self.port
                ))
            }
            HostKeyStatus::Unknown => {
                // For now, we'll accept unknown hosts with a warning
                // In a production app, you'd want to prompt the user to accept
                tracing::warn!(
                    "SSH host {}:{} is not in known_hosts. Accepting key on first use.",
                    self.host,
                    self.port
                );
                // TODO: Add the key to known_hosts with user confirmation via UI
                Ok(true)
            }
        }
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
        let password = self.password.clone();

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
                password,
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

#[allow(clippy::too_many_arguments)]
async fn run_ssh_tail(
    host: String,
    port: u16,
    user: String,
    path: String,
    key_path: Option<PathBuf>,
    password: Option<String>,
    name: String,
    info: Arc<Mutex<SourceInfo>>,
    sender: mpsc::Sender<SourceEvent>,
    stop_rx: &mut watch::Receiver<bool>,
) -> Result<()> {
    let config = client::Config::default();
    let config = Arc::new(config);
    let handler = SshHandler::new(host.clone(), port);

    let mut session = client::connect(config, (host.as_str(), port), handler)
        .await
        .context("Failed to connect to SSH server")?;

    // Try to authenticate
    let mut authenticated = false;

    // Get SSH directory
    let ssh_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ssh");

    // Build list of keys to try
    let mut keys_to_try: Vec<PathBuf> = Vec::new();

    // If a specific key path was provided, try it first
    if let Some(ref kp) = key_path {
        keys_to_try.push(kp.clone());
    }

    // Add default key locations
    keys_to_try.push(ssh_dir.join("id_ed25519"));
    keys_to_try.push(ssh_dir.join("id_rsa"));
    keys_to_try.push(ssh_dir.join("id_ecdsa"));

    // Try each key
    for key_file in &keys_to_try {
        if key_file.exists() {
            if let Ok(key) = load_secret_key(key_file, None) {
                if let Ok(true) = session.authenticate_publickey(&user, Arc::new(key)).await {
                    authenticated = true;
                    tracing::info!("SSH authenticated with key: {}", key_file.display());
                    break;
                }
            }
        }
    }

    // If key auth failed and password is provided, try password auth
    if !authenticated {
        if let Some(ref pwd) = password {
            if !pwd.is_empty() {
                if let Ok(true) = session.authenticate_password(&user, pwd).await {
                    authenticated = true;
                    tracing::info!("SSH authenticated with password");
                }
            }
        }
    }

    if !authenticated {
        let tried_keys: Vec<String> = keys_to_try
            .iter()
            .filter(|k| k.exists())
            .map(|k| k.display().to_string())
            .collect();
        let key_info = if tried_keys.is_empty() {
            "No SSH keys found".to_string()
        } else {
            format!("Tried keys: {}", tried_keys.join(", "))
        };
        let pwd_info = if password.is_some() {
            ", also tried password"
        } else {
            ""
        };
        anyhow::bail!("SSH authentication failed. {}{}", key_info, pwd_info);
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
    // Use shlex to properly escape the path to prevent command injection
    let escaped_path = shlex::try_quote(&path)
        .map_err(|_| anyhow::anyhow!("Invalid path: contains null bytes"))?;
    // Use tail -n +1 to read from the beginning, -F to follow (and handle rotation)
    let command = format!("tail -n +1 -F {}", escaped_path);
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
                        let bytes_len = data.len() as u64;
                        buffer.push_str(&String::from_utf8_lossy(&data));

                        while let Some(pos) = buffer.find('\n') {
                            let line = buffer[..pos].to_string();
                            buffer = buffer[pos + 1..].to_string();

                            if !line.trim().is_empty() {
                                line_count += 1;

                                let _ = sender.send(SourceEvent::Line {
                                    source: name.clone(),
                                    line: line.trim_end().to_string(),
                                }).await;
                            }
                        }

                        // Update info and send status change so main app sees file size
                        {
                            let mut info_guard = info.lock().await;
                            info_guard.line_count = line_count;
                            let current = info_guard.file_size.unwrap_or(0);
                            info_guard.file_size = Some(current + bytes_len);
                            let _ = sender.send(SourceEvent::StatusChange {
                                source: name.clone(),
                                info: info_guard.clone(),
                            }).await;
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
