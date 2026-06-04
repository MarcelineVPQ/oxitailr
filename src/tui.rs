//! Terminal UI frontend (ratatui + crossterm).
//!
//! The loop is fully event-driven: it `select!`s over terminal input and the
//! source/alert channels and only redraws on a change, so it sits at ~0% CPU
//! when idle. ratatui diffs at the character level, so redraws are cheap.

use crate::app::{AppCore, CoreChannels, DisplayLine, HighlightMatcher};
use crate::models::LogLevel;
use crate::state::{load_session, save_session, WindowState};
use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::collections::{HashMap, HashSet};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(PartialEq)]
enum Mode {
    Normal,
    Filter,
    Search,
    /// Typing/pasting a local file path to open (also where a dragged-in path lands).
    OpenFile,
}

/// Fields of the "add SSH source" form, in tab order.
const SSH_LABELS: [&str; 6] = [
    "Name",
    "Host",
    "User",
    "Remote path",
    "Port",
    "Key path (optional)",
];

struct SshForm {
    fields: [String; 6],
    focus: usize,
}

impl SshForm {
    fn new() -> Self {
        let mut fields: [String; 6] = std::array::from_fn(|_| String::new());
        fields[4] = "22".to_string(); // default port
        Self { fields, focus: 0 }
    }
}

/// Toggleable settings shown in the settings modal.
const SETTINGS_ITEMS: [&str; 2] = ["Show timestamps", "Auto-parse JSON"];

struct Ui {
    /// Index into the sorted source list (tab selection); 0 if none.
    tab: usize,
    /// Index of the first visible line within the filtered view.
    scroll: usize,
    /// Index of the selected line within the filtered view (the cursor). Only
    /// meaningful when not following; it anchors bookmarks and copy.
    cursor: usize,
    /// Stick to the bottom as new lines arrive (no cursor highlight while on).
    follow: bool,
    mode: Mode,
    search: String,
    /// Buffer for the open-file prompt (Mode::OpenFile).
    open_path: String,
    show_help: bool,
    show_alerts: bool,
    /// Active SSH add form, if open.
    ssh_form: Option<SshForm>,
    /// Settings modal open + selected row.
    settings: Option<usize>,
    /// Filter-preset picker open + selected row.
    presets: Option<usize>,
    /// Per-source bookmarks: source name -> set of line numbers.
    bookmarks: HashMap<String, HashSet<usize>>,
    /// Rows available for log lines on the last draw (for paging).
    viewport: usize,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            tab: 0,
            scroll: 0,
            cursor: 0,
            follow: true,
            mode: Mode::Normal,
            search: String::new(),
            open_path: String::new(),
            show_help: false,
            show_alerts: false,
            ssh_form: None,
            settings: None,
            presets: None,
            bookmarks: HashMap::new(),
            viewport: 20,
        }
    }
}

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Run the TUI to completion. Restores the terminal on the way out (even on error).
pub async fn run(mut core: AppCore, mut channels: CoreChannels) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Bracketed paste lets a dragged-in / pasted file path arrive as one chunk.
    crossterm::execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run_loop(&mut terminal, &mut core, &mut channels).await;

    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    result
}

fn session_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("oxitailr")
        .join("session.json")
}

async fn run_loop(terminal: &mut Term, core: &mut AppCore, ch: &mut CoreChannels) -> Result<()> {
    let mut ui = Ui::default();

    // Restore saved bookmarks, and — when nothing was opened from the CLI or
    // config — the files that were open last time.
    if let Some(session) = load_session(&session_path()) {
        ui.bookmarks = session
            .bookmarks
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        if core.started_empty && core.config.general.remember_last_session {
            for path in &session.open_local_files {
                core.add_local_source(std::path::PathBuf::from(path));
            }
        }
    }

    let mut events = EventStream::new();
    // Coalesce bursts of incoming lines into at most ~20 redraws/sec — plenty
    // smooth for a log, and a third less render work than 30fps under load.
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut needs_draw = true;
    let mut dirty = false;

    loop {
        if needs_draw {
            terminal.draw(|f| draw(f, core, &mut ui))?;
            needs_draw = false;
        }

        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) if key.kind != KeyEventKind::Release => {
                        if handle_key(key.code, key.modifiers, core, &mut ui) {
                            break;
                        }
                        needs_draw = true;
                    }
                    Some(Ok(Event::Paste(text))) => {
                        handle_paste(text, core, &mut ui);
                        needs_draw = true;
                    }
                    Some(Ok(Event::Resize(_, _))) => needs_draw = true,
                    Some(Err(_)) | None => break,
                    _ => {}
                }
            }
            maybe_src = ch.events.recv() => {
                if let Some(evt) = maybe_src {
                    let mut changed = core.ingest(evt);
                    while let Ok(e) = ch.events.try_recv() {
                        changed |= core.ingest(e);
                    }
                    if changed { dirty = true; }
                }
            }
            maybe_alert = ch.alerts.recv() => {
                if let Some(alert) = maybe_alert {
                    core.pending_alerts.push(alert);
                    if core.pending_alerts.len() > 20 {
                        core.pending_alerts.remove(0);
                    }
                    dirty = true;
                }
            }
            _ = tick.tick(), if dirty => {
                dirty = false;
                needs_draw = true;
            }
        }
    }

    // Persist bookmarks (and the open files) on the way out.
    let bookmarks: std::collections::HashMap<String, Vec<usize>> = ui
        .bookmarks
        .iter()
        .filter(|(_, set)| !set.is_empty())
        .map(|(k, set)| {
            let mut v: Vec<usize> = set.iter().copied().collect();
            v.sort_unstable();
            (k.clone(), v)
        })
        .collect();
    save_session(
        &session_path(),
        core.open_local_paths(),
        Vec::new(),
        WindowState::default(),
        bookmarks,
    );
    Ok(())
}

