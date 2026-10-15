//! Oxitailr desktop: the same engine as the terminal app (`oxitailr-core`),
//! drawn in a window.
//!
//! One `AppCore` lives behind a mutex. A background task drains the source
//! and alert channels into it and tells the window "changed" at most ten
//! times a second; the window then asks only for the rows it shows. Nothing
//! runs on a timer while nothing changes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod secrets_cmd;
mod ssh;
mod tray;
mod view;
#[cfg(test)]
mod view_tests;

use oxitailr_core::app::{AppCore, CoreChannels};
use oxitailr_core::config::{self, AppConfig};
use oxitailr_core::models::SourceInfo;
use oxitailr_core::source::{SourceCommand, SourceEvent};
use oxitailr_core::state::{load_session, save_session, WindowState};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tauri_plugin_notification::NotificationExt;
use view::{row_text, segments, Row, View};

/// "changed" notices: at most every 100 ms while the window is focused, every
/// 500 ms while it's visible but another app is in front, none while hidden.
const NOTIFY_EVERY: Duration = Duration::from_millis(100);
const NOTIFY_UNFOCUSED: Duration = Duration::from_millis(500);
/// Desktop notifications for alerts: at most one per this interval; alerts in
/// between are grouped into the next one.
const ALERT_NOTIFY_GAP: Duration = Duration::from_secs(10);
/// Source events ingested per lock. A big first read must not hold the lock
/// (and so every command) for seconds; between bursts the lock is released.
const EVENTS_PER_LOCK: usize = 64;
/// Recent alerts kept for the window.
const ALERTS_KEPT: usize = 50;

struct Host {
    core: AppCore,
    view: View,
    /// Bookmarked line numbers per source, for this session. Line numbers
    /// restart after Clear/Reload and on every launch, so bookmarks are
    /// dropped then rather than left pointing at unrelated lines.
    bookmarks: HashMap<String, HashSet<usize>>,
    alerts: Vec<AlertOut>,
    /// Sources the user closed. Their tailers report a last status change as
    /// they stop; that must not bring the tab back.
    closed: HashSet<String>,
    /// Bumped whenever the source list changes, so the window refetches it
    /// only then instead of with every batch of lines.
    src_rev: u64,
    /// Alerts that arrived while the window wasn't focused (tray status line).
    unseen: usize,
}

/// How much of the user's attention the window has (set by the page).
const HIDDEN: u8 = 0;
const VISIBLE: u8 = 1;
const FOCUSED: u8 = 2;
struct Attention(AtomicU8);

impl Attention {
    fn get(&self) -> u8 {
        self.0.load(Ordering::Relaxed)
    }
}

