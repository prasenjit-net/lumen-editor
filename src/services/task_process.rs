//! Process transport for tasks. Owns the child, its reader threads and the stop
//! protocol; it never interprets output. Everything sits behind `TaskLauncher` /
//! `TaskProcess` so the app layer and its tests never spawn real processes.
//!
//! Output travels over a bounded channel: if the UI falls behind, readers block
//! (the pipe backs up) instead of the app buffering without limit, and the UI
//! drains every frame regardless of which tab is visible. Process exit and pipe
//! closure are separate events so final output is not lost to an early exit report.
// Wired to the task runner controller in the next step.
#![allow(dead_code)]
use super::{
    language_server::ServerWake,
    process_tree::{
        force_kill_process_tree, isolate_process_tree, resolve_program, terminate_process_tree,
    },
};
use crate::model::task_output::Stream;
use lumen_task_core::{Program, ResolvedTask};
use std::{
    ffi::{OsStr, OsString},
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

/// Queue depth in chunks. With 8 KiB reads this bounds buffered output near 512 KiB.
const QUEUE_CHUNKS: usize = 64;
const READ_CHUNK: usize = 8 * 1024;

/// How shell tasks are run: a non-interactive shell with one command string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShellSpec {
    pub(crate) program: OsString,
    /// Arguments placed before the command string (`-c`, `/C`).
    pub(crate) args: Vec<OsString>,
}

