//! SSH source configuration dialog.

use crate::credentials::get_ssh_password;
use eframe::egui;

/// State for the SSH source dialog
#[derive(Default)]
pub struct SshDialogState {
    pub open: bool,
    pub editing: Option<String>,
    pub name: String,
    pub host: String,
    pub port: String,
    pub user: String,
    pub remote_path: String,
    pub key_path: String,
    pub password: String,
    pub error: Option<String>,
    pub auto_open: bool,
}

impl SshDialogState {
    pub fn reset(&mut self) {
        self.editing = None;
        self.name.clear();
        self.host.clear();
        self.port = "22".to_string();
        self.user.clear();
        self.remote_path.clear();
        self.key_path.clear();
        self.password.clear();
        self.error = None;
        self.auto_open = false;
    }

    pub fn load_from_saved(&mut self, source: &SavedSshSource) {
        self.editing = Some(source.name.clone());
        self.name = source.name.clone();
        self.host = source.host.clone();
        self.port = source.port.to_string();
        self.user = source.user.clone();
        self.remote_path = source.remote_path.clone();
        self.key_path = source.key_path.clone().unwrap_or_default();
        self.password = get_ssh_password(&source.name).unwrap_or_default();
        self.error = None;
        self.auto_open = source.auto_open;
    }

    pub fn to_saved(&self) -> SavedSshSource {
        SavedSshSource {
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port.parse().unwrap_or(22),
            user: self.user.clone(),
            remote_path: self.remote_path.clone(),
            key_path: if self.key_path.is_empty() {
                None
            } else {
                Some(self.key_path.clone())
            },
            auto_open: self.auto_open,
        }
    }
}

/// Saved SSH source for JSON persistence
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SavedSshSource {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub remote_path: String,
    pub key_path: Option<String>,
    #[serde(default)]
    pub auto_open: bool,
}

/// Render result from the SSH dialog
pub enum SshDialogResult {
    None,
    Save(SavedSshSource, Option<String>),
    Connect(SavedSshSource, Option<String>),
}

/// Render the SSH dialog and return the result
pub fn render_ssh_dialog(
    ctx: &egui::Context,
    dialog: &mut SshDialogState,
    ssh_sources_path: &std::path::Path,
) -> SshDialogResult {
    if !dialog.open {
        return SshDialogResult::None;
    }

    let mut result = SshDialogResult::None;
    let is_editing = dialog.editing.is_some();
    let title = if is_editing {
        "Edit SSH Source"
    } else {
        "Add SSH Source"
    };

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("ssh_dialog_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Name:");
                    if is_editing {
                        ui.label(&dialog.name);
                    } else {
                        ui.text_edit_singleline(&mut dialog.name);
                    }
                    ui.end_row();

                    ui.label("Host:");
                    ui.text_edit_singleline(&mut dialog.host);
                    ui.end_row();

                    ui.label("Port:");
                    ui.text_edit_singleline(&mut dialog.port);
                    ui.end_row();

                    ui.label("User:");
                    ui.text_edit_singleline(&mut dialog.user);
                    ui.end_row();

                    ui.label("Remote Path:");
                    ui.text_edit_singleline(&mut dialog.remote_path);
                    ui.end_row();

                    ui.label("Key Path (optional):");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut dialog.key_path);
                        if ui.button("Browse").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_directory(dirs::home_dir().unwrap_or_default().join(".ssh"))
                                .pick_file()
                            {
                                dialog.key_path = path.display().to_string();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Password (optional):");
                    ui.add(egui::TextEdit::singleline(&mut dialog.password).password(true));
                    ui.end_row();

                    ui.label("Auto-open on startup:");
                    ui.checkbox(&mut dialog.auto_open, "");
                    ui.end_row();
                });

            ui.label(
                egui::RichText::new(
                    "Note: Will try SSH keys first (id_ed25519, id_rsa), then password",
                )
                .small()
                .color(egui::Color32::from_rgb(120, 120, 120)),
            );
            ui.label(
                egui::RichText::new("Password is stored encrypted locally")
                    .small()
                    .color(egui::Color32::from_rgb(100, 150, 100)),
            );

            if let Some(ref err) = dialog.error {
                ui.colored_label(egui::Color32::RED, err);
            }

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    dialog.open = false;
                }

                if ui.button("Save").clicked() {
                    if let Some(err) = validate_ssh_dialog(dialog) {
                        dialog.error = Some(err);
                    } else {
                        let saved = dialog.to_saved();
                        let password = if dialog.password.is_empty() {
                            None
                        } else {
                            Some(dialog.password.clone())
                        };
                        result = SshDialogResult::Save(saved, password);
                        dialog.open = false;
                    }
                }

                let connect_label = if is_editing {
                    "Save & Reconnect"
                } else {
                    "Save & Connect"
                };
                if ui.button(connect_label).clicked() {
                    if let Some(err) = validate_ssh_dialog(dialog) {
                        dialog.error = Some(err);
                    } else {
                        let saved = dialog.to_saved();
                        let password = if dialog.password.is_empty() {
                            None
                        } else {
                            Some(dialog.password.clone())
                        };
                        result = SshDialogResult::Connect(saved, password);
                        dialog.open = false;
                    }
                }
            });

            ui.add_space(5.0);
            ui.label(
                egui::RichText::new(format!("Saved to: {}", ssh_sources_path.display()))
                    .small()
                    .color(egui::Color32::from_rgb(100, 100, 100)),
            );
        });

    result
}

fn validate_ssh_dialog(dialog: &SshDialogState) -> Option<String> {
    if dialog.name.is_empty() {
        return Some("Name is required".to_string());
    }
    if dialog.host.is_empty() {
        return Some("Host is required".to_string());
    }
    if dialog.user.is_empty() {
        return Some("User is required".to_string());
    }
    if dialog.remote_path.is_empty() {
        return Some("Remote path is required".to_string());
    }
    None
}
