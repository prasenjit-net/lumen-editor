use super::LumenApp;
use super::command::Command;
use super::view_state::{Activity, ExplorerSelection, SaveConflict};
use crate::model::{Document, display_name};
use eframe::egui;
use std::path::PathBuf;
impl LumenApp {
    pub(crate) fn request_quit(&mut self, ctx: &egui::Context) {
        if self.guard_quit_for_tasks() {
            return;
        }
        if let Some(id) = self.view.task_editors.first_dirty() {
            // Unsaved task drafts are never dropped silently: ask first, then the
            // user quits again once the prompt is answered.
            self.view.task_editors.activate(id);
            self.view.task_close_prompt = Some(id);
            self.view.leave_page_tabs_for_task(id);
            self.view.status = "Unsaved task changes: save or discard them, then quit again".into();
        } else if self.view.settings.has_drafts() {
            self.view.settings.pending_quit = true;
        } else if self.editor.has_unsaved_changes() {
            self.view.pending_quit = true;
        } else {
            if self.clear_recovery_for_exit() {
                self.close_window(ctx);
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
            self.open_folder_path(folder);
        }
    }

    /// Open `folder` as the workspace. Shared by the native dialog, the welcome
    /// list, the recent-folder picker and menus. A folder that is gone (or is a
    /// file) is reported and dropped from the history; nothing else changes.
    pub(crate) fn open_folder_path(&mut self, folder: PathBuf) {
        if !self.file_system.is_directory(&folder) {
            self.view.navigation.forget_folder(&folder);
            self.save_recent_files();
            self.view.status = format!("Folder not found: {}", folder.display());
            return;
        }
        if self.guard_workspace_change(Some(folder.clone())) {
            return;
        }
        super::source_control::close_diff_preview(&mut self.editor, &mut self.view);
        self.cancel_file_index();
        self.view.navigation.indexed_root = None;
        self.cancel_workspace_search();
        self.view.workspace_search.results = None;
        self.view.workspace_search.completed_request = None;
        self.editor.workspace = Some(folder.clone());
        // A comparison belongs to the repository it was opened from.
        self.source_control_job = None;
        self.view.source_control = Default::default();
        self.view.expanded_dirs.clear();
        self.view.expanded_dirs.insert(folder.clone());
        self.view.explorer_selection = Some(ExplorerSelection::Directory(folder.clone()));
        self.view.explorer_edit = None;
        self.view.delete_prompt = None;
        self.view.activity = Activity::Explorer;
        self.view.sidebar_visible = true;
        self.view.status = format!("Opened folder {}", display_name(&folder));
        self.view.navigation.remember_folder(&folder);
        self.save_recent_files();
    }

    /// Forget every recently opened file and folder, in memory and on disk.
    pub(crate) fn clear_recent_history(&mut self) {
        self.view.navigation.recent.clear();
        self.view.navigation.recent_folders.clear();
        self.view.navigation.matched_query = None;
        self.save_recent_files();
        self.view.status = "Cleared recently opened files and folders".into();
    }

    pub(crate) fn close_folder(&mut self) {
        if self.editor.workspace.is_none() {
            self.view.status = "No folder is open".into();
            return;
        }
        if self.guard_workspace_change(None) {
            return;
        }
        let Some(folder) = self.editor.workspace.take() else {
            return;
        };
        super::source_control::close_diff_preview(&mut self.editor, &mut self.view);
        self.cancel_file_index();
        self.view.navigation.indexed_root = None;
        self.view.navigation.paths.clear();
        self.view.navigation.mode = None;
        self.cancel_workspace_search();
        self.view.workspace_search = Default::default();
        self.source_control_job = None;
        self.view.source_control = Default::default();
        self.view.expanded_dirs.clear();
        self.view.explorer_selection = None;
        self.view.explorer_edit = None;
        self.view.delete_prompt = None;
        self.view.explorer_clipboard = None;
        self.view.explorer_focused = false;
        self.close_terminal();
        self.close_workspace_language_server();
        self.view.status = format!("Closed folder {}", display_name(&folder));
    }

    pub(crate) fn open_file(&mut self, path: PathBuf) {
        self.view.leave_page_tabs();
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
            if diff.diff_only_document.is_some_and(|id| {
                self.editor
                    .files
                    .iter()
                    .any(|file| file.id == id && file.path.as_deref() == Some(path.as_path()))
            }) {
                diff.diff_only_document = None;
            }
        }
        if let Some(index) = self
            .editor
            .files
            .iter()
            .position(|file| file.path.as_deref() == Some(path.as_path()))
        {
            self.editor.set_active_index(Some(index));
            self.remember_recent_file(&path);
            return;
        }
        match self.file_system.read_text(&path) {
            Ok(text) => {
                self.editor
                    .files
                    .push(Document::from_path(path.clone(), text));
                self.editor
                    .set_active_index(Some(self.editor.files.len() - 1));
                self.remember_recent_file(&path);
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
                let saved_document = file.id;
                self.remember_recent_file(&path);
                self.view.navigation.add_workspace_path(&path);
                self.view.save_conflict = None;
                self.view.status = "Saved".into();
                self.settings_file_saved(&path);
                self.notify_language_document_saved(saved_document);
            }
            Err(error)
                if same_path
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound
                    ) =>
            {
                file.disk_changed = true;
                let disk_text = match self.file_system.read_text(&path) {
                    Ok(text) => Some(text),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => {
                        self.view.status =
                            format!("Save blocked; changed file could not be read: {error}");
                        return;
                    }
                };
                self.view.save_conflict = Some(SaveConflict {
                    document: file.id,
                    revision: file.revision,
                    disk_text,
                    path,
                    show_comparison: false,
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
        let current = match self.file_system.read_text(&conflict.path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.view.status = format!("Could not recheck disk file: {error}");
                self.view.save_conflict = Some(conflict);
                return;
            }
        };
        if current != conflict.disk_text {
            self.view.status =
                "Disk changed again. Review the newest version before overwriting.".into();
            self.view.save_conflict = Some(SaveConflict {
                disk_text: current,
                show_comparison: true,
                ..conflict
            });
            return;
        }
        let result = match &conflict.disk_text {
            Some(expected) => {
                self.file_system
                    .write_text_if_unchanged(&conflict.path, expected, &file.text)
            }
            None => self
                .file_system
                .write_text_if_missing(&conflict.path, &file.text),
        };
        match result {
            Ok(()) => {
                file.mark_saved(conflict.path.clone());
                let saved_document = file.id;
                self.remember_recent_file(&conflict.path);
                self.view.status = "External version overwritten by explicit request".into();
                self.settings_file_saved(&conflict.path);
                self.notify_language_document_saved(saved_document);
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

    pub(crate) fn save_conflict_as(&mut self, document: crate::model::DocumentId) {
        let Some(index) = self
            .editor
            .files
            .iter()
            .position(|file| file.id == document)
        else {
            return;
        };
        let Some(path) = self.dialogs.save_as(&self.editor.files[index].name) else {
            return;
        };
        if self.editor.files[index].path.as_ref() == Some(&path) {
            self.view.status =
                "Choose another path for Save As; the original still has a disk conflict.".into();
            return;
        }
        self.save_file_at(index, path);
    }

    pub(crate) fn review_disk_change(&mut self, document: crate::model::DocumentId) {
        let Some(file) = self
            .editor
            .files
            .iter()
            .find(|file| file.id == document && file.disk_changed)
        else {
            return;
        };
        let Some(path) = file.path.clone() else {
            return;
        };
        let disk_text = match self.file_system.read_text(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.view.status = format!("Could not read changed file: {error}");
                return;
            }
        };
        self.view.save_conflict = Some(SaveConflict {
            document,
            revision: file.revision,
            disk_text,
            path,
            show_comparison: true,
        });
    }

    pub(crate) fn save_active(&mut self) {
        let Some(index) = self.editor.active_index() else {
            return;
        };
        if let Some(path) = self.editor.files[index].path.clone() {
            self.save_file_at(index, path);
        } else {
            self.save_as_active();
        }
    }

    pub(crate) fn save_as_active(&mut self) {
        let Some(index) = self.editor.active_index() else {
            return;
        };
        if let Some(path) = self.dialogs.save_as(&self.editor.files[index].name) {
            self.save_file_at(index, path);
        }
    }

    pub(crate) fn request_close(&mut self, id: crate::model::DocumentId) {
        Command::RequestClose(id).apply_local(&mut self.editor, &mut self.view);
    }

    pub(crate) fn close_file(&mut self, id: crate::model::DocumentId) {
        Command::CloseFile(id).apply_local(&mut self.editor, &mut self.view);
    }
}
