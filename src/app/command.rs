use super::editor_action::EditorAction;
use super::view_state::{Activity, ExplorerEdit, ViewState};
use super::workspace_search::ReplaceScope;
use crate::model::{DocumentId, EditorState};
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
    PreviewWorkspaceReplace,
    PreviewWorkspaceReplaceScope(ReplaceScope),
    ApplyWorkspaceReplace,
    OpenWorkspaceHit(PathBuf, Range<usize>),
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
    RequestClose(usize),
    CloseFile(usize),
    SaveAndClose(usize),
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
                    editor.new_file();
                    view.focus_editor = true;
                    view.status = "New untitled file".into();
                }
            }
            Self::RequestClose(index) => {
                if editor.files.get(index).is_some_and(|file| file.dirty) {
                    view.pending_close = Some(index);
                } else {
                    Self::CloseFile(index).apply_local(editor, view);
                }
            }
            Self::CloseFile(index) => {
                if index < editor.files.len() {
                    view.find.remove(&editor.files[index].id);
                    editor.close_file(index);
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
            Self::Save => Some("save"),
            Self::SaveAs => Some("save_as"),
            Self::CloseEditor => Some("close"),
            Self::Quit => Some("quit"),
            Self::Palette => Some("palette"),
            Self::ToggleSidebar => Some("sidebar"),
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
            "save" => Self::Save,
            "save_as" => Self::SaveAs,
            "close" => Self::CloseEditor,
            "quit" => Self::Quit,
            "palette" => Self::Palette,
            "sidebar" => Self::ToggleSidebar,
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
