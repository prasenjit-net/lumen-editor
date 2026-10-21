use super::DocumentId;
use lumen_editor_core::syntax::Language;
use std::path::{Path, PathBuf};
pub(crate) struct Document {
    pub(crate) id: DocumentId,
    pub(crate) revision: u64,
    pub(crate) path: Option<PathBuf>,
    pub(crate) name: String,
    pub(crate) language: Language,
    pub(crate) text: String,
    pub(crate) dirty: bool,
    pub(crate) disk_changed: bool,
    saved_text: String,
    pub(crate) editing: Option<lumen_editor_core::EditorBuffer>,
}

impl Document {
    pub(crate) fn from_path(path: PathBuf, text: String) -> Self {
        Self {
            id: DocumentId::allocate(),
            revision: 0,
            name: display_name(&path),
            language: Language::from_path(&path),
            path: Some(path),
            saved_text: text.clone(),
            text,
            dirty: false,
            disk_changed: false,
            editing: None,
        }
    }

    pub(crate) fn mark_saved(&mut self, path: PathBuf) {
        self.relocate(path);
        self.saved_text.clone_from(&self.text);
        self.dirty = false;
        self.disk_changed = false;
        if let Some(buffer) = &mut self.editing {
            buffer.mark_saved();
        }
    }

    /// Test helper: account for a direct edit of `text` against the saved contents.
    #[cfg(test)]
    pub(crate) fn record_text_change(&mut self) {
        debug_assert!(
            self.editing.is_none(),
            "Shared documents must synchronize their core, not edit the snapshot"
        );
        self.revision = self.revision.wrapping_add(1);
        self.dirty = self.text != self.saved_text;
    }

    pub(crate) fn relocate(&mut self, path: PathBuf) {
        self.name = display_name(&path);
        self.language = Language::from_path(&path);
        self.path = Some(path);
    }
    pub(crate) fn saved_text(&self) -> &str {
        &self.saved_text
    }

    pub(crate) fn restored(
        path: Option<PathBuf>,
        name: String,
        text: String,
        saved: String,
    ) -> Self {
        let mut document = match path {
            Some(path) => Self::from_path(path, saved),
            None => Self::untitled(0),
        };
        document.name = name;
        document.text = text;
        document.dirty = document.text != document.saved_text;
        document
    }

    pub(crate) fn reload_from_disk(&mut self, text: String) {
        self.text = text.clone();
        self.saved_text = text;
        self.dirty = false;
        self.disk_changed = false;
        self.revision = self.revision.wrapping_add(1);
        {
            self.editing = None;
        }
    }
    pub(crate) fn untitled(number: usize) -> Self {
        Self {
            id: DocumentId::allocate(),
            revision: 0,
            path: None,
            name: format!("untitled-{number}.txt"),
            language: Language::Plain,
            text: String::new(),
            dirty: false,
            disk_changed: false,
            saved_text: String::new(),
            editing: None,
        }
    }

    pub(crate) fn enable_shared_editing(&mut self) {
        if self.editing.is_none() {
            let mut buffer = lumen_editor_core::EditorBuffer::new(self.saved_text.clone());
            if self.text != self.saved_text {
                buffer.replace_document(&self.text).unwrap();
            }
            self.editing = Some(buffer);
        }
    }

    pub(crate) fn sync_shared_editing(&mut self) {
        if let Some(buffer) = &self.editing {
            if self.revision != buffer.revision() {
                self.text.clear();
                self.text.push_str(buffer.text());
            }
            self.revision = buffer.revision();
            self.dirty = buffer.is_dirty();
        }
    }
}

pub(crate) fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .into()
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn exact_filename_language_updates_on_relocation_without_touching_text() {
        let mut document =
            Document::from_path(PathBuf::from("src/CMakeLists.txt"), "project(Lumen)".into());
        assert_eq!(document.language, Language::CMake);
        document.relocate(PathBuf::from("src/notes.txt"));
        assert_eq!(document.language, Language::Plain);
        document.relocate(PathBuf::from("src/Makefile"));
        assert_eq!(document.language, Language::Makefile);
        assert_eq!(document.text, "project(Lumen)");
        assert!(!document.dirty);
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;

    #[test]
    fn unchanged_frames_retain_snapshot_and_edits_undo_and_save_still_sync() {
        let mut document = Document::from_path("large.txt".into(), "line\n".repeat(10_000));
        document.enable_shared_editing();
        let snapshot = document.text.as_ptr();
        document.sync_shared_editing();
        assert_eq!(snapshot, document.text.as_ptr());
        assert!(!document.dirty);
        document
            .editing
            .as_mut()
            .unwrap()
            .replace_document("changed")
            .unwrap();
        document.sync_shared_editing();
        assert_eq!(document.text, "changed");
        assert!(document.dirty);
        document.editing.as_mut().unwrap().undo();
        document.sync_shared_editing();
        assert_eq!(document.text, "line\n".repeat(10_000));
        assert!(!document.dirty);
        document
            .editing
            .as_mut()
            .unwrap()
            .replace_document("saved")
            .unwrap();
        document.sync_shared_editing();
        document.mark_saved("large.txt".into());
        document.sync_shared_editing();
        assert_eq!(document.text, "saved");
        assert!(!document.dirty);
    }
}
