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
                self.command(Command::NewUntitled);
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
            if ui
                .add_enabled(
                    self.editor.workspace.is_some(),
                    egui::Button::new(format!("{}  Close Folder", icon::X)),
                )
                .clicked()
            {
                self.command(Command::CloseFolder);
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
            ui.separator();
            if ui
                .add_enabled(
                    self.editor.active_file.is_some() || self.view.settings.open,
                    egui::Button::new(format!("{}  Close Document     ⌘W", icon::X)),
                )
                .clicked()
            {
                self.command(Command::CloseEditor);
                ui.close();
            }
        });
    }

    pub(crate) fn palette(&mut self, ctx: &egui::Context) {
        if !self.view.command_palette {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.view.command_palette = false;
            self.view.command_palette_focus = false;
            return;
        }
        let mut open = true;
        egui::Window::new("Command Palette")
            .open(&mut open)
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
                let query = ui.add(
                    TextEdit::singleline(&mut self.view.search)
                        .id(egui::Id::new("command_palette_query"))
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY),
                );
                if self.view.command_palette_focus {
                    query.request_focus();
                    self.view.command_palette_focus = false;
                }
                ui.separator();
                for (label, action) in [
                    (
                        format!("{}  Preferences: Open Settings", icon::GEAR),
                        Command::Settings,
                    ),
                    (
                        format!("{}  File: New Text File", icon::FILE_PLUS),
                        Command::NewUntitled,
                    ),
                    (
                        format!("{}  Explorer: New File", icon::FILE_PLUS),
                        Command::NewFile,
                    ),
                    (
                        format!("{}  Explorer: New Folder", icon::NEW_FOLDER),
                        Command::NewFolder,
                    ),
                    (
                        format!("{}  File: Open File", icon::FILE_ARROW_UP),
                        Command::OpenFile,
                    ),
                    (format!("{}  Go to File", icon::FILE), Command::QuickOpen),
                    ("Go to Line/Column".into(), Command::GoToLine),
                    ("Go to Definition".into(), Command::GoToDefinition),
                    ("Show Hover".into(), Command::ShowHover),
                    ("Trigger Suggest".into(), Command::TriggerCompletion),
                    ("Find All References".into(), Command::FindReferences),
                    ("Rename Symbol".into(), Command::RenameSymbol),
                    ("Format Document".into(), Command::FormatDocument),
                    ("Show Signature Help".into(), Command::ShowSignatureHelp),
                    (
                        "Go to Symbol in Editor".into(),
                        Command::ShowDocumentSymbols,
                    ),
                    (
                        format!("{}  File: Open Folder", icon::FOLDER_OPEN),
                        Command::OpenFolder,
                    ),
                    (
                        format!("{}  File: Close Folder", icon::X),
                        Command::CloseFolder,
                    ),
                    (format!("{}  File: Save", icon::FLOPPY_DISK), Command::Save),
                    (
                        format!("{}  File: Close Document", icon::X),
                        Command::CloseEditor,
                    ),
                    (
                        format!("{}  Editor: Find", icon::MAGNIFYING_GLASS),
                        Command::Find,
                    ),
                    ("Editor: Replace".into(), Command::Replace),
                    (
                        format!("{}  Search: Find in Files", icon::MAGNIFYING_GLASS),
                        Command::ShowWorkspaceSearch,
                    ),
                    (
                        format!("{}  Terminal: Toggle Terminal", icon::TERMINAL),
                        Command::ToggleTerminal,
                    ),
                    (
                        format!("{}  View: Output", icon::OUTPUT),
                        Command::ToggleOutput,
                    ),
                    (
                        format!("{}  View: Problems", icon::WARNING),
                        Command::ToggleProblems,
                    ),
                ] {
                    if ui.selectable_label(false, label).clicked() {
                        self.command(action);
                        self.view.command_palette = false;
                    }
                }
            });
        if !open {
            self.view.command_palette = false;
            self.view.command_palette_focus = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::LumenApp, ui::theme::configure_ui};

    fn frame(app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.palette(ctx)),
        );
    }

    #[test]
    fn escape_closes_palette_even_when_query_has_focus() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        app.execute(Command::Palette, &ctx);
        frame(&mut app, &ctx, vec![]);
        frame(&mut app, &ctx, vec![]);
        assert!(ctx.memory(|memory| memory.has_focus(egui::Id::new("command_palette_query"))));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!app.view.command_palette);
    }
}
