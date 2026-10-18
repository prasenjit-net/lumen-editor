//! Tab navigation and recovery commands: next/previous, reopen, and the
//! active-tab-relative close commands the palette and shortcuts use.
use super::{LumenApp, tab_close::TabCloseScope, view_state::ClosedTab};
use crate::model::DocumentId;

/// Where to put a tab. Positions count visible tabs only: the document behind a
/// diff view has no tab and is never a destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabMove {
    Left,
    Right,
    /// The tab's position among the visible tabs after the move.
    To(usize),
}

impl LumenApp {
    /// Reorder one tab. Does not activate it, change content, or touch dirty flags.
    pub(crate) fn move_tab(&mut self, id: DocumentId, movement: TabMove) {
        // A tab in the other group is reordered within its own group.
        if self.editor.index_of(id).is_some() {
            let group = self.editor.group_of(id);
            self.editor.focus_group(group);
        }
        let tabs = self.visible_documents();
        let Some(from) = tabs.iter().position(|tab| *tab == id) else {
            return;
        };
        let target = match movement {
            TabMove::Left => from.checked_sub(1),
            TabMove::Right => (from + 1 < tabs.len()).then_some(from + 1),
            TabMove::To(position) => Some(position.min(tabs.len() - 1)),
        };
        let Some(target) = target.filter(|target| *target != from) else {
            return;
        };
        // The destination is expressed through the tab currently there, so a
        // hidden diff-only document in between cannot shift the result.
        let Some(anchor) = self.editor.index_of(tabs[target]) else {
            return;
        };
        if self.editor.move_document(id, anchor) {
            self.view.status = "Moved tab".into();
        }
    }

    /// Whether the user has turned on split editing.
    pub(crate) fn split_enabled(&self) -> bool {
        self.view.settings.boolean("workbench.split_editor")
    }

    pub(crate) fn split_editor(&mut self) {
        match self.editor.active_id() {
            Some(id) => self.move_tab_to_group(id),
            None => self.view.status = "Open a tab to split the editor".into(),
        }
    }

    /// Send `id` to the other group, opening the second group if there is only
    /// one. A group is never left empty by opening a split, and nothing is
    /// duplicated: each file lives in one group.
    pub(crate) fn move_tab_to_group(&mut self, id: DocumentId) {
        if !self.split_enabled() {
            self.view.status = "Turn on Split Editor in Settings first".into();
            return;
        }
        let target = if self.editor.group_count() > 1 {
            1 - self.editor.group_of(id)
        } else {
            1
        };
        if self.editor.move_to_group(id, target) {
            self.view.focus_editor = true;
            self.view.status = "Moved tab to the other editor group".into();
        } else {
            self.view.status = "Open another tab before splitting the editor".into();
        }
    }

    /// A tab dropped onto a group: it joins that group at `position`.
    pub(crate) fn move_tab_to_group_at(&mut self, id: DocumentId, group: usize, position: usize) {
        if !self.split_enabled() {
            self.view.status = "Turn on Split Editor in Settings first".into();
            return;
        }
        if self.editor.move_to_group_at(id, group, Some(position)) {
            self.view.focus_editor = true;
            self.view.status = "Moved tab to the other editor group".into();
        }
    }

    pub(crate) fn focus_next_group(&mut self) {
        let count = self.editor.group_count();
        if count < 2 {
            self.view.status = "The editor is not split".into();
            return;
        }
        let next = (self.editor.focused_group() + 1) % count;
        self.focus_group_command(next);
    }

    pub(crate) fn focus_group_command(&mut self, group: usize) {
        if group >= self.editor.group_count() {
            self.view.status = "The editor is not split".into();
            return;
        }
        self.editor.focus_group(group);
        self.view.focus_editor = true;
    }

    pub(crate) fn move_active_tab(&mut self, movement: TabMove) {
        match self.editor.active_id() {
            Some(id) => self.move_tab(id, movement),
            None => self.view.status = "No tab is active".into(),
        }
    }

