//! Confirmation workflow for actions that change history: merge, cherry-pick,
//! revert, reset and force-with-lease. Each action is first planned against the
//! current HEAD (what it will do, what could be lost) and only applied from that
//! reviewed plan; a repository that moved in between refuses it as stale.
use super::LumenApp;
use crate::services::source_control::{HistoryAction, HistoryPlan, ResetMode};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

type PlanResult = Result<(HistoryAction, HistoryPlan), String>;

pub(crate) struct HistoryOpDialog {
    pub(crate) root: PathBuf,
    pub(crate) title: String,
    pub(crate) action: HistoryAction,
    pub(crate) plan: Option<HistoryPlan>,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    /// Parents of the commit being cherry-picked or reverted (merge → choice).
    pub(crate) parent_count: usize,
    receiver: Option<Receiver<PlanResult>>,
}

/// The remote and branch a force push would target, from the upstream
/// (`origin/main` + local `main`). Differing names are refused rather than guessed.
pub(crate) fn force_push_target(
    snapshot: &crate::services::source_control::GitSnapshot,
) -> Result<(String, String), String> {
    use crate::services::source_control::GitHead;
    let GitHead::Attached(branch) = &snapshot.head else {
        return Err("Check out a branch before force pushing".into());
    };
    let upstream = snapshot
        .upstream
        .as_ref()
        .ok_or("This branch has no upstream; publish it first")?;
    // Remote names may contain `/`, so match against the configured remotes.
    let (remote, remote_branch) = snapshot
        .remotes
        .iter()
        .find_map(|remote| {
            upstream
                .name
                .strip_prefix(&format!("{}/", remote.name))
                .map(|rest| (remote.name.clone(), rest.to_owned()))
        })
        .ok_or("The upstream's remote is not configured")?;
    if &remote_branch != branch {
        return Err(format!(
            "Force push needs the same branch name on both sides ({branch} tracks {})",
            upstream.name
        ));
    }
    Ok((remote, branch.clone()))
}

