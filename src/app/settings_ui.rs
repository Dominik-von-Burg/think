use eframe::egui::{self, Color32, RichText};

use super::{NotebookApp, SettingsDialog, SettingsPage};
use crate::mcp;

const MUTED: Color32 = Color32::from_rgb(164, 166, 174);
const ERROR: Color32 = Color32::from_rgb(236, 139, 132);
const SUCCESS: Color32 = Color32::from_rgb(151, 198, 167);
const ACCENT: Color32 = Color32::from_rgb(47, 77, 109);
const BORDER: Color32 = Color32::from_rgb(63, 65, 74);

impl NotebookApp {
    pub(super) fn draw_settings(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.settings_dialog.take() else {
            return;
        };
        let mut close = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Area::new(egui::Id::new("settings_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(ctx.screen_rect().min)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(155));
                close |= ui.allocate_rect(screen, egui::Sense::click()).clicked();
            });

        let width = (ctx.screen_rect().width() - 54.0).clamp(280.0, 450.0);
        egui::Window::new("Settings")
            .title_bar(false)
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(width)
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(28, 29, 33))
                    .stroke(egui::Stroke::new(1.0_f32, BORDER))
                    .rounding(egui::Rounding::same(14.0))
                    .inner_margin(egui::Margin::same(22.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
                ui.visuals_mut().selection.bg_fill = ACCENT;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings").size(23.0).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= ui
                            .add(
                                egui::Button::new(RichText::new("×").size(22.0).color(MUTED))
                                    .frame(false),
                            )
                            .on_hover_text("Close settings")
                            .clicked();
                    });
                });
                ui.horizontal(|ui| {
                    for (page, title) in [
                        (SettingsPage::Sync, "GitHub sync"),
                        (SettingsPage::Mcp, "MCP access"),
                    ] {
                        if ui
                            .add_sized(
                                [110.0, 30.0],
                                egui::Button::new(title).selected(dialog.page == page),
                            )
                            .clicked()
                        {
                            dialog.page = page;
                        }
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("settings_content")
                    .max_height((ctx.screen_rect().height() - 136.0).max(125.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| match dialog.page {
                        SettingsPage::Sync => self.show_sync_settings(ui, &mut dialog),
                        SettingsPage::Mcp => self.show_mcp_settings(ui),
                    });
            });

        if !close {
            self.settings_dialog = Some(dialog);
        }
    }

    fn show_sync_settings(&mut self, ui: &mut egui::Ui, dialog: &mut SettingsDialog) {
        let can_sync = self.sync_configured()
            && !self.sync_in_progress
            && (self.paths.notebook.is_file() || self.dirty_since.is_some());
        card(ui, |ui| {
            let (title, color, description) = if self.sync_in_progress {
                ("Syncing", MUTED, "Sending your notebook to GitHub.")
            } else if self.sync_error.is_some() {
                ("Sync failed", ERROR, "")
            } else if !self.sync_configured() {
                (
                    "Not configured",
                    MUTED,
                    "Add a repository and token to enable GitHub sync.",
                )
            } else if self.last_change.is_some() {
                (
                    "Waiting to sync",
                    MUTED,
                    "Think will sync after you pause typing.",
                )
            } else if self.synced {
                ("Synced", SUCCESS, "Your latest sync completed.")
            } else {
                (
                    "Ready to sync",
                    SUCCESS,
                    "Think will sync the next time you edit the notebook.",
                )
            };
            ui.label(RichText::new(title).color(color).strong());
            if !description.is_empty() {
                ui.label(RichText::new(description).color(MUTED).size(13.0));
            }
            if let Some(error) = &self.sync_error {
                ui.add(
                    egui::Label::new(RichText::new(error).size(13.0))
                        .wrap()
                        .selectable(true),
                );
            }
            if self.sync_configured() || self.sync_error.is_some() {
                ui.horizontal(|ui| {
                    let action = if self.sync_error.is_some() {
                        "Retry sync"
                    } else {
                        "Sync now"
                    };
                    if ui
                        .add_enabled(can_sync, egui::Button::new(action))
                        .clicked()
                    {
                        self.sync_now();
                    }
                    if let Some(error) = &self.sync_error {
                        if ui.button("Copy details").clicked() {
                            ui.ctx().copy_text(error.clone());
                        }
                    }
                });
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Repository URL").strong());
        let url = ui
            .add(field(&mut dialog.repo_url).hint_text("https://github.com/you/private-notebook"));
        ui.add_space(4.0);
        ui.label(RichText::new("Personal access token").strong());
        let token = ui.add(
            field(&mut dialog.token)
                .password(true)
                .hint_text("Fine-grained token with Contents read/write"),
        );
        ui.label(
            RichText::new("Stored in your system keyring.")
                .color(MUTED)
                .size(12.0),
        );
        if url.changed() || token.changed() {
            dialog.saved = false;
            dialog.error = None;
        }
        if let Some(error) = &dialog.error {
            ui.colored_label(ERROR, error);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_sized([112.0, 31.0], primary("Save settings"))
                .clicked()
            {
                match self.save_settings(dialog) {
                    Ok(()) => {
                        dialog.saved = true;
                        dialog.error = None;
                    }
                    Err(error) => {
                        dialog.saved = false;
                        dialog.error = Some(error);
                    }
                }
            }
            if dialog.saved {
                ui.colored_label(SUCCESS, "Saved");
            }
        });
    }

    fn show_mcp_settings(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            if let Some(server) = &self.mcp {
                ui.label(RichText::new("Access approved").color(SUCCESS).strong());
                ui.label(
                    RichText::new(
                        "The approved client can read and edit this notebook while Think is open.",
                    )
                    .color(MUTED)
                    .size(13.0),
                );
                ui.add(
                    egui::Label::new(RichText::new(mcp::URL).monospace().size(12.0))
                        .selectable(true),
                );
                ui.horizontal(|ui| {
                    if ui.button("Copy URL").clicked() {
                        ui.ctx().copy_text(mcp::URL.to_owned());
                    }
                    if ui.button("Copy access token").clicked() {
                        ui.ctx().copy_text(server.token.clone());
                    }
                });
            } else {
                ui.label(RichText::new("Access off").color(MUTED).strong());
                ui.label(
                    RichText::new("Approve a local MCP client to read and edit this notebook.")
                        .color(MUTED)
                        .size(13.0),
                );
            }
        });
        ui.add_space(8.0);
        if self.mcp.is_some() {
            if ui
                .button(RichText::new("Revoke access").color(ERROR))
                .clicked()
            {
                self.mcp_error = self.revoke_mcp().err().map(|error| error.to_string());
            }
        } else if ui
            .add_sized([146.0, 31.0], primary("Approve MCP client"))
            .clicked()
        {
            self.mcp_error = self
                .approve_mcp(ui.ctx())
                .err()
                .map(|error| error.to_string());
        }
        if let Some(error) = &self.mcp_error {
            ui.colored_label(ERROR, error);
        }
    }
}

fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(Color32::from_rgb(35, 37, 43))
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}

fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).strong())
        .fill(ACCENT)
        .rounding(egui::Rounding::same(6.0))
}

fn field(text: &mut String) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text)
        .desired_width(f32::INFINITY)
        .margin(egui::Margin::symmetric(9.0, 7.0))
}