    /// Make `id` the visible tab, leaving Settings or a diff view if one was showing.
    pub(crate) fn show_document_tab(&mut self, id: DocumentId) {
        if !self.editor.activate(id) {
            return;
        }
        self.view.leave_page_tabs();
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
        }
        self.view.focus_editor = true;
    }

    /// Move to the next (`step` > 0) or previous tab in strip order, wrapping.
    /// With nothing active the first (or last) tab is chosen.
    pub(crate) fn cycle_tab(&mut self, step: isize) {
        let tabs = self.visible_documents();
        if tabs.is_empty() {
            self.view.status = "No tabs are open".into();
            return;
        }
        let current = self
            .editor
            .active_id()
            .and_then(|id| tabs.iter().position(|tab| *tab == id));
        let target = match current {
            Some(position) => (position as isize + step).rem_euclid(tabs.len() as isize) as usize,
            None if step >= 0 => 0,
            None => tabs.len() - 1,
        };
        self.show_document_tab(tabs[target]);
    }

    /// Reopen the most recently closed saved document at its old caret. Entries
    /// already open again are skipped; a file that has since gone is reported by
    /// the ordinary open path and dropped.
    pub(crate) fn reopen_closed_tab(&mut self) {
        while let Some(ClosedTab { path, caret }) = self.view.closed_tabs.pop() {
            if self
                .editor
                .files
                .iter()
                .any(|file| file.path.as_ref() == Some(&path))
            {
                continue;
            }
            let before = self.editor.files.len();
            self.open_file(path);
            if self.editor.files.len() > before
                && let Some(document) = self.editor.active()
            {
                let mut byte = caret.min(document.text.len());
                while !document.text.is_char_boundary(byte) {
                    byte -= 1;
                }
                self.view.pending_goto = Some((document.id, byte));
            }
            return;
        }
        self.view.status = "No recently closed tabs".into();
    }

    /// Close tabs relative to the active one. Does nothing without an active tab.
    pub(crate) fn close_tabs_relative(&mut self, scope: fn(DocumentId) -> TabCloseScope) {
        match self.editor.active_id() {
            Some(id) => self.begin_bulk_close(scope(id)),
            None => self.view.status = "No tab is active".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::view_state::MAX_CLOSED_TABS, model::Document};
    use std::path::PathBuf;

    fn saved_doc(path: &str) -> Document {
        Document::from_path(PathBuf::from(path), "text".into())
    }

    fn app_with_saved(paths: &[&str]) -> LumenApp {
        let mut app = LumenApp::default();
        for path in paths {
            app.editor.files.push(saved_doc(path));
        }
        app.editor.set_active_index(Some(0));
        app
    }

    fn ids(app: &LumenApp) -> Vec<DocumentId> {
        app.editor.files.iter().map(|f| f.id).collect()
    }

    #[test]
    fn closing_a_saved_tab_records_it_but_dirty_and_untitled_tabs_are_not_resurrectable() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs"]);
        app.editor.files[1].dirty = true;
        app.editor.new_file();
        let ids = ids(&app);
        app.request_close(ids[0]);
        assert_eq!(app.view.closed_tabs.len(), 1);
        assert_eq!(app.view.closed_tabs[0].path, PathBuf::from("/p/a.rs"));
        // Discarding unsaved edits via "Don't Save" must not make the discarded
        // text recoverable; the untitled buffer has no path to reopen.
        app.close_file(ids[1]);
        app.close_file(ids[2]);
        assert_eq!(app.view.closed_tabs.len(), 1);
    }

    #[test]
    fn the_closed_tab_list_is_capped_deduplicated_and_newest_last() {
        let paths: Vec<String> = (0..MAX_CLOSED_TABS + 5)
            .map(|n| format!("/p/{n}.rs"))
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut app = app_with_saved(&refs);
        for id in ids(&app) {
            app.close_file(id);
        }
        assert_eq!(app.view.closed_tabs.len(), MAX_CLOSED_TABS);
        assert_eq!(
            app.view.closed_tabs.last().unwrap().path,
            PathBuf::from(format!("/p/{}.rs", MAX_CLOSED_TABS + 4))
        );
        // Closing the same path again moves it to the end instead of duplicating.
        let mut again = app_with_saved(&["/p/x.rs", "/p/y.rs"]);
        let first = again.editor.files[0].id;
        again.close_file(first);
        again.editor.files.push(saved_doc("/p/x.rs"));
        let reopened = again.editor.files.last().unwrap().id;
        again.close_file(reopened);
        assert_eq!(
            again
                .view
                .closed_tabs
                .iter()
                .filter(|closed| closed.path.as_path() == std::path::Path::new("/p/x.rs"))
                .count(),
            1
        );
    }

    #[test]
    fn reopening_restores_the_file_at_its_caret_and_skips_entries_already_open() {
        let root = std::env::temp_dir().join(format!("lumen-reopen-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (a, b) = (root.join("a.txt"), root.join("b.txt"));
        std::fs::write(&a, "héllo world").unwrap();
        std::fs::write(&b, "second").unwrap();
        let mut app = LumenApp::default();
        app.view.closed_tabs = vec![
            ClosedTab {
                path: a.clone(),
                caret: 2,
            },
            ClosedTab {
                path: b.clone(),
                caret: 3,
            },
        ];
        app.open_file(b.clone());
        // b is already open, so it is skipped and a comes back.
        app.reopen_closed_tab();
        let document = app.editor.active().unwrap();
        assert_eq!(document.path.as_deref(), Some(a.as_path()));
        // Byte 2 is inside the two-byte `é`; the caret lands on a boundary.
        let (id, byte) = app.view.pending_goto.expect("caret restore");
        assert_eq!(id, document.id);
        assert!(document.text.is_char_boundary(byte));
        assert!(app.view.closed_tabs.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reopening_with_nothing_closed_or_a_vanished_file_changes_no_buffers() {
        let mut app = LumenApp::default();
        app.reopen_closed_tab();
        assert_eq!(app.view.status, "No recently closed tabs");
        app.view.closed_tabs = vec![ClosedTab {
            path: std::env::temp_dir().join("lumen-vanished/none.txt"),
            caret: 0,
        }];
        app.reopen_closed_tab();
        assert!(app.editor.files.is_empty());
        assert!(app.view.closed_tabs.is_empty(), "the dead entry is dropped");
        assert!(!app.view.status.is_empty());
    }

    #[test]
    fn next_and_previous_wrap_skip_the_hidden_diff_document_and_leave_settings() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs", "/p/c.rs"]);
        let ids = ids(&app);
        app.cycle_tab(1);
        assert_eq!(app.editor.active_id(), Some(ids[1]));
        app.cycle_tab(-1);
        app.cycle_tab(-1);
        assert_eq!(app.editor.active_id(), Some(ids[2]), "wraps to the last");
        app.cycle_tab(1);
        assert_eq!(app.editor.active_id(), Some(ids[0]), "wraps to the first");
        // The diff-only document has no tab, so cycling steps over it.
        app.view.source_control.diff = Some(crate::app::view_state::SourceDiffView {
            root: "/p".into(),
            path: "b.rs".into(),
            side: crate::services::source_control::DiffSide::Staged,
            comparison: Err("unused".into()),
            document: Some(ids[1]),
            diff_only_document: Some(ids[1]),
            opened_revision: None,
            active: false,
        });
        app.cycle_tab(1);
        assert_eq!(app.editor.active_id(), Some(ids[2]));
        // From the Settings page, Next lands on a document and closes the page.
        app.view.settings.open = true;
        app.cycle_tab(1);
        assert!(!app.view.settings.open);
        assert_eq!(app.editor.active_id(), Some(ids[0]));
    }

    #[test]
    fn cycling_with_no_tabs_is_a_noop_with_a_message() {
        let mut app = LumenApp::default();
        app.cycle_tab(1);
        assert_eq!(app.view.status, "No tabs are open");
        assert!(app.editor.active_id().is_none());
    }

    #[test]
    fn move_left_and_right_swap_neighbours_and_stop_at_the_ends() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs", "/p/c.rs"]);
        let ids = ids(&app);
        app.move_tab(ids[0], TabMove::Left);
        assert_eq!(
            super::super::tab_close::visible_documents(&app.editor, &app.view),
            ids
        );
        app.move_tab(ids[0], TabMove::Right);
        assert_eq!(self::ids(&app), [ids[1], ids[0], ids[2]]);
        app.move_tab(ids[2], TabMove::Right);
        assert_eq!(
            self::ids(&app),
            [ids[1], ids[0], ids[2]],
            "last tab stays last"
        );
        app.move_tab(ids[2], TabMove::To(0));
        assert_eq!(self::ids(&app), [ids[2], ids[1], ids[0]]);
        app.move_tab(ids[2], TabMove::To(99));
        assert_eq!(self::ids(&app), [ids[1], ids[0], ids[2]]);
    }

    #[test]
    fn a_hidden_diff_document_never_becomes_a_destination() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/hidden.rs", "/p/c.rs"]);
        let ids = ids(&app);
        app.view.source_control.diff = Some(crate::app::view_state::SourceDiffView {
            root: "/p".into(),
            path: "hidden.rs".into(),
            side: crate::services::source_control::DiffSide::Staged,
            comparison: Err("unused".into()),
            document: Some(ids[1]),
            diff_only_document: Some(ids[1]),
            opened_revision: None,
            active: false,
        });
        // Visible tabs are a and c; moving a right swaps it past c, hidden or not.
        app.move_tab(ids[0], TabMove::Right);
        let visible = super::super::tab_close::visible_documents(&app.editor, &app.view);
        assert_eq!(visible, [ids[2], ids[0]]);
        app.move_tab(ids[0], TabMove::Right);
        assert_eq!(
            super::super::tab_close::visible_documents(&app.editor, &app.view),
            [ids[2], ids[0]],
            "already last"
        );
    }

    #[test]
    fn reordering_keeps_the_active_tab_and_a_pending_close_prompt_on_their_documents() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs", "/p/c.rs"]);
        let ids = ids(&app);
        app.editor.files[1].dirty = true;
        app.request_close(ids[1]);
        assert_eq!(app.view.pending_close, Some(ids[1]));
        app.editor.activate(ids[2]);
        app.move_tab(ids[0], TabMove::To(2));
        app.move_tab(ids[1], TabMove::Left);
        assert_eq!(app.editor.active_id(), Some(ids[2]));
        assert_eq!(app.view.pending_close, Some(ids[1]));
        assert!(app.editor.files[app.editor.index_of(ids[1]).unwrap()].dirty);
    }

    #[test]
    fn moving_the_active_tab_needs_an_active_tab() {
        let mut app = LumenApp::default();
        app.move_active_tab(TabMove::Left);
        assert_eq!(app.view.status, "No tab is active");
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs"]);
        let ids = ids(&app);
        app.move_active_tab(TabMove::Right);
        assert_eq!(self::ids(&app), [ids[1], ids[0]]);
        assert_eq!(app.editor.active_id(), Some(ids[0]));
    }

    fn enable_split(app: &mut LumenApp) {
        app.view.settings.user.persisted.insert(
            "workbench.split_editor".into(),
            lumen_settings_core::Value::Bool(true),
        );
    }

    #[test]
    fn splitting_is_off_by_default_and_changes_nothing_until_enabled() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs"]);
        app.split_editor();
        assert_eq!(app.editor.group_count(), 1);
        assert!(app.view.status.contains("Split Editor"));
        enable_split(&mut app);
        app.split_editor();
        assert_eq!(app.editor.group_count(), 2);
    }

    #[test]
    fn a_lone_tab_cannot_open_a_split_and_the_message_says_why() {
        let mut app = app_with_saved(&["/p/a.rs"]);
        enable_split(&mut app);
        app.split_editor();
        assert_eq!(app.editor.group_count(), 1);
        assert!(app.view.status.contains("another tab"));
        let mut empty = LumenApp::default();
        enable_split(&mut empty);
        empty.split_editor();
        assert_eq!(empty.editor.group_count(), 1);
    }

    #[test]
    fn tab_commands_act_on_the_focused_groups_tabs_only() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs", "/p/c.rs", "/p/d.rs"]);
        enable_split(&mut app);
        let ids = ids(&app);
        app.editor.activate(ids[2]);
        app.split_editor();
        app.move_tab_to_group(ids[3]);
        // Left group: a b. Right group: c d, focused with d active.
        assert_eq!(app.visible_documents(), [ids[2], ids[3]]);
        app.cycle_tab(1);
        assert_eq!(
            app.editor.active_id(),
            Some(ids[2]),
            "wraps inside the group"
        );
        app.cycle_tab(1);
        assert_eq!(app.editor.active_id(), Some(ids[3]));
        // Close All only closes the focused group; the other keeps its tabs.
        app.begin_bulk_close(TabCloseScope::All);
        assert_eq!(app.editor.files.len(), 2);
        assert_eq!(app.editor.group_count(), 1, "the emptied group disappears");
        assert!(
            app.editor
                .files
                .iter()
                .all(|f| f.id == ids[0] || f.id == ids[1])
        );
    }

    #[test]
    fn moving_between_groups_keeps_content_and_focus_commands_switch_groups() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs", "/p/c.rs"]);
        enable_split(&mut app);
        let ids = ids(&app);
        app.editor.files[1].dirty = true;
        app.editor.files[1].text = "unsaved".into();
        app.move_tab_to_group(ids[1]);
        assert_eq!(app.editor.focused_group(), 1);
        let moved = &app.editor.files[app.editor.index_of(ids[1]).unwrap()];
        assert!(moved.dirty);
        assert_eq!(moved.text, "unsaved");
        app.focus_next_group();
        assert_eq!(app.editor.focused_group(), 0);
        app.focus_group_command(1);
        assert_eq!(app.editor.active_id(), Some(ids[1]));
        app.focus_group_command(5);
        assert_eq!(app.view.status, "The editor is not split");
        // Move it back: the groups merge again.
        app.move_tab_to_group(ids[1]);
        assert_eq!(app.editor.group_count(), 1);
    }

    #[test]
    fn reordering_a_tab_in_the_unfocused_group_focuses_that_group_first() {
        let mut app = app_with_saved(&["/p/a.rs", "/p/b.rs", "/p/c.rs", "/p/d.rs"]);
        enable_split(&mut app);
        let ids = ids(&app);
        app.move_tab_to_group(ids[2]);
        app.move_tab_to_group(ids[3]);
        app.editor.focus_group(0);
        app.move_tab(ids[3], TabMove::Left);
        assert_eq!(app.editor.documents_in(1), [ids[3], ids[2]]);
        assert_eq!(app.editor.documents_in(0), [ids[0], ids[1]]);
    }
}
