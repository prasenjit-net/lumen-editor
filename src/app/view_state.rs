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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
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
    pub(crate) pending_text_input: Option<super::editor_input::PendingTextInput>,
    pub(crate) text_clipboard: super::editor_clipboard::TextClipboard,
    pub(crate) find: std::collections::HashMap<crate::model::DocumentId, super::find::FindState>,
    pub(crate) pending_editor_action: Option<super::editor_action::PendingEditorAction>,
    pub(crate) editor_paste_target: Option<super::editor_action::PasteTarget>,
    pub(crate) activity: Activity,
    pub(crate) expanded_dirs: HashSet<PathBuf>,
    pub(crate) explorer_selection: Option<ExplorerSelection>,
    pub(crate) explorer_edit: Option<ExplorerEdit>,
    pub(crate) delete_prompt: Option<DeletePrompt>,
    pub(crate) explorer_clipboard: Option<ExplorerClipboard>,
    pub(crate) explorer_focused: bool,
    pub(crate) focus_editor: bool,
    pub(crate) command_palette: bool,
    pub(crate) search: String,
    pub(crate) sidebar_visible: bool,
    pub(crate) status: String,
    pub(crate) pending_close: Option<usize>,
    pub(crate) window_title: String,
    pub(crate) pending_quit: bool,
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
            pending_text_input: None,
            text_clipboard: Default::default(),
            find: Default::default(),
            pending_editor_action: None,
            editor_paste_target: None,
            activity: Activity::Explorer,
            expanded_dirs: HashSet::new(),
            explorer_selection: None,
            explorer_edit: None,
            delete_prompt: None,
            explorer_clipboard: None,
            explorer_focused: false,
            focus_editor: false,
            command_palette: false,
            search: String::new(),
            sidebar_visible: true,
            status: "Ready".into(),
            pending_close: None,
            window_title: String::new(),
            pending_quit: false,
            allow_quit: false,
        }
    }
}
