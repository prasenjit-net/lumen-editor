//! Tab navigation and recovery commands: next/previous, reopen, and the
//! active-tab-relative close commands the palette and shortcuts use.
use super::{LumenApp, tab_close::TabCloseScope, view_state::ClosedTab};
use crate::model::DocumentId;

impl LumenApp {
    /// Make `id` the visible tab, leaving Settings or a diff view if one was showing.
    pub(crate) fn show_document_tab(&mut self, id: DocumentId) {
        if !self.editor.activate(id) {
            return;
        }
        self.view.settings.open = false;
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
}
