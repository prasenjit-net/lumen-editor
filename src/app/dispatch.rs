use super::{LumenApp, command::Command};
use eframe::egui;

impl LumenApp {
    pub(crate) fn execute(&mut self, command: Command, ctx: &egui::Context) {
        let explorer_clipboard = self.view.explorer_focused
            && self.view.explorer_edit.is_none()
            && self.view.delete_prompt.is_none()
            && !self.view.command_palette
            && !ctx.wants_keyboard_input();
        match command {
            Command::EditorAction(document, action) => {
                if self.editor.active().is_some_and(|file| file.id == document) {
                    self.view.pending_editor_action =
                        Some(super::editor_action::PendingEditorAction { document, action });
                    ctx.request_repaint();
                }
            }
            Command::NewFile => self.new_file(),
            Command::NewUntitled | Command::CollapseFolders | Command::RefreshExplorer => {
                command.apply_local(&mut self.editor, &mut self.view);
            }
            Command::NewFolder => self.new_folder(),
            Command::RenamePath(path) => self.rename_path(path),
            Command::DeletePath(path) => self.request_delete(path),
            Command::ConfirmDelete => self.confirm_delete(),
            Command::ExplorerCopy(path) => self.explorer_copy(path, false),
            Command::ExplorerCut(path) => self.explorer_copy(path, true),
            Command::ExplorerPaste => self.explorer_paste(),
            Command::CopyPath(path, relative) => {
                let path = if relative {
                    self.editor
                        .workspace
                        .as_ref()
                        .and_then(|root| path.strip_prefix(root).ok())
                        .unwrap_or(&path)
                        .to_path_buf()
                } else {
                    std::fs::canonicalize(&path).unwrap_or(path)
                };
                ctx.copy_text(path.display().to_string());
            }
            Command::RevealPath(path) => {
                if let Err(error) = crate::services::reveal_in_file_manager(&path) {
                    self.view.status = format!("Could not reveal file: {error}");
                }
            }
            Command::CommitExplorerEdit => self.commit_explorer_edit(),
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
            Command::Copy | Command::Cut if explorer_clipboard => {
                if let Some(selection) = &self.view.explorer_selection {
                    self.explorer_copy(selection.path().into(), command == Command::Cut);
                }
            }
            Command::Paste if explorer_clipboard => self.explorer_paste(),
            Command::Copy => super::editor_action::EditorAction::Copy.send(ctx),
            Command::Cut => super::editor_action::EditorAction::Cut.send(ctx),
            Command::Paste => super::editor_action::EditorAction::Paste.send(ctx),
            Command::Undo => super::editor_action::EditorAction::Undo.send(ctx),
            Command::Redo => super::editor_action::EditorAction::Redo.send(ctx),
            Command::SelectAll => super::editor_action::EditorAction::SelectAll.send(ctx),
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
