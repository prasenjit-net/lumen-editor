//! Asynchronous, read-only Git status and diff requests.
use super::{LumenApp, view_state::Activity};
use crate::services::source_control::{DiffSide, GitComparison, GitSnapshot};
use crate::{app::view_state::ViewState, model::EditorState};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

pub(crate) struct SourceControlJob {
    receiver: Receiver<GitResult>,
    /// Mutations are never replaced or overlapped: dropping the receiver would
    /// hide the outcome of work that is still running.
    mutating: bool,
}

const SOURCE_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// Closing a diff discards only a clean document opened just for that preview.
/// A dirty document becomes a regular tab so unsaved work stays reachable.
pub(crate) fn close_diff_preview(editor: &mut EditorState, view: &mut ViewState) {
    let Some(diff) = view.source_control.diff.take() else {
        return;
    };
    if let Some(id) = diff.diff_only_document
        && let Some(index) = editor
            .files
            .iter()
            .position(|file| file.id == id && !file.dirty)
    {
        view.find.remove(&id);
        editor.close_file(index);
    }
    if editor.active_index().is_none() && !editor.files.is_empty() {
        editor.set_active_index(Some(editor.files.len() - 1));
    }
}

enum GitResult {
    Snapshot(PathBuf, Result<Option<GitSnapshot>, String>),
    Diff {
        root: PathBuf,
        path: PathBuf,
        side: DiffSide,
        result: Result<GitComparison, String>,
    },
    Operation(Result<String, String>),
    Commit {
        result: Result<String, String>,
        message: String,
    },
    Revert {
        paths: Vec<PathBuf>,
        result: Result<String, String>,
    },
}

impl LumenApp {
    pub(crate) fn ensure_source_control(&mut self, ctx: &egui::Context) {
        if self.view.activity != Activity::SourceControl || !self.view.sidebar_visible {
            self.view.source_control.commit_focused = false;
            return;
        }
        if self.view.source_control.workspace != self.editor.workspace {
            self.refresh_source_control(ctx);
        } else if !self.view.source_control.commit_focused && self.source_control_job.is_none() {
            // Git has no portable notification for external index and worktree
            // changes. Poll only while this pane is visible and not being edited.
            let elapsed = self
                .view
                .source_control
                .last_refresh
                .map_or(SOURCE_REFRESH_INTERVAL, |last| last.elapsed());
            if elapsed >= SOURCE_REFRESH_INTERVAL {
                self.refresh_source_control(ctx);
            } else {
                ctx.request_repaint_after(SOURCE_REFRESH_INTERVAL - elapsed);
            }
        }
    }

    /// True while a Git mutation is running. Reads must wait for its result
    /// (which refreshes the view itself) instead of replacing the job.
    fn source_mutation_in_flight(&self) -> bool {
        self.source_control_job
            .as_ref()
            .is_some_and(|job| job.mutating)
    }

    /// Claims the single mutation slot, or explains why the action is refused.
    fn begin_source_mutation(&mut self) -> bool {
        if self.source_mutation_in_flight() {
            self.view.status = "Wait for the current Git operation to finish".into();
            return false;
        }
        self.source_control_job = None;
        self.view.source_control.loading = true;
        true
    }

