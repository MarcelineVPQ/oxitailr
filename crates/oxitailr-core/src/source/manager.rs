use super::{LocalFileSource, Source, SourceCommand, SourceEvent, SshSource};
use crate::secrets::{saved_ssh_password, SecretLookup, SecretString};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct SourceManager {
    sources: HashMap<String, Box<dyn Source>>,
    event_tx: mpsc::Sender<SourceEvent>,
    event_rx: Option<mpsc::Receiver<SourceEvent>>,
    /// Where an SSH source added without a password looks for a saved one.
    secrets: SecretLookup,
}

impl SourceManager {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        Self {
            sources: HashMap::new(),
            event_tx,
            event_rx: Some(event_rx),
            secrets: SecretLookup::Disabled,
        }
    }

    pub fn with_secrets(mut self, secrets: SecretLookup) -> Self {
        self.secrets = secrets;
        self
    }

    /// The saved password for an SSH source, looked up off the async workers
    /// (keychain calls block on D-Bus / the OS and may show an unlock prompt).
    async fn saved_password(
        &self,
        name: &str,
        user: &str,
        host: &str,
        port: u16,
    ) -> Option<SecretString> {
        if matches!(self.secrets, SecretLookup::Disabled) {
            return None;
        }
        let secrets = self.secrets.clone();
        let (name, user, host) = (name.to_string(), user.to_string(), host.to_string());
        tokio::task::spawn_blocking(move || {
            let store = secrets.resolve()?;
            saved_ssh_password(store.as_ref(), &name, &user, &host, port)
        })
        .await
        .ok()
        .flatten()
    }

    pub fn add_local_source(&mut self, name: String, path: PathBuf) {
        let source = LocalFileSource::new(name.clone(), path);
        self.sources.insert(name, Box::new(source));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_ssh_source(
        &mut self,
        name: String,
        host: String,
        user: String,
        path: String,
        port: Option<u16>,
        key_path: Option<PathBuf>,
        password: Option<SecretString>,
    ) {
        let mut source = SshSource::new(name.clone(), host, user, path);
        if let Some(p) = port {
            source = source.with_port(p);
        }
        if let Some(k) = key_path {
            source = source.with_key_path(k);
        }
        if let Some(pwd) = password {
            source = source.with_secret_password(pwd);
        }
        self.sources.insert(name, Box::new(source));
    }

    pub fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<SourceEvent>> {
        self.event_rx.take()
    }

    pub async fn start_source(&mut self, name: &str) -> Result<()> {
        if let Some(source) = self.sources.get_mut(name) {
            source.start(self.event_tx.clone()).await?;
        }
        Ok(())
    }

    pub async fn stop_source(&mut self, name: &str) -> Result<()> {
        if let Some(source) = self.sources.get_mut(name) {
            source.stop().await?;
        }
        Ok(())
    }

    /// Take ownership of the manager and process [`SourceCommand`]s until the
    /// channel closes. Run this on the async runtime so all source start/stop
    /// work stays off the UI thread.
    pub async fn run(mut self, mut commands: mpsc::UnboundedReceiver<SourceCommand>) {
        while let Some(cmd) = commands.recv().await {
            match cmd {
                SourceCommand::AddLocal { name, path } => {
                    self.add_local_source(name.clone(), path);
                    if let Err(e) = self.start_source(&name).await {
                        tracing::error!("Failed to start source '{}': {}", name, e);
                    }
                }
                SourceCommand::AddSsh {
                    name,
                    host,
                    user,
                    path,
                    port,
                    key_path,
                    password,
                } => {
                    let password = match password {
                        Some(p) => Some(p),
                        None => {
                            self.saved_password(&name, &user, &host, port.unwrap_or(22))
                                .await
                        }
                    };
                    self.add_ssh_source(name.clone(), host, user, path, port, key_path, password);
                    if let Err(e) = self.start_source(&name).await {
                        tracing::error!("Failed to start source '{}': {}", name, e);
                    }
                }
                SourceCommand::Remove { name } => {
                    let _ = self.stop_source(&name).await;
                }
                SourceCommand::Reload { names } => {
                    for name in &names {
                        let _ = self.stop_source(name).await;
                    }
                    for name in &names {
                        let _ = self.start_source(name).await;
                    }
                }
                SourceCommand::StopAll { names } => {
                    for name in &names {
                        let _ = self.stop_source(name).await;
                    }
                }
            }
        }
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}
