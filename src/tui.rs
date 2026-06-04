//! Terminal UI frontend (ratatui + crossterm).
//!
//! The loop is fully event-driven: it `select!`s over terminal input and the
//! source/alert channels and only redraws on a change, so it sits at ~0% CPU
//! when idle. ratatui diffs at the character level, so redraws are cheap.

use crate::app::{AppCore, CoreChannels, DisplayLine};
use crate::models::LogLevel;
use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
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
use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;

#[derive(PartialEq)]
enum Mode {
    Normal,
    Filter,
    Search,
}

struct Ui {
    /// Index into the sorted source list (tab selection); 0 if none.
    tab: usize,
    /// Index of the first visible line within the filtered view.
    scroll: usize,
    /// Stick to the bottom as new lines arrive.
    follow: bool,
    mode: Mode,
    search: String,
    /// Indices (within the filtered view) of search matches.
    matches: Vec<usize>,
    current_match: Option<usize>,
    show_help: bool,
    /// Rows available for log lines on the last draw (for paging).
    viewport: usize,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            tab: 0,
            scroll: 0,
            follow: true,
            mode: Mode::Normal,
            search: String::new(),
            matches: Vec::new(),
            current_match: None,
            show_help: false,
            viewport: 20,
        }
    }
}

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Run the TUI to completion. Restores the terminal on the way out (even on error).
pub async fn run(mut core: AppCore, mut channels: CoreChannels) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let result = run_loop(&mut terminal, &mut core, &mut channels).await;

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

async fn run_loop(terminal: &mut Term, core: &mut AppCore, ch: &mut CoreChannels) -> Result<()> {
    let mut ui = Ui::default();
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
    Ok(())
}

/// Compute the total number of lines passing the filter and just the visible
/// window (at most `height` lines), without allocating a Vec over the whole
/// buffer each frame. Returns `(total_passing, window)`.
fn compute_view<'a>(
    core: &'a AppCore,
    selected: Option<&str>,
    follow: bool,
    scroll: &mut usize,
    height: usize,
) -> (usize, Vec<&'a Arc<DisplayLine>>) {
    let lines = &core.log_state.lines;
    let pass = |line: &&Arc<DisplayLine>| -> bool {
        if let Some(sel) = selected {
            if line.entry.source != sel {
                return false;
            }
        }
        core.passes_filter(line)
    };
    let total = lines.iter().filter(pass).count();
    let max_first = total.saturating_sub(height);
    let first = if follow {
        max_first
    } else {
        (*scroll).min(max_first)
    };
    *scroll = first;
    let window = lines.iter().filter(pass).skip(first).take(height).collect();
    (total, window)
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
    let height = log_area.height as usize;
    ui.viewport = height;
    let (total, window) = compute_view(core, selected, ui.follow, &mut ui.scroll, height);

    let search_lower = ui.search.to_lowercase();
    draw_header(f, chunks[0], core, ui, &sources);
    draw_log(f, log_area, core, &window, sources.len() > 1, &search_lower);
    draw_status(f, chunks[2], core, ui, total);

    if ui.show_help {
        draw_help(f, f.area());
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

fn draw_log(
    f: &mut Frame,
    area: Rect,
    core: &AppCore,
    window: &[&Arc<DisplayLine>],
    show_source: bool,
    search_lower: &str,
) {
    let mut out: Vec<Line> = Vec::with_capacity(window.len());
    for line in window {
        out.push(render_line(line, show_source, core, search_lower));
    }
    f.render_widget(Paragraph::new(out), area);
}

fn render_line<'a>(
    line: &'a DisplayLine,
    show_source: bool,
    core: &AppCore,
    search_lower: &str,
) -> Line<'a> {
    let mut spans: Vec<Span> = Vec::new();
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
        let msg = &line.entry.message;
        if !search_lower.is_empty() && msg.to_lowercase().contains(search_lower) {
            highlight_matches(msg, search_lower, base, &mut spans);
        } else {
            spans.push(Span::styled(msg.clone(), base));
        }
    }

    Line::from(spans)
}

