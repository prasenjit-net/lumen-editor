use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(crate) enum ExplorerSelection {
    Directory(PathBuf),
    File(PathBuf),
}

impl ExplorerSelection {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Directory(path) | Self::File(path) => path,
        }
    }

    pub(crate) fn directory(&self) -> Option<&Path> {
        match self {
            Self::Directory(path) => Some(path),
            Self::File(path) => path.parent(),
        }
    }
}

pub(crate) struct ExplorerEdit {
    pub(crate) directory: PathBuf,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
    pub(crate) focus_name: bool,
    pub(crate) kind: ExplorerEditKind,
}

#[derive(Clone)]
pub(crate) enum ExplorerEditKind {
    NewFile,
    NewFolder,
    Rename(PathBuf),
}

#[derive(Clone)]
pub(crate) struct ExplorerClipboard {
    pub(crate) path: PathBuf,
    pub(crate) cut: bool,
}

pub(crate) struct DeletePrompt {
    pub(crate) path: PathBuf,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavigationMode {
    QuickOpen,
    GoToLine,
    RecentFolders,
}

#[derive(Default)]
pub(crate) struct NavigationView {
    pub(crate) mode: Option<NavigationMode>,
    pub(crate) query: String,
    pub(crate) focus: bool,
    pub(crate) selected: usize,
    pub(crate) matches: Vec<PathBuf>,
    pub(crate) matched_query: Option<String>,
    pub(crate) indexed_root: Option<PathBuf>,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) running: bool,
    pub(crate) limited: bool,
    pub(crate) errors: usize,
    pub(crate) recent: Vec<PathBuf>,
    /// Recently opened workspace folders, most recent first.
    pub(crate) recent_folders: Vec<PathBuf>,
    pub(crate) recent_warning: Option<String>,
    pub(crate) error: Option<String>,
}

pub(crate) struct SaveConflict {
    pub(crate) document: crate::model::DocumentId,
    pub(crate) revision: u64,
    pub(crate) path: PathBuf,
    pub(crate) disk_text: Option<String>,
    pub(crate) show_comparison: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LanguageServerStatus {
    Starting(String),
    Ready(String),
    Available(String),
    Missing(String),
    Error(String),
}

/// Inline rename prompt for the identifier under the caret.
#[derive(Clone, Debug)]
pub(crate) struct RenamePrompt {
    pub(crate) document: crate::model::DocumentId,
    pub(crate) name: String,
    pub(crate) original: String,
    pub(crate) focus: bool,
}

/// An open completion popup. Typing narrows `items` through
/// `completion::visible`; `selected` indexes that filtered list.
#[derive(Clone, Debug)]
pub(crate) struct CompletionSession {
    pub(crate) document: crate::model::DocumentId,
    /// Caret byte when the server was asked; its edit ranges are valid there.
    pub(crate) origin: usize,
    pub(crate) word_start: usize,
    pub(crate) items: Vec<lumen_language_services::incoming::CompletionItem>,
    pub(crate) selected: usize,
    /// Item index chosen with the pointer, applied before the next editor input.
    pub(crate) pending_accept: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct SemanticSnapshot {
    pub(crate) revision: u64,
    pub(crate) spans: std::sync::Arc<Vec<lumen_language_services::TokenSpan>>,
}

/// Settings-page snapshot of the active file's language server; built by the
/// controller so rendering never probes PATH or reads session state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LanguageServerSummary {
    pub(crate) language: &'static str,
    pub(crate) state: String,
    pub(crate) command: Option<String>,
    pub(crate) capabilities: Vec<&'static str>,
    pub(crate) error: Option<String>,
    pub(crate) project_path_ignored: bool,
}

#[derive(Clone)]
pub(crate) struct LanguageLocation {
    pub(crate) label: String,
    pub(crate) path: PathBuf,
    pub(crate) position: lumen_language_services::Position,
}

#[derive(Default)]
pub(crate) struct LanguageNavigationView {
    pub(crate) open: bool,
    pub(crate) title: String,
    pub(crate) query: String,
    pub(crate) focus: bool,
    pub(crate) selected: usize,
    pub(crate) locations: Vec<LanguageLocation>,
}

#[derive(Default)]
pub(crate) struct WorkspaceSearchView {
    pub(crate) query: String,
    pub(crate) replacement: String,
    pub(crate) include: String,
    pub(crate) exclude: String,
    pub(crate) match_case: bool,
    pub(crate) whole_word: bool,
    pub(crate) replace_visible: bool,
    pub(crate) details_visible: bool,
    pub(crate) focus_query: bool,
    pub(crate) running: bool,
    pub(crate) progress: Option<crate::services::workspace_search::SearchProgress>,
    pub(crate) results: Option<crate::services::workspace_search::SearchResult>,
    pub(crate) completed_request: Option<crate::services::workspace_search::SearchRequest>,
    pub(crate) selected: HashSet<PathBuf>,
    pub(crate) expanded_files: HashSet<PathBuf>,
    pub(crate) expanded_folders: HashSet<PathBuf>,
    pub(crate) preview_open: bool,
    pub(crate) confirmation_open: bool,
    pub(crate) preview_replacement: Option<String>,
    pub(crate) preview_selected: HashSet<PathBuf>,
    pub(crate) preview_page: usize,
    pub(crate) report: Vec<String>,
    pub(crate) replaced_files: usize,
}

#[derive(Default)]
pub(crate) struct CommitDraft {
    pub(crate) message: String,
    pub(crate) amend: bool,
}

impl SourceControlView {
    /// The repository owning `path`: the deepest discovered root containing it,
    /// never an outer repository when a nested one claims the file.
    pub(crate) fn owning_snapshot(
        &self,
        path: &std::path::Path,
    ) -> Option<&crate::services::source_control::GitSnapshot> {
        self.repositories
            .iter()
            .chain(self.snapshot.as_ref())
            .filter(|snapshot| path.starts_with(&snapshot.root))
            .max_by_key(|snapshot| snapshot.root.components().count())
    }
}

/// Collapsible sections below the change groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ExtraSection {
    Stashes,
    Outgoing,
    Incoming,
}

