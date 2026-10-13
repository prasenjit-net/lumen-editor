use super::{
    LumenApp,
    view_state::{
        Activity, DeletePrompt, ExplorerClipboard, ExplorerEdit, ExplorerEditKind,
        ExplorerSelection,
    },
};
use crate::model::{Document, display_name, validate_file_name, validate_relative_name};
use std::path::{Component, Path, PathBuf};

impl LumenApp {
    pub(crate) fn explorer_target_directory(&self) -> Option<PathBuf> {
        let root = self.editor.workspace.as_ref()?;
        Some(
            self.view
                .explorer_selection
                .as_ref()
                .and_then(|selection| selection.directory())
                .filter(|directory| directory.starts_with(root))
                .unwrap_or(root)
                .to_path_buf(),
        )
    }

    fn mutable_explorer_path(&self, path: &Path) -> bool {
        self.editor
            .workspace
            .as_ref()
            .is_some_and(|root| path != root && path.starts_with(root))
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
    }

    pub(crate) fn new_folder(&mut self) {
        let Some(directory) = self.explorer_target_directory() else {
            return;
        };
        let mut edit = ExplorerEdit::new(directory);
        edit.kind = ExplorerEditKind::NewFolder;
        self.view.explorer_edit = Some(edit);
        self.view.reveal_edit_directory(&self.editor);
    }

    pub(crate) fn rename_path(&mut self, path: PathBuf) {
        if !self.mutable_explorer_path(&path) {
            return;
        }
        let Some(parent) = path.parent() else {
            return;
        };
        let mut edit = ExplorerEdit::new(parent.to_path_buf());
        edit.name = display_name(&path);
        edit.kind = ExplorerEditKind::Rename(path);
        self.view.explorer_edit = Some(edit);
        self.view.reveal_edit_directory(&self.editor);
    }

    pub(crate) fn commit_explorer_edit(&mut self) {
        let Some(edit) = &mut self.view.explorer_edit else {
            return;
        };
        let relative = match &edit.kind {
            ExplorerEditKind::Rename(_) => validate_file_name(&edit.name).map(PathBuf::from),
            _ => validate_relative_name(&edit.name),
        };
        let relative = match relative {
            Ok(name) => name,
            Err(error) => {
                edit.error = Some(error.into());
                return;
            }
        };
        let target = edit.directory.join(&relative);
        let kind = if matches!(edit.kind, ExplorerEditKind::NewFile)
            && edit.name.trim().ends_with(['/', '\\'])
        {
            ExplorerEditKind::NewFolder
        } else {
            edit.kind.clone()
        };
        if !matches!(kind, ExplorerEditKind::Rename(_))
            && let Some(parent) = relative.parent()
        {
            let mut directory = edit.directory.clone();
            for component in parent.components() {
                directory.push(component);
                if self.file_system.is_directory(&directory) {
                    continue;
                }
                if let Err(error) = self.file_system.create_directory(&directory) {
                    if error.kind() == std::io::ErrorKind::AlreadyExists
                        && self.file_system.is_directory(&directory)
                    {
                        continue;
                    }
                    edit.error = Some(format!("Could not create parent folder: {error}"));
                    return;
                }
            }
        }
        let result = match &kind {
            ExplorerEditKind::NewFile => self.file_system.create_file(&target),
            ExplorerEditKind::NewFolder => self.file_system.create_directory(&target),
            ExplorerEditKind::Rename(source) if *source == target => Ok(()),
            ExplorerEditKind::Rename(source) => self.file_system.rename_entry(source, &target),
        };
        if let Err(error) = result {
            edit.error = Some(if error.kind() == std::io::ErrorKind::AlreadyExists {
                "A file or folder with that name already exists.".into()
            } else {
                format!("Could not complete operation: {error}")
            });
            return;
        }
        match kind {
            ExplorerEditKind::NewFile => {
                self.view.navigation.add_workspace_path(&target);
                self.remember_recent_file(&target);
                let index = self
                    .editor
                    .files
                    .iter()
                    .position(|file| file.path.as_ref() == Some(&target))
                    .unwrap_or_else(|| {
                        self.editor
                            .files
                            .push(Document::from_path(target.clone(), String::new()));
                        self.editor.files.len() - 1
                    });
                self.editor.active_file = Some(index);
                self.view.focus_editor = true;
                self.view.explorer_focused = false;
            }
            ExplorerEditKind::Rename(source) => self.relocate_open_paths(&source, &target),
            ExplorerEditKind::NewFolder => {}
        }
        self.select_and_reveal(&target);
        self.view.explorer_edit = None;
        self.view.status = format!("Ready: {}", display_name(&target));
    }

    fn select_and_reveal(&mut self, path: &Path) {
        let directory = self.file_system.is_directory(path);
        self.view.explorer_selection = Some(if directory {
            ExplorerSelection::Directory(path.into())
        } else {
            ExplorerSelection::File(path.into())
        });
        if let Some(root) = &self.editor.workspace {
            for ancestor in path
                .parent()
                .into_iter()
                .flat_map(|parent| parent.ancestors())
                .take_while(|ancestor| ancestor.starts_with(root))
            {
                self.view.expanded_dirs.insert(ancestor.into());
            }
        }
        self.view.activity = Activity::Explorer;
        self.view.sidebar_visible = true;
    }

