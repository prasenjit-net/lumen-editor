//! Child-process transport for ACP agents.
//!
//! Owns the child, its stdio threads and shutdown. It never interprets messages:
//! the reader thread only splits stdout into JSON lines. App code decides what an
//! event means. Everything is behind `AgentLauncher`/`AgentConnection` so tests
//! and the app layer never spawn a real process.
// Wired up by the agent controller in Phase 2.
#![allow(dead_code)]

use super::language_server::ServerWake;
use lumen_agent_protocol::framing::{self, LineDecoder};
use serde_json::Value;
use std::{
    ffi::{OsStr, OsString},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

/// Longest stderr line forwarded to the log; agents sometimes print huge blobs.
const MAX_STDERR_LINE: usize = 4 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentCommand {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
    /// Added to (or overriding) the inherited environment.
    pub(crate) env: Vec<(OsString, OsString)>,
    pub(crate) cwd: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AgentEvent {
    Message(Value),
    /// A stdout line that was not a JSON message; the stream continues.
    FrameError(String),
    Stderr(String),
    /// Stdout closed: the process ended or closed its output.
    Exited,
}

pub(crate) trait AgentLauncher: Send + Sync {
    fn launch(
        &self,
        command: &AgentCommand,
        wake: ServerWake,
    ) -> Result<Box<dyn AgentConnection>, String>;
}

pub(crate) trait AgentConnection: Send {
    fn send(&mut self, message: &Value) -> Result<(), String>;
    fn drain_events(&self) -> Vec<AgentEvent>;
    /// `Some(description)` once the process has ended.
    fn exit_reason(&mut self) -> Result<Option<String>, String>;
    /// Close stdin, give the agent `grace` to exit, then kill its whole process tree.
    fn stop(&mut self, grace: Duration);
}

#[derive(Default)]
pub(crate) struct LocalAgentLauncher;

impl AgentLauncher for LocalAgentLauncher {
    fn launch(
        &self,
        command: &AgentCommand,
        wake: ServerWake,
    ) -> Result<Box<dyn AgentConnection>, String> {
        AgentProcess::spawn(command, wake).map(|process| Box::new(process) as _)
    }
}

pub(crate) struct AgentProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<AgentEvent>,
    stopped: bool,
}

impl AgentProcess {
    fn spawn(command: &AgentCommand, wake: ServerWake) -> Result<Self, String> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in &command.env {
            process.env(name, value);
        }
        isolate_process_tree(&mut process);
        let mut child = process.spawn().map_err(|error| {
            format!(
                "Could not start agent {}: {error}",
                command.program.display()
            )
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("Agent has no stdout")?;
        let stderr = child.stderr.take().ok_or("Agent has no stderr")?;
        let (sender, events) = mpsc::channel();
        let (stdout_sender, stdout_wake) = (sender.clone(), wake.clone());
        thread::spawn(move || read_stdout(stdout, stdout_sender, stdout_wake));
        thread::spawn(move || read_stderr(stderr, sender, wake));
        Ok(Self {
            child,
            stdin,
            events,
            stopped: false,
        })
    }
}

impl AgentConnection for AgentProcess {
    fn send(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("Agent input is closed")?;
        stdin
            .write_all(&framing::encode(message))
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("Could not send to the agent: {error}"))
    }

    fn drain_events(&self) -> Vec<AgentEvent> {
        self.events.try_iter().collect()
    }

    fn exit_reason(&mut self) -> Result<Option<String>, String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Ok(Some(describe_exit(&status))),
            Ok(None) => Ok(None),
            Err(error) => Err(format!("Could not inspect the agent process: {error}")),
        }
    }

    fn stop(&mut self, grace: Duration) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.stdin = None; // closing stdin is the polite shutdown request
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                // The agent exited, but it may have left helpers behind.
                kill_process_tree(&self.child);
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        kill_process_tree(&self.child);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        self.stop(Duration::from_millis(100));
    }
}

fn describe_exit(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(0) => "Agent exited normally".to_owned(),
        Some(code) => format!("Agent exited with code {code}"),
        None => "Agent was terminated by a signal".to_owned(),
    }
}

