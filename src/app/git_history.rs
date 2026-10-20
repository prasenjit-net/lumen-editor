//! History dialog: a paginated, read-only commit list with details. Pages are
//! anchored to the commit that was HEAD when the list opened, and every read
//! runs on a worker so nothing here touches the index or worktree.
use super::LumenApp;
use crate::services::source_control::{
    GitBlameLine, GitCommit, GitCommitDetails, GitCommitFile, GitComparison, GitLogPage,
    GitLogQuery,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

pub(crate) const HISTORY_PAGE: usize = 100;

#[derive(Default)]
pub(crate) struct HistoryDialog {
    pub(crate) root: PathBuf,
    pub(crate) anchor: Option<String>,
    pub(crate) commits: Vec<GitCommit>,
    pub(crate) has_more: bool,
    pub(crate) loading: bool,
    pub(crate) selected: Option<String>,
    /// The selected commit as last seen in the list, kept so the details stay
    /// visible when a reload does not (yet) contain it.
    pub(crate) selected_commit: Option<GitCommit>,
    pub(crate) details: Option<GitCommitDetails>,
    /// Which parent of a merge commit the files are compared against.
    pub(crate) parent: usize,
    pub(crate) compare: Option<CommitCompare>,
    /// Filter boxes; they only take effect when applied.
    pub(crate) filter_text: String,
    pub(crate) filter_author: String,
    pub(crate) filter_path: String,
    pub(crate) filter_ref: Option<String>,
    pub(crate) refs: Vec<String>,
    /// The query the loaded pages belong to; later pages reuse it unchanged.
    pub(crate) active: GitLogQuery,
    pub(crate) blame: Option<BlameView>,
    pub(crate) error: Option<String>,
    /// A page read failed; later pages load only on an explicit retry, so a
    /// persistent failure cannot spin.
    pub(crate) page_failed: bool,
    /// What the repository looked like when the list was read; a different
    /// value after a scan means the list is out of date.
    head_key: Option<HeadKey>,
    refs_receiver: Option<Receiver<Result<Vec<String>, String>>>,
    blame_receiver: Option<Receiver<BlameResult>>,
    compare_receiver: Option<Receiver<CompareResult>>,
    page_receiver: Option<Receiver<Result<GitLogPage, String>>>,
    details_receiver: Option<Receiver<(String, Result<GitCommitDetails, String>)>>,
}

/// The parts of a repository's state that move when its history changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeadKey {
    branch: String,
    last_commit: Option<String>,
    ahead_behind: Option<(u32, u32)>,
}

impl HeadKey {
    fn of(snapshot: &crate::services::source_control::GitSnapshot) -> Self {
        Self {
            branch: snapshot.branch.clone(),
            last_commit: snapshot.last_commit.clone(),
            ahead_behind: snapshot
                .upstream
                .as_ref()
                .and_then(|upstream| upstream.ahead_behind),
        }
    }
}

impl HistoryDialog {
    /// The selected commit: from the loaded list, else as last seen.
    pub(crate) fn current_commit(&self) -> Option<&GitCommit> {
        let id = self.selected.as_ref()?;
        self.commits
            .iter()
            .find(|commit| &commit.id == id)
            .or(self.selected_commit.as_ref().filter(|c| &c.id == id))
    }
}

/// Line-by-line authorship of the saved contents of a file.
pub(crate) struct BlameView {
    pub(crate) path: String,
    pub(crate) lines: Option<Result<Vec<GitBlameLine>, String>>,
}

type BlameResult = (String, Result<Vec<GitBlameLine>, String>);

type CompareResult = (CommitCompareKey, Result<GitComparison, String>);

/// What a comparison belongs to; an answer for anything else is dropped.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CommitCompareKey {
    pub(crate) commit: String,
    pub(crate) parent: usize,
    pub(crate) path: String,
}

/// A read-only before/after view of one file of the selected commit.
pub(crate) struct CommitCompare {
    pub(crate) key: CommitCompareKey,
    pub(crate) file: GitCommitFile,
    pub(crate) result: Option<Result<std::sync::Arc<GitComparison>, String>>,
    pub(crate) scroll: super::scroll_sync::ScrollSync,
}

impl LumenApp {
    /// Explicit "Show History": shows the tab and re-reads what Git has now.
    pub(crate) fn open_history_dialog(&mut self, ctx: &egui::Context) {
        self.open_history_tab(ctx, true);
    }

