//! Stash dialog workflow: create, inspect, apply, pop and drop. Entries are
//! addressed by commit id so another stash operation cannot redirect an action,
//! and no action runs while editor text under the repository is unsaved
//! (a stash only ever covers what is on disk).
use super::LumenApp;
use crate::services::source_control::GitStash;
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

type StashFiles = (String, Result<Vec<(String, String)>, String>);

#[derive(Default)]
pub(crate) struct StashDialog {
    pub(crate) root: PathBuf,
    pub(crate) stashes: Vec<GitStash>,
    pub(crate) loading: bool,
    pub(crate) message: String,
    pub(crate) include_untracked: bool,
    pub(crate) restore_index: bool,
    pub(crate) selected: Option<String>,
    pub(crate) files: Vec<(String, String)>,
    pub(crate) confirm_drop: Option<String>,
    pub(crate) error: Option<String>,
    list_receiver: Option<Receiver<Result<Vec<GitStash>, String>>>,
    files_receiver: Option<Receiver<StashFiles>>,
}

impl LumenApp {
    pub(crate) fn open_stash_dialog(&mut self, ctx: &egui::Context) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if self.view.stash_dialog.is_some() {
            return;
        }
        self.view.stash_dialog = Some(StashDialog {
            root,
            ..Default::default()
        });
        self.request_stashes(ctx);
    }

    pub(crate) fn close_stash_dialog(&mut self) {
        self.view.stash_dialog = None;
    }

    pub(crate) fn request_stashes(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.stash_dialog else {
            return;
        };
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(service.stashes(&root));
            worker_ctx.request_repaint();
        });
        dialog.list_receiver = Some(receiver);
        dialog.loading = true;
    }

    pub(crate) fn select_stash(&mut self, id: String, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.stash_dialog else {
            return;
        };
        dialog.selected = Some(id.clone());
        dialog.files.clear();
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = service.stash_files(&root, &id);
            let _ = sender.send((id, result));
            worker_ctx.request_repaint();
        });
        dialog.files_receiver = Some(receiver);
    }

    pub(crate) fn poll_stashes(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.stash_dialog else {
            return;
        };
        if let Some(receiver) = &dialog.list_receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    dialog.list_receiver = None;
                    dialog.loading = false;
                    match result {
                        Ok(stashes) => {
                            dialog.stashes = stashes;
                            if dialog
                                .selected
                                .as_ref()
                                .is_some_and(|id| !dialog.stashes.iter().any(|s| &s.id == id))
                            {
                                dialog.selected = None;
                                dialog.files.clear();
                            }
                        }
                        Err(error) => dialog.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    dialog.list_receiver = None;
                    dialog.loading = false;
                }
            }
        }
        if let Some(receiver) = &dialog.files_receiver {
            match receiver.try_recv() {
                Ok((id, result)) => {
                    dialog.files_receiver = None;
                    // Ignore a slow answer for a stash that is no longer selected.
                    if dialog.selected.as_ref() == Some(&id) {
                        match result {
                            Ok(files) => dialog.files = files,
                            Err(error) => dialog.error = Some(error),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => dialog.files_receiver = None,
            }
        }
    }

    fn stash_blocker(&self) -> Option<String> {
        let root = &self.view.stash_dialog.as_ref()?.root;
        self.editor
            .files
            .iter()
            .any(|file| {
                file.dirty
                    && file
                        .path
                        .as_ref()
                        .is_some_and(|path| path.starts_with(root))
            })
            .then(|| {
                "Save or discard unsaved edits first: stashes only cover what is saved on disk"
                    .to_owned()
            })
    }

    fn set_stash_error(&mut self, error: String) {
        if let Some(dialog) = &mut self.view.stash_dialog {
            dialog.error = Some(error);
        }
    }

    pub(crate) fn create_stash(&mut self, ctx: &egui::Context) {
        if let Some(blocker) = self.stash_blocker() {
            self.set_stash_error(blocker);
            return;
        }
        let Some(dialog) = &mut self.view.stash_dialog else {
            return;
        };
        dialog.error = None;
        let message = std::mem::take(&mut dialog.message);
        let untracked = dialog.include_untracked;
        self.run_source_repo_operation(ctx, "Stashed changes", move |service, root| {
            service.stash_push(root, &message, untracked)
        });
    }

    pub(crate) fn apply_stash(&mut self, id: String, pop: bool, ctx: &egui::Context) {
        if let Some(blocker) = self.stash_blocker() {
            self.set_stash_error(blocker);
            return;
        }
        let Some(dialog) = &mut self.view.stash_dialog else {
            return;
        };
        dialog.error = None;
        let restore_index = dialog.restore_index;
        let label = if pop {
            "Applied and removed stash"
        } else {
            "Applied stash"
        };
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.stash_apply(root, &id, restore_index, pop)
        });
    }

    pub(crate) fn request_stash_drop(&mut self, id: String) {
        if let Some(dialog) = &mut self.view.stash_dialog {
            dialog.confirm_drop = Some(id);
            dialog.error = None;
        }
    }

    pub(crate) fn confirm_stash_drop(&mut self, ctx: &egui::Context) {
        let Some(id) = self
            .view
            .stash_dialog
            .as_mut()
            .and_then(|dialog| dialog.confirm_drop.take())
        else {
            return;
        };
        self.run_source_repo_operation(ctx, "Dropped stash", move |service, root| {
            service.stash_drop(root, &id)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;
    use crate::services::source_control::GitSnapshot;
    use std::process::Command;

    fn app_with_dialog(root: &std::path::Path) -> LumenApp {
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.to_path_buf());
        app.view.source_control.workspace = Some(root.to_path_buf());
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: root.to_path_buf(),
            ..Default::default()
        });
        app.view.stash_dialog = Some(StashDialog {
            root: root.to_path_buf(),
            ..Default::default()
        });
        app
    }

    #[test]
    fn unsaved_editor_text_blocks_every_stash_action() {
        let ctx = egui::Context::default();
        let mut app = app_with_dialog(std::path::Path::new("/repo"));
        let mut document = Document::from_path("/repo/a.rs".into(), "saved".into());
        document.text = "typed".into();
        document.record_text_change();
        app.editor.files.push(document);
        app.create_stash(&ctx);
        app.apply_stash("0".repeat(40), true, &ctx);
        assert!(app.source_control_job.is_none());
        let error = app
            .view
            .stash_dialog
            .as_ref()
            .unwrap()
            .error
            .clone()
            .unwrap();
        assert!(error.contains("only cover what is saved"), "{error}");
        app.request_stash_drop("abc".into());
        assert!(app.view.stash_dialog.as_ref().unwrap().error.is_none());
        assert!(
            app.source_control_job.is_none(),
            "drop waits for confirmation"
        );
    }

    #[test]
    fn stashing_and_popping_through_the_controller_round_trips_files() {
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
        std::fs::write(root.join("f.txt"), "base\n").unwrap();
        git(&["add", "--", "f.txt"]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(root.join("f.txt"), "wip\n").unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.view.source_control.workspace = Some(root.clone());
        app.view.source_control.snapshot = app.source_control_service.snapshot(&root).unwrap();
        let ctx = egui::Context::default();
        app.open_stash_dialog(&ctx);
        let wait = |app: &mut LumenApp| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while (app.source_control_job.is_some()
                || app
                    .view
                    .stash_dialog
                    .as_ref()
                    .is_some_and(|d| d.loading || d.files_receiver.is_some()))
                && std::time::Instant::now() < deadline
            {
                app.poll_source_control(&ctx);
                app.poll_stashes(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&mut app);
        app.view.stash_dialog.as_mut().unwrap().message = "my work".into();
        app.create_stash(&ctx);
        wait(&mut app);
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "base\n"
        );
        let stash = app.view.stash_dialog.as_ref().unwrap().stashes[0].clone();
        assert!(stash.message.contains("my work"));
        app.select_stash(stash.id.clone(), &ctx);
        wait(&mut app);
        assert_eq!(app.view.stash_dialog.as_ref().unwrap().files[0].1, "f.txt");
        app.apply_stash(stash.id, true, &ctx);
        wait(&mut app);
        assert_eq!(
            std::fs::read_to_string(root.join("f.txt")).unwrap(),
            "wip\n"
        );
        assert!(app.view.stash_dialog.as_ref().unwrap().stashes.is_empty());
    }
}
