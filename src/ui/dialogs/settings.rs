//! Settings configuration dialog.

use crate::config::LogLevelColors;
use crate::Theme;
use eframe::egui;

/// State for the settings dialog
pub struct SettingsDialogState {
    pub open: bool,
    pub buffer_size: String,
    pub font_size: f32,
    pub show_timestamps: bool,
    pub show_source: bool,
    pub wrap_lines: bool,
    pub auto_scroll: bool,
    pub use_auto_parser: bool,
    pub line_spacing: f32,
    pub tab_width: String,
    pub update_interval_ms: String,
    pub theme: Theme,
    pub remember_last_session: bool,
    pub vim_mode: bool,
    pub log_level_colors: LogLevelColors,
}

impl Default for SettingsDialogState {
    fn default() -> Self {
        Self {
            open: false,
            buffer_size: "10000".to_string(),
            font_size: 13.0,
            show_timestamps: true,
            show_source: true,
            wrap_lines: false,
            auto_scroll: true,
            use_auto_parser: true,
            line_spacing: 1.0,
            tab_width: "4".to_string(),
            update_interval_ms: "100".to_string(),
            theme: Theme::System,
            remember_last_session: true,
            vim_mode: false,
            log_level_colors: LogLevelColors::default(),
        }
    }
}

/// Result of the settings dialog
pub enum SettingsDialogResult {
    None,
    Apply,
    Save,
    OpenHighlightDialog,
}

/// Render the settings dialog and return the result
pub fn render_settings_dialog(
    ctx: &egui::Context,
    dialog: &mut SettingsDialogState,
    config_path: &std::path::Path,
    highlight_rules_count: usize,
) -> SettingsDialogResult {
    if !dialog.open {
        return SettingsDialogResult::None;
    }

    let mut result = SettingsDialogResult::None;

    egui::Window::new("Settings")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.add_space(5.0);

            egui::Grid::new("settings_grid")
                .num_columns(2)
                .spacing([20.0, 8.0])
                .show(ui, |ui| {
                    // Display settings
                    ui.label(egui::RichText::new("Display").strong());
                    ui.end_row();

                    ui.label("Font size:");
                    ui.add(egui::Slider::new(&mut dialog.font_size, 8.0..=24.0));
                    ui.end_row();

                    ui.label("Line spacing:");
                    ui.add(egui::Slider::new(&mut dialog.line_spacing, 0.5..=3.0).step_by(0.1));
                    ui.end_row();

                    ui.label("Tab width:");
                    ui.add(egui::TextEdit::singleline(&mut dialog.tab_width).desired_width(60.0));
                    ui.end_row();

                    ui.label("Show timestamps:");
                    ui.checkbox(&mut dialog.show_timestamps, "");
                    ui.end_row();

                    ui.label("Show source:");
                    ui.checkbox(&mut dialog.show_source, "");
                    ui.end_row();

                    ui.label("Wrap lines:");
                    ui.checkbox(&mut dialog.wrap_lines, "");
                    ui.end_row();

                    ui.label("");
                    ui.end_row();

                    // Behavior settings
                    ui.label(egui::RichText::new("Behavior").strong());
                    ui.end_row();

                    ui.label("Auto-scroll:");
                    ui.checkbox(&mut dialog.auto_scroll, "");
                    ui.end_row();

                    ui.label("Auto-parse JSON:");
                    ui.checkbox(&mut dialog.use_auto_parser, "");
                    ui.end_row();

                    ui.label("Buffer size:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.buffer_size).desired_width(100.0),
                    );
                    ui.end_row();

                    ui.label("Update interval (ms):");
                    ui.add(
                        egui::TextEdit::singleline(&mut dialog.update_interval_ms)
                            .desired_width(80.0),
                    );
                    ui.end_row();

                    ui.label("Remember last session:");
                    ui.checkbox(&mut dialog.remember_last_session, "");
                    ui.end_row();

                    ui.label("Vim keybindings:");
                    ui.checkbox(&mut dialog.vim_mode, "").on_hover_text(
                        "j/k scroll, G/gg jump, Ctrl+d/u page, / search, n/N next/prev match",
                    );
                    ui.end_row();

                    ui.label("");
                    ui.end_row();

                    // Appearance settings
                    ui.label(egui::RichText::new("Appearance").strong());
                    ui.end_row();

                    ui.label("Theme:");
                    egui::ComboBox::from_id_salt("theme_selector")
                        .selected_text(dialog.theme.as_str())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut dialog.theme, Theme::System, "System");
                            ui.selectable_value(&mut dialog.theme, Theme::Light, "Light");
                            ui.selectable_value(&mut dialog.theme, Theme::Dark, "Dark");
                        });
                    ui.end_row();

                    ui.label("");
                    ui.end_row();

                    // Log Level Colors
                    ui.label(egui::RichText::new("Log Level Colors").strong());
                    ui.end_row();

                    fn color_picker_row(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3]) {
                        ui.label(label);
                        let mut egui_color = egui::Color32::from_rgb(color[0], color[1], color[2]);
                        if egui::color_picker::color_edit_button_srgba(
                            ui,
                            &mut egui_color,
                            egui::color_picker::Alpha::Opaque,
                        )
                        .changed()
                        {
                            *color = [egui_color.r(), egui_color.g(), egui_color.b()];
                        }
                        ui.end_row();
                    }

                    color_picker_row(ui, "Trace:", &mut dialog.log_level_colors.trace);
                    color_picker_row(ui, "Debug:", &mut dialog.log_level_colors.debug);
                    color_picker_row(ui, "Info:", &mut dialog.log_level_colors.info);
                    color_picker_row(ui, "Warn:", &mut dialog.log_level_colors.warn);
                    color_picker_row(ui, "Error:", &mut dialog.log_level_colors.error);
                    color_picker_row(ui, "Fatal:", &mut dialog.log_level_colors.fatal);

                    ui.label("");
                    ui.end_row();

                    // Highlighting
                    ui.label(egui::RichText::new("Highlighting").strong());
                    ui.end_row();

                    ui.label("Highlight rules:");
                    if ui
                        .button(format!("Configure ({} rules)", highlight_rules_count))
                        .clicked()
                    {
                        result = SettingsDialogResult::OpenHighlightDialog;
                    }
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Config: {}", config_path.display()))
                        .small()
                        .color(egui::Color32::from_rgb(120, 120, 120)),
                );
            });

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    dialog.open = false;
                }

                if ui.button("Apply").clicked() {
                    result = SettingsDialogResult::Apply;
                    dialog.open = false;
                }

                if ui.button("Save to File").clicked() {
                    result = SettingsDialogResult::Save;
                    dialog.open = false;
                }
            });
        });

    result
}
