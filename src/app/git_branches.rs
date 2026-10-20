//! Branch picker workflow: list, switch, create, rename and delete branches.
//! Reads run on their own worker; changes go through the single Git mutation
//! slot so they cannot overlap other Git work.
use super::LumenApp;
use crate::services::source_control::GitBranch;
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

#[derive(Default)]
pub(crate) struct BranchDialog {
    pub(crate) root: PathBuf,
    pub(crate) branches: Vec<GitBranch>,
    pub(crate) loading: bool,
    pub(crate) filter: String,
    pub(crate) new_name: String,
    pub(crate) rename_to: String,
    /// A local branch awaiting delete confirmation.
    pub(crate) confirm_delete: Option<String>,
    pub(crate) error: Option<String>,
    receiver: Option<Receiver<Result<Vec<GitBranch>, String>>>,
}

impl LumenApp {
    pub(crate) fn open_branch_dialog(&mut self, ctx: &egui::Context) {
        let Some(root) = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
        else {
            return;
        };
        if self.view.branch_dialog.is_some() {
            return;
        }
        self.view.branch_dialog = Some(BranchDialog {
            root,
            ..Default::default()
        });
        self.request_branches(ctx);
    }

    pub(crate) fn close_branch_dialog(&mut self) {
        self.view.branch_dialog = None;
    }

