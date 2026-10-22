//! Headless application core: owns the log buffer, ingestion, sources, filters,
//! and alerts. Frontend-agnostic — no terminal or GUI types live here, so the
//! same core drives the TUI (and could drive anything else).

use crate::alert::{AlertDispatcher, AlertEvent, AlertRule};
use crate::config::{AlertConfig, AppConfig, HighlightConfig, SourceConfig};
use crate::filter::{FilterEngine, FilterRule};
use crate::models::{LogEntry, LogLevel, SourceInfo, SourceType};
use crate::parser::{JsonParser, Parser, PlainParser};
use crate::source::{SourceCommand, SourceEvent, SourceManager};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

/// A compiled highlight rule. The color stays as raw RGB so this module keeps
/// no dependency on any frontend's color type; the renderer converts it.
pub struct HighlightMatcher {
    /// Compiled regex when the rule is a regex; `None` for substring rules.
    pub regex: Option<regex::Regex>,
    /// Lowercased needle for case-insensitive substring rules.
    pub needle_lower: String,
    pub color: [u8; 3],
}

impl HighlightMatcher {
    fn compile(cfg: &HighlightConfig) -> Option<Self> {
        let regex = if cfg.regex {
            Some(regex::Regex::new(&cfg.pattern).ok()?)
        } else {
            None
        };
        Some(Self {
            regex,
            needle_lower: cfg.pattern.to_lowercase(),
            color: cfg.color,
        })
    }
}

/// A buffered log line. ANSI color spans are parsed lazily at render time
/// (only for visible lines that contain escape codes), so this is cheap to
/// build for every ingested line.
pub struct DisplayLine {
    pub entry: LogEntry,
    pub has_ansi: bool,
    pub line_num: usize,
}

impl DisplayLine {
    pub fn from_entry(entry: LogEntry, line_num: usize) -> Self {
        let has_ansi = entry.raw.contains('\x1b');
        Self {
            entry,
            has_ansi,
            line_num,
        }
    }
}

/// Ring buffer of log lines, capped at `max_lines`.
pub struct LogState {
    pub lines: VecDeque<Arc<DisplayLine>>,
    max_lines: usize,
    pub total_lines_read: usize,
    /// Bumped on every mutation; a cheap "did the buffer change" signal.
    pub version: u64,
}

impl LogState {
    fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(max_lines.min(64)),
            max_lines,
            total_lines_read: 0,
            version: 0,
        }
    }

    fn add_entry(&mut self, entry: LogEntry) -> Arc<DisplayLine> {
        self.total_lines_read += 1;
        let display_line = Arc::new(DisplayLine::from_entry(entry, self.total_lines_read));
        if self.lines.len() >= self.max_lines {
            self.lines.pop_front();
        }
        self.lines.push_back(Arc::clone(&display_line));
        self.version += 1;
        display_line
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.total_lines_read = 0;
        self.version += 1;
    }
}

/// Convert a config alert into a runtime [`AlertRule`].
pub fn convert_alert_config(cfg: &AlertConfig) -> Option<AlertRule> {
    Some(AlertRule {
        name: cfg.name.clone(),
        pattern: FilterRule::Regex {
            pattern: cfg.pattern.clone(),
        },
        actions: cfg.actions.clone(),
        cooldown_seconds: cfg.cooldown_seconds,
    })
}

/// The six log levels, in severity order, matched to the `show_levels` mask.
const LEVELS: [LogLevel; 6] = [
    LogLevel::Trace,
    LogLevel::Debug,
    LogLevel::Info,
    LogLevel::Warn,
    LogLevel::Error,
    LogLevel::Fatal,
];