/// Iterator over the lines passing the level/filter rules and the selected tab.
fn passing<'a>(
    core: &'a AppCore,
    selected: Option<&'a str>,
) -> impl Iterator<Item = &'a Arc<DisplayLine>> {
    core.log_state.lines.iter().filter(move |line| {
        if let Some(sel) = selected {
            if line.entry.source != sel {
                return false;
            }
        }
        core.passes_filter(line)
    })
}

fn count_passing(core: &AppCore, selected: Option<&str>) -> usize {
    passing(core, selected).count()
}

/// The visible window: passing lines `[first, first+height)`.
fn window_slice<'a>(
    core: &'a AppCore,
    selected: Option<&'a str>,
    first: usize,
    height: usize,
) -> Vec<&'a Arc<DisplayLine>> {
    passing(core, selected).skip(first).take(height).collect()
}

/// The filtered-view index of the nth match of `needle` searching from `from`
/// in `dir` (+1 forward / -1 back). Used for `n`/`N`.
fn find_match(
    core: &AppCore,
    selected: Option<&str>,
    needle: &str,
    from: usize,
    forward: bool,
) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let needle = needle.to_lowercase();
    let hits: Vec<usize> = passing(core, selected)
        .enumerate()
        .filter(|(_, l)| l.entry.message.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect();
    if hits.is_empty() {
        return None;
    }
    if forward {
        hits.iter().find(|&&i| i > from).or(hits.first()).copied()
    } else {
        hits.iter()
            .rev()
            .find(|&&i| i < from)
            .or(hits.last())
            .copied()
    }
}

/// The line under the cursor (its filtered-view index `ui.cursor`).
fn cursor_line<'a>(
    core: &'a AppCore,
    ui: &Ui,
    selected: Option<&'a str>,
) -> Option<&'a Arc<DisplayLine>> {
    passing(core, selected).nth(ui.cursor)
}

fn is_bookmarked(bookmarks: &HashMap<String, HashSet<usize>>, line: &DisplayLine) -> bool {
    bookmarks
        .get(&line.entry.source)
        .is_some_and(|s| s.contains(&line.line_num))
}

/// Next/previous bookmarked line (filtered-view index) relative to `from`.
fn find_bookmark(
    core: &AppCore,
    selected: Option<&str>,
    bookmarks: &HashMap<String, HashSet<usize>>,
    from: usize,
    forward: bool,
) -> Option<usize> {
    let hits: Vec<usize> = passing(core, selected)
        .enumerate()
        .filter(|(_, l)| is_bookmarked(bookmarks, l))
        .map(|(i, _)| i)
        .collect();
    if hits.is_empty() {
        return None;
    }
    if forward {
        hits.iter().find(|&&i| i > from).or(hits.first()).copied()
    } else {
        hits.iter()
            .rev()
            .find(|&&i| i < from)
            .or(hits.last())
            .copied()
    }
}

fn sorted_sources(core: &AppCore) -> Vec<String> {
    let mut names: Vec<String> = core.source_infos.keys().cloned().collect();
    names.sort();
    names
}

fn level_color(level: Option<LogLevel>) -> Color {
    match level {
        Some(LogLevel::Trace) => Color::DarkGray,
        Some(LogLevel::Debug) => Color::Gray,
        Some(LogLevel::Info) => Color::Cyan,
        Some(LogLevel::Warn) => Color::Yellow,
        Some(LogLevel::Error) => Color::Red,
        Some(LogLevel::Fatal) => Color::Magenta,
        None => Color::Reset,
    }
}

