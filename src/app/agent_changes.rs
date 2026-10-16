//! "Changed by the agent": snapshot the folder before a turn, diff it afterwards,
//! and let the user review and revert what the agent did.
//!
//! This is the primary safety net for agents that edit through their own shell
//! commands (Codex) and a second line of defence for the rest. Reverts are guarded
//! like every other write: the file must still be exactly what the agent left,
//! and a buffer with unsaved edits is never touched.
use super::{LumenApp, agent::AgentKind};
use crate::{
    model::output_log::LogLevel,
    services::workspace_snapshot::{
        ChangeKind, FileChange, Limits, Snapshot, capture, changes as diff_snapshots,
    },
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    Pending,
    Reverted,
    Failed(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ChangeRow {
    pub(crate) change: FileChange,
    pub(crate) state: RowState,
    pub(crate) expanded: bool,
}

/// The files one turn changed.
#[derive(Clone, Debug)]
pub(crate) struct TurnChanges {
    pub(crate) root: PathBuf,
    pub(crate) rows: Vec<ChangeRow>,
    /// The folder was too large to track completely.
    pub(crate) truncated: bool,
    /// The file list is expanded.
    pub(crate) open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangeAction {
    ToggleOpen,
    ToggleFile(PathBuf),
    Revert(PathBuf),
    RevertAll,
    Dismiss,
    OpenFile(PathBuf),
}

enum Purpose {
    /// Take the "before" picture, then send this prompt.
    Before(super::agent::Prompt),
    After,
}

struct Job {
    receiver: Receiver<Snapshot>,
    cancel: Arc<AtomicBool>,
    purpose: Purpose,
}

/// Per-agent snapshot bookkeeping.
#[derive(Default)]
pub(crate) struct Tracking {
    /// State at the end of the previous turn; makes the next scan incremental.
    baseline: Option<Snapshot>,
    before: Option<Snapshot>,
    job: Option<Job>,
}

impl LumenApp {
    /// The folder to watch for this turn, if tracking applies: tracking is on and a
    /// real folder is open (never the home-directory fallback).
    fn tracking_root(&self, kind: AgentKind) -> Option<PathBuf> {
        if !self.view.settings.boolean("agent.track_changes") {
            return None;
        }
        let _ = kind;
        let root = self.editor.workspace.clone()?;
        Some(root.canonicalize().unwrap_or(root))
    }

    fn spawn_snapshot(root: PathBuf, previous: Option<Snapshot>, purpose: Purpose) -> Job {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        thread::spawn(move || {
            let snapshot = capture(&root, previous.as_ref(), &Limits::default(), &flag);
            let _ = sender.send(snapshot);
        });
        Job {
            receiver,
            cancel,
            purpose,
        }
    }

    /// Start the "before" scan; the prompt is sent once it finishes. Returns false
    /// when tracking does not apply and the caller should send right away.
    pub(super) fn begin_tracked_turn(
        &mut self,
        kind: AgentKind,
        prompt: &super::agent::Prompt,
    ) -> bool {
        let Some(root) = self.tracking_root(kind) else {
            return false;
        };
        let previous = self.runtime_tracking(kind).baseline.clone();
        self.view.agents.session_mut(kind).changes = None;
        self.view.agents.session_mut(kind).preparing = true;
        let job = Self::spawn_snapshot(root, previous, Purpose::Before(prompt.clone()));
        self.runtime_tracking(kind).job = Some(job);
        true
    }

    /// The turn ended: scan again and compare.
    pub(super) fn begin_after_snapshot(&mut self, kind: AgentKind) {
        let tracking = self.runtime_tracking(kind);
        let Some(before) = tracking.before.clone() else {
            return;
        };
        let root = before.root().to_path_buf();
        self.view.agents.session_mut(kind).finishing = true;
        let job = Self::spawn_snapshot(root, Some(before), Purpose::After);
        self.runtime_tracking(kind).job = Some(job);
    }

    fn runtime_tracking(&mut self, kind: AgentKind) -> &mut Tracking {
        &mut self.agent_runtime.tracking[kind.index()]
    }

    /// Abandon any scan in flight (cancel, crash, new session).
    pub(super) fn cancel_snapshot(&mut self, kind: AgentKind) {
        let tracking = self.runtime_tracking(kind);
        if let Some(job) = tracking.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        tracking.before = None;
        let session = self.view.agents.session_mut(kind);
        session.preparing = false;
        session.finishing = false;
    }

    pub(super) fn reset_tracking(&mut self, kind: AgentKind) {
        self.cancel_snapshot(kind);
        self.runtime_tracking(kind).baseline = None;
    }

    pub(super) fn poll_snapshots(&mut self, kind: AgentKind, ctx: &egui::Context) {
        let result = match self.runtime_tracking(kind).job.as_ref() {
            Some(job) => job.receiver.try_recv(),
            None => return,
        };
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(40));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.runtime_tracking(kind).job = None;
                self.view.agents.session_mut(kind).preparing = false;
                self.view.agents.session_mut(kind).finishing = false;
                return;
            }
        };
        let job = self
            .runtime_tracking(kind)
            .job
            .take()
            .expect("job was present");
        match job.purpose {
            Purpose::Before(prompt) => {
                self.view.agents.session_mut(kind).preparing = false;
                if snapshot.truncated {
                    Self::agent_log(
                        kind,
                        LogLevel::Warn,
                        "Folder too large to track completely; change review may miss files",
                    );
                }
                self.runtime_tracking(kind).before = Some(snapshot);
                self.send_prompt_now(kind, prompt);
            }
            Purpose::After => self.finish_tracked_turn(kind, snapshot),
        }
    }

    fn finish_tracked_turn(&mut self, kind: AgentKind, after: Snapshot) {
        self.view.agents.session_mut(kind).finishing = false;
        let tracking = self.runtime_tracking(kind);
        let before = tracking.before.take();
        let truncated = after.truncated || before.as_ref().is_some_and(|b| b.truncated);
        let root = after.root().to_path_buf();
        let found = before
            .map(|before| diff_snapshots(&before, &after))
            .unwrap_or_default();
        tracking.baseline = Some(after);
        // Buffers without unsaved edits follow the disk; the rest keep their text
        // and the existing "changed on disk" flow applies.
        for change in &found {
            if let (Some(text), true) = (&change.after, change.kind != ChangeKind::Deleted) {
                self.reload_clean_document(&change.path, text);
            }
        }
        if found.is_empty() {
            return;
        }
        let summary = format!("{} file(s) changed by the agent", found.len());
        Self::agent_log(kind, LogLevel::Info, &summary);
        self.view.agents.session_mut(kind).changes = Some(TurnChanges {
            root,
            rows: found
                .into_iter()
                .map(|change| ChangeRow {
                    change,
                    state: RowState::Pending,
                    expanded: false,
                })
                .collect(),
            truncated,
            open: false,
        });
    }

    fn reload_clean_document(&mut self, path: &std::path::Path, text: &str) {
        if let Some(document) = self
            .editor
            .files
            .iter_mut()
            .find(|document| document.path.as_deref() == Some(path) && !document.dirty)
        {
            document.reload_from_disk(text.to_owned());
        }
    }

    pub(crate) fn apply_change_action(&mut self, kind: AgentKind, action: ChangeAction) {
        match action {
            ChangeAction::Dismiss => self.view.agents.session_mut(kind).changes = None,
            ChangeAction::ToggleOpen => {
                if let Some(changes) = &mut self.view.agents.session_mut(kind).changes {
                    changes.open = !changes.open;
                }
            }
            ChangeAction::ToggleFile(path) => {
                if let Some(row) = self.change_row_mut(kind, &path) {
                    row.expanded = !row.expanded;
                }
            }
            ChangeAction::OpenFile(path) => self.open_file(path),
            ChangeAction::Revert(path) => self.revert_change(kind, &path),
            ChangeAction::RevertAll => {
                let paths: Vec<PathBuf> = self
                    .view
                    .agents
                    .session(kind)
                    .changes
                    .iter()
                    .flat_map(|changes| &changes.rows)
                    .filter(|row| row.state == RowState::Pending && !row.change.opaque)
                    .map(|row| row.change.path.clone())
                    .collect();
                for path in paths {
                    self.revert_change(kind, &path);
                }
            }
        }
    }

    fn change_row_mut(
        &mut self,
        kind: AgentKind,
        path: &std::path::Path,
    ) -> Option<&mut ChangeRow> {
        self.view
            .agents
            .session_mut(kind)
            .changes
            .as_mut()?
            .rows
            .iter_mut()
            .find(|row| row.change.path == path)
    }

    /// Put one file back to what it was before the turn, if nothing else touched
    /// it since the agent finished.
    fn revert_change(&mut self, kind: AgentKind, path: &std::path::Path) {
        let Some(changes) = &self.view.agents.session(kind).changes else {
            return;
        };
        let root = changes.root.clone();
        let Some(row) = changes.rows.iter().find(|row| row.change.path == path) else {
            return;
        };
        if row.state != RowState::Pending {
            return;
        }
        let change = row.change.clone();
        let outcome = self.try_revert(&root, &change);
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let state = match outcome {
            Ok(()) => {
                Self::agent_log(
                    kind,
                    LogLevel::Info,
                    &format!("Reverted {}", path.display()),
                );
                self.view.status = format!("Reverted {name}");
                RowState::Reverted
            }
            Err(reason) => {
                Self::agent_log(
                    kind,
                    LogLevel::Warn,
                    &format!("Could not revert {}: {reason}", path.display()),
                );
                self.view.status = format!("Could not revert {name}: {reason}");
                RowState::Failed(reason)
            }
        };
        if let Some(row) = self.change_row_mut(kind, path) {
            row.state = state;
        }
    }

    fn try_revert(&mut self, root: &std::path::Path, change: &FileChange) -> Result<(), String> {
        if change.opaque {
            return Err("binary or large files cannot be reverted here".into());
        }
        if self.editor.files.iter().any(|document| {
            document.dirty && document.path.as_deref() == Some(change.path.as_path())
        }) {
            return Err("it has unsaved changes in an editor tab".into());
        }
        let io = |error: std::io::Error| error.to_string();
        match change.kind {
            ChangeKind::Modified => {
                let (before, after) = (change.before.as_deref(), change.after.as_deref());
                let (Some(before), Some(after)) = (before, after) else {
                    return Err("no text to restore".into());
                };
                self.file_system
                    .write_workspace_replace(root, &change.path, after, before)
                    .map_err(|error| match error.kind() {
                        std::io::ErrorKind::AlreadyExists => {
                            "it changed again since the agent finished".to_owned()
                        }
                        _ => io(error),
                    })?;
                self.reload_clean_document(&change.path, before);
            }
            ChangeKind::Added => {
                let expected = change.after.as_deref().ok_or("no text recorded")?;
                match self.file_system.read_text(&change.path) {
                    Ok(current) if current == expected => {}
                    Ok(_) => return Err("it changed again since the agent finished".into()),
                    Err(error) => return Err(io(error)),
                }
                // Moved to the Trash, never deleted outright.
                self.file_system.trash_entry(&change.path).map_err(io)?;
            }
            ChangeKind::Deleted => {
                let before = change.before.as_deref().ok_or("no text recorded")?;
                self.file_system
                    .write_text_if_missing(&change.path, before)
                    .map_err(|error| match error.kind() {
                        std::io::ErrorKind::AlreadyExists => {
                            "a file with that name exists again".to_owned()
                        }
                        _ => io(error),
                    })?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::command::Command,
        model::Document,
        services::{
            FileSystem, LocalFileSystem,
            agent_process::{AgentEvent, fake::FakeAgentLauncher},
        },
    };
    use serde_json::{Value, json};
    use std::{path::Path, time::Instant};

    /// Real files, but "Trash" is a plain delete so tests never touch the user's Trash.
    struct TestFileSystem(LocalFileSystem);

    impl FileSystem for TestFileSystem {
        fn read_text(&self, path: &Path) -> std::io::Result<String> {
            self.0.read_text(path)
        }
        fn write_text(&self, path: &Path, text: &str) -> std::io::Result<()> {
            self.0.write_text(path, text)
        }
        fn create_file(&self, path: &Path) -> std::io::Result<()> {
            self.0.create_file(path)
        }
        fn create_directory(&self, path: &Path) -> std::io::Result<()> {
            self.0.create_directory(path)
        }
        fn rename_entry(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.0.rename_entry(from, to)
        }
        fn copy_entry(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.0.copy_entry(from, to)
        }
        fn trash_entry(&self, path: &Path) -> std::io::Result<()> {
            std::fs::remove_file(path)
        }
        fn entry_exists(&self, path: &Path) -> bool {
            self.0.entry_exists(path)
        }
        fn children(&self, path: &Path) -> Vec<PathBuf> {
            self.0.children(path)
        }
        fn is_directory(&self, path: &Path) -> bool {
            self.0.is_directory(path)
        }
    }

    struct Harness {
        app: LumenApp,
        fake: FakeAgentLauncher,
        ctx: egui::Context,
        dir: tempfile::TempDir,
    }

    impl Harness {
        fn new() -> Self {
            let fake = FakeAgentLauncher::default();
            let mut app = LumenApp::default();
            app.agent_runtime.launcher = Arc::new(fake.clone());
            app.file_system = Box::new(TestFileSystem(LocalFileSystem));
            let program = std::env::current_exe().unwrap().display().to_string();
            app.view.settings.user.persisted.insert(
                "agent.claude.command".into(),
                lumen_settings_core::Value::Text(program),
            );
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            std::fs::write(root.join("edit.txt"), "one\ntwo\n").unwrap();
            std::fs::write(root.join("gone.txt"), "bye\n").unwrap();
            std::fs::write(root.join("keep.txt"), "same\n").unwrap();
            app.editor.workspace = Some(root);
            Self {
                app,
                fake,
                ctx: egui::Context::default(),
                dir,
            }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().canonicalize().unwrap()
        }

        fn sent(&self, method: &str) -> Option<Value> {
            self.fake
                .sent
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|m| m["method"] == method)
                .cloned()
        }

        fn deliver(&mut self, message: Value) {
            self.fake
                .queued
                .lock()
                .unwrap()
                .push(AgentEvent::Message(message));
            self.app.poll_agents(&self.ctx);
        }

        fn poll_until(&mut self, what: &str, mut done: impl FnMut(&LumenApp) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(&self.app) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                self.app.poll_agents(&self.ctx);
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn prompts_sent(fake: &FakeAgentLauncher) -> usize {
            fake.sent
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m["method"] == "session/prompt")
                .count()
        }

        fn wait_for_prompt(&mut self, count: usize) {
            let fake = self.fake.clone();
            self.poll_until("the prompt", move |_| Self::prompts_sent(&fake) >= count);
        }

        /// Send "go", finish the handshake, and wait until the prompt reaches the agent.
        fn start_turn(&mut self) {
            self.app.view.agents.session_mut(AgentKind::Claude).draft = "go".into();
            self.app.execute(Command::SendAgentPrompt, &self.ctx);
            let init = self.sent("initialize").unwrap();
            self.deliver(json!({"jsonrpc":"2.0","id":init["id"],"result":{"protocolVersion":1}}));
            let new = self.sent("session/new").unwrap();
            self.deliver(json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"s1"}}));
        }

        fn finish_turn(&mut self) {
            let prompt = self.sent("session/prompt").expect("prompt was sent");
            self.deliver(
                json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}}),
            );
            self.poll_until("the comparison", |app| {
                !app.view.agents.session(AgentKind::Claude).finishing
            });
        }

        fn changes(&self) -> Option<&TurnChanges> {
            self.app
                .view
                .agents
                .session(AgentKind::Claude)
                .changes
                .as_ref()
        }

        /// What an agent turn does to the folder.
        fn agent_edits_files(&self) {
            let root = self.root();
            std::fs::write(root.join("edit.txt"), "one\nTWO\n").unwrap();
            std::fs::write(root.join("new.txt"), "fresh\n").unwrap();
            std::fs::remove_file(root.join("gone.txt")).unwrap();
        }

        fn run_edit_turn(&mut self) {
            self.start_turn();
            self.wait_for_prompt(1);
            self.agent_edits_files();
            self.finish_turn();
        }

        fn state(&self, name: &str) -> RowState {
            self.changes()
                .unwrap()
                .rows
                .iter()
                .find(|row| row.change.path.ends_with(name))
                .unwrap()
                .state
                .clone()
        }
    }

    #[test]
    fn the_prompt_waits_for_the_snapshot_and_the_turn_ends_with_a_review() {
        let mut h = Harness::new();
        h.start_turn();
        assert!(h.app.view.agents.session(AgentKind::Claude).running);
        h.wait_for_prompt(1);
        assert!(!h.app.view.agents.session(AgentKind::Claude).preparing);
        h.agent_edits_files();
        h.finish_turn();

        let changes = h.changes().expect("changes are offered for review");
        let summary: Vec<_> = changes
            .rows
            .iter()
            .map(|row| {
                format!(
                    "{:?} {}",
                    row.change.kind,
                    row.change.path.file_name().unwrap().to_string_lossy()
                )
            })
            .collect();
        assert_eq!(
            summary,
            ["Modified edit.txt", "Deleted gone.txt", "Added new.txt"]
        );
        assert!(
            changes
                .rows
                .iter()
                .all(|row| row.state == RowState::Pending)
        );
        assert!(!h.app.view.agents.session(AgentKind::Claude).running);
    }

    #[test]
    fn a_turn_that_changes_nothing_shows_no_review_and_open_clean_tabs_follow_the_disk() {
        let mut h = Harness::new();
        let path = h.root().join("edit.txt");
        h.app
            .editor
            .files
            .push(Document::from_path(path.clone(), "one\ntwo\n".into()));
        h.start_turn();
        h.wait_for_prompt(1);
        h.finish_turn();
        assert!(h.changes().is_none());

        // Second turn edits the open file: the clean tab shows the new text.
        h.app.view.agents.session_mut(AgentKind::Claude).draft = "again".into();
        h.app.execute(Command::SendAgentPrompt, &h.ctx);
        h.wait_for_prompt(2);
        std::fs::write(&path, "one\nTWO\n").unwrap();
        h.finish_turn();
        assert_eq!(h.app.editor.files[0].text, "one\nTWO\n");
        assert!(!h.app.editor.files[0].dirty);
    }

    #[test]
    fn reverting_restores_modified_deleted_and_added_files() {
        let mut h = Harness::new();
        h.run_edit_turn();
        let root = h.root();
        h.app.execute(
            Command::AgentChanges(
                AgentKind::Claude,
                ChangeAction::Revert(root.join("edit.txt")),
            ),
            &h.ctx,
        );
        assert_eq!(
            std::fs::read_to_string(root.join("edit.txt")).unwrap(),
            "one\ntwo\n"
        );
        h.app.execute(
            Command::AgentChanges(AgentKind::Claude, ChangeAction::RevertAll),
            &h.ctx,
        );
        assert_eq!(
            std::fs::read_to_string(root.join("gone.txt")).unwrap(),
            "bye\n"
        );
        assert!(!root.join("new.txt").exists(), "the added file was removed");
        for name in ["edit.txt", "gone.txt", "new.txt"] {
            assert_eq!(h.state(name), RowState::Reverted, "{name}");
        }
        assert_eq!(
            std::fs::read_to_string(root.join("keep.txt")).unwrap(),
            "same\n"
        );
    }

    #[test]
    fn a_file_changed_again_after_the_turn_is_not_reverted() {
        let mut h = Harness::new();
        h.run_edit_turn();
        let root = h.root();
        std::fs::write(root.join("edit.txt"), "user edit\n").unwrap();
        std::fs::write(root.join("new.txt"), "user rewrote it\n").unwrap();
        std::fs::write(root.join("gone.txt"), "recreated\n").unwrap();
        h.app.execute(
            Command::AgentChanges(AgentKind::Claude, ChangeAction::RevertAll),
            &h.ctx,
        );
        assert_eq!(
            std::fs::read_to_string(root.join("edit.txt")).unwrap(),
            "user edit\n"
        );
        assert!(root.join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("gone.txt")).unwrap(),
            "recreated\n"
        );
        for name in ["edit.txt", "gone.txt", "new.txt"] {
            assert!(
                matches!(h.state(name), RowState::Failed(_)),
                "{name}: {:?}",
                h.state(name)
            );
        }
    }

    #[test]
    fn unsaved_editor_text_blocks_a_revert() {
        let mut h = Harness::new();
        let path = h.root().join("edit.txt");
        h.run_edit_turn();
        let mut document = Document::from_path(path.clone(), "one\nTWO\n".into());
        document.dirty = true;
        h.app.editor.files.push(document);
        h.app.execute(
            Command::AgentChanges(AgentKind::Claude, ChangeAction::Revert(path.clone())),
            &h.ctx,
        );
        assert!(
            matches!(h.state("edit.txt"), RowState::Failed(reason) if reason.contains("unsaved"))
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "one\nTWO\n");
    }

    #[test]
    fn keep_dismisses_and_a_new_message_clears_an_unreviewed_set() {
        let mut h = Harness::new();
        h.run_edit_turn();
        h.app.execute(
            Command::AgentChanges(AgentKind::Claude, ChangeAction::ToggleOpen),
            &h.ctx,
        );
        assert!(h.changes().unwrap().open);
        h.app.execute(
            Command::AgentChanges(AgentKind::Claude, ChangeAction::Dismiss),
            &h.ctx,
        );
        assert!(h.changes().is_none());

        h.run_edit_turn_again();
        assert!(h.changes().is_some());
        h.app.view.agents.session_mut(AgentKind::Claude).draft = "next".into();
        h.app.execute(Command::SendAgentPrompt, &h.ctx);
        assert!(
            h.changes().is_none(),
            "starting another turn replaces the old review"
        );
    }

    impl Harness {
        fn run_edit_turn_again(&mut self) {
            self.app.view.agents.session_mut(AgentKind::Claude).draft = "more".into();
            self.app.execute(Command::SendAgentPrompt, &self.ctx);
            let before = Self::prompts_sent(&self.fake);
            self.wait_for_prompt(before + 1);
            std::fs::write(self.root().join("again.txt"), "x\n").unwrap();
            self.finish_turn();
        }
    }

    #[test]
    fn cancelling_while_the_snapshot_is_prepared_sends_nothing() {
        let mut h = Harness::new();
        h.start_turn();
        // The snapshot job is still in flight (the test has not polled yet).
        assert!(h.app.view.agents.session(AgentKind::Claude).preparing);
        h.app.execute(Command::CancelAgentTurn, &h.ctx);
        assert!(!h.app.view.agents.session(AgentKind::Claude).running);
        thread::sleep(Duration::from_millis(200));
        h.app.poll_agents(&h.ctx);
        assert!(
            h.sent("session/prompt").is_none(),
            "the message was dropped"
        );
    }

    #[test]
    fn tracking_is_skipped_without_a_folder_or_when_turned_off() {
        let mut h = Harness::new();
        h.app.view.settings.user.persisted.insert(
            "agent.track_changes".into(),
            lumen_settings_core::Value::Bool(false),
        );
        h.start_turn();
        assert!(h.sent("session/prompt").is_some(), "sent immediately");
        h.finish_turn();
        assert!(h.changes().is_none());

        let mut h = Harness::new();
        h.app.editor.workspace = None;
        h.start_turn();
        assert!(
            h.sent("session/prompt").is_some(),
            "no folder: nothing to track"
        );
    }

    #[test]
    fn a_crash_mid_turn_abandons_the_snapshot() {
        let mut h = Harness::new();
        h.start_turn();
        h.fake.queued.lock().unwrap().push(AgentEvent::Exited);
        h.app.poll_agents(&h.ctx);
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert!(!session.preparing && !session.running);
        assert!(h.app.agent_runtime.tracking[0].job.is_none());
    }
}
