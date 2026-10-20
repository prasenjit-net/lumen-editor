//! Clone Repository workflow: validation, a cancellable background clone, and
//! opening the result only on success.
use super::LumenApp;
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

#[derive(Default)]
pub(crate) struct CloneDialog {
    pub(crate) url: String,
    pub(crate) parent: String,
    pub(crate) name: String,
    /// Once the user edits the folder name, stop deriving it from the URL.
    pub(crate) name_edited: bool,
    pub(crate) error: Option<String>,
    pub(crate) run: Option<CloneRun>,
}

pub(crate) struct CloneRun {
    receiver: Receiver<CloneEvent>,
    cancel: Arc<AtomicBool>,
    pub(crate) progress: String,
    target: PathBuf,
}

enum CloneEvent {
    Progress(String),
    Finished(Result<(), String>),
}

/// `https://host/team/project.git` and `git@host:team/project` → `project`.
pub(crate) fn repository_name(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or_default();
    last.strip_suffix(".git").unwrap_or(last).to_owned()
}

/// A folder name that cannot escape the parent directory.
fn valid_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.starts_with('-')
}

impl CloneDialog {
    /// Keeps the derived folder name in step with the URL until it is edited.
    pub(crate) fn sync_name(&mut self) {
        if !self.name_edited {
            self.name = repository_name(&self.url);
        }
    }

    /// The directory that would be created, or why the inputs are not usable.
    pub(crate) fn target(&self) -> Result<PathBuf, String> {
        if self.url.trim().is_empty() || self.url.trim().starts_with('-') {
            return Err("Enter a repository URL or path".into());
        }
        let parent = PathBuf::from(self.parent.trim());
        if self.parent.trim().is_empty() || !parent.is_dir() {
            return Err("Choose an existing destination folder".into());
        }
        if !valid_folder_name(self.name.trim()) {
            return Err("Enter a folder name without path separators".into());
        }
        let target = parent.join(self.name.trim());
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(format!("{} already exists", target.display()));
        }
        Ok(target)
    }
}

