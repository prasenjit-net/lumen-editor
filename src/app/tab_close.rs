//! Closing several editor tabs at once, one dirty buffer prompt at a time.
use super::{LumenApp, command::Command};
use crate::model::DocumentId;

/// Which tabs a bulk close targets, relative to the tab it was invoked on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabCloseScope {
    Others(DocumentId),
    ToTheRight(DocumentId),
    ToTheLeft(DocumentId),
    /// Every tab without unsaved changes; never prompts.
    Saved,
    All,
}

/// Documents still to close, in tab order, and the one currently awaiting the
/// user. Targets are stable document IDs, never tab indices.
#[derive(Default)]
pub(crate) struct BulkClose {
    queue: Vec<DocumentId>,
    awaiting: Option<DocumentId>,
}

impl BulkClose {
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
        self.awaiting = None;
    }
}

/// Tabs shown in the focused group's strip: the document backing an open diff is hidden.
pub(crate) fn visible_documents(
    editor: &crate::model::EditorState,
    view: &super::view_state::ViewState,
) -> Vec<DocumentId> {
    let hidden = view
        .source_control
        .diff
        .as_ref()
        .and_then(|diff| diff.diff_only_document);
    // Tab commands act on the focused editor group's strip.
    editor
        .documents_in(editor.focused_group())
        .into_iter()
        .filter(|id| Some(*id) != hidden)
        .collect()
}

impl LumenApp {
    pub(crate) fn visible_documents(&self) -> Vec<DocumentId> {
        visible_documents(&self.editor, &self.view)
    }

    pub(crate) fn begin_bulk_close(&mut self, scope: TabCloseScope) {
        let visible = self.visible_documents();
        let targets: Vec<DocumentId> = match scope {
            TabCloseScope::All => visible,
            TabCloseScope::Others(keep) => visible.into_iter().filter(|id| *id != keep).collect(),
            TabCloseScope::ToTheRight(from) => match visible.iter().position(|id| *id == from) {
                Some(position) => visible[position + 1..].to_vec(),
                None => Vec::new(),
            },
            TabCloseScope::ToTheLeft(from) => match visible.iter().position(|id| *id == from) {
                Some(position) => visible[..position].to_vec(),
                None => Vec::new(),
            },
            TabCloseScope::Saved => visible
                .into_iter()
                .filter(|id| {
                    self.editor
                        .index_of(*id)
                        .is_some_and(|index| !self.editor.files[index].dirty)
                })
                .collect(),
        };
        if targets.is_empty() {
            self.view.status = "No tabs to close".into();
            return;
        }
        // Close everything that is clean first, so the prompts for unsaved
        // buffers come last and never leave clean tabs waiting behind them.
        let is_dirty = |app: &Self, id: &DocumentId| {
            app.editor
                .files
                .iter()
                .any(|file| file.id == *id && file.dirty)
        };
        let (dirty, clean): (Vec<_>, Vec<_>) =
            targets.into_iter().partition(|id| is_dirty(self, id));
        self.view.bulk_close.queue = clean.into_iter().chain(dirty).collect();
        self.view.bulk_close.awaiting = None;
        self.advance_bulk_close();
    }

    /// Close clean targets immediately and stop at the first dirty one so the
    /// existing unsaved-changes prompt decides it. Call every frame: it resumes
    /// once that prompt has been answered, and abandons the rest if the answer
    /// left the document open (Cancel, or a Save that did not complete).
    pub(crate) fn advance_bulk_close(&mut self) {
        if self.view.pending_close.is_some() {
            return;
        }
        if let Some(awaited) = self.view.bulk_close.awaiting.take()
            && self.editor.index_of(awaited).is_some()
        {
            self.view.bulk_close.clear();
            return;
        }
        while !self.view.bulk_close.queue.is_empty() {
            let id = self.view.bulk_close.queue.remove(0);
            let Some(index) = self.editor.index_of(id) else {
                continue;
            };
            if self.editor.files[index].dirty {
                self.view.bulk_close.awaiting = Some(id);
                self.view.pending_close = Some(id);
                return;
            }
            self.command_local(Command::CloseFile(id));
        }
        self.view.status = "Closed tabs".into();
    }

