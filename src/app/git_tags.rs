//! Tags dialog: list, create (lightweight or annotated), confirmed delete and an
//! explicit push to a chosen remote. A tag is never moved or overwritten, and a
//! push never picks the remote on the user's behalf.
use super::LumenApp;
use crate::services::source_control::{GitLogQuery, GitTag};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    time::Duration,
};

#[derive(Default)]
pub(crate) struct TagsDialog {
    pub(crate) root: PathBuf,
    pub(crate) tags: Vec<GitTag>,
    pub(crate) loading: bool,
    pub(crate) name: String,
    /// Non-empty makes the tag annotated.
    pub(crate) message: String,
    /// The commit to tag; `None` means the current HEAD.
    pub(crate) target: Option<String>,
    pub(crate) remotes: Vec<String>,
    pub(crate) remote: Option<String>,
    pub(crate) confirm_delete: Option<String>,
    pub(crate) error: Option<String>,
    receiver: Option<Receiver<Result<Vec<GitTag>, String>>>,
}

impl LumenApp {
    pub(crate) fn open_tags_dialog(&mut self, target: Option<String>, ctx: &egui::Context) {
        // A tag on a commit comes from the History tab and belongs to its repository.
        if target.is_some() && !self.target_history_repository() {
            return;
        }
        let Some(snapshot) = &self.view.source_control.snapshot else {
            return;
        };
        let remotes: Vec<_> = snapshot.remotes.iter().map(|r| r.name.clone()).collect();
        // A single remote can be preselected; with several the user must choose.
        let remote = (remotes.len() == 1).then(|| remotes[0].clone());
        let root = snapshot.root.clone();
        self.view.tags_dialog = Some(TagsDialog {
            root,
            target,
            remotes,
            remote,
            ..Default::default()
        });
        self.request_tags(ctx);
    }

    pub(crate) fn close_tags_dialog(&mut self) {
        self.view.tags_dialog = None;
    }