/// The groups a repository's changes are listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ChangeGroup {
    Conflicts,
    Staged,
    Working,
}

impl ChangeGroup {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Conflicts => "MERGE CONFLICTS",
            Self::Staged => "STAGED CHANGES",
            Self::Working => "CHANGES",
        }
    }
}

#[derive(Default)]
pub(crate) struct SourceControlView {
    pub(crate) workspace: Option<PathBuf>,
    /// The selected repository's snapshot; every action targets this one.
    pub(crate) snapshot: Option<crate::services::source_control::GitSnapshot>,
    /// Every discovered repository, including the selected one.
    pub(crate) repositories: Vec<crate::services::source_control::GitSnapshot>,
    /// Repositories whose status could not be read, with the reason.
    pub(crate) repository_errors: Vec<(PathBuf, String)>,
    /// Commit drafts of repositories that are not currently selected.
    pub(crate) parked_drafts: std::collections::BTreeMap<PathBuf, CommitDraft>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    /// Initial branch typed for "Initialize Repository"; empty means `main`.
    pub(crate) init_branch: String,
    pub(crate) diff: Option<SourceDiffView>,
    /// Keeps the two diff panes on the same line; reset when the diff changes.
    pub(crate) diff_scroll: super::scroll_sync::ScrollSync,
    pub(crate) diff_scroll_key: Option<(PathBuf, crate::services::source_control::DiffSide)>,
    /// True only while the diff's editable pane is being drawn.
    pub(crate) diff_scroll_hook: bool,
    pub(crate) commit_message: String,
    pub(crate) amend_commit: bool,
    pub(crate) commit_error: Option<String>,
    pub(crate) commit_focus: bool,
    pub(crate) commit_focused: bool,
    pub(crate) revert_prompt: Option<SourceRevertPrompt>,
    /// A confirmation waiting for the user before aborting an operation.
    pub(crate) abort_prompt: bool,
    pub(crate) last_refresh: Option<std::time::Instant>,
    /// Change groups the user collapsed, keyed by repository and `ChangeGroup`.
    pub(crate) collapsed_groups: std::collections::BTreeSet<(PathBuf, ChangeGroup)>,
    /// Selected repositories whose section body the user collapsed.
    pub(crate) collapsed_repos: std::collections::BTreeSet<PathBuf>,
    /// Stashes and Outgoing/Incoming lists of the selected repository.
    pub(crate) extras: super::repo_extras::ExtrasState,
    /// Extra sections the user expanded; they start collapsed.
    pub(crate) expanded_extras: std::collections::BTreeSet<(PathBuf, ExtraSection)>,
}

