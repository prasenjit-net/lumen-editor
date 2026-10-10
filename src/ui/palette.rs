use super::Workbench;
use super::theme::{BORDER, PANEL};
use crate::app::command::Command;
use crate::icon;
use eframe::egui::{self, CornerRadius, Frame, Stroke, TextEdit, Vec2};
impl Workbench<'_> {
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui
                .button(format!("{}  New Text File     ⌘N", icon::FILE_PLUS))
                .clicked()
            {
                self.command(Command::NewFile);
                ui.close();
            }
            if ui
                .button(format!("{}  Open File…          ⌘O", icon::FILE_ARROW_UP))
                .clicked()
            {
                self.command(Command::OpenFile);
                ui.close();
            }
            if ui
                .button(format!("{}  Open Folder…     ⌘⇧O", icon::FOLDER_OPEN))
                .clicked()
            {
                self.command(Command::OpenFolder);
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.editor.active_file.is_some(),
                    egui::Button::new(format!("{}  Save                 ⌘S", icon::FLOPPY_DISK)),
                )
                .clicked()
            {
                self.command(Command::Save);
                ui.close();
            }
            if ui
                .add_enabled(
                    self.editor.active_file.is_some(),
                    egui::Button::new(format!("{}  Save As…          ⌘⇧S", icon::FLOPPY_DISK)),
                )
                .clicked()
            {
                self.command(Command::SaveAs);
                ui.close();
            }
        });
    }

    pub(crate) fn palette(&mut self, ctx: &egui::Context) {
        if !self.view.command_palette {
            return;
        }
        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 72.0))
            .frame(
                Frame::window(&ctx.style())
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.view.search)
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY),
                );
                ui.separator();
                for (label, action) in [
                    (
                        format!("{}  File: New Text File", icon::FILE_PLUS),
                        Command::NewFile,
                    ),
                    (
                        format!("{}  File: Open File", icon::FILE_ARROW_UP),
                        Command::OpenFile,
                    ),
                    (
                        format!("{}  File: Open Folder", icon::FOLDER_OPEN),
                        Command::OpenFolder,
                    ),
                    (format!("{}  File: Save", icon::FLOPPY_DISK), Command::Save),
                ] {
                    if ui.selectable_label(false, label).clicked() {
                        self.command(action);
                        self.view.command_palette = false;
                    }
                }
            });
    }
}