fn draw(f: &mut Frame, core: &AppCore, ui: &mut Ui) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tabs / header
            Constraint::Min(1),    // log
            Constraint::Length(1), // status / input
        ])
        .split(f.area());

    let sources = sorted_sources(core);
    let selected = sources
        .get(ui.tab)
        .map(|s| s.as_str())
        .filter(|_| sources.len() > 1);

    let log_area = chunks[1];
    let height = (log_area.height as usize).max(1);
    ui.viewport = height;

    let total = count_passing(core, selected);
    let max_first = total.saturating_sub(height);
    if ui.follow {
        ui.cursor = total.saturating_sub(1);
        ui.scroll = max_first;
    } else if total > 0 && ui.cursor >= total - 1 {
        // Reaching the bottom re-engages follow, so scrolling back down resumes
        // auto-tailing without having to hit Space/G.
        ui.follow = true;
        ui.cursor = total - 1;
        ui.scroll = max_first;
    } else {
        // Keep the cursor on screen.
        ui.cursor = ui.cursor.min(total.saturating_sub(1));
        if ui.cursor < ui.scroll {
            ui.scroll = ui.cursor;
        } else if ui.cursor >= ui.scroll + height {
            ui.scroll = ui.cursor + 1 - height;
        }
        ui.scroll = ui.scroll.min(max_first);
    }
    let window = window_slice(core, selected, ui.scroll, height);
    let cursor_row = (!ui.follow && total > 0).then(|| ui.cursor - ui.scroll);

    let search_lower = ui.search.to_lowercase();
    draw_header(f, chunks[0], core, ui, &sources);
    draw_log(
        f,
        log_area,
        core,
        &window,
        sources.len() > 1,
        &search_lower,
        cursor_row,
        &ui.bookmarks,
    );
    draw_status(f, chunks[2], core, ui, total);

    if ui.show_help {
        draw_help(f, f.area());
    }
    if ui.show_alerts {
        draw_alerts(f, f.area(), core);
    }
    if let Some(sel) = ui.settings {
        draw_settings(f, f.area(), core, sel);
    }
    if let Some(sel) = ui.presets {
        draw_presets(f, f.area(), core, sel);
    }
    if let Some(form) = &ui.ssh_form {
        draw_ssh_form(f, f.area(), form);
    }
}