impl ShellSpec {
    pub(crate) fn platform_default() -> Self {
        if cfg!(windows) {
            Self {
                program: "cmd.exe".into(),
                args: vec!["/C".into()],
            }
        } else {
            Self {
                program: "/bin/sh".into(),
                args: vec!["-c".into()],
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskEvent {
    Output(Stream, Vec<u8>),
    /// A stream reached end-of-file (the child exited or closed it).
    Closed(Stream),
}

/// How a run ended, from the OS's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExitReport {
    Code(i32),
    /// Terminated by a signal (Unix).
    Signal(i32),
    Unknown,
}

impl ExitReport {
    pub(crate) fn from_status(status: ExitStatus) -> Self {
        if let Some(code) = status.code() {
            return Self::Code(code);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return Self::Signal(signal);
            }
        }
        Self::Unknown
    }

    pub(crate) fn describe(self) -> String {
        match self {
            Self::Code(code) => format!("exit code {code}"),
            Self::Signal(signal) => format!("signal {signal}"),
            Self::Unknown => "unknown exit status".into(),
        }
    }
}

pub(crate) trait TaskLauncher: Send + Sync {
    /// `path` replaces the inherited `PATH` when given (login-shell directories first).
    fn launch(
        &self,
        task: &ResolvedTask,
        shell: &ShellSpec,
        path: Option<&OsStr>,
        wake: ServerWake,
    ) -> Result<Box<dyn TaskProcess>, String>;
}

pub(crate) trait TaskProcess: Send {
    /// Up to `max_bytes` of queued output (always at least one event when any is
    /// queued), plus stream-closed markers. Never blocks.
    fn drain(&mut self, max_bytes: usize) -> Vec<TaskEvent>;
    /// `Some` once the process itself has exited (its pipes may still hold output).
    fn try_exit(&mut self) -> Result<Option<ExitReport>, String>;
    /// Ask the tree to stop. True when the request was delivered; confirmation is
    /// `try_exit` returning `Some`. Safe to repeat.
    fn request_stop(&mut self) -> bool;
    /// Kill the tree now. Does not wait; poll `try_exit`.
    fn force_stop(&mut self) -> bool;
    /// Whether this run is a pseudo-terminal (output is raw terminal bytes and
    /// input can be sent).
    fn is_pty(&self) -> bool {
        false
    }
    /// Send keyboard input. Only pseudo-terminal runs accept it.
    fn write(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Err("This task has no input; set transport = \"pty\" to interact with it".into())
    }
    /// Tell a pseudo-terminal its new size in cells.
    fn resize(&mut self, _rows: u16, _columns: u16) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct LocalTaskLauncher;

impl TaskLauncher for LocalTaskLauncher {
    fn launch(
        &self,
        task: &ResolvedTask,
        shell: &ShellSpec,
        path: Option<&OsStr>,
        wake: ServerWake,
    ) -> Result<Box<dyn TaskProcess>, String> {
        match task.transport {
            lumen_task_core::Transport::Pipe => {
                PipeProcess::spawn(task, shell, path, wake).map(|process| Box::new(process) as _)
            }
            lumen_task_core::Transport::Pty => {
                PtyProcess::spawn(task, shell, path, wake).map(|process| Box::new(process) as _)
            }
        }
    }
}

struct PipeProcess {
    child: Child,
    events: Receiver<TaskEvent>,
    exited: Option<ExitReport>,
}

impl PipeProcess {
    fn spawn(
        task: &ResolvedTask,
        shell: &ShellSpec,
        path: Option<&OsStr>,
        wake: ServerWake,
    ) -> Result<Self, String> {
        let mut command = build_command(task, shell, path)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        isolate_process_tree(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start '{}': {error}", task.label))?;
        let stdout = child.stdout.take().ok_or("The task has no stdout")?;
        let stderr = child.stderr.take().ok_or("The task has no stderr")?;
        let (sender, events) = mpsc::sync_channel(QUEUE_CHUNKS);
        let (out_sender, out_wake) = (sender.clone(), wake.clone());
        thread::spawn(move || read_stream(stdout, Stream::Stdout, out_sender, out_wake));
        thread::spawn(move || read_stream(stderr, Stream::Stderr, sender, wake));
        Ok(Self {
            child,
            events,
            exited: None,
        })
    }
}

/// The command to spawn, with the environment applied. Separate so the exact
/// program, arguments and environment can be tested without running anything.
pub(crate) fn build_command(
    task: &ResolvedTask,
    shell: &ShellSpec,
    path: Option<&OsStr>,
) -> Result<Command, String> {
    let effective_path = match task.env.get("PATH") {
        Some(task_path) => Some(OsString::from(task_path)),
        None => path
            .map(OsStr::to_os_string)
            .or_else(|| std::env::var_os("PATH")),
    };
    let mut command = match &task.program {
        Program::Process { program, args } => {
            let resolved = resolve_program(program, effective_path.as_deref())
                .ok_or_else(|| format!("'{program}' was not found on PATH"))?;
            let mut command = Command::new(resolved);
            command.args(args);
            command
        }
        Program::Shell { command: text } => {
            let mut command = Command::new(&shell.program);
            command.args(&shell.args);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.raw_arg(text);
            }
            #[cfg(not(windows))]
            command.arg(text);
            command
        }
    };
    command.current_dir(&task.cwd);
    for name in &task.unset_env {
        command.env_remove(name);
    }
    for (name, value) in &task.env {
        command.env(name, value);
    }
    if let Some(path) = effective_path {
        command.env("PATH", path);
    }
    Ok(command)
}

fn read_stream<R: Read>(
    mut source: R,
    stream: Stream,
    sender: SyncSender<TaskEvent>,
    wake: ServerWake,
) {
    let mut buffer = [0u8; READ_CHUNK];
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                // A full queue blocks here until the UI drains it; a closed one
                // means the run was dropped, so stop reading.
                if sender
                    .send(TaskEvent::Output(stream, buffer[..read].to_vec()))
                    .is_err()
                {
                    return;
                }
                wake();
            }
        }
    }
    let _ = sender.send(TaskEvent::Closed(stream));
    wake();
}

