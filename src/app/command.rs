use super::editor_action::EditorAction;
use super::view_state::{ExplorerEdit, ViewState};
use crate::model::{DocumentId, EditorState};
use std::path::PathBuf;

/// Shared actions from native menus, shortcuts and workbench components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    Find,
    Replace,
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
    CloseEditor,
    Quit,
    Palette,
    ToggleSidebar,
    Welcome,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
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
            Self::NewUntitled => {
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
    pub(crate) fn menu_id(&self) -> Option<&'static str> {
        match self {
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

    pub(crate) fn from_menu_id(id: &str) -> Option<Self> {
        Some(match id {
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
