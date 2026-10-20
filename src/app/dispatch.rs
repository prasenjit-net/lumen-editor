use super::{LumenApp, command::Command, tab_close::TabCloseScope, tab_commands::TabMove};
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
                self.view.task_editors.deactivate();
                self.view.chats.deactivate();
                self.enter_page_tab(ctx);
            }
            Command::NativeNewSession(provider) => self.native_new_session(provider, ctx),
            Command::OpenChat(id) => self.open_chat(id, ctx),
            Command::CloseChat(id) => self.view.chats.close(id),
            Command::ChatSend(id) => self.chat_send(id),
            Command::ChatStop(id) => self.chat_stop(id),
            Command::ChatCancelQueued(id, ticket) => self.chat_cancel_queued(id, ticket),
            Command::ChatApproval(id, token, decision) => self.chat_approval(id, token, decision),
            Command::ChatSetConfig(id, option, value) => self.chat_set_config(id, option, value),
            Command::ChatResetConfig(id, option) => self.chat_reset_config(id, option),
            Command::ArchiveChat(id) => self.archive_chat(id),
            Command::PinChat(id, pinned) => self.pin_chat(id, pinned),
            Command::NewTaskEditor(scope) => {
                self.new_task_editor(scope);
                if self.view.task_editors.active().is_some() {
                    self.show_task_editor_page(ctx);
                }
            }
            Command::OpenTaskEditor(key) => {
                if self.open_task_editor(key) {
                    self.show_task_editor_page(ctx);
                }
            }
            Command::ActivateTaskEditor(id) => {
                if self.view.task_editors.activate(id) {
                    self.show_task_editor_page(ctx);
                }
            }
            Command::SaveTaskEditor(id) => {
                self.save_task_editor(id);
            }
            Command::RevertTaskEditor(id) => self.revert_task_editor(id),
            Command::ReloadTaskEditor(id) => self.reload_task_editor(id),
            Command::CloseTaskEditor(id) => self.request_close_task_editor(id),
            Command::SaveAndCloseTaskEditor(id) => self.save_and_close_task_editor(id),
            Command::DiscardTaskEditor(id) => self.discard_task_editor(id),
            Command::CancelTaskEditorClose => self.view.task_close_prompt = None,
            Command::OpenTaskToml(scope) => self.open_task_toml(scope),
            Command::RefreshTasks => self.refresh_tasks(),
            Command::RunTask(key) => self.run_task(key, ctx),
            Command::RunSelectedTask => match self.view.tasks.selected.clone() {
                Some(key) => self.run_task(key, ctx),
                None => self.view.status = "Select a task in Run and Debug first".into(),
            },
            Command::RerunLastTask => match self.view.task_runs.last.clone() {
                Some(key) => self.run_task(key, ctx),
                None => self.view.status = "No task has been run yet".into(),
            },
            Command::RunDefaultTask(group) => {
                // The list may never have been shown, so read it fresh first.
                self.refresh_tasks();
                match self.view.tasks.default_for_group(group) {
                    Some(key) => self.run_task(key, ctx),
                    None => self.view.status = format!("No default {group} task is defined"),
                }
            }
            Command::ShowRunningTasks => {
                self.view.activity = super::view_state::Activity::RunDebug;
                self.view.sidebar_visible = true;
                self.view.explorer_focused = false;
            }
            Command::StopTaskRun(id) => self.stop_task_run(id, super::task_runs::StopReason::User),
            Command::RestartTaskRun(id) => self.restart_task_run(id, ctx),
            Command::CloseTaskRun(id) => self.close_task_run(id),
            Command::StopAndCloseTaskRun(id) => {
                self.view.task_run_close_prompt = None;
                self.stop_task_run(id, super::task_runs::StopReason::Close);
                self.view.task_runs.close_when_finished.push(id);
            }
            Command::CancelCloseTaskRun => self.view.task_run_close_prompt = None,
            Command::ShowTaskRun(id) => {
                super::output::clear_text_selection(ctx);
                self.show_task_run(id);
            }
            Command::ClearTaskOutput(id) => self.clear_task_output(id),
            Command::SendTaskInput(id, bytes) => self.send_task_input(id, &bytes),
            Command::ResizeTaskTerminal(id, rows, columns) => {
                self.resize_task_terminal(id, rows, columns)
            }
            Command::CopyTaskOutput(id) => self.copy_task_output(id, ctx),
            Command::SaveTaskOutput(id) => self.save_task_output(id),
            Command::ConfirmTaskRun(save_first) => self.confirm_pending_task(save_first, ctx),
            Command::CancelTaskRun => self.cancel_pending_task(),
            Command::SubmitTaskInputs => self.submit_task_inputs(ctx),
            Command::CancelTaskInputs => self.cancel_task_inputs(),
            Command::HideTaskPanel => self.view.task_runs.panel_open = false,
            Command::ConfirmWorkspaceSwitch => self.confirm_workspace_switch(),
            Command::CancelWorkspaceSwitch => self.cancel_workspace_switch(),
            Command::ConfirmTaskQuit => self.confirm_task_quit(std::time::Instant::now()),
            Command::CancelTaskQuit => self.cancel_task_quit(),
            Command::SelectTask(key) => self.view.tasks.selected = Some(key),
            Command::DuplicateTask(key) => {
                self.duplicate_task(key);
                if self.view.task_editors.active().is_some() {
                    self.show_task_editor_page(ctx);
                }
            }
            Command::RequestDeleteTask(key) => self.request_delete_task(key),
            Command::ConfirmDeleteTask(key) => self.delete_task_definition(key),
            Command::CancelDeleteTask => self.view.task_delete_prompt = None,
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
            Command::ToggleTerminal => {
                super::output::clear_text_selection(ctx);
                self.toggle_terminal(ctx);
            }
            Command::ToggleProblems => {
                self.view.problems.open = !self.view.problems.open;
                if self.view.problems.open {
                    self.view.terminal.open = false;
                    self.view.output.open = false;
                }
            }
            Command::ToggleAgentPanel => self.toggle_agent_panel(),
            Command::SelectAgent(kind) => self.select_agent(kind),
            Command::SendAgentPrompt => self.send_agent_prompt(ctx),
            Command::CancelAgentTurn => self.cancel_agent_turn(),
            Command::NewAgentSession => self.new_agent_session(),
            Command::RetryAgent => self.retry_agent(ctx),
            Command::AuthenticateAgent(method) => self.authenticate_agent(&method),
            Command::AnswerAgentPermission(kind, id, option) => {
                self.answer_agent_permission(kind, &id, option.as_deref());
            }
            Command::AgentChanges(kind, action) => self.apply_change_action(kind, action),
            Command::AskAgentAboutSelection => self.ask_agent_about_selection(),
            Command::AddFileToAgent(path) => self.add_file_to_agent(path),
            Command::FixProblemWithAgent(document, range) => {
                self.fix_problem_with_agent(document, range);
            }
            Command::RemoveAgentAttachment(kind, index) => {
                self.remove_agent_attachment(kind, index);
            }
            Command::SetAgentConfig(kind, id, value) => self.set_agent_config(kind, &id, &value),
            Command::ConfirmAgentConfig(accepted) => self.confirm_agent_config(accepted),
            Command::PickAgentMention(kind, relative) => self.pick_agent_mention(kind, &relative),
            Command::PickSlash(kind, name) => self.pick_agent_command(kind, &name),
            Command::ResolveUnsavedFiles(choice) => self.resolve_unsaved_choice(choice, ctx),
            Command::ToggleAgentHistory => self.toggle_agent_history(ctx),
            Command::MoreAgentHistory => self.more_agent_history(ctx),
            Command::ResumeAgentSession(kind, id) => self.resume_agent_session(kind, &id, ctx),
            Command::AllowAgentNpx => self.allow_agent_npx(ctx),
            Command::ToggleOutput => {
                super::output::clear_text_selection(ctx);
                self.toggle_output();
            }
            Command::ShowOutput => {
                super::output::clear_text_selection(ctx);
                self.show_output();
            }
            Command::ShowTerminal => {
                super::output::clear_text_selection(ctx);
                if !self.view.terminal.open {
                    self.toggle_terminal(ctx);
                }
            }
            Command::CloseOutput => self.view.output.open = false,
            Command::ClearOutput => self.clear_output(),
            Command::CopyOutput => self.copy_output(ctx),
            Command::SendTerminalInput(bytes) => self.send_terminal_input(&bytes, ctx),
            Command::CloseTerminal => self.close_terminal(),
            Command::CloseProblems => self.view.problems.open = false,
            Command::OpenProblem(document, range) => self.open_problem(document, range),
            Command::OpenTaskProblem(path, line, column) => {
                self.open_task_problem(path, line, column)
            }
            Command::ClearTaskProblems => self.clear_task_problems(),
            Command::ClearTerminal => self.clear_terminal(),
            Command::PullSourceControl => self.pull_source_control(ctx),
            Command::PushSourceControl => self.push_source_control(ctx),
            Command::ShowSourceDiff(path, side) => self.show_source_diff(path, side, ctx),
            Command::SelectRepository(root) => self.select_repository(root),
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
            Command::GoToDefinition => self.request_language_definition(),
            Command::ShowHover => self.request_language_hover(),
            Command::FindReferences => self.request_language_references(),
            Command::FormatDocument => self.request_language_formatting(),
            Command::RenameSymbol => self.begin_language_rename(),
            Command::SubmitRename => self.submit_language_rename(),
            Command::CancelRename => self.cancel_language_rename(),
            Command::ApplyLanguageEdit => self.apply_language_edit(),
            Command::CancelLanguageEdit => self.cancel_language_edit(),
            Command::TriggerCompletion => self.request_language_completion(),
            Command::ShowSignatureHelp => self.request_language_signature_help(),
            Command::ShowDocumentSymbols => self.request_document_symbols(),
            Command::OpenLanguageLocation(index) => self.open_language_location(index),
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
            Command::CloseFolder => self.close_folder(),
            Command::OpenPath(path) => self.open_file(path),
            Command::RevealInExplorer(path) => self.reveal_in_explorer(path),
            Command::OpenRecentFolder(path) => {
                self.view.navigation.mode = None;
                self.open_folder_path(path);
            }
            Command::ShowRecentFolders => self.open_recent_folder_picker(),
            Command::ClearRecentHistory => self.clear_recent_history(),
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
            Command::Save => match self.view.task_editors.active() {
                // The focused task editor owns Save while its page shows.
                Some(id) => {
                    self.save_task_editor(id);
                }
                None => self.save_active(),
            },
            Command::SaveAs => self.save_as_active(),
            Command::OverwriteConflict(document) => self.overwrite_conflict(document),
            Command::ReloadConflict(document) => self.reload_conflict(document),
            Command::SaveConflictAs(document) => self.save_conflict_as(document),
            Command::ReviewDiskChange(document) => self.review_disk_change(document),
            Command::DismissConflict => self.view.save_conflict = None,
            Command::RequestClose(id) => self.request_close(id),
            Command::CloseFile(id) => self.close_file(id),
            Command::CloseTabs(scope) => self.begin_bulk_close(scope),
            Command::CloseOtherTabs => self.close_tabs_relative(TabCloseScope::Others),
            Command::CloseTabsToLeft => self.close_tabs_relative(TabCloseScope::ToTheLeft),
            Command::CloseTabsToRight => self.close_tabs_relative(TabCloseScope::ToTheRight),
            Command::CloseSavedTabs => self.begin_bulk_close(TabCloseScope::Saved),
            Command::CloseAllTabs => self.begin_bulk_close(TabCloseScope::All),
            Command::NextTab => self.cycle_tab(1),
            Command::PreviousTab => self.cycle_tab(-1),
            Command::ReopenClosedTab => self.reopen_closed_tab(),
            Command::MoveTab(id, movement) => self.move_tab(id, movement),
            Command::MoveActiveTabLeft => self.move_active_tab(TabMove::Left),
            Command::MoveActiveTabRight => self.move_active_tab(TabMove::Right),
            Command::SplitEditor => self.split_editor(),
            Command::MoveTabToGroup(id) => self.move_tab_to_group(id),
            Command::MoveTabToGroupAt(id, group, position) => {
                self.move_tab_to_group_at(id, group, position)
            }
            Command::FocusNextGroup => self.focus_next_group(),
            Command::ToggleSplitOrientation => {
                if self.editor.group_count() < 2 {
                    self.view.status = "The editor is not split".into();
                } else {
                    self.view.split_vertical = !self.view.split_vertical;
                    self.view.status = if self.view.split_vertical {
                        "Editor groups stacked".into()
                    } else {
                        "Editor groups side by side".into()
                    };
                }
            }
            Command::FocusGroup(group) => self.focus_group_command(group),
            Command::CloseEditor => {
                if let Some(id) = self.view.chats.active() {
                    self.view.chats.close(id);
                    return;
                }
                if let Some(id) = self.view.task_editors.active() {
                    self.request_close_task_editor(id);
                    return;
                }
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
                if let Some(id) = self.editor.active_id() {
                    self.request_close(id);
                }
            }
            Command::Quit => self.request_quit(ctx),
            Command::Palette => {
                if !self.view.command_palette {
                    // Run Task entries list the tasks as they are on disk now.
                    self.refresh_tasks();
                    self.view.palette_previous_focus = ctx.memory(|memory| memory.focused());
                    self.view.palette_query.clear();
                    self.view.palette_selected = 0;
                }
                self.view.navigation.mode = None;
                self.view.command_palette = true;
                self.view.command_palette_focus = true;
            }
            Command::ToggleSidebar => self.view.sidebar_visible = !self.view.sidebar_visible,
            Command::Welcome => {
                self.view.status = format!(
                    "Lumen — New: {} · Open: {} · Save: {}",
                    crate::ui::shortcut::primary("N"),
                    crate::ui::shortcut::primary("O"),
                    crate::ui::shortcut::primary("S")
                )
            }
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
            Command::SaveAndClose(id) => {
                self.view.pending_close = None;
                // Save the document the prompt is about, not whichever tab
                // happens to be active. It stays active if the save is cancelled
                // or fails, so the user lands on the file that is still unsaved.
                if self.editor.activate(id) {
                    self.save_active();
                    if self
                        .editor
                        .index_of(id)
                        .is_some_and(|index| !self.editor.files[index].dirty)
                    {
                        self.close_file(id);
                    }
                }
            }
            Command::SaveAllAndQuit => {
                let active = self.editor.active_index();
                for index in 0..self.editor.files.len() {
                    if self.editor.files[index].dirty {
                        self.editor.set_active_index(Some(index));
                        self.save_active();
                        if self.editor.files[index].dirty {
                            break;
                        }
                    }
                }
                self.editor.set_active_index(active);
                if !self.editor.has_unsaved_changes() {
                    self.finish_quit(ctx);
                }
            }
            Command::DiscardAndQuit => self.finish_quit(ctx),
        }
    }

    /// Common prelude for showing a non-document page: it takes over from the
    /// explorer and from any text widget that held keyboard focus.
    pub(super) fn enter_page_tab(&mut self, ctx: &egui::Context) {
        self.view.explorer_focused = false;
        super::editor_input::cancel_input(&mut self.view);
        self.view.pending_editor_action = None;
        if let Some(file) = self.editor.active() {
            let id = super::editor_action::text_widget_id(file.id);
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
    }

    /// The task editor was just opened or activated: it replaces Settings and
    /// the diff view as the page the editor area shows.
    fn show_task_editor_page(&mut self, ctx: &egui::Context) {
        self.view.settings.open = false;
        self.view.chats.deactivate();
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
        }
        self.enter_page_tab(ctx);
    }

    fn finish_quit(&mut self, ctx: &egui::Context) {
        if !self.clear_recovery_for_exit() {
            return;
        }
        self.view.pending_quit = false;
        self.close_window(ctx);
    }
}