#[derive(Default)]
pub(crate) struct TerminalView {
    pub(crate) open: bool,
    pub(crate) emulator: Option<lumen_terminal_core::Terminal>,
    pub(crate) error: Option<String>,
    pub(crate) scroll_to_bottom: bool,
    pub(crate) rows: u16,
    pub(crate) columns: u16,
    pub(crate) applied_rows: u16,
    pub(crate) applied_columns: u16,
}

/// Read-only log panel sharing the terminal's slot in the bottom panel.
#[derive(Default)]
pub(crate) struct OutputView {
    pub(crate) open: bool,
    pub(crate) entries: Vec<crate::model::output_log::LogEntry>,
    /// Newest log sequence copied into `entries`.
    pub(crate) seen: u64,
    pub(crate) scroll_to_bottom: bool,
}

pub(crate) struct ProblemsView {
    pub(crate) open: bool,
    pub(crate) filter: String,
    pub(crate) show_errors: bool,
    pub(crate) show_warnings: bool,
}

impl Default for ProblemsView {
    fn default() -> Self {
        Self {
            open: false,
            filter: String::new(),
            show_errors: true,
            show_warnings: true,
        }
    }
}

/// Confirmation state stays in the app layer; the UI only presents it and
/// emits typed commands, while the Git service performs the operation.
pub(crate) struct SourceRevertPrompt {
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) error: Option<String>,
}

pub(crate) struct SourceDiffView {
    pub(crate) root: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) side: crate::services::source_control::DiffSide,
    pub(crate) comparison:
        Result<std::sync::Arc<crate::services::source_control::GitComparison>, String>,
    pub(crate) document: Option<crate::model::DocumentId>,
    /// Document opened solely for this diff; its ordinary tab stays hidden.
    pub(crate) diff_only_document: Option<crate::model::DocumentId>,
    pub(crate) opened_revision: Option<u64>,
    pub(crate) active: bool,
}

impl ExplorerEdit {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            name: String::new(),
            error: None,
            focus_name: true,
            kind: ExplorerEditKind::NewFile,
        }
    }
}

/// Geometry of one group's tab strip in the current frame.
#[derive(Clone, Debug)]
pub(crate) struct TabStripGeometry {
    pub(crate) group: usize,
    /// Visible strip: as wide as its viewport and as tall as its tabs.
    pub(crate) band: eframe::egui::Rect,
    pub(crate) tabs: Vec<(crate::model::DocumentId, eframe::egui::Rect)>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TabDrag {
    pub(crate) source_group: usize,
}

/// A document closed without losing content: enough to open it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClosedTab {
    pub(crate) path: PathBuf,
    pub(crate) caret: usize,
}

pub(crate) const MAX_CLOSED_TABS: usize = 20;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Activity {
    #[default]
    Explorer,
    Search,
    SourceControl,
    RunDebug,
}

impl Activity {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Explorer => "EXPLORER",
            Self::Search => "SEARCH",
            Self::SourceControl => "SOURCE CONTROL",
            Self::RunDebug => "RUN AND DEBUG",
        }
    }
}

/// Whether the right-hand agent session list is showing.
#[derive(Default)]
pub(crate) struct AgentsView {
    pub(crate) open: bool,
}

