//! Alert rules configuration dialog.

use crate::alert::{AlertAction, AlertRule};
use crate::filter::FilterRule;
use eframe::egui;

/// State for the alert dialog
#[derive(Default)]
pub struct AlertDialogState {
    pub open: bool,
    pub editing_index: Option<usize>,
    pub name: String,
    pub pattern: String,
    pub use_regex: bool,
    pub sound_enabled: bool,
    pub desktop_enabled: bool,
    pub desktop_title: String,
    pub webhook_enabled: bool,
    pub webhook_url: String,
    pub cooldown_seconds: String,
}

impl AlertDialogState {
    pub fn reset_form(&mut self) {
        self.editing_index = None;
        self.name.clear();
        self.pattern.clear();
        self.use_regex = false;
        self.sound_enabled = false;
        self.desktop_enabled = false;
        self.desktop_title.clear();
        self.webhook_enabled = false;
        self.webhook_url.clear();
        self.cooldown_seconds.clear();
    }

    pub fn load_rule(&mut self, index: usize, rule: &AlertRule) {
        self.editing_index = Some(index);
        self.name = rule.name.clone();

        // Extract pattern from FilterRule
        match &rule.pattern {
            FilterRule::Contains { pattern, .. } => {
                self.pattern = pattern.clone();
                self.use_regex = false;
            }
            FilterRule::Regex { pattern } => {
                self.pattern = pattern.clone();
                self.use_regex = true;
            }
            _ => {
                self.pattern.clear();
                self.use_regex = false;
            }
        }

        // Extract actions
        self.sound_enabled = false;
        self.desktop_enabled = false;
        self.desktop_title.clear();
        self.webhook_enabled = false;
        self.webhook_url.clear();

        for action in &rule.actions {
            match action {
                AlertAction::Sound => self.sound_enabled = true,
                AlertAction::Desktop { title } => {
                    self.desktop_enabled = true;
                    self.desktop_title = title.clone().unwrap_or_default();
                }
                AlertAction::Webhook { url } => {
                    self.webhook_enabled = true;
                    self.webhook_url = url.clone();
                }
                AlertAction::Visual => {}
            }
        }

        self.cooldown_seconds = rule
            .cooldown_seconds
            .map(|s| s.to_string())
            .unwrap_or_default();
    }

    pub fn build_rule(&self) -> Option<AlertRule> {
        if self.name.is_empty() || self.pattern.is_empty() {
            return None;
        }

        let pattern = if self.use_regex {
            FilterRule::Regex {
                pattern: self.pattern.clone(),
            }
        } else {
            FilterRule::Contains {
                pattern: self.pattern.clone(),
                case_sensitive: false,
            }
        };

        let mut actions = Vec::new();

        if self.sound_enabled {
            actions.push(AlertAction::Sound);
        }

        if self.desktop_enabled {
            actions.push(AlertAction::Desktop {
                title: if self.desktop_title.is_empty() {
                    None
                } else {
                    Some(self.desktop_title.clone())
                },
            });
        }

        if self.webhook_enabled && !self.webhook_url.is_empty() {
            actions.push(AlertAction::Webhook {
                url: self.webhook_url.clone(),
            });
        }

        let cooldown_seconds = self.cooldown_seconds.parse::<u64>().ok();

        Some(AlertRule {
            name: self.name.clone(),
            pattern,
            actions,
            cooldown_seconds,
        })
    }
}

/// Result of the alert dialog
pub enum AlertDialogResult {
    None,
    Save(AlertRule),
    Update(usize, AlertRule),
    Delete(usize),
}