fn read_stdout(mut output: impl Read, sender: Sender<AgentEvent>, wake: ServerWake) {
    let mut decoder = LineDecoder::default();
    let mut buffer = [0; 8192];
    loop {
        let count = match output.read(&mut buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        for result in decoder.push(&buffer[..count]) {
            let event = match result {
                Ok(message) => AgentEvent::Message(message),
                Err(error) => AgentEvent::FrameError(error.to_string()),
            };
            if sender.send(event).is_err() {
                return;
            }
        }
        wake();
    }
    let _ = sender.send(AgentEvent::Exited);
    wake();
}

fn read_stderr(mut output: impl Read, sender: Sender<AgentEvent>, wake: ServerWake) {
    let mut pending = Vec::new();
    let mut buffer = [0; 4096];
    let flush = |line: &mut Vec<u8>| {
        if line.is_empty() {
            return true;
        }
        let truncated = line.len() > MAX_STDERR_LINE;
        line.truncate(MAX_STDERR_LINE);
        let mut text = String::from_utf8_lossy(line).trim_end().to_owned();
        if truncated {
            text.push('…');
        }
        line.clear();
        let delivered = text.is_empty() || sender.send(AgentEvent::Stderr(text)).is_ok();
        wake();
        delivered
    };
    loop {
        let count = match output.read(&mut buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                if !flush(&mut pending) {
                    return;
                }
            } else if pending.len() <= MAX_STDERR_LINE {
                pending.push(*byte);
            }
        }
    }
    flush(&mut pending);
}

/// Put the child in its own process group (Unix) so the whole tree can be killed.
fn isolate_process_tree(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
}

