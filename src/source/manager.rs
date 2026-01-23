use super::{LocalFileSource, Source, SourceEvent, SshSource};
use crate::models::SourceInfo;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub struct SourceManager {
    sources: HashMap<String, Box<dyn Source>>,
    event_tx: mpsc::Sender<SourceEvent>,
    event_rx: Option<mpsc::Receiver<SourceEvent>>,
}

impl SourceManager {
    pub fn new() -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        Self {
            sources: HashMap::new(),
            event_tx,
            event_rx: Some(event_rx),
        }
    }

    pub fn add_local_source(&mut self, name: String, path: PathBuf) {
        let source = LocalFileSource::new(name.clone(), path);
        self.sources.insert(name, Box::new(source));
    }

    pub fn add_ssh_source(
        &mut self,
        name: String,
        host: String,
        user: String,
        path: String,
        port: Option<u16>,
        key_path: Option<PathBuf>,
    ) {
        let mut source = SshSource::new(name.clone(), host, user, path);
        if let Some(p) = port {
            source = source.with_port(p);
        }
        if let Some(k) = key_path {
            source = source.with_key_path(k);
        }
        self.sources.insert(name, Box::new(source));
    }

    pub fn take_event_receiver(&mut self) -> Option<mpsc::Receiver<SourceEvent>> {
        self.event_rx.take()
    }

    pub async fn start_all(&mut self) -> Result<()> {
        for source in self.sources.values_mut() {
            source.start(self.event_tx.clone()).await?;
        }
        Ok(())
    }

    pub async fn start_source(&mut self, name: &str) -> Result<()> {
        if let Some(source) = self.sources.get_mut(name) {
            source.start(self.event_tx.clone()).await?;
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) -> Result<()> {
        for source in self.sources.values_mut() {
            source.stop().await?;
        }
        Ok(())
    }

    pub async fn stop_source(&mut self, name: &str) -> Result<()> {
        if let Some(source) = self.sources.get_mut(name) {
            source.stop().await?;
        }
        Ok(())
    }

    pub fn get_source_info(&self, name: &str) -> Option<SourceInfo> {
        self.sources.get(name).map(|s| s.info())
    }

    pub fn get_all_source_info(&self) -> Vec<SourceInfo> {
        self.sources.values().map(|s| s.info()).collect()
    }

    pub fn source_names(&self) -> Vec<String> {
        self.sources.keys().cloned().collect()
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}
