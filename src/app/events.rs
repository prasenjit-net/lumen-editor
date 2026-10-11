use super::LumenApp;
use super::command::Command;
use eframe::egui;
impl LumenApp {
    pub(crate) fn handle_events(&mut self, ctx: &egui::Context) {
        super::editor_action::validate_paste(&mut self.view, self.editor.active(), ctx);
        // Editor metadata follows the delivered keyboard segment, not the whole
        // raw batch. Other text fields keep ordinary clipboard handling.
        let editor_focused = self.editor.active().is_some_and(|file| {
            ctx.memory(|memory| memory.has_focus(super::editor_action::text_widget_id(file.id)))
        });
        if !editor_focused {
            self.view.text_clipboard.observe_input(ctx);
            super::editor_input::cancel_input(&mut self.view);
        }
        let find = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F);
        let replace = egui::KeyboardShortcut::new(
            if cfg!(target_os = "macos") {
                egui::Modifiers::COMMAND | egui::Modifiers::ALT
            } else {
                egui::Modifiers::COMMAND
            },
            if cfg!(target_os = "macos") {
                egui::Key::F
            } else {
                egui::Key::H
            },
        );
        if ctx.input_mut(|input| input.consume_shortcut(&replace)) {
            self.execute(Command::Replace, ctx);
        } else if ctx.input_mut(|input| input.consume_shortcut(&find)) {
            self.execute(Command::Find, ctx);
        }
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
                let open = self.editor.active_file.is_some();
                let text_input =
                    ctx.wants_keyboard_input() || (open && !self.view.explorer_focused);
                let explorer = self.view.explorer_focused
                    && self.view.explorer_selection.is_some()
                    && self.view.explorer_edit.is_none()
                    && self.view.delete_prompt.is_none()
                    && !self.view.command_palette
                    && !ctx.wants_keyboard_input();
                menu.set_context(open, text_input, explorer);
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
                self.execute(Command::NewUntitled, ctx);
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
        self.route_platform_paste(ctx);
        self.start_clipboard_read(ctx);
        self.poll_clipboard(ctx);
        super::editor_action::finish_paste_validation(&mut self.view, self.editor.active(), ctx);
    }
}
