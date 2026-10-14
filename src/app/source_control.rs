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
}

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
    if editor.active_file.is_none() && !editor.files.is_empty() {
        editor.active_file = Some(editor.files.len() - 1);
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
}

impl LumenApp {
    pub(crate) fn ensure_source_control(&mut self, ctx: &egui::Context) {
        if self.view.activity != Activity::SourceControl || !self.view.sidebar_visible {
            return;
        }
        if self.view.source_control.workspace != self.editor.workspace {
            self.refresh_source_control(ctx);
        }
    }

    pub(crate) fn refresh_source_control(&mut self, ctx: &egui::Context) {
        self.source_control_job = None;
        if self.view.source_control.workspace != self.editor.workspace {
            close_diff_preview(&mut self.editor, &mut self.view);
        }
        let view = &mut self.view.source_control;
        view.workspace = self.editor.workspace.clone();
        view.snapshot = None;
        view.error = None;
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
        self.source_control_job = Some(SourceControlJob { receiver });
        ctx.request_repaint_after(Duration::from_millis(100));
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
        self.source_control_job = Some(SourceControlJob { receiver });
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
        self.view.settings.open = false;
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
            self.editor.active_file = None;
        }
        self.view.source_control.loading = false;
    }
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
        app.refresh_source_control(&egui::Context::default());
        assert!(app.view.source_control.diff.is_some());
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