    pub(crate) fn request_tags(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.tags_dialog else {
            return;
        };
        let root = dialog.root.clone();
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(service.tags(&root));
            worker_ctx.request_repaint();
        });
        dialog.receiver = Some(receiver);
        dialog.loading = true;
    }

    pub(crate) fn poll_tags(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.tags_dialog else {
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
                    Ok(tags) => dialog.tags = tags,
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

    fn set_tags_error(&mut self, error: String) {
        if let Some(dialog) = &mut self.view.tags_dialog {
            dialog.error = Some(error);
        }
    }

    pub(crate) fn create_tag(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.tags_dialog else {
            return;
        };
        let name = dialog.name.trim().to_owned();
        if name.is_empty() {
            self.set_tags_error("Enter a tag name".into());
            return;
        }
        if dialog.tags.iter().any(|tag| tag.name == name) {
            self.set_tags_error(format!(
                "A tag named {name} already exists; tags are never moved"
            ));
            return;
        }
        dialog.error = None;
        let message = dialog.message.trim().to_owned();
        let target = dialog.target.clone();
        dialog.name.clear();
        dialog.message.clear();
        let label = format!("Created tag {name}");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            let commit = match target {
                Some(commit) => commit,
                None => service
                    .log(root, &GitLogQuery::default(), None, 0, 1)?
                    .commits
                    .first()
                    .map(|commit| commit.id.clone())
                    .ok_or_else(|| "Make a first commit before tagging".to_owned())?,
            };
            service.create_tag(
                root,
                &name,
                &commit,
                (!message.is_empty()).then_some(&message),
            )
        });
    }

    pub(crate) fn request_tag_delete(&mut self, name: String) {
        if let Some(dialog) = &mut self.view.tags_dialog {
            dialog.confirm_delete = Some(name);
            dialog.error = None;
        }
    }

    pub(crate) fn confirm_tag_delete(&mut self, ctx: &egui::Context) {
        let Some(name) = self
            .view
            .tags_dialog
            .as_mut()
            .and_then(|dialog| dialog.confirm_delete.take())
        else {
            return;
        };
        let label = format!("Deleted tag {name} (remote copies are unchanged)");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.delete_tag(root, &name)
        });
    }

    pub(crate) fn choose_tag_remote(&mut self, remote: String) {
        if let Some(dialog) = &mut self.view.tags_dialog {
            dialog.remote = Some(remote);
        }
    }

    pub(crate) fn push_tag(&mut self, name: String, ctx: &egui::Context) {
        let Some(remote) = self
            .view
            .tags_dialog
            .as_ref()
            .and_then(|dialog| dialog.remote.clone())
        else {
            self.set_tags_error("Choose the remote to push to".into());
            return;
        };
        let label = format!("Pushed tag {name} to {remote}");
        self.run_source_repo_operation(ctx, label, move |service, root| {
            service.push_tag(root, &remote, &name)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::source_control::{GitRemote, GitSnapshot};
    use std::process::Command;

    fn remote(name: &str) -> GitRemote {
        GitRemote {
            name: name.into(),
            fetch_url: String::new(),
            push_url: None,
        }
    }

    #[test]
    fn several_remotes_require_an_explicit_choice_and_duplicates_are_refused() {
        let ctx = egui::Context::default();
        let mut app = LumenApp::default();
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: "/repo".into(),
            remotes: vec![remote("origin"), remote("backup")],
            ..Default::default()
        });
        app.open_tags_dialog(None, &ctx);
        assert!(app.view.tags_dialog.as_ref().unwrap().remote.is_none());
        app.push_tag("v1".into(), &ctx);
        assert!(app.source_control_job.is_none());
        assert!(
            app.view
                .tags_dialog
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("Choose the remote")
        );
        app.view.tags_dialog.as_mut().unwrap().tags = vec![GitTag {
            name: "v1".into(),
            ..Default::default()
        }];
        app.view.tags_dialog.as_mut().unwrap().name = "v1".into();
        app.create_tag(&ctx);
        assert!(app.source_control_job.is_none());
        assert!(
            app.view
                .tags_dialog
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("never moved")
        );
        app.request_tag_delete("v1".into());
        assert!(
            app.source_control_job.is_none(),
            "delete waits for confirmation"
        );
        // A lone remote is preselected.
        app.view.source_control.snapshot.as_mut().unwrap().remotes = vec![remote("origin")];
        app.close_tags_dialog();
        app.open_tags_dialog(None, &ctx);
        assert_eq!(
            app.view.tags_dialog.as_ref().unwrap().remote.as_deref(),
            Some("origin")
        );
    }

    #[test]
    fn tags_are_created_listed_and_deleted_through_the_controller() {
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
        app.view.source_control.snapshot = app.source_control_service.snapshot(&root).unwrap();
        let ctx = egui::Context::default();
        app.open_tags_dialog(None, &ctx);
        let wait = |app: &mut LumenApp| {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while (app.source_control_job.is_some()
                || app.view.tags_dialog.as_ref().is_some_and(|d| d.loading))
                && std::time::Instant::now() < deadline
            {
                app.poll_source_control(&ctx);
                app.poll_tags(&ctx);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait(&mut app);
        {
            let dialog = app.view.tags_dialog.as_mut().unwrap();
            dialog.name = "v1".into();
            dialog.message = "First release".into();
        }
        app.create_tag(&ctx);
        wait(&mut app);
        app.request_tags(&ctx);
        wait(&mut app);
        let tags = &app.view.tags_dialog.as_ref().unwrap().tags;
        assert_eq!(tags.len(), 1);
        assert!(tags[0].annotated && tags[0].message == "First release");
        app.request_tag_delete("v1".into());
        app.confirm_tag_delete(&ctx);
        wait(&mut app);
        app.request_tags(&ctx);
        wait(&mut app);
        assert!(app.view.tags_dialog.as_ref().unwrap().tags.is_empty());
    }
}
