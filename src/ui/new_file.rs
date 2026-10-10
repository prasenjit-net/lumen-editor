use super::{Workbench, theme::MUTED};
use crate::app::command::Command;
use eframe::egui::{self, RichText, TextEdit};

impl Workbench<'_> {
    pub(crate) fn new_file_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &mut self.view.new_file_prompt else {
            return;
        };
        let mut create = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("new_file_prompt")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.heading("New File");
            ui.label(RichText::new("Create in").small().color(MUTED));
            ui.add(egui::Label::new(prompt.directory.display().to_string()).truncate())
                .on_hover_text(prompt.directory.display().to_string());
            ui.add_space(6.0);
            let response = ui.add(
                TextEdit::singleline(&mut prompt.name)
                    .id(egui::Id::new("new_file_name"))
                    .hint_text("File name, e.g. main.rs")
                    .desired_width(f32::INFINITY),
            );
            if prompt.focus_name {
                response.request_focus();
                prompt.focus_name = false;
            }
            if response.changed() {
                prompt.error = None;
            }
            if let Some(error) = &prompt.error {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                create = ui.button("Create").clicked();
                cancel = ui.button("Cancel").clicked();
            });
            create |= (response.has_focus() || response.lost_focus())
                && ui.input(|input| input.key_pressed(egui::Key::Enter));
        });
        if cancel || modal.should_close() {
            self.view.new_file_prompt = None;
        } else if create {
            self.command(Command::CreateFile);
        }
    }
}