fn draw_header(f: &mut Frame, area: Rect, _core: &AppCore, ui: &Ui, sources: &[String]) {
    let mut spans = vec![Span::styled(
        " oxitailr ",
        Style::default().bg(Color::Blue).fg(Color::White),
    )];
    if sources.is_empty() {
        spans.push(Span::raw(
            "  (no sources — pass a file on the command line)",
        ));
    } else {
        spans.push(Span::raw(" "));
        for (i, name) in sources.iter().enumerate() {
            let style = if i == ui.tab {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::Gray)
            };
            spans.push(Span::styled(format!(" {} ", name), style));
            spans.push(Span::raw(" "));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[allow(clippy::too_many_arguments)]
fn draw_log(
    f: &mut Frame,
    area: Rect,
    core: &AppCore,
    window: &[&Arc<DisplayLine>],
    show_source: bool,
    search_lower: &str,
    cursor_row: Option<usize>,
    bookmarks: &HashMap<String, HashSet<usize>>,
) {
    let mut out: Vec<Line> = Vec::with_capacity(window.len());
    for (i, line) in window.iter().enumerate() {
        let bookmarked = is_bookmarked(bookmarks, line);
        let mut rendered = render_line(line, show_source, core, search_lower, bookmarked);
        if Some(i) == cursor_row {
            // Row highlight: a base background the per-span fg colors show over.
            rendered = rendered.style(Style::default().bg(Color::Rgb(45, 45, 70)));
        }
        out.push(rendered);
    }
    f.render_widget(Paragraph::new(out), area);
}

fn render_line<'a>(
    line: &'a DisplayLine,
    show_source: bool,
    core: &AppCore,
    search_lower: &str,
    bookmarked: bool,
) -> Line<'a> {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(if bookmarked {
        Span::styled("★", Style::default().fg(Color::Yellow))
    } else {
        Span::raw(" ")
    });
    spans.push(Span::styled(
        format!("{:>6} ", line.line_num),
        Style::default().fg(Color::DarkGray),
    ));

    if show_source {
        spans.push(Span::styled(
            format!("[{}] ", line.entry.source),
            Style::default().fg(Color::Blue),
        ));
    }

    if core.config.general.show_timestamps {
        if let Some(ts) = &line.entry.timestamp {
            spans.push(Span::styled(
                format!("{} ", ts.format("%H:%M:%S%.3f")),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    if let Some(level) = line.entry.level {
        spans.push(Span::styled(
            format!("{:>5} ", level.as_str()),
            Style::default()
                .fg(level_color(Some(level)))
                .add_modifier(Modifier::BOLD),
        ));
    }

    if line.has_ansi {
        ansi_to_spans(&line.entry.raw, &mut spans);
    } else {
        let base = Style::default().fg(level_color(line.entry.level));
        styled_message(
            &line.entry.message,
            base,
            &core.highlights,
            search_lower,
            &mut spans,
        );
    }

    Line::from(spans)
}

/// Render `msg` into spans, painting config highlight rules in their color and
/// the live search term with a yellow background (search takes precedence).
/// Everything else uses `base`. Operates on byte offsets; non-ASCII substring
/// matches are skipped to stay panic-free (regex rules still apply).
fn styled_message(
    msg: &str,
    base: Style,
    highlights: &[HighlightMatcher],
    search_lower: &str,
    out: &mut Vec<Span>,
) {
    // Fast path: nothing to highlight.
    if highlights.is_empty() && search_lower.is_empty() {
        out.push(Span::styled(msg.to_string(), base));
        return;
    }

    let n = msg.len();
    let mut marks: Vec<Option<Style>> = vec![None; n];
    let fill =
        |marks: &mut [Option<Style>], start: usize, end: usize, style: Style, force: bool| {
            for m in marks.iter_mut().take(end.min(n)).skip(start.min(n)) {
                if force || m.is_none() {
                    *m = Some(style);
                }
            }
        };

    let lower = msg.to_lowercase();
    let ascii_aligned = lower.len() == n;

    // Config highlight rules (first matching rule wins on overlap).
    for h in highlights {
        let style = Style::default().fg(Color::Rgb(h.color[0], h.color[1], h.color[2]));
        if let Some(re) = &h.regex {
            for m in re.find_iter(msg) {
                fill(&mut marks, m.start(), m.end(), style, false);
            }
        } else if ascii_aligned && !h.needle_lower.is_empty() {
            let mut pos = 0;
            while let Some(rel) = lower[pos..].find(&h.needle_lower) {
                let start = pos + rel;
                let end = start + h.needle_lower.len();
                fill(&mut marks, start, end, style, false);
                pos = end;
            }
        }
    }

    // Search term overrides highlight colors.
    if ascii_aligned && !search_lower.is_empty() {
        let hl = Style::default().bg(Color::Yellow).fg(Color::Black);
        let mut pos = 0;
        while let Some(rel) = lower[pos..].find(search_lower) {
            let start = pos + rel;
            let end = start + search_lower.len();
            fill(&mut marks, start, end, hl, true);
            pos = end;
        }
    }

    // Coalesce runs of equal style into spans (split on char boundaries).
    let mut run_start = 0usize;
    let mut run_style = marks.first().copied().flatten().unwrap_or(base);
    for (i, _) in msg.char_indices().skip(1) {
        let style = marks[i].unwrap_or(base);
        if style != run_style {
            out.push(Span::styled(msg[run_start..i].to_string(), run_style));
            run_start = i;
            run_style = style;
        }
    }
    out.push(Span::styled(msg[run_start..].to_string(), run_style));
}

/// Minimal ANSI SGR → ratatui span conversion (colors + bold; other codes reset).
fn ansi_to_spans(raw: &str, out: &mut Vec<Span>) {
    let mut style = Style::default();
    let mut text = String::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // Flush pending text.
            if !text.is_empty() {
                out.push(Span::styled(std::mem::take(&mut text), style));
            }
            // Parse the parameters up to the final byte.
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'm' && bytes[j] != b'\x1b' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'm' {
                let params = &raw[i + 2..j];
                style = apply_sgr(style, params);
                i = j + 1;
                continue;
            }
            i = j;
        } else {
            text.push(bytes[i] as char);
            i += 1;
        }
    }
    if !text.is_empty() {
        out.push(Span::styled(text, style));
    }
}

fn apply_sgr(mut style: Style, params: &str) -> Style {
    for code in params.split(';') {
        match code.parse::<u8>().unwrap_or(0) {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            30 => style = style.fg(Color::Black),
            31 => style = style.fg(Color::Red),
            32 => style = style.fg(Color::Green),
            33 => style = style.fg(Color::Yellow),
            34 => style = style.fg(Color::Blue),
            35 => style = style.fg(Color::Magenta),
            36 => style = style.fg(Color::Cyan),
            37 => style = style.fg(Color::Gray),
            90 => style = style.fg(Color::DarkGray),
            91 => style = style.fg(Color::LightRed),
            92 => style = style.fg(Color::LightGreen),
            93 => style = style.fg(Color::LightYellow),
            94 => style = style.fg(Color::LightBlue),
            95 => style = style.fg(Color::LightMagenta),
            96 => style = style.fg(Color::LightCyan),
            97 => style = style.fg(Color::White),
            _ => {}
        }
    }
    style
}

fn draw_status(f: &mut Frame, area: Rect, core: &AppCore, ui: &Ui, shown: usize) {
    if ui.mode != Mode::Normal {
        let (label, value) = match ui.mode {
            Mode::Filter => ("filter", &core.filter_text),
            Mode::Search => ("search", &ui.search),
            Mode::OpenFile => ("open (type/drag a path, Enter)", &ui.open_path),
            Mode::Normal => unreachable!(),
        };
        let mut spans = vec![
            Span::styled(
                format!(" {}: ", label),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(value.clone()),
            Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ];
        if let Some(err) = &core.filter_error {
            if ui.mode == Mode::Filter {
                spans.push(Span::styled(
                    format!("  [{}]", err),
                    Style::default().fg(Color::Red),
                ));
            }
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let levels = "TDIWEF";
    let mut level_spans: Vec<Span> = vec![Span::raw(" lvl:")];
    for (i, ch) in levels.chars().enumerate() {
        let on = core.show_levels[i];
        level_spans.push(Span::styled(
            ch.to_string(),
            if on {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM)
            },
        ));
    }

    let mb = core.total_bytes() as f64 / (1024.0 * 1024.0);
    // It tails live by default. The only time we surface state is when the user
    // has deliberately scrolled up to read back — then we show how to return to
    // live. No "pause"/"follow" toggle to manage.
    let (follow_label, follow_style) = if ui.follow {
        (
            " ● LIVE ".to_string(),
            Style::default().bg(Color::Green).fg(Color::Black),
        )
    } else {
        (
            " ↑ history — End → live ".to_string(),
            Style::default().bg(Color::Yellow).fg(Color::Black),
        )
    };
    let search = if ui.search.is_empty() {
        String::new()
    } else {
        format!(" /{}  (n/N)", ui.search)
    };

    let mut spans = vec![
        Span::styled(follow_label, follow_style),
        Span::raw(format!(
            " {} shown / {} total ",
            shown, core.log_state.total_lines_read
        )),
    ];
    spans.extend(level_spans);
    spans.push(Span::raw(format!("  {:.1} MB", mb)));
    if !search.is_empty() {
        spans.push(Span::styled(search, Style::default().fg(Color::Yellow)));
    }
    if !core.pending_alerts.is_empty() {
        spans.push(Span::styled(
            format!("  ⚠ {} (a)", core.pending_alerts.len()),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ));
    }
    spans.push(Span::styled(
        "   ?=help q=quit",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn draw_help(f: &mut Frame, area: Rect) {
    let rect = centered(area, 58, 21);
    let text = vec![
        Line::from("  j/k  ↓/↑       move cursor"),
        Line::from("  Ctrl+d/u       half page     PgDn/PgUp  page"),
        Line::from("  g              jump to top (scroll back through history)"),
        Line::from("  G / End / Space  jump to the newest line (resume live tail)"),
        Line::from("  Tab / S-Tab    switch source"),
        Line::from("  1..6           toggle level  T D I W E F"),
        Line::from("  f              filter (regex/substring)"),
        Line::from("  p              filter presets (from config)"),
        Line::from("  /  n  N        search, next / prev match"),
        Line::from("  b  ]  [        bookmark, next / prev bookmark"),
        Line::from("  y              copy line to clipboard"),
        Line::from("  O              open a file (or drag one onto the window)"),
        Line::from("  o / S          add SSH source / settings"),
        Line::from("  a              alerts        r reload   c clear"),
        Line::from("  ?              help          q / Esc  quit"),
    ];
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Keys ")),
        rect,
    );
}

fn draw_alerts(f: &mut Frame, area: Rect, core: &AppCore) {
    let rect = centered(area, 80, 16);
    let mut lines: Vec<Line> = Vec::new();
    if core.pending_alerts.is_empty() {
        lines.push(Line::from("  (no alerts)"));
    } else {
        for ev in core
            .pending_alerts
            .iter()
            .rev()
            .take(rect.height as usize - 2)
        {
            let msg: String = ev.entry.message.chars().take(60).collect();
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {} ", ev.rule_name),
                    Style::default().fg(Color::Black).bg(Color::Yellow),
                ),
                Span::raw(" "),
                Span::raw(msg),
            ]));
        }
    }
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Alerts (any key to close) "),
        ),
        rect,
    );
}

