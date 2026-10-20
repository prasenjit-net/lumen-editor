use super::agent::AgentKind;
use super::editor_action::EditorAction;
use super::tab_commands::TabMove;
use super::view_state::{Activity, ClosedTab, ExplorerEdit, MAX_CLOSED_TABS, ViewState};
use super::workspace_search::ReplaceScope;
use crate::model::{Document, DocumentId, EditorState};
use crate::services::source_control::DiffSide;
use std::ops::Range;
use std::path::PathBuf;

/// Shared actions from native menus, shortcuts and workbench components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Settings,
    /// Start a new native agent chat in the current workspace.
    NativeNewSession(lumen_agent_core::ProviderId),
    OpenChat(lumen_agent_core::SessionId),
    CloseChat(lumen_agent_core::SessionId),
    /// Send the session's composer text.
    ChatSend(lumen_agent_core::SessionId),
    ChatStop(lumen_agent_core::SessionId),
    ChatCancelQueued(
        lumen_agent_core::SessionId,
        crate::model::agent_scheduler::Ticket,
    ),
    ChatApproval(
        lumen_agent_core::SessionId,
        lumen_agent_core::RequestToken,
        lumen_agent_core::api::ApprovalDecisionId,
    ),
    /// Change one setting (model, permission mode, ...) of a session.
    ChatSetConfig(lumen_agent_core::SessionId, String, String),
    /// Return a setting to its default (not the same as picking a value).
    ChatResetConfig(lumen_agent_core::SessionId, String),
    ArchiveChat(lumen_agent_core::SessionId),
    PinChat(lumen_agent_core::SessionId, bool),
    /// Open a blank task editor tab in the given scope.
    NewTaskEditor(lumen_task_core::Scope),
    /// Open (or focus) an existing task for editing.
    OpenTaskEditor(lumen_task_core::TaskKey),
    ActivateTaskEditor(super::task_editor::TaskEditorId),
    SaveTaskEditor(super::task_editor::TaskEditorId),
    RevertTaskEditor(super::task_editor::TaskEditorId),
    ReloadTaskEditor(super::task_editor::TaskEditorId),
    /// Close a task editor, asking first when it has unsaved changes.
    CloseTaskEditor(super::task_editor::TaskEditorId),
    SaveAndCloseTaskEditor(super::task_editor::TaskEditorId),
    DiscardTaskEditor(super::task_editor::TaskEditorId),
    CancelTaskEditorClose,
    RefreshTasks,
    RunTask(lumen_task_core::TaskKey),
    RunSelectedTask,
    RerunLastTask,
    /// Run the default task of a group such as `build` or `test`.
    RunDefaultTask(&'static str),
    /// Switch to Run and Debug, where active runs are listed.
    ShowRunningTasks,
    StopTaskRun(super::task_runs::RunId),
    RestartTaskRun(super::task_runs::RunId),
    /// Remove a finished run's tab, or ask before stopping an active one.
    CloseTaskRun(super::task_runs::RunId),
    /// Stop an active run and remove its tab (answer to the close prompt).
    StopAndCloseTaskRun(super::task_runs::RunId),
    CancelCloseTaskRun,
    ShowTaskRun(super::task_runs::RunId),
    ClearTaskOutput(super::task_runs::RunId),
    /// Keyboard input for an interactive (PTY) task run.
    SendTaskInput(super::task_runs::RunId, Vec<u8>),
    /// The task terminal was measured at this many rows and columns.
    ResizeTaskTerminal(super::task_runs::RunId, u16, u16),
    CopyTaskOutput(super::task_runs::RunId),
    /// Save the retained output to a file chosen in a Save dialog.
    SaveTaskOutput(super::task_runs::RunId),
    /// Answer the run review prompt; `true` saves dirty files first.
    ConfirmTaskRun(bool),
    CancelTaskRun,
    /// Answer the input prompt of a run (the values live in the prompt itself).
    SubmitTaskInputs,
    CancelTaskInputs,
    /// Hide the task output panel; runs keep going.
    HideTaskPanel,
    /// Stop the workspace's running tasks, then change folder.
    ConfirmWorkspaceSwitch,
    CancelWorkspaceSwitch,
    /// Stop running tasks, then quit.
    ConfirmTaskQuit,
    CancelTaskQuit,
    SelectTask(lumen_task_core::TaskKey),
    DuplicateTask(lumen_task_core::TaskKey),
    RequestDeleteTask(lumen_task_core::TaskKey),
    ConfirmDeleteTask(lumen_task_core::TaskKey),
    CancelDeleteTask,
    /// Open the raw tasks.toml of a scope as a document.
    OpenTaskToml(lumen_task_core::Scope),
    ApplySettings,
    ReloadSettings,
    OpenSettingsFile,
    DiscardSettingsAndQuit,
    Find,
    Replace,
    StartWorkspaceSearch,
    CancelWorkspaceSearch,
    ClearWorkspaceSearch,
    RefreshSourceControl,
    ToggleTerminal,
    ToggleProblems,
    ToggleOutput,
    ToggleAgentPanel,
    SelectAgent(AgentKind),
    SendAgentPrompt,
    CancelAgentTurn,
    NewAgentSession,
    RetryAgent,
    AuthenticateAgent(String),
    /// Answer a queued permission request; `None` declines it.
    AnswerAgentPermission(
        AgentKind,
        lumen_agent_protocol::rpc::RequestId,
        Option<String>,
    ),
    AllowAgentNpx,
    ToggleAgentHistory,
    MoreAgentHistory,
    ResumeAgentSession(AgentKind, String),
    ResolveUnsavedFiles(super::agent::UnsavedChoice),
    /// Attach the editor selection (or current file) to the next agent message.
    AskAgentAboutSelection,
    AddFileToAgent(PathBuf),
    FixProblemWithAgent(DocumentId, Range<usize>),
    RemoveAgentAttachment(AgentKind, usize),
    /// Set a session option (mode, model, ...). Risky values ask first.
    SetAgentConfig(AgentKind, String, String),
    ConfirmAgentConfig(bool),
    /// Complete the `@` mention or `/` command being typed.
    PickAgentMention(AgentKind, String),
    PickSlash(AgentKind, String),
    /// Review, revert or dismiss what the agent changed.
    AgentChanges(AgentKind, super::agent_changes::ChangeAction),
    ShowOutput,
    ShowTerminal,
    CloseOutput,
    ClearOutput,
    CopyOutput,
    SendTerminalInput(Vec<u8>),
    CloseTerminal,
    CloseProblems,
    OpenProblem(DocumentId, Range<usize>),
    /// Open a task problem's file at line and column (1-based).
    OpenTaskProblem(PathBuf, usize, usize),
    ClearTaskProblems,
    ClearTerminal,
    PullSourceControl,
    PushSourceControl,
    FetchSourceControl,
    PublishSourceBranch,
    InitSourceRepository,
    ShowSourceDiff(PathBuf, DiffSide),
    SelectRepository(PathBuf),
    StagePaths(Vec<PathBuf>),
    UnstagePaths(Vec<PathBuf>),
    RequestSourceRevert(Vec<PathBuf>),
    ConfirmSourceRevert,
    StageHunk(PathBuf, usize),
    CommitSourceControl,
    PreviewWorkspaceReplace,
    PreviewWorkspaceReplaceScope(ReplaceScope),
    ApplyWorkspaceReplace,
    OpenWorkspaceHit(PathBuf, Range<usize>),
    QuickOpen,
    GoToLine,
    GoToDefinition,
    ShowHover,
    TriggerCompletion,
    FindReferences,
    FormatDocument,
    RenameSymbol,
    SubmitRename,
    CancelRename,
    ApplyLanguageEdit,
    CancelLanguageEdit,
    ShowSignatureHelp,
    ShowDocumentSymbols,
    OpenLanguageLocation(usize),
    OpenQuickPath(PathBuf),
    NavigateTo(DocumentId, String),
    EditorAction(DocumentId, EditorAction),
    NewFile,
    NewUntitled,
    NewFolder,
    RenamePath(PathBuf),
    DeletePath(PathBuf),
    ConfirmDelete,
    ExplorerCopy(PathBuf),
    ExplorerCut(PathBuf),
    ExplorerPaste,
    CopyPath(PathBuf, bool),
    RevealPath(PathBuf),
    CollapseFolders,
    RefreshExplorer,
    CommitExplorerEdit,
    OpenFile,
    OpenFolder,
    CloseFolder,
    Save,
    SaveAs,
    OverwriteConflict(DocumentId),
    ReloadConflict(DocumentId),
    SaveConflictAs(DocumentId),
    ReviewDiskChange(DocumentId),
    DismissConflict,
    CloseEditor,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Quit,
    Palette,
    ToggleSidebar,
    ActivateActivity(Activity),
    ShowWorkspaceSearch,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Welcome,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Copy,
    Cut,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Paste,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Undo,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Redo,
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    SelectAll,
    OpenPath(PathBuf),
    /// Open a previously used workspace folder.
    OpenRecentFolder(PathBuf),
    ShowRecentFolders,
    ClearRecentHistory,
    /// Show a workspace file or folder in the Explorer sidebar.
    RevealInExplorer(PathBuf),
    RequestClose(DocumentId),
    CloseTabs(crate::app::tab_close::TabCloseScope),
    /// Tab commands relative to the active tab, for the palette and shortcuts.
    CloseOtherTabs,
    CloseTabsToLeft,
    CloseTabsToRight,
    CloseSavedTabs,
    CloseAllTabs,
    NextTab,
    PreviousTab,
    ReopenClosedTab,
    /// Reorder a document tab, by stable ID.
    MoveTab(DocumentId, TabMove),
    MoveActiveTabLeft,
    MoveActiveTabRight,
    /// Open the second editor group with the active tab, or move it across.
    SplitEditor,
    /// Send a tab to the other editor group (opening it if needed).
    MoveTabToGroup(DocumentId),
    /// Drop a tab into `group` at `position` among that group's tabs.
    MoveTabToGroupAt(DocumentId, usize, usize),
    FocusNextGroup,
    /// Switch the two editor groups between side by side and stacked.
    ToggleSplitOrientation,
    FocusGroup(usize),
    CloseFile(DocumentId),
    SaveAndClose(DocumentId),
    SaveAllAndQuit,
    DiscardAndQuit,
}

