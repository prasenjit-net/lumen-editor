//! Editor groups (split panes) over the shared, ordered document list.
//!
//! `files` stays the single ordering of documents and `active_file` stays "the
//! document being worked on" - the active tab of the *focused* group - so every
//! caller that only asks for the active document keeps working. A document
//! belongs to exactly one group (v1: one view per document), and a group's tab
//! order is simply the relative order of its documents in `files`.
use super::{DocumentId, EditorState};
use std::collections::HashMap;

pub(crate) const MAX_GROUPS: usize = 2;

#[derive(Default)]
pub(crate) struct GroupState {
    pub(super) assignment: HashMap<DocumentId, usize>,
    /// Remembered active document of each group while it is not focused.
    pub(super) active: [Option<DocumentId>; MAX_GROUPS],
    pub(super) focused: usize,
    pub(super) split: bool,
}

impl EditorState {
    pub(crate) fn group_count(&self) -> usize {
        if self.groups.split { MAX_GROUPS } else { 1 }
    }

    pub(crate) fn focused_group(&self) -> usize {
        self.groups.focused
    }

    /// Group a document is in. A document not seen yet joins the focused group.
    pub(crate) fn group_of(&self, id: DocumentId) -> usize {
        self.groups
            .assignment
            .get(&id)
            .copied()
            .filter(|group| *group < self.group_count())
            .unwrap_or(self.groups.focused)
    }

    /// Documents of `group`, in tab order.
    pub(crate) fn documents_in(&self, group: usize) -> Vec<DocumentId> {
        self.files
            .iter()
            .map(|document| document.id)
            .filter(|id| self.group_of(*id) == group)
            .collect()
    }

    /// The tab showing in `group`: the real active document for the focused
    /// group, the remembered one (or its first tab) for the others.
    #[cfg(test)]
    pub(crate) fn group_active(&self, group: usize) -> Option<DocumentId> {
        if group == self.groups.focused {
            return self.active_id();
        }
        self.groups.active[group.min(MAX_GROUPS - 1)]
            .filter(|id| self.index_of(*id).is_some() && self.group_of(*id) == group)
            .or_else(|| self.documents_in(group).first().copied())
    }

    /// Record every document that has no group yet in the focused one and forget
    /// documents that are gone. Cheap and idempotent.
    pub(crate) fn sync_groups(&mut self) {
        let focused = self.groups.focused;
        for document in &self.files {
            self.groups.assignment.entry(document.id).or_insert(focused);
        }
        let open: std::collections::HashSet<_> =
            self.files.iter().map(|document| document.id).collect();
        self.groups.assignment.retain(|id, _| open.contains(id));
    }

    /// Switch focus to `group`, making its remembered tab the active document.
    pub(crate) fn focus_group(&mut self, group: usize) {
        if group >= self.group_count() || group == self.groups.focused {
            return;
        }
        self.sync_groups();
        let leaving = self.groups.focused;
        self.groups.active[leaving] = self.active_id();
        // Resolve the target before moving focus: `group_active` reads the live
        // active document for whichever group is focused.
        let target = self.fallback_active(group);
        self.groups.focused = group;
        self.active_file = target.and_then(|id| self.index_of(id));
    }

    /// Nearest tab in `group` to the document at `index`: the next one, else the
    /// previous one. Looks only at other documents.
    pub(super) fn neighbour_in_group(&self, index: usize, group: usize) -> Option<DocumentId> {
        let in_group = |document: &&super::Document| self.group_of(document.id) == group;
        self.files[index + 1..]
            .iter()
            .find(in_group)
            .or_else(|| self.files[..index].iter().rev().find(in_group))
            .map(|document| document.id)
    }

    /// An emptied group disappears: the remaining one takes over everything.
    pub(super) fn collapse_empty_groups(&mut self) {
        if !self.groups.split {
            return;
        }
        let first_empty = self.documents_in(0).is_empty();
        let second_empty = self.documents_in(1).is_empty();
        if !first_empty && !second_empty {
            return;
        }
        let survivor = usize::from(first_empty && !second_empty);
        // The survivor's tab: the live active one if it had focus, else the one
        // remembered while it did not. Callers resolve it against what still exists.
        let remembered = if survivor == self.groups.focused {
            self.active_id()
        } else {
            self.groups.active[survivor]
        };
        if survivor == 1 {
            for group in self.groups.assignment.values_mut() {
                *group = 0;
            }
        }
        self.groups.split = false;
        self.groups.focused = 0;
        self.groups.active = [remembered, None];
    }

    /// The tab to show in `group` when `active_file` cannot be trusted (right after
    /// a removal): the remembered tab if it still exists, else the first tab.
    pub(super) fn fallback_active(&self, group: usize) -> Option<DocumentId> {
        self.groups.active[group.min(MAX_GROUPS - 1)]
            .filter(|id| self.index_of(*id).is_some() && self.group_of(*id) == group)
            .or_else(|| self.documents_in(group).first().copied())
    }

