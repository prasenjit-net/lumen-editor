//! Closing several editor tabs at once, one dirty buffer prompt at a time.
use super::{LumenApp, command::Command};
use crate::model::DocumentId;

/// Which tabs a bulk close targets, relative to the tab it was invoked on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabCloseScope {
    Others(DocumentId),
    ToTheRight(DocumentId),
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

impl LumenApp {
    /// Tabs shown in the strip: the document backing an open diff is hidden.
    fn visible_documents(&self) -> Vec<DocumentId> {
        let hidden = self
            .view
            .source_control
            .diff
            .as_ref()
            .and_then(|diff| diff.diff_only_document);
        self.editor
            .files
            .iter()
            .map(|file| file.id)
            .filter(|id| Some(*id) != hidden)
            .collect()
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
        };
        if targets.is_empty() {
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
            && self.editor.files.iter().any(|file| file.id == awaited)
        {
            self.view.bulk_close.clear();
            return;
        }
        let active = self.editor.active().map(|file| file.id);
        while !self.view.bulk_close.queue.is_empty() {
            let id = self.view.bulk_close.queue.remove(0);
            let Some(index) = self.editor.files.iter().position(|file| file.id == id) else {
                continue;
            };
            if self.editor.files[index].dirty {
                self.view.bulk_close.awaiting = Some(id);
                self.view.pending_close = Some(index);
                return;
            }
            self.command_local(Command::CloseFile(index));
            // Closing another tab must not change which tab is active.
            if let Some(active) = active
                && let Some(position) = self.editor.files.iter().position(|file| file.id == active)
            {
                self.editor.active_file = Some(position);
            }
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
        app.editor.active_file = Some(3);
        app.begin_bulk_close(TabCloseScope::Others(ids[1]));
        assert_eq!(open_ids(&app), [ids[1]]);
        assert_eq!(app.view.pending_close, None);
        // Closing around the active tab never changes which tab is active.
        let (mut app, ids) = app_with_tabs(4);
        app.editor.active_file = Some(3);
        app.begin_bulk_close(TabCloseScope::ToTheRight(ids[0]));
        assert_eq!(open_ids(&app), [ids[0]]);
        let (mut app, ids) = app_with_tabs(4);
        app.editor.active_file = Some(3);
        app.begin_bulk_close(TabCloseScope::Others(ids[3]));
        assert_eq!(app.editor.active_file, Some(0));
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
        assert_eq!(app.view.pending_close, Some(0));
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
}
