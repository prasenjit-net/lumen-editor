use super::LumenApp;
#[cfg(not(target_os = "macos"))]
use super::command::Command;
use eframe::egui;
impl LumenApp {
    pub(crate) fn handle_events(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && !self.view.allow_quit
            && self.editor.has_unsaved_changes()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.view.pending_quit = true;
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let actions = self
                .native_menu
                .as_ref()
                .map(|menu| menu.actions())
                .unwrap_or_default();
            for action in actions {
                self.execute(action, ctx);
            }
            if let Some(menu) = &self.native_menu {
                menu.set_document_open(self.editor.active_file.is_some());
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) {
                self.execute(Command::Palette, ctx);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::B)) {
                self.execute(Command::ToggleSidebar, ctx);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::N)) {
                self.execute(Command::NewFile, ctx);
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.execute(Command::OpenFolder, ctx);
                } else {
                    self.execute(Command::OpenFile, ctx);
                }
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.execute(Command::SaveAs, ctx);
                } else {
                    self.execute(Command::Save, ctx);
                }
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::W)) {
                if let Some(index) = self.editor.active_file {
                    self.execute(Command::RequestClose(index), ctx);
                }
            }
        }
    }
}
