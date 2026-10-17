//! Running tasks: run state, the per-frame poller and the start/stop/restart
//! workflows. Processes live behind `TaskProcess`; the UI shows `TaskRuns` and
//! issues typed commands. Delayed events can only reach the run that owns the
//! process (runs are addressed by `RunId`, never by tab position or label).
use super::LumenApp;
use crate::{
    model::task_output::RunLog,
    services::task_process::{ExitReport, TaskEvent, TaskProcess},
};
use eframe::egui;
use lumen_task_core::{ResolvedTask, SavePolicy, TaskDef, TaskKey};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// Politeness window between "please stop" and killing the tree.
pub(crate) const STOP_GRACE: Duration = Duration::from_secs(3);
/// After the process exits, how long to wait for its pipes to close before
/// concluding that a leftover descendant is holding them open.
pub(crate) const DRAIN_DEADLINE: Duration = Duration::from_secs(2);
/// Output processed per frame across all runs.
pub(crate) const FRAME_BYTES: usize = 256 * 1024;
/// Retained text per run, and across all runs.
pub(crate) const RUN_LOG_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const GLOBAL_LOG_BYTES: usize = 32 * 1024 * 1024;
/// Completed runs kept for inspection; active runs are never evicted.
pub(crate) const MAX_FINISHED_RUNS: usize = 20;
/// Concurrent running processes (settings will make this configurable, 1-32).
pub(crate) const MAX_ACTIVE_RUNS: usize = 4;
/// Scrollback kept by a pseudo-terminal run (a bounded emulator, not a text log).
pub(crate) const PTY_HISTORY_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RunId(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    Succeeded(ExitReport),
    /// Exited with a code outside the task's allowed exit codes, or by signal.
    Failed(ExitReport),
    SpawnFailed(String),
    Cancelled,
    TimedOut,
    /// A task group in which every task succeeded.
    Completed,
    /// A task group that ended because the named task failed (or could not start).
    GroupFailed(String),
}