impl TaskProcess for PipeProcess {
    fn drain(&mut self, max_bytes: usize) -> Vec<TaskEvent> {
        let mut events = Vec::new();
        let mut bytes = 0;
        while let Ok(event) = self.events.try_recv() {
            if let TaskEvent::Output(_, data) = &event {
                bytes += data.len();
            }
            events.push(event);
            if bytes >= max_bytes {
                break;
            }
        }
        events
    }

    fn try_exit(&mut self) -> Result<Option<ExitReport>, String> {
        if self.exited.is_some() {
            return Ok(self.exited);
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exited = Some(ExitReport::from_status(status));
                Ok(self.exited)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(format!("Could not inspect the task process: {error}")),
        }
    }

    fn request_stop(&mut self) -> bool {
        self.exited.is_none() && terminate_process_tree(self.child.id())
    }

    fn force_stop(&mut self) -> bool {
        // Also kill descendants that outlive an already-reaped leader.
        let delivered = force_kill_process_tree(self.child.id());
        let _ = self.child.kill();
        delivered
    }
}

impl Drop for PipeProcess {
    fn drop(&mut self) {
        // Dropping a run never leaves its tree running unobserved.
        if self.exited.is_none() && matches!(self.child.try_wait(), Ok(None)) {
            force_kill_process_tree(self.child.id());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

// ------------------------------------------------------------------ PTY

/// Initial size before the panel measures itself and resizes the terminal.
pub(crate) const PTY_ROWS: u16 = 24;
pub(crate) const PTY_COLUMNS: u16 = 100;

struct PtyProcess {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    pid: Option<u32>,
    events: Receiver<TaskEvent>,
    exited: Option<ExitReport>,
}

impl PtyProcess {
    fn spawn(
        task: &ResolvedTask,
        shell: &ShellSpec,
        path: Option<&OsStr>,
        wake: ServerWake,
    ) -> Result<Self, String> {
        let command = build_pty_command(task, shell, path)?;
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: PTY_ROWS,
                cols: PTY_COLUMNS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                format!("Could not create a terminal for '{}': {error}", task.label)
            })?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("Could not start '{}': {error}", task.label))?;
        // Only the child keeps the slave open, so EOF means it and its
        // descendants are gone (or have released the terminal).
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("Could not read the task's terminal: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("Could not write to the task's terminal: {error}"))?;
        let pid = child.process_id();
        let (sender, events) = mpsc::sync_channel(QUEUE_CHUNKS);
        thread::spawn(move || read_pty(reader, sender, wake));
        Ok(Self {
            master: pair.master,
            writer,
            child,
            pid,
            events,
            exited: None,
        })
    }
}

/// The PTY command: same program/argument/environment rules as pipes, plus the
/// terminal type the emulator on the other end understands.
fn build_pty_command(
    task: &ResolvedTask,
    shell: &ShellSpec,
    path: Option<&OsStr>,
) -> Result<portable_pty::CommandBuilder, String> {
    let effective_path = match task.env.get("PATH") {
        Some(task_path) => Some(OsString::from(task_path)),
        None => path
            .map(OsStr::to_os_string)
            .or_else(|| std::env::var_os("PATH")),
    };
    let mut command = match &task.program {
        Program::Process { program, args } => {
            let resolved = resolve_program(program, effective_path.as_deref())
                .ok_or_else(|| format!("'{program}' was not found on PATH"))?;
            let mut command = portable_pty::CommandBuilder::new(resolved);
            command.args(args);
            command
        }
        Program::Shell { command: text } => {
            let mut command = portable_pty::CommandBuilder::new(&shell.program);
            command.args(&shell.args);
            command.arg(text);
            command
        }
    };
    command.cwd(&task.cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for name in &task.unset_env {
        command.env_remove(name);
    }
    for (name, value) in &task.env {
        command.env(name, value);
    }
    if let Some(path) = effective_path {
        command.env("PATH", path);
    }
    Ok(command)
}

fn read_pty(mut reader: Box<dyn Read + Send>, sender: SyncSender<TaskEvent>, wake: ServerWake) {
    let mut buffer = [0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if sender
                    .send(TaskEvent::Output(Stream::Stdout, buffer[..read].to_vec()))
                    .is_err()
                {
                    return;
                }
                wake();
            }
        }
    }
    // One terminal stream: report both as closed so the run can complete.
    let _ = sender.send(TaskEvent::Closed(Stream::Stdout));
    let _ = sender.send(TaskEvent::Closed(Stream::Stderr));
    wake();
}