/// Kill the child and everything it started (npx -> node -> agent -> tools).
fn kill_process_tree(child: &Child) {
    #[cfg(unix)]
    {
        let group = -(child.id() as libc::pid_t);
        // SAFETY: signalling a process group we created; failure (already gone) is fine.
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        thread::sleep(Duration::from_millis(30));
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

// ------------------------------------------------------------- environment

/// Find an executable the way a shell would. On Windows, bare names also try
/// `PATHEXT`-style suffixes, which is how `npx` becomes `npx.cmd`.
pub(crate) fn resolve_program(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for directory in std::env::split_paths(path_var?) {
        for suffix in suffixes {
            let path = directory.join(format!("{name}{suffix}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Login-shell directories first (they are what the user's terminal sees), then
/// whatever the app inherited, without duplicates.
pub(crate) fn merge_path(login: Option<&OsStr>, inherited: Option<&OsStr>) -> OsString {
    let mut seen = Vec::new();
    for variable in [login, inherited].into_iter().flatten() {
        for directory in std::env::split_paths(variable) {
            if !directory.as_os_str().is_empty() && !seen.contains(&directory) {
                seen.push(directory);
            }
        }
    }
    std::env::join_paths(seen).unwrap_or_default()
}

const PATH_BEGIN: &str = "__LUMEN_PATH_BEGIN__";
const PATH_END: &str = "__LUMEN_PATH_END__";

/// Text between the markers; shells print banners and escape codes around it.
pub(crate) fn extract_marked_path(output: &str) -> Option<String> {
    let start = output.find(PATH_BEGIN)? + PATH_BEGIN.len();
    let end = output[start..].find(PATH_END)? + start;
    let path = output[start..end].trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// `PATH` as the user's login shell sees it. GUI apps started from the Dock or a
/// file manager do not inherit it, so `node`, `npx` or `codex` would look missing.
/// Returns `None` on Windows (GUI apps inherit the real `PATH` there), on timeout,
/// or when the shell prints nothing usable.
pub(crate) fn login_shell_path(shell: &Path, timeout: Duration) -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    let script = format!("printf '%s%s%s' {PATH_BEGIN} \"$PATH\" {PATH_END}");
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = sender.send(text);
    });
    match receiver.recv_timeout(timeout) {
        Ok(text) => {
            let _ = child.wait();
            extract_marked_path(&text).map(OsString::from)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Cached for the life of the process: spawning an interactive shell takes
/// noticeable time and its answer does not change while Lumen runs.
pub(crate) fn cached_login_path() -> Option<&'static OsString> {
    static PATH: std::sync::OnceLock<Option<OsString>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var_os("SHELL").map(PathBuf::from)?;
        login_shell_path(&shell, Duration::from_secs(5))
    })
    .as_ref()
}

// --------------------------------------------------------------- test double

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Scripted agent for app tests: records what was sent, replays queued events.
    #[derive(Clone, Default)]
    pub(crate) struct FakeAgentLauncher {
        pub(crate) launched: Arc<Mutex<Vec<AgentCommand>>>,
        pub(crate) sent: Arc<Mutex<Vec<Value>>>,
        pub(crate) queued: Arc<Mutex<Vec<AgentEvent>>>,
        pub(crate) fail_with: Arc<Mutex<Option<String>>>,
        pub(crate) stops: Arc<Mutex<usize>>,
    }

    struct FakeConnection(FakeAgentLauncher);

    impl AgentLauncher for FakeAgentLauncher {
        fn launch(
            &self,
            command: &AgentCommand,
            _wake: ServerWake,
        ) -> Result<Box<dyn AgentConnection>, String> {
            self.launched.lock().unwrap().push(command.clone());
            if let Some(error) = self.fail_with.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(Box::new(FakeConnection(self.clone())))
        }
    }

    impl AgentConnection for FakeConnection {
        fn send(&mut self, message: &Value) -> Result<(), String> {
            self.0.sent.lock().unwrap().push(message.clone());
            Ok(())
        }

        fn drain_events(&self) -> Vec<AgentEvent> {
            std::mem::take(&mut *self.0.queued.lock().unwrap())
        }

        fn exit_reason(&mut self) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn stop(&mut self, _grace: Duration) {
            *self.0.stops.lock().unwrap() += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn no_wake() -> ServerWake {
        Arc::new(|| {})
    }

    #[test]
    fn marked_paths_survive_shell_noise() {
        let noisy =
            format!("Last login: now\n\u{1b}[0m{PATH_BEGIN}/opt/bin:/usr/bin{PATH_END}\nbye");
        assert_eq!(
            extract_marked_path(&noisy).as_deref(),
            Some("/opt/bin:/usr/bin")
        );
        assert_eq!(extract_marked_path("nothing here"), None);
        assert_eq!(
            extract_marked_path(&format!("{PATH_BEGIN}{PATH_END}")),
            None
        );
        assert_eq!(extract_marked_path(&format!("{PATH_BEGIN}/a")), None);
    }

    #[test]
    fn merged_path_keeps_login_directories_first_without_duplicates() {
        let merged = merge_path(
            Some(OsStr::new(if cfg!(windows) {
                "C:\\n;C:\\x"
            } else {
                "/n:/x"
            })),
            Some(OsStr::new(if cfg!(windows) {
                "C:\\x;C:\\y;"
            } else {
                "/x:/y:"
            })),
        );
        let parts: Vec<_> = std::env::split_paths(&merged).collect();
        let expected: Vec<PathBuf> = if cfg!(windows) {
            vec!["C:\\n".into(), "C:\\x".into(), "C:\\y".into()]
        } else {
            vec!["/n".into(), "/x".into(), "/y".into()]
        };
        assert_eq!(parts, expected);
        assert!(merge_path(None, None).is_empty());
    }

    #[test]
    fn programs_resolve_from_path_and_explicit_paths_must_exist() {
        let directory = tempfile::tempdir().unwrap();
        let tool = directory
            .path()
            .join(if cfg!(windows) { "tool.cmd" } else { "tool" });
        std::fs::write(&tool, "").unwrap();
        let path = std::env::join_paths([directory.path()]).unwrap();
        assert_eq!(resolve_program("tool", Some(&path)), Some(tool.clone()));
        assert_eq!(resolve_program("missing", Some(&path)), None);
        assert_eq!(resolve_program("tool", None), None);
        assert_eq!(resolve_program(tool.to_str().unwrap(), None), Some(tool));
        assert_eq!(
            resolve_program(directory.path().join("nope").to_str().unwrap(), None),
            None
        );
    }

    #[test]
    fn fake_launcher_records_commands_messages_and_stops() {
        let launcher = fake::FakeAgentLauncher::default();
        let command = AgentCommand {
            program: "agent".into(),
            args: vec![],
            env: vec![],
            cwd: "/w".into(),
        };
        let mut connection = launcher.launch(&command, no_wake()).unwrap();
        launcher.queued.lock().unwrap().push(AgentEvent::Exited);
        connection.send(&json!({"a": 1})).unwrap();
        assert_eq!(launcher.sent.lock().unwrap().len(), 1);
        assert_eq!(connection.drain_events(), vec![AgentEvent::Exited]);
        assert!(connection.drain_events().is_empty());
        connection.stop(Duration::ZERO);
        assert_eq!(*launcher.stops.lock().unwrap(), 1);
        *launcher.fail_with.lock().unwrap() = Some("nope".into());
        assert_eq!(
            launcher.launch(&command, no_wake()).err().as_deref(),
            Some("nope")
        );
    }

    #[cfg(unix)]
    mod unix {
        use super::*;

        fn shell_agent(script: &str, cwd: &Path) -> AgentCommand {
            AgentCommand {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: vec![],
                cwd: cwd.to_path_buf(),
            }
        }

        fn wait_for(
            connection: &mut Box<dyn AgentConnection>,
            mut done: impl FnMut(&[AgentEvent]) -> bool,
        ) -> Vec<AgentEvent> {
            let mut all = Vec::new();
            for _ in 0..400 {
                all.extend(connection.drain_events());
                if done(&all) {
                    return all;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out; events: {all:?}");
        }

        #[test]
        fn requests_are_answered_stderr_is_forwarded_and_bad_lines_do_not_stop_the_stream() {
            let directory = tempfile::tempdir().unwrap();
            let script = r#"echo oops >&2; echo 'not json'; read line;
                printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"ok":true}}'; read more"#;
            let mut agent = LocalAgentLauncher
                .launch(&shell_agent(script, directory.path()), no_wake())
                .unwrap();
            agent
                .send(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
                .unwrap();
            let events = wait_for(&mut agent, |events| {
                events.iter().any(|e| matches!(e, AgentEvent::Message(_)))
            });
            assert!(events.contains(&AgentEvent::Stderr("oops".into())));
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, AgentEvent::FrameError(_)))
            );
            assert!(events.contains(&AgentEvent::Message(
                json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}})
            )));
            agent.stop(Duration::from_secs(2));
            assert!(agent.exit_reason().unwrap().is_some());
        }

        #[test]
        fn closing_stdin_lets_a_well_behaved_agent_exit_on_its_own() {
            let directory = tempfile::tempdir().unwrap();
            let mut agent = LocalAgentLauncher
                .launch(&shell_agent("cat >/dev/null", directory.path()), no_wake())
                .unwrap();
            let started = Instant::now();
            agent.stop(Duration::from_secs(5));
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "no kill was needed"
            );
            assert_eq!(
                agent.exit_reason().unwrap().as_deref(),
                Some("Agent exited normally")
            );
            assert!(
                agent.send(&json!({})).is_err(),
                "input is closed after stop"
            );
        }

        #[test]
        fn stopping_kills_helpers_the_agent_left_running() {
            let directory = tempfile::tempdir().unwrap();
            let pid_file = directory.path().join("helper.pid");
            let script = format!("sleep 300 & echo $! > '{}'; wait", pid_file.display());
            let mut agent = LocalAgentLauncher
                .launch(&shell_agent(&script, directory.path()), no_wake())
                .unwrap();
            let helper: i32 = (0..400)
                .find_map(|_| {
                    thread::sleep(Duration::from_millis(10));
                    std::fs::read_to_string(&pid_file).ok()?.trim().parse().ok()
                })
                .expect("helper started");
            assert_eq!(unsafe { libc::kill(helper, 0) }, 0, "helper is alive");
            agent.stop(Duration::from_millis(50));
            let gone = (0..200).any(|_| {
                thread::sleep(Duration::from_millis(10));
                unsafe { libc::kill(helper, 0) != 0 }
            });
            assert!(gone, "helper process {helper} survived stop()");
        }

        #[test]
        fn eof_on_stdout_is_reported_as_exited_and_missing_programs_fail_cleanly() {
            let directory = tempfile::tempdir().unwrap();
            let mut agent = LocalAgentLauncher
                .launch(&shell_agent("exit 3", directory.path()), no_wake())
                .unwrap();
            wait_for(&mut agent, |events| events.contains(&AgentEvent::Exited));
            let reason = (0..200)
                .find_map(|_| {
                    thread::sleep(Duration::from_millis(10));
                    agent.exit_reason().ok().flatten()
                })
                .expect("exit status");
            assert_eq!(reason, "Agent exited with code 3");
            let missing = AgentCommand {
                program: "/definitely/not/here".into(),
                args: vec![],
                env: vec![],
                cwd: directory.path().to_path_buf(),
            };
            let error = LocalAgentLauncher
                .launch(&missing, no_wake())
                .err()
                .unwrap();
            assert!(error.contains("Could not start agent"), "{error}");
        }

        #[test]
        fn environment_overrides_reach_the_agent() {
            let directory = tempfile::tempdir().unwrap();
            let mut command = shell_agent(
                r#"printf '{"jsonrpc":"2.0","method":"env","params":{"v":"%s"}}\n' "$LUMEN_TEST_VALUE"; read x"#,
                directory.path(),
            );
            command
                .env
                .push(("LUMEN_TEST_VALUE".into(), "from-lumen".into()));
            let mut agent = LocalAgentLauncher.launch(&command, no_wake()).unwrap();
            let events = wait_for(&mut agent, |events| {
                events.iter().any(|e| matches!(e, AgentEvent::Message(_)))
            });
            assert!(events.contains(&AgentEvent::Message(
                json!({"jsonrpc":"2.0","method":"env","params":{"v":"from-lumen"}})
            )));
            agent.stop(Duration::from_millis(100));
        }

        #[test]
        fn login_shell_path_is_read_and_slow_shells_time_out() {
            let path = login_shell_path(Path::new("/bin/sh"), Duration::from_secs(10))
                .expect("sh prints PATH");
            assert!(std::env::split_paths(&path).any(|dir| dir == Path::new("/bin")));

            let directory = tempfile::tempdir().unwrap();
            let slow = directory.path().join("slow-shell");
            std::fs::write(&slow, "#!/bin/sh\nsleep 30\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
            let started = Instant::now();
            assert_eq!(login_shell_path(&slow, Duration::from_millis(300)), None);
            assert!(started.elapsed() < Duration::from_secs(5));
            assert_eq!(
                login_shell_path(Path::new("/definitely/not/a/shell"), Duration::from_secs(1)),
                None
            );
        }
    }

    /// Live check against a real adapter. Set the command line in
    /// `LUMEN_ACP_CLAUDE` / `LUMEN_ACP_CODEX`, for example
    /// `LUMEN_ACP_CLAUDE="node /path/to/claude-agent-acp/dist/index.js"`, then
    /// `cargo test live_ -- --ignored --nocapture`. Needs a logged-in agent and
    /// sends one tiny prompt (no tools).
    fn live_turn(variable: &str) {
        use lumen_agent_protocol::{
            handshake::check_version,
            reducer::Conversation,
            requests,
            rpc::{Incoming, RequestId, RequestTracker, parse_incoming},
            types::{
                StopReason, parse_initialize, parse_new_session, parse_stop_reason, parse_update,
            },
        };
        let Some(line) = std::env::var_os(variable) else {
            panic!("set {variable} to the adapter command line");
        };
        let line = line.to_string_lossy().into_owned();
        let mut words = line.split_whitespace();
        let program = PathBuf::from(words.next().expect("a program"));
        let workspace = tempfile::tempdir().unwrap();
        let command = AgentCommand {
            program,
            args: words.map(OsString::from).collect(),
            env: vec![],
            cwd: workspace.path().to_path_buf(),
        };
        let mut agent = LocalAgentLauncher.launch(&command, no_wake()).unwrap();
        let mut tracker = RequestTracker::default();
        let mut conversation = Conversation::default();
        let (mut session, mut stop) = (None::<String>, None);
        let cwd = workspace
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string();

        let id = tracker.register("initialize", Instant::now(), None);
        agent
            .send(&requests::initialize(id, "test", true, false))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while stop.is_none() && Instant::now() < deadline {
            for event in agent.drain_events() {
                let AgentEvent::Message(message) = event else {
                    continue;
                };
                match parse_incoming(&message).unwrap() {
                    Incoming::Response { id, outcome } => {
                        let method = tracker.complete(&id).unwrap_or_default();
                        let result = outcome.unwrap_or_else(|error| panic!("{method}: {error:?}"));
                        match method.as_str() {
                            "initialize" => {
                                let init = parse_initialize(&result).unwrap();
                                assert_eq!(check_version(&init), Ok(()));
                                let id = tracker.register("session/new", Instant::now(), None);
                                agent
                                    .send(&requests::session_new(id, &cwd, vec![]))
                                    .unwrap();
                            }
                            "session/new" => {
                                let new = parse_new_session(&result).unwrap();
                                let id = tracker.register("session/prompt", Instant::now(), None);
                                agent
                                    .send(&requests::session_prompt(
                                        id,
                                        &new.session_id,
                                        vec![requests::text_block(
                                            "Reply with only the word pong. Use no tools.",
                                        )],
                                    ))
                                    .unwrap();
                                session = Some(new.session_id);
                            }
                            _ => stop = Some(parse_stop_reason(&result)),
                        }
                    }
                    Incoming::Notification { method, params } if method == "session/update" => {
                        conversation.apply(parse_update(&params["update"]).unwrap());
                    }
                    Incoming::Request {
                        id: RequestId::Number(_) | RequestId::Text(_),
                        method,
                        ..
                    } => {
                        panic!("unexpected agent request {method}");
                    }
                    _ => {}
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        agent.stop(Duration::from_secs(2));
        assert!(session.is_some(), "session was created");
        assert_eq!(stop, Some(StopReason::EndTurn));
        assert!(
            conversation.agent_text().to_lowercase().contains("pong"),
            "{:?}",
            conversation.agent_text()
        );
    }

    #[test]
    #[ignore = "needs a logged-in Claude adapter; see live_turn"]
    fn live_claude_adapter_completes_a_turn() {
        live_turn("LUMEN_ACP_CLAUDE");
    }

    #[test]
    #[ignore = "needs a logged-in Codex adapter; see live_turn"]
    fn live_codex_adapter_completes_a_turn() {
        live_turn("LUMEN_ACP_CODEX");
    }
}