fn draw_settings(f: &mut Frame, area: Rect, core: &AppCore, sel: usize) {
    let rect = centered(area, 46, 7);
    let mut lines: Vec<Line> = Vec::new();
    for (i, item) in SETTINGS_ITEMS.iter().enumerate() {
        let on = match i {
            0 => core.config.general.show_timestamps,
            1 => core.config.general.auto_parse_json,
            _ => false,
        };
        let style = if i == sel {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("  [{}] {} ", if on { 'x' } else { ' ' }, item),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ move · Space toggle · Esc close",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Settings ")),
        rect,
    );
}

/// Filter-preset picker. Row 0 clears any applied preset; rows 1.. are the
/// named presets from `[filters.<name>]` in the config.
fn draw_presets(f: &mut Frame, area: Rect, core: &AppCore, sel: usize) {
    let names = core.preset_names();
    let rect = centered(area, 50, (names.len() + 5).clamp(6, 18) as u16);
    let mut lines: Vec<Line> = Vec::new();
    let row = |label: String, selected: bool| {
        let style = if selected {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
        };
        Line::from(Span::styled(label, style))
    };
    lines.push(row("  ✕ clear filter ".to_string(), sel == 0));
    if names.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no presets — add [filters.<name>] to config)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, name) in names.iter().enumerate() {
            lines.push(row(format!("  {} ", name), sel == i + 1));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ move · Enter apply · Esc close",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Filter presets "),
        ),
        rect,
    );
}