fn signal_number(name: &str) -> Option<i32> {
    // portable-pty reports the signal by its description, which some platforms
    // suffix with the number ("Terminated: 15").
    if let Some((_, number)) = name.rsplit_once(": ")
        && let Ok(number) = number.trim().parse()
    {
        return Some(number);
    }
    Some(match name {
        "Hangup" => 1,
        "Interrupt" => 2,
        "Quit" => 3,
        "Aborted" | "Abort trap" => 6,
        "Killed" => 9,
        "Segmentation fault" => 11,
        "Terminated" => 15,
        _ => return None,
    })
}

impl TaskProcess for PtyProcess {
    fn drain(&mut self, max_bytes: usize) -> Vec<TaskEvent> {
        let mut events = Vec::new();
        let mut bytes = 0;
        while let Ok(event) = self.events.try_recv() {
            if let TaskEvent::Output(_, data) = &event {
                bytes += data.len();
            }
            events.push(event);
            if bytes >= max_bytes {
                break;
            }
        }
        events
    }

    fn try_exit(&mut self) -> Result<Option<ExitReport>, String> {
        if self.exited.is_some() {
            return Ok(self.exited);
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exited = Some(match status.signal().and_then(signal_number) {
                    Some(signal) => ExitReport::Signal(signal),
                    None if status.signal().is_some() => ExitReport::Unknown,
                    None => ExitReport::Code(status.exit_code() as i32),
                });
                Ok(self.exited)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(format!("Could not inspect the task process: {error}")),
        }
    }

    fn request_stop(&mut self) -> bool {
        self.exited.is_none() && self.pid.is_some_and(terminate_process_tree)
    }

    fn force_stop(&mut self) -> bool {
        let delivered = self.pid.is_some_and(force_kill_process_tree);
        let _ = self.child.kill();
        delivered
    }

