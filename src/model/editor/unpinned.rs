//! Unpinned (transient) tabs. At most one document per editor group is
//! unpinned; opening another unpinned file replaces it in place. Pinned is the
//! default and the transition is one-way. This is tab metadata, so nothing here
//! touches document content or dirty logic.
use super::{Document, DocumentId, EditorState};

/// An unpinned document and the revision it was opened at, so any later edit
/// (including undo back to the saved text) is noticed as a touch.
pub(crate) struct UnpinnedTab {
    id: DocumentId,
    revision: u64,
}

impl EditorState {
    pub(crate) fn is_unpinned(&self, id: DocumentId) -> bool {
        self.unpinned.iter().any(|tab| tab.id == id)
    }

    /// Make `id` permanent. Pinning a pinned or unknown document is a no-op.
    pub(crate) fn pin(&mut self, id: DocumentId) {
        self.unpinned.retain(|tab| tab.id != id);
    }

    /// Mark an already open document as the unpinned tab of its group. Any other
    /// unpinned tab of that group becomes pinned so the one-per-group rule holds.
    pub(crate) fn mark_unpinned(&mut self, id: DocumentId) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let group = self.group_of(id);
        let others: Vec<_> = self
            .unpinned
            .iter()
            .map(|tab| tab.id)
            .filter(|other| self.group_of(*other) == group)
            .collect();
        for other in others {
            self.pin(other);
        }
        self.unpinned.push(UnpinnedTab {
            id,
            revision: self.files[index].revision,
        });
    }

    /// Pin every unpinned tab that was edited or became dirty, and forget closed
    /// ones. Run each frame and before any replacement, so an edit pins before
    /// the next open can replace the tab.
    pub(crate) fn pin_touched(&mut self) {
        let files = &self.files;
        self.unpinned.retain(|tab| {
            files
                .iter()
                .find(|file| file.id == tab.id)
                .is_some_and(|file| !file.dirty && file.revision == tab.revision)
        });
    }

    /// The unpinned tab of the focused group, if any.
    pub(crate) fn unpinned_in_focused_group(&mut self) -> Option<DocumentId> {
        self.pin_touched();
        let group = self.groups.focused;
        self.unpinned
            .iter()
            .map(|tab| tab.id)
            .find(|id| self.group_of(*id) == group)
    }

    /// Replace the unpinned document `old` by `document` at the same position and
    /// group, activate it and keep it unpinned. False (no change) if `old` is
    /// not an unpinned tab.
    pub(crate) fn replace_unpinned(&mut self, old: DocumentId, document: Document) -> bool {
        if !self.is_unpinned(old) {
            return false;
        }
        let Some(index) = self.index_of(old) else {
            return false;
        };
        self.sync_groups();
        let group = self.group_of(old);
        let new = document.id;
        self.files[index] = document;
        self.groups.assignment.remove(&old);
        self.groups.assignment.insert(new, group);
        for remembered in &mut self.groups.active {
            if *remembered == Some(old) {
                *remembered = Some(new);
            }
        }
        self.pin(old);
        self.set_active_index(Some(index));
        self.mark_unpinned(new);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(name: &str) -> Document {
        Document::from_path(name.into(), "text".into())
    }

    fn editor() -> EditorState {
        let mut editor = EditorState::default();
        for name in ["/a", "/b", "/c"] {
            editor.files.push(opened(name));
        }
        editor
    }

    #[test]
    fn new_tabs_are_pinned() {
        let editor = editor();
        assert!(editor.files.iter().all(|file| !editor.is_unpinned(file.id)));
    }

    #[test]
    fn replacement_keeps_position_and_activates_the_new_tab() {
        let mut editor = editor();
        let old = editor.files[1].id;
        editor.mark_unpinned(old);
        let new = opened("/d");
        let new_id = new.id;
        assert!(editor.replace_unpinned(old, new));
        assert_eq!(editor.files.len(), 3);
        assert_eq!(editor.files[1].id, new_id);
        assert_eq!(editor.active_id(), Some(new_id));
        assert!(editor.is_unpinned(new_id));
        assert!(editor.index_of(old).is_none());
    }

    #[test]
    fn pinned_tabs_are_never_replaced() {
        let mut editor = editor();
        let pinned = editor.files[0].id;
        assert!(!editor.replace_unpinned(pinned, opened("/d")));
        assert_eq!(editor.files.len(), 3);
    }

    #[test]
    fn editing_or_dirtying_pins() {
        let mut editor = editor();
        let (a, b) = (editor.files[0].id, editor.files[1].id);
        editor.mark_unpinned(a);
        editor.files[0].text.push('x');
        editor.files[0].record_text_change();
        editor.pin_touched();
        assert!(!editor.is_unpinned(a));
        editor.mark_unpinned(b);
        editor.files[1].dirty = true;
        assert_eq!(editor.unpinned_in_focused_group(), None);
    }

    #[test]
    fn undoing_back_to_the_saved_text_still_counts_as_a_touch() {
        let mut editor = editor();
        let a = editor.files[0].id;
        editor.mark_unpinned(a);
        editor.files[0].text.push('x');
        editor.files[0].record_text_change();
        editor.files[0].text.pop();
        editor.files[0].record_text_change();
        editor.pin_touched();
        assert!(!editor.is_unpinned(a));
    }

    #[test]
    fn marking_another_unpinned_pins_the_previous_one_in_that_group() {
        let mut editor = editor();
        let (a, b) = (editor.files[0].id, editor.files[1].id);
        editor.mark_unpinned(a);
        editor.mark_unpinned(b);
        assert!(!editor.is_unpinned(a));
        assert!(editor.is_unpinned(b));
    }

    #[test]
    fn closing_the_unpinned_tab_leaves_no_stale_slot() {
        let mut editor = editor();
        let a = editor.files[0].id;
        editor.mark_unpinned(a);
        editor.close_document(a);
        assert_eq!(editor.unpinned_in_focused_group(), None);
        assert!(editor.unpinned.is_empty());
    }

    #[test]
    fn pinning_is_one_way_and_idempotent() {
        let mut editor = editor();
        let a = editor.files[0].id;
        editor.pin(a);
        assert!(!editor.is_unpinned(a));
        editor.mark_unpinned(a);
        editor.pin(a);
        editor.pin(a);
        assert!(!editor.is_unpinned(a));
    }
}