    /// Shows the History tab. `reload` re-reads an already loaded list; the
    /// entry points that go on to reset the list themselves pass false.
    pub(crate) fn open_history_tab(&mut self, ctx: &egui::Context, reload: bool) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        // One History tab, following the repository it was opened for; showing
        // it for another repository starts it over.
        let same_repository = self
            .view
            .history_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.root == root);
        if same_repository {
            self.activate_history_tab(ctx);
            if reload {
                self.reload_history(ctx);
            }
            return;
        }
        let head_key = self.history_head_key(&root);
        self.view.history_dialog = Some(HistoryDialog {
            root,
            head_key,
            ..Default::default()
        });
        self.activate_history_tab(ctx);
        self.load_history_page(ctx);
        self.request_history_refs(ctx);
    }

    /// Makes the History tab the page the editor area shows; the same path as
    /// the other page tabs, so Explorer focus and pending editor input end.
    pub(crate) fn activate_history_tab(&mut self, ctx: &egui::Context) {
        if self.view.history_dialog.is_none() {
            return;
        }
        self.view.leave_page_tabs();
        self.view.task_editors.deactivate();
        self.view.chats.deactivate();
        self.view.history_active = true;
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
        }
        self.enter_page_tab(ctx);
    }

    fn history_head_key(&self, root: &std::path::Path) -> Option<HeadKey> {
        let view = &self.view.source_control;
        view.snapshot
            .iter()
            .chain(view.repositories.iter())
            .find(|repo| repo.root == root)
            .map(HeadKey::of)
    }

    /// Called after a scan: reloads the list when the repository's branch,
    /// latest commit or upstream distance no longer match what it was read for.
    pub(crate) fn reload_history_if_head_moved(&mut self, ctx: &egui::Context) {
        let Some(root) = self
            .view
            .history_dialog
            .as_ref()
            .map(|dialog| dialog.root.clone())
        else {
            return;
        };
        let Some(current) = self.history_head_key(&root) else {
            return;
        };
        let known = self
            .view
            .history_dialog
            .as_ref()
            .and_then(|dialog| dialog.head_key.clone());
        if known.is_some_and(|known| known != current) {
            self.reload_history(ctx);
        } else if let Some(dialog) = &mut self.view.history_dialog {
            dialog.head_key.get_or_insert(current);
        }
    }

    /// Reads the log and refs again after a repository change. Filters and the
    /// selected commit stay; the pages restart from the current tip.
    pub(crate) fn reload_history(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        dialog.commits.clear();
        dialog.anchor = None;
        dialog.has_more = false;
        dialog.page_receiver = None;
        dialog.loading = false;
        dialog.page_failed = false;
        dialog.error = None;
        // Blame describes the saved file as it was; drop it rather than mislead.
        dialog.blame = None;
        dialog.blame_receiver = None;
        let root = dialog.root.clone();
        let key = self.history_head_key(&root);
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.head_key = key;
        }
        self.load_history_page(ctx);
        self.request_history_refs(ctx);
    }

    /// Commit actions from the History tab act on the tab's own repository.
    /// The sidebar selection moves there first; false when it cannot.
    pub(crate) fn target_history_repository(&mut self) -> bool {
        let Some(root) = self
            .view
            .history_dialog
            .as_ref()
            .map(|dialog| dialog.root.clone())
        else {
            return true;
        };
        let selected = |app: &Self| {
            app.view
                .source_control
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.root.clone())
        };
        if selected(self).as_ref() == Some(&root) {
            return true;
        }
        self.select_repository(root.clone());
        if selected(self).as_ref() == Some(&root) {
            return true;
        }
        if !self.source_mutation_in_flight() {
            self.view.status = "The History tab's repository is no longer available".into();
        }
        false
    }

    fn request_history_refs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(service.history_refs(&root));
            worker_ctx.request_repaint();
        });
        dialog.refs_receiver = Some(receiver);
    }

    /// Restarts the list with the filter boxes' current values.
    pub(crate) fn apply_history_filter(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let path = dialog.filter_path.trim();
        dialog.active = GitLogQuery {
            text: dialog.filter_text.trim().to_owned(),
            author: dialog.filter_author.trim().to_owned(),
            path: (!path.is_empty()).then(|| path.replace('\\', "/")),
            reference: dialog.filter_ref.clone(),
        };
        dialog.commits.clear();
        dialog.anchor = None;
        dialog.has_more = false;
        dialog.selected = None;
        dialog.selected_commit = None;
        dialog.details = None;
        dialog.compare = None;
        dialog.blame = None;
        dialog.error = None;
        dialog.page_failed = false;
        // Dropping the receivers discards answers for the previous query.
        dialog.page_receiver = None;
        dialog.loading = false;
        self.load_history_page(ctx);
    }

    pub(crate) fn clear_history_filter(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.filter_text.clear();
            dialog.filter_author.clear();
            dialog.filter_path.clear();
            dialog.filter_ref = None;
        }
        self.apply_history_filter(ctx);
    }

    pub(crate) fn set_history_ref(&mut self, reference: Option<String>, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.filter_ref = reference;
        }
        self.apply_history_filter(ctx);
    }

    /// The saved file open in the editor, relative to the selected repository.
    fn active_repository_file(&self) -> Option<String> {
        let root = &self.view.source_control.snapshot.as_ref()?.root;
        let path = self.editor.active()?.path.as_ref()?;
        let relative = path.strip_prefix(root).ok()?;
        relative.to_str().map(|text| text.replace('\\', "/"))
    }

    /// History of the active editor file, following renames.
    pub(crate) fn open_file_history(&mut self, ctx: &egui::Context) {
        let Some(path) = self.active_repository_file() else {
            self.view.status = "Open a file inside the repository to see its history".into();
            return;
        };
        self.open_history_tab(ctx, false);
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.filter_path = path;
        }
        self.apply_history_filter(ctx);
    }

    /// Blame for the saved contents of the active editor file.
    pub(crate) fn open_blame(&mut self, ctx: &egui::Context) {
        let Some(path) = self.active_repository_file() else {
            self.view.status = "Open a file inside the repository to blame it".into();
            return;
        };
        self.open_history_tab(ctx, false);
        self.request_blame(path, ctx);
    }

    fn request_blame(&mut self, path: String, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let result = service.blame(&root, &worker_path);
            let _ = sender.send((worker_path, result));
            worker_ctx.request_repaint();
        });
        dialog.blame_receiver = Some(receiver);
        dialog.blame = Some(BlameView { path, lines: None });
        // The page shows a comparison before blame, so end any open one.
        dialog.compare = None;
        dialog.compare_receiver = None;
    }

    pub(crate) fn close_blame(&mut self) {
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.blame = None;
            dialog.blame_receiver = None;
        }
    }

    /// Jumps from a blame line to its commit by searching for its id.
    pub(crate) fn show_blamed_commit(&mut self, id: String, ctx: &egui::Context) {
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.filter_text = id.clone();
            dialog.filter_author.clear();
            dialog.filter_path.clear();
            dialog.filter_ref = None;
        }
        self.apply_history_filter(ctx);
        self.select_commit(id, ctx);
    }

    pub(crate) fn close_history_dialog(&mut self) {
        self.view.history_dialog = None;
        self.view.history_active = false;
    }

    /// Requests the next page after the rows already loaded.
    pub(crate) fn load_history_page(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        if dialog.loading {
            return;
        }
        let root = dialog.root.clone();
        let anchor = dialog.anchor.clone();
        let query = dialog.active.clone();
        let skip = dialog.commits.len();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(service.log(&root, &query, anchor.as_deref(), skip, HISTORY_PAGE));
            worker_ctx.request_repaint();
        });
        dialog.page_receiver = Some(receiver);
        dialog.loading = true;
        dialog.page_failed = false;
    }

    pub(crate) fn select_commit(&mut self, id: String, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        dialog.selected_commit = dialog.commits.iter().find(|c| c.id == id).cloned();
        dialog.selected = Some(id.clone());
        dialog.parent = 0;
        dialog.compare = None;
        self.request_commit_details(ctx);
    }

    fn request_commit_details(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let Some(id) = dialog.selected.clone() else {
            return;
        };
        dialog.details = None;
        let root = dialog.root.clone();
        let parent = dialog.parent;
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = service.commit_details(&root, &id, parent);
            let _ = sender.send((id, result));
            worker_ctx.request_repaint();
        });
        dialog.details_receiver = Some(receiver);
    }

    /// Chooses which parent of a merge commit its files are compared against.
    pub(crate) fn choose_history_parent(&mut self, parent: usize, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let parents = dialog
            .current_commit()
            .map_or(0, |commit| commit.parents.len());
        if parent >= parents.max(1) || parent == dialog.parent {
            return;
        }
        dialog.parent = parent;
        dialog.compare = None;
        self.request_commit_details(ctx);
    }

    pub(crate) fn open_commit_file(&mut self, index: usize, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let (Some(commit), Some(file)) = (
            dialog.selected.clone(),
            dialog
                .details
                .as_ref()
                .and_then(|details| details.files.get(index).cloned()),
        ) else {
            return;
        };
        let key = CommitCompareKey {
            commit: commit.clone(),
            parent: dialog.parent,
            path: file.path.clone(),
        };
        let (root, parent) = (dialog.root.clone(), dialog.parent);
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let worker_file = file.clone();
        let worker_key = key.clone();
        std::thread::spawn(move || {
            let result = service.commit_comparison(&root, &commit, parent, &worker_file);
            let _ = sender.send((worker_key, result));
            worker_ctx.request_repaint();
        });
        dialog.compare_receiver = Some(receiver);
        dialog.compare = Some(CommitCompare {
            key,
            file,
            result: None,
            scroll: super::scroll_sync::ScrollSync::at_top(),
        });
    }

    pub(crate) fn close_commit_file(&mut self) {
        if let Some(dialog) = &mut self.view.history_dialog {
            dialog.compare = None;
            dialog.compare_receiver = None;
        }
    }

    pub(crate) fn poll_history(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        if let Some(receiver) = &dialog.page_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    dialog.page_receiver = None;
                    dialog.loading = false;
                    match result {
                        Ok(page) => {
                            dialog.anchor.get_or_insert(page.anchor);
                            dialog.has_more = page.has_more;
                            dialog.commits.extend(page.commits);
                            dialog.error = None;
                            // A selection made before its commit loaded (a jump
                            // from blame or a sync list) resolves now.
                            if let Some(id) = &dialog.selected
                                && dialog.selected_commit.as_ref().map(|c| &c.id) != Some(id)
                            {
                                dialog.selected_commit =
                                    dialog.commits.iter().find(|c| &c.id == id).cloned();
                            }
                        }
                        Err(error) => {
                            dialog.error = Some(error);
                            dialog.page_failed = true;
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    dialog.page_receiver = None;
                    dialog.loading = false;
                }
            }
        }
        if let Some(receiver) = &dialog.refs_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    dialog.refs_receiver = None;
                    if let Ok(refs) = result {
                        dialog.refs = refs;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => dialog.refs_receiver = None,
            }
        }
        if let Some(receiver) = &dialog.blame_receiver {
            match receiver.try_recv() {
                Ok((path, result)) => {
                    dialog.blame_receiver = None;
                    if let Some(blame) = dialog.blame.as_mut().filter(|b| b.path == path) {
                        blame.lines = Some(result);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => dialog.blame_receiver = None,
            }
        }
        if let Some(receiver) = &dialog.compare_receiver {
            match receiver.try_recv() {
                Ok((key, result)) => {
                    dialog.compare_receiver = None;
                    if let Some(compare) = dialog.compare.as_mut().filter(|c| c.key == key) {
                        compare.result = Some(result.map(std::sync::Arc::new));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => dialog.compare_receiver = None,
            }
        }
        if let Some(receiver) = &dialog.details_receiver {
            match receiver.try_recv() {
                Ok((id, result)) => {
                    dialog.details_receiver = None;
                    // A slow answer for a commit that is no longer selected is dropped.
                    if dialog.selected.as_ref() == Some(&id) {
                        match result {
                            Ok(details) => dialog.details = Some(details),
                            Err(error) => dialog.error = Some(error),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => dialog.details_receiver = None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::source_control::GitSnapshot;
    use std::process::Command;

    fn app_with_repository(root: &str) -> LumenApp {
        let mut app = LumenApp::default();
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from(root),
            ..Default::default()
        });
        app
    }

    #[test]
    fn opening_history_shows_its_tab_and_replaces_other_pages() {
        let mut app = app_with_repository("/w/a");
        app.view.settings.open = true;
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        assert!(app.view.history_active && !app.view.settings.open);
        assert!(app.view.page_tab_active());

        // Choosing another page leaves the tab open but not active.
        app.view.leave_page_tabs();
        assert!(!app.view.history_active && app.view.history_dialog.is_some());
        // Reopening reuses the loaded state of the same repository.
        app.view.history_dialog.as_mut().unwrap().filter_text = "kept".into();
        app.open_history_dialog(&ctx);
        assert!(app.view.history_active);
        assert_eq!(
            app.view.history_dialog.as_ref().unwrap().filter_text,
            "kept"
        );

        // Another repository starts over; closing removes the tab.
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from("/w/b"),
            ..Default::default()
        });
        app.open_history_dialog(&ctx);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert_eq!(dialog.root, PathBuf::from("/w/b"));
        assert!(dialog.filter_text.is_empty());
        app.close_history_dialog();
        assert!(app.view.history_dialog.is_none() && !app.view.history_active);
    }

    #[test]
    fn a_review_or_tag_dialog_leaves_the_history_tab_open() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        app.plan_history_action(
            crate::services::source_control::HistoryAction::Revert {
                commit: "abc".into(),
                parent: None,
            },
            "Revert abc".into(),
            &ctx,
        );
        assert!(app.view.history_op_dialog.is_some());
        assert!(app.view.history_dialog.is_some() && app.view.history_active);
        app.open_tags_dialog(Some("abc".into()), &ctx);
        assert!(app.view.history_dialog.is_some() && app.view.history_active);
    }

    fn app_with_two_repositories() -> LumenApp {
        let mut app = LumenApp::default();
        app.apply_repository_scan(
            std::path::Path::new("/w"),
            crate::app::source_control::RepositoryScan {
                snapshots: ["/w/a", "/w/b"]
                    .map(|root| GitSnapshot {
                        root: PathBuf::from(root),
                        branch: "main".into(),
                        ..Default::default()
                    })
                    .to_vec(),
                errors: Vec::new(),
            },
        );
        app
    }

    fn selected_root(app: &LumenApp) -> PathBuf {
        app.view
            .source_control
            .snapshot
            .as_ref()
            .unwrap()
            .root
            .clone()
    }

    #[test]
    fn commit_actions_run_against_the_history_tabs_repository_never_the_selected_one() {
        use crate::services::source_control::{HistoryAction, ResetMode};
        let mut app = app_with_two_repositories();
        let ctx = egui::Context::default();
        app.select_repository("/w/a".into());
        app.open_history_dialog(&ctx);
        app.select_repository("/w/b".into());
        assert_eq!(selected_root(&app), PathBuf::from("/w/b"));

        app.plan_history_action(
            HistoryAction::Reset {
                target: "abc".into(),
                mode: ResetMode::Hard,
                backup: true,
            },
            "Reset".into(),
            &ctx,
        );
        assert_eq!(selected_root(&app), PathBuf::from("/w/a"));
        assert_eq!(
            app.view.history_op_dialog.as_ref().unwrap().root,
            PathBuf::from("/w/a"),
            "the review must be for the repository the commit belongs to"
        );

        // A tag on a commit follows the same rule.
        app.view.history_op_dialog = None;
        app.select_repository("/w/b".into());
        app.open_tags_dialog(Some("abc".into()), &ctx);
        assert_eq!(
            app.view.tags_dialog.as_ref().unwrap().root,
            PathBuf::from("/w/a")
        );

        // Without a commit the tags dialog keeps following the selection.
        app.view.tags_dialog = None;
        app.select_repository("/w/b".into());
        app.open_tags_dialog(None, &ctx);
        assert_eq!(
            app.view.tags_dialog.as_ref().unwrap().root,
            PathBuf::from("/w/b")
        );
    }

    #[test]
    fn a_busy_repository_refuses_to_move_the_selection_and_the_action_does_not_run() {
        use crate::services::source_control::HistoryAction;
        let mut app = app_with_two_repositories();
        let ctx = egui::Context::default();
        app.select_repository("/w/a".into());
        app.open_history_dialog(&ctx);
        app.select_repository("/w/b".into());
        let (_sender, receiver) = mpsc::channel();
        app.source_control_job = Some(crate::app::source_control::SourceControlJob {
            receiver,
            mutating: true,
        });
        app.plan_history_action(
            HistoryAction::Revert {
                commit: "abc".into(),
                parent: None,
            },
            "Revert".into(),
            &ctx,
        );
        assert!(app.view.history_op_dialog.is_none());
        assert_eq!(selected_root(&app), PathBuf::from("/w/b"));
    }

    fn make_stale(app: &mut LumenApp) {
        let dialog = app.view.history_dialog.as_mut().unwrap();
        dialog.loading = false;
        dialog.page_receiver = None;
        dialog.commits = vec![GitCommit {
            id: "old".into(),
            ..Default::default()
        }];
        dialog.anchor = Some("old".into());
        dialog.filter_text = "kept".into();
    }

    fn scan_with(app: &mut LumenApp, last_commit: &str) {
        app.apply_repository_scan(
            std::path::Path::new("/w"),
            crate::app::source_control::RepositoryScan {
                snapshots: vec![GitSnapshot {
                    root: PathBuf::from("/w/a"),
                    branch: "main".into(),
                    last_commit: Some(last_commit.into()),
                    ..Default::default()
                }],
                errors: Vec::new(),
            },
        );
    }

    #[test]
    fn only_an_explicit_show_reloads_and_the_other_entry_points_keep_the_list() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        make_stale(&mut app);
        app.open_history_tab(&ctx, false);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert_eq!(
            dialog.commits.len(),
            1,
            "blame and file history do not reload"
        );

        app.open_history_dialog(&ctx);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.commits.is_empty() && dialog.loading && dialog.anchor.is_none());
        assert_eq!(dialog.filter_text, "kept", "filters survive a reload");
    }

    #[test]
    fn the_log_reloads_when_a_scan_shows_the_head_moved_and_not_for_other_changes() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        scan_with(&mut app, "first");
        app.select_repository("/w/a".into());
        app.open_history_dialog(&ctx);
        make_stale(&mut app);

        // The same tip (a stage, an unrelated refresh) leaves the list alone.
        scan_with(&mut app, "first");
        app.reload_history_if_head_moved(&ctx);
        assert_eq!(app.view.history_dialog.as_ref().unwrap().commits.len(), 1);

        // A new commit, pull, switch or reset changes the key and reloads.
        scan_with(&mut app, "second");
        app.reload_history_if_head_moved(&ctx);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.commits.is_empty() && dialog.loading);
        // The reload recorded the new state, so it does not repeat.
        make_stale(&mut app);
        app.reload_history_if_head_moved(&ctx);
        assert_eq!(app.view.history_dialog.as_ref().unwrap().commits.len(), 1);
    }

    #[test]
    fn a_tag_operation_reloads_the_log_but_staging_does_not() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        let finish = |app: &mut LumenApp| {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(crate::app::source_control::GitResult::Operation(Ok(
                    "Done".into()
                )))
                .unwrap();
            app.source_control_job = Some(crate::app::source_control::SourceControlJob {
                receiver,
                mutating: false,
            });
            app.poll_source_control(&ctx);
        };
        make_stale(&mut app);
        finish(&mut app);
        assert_eq!(
            app.view.history_dialog.as_ref().unwrap().commits.len(),
            1,
            "staging leaves the list, scroll position and blame alone"
        );
        app.view.tags_dialog = Some(Default::default());
        finish(&mut app);
        assert!(app.view.history_dialog.as_ref().unwrap().commits.is_empty());
    }

    #[test]
    fn a_repository_that_failed_to_read_keeps_its_history_tab() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        scan_with(&mut app, "first");
        app.select_repository("/w/a".into());
        app.open_history_dialog(&ctx);
        app.apply_repository_scan(
            std::path::Path::new("/w"),
            crate::app::source_control::RepositoryScan {
                snapshots: Vec::new(),
                errors: vec![(PathBuf::from("/w/a"), "index.lock exists".into())],
            },
        );
        assert!(
            app.view.history_dialog.is_some(),
            "a transient failure is not a vanished repository"
        );
    }

    #[test]
    fn the_selection_survives_a_reload_that_does_not_contain_it() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        let commit = |id: &str| GitCommit {
            id: id.into(),
            parents: vec!["p1".into(), "p2".into()],
            ..Default::default()
        };
        app.view.history_dialog.as_mut().unwrap().commits = vec![commit("deep")];
        app.select_commit("deep".into(), &ctx);
        app.reload_history(&ctx);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.commits.is_empty());
        assert_eq!(
            dialog.current_commit().map(|c| c.id.as_str()),
            Some("deep"),
            "the details pane keeps what was selected"
        );
        // A merge's parent choice still works from the remembered commit.
        app.choose_history_parent(1, &ctx);
        assert_eq!(app.view.history_dialog.as_ref().unwrap().parent, 1);
        // A new query clears the selection.
        app.apply_history_filter(&ctx);
        assert!(
            app.view
                .history_dialog
                .as_ref()
                .unwrap()
                .current_commit()
                .is_none()
        );
    }

    #[test]
    fn a_failed_page_waits_for_retry_and_blame_replaces_a_comparison() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        let (sender, receiver) = mpsc::channel();
        sender.send(Err("index.lock exists".to_owned())).unwrap();
        let dialog = app.view.history_dialog.as_mut().unwrap();
        dialog.has_more = true;
        dialog.page_receiver = Some(receiver);
        dialog.loading = true;
        app.poll_history(&ctx);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.page_failed && !dialog.loading && dialog.error.is_some());
        app.load_history_page(&ctx);
        assert!(
            !app.view.history_dialog.as_ref().unwrap().page_failed,
            "retry clears it"
        );

        let dialog = app.view.history_dialog.as_mut().unwrap();
        dialog.compare = Some(CommitCompare {
            key: CommitCompareKey {
                commit: "c".into(),
                parent: 0,
                path: "f".into(),
            },
            file: Default::default(),
            result: None,
            scroll: crate::app::scroll_sync::ScrollSync::at_top(),
        });
        app.request_blame("f.txt".into(), &ctx);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.compare.is_none() && dialog.blame.is_some());
    }

    #[test]
    fn close_editor_closes_the_history_tab_and_a_task_editor_replaces_it() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        let mut document = crate::model::Document::from_path("/w/a/f.txt".into(), "x\n".into());
        document.dirty = true;
        app.editor.files.push(document);
        app.editor.set_active_index(Some(0));
        app.open_history_dialog(&ctx);
        app.execute(crate::app::command::Command::CloseEditor, &ctx);
        assert!(app.view.history_dialog.is_none() && !app.view.history_active);
        assert!(
            app.view.pending_close.is_none(),
            "a hidden dirty document must not get a save prompt"
        );

        app.open_history_dialog(&ctx);
        app.show_task_editor_page(&ctx);
        assert!(!app.view.history_active, "the task page takes over");
        assert!(app.view.history_dialog.is_some(), "the tab stays open");
    }

    #[test]
    fn showing_the_history_tab_ends_explorer_focus_like_other_pages() {
        let mut app = app_with_repository("/w/a");
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        app.view.leave_page_tabs();
        app.view.explorer_focused = true;
        app.execute(crate::app::command::Command::ShowHistoryTab, &ctx);
        assert!(app.view.history_active && !app.view.explorer_focused);
    }

    #[test]
    fn history_loads_in_pages_and_selecting_a_commit_loads_its_details() {
        let directory = tempfile::tempdir().unwrap();
        let root = crate::services::source_control::canonical_path(directory.path()).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["checkout", "-q", "-b", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        git(&["config", "core.autocrlf", "false"]);
        for n in 0..(HISTORY_PAGE + 5) {
            std::fs::write(root.join("f.txt"), format!("{n}\n")).unwrap();
            git(&["add", "--", "f.txt"]);
            git(&["commit", "-q", "-m", &format!("commit {n}")]);
        }
        let mut app = LumenApp::default();
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: root.clone(),
            ..Default::default()
        });
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        let wait = |app: &mut LumenApp| {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while app.view.history_dialog.as_ref().is_some_and(|d| {
                d.loading || d.details_receiver.is_some() || d.compare_receiver.is_some()
            }) && std::time::Instant::now() < deadline
            {
                app.poll_history(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&mut app);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert_eq!(dialog.commits.len(), HISTORY_PAGE);
        assert!(dialog.has_more);
        app.load_history_page(&ctx);
        wait(&mut app);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert_eq!(dialog.commits.len(), HISTORY_PAGE + 5);
        assert!(!dialog.has_more);
        let id = dialog.commits[0].id.clone();
        app.select_commit(id, &ctx);
        wait(&mut app);
        let details = app
            .view
            .history_dialog
            .as_ref()
            .unwrap()
            .details
            .as_ref()
            .unwrap();
        assert!(details.message.starts_with("commit"));
        assert_eq!(details.files[0].path, "f.txt");
        app.open_commit_file(0, &ctx);
        wait(&mut app);
        let compare = app
            .view
            .history_dialog
            .as_ref()
            .unwrap()
            .compare
            .as_ref()
            .unwrap();
        let comparison = compare.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(
            comparison.changed.as_deref(),
            Some(&*format!("{}\n", HISTORY_PAGE + 4))
        );
        assert_eq!(comparison.base, format!("{}\n", HISTORY_PAGE + 3));
        // Choosing another commit leaves the comparison view.
        let other = app.view.history_dialog.as_ref().unwrap().commits[1]
            .id
            .clone();
        app.select_commit(other, &ctx);
        assert!(app.view.history_dialog.as_ref().unwrap().compare.is_none());
        // A single-parent commit offers no other parent.
        app.choose_history_parent(1, &ctx);
        assert_eq!(app.view.history_dialog.as_ref().unwrap().parent, 0);
    }

    #[test]
    fn filters_file_history_and_blame_drive_the_dialog() {
        let directory = tempfile::tempdir().unwrap();
        let root = crate::services::source_control::canonical_path(directory.path()).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["checkout", "-q", "-b", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        git(&["config", "core.autocrlf", "false"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(&["add", "--", "a.txt"]);
        git(&["commit", "-q", "-m", "add a"]);
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git(&["add", "--", "b.txt"]);
        git(&["commit", "-q", "-m", "add b"]);
        let mut app = LumenApp::default();
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: root.clone(),
            ..Default::default()
        });
        let ctx = egui::Context::default();
        let wait = |app: &mut LumenApp| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while app.view.history_dialog.as_ref().is_some_and(|d| {
                d.loading
                    || d.details_receiver.is_some()
                    || d.blame_receiver.is_some()
                    || d.refs_receiver.is_some()
            }) && std::time::Instant::now() < deadline
            {
                app.poll_history(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        // No file open: friendly refusal, no dialog.
        app.open_file_history(&ctx);
        assert!(app.view.history_dialog.is_none());
        assert!(app.view.status.contains("Open a file"));
        app.open_history_dialog(&ctx);
        wait(&mut app);
        assert_eq!(app.view.history_dialog.as_ref().unwrap().commits.len(), 2);
        assert!(
            app.view
                .history_dialog
                .as_ref()
                .unwrap()
                .refs
                .contains(&"main".to_owned())
        );
        app.view.history_dialog.as_mut().unwrap().filter_text = "add a".into();
        app.apply_history_filter(&ctx);
        wait(&mut app);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert_eq!(dialog.commits.len(), 1);
        assert_eq!(dialog.active.text, "add a");
        app.clear_history_filter(&ctx);
        wait(&mut app);
        assert_eq!(app.view.history_dialog.as_ref().unwrap().commits.len(), 2);
        // File history and blame use the active editor file.
        app.close_history_dialog();
        let mut document = crate::model::Document::from_path(root.join("b.txt"), "b\n".into());
        document.dirty = false;
        app.editor.files.push(document);
        app.editor.set_active_index(Some(0));
        app.open_file_history(&ctx);
        wait(&mut app);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert_eq!(dialog.active.path.as_deref(), Some("b.txt"));
        assert_eq!(dialog.commits.len(), 1);
        app.open_blame(&ctx);
        wait(&mut app);
        let blame = app
            .view
            .history_dialog
            .as_ref()
            .unwrap()
            .blame
            .as_ref()
            .unwrap();
        let lines = blame.lines.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(lines[0].summary, "add b");
        let id = lines[0].commit.clone();
        app.show_blamed_commit(id.clone(), &ctx);
        wait(&mut app);
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.blame.is_none());
        assert_eq!(dialog.commits.len(), 1);
        assert_eq!(dialog.selected.as_deref(), Some(id.as_str()));
        assert!(dialog.details.is_some());
    }

    #[test]
    fn an_unborn_repository_shows_an_empty_history_without_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let root = crate::services::source_control::canonical_path(directory.path()).unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "--quiet"])
            .output()
            .unwrap();
        let mut app = LumenApp::default();
        app.view.source_control.snapshot = Some(GitSnapshot {
            root,
            ..Default::default()
        });
        let ctx = egui::Context::default();
        app.open_history_dialog(&ctx);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.view.history_dialog.as_ref().is_some_and(|d| d.loading)
            && std::time::Instant::now() < deadline
        {
            app.poll_history(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        let dialog = app.view.history_dialog.as_ref().unwrap();
        assert!(dialog.commits.is_empty() && dialog.error.is_none());
    }
}