    pub(crate) fn request_branches(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.branch_dialog else {
            return;
        };
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(service.branches(&root));
            worker_ctx.request_repaint();
        });
        // Replacing the receiver drops any older, slower request.
        dialog.receiver = Some(receiver);
        dialog.loading = true;
    }

    pub(crate) fn poll_branches(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.branch_dialog else {
            return;
        };
        let Some(receiver) = &dialog.receiver else {
            return;
        };
        match receiver.try_recv() {
            Ok(result) => {
                dialog.receiver = None;
                dialog.loading = false;
                match result {
                    Ok(branches) => {
                        dialog.branches = branches;
                        dialog.error = None;
                    }
                    Err(error) => dialog.error = Some(error),
                }
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(mpsc::TryRecvError::Disconnected) => {
                dialog.receiver = None;
                dialog.loading = false;
            }
        }
    }

    /// Why a branch change must wait: unsaved editor text under the repository
    /// (a switch rewrites files on disk).
    fn branch_change_blocker(&self) -> Option<String> {
        let root = &self.view.branch_dialog.as_ref()?.root;
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
            .then(|| "Save or discard unsaved edits in this repository before switching".into())
    }

    fn set_branch_error(&mut self, error: String) {
        if let Some(dialog) = &mut self.view.branch_dialog {
            dialog.error = Some(error);
        }
    }

    pub(crate) fn switch_to_branch(&mut self, name: String, remote: bool, ctx: &egui::Context) {
        let Some(dialog) = &self.view.branch_dialog else {
            return;
        };
        let Some(branch) = dialog
            .branches
            .iter()
            .find(|branch| branch.name == name && branch.remote == remote)
        else {
            return;
        };
        if branch.current {
            self.close_branch_dialog();
            return;
        }
        if let Some(path) = &branch.checked_out_at {
            let message = format!(
                "{name} is checked out in another worktree: {}",
                path.display()
            );
            self.set_branch_error(message);
            return;
        }
        if let Some(blocker) = self.branch_change_blocker() {
            self.set_branch_error(blocker);
            return;
        }
        // A local branch of the same name as `origin/name` would be shadowed.
        let local = name.rsplit_once('/').map(|(_, short)| short.to_owned());
        if remote
            && local.as_ref().is_some_and(|short| {
                dialog
                    .branches
                    .iter()
                    .any(|other| !other.remote && &other.name == short)
            })
        {
            let message = format!(
                "A local branch named {} already exists; switch to it instead",
                local.unwrap_or_default()
            );
            self.set_branch_error(message);
            return;
        }
        self.close_branch_dialog();
        let label = format!("Switched to {name}");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.switch_branch(root, &name, remote)
        });
    }

    pub(crate) fn create_new_branch(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.view.branch_dialog else {
            return;
        };
        let name = dialog.new_name.trim().to_owned();
        if name.is_empty() {
            self.set_branch_error("Enter a name for the new branch".into());
            return;
        }
        if dialog.branches.iter().any(|b| !b.remote && b.name == name) {
            self.set_branch_error(format!("A branch named {name} already exists"));
            return;
        }
        self.close_branch_dialog();
        let label = format!("Created and switched to {name}");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.create_branch(root, &name)
        });
    }

    pub(crate) fn rename_current_branch(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.view.branch_dialog else {
            return;
        };
        let Some(current) = dialog
            .branches
            .iter()
            .find(|branch| branch.current && !branch.remote)
            .map(|branch| branch.name.clone())
        else {
            self.set_branch_error("Check out a local branch to rename it".into());
            return;
        };
        let to = dialog.rename_to.trim().to_owned();
        if to.is_empty() || to == current {
            self.set_branch_error("Enter a different name".into());
            return;
        }
        if dialog.branches.iter().any(|b| !b.remote && b.name == to) {
            self.set_branch_error(format!("A branch named {to} already exists"));
            return;
        }
        if let Some(dialog) = &mut self.view.branch_dialog {
            dialog.rename_to.clear();
        }
        let label = format!("Renamed {current} to {to}");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.rename_branch(root, &current, &to)
        });
    }

    pub(crate) fn request_branch_delete(&mut self, name: String) {
        let Some(dialog) = &mut self.view.branch_dialog else {
            return;
        };
        let deletable = dialog
            .branches
            .iter()
            .any(|b| !b.remote && !b.current && b.checked_out_at.is_none() && b.name == name);
        if deletable {
            dialog.confirm_delete = Some(name);
            dialog.error = None;
        } else {
            dialog.error =
                Some("Only a local branch that is not checked out can be deleted".into());
        }
    }

    pub(crate) fn confirm_branch_delete(&mut self, ctx: &egui::Context) {
        let Some(name) = self
            .view
            .branch_dialog
            .as_mut()
            .and_then(|dialog| dialog.confirm_delete.take())
        else {
            return;
        };
        let label = format!("Deleted branch {name}");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.delete_branch(root, &name)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;
    use crate::services::source_control::GitSnapshot;
    use std::process::Command;

    fn branch(name: &str, remote: bool, current: bool) -> GitBranch {
        GitBranch {
            name: name.into(),
            remote,
            current,
            ..Default::default()
        }
    }

    fn app_with_dialog(root: &str, branches: Vec<GitBranch>) -> LumenApp {
        let mut app = LumenApp::default();
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: root.into(),
            ..Default::default()
        });
        app.view.branch_dialog = Some(BranchDialog {
            root: root.into(),
            branches,
            ..Default::default()
        });
        app
    }

    fn dialog_error(app: &LumenApp) -> String {
        app.view
            .branch_dialog
            .as_ref()
            .and_then(|dialog| dialog.error.clone())
            .unwrap_or_default()
    }

    #[test]
    fn switching_is_blocked_by_unsaved_edits_other_worktrees_and_shadowed_names() {
        let ctx = egui::Context::default();
        let mut occupied = branch("busy", false, false);
        occupied.checked_out_at = Some("/elsewhere".into());
        let mut app = app_with_dialog(
            "/repo",
            vec![
                branch("main", false, true),
                occupied,
                branch("dev", false, false),
                branch("origin/dev", true, false),
                branch("origin/new", true, false),
            ],
        );
        app.switch_to_branch("busy".into(), false, &ctx);
        assert!(dialog_error(&app).contains("another worktree"));
        app.switch_to_branch("origin/dev".into(), true, &ctx);
        assert!(dialog_error(&app).contains("already exists"));
        let mut document = Document::from_path("/repo/a.rs".into(), "saved".into());
        document.text = "edited".into();
        document.record_text_change();
        app.editor.files.push(document);
        app.switch_to_branch("dev".into(), false, &ctx);
        assert!(dialog_error(&app).contains("unsaved"));
        assert!(app.source_control_job.is_none());
        assert!(app.view.branch_dialog.is_some());
    }

    #[test]
    fn create_rename_and_delete_validate_before_running_git() {
        let ctx = egui::Context::default();
        let mut app = app_with_dialog(
            "/repo",
            vec![branch("main", false, true), branch("dev", false, false)],
        );
        app.create_new_branch(&ctx);
        assert!(dialog_error(&app).contains("Enter a name"));
        app.view.branch_dialog.as_mut().unwrap().new_name = "dev".into();
        app.create_new_branch(&ctx);
        assert!(dialog_error(&app).contains("already exists"));
        app.view.branch_dialog.as_mut().unwrap().rename_to = "dev".into();
        app.rename_current_branch(&ctx);
        assert!(dialog_error(&app).contains("already exists"));
        app.request_branch_delete("main".into());
        assert!(dialog_error(&app).contains("not checked out"));
        app.request_branch_delete("dev".into());
        assert_eq!(
            app.view
                .branch_dialog
                .as_ref()
                .unwrap()
                .confirm_delete
                .as_deref(),
            Some("dev")
        );
        assert!(
            app.source_control_job.is_none(),
            "nothing runs before confirmation"
        );
    }

    #[test]
    fn creating_and_switching_branches_runs_through_git_and_refreshes() {
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
        std::fs::write(root.join("f.txt"), "x\n").unwrap();
        git(&["add", "--", "f.txt"]);
        git(&["commit", "-q", "-m", "init"]);
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.view.source_control.workspace = Some(root.clone());
        app.view.source_control.snapshot = app.source_control_service.snapshot(&root).unwrap();
        let ctx = egui::Context::default();
        app.open_branch_dialog(&ctx);
        app.view.branch_dialog.as_mut().unwrap().new_name = "feature/x".into();
        let wait = |app: &mut LumenApp| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while (app.source_control_job.is_some()
                || app
                    .view
                    .branch_dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.loading))
                && std::time::Instant::now() < deadline
            {
                app.poll_source_control(&ctx);
                app.poll_branches(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&mut app);
        assert!(
            app.view
                .branch_dialog
                .as_ref()
                .unwrap()
                .branches
                .iter()
                .any(|b| b.name == "main")
        );
        app.create_new_branch(&ctx);
        wait(&mut app);
        assert!(
            app.view.branch_dialog.is_none(),
            "dialog closes after switching"
        );
        let snapshot = app.view.source_control.snapshot.as_ref().unwrap();
        assert_eq!(snapshot.branch, "feature/x");
    }
}