struct Shared(Mutex<Host>);

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Host> {
        // A panic while holding the lock leaves the data usable; keep going.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Serialize, Clone)]
struct AlertOut {
    rule: String,
    source: String,
    text: String,
}

#[derive(Serialize)]
struct Snapshot {
    version: String,
    sources: Vec<SourceInfo>,
    presets: Vec<String>,
    show_timestamps: bool,
    total_lines: usize,
    shown: usize,
    bytes: u64,
    alerts: Vec<AlertOut>,
    src_rev: u64,
}

/// What a rows request returns: the rows, plus enough to keep a paused view
/// steady and to tell whether the source list needs refetching.
#[derive(Serialize)]
struct RowsOut {
    total: usize,
    rows: Vec<Row>,
    /// The view's rebuild count; positions from another epoch are meaningless.
    epoch: u64,
    /// Rows dropped off the front in this epoch (buffer eviction).
    dropped: u64,
    lines: usize,
    bytes: u64,
    src_rev: u64,
    alerts: usize,
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oxitailr")
}

/// The desktop app keeps its own session (open files), separate from the
/// terminal app's session.json, so the two never overwrite each other.
fn session_path() -> PathBuf {
    config_dir().join("desktop-session.json")
}

fn load_config() -> AppConfig {
    let path = config_dir().join("config.toml");
    if path.exists() {
        match config::load_config(&path) {
            Ok(cfg) => return cfg,
            Err(e) => tracing::warn!("config {} not loaded: {e}; using defaults", path.display()),
        }
    }
    AppConfig::default()
}

fn sorted_sources(core: &AppCore) -> Vec<SourceInfo> {
    let mut v: Vec<SourceInfo> = core.source_infos.values().cloned().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

fn source_name(ev: &SourceEvent) -> &str {
    match ev {
        SourceEvent::Line { source, .. }
        | SourceEvent::Lines { source, .. }
        | SourceEvent::StatusChange { source, .. }
        | SourceEvent::Error { source, .. } => source,
    }
}

// ---------------------------------------------------------------- commands ---
// The ones that touch the buffer are async: Tauri runs those off the main
// thread, so a busy lock never freezes the window.

#[tauri::command]
async fn state(shared: State<'_, Shared>) -> Result<Snapshot, String> {
    let mut guard = shared.lock();
    let h = &mut *guard;
    h.view.refresh(&h.core);
    Ok(Snapshot {
        version: env!("CARGO_PKG_VERSION").to_string(),
        sources: sorted_sources(&h.core),
        presets: h.core.preset_names(),
        show_timestamps: h.core.config.general.show_timestamps,
        total_lines: h.core.log_state.lines.len(),
        shown: h.view.len(),
        bytes: h.core.total_bytes(),
        alerts: h.alerts.clone(),
        src_rev: h.src_rev,
    })
}

#[tauri::command]
fn open_paths(shared: State<Shared>, paths: Vec<String>) {
    let mut h = shared.lock();
    for p in paths {
        let path = PathBuf::from(p);
        if let Some(name) = path.file_name() {
            h.closed.remove(&*name.to_string_lossy());
        }
        h.core.add_local_source(path);
    }
    h.src_rev += 1;
}

#[tauri::command]
fn close_source(shared: State<Shared>, name: String) {
    let mut h = shared.lock();
    let _ = h
        .core
        .source_cmd_tx
        .send(SourceCommand::Remove { name: name.clone() });
    h.core.source_infos.remove(&name);
    h.closed.insert(name);
    h.src_rev += 1;
    h.view.invalidate();
}

/// Set what the view shows. Returns the filter's error, if the text isn't a
/// valid pattern (it still filters as a substring then, like the terminal app).
#[tauri::command]
async fn set_view(
    shared: State<'_, Shared>,
    tab: Option<String>,
    filter: String,
    levels: [bool; 6],
    search: String,
) -> Result<Option<String>, String> {
    let mut h = shared.lock();
    h.view.tab = tab;
    h.view.search_lower = search.to_lowercase();
    h.core.show_levels = levels;
    if h.core.filter_text != filter {
        h.core.filter_text = filter;
        h.core.update_filter();
    }
    h.view.invalidate();
    Ok(h.core.filter_error.clone())
}

#[tauri::command]
async fn rows(shared: State<'_, Shared>, start: usize, count: usize) -> Result<RowsOut, String> {
    let mut guard = shared.lock();
    let h = &mut *guard;
    h.view.refresh(&h.core);
    let total = h.view.len();
    let show_ts = h.core.config.general.show_timestamps;
    let end = start.saturating_add(count.min(500)).min(total);
    let rows = (start.min(end)..end)
        .filter_map(|i| {
            let line = h.view.line(&h.core, i)?;
            let text = row_text(line);
            Some(Row {
                i,
                n: line.line_num,
                src: line.entry.source.clone(),
                ts: if show_ts {
                    line.entry
                        .timestamp
                        .map(|t| t.format("%H:%M:%S%.3f").to_string())
                } else {
                    None
                },
                lvl: line.entry.level.map(|l| l.as_str()),
                segs: segments(&text, &h.core.highlights, &h.view.search_lower),
                mark: h
                    .bookmarks
                    .get(&line.entry.source)
                    .is_some_and(|s| s.contains(&line.line_num)),
            })
        })
        .collect();
    Ok(RowsOut {
        total,
        rows,
        epoch: h.view.epoch,
        dropped: h.view.dropped,
        lines: h.core.log_state.lines.len(),
        bytes: h.core.total_bytes(),
        src_rev: h.src_rev,
        alerts: h.alerts.len(),
    })
}

/// The next (or previous) view position whose text contains `term`.
/// `from: None` starts at the first (or last) row, included.
#[tauri::command]
async fn find(
    shared: State<'_, Shared>,
    term: String,
    from: Option<usize>,
    forward: bool,
) -> Result<Option<usize>, String> {
    let needle = term.to_lowercase();
    if needle.is_empty() {
        return Ok(None);
    }
    let mut guard = shared.lock();
    let h = &mut *guard;
    h.view.refresh(&h.core);
    Ok(h.view.find(&h.core, from, forward, |l| {
        l.entry.message.to_lowercase().contains(&needle)
    }))
}

#[tauri::command]
fn toggle_bookmark(shared: State<Shared>, source: String, line: usize) -> bool {
    let mut h = shared.lock();
    let set = h.bookmarks.entry(source).or_default();
    if set.remove(&line) {
        false
    } else {
        set.insert(line);
        true
    }
}

#[tauri::command]
async fn find_bookmark(
    shared: State<'_, Shared>,
    from: Option<usize>,
    forward: bool,
) -> Result<Option<usize>, String> {
    let mut guard = shared.lock();
    let h = &mut *guard;
    h.view.refresh(&h.core);
    let marks = &h.bookmarks;
    Ok(h.view.find(&h.core, from, forward, |l| {
        marks
            .get(&l.entry.source)
            .is_some_and(|s| s.contains(&l.line_num))
    }))
}

#[tauri::command]
fn apply_preset(shared: State<Shared>, name: Option<String>) -> bool {
    let mut h = shared.lock();
    let ok = match name {
        Some(n) => h.core.apply_filter_preset(&n),
        None => {
            h.core.clear_filter_rules();
            true
        }
    };
    h.view.invalidate();
    ok
}

/// Line numbers restart after these two, so bookmarks go with them.
#[tauri::command]
fn clear(shared: State<Shared>) {
    let mut h = shared.lock();
    h.core.log_state.clear();
    h.bookmarks.clear();
    h.view.invalidate();
}

#[tauri::command]
fn reload(shared: State<Shared>) {
    let mut h = shared.lock();
    h.core.reload();
    h.bookmarks.clear();
    h.view.invalidate();
}

#[tauri::command]
fn dismiss_alerts(shared: State<Shared>) {
    shared.lock().alerts.clear();
}

/// The page reports its attention: "focused", "visible" (on screen, another
/// app in front) or "hidden" (minimized, other desktop, or in the tray).
#[tauri::command]
fn set_attention(
    app: AppHandle,
    attention: State<Attention>,
    shared: State<Shared>,
    level: String,
) {
    let next = match level.as_str() {
        "focused" => FOCUSED,
        "visible" => VISIBLE,
        _ => HIDDEN,
    };
    let prev = attention.0.swap(next, Ordering::Relaxed);
    if next == FOCUSED {
        let mut h = shared.lock();
        if h.unseen > 0 {
            h.unseen = 0;
            let text = status_text(&h);
            drop(h);
            tray::set_status(&app, &text);
        }
    }
    // Back from hidden: one catch-up, since nothing was sent meanwhile.
    if prev == HIDDEN && next != HIDDEN {
        let _ = app.emit("changed", ());
    }
}

fn status_text(h: &Host) -> String {
    let files = match h.core.source_infos.len() {
        0 => "No files open".to_string(),
        1 => "Tailing 1 file".to_string(),
        n => format!("Tailing {n} files"),
    };
    match h.unseen {
        0 => files,
        1 => format!("{files} · 1 new alert"),
        n => format!("{files} · {n} new alerts"),
    }
}

/// One desktop notification for the alerts gathered since the last one.
fn notify_alerts(app: &AppHandle, pending: &[AlertOut]) {
    let clip = |t: &str| -> String {
        let t: String = t.chars().take(140).collect();
        t
    };
    let (title, body) = match pending {
        [] => return,
        [a] => (
            format!("Oxitailr: {}", a.rule),
            format!("{}: {}", a.source, clip(&a.text)),
        ),
        many => {
            let mut per: Vec<(String, usize)> = Vec::new();
            for a in many {
                match per.iter_mut().find(|(s, _)| *s == a.source) {
                    Some((_, n)) => *n += 1,
                    None => per.push((a.source.clone(), 1)),
                }
            }
            let body = per
                .iter()
                .map(|(s, n)| format!("{s}: {n}"))
                .collect::<Vec<_>>()
                .join(", ");
            (format!("Oxitailr: {} alerts", many.len()), body)
        }
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("could not show a notification: {e}");
    }
}

// -------------------------------------------------------------------- pump ---

/// Ingest one batch of events: at most [`EVENTS_PER_LOCK`] under one lock.
/// Returns whether anything changed and whether the source list did.
fn ingest_batch(shared: &Shared, first: SourceEvent, ch: &mut CoreChannels) -> (bool, bool) {
    let mut h = shared.lock();
    let (mut changed, mut sources_changed) = (false, false);
    let mut next = Some(first);
    let mut n = 0;
    while let Some(ev) = next.take() {
        if !h.closed.contains(source_name(&ev)) {
            sources_changed |= matches!(ev, SourceEvent::StatusChange { .. });
            changed |= h.core.ingest(ev);
        }
        n += 1;
        if n < EVENTS_PER_LOCK {
            next = ch.events.try_recv().ok();
        }
    }
    if sources_changed {
        h.src_rev += 1;
    }
    (changed, sources_changed)
}

/// Drain source and alert events into the core; notify the window, coalesced
/// and paced by how much attention it has. While it's hidden nothing is sent
/// and the timer doesn't run.
async fn pump(app: AppHandle, mut ch: CoreChannels) {
    let mut tick = tokio::time::interval(NOTIFY_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut alert_tick = tokio::time::interval(Duration::from_secs(1));
    alert_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = false;
    let mut sources_dirty = true;
    let mut last_emit = Instant::now();
    let mut pending: Vec<AlertOut> = Vec::new();
    let mut last_notice: Option<Instant> = None;
    loop {
        let attention = app.state::<Attention>().get();
        tokio::select! {
            ev = ch.events.recv() => {
                let Some(ev) = ev else { break };
                let (changed, src) = ingest_batch(&app.state::<Shared>(), ev, &mut ch);
                dirty |= changed;
                sources_dirty |= src;
                // Let commands take the lock between bursts.
                tokio::task::yield_now().await;
            }
            alert = ch.alerts.recv() => {
                let Some(a) = alert else { continue };
                let out = AlertOut {
                    rule: a.rule_name,
                    source: a.entry.source.clone(),
                    text: a.entry.message.clone(),
                };
                {
                    let shared = app.state::<Shared>();
                    let mut h = shared.lock();
                    h.alerts.push(out.clone());
                    let excess = h.alerts.len().saturating_sub(ALERTS_KEPT);
                    h.alerts.drain(..excess);
                    if attention != FOCUSED {
                        h.unseen += 1;
                        sources_dirty = true; // refresh the tray status line
                    }
                }
                if attention != FOCUSED {
                    pending.push(out.clone());
                }
                let _ = app.emit("alert", out);
                dirty = true;
            }
            // Gathered alerts become one notification, at most every ALERT_NOTIFY_GAP.
            _ = alert_tick.tick(), if !pending.is_empty() => {
                if last_notice.is_none_or(|t| t.elapsed() >= ALERT_NOTIFY_GAP) {
                    if tray::notify_enabled(&app) {
                        notify_alerts(&app, &pending);
                    }
                    pending.clear();
                    last_notice = Some(Instant::now());
                }
            }
            // Only ticks while something changed and someone can see it.
            _ = tick.tick(), if dirty && attention != HIDDEN => {
                if attention == VISIBLE && last_emit.elapsed() < NOTIFY_UNFOCUSED {
                    continue;
                }
                dirty = false;
                last_emit = Instant::now();
                if sources_dirty {
                    sources_dirty = false;
                    let text = status_text(&app.state::<Shared>().lock());
                    tray::set_status(&app, &text);
                }
                let _ = app.emit("changed", ());
            }
        }
    }
}

fn save(shared: &Shared) {
    let h = shared.lock();
    save_session(
        &session_path(),
        h.core.open_local_paths(),
        Vec::new(),
        WindowState::default(),
        HashMap::new(),
    );
}

fn main() {
    // Files named on the command line open at start, like the terminal app.
    let files: Vec<PathBuf> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();

    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime"),
    );
    let (mut core, channels) = AppCore::new(runtime.clone(), load_config(), files);

    if let Some(session) = load_session(&session_path()) {
        if core.started_empty && core.config.general.remember_last_session {
            for path in &session.open_local_files {
                core.add_local_source(PathBuf::from(path));
            }
        }
    }

    let host = Shared(Mutex::new(Host {
        core,
        view: View::default(),
        bookmarks: HashMap::new(),
        alerts: Vec::new(),
        closed: HashSet::new(),
        src_rev: 0,
        unseen: 0,
    }));

    let app = tauri::Builder::default()
        // A second launch brings this window forward instead of opening another.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(host)
        .manage(Attention(AtomicU8::new(FOCUSED)))
        .invoke_handler(tauri::generate_handler![
            state,
            open_paths,
            close_source,
            set_view,
            rows,
            find,
            toggle_bookmark,
            find_bookmark,
            apply_preset,
            clear,
            reload,
            dismiss_alerts,
            ssh::add_ssh,
            set_attention,
            secrets_cmd::secret_backend,
            secrets_cmd::saved_secrets,
            secrets_cmd::ssh_password_saved,
            secrets_cmd::reveal_secret,
            secrets_cmd::forget_secret,
            secrets_cmd::ssh_secret_id,
        ])
        .setup(move |app| {
            tray::setup(app)?;
            // The core holds the runtime too, so it lives as long as the app.
            runtime.spawn(pump(app.handle().clone(), channels));
            Ok(())
        })
        .on_window_event(tray::on_window_event)
        .build(tauri::generate_context!())
        .expect("oxitailr desktop failed to start");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            save(&app.state::<Shared>());
        }
    });
}
