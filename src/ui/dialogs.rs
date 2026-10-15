use super::Workbench;
use super::theme::MUTED;
use crate::app::{command::Command, view_state::LanguageServerStatus};
use eframe::egui::{self, RichText, Vec2};
impl Workbench<'_> {
    pub(crate) fn language_server_details_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.language_server_details_open {
            return;
        }
        let Some(LanguageServerStatus::Error(message)) = self.view.language_server_status.as_ref()
        else {
            self.view.language_server_details_open = false;
            return;
        };
        let message = message.clone();
        egui::Window::new("Language Server Error")
            .open(&mut self.view.language_server_details_open)
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .default_height(320.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("The language server stopped. The diagnostic below is selectable and can be copied for troubleshooting.")
                        .color(MUTED),
                );
                ui.add_space(6.0);
                egui::ScrollArea::both()
                    .id_salt("language_server_error_details")
                    .auto_shrink([false, false])
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            egui::Label::new(RichText::new(&message).monospace())
                                .selectable(true),
                        );
                    });
                ui.add_space(6.0);
                if ui.button("Copy Error").clicked() {
                    ui.ctx().copy_text(message.clone());
                    self.view.status = "Language server error copied".into();
                }
            });
    }

    pub(crate) fn safety_details_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.safety_details_open {
            return;
        }
        egui::Window::new("Recovery and save warnings")
            .open(&mut self.view.safety_details_open)
            .resizable(true)
            .show(ctx, |ui| {
                for notice in &self.view.safety_notices {
                    ui.label(notice);
                }
            });
    }
    pub(crate) fn save_conflict_dialog(&mut self, ctx: &egui::Context) {
        let Some(conflict) = &self.view.save_conflict else {
            return;
        };
        let document = conflict.document;
        let show_comparison = conflict.show_comparison;
        let disk_text = if show_comparison {
            conflict.disk_text.as_deref().map(preview_text)
        } else {
            None
        };
        let editor_text = if show_comparison {
            self.editor
                .files
                .iter()
                .find(|file| file.id == document)
                .map(|file| preview_text(&file.text))
                .unwrap_or_default()
        } else {
            String::new()
        };
        let name = conflict
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| conflict.path.display().to_string());
        egui::Window::new("File changed on disk")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{name} changed outside Lumen since it was opened or saved."
                ));
                ui.label(RichText::new("Your edits have not been overwritten.").color(MUTED));
                ui.label(RichText::new("Reload discards this tab's unsaved edits; Overwrite replaces the external version.").color(MUTED));
                if ui.button(if show_comparison { "Hide Comparison" } else { "Compare Versions" }).clicked()
                    && let Some(conflict) = &mut self.view.save_conflict {
                    conflict.show_comparison = !show_comparison;
                }
                if show_comparison {
                    ui.horizontal_top(|ui| {
                        ui.set_min_width(620.0);
                        ui.vertical(|ui| {
                            ui.label("On Disk");
                            egui::ScrollArea::both().id_salt("conflict_disk").max_height(260.0).max_width(300.0).show(ui, |ui| {
                                ui.set_min_width(290.0);
                                ui.label(RichText::new(disk_text.as_deref().unwrap_or("(File missing from disk)")).monospace());
                            });
                        });
                        ui.vertical(|ui| {
                            ui.label("In Editor (unsaved)");
                            egui::ScrollArea::both().id_salt("conflict_editor").max_height(260.0).max_width(300.0).show(ui, |ui| {
                                ui.set_min_width(290.0);
                                ui.label(RichText::new(&editor_text).monospace());
                            });
                        });
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button("Keep Editing").clicked() {
                        self.command(Command::DismissConflict);
                    }
                    if ui
                        .button("Overwrite Disk File")
                        .on_hover_text(
                            "Replace the external version with your current editor contents",
                        )
                        .clicked()
                    {
                        self.command(Command::OverwriteConflict(document));
                    }
                    if ui.button("Reload from Disk").on_hover_text("Discard this tab's unsaved edits and open the external version").clicked() {
                        self.command(Command::ReloadConflict(document));
                    }
                    if ui.button("Save As…").on_hover_text("Keep both versions by saving editor contents to another path").clicked() {
                        self.command(Command::SaveConflictAs(document));
                    }
                });
            });
    }
    pub(crate) fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.view.pending_quit {
            return;
        }
        egui::Window::new("Save changes before quitting?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("There are unsaved changes in your open files.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save All").clicked() {
                        self.command(Command::SaveAllAndQuit);
                    }
                    if ui.button("Don't Save").clicked() {
                        self.command(Command::DiscardAndQuit);
                    }
                    if ui.button("Cancel").clicked() {
                        self.view.pending_quit = false;
                    }
                });
            });
    }

    pub(crate) fn unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        let Some(index) = self.view.pending_close else {
            return;
        };
        let name = self
            .editor
            .files
            .get(index)
            .map(|file| file.name.clone())
            .unwrap_or_default();
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("Save changes to {name}?"));
                ui.label(RichText::new("Your edits will be lost if you don't save.").color(MUTED));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.command(Command::SaveAndClose(index));
                    }
                    if ui.button("Don't Save").clicked() {
                        self.view.pending_close = None;
                        self.command(Command::CloseFile(index));
                    }
                    if ui.button("Cancel").clicked() {
                        self.view.pending_close = None;
                    }
                });
            });
    }
}

fn preview_text(text: &str) -> String {
    const LIMIT: usize = 20_000;
    let mut characters = text.chars();
    let mut preview: String = characters.by_ref().take(LIMIT).collect();
    if characters.next().is_some() {
        preview.push_str("\n… preview truncated; original text is unchanged");
    }
    preview
}
