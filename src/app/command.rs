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
    ClearTerminal,
    PullSourceControl,
    PushSourceControl,
    ShowSourceDiff(PathBuf, DiffSide),
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
    FocusNextGroup,
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
                view.settings.open = false;
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
                    view.settings.open = false;
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
