use super::LumenApp;
use super::command::Command;
use eframe::egui;
impl LumenApp {
    pub(crate) fn handle_events(&mut self, ctx: &egui::Context) {
        let editor_has_focus = self.editor.active().is_some_and(|document| {
            ctx.memory(|memory| memory.has_focus(super::editor_action::text_widget_id(document.id)))
        });
        if editor_has_focus
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F12))
        {
            self.execute(Command::GoToDefinition, ctx);
        }
        let symbols = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::O,
        );
        if editor_has_focus && ctx.input_mut(|input| input.consume_shortcut(&symbols)) {
            self.execute(Command::ShowDocumentSymbols, ctx);
        }
        for (modifiers, command) in [
            (egui::Modifiers::CTRL, Command::TriggerCompletion),
            (
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                Command::ShowSignatureHelp,
            ),
        ] {
            let shortcut = egui::KeyboardShortcut::new(modifiers, egui::Key::Space);
            if editor_has_focus && ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
                self.execute(command, ctx);
            }
        }
        for (modifiers, key, command) in [
            (
                egui::Modifiers::SHIFT,
                egui::Key::F12,
                Command::FindReferences,
            ),
            (egui::Modifiers::NONE, egui::Key::F2, Command::RenameSymbol),
            (
                egui::Modifiers::SHIFT | egui::Modifiers::ALT,
                egui::Key::F,
                Command::FormatDocument,
            ),
        ] {
            let shortcut = egui::KeyboardShortcut::new(modifiers, key);
            if editor_has_focus && ctx.input_mut(|input| input.consume_shortcut(&shortcut)) {
                self.execute(command, ctx);
            }
        }
        let terminal_focused = self.view.terminal.open
            && ctx.memory(|memory| memory.has_focus(egui::Id::new("terminal_canvas")));
        // VS Code uses Ctrl+` on every desktop platform. Keep it available
        // while the terminal owns focus so the same shortcut can also close it.
        let terminal = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Backtick);
        if ctx.input_mut(|input| input.consume_shortcut(&terminal)) {
            self.execute(Command::ToggleTerminal, ctx);
        }
        let output = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::U,
        );
        if !terminal_focused && ctx.input_mut(|input| input.consume_shortcut(&output)) {
            self.execute(Command::ToggleOutput, ctx);
        }
        let settings = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Comma);
        if !terminal_focused && ctx.input_mut(|input| input.consume_shortcut(&settings)) {
            self.execute(Command::Settings, ctx);
        }
        super::editor_action::validate_paste(&mut self.view, self.editor.active(), ctx);
        // Editor metadata follows the delivered keyboard segment, not the whole
        // raw batch. Other text fields keep ordinary clipboard handling.
        let editor_focused = self.editor.active().is_some_and(|file| {
            ctx.memory(|memory| memory.has_focus(super::editor_action::text_widget_id(file.id)))
        });
        if !editor_focused && !terminal_focused {
            #[cfg(feature = "shared-editor")]
            if ctx.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy | egui::Event::Cut))
            }) && let Some(surface) = &mut self.view.shared_surface
            {
                surface.clear_clipboard();
            }
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
        if !terminal_focused
            && !self.view.settings.open
            && ctx.input_mut(|input| input.consume_shortcut(&replace))
        {
            self.execute(Command::Replace, ctx);
        } else if !terminal_focused
            && !self.view.settings.open
            && ctx.input_mut(|input| input.consume_shortcut(&find))
        {
            self.execute(Command::Find, ctx);
        }
        if ctx.input(|input| input.viewport().close_requested())
            && !self.view.allow_quit
            && (self.editor.has_unsaved_changes() || self.view.settings.has_drafts())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_quit(ctx);
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
                    && self.view.navigation.mode.is_none()
                    && !ctx.wants_keyboard_input();
                menu.set_context(
                    open,
                    text_input,
                    explorer,
                    self.view.settings.open,
                    self.view.settings.selected().pending() > 0,
                    self.editor.workspace.is_some(),
                );
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if !terminal_focused {
                let palette = egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::P,
                );
                if ctx.input_mut(|input| input.consume_shortcut(&palette)) {
                    self.execute(Command::Palette, ctx);
                }
                let quick_open =
                    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::P);
                if ctx.input_mut(|input| input.consume_shortcut(&quick_open)) {
                    self.execute(Command::QuickOpen, ctx);
                }
                let go_to_line =
                    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::G);
                if ctx.input_mut(|input| input.consume_shortcut(&go_to_line)) {
                    self.execute(Command::GoToLine, ctx);
                }
                let workspace_search = egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::F,
                );
                if ctx.input_mut(|input| input.consume_shortcut(&workspace_search)) {
                    self.execute(Command::ShowWorkspaceSearch, ctx);
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
                    self.execute(Command::CloseEditor, ctx);
                }
            }
        }
        if !terminal_focused {
            self.route_platform_paste(ctx);
            self.start_clipboard_read(ctx);
            self.poll_clipboard(ctx);
        }
        super::editor_action::finish_paste_validation(&mut self.view, self.editor.active(), ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_backtick_toggles_the_terminal_even_when_its_canvas_is_focused() {
        let ctx = egui::Context::default();
        let mut app = LumenApp::default();
        app.view.terminal.open = true;
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("terminal_canvas")));
        let _ = ctx.run(
            egui::RawInput {
                modifiers: egui::Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                events: vec![egui::Event::Key {
                    key: egui::Key::Backtick,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl: true,
                        ..Default::default()
                    },
                }],
                ..Default::default()
            },
            |ctx| app.handle_events(ctx),
        );
        assert!(!app.view.terminal.open);
    }
}