impl LumenApp {
    pub(crate) fn open_clone_dialog(&mut self) {
        if self.view.clone_dialog.is_some() {
            return;
        }
        let parent = self
            .editor
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.parent())
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        self.view.clone_dialog = Some(CloneDialog {
            parent,
            ..Default::default()
        });
    }

    pub(crate) fn pick_clone_destination(&mut self) {
        let Some(folder) = self.dialogs.open_folder() else {
            return;
        };
        if let Some(dialog) = &mut self.view.clone_dialog {
            dialog.parent = folder.display().to_string();
        }
    }

    pub(crate) fn start_clone(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.view.clone_dialog else {
            return;
        };
        if dialog.run.is_some() {
            return;
        }
        let target = match dialog.target() {
            Ok(target) => target,
            Err(error) => {
                dialog.error = Some(error);
                return;
            }
        };
        dialog.error = None;
        let url = dialog.url.trim().to_owned();
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let service = self.source_control_service.clone();
        let worker_ctx = ctx.clone();
        let worker_cancel = cancel.clone();
        let worker_target = target.clone();
        std::thread::spawn(move || {
            let progress_sender = sender.clone();
            let progress_ctx = worker_ctx.clone();
            let result = service.clone_repository(&url, &worker_target, &worker_cancel, &|line| {
                let _ = progress_sender.send(CloneEvent::Progress(line));
                progress_ctx.request_repaint();
            });
            let _ = sender.send(CloneEvent::Finished(result));
            worker_ctx.request_repaint();
        });
        dialog.run = Some(CloneRun {
            receiver,
            cancel,
            progress: "Starting clone…".into(),
            target,
        });
    }

    /// Cancelling a running clone stops Git and removes the partial folder;
    /// with nothing running it simply closes the dialog.
    pub(crate) fn cancel_clone(&mut self) {
        match self
            .view
            .clone_dialog
            .as_ref()
            .and_then(|dialog| dialog.run.as_ref())
        {
            Some(run) => {
                run.cancel.store(true, Ordering::SeqCst);
                if let Some(run) = self
                    .view
                    .clone_dialog
                    .as_mut()
                    .and_then(|dialog| dialog.run.as_mut())
                {
                    run.progress = "Cancelling…".into();
                }
            }
            None => self.view.clone_dialog = None,
        }
    }

    pub(crate) fn poll_clone(&mut self, ctx: &egui::Context) {
        let Some(run) = self
            .view
            .clone_dialog
            .as_mut()
            .and_then(|dialog| dialog.run.as_mut())
        else {
            return;
        };
        let mut finished = None;
        loop {
            match run.receiver.try_recv() {
                Ok(CloneEvent::Progress(line)) => run.progress = line,
                Ok(CloneEvent::Finished(result)) => {
                    finished = Some(result);
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = Some(Err("Git clone stopped unexpectedly".into()));
                    break;
                }
            }
        }
        match finished {
            None => ctx.request_repaint_after(Duration::from_millis(100)),
            Some(result) => {
                let target = run.target.clone();
                let cancelled = run.cancel.load(Ordering::SeqCst);
                match result {
                    Ok(()) => {
                        self.view.clone_dialog = None;
                        self.open_folder_path(target);
                    }
                    Err(_) if cancelled => {
                        self.view.clone_dialog = None;
                        self.view.status = "Clone cancelled".into();
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut self.view.clone_dialog {
                            dialog.run = None;
                            dialog.error = Some(error);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn source_repo(top: &std::path::Path) -> PathBuf {
        let source = top.join("source");
        std::fs::create_dir_all(&source).unwrap();
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.name", "T"]);
        git(&source, &["config", "user.email", "t@example.invalid"]);
        git(&source, &["config", "commit.gpgsign", "false"]);
        std::fs::write(source.join("a.txt"), "a\n").unwrap();
        git(&source, &["add", "--", "a.txt"]);
        git(&source, &["commit", "-q", "-m", "init"]);
        source
    }

    #[test]
    fn folder_names_come_from_urls_and_cannot_escape_the_parent() {
        assert_eq!(repository_name("https://h/team/project.git"), "project");
        assert_eq!(repository_name("git@host:team/project"), "project");
        assert_eq!(repository_name("/srv/repos/thing/"), "thing");
        assert!(!valid_folder_name("../x"));
        assert!(!valid_folder_name(".."));
        assert!(!valid_folder_name("-flag"));
        assert!(valid_folder_name("project"));
    }

    #[test]
    fn target_validation_rejects_missing_parent_existing_destination_and_bad_input() {
        let directory = tempfile::tempdir().unwrap();
        let mut dialog = CloneDialog {
            url: "https://h/x.git".into(),
            parent: directory.path().display().to_string(),
            ..Default::default()
        };
        dialog.sync_name();
        assert_eq!(dialog.target().unwrap(), directory.path().join("x"));
        std::fs::create_dir(directory.path().join("x")).unwrap();
        assert!(dialog.target().unwrap_err().contains("already exists"));
        dialog.parent = directory.path().join("missing").display().to_string();
        assert!(dialog.target().unwrap_err().contains("destination"));
        dialog.parent = directory.path().display().to_string();
        dialog.url = "--upload-pack=evil".into();
        assert!(dialog.target().is_err());
    }

    #[test]
    fn a_successful_clone_opens_the_new_folder() {
        let directory = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(directory.path()).unwrap();
        let source = source_repo(&top);
        let mut app = LumenApp::default();
        app.open_clone_dialog();
        {
            let dialog = app.view.clone_dialog.as_mut().unwrap();
            dialog.url = source.display().to_string();
            dialog.parent = top.display().to_string();
            dialog.name = "copy".into();
            dialog.name_edited = true;
        }
        let ctx = egui::Context::default();
        app.start_clone(&ctx);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while app.view.clone_dialog.is_some() && std::time::Instant::now() < deadline {
            app.poll_clone(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.view.clone_dialog.is_none(), "clone did not finish");
        assert_eq!(
            app.editor.workspace.as_deref(),
            Some(top.join("copy").as_path())
        );
        assert!(top.join("copy/a.txt").exists());
    }

    #[test]
    fn a_failed_clone_keeps_the_dialog_with_the_error_and_leaves_no_folder() {
        let directory = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(directory.path()).unwrap();
        let mut app = LumenApp::default();
        app.open_clone_dialog();
        {
            let dialog = app.view.clone_dialog.as_mut().unwrap();
            dialog.url = top.join("does-not-exist").display().to_string();
            dialog.parent = top.display().to_string();
            dialog.name = "copy".into();
            dialog.name_edited = true;
        }
        let ctx = egui::Context::default();
        app.start_clone(&ctx);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while app
            .view
            .clone_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.run.is_some())
            && std::time::Instant::now() < deadline
        {
            app.poll_clone(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        let dialog = app.view.clone_dialog.as_ref().expect("dialog stays open");
        assert!(dialog.error.as_deref().unwrap().contains("clone failed"));
        assert!(!top.join("copy").exists());
        assert!(app.editor.workspace.is_none());
    }

    #[test]
    fn a_cancelled_clone_never_overwrites_or_leaves_partial_output() {
        use crate::services::source_control::{LocalGit, SourceControlService};
        let directory = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(directory.path()).unwrap();
        let source = source_repo(&top);
        let cancel = AtomicBool::new(true);
        let target = top.join("cancelled");
        let result =
            LocalGit.clone_repository(&source.display().to_string(), &target, &cancel, &|_| {});
        assert_eq!(result.unwrap_err(), "Clone cancelled");
        assert!(!target.exists());
        // An existing destination is refused untouched, even with data inside.
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep.txt"), "mine").unwrap();
        let cancel = AtomicBool::new(false);
        let result =
            LocalGit.clone_repository(&source.display().to_string(), &target, &cancel, &|_| {});
        assert!(result.unwrap_err().contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(target.join("keep.txt")).unwrap(),
            "mine"
        );
    }
}