    pub(crate) fn refresh_source_control(&mut self, ctx: &egui::Context) {
        if self.source_mutation_in_flight() {
            return;
        }
        self.source_control_job = None;
        let workspace_changed = self.view.source_control.workspace != self.editor.workspace;
        if workspace_changed {
            close_diff_preview(&mut self.editor, &mut self.view);
        }
        let view = &mut self.view.source_control;
        view.workspace = self.editor.workspace.clone();
        // Periodic refresh must not blank the panel while Git runs. Keeping the
        // last coherent snapshot also keeps virtual rows and hover state stable.
        if workspace_changed {
            view.snapshot = None;
        }
        view.error = None;
        view.last_refresh = Some(std::time::Instant::now());
        let Some(workspace) = view.workspace.clone() else {
            view.loading = false;
            return;
        };
        view.loading = true;
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = service.snapshot(&workspace);
            let _ = sender.send(GitResult::Snapshot(workspace, result));
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: false,
        });
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    pub(crate) fn stage_source_paths(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        self.run_source_operation(paths, ctx, "Staged changes", |service, root, paths| {
            service.stage(root, paths)
        });
    }

    pub(crate) fn unstage_source_paths(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        self.run_source_operation(paths, ctx, "Unstaged changes", |service, root, paths| {
            service.unstage(root, paths)
        });
    }

    /// Pulls only when Lumen has no unsaved workspace buffers. A fast-forward
    /// pull may replace files on disk, so an in-memory dirty buffer must never
    /// be left silently diverged from the repository.
    pub(crate) fn pull_source_control(&mut self, ctx: &egui::Context) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if self.editor.files.iter().any(|file| {
            file.dirty
                && file
                    .path
                    .as_ref()
                    .is_some_and(|path| path.starts_with(&root))
        }) {
            self.view.source_control.error = Some(
                "Save or discard unsaved workspace edits before pulling remote changes".into(),
            );
            return;
        }
        self.run_source_remote_operation(ctx, "Pulled latest changes", |service, root| {
            service.pull(root)
        });
    }

    pub(crate) fn push_source_control(&mut self, ctx: &egui::Context) {
        self.run_source_remote_operation(ctx, "Pushed local commits", |service, root| {
            service.push(root)
        });
    }

    pub(crate) fn request_source_revert(&mut self, paths: Vec<PathBuf>) {
        let Some(snapshot) = &self.view.source_control.snapshot else {
            return;
        };
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|path| source_path_is_revertible(snapshot, path))
            .collect();
        if paths.is_empty() {
            self.view.source_control.error = Some(
                "Only tracked files already present in HEAD can be discarded. Untracked files are kept."
                    .into(),
            );
            return;
        }
        self.view.source_control.revert_prompt =
            Some(super::view_state::SourceRevertPrompt { paths, error: None });
    }

    pub(crate) fn confirm_source_revert(&mut self, ctx: &egui::Context) {
        let in_flight = self.source_mutation_in_flight();
        let Some(prompt) = &mut self.view.source_control.revert_prompt else {
            return;
        };
        let Some(snapshot) = &self.view.source_control.snapshot else {
            prompt.error =
                Some("Repository status is no longer available; refresh and try again".into());
            return;
        };
        let paths = prompt.paths.clone();
        if self.editor.files.iter().any(|file| {
            file.dirty
                && file.path.as_ref().is_some_and(|path| {
                    path.strip_prefix(&snapshot.root)
                        .is_ok_and(|relative| paths.contains(&relative.to_path_buf()))
                })
        }) {
            prompt.error =
                Some("Save or discard the open editor changes before discarding this file".into());
            return;
        }
        let root = snapshot.root.clone();
        if in_flight {
            prompt.error = Some("Wait for the current Git operation to finish".into());
            return;
        }
        self.source_control_job = None;
        self.view.source_control.loading = true;
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = service
                .revert(&root, &paths)
                .map(|_| "Discarded source-control changes".into());
            let _ = sender.send(GitResult::Revert { paths, result });
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
    }

    pub(crate) fn stage_source_hunk(&mut self, path: PathBuf, hunk: usize, ctx: &egui::Context) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        let Some(diff) = self.view.source_control.diff.as_ref() else {
            return;
        };
        if diff.path != path || diff.side != DiffSide::Working {
            return;
        }
        if diff.document.is_some_and(|id| {
            self.editor
                .files
                .iter()
                .any(|file| file.id == id && file.dirty)
        }) {
            self.view.status = "Save or discard editor edits before staging a hunk".into();
            return;
        }
        let Some(expected) = diff
            .comparison
            .as_ref()
            .ok()
            .and_then(|comparison| comparison.hunks.get(hunk))
            .map(|hunk| hunk.patch.clone())
        else {
            return;
        };
        if !self.begin_source_mutation() {
            return;
        }
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = service
                .stage_hunk(&root, &root.join(&path), hunk, &expected)
                .map(|_| "Staged hunk".into());
            let _ = sender.send(GitResult::Operation(result));
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
    }

    pub(crate) fn commit_source_control(&mut self, ctx: &egui::Context) {
        let message = self.view.source_control.commit_message.trim().to_owned();
        let amend = self.view.source_control.amend_commit;
        if message.is_empty() && !amend {
            self.view.source_control.commit_error = Some("Enter a commit message".into());
            self.view.source_control.commit_focus = true;
            return;
        }
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if !self.begin_source_mutation() {
            return;
        }
        self.view.source_control.commit_error = None;
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = service.commit(&root, &message, amend).map(|_| {
                if amend {
                    "Amended last commit".into()
                } else {
                    "Committed changes".into()
                }
            });
            let _ = sender.send(GitResult::Commit { result, message });
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
    }

    fn run_source_operation<F>(
        &mut self,
        paths: Vec<PathBuf>,
        ctx: &egui::Context,
        success: &'static str,
        operation: F,
    ) where
        F: FnOnce(
                &dyn crate::services::source_control::SourceControlService,
                &std::path::Path,
                &[PathBuf],
            ) -> Result<(), String>
            + Send
            + 'static,
    {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if !self.begin_source_mutation() {
            return;
        }
        self.view.source_control.error = None;
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = operation(service.as_ref(), &root, &paths).map(|_| success.into());
            let _ = sender.send(GitResult::Operation(result));
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
    }

    fn run_source_remote_operation<F>(
        &mut self,
        ctx: &egui::Context,
        success: &'static str,
        operation: F,
    ) where
        F: FnOnce(
                &dyn crate::services::source_control::SourceControlService,
                &std::path::Path,
            ) -> Result<(), String>
            + Send
            + 'static,
    {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if !self.begin_source_mutation() {
            return;
        }
        self.view.source_control.error = None;
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = operation(service.as_ref(), &root).map(|_| success.into());
            let _ = sender.send(GitResult::Operation(result));
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
    }

    pub(crate) fn show_source_diff(&mut self, path: PathBuf, side: DiffSide, ctx: &egui::Context) {
        let Some(snapshot) = &self.view.source_control.snapshot else {
            return;
        };
        if !snapshot.changes.iter().any(|change| {
            change.path == path
                && match side {
                    DiffSide::Staged => change.index != ' ' && change.index != '?',
                    DiffSide::Working => change.worktree != ' ' && change.worktree != '?',
                    DiffSide::Untracked => change.index == '?' && change.worktree == '?',
                }
        }) {
            return;
        }
        if self.source_mutation_in_flight() {
            return;
        }
        let root = snapshot.root.clone();
        self.source_control_job = None;
        self.view.source_control.loading = true;
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = service.comparison(&root, &root.join(&path), side);
            let _ = sender.send(GitResult::Diff {
                root,
                path,
                side,
                result,
            });
            worker_ctx.request_repaint();
        });
        self.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: false,
        });
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    pub(crate) fn poll_source_control(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.source_control_job else {
            return;
        };
        match job.receiver.try_recv() {
            Ok(GitResult::Snapshot(workspace, result)) => {
                if self.view.source_control.workspace.as_ref() == Some(&workspace) {
                    match result {
                        Ok(snapshot) => self.view.source_control.snapshot = snapshot,
                        Err(error) => self.view.source_control.error = Some(error),
                    }
                    self.view.source_control.loading = false;
                }
                self.source_control_job = None;
            }
            Ok(GitResult::Diff {
                root,
                path,
                side,
                result,
            }) => {
                self.finish_source_diff(root, path, side, result);
                self.source_control_job = None;
            }
            Ok(GitResult::Operation(result)) => {
                self.view.source_control.loading = false;
                self.source_control_job = None;
                self.reconcile_clean_documents();
                match result {
                    Ok(status) => {
                        self.view.status = status;
                        self.refresh_source_control(ctx);
                    }
                    Err(error) => {
                        self.view.source_control.error = Some(error);
                    }
                }
            }
            Ok(GitResult::Commit { result, message }) => {
                self.view.source_control.loading = false;
                self.source_control_job = None;
                self.reconcile_clean_documents();
                match result {
                    Ok(status) => {
                        self.view.status = status;
                        if self.view.source_control.commit_message.trim() == message {
                            self.view.source_control.commit_message.clear();
                            self.view.source_control.amend_commit = false;
                        }
                        self.view.source_control.commit_error = None;
                        self.refresh_source_control(ctx);
                    }
                    Err(error) => self.view.source_control.commit_error = Some(error),
                }
            }
            Ok(GitResult::Revert { paths, result }) => {
                self.view.source_control.loading = false;
                self.source_control_job = None;
                match result {
                    Ok(status) => {
                        self.reload_reverted_documents(&paths);
                        close_diff_preview(&mut self.editor, &mut self.view);
                        self.view.source_control.revert_prompt = None;
                        self.view.status = status;
                        self.refresh_source_control(ctx);
                    }
                    Err(error) => {
                        if let Some(prompt) = &mut self.view.source_control.revert_prompt {
                            prompt.error = Some(error);
                        } else {
                            self.view.source_control.error = Some(error);
                        }
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.view.source_control.loading = false;
                self.view.source_control.error = Some("Git request stopped unexpectedly".into());
                self.source_control_job = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    fn finish_source_diff(
        &mut self,
        root: PathBuf,
        path: PathBuf,
        side: DiffSide,
        mut result: Result<GitComparison, String>,
    ) {
        if !self
            .view
            .source_control
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.root == root)
        {
            return;
        }
        close_diff_preview(&mut self.editor, &mut self.view);
        let mut document = None;
        let mut diff_only_document = None;
        let mut opened_revision = None;
        if side != DiffSide::Staged && result.is_ok() {
            let full_path = root.join(&path);
            if full_path.is_file() {
                let was_open = self
                    .editor
                    .files
                    .iter()
                    .any(|file| file.path.as_deref() == Some(full_path.as_path()));
                self.open_file(full_path.clone());
                document = self
                    .editor
                    .active()
                    .filter(|file| file.path.as_deref() == Some(full_path.as_path()))
                    .map(|file| file.id);
                if !was_open {
                    diff_only_document = document;
                }
                opened_revision = self
                    .editor
                    .active()
                    .filter(|file| {
                        Some(file.id) == document
                            && result.as_ref().is_ok_and(|comparison| {
                                comparison.changed.as_deref() == Some(file.text.as_str())
                            })
                    })
                    .map(|file| file.revision);
                if document.is_none() {
                    result = Err("Could not open the changed file for editing".into());
                }
            } else if let Ok(comparison) = &mut result {
                // A deleted working file has an empty, read-only right side.
                comparison.changed = Some(String::new());
            }
        }
        self.view.leave_page_tabs();
        self.view.source_control.diff = Some(super::view_state::SourceDiffView {
            root,
            path,
            side,
            comparison: result.map(std::sync::Arc::new),
            document,
            diff_only_document,
            opened_revision,
            active: true,
        });
        if side == DiffSide::Staged {
            self.editor.set_active_index(None);
        }
        self.view.source_control.loading = false;
    }

    /// Brings clean open documents in line with the disk after a Git operation
    /// (pull, hook rewrites, partial failures). Dirty buffers are never touched;
    /// the regular changed-on-disk check flags them.
    fn reconcile_clean_documents(&mut self) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        for file in &mut self.editor.files {
            let Some(path) = &file.path else { continue };
            if file.dirty || !path.starts_with(&root) {
                continue;
            }
            if let Ok(text) = self.file_system.read_text(path)
                && text != file.text
            {
                file.reload_from_disk(text);
            }
        }
    }

    fn reload_reverted_documents(&mut self, paths: &[PathBuf]) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| &snapshot.root)
        else {
            return;
        };
        for file in &mut self.editor.files {
            let Some(path) = &file.path else { continue };
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            if !paths.contains(&relative.to_path_buf()) {
                continue;
            }
            match self.file_system.read_text(path) {
                Ok(text) => file.reload_from_disk(text),
                Err(error) => {
                    self.view.status = format!(
                        "Discarded changes, but could not reload {}: {error}",
                        path.display()
                    );
                }
            }
        }
    }
}

