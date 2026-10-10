use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

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

pub(crate) struct NewFilePrompt {
    pub(crate) directory: PathBuf,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
    pub(crate) focus_name: bool,
}

impl NewFilePrompt {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            name: String::new(),
            error: None,
            focus_name: true,
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
    pub(crate) activity: Activity,
    pub(crate) expanded_dirs: HashSet<PathBuf>,
    pub(crate) explorer_selection: Option<ExplorerSelection>,
    pub(crate) new_file_prompt: Option<NewFilePrompt>,
    pub(crate) command_palette: bool,
    pub(crate) search: String,
    pub(crate) sidebar_visible: bool,
    pub(crate) status: String,
    pub(crate) pending_close: Option<usize>,
    pub(crate) window_title: String,
    pub(crate) pending_quit: bool,
    pub(crate) allow_quit: bool,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            activity: Activity::Explorer,
            expanded_dirs: HashSet::new(),
            explorer_selection: None,
            new_file_prompt: None,
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
