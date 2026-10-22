use super::{Document, DocumentId};
mod groups;
mod unpinned;
pub(crate) use groups::GroupState;
use std::path::PathBuf;
use unpinned::UnpinnedTab;
pub(crate) struct EditorState {
    pub(crate) files: Vec<Document>,
    /// Index into `files`. Private on purpose: callers ask for the active
    /// document through the accessors below, so what "active" means can change
    /// (for example with several editor groups) without touching every caller.
    active_file: Option<usize>,
    groups: GroupState,
    unpinned: Vec<UnpinnedTab>,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) next_untitled: usize,
}
impl Default for EditorState {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            active_file: None,
            groups: GroupState::default(),
            unpinned: Vec::new(),
            workspace: None,
            next_untitled: 1,
        }
    }
}
impl EditorState {
    #[cfg(test)]
    pub(crate) fn with_workspace(workspace: PathBuf) -> Self {
        Self {
            workspace: Some(workspace),
            ..Self::default()
        }
    }
    pub(crate) fn active(&self) -> Option<&Document> {
        self.active_file.and_then(|index| self.files.get(index))
    }
    pub(crate) fn active_index(&self) -> Option<usize> {
        self.active_file.filter(|index| *index < self.files.len())
    }
    pub(crate) fn active_id(&self) -> Option<DocumentId> {
        self.active().map(|document| document.id)
    }
    pub(crate) fn index_of(&self, id: DocumentId) -> Option<usize> {
        self.files.iter().position(|document| document.id == id)
    }
    /// Make `id` the active document; false (and no change) if it is not open.
    pub(crate) fn activate(&mut self, id: DocumentId) -> bool {
        match self.index_of(id) {
            Some(index) => {
                self.set_active_index(Some(index));
                true
            }
            None => false,
        }
    }
    /// Set the active tab by position. An index past the end clears the active
    /// document instead of pointing at nothing.
    pub(crate) fn set_active_index(&mut self, index: Option<usize>) {
        let index = index.filter(|index| *index < self.files.len());
        if let Some(index) = index {
            // Activating a tab in the other group moves focus there first, so
            // the remembered active tab of the group being left is kept.
            self.sync_groups();
            let group = self.group_of(self.files[index].id);
            self.focus_group(group);
        }
        self.active_file = index;
    }
    pub(crate) fn new_file(&mut self) {
        self.files.push(Document::untitled(self.next_untitled));
        self.next_untitled += 1;
        self.set_active_index(Some(self.files.len() - 1));
    }
    /// Close the document at `index`. Closing the active tab activates its
    /// neighbour in the same group; closing any other tab keeps the *same
    /// document* active. A group left empty disappears.
    pub(crate) fn close_file(&mut self, index: usize) {
        if index >= self.files.len() {
            return;
        }
        self.sync_groups();
        let id = self.files[index].id;
        let group = self.group_of(id);
        let neighbour = self.neighbour_in_group(index, group);
        let chosen = if group == self.groups.focused
            && (self.active_file.is_none() || self.active_id() == Some(id))
        {
            neighbour
        } else {
            self.active_id()
        };
        if self.groups.active[group] == Some(id) {
            self.groups.active[group] = neighbour;
        }
        self.files.remove(index);
        self.groups.assignment.remove(&id);
        self.collapse_empty_groups();
        let focused = self.groups.focused;
        self.active_file = chosen
            .filter(|chosen| self.index_of(*chosen).is_some())
            .or_else(|| self.fallback_active(focused))
            .and_then(|chosen| self.index_of(chosen));
        // A preview cannot outlive the document it shows.
        let previews: Vec<DocumentId> = self
            .files
            .iter()
            .filter(|file| file.view_of == Some(id))
            .map(|file| file.id)
            .collect();
        for preview in previews {
            self.close_document(preview);
        }
    }
    /// Move a document so it lands at `to`, an index into `files` as it is
    /// *before* the move. Content, dirty flags and the active document are
    /// untouched; only the order changes. Returns whether anything moved.
    pub(crate) fn move_document(&mut self, id: DocumentId, to: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let to = to.min(self.files.len() - 1);
        if from == to {
            return false;
        }
        let active = self.active_id();
        self.reposition(id, to);
        self.active_file = active.and_then(|active| self.index_of(active));
        true
    }