pub struct AppCore {
    pub log_state: LogState,
    pub filter_engine: FilterEngine,
    pub filter_text: String,
    pub filter_error: Option<String>,
    /// Per-level visibility, indexed like [`LEVELS`].
    pub show_levels: [bool; 6],
    pub config: AppConfig,
    pub source_infos: HashMap<String, SourceInfo>,
    pub source_cmd_tx: mpsc::UnboundedSender<SourceCommand>,
    pub pending_alerts: Vec<AlertEvent>,
    alert_dispatcher: Arc<AlertDispatcher>,
    alert_rules: Vec<AlertRule>,
    runtime: Arc<Runtime>,
    plain_parser: PlainParser,
    json_parser: JsonParser,
    /// Compiled highlight rules from config (applied at render time).
    pub highlights: Vec<HighlightMatcher>,
    /// True when nothing was opened from the CLI or config auto-open — the
    /// frontend uses this to decide whether to restore the previous session.
    pub started_empty: bool,
}

/// Channel receivers handed to the frontend event loop (kept out of [`AppCore`]
/// so the loop can `select!` on them without borrowing the whole core).
pub struct CoreChannels {
    pub events: mpsc::Receiver<SourceEvent>,
    pub alerts: mpsc::Receiver<AlertEvent>,
}

impl AppCore {
    pub fn new(
        runtime: Arc<Runtime>,
        config: AppConfig,
        initial_files: Vec<PathBuf>,
    ) -> (Self, CoreChannels) {
        let buffer_size = config.general.buffer_size;

        // Sources run on the async runtime, driven by a command channel.
        let mut source_manager = SourceManager::new();
        let event_rx = source_manager
            .take_event_receiver()
            .expect("source receiver available once");
        let (source_cmd_tx, source_cmd_rx) = mpsc::unbounded_channel();
        runtime.spawn(source_manager.run(source_cmd_rx));

        // Alerts.
        let (alert_dispatcher, alert_rx) = AlertDispatcher::new();
        let alert_dispatcher = Arc::new(alert_dispatcher);
        let alert_rules: Vec<AlertRule> = config
            .alerts
            .iter()
            .filter_map(convert_alert_config)
            .collect();
        {
            let dispatcher = alert_dispatcher.clone();
            let rules = alert_rules.clone();
            runtime.spawn(async move {
                for rule in rules {
                    dispatcher.add_rule(rule).await;
                }
            });
        }

        let mut core = Self {
            log_state: LogState::new(buffer_size),
            filter_engine: FilterEngine::new(),
            filter_text: String::new(),
            filter_error: None,
            show_levels: [true; 6],
            config,
            source_infos: HashMap::new(),
            source_cmd_tx,
            pending_alerts: Vec::new(),
            alert_dispatcher,
            alert_rules,
            runtime,
            plain_parser: PlainParser::new(),
            json_parser: JsonParser::new(),
            highlights: Vec::new(),
            started_empty: true,
        };

        core.highlights = core
            .config
            .highlights
            .iter()
            .filter_map(HighlightMatcher::compile)
            .collect();

        let had_cli_files = !initial_files.is_empty();
        for path in initial_files {
            core.add_local_source(path);
        }
        let sources = core.config.sources.clone();
        let mut opened_config = false;
        for source_config in sources {
            // A source opens on startup only when it is both enabled and marked
            // auto_open — matching the egui app's behavior.
            if source_config.is_enabled() && source_config.auto_open() {
                core.add_source_from_config(source_config);
                opened_config = true;
            }
        }
        core.started_empty = !had_cli_files && !opened_config;

        (
            core,
            CoreChannels {
                events: event_rx,
                alerts: alert_rx,
            },
        )
    }