pub(crate) struct ViewState {
    pub(crate) clone_dialog: Option<super::git_clone::CloneDialog>,
    pub(crate) branch_dialog: Option<super::git_branches::BranchDialog>,
    pub(crate) stash_dialog: Option<super::git_stash::StashDialog>,
    /// The History tab's state; present while the tab is open.
    pub(crate) history_dialog: Option<super::git_history::HistoryDialog>,
    /// The History tab is the page the editor area shows.
    pub(crate) history_active: bool,
    pub(crate) history_op_dialog: Option<super::git_history_ops::HistoryOpDialog>,
    pub(crate) tags_dialog: Option<super::git_tags::TagsDialog>,
    pub(crate) settings: super::settings::SettingsState,
    pub(crate) task_editors: super::task_editor::TaskEditorState,
    pub(crate) chats: super::chat_tabs::ChatTabs,
    /// The native agent controller; `None` until native agents are enabled.
    pub(crate) native_agents: Option<super::agent_hub::AgentHub>,
    pub(crate) tasks: super::task_catalog::TaskCatalog,
    pub(crate) task_runs: super::task_runs::TaskRuns,
    /// A run waiting on the review / unsaved-files prompt.
    pub(crate) task_pending: Option<super::task_runs::PendingTaskRun>,
    /// A run waiting for the values of its declared inputs.
    pub(crate) task_inputs: Option<super::task_runs::PendingInputs>,
    /// A running task whose tab is being closed, awaiting Stop and Close / Cancel.
    pub(crate) task_run_close_prompt: Option<super::task_runs::RunId>,
    pub(crate) task_workspace_switch: Option<super::task_runs::PendingWorkspaceSwitch>,
    pub(crate) task_quit: Option<super::task_runs::PendingTaskQuit>,
    /// A definition awaiting delete confirmation.
    pub(crate) task_delete_prompt: Option<lumen_task_core::TaskKey>,
    pub(crate) shared_surface: Option<lumen_editor_surface::EditorSurface>,
    /// Live editor surface of each editor group while the editor is split; the
    /// focused group's lives in `shared_surface` between frames.
    pub(crate) group_surfaces: Vec<Option<lumen_editor_surface::EditorSurface>>,
    /// Set while a non-first editor group draws, so it shows no Settings/Diff tab.
    pub(crate) hide_virtual_tabs: bool,
    /// Where each editor group's tabs were drawn this frame, for drops between groups.
    pub(crate) tab_strips: Vec<TabStripGeometry>,
    /// The tab being dragged this frame, if any.
    pub(crate) tab_drag: Option<TabDrag>,
    /// A drag released away from its own strip: resolved once every group has drawn.
    pub(crate) tab_drop_request: Option<(crate::model::DocumentId, eframe::egui::Pos2)>,
    /// Stack the two editor groups top to bottom instead of side by side.
    pub(crate) split_vertical: bool,
    /// Share of the editor area given to the first group when split (0..1).
    pub(crate) split_ratio: f32,
    pub(crate) shared_output: Option<lumen_editor_surface::SurfaceOutput>,
    pub(crate) shared_paste_selection: Option<lumen_editor_core::SelectionSet>,
    pub(crate) find: std::collections::HashMap<crate::model::DocumentId, super::find::FindState>,
    pub(crate) pending_editor_action: Option<super::editor_action::PendingEditorAction>,
    pub(crate) editor_paste_target: Option<super::editor_action::PasteTarget>,
    pub(crate) clipboard_read: Option<crate::services::ClipboardRequestId>,
    pub(crate) activity: Activity,
    pub(crate) expanded_dirs: HashSet<PathBuf>,
    pub(crate) explorer_selection: Option<ExplorerSelection>,
    pub(crate) explorer_edit: Option<ExplorerEdit>,
    pub(crate) delete_prompt: Option<DeletePrompt>,
    pub(crate) explorer_clipboard: Option<ExplorerClipboard>,
    pub(crate) explorer_focused: bool,
    pub(crate) focus_editor: bool,
    pub(crate) command_palette: bool,
    pub(crate) command_palette_focus: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) palette_previous_focus: Option<eframe::egui::Id>,
    pub(crate) navigation: NavigationView,
    pub(crate) pending_goto: Option<(crate::model::DocumentId, usize)>,
    pub(crate) pending_problem_selection:
        Option<(crate::model::DocumentId, std::ops::Range<usize>)>,
    pub(crate) fold_tabs:
        std::collections::HashMap<crate::model::DocumentId, super::folding::FoldTabState>,
    pub(crate) workspace_search: WorkspaceSearchView,
    pub(crate) source_control: SourceControlView,
    pub(crate) terminal: TerminalView,
    pub(crate) problems: ProblemsView,
    /// What the bottom panel showed when it was last hidden with its toggle.
    pub(crate) bottom_restore: Option<super::output::BottomView>,
    pub(crate) output: OutputView,
    pub(crate) agents: AgentsView,
    pub(crate) language_server_document: Option<crate::model::DocumentId>,
    pub(crate) language_server_status: Option<LanguageServerStatus>,
    /// Latest semantic tokens per document; valid only for the stored revision.
    pub(crate) language_semantic_tokens:
        std::collections::HashMap<crate::model::DocumentId, SemanticSnapshot>,
    pub(crate) language_completion: Option<CompletionSession>,
    pub(crate) rename_prompt: Option<RenamePrompt>,
    /// A language-server edit awaiting the user's review.
    pub(crate) language_edit_plan: Option<super::workspace_edit::EditPlan>,
    /// Signature help is shown for the revision it was requested at.
    pub(crate) language_signature: Option<(
        crate::model::DocumentId,
        u64,
        lumen_language_services::incoming::SignatureHelp,
    )>,
    pub(crate) language_server_summary: Option<LanguageServerSummary>,
    pub(crate) language_server_details_open: bool,
    pub(crate) language_hover: Option<String>,
    /// Document and caret byte the hover was requested at; it follows that spot
    /// and disappears when the caret moves away.
    pub(crate) language_hover_anchor: Option<(crate::model::DocumentId, usize)>,
    pub(crate) language_navigation: LanguageNavigationView,
    pub(crate) language_server_diagnostics: std::collections::HashMap<
        crate::model::DocumentId,
        Vec<lumen_language_services::diagnostics::Diagnostic>,
    >,
    pub(crate) sidebar_visible: bool,
    pub(crate) status: String,
    pub(crate) safety_notices: Vec<String>,
    pub(crate) safety_details_open: bool,
    /// Document waiting on the unsaved-changes prompt. A stable ID, so reordering
    /// or closing other tabs cannot retarget the prompt.
    pub(crate) pending_close: Option<crate::model::DocumentId>,
    /// A dirty task editor awaiting Save / Don't Save / Cancel.
    pub(crate) task_close_prompt: Option<super::task_editor::TaskEditorId>,
    pub(crate) bulk_close: crate::app::tab_close::BulkClose,
    /// Recently closed saved documents, newest last, for Reopen Closed Tab.
    pub(crate) closed_tabs: Vec<ClosedTab>,
    pub(crate) window_title: String,
    pub(crate) pending_quit: bool,
    pub(crate) save_conflict: Option<SaveConflict>,
    pub(crate) allow_quit: bool,
    /// When native agents were asked to stop for quitting, and the time after
    /// which the window closes regardless.
    pub(crate) native_quit_deadline: Option<std::time::Instant>,
}
impl ViewState {
    /// A Git dialog or confirmation is open over the workbench.
    pub(crate) fn git_modal_open(&self) -> bool {
        self.clone_dialog.is_some()
            || self.branch_dialog.is_some()
            || self.stash_dialog.is_some()
            || self.history_op_dialog.is_some()
            || self.tags_dialog.is_some()
            || self.source_control.revert_prompt.is_some()
            || self.source_control.abort_prompt
    }