fn source_path_is_revertible(snapshot: &GitSnapshot, path: &std::path::Path) -> bool {
    snapshot.changes.iter().any(|change| {
        change.path == path
            && change.index != '?'
            && change.index != 'A'
            && (change.index != ' ' || change.worktree != ' ')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::command::Command,
        model::Document,
        services::source_control::{GitChange, GitSnapshot},
    };

    fn comparison() -> GitComparison {
        GitComparison {
            base: "saved\n".into(),
            changed: None,
            old_lines: [1].into(),
            new_lines: [1].into(),
            hunks: Vec::new(),
        }
    }

    fn app_with_change(root: &std::path::Path, path: &std::path::Path) -> LumenApp {
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.to_path_buf());
        app.view.source_control.workspace = Some(root.to_path_buf());
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: root.to_path_buf(),
            branch: "main".into(),
            changes: vec![GitChange {
                path: path.to_path_buf(),
                index: 'M',
                worktree: 'M',
            }],
        });
        app
    }

    #[test]
    fn staging_does_not_clear_commit_draft() {
        let mut app = app_with_change(
            std::path::Path::new("/workspace"),
            std::path::Path::new("src/main.rs"),
        );
        app.view.source_control.commit_message = "Keep this draft".into();
        app.view.source_control.amend_commit = true;
        let (sender, receiver) = mpsc::channel();
        sender
            .send(GitResult::Operation(Ok("Staged changes".into())))
            .unwrap();
        app.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: false,
        });
        app.poll_source_control(&egui::Context::default());
        assert_eq!(app.view.source_control.commit_message, "Keep this draft");
        assert!(app.view.source_control.amend_commit);
    }

    #[test]
    fn automatic_refresh_waits_while_commit_message_is_focused() {
        let mut app = app_with_change(
            std::path::Path::new("/workspace"),
            std::path::Path::new("src/main.rs"),
        );
        app.view.activity = Activity::SourceControl;
        app.view.sidebar_visible = true;
        app.view.source_control.last_refresh =
            Some(std::time::Instant::now() - SOURCE_REFRESH_INTERVAL);
        app.view.source_control.commit_focused = true;
        let ctx = egui::Context::default();
        app.ensure_source_control(&ctx);
        assert!(app.source_control_job.is_none());

        app.view.source_control.commit_focused = false;
        app.ensure_source_control(&ctx);
        assert!(app.source_control_job.is_some());
    }

    #[test]
    fn a_running_mutation_is_not_replaced_by_refresh_diff_or_another_mutation() {
        let root = std::path::Path::new("/workspace");
        let mut app = app_with_change(root, std::path::Path::new("src/main.rs"));
        let (sender, receiver) = mpsc::channel();
        app.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
        let ctx = egui::Context::default();
        app.refresh_source_control(&ctx);
        app.show_source_diff("src/main.rs".into(), DiffSide::Working, &ctx);
        app.stage_source_paths(vec!["src/main.rs".into()], &ctx);
        assert_eq!(
            app.view.status,
            "Wait for the current Git operation to finish"
        );
        // The original job is still the one polled, so its result is not lost.
        sender
            .send(GitResult::Operation(Ok("Committed changes".into())))
            .unwrap();
        app.poll_source_control(&ctx);
        assert_eq!(app.view.status, "Committed changes");
    }

    #[test]
    fn clean_documents_reload_after_an_operation_but_dirty_ones_are_kept() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        std::fs::write(root.join("clean.txt"), "new from pull\n").unwrap();
        std::fs::write(root.join("dirty.txt"), "new from pull\n").unwrap();
        let mut app = app_with_change(&root, std::path::Path::new("clean.txt"));
        app.editor
            .files
            .push(Document::from_path(root.join("clean.txt"), "old\n".into()));
        let mut dirty = Document::from_path(root.join("dirty.txt"), "old\n".into());
        dirty.text = "my edit\n".into();
        dirty.record_text_change();
        app.editor.files.push(dirty);
        let (sender, receiver) = mpsc::channel();
        app.source_control_job = Some(SourceControlJob {
            receiver,
            mutating: true,
        });
        sender
            .send(GitResult::Operation(Err("partial failure".into())))
            .unwrap();
        app.poll_source_control(&egui::Context::default());
        assert_eq!(app.editor.files[0].text, "new from pull\n");
        assert!(!app.editor.files[0].dirty);
        assert_eq!(app.editor.files[1].text, "my edit\n");
        assert!(app.editor.files[1].dirty);
    }

    #[test]
    fn pull_rejects_unsaved_workspace_documents() {
        let root = std::path::Path::new("/workspace");
        let mut app = app_with_change(root, std::path::Path::new("src/main.rs"));
        let mut document = Document::from_path(root.join("src/main.rs"), "saved".into());
        document.text = "unsaved".into();
        document.record_text_change();
        app.editor.files.push(document);

        app.pull_source_control(&egui::Context::default());

        assert!(app.source_control_job.is_none());
        assert_eq!(
            app.view.source_control.error.as_deref(),
            Some("Save or discard unsaved workspace edits before pulling remote changes")
        );
    }

    #[test]
    fn unstage_all_keeps_working_changes_visible() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
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
        std::fs::write(root.join("one.rs"), "original\n").unwrap();
        git(&["add", "--", "one.rs"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ]);
        std::fs::write(root.join("one.rs"), "changed\n").unwrap();
        std::fs::write(root.join("two.rs"), "new\n").unwrap();
        git(&["add", "--", "one.rs", "two.rs"]);

        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.to_path_buf());
        app.view.source_control.workspace = Some(root.to_path_buf());
        app.view.source_control.snapshot = app.source_control_service.snapshot(root).unwrap();
        app.view.source_control.commit_message = "Draft stays here".into();
        let paths = app
            .view
            .source_control
            .snapshot
            .as_ref()
            .unwrap()
            .changes
            .iter()
            .map(|change| change.path.clone())
            .collect();
        let ctx = egui::Context::default();
        app.unstage_source_paths(paths, &ctx);
        // Completing an operation schedules a second asynchronous status refresh.
        // Windows Git process startup is slower than the old one-second polling
        // window, so wait for the complete operation within a bounded deadline.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.source_control_job.is_some() && std::time::Instant::now() < deadline {
            app.poll_source_control(&ctx);
            if app.source_control_job.is_some() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(app.source_control_job.is_none(), "Git operation timed out");
        assert!(app.view.source_control.error.is_none());
        let snapshot = app.view.source_control.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.changes.len(), 2);
        assert!(
            snapshot
                .changes
                .iter()
                .all(|change| change.index == ' ' || change.index == '?')
        );
        assert!(snapshot.changes.iter().any(|change| change.worktree == 'M'));
        assert!(snapshot.changes.iter().any(|change| change.index == '?'));
        assert_eq!(app.view.source_control.commit_message, "Draft stays here");
    }

    #[test]
    fn only_head_backed_tracked_changes_can_be_discarded() {
        let root = tempfile::tempdir().unwrap();
        let snapshot = GitSnapshot {
            root: root.path().to_path_buf(),
            branch: "main".into(),
            changes: vec![
                GitChange {
                    path: "modified.rs".into(),
                    index: ' ',
                    worktree: 'M',
                },
                GitChange {
                    path: "staged.rs".into(),
                    index: 'M',
                    worktree: ' ',
                },
                GitChange {
                    path: "added.rs".into(),
                    index: 'A',
                    worktree: ' ',
                },
                GitChange {
                    path: "new.rs".into(),
                    index: '?',
                    worktree: '?',
                },
            ],
        };
        assert!(source_path_is_revertible(
            &snapshot,
            std::path::Path::new("modified.rs")
        ));
        assert!(source_path_is_revertible(
            &snapshot,
            std::path::Path::new("staged.rs")
        ));
        assert!(!source_path_is_revertible(
            &snapshot,
            std::path::Path::new("added.rs")
        ));
        assert!(!source_path_is_revertible(
            &snapshot,
            std::path::Path::new("new.rs")
        ));
    }

    #[test]
    fn working_comparison_reuses_dirty_document_without_changing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.rs");
        std::fs::write(&path, "on disk\n").unwrap();
        let mut app = app_with_change(directory.path(), std::path::Path::new("file.rs"));
        let mut document = Document::from_path(path, "on disk\n".into());
        document.text = "unsaved edit\n".into();
        document.record_text_change();
        let id = document.id;
        app.editor.files.push(document);

        app.finish_source_diff(
            directory.path().to_path_buf(),
            PathBuf::from("file.rs"),
            DiffSide::Working,
            Ok(comparison()),
        );

        assert_eq!(app.editor.files.len(), 1);
        assert_eq!(app.editor.files[0].text, "unsaved edit\n");
        assert!(app.editor.files[0].dirty);
        let diff = app.view.source_control.diff.as_ref().unwrap();
        assert_eq!(diff.document, Some(id));
        assert_eq!(diff.diff_only_document, None);
        assert_eq!(diff.opened_revision, None);
        assert!(diff.active);
    }

    #[test]
    fn staged_comparison_is_read_only_and_does_not_open_document() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_with_change(directory.path(), std::path::Path::new("file.rs"));
        app.finish_source_diff(
            directory.path().to_path_buf(),
            PathBuf::from("file.rs"),
            DiffSide::Staged,
            Ok(comparison()),
        );
        assert!(app.editor.files.is_empty());
        let diff = app.view.source_control.diff.as_ref().unwrap();
        assert_eq!(diff.document, None);
        assert_eq!(diff.diff_only_document, None);
        assert!(diff.active);
    }

    #[test]
    fn refresh_preserves_open_comparison_in_same_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = app_with_change(directory.path(), std::path::Path::new("file.rs"));
        app.finish_source_diff(
            directory.path().to_path_buf(),
            PathBuf::from("file.rs"),
            DiffSide::Staged,
            Ok(comparison()),
        );
        let previous = app.view.source_control.snapshot.clone().unwrap();
        app.refresh_source_control(&egui::Context::default());
        assert!(app.view.source_control.diff.is_some());
        assert_eq!(
            app.view.source_control.snapshot.as_ref().unwrap().changes,
            previous.changes
        );
    }

    #[test]
    fn git_open_is_diff_only_until_explorer_opens_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.rs");
        std::fs::write(&path, "on disk\n").unwrap();
        let mut app = app_with_change(directory.path(), std::path::Path::new("file.rs"));
        app.finish_source_diff(
            directory.path().to_path_buf(),
            PathBuf::from("file.rs"),
            DiffSide::Working,
            Ok(comparison()),
        );
        let id = app.editor.active().unwrap().id;
        assert_eq!(
            app.view
                .source_control
                .diff
                .as_ref()
                .unwrap()
                .diff_only_document,
            Some(id)
        );
        app.open_file(path);
        assert_eq!(app.editor.files.len(), 1);
        let diff = app.view.source_control.diff.as_ref().unwrap();
        assert_eq!(diff.diff_only_document, None);
        assert!(!diff.active);
    }

    #[test]
    fn closing_diff_drops_clean_preview_but_keeps_dirty_edit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file.rs");
        std::fs::write(&path, "on disk\n").unwrap();
        let mut app = app_with_change(directory.path(), std::path::Path::new("file.rs"));
        let open = |app: &mut LumenApp| {
            app.finish_source_diff(
                directory.path().to_path_buf(),
                PathBuf::from("file.rs"),
                DiffSide::Working,
                Ok(comparison()),
            );
        };
        open(&mut app);
        close_diff_preview(&mut app.editor, &mut app.view);
        assert!(app.editor.files.is_empty());
        open(&mut app);
        let file = app.editor.files.last_mut().unwrap();
        file.text.push_str("unsaved\n");
        file.record_text_change();
        close_diff_preview(&mut app.editor, &mut app.view);
        assert_eq!(app.editor.files.len(), 1);
        assert!(app.editor.files[0].dirty);
        assert!(app.view.source_control.diff.is_none());
    }

    #[test]
    fn close_editor_command_closes_diff_tab_without_discarding_dirty_buffer() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("file.rs"), "on disk\n").unwrap();
        let mut app = app_with_change(directory.path(), std::path::Path::new("file.rs"));
        app.finish_source_diff(
            directory.path().to_path_buf(),
            PathBuf::from("file.rs"),
            DiffSide::Working,
            Ok(comparison()),
        );
        let document = app.editor.files.last_mut().unwrap();
        document.text.push_str("unsaved\n");
        document.record_text_change();
        app.execute(Command::CloseEditor, &egui::Context::default());
        assert!(app.view.source_control.diff.is_none());
        assert!(app.view.pending_close.is_none());
        assert!(app.editor.active().unwrap().dirty);
    }
}