impl LumenApp {
    fn selected_root(&self) -> Option<PathBuf> {
        self.view
            .source_control
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.root.clone())
    }

    /// Opens the review dialog for `action` and computes its plan.
    pub(crate) fn plan_history_action(
        &mut self,
        action: HistoryAction,
        title: String,
        ctx: &egui::Context,
    ) {
        // Commit actions come from the History tab, which may show a repository
        // other than the selected one; they must never run against the wrong one.
        let from_history = matches!(
            action,
            HistoryAction::CherryPick { .. }
                | HistoryAction::Revert { .. }
                | HistoryAction::Reset { .. }
        );
        if from_history && !self.target_history_repository() {
            return;
        }
        let Some(root) = self.selected_root() else {
            return;
        };
        if self.source_mutation_in_flight() {
            self.view.status = "Wait for the current Git operation to finish".into();
            return;
        }
        let parent_count = match &action {
            HistoryAction::CherryPick { commit, .. } | HistoryAction::Revert { commit, .. } => self
                .view
                .history_dialog
                .as_ref()
                .and_then(|dialog| {
                    dialog
                        .commits
                        .iter()
                        .chain(dialog.selected_commit.iter())
                        .find(|c| &c.id == commit)
                })
                .map_or(1, |commit| commit.parents.len()),
            _ => 1,
        };
        // One modal at a time; the History tab stays open behind the review.
        self.view.branch_dialog = None;
        self.view.history_op_dialog = Some(HistoryOpDialog {
            root,
            title,
            action,
            plan: None,
            loading: false,
            error: None,
            parent_count,
            receiver: None,
        });
        self.request_history_plan(ctx);
    }

    /// Force push also reads the remote's current tip, which becomes the lease.
    pub(crate) fn plan_force_push(&mut self, ctx: &egui::Context) {
        let Some(snapshot) = &self.view.source_control.snapshot else {
            return;
        };
        let (remote, branch) = match force_push_target(snapshot) {
            Ok(target) => target,
            Err(error) => {
                self.view.source_control.error = Some(error);
                return;
            }
        };
        let root = snapshot.root.clone();
        if self.source_mutation_in_flight() {
            self.view.status = "Wait for the current Git operation to finish".into();
            return;
        }
        let title = format!("Force push {branch} to {remote}");
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let (worker_root, worker_remote, worker_branch) =
            (root.clone(), remote.clone(), branch.clone());
        std::thread::spawn(move || {
            let result = (|| {
                let tip = service
                    .remote_branch_tip(&worker_root, &worker_remote, &worker_branch)?
                    .ok_or_else(|| {
                        "The branch does not exist on the remote; use Push".to_owned()
                    })?;
                let action = HistoryAction::ForcePush {
                    remote: worker_remote,
                    branch: worker_branch,
                    expected: tip,
                };
                let plan = service.plan_history_action(&worker_root, &action)?;
                Ok((action, plan))
            })();
            let _ = sender.send(result);
            worker_ctx.request_repaint();
        });
        self.view.history_op_dialog = Some(HistoryOpDialog {
            root,
            title,
            action: HistoryAction::ForcePush {
                remote,
                branch,
                expected: String::new(),
            },
            plan: None,
            loading: true,
            error: None,
            parent_count: 1,
            receiver: Some(receiver),
        });
    }

    fn request_history_plan(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_op_dialog else {
            return;
        };
        let (root, action) = (dialog.root.clone(), dialog.action.clone());
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = service
                .plan_history_action(&root, &action)
                .map(|plan| (action, plan));
            let _ = sender.send(result);
            worker_ctx.request_repaint();
        });
        // Replacing the receiver drops any older plan for different options.
        dialog.receiver = Some(receiver);
        dialog.loading = true;
        dialog.plan = None;
        dialog.error = None;
    }

    pub(crate) fn poll_history_op(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_op_dialog else {
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
                    Ok((action, plan)) => {
                        dialog.action = action;
                        dialog.plan = Some(plan);
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

    pub(crate) fn cancel_history_op(&mut self) {
        self.view.history_op_dialog = None;
    }

    /// Changes an option of the pending action and plans it again.
    pub(crate) fn set_history_option(&mut self, option: HistoryOption, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.history_op_dialog else {
            return;
        };
        match (&mut dialog.action, option) {
            (HistoryAction::Merge { no_ff, .. }, HistoryOption::NoFastForward(value)) => {
                *no_ff = value;
            }
            (HistoryAction::Reset { mode, .. }, HistoryOption::ResetMode(value)) => *mode = value,
            (HistoryAction::Reset { backup, .. }, HistoryOption::Backup(value)) => *backup = value,
            (
                HistoryAction::CherryPick { parent, .. } | HistoryAction::Revert { parent, .. },
                HistoryOption::Parent(value),
            ) => *parent = Some(value),
            _ => return,
        }
        self.request_history_plan(ctx);
    }

    pub(crate) fn confirm_history_action(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.view.history_op_dialog else {
            return;
        };
        let Some(plan) = dialog.plan.clone().filter(|plan| plan.blocked.is_none()) else {
            return;
        };
        if self.source_has_dirty_documents() {
            if let Some(dialog) = &mut self.view.history_op_dialog {
                dialog.error = Some(
                    "Save or discard unsaved edits in this repository first: history changes rewrite files on disk"
                        .into(),
                );
            }
            return;
        }
        let action = dialog.action.clone();
        let label = match &action {
            HistoryAction::Merge { .. } => "Merged",
            HistoryAction::CherryPick { .. } => "Cherry-picked",
            HistoryAction::Revert { .. } => "Reverted",
            HistoryAction::Reset { .. } => "Reset branch",
            HistoryAction::ForcePush { .. } => "Force pushed",
        };
        self.view.history_op_dialog = None;
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.apply_history_action(root, &action, &plan.head)
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryOption {
    NoFastForward(bool),
    ResetMode(ResetMode),
    Backup(bool),
    Parent(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;
    use crate::services::source_control::{GitHead, GitRemote, GitSnapshot, GitUpstream};
    use std::process::Command;

    fn remote(name: &str) -> GitRemote {
        GitRemote {
            name: name.into(),
            fetch_url: String::new(),
            push_url: None,
        }
    }

    #[test]
    fn force_push_targets_the_upstream_only_when_names_match() {
        let mut snapshot = GitSnapshot {
            head: GitHead::Attached("main".into()),
            remotes: vec![remote("origin"), remote("team/fork")],
            upstream: Some(GitUpstream {
                name: "origin/main".into(),
                ahead_behind: None,
            }),
            ..Default::default()
        };
        assert_eq!(
            force_push_target(&snapshot).unwrap(),
            ("origin".to_owned(), "main".to_owned())
        );
        snapshot.upstream.as_mut().unwrap().name = "origin/other".into();
        assert!(
            force_push_target(&snapshot)
                .unwrap_err()
                .contains("same branch name")
        );
        snapshot.head = GitHead::Attached("x".into());
        snapshot.upstream.as_mut().unwrap().name = "team/fork/x".into();
        assert_eq!(
            force_push_target(&snapshot).unwrap(),
            ("team/fork".to_owned(), "x".to_owned())
        );
        snapshot.upstream = None;
        assert!(
            force_push_target(&snapshot)
                .unwrap_err()
                .contains("no upstream")
        );
        snapshot.head = GitHead::Detached("abc".into());
        assert!(
            force_push_target(&snapshot)
                .unwrap_err()
                .contains("Check out")
        );
    }

    fn repo_app(root: &std::path::Path) -> LumenApp {
        let git = |args: &[&str]| {
            let output = Command::new("git")
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
        git(&["checkout", "-q", "-b", "main"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        git(&["config", "core.autocrlf", "false"]);
        for n in 1..=2 {
            std::fs::write(root.join("f.txt"), format!("{n}\n")).unwrap();
            git(&["add", "--", "f.txt"]);
            git(&["commit", "-q", "-m", &format!("c{n}")]);
        }
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.to_path_buf());
        app.view.source_control.workspace = Some(root.to_path_buf());
        app.view.source_control.snapshot = app.source_control_service.snapshot(root).unwrap();
        app
    }

    fn wait(app: &mut LumenApp, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while (app.source_control_job.is_some()
            || app
                .view
                .history_op_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.loading))
            && std::time::Instant::now() < deadline
        {
            app.poll_source_control(ctx);
            app.poll_history_op(ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_reset_is_planned_adjusted_and_applied_from_the_reviewed_plan() {
        let directory = tempfile::tempdir().unwrap();
        let root = crate::services::source_control::canonical_path(directory.path()).unwrap();
        let mut app = repo_app(&root);
        let first = {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["rev-parse", "HEAD~1"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        let ctx = egui::Context::default();
        app.plan_history_action(
            HistoryAction::Reset {
                target: first.clone(),
                mode: ResetMode::Mixed,
                backup: false,
            },
            "Reset".into(),
            &ctx,
        );
        wait(&mut app, &ctx);
        let plan = app
            .view
            .history_op_dialog
            .as_ref()
            .unwrap()
            .plan
            .clone()
            .unwrap();
        assert!(plan.warnings.iter().any(|w| w.contains("No backup branch")));
        // Choosing a backup re-plans, so the dialog shows the new effects.
        app.set_history_option(HistoryOption::Backup(true), &ctx);
        assert!(app.view.history_op_dialog.as_ref().unwrap().plan.is_none());
        wait(&mut app, &ctx);
        let plan = app
            .view
            .history_op_dialog
            .as_ref()
            .unwrap()
            .plan
            .clone()
            .unwrap();
        assert!(plan.lines.iter().any(|l| l.contains("lumen-backup")));
        app.confirm_history_action(&ctx);
        assert!(app.view.history_op_dialog.is_none());
        wait(&mut app, &ctx);
        assert_eq!(
            app.view.source_control.snapshot.as_ref().unwrap().branch,
            "main"
        );
        let head = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), first);
    }

    #[test]
    fn unsaved_edits_block_confirmation_and_a_blocked_plan_cannot_run() {
        let directory = tempfile::tempdir().unwrap();
        let root = crate::services::source_control::canonical_path(directory.path()).unwrap();
        let mut app = repo_app(&root);
        let head = {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        let ctx = egui::Context::default();
        // Resetting to the commit HEAD already is has nothing to do.
        app.plan_history_action(
            HistoryAction::Reset {
                target: head.clone(),
                mode: ResetMode::Soft,
                backup: false,
            },
            "Reset".into(),
            &ctx,
        );
        wait(&mut app, &ctx);
        assert!(
            app.view
                .history_op_dialog
                .as_ref()
                .unwrap()
                .plan
                .as_ref()
                .unwrap()
                .blocked
                .is_some()
        );
        app.confirm_history_action(&ctx);
        assert!(app.source_control_job.is_none());
        assert!(app.view.history_op_dialog.is_some(), "stays open");
        app.cancel_history_op();
        let mut document = Document::from_path(root.join("f.txt"), "2\n".into());
        document.text = "typed".into();
        document.record_text_change();
        app.editor.files.push(document);
        app.plan_history_action(
            HistoryAction::Reset {
                target: head,
                mode: ResetMode::Hard,
                backup: true,
            },
            "Reset".into(),
            &ctx,
        );
        wait(&mut app, &ctx);
        app.confirm_history_action(&ctx);
        let dialog = app.view.history_op_dialog.as_ref().unwrap();
        assert!(app.source_control_job.is_none());
        assert!(
            dialog
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("unsaved edits")
        );
    }
}
