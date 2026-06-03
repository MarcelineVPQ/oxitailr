//! Log view rendering panel.

use crate::config::LogLevelColors;
use crate::models::LogLevel;
use crate::ui::ansi::{parse_ansi_line, ColoredSpan};
use crate::{find_matching_highlight, FilterSig};
use eframe::egui;

/// Color for a log level, with optional custom colors
pub fn log_level_color(
    level: Option<&LogLevel>,
    custom_colors: Option<&LogLevelColors>,
) -> egui::Color32 {
    match (level, custom_colors) {
        (Some(LogLevel::Trace), Some(c)) => {
            egui::Color32::from_rgb(c.trace[0], c.trace[1], c.trace[2])
        }
        (Some(LogLevel::Debug), Some(c)) => {
            egui::Color32::from_rgb(c.debug[0], c.debug[1], c.debug[2])
        }
        (Some(LogLevel::Info), Some(c)) => egui::Color32::from_rgb(c.info[0], c.info[1], c.info[2]),
        (Some(LogLevel::Warn), Some(c)) => egui::Color32::from_rgb(c.warn[0], c.warn[1], c.warn[2]),
        (Some(LogLevel::Error), Some(c)) => {
            egui::Color32::from_rgb(c.error[0], c.error[1], c.error[2])
        }
        (Some(LogLevel::Fatal), Some(c)) => {
            egui::Color32::from_rgb(c.fatal[0], c.fatal[1], c.fatal[2])
        }
        (Some(LogLevel::Trace), None) => egui::Color32::from_rgb(100, 100, 100),
        (Some(LogLevel::Debug), None) => egui::Color32::from_rgb(140, 140, 140),
        (Some(LogLevel::Info), None) => egui::Color32::from_rgb(80, 180, 220),
        (Some(LogLevel::Warn), None) => egui::Color32::from_rgb(220, 180, 50),
        (Some(LogLevel::Error), None) => egui::Color32::from_rgb(220, 80, 80),
        (Some(LogLevel::Fatal), None) => egui::Color32::from_rgb(255, 50, 150),
        (None, _) => egui::Color32::from_rgb(200, 200, 200),
    }
}

/// A display line with pre-parsed ANSI colors
#[derive(Clone)]
pub struct DisplayLine {
    pub entry: crate::models::LogEntry,
    pub spans: Vec<ColoredSpan>,
    pub has_ansi: bool,
    pub line_num: usize,
}

impl DisplayLine {
    pub fn from_entry(entry: crate::models::LogEntry, line_num: usize) -> Self {
        let has_ansi = entry.raw.contains("\x1b[");
        let spans = parse_ansi_line(&entry.raw);
        Self {
            entry,
            spans,
            has_ansi,
            line_num,
        }
    }
}

