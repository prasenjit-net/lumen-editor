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
pub(crate) struct SourceControlView {
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) snapshot: Option<crate::services::source_control::GitSnapshot>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
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
    pub(crate) last_refresh: Option<std::time::Instant>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Activity {
    #[default]
    Explorer,
    Search,
    SourceControl,
    RunDebug,
    Extensions,
}

impl Activity {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Explorer => "EXPLORER",
            Self::Search => "SEARCH",
            Self::SourceControl => "SOURCE CONTROL",
            Self::RunDebug => "RUN AND DEBUG",
            Self::Extensions => "EXTENSIONS",
        }
    }
}

pub(crate) struct ViewState {
    pub(crate) settings: super::settings::SettingsState,
    #[cfg(feature = "shared-editor")]
    pub(crate) shared_editor: bool,
    #[cfg(feature = "shared-editor")]
    pub(crate) shared_surface: Option<lumen_editor_surface::EditorSurface>,
    #[cfg(feature = "shared-editor")]
    pub(crate) shared_output: Option<lumen_editor_surface::SurfaceOutput>,
    #[cfg(feature = "shared-editor")]
    pub(crate) shared_paste_selection: Option<lumen_editor_core::SelectionSet>,
    pub(crate) pending_text_input: Option<super::editor_input::PendingTextInput>,
    pub(crate) text_clipboard: super::editor_clipboard::TextClipboard,
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
    pub(crate) output: OutputView,
    pub(crate) agents: super::agent::AgentsView,
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
    pub(crate) language_navigation: LanguageNavigationView,
    pub(crate) language_server_diagnostics: std::collections::HashMap<
        crate::model::DocumentId,
        Vec<lumen_language_services::diagnostics::Diagnostic>,
    >,
    pub(crate) sidebar_visible: bool,
    pub(crate) status: String,
    pub(crate) safety_notices: Vec<String>,
    pub(crate) safety_details_open: bool,
    pub(crate) pending_close: Option<usize>,
    pub(crate) bulk_close: crate::app::tab_close::BulkClose,
    pub(crate) window_title: String,
    pub(crate) pending_quit: bool,
    pub(crate) save_conflict: Option<SaveConflict>,
    pub(crate) allow_quit: bool,
}
impl ViewState {
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
            settings: Default::default(),
            #[cfg(feature = "shared-editor")]
            shared_editor: true,
            #[cfg(feature = "shared-editor")]
            shared_surface: None,
            #[cfg(feature = "shared-editor")]
            shared_output: None,
            #[cfg(feature = "shared-editor")]
            shared_paste_selection: None,
            pending_text_input: None,
            text_clipboard: Default::default(),
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
            language_navigation: Default::default(),
            language_server_diagnostics: std::collections::HashMap::new(),
            sidebar_visible: true,
            status: "Ready".into(),
            safety_notices: Vec::new(),
            safety_details_open: false,
            pending_close: None,
            bulk_close: Default::default(),
            window_title: String::new(),
            pending_quit: false,
            save_conflict: None,
            allow_quit: false,
        }
    }
}