fn draw_ssh_form(f: &mut Frame, area: Rect, form: &SshForm) {
    let rect = centered(area, 62, 12);
    let mut lines: Vec<Line> = Vec::new();
    for (i, label) in SSH_LABELS.iter().enumerate() {
        let focused = i == form.focus;
        let label_style = if focused {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default().fg(Color::Gray)
        };
        let mut spans = vec![
            Span::styled(format!(" {:>19}: ", label), label_style),
            Span::raw(form.fields[i].clone()),
        ];
        if focused {
            spans.push(Span::styled(
                "_",
                Style::default().add_modifier(Modifier::SLOW_BLINK),
            ));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab/↑↓ move · Enter connect · Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Add SSH source "),
        ),
        rect,
    );
}

fn handle_ssh_form(code: KeyCode, core: &mut AppCore, ui: &mut Ui) {
    let n = SSH_LABELS.len();
    let mut close = false;
    let mut submit = false;
    if let Some(form) = ui.ssh_form.as_mut() {
        match code {
            KeyCode::Esc => close = true,
            KeyCode::Enter => submit = true,
            KeyCode::Tab | KeyCode::Down => form.focus = (form.focus + 1) % n,
            KeyCode::BackTab | KeyCode::Up => form.focus = (form.focus + n - 1) % n,
            KeyCode::Backspace => {
                form.fields[form.focus].pop();
            }
            KeyCode::Char(c) => form.fields[form.focus].push(c),
            _ => {}
        }
    }
    if submit {
        if let Some(form) = ui.ssh_form.take() {
            let f = form.fields;
            let host = f[1].trim().to_string();
            if !host.is_empty() {
                let name = if f[0].trim().is_empty() {
                    host.clone()
                } else {
                    f[0].trim().to_string()
                };
                let port = f[4].trim().parse::<u16>().ok();
                let key = f[5].trim();
                let key_path = (!key.is_empty()).then(|| PathBuf::from(key));
                core.add_ssh_source(
                    name,
                    host,
                    f[2].trim().to_string(),
                    f[3].trim().to_string(),
                    port,
                    key_path,
                    None,
                );
            }
        }
    } else if close {
        ui.ssh_form = None;
    }
}

