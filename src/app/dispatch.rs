use super::{LumenApp, command::Command};
use eframe::egui;

impl LumenApp {
    pub(crate) fn execute(&mut self, command: Command, ctx: &egui::Context) {
        if matches!(
            command,
            Command::Copy | Command::Cut | Command::CopyPath(_, _)
        ) {
            self.view.text_clipboard.clear();
            #[cfg(feature = "shared-editor")]
            if let Some(surface) = &mut self.view.shared_surface {
                surface.clear_clipboard();
            }
        }
        let explorer_clipboard = self.view.explorer_focused
            && self.view.explorer_edit.is_none()
            && self.view.delete_prompt.is_none()
            && !self.view.command_palette
            && self.view.navigation.mode.is_none()
            && !ctx.wants_keyboard_input();
        match command {
            Command::Settings => {
                self.view.settings.tab_open = true;
                self.view.settings.open = true;
                self.view.settings.focus_search = true;
                self.view.explorer_focused = false;
                super::editor_input::cancel_input(&mut self.view);
                self.view.pending_editor_action = None;
                if let Some(file) = self.editor.active() {
                    let id = super::editor_action::text_widget_id(file.id);
                    ctx.memory_mut(|memory| memory.surrender_focus(id));
                }
            }
            Command::ApplySettings => self.apply_settings(),
            Command::ReloadSettings => self.reload_settings(),
            Command::DiscardSettingsAndQuit => {
                self.view.settings.discard_drafts();
                self.request_quit(ctx);
            }
            Command::OpenSettingsFile => {
                if let Some(path) = self.view.settings.selected().path.clone() {
                    self.open_file(path);
                }
            }
            Command::Find if !self.view.settings.open => self.open_find(false, ctx),
            Command::Replace if !self.view.settings.open => self.open_find(true, ctx),
            Command::Find | Command::Replace => (),
            Command::StartWorkspaceSearch => self.start_workspace_search(ctx),
            Command::CancelWorkspaceSearch => self.cancel_workspace_search(),
            Command::ClearWorkspaceSearch => self.clear_workspace_search(),
            Command::RefreshSourceControl => self.refresh_source_control(ctx),
            Command::PullSourceControl => self.pull_source_control(ctx),
            Command::PushSourceControl => self.push_source_control(ctx),
            Command::ShowSourceDiff(path, side) => self.show_source_diff(path, side, ctx),
            Command::StagePaths(paths) => self.stage_source_paths(paths, ctx),
            Command::UnstagePaths(paths) => self.unstage_source_paths(paths, ctx),
            Command::RequestSourceRevert(paths) => self.request_source_revert(paths),
            Command::ConfirmSourceRevert => self.confirm_source_revert(ctx),
            Command::StageHunk(path, hunk) => self.stage_source_hunk(path, hunk, ctx),
            Command::CommitSourceControl => self.commit_source_control(ctx),
            Command::PreviewWorkspaceReplace => self.preview_workspace_replace(),
            Command::PreviewWorkspaceReplaceScope(scope) => {
                self.preview_workspace_replace_scope(scope)
            }
            Command::ApplyWorkspaceReplace => self.apply_workspace_replace(),
            Command::OpenWorkspaceHit(path, range) => self.open_workspace_hit(path, range),
            Command::QuickOpen => self.open_quick_open(ctx),
            Command::GoToLine => self.open_go_to_line(),
            Command::OpenQuickPath(path) => self.open_quick_path(path),
            Command::NavigateTo(document, input) => self.go_to_line(document, &input),
            Command::EditorAction(document, action) => {
                if !self.view.settings.open
                    && self.editor.active().is_some_and(|file| file.id == document)
                {
                    self.view.pending_editor_action =
                        Some(super::editor_action::PendingEditorAction { document, action });
                    ctx.request_repaint();
                }
            }
            Command::NewFile => self.new_file(),
            Command::NewUntitled
            | Command::CollapseFolders
            | Command::RefreshExplorer
            | Command::ActivateActivity(_)
            | Command::ShowWorkspaceSearch => {
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
            Command::Save if self.view.settings.open => {
                if self.view.settings.selected().pending() > 0 {
                    self.apply_settings();
                }
            }
            Command::SaveAs if self.view.settings.open => {
                self.view.status =
                    "Settings use their selected User/Project config path; Save As is unavailable."
                        .into();
            }
            Command::Save => self.save_active(),
            Command::SaveAs => self.save_as_active(),
            Command::OverwriteConflict(document) => self.overwrite_conflict(document),
            Command::ReloadConflict(document) => self.reload_conflict(document),
            Command::SaveConflictAs(document) => self.save_conflict_as(document),
            Command::ReviewDiskChange(document) => self.review_disk_change(document),
            Command::DismissConflict => self.view.save_conflict = None,
            Command::RequestClose(index) => self.request_close(index),
            Command::CloseFile(index) => self.close_file(index),
            Command::CloseEditor => {
                if self.view.settings.open {
                    self.view.settings.tab_open = false;
                    self.view.settings.open = false;
                    return;
                }
                if self
                    .view
                    .source_control
                    .diff
                    .as_ref()
                    .is_some_and(|diff| diff.active)
                {
                    super::source_control::close_diff_preview(&mut self.editor, &mut self.view);
                    return;
                }
                if let Some(index) = self.editor.active_file {
                    self.request_close(index);
                }
            }
            Command::Quit => self.request_quit(ctx),
            Command::Palette => {
                self.view.navigation.mode = None;
                self.view.command_palette = true;
                self.view.command_palette_focus = true;
            }
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
            Command::Paste => {
                if let Some(document) = self.editor.active().filter(|document| {
                    ctx.memory(|memory| {
                        memory.has_focus(super::editor_action::text_widget_id(document.id))
                    })
                }) {
                    super::editor_action::request_paste(&mut self.view, document, ctx);
                } else {
                    super::editor_action::EditorAction::Paste.send(ctx);
                }
            }
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
        if !self.clear_recovery_for_exit() {
            return;
        }
        self.view.allow_quit = true;
        self.view.pending_quit = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
