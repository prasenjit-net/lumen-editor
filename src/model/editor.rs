use super::Document;
use std::path::PathBuf;
pub(crate) struct EditorState {
    pub(crate) files: Vec<Document>,
    pub(crate) active_file: Option<usize>,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) next_untitled: usize,
}
impl Default for EditorState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            active_file: None,
            workspace: None,
            next_untitled: 1,
        }
    }
}
impl EditorState {
    pub(crate) fn active(&self) -> Option<&Document> {
        self.active_file.and_then(|index| self.files.get(index))
    }
    pub(crate) fn new_file(&mut self) {
        self.files.push(Document::untitled(self.next_untitled));
        self.next_untitled += 1;
        self.active_file = Some(self.files.len() - 1);
    }
    pub(crate) fn close_file(&mut self, index: usize) {
        if index >= self.files.len() {
            return;
        }
        self.files.remove(index);
        self.active_file = match self.files.len() {
            0 => None,
            count => Some(index.min(count - 1)),
        };
    }
    pub(crate) fn has_unsaved_changes(&self) -> bool {
        self.files.iter().any(|file| file.dirty)
    }

    pub(crate) fn detach_deleted(&mut self, path: &std::path::Path) {
        for index in (0..self.files.len()).rev() {
            if !self.files[index]
                .path
                .as_ref()
                .is_some_and(|file| file.starts_with(path))
            {
                continue;
            }
            if self.files[index].dirty {
                self.files[index].path = None;
            } else {
                self.files.remove(index);
                self.active_file = self.active_file.and_then(|active| {
                    if self.files.is_empty() {
                        None
                    } else if active > index {
                        Some(active - 1)
                    } else {
                        Some(active.min(self.files.len() - 1))
                    }
                });
            }
        }
    }
}
