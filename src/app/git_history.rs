//! History dialog: a paginated, read-only commit list with details. Pages are
//! anchored to the commit that was HEAD when the list opened, and every read
//! runs on a worker so nothing here touches the index or worktree.
use super::LumenApp;
use crate::services::source_control::{GitCommit, GitCommitDetails, GitLogPage};
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
    pub(crate) error: Option<String>,
    page_receiver: Option<Receiver<Result<GitLogPage, String>>>,
    details_receiver: Option<Receiver<(String, Result<GitCommitDetails, String>)>>,
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
        let skip = dialog.commits.len();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(service.log(&root, anchor.as_deref(), skip, HISTORY_PAGE));
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
        dialog.details = None;
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = service.commit_details(&root, &id);
            let _ = sender.send((id, result));
            worker_ctx.request_repaint();
        });
        dialog.details_receiver = Some(receiver);
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
            while app
                .view
                .history_dialog
                .as_ref()
                .is_some_and(|d| d.loading || d.details_receiver.is_some())
                && std::time::Instant::now() < deadline
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
        assert_eq!(details.files[0].1, "f.txt");
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