impl Command {
    /// Apply buffer-only actions immediately, including inside a render pass.
    /// This keeps tab changes visible in the same frame without giving the UI
    /// access to dialogs or filesystem writes. Other actions go to the controller.
    pub(crate) fn apply_local(&self, editor: &mut EditorState, view: &mut ViewState) -> bool {
        match *self {
            Self::ActivateActivity(activity) => {
                view.sidebar_visible = !(view.sidebar_visible && view.activity == activity);
                view.activity = activity;
                // Hidden or replaced explorer views must not intercept editor shortcuts.
                view.explorer_focused = false;
            }
            Self::ShowWorkspaceSearch => {
                view.activity = Activity::Search;
                view.sidebar_visible = true;
                view.workspace_search.focus_query = true;
                view.explorer_focused = false;
            }
            Self::NewUntitled => {
                view.leave_page_tabs();
                if let Some(diff) = &mut view.source_control.diff {
                    diff.active = false;
                }
                editor.new_file();
                view.focus_editor = true;
                view.status = "New untitled file".into();
            }
            Self::CollapseFolders => view.expanded_dirs.clear(),
            Self::RefreshExplorer => view.status = "Explorer refreshed".into(),
            Self::NewFile => {
                if let Some(root) = &editor.workspace {
                    if view.explorer_edit.is_none() {
                        let directory = view
                            .explorer_selection
                            .as_ref()
                            .and_then(|selection| selection.directory())
                            .filter(|directory| directory.starts_with(root))
                            .unwrap_or(root)
                            .to_path_buf();
                        view.explorer_edit = Some(ExplorerEdit::new(directory));
                        view.reveal_edit_directory(editor);
                    }
                } else {
                    view.leave_page_tabs();
                    if let Some(diff) = &mut view.source_control.diff {
                        diff.active = false;
                    }
                    editor.new_file();
                    view.focus_editor = true;
                    view.status = "New untitled file".into();
                }
            }
            Self::RequestClose(id) => {
                if editor
                    .index_of(id)
                    .is_some_and(|index| editor.files[index].dirty)
                {
                    view.pending_close = Some(id);
                } else {
                    Self::CloseFile(id).apply_local(editor, view);
                }
            }
            Self::CloseFile(id) => {
                if editor.index_of(id).is_some() {
                    if view
                        .source_control
                        .diff
                        .as_ref()
                        .and_then(|diff| diff.document)
                        == Some(id)
                    {
                        view.source_control.diff = None;
                    }
                    view.find.remove(&id);
                    // A document closed with all its content on disk can come
                    // back with Reopen Closed Tab; unsaved or untitled ones cannot.
                    if let Some(index) = editor.index_of(id)
                        && let document = &editor.files[index]
                        && !document.dirty
                        && let Some(path) = document.path.clone()
                    {
                        view.closed_tabs.retain(|closed| closed.path != path);
                        view.closed_tabs.push(ClosedTab {
                            path,
                            caret: caret_byte(document),
                        });
                        let excess = view.closed_tabs.len().saturating_sub(MAX_CLOSED_TABS);
                        view.closed_tabs.drain(..excess);
                    }
                    editor.close_document(id);
                    view.status = "Closed file".into();
                }
            }
            _ => return false,
        }
        true
    }