    /// Reorder only: move `id` to `to` (an index before the move).
    fn reposition(&mut self, id: DocumentId, to: usize) {
        let Some(from) = self.index_of(id) else {
            return;
        };
        let to = to.min(self.files.len() - 1);
        let document = self.files.remove(from);
        self.files.insert(to, document);
    }

    pub(crate) fn close_document(&mut self, id: DocumentId) {
        if let Some(index) = self.index_of(id) {
            self.close_file(index);
        }
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
                self.close_file(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(count: usize) -> EditorState {
        let mut editor = EditorState::default();
        for _ in 0..count {
            editor.new_file();
        }
        editor
    }

    fn ids(editor: &EditorState) -> Vec<DocumentId> {
        editor.files.iter().map(|file| file.id).collect()
    }

    #[test]
    fn closing_another_tab_keeps_the_same_document_active() {
        for closed in [0, 1, 3] {
            let mut editor = editor(4);
            editor.set_active_index(Some(2));
            let active = editor.active_id();
            let victim = editor.files[closed].id;
            editor.close_document(victim);
            assert_eq!(editor.active_id(), active, "closing tab {closed}");
            assert_eq!(editor.files.len(), 3);
        }
    }

    #[test]
    fn closing_the_active_tab_activates_its_neighbour() {
        let mut editor = editor(3);
        let ids = ids(&editor);
        editor.activate(ids[1]);
        editor.close_document(ids[1]);
        assert_eq!(editor.active_id(), Some(ids[2]));
        editor.close_document(ids[2]);
        assert_eq!(
            editor.active_id(),
            Some(ids[0]),
            "falls back to the last tab"
        );
        editor.close_document(ids[0]);
        assert_eq!(editor.active_id(), None);
        assert_eq!(editor.active_index(), None);
    }

    #[test]
    fn activation_and_lookup_use_identity_not_position() {
        let mut editor = editor(3);
        let ids = ids(&editor);
        assert!(editor.activate(ids[0]));
        // Shuffling tabs (as reordering will) leaves identity intact.
        editor.files.swap(0, 2);
        assert_eq!(editor.index_of(ids[0]), Some(2));
        editor.set_active_index(editor.index_of(ids[0]));
        assert_eq!(editor.active_id(), Some(ids[0]));
        let mut other = EditorState::default();
        other.new_file();
        let stranger = other.files[0].id;
        assert!(!editor.activate(stranger));
        assert_eq!(
            editor.active_id(),
            Some(ids[0]),
            "unknown id changes nothing"
        );
        editor.close_document(stranger);
        assert_eq!(editor.files.len(), 3, "closing an unknown id is a no-op");
    }

    #[test]
    fn an_out_of_range_active_index_clears_instead_of_dangling() {
        let mut editor = editor(2);
        editor.set_active_index(Some(5));
        assert_eq!(editor.active_index(), None);
        assert!(editor.active().is_none());
    }

    #[test]
    fn moving_a_document_changes_only_the_order() {
        let mut editor = editor(4);
        let ids = ids(&editor);
        editor.files[1].dirty = true;
        editor.files[1].text = "edited".into();
        editor.activate(ids[2]);
        // Right: lands after the document that was at the target index.
        assert!(editor.move_document(ids[0], 2));
        assert_eq!(super::tests::ids(&editor), [ids[1], ids[2], ids[0], ids[3]]);
        // Left.
        assert!(editor.move_document(ids[3], 0));
        assert_eq!(super::tests::ids(&editor), [ids[3], ids[1], ids[2], ids[0]]);
        assert_eq!(
            editor.active_id(),
            Some(ids[2]),
            "active document follows its tab"
        );
        let moved = editor.index_of(ids[1]).unwrap();
        assert!(editor.files[moved].dirty);
        assert_eq!(editor.files[moved].text, "edited");
    }

    #[test]
    fn moving_to_the_same_place_an_unknown_id_or_past_the_end_is_safe() {
        let mut editor = editor(3);
        let ids = ids(&editor);
        assert!(!editor.move_document(ids[1], 1), "already there");
        let mut other = EditorState::default();
        other.new_file();
        assert!(!editor.move_document(other.files[0].id, 0), "unknown id");
        assert!(
            editor.move_document(ids[0], 99),
            "clamps to the last position"
        );
        assert_eq!(super::tests::ids(&editor), [ids[1], ids[2], ids[0]]);
        let mut empty = EditorState::default();
        assert!(!empty.move_document(ids[0], 0));
    }
}
