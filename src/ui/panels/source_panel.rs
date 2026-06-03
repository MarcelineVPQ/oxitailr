//! Source management side panel.

use crate::get_ssh_password;
use crate::models::{self, SourceStatus};
use crate::{SavedLocalSource, SavedSshSource};
use eframe::egui;
use std::path::PathBuf;

impl crate::TailLoggerApp {
    pub(crate) fn render_source_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sources");
        ui.separator();

        let source_names: Vec<String> = self.source_infos.keys().cloned().collect();
        let mut to_remove = Vec::new();
        let mut to_edit: Option<String> = None;
        let mut toggle_auto_open_local: Option<String> = None;
        let mut toggle_auto_open_ssh: Option<String> = None;

        for name in &source_names {
            if let Some(info) = self.source_infos.get(name) {
                ui.horizontal(|ui| {
                    let is_auto_open = match info.source_type {
                        models::SourceType::Local => self.is_local_source_auto_open(&info.path),
                        models::SourceType::Ssh => self.is_ssh_source_auto_open(name),
                    };
                    let star = if is_auto_open { "★" } else { "☆" };
                    let star_color = if is_auto_open {
                        egui::Color32::from_rgb(255, 200, 50)
                    } else {
                        egui::Color32::from_rgb(100, 100, 100)
                    };
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(star).color(star_color))
                                .frame(false),
                        )
                        .on_hover_text("Toggle auto-open on startup")
                        .clicked()
                    {
                        match info.source_type {
                            models::SourceType::Local => {
                                toggle_auto_open_local = Some(info.path.clone())
                            }
                            models::SourceType::Ssh => toggle_auto_open_ssh = Some(name.clone()),
                        }
                    }

                    let status_color = match info.status {
                        SourceStatus::Connected => egui::Color32::from_rgb(50, 205, 50),
                        SourceStatus::Connecting => egui::Color32::from_rgb(255, 200, 0),
                        SourceStatus::Disconnected => egui::Color32::from_rgb(150, 150, 150),
                        SourceStatus::Error => egui::Color32::from_rgb(255, 50, 50),
                    };

                    ui.colored_label(status_color, info.status_symbol());
                    ui.label(&info.name);
                    ui.label(format!("({})", info.source_type));

                    if info.source_type == models::SourceType::Ssh
                        && ui.small_button("Edit").clicked()
                    {
                        to_edit = Some(name.clone());
                    }

                    if ui.small_button("x").clicked() {
                        to_remove.push(name.clone());
                    }
                });

                ui.label(
                    egui::RichText::new(format!("  {} lines", info.line_count))
                        .small()
                        .color(egui::Color32::from_rgb(150, 150, 150)),
                );
            }
        }

        if let Some(path) = toggle_auto_open_local {
            if self.find_saved_local_source(&path).is_none() {
                let name = PathBuf::from(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "local".to_string());
                self.update_or_add_local_source(SavedLocalSource {
                    name,
                    path: path.clone(),
                    auto_open: true,
                });
            } else {
                self.toggle_local_source_auto_open(&path);
            }
        }
        if let Some(name) = toggle_auto_open_ssh {
            self.toggle_ssh_source_auto_open(&name);
        }

        if let Some(name) = to_edit {
            if let Some(saved) = self.find_saved_ssh_source(&name).cloned() {
                self.ssh_dialog.load_from_saved(&saved);
                self.ssh_dialog.open = true;
            }
        }

        for name in to_remove {
            self.remove_source(&name);
        }

        ui.separator();

        // Saved SSH sources section
        if !self.saved_ssh_sources.is_empty() {
            ui.label(
                egui::RichText::new("Saved SSH Sources")
                    .small()
                    .color(egui::Color32::from_rgb(150, 150, 150)),
            );

            let saved_names: Vec<String> = self
                .saved_ssh_sources
                .iter()
                .map(|s| s.name.clone())
                .collect();
            let active_names: std::collections::HashSet<String> =
                self.source_infos.keys().cloned().collect();

            let mut connect_source: Option<SavedSshSource> = None;
            let mut delete_source: Option<String> = None;
            let mut edit_saved: Option<SavedSshSource> = None;
            let mut toggle_saved_auto_open: Option<String> = None;

            for name in &saved_names {
                let is_active = active_names.contains(name);
                if !is_active {
                    ui.horizontal(|ui| {
                        let is_auto_open = self.is_ssh_source_auto_open(name);
                        let star = if is_auto_open { "★" } else { "☆" };
                        let star_color = if is_auto_open {
                            egui::Color32::from_rgb(255, 200, 50)
                        } else {
                            egui::Color32::from_rgb(100, 100, 100)
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(star).color(star_color).small(),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Toggle auto-open on startup")
                            .clicked()
                        {
                            toggle_saved_auto_open = Some(name.clone());
                        }

                        ui.label(
                            egui::RichText::new(name)
                                .small()
                                .color(egui::Color32::from_rgb(120, 120, 120)),
                        );

                        if ui.small_button("Connect").clicked() {
                            if let Some(saved) = self.find_saved_ssh_source(name).cloned() {
                                connect_source = Some(saved);
                            }
                        }

                        if ui.small_button("Edit").clicked() {
                            if let Some(saved) = self.find_saved_ssh_source(name).cloned() {
                                edit_saved = Some(saved);
                            }
                        }

                        if ui.small_button("x").clicked() {
                            delete_source = Some(name.clone());
                        }
                    });
                }
            }

            if let Some(name) = toggle_saved_auto_open {
                self.toggle_ssh_source_auto_open(&name);
            }

            if let Some(source) = connect_source {
                let key_path = source.key_path.as_ref().map(PathBuf::from);
                let password = get_ssh_password(&source.name);
                self.add_ssh_source(
                    source.name,
                    source.host,
                    source.port,
                    source.user,
                    source.remote_path,
                    key_path,
                    password,
                );
            }

            if let Some(source) = edit_saved {
                self.ssh_dialog.load_from_saved(&source);
                self.ssh_dialog.open = true;
            }

            if let Some(name) = delete_source {
                self.remove_saved_ssh_source(&name);
            }

            ui.separator();
        }

        ui.horizontal(|ui| {
            if ui.button("+ Local File").clicked() {
                self.open_file_dialog();
            }

            if ui.button("+ SSH").clicked() {
                self.ssh_dialog.open = true;
                self.ssh_dialog.reset();
                self.ssh_dialog.port = "22".to_string();
            }
        });
    }
}