    /// A task-related modal prompt is waiting for an answer.
    pub(crate) fn task_prompt_open(&self) -> bool {
        self.task_close_prompt.is_some()
            || self.task_delete_prompt.is_some()
            || self.task_pending.is_some()
            || self.task_inputs.is_some()
            || self.task_run_close_prompt.is_some()
            || self.task_workspace_switch.is_some()
            || self.task_quit.is_some()
    }

    /// Stop showing a non-document page (Settings or a task editor); the tabs stay open.
    pub(crate) fn leave_page_tabs(&mut self) {
        self.settings.open = false;
        self.history_active = false;
        self.task_editors.deactivate();
        self.chats.deactivate();
    }

    /// Make a task editor the visible page (used when it must be answered).
    pub(crate) fn leave_page_tabs_for_task(&mut self, id: super::task_editor::TaskEditorId) {
        self.settings.open = false;
        self.history_active = false;
        self.chats.deactivate();
        self.task_editors.activate(id);
    }

    /// A non-document page (Settings or a task editor) is what the editor area shows.
    pub(crate) fn page_tab_active(&self) -> bool {
        self.settings.open
            || self.history_active
            || self.task_editors.active().is_some()
            || self.chats.active().is_some()
    }

    pub(crate) fn reveal_edit_directory(&mut self, editor: &crate::model::EditorState) {
        self.activity = Activity::Explorer;
        self.sidebar_visible = true;
        if let (Some(root), Some(edit)) = (&editor.workspace, &self.explorer_edit) {
            for directory in edit
                .directory
                .ancestors()
                .take_while(|path| path.starts_with(root))
            {
                self.expanded_dirs.insert(directory.to_path_buf());
            }
        }
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            clone_dialog: None,
            branch_dialog: None,
            stash_dialog: None,
            history_dialog: None,
            history_active: false,
            history_op_dialog: None,
            tags_dialog: None,
            settings: Default::default(),
            shared_surface: None,
            group_surfaces: Vec::new(),
            hide_virtual_tabs: false,
            task_editors: Default::default(),
            chats: Default::default(),
            native_agents: None,
            tasks: Default::default(),
            task_runs: Default::default(),
            task_pending: None,
            task_inputs: None,
            task_run_close_prompt: None,
            task_workspace_switch: None,
            task_quit: None,
            task_delete_prompt: None,
            tab_strips: Vec::new(),
            tab_drag: None,
            tab_drop_request: None,
            split_vertical: false,
            split_ratio: 0.5,
            shared_output: None,
            shared_paste_selection: None,
            find: Default::default(),
            pending_editor_action: None,
            editor_paste_target: None,
            clipboard_read: None,
            activity: Activity::Explorer,
            expanded_dirs: HashSet::new(),
            explorer_selection: None,
            explorer_edit: None,
            delete_prompt: None,
            explorer_clipboard: None,
            explorer_focused: false,
            focus_editor: false,
            command_palette: false,
            command_palette_focus: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_previous_focus: None,
            navigation: NavigationView::default(),
            pending_goto: None,
            pending_problem_selection: None,
            fold_tabs: Default::default(),
            workspace_search: WorkspaceSearchView::default(),
            source_control: SourceControlView::default(),
            terminal: TerminalView::default(),
            problems: ProblemsView::default(),
            bottom_restore: None,
            output: OutputView::default(),
            agents: Default::default(),
            language_server_document: None,
            language_server_status: None,
            language_semantic_tokens: std::collections::HashMap::new(),
            language_completion: None,
            rename_prompt: None,
            language_edit_plan: None,
            language_signature: None,
            language_server_summary: None,
            language_server_details_open: false,
            language_hover: None,
            language_hover_anchor: None,
            language_navigation: Default::default(),
            language_server_diagnostics: std::collections::HashMap::new(),
            sidebar_visible: true,
            status: "Ready".into(),
            safety_notices: Vec::new(),
            safety_details_open: false,
            pending_close: None,
            task_close_prompt: None,
            bulk_close: Default::default(),
            closed_tabs: Vec::new(),
            window_title: String::new(),
            pending_quit: false,
            save_conflict: None,
            allow_quit: false,
            native_quit_deadline: None,
        }
    }
}