    /// Move a document to `target`, opening the second group if needed. A lone
    /// tab cannot start a split (its group would be left empty). The document
    /// becomes the active tab of `target`, which takes focus, and is placed after
    /// that group's last tab.
    pub(crate) fn move_to_group(&mut self, id: DocumentId, target: usize) -> bool {
        self.move_to_group_at(id, target, None)
    }

    /// Like `move_to_group`, but the tab lands at `position` among the
    /// destination's other tabs (`None`, or a position past the end, means last).
    pub(crate) fn move_to_group_at(
        &mut self,
        id: DocumentId,
        target: usize,
        position: Option<usize>,
    ) -> bool {
        if target >= MAX_GROUPS || self.index_of(id).is_none() {
            return false;
        }
        self.sync_groups();
        let from = self.group_of(id);
        if from == target {
            return false;
        }
        if !self.groups.split && self.documents_in(from).len() < 2 {
            return false;
        }
        let index = self.index_of(id).expect("checked above");
        let successor = self.neighbour_in_group(index, from);
        self.groups.split = true;
        self.groups.assignment.insert(id, target);
        self.groups.active[from] = successor;
        let others: Vec<DocumentId> = self
            .documents_in(target)
            .into_iter()
            .filter(|other| *other != id)
            .collect();
        let from = self.index_of(id).expect("document is open");
        match position.and_then(|position| others.get(position)) {
            // Land just before the tab currently at `position`.
            Some(before) => {
                let before = self.index_of(*before).expect("document is open");
                self.reposition(id, if from > before { before } else { before - 1 });
            }
            // Land after the destination's last tab.
            None => {
                if let Some(last) = others.last().and_then(|last| self.index_of(*last)) {
                    // Moving left inserts before the anchor, so step past it.
                    self.reposition(id, if from > last { last + 1 } else { last });
                }
            }
        }
        self.collapse_empty_groups();
        if self.groups.split {
            self.groups.focused = target;
        }
        self.groups.active[self.groups.focused] = Some(id);
        self.active_file = self.index_of(id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(count: usize) -> (EditorState, Vec<DocumentId>) {
        let mut editor = EditorState::default();
        for _ in 0..count {
            editor.new_file();
        }
        let ids = editor.files.iter().map(|file| file.id).collect();
        (editor, ids)
    }

    fn order(editor: &EditorState) -> Vec<DocumentId> {
        editor.files.iter().map(|file| file.id).collect()
    }

    #[test]
    fn a_lone_tab_cannot_start_a_split_but_two_tabs_can() {
        let (mut editor, ids) = editor(1);
        assert!(!editor.move_to_group(ids[0], 1));
        assert_eq!(editor.group_count(), 1);
        let (mut editor, ids) = self::editor(3);
        assert!(editor.move_to_group(ids[1], 1));
        assert_eq!(editor.group_count(), 2);
        assert_eq!(editor.documents_in(0), [ids[0], ids[2]]);
        assert_eq!(editor.documents_in(1), [ids[1]]);
        assert_eq!(editor.focused_group(), 1, "the moved tab takes focus");
        assert_eq!(editor.active_id(), Some(ids[1]));
        assert_eq!(
            editor.group_active(0),
            Some(ids[2]),
            "the left group falls to the neighbour of the tab that left"
        );
    }

    #[test]
    fn activating_a_tab_in_the_other_group_moves_focus_and_remembers_the_old_tab() {
        let (mut editor, ids) = editor(4);
        editor.move_to_group(ids[3], 1);
        editor.activate(ids[2]);
        assert_eq!(editor.focused_group(), 0);
        assert_eq!(editor.active_id(), Some(ids[2]));
        editor.activate(ids[3]);
        assert_eq!(editor.focused_group(), 1);
        editor.activate(ids[0]);
        assert_eq!(editor.focused_group(), 0);
        // The right group still shows the tab it had, and group 0 keeps ids[0].
        assert_eq!(editor.group_active(1), Some(ids[3]));
        editor.focus_group(1);
        assert_eq!(editor.active_id(), Some(ids[3]));
        editor.focus_group(0);
        assert_eq!(
            editor.active_id(),
            Some(ids[0]),
            "focus restores each group's tab"
        );
    }

    #[test]
    fn new_documents_open_in_the_focused_group() {
        let (mut editor, ids) = editor(3);
        editor.move_to_group(ids[2], 1);
        editor.new_file();
        let fresh = editor.files.last().unwrap().id;
        assert_eq!(editor.group_of(fresh), 1);
        editor.focus_group(0);
        editor.new_file();
        let later = editor.files.last().unwrap().id;
        assert_eq!(editor.group_of(later), 0);
    }

    #[test]
    fn closing_the_active_tab_picks_a_neighbour_in_its_own_group() {
        let (mut editor, ids) = editor(5);
        editor.move_to_group(ids[1], 1);
        editor.move_to_group(ids[3], 1);
        // Files order is 0 2 4 | 1 3 (moves append to the destination).
        assert_eq!(editor.documents_in(1), [ids[1], ids[3]]);
        editor.activate(ids[1]);
        editor.close_document(ids[1]);
        assert_eq!(editor.active_id(), Some(ids[3]), "stays in the right group");
        assert_eq!(editor.focused_group(), 1);
        editor.activate(ids[2]);
        editor.close_document(ids[2]);
        assert_eq!(editor.active_id(), Some(ids[4]), "next in the left group");
    }

    #[test]
    fn closing_a_tab_in_the_other_group_keeps_the_active_document() {
        let (mut editor, ids) = editor(4);
        editor.move_to_group(ids[2], 1);
        editor.move_to_group(ids[3], 1);
        editor.activate(ids[0]);
        editor.close_document(ids[2]);
        assert_eq!(editor.active_id(), Some(ids[0]));
        assert_eq!(editor.focused_group(), 0);
        assert_eq!(editor.group_active(1), Some(ids[3]));
    }

    #[test]
    fn an_emptied_group_disappears_and_focus_returns_to_the_survivor() {
        let (mut editor, ids) = editor(3);
        editor.move_to_group(ids[2], 1);
        editor.close_document(ids[2]);
        assert_eq!(editor.group_count(), 1);
        assert_eq!(editor.focused_group(), 0);
        assert!(editor.active_id().is_some());
        // Emptying the *first* group promotes the second.
        let (mut editor, ids) = self::editor(3);
        editor.move_to_group(ids[1], 1);
        editor.close_document(ids[0]);
        editor.close_document(ids[2]);
        assert_eq!(editor.group_count(), 1);
        assert_eq!(order(&editor), [ids[1]]);
        assert_eq!(editor.group_of(ids[1]), 0);
        assert_eq!(editor.active_id(), Some(ids[1]));
    }

    #[test]
    fn moving_the_only_tab_of_a_group_back_merges_the_groups() {
        let (mut editor, ids) = editor(3);
        editor.move_to_group(ids[2], 1);
        assert!(editor.move_to_group(ids[2], 0));
        assert_eq!(editor.group_count(), 1);
        assert_eq!(editor.active_id(), Some(ids[2]));
        assert_eq!(editor.focused_group(), 0);
    }

    #[test]
    fn moving_between_groups_changes_membership_not_content() {
        let (mut editor, ids) = editor(3);
        editor.files[1].dirty = true;
        editor.files[1].text = "unsaved".into();
        editor.move_to_group(ids[1], 1);
        let moved = &editor.files[editor.index_of(ids[1]).unwrap()];
        assert!(moved.dirty);
        assert_eq!(moved.text, "unsaved");
        assert_eq!(editor.files.len(), 3);
        // A move to the group it is already in, or to a bad group, does nothing.
        assert!(!editor.move_to_group(ids[1], 1));
        assert!(!editor.move_to_group(ids[1], 7));
    }

    #[test]
    fn reordering_stays_within_the_relative_order_of_a_group() {
        let (mut editor, ids) = editor(4);
        editor.move_to_group(ids[1], 1);
        editor.move_to_group(ids[3], 1);
        // Group 0: 0 2; group 1: 1 3. Move tab 2 before tab 0 in its group.
        let anchor = editor.index_of(ids[0]).unwrap();
        assert!(editor.move_document(ids[2], anchor));
        assert_eq!(editor.documents_in(0), [ids[2], ids[0]]);
        assert_eq!(editor.documents_in(1), [ids[1], ids[3]]);
    }

    #[test]
    fn a_tab_can_land_at_a_chosen_position_in_the_other_group() {
        for (position, expected) in [
            (Some(0), [3, 1, 2]),
            (Some(1), [1, 3, 2]),
            (Some(2), [1, 2, 3]),
            (Some(9), [1, 2, 3]),
            (None, [1, 2, 3]),
        ] {
            let (mut editor, ids) = editor(5);
            // Left group: 0 4; right group: 1 2.
            editor.move_to_group(ids[1], 1);
            editor.move_to_group(ids[2], 1);
            assert!(editor.move_to_group_at(ids[3], 1, position));
            assert_eq!(
                editor.documents_in(1),
                expected.map(|n| ids[n]),
                "position {position:?}"
            );
            assert_eq!(editor.documents_in(0), [ids[0], ids[4]]);
            assert_eq!(editor.active_id(), Some(ids[3]));
        }
    }

    #[test]
    fn dropping_at_a_position_can_also_open_the_second_group() {
        let (mut editor, ids) = editor(3);
        assert!(editor.move_to_group_at(ids[2], 1, Some(0)));
        assert_eq!(editor.group_count(), 2);
        assert_eq!(editor.documents_in(1), [ids[2]]);
        // Moving into the group it is already in changes nothing.
        assert!(!editor.move_to_group_at(ids[2], 1, Some(0)));
    }
}