impl RunOutcome {
    pub(crate) fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded(_) | Self::Completed)
    }

    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Succeeded(report) => format!("Succeeded ({})", report.describe()),
            Self::Failed(report) => format!("Failed ({})", report.describe()),
            Self::SpawnFailed(message) => format!("Could not start: {message}"),
            Self::Cancelled => "Stopped".into(),
            Self::TimedOut => "Timed out and was stopped".into(),
            Self::Completed => "Completed".into(),
            Self::GroupFailed(label) => format!("Stopped: '{label}' failed"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopReason {
    User,
    Timeout,
    Restart,
    Close,
    Quit,
    Workspace,
    /// A sibling failed under the group's fail-fast policy.
    Group,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RunState {
    Running,
    /// A stop was requested; waiting for the process to confirm it exited.
    Stopping,
    Finished(RunOutcome),
}

pub(crate) struct TaskRun {
    pub(crate) id: RunId,
    pub(crate) key: TaskKey,
    pub(crate) label: String,
    pub(crate) cwd: PathBuf,
    /// The command line shown in the tab tooltip (a snapshot, never re-resolved).
    pub(crate) command_line: String,
    pub(crate) workspace: Option<PathBuf>,
    /// A task group's own tab: no process, only the group's progress log.
    pub(crate) summary: bool,
    /// The task group this run belongs to (its parent summary or a child).
    pub(crate) invocation: Option<super::task_graph::InvocationId>,
    pub(crate) started: Instant,
    pub(crate) started_at: String,
    pub(crate) ended: Option<Instant>,
    pub(crate) state: RunState,
    pub(crate) log: RunLog,
    /// Pseudo-terminal runs render through an emulator instead of the text log.
    terminal: Option<lumen_terminal_core::Terminal>,
    /// The emulator's screen as of the last output, rebuilt only when it changed.
    pub(crate) screen: Option<std::sync::Arc<lumen_terminal_core::Screen>>,
    screen_dirty: bool,
    /// Rows and columns the PTY and emulator currently have.
    pub(crate) pty_size: (u16, u16),
    resolved: ResolvedTask,
    process: Option<Box<dyn TaskProcess>>,
    exit: Option<ExitReport>,
    exited_at: Option<Instant>,
    closed_streams: u8,
    stop: Option<StopReason>,
    stop_requested_at: Option<Instant>,
    forced: bool,
    pub(super) restart_after: bool,
}

impl TaskRun {
    pub(crate) fn is_active(&self) -> bool {
        !matches!(self.state, RunState::Finished(_))
    }

    pub(super) fn new_summary(
        key: TaskKey,
        label: String,
        order: String,
        workspace: Option<PathBuf>,
        invocation: super::task_graph::InvocationId,
        now: Instant,
        log: RunLog,
    ) -> Self {
        Self {
            id: RunId(0),
            key,
            label,
            cwd: workspace.clone().unwrap_or_default(),
            command_line: order,
            workspace,
            summary: true,
            invocation: Some(invocation),
            started: now,
            started_at: chrono::Local::now().format("%H:%M:%S").to_string(),
            ended: None,
            state: RunState::Running,
            log,
            terminal: None,
            screen: None,
            screen_dirty: false,
            pty_size: (0, 0),
            resolved: ResolvedTask {
                label: String::new(),
                program: lumen_task_core::Program::Shell {
                    command: String::new(),
                },
                cwd: PathBuf::new(),
                env: Default::default(),
                unset_env: Vec::new(),
                transport: lumen_task_core::Transport::Pipe,
                exit_codes: vec![0],
                timeout_seconds: None,
                is_background: false,
                ready_pattern: None,
                ready_timeout_seconds: None,
            },
            process: None,
            exit: None,
            exited_at: None,
            closed_streams: 0,
            stop: None,
            stop_requested_at: None,
            forced: false,
            restart_after: false,
        }
    }

    pub(crate) fn outcome(&self) -> Option<&RunOutcome> {
        match &self.state {
            RunState::Finished(outcome) => Some(outcome),
            _ => None,
        }
    }

    pub(crate) fn is_pty(&self) -> bool {
        self.terminal.is_some()
    }

    /// Whether keyboard input can reach this run right now.
    pub(crate) fn accepts_input(&self) -> bool {
        self.is_pty() && matches!(self.state, RunState::Running)
    }

    pub(crate) fn has_output(&self) -> bool {
        match &self.terminal {
            Some(_) => self
                .screen
                .as_ref()
                .is_some_and(|screen| (0..screen.rows).any(|row| !screen.line(row).is_empty())),
            None => !self.log.is_empty(),
        }
    }

    /// Retained output as text, for Copy.
    pub(crate) fn output_text(&self) -> String {
        match &self.terminal {
            Some(terminal) => terminal.text(),
            None => self.log.text(),
        }
    }

    pub(crate) fn elapsed(&self, now: Instant) -> Duration {
        self.ended
            .unwrap_or(now)
            .saturating_duration_since(self.started)
    }
}

#[derive(Default)]
pub(crate) struct TaskRuns {
    pub(super) runs: Vec<TaskRun>,
    next_id: u64,
    pub(super) invocations: Vec<super::task_graph::Invocation>,
    pub(super) next_invocation: u64,
    /// The run whose tab is shown in the bottom panel.
    pub(crate) selected: Option<RunId>,
    /// Whether the task output panel is showing.
    pub(crate) panel_open: bool,
    /// Focus the interactive terminal on the next frame (set when a run is shown).
    pub(crate) focus_canvas: bool,
    /// The task most recently started, for Rerun Last Task.
    pub(crate) last: Option<TaskKey>,
    /// Runs to remove from the panel as soon as they finish (Stop and Close).
    pub(crate) close_when_finished: Vec<RunId>,
}

impl TaskRuns {
    pub(crate) fn runs(&self) -> &[TaskRun] {
        &self.runs
    }

    pub(crate) fn get(&self, id: RunId) -> Option<&TaskRun> {
        self.runs.iter().find(|run| run.id == id)
    }

    pub(super) fn get_mut(&mut self, id: RunId) -> Option<&mut TaskRun> {
        self.runs.iter_mut().find(|run| run.id == id)
    }

    pub(crate) fn selected_run(&self) -> Option<&TaskRun> {
        self.get(self.selected?)
    }

    pub(crate) fn active_count(&self) -> usize {
        self.runs.iter().filter(|run| run.is_active()).count()
    }

    /// Running processes: what the concurrency limit counts (a group's summary
    /// tab has no process).
    pub(crate) fn active_process_count(&self) -> usize {
        self.runs
            .iter()
            .filter(|run| run.is_active() && !run.summary)
            .count()
    }

    /// The label of the group a child run belongs to, if its summary tab exists.
    pub(crate) fn parent_label(&self, id: RunId) -> Option<String> {
        let run = self.get(id)?;
        if run.summary {
            return None;
        }
        let group = run.invocation?;
        self.runs
            .iter()
            .find(|candidate| candidate.summary && candidate.invocation == Some(group))
            .map(|parent| parent.label.clone())
    }

    /// Whether a run belongs to a group that is still going (its tab must stay).
    pub(super) fn in_live_group(&self, id: RunId) -> bool {
        self.get(id)
            .and_then(|run| run.invocation)
            .is_some_and(|group| self.invocations.iter().any(|inv| inv.id == group))
    }

    pub(crate) fn active_for(&self, key: &TaskKey) -> Option<&TaskRun> {
        self.runs
            .iter()
            .find(|run| run.is_active() && run.key == *key)
    }

    /// Tab title: the label, with `· #n` when several runs share a label.
    pub(crate) fn title(&self, id: RunId) -> String {
        let Some(run) = self.get(id) else {
            return String::new();
        };
        let same: Vec<_> = self.runs.iter().filter(|r| r.label == run.label).collect();
        if same.len() < 2 {
            return run.label.clone();
        }
        let position = same.iter().position(|r| r.id == id).unwrap_or(0) + 1;
        format!("{} · #{position}", run.label)
    }

    pub(super) fn insert(&mut self, mut run: TaskRun) -> RunId {
        self.next_id += 1;
        run.id = RunId(self.next_id);
        let id = run.id;
        self.runs.push(run);
        id
    }

    /// Remove a finished run; active runs must be stopped first.
    pub(crate) fn remove_finished(&mut self, id: RunId) -> bool {
        let Some(position) = self.runs.iter().position(|run| run.id == id) else {
            return false;
        };
        if self.runs[position].is_active() {
            return false;
        }
        self.runs.remove(position);
        if self.selected == Some(id) {
            self.selected = self.runs.last().map(|run| run.id);
        }
        true
    }

    /// Drop the oldest finished runs beyond the count and global text budgets.
    fn enforce_retention(&mut self) {
        loop {
            let finished = self.runs.iter().filter(|run| !run.is_active()).count();
            let bytes: usize = self.runs.iter().map(|run| run.log.retained_bytes()).sum();
            if finished <= MAX_FINISHED_RUNS && bytes <= GLOBAL_LOG_BYTES {
                return;
            }
            let Some(oldest) = self.runs.iter().find(|run| !run.is_active()).map(|r| r.id) else {
                return;
            };
            self.remove_finished(oldest);
        }
    }
}

/// A command line for display: program and arguments, or the shell command.
pub(super) fn command_line(task: &ResolvedTask) -> String {
    match &task.program {
        lumen_task_core::Program::Process { program, args } => std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        lumen_task_core::Program::Shell { command } => command.clone(),
    }
}

impl LumenApp {
    /// Resolve and start a task (and everything it depends on) by key, or stage the
    /// review/save prompt first. Nothing launches until the whole graph validated
    /// and resolved.
    pub(crate) fn run_task(&mut self, key: TaskKey, ctx: &egui::Context) {
        let plan = match self.plan_for_run(&key) {
            Ok(plan) => plan,
            Err(message) => {
                self.view.status = message;
                return;
            }
        };
        if plan.root_single
            && let Some(active) = self.view.task_runs.active_for(&key)
        {
            let id = active.id;
            self.show_task_run(id);
            self.view.status = format!(
                "'{}' is already running; use Restart to run it again",
                plan.label
            );
            return;
        }
        // Different invocations never silently share a process: a single-instance
        // task that is already running blocks a group that needs it.
        if let Some(node) = plan.nodes.iter().find(|node| {
            node.key != key
                && node.single
                && node.resolved.is_some()
                && self.view.task_runs.active_for(&node.key).is_some()
        }) {
            self.view.status = format!(
                "Cannot run '{}': '{}' is already running",
                plan.label, node.label
            );
            return;
        }
        if self.view.task_runs.active_process_count() >= MAX_ACTIVE_RUNS {
            self.view.status =
                format!("Too many tasks are running ({MAX_ACTIVE_RUNS}); stop one first");
            return;
        }
        let review: Vec<ReviewItem> =
            plan.nodes
                .iter()
                .filter_map(|node| {
                    let resolved = node.resolved.as_ref()?;
                    let untrusted =
                        node.key.scope == lumen_task_core::Scope::Workspace
                            && self.editor.workspace.as_deref().is_some_and(|root| {
                                !self.task_trust.is_trusted(root, &node.digest)
                            });
                    untrusted.then(|| ReviewItem {
                        label: node.label.clone(),
                        command_line: command_line(resolved),
                        cwd: resolved.cwd.display().to_string(),
                        env_names: resolved.env.keys().cloned().collect(),
                        digest: node.digest.clone(),
                    })
                })
                .collect();
        let save = plan.save_policy();
        let dirty = if save == SavePolicy::None {
            Vec::new()
        } else {
            self.dirty_saved_documents()
        };
        if !review.is_empty() || (save == SavePolicy::Prompt && !dirty.is_empty()) {
            self.view.task_pending = Some(PendingTaskRun {
                plan,
                review,
                dirty_files: dirty,
            });
            return;
        }
        if save == SavePolicy::Save && !self.save_dirty_documents().is_empty() {
            self.view.status = "Not run: some files could not be saved".into();
            return;
        }
        self.start_plan(plan, ctx);
    }

    pub(super) fn resolve_for_run(
        &self,
        key: &TaskKey,
        def: &TaskDef,
    ) -> Result<ResolvedTask, String> {
        let active_file = self
            .editor
            .active()
            .and_then(|document| document.path.clone());
        let user_base = self
            .task_user_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(std::path::Path::to_path_buf);
        let env = |name: &str| std::env::var(name).ok();
        let ctx = lumen_task_core::ResolveContext {
            workspace: self.editor.workspace.as_deref(),
            active_file: active_file.as_deref(),
            user_base: user_base.as_deref(),
            platform: lumen_task_core::Platform::current(),
            env: &env,
        };
        lumen_task_core::resolve(def, key.scope, &ctx).map_err(|error| error.to_string())
    }

    /// Dirty documents that have a path on disk (untitled buffers are never saved
    /// implicitly: a task cannot see them and no path is invented).
    pub(crate) fn dirty_saved_documents(&self) -> Vec<String> {
        self.editor
            .files
            .iter()
            .filter(|document| document.dirty && document.path.is_some())
            .map(|document| document.name.clone())
            .collect()
    }

    /// Save every dirty file that has a path; returns the names that stayed dirty.
    pub(crate) fn save_dirty_documents(&mut self) -> Vec<String> {
        let active = self.editor.active_index();
        for index in 0..self.editor.files.len() {
            if self.editor.files[index].dirty && self.editor.files[index].path.is_some() {
                self.editor.set_active_index(Some(index));
                self.save_active();
            }
        }
        self.editor.set_active_index(active);
        self.dirty_saved_documents()
    }

    /// Answer the review/save prompt. `save_first` writes dirty files before the run.
    pub(crate) fn confirm_pending_task(&mut self, save_first: bool, ctx: &egui::Context) {
        let Some(pending) = self.view.task_pending.take() else {
            return;
        };
        if save_first && !self.save_dirty_documents().is_empty() {
            self.view.status = "Not run: some files could not be saved".into();
            return;
        }
        if let Some(root) = self.editor.workspace.clone() {
            for item in &pending.review {
                if let Err(message) = self.task_trust.trust(&root, &item.digest) {
                    // Not being able to remember the review must not block this run.
                    self.view.status = format!("Could not remember the review: {message}");
                }
            }
        }
        self.start_plan(pending.plan, ctx);
    }

    pub(crate) fn cancel_pending_task(&mut self) {
        self.view.task_pending = None;
    }

    /// Launch a resolved task. A spawn failure is a visible, finished run, not a
    /// silent status line.
    pub(crate) fn start_run(
        &mut self,
        key: TaskKey,
        resolved: ResolvedTask,
        ctx: &egui::Context,
        reveal: bool,
        invocation: Option<super::task_graph::InvocationId>,
    ) -> RunId {
        // Dedicated output reuse: a rerun replaces that task's completed tab.
        let finished: Vec<RunId> = self
            .view
            .task_runs
            .runs
            .iter()
            .filter(|run| !run.is_active() && run.key == key)
            .map(|run| run.id)
            .collect();
        let finished: Vec<RunId> = finished
            .into_iter()
            .filter(|id| !self.view.task_runs.in_live_group(*id))
            .collect();
        let reuse = self.last_reuse(&key);
        if reuse == lumen_task_core::OutputReuse::Dedicated {
            for id in finished {
                self.view.task_runs.remove_finished(id);
            }
        }
        let repaint = ctx.clone();
        let wake: crate::services::language_server::ServerWake =
            std::sync::Arc::new(move || repaint.request_repaint());
        let path = self.task_search_path();
        let launched =
            self.task_launcher
                .launch(&resolved, &self.task_shell, path.as_deref(), wake);
        let now = Instant::now();
        let mut run = TaskRun {
            id: RunId(0),
            key: key.clone(),
            label: resolved.label.clone(),
            cwd: resolved.cwd.clone(),
            command_line: command_line(&resolved),
            workspace: self.editor.workspace.clone(),
            summary: false,
            invocation,
            started: now,
            started_at: chrono::Local::now().format("%H:%M:%S").to_string(),
            ended: None,
            state: RunState::Running,
            log: RunLog::new(RUN_LOG_BYTES),
            terminal: None,
            screen: None,
            screen_dirty: false,
            pty_size: (
                crate::services::task_process::PTY_ROWS,
                crate::services::task_process::PTY_COLUMNS,
            ),
            resolved,
            process: None,
            exit: None,
            exited_at: None,
            closed_streams: 0,
            stop: None,
            stop_requested_at: None,
            forced: false,
            restart_after: false,
        };
        match launched {
            Ok(process) => {
                if process.is_pty() {
                    run.terminal = Some(lumen_terminal_core::Terminal::with_history(
                        usize::from(run.pty_size.0),
                        usize::from(run.pty_size.1),
                        PTY_HISTORY_LINES,
                    ));
                }
                run.process = Some(process);
                self.view.status = format!("Running '{}'", run.label);
            }
            Err(message) => {
                run.ended = Some(now);
                run.state = RunState::Finished(RunOutcome::SpawnFailed(message.clone()));
                run.log.push(
                    crate::model::task_output::Stream::Stderr,
                    format!("{message}\n").as_bytes(),
                );
                run.log.finish();
                self.view.status = message;
            }
        }
        let id = self.view.task_runs.insert(run);
        if invocation.is_none() {
            self.view.task_runs.last = Some(key.clone());
        }
        if reveal {
            self.reveal_task_run(id, &key);
        }
        self.view.task_runs.enforce_retention();
        id
    }

    /// Output reuse policy of the task's current definition (default: dedicated).
    fn last_reuse(&self, key: &TaskKey) -> lumen_task_core::OutputReuse {
        self.view
            .tasks
            .get(key)
            .map(TaskDef::effective_reuse)
            .unwrap_or(lumen_task_core::OutputReuse::Dedicated)
    }

    fn reveal_task_run(&mut self, id: RunId, key: &TaskKey) {
        let reveal = self
            .view
            .tasks
            .get(key)
            .map(TaskDef::effective_reveal)
            .unwrap_or(lumen_task_core::Reveal::Always);
        self.view.task_runs.selected = Some(id);
        if reveal == lumen_task_core::Reveal::Always {
            self.show_task_run(id);
        }
    }

    /// The PATH tasks see: login-shell directories first on platforms where a GUI
    /// app does not inherit them.
    fn task_search_path(&self) -> Option<std::ffi::OsString> {
        if !self.task_login_path {
            return None;
        }
        Some(crate::services::agent_process::merge_path(
            crate::services::agent_process::cached_login_path().map(|path| path.as_os_str()),
            std::env::var_os("PATH").as_deref(),
        ))
    }

    /// Make a run's tab the visible bottom panel.
    pub(crate) fn show_task_run(&mut self, id: RunId) {
        if self.view.task_runs.get(id).is_none() {
            return;
        }
        self.view.task_runs.selected = Some(id);
        self.view.task_runs.panel_open = true;
        self.view.terminal.open = false;
        self.view.output.open = false;
        // An interactive run takes the keyboard, like opening the Terminal does.
        self.view.task_runs.focus_canvas = self
            .view
            .task_runs
            .get(id)
            .is_some_and(TaskRun::accepts_input);
    }

    /// Send keyboard input to an interactive run; anything else is refused.
    pub(crate) fn send_task_input(&mut self, id: RunId, bytes: &[u8]) {
        let Some(run) = self.view.task_runs.get_mut(id) else {
            return;
        };
        if !run.accepts_input() {
            return;
        }
        let result = run
            .process
            .as_mut()
            .map_or(Err("The task is not running".into()), |p| p.write(bytes));
        if let Err(message) = result {
            self.view.status = message;
        }
    }

    /// Match the PTY and its emulator to the panel's measured size.
    pub(crate) fn resize_task_terminal(&mut self, id: RunId, rows: u16, columns: u16) {
        let (rows, columns) = (rows.max(1), columns.max(1));
        let Some(run) = self.view.task_runs.get_mut(id) else {
            return;
        };
        if !run.is_pty() || run.pty_size == (rows, columns) {
            return;
        }
        run.pty_size = (rows, columns);
        if let Some(terminal) = &mut run.terminal {
            terminal.resize(usize::from(rows), usize::from(columns));
            run.screen_dirty = true;
        }
        if run.is_active()
            && let Some(process) = run.process.as_mut()
            && let Err(message) = process.resize(rows, columns)
        {
            self.view.status = message;
        }
        // Rebuild now: the next frame draws at the new size.
        refresh_screen(run);
    }

    pub(crate) fn stop_task_run(&mut self, id: RunId, reason: StopReason) {
        let Some(run) = self.view.task_runs.get_mut(id) else {
            return;
        };
        if reason == StopReason::Quit {
            // Quitting wins over a pending restart.
            run.restart_after = false;
        }
        if !run.is_active() || run.stop.is_some() {
            return;
        }
        if run.summary {
            // A group has no process: winding it down stops its children.
            run.stop.get_or_insert(reason);
            self.stop_invocation(id, reason);
            return;
        }
        run.stop.get_or_insert(reason);
        run.state = RunState::Stopping;
        run.stop_requested_at = Some(Instant::now());
        let delivered = run
            .process
            .as_mut()
            .is_some_and(|process| process.request_stop());
        if !delivered && run.exit.is_none() {
            // The polite request could not be delivered; the grace timer still
            // escalates to a forced stop, and we say so rather than imply success.
            self.view.status = format!(
                "Could not ask '{}' to stop politely; it will be killed shortly",
                self.view
                    .task_runs
                    .get(id)
                    .map(|r| r.label.clone())
                    .unwrap_or_default()
            );
        }
    }

    /// Stop, wait for confirmed termination, then run the task again.
    pub(crate) fn restart_task_run(&mut self, id: RunId, ctx: &egui::Context) {
        // Restarting a task that belongs to a running group restarts the group.
        let id = self
            .view
            .task_runs
            .get(id)
            .filter(|run| !run.summary)
            .and_then(|run| run.invocation)
            .and_then(|group| {
                self.view
                    .task_runs
                    .invocations
                    .iter()
                    .find(|inv| inv.id == group)
                    .map(|inv| inv.parent)
            })
            .unwrap_or(id);
        let Some(run) = self.view.task_runs.get_mut(id) else {
            return;
        };
        if run.is_active() {
            run.restart_after = true;
            self.stop_task_run(id, StopReason::Restart);
        } else {
            let key = run.key.clone();
            self.run_task(key, ctx);
        }
    }

    /// Remove a finished run, or ask before stopping an active one.
    pub(crate) fn close_task_run(&mut self, id: RunId) {
        let Some(run) = self.view.task_runs.get(id) else {
            return;
        };
        if run.is_active() {
            self.view.task_run_close_prompt = Some(id);
        } else {
            self.view.task_runs.remove_finished(id);
        }
    }

    pub(crate) fn clear_task_output(&mut self, id: RunId) {
        if let Some(run) = self.view.task_runs.get_mut(id) {
            run.log.clear();
            if let Some(terminal) = &mut run.terminal {
                terminal.clear();
                run.screen_dirty = true;
                refresh_screen(run);
            }
        }
    }

    /// Stop everything for application exit; completion is polled.
    pub(crate) fn stop_all_task_runs(&mut self, reason: StopReason) {
        let ids: Vec<RunId> = self
            .view
            .task_runs
            .runs
            .iter()
            .filter(|run| run.is_active())
            .map(|run| run.id)
            .collect();
        for id in ids {
            self.stop_task_run(id, reason);
        }
    }

    /// Workspace-scope runs started under the current folder that are still going.
    pub(crate) fn workspace_runs_active(&self) -> Vec<RunId> {
        self.view
            .task_runs
            .runs
            .iter()
            .filter(|run| {
                run.is_active()
                    && run.key.scope == lumen_task_core::Scope::Workspace
                    && run.workspace == self.editor.workspace
            })
            .map(|run| run.id)
            .collect()
    }

    /// Ask before a folder change strands running workspace tasks. Returns true
    /// when the change must wait for the user's answer.
    pub(crate) fn guard_workspace_change(&mut self, target: Option<PathBuf>) -> bool {
        if self
            .view
            .task_workspace_switch
            .as_ref()
            .is_some_and(|p| p.stopping)
        {
            return false;
        }
        if target == self.editor.workspace || self.workspace_runs_active().is_empty() {
            return false;
        }
        self.view.task_workspace_switch = Some(PendingWorkspaceSwitch {
            target,
            stopping: false,
        });
        true
    }

    pub(crate) fn confirm_workspace_switch(&mut self) {
        let Some(pending) = &mut self.view.task_workspace_switch else {
            return;
        };
        pending.stopping = true;
        for id in self.workspace_runs_active() {
            self.stop_task_run(id, StopReason::Workspace);
        }
    }

    pub(crate) fn cancel_workspace_switch(&mut self) {
        self.view.task_workspace_switch = None;
    }

    /// Ask before quitting with tasks running. True when quit must wait.
    pub(crate) fn guard_quit_for_tasks(&mut self) -> bool {
        if self.view.task_runs.active_count() == 0 {
            return false;
        }
        if self.view.task_quit.is_none() {
            self.view.task_quit = Some(PendingTaskQuit { deadline: None });
        }
        true
    }

    pub(crate) fn confirm_task_quit(&mut self, now: Instant) {
        let Some(pending) = &mut self.view.task_quit else {
            return;
        };
        pending.deadline = Some(now + STOP_GRACE + Duration::from_secs(3));
        self.stop_all_task_runs(StopReason::Quit);
    }

    pub(crate) fn cancel_task_quit(&mut self) {
        self.view.task_quit = None;
    }

    /// Finish transitions that were waiting for runs to stop.
    fn advance_task_transitions(&mut self, now: Instant, ctx: &egui::Context) {
        if let Some(pending) = &self.view.task_workspace_switch
            && pending.stopping
            && self.workspace_runs_active().is_empty()
        {
            let target = pending.target.clone();
            match target {
                Some(folder) => self.open_folder_path(folder),
                None => self.close_folder(),
            }
            self.view.task_workspace_switch = None;
        }
        if let Some(pending) = &self.view.task_quit
            && let Some(deadline) = pending.deadline
            && (self.view.task_runs.active_count() == 0 || now >= deadline)
        {
            // Past the deadline, whatever is left is dropped (which kills its tree).
            self.view.task_quit = None;
            self.view.task_runs.runs.retain(|run| !run.is_active());
            self.request_quit(ctx);
        }
    }

    /// Per-frame: drain output within the budget, notice exits, escalate stops,
    /// enforce timeouts and publish final results. Cheap when nothing is running.
    pub(crate) fn poll_task_runs(&mut self, ctx: &egui::Context) {
        self.poll_task_runs_at(Instant::now(), ctx);
    }

    pub(crate) fn poll_task_runs_at(&mut self, now: Instant, ctx: &egui::Context) {
        let active = self.view.task_runs.active_count();
        if active == 0 {
            self.advance_task_transitions(now, ctx);
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        let budget = FRAME_BYTES / self.view.task_runs.active_process_count().max(1);
        let mut messages = Vec::new();
        let mut restarts = Vec::new();
        for run in self
            .view
            .task_runs
            .runs
            .iter_mut()
            .filter(|r| r.is_active())
        {
            if let Some(message) = poll_run(run, now, budget) {
                messages.push(message);
            }
            if !run.is_active()
                && run.restart_after
                && run.outcome() == Some(&RunOutcome::Cancelled)
            {
                restarts.push(run.key.clone());
            }
        }
        if let Some(message) = messages.pop() {
            self.view.status = message;
        }
        self.advance_invocations(now, ctx);
        for key in restarts {
            self.run_task(key, ctx);
        }
        let done: Vec<RunId> = self
            .view
            .task_runs
            .close_when_finished
            .iter()
            .copied()
            .filter(|id| {
                self.view
                    .task_runs
                    .get(*id)
                    .is_none_or(|run| !run.is_active())
            })
            .collect();
        for id in done {
            self.view
                .task_runs
                .close_when_finished
                .retain(|other| *other != id);
            self.view.task_runs.remove_finished(id);
        }
        self.view.task_runs.enforce_retention();
        self.advance_task_transitions(now, ctx);
    }

    pub(crate) fn copy_task_output(&mut self, id: RunId, ctx: &egui::Context) {
        if let Some(run) = self.view.task_runs.get(id) {
            let text = run.output_text();
            ctx.copy_text(text);
            self.view.status = "Copied task output".into();
        }
    }
}

fn feed(run: &mut TaskRun, stream: crate::model::task_output::Stream, bytes: &[u8]) {
    match &mut run.terminal {
        Some(terminal) => {
            terminal.feed(bytes);
            run.screen_dirty = true;
        }
        None => run.log.push(stream, bytes),
    }
}

/// Rebuild the cached screen if output arrived since the last build.
fn refresh_screen(run: &mut TaskRun) {
    if run.screen_dirty
        && let Some(terminal) = &run.terminal
    {
        run.screen = Some(std::sync::Arc::new(terminal.screen()));
        run.screen_dirty = false;
    }
}

/// Advance one run; returns a status message when it just finished.
fn poll_run(run: &mut TaskRun, now: Instant, budget: usize) -> Option<String> {
    // The process is detached from the run while it is polled so output can be
    // fed into the run's log or emulator without aliasing the borrow.
    let mut process = run.process.take()?;
    let message = poll_run_with(run, process.as_mut(), now, budget);
    if run.is_active() {
        run.process = Some(process);
    }
    message
}

fn poll_run_with(
    run: &mut TaskRun,
    process: &mut dyn TaskProcess,
    now: Instant,
    budget: usize,
) -> Option<String> {
    for event in process.drain(budget) {
        match event {
            TaskEvent::Output(stream, bytes) => feed(run, stream, &bytes),
            TaskEvent::Closed(_) => run.closed_streams += 1,
        }
    }
    refresh_screen(run);
    if run.exit.is_none() {
        match process.try_exit() {
            Ok(Some(report)) => {
                run.exit = Some(report);
                run.exited_at = Some(now);
            }
            Ok(None) => {}
            Err(message) => {
                run.log.push(
                    crate::model::task_output::Stream::Stderr,
                    format!("{message}\n").as_bytes(),
                );
            }
        }
    }
    if let Some(limit) = run.resolved.timeout_seconds
        && run.stop.is_none()
        && run.exit.is_none()
        && now.saturating_duration_since(run.started) >= Duration::from_secs(u64::from(limit))
    {
        run.stop = Some(StopReason::Timeout);
        run.state = RunState::Stopping;
        run.stop_requested_at = Some(now);
        process.request_stop();
    }
    if run.exit.is_none()
        && !run.forced
        && let Some(requested) = run.stop_requested_at
        && now.saturating_duration_since(requested) >= STOP_GRACE
    {
        run.forced = true;
        process.force_stop();
    }
    let exited = run.exit?;
    let drained = run.closed_streams >= 2;
    let overdue = run
        .exited_at
        .is_some_and(|at| now.saturating_duration_since(at) >= DRAIN_DEADLINE);
    if !(drained || overdue) {
        return None;
    }
    if !drained {
        // A descendant outlived the process and holds the pipes open: stop
        // waiting for output and take the leftovers down with the tree.
        process.force_stop();
    }
    // Everything already queued is delivered before the final summary.
    loop {
        let events = process.drain(usize::MAX);
        if events.is_empty() {
            break;
        }
        for event in events {
            if let TaskEvent::Output(stream, bytes) = event {
                feed(run, stream, &bytes);
            }
        }
    }
    refresh_screen(run);
    run.log.finish();
    let outcome = match run.stop {
        Some(StopReason::Timeout) => RunOutcome::TimedOut,
        Some(_) => RunOutcome::Cancelled,
        None => match exited {
            ExitReport::Code(code) if run.resolved.exit_codes.contains(&code) => {
                RunOutcome::Succeeded(exited)
            }
            other => RunOutcome::Failed(other),
        },
    };
    run.ended = Some(now);
    let message = format!("'{}': {}", run.label, outcome.summary());
    run.state = RunState::Finished(outcome);
    Some(message)
}

/// A folder change waiting for the user to decide what happens to running tasks.
pub(crate) struct PendingWorkspaceSwitch {
    /// The folder to open, or `None` for Close Folder.
    pub(crate) target: Option<PathBuf>,
    /// The user chose to stop the tasks; the switch happens once they have.
    pub(crate) stopping: bool,
}

/// Quit waiting on running tasks.
pub(crate) struct PendingTaskQuit {
    /// The user chose to stop them; quit continues once they have (or at the deadline).
    pub(crate) deadline: Option<Instant>,
}

/// One command the first-run review shows.
pub(crate) struct ReviewItem {
    pub(crate) label: String,
    pub(crate) command_line: String,
    pub(crate) cwd: String,
    pub(crate) env_names: Vec<String>,
    pub(crate) digest: String,
}

/// A run waiting on the review / unsaved-files prompt.
pub(crate) struct PendingTaskRun {
    pub(crate) plan: super::task_graph::StartPlan,
    /// Workspace commands not reviewed yet (empty when nothing needs review).
    pub(crate) review: Vec<ReviewItem>,
    pub(crate) dirty_files: Vec<String>,
}

impl PendingTaskRun {
    pub(crate) fn needs_review(&self) -> bool {
        !self.review.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::command::Command,
        model::{Document, task_output::Stream},
        services::task_process::fake::FakeTaskLauncher,
    };
    use lumen_task_core::Scope;
    use std::sync::Arc;

    struct Harness {
        app: LumenApp,
        fake: FakeTaskLauncher,
        ctx: egui::Context,
        root: tempfile::TempDir,
    }

    const TASKS: &str = r#"schema_version = 1

[[tasks]]
id = "build"
label = "Build"
type = "process"
command = "cargo"
args = ["build"]
save = "none"

[[tasks]]
id = "slow"
label = "Slow"
type = "shell"
command = "sleep 100"
save = "none"
timeout_seconds = 2
transport = "pipe"

[[tasks]]
id = "term"
label = "Term"
type = "shell"
command = "echo hi"
save = "none"

[[tasks]]
id = "multi"
label = "Multi"
type = "process"
command = "echo"
save = "none"
instance_policy = "parallel"
exit_codes = [0, 3]
"#;

    impl Harness {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let path = crate::services::tasks::workspace_path(root.path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, TASKS).unwrap();
            let fake = FakeTaskLauncher::default();
            let mut app = LumenApp::default();
            app.editor.workspace = Some(root.path().to_path_buf());
            app.task_launcher = Arc::new(fake.clone());
            app.task_user_path = Some(root.path().join("home/.lumen/tasks.toml"));
            Self {
                app,
                fake,
                ctx: egui::Context::default(),
                root,
            }
        }

        fn key(id: &str) -> TaskKey {
            TaskKey::new(Scope::Workspace, id)
        }

        fn run(&mut self, id: &str) {
            self.app.execute(Command::RunTask(Self::key(id)), &self.ctx);
        }

        fn poll_at(&mut self, at: Instant) {
            self.app.poll_task_runs_at(at, &self.ctx);
        }

        fn run_at(&self, index: usize) -> &TaskRun {
            &self.app.view.task_runs.runs()[index]
        }

        fn finish(&mut self, process: usize, code: i32) {
            let state = self.fake.process(process);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Code(code));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }

        fn trust_everything(&mut self) {
            // Workspace tasks are reviewed on first run; most tests are about running.
            let path = crate::services::tasks::workspace_path(self.root.path());
            let file = lumen_task_core::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
            for def in &file.tasks {
                let digest = lumen_task_core::execution_digest(def);
                self.app
                    .task_trust
                    .trust(self.root.path(), &digest)
                    .unwrap();
            }
        }

        /// Replace the workspace tasks file and trust everything in it.
        fn write_tasks(&mut self, body: &str) {
            let path = crate::services::tasks::workspace_path(self.root.path());
            std::fs::write(&path, format!("schema_version = 1\n{body}")).unwrap();
            self.trust_everything();
        }

        /// Report a child process as finished (exit code, pipes closed).
        fn complete(&mut self, process: usize, code: i32) {
            self.finish(process, code);
            self.poll_at(Instant::now());
        }

        fn launched_labels(&self) -> Vec<String> {
            self.fake
                .launched
                .lock()
                .unwrap()
                .iter()
                .map(|t| t.label.clone())
                .collect()
        }

        fn summary(&self) -> &TaskRun {
            self.app
                .view
                .task_runs
                .runs()
                .iter()
                .find(|run| run.summary)
                .expect("a group tab")
        }
    }

    #[test]
    fn a_reviewed_task_runs_with_a_resolved_command_streams_output_and_reports_success() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        let launched = h.fake.launched.lock().unwrap().clone();
        assert_eq!(launched.len(), 1);
        assert_eq!(
            launched[0].program,
            lumen_task_core::Program::Process {
                program: "cargo".into(),
                args: vec!["build".into()]
            }
        );
        assert_eq!(launched[0].cwd, h.root.path());
        assert!(h.app.view.task_runs.panel_open && h.app.view.task_runs.selected.is_some());
        assert!(!h.app.view.terminal.open && !h.app.view.output.open);
        assert!(matches!(h.run_at(0).state, RunState::Running));

        h.fake
            .process(0)
            .lock()
            .unwrap()
            .queued
            .push(TaskEvent::Output(
                Stream::Stdout,
                b"Compiling\nDone".to_vec(),
            ));
        let now = Instant::now();
        h.poll_at(now);
        assert_eq!(h.run_at(0).log.lines().len(), 1);
        assert_eq!(h.run_at(0).log.partial()[0].text, "Done");
        assert!(
            h.run_at(0).is_active(),
            "output alone does not finish a run"
        );
        h.finish(0, 0);
        h.poll_at(now);
        let run = h.run_at(0);
        assert_eq!(
            run.outcome(),
            Some(&RunOutcome::Succeeded(ExitReport::Code(0)))
        );
        assert!(
            run.log.text().contains("Done"),
            "the unfinished last line is flushed at the end"
        );
        assert!(h.app.view.status.contains("Succeeded"));
    }

    #[test]
    fn exit_codes_are_judged_against_the_allowed_list() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        h.finish(0, 2);
        h.poll_at(Instant::now());
        assert_eq!(
            h.run_at(0).outcome(),
            Some(&RunOutcome::Failed(ExitReport::Code(2)))
        );
        h.run("multi");
        h.finish(1, 3);
        h.poll_at(Instant::now());
        assert!(
            h.app
                .view
                .task_runs
                .runs()
                .iter()
                .any(|r| r.label == "Multi" && r.outcome().is_some_and(RunOutcome::is_success))
        );
        // A signal that was not requested is a failure, not a clean stop.
        h.run("build");
        {
            let state = h.fake.process(2);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Signal(11));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(Instant::now());
        let build = h
            .app
            .view
            .task_runs
            .runs()
            .iter()
            .find(|r| r.label == "Build")
            .unwrap();
        assert_eq!(
            build.outcome(),
            Some(&RunOutcome::Failed(ExitReport::Signal(11)))
        );
    }

    #[test]
    fn a_spawn_failure_is_a_visible_finished_run_with_its_message() {
        let mut h = Harness::new();
        h.trust_everything();
        *h.fake.fail_with.lock().unwrap() = Some("'cargo' was not found on PATH".into());
        h.run("build");
        let run = h.run_at(0);
        assert!(
            matches!(run.outcome(), Some(RunOutcome::SpawnFailed(m)) if m.contains("not found"))
        );
        assert!(run.log.text().contains("not found on PATH"));
        assert!(h.app.view.task_runs.panel_open);
        assert_eq!(h.app.view.task_runs.active_count(), 0);
    }

    #[test]
    fn a_single_instance_task_reveals_its_active_run_instead_of_starting_another() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        let first = h.app.view.task_runs.selected;
        h.app.view.task_runs.panel_open = false;
        h.run("build");
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
        assert_eq!(h.app.view.task_runs.selected, first);
        assert!(h.app.view.task_runs.panel_open, "the active run is shown");
        assert!(h.app.view.status.contains("already running"));
        // parallel tasks may overlap and get distinct tab titles
        h.run("multi");
        h.run("multi");
        assert_eq!(h.fake.launched.lock().unwrap().len(), 3);
        let ids: Vec<_> = h
            .app
            .view
            .task_runs
            .runs()
            .iter()
            .map(|r| h.app.view.task_runs.title(r.id))
            .collect();
        assert_eq!(ids, ["Build", "Multi · #1", "Multi · #2"]);
    }

    #[test]
    fn dedicated_output_replaces_the_finished_tab_and_never_an_active_one() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        h.finish(0, 0);
        h.poll_at(Instant::now());
        h.app.view.tasks.stale = false;
        h.run("build");
        assert_eq!(
            h.app.view.task_runs.runs().len(),
            1,
            "the old tab was replaced"
        );
        assert!(h.run_at(0).is_active());
    }

    #[test]
    fn stopping_waits_for_confirmed_exit_then_escalates_to_a_kill_after_the_grace() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        h.fake.process(0).lock().unwrap().ignores_stop = true;
        let start = Instant::now();
        let id = h.run_at(0).id;
        h.app.execute(Command::StopTaskRun(id), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
        assert!(matches!(h.run_at(0).state, RunState::Stopping));
        h.poll_at(start + Duration::from_millis(500));
        assert!(
            h.run_at(0).is_active(),
            "no confirmation yet, so not stopped"
        );
        assert_eq!(h.fake.process(0).lock().unwrap().force_stops, 0);
        h.poll_at(start + STOP_GRACE + Duration::from_millis(50));
        assert_eq!(h.fake.process(0).lock().unwrap().force_stops, 1);
        // The kill is confirmed by the exit report plus closed pipes.
        {
            let state = h.fake.process(0);
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stdout));
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(start + STOP_GRACE + Duration::from_millis(100));
        assert_eq!(h.run_at(0).outcome(), Some(&RunOutcome::Cancelled));
        // A stop request for an already finished run changes nothing.
        h.app.execute(Command::StopTaskRun(id), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
    }

    #[test]
    fn a_cooperative_process_stops_without_being_killed() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        let id = h.run_at(0).id;
        h.app.execute(Command::StopTaskRun(id), &h.ctx);
        {
            let state = h.fake.process(0);
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stdout));
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(Instant::now());
        assert_eq!(h.run_at(0).outcome(), Some(&RunOutcome::Cancelled));
        assert_eq!(h.fake.process(0).lock().unwrap().force_stops, 0);
    }

    #[test]
    fn restart_launches_again_only_after_the_old_process_is_confirmed_gone() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        let id = h.run_at(0).id;
        h.fake.process(0).lock().unwrap().ignores_stop = true;
        h.app.execute(Command::RestartTaskRun(id), &h.ctx);
        h.poll_at(Instant::now());
        assert_eq!(
            h.fake.launched.lock().unwrap().len(),
            1,
            "still waiting for the old one"
        );
        {
            let state = h.fake.process(0);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Signal(15));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(Instant::now());
        assert_eq!(h.fake.launched.lock().unwrap().len(), 2);
        assert_eq!(h.app.view.task_runs.runs().len(), 1);
        assert!(h.run_at(0).is_active());
        // Restarting a finished run is just running it again.
        h.finish(1, 0);
        h.poll_at(Instant::now());
        let id = h.run_at(0).id;
        h.app.execute(Command::RestartTaskRun(id), &h.ctx);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_timeout_requests_a_stop_and_is_reported_as_a_timeout() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("slow");
        let start = Instant::now();
        h.poll_at(start + Duration::from_secs(1));
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 0);
        h.poll_at(start + Duration::from_secs(3));
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
        {
            let state = h.fake.process(0);
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stdout));
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(start + Duration::from_secs(3));
        assert_eq!(h.run_at(0).outcome(), Some(&RunOutcome::TimedOut));
    }

    #[test]
    fn output_queued_at_exit_is_delivered_and_a_pipe_held_open_ends_at_the_deadline() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        let start = Instant::now();
        {
            let state = h.fake.process(0);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Code(0));
            // Only stdout closed; a descendant keeps stderr open.
            state
                .queued
                .push(TaskEvent::Output(Stream::Stdout, b"last words\n".to_vec()));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
        }
        h.poll_at(start);
        assert!(h.run_at(0).is_active(), "waiting for the other pipe");
        assert!(h.run_at(0).log.text().contains("last words"));
        h.poll_at(start + DRAIN_DEADLINE + Duration::from_millis(10));
        assert_eq!(
            h.run_at(0).outcome(),
            Some(&RunOutcome::Succeeded(ExitReport::Code(0)))
        );
        assert_eq!(
            h.fake.process(0).lock().unwrap().force_stops,
            1,
            "leftover descendants are taken down"
        );
    }

    #[test]
    fn output_is_processed_within_a_per_frame_budget() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        {
            let state = h.fake.process(0);
            let mut state = state.lock().unwrap();
            for _ in 0..100 {
                state
                    .queued
                    .push(TaskEvent::Output(Stream::Stdout, vec![b'x'; 8 * 1024]));
            }
        }
        h.poll_at(Instant::now());
        let queued = h.fake.process(0).lock().unwrap().queued.len();
        assert!(
            queued > 0 && queued < 100,
            "one frame took a bounded share, left {queued}"
        );
        for _ in 0..10 {
            h.poll_at(Instant::now());
        }
        assert!(
            h.fake.process(0).lock().unwrap().queued.is_empty(),
            "later frames drain the rest"
        );
    }

    #[test]
    fn workspace_tasks_are_reviewed_once_per_definition_and_user_tasks_are_not() {
        let mut h = Harness::new();
        h.run("build");
        assert!(
            h.fake.launched.lock().unwrap().is_empty(),
            "nothing runs before the review"
        );
        let pending = h.app.view.task_pending.as_ref().expect("review prompt");
        assert!(pending.needs_review() && pending.review[0].command_line == "cargo build");
        h.app.execute(Command::CancelTaskRun, &h.ctx);
        assert!(h.app.view.task_pending.is_none() && h.fake.launched.lock().unwrap().is_empty());
        h.run("build");
        h.app.execute(Command::ConfirmTaskRun(false), &h.ctx);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
        h.finish(0, 0);
        h.poll_at(Instant::now());
        h.run("build");
        assert_eq!(
            h.fake.launched.lock().unwrap().len(),
            2,
            "trusted: no second prompt"
        );
        h.finish(1, 0);
        h.poll_at(Instant::now());
        // Editing what the task executes means reviewing it again.
        let path = crate::services::tasks::workspace_path(h.root.path());
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("args = [\"build\"]", "args = [\"build\", \"--release\"]");
        std::fs::write(&path, text).unwrap();
        h.run("build");
        assert!(h.app.view.task_pending.is_some());
        assert_eq!(h.fake.launched.lock().unwrap().len(), 2);
        h.app.execute(Command::CancelTaskRun, &h.ctx);
        // A personal task is explicit execution and is not reviewed.
        let user_path = h.app.task_user_path.clone().unwrap();
        std::fs::create_dir_all(user_path.parent().unwrap()).unwrap();
        std::fs::write(&user_path, "schema_version = 1\n[[tasks]]\nid = \"mine\"\nlabel = \"Mine\"\ntype = \"process\"\ncommand = \"echo\"\nsave = \"none\"\n").unwrap();
        h.app
            .execute(Command::RunTask(TaskKey::new(Scope::User, "mine")), &h.ctx);
        assert!(h.app.view.task_pending.is_none());
        assert_eq!(h.fake.launched.lock().unwrap().len(), 3);
    }

    #[test]
    fn unsaved_files_prompt_and_each_answer_is_honored() {
        let mut h = Harness::new();
        h.trust_everything();
        let path = crate::services::tasks::workspace_path(h.root.path());
        let text = std::fs::read_to_string(&path).unwrap().replacen(
            "save = \"none\"",
            "save = \"prompt\"",
            1,
        );
        std::fs::write(&path, text).unwrap();
        let file = h.root.path().join("a.rs");
        std::fs::write(&file, "old").unwrap();
        let mut document = Document::from_path(file.clone(), "old".into());
        document.text = "new".into();
        document.dirty = true;
        h.app.editor.files.push(document);
        h.app.editor.set_active_index(Some(0));
        // The edited definition changed the digest (save is not execution-affecting) so trust holds.
        h.run("build");
        let pending = h
            .app
            .view
            .task_pending
            .as_ref()
            .expect("prompt for unsaved files");
        assert_eq!(pending.dirty_files, ["a.rs"]);
        assert!(!pending.needs_review());
        assert!(h.fake.launched.lock().unwrap().is_empty());
        h.app.execute(Command::ConfirmTaskRun(false), &h.ctx);
        assert_eq!(
            h.fake.launched.lock().unwrap().len(),
            1,
            "run without saving"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "old",
            "files untouched"
        );
        h.finish(0, 0);
        h.poll_at(Instant::now());
        h.run("build");
        h.app.execute(Command::ConfirmTaskRun(true), &h.ctx);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert!(!h.app.editor.files[0].dirty);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 2);
    }

    #[test]
    fn save_all_policy_saves_first_and_a_failed_save_blocks_the_run() {
        let mut h = Harness::new();
        h.trust_everything();
        let path = crate::services::tasks::workspace_path(h.root.path());
        let text = std::fs::read_to_string(&path).unwrap().replacen(
            "save = \"none\"",
            "save = \"save\"",
            1,
        );
        std::fs::write(&path, text).unwrap();
        let file = h.root.path().join("b.rs");
        std::fs::write(&file, "disk").unwrap();
        let mut document = Document::from_path(file.clone(), "disk".into());
        document.text = "edited".into();
        document.dirty = true;
        h.app.editor.files.push(document);
        h.app.editor.set_active_index(Some(0));
        h.run("build");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited");
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
        h.finish(0, 0);
        h.poll_at(Instant::now());
        // The disk copy changed behind Lumen's back: the save is refused, nothing runs.
        std::fs::write(&file, "someone else").unwrap();
        h.app.editor.files[0].text = "mine".into();
        h.app.editor.files[0].dirty = true;
        h.run("build");
        assert_eq!(
            h.fake.launched.lock().unwrap().len(),
            1,
            "no run when files could not be saved"
        );
        assert!(h.app.view.status.contains("could not be saved"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "someone else");
    }

    #[test]
    fn unresolvable_variables_and_compound_tasks_do_not_start() {
        let mut h = Harness::new();
        let path = crate::services::tasks::workspace_path(h.root.path());
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n[[tasks]]\nid = \"needs-file\"\nlabel = \"Needs file\"\ntype = \"process\"\ncommand = \"cat\"\nargs = [\"${file}\"]\nsave = \"none\"\n\n[[tasks]]\nid = \"all\"\nlabel = \"All\"\ntype = \"compound\"\ndepends_on = [\"build\"]\n");
        std::fs::write(&path, text).unwrap();
        h.run("needs-file");
        assert!(h.fake.launched.lock().unwrap().is_empty() && h.app.view.task_pending.is_none());
        assert!(
            h.app.view.status.contains("needs a saved file"),
            "{}",
            h.app.view.status
        );
        h.run("ghost");
        assert!(h.app.view.status.contains("no longer exists"));
        assert!(h.app.view.task_runs.runs().is_empty());
    }

    #[test]
    fn the_concurrency_limit_is_enforced_with_an_explanation() {
        let mut h = Harness::new();
        h.trust_everything();
        for _ in 0..MAX_ACTIVE_RUNS {
            h.run("multi");
        }
        assert_eq!(h.fake.launched.lock().unwrap().len(), MAX_ACTIVE_RUNS);
        h.run("multi");
        assert_eq!(h.fake.launched.lock().unwrap().len(), MAX_ACTIVE_RUNS);
        assert!(h.app.view.status.contains("Too many tasks"));
    }

    #[test]
    fn completed_history_is_bounded_and_active_runs_are_never_evicted() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("slow");
        let slow = h.run_at(0).id;
        for n in 0..(MAX_FINISHED_RUNS + 5) {
            h.run("multi");
            let index = h.fake.launched.lock().unwrap().len() - 1;
            h.finish(index, 0);
            h.poll_at(Instant::now());
            let _ = n;
        }
        let finished = h
            .app
            .view
            .task_runs
            .runs()
            .iter()
            .filter(|r| !r.is_active())
            .count();
        assert!(finished <= MAX_FINISHED_RUNS, "{finished}");
        assert!(
            h.app
                .view
                .task_runs
                .get(slow)
                .is_some_and(TaskRun::is_active)
        );
    }

    #[test]
    fn closing_a_finished_tab_removes_it_and_an_active_one_asks_first() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        let id = h.run_at(0).id;
        h.app.execute(Command::CloseTaskRun(id), &h.ctx);
        assert_eq!(h.app.view.task_run_close_prompt, Some(id));
        assert_eq!(h.app.view.task_runs.runs().len(), 1);
        h.app.execute(Command::CancelCloseTaskRun, &h.ctx);
        assert!(
            h.app.view.task_run_close_prompt.is_none()
                && h.fake.process(0).lock().unwrap().stop_requests == 0
        );
        h.app.execute(Command::CloseTaskRun(id), &h.ctx);
        h.app.execute(Command::StopAndCloseTaskRun(id), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
        assert_eq!(
            h.app.view.task_runs.runs().len(),
            1,
            "removed only once it has stopped"
        );
        {
            let state = h.fake.process(0);
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stdout));
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(Instant::now());
        assert!(h.app.view.task_runs.runs().is_empty());
        // A finished tab closes at once.
        h.run("build");
        h.finish(1, 0);
        h.poll_at(Instant::now());
        let id = h.run_at(0).id;
        h.app.execute(Command::CloseTaskRun(id), &h.ctx);
        assert!(
            h.app.view.task_runs.runs().is_empty() && h.app.view.task_run_close_prompt.is_none()
        );
    }

    #[test]
    fn clear_and_copy_act_on_the_named_run_only() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        h.run("multi");
        let (a, b) = (h.run_at(0).id, h.run_at(1).id);
        for process in 0..2 {
            h.fake
                .process(process)
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Output(
                    Stream::Stdout,
                    format!("run{process}\n").into_bytes(),
                ));
        }
        h.poll_at(Instant::now());
        h.app.execute(Command::ClearTaskOutput(a), &h.ctx);
        assert!(h.app.view.task_runs.get(a).unwrap().log.is_empty());
        assert!(
            h.app
                .view
                .task_runs
                .get(b)
                .unwrap()
                .log
                .text()
                .contains("run1")
        );
        h.app.execute(Command::CopyTaskOutput(b), &h.ctx);
        assert_eq!(h.app.view.status, "Copied task output");
    }

    #[test]
    fn quitting_with_running_tasks_asks_stops_them_then_continues_the_quit() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        h.app.request_quit(&h.ctx);
        assert!(h.app.view.task_quit.is_some() && !h.app.view.allow_quit);
        assert_eq!(
            h.fake.process(0).lock().unwrap().stop_requests,
            0,
            "asking stops nothing"
        );
        h.app.execute(Command::CancelTaskQuit, &h.ctx);
        assert!(h.app.view.task_quit.is_none() && h.run_at(0).is_active());

        h.app.request_quit(&h.ctx);
        h.app.execute(Command::ConfirmTaskQuit, &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
        assert!(
            !h.app.view.allow_quit,
            "not until the process is confirmed gone"
        );
        {
            let state = h.fake.process(0);
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stdout));
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(Instant::now());
        assert!(
            h.app.view.allow_quit,
            "quit continued once nothing was running"
        );
        assert!(h.app.view.task_quit.is_none());
    }

    #[test]
    fn a_task_that_will_not_stop_does_not_hold_the_quit_forever() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        h.fake.process(0).lock().unwrap().ignores_stop = true;
        h.app.request_quit(&h.ctx);
        let start = Instant::now();
        h.app.confirm_task_quit(start);
        h.poll_at(start + Duration::from_secs(1));
        assert!(!h.app.view.allow_quit);
        h.poll_at(start + STOP_GRACE + Duration::from_secs(4));
        assert!(
            h.app.view.allow_quit,
            "past the deadline the leftovers are dropped"
        );
        assert_eq!(h.app.view.task_runs.active_count(), 0);
        assert!(
            h.fake.process(0).lock().unwrap().dropped,
            "dropping the run releases its process"
        );
    }

    #[test]
    fn changing_folder_with_running_workspace_tasks_asks_and_only_then_switches() {
        let mut h = Harness::new();
        h.trust_everything();
        let other = tempfile::tempdir().unwrap();
        h.run("build");
        h.app.open_folder_path(other.path().to_path_buf());
        assert_eq!(
            h.app.editor.workspace.as_deref(),
            Some(h.root.path()),
            "not switched yet"
        );
        assert!(h.app.view.task_workspace_switch.is_some());
        h.app.execute(Command::CancelWorkspaceSwitch, &h.ctx);
        assert!(h.app.view.task_workspace_switch.is_none() && h.run_at(0).is_active());

        h.app.open_folder_path(other.path().to_path_buf());
        h.app.execute(Command::ConfirmWorkspaceSwitch, &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
        assert_eq!(
            h.app.editor.workspace.as_deref(),
            Some(h.root.path()),
            "waits for the stop"
        );
        {
            let state = h.fake.process(0);
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stdout));
            state
                .lock()
                .unwrap()
                .queued
                .push(TaskEvent::Closed(Stream::Stderr));
        }
        h.poll_at(Instant::now());
        assert_eq!(h.app.editor.workspace.as_deref(), Some(other.path()));
        assert!(h.app.view.task_workspace_switch.is_none());
        assert_eq!(h.run_at(0).outcome(), Some(&RunOutcome::Cancelled));
    }

    #[test]
    fn close_folder_asks_too_and_unrelated_changes_do_not() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("build");
        // Reopening the same folder is not a change.
        h.app.open_folder_path(h.root.path().to_path_buf());
        assert!(h.app.view.task_workspace_switch.is_none());
        h.app.execute(Command::CloseFolder, &h.ctx);
        assert!(h.app.view.task_workspace_switch.is_some() && h.app.editor.workspace.is_some());
        h.app.execute(Command::CancelWorkspaceSwitch, &h.ctx);
        // With nothing running a folder closes at once.
        h.finish(0, 0);
        h.poll_at(Instant::now());
        h.app.execute(Command::CloseFolder, &h.ctx);
        assert!(h.app.editor.workspace.is_none());
    }

    #[test]
    fn personal_task_runs_do_not_block_a_folder_change() {
        let mut h = Harness::new();
        let user_path = h.app.task_user_path.clone().unwrap();
        std::fs::create_dir_all(user_path.parent().unwrap()).unwrap();
        std::fs::write(&user_path, "schema_version = 1\n[[tasks]]\nid = \"mine\"\nlabel = \"Mine\"\ntype = \"process\"\ncommand = \"echo\"\nsave = \"none\"\n").unwrap();
        h.app
            .execute(Command::RunTask(TaskKey::new(Scope::User, "mine")), &h.ctx);
        assert_eq!(h.app.view.task_runs.active_count(), 1);
        let other = tempfile::tempdir().unwrap();
        h.app.open_folder_path(other.path().to_path_buf());
        assert_eq!(h.app.editor.workspace.as_deref(), Some(other.path()));
        assert!(h.run_at(0).is_active(), "the personal task keeps running");
    }

    #[test]
    fn rerun_last_default_group_and_selected_commands_find_their_task() {
        let mut h = Harness::new();
        h.trust_everything();
        h.app.execute(Command::RerunLastTask, &h.ctx);
        assert!(h.app.view.status.contains("No task has been run"));
        h.app.execute(Command::RunSelectedTask, &h.ctx);
        assert!(h.app.view.status.contains("Select a task"));
        h.app.execute(Command::RunDefaultTask("build"), &h.ctx);
        assert!(h.app.view.status.contains("No default build task"));
        let path = crate::services::tasks::workspace_path(h.root.path());
        let text = std::fs::read_to_string(&path).unwrap().replacen(
            "args = [\"build\"]",
            "args = [\"build\"]\ngroup = \"build\"\nis_default = true",
            1,
        );
        std::fs::write(&path, text).unwrap();
        h.trust_everything();
        h.app.execute(Command::RunDefaultTask("build"), &h.ctx);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
        h.finish(0, 0);
        h.poll_at(Instant::now());
        h.app.execute(Command::RerunLastTask, &h.ctx);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 2);
        h.app
            .execute(Command::SelectTask(Harness::key("multi")), &h.ctx);
        h.app.execute(Command::RunSelectedTask, &h.ctx);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 3);
        h.app.execute(Command::ShowRunningTasks, &h.ctx);
        assert_eq!(
            h.app.view.activity,
            crate::app::view_state::Activity::RunDebug
        );
    }

    #[test]
    fn shell_tasks_default_to_a_terminal_that_renders_escape_sequences() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("term");
        assert!(h.fake.launched.lock().unwrap()[0].transport == lumen_task_core::Transport::Pty);
        assert!(h.run_at(0).is_pty() && h.run_at(0).accepts_input());
        assert!(!h.run_at(0).has_output());
        h.fake
            .process(0)
            .lock()
            .unwrap()
            .queued
            .push(TaskEvent::Output(
                Stream::Stdout,
                b"\x1b[31mred\x1b[0m text\r\nsecond".to_vec(),
            ));
        h.poll_at(Instant::now());
        let run = h.run_at(0);
        let screen = run.screen.as_ref().expect("screen rebuilt after output");
        assert_eq!(screen.line(0), "red text");
        assert_eq!(screen.line(1), "second");
        assert_ne!(
            screen.cell(0, 0).unwrap().foreground,
            lumen_terminal_core::TerminalColor::Named(256),
            "colour reached the emulator"
        );
        assert!(
            run.has_output() && run.log.is_empty(),
            "no raw log for a terminal run"
        );
        assert_eq!(run.output_text(), "red text\nsecond\n");
        // A pipe run is not interactive and keeps the text log.
        h.run("build");
        let pipe = h
            .app
            .view
            .task_runs
            .runs()
            .iter()
            .find(|r| r.label == "Build")
            .unwrap();
        assert!(!pipe.is_pty() && !pipe.accepts_input());
    }

    #[test]
    fn keyboard_input_reaches_only_a_running_terminal_run() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("term");
        let id = h.run_at(0).id;
        h.app
            .execute(Command::SendTaskInput(id, b"ls\r".to_vec()), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().written, b"ls\r");
        h.fake.process(0).lock().unwrap().write_error = Some("pipe broke".into());
        h.app
            .execute(Command::SendTaskInput(id, b"x".to_vec()), &h.ctx);
        assert_eq!(h.app.view.status, "pipe broke");
        h.fake.process(0).lock().unwrap().write_error = None;
        // A pipe run never receives input.
        h.run("build");
        let pipe = h.app.view.task_runs.runs()[1].id;
        h.app
            .execute(Command::SendTaskInput(pipe, b"x".to_vec()), &h.ctx);
        assert!(h.fake.process(1).lock().unwrap().written.is_empty());
        // Once finished, input is dropped rather than written to a dead process.
        h.finish(0, 0);
        h.poll_at(Instant::now());
        h.app
            .execute(Command::SendTaskInput(id, b"late".to_vec()), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().written, b"ls\r");
    }

    #[test]
    fn the_terminal_follows_the_measured_panel_size_and_ignores_no_ops() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("term");
        let id = h.run_at(0).id;
        assert_eq!(h.fake.process(0).lock().unwrap().size, None);
        h.app
            .execute(Command::ResizeTaskTerminal(id, 30, 90), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().size, Some((30, 90)));
        assert_eq!(h.run_at(0).pty_size, (30, 90));
        h.fake.process(0).lock().unwrap().size = None;
        h.app
            .execute(Command::ResizeTaskTerminal(id, 30, 90), &h.ctx);
        assert_eq!(
            h.fake.process(0).lock().unwrap().size,
            None,
            "an unchanged size is not re-sent"
        );
        h.app.execute(Command::ResizeTaskTerminal(id, 0, 0), &h.ctx);
        assert_eq!(h.run_at(0).pty_size, (1, 1), "never below one cell");
        h.run("build");
        let pipe = h.app.view.task_runs.runs()[1].id;
        h.app
            .execute(Command::ResizeTaskTerminal(pipe, 10, 10), &h.ctx);
        assert!(h.fake.process(1).lock().unwrap().size.is_none());
    }

    #[test]
    fn clear_copy_and_focus_apply_to_terminal_runs_too() {
        let mut h = Harness::new();
        h.trust_everything();
        h.run("term");
        let id = h.run_at(0).id;
        assert!(
            h.app.view.task_runs.focus_canvas,
            "an interactive run takes the keyboard"
        );
        h.fake
            .process(0)
            .lock()
            .unwrap()
            .queued
            .push(TaskEvent::Output(Stream::Stdout, b"hello\r\n".to_vec()));
        h.poll_at(Instant::now());
        h.app.execute(Command::CopyTaskOutput(id), &h.ctx);
        assert_eq!(h.app.view.status, "Copied task output");
        h.app.execute(Command::ClearTaskOutput(id), &h.ctx);
        assert!(!h.run_at(0).has_output());
        assert_eq!(h.run_at(0).output_text(), "");
        h.app.view.task_runs.focus_canvas = false;
        h.run("build");
        assert!(
            !h.app.view.task_runs.focus_canvas,
            "pipe runs do not steal focus"
        );
    }

    /// `n` simple process tasks A.. with the given extra TOML lines per task, then
    /// a compound task `all`.
    fn group_file(extra: &[(&str, &str)], all: &str) -> String {
        let mut body = String::new();
        for (id, more) in extra {
            let label = id.to_uppercase();
            body.push_str(&format!(
                "\n[[tasks]]\nid = \"{id}\"\nlabel = \"{label}\"\ntype = \"process\"\ncommand = \"echo\"\nargs = [\"{id}\"]\nsave = \"none\"\n{more}\n"
            ));
        }
        body.push_str(&format!(
            "\n[[tasks]]\nid = \"all\"\nlabel = \"All\"\ntype = \"compound\"\n{all}\n"
        ));
        body
    }

    #[test]
    fn a_sequence_runs_one_task_at_a_time_in_order_and_the_group_reports_completion() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", ""), ("c", "")],
            "depends_on = [\"a\", \"b\", \"c\"]\ndepends_order = \"sequence\"",
        ));
        h.run("all");
        assert_eq!(h.launched_labels(), ["A"], "only the first starts");
        assert!(h.summary().is_active() && h.app.view.task_runs.selected == Some(h.summary().id));
        h.complete(0, 0);
        assert_eq!(h.launched_labels(), ["A", "B"]);
        h.complete(1, 0);
        assert_eq!(h.launched_labels(), ["A", "B", "C"]);
        assert!(h.summary().is_active(), "not done until the last one is");
        h.complete(2, 0);
        let summary = h.summary();
        assert_eq!(summary.outcome(), Some(&RunOutcome::Completed));
        let text = summary.log.text();
        for needle in ["[start] A", "[ok] A", "[start] C", "[ok] C", "Completed"] {
            assert!(text.contains(needle), "{needle} in {text}");
        }
        // The parent and each child have their own tabs; children know their parent.
        let runs = h.app.view.task_runs.runs();
        assert_eq!(runs.len(), 4);
        let child = runs.iter().find(|r| r.label == "B").unwrap();
        assert_eq!(
            h.app.view.task_runs.parent_label(child.id).as_deref(),
            Some("All")
        );
        assert!(h.app.view.status.contains("Completed"));
    }

    #[test]
    fn parallel_dependencies_start_together_and_a_shared_dependency_runs_once() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[
                ("build", ""),
                ("t1", "depends_on = [\"build\"]"),
                ("t2", "depends_on = [\"build\"]"),
            ],
            "depends_on = [\"t1\", \"t2\", \"build\"]\ndepends_order = \"parallel\"",
        ));
        h.run("all");
        assert_eq!(
            h.launched_labels(),
            ["BUILD"],
            "the shared prerequisite first, once"
        );
        h.complete(0, 0);
        assert_eq!(
            h.launched_labels(),
            ["BUILD", "T1", "T2"],
            "siblings run side by side"
        );
        h.complete(1, 0);
        h.complete(2, 0);
        assert_eq!(
            h.launched_labels().iter().filter(|l| *l == "BUILD").count(),
            1
        );
        assert_eq!(h.summary().outcome(), Some(&RunOutcome::Completed));
    }

    #[test]
    fn fail_fast_stops_running_siblings_and_skips_what_has_not_started() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", ""), ("c", ""), ("d", ""), ("e", "")],
            "depends_on = [\"a\", \"b\", \"c\", \"d\", \"e\"]\ndepends_order = \"parallel\"",
        ));
        h.run("all");
        assert_eq!(
            h.launched_labels().len(),
            MAX_ACTIVE_RUNS,
            "limited by the concurrency cap"
        );
        h.complete(0, 1);
        assert_eq!(
            h.fake.process(1).lock().unwrap().stop_requests,
            1,
            "siblings are stopped"
        );
        assert_eq!(h.fake.process(2).lock().unwrap().stop_requests, 1);
        assert_eq!(h.fake.process(3).lock().unwrap().stop_requests, 1);
        assert_eq!(h.launched_labels().len(), MAX_ACTIVE_RUNS, "E never starts");
        for process in 1..4 {
            h.complete(process, 0);
        }
        let summary = h.summary();
        assert_eq!(
            summary.outcome(),
            Some(&RunOutcome::GroupFailed("A".into()))
        );
        assert!(
            summary.log.text().contains("[skipped] E"),
            "{}",
            summary.log.text()
        );
    }

    #[test]
    fn the_continue_policy_lets_independent_tasks_finish_and_skips_only_dependents() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", ""), ("after_a", "depends_on = [\"a\"]")],
            "depends_on = [\"a\", \"b\", \"after_a\"]\ndepends_order = \"parallel\"\non_failure = \"continue\"",
        ));
        h.run("all");
        assert_eq!(h.launched_labels(), ["A", "B"]);
        h.complete(0, 2);
        assert_eq!(
            h.fake.process(1).lock().unwrap().stop_requests,
            0,
            "B is left alone"
        );
        assert!(h.summary().is_active());
        h.complete(1, 0);
        let summary = h.summary();
        assert_eq!(
            summary.outcome(),
            Some(&RunOutcome::GroupFailed("A".into()))
        );
        assert!(summary.log.text().contains("[skipped] AFTER_A"));
        assert_eq!(h.launched_labels(), ["A", "B"], "the dependent never ran");
    }

    #[test]
    fn a_task_that_cannot_start_fails_the_group() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "")],
            "depends_on = [\"a\", \"b\"]\ndepends_order = \"sequence\"",
        ));
        h.run("all");
        *h.fake.fail_with.lock().unwrap() = Some("cannot spawn b".into());
        h.complete(0, 0);
        let summary = h.summary();
        assert_eq!(
            summary.outcome(),
            Some(&RunOutcome::GroupFailed("B".into()))
        );
        let b = h
            .app
            .view
            .task_runs
            .runs()
            .iter()
            .find(|r| r.label == "B")
            .unwrap();
        assert!(matches!(b.outcome(), Some(RunOutcome::SpawnFailed(_))));
    }

    #[test]
    fn the_concurrency_limit_queues_tasks_and_starts_them_as_slots_free() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[
                ("a", ""),
                ("b", ""),
                ("c", ""),
                ("d", ""),
                ("e", ""),
                ("f", ""),
            ],
            "depends_on = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\ndepends_order = \"parallel\"",
        ));
        h.run("all");
        assert_eq!(h.launched_labels().len(), MAX_ACTIVE_RUNS);
        h.complete(0, 0);
        assert_eq!(
            h.launched_labels().len(),
            MAX_ACTIVE_RUNS + 1,
            "one finished, one more starts"
        );
        h.complete(1, 0);
        assert_eq!(h.launched_labels().len(), 6);
        for process in 2..6 {
            h.complete(process, 0);
        }
        assert_eq!(h.summary().outcome(), Some(&RunOutcome::Completed));
        assert_eq!(h.app.view.task_runs.active_count(), 0);
    }

    #[test]
    fn stopping_a_group_stops_its_children_skips_the_rest_and_ends_stopped() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "")],
            "depends_on = [\"a\", \"b\"]\ndepends_order = \"sequence\"",
        ));
        h.run("all");
        let parent = h.summary().id;
        h.app.execute(Command::StopTaskRun(parent), &h.ctx);
        assert_eq!(h.fake.process(0).lock().unwrap().stop_requests, 1);
        assert!(matches!(h.summary().state, RunState::Stopping));
        h.poll_at(Instant::now());
        h.complete(0, 0);
        assert_eq!(h.summary().outcome(), Some(&RunOutcome::Cancelled));
        assert_eq!(h.launched_labels(), ["A"], "B never started");
        assert!(h.summary().log.text().contains("[skipped] B"));
    }

    #[test]
    fn restarting_a_group_or_one_of_its_children_runs_the_whole_group_again() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "")],
            "depends_on = [\"a\", \"b\"]\ndepends_order = \"parallel\"",
        ));
        h.run("all");
        let child = h
            .app
            .view
            .task_runs
            .runs()
            .iter()
            .find(|r| r.label == "A")
            .unwrap()
            .id;
        h.app.execute(Command::RestartTaskRun(child), &h.ctx);
        assert_eq!(
            h.fake.process(0).lock().unwrap().stop_requests,
            1,
            "the group is wound down"
        );
        assert_eq!(
            h.launched_labels().len(),
            2,
            "no new process until the group has stopped"
        );
        h.complete(0, 0);
        h.complete(1, 0);
        assert_eq!(
            h.launched_labels().len(),
            4,
            "the whole group started again"
        );
        assert_eq!(
            h.app
                .view
                .task_runs
                .runs()
                .iter()
                .filter(|r| r.summary)
                .count(),
            1
        );
    }

    #[test]
    fn the_review_covers_every_workspace_command_in_the_group_before_anything_starts() {
        let mut h = Harness::new();
        let path = crate::services::tasks::workspace_path(h.root.path());
        std::fs::write(
            &path,
            format!(
                "schema_version = 1\n{}",
                group_file(&[("a", ""), ("b", "")], "depends_on = [\"a\", \"b\"]")
            ),
        )
        .unwrap();
        h.run("all");
        let pending = h.app.view.task_pending.as_ref().expect("review prompt");
        assert_eq!(pending.review.len(), 2, "both commands are listed");
        assert_eq!(pending.plan.label, "All");
        assert!(h.fake.launched.lock().unwrap().is_empty());
        h.app.execute(Command::ConfirmTaskRun(false), &h.ctx);
        assert_eq!(h.launched_labels(), ["A"]);
        h.complete(0, 0);
        h.complete(1, 0);
        // Both are trusted now, so the group runs again without a prompt.
        h.run("all");
        assert!(h.app.view.task_pending.is_none());
    }

    #[test]
    fn a_bad_graph_aborts_before_any_process_starts() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("bad", "cwd = \"${fileDirname}\"")],
            "depends_on = [\"a\", \"bad\"]",
        ));
        h.run("all");
        assert!(
            h.fake.launched.lock().unwrap().is_empty(),
            "nothing started"
        );
        assert!(
            h.app.view.status.contains("'BAD'") && h.app.view.status.contains("saved file"),
            "{}",
            h.app.view.status
        );

        h.write_tasks(&group_file(
            &[(
                "a",
                "is_background = true\n[tasks.background]\nready_pattern = \"up\"",
            )],
            "depends_on = [\"a\"]",
        ));
        h.run("all");
        assert!(
            h.app.view.status.contains("background task"),
            "{}",
            h.app.view.status
        );
        assert!(h.fake.launched.lock().unwrap().is_empty());

        // A cycle is caught when the file is read: the whole file is refused.
        let path = crate::services::tasks::workspace_path(h.root.path());
        std::fs::write(
            &path,
            format!(
                "schema_version = 1\n{}",
                group_file(
                    &[("a", "depends_on = [\"b\"]"), ("b", "depends_on = [\"a\"]")],
                    "depends_on = [\"a\"]"
                )
            ),
        )
        .unwrap();
        h.run("all");
        assert!(
            h.app.view.status.contains("has problems") && h.app.view.status.contains("cycle"),
            "{}",
            h.app.view.status
        );
        assert!(h.fake.launched.lock().unwrap().is_empty());
    }

    #[test]
    fn a_task_with_dependencies_runs_them_first_then_itself_and_compounds_nest() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "depends_on = [\"a\"]"), ("c", "")],
            "depends_on = [\"b\", \"c\"]\ndepends_order = \"sequence\"",
        ));
        h.run("b");
        assert_eq!(
            h.launched_labels(),
            ["A"],
            "a process task waits for its dependency"
        );
        h.complete(0, 0);
        h.complete(1, 0);
        assert_eq!(h.summary().outcome(), Some(&RunOutcome::Completed));
        // A compound that depends on a compound: the middle node only orders.
        let mut h = Harness::new();
        h.write_tasks(&format!(
            "{}\n[[tasks]]\nid = \"top\"\nlabel = \"Top\"\ntype = \"compound\"\ndepends_on = [\"all\", \"c\"]\ndepends_order = \"sequence\"\n",
            group_file(&[("a", ""), ("c", "")], "depends_on = [\"a\"]")
        ));
        h.run("top");
        assert_eq!(h.launched_labels(), ["A"]);
        h.complete(0, 0);
        assert_eq!(
            h.launched_labels(),
            ["A", "C"],
            "the nested compound completed instantly"
        );
        h.complete(1, 0);
        assert_eq!(h.summary().outcome(), Some(&RunOutcome::Completed));
    }

    #[test]
    fn an_already_running_group_or_member_is_not_started_twice() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "")],
            "depends_on = [\"a\", \"b\"]\ndepends_order = \"parallel\"",
        ));
        h.run("all");
        let launched = h.fake.launched.lock().unwrap().len();
        h.app.view.task_runs.panel_open = false;
        h.run("all");
        assert_eq!(h.fake.launched.lock().unwrap().len(), launched);
        assert!(h.app.view.task_runs.panel_open && h.app.view.status.contains("already running"));
        // A member that is running on its own blocks a group that needs it.
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "")],
            "depends_on = [\"a\", \"b\"]",
        ));
        h.run("a");
        h.run("all");
        assert_eq!(h.launched_labels(), ["A"]);
        assert!(
            h.app.view.status.contains("'A' is already running"),
            "{}",
            h.app.view.status
        );
    }

    #[test]
    fn quitting_stops_a_whole_group_and_its_children() {
        let mut h = Harness::new();
        h.write_tasks(&group_file(
            &[("a", ""), ("b", "")],
            "depends_on = [\"a\", \"b\"]\ndepends_order = \"parallel\"",
        ));
        h.run("all");
        h.app.request_quit(&h.ctx);
        assert!(h.app.view.task_quit.is_some());
        h.app.execute(Command::ConfirmTaskQuit, &h.ctx);
        for process in 0..2 {
            assert_eq!(h.fake.process(process).lock().unwrap().stop_requests, 1);
            h.finish(process, 0);
        }
        h.poll_at(Instant::now());
        h.poll_at(Instant::now());
        assert!(h.app.view.allow_quit);
    }

    #[test]
    fn a_workspace_group_can_include_a_personal_task_by_explicit_reference_only() {
        let mut h = Harness::new();
        let user_path = h.app.task_user_path.clone().unwrap();
        std::fs::create_dir_all(user_path.parent().unwrap()).unwrap();
        std::fs::write(
            &user_path,
            "schema_version = 1\n[[tasks]]\nid = \"lint\"\nlabel = \"Personal lint\"\ntype = \"process\"\ncommand = \"echo\"\nargs = [\"lint\"]\ncwd = \"/\"\nsave = \"none\"\n",
        )
        .unwrap();
        h.write_tasks(&group_file(
            &[("a", "")],
            "depends_on = [\"a\", \"user:lint\"]\ndepends_order = \"sequence\"",
        ));
        h.run("all");
        assert_eq!(h.launched_labels(), ["A"]);
        h.complete(0, 0);
        assert_eq!(h.launched_labels(), ["A", "Personal lint"]);
        assert!(
            h.app.view.task_pending.is_none(),
            "personal tasks are not reviewed"
        );
        h.complete(1, 0);
        assert_eq!(h.summary().outcome(), Some(&RunOutcome::Completed));
        // Without the explicit prefix the same id would mean a workspace task.
        let path = crate::services::tasks::workspace_path(h.root.path());
        std::fs::write(
            &path,
            format!(
                "schema_version = 1\n{}",
                group_file(&[("a", "")], "depends_on = [\"lint\"]")
            ),
        )
        .unwrap();
        h.run("all");
        assert!(
            h.app.view.status.contains("problems") || h.app.view.status.contains("unknown"),
            "{}",
            h.app.view.status
        );
    }
}