    fn command_local(&mut self, command: Command) {
        command.apply_local(&mut self.editor, &mut self.view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_tabs(count: usize) -> (LumenApp, Vec<DocumentId>) {
        let mut app = LumenApp::default();
        for _ in 0..count {
            app.editor.new_file();
        }
        let ids = app.editor.files.iter().map(|file| file.id).collect();
        (app, ids)
    }

    fn open_ids(app: &LumenApp) -> Vec<DocumentId> {
        app.editor.files.iter().map(|file| file.id).collect()
    }

    #[test]
    fn close_others_keeps_the_chosen_tab_active_and_closes_clean_tabs() {
        let (mut app, ids) = app_with_tabs(4);
        app.editor.set_active_index(Some(3));
        app.begin_bulk_close(TabCloseScope::Others(ids[1]));
        assert_eq!(open_ids(&app), [ids[1]]);
        assert_eq!(app.view.pending_close, None);
        // Closing around the active tab never changes which tab is active.
        let (mut app, ids) = app_with_tabs(4);
        app.editor.set_active_index(Some(3));
        app.begin_bulk_close(TabCloseScope::ToTheRight(ids[0]));
        assert_eq!(open_ids(&app), [ids[0]]);
        let (mut app, ids) = app_with_tabs(4);
        app.editor.set_active_index(Some(3));
        app.begin_bulk_close(TabCloseScope::Others(ids[3]));
        assert_eq!(app.editor.active_index(), Some(0));
        assert_eq!(open_ids(&app), [ids[3]]);
    }

    #[test]
    fn a_dirty_tab_prompts_and_cancel_abandons_the_rest() {
        let (mut app, ids) = app_with_tabs(4);
        app.editor.files[1].dirty = true;
        app.editor.files[1].text = "keep me".into();
        app.begin_bulk_close(TabCloseScope::All);
        // Every clean tab closed; the dirty one now awaits the user.
        assert_eq!(open_ids(&app), [ids[1]]);
        assert_eq!(app.view.pending_close, Some(app.editor.files[0].id));
        // Cancel clears the prompt but leaves the document open.
        app.view.pending_close = None;
        app.advance_bulk_close();
        assert_eq!(open_ids(&app), [ids[1]]);
        assert!(app.editor.files[0].dirty);
        assert_eq!(app.editor.files[0].text, "keep me");
        // Nothing resumes later.
        app.advance_bulk_close();
        assert_eq!(open_ids(&app).len(), 1);
    }

    #[test]
    fn discarding_a_dirty_tab_lets_the_remaining_tabs_close() {
        let (mut app, ids) = app_with_tabs(3);
        app.editor.files[1].dirty = true;
        app.begin_bulk_close(TabCloseScope::All);
        assert_eq!(open_ids(&app), [ids[1]]);
        // "Don't Save" closes the prompted tab, as the dialog does.
        let index = app.view.pending_close.take().unwrap();
        app.close_file(index);
        app.advance_bulk_close();
        assert!(open_ids(&app).is_empty());
    }

    #[test]
    fn the_diff_only_documents_hidden_tab_is_not_closed_by_bulk_actions() {
        let (mut app, ids) = app_with_tabs(3);
        app.view.source_control.diff = Some(crate::app::view_state::SourceDiffView {
            root: "/workspace".into(),
            path: "file.rs".into(),
            side: crate::services::source_control::DiffSide::Staged,
            comparison: Err("unused".into()),
            document: Some(ids[2]),
            diff_only_document: Some(ids[2]),
            opened_revision: None,
            active: false,
        });
        app.begin_bulk_close(TabCloseScope::All);
        assert!(open_ids(&app).contains(&ids[2]));
    }

    #[test]
    fn close_to_the_left_closes_only_tabs_before_the_chosen_one() {
        let (mut app, ids) = app_with_tabs(4);
        app.begin_bulk_close(TabCloseScope::ToTheLeft(ids[2]));
        assert_eq!(open_ids(&app), [ids[2], ids[3]]);
        app.begin_bulk_close(TabCloseScope::ToTheLeft(ids[2]));
        assert_eq!(open_ids(&app), [ids[2], ids[3]]);
        assert_eq!(
            app.view.status, "No tabs to close",
            "nothing is to the left"
        );
    }

    #[test]
    fn close_saved_never_prompts_and_keeps_every_unsaved_tab() {
        let (mut app, ids) = app_with_tabs(4);
        app.editor.files[1].dirty = true;
        app.editor.files[1].text = "keep me".into();
        app.begin_bulk_close(TabCloseScope::Saved);
        assert_eq!(open_ids(&app), [ids[1]]);
        assert_eq!(
            app.view.pending_close, None,
            "saved tabs close without a prompt"
        );
        assert_eq!(app.editor.files[0].text, "keep me");
        assert!(app.editor.files[0].dirty);
        app.begin_bulk_close(TabCloseScope::Saved);
        assert_eq!(app.view.status, "No tabs to close");
    }
}