    fn is_pty(&self) -> bool {
        true
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        use std::io::Write;
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("Could not send input to the task: {error}"))
    }

    fn resize(&mut self, rows: u16, columns: u16) -> Result<(), String> {
        self.master
            .resize(portable_pty::PtySize {
                rows: rows.max(1),
                cols: columns.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Could not resize the task's terminal: {error}"))
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        if self.exited.is_none() && matches!(self.child.try_wait(), Ok(None)) {
            if let Some(pid) = self.pid {
                force_kill_process_tree(pid);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

// --------------------------------------------------------------- test double

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Scripted process state shared with the test that launched it.
    #[derive(Default)]
    pub(crate) struct FakeState {
        pub(crate) queued: Vec<TaskEvent>,
        pub(crate) exit: Option<ExitReport>,
        pub(crate) stop_requests: usize,
        pub(crate) force_stops: usize,
        /// Exit only after a forced stop (a process that ignores the polite request).
        pub(crate) ignores_stop: bool,
        pub(crate) dropped: bool,
        pub(crate) pty: bool,
        pub(crate) written: Vec<u8>,
        pub(crate) size: Option<(u16, u16)>,
        pub(crate) write_error: Option<String>,
    }

    #[derive(Clone, Default)]
    pub(crate) struct FakeTaskLauncher {
        pub(crate) launched: Arc<Mutex<Vec<ResolvedTask>>>,
        pub(crate) processes: Arc<Mutex<Vec<Arc<Mutex<FakeState>>>>>,
        pub(crate) fail_with: Arc<Mutex<Option<String>>>,
    }

    impl FakeTaskLauncher {
        pub(crate) fn process(&self, index: usize) -> Arc<Mutex<FakeState>> {
            self.processes.lock().unwrap()[index].clone()
        }
    }

    struct FakeProcess(Arc<Mutex<FakeState>>);

    impl TaskLauncher for FakeTaskLauncher {
        fn launch(
            &self,
            task: &ResolvedTask,
            _shell: &ShellSpec,
            _path: Option<&OsStr>,
            _wake: ServerWake,
        ) -> Result<Box<dyn TaskProcess>, String> {
            if let Some(message) = self.fail_with.lock().unwrap().clone() {
                return Err(message);
            }
            self.launched.lock().unwrap().push(task.clone());
            let state = Arc::new(Mutex::new(FakeState {
                pty: task.transport == lumen_task_core::Transport::Pty,
                ..FakeState::default()
            }));
            self.processes.lock().unwrap().push(state.clone());
            Ok(Box::new(FakeProcess(state)))
        }
    }

    impl TaskProcess for FakeProcess {
        fn drain(&mut self, max_bytes: usize) -> Vec<TaskEvent> {
            let mut state = self.0.lock().unwrap();
            let mut taken = Vec::new();
            let mut bytes = 0;
            while !state.queued.is_empty() && bytes < max_bytes {
                let event = state.queued.remove(0);
                if let TaskEvent::Output(_, data) = &event {
                    bytes += data.len();
                }
                taken.push(event);
            }
            taken
        }
        fn try_exit(&mut self) -> Result<Option<ExitReport>, String> {
            Ok(self.0.lock().unwrap().exit)
        }
        fn request_stop(&mut self) -> bool {
            let mut state = self.0.lock().unwrap();
            state.stop_requests += 1;
            if !state.ignores_stop && state.exit.is_none() {
                state.exit = Some(ExitReport::Signal(15));
            }
            true
        }
        fn force_stop(&mut self) -> bool {
            let mut state = self.0.lock().unwrap();
            state.force_stops += 1;
            if state.exit.is_none() {
                state.exit = Some(ExitReport::Signal(9));
            }
            true
        }
        fn is_pty(&self) -> bool {
            self.0.lock().unwrap().pty
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            if let Some(error) = state.write_error.clone() {
                return Err(error);
            }
            if !state.pty {
                return Err("This task has no input".into());
            }
            state.written.extend_from_slice(bytes);
            Ok(())
        }
        fn resize(&mut self, rows: u16, columns: u16) -> Result<(), String> {
            self.0.lock().unwrap().size = Some((rows, columns));
            Ok(())
        }
    }

    impl Drop for FakeProcess {
        fn drop(&mut self) {
            self.0.lock().unwrap().dropped = true;
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::model::task_output::RunLog;
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        time::{Duration, Instant},
    };

    fn task(program: Program) -> ResolvedTask {
        ResolvedTask {
            label: "t".into(),
            program,
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            unset_env: Vec::new(),
            transport: lumen_task_core::Transport::Pipe,
            exit_codes: vec![0],
            timeout_seconds: None,
            is_background: false,
            ready_pattern: None,
            ready_timeout_seconds: None,
        }
    }

    fn shell(command: &str) -> ResolvedTask {
        task(Program::Shell {
            command: command.into(),
        })
    }

    fn launch(task: &ResolvedTask) -> Box<dyn TaskProcess> {
        LocalTaskLauncher
            .launch(
                task,
                &ShellSpec::platform_default(),
                None,
                std::sync::Arc::new(|| {}),
            )
            .expect("spawns")
    }

    /// Run to completion: exit AND both streams closed, collecting output.
    fn run(task: &ResolvedTask) -> (ExitReport, RunLog) {
        let mut process = launch(task);
        let mut log = RunLog::new(1 << 20);
        let (mut closed, mut exit) = (0, None);
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            for event in process.drain(usize::MAX) {
                match event {
                    TaskEvent::Output(stream, bytes) => log.push(stream, &bytes),
                    TaskEvent::Closed(_) => closed += 1,
                }
            }
            exit = exit.or(process.try_exit().unwrap());
            if let Some(report) = exit
                && closed == 2
            {
                log.finish();
                return (report, log);
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("task did not finish: exit={exit:?} closed={closed}");
    }

    fn texts(log: &RunLog) -> Vec<String> {
        log.lines().iter().map(|line| line.text.clone()).collect()
    }

    #[test]
    fn captures_both_streams_and_the_exit_code() {
        let (exit, log) = run(&shell("echo out; echo err 1>&2; exit 3"));
        assert_eq!(exit, ExitReport::Code(3));
        let lines: Vec<_> = log
            .lines()
            .iter()
            .map(|l| (l.stream, l.text.as_str()))
            .collect();
        assert!(
            lines.contains(&(Stream::Stdout, "out")) && lines.contains(&(Stream::Stderr, "err"))
        );
    }

    #[test]
    fn process_arguments_keep_their_boundaries_and_metacharacters_are_literal() {
        let task = task(Program::Process {
            program: "printf".into(),
            args: vec![
                "[%s]\\n".into(),
                "a b".into(),
                "$HOME; echo no".into(),
                "*".into(),
            ],
        });
        let (exit, log) = run(&task);
        assert_eq!(exit, ExitReport::Code(0));
        assert_eq!(texts(&log), ["[a b]", "[$HOME; echo no]", "[*]"]);
    }

    #[test]
    fn working_directory_environment_and_unset_apply() {
        let dir = tempfile::tempdir().unwrap();
        let mut task = shell("pwd; echo \"A=$A B=${B-unset}\"");
        task.cwd = dir.path().to_path_buf();
        task.env.insert("A".into(), "one".into());
        task.unset_env = vec!["B".into()];
        // SAFETY of the test: only reads/sets a child-only variable name.
        let (_, log) = run(&task);
        let lines = texts(&log);
        assert_eq!(
            PathBuf::from(&lines[0]).canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
        assert_eq!(lines[1], "A=one B=unset");
    }

    #[test]
    fn a_missing_program_or_directory_is_a_spawn_error_not_a_run() {
        let missing = task(Program::Process {
            program: "definitely-not-a-program-xyz".into(),
            args: vec![],
        });
        let error = LocalTaskLauncher
            .launch(
                &missing,
                &ShellSpec::platform_default(),
                None,
                std::sync::Arc::new(|| {}),
            )
            .err()
            .expect("fails");
        assert!(error.contains("not found on PATH"), "{error}");
        let mut bad_dir = shell("true");
        bad_dir.cwd = "/definitely/not/a/dir".into();
        let error = LocalTaskLauncher
            .launch(
                &bad_dir,
                &ShellSpec::platform_default(),
                None,
                std::sync::Arc::new(|| {}),
            )
            .err()
            .expect("fails");
        assert!(error.contains("Could not start"), "{error}");
    }

    #[test]
    fn the_path_override_is_used_to_find_programs() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("mytool");
        std::fs::write(&tool, "#!/bin/sh\necho from-mytool\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let task = task(Program::Process {
            program: "mytool".into(),
            args: vec![],
        });
        let mut process = LocalTaskLauncher
            .launch(
                &task,
                &ShellSpec::platform_default(),
                Some(dir.path().as_os_str()),
                std::sync::Arc::new(|| {}),
            )
            .unwrap();
        let mut log = RunLog::new(1 << 16);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && log.lines().is_empty() {
            for event in process.drain(usize::MAX) {
                if let TaskEvent::Output(stream, bytes) = event {
                    log.push(stream, &bytes);
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(texts(&log), ["from-mytool"]);
    }

    #[test]
    fn stopping_ends_the_whole_tree_including_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild.pid");
        // The shell starts a background sleeper (a descendant) and records its pid.
        let script = format!("sleep 60 & echo $! > '{}'; wait", marker.display());
        let mut process = launch(&shell(&script));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let pid: i32 = loop {
            if let Ok(text) = std::fs::read_to_string(&marker)
                && let Ok(pid) = text.trim().parse()
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "grandchild pid never written");
            thread::sleep(Duration::from_millis(10));
        };
        assert!(alive(pid), "descendant is running before the stop");
        assert!(process.request_stop());
        let mut exit = None;
        while exit.is_none() && Instant::now() < deadline {
            exit = process.try_exit().unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(exit, Some(ExitReport::Signal(15))), "{exit:?}");
        // The descendant must be gone too (poll: signals are asynchronous).
        let gone_by = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < gone_by {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "the descendant survived the stop");
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn a_process_that_ignores_the_polite_request_dies_to_the_forced_stop() {
        let mut process = launch(&shell("trap '' TERM; while true; do sleep 1; done"));
        thread::sleep(Duration::from_millis(300));
        assert!(process.request_stop());
        thread::sleep(Duration::from_millis(300));
        assert!(
            process.try_exit().unwrap().is_none(),
            "ignored SIGTERM, still running"
        );
        assert!(process.force_stop());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut exit = None;
        while exit.is_none() && Instant::now() < deadline {
            exit = process.try_exit().unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(exit, Some(ExitReport::Signal(9))), "{exit:?}");
    }

    #[test]
    fn a_flood_of_output_is_bounded_in_flight_and_fully_delivered_when_drained() {
        // ~12 MiB, far beyond the queue, so the reader must block on a slow consumer.
        let task = shell("head -c 12582912 /dev/zero | tr '\\0' 'x'; echo");
        let mut process = launch(&task);
        thread::sleep(Duration::from_millis(500));
        // Nothing has been drained, yet the child was held back by backpressure.
        let mut total = 0usize;
        let (mut closed, mut exit) = (0, None);
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            for event in process.drain(256 * 1024) {
                match event {
                    TaskEvent::Output(_, bytes) => total += bytes.len(),
                    TaskEvent::Closed(_) => closed += 1,
                }
            }
            exit = exit.or(process.try_exit().unwrap());
            if exit.is_some() && closed == 2 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(exit, Some(ExitReport::Code(0)));
        assert_eq!(
            total,
            12_582_912 + 1,
            "every byte arrived, none were dropped in transport"
        );
    }

    #[test]
    fn drain_respects_the_per_call_budget_but_always_makes_progress() {
        let mut process = launch(&shell("head -c 200000 /dev/zero | tr '\\0' 'y'"));
        thread::sleep(Duration::from_millis(300));
        let first = process.drain(1);
        assert!(
            !first.is_empty(),
            "at least one event even with a tiny budget"
        );
        let bytes: usize = first
            .iter()
            .map(|event| match event {
                TaskEvent::Output(_, data) => data.len(),
                _ => 0,
            })
            .sum();
        assert!(
            bytes <= READ_CHUNK,
            "a one-byte budget takes a single chunk, got {bytes}"
        );
    }

    fn pty(command: &str) -> ResolvedTask {
        let mut task = shell(command);
        task.transport = lumen_task_core::Transport::Pty;
        task
    }

    /// Everything the terminal emitted, until the run completes.
    fn pty_text(process: &mut Box<dyn TaskProcess>, until: &str) -> String {
        pty_text_counting(process, until, &mut 0)
    }

    fn pty_text_counting(
        process: &mut Box<dyn TaskProcess>,
        until: &str,
        closed: &mut usize,
    ) -> String {
        let mut text = String::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            for event in process.drain(usize::MAX) {
                match event {
                    TaskEvent::Output(_, bytes) => text.push_str(&String::from_utf8_lossy(&bytes)),
                    TaskEvent::Closed(_) => *closed += 1,
                }
            }
            if text.contains(until) {
                return text;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("never saw {until:?} in {text:?}");
    }

    #[test]
    fn a_pty_run_has_a_terminal_a_colour_term_and_a_real_exit_code() {
        let mut process = launch(&pty(
            "[ -t 1 ] && echo tty-yes; echo \"$TERM|$COLORTERM\"; exit 4",
        ));
        assert!(process.is_pty());
        let mut closed = 0;
        let text = pty_text_counting(&mut process, "xterm-256color|truecolor", &mut closed);
        assert!(text.contains("tty-yes"), "stdout is a terminal: {text:?}");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut exit = None;
        while exit.is_none() && Instant::now() < deadline {
            exit = process.try_exit().unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(exit, Some(ExitReport::Code(4)));
        // Both logical streams close so the run can complete.
        let end = Instant::now() + Duration::from_secs(10);
        while closed < 2 && Instant::now() < end {
            closed += process
                .drain(usize::MAX)
                .iter()
                .filter(|event| matches!(event, TaskEvent::Closed(_)))
                .count();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(closed, 2);
    }

    #[test]
    fn a_pty_run_takes_keyboard_input_and_a_pipe_run_refuses_it() {
        let mut process = launch(&pty("printf 'name? '; read name; echo \"got:$name\""));
        pty_text(&mut process, "name?");
        process.write(b"lumen\r").unwrap();
        let text = pty_text(&mut process, "got:lumen");
        assert!(text.contains("got:lumen"));
        let mut piped = launch(&shell("sleep 5"));
        assert!(!piped.is_pty());
        let error = piped.write(b"x").unwrap_err();
        assert!(error.contains("no input"), "{error}");
        piped.force_stop();
    }

    #[test]
    fn a_pty_run_follows_resizes() {
        let mut process = launch(&pty("sleep 1; stty size"));
        process.resize(31, 97).unwrap();
        let text = pty_text(&mut process, "31 97");
        assert!(text.contains("31 97"), "{text:?}");
    }

    #[test]
    fn stopping_a_pty_run_ends_its_descendants_too() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pid");
        let mut process = launch(&pty(&format!(
            "sleep 60 & echo $! > '{}'; wait",
            marker.display()
        )));
        let deadline = Instant::now() + Duration::from_secs(10);
        let pid: i32 = loop {
            if let Ok(text) = std::fs::read_to_string(&marker)
                && let Ok(pid) = text.trim().parse()
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "pid never written");
            thread::sleep(Duration::from_millis(10));
        };
        assert!(alive(pid));
        assert!(process.request_stop());
        let mut exit = None;
        while exit.is_none() && Instant::now() < deadline {
            exit = process.try_exit().unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(exit, Some(ExitReport::Signal(15))), "{exit:?}");
        let gone_by = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < gone_by {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "the descendant survived the stop");
    }

    #[test]
    fn a_pty_run_that_cannot_start_reports_why() {
        let missing = {
            let mut task = pty("true");
            task.program = Program::Process {
                program: "definitely-not-a-program-xyz".into(),
                args: vec![],
            };
            task
        };
        let error = LocalTaskLauncher
            .launch(
                &missing,
                &ShellSpec::platform_default(),
                None,
                std::sync::Arc::new(|| {}),
            )
            .err()
            .expect("fails");
        assert!(error.contains("not found on PATH"), "{error}");
    }

    #[test]
    fn dropping_a_running_process_kills_its_tree() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("pid");
        let process = launch(&shell(&format!(
            "sleep 60 & echo $! > '{}'; wait",
            marker.display()
        )));
        let deadline = Instant::now() + Duration::from_secs(10);
        let pid: i32 = loop {
            if let Ok(text) = std::fs::read_to_string(&marker)
                && let Ok(pid) = text.trim().parse()
            {
                break pid;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        drop(process);
        let gone_by = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < gone_by {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "dropping the run left a descendant running");
    }
}