    /// Stable native menu identifiers; data-carrying UI actions have no menu ID.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) fn menu_id(&self) -> Option<&'static str> {
        match self {
            Self::Settings => Some("settings"),
            Self::Find => Some("find"),
            Self::Replace => Some("replace"),
            Self::NewUntitled => Some("new"),
            Self::OpenFile => Some("open"),
            Self::OpenFolder => Some("folder"),
            Self::CloseFolder => Some("close_folder"),
            Self::ShowRecentFolders => Some("recent_folders"),
            Self::NextTab => Some("next_tab"),
            Self::PreviousTab => Some("previous_tab"),
            Self::ReopenClosedTab => Some("reopen_tab"),
            Self::Save => Some("save"),
            Self::SaveAs => Some("save_as"),
            Self::CloseEditor => Some("close"),
            Self::Quit => Some("quit"),
            Self::Palette => Some("palette"),
            Self::QuickOpen => Some("quick_open"),
            Self::GoToLine => Some("go_to_line"),
            Self::ToggleSidebar => Some("sidebar"),
            Self::ToggleTerminal => Some("terminal"),
            Self::ToggleOutput => Some("output"),
            Self::ShowWorkspaceSearch => Some("workspace_search"),
            Self::Welcome => Some("welcome"),
            Self::Copy => Some("copy"),
            Self::Cut => Some("cut"),
            Self::Paste => Some("paste"),
            Self::Undo => Some("undo"),
            Self::Redo => Some("redo"),
            Self::SelectAll => Some("select_all"),
            _ => None,
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", test))]
    pub(crate) fn from_menu_id(id: &str) -> Option<Self> {
        Some(match id {
            "settings" => Self::Settings,
            "find" => Self::Find,
            "replace" => Self::Replace,
            "new" => Self::NewUntitled,
            "open" => Self::OpenFile,
            "folder" => Self::OpenFolder,
            "close_folder" => Self::CloseFolder,
            "recent_folders" => Self::ShowRecentFolders,
            "next_tab" => Self::NextTab,
            "previous_tab" => Self::PreviousTab,
            "reopen_tab" => Self::ReopenClosedTab,
            "save" => Self::Save,
            "save_as" => Self::SaveAs,
            "close" => Self::CloseEditor,
            "quit" => Self::Quit,
            "palette" => Self::Palette,
            "quick_open" => Self::QuickOpen,
            "go_to_line" => Self::GoToLine,
            "sidebar" => Self::ToggleSidebar,
            "terminal" => Self::ToggleTerminal,
            "output" => Self::ToggleOutput,
            "workspace_search" => Self::ShowWorkspaceSearch,
            "welcome" => Self::Welcome,
            "copy" => Self::Copy,
            "cut" => Self::Cut,
            "paste" => Self::Paste,
            "undo" => Self::Undo,
            "redo" => Self::Redo,
            "select_all" => Self::SelectAll,
            _ => return None,
        })
    }
}

/// Primary caret of the document, or the start when it has no live editor state.
fn caret_byte(document: &Document) -> usize {
    #[cfg(feature = "shared-editor")]
    if let Some(buffer) = document.editing.as_ref() {
        let selections = buffer.selections();
        return selections.selections()[selections.primary_index()].head;
    }
    let _ = document;
    0
}