/// Handle a key. Returns true if the app should quit.
fn handle_key(code: KeyCode, mods: KeyModifiers, core: &mut AppCore, ui: &mut Ui) -> bool {
    // Modal forms capture all input.
    if ui.ssh_form.is_some() {
        handle_ssh_form(code, core, ui);
        return false;
    }
    if let Some(sel) = ui.settings {
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('S') => ui.settings = None,
            KeyCode::Up | KeyCode::Char('k') => ui.settings = Some(sel.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => {
                ui.settings = Some((sel + 1).min(SETTINGS_ITEMS.len() - 1))
            }
            KeyCode::Enter | KeyCode::Char(' ') => match sel {
                0 => core.config.general.show_timestamps = !core.config.general.show_timestamps,
                1 => core.config.general.auto_parse_json = !core.config.general.auto_parse_json,
                _ => {}
            },
            _ => {}
        }
        return false;
    }
    if let Some(sel) = ui.presets {
        // Row 0 clears the filter; rows 1.. are the named presets.
        let names = core.preset_names();
        let max = names.len(); // selectable rows: 0..=names.len()
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('p') => ui.presets = None,
            KeyCode::Up | KeyCode::Char('k') => ui.presets = Some(sel.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => ui.presets = Some((sel + 1).min(max)),
            KeyCode::Enter => {
                if sel == 0 {
                    core.clear_filter_rules();
                } else if let Some(name) = names.get(sel - 1) {
                    core.apply_filter_preset(name);
                }
                ui.presets = None;
                ui.follow = true;
            }
            _ => {}
        }
        return false;
    }

    match ui.mode {
        Mode::Filter => {
            match code {
                KeyCode::Esc => ui.mode = Mode::Normal,
                KeyCode::Enter => ui.mode = Mode::Normal,
                KeyCode::Backspace => {
                    core.filter_text.pop();
                    core.update_filter();
                }
                KeyCode::Char(c) => {
                    core.filter_text.push(c);
                    core.update_filter();
                }
                _ => {}
            }
            return false;
        }
        Mode::Search => {
            match code {
                KeyCode::Esc => {
                    ui.search.clear();
                    ui.mode = Mode::Normal;
                }
                KeyCode::Enter => ui.mode = Mode::Normal,
                KeyCode::Backspace => {
                    ui.search.pop();
                }
                KeyCode::Char(c) => ui.search.push(c),
                _ => {}
            }
            return false;
        }
        Mode::OpenFile => {
            match code {
                KeyCode::Esc => {
                    ui.open_path.clear();
                    ui.mode = Mode::Normal;
                }
                KeyCode::Enter => open_path_now(core, ui),
                KeyCode::Backspace => {
                    ui.open_path.pop();
                }
                KeyCode::Char(c) => ui.open_path.push(c),
                _ => {}
            }
            return false;
        }
        Mode::Normal => {}
    }

    if ui.show_help {
        ui.show_help = false; // any key closes help
        return false;
    }
    if ui.show_alerts && code != KeyCode::Char('a') {
        ui.show_alerts = false; // any key (other than the toggle) closes alerts
        return false;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('?') => ui.show_help = true,
        KeyCode::Char('f') => ui.mode = Mode::Filter,
        KeyCode::Char('/') => {
            ui.search.clear();
            ui.mode = Mode::Search;
        }
        KeyCode::Char('r') => core.reload(),
        KeyCode::Char('c') => {
            core.log_state.clear();
            ui.scroll = 0;
        }
        KeyCode::Char(' ') => ui.follow = true,
        KeyCode::Char('j') | KeyCode::Down => cursor_move(ui, 1),
        KeyCode::Char('k') | KeyCode::Up => cursor_move(ui, -1),
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => {
            cursor_move(ui, (ui.viewport / 2) as isize)
        }
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => {
            cursor_move(ui, -((ui.viewport / 2) as isize))
        }
        KeyCode::PageDown => cursor_move(ui, ui.viewport as isize),
        KeyCode::PageUp => cursor_move(ui, -(ui.viewport as isize)),
        KeyCode::Char('g') | KeyCode::Home => {
            ui.follow = false;
            ui.cursor = 0;
            ui.scroll = 0;
        }
        KeyCode::Char('G') | KeyCode::End => ui.follow = true,
        KeyCode::Char('n') | KeyCode::Char('N') => {
            let selected = current_source(core, ui);
            let forward = matches!(code, KeyCode::Char('n'));
            if let Some(idx) = find_match(core, selected.as_deref(), &ui.search, ui.cursor, forward)
            {
                ui.follow = false;
                ui.cursor = idx;
            }
        }
        KeyCode::Char('b') => {
            let selected = current_source(core, ui);
            if let Some((source, line_num)) = cursor_line(core, ui, selected.as_deref())
                .map(|l| (l.entry.source.clone(), l.line_num))
            {
                let set = ui.bookmarks.entry(source).or_default();
                if !set.remove(&line_num) {
                    set.insert(line_num);
                }
            }
        }
        KeyCode::Char(']') | KeyCode::Char('[') => {
            let selected = current_source(core, ui);
            let forward = matches!(code, KeyCode::Char(']'));
            if let Some(idx) =
                find_bookmark(core, selected.as_deref(), &ui.bookmarks, ui.cursor, forward)
            {
                ui.follow = false;
                ui.cursor = idx;
            }
        }
        KeyCode::Char('y') => {
            let selected = current_source(core, ui);
            if let Some(line) = cursor_line(core, ui, selected.as_deref()) {
                copy_to_clipboard(&line.entry.raw);
            }
        }
        KeyCode::Char('a') => ui.show_alerts = !ui.show_alerts,
        KeyCode::Char('O') => {
            ui.open_path.clear();
            ui.mode = Mode::OpenFile;
        }
        KeyCode::Char('o') => ui.ssh_form = Some(SshForm::new()),
        KeyCode::Char('S') => ui.settings = Some(0),
        KeyCode::Char('p') => ui.presets = Some(0),
        KeyCode::Tab => {
            let n = core.source_infos.len().max(1);
            ui.tab = (ui.tab + 1) % n;
        }
        KeyCode::BackTab => {
            let n = core.source_infos.len().max(1);
            ui.tab = (ui.tab + n - 1) % n;
        }
        KeyCode::Char(d @ '1'..='6') => {
            let idx = d as usize - '1' as usize;
            core.show_levels[idx] = !core.show_levels[idx];
        }
        _ => {}
    }
    false
}

