//! Process transport for the native agent backends (`lumen-agent-core`'s
//! [`ChildLauncher`]). Owns the child, its reader threads and a writer thread so
//! a stalled agent can never block the UI; it never interprets the bytes.
//!
//! Output travels over a bounded channel: if the UI falls behind, readers block
//! (the pipe backs up) instead of buffering without limit. The process tree is
//! isolated so a stop reaches the agent's tool processes too (see
//! `process_tree` for the platform boundaries).
use super::process_tree::{
    force_kill_process_tree, isolate_process_tree, resolve_program, terminate_process_tree,
};
use lumen_agent_core::launcher::{
    Child, ChildEvent, ChildLauncher, ChildSpec, ExitInfo, LaunchError, Wake,
};
use std::{
    ffi::OsString,
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
};

/// Queue depth in chunks; with 8 KiB reads this bounds buffered output near 512 KiB.
const QUEUE_CHUNKS: usize = 64;
const READ_CHUNK: usize = 8 * 1024;
/// Pending outgoing messages; more than this means the agent has stopped reading.
const WRITE_QUEUE: usize = 256;

#[derive(Clone, Default)]
pub(crate) struct NativeChildLauncher {
    /// Replaces the inherited `PATH` when set (login-shell directories first), so
    /// a GUI-launched app finds the same `claude`/`codex` a terminal does.
    pub(crate) path: Option<OsString>,
}

impl ChildLauncher for NativeChildLauncher {
    fn launch(&self, spec: &ChildSpec, wake: Wake) -> Result<Box<dyn Child>, LaunchError> {
        let path = self.path.clone().or_else(|| std::env::var_os("PATH"));
        let program = spec.program.to_string_lossy();
        let resolved = resolve_program(&program, path.as_deref())
            .ok_or_else(|| LaunchError::NotFound(format!("'{program}' was not found on PATH")))?;
        let mut command = Command::new(resolved);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in &spec.env_remove {
            command.env_remove(name);
        }
        for (name, value) in &spec.env {
            command.env(name, value);
        }
        if let Some(path) = path {
            command.env("PATH", path);
        }
        isolate_process_tree(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| LaunchError::Failed(format!("{program}: {error}")))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            return Err(LaunchError::Failed(format!("{program}: no pipes")));
        };
        let (sender, events) = mpsc::sync_channel(QUEUE_CHUNKS);
        let (out_sender, out_wake) = (sender.clone(), wake.clone());
        thread::spawn(move || {
            read_stream(
                stdout,
                ChildEvent::Stdout,
                ChildEvent::StdoutClosed,
                out_sender,
                out_wake,
            )
        });
        thread::spawn(move || {
            read_stream(
                stderr,
                ChildEvent::Stderr,
                ChildEvent::StderrClosed,
                sender,
                wake,
            )
        });
        let (writer, queued) = mpsc::sync_channel::<Vec<u8>>(WRITE_QUEUE);
        thread::spawn(move || write_stream(stdin, queued));
        Ok(Box::new(NativeChild {
            child,
            events,
            writer: Some(writer),
            exited: None,
        }))
    }
}

fn read_stream<R: Read>(
    mut source: R,
    data: fn(Vec<u8>) -> ChildEvent,
    closed: ChildEvent,
    sender: SyncSender<ChildEvent>,
    wake: Wake,
) {
    let mut buffer = [0u8; READ_CHUNK];
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                // A full queue blocks here until the UI drains it; a closed one
                // means the child was dropped, so stop reading.
                if sender.send(data(buffer[..read].to_vec())).is_err() {
                    return;
                }
                wake();
            }
        }
    }
    let _ = sender.send(closed);
    wake();
}

/// Dropping the sender closes the channel, which ends the thread and closes stdin.
fn write_stream<W: Write>(mut sink: W, queued: Receiver<Vec<u8>>) {
    while let Ok(bytes) = queued.recv() {
        if sink.write_all(&bytes).and_then(|()| sink.flush()).is_err() {
            return;
        }
    }
}

struct NativeChild {
    child: std::process::Child,
    events: Receiver<ChildEvent>,
    writer: Option<SyncSender<Vec<u8>>>,
    exited: Option<ExitInfo>,
}

