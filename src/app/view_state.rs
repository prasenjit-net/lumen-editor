use std::{collections::HashSet, path::PathBuf};

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