    fn relocate_open_paths(&mut self, source: &Path, target: &Path) {
        self.view.navigation.relocate(source, target);
        self.save_recent_files();
        for file in &mut self.editor.files {
            if let Some(suffix) = file
                .path
                .as_ref()
                .and_then(|path| path.strip_prefix(source).ok())
            {
                file.relocate(target.join(suffix));
            }
        }
        self.view.expanded_dirs = self
            .view
            .expanded_dirs
            .drain()
            .map(|path| {
                path.strip_prefix(source)
                    .map(|suffix| target.join(suffix))
                    .unwrap_or(path)
            })
            .collect();
        if let Some(clipboard) = &mut self.view.explorer_clipboard
            && let Ok(suffix) = clipboard.path.strip_prefix(source)
        {
            clipboard.path = target.join(suffix);
        }
    }

    pub(crate) fn request_delete(&mut self, path: PathBuf) {
        if self.mutable_explorer_path(&path) {
            self.view.delete_prompt = Some(DeletePrompt { path, error: None });
        }
    }

    pub(crate) fn confirm_delete(&mut self) {
        let Some(prompt) = &self.view.delete_prompt else {
            return;
        };
        let path = prompt.path.clone();
        if !self.mutable_explorer_path(&path) {
            self.view.delete_prompt = None;
            return;
        }
        if let Err(error) = self.file_system.trash_entry(&path) {
            self.view.delete_prompt.as_mut().unwrap().error =
                Some(format!("Could not move to Trash/Recycle Bin: {error}"));
            return;
        }
        self.editor.detach_deleted(&path);
        self.view.navigation.forget_deleted(&path);
        self.save_recent_files();
        self.view
            .expanded_dirs
            .retain(|directory| !directory.starts_with(&path));
        if self
            .view
            .explorer_clipboard
            .as_ref()
            .is_some_and(|clipboard| clipboard.path.starts_with(&path))
        {
            self.view.explorer_clipboard = None;
        }
        self.view.explorer_selection = path
            .parent()
            .map(|parent| ExplorerSelection::Directory(parent.into()));
        self.view.delete_prompt = None;
        self.view.status = format!("Moved {} to Trash/Recycle Bin", display_name(&path));
    }

    pub(crate) fn explorer_copy(&mut self, path: PathBuf, cut: bool) {
        if self.mutable_explorer_path(&path) {
            self.view.explorer_clipboard = Some(ExplorerClipboard { path, cut });
            self.view.status = if cut {
                "Cut: choose a folder and Paste"
            } else {
                "Copied: choose a folder and Paste"
            }
            .into();
        }
    }

    pub(crate) fn explorer_paste(&mut self) {
        let Some(clipboard) = self.view.explorer_clipboard.clone() else {
            return;
        };
        let Some(directory) = self.explorer_target_directory() else {
            return;
        };
        if !self.mutable_explorer_path(&clipboard.path) {
            return;
        }
        if self.file_system.is_directory(&clipboard.path) && directory.starts_with(&clipboard.path)
        {
            self.view.status = "Cannot paste a folder inside itself.".into();
            return;
        }
        let mut target = directory.join(display_name(&clipboard.path));
        if clipboard.cut && target == clipboard.path {
            return;
        }
        if self.file_system.entry_exists(&target) {
            if clipboard.cut {
                self.view.status = "Destination already exists; nothing was moved.".into();
                return;
            }
            let name = display_name(&clipboard.path);
            let (stem, extension) = if self.file_system.is_directory(&clipboard.path) {
                (name.clone(), String::new())
            } else {
                let stem = clipboard
                    .path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(&name)
                    .to_owned();
                let extension = clipboard
                    .path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| format!(".{ext}"))
                    .unwrap_or_default();
                (stem, extension)
            };
            for index in 1.. {
                let suffix = if index == 1 {
                    " copy".into()
                } else {
                    format!(" copy {index}")
                };
                target = directory.join(format!("{stem}{suffix}{extension}"));
                if !self.file_system.entry_exists(&target) {
                    break;
                }
            }
        }
        let result = if clipboard.cut {
            self.file_system.rename_entry(&clipboard.path, &target)
        } else {
            self.file_system.copy_entry(&clipboard.path, &target)
        };
        match result {
            Ok(()) => {
                if clipboard.cut {
                    self.relocate_open_paths(&clipboard.path, &target);
                    self.view.explorer_clipboard = None;
                }
                self.select_and_reveal(&target);
                self.view.status = format!("Pasted {}", display_name(&target));
            }
            Err(error) => {
                self.view.status = format!(
                    "Paste failed: {error}. A failed copy may leave a partial destination at {}.",
                    target.display()
                )
            }
        }
    }
}
