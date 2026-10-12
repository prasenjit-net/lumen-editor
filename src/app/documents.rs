use super::LumenApp;
use super::command::Command;
use super::view_state::{Activity, ExplorerSelection, SaveConflict};
use crate::model::{Document, display_name};
use eframe::egui;
use std::path::PathBuf;
impl LumenApp {
    pub(crate) fn request_quit(&mut self, ctx: &egui::Context) {
        if self.view.settings.has_drafts() {
            self.view.settings.pending_quit = true;
        } else if self.editor.has_unsaved_changes() {
            self.view.pending_quit = true;
        } else {
            if self.clear_recovery_for_exit() {
                self.view.allow_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub(crate) fn new_file(&mut self) {
        Command::NewFile.apply_local(&mut self.editor, &mut self.view);
    }

    pub(crate) fn choose_file(&mut self) {
        if let Some(path) = self.dialogs.open_file() {
            self.open_file(path);
        }
    }

    pub(crate) fn choose_folder(&mut self) {
        if let Some(folder) = self.dialogs.open_folder() {
            self.editor.workspace = Some(folder.clone());
            self.view.expanded_dirs.clear();
            self.view.expanded_dirs.insert(folder.clone());
            self.view.explorer_selection = Some(ExplorerSelection::Directory(folder.clone()));
            self.view.explorer_edit = None;
            self.view.delete_prompt = None;
            self.view.activity = Activity::Explorer;
            self.view.sidebar_visible = true;
            self.view.status = format!("Opened folder {}", display_name(&folder));
        }
    }

    pub(crate) fn open_file(&mut self, path: PathBuf) {
        self.view.settings.open = false;
        if let Some(index) = self
            .editor
            .files
            .iter()
            .position(|file| file.path.as_deref() == Some(path.as_path()))
        {
            self.editor.active_file = Some(index);
            return;
        }
        match self.file_system.read_text(&path) {
            Ok(text) => {
                self.editor.files.push(Document::from_path(path, text));
                self.editor.active_file = Some(self.editor.files.len() - 1);
                self.view.status = "File opened".into();
            }
            Err(error) => self.view.status = format!("Could not open file: {error}"),
        }
    }

    pub(crate) fn save_file_at(&mut self, index: usize, path: PathBuf) {
        let Some(file) = self.editor.files.get_mut(index) else {
            return;
        };
        let same_path = file.path.as_deref() == Some(path.as_path());
        let result = if same_path {
            self.file_system
                .write_text_if_unchanged(&path, file.saved_text(), &file.text)
        } else {
            self.file_system.write_text(&path, &file.text)
        };
        match result {
            Ok(()) => {
                file.mark_saved(path.clone());
                self.view.save_conflict = None;
                self.view.status = "Saved".into();
                self.settings_file_saved(&path);
            }
            Err(error)
                if same_path
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound
                    ) =>
            {
                file.disk_changed = true;
                self.view.save_conflict = Some(SaveConflict {
                    document: file.id,
                    revision: file.revision,
                    path,
                });
                self.view.status =
                    "File changed on disk. Save was blocked; your edits are intact.".into();
            }
            Err(error) => self.view.status = format!("Could not save file: {error}"),
        }
    }

    pub(crate) fn overwrite_conflict(&mut self, document: crate::model::DocumentId) {
        let Some(conflict) = self.view.save_conflict.take() else {
            return;
        };
        if conflict.document != document {
            return;
        }
        let Some(file) = self
            .editor
            .files
            .iter_mut()
            .find(|file| file.id == document)
        else {
            return;
        };
        if file.revision != conflict.revision || file.path.as_ref() != Some(&conflict.path) {
            self.view.status =
                "Document changed since the warning. Save again to review the conflict.".into();
            return;
        }
        match self.file_system.write_text(&conflict.path, &file.text) {
            Ok(()) => {
                file.mark_saved(conflict.path.clone());
                self.view.status = "External version overwritten by explicit request".into();
                self.settings_file_saved(&conflict.path);
            }
            Err(error) => {
                self.view.save_conflict = Some(conflict);
                self.view.status = format!("Could not overwrite file: {error}");
            }
        }
    }

    pub(crate) fn reload_conflict(&mut self, document: crate::model::DocumentId) {
        let Some(conflict) = self.view.save_conflict.take() else {
            return;
        };
        if conflict.document != document {
            return;
        }
        let Some(file) = self
            .editor
            .files
            .iter_mut()
            .find(|file| file.id == document)
        else {
            return;
        };
        if file.revision != conflict.revision || file.path.as_ref() != Some(&conflict.path) {
            self.view.status =
                "Document changed since the warning. Save again to review the conflict.".into();
            return;
        }
        match self.file_system.read_text(&conflict.path) {
            Ok(text) => {
                file.reload_from_disk(text);
                self.view.pending_editor_action = None;
                self.view.editor_paste_target = None;
                super::editor_input::cancel_input(&mut self.view);
                self.view.status =
                    "Reloaded external file; unsaved edits in this tab were discarded".into();
            }
            Err(error) => {
                self.view.save_conflict = Some(conflict);
                self.view.status = format!("Could not reload file: {error}");
            }
        }
    }

    pub(crate) fn save_active(&mut self) {
        let Some(index) = self.editor.active_file else {
            return;
        };
        if let Some(path) = self.editor.files[index].path.clone() {
            self.save_file_at(index, path);
        } else {
            self.save_as_active();
        }
    }

    pub(crate) fn save_as_active(&mut self) {
        let Some(index) = self.editor.active_file else {
            return;
        };
        if let Some(path) = self.dialogs.save_as(&self.editor.files[index].name) {
            self.save_file_at(index, path);
        }
    }

    pub(crate) fn request_close(&mut self, index: usize) {
        Command::RequestClose(index).apply_local(&mut self.editor, &mut self.view);
    }

    pub(crate) fn close_file(&mut self, index: usize) {
        Command::CloseFile(index).apply_local(&mut self.editor, &mut self.view);
    }
}
