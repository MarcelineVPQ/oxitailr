//! Highlight rules configuration dialog.

use eframe::egui;

/// Highlight rule for configurable highlighting
#[derive(Clone)]
pub struct HighlightRule {
    pub pattern: String,
    pub foreground: egui::Color32,
    pub background: egui::Color32,
    pub bold: bool,
    pub italic: bool,
    pub ignore_case: bool,
    pub enabled: bool,
}

impl Default for HighlightRule {
    fn default() -> Self {
        Self {
            pattern: String::new(),
            foreground: egui::Color32::WHITE,
            background: egui::Color32::from_rgb(200, 50, 50),
            bold: false,
            italic: false,
            ignore_case: true,
            enabled: true,
        }
    }
}

impl HighlightRule {
    pub fn matches(&self, text: &str) -> bool {
        if self.pattern.is_empty() || !self.enabled {
            return false;
        }
        if self.ignore_case {
            text.to_lowercase().contains(&self.pattern.to_lowercase())
        } else {
            text.contains(&self.pattern)
        }
    }
}

/// State for the highlight dialog
#[derive(Default)]
pub struct HighlightDialogState {
    pub open: bool,
    pub editing_index: Option<usize>,
    pub current_rule: HighlightRule,
}

/// Find a matching highlight rule for a line of text
pub fn find_matching_highlight<'a>(rules: &'a [HighlightRule], text: &str) -> Option<&'a HighlightRule> {
    rules.iter().find(|rule| rule.matches(text))
}

/// Get default highlight rules
pub fn default_highlight_rules() -> Vec<HighlightRule> {
    vec![
        HighlightRule {
            pattern: "ERROR".to_string(),
            foreground: egui::Color32::WHITE,
            background: egui::Color32::from_rgb(180, 40, 40),
            bold: true,
            ignore_case: true,
            enabled: true,
            ..Default::default()
        },
        HighlightRule {
            pattern: "WARN".to_string(),
            foreground: egui::Color32::BLACK,
            background: egui::Color32::from_rgb(230, 180, 50),
            bold: false,
            ignore_case: true,
            enabled: true,
            ..Default::default()
        },
    ]
}

/// Render the highlight dialog
pub fn render_highlight_dialog(
    ctx: &egui::Context,
    dialog: &mut HighlightDialogState,
    rules: &mut Vec<HighlightRule>,
) {
    if !dialog.open {
        return;
    }

    let mut close_dialog = false;
    let mut delete_rule: Option<usize> = None;
    let mut edit_rule: Option<usize> = None;

    egui::Window::new("Highlight Rules")
        .collapsible(false)
        .resizable(true)
        .default_size([500.0, 400.0])
        .show(ctx, |ui| {
            ui.heading("Configured Rules");
            ui.add_space(5.0);

            // List existing rules
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (i, rule) in rules.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut rule.enabled, "");

                            // Preview color
                            let preview_size = egui::vec2(60.0, 18.0);
                            let (rect, _) =
                                ui.allocate_exact_size(preview_size, egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, rule.background);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "Sample",
                                egui::FontId::monospace(12.0),
                                rule.foreground,
                            );

                            ui.label(&rule.pattern);

                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("x").clicked() {
                                        delete_rule = Some(i);
                                    }
                                    if ui.small_button("Edit").clicked() {
                                        edit_rule = Some(i);
                                    }
                                },
                            );
                        });
                    }
                });

            ui.separator();
            ui.add_space(5.0);

            // Rule editor
            let editing_text = if dialog.editing_index.is_some() {
                "Edit Rule"
            } else {
                "New Rule"
            };
            ui.heading(editing_text);
            ui.add_space(5.0);

            egui::Grid::new("highlight_editor")
                .num_columns(2)
                .spacing([10.0, 5.0])
                .show(ui, |ui| {
                    ui.label("Pattern:");
                    ui.text_edit_singleline(&mut dialog.current_rule.pattern);
                    ui.end_row();

                    ui.label("Ignore case:");
                    ui.checkbox(&mut dialog.current_rule.ignore_case, "");
                    ui.end_row();

                    ui.label("Foreground:");
                    ui.color_edit_button_srgba(&mut dialog.current_rule.foreground);
                    ui.end_row();

                    ui.label("Background:");
                    ui.color_edit_button_srgba(&mut dialog.current_rule.background);
                    ui.end_row();

                    ui.label("Bold:");
                    ui.checkbox(&mut dialog.current_rule.bold, "");
                    ui.end_row();

                    ui.label("Italic:");
                    ui.checkbox(&mut dialog.current_rule.italic, "");
                    ui.end_row();
                });

            ui.add_space(10.0);

            // Preview
            ui.horizontal(|ui| {
                ui.label("Preview:");
                let preview_size = egui::vec2(150.0, 20.0);
                let (rect, _) = ui.allocate_exact_size(preview_size, egui::Sense::hover());
                ui.painter()
                    .rect_filled(rect, 2.0, dialog.current_rule.background);

                let font_id = egui::FontId::monospace(13.0);
                let sample_text = if dialog.current_rule.pattern.is_empty() {
                    "Sample Text"
                } else {
                    &dialog.current_rule.pattern
                };
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    sample_text,
                    font_id,
                    dialog.current_rule.foreground,
                );
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if dialog.editing_index.is_some() {
                    if ui.button("Update").clicked() {
                        if let Some(idx) = dialog.editing_index {
                            if idx < rules.len() {
                                rules[idx] = dialog.current_rule.clone();
                            }
                        }
                        dialog.editing_index = None;
                        dialog.current_rule = HighlightRule::default();
                    }
                    if ui.button("Cancel Edit").clicked() {
                        dialog.editing_index = None;
                        dialog.current_rule = HighlightRule::default();
                    }
                } else if ui.button("Add Rule").clicked()
                    && !dialog.current_rule.pattern.is_empty()
                {
                    let new_rule = dialog.current_rule.clone();
                    rules.push(new_rule);
                    dialog.current_rule = HighlightRule::default();
                }
            });

            ui.separator();
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    close_dialog = true;
                }
            });
        });

    // Handle deferred actions
    if let Some(idx) = delete_rule {
        if idx < rules.len() {
            rules.remove(idx);
        }
    }

    if let Some(idx) = edit_rule {
        if idx < rules.len() {
            dialog.editing_index = Some(idx);
            dialog.current_rule = rules[idx].clone();
        }
    }

    if close_dialog {
        dialog.open = false;
    }
}