impl crate::TailLoggerApp {
    pub(crate) fn render_central_log_view(&mut self, ui: &mut egui::Ui) {
        if self.source_infos.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading("Welcome to Oxitailr");
                ui.add_space(20.0);
                ui.label("Add a log source to get started");
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    if ui.button("📂 Open Local File").clicked() {
                        self.open_file_dialog();
                    }
                    if ui.button("🔗 Add SSH Source").clicked() {
                        self.ssh_dialog.open = true;
                        self.ssh_dialog.reset();
                        self.ssh_dialog.port = "22".to_string();
                    }
                });
                ui.add_space(40.0);
                ui.label("Or run from command line:");
                ui.code("oxitailr /path/to/file.log");
                ui.add_space(20.0);
                ui.label("Config file location:");
                ui.code(self.config_path.display().to_string());
            });
            return;
        }

        ui.horizontal(|ui| {
            let mut source_names: Vec<String> = self.source_infos.keys().cloned().collect();
            source_names.sort(); // Ensure consistent tab order

            // Auto-select first source if none selected
            if self.selected_source.is_none() && !source_names.is_empty() {
                self.selected_source = Some(source_names[0].clone());
            }

            for name in &source_names {
                let is_selected = self.selected_source.as_ref() == Some(name);
                let info = self.source_infos.get(name);

                let status_symbol = info.map(|i| i.status_symbol()).unwrap_or("?");
                let tab_text = format!("{} {}", status_symbol, name);

                if ui.selectable_label(is_selected, tab_text).clicked() {
                    self.selected_source = Some(name.clone());
                }
            }
        });

        ui.separator();

        let text_style = egui::TextStyle::Monospace;
        let row_height =
            ui.text_style_height(&text_style) * (self.font_size / 13.0) * self.line_spacing;

        let font_size = self.font_size;
        let line_spacing = self.line_spacing;
        let show_source = self.show_source;
        let show_timestamps = self.show_timestamps;
        let source_count = self.source_infos.len();
        let highlight_rules = self.highlight_rules.clone();

        // Rebuild the filtered view only when the buffer or filter inputs
        // actually change; otherwise reuse last frame's result. This avoids
        // re-filtering and cloning the entire line buffer every frame.
        let dynamic_filter = self.filter_engine.has_dynamic_rules();
        let sig = FilterSig {
            log_version: self.log_state.lock().map(|s| s.version).unwrap_or(0),
            filter_generation: self.filter_engine.generation(),
            levels: [
                self.show_trace,
                self.show_debug,
                self.show_info,
                self.show_warning,
                self.show_error,
                self.show_fatal,
            ],
            selected_source: self.selected_source.clone(),
        };
        if dynamic_filter || self.filtered_cache_sig.as_ref() != Some(&sig) {
            let rebuilt: Vec<std::sync::Arc<DisplayLine>> = {
                let state = self.log_state.lock().unwrap();
                state
                    .lines
                    .iter()
                    .filter(|line| {
                        if let Some(ref selected) = self.selected_source {
                            if &line.entry.source != selected {
                                return false;
                            }
                        }
                        self.line_matches_filter(line)
                    })
                    // Arc clone: a refcount bump, not a deep copy of the line.
                    .cloned()
                    .collect()
            };
            self.filtered_cache = rebuilt;
            // Time-based filters change with no input change, so never cache them.
            self.filtered_cache_sig = if dynamic_filter { None } else { Some(sig) };
            // Row heights no longer correspond to the new view.
            self.wrap_row_heights.clear();
        }
        // Move the cache out so the render closures below can borrow `self`
        // mutably (bookmarks, scroll state) without conflict; restored after.
        let filtered_lines = std::mem::take(&mut self.filtered_cache);

        let total_rows = filtered_lines.len();
        let search_lower = self.search_text.to_lowercase();
        let mut local_scroll_row = self.current_scroll_row;

        // Calculate row height including spacing (what show_rows uses internally)
        let spacing_y = ui.spacing().item_spacing.y;
        let row_height_with_spacing = row_height + spacing_y;

        // Handle bookmark jump - just validate target exists, scroll happens during render via scroll_to_me
        let bookmark_target_line: Option<usize> =
            if let Some(target_line_num) = self.bookmark_jump_target.take() {
                if filtered_lines
                    .iter()
                    .any(|line| line.line_num == target_line_num)
                {
                    self.auto_scroll = false;
                    Some(target_line_num)
                } else {
                    None
                }
            } else {
                None
            };
        let visible_height = ui.available_height();
        // Note: max_offset is approximate for wrap_lines mode, but egui handles clamping
        let content_height = total_rows as f32 * row_height_with_spacing;
        let max_offset = (content_height - visible_height).max(0.0);

        // Build search matches
        if !search_lower.is_empty() {
            let new_matches: Vec<usize> = filtered_lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.entry.message.to_lowercase().contains(&search_lower))
                .map(|(i, _)| i)
                .collect();

            if new_matches != self.search_matches {
                self.search_matches = new_matches;
                // Reset current match if it's now out of bounds
                if let Some(idx) = self.current_match {
                    if idx >= self.search_matches.len() {
                        self.current_match = if self.search_matches.is_empty() {
                            None
                        } else {
                            Some(0)
                        };
                    }
                }
            }
        } else {
            self.search_matches.clear();
            self.current_match = None;
        }

        let mut scroll_request: Option<f32> = None;
        let mut search_nav_forward: Option<bool> = None;
        let mut focus_search: bool = false;
        let text_input_focused = ui.ctx().memory(|m| m.focused().is_some());

        ui.ctx().input(|i| {
            if i.key_pressed(egui::Key::PageDown) {
                let page_size = visible_height * 0.9;
                scroll_request = Some(self.current_scroll_offset + page_size);
                self.auto_scroll = false;
            }
            if i.key_pressed(egui::Key::PageUp) {
                let page_size = visible_height * 0.9;
                scroll_request = Some((self.current_scroll_offset - page_size).max(0.0));
                self.auto_scroll = false;
            }
            if i.key_pressed(egui::Key::Home) {
                scroll_request = Some(0.0);
                self.auto_scroll = false;
            }
            if i.key_pressed(egui::Key::End) {
                // Use 3x max_offset to account for wrapped lines in wrap_lines mode
                scroll_request = Some(max_offset * 3.0);
                self.auto_scroll = true;
            }
            // F3 for next match, Shift+F3 for previous match
            if i.key_pressed(egui::Key::F3) {
                search_nav_forward = Some(!i.modifiers.shift);
            }

            // Vim mode keybindings (only when no text input is focused)
            if self.vim_mode_enabled && !text_input_focused {
                // j - scroll down one line
                if i.key_pressed(egui::Key::J) && !i.modifiers.ctrl {
                    scroll_request = Some(self.current_scroll_offset + row_height_with_spacing);
                    self.auto_scroll = false;
                    self.vim_pending_key = None;
                }
                // k - scroll up one line
                if i.key_pressed(egui::Key::K) && !i.modifiers.ctrl {
                    scroll_request =
                        Some((self.current_scroll_offset - row_height_with_spacing).max(0.0));
                    self.auto_scroll = false;
                    self.vim_pending_key = None;
                }
                // G (shift+g) - jump to end
                if i.key_pressed(egui::Key::G) && i.modifiers.shift {
                    scroll_request = Some(max_offset * 3.0);
                    self.auto_scroll = true;
                    self.vim_pending_key = None;
                }
                // g - first press stores pending, second press jumps to start
                if i.key_pressed(egui::Key::G) && !i.modifiers.shift {
                    if self.vim_pending_key == Some('g') {
                        scroll_request = Some(0.0);
                        self.auto_scroll = false;
                        self.vim_pending_key = None;
                    } else {
                        self.vim_pending_key = Some('g');
                    }
                }
                // Ctrl+d - page down
                if i.key_pressed(egui::Key::D) && i.modifiers.ctrl {
                    let page_size = visible_height * 0.5;
                    scroll_request = Some(self.current_scroll_offset + page_size);
                    self.auto_scroll = false;
                    self.vim_pending_key = None;
                }
                // Ctrl+u - page up
                if i.key_pressed(egui::Key::U) && i.modifiers.ctrl {
                    let page_size = visible_height * 0.5;
                    scroll_request = Some((self.current_scroll_offset - page_size).max(0.0));
                    self.auto_scroll = false;
                    self.vim_pending_key = None;
                }
                // Ctrl+f - page down (alternate)
                if i.key_pressed(egui::Key::F) && i.modifiers.ctrl {
                    let page_size = visible_height * 0.9;
                    scroll_request = Some(self.current_scroll_offset + page_size);
                    self.auto_scroll = false;
                    self.vim_pending_key = None;
                }
                // Ctrl+b - page up (alternate)
                if i.key_pressed(egui::Key::B) && i.modifiers.ctrl {
                    let page_size = visible_height * 0.9;
                    scroll_request = Some((self.current_scroll_offset - page_size).max(0.0));
                    self.auto_scroll = false;
                    self.vim_pending_key = None;
                }
                // / - focus search field
                if i.key_pressed(egui::Key::Slash) {
                    focus_search = true;
                    self.vim_pending_key = None;
                }
                // n - next search match
                if i.key_pressed(egui::Key::N) && !i.modifiers.shift && !i.modifiers.ctrl {
                    search_nav_forward = Some(true);
                    self.vim_pending_key = None;
                }
                // N (shift+n) - previous search match
                if i.key_pressed(egui::Key::N) && i.modifiers.shift {
                    search_nav_forward = Some(false);
                    self.vim_pending_key = None;
                }

                // Clear pending key on any other key press that wasn't g
                if !i.key_pressed(egui::Key::G)
                    && i.keys_down.iter().any(|_| true)
                    && self.vim_pending_key.is_some()
                {
                    self.vim_pending_key = None;
                }
            }
        });

        // Handle search navigation after input block
        if let Some(forward) = search_nav_forward {
            self.navigate_search_match(forward);
        }

        // Focus search field for vim / command
        if focus_search {
            // We'll need to focus the search field - handled via request_focus on the search text edit
            // For now, just clear the search text to indicate focus
        }

        // Determine scroll offset - search navigation and scroll_to_row
        // Bookmark jumps are handled via scroll_to_me() during render
        let scroll_offset: Option<f32> = if let Some(offset) = scroll_request {
            self.scroll_to_row = None;
            Some(offset)
        } else if let Some(row) = self.scroll_to_row.take() {
            Some(row as f32 * row_height_with_spacing)
        } else if total_rows > 0 && self.initial_scroll_pending {
            self.initial_scroll_pending = false;
            Some(max_offset * 3.0) // Account for wrapped lines in wrap_lines mode
        } else {
            None
        };

        // Only enable stick_to_bottom when new lines arrive, not continuously
        // Continuous stick_to_bottom interferes with click detection (bookmarks)
        // and conflicts with manual scroll_offset causing shaking
        let stick_bottom = self.auto_scroll && self.new_lines_received;

        let mut scroll_area = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(stick_bottom);

        if let Some(offset) = scroll_offset {
            scroll_area = scroll_area.vertical_scroll_offset(offset);
        }

        if self.wrap_lines {
            // Variable-height virtualization: render only the rows that
            // intersect the viewport, positioned with per-row heights that
            // were measured on previous frames. Heights are reset when the
            // view or layout width changes, and corrected each frame from the
            // real rendered rect, so the estimate converges as rows are seen.
            let est_height = row_height_with_spacing;
            let width = ui.available_width();
            if self.wrap_row_heights.len() != total_rows
                || (self.wrap_heights_width - width).abs() > 1.0
            {
                self.wrap_row_heights = vec![est_height; total_rows];
                self.wrap_heights_width = width;
            }
            let mut wrap_heights = std::mem::take(&mut self.wrap_row_heights);
            let total_height: f32 = wrap_heights.iter().sum();
            const OVERSCAN: usize = 3;

            let output = scroll_area.show_viewport(ui, |ui, viewport| {
                ui.spacing_mut().item_spacing.y = 4.0 * line_spacing;

                // Locate the first row to render, then back up a few for overscan.
                let mut first = 0usize;
                let mut top_skip = 0.0f32;
                while first < total_rows && top_skip + wrap_heights[first] < viewport.min.y {
                    top_skip += wrap_heights[first];
                    first += 1;
                }
                for _ in 0..OVERSCAN {
                    if first > 0 {
                        first -= 1;
                        top_skip -= wrap_heights[first];
                    }
                }
                local_scroll_row = first.min(total_rows.saturating_sub(1));

                // Reserve the space above the first rendered row.
                ui.add_space(top_skip.max(0.0));

                let mut bookmark_toggle: Option<(String, usize)> = None;
                let mut rendered_bottom = top_skip;
                let mut past = 0usize;
                let mut row = first;
                while row < total_rows {
                    if let Some(line) = filtered_lines.get(row) {
                        let line_entry = line.entry.clone();
                        let line_raw = line.entry.raw.clone();
                        let line_num = line.line_num;
                        let line_source = line.entry.source.clone();
                        let is_bookmarked = self
                            .bookmarks
                            .get(&line_source)
                            .map(|b| b.contains(&line_num))
                            .unwrap_or(false);

                        let row_response = ui.horizontal_wrapped(|ui| {
                            // Bookmark toggle
                            let bookmark_icon = if is_bookmarked { "★" } else { "☆" };
                            let bookmark_color = if is_bookmarked {
                                egui::Color32::from_rgb(255, 200, 50)
                            } else {
                                egui::Color32::from_rgb(100, 100, 100)
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(bookmark_icon)
                                            .size(font_size)
                                            .color(bookmark_color),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Toggle bookmark")
                                .clicked()
                            {
                                bookmark_toggle = Some((line_source.clone(), line_num));
                            }

                            ui.label(
                                egui::RichText::new(format!("{:6} ", line.line_num))
                                    .monospace()
                                    .size(font_size)
                                    .color(egui::Color32::from_rgb(100, 100, 100)),
                            );

                            if show_source && source_count > 1 {
                                let source_color = egui::Color32::from_rgb(100, 150, 200);
                                ui.label(
                                    egui::RichText::new(format!("[{}] ", line.entry.source))
                                        .monospace()
                                        .size(font_size)
                                        .color(source_color),
                                );
                            }

                            if show_timestamps {
                                if let Some(ts) = &line.entry.timestamp {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} ",
                                            ts.format("%H:%M:%S%.3f")
                                        ))
                                        .monospace()
                                        .size(font_size)
                                        .color(egui::Color32::from_rgb(120, 120, 120)),
                                    );
                                }
                            }

                            if let Some(level) = &line.entry.level {
                                let level_color = log_level_color(
                                    Some(level),
                                    Some(&self.config.general.log_level_colors),
                                );
                                ui.label(
                                    egui::RichText::new(format!("[{:5}] ", level.as_str()))
                                        .monospace()
                                        .size(font_size)
                                        .color(level_color),
                                );
                            }

                            let has_search_match = !search_lower.is_empty()
                                && line.entry.message.to_lowercase().contains(&search_lower);

                            let highlight =
                                find_matching_highlight(&highlight_rules, &line.entry.raw);

                            let (fg_color, bg_color, is_bold, is_italic) =
                                if let Some(rule) = highlight {
                                    (
                                        rule.foreground,
                                        Some(rule.background),
                                        rule.bold,
                                        rule.italic,
                                    )
                                } else {
                                    (
                                        log_level_color(
                                            line.entry.level.as_ref(),
                                            Some(&self.config.general.log_level_colors),
                                        ),
                                        None,
                                        false,
                                        false,
                                    )
                                };

                            let mut text = egui::RichText::new(&line.entry.message)
                                .monospace()
                                .size(font_size)
                                .color(fg_color);

                            if is_bold {
                                text = text.strong();
                            }
                            if is_italic {
                                text = text.italics();
                            }
                            if let Some(bg) = bg_color {
                                text = text.background_color(bg);
                            }
                            if has_search_match && bg_color.is_none() {
                                text = text.background_color(egui::Color32::from_rgb(100, 100, 0));
                            }

                            ui.label(text);

                            if !line.entry.fields.is_empty() {
                                ui.label(
                                    egui::RichText::new(" {...}")
                                        .monospace()
                                        .size(font_size * 0.9)
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                )
                                .on_hover_ui(|ui| {
                                    ui.label("JSON Fields:");
                                    for (key, value) in &line.entry.fields {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{}:", key))
                                                    .color(egui::Color32::from_rgb(150, 200, 150)),
                                            );
                                            ui.label(format!("{}", value));
                                        });
                                    }
                                });
                            }
                        });

                        // Record the measured height for next frame's layout.
                        let h = row_response.response.rect.height() + ui.spacing().item_spacing.y;
                        wrap_heights[row] = h;
                        rendered_bottom += h;

                        // Check if this is the bookmark target - scroll to it
                        if bookmark_target_line == Some(line_num) {
                            row_response.response.scroll_to_me(Some(egui::Align::TOP));
                        }

                        // Context menu for copying (use row_response directly, no separate interact)
                        row_response.response.context_menu(|ui| {
                            if ui.button("Copy Line").clicked() {
                                ui.output_mut(|o| o.copied_text = line_entry.message.clone());
                                ui.close_menu();
                            }
                            if ui.button("Copy with Timestamp").clicked() {
                                let text = if let Some(ts) = &line_entry.timestamp {
                                    format!(
                                        "{} {}",
                                        ts.format("%Y-%m-%d %H:%M:%S%.3f"),
                                        line_entry.message
                                    )
                                } else {
                                    line_entry.message.clone()
                                };
                                ui.output_mut(|o| o.copied_text = text);
                                ui.close_menu();
                            }
                            if ui.button("Copy Raw").clicked() {
                                ui.output_mut(|o| o.copied_text = line_raw.clone());
                                ui.close_menu();
                            }
                        });

                        // Handle bookmark toggle
                        if let Some((source, ln)) = bookmark_toggle.take() {
                            let source_bookmarks = self.bookmarks.entry(source).or_default();
                            if source_bookmarks.contains(&ln) {
                                source_bookmarks.remove(&ln);
                            } else {
                                source_bookmarks.insert(ln);
                            }
                        }
                    }

                    row += 1;
                    // Stop a few rows past the bottom edge (overscan).
                    if rendered_bottom > viewport.max.y {
                        past += 1;
                        if past >= OVERSCAN {
                            break;
                        }
                    }
                }

                // Reserve the remaining space below so the scrollbar maps to
                // the full (estimated) content height.
                let bottom = (total_height - rendered_bottom).max(0.0);
                ui.add_space(bottom);
            });
            self.wrap_row_heights = wrap_heights;
            // Track scroll offset for pixel-based Page Up/Down
            self.current_scroll_offset = output.state.offset.y;
        } else {
            let output = scroll_area.show_rows(ui, row_height, total_rows, |ui, row_range| {
                local_scroll_row = row_range.start;
                let mut bookmark_toggle: Option<(String, usize)> = None;

                for row in row_range {
                    if let Some(line) = filtered_lines.get(row) {
                        let line_entry = line.entry.clone();
                        let line_raw = line.entry.raw.clone();
                        let line_num = line.line_num;
                        let line_source = line.entry.source.clone();
                        let is_bookmarked = self
                            .bookmarks
                            .get(&line_source)
                            .map(|b| b.contains(&line_num))
                            .unwrap_or(false);
                        let row_response = ui.horizontal(|ui| {
                            // Bookmark toggle
                            let bookmark_icon = if is_bookmarked { "★" } else { "☆" };
                            let bookmark_color = if is_bookmarked {
                                egui::Color32::from_rgb(255, 200, 50)
                            } else {
                                egui::Color32::from_rgb(100, 100, 100)
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(bookmark_icon)
                                            .size(font_size)
                                            .color(bookmark_color),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Toggle bookmark")
                                .clicked()
                            {
                                bookmark_toggle = Some((line_source.clone(), line_num));
                            }

                            ui.label(
                                egui::RichText::new(format!("{:6} ", line.line_num))
                                    .monospace()
                                    .size(font_size)
                                    .color(egui::Color32::from_rgb(100, 100, 100)),
                            );

                            if show_source && source_count > 1 {
                                let source_color = egui::Color32::from_rgb(100, 150, 200);
                                ui.label(
                                    egui::RichText::new(format!("[{}] ", line.entry.source))
                                        .monospace()
                                        .size(font_size)
                                        .color(source_color),
                                );
                            }

                            if show_timestamps {
                                if let Some(ts) = &line.entry.timestamp {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} ",
                                            ts.format("%H:%M:%S%.3f")
                                        ))
                                        .monospace()
                                        .size(font_size)
                                        .color(egui::Color32::from_rgb(120, 120, 120)),
                                    );
                                }
                            }

                            if let Some(level) = &line.entry.level {
                                let level_color = log_level_color(
                                    Some(level),
                                    Some(&self.config.general.log_level_colors),
                                );
                                ui.label(
                                    egui::RichText::new(format!("[{:5}] ", level.as_str()))
                                        .monospace()
                                        .size(font_size)
                                        .color(level_color),
                                );
                            }

                            let has_search_match = !search_lower.is_empty()
                                && line.entry.message.to_lowercase().contains(&search_lower);

                            let display_text = &line.entry.message;
                            let highlight =
                                find_matching_highlight(&highlight_rules, &line.entry.raw);

                            if let Some(rule) = highlight {
                                let mut text = egui::RichText::new(display_text)
                                    .monospace()
                                    .size(font_size)
                                    .color(rule.foreground)
                                    .background_color(rule.background);

                                if rule.bold {
                                    text = text.strong();
                                }
                                if rule.italic {
                                    text = text.italics();
                                }

                                ui.label(text);
                            } else if line.has_ansi {
                                for span in &line.spans {
                                    let mut text = egui::RichText::new(&span.text)
                                        .monospace()
                                        .size(font_size)
                                        .color(span.color);

                                    if span.bold {
                                        text = text.strong();
                                    }

                                    if has_search_match
                                        && span.text.to_lowercase().contains(&search_lower)
                                    {
                                        text = text
                                            .background_color(egui::Color32::from_rgb(100, 100, 0));
                                    }

                                    ui.label(text);
                                }
                            } else {
                                let color = log_level_color(
                                    line.entry.level.as_ref(),
                                    Some(&self.config.general.log_level_colors),
                                );
                                let mut text = egui::RichText::new(display_text)
                                    .monospace()
                                    .size(font_size)
                                    .color(color);

                                if has_search_match {
                                    text =
                                        text.background_color(egui::Color32::from_rgb(100, 100, 0));
                                }

                                ui.label(text);
                            }

                            if !line.entry.fields.is_empty() {
                                ui.label(
                                    egui::RichText::new(" {...}")
                                        .monospace()
                                        .size(font_size * 0.9)
                                        .color(egui::Color32::from_rgb(100, 100, 100)),
                                )
                                .on_hover_ui(|ui| {
                                    ui.label("JSON Fields:");
                                    for (key, value) in &line.entry.fields {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!("{}:", key))
                                                    .color(egui::Color32::from_rgb(150, 200, 150)),
                                            );
                                            ui.label(format!("{}", value));
                                        });
                                    }
                                });
                            }
                        });

                        // Check if this is the bookmark target - scroll to it
                        if bookmark_target_line == Some(line_num) {
                            row_response.response.scroll_to_me(Some(egui::Align::TOP));
                        }

                        // Context menu for copying (use row_response directly, no separate interact)
                        row_response.response.context_menu(|ui| {
                            if ui.button("Copy Line").clicked() {
                                ui.output_mut(|o| o.copied_text = line_entry.message.clone());
                                ui.close_menu();
                            }
                            if ui.button("Copy with Timestamp").clicked() {
                                let text = if let Some(ts) = &line_entry.timestamp {
                                    format!(
                                        "{} {}",
                                        ts.format("%Y-%m-%d %H:%M:%S%.3f"),
                                        line_entry.message
                                    )
                                } else {
                                    line_entry.message.clone()
                                };
                                ui.output_mut(|o| o.copied_text = text);
                                ui.close_menu();
                            }
                            if ui.button("Copy Raw").clicked() {
                                ui.output_mut(|o| o.copied_text = line_raw.clone());
                                ui.close_menu();
                            }
                        });

                        // Handle bookmark toggle
                        if let Some((source, ln)) = bookmark_toggle.take() {
                            let source_bookmarks = self.bookmarks.entry(source).or_default();
                            if source_bookmarks.contains(&ln) {
                                source_bookmarks.remove(&ln);
                            } else {
                                source_bookmarks.insert(ln);
                            }
                        }
                    }
                }
            });
            // Track scroll offset for pixel-based Page Up/Down
            self.current_scroll_offset = output.state.offset.y;
        }

        // Always sync current_scroll_row from scroll area state
        self.current_scroll_row = local_scroll_row;

        // Return the filtered view to the cache for reuse next frame.
        self.filtered_cache = filtered_lines;
    }
}