/// Render the alert dialog and return the result
pub fn render_alert_dialog(
    ctx: &egui::Context,
    dialog: &mut AlertDialogState,
    rules: &[AlertRule],
) -> AlertDialogResult {
    if !dialog.open {
        return AlertDialogResult::None;
    }

    let mut result = AlertDialogResult::None;
    let mut close_dialog = false;

    egui::Window::new("Alert Rules")
        .collapsible(false)
        .resizable(true)
        .default_size([450.0, 500.0])
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(5.0);

            // Existing rules list
            ui.label(egui::RichText::new("Existing Rules:").strong());
            ui.add_space(5.0);

            egui::ScrollArea::vertical()
                .max_height(150.0)
                .show(ui, |ui| {
                    if rules.is_empty() {
                        ui.label(
                            egui::RichText::new("No alert rules configured")
                                .italics()
                                .color(egui::Color32::from_rgb(150, 150, 150)),
                        );
                    } else {
                        let mut edit_idx: Option<usize> = None;
                        let mut delete_idx: Option<usize> = None;

                        for (i, rule) in rules.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let is_editing = dialog.editing_index == Some(i);
                                let marker = if is_editing { "▶" } else { "●" };
                                let marker_color = if is_editing {
                                    egui::Color32::from_rgb(100, 200, 100)
                                } else {
                                    egui::Color32::from_rgb(100, 150, 200)
                                };

                                ui.colored_label(marker_color, marker);
                                ui.label(&rule.name);

                                // Show pattern summary
                                let pattern_text = match &rule.pattern {
                                    FilterRule::Contains { pattern, .. } => {
                                        format!("\"{}\"", pattern)
                                    }
                                    FilterRule::Regex { pattern } => format!("/{}/", pattern),
                                    _ => "...".to_string(),
                                };
                                ui.label(
                                    egui::RichText::new(pattern_text)
                                        .small()
                                        .color(egui::Color32::from_rgb(150, 150, 150)),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("🗑").on_hover_text("Delete").clicked()
                                        {
                                            delete_idx = Some(i);
                                        }
                                        if ui.small_button("✎").on_hover_text("Edit").clicked() {
                                            edit_idx = Some(i);
                                        }
                                    },
                                );
                            });
                        }

                        if let Some(idx) = edit_idx {
                            if let Some(rule) = rules.get(idx) {
                                dialog.load_rule(idx, rule);
                            }
                        }

                        if let Some(idx) = delete_idx {
                            result = AlertDialogResult::Delete(idx);
                            if dialog.editing_index == Some(idx) {
                                dialog.reset_form();
                            }
                        }
                    }
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Add/Edit form
            let form_title = if dialog.editing_index.is_some() {
                "Edit Rule:"
            } else {
                "Add New Rule:"
            };
            ui.label(egui::RichText::new(form_title).strong());
            ui.add_space(5.0);

            egui::Grid::new("alert_form_grid")
                .num_columns(2)
                .spacing([20.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.name)
                            .hint_text("Rule name")
                            .desired_width(250.0),
                    );
                    ui.end_row();

                    ui.label("Pattern:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.pattern)
                            .hint_text("Text to match")
                            .desired_width(250.0),
                    );
                    ui.end_row();

                    ui.label("");
                    ui.checkbox(&mut dialog.use_regex, "Use Regex");
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.label(egui::RichText::new("Actions:").strong());
            ui.add_space(5.0);

            ui.checkbox(&mut dialog.sound_enabled, "Sound (beep)");

            ui.horizontal(|ui| {
                ui.checkbox(&mut dialog.desktop_enabled, "Desktop notification");
            });

            if dialog.desktop_enabled {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label("Title:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.desktop_title)
                            .hint_text("Optional custom title")
                            .desired_width(200.0),
                    );
                });
            }

            ui.horizontal(|ui| {
                ui.checkbox(&mut dialog.webhook_enabled, "Webhook");
            });

            if dialog.webhook_enabled {
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.label("URL:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.webhook_url)
                            .hint_text("https://...")
                            .desired_width(220.0),
                    );
                });
            }

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label("Cooldown:");
                ui.add(
                    egui::TextEdit::singleline(&mut dialog.cooldown_seconds)
                        .hint_text("seconds")
                        .desired_width(60.0),
                );
                ui.label("seconds (optional)");
            });

            ui.add_space(15.0);
            ui.separator();
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("Close").clicked() {
                    close_dialog = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save_text = if dialog.editing_index.is_some() {
                        "Update Rule"
                    } else {
                        "Add Rule"
                    };

                    if ui.button(save_text).clicked() {
                        if let Some(rule) = dialog.build_rule() {
                            if let Some(idx) = dialog.editing_index {
                                result = AlertDialogResult::Update(idx, rule);
                            } else {
                                result = AlertDialogResult::Save(rule);
                            }
                            dialog.reset_form();
                        }
                    }

                    if dialog.editing_index.is_some() && ui.button("Cancel Edit").clicked() {
                        dialog.reset_form();
                    }
                });
            });
        });

    if close_dialog {
        dialog.open = false;
        dialog.reset_form();
    }

    result
}
