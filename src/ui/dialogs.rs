use super::Workbench;
use super::theme::MUTED;
use crate::app::command::Command;
use eframe::egui::{self, RichText, Vec2};
impl Workbench<'_> {
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