fn cursor_move(ui: &mut Ui, delta: isize) {
    ui.follow = false;
    ui.cursor = (ui.cursor as isize + delta).max(0) as usize; // clamped to total in draw
}

/// Handle a paste event. Terminals deliver a dragged-in file as its path (the
/// whole thing as one chunk thanks to bracketed paste), so pasting/dragging
/// into the open-file prompt fills it; in normal mode it pops that prompt
/// prefilled, which is as close to "drag onto the window" as a terminal allows.
fn handle_paste(text: String, core: &mut AppCore, ui: &mut Ui) {
    let line = text.trim_end_matches(['\n', '\r']);
    match ui.mode {
        Mode::Filter => {
            core.filter_text.push_str(line);
            core.update_filter();
        }
        Mode::Search => ui.search.push_str(line),
        Mode::OpenFile => ui.open_path.push_str(&clean_path(line)),
        Mode::Normal => {
            ui.open_path = clean_path(line);
            ui.mode = Mode::OpenFile;
        }
    }
}

/// Strip the decoration a terminal/file-manager adds to a dragged path:
/// surrounding quotes and a `file://` prefix.
fn clean_path(s: &str) -> String {
    let t = s.trim().trim_matches(['\'', '"']).trim();
    t.strip_prefix("file://").unwrap_or(t).to_string()
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), rest);
        }
    }
    p.to_string()
}

/// Open the path in the open-file prompt as a new local source.
fn open_path_now(core: &mut AppCore, ui: &mut Ui) {
    let raw = ui.open_path.trim();
    if !raw.is_empty() {
        core.add_local_source(PathBuf::from(expand_tilde(raw)));
    }
    ui.open_path.clear();
    ui.mode = Mode::Normal;
}

/// Copy text to the system clipboard via the OSC 52 terminal escape, which
/// works locally and over SSH (no external clipboard dependency).
fn copy_to_clipboard(text: &str) {
    use base64::Engine;
    use std::io::Write;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text);
    let seq = format!("\x1b]52;c;{}\x07", b64);
    let mut out = io::stdout();
    let _ = out.write_all(seq.as_bytes());
    let _ = out.flush();
}

/// The source name for the current tab, if more than one source is open.
fn current_source(core: &AppCore, ui: &Ui) -> Option<String> {
    let sources = sorted_sources(core);
    if sources.len() > 1 {
        sources.get(ui.tab).cloned()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hl(needle: &str, color: [u8; 3]) -> HighlightMatcher {
        HighlightMatcher {
            regex: None,
            needle_lower: needle.to_lowercase(),
            color,
        }
    }

    /// The concatenation of all rendered spans must reproduce the input exactly.
    fn rendered_text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn no_rules_is_single_base_span() {
        let base = Style::default();
        let mut out = Vec::new();
        styled_message("hello world", base, &[], "", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(rendered_text(&out), "hello world");
    }

    #[test]
    fn substring_highlight_colors_match_only() {
        let base = Style::default();
        let rules = [hl("error", [255, 0, 0])];
        let mut out = Vec::new();
        styled_message("an ERROR here", base, &rules, "", &mut out);
        assert_eq!(rendered_text(&out), "an ERROR here");
        // The matched run carries the rule color; surrounding text does not.
        let colored: Vec<&str> = out
            .iter()
            .filter(|s| s.style.fg == Some(Color::Rgb(255, 0, 0)))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(colored, vec!["ERROR"]);
    }

    #[test]
    fn search_overrides_highlight_on_overlap() {
        let base = Style::default();
        let rules = [hl("error", [255, 0, 0])];
        let mut out = Vec::new();
        styled_message("the error line", base, &rules, "error", &mut out);
        assert_eq!(rendered_text(&out), "the error line");
        // "error" should be the search style (yellow bg), not the rule color.
        let search_span = out.iter().find(|s| s.content.as_ref() == "error").unwrap();
        assert_eq!(search_span.style.bg, Some(Color::Yellow));
    }

    #[test]
    fn roundtrips_with_multiple_matches() {
        let base = Style::default();
        let rules = [hl("ab", [1, 2, 3])];
        let mut out = Vec::new();
        styled_message("ab cd ab", base, &rules, "", &mut out);
        assert_eq!(rendered_text(&out), "ab cd ab");
    }
}
