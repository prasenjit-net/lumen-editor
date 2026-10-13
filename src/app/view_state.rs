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
    pub(crate) navigation: NavigationView,
    pub(crate) pending_goto: Option<(crate::model::DocumentId, usize)>,
    pub(crate) search: String,
    pub(crate) workspace_search: WorkspaceSearchView,
    pub(crate) sidebar_visible: bool,
    pub(crate) status: String,
    pub(crate) safety_notices: Vec<String>,
    pub(crate) safety_details_open: bool,
    pub(crate) pending_close: Option<usize>,
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
            navigation: NavigationView::default(),
            pending_goto: None,
            search: String::new(),
            workspace_search: WorkspaceSearchView::default(),
            sidebar_visible: true,
            status: "Ready".into(),
            safety_notices: Vec::new(),
            safety_details_open: false,
            pending_close: None,
            window_title: String::new(),
            pending_quit: false,
            save_conflict: None,
            allow_quit: false,
        }
    }
}
