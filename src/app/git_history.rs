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
    refs_receiver: Option<Receiver<Result<Vec<String>, String>>>,
    blame_receiver: Option<Receiver<BlameResult>>,
    compare_receiver: Option<Receiver<CompareResult>>,
    page_receiver: Option<Receiver<Result<GitLogPage, String>>>,
    details_receiver: Option<Receiver<(String, Result<GitCommitDetails, String>)>>,
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
    pub(crate) fn open_history_dialog(&mut self, ctx: &egui::Context) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if self.view.history_dialog.is_some() {
            return;
        }
        self.view.history_dialog = Some(HistoryDialog {
            root,
            ..Default::default()
        });
        self.load_history_page(ctx);
        self.request_history_refs(ctx);
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
        dialog.details = None;
        dialog.compare = None;
        dialog.blame = None;
        dialog.error = None;
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
        self.open_history_dialog(ctx);
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
        self.open_history_dialog(ctx);
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
    }

    pub(crate) fn select_commit(&mut self, id: String, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
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
            .commits
            .iter()
            .find(|commit| Some(&commit.id) == dialog.selected.as_ref())
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
                        }
                        Err(error) => dialog.error = Some(error),
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
