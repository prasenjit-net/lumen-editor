use super::{LumenApp, command::Command};
use eframe::egui;

impl LumenApp {
    pub(crate) fn execute(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::NewFile => self.new_file(),
            Command::CreateFile => self.create_file(),
            Command::OpenFile => self.choose_file(),
            Command::OpenFolder => self.choose_folder(),
            Command::OpenPath(path) => self.open_file(path),
            Command::Save => self.save_active(),
            Command::SaveAs => self.save_as_active(),
            Command::RequestClose(index) => self.request_close(index),
            Command::CloseFile(index) => self.close_file(index),
            Command::CloseEditor => {
                if let Some(index) = self.editor.active_file {
                    self.request_close(index);
                }
            }
            Command::Quit => self.request_quit(ctx),
            Command::Palette => self.view.command_palette = true,
            Command::ToggleSidebar => self.view.sidebar_visible = !self.view.sidebar_visible,
            Command::Welcome => self.view.status = "Lumen — New: ⌘N · Open: ⌘O · Save: ⌘S".into(),
            Command::Copy => ctx.input_mut(|input| input.events.push(egui::Event::Copy)),
            Command::Cut => ctx.input_mut(|input| input.events.push(egui::Event::Cut)),
            Command::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Command::Undo | Command::Redo | Command::SelectAll => ctx.input_mut(|input| {
                input.events.push(egui::Event::Key {
                    key: if command == Command::SelectAll {
                        egui::Key::A
                    } else {
                        egui::Key::Z
                    },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        command: true,
                        mac_cmd: cfg!(target_os = "macos"),
                        ctrl: cfg!(target_os = "windows"),
                        shift: command == Command::Redo,
                        ..Default::default()
                    },
                });
            }),
            Command::SaveAndClose(index) => {
                // Preserve the current dialog's active-buffer save semantics.
                self.save_active();
                self.view.pending_close = None;
                if self.editor.files.get(index).is_some_and(|file| !file.dirty) {
                    self.close_file(index);
                }
            }
            Command::SaveAllAndQuit => {
                let active = self.editor.active_file;
                for index in 0..self.editor.files.len() {
                    if self.editor.files[index].dirty {
                        self.editor.active_file = Some(index);
                        self.save_active();
                        if self.editor.files[index].dirty {
                            break;
                        }
                    }
                }
                self.editor.active_file = active;
                if !self.editor.has_unsaved_changes() {
                    self.finish_quit(ctx);
                }
            }
            Command::DiscardAndQuit => self.finish_quit(ctx),
        }
    }

    fn finish_quit(&mut self, ctx: &egui::Context) {
        self.view.allow_quit = true;
        self.view.pending_quit = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