impl Child for NativeChild {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        let Some(writer) = &self.writer else {
            return Err("the process input is closed".into());
        };
        match writer.try_send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err("the process is not reading its input".into()),
            Err(TrySendError::Disconnected(_)) => Err("the process has closed its input".into()),
        }
    }

    fn close_stdin(&mut self) {
        self.writer = None;
    }

    fn drain(&mut self, max_bytes: usize) -> Vec<ChildEvent> {
        let mut events = Vec::new();
        let mut bytes = 0;
        while let Ok(event) = self.events.try_recv() {
            if let ChildEvent::Stdout(data) | ChildEvent::Stderr(data) = &event {
                bytes += data.len();
            }
            events.push(event);
            if bytes >= max_bytes {
                break;
            }
        }
        events
    }

    fn try_exit(&mut self) -> Result<Option<ExitInfo>, String> {
        if self.exited.is_some() {
            return Ok(self.exited);
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exited = Some(exit_info(status));
                Ok(self.exited)
            }
            Ok(None) => Ok(None),
            Err(error) => Err(format!("could not inspect the process: {error}")),
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

impl Drop for NativeChild {
    fn drop(&mut self) {
        // Dropping a child never leaves its tree running unobserved.
        if self.exited.is_none() && matches!(self.child.try_wait(), Ok(None)) {
            force_kill_process_tree(self.child.id());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn exit_info(status: std::process::ExitStatus) -> ExitInfo {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    ExitInfo {
        code: status.code(),
        signal,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    fn spec(args: &[&str]) -> ChildSpec {
        ChildSpec {
            program: "/bin/sh".into(),
            args: args.iter().map(OsString::from).collect(),
            env: Vec::new(),
            env_remove: Vec::new(),
            cwd: std::env::temp_dir(),
        }
    }

    fn collect(child: &mut dyn Child) -> (Vec<u8>, ExitInfo) {
        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            for event in child.drain(usize::MAX) {
                if let ChildEvent::Stdout(bytes) = event {
                    out.extend(bytes);
                }
            }
            if let Ok(Some(info)) = child.try_exit() {
                thread::sleep(Duration::from_millis(50));
                for event in child.drain(usize::MAX) {
                    if let ChildEvent::Stdout(bytes) = event {
                        out.extend(bytes);
                    }
                }
                return (out, info);
            }
            assert!(Instant::now() < deadline, "child did not exit");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn output_input_and_exit_codes_flow_through_the_pipes() {
        let launcher = NativeChildLauncher::default();
        let mut child = launcher
            .launch(
                &spec(&["-c", "read line; echo got:$line; exit 3"]),
                Arc::new(|| {}),
            )
            .unwrap();
        child.write(b"hello\n").unwrap();
        let (out, info) = collect(child.as_mut());
        assert_eq!(String::from_utf8_lossy(&out), "got:hello\n");
        assert_eq!(info.code, Some(3));
    }

    #[test]
    fn closing_stdin_lets_a_reader_finish() {
        let launcher = NativeChildLauncher::default();
        let mut child = launcher
            .launch(&spec(&["-c", "cat"]), Arc::new(|| {}))
            .unwrap();
        child.write(b"abc").unwrap();
        child.close_stdin();
        let (out, info) = collect(child.as_mut());
        assert_eq!(out, b"abc");
        assert_eq!(info.code, Some(0));
        assert!(child.write(b"x").is_err());
    }

    #[test]
    fn a_missing_program_is_reported_as_not_found() {
        let launcher = NativeChildLauncher::default();
        let mut missing = spec(&[]);
        missing.program = "definitely-not-a-real-program-xyz".into();
        assert!(matches!(
            launcher.launch(&missing, Arc::new(|| {})),
            Err(LaunchError::NotFound(_))
        ));
    }

    #[test]
    fn stop_reaches_descendants_of_the_process_group() {
        let launcher = NativeChildLauncher::default();
        let mut child = launcher
            .launch(&spec(&["-c", "sleep 30 & wait"]), Arc::new(|| {}))
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(child.request_stop());
        let (_, info) = collect(child.as_mut());
        assert!(info.signal == Some(15) || info.code.is_some());
    }
}

/// Live checks against the real, logged-in CLIs. Ignored by default; run with
/// `cargo test -p lumen-editor live_ -- --ignored --nocapture`. They send one tiny
/// prompt each and need network access and an existing login.
#[cfg(all(test, unix))]
mod live {
    use super::*;
    use lumen_agent_claude::{ClaudeBackend, ClaudeConfig};
    use lumen_agent_codex::{CodexBackend, CodexConfig};
    use lumen_agent_core::{
        ClientRequestId, EventEnvelope, PollBudget, SessionId,
        api::{
            AgentBackend, AgentOperation, ClientTurnId, ConfigPatch, InputBlock, OperationOutcome,
            OperationOutput, Request,
        },
        event::{AgentEvent, TurnOutcome},
        model::SessionModel,
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    fn pump(
        backend: &mut dyn AgentBackend,
        seen: &mut Vec<EventEnvelope>,
        mut done: impl FnMut(&[EventEnvelope]) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while Instant::now() < deadline {
            seen.extend(backend.poll_events(PollBudget::FRAME));
            if done(seen) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out; events: {seen:#?}");
    }

    fn run_turn(backend: &mut dyn AgentBackend, config: ConfigPatch) -> SessionModel {
        let mut seen = Vec::new();
        let workspace = std::env::temp_dir();
        let mut id = 0;
        let mut call = |backend: &mut dyn AgentBackend,
                        seen: &mut Vec<EventEnvelope>,
                        operation| {
            id += 1;
            let op = backend
                .try_submit(Request {
                    client_request_id: ClientRequestId(id),
                    operation,
                })
                .unwrap();
            pump(backend, seen, |e| {
                e.iter().any(|x| {
                    x.operation == Some(op) && matches!(x.event, AgentEvent::OperationFinished(_))
                })
            });
            seen.iter()
                .find_map(|x| match (&x.event, x.operation) {
                    (AgentEvent::OperationFinished(o), Some(n)) if n == op => Some(o.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let probe = call(backend, &mut seen, AgentOperation::Probe);
        println!("probe: {probe:?}");
        assert!(matches!(
            probe,
            OperationOutcome::Succeeded(OperationOutput::Probed { .. })
        ));
        let created = call(
            backend,
            &mut seen,
            AgentOperation::CreateSession {
                session: SessionId(1),
                workspace,
                config,
            },
        );
        assert!(
            matches!(
                created,
                OperationOutcome::Succeeded(OperationOutput::SessionReady { .. })
            ),
            "{created:?}"
        );
        let started = call(
            backend,
            &mut seen,
            AgentOperation::StartTurn {
                session: SessionId(1),
                client_turn: ClientTurnId("live".into()),
                input: vec![InputBlock::Text("Reply with exactly the word: ok".into())],
                expected_revision: None,
            },
        );
        assert!(
            matches!(started, OperationOutcome::Succeeded(_)),
            "{started:?}"
        );
        pump(backend, &mut seen, |e| {
            e.iter()
                .any(|x| matches!(x.event, AgentEvent::TurnFinished(_)))
        });
        let mut model = SessionModel::default();
        for event in seen.iter().filter(|e| e.session == Some(SessionId(1))) {
            model.apply(event);
        }
        let shutdown = backend.begin_shutdown(Duration::from_secs(3)).unwrap();
        pump(backend, &mut seen, |e| {
            e.iter().any(|x| x.operation == Some(shutdown))
        });
        model
    }

    #[test]
    #[ignore = "needs the real claude CLI, a login and network"]
    fn live_claude_answers_one_prompt() {
        let launcher = Arc::new(NativeChildLauncher::default());
        let mut backend =
            ClaudeBackend::new(launcher, Arc::new(|| {}), ClaudeConfig::new("claude"));
        let model = run_turn(&mut backend, ConfigPatch::set("effort", "low"));
        println!("{:#?}", model.config);
        assert_eq!(model.last_outcome, Some(TurnOutcome::Completed));
    }

    #[test]
    #[ignore = "needs the real codex CLI, a login and network"]
    fn live_codex_answers_one_prompt() {
        let launcher = Arc::new(NativeChildLauncher::default());
        let mut backend = CodexBackend::new(launcher, Arc::new(|| {}), CodexConfig::new("codex"));
        let model = run_turn(&mut backend, ConfigPatch::set("effort", "low"));
        println!("{:#?}", model.items);
        assert_eq!(model.last_outcome, Some(TurnOutcome::Completed));
    }
}