/// Split `text` so that case-insensitive occurrences of `needle` get a yellow
/// background; everything else uses `base`.
fn highlight_matches(text: &str, needle: &str, base: Style, out: &mut Vec<Span>) {
    let hay = text.to_lowercase();
    let hl = Style::default().bg(Color::Yellow).fg(Color::Black);
    let mut pos = 0usize;
    while let Some(rel) = hay[pos..].find(needle) {
        let start = pos + rel;
        if start > pos {
            out.push(Span::styled(text[pos..start].to_string(), base));
        }
        let end = start + needle.len();
        out.push(Span::styled(text[start..end].to_string(), hl));
        pos = end;
    }
    if pos < text.len() {
        out.push(Span::styled(text[pos..].to_string(), base));
    }
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
    if ui.mode == Mode::Filter || ui.mode == Mode::Search {
        let (label, value) = if ui.mode == Mode::Filter {
            ("filter", &core.filter_text)
        } else {
            ("search", &ui.search)
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
    let follow = if ui.follow { "FOLLOW" } else { "      " };
    let search = if ui.search.is_empty() {
        String::new()
    } else {
        let n = ui.matches.len();
        let cur = ui.current_match.map(|m| m + 1).unwrap_or(0);
        format!(" search:'{}' {}/{}", ui.search, cur, n)
    };

    let mut spans = vec![
        Span::styled(
            format!(" {} ", follow),
            Style::default().bg(Color::Blue).fg(Color::White),
        ),
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
    spans.push(Span::styled(
        "   ?=help q=quit",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let w = 56u16.min(area.width.saturating_sub(2));
    let h = 18u16.min(area.height.saturating_sub(2));
    let rect = Rect {
        x: (area.width.saturating_sub(w)) / 2,
        y: (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let text = vec![
        Line::from("Oxitailr — keys"),
        Line::from(""),
        Line::from("  j/k  ↓/↑      scroll line"),
        Line::from("  Ctrl+d/u      half page"),
        Line::from("  PgDn/PgUp     page"),
        Line::from("  g / G         top / bottom (G = follow)"),
        Line::from("  Space         toggle follow"),
        Line::from("  Tab / Shift+Tab   switch source"),
        Line::from("  1..6          toggle level T D I W E F"),
        Line::from("  f             filter (regex/substring)"),
        Line::from("  /             search    n/N next/prev"),
        Line::from("  r             reload     c  clear"),
        Line::from("  ? quit help   q / Esc  quit"),
    ];
    f.render_widget(ratatui::widgets::Clear, rect);
    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Help ")),
        rect,
    );
}

/// Handle a key. Returns true if the app should quit.
fn handle_key(code: KeyCode, mods: KeyModifiers, core: &mut AppCore, ui: &mut Ui) -> bool {
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
        Mode::Normal => {}
    }

    if ui.show_help {
        // Any key closes help.
        ui.show_help = false;
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
        KeyCode::Char(' ') => ui.follow = !ui.follow,
        KeyCode::Char('j') | KeyCode::Down => scroll_by(ui, 1),
        KeyCode::Char('k') | KeyCode::Up => scroll_by(ui, -1),
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => {
            scroll_by(ui, (ui.viewport / 2) as isize)
        }
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => {
            scroll_by(ui, -((ui.viewport / 2) as isize))
        }
        KeyCode::PageDown => scroll_by(ui, ui.viewport as isize),
        KeyCode::PageUp => scroll_by(ui, -(ui.viewport as isize)),
        KeyCode::Char('g') | KeyCode::Home => {
            ui.follow = false;
            ui.scroll = 0;
        }
        KeyCode::Char('G') | KeyCode::End => ui.follow = true,
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

fn scroll_by(ui: &mut Ui, delta: isize) {
    ui.follow = false;
    let cur = ui.scroll as isize;
    ui.scroll = (cur + delta).max(0) as usize;
}