    pub fn add_local_source(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "local".to_string());
        let _ = self
            .source_cmd_tx
            .send(SourceCommand::AddLocal { name, path });
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
        password: Option<String>,
    ) {
        let _ = self.source_cmd_tx.send(SourceCommand::AddSsh {
            name,
            host,
            user,
            path,
            port,
            key_path,
            password,
        });
    }

    fn add_source_from_config(&mut self, source_config: SourceConfig) {
        let cmd = match source_config {
            SourceConfig::Local { name, path, .. } => SourceCommand::AddLocal {
                name,
                path: PathBuf::from(path),
            },
            SourceConfig::Ssh {
                name,
                host,
                port,
                user,
                path,
                key_path,
                ..
            } => SourceCommand::AddSsh {
                name,
                host,
                user,
                path,
                port: Some(port),
                key_path,
                password: None,
            },
        };
        let _ = self.source_cmd_tx.send(cmd);
    }

    /// Stop and restart every source, re-reading each file from the start.
    pub fn reload(&mut self) {
        self.log_state.clear();
        let names: Vec<String> = self.source_infos.keys().cloned().collect();
        let _ = self.source_cmd_tx.send(SourceCommand::Reload { names });
    }

    /// Paths of the currently open local sources, for session persistence.
    pub fn open_local_paths(&self) -> Vec<String> {
        self.source_infos
            .values()
            .filter(|i| i.source_type == SourceType::Local)
            .map(|i| i.path.clone())
            .collect()
    }

    /// Names of the configured filter presets (`[filters.<name>]`), sorted.
    pub fn preset_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.config.filters.keys().cloned().collect();
        names.sort();
        names
    }

    /// Apply a named filter preset from config, replacing the current include/
    /// exclude rules (the live text filter is left untouched). Returns false if
    /// no preset by that name exists.
    pub fn apply_filter_preset(&mut self, name: &str) -> bool {
        let Some(cfg) = self.config.filters.get(name).cloned() else {
            return false;
        };
        self.filter_engine.clear_rules();
        for pattern in &cfg.include {
            self.filter_engine
                .add_include_rule(FilterRule::regex(pattern));
        }
        for pattern in &cfg.exclude {
            self.filter_engine
                .add_exclude_rule(FilterRule::regex(pattern));
        }
        for rule in cfg.rules {
            self.filter_engine.add_include_rule(rule);
        }
        true
    }

    /// Clear any applied preset (include/exclude rules).
    pub fn clear_filter_rules(&mut self) {
        self.filter_engine.clear_rules();
    }

    /// Recompile the live filter from `filter_text`.
    pub fn update_filter(&mut self) {
        let text = self.filter_text.clone();
        self.filter_engine
            .set_live_filter(if text.is_empty() { None } else { Some(text) });
        self.filter_error = if self.filter_text.is_empty() {
            None
        } else {
            match regex::Regex::new(&self.filter_text) {
                Ok(_) => None,
                Err(e) => Some(format!("{}", e)),
            }
        };
    }

    /// Ingest one source event. Returns true if the buffer/status changed (i.e.
    /// the screen should be redrawn).
    pub fn ingest(&mut self, event: SourceEvent) -> bool {
        match event {
            SourceEvent::Line { source, line } => {
                self.ingest_line(&source, line);
                true
            }
            SourceEvent::Lines { source, lines } => {
                for line in lines {
                    self.ingest_line(&source, line);
                }
                true
            }
            SourceEvent::StatusChange { source, info } => {
                self.source_infos.insert(source, info);
                true
            }
            SourceEvent::Error { source, error } => {
                tracing::error!("Source {} error: {}", source, error);
                false
            }
        }
    }

    fn ingest_line(&mut self, source: &str, line: String) {
        let entry = if self.config.general.auto_parse_json
            && line.trim_start().starts_with('{')
            && line.trim_end().ends_with('}')
        {
            self.json_parser.parse(source, &line)
        } else {
            self.plain_parser.parse(source, &line)
        };

        if self.alert_rules.is_empty() {
            self.log_state.add_entry(entry);
        } else {
            let display = self.log_state.add_entry(entry);
            let dispatcher = self.alert_dispatcher.clone();
            let line = Arc::clone(&display);
            self.runtime.spawn(async move {
                let _ = dispatcher.check_and_alert(&line.entry).await;
            });
        }
    }

    /// Whether a line passes the level mask and the filter engine. (Per-source
    /// tab filtering is applied by the frontend.)
    pub fn passes_filter(&self, line: &DisplayLine) -> bool {
        let level_ok = match &line.entry.level {
            Some(level) => {
                let idx = LEVELS.iter().position(|l| l == level).unwrap_or(0);
                self.show_levels[idx]
            }
            None => true,
        };
        level_ok && self.filter_engine.matches(&line.entry)
    }

    /// Total bytes reported across all sources (for the status bar).
    pub fn total_bytes(&self) -> u64 {
        self.source_infos.values().filter_map(|i| i.file_size).sum()
    }
}
