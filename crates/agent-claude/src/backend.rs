//! `ClaudeBackend`: the common interface over the native `claude` CLI.
//!
//! One `claude -p --input-format stream-json ...` child per *running* session,
//! started lazily on the first turn (a session itself is just a native session id
//! until then) and relaunched with `--resume` after it is detached or dies.
//! Permissions arrive as `can_use_tool` control requests on the same stream
//! (`--permission-prompt-tool stdio`), so no MCP endpoint or helper process exists.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsString,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use lumen_agent_core::{
    AgentError, Capabilities, ClientRequestId, EventEnvelope, ItemId, NativeSessionRef,
    OperationId, PollBudget, ProfileId, ProviderDescriptor, ProviderId, RequestToken,
    RuntimeGeneration, SessionId, SubmitError, Support, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, AuthState, ConfigPatch, ConfigRevision,
        InputBlock, OperationOutcome, OperationOutput, QuestionAnswer, Request, SessionConfig,
    },
    event::{AgentEvent, TurnOutcome},
    launcher::{Child, ChildEvent, ChildLauncher, ChildSpec, Wake},
    lines::{Line, LineBuffer},
    model::{MessagePhase, ToolStatus, ToolUpdate},
};
use serde_json::Value;

use crate::{
    convert,
    wire::{self, Message},
};

const QUEUE_LIMIT: usize = 64;
const DRAIN_BYTES: usize = 256 * 1024;
/// How long a one-shot CLI command (`--version`, `auth status`) may take.
const ONE_SHOT_TIMEOUT: Duration = Duration::from_secs(20);
/// Grace for a retired runtime before it is killed.
const RETIRE_GRACE: Duration = Duration::from_secs(3);
const DENY_MESSAGE: &str = "The user denied this action in Lumen.";
const NO_QUESTIONS: &str = "Lumen cannot show questions to the user yet. Continue without asking, stating any assumption you make.";

#[derive(Clone)]
pub struct ClaudeConfig {
    /// The `claude` executable.
    pub executable: PathBuf,
    pub profile: ProfileId,
    /// Mints a native session id (a UUID). Injected so tests are deterministic.
    pub new_session_id: Arc<dyn Fn() -> String + Send + Sync>,
}

impl ClaudeConfig {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            profile: ProfileId::default_profile(),
            new_session_id: Arc::new(random_uuid),
        }
    }
}

/// A random version-4-shaped UUID from the standard library's randomized hasher
/// (uniqueness, not secrecy: a session id is not a credential).
fn random_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut bytes = [0u8; 16];
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_usize(i);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex[0..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..16].concat()
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OneShotKind {
    /// `claude --version`; a probe continues with the auth status.
    Version,
    /// `claude auth status`; `probe_version` is set when it finishes a probe.
    AuthStatus { probe_version: Option<String> },
}

struct OneShot {
    op: OperationId,
    child: Box<dyn Child>,
    out: Vec<u8>,
    kind: OneShotKind,
    deadline: Instant,
    exited: bool,
}

struct Retiring {
    child: Box<dyn Child>,
    deadline: Instant,
}

/// A permission request waiting for the user.
struct PendingRequest {
    request_id: String,
    input: Value,
    tool_use_id: Option<String>,
    token: RequestToken,
}

struct Active {
    turn: TurnId,
    interrupt_requested: bool,
    /// Tool calls the user denied, so their result is shown as declined.
    denied: HashSet<String>,
}

#[derive(Default)]
struct StreamState {
    message_id: String,
    blocks: HashMap<usize, Block>,
    /// Messages whose text arrived as partial events (their final `assistant`
    /// message must not repeat it).
    streamed: HashSet<String>,
    tools: HashSet<String>,
}

enum Block {
    Text(ItemId),
    Tool,
    Ignored,
}

struct Runtime {
    child: Box<dyn Child>,
    lines: LineBuffer,
    generation: u64,
    active: Option<Active>,
    requests: HashMap<String, PendingRequest>,
    controls: HashMap<String, OperationId>,
    stream: StreamState,
    /// The settings this process was launched with.
    launched_model: String,
    launched_mode: String,
    stderr_tail: String,
}

struct Session {
    native: String,
    workspace: PathBuf,
    config: SessionConfig,
    /// A user message has been sent, so a new process must `--resume` rather than
    /// create the session.
    history: bool,
    runtime: Option<Runtime>,
    turns: u64,
}

struct Shutdown {
    op: OperationId,
    deadline: Instant,
}

pub struct ClaudeBackend {
    launcher: Arc<dyn ChildLauncher>,
    wake: Wake,
    config: ClaudeConfig,
    descriptor: ProviderDescriptor,
    capabilities: Capabilities,
    next_generation: u64,
    sequences: HashMap<u64, u64>,
    queue: VecDeque<(OperationId, AgentOperation)>,
    out: VecDeque<EventEnvelope>,
    sessions: HashMap<SessionId, Session>,
    one_shots: Vec<OneShot>,
    retiring: Vec<Retiring>,
    seen: HashSet<ClientRequestId>,
    next_operation: u64,
    next_control: u64,
    shutdown: Option<Shutdown>,
    closed: bool,
    /// Lines that were not valid stream-json (counted, never fatal).
    pub ignored_frames: u64,
}

fn capabilities() -> Capabilities {
    let mut caps = Capabilities::none();
    let history = || {
        Support::unsupported(
            "reading Claude Code's session files is not implemented yet; Lumen keeps its own transcript cache",
        )
    };
    caps.streaming = Support::Supported;
    caps.list_history = history();
    caps.read_history = history();
    caps.resume = Support::Supported;
    caps.rename =
        Support::unsupported("Claude Code has no rename call; Lumen keeps the title locally");
    caps.archive =
        Support::unsupported("Claude Code has no archive call; Lumen hides the session locally");
    caps.attachments_text = Support::Supported;
    caps.attachments_image = Support::unsupported("not implemented for Claude Code yet");
    caps.approvals = Support::Supported;
    caps.questions = Support::unsupported("questions from Claude Code are declined for now");
    caps.config_mutation = Support::Supported;
    caps.command_discovery = Support::unsupported("not implemented for Claude Code yet");
    caps.usage = Support::Supported;
    caps.enforced_read_only = Support::unsupported("not verified for Claude Code");
    caps.interrupt = Support::Supported;
    caps.native_multi_session = Support::unsupported("one process per session");
    caps
}

impl ClaudeBackend {
    pub fn new(launcher: Arc<dyn ChildLauncher>, wake: Wake, config: ClaudeConfig) -> Self {
        let mut descriptor = ProviderDescriptor::new(ProviderId::Claude);
        descriptor.profile = config.profile.clone();
        Self {
            launcher,
            wake,
            config,
            descriptor,
            capabilities: capabilities(),
            next_generation: 0,
            sequences: HashMap::new(),
            queue: VecDeque::new(),
            out: VecDeque::new(),
            sessions: HashMap::new(),
            one_shots: Vec::new(),
            retiring: Vec::new(),
            seen: HashSet::new(),
            next_operation: 1,
            next_control: 1,
            shutdown: None,
            closed: false,
            ignored_frames: 0,
        }
    }

    // ------------------------------------------------------------ plumbing

    fn emit_in(
        &mut self,
        generation: u64,
        op: Option<OperationId>,
        session: Option<SessionId>,
        turn: Option<TurnId>,
        event: AgentEvent,
    ) {
        let sequence = self.sequences.entry(generation).or_insert(0);
        *sequence += 1;
        let sequence = *sequence;
        self.out.push_back(EventEnvelope {
            provider: ProviderId::Claude,
            profile: self.config.profile.clone(),
            generation: RuntimeGeneration(generation),
            operation: op,
            session,
            turn,
            sequence,
            event,
        });
    }

    /// An event of a session: stamped with its runtime's generation (0 before any
    /// process has been launched for it).
    fn emit(
        &mut self,
        op: Option<OperationId>,
        session: SessionId,
        turn: Option<TurnId>,
        event: AgentEvent,
    ) {
        let generation = self
            .sessions
            .get(&session)
            .and_then(|s| s.runtime.as_ref().map(|r| r.generation))
            .unwrap_or(0);
        self.emit_in(generation, op, Some(session), turn, event);
    }

    fn finish(&mut self, op: OperationId, outcome: OperationOutcome) {
        self.emit_in(
            0,
            Some(op),
            None,
            None,
            AgentEvent::OperationFinished(outcome),
        );
    }

    fn fail(&mut self, op: OperationId, error: AgentError) {
        self.finish(op, OperationOutcome::Failed(error));
    }

    fn unsupported(&mut self, op: OperationId, reason: &str) {
        self.fail(
            op,
            AgentError::Unsupported {
                reason: reason.into(),
            },
        );
    }

    fn token(
        &self,
        session: SessionId,
        turn: &TurnId,
        generation: u64,
        native: &str,
    ) -> RequestToken {
        RequestToken {
            provider: ProviderId::Claude,
            profile: self.config.profile.clone(),
            session,
            turn: turn.clone(),
            generation: RuntimeGeneration(generation),
            native: native.to_string(),
        }
    }

    fn spawn(&self, args: Vec<OsString>, cwd: PathBuf) -> Result<Box<dyn Child>, AgentError> {
        let spec = ChildSpec {
            program: self.config.executable.clone(),
            args,
            env: Vec::new(),
            env_remove: Vec::new(),
            cwd,
        };
        self.launcher
            .launch(&spec, self.wake.clone())
            .map_err(|error| match error {
                lumen_agent_core::launcher::LaunchError::NotFound(detail) => {
                    AgentError::ProviderMissing { detail }
                }
                lumen_agent_core::launcher::LaunchError::Failed(detail) => {
                    AgentError::Transport { detail }
                }
            })
    }

    fn start_one_shot(&mut self, op: OperationId, args: &[&str], kind: OneShotKind) {
        match self.spawn(
            args.iter().map(OsString::from).collect(),
            std::env::temp_dir(),
        ) {
            Ok(mut child) => {
                child.close_stdin();
                self.one_shots.push(OneShot {
                    op,
                    child,
                    out: Vec::new(),
                    kind,
                    deadline: Instant::now() + ONE_SHOT_TIMEOUT,
                    exited: false,
                });
            }
            Err(error) => self.fail(op, error),
        }
    }

    // ---------------------------------------------------------- operations

    fn dispatch(&mut self, op: OperationId, operation: AgentOperation) {
        use AgentOperation as O;
        match operation {
            O::Probe => self.start_one_shot(op, &["--version"], OneShotKind::Version),
            O::ReadAuthState => self.start_one_shot(
                op,
                &["auth", "status"],
                OneShotKind::AuthStatus {
                    probe_version: None,
                },
            ),
            O::BeginAuthentication { .. } | O::CancelAuthentication { .. } => self.unsupported(
                op,
                "sign in with `claude auth login` in a terminal, then retry",
            ),
            O::ListSessions { .. } | O::ReadHistory { .. } => self.unsupported(
                op,
                "reading Claude Code's session files is not implemented yet",
            ),
            O::UpdateSessionMetadata { .. } => self.unsupported(
                op,
                "Claude Code has no rename call; Lumen keeps the title locally",
            ),
            O::ArchiveSession { .. } => self.unsupported(
                op,
                "Claude Code has no archive call; Lumen hides the session locally",
            ),
            O::ListCommands { .. } => self.unsupported(
                op,
                "command discovery is not implemented for Claude Code yet",
            ),
            O::CreateSession {
                session,
                workspace,
                config,
            } => self.create_session(op, session, workspace, config),
            O::AttachSession {
                session,
                native,
                workspace,
            } => self.attach_session(op, session, native, workspace),
            O::ReadConfig { session } => match self.sessions.get(&session) {
                Some(record) => {
                    let out = OperationOutput::Config(record.config.clone());
                    self.finish(op, OperationOutcome::Succeeded(out));
                }
                None => self.fail(
                    op,
                    AgentError::NotFound {
                        what: "that session".into(),
                    },
                ),
            },
            O::SetConfig {
                session,
                expected_revision,
                patch,
            } => self.set_config(op, session, expected_revision, patch),
            O::StartTurn {
                session,
                input,
                expected_revision,
                ..
            } => self.start_turn(op, session, input, expected_revision),
            O::InterruptTurn { session, turn } => self.interrupt(op, session, turn),
            O::ReplyApproval { token, decision } => self.reply_approval(op, token, decision),
            O::ReplyQuestion { token, answers } => self.reply_question(op, token, answers),
            O::DetachSession { session } => self.detach(op, session),
        }
    }

    fn model_and_mode(patch: &ConfigPatch, config: &mut SessionConfig) -> Result<(), AgentError> {
        for (id, value) in &patch.values {
            let valid = config
                .get(id)
                .is_some_and(|o| o.choices.iter().any(|c| c.value == *value));
            if !valid {
                return Err(AgentError::InvalidConfig {
                    detail: format!("{id} cannot be {value}"),
                });
            }
        }
        for (id, value) in &patch.values {
            if let Some(option) = config.options.iter_mut().find(|o| o.id == *id) {
                option.current = value.clone();
            }
        }
        Ok(())
    }

    fn register(
        &mut self,
        session: SessionId,
        native: String,
        workspace: PathBuf,
        config: SessionConfig,
        history: bool,
    ) {
        self.sessions.insert(
            session,
            Session {
                native,
                workspace,
                config,
                history,
                runtime: None,
                turns: 0,
            },
        );
    }

    fn create_session(
        &mut self,
        op: OperationId,
        session: SessionId,
        workspace: PathBuf,
        patch: ConfigPatch,
    ) {
        if self.sessions.contains_key(&session) {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that session id".into(),
                },
            );
        }
        let mut config = convert::session_config(1, "default", "default");
        if let Err(error) = Self::model_and_mode(&patch, &mut config) {
            return self.fail(op, error);
        }
        let native = (self.config.new_session_id)();
        self.register(session, native.clone(), workspace, config.clone(), false);
        self.emit(Some(op), session, None, AgentEvent::SessionCreated);
        self.emit(
            Some(op),
            session,
            None,
            AgentEvent::ConfigChanged(config.clone()),
        );
        let out = OperationOutput::SessionReady {
            native: NativeSessionRef {
                provider: ProviderId::Claude,
                profile: self.config.profile.clone(),
                id: native,
            },
            config,
            capabilities: Box::new(self.capabilities.clone()),
        };
        self.finish(op, OperationOutcome::Succeeded(out));
    }

    fn attach_session(
        &mut self,
        op: OperationId,
        session: SessionId,
        native: NativeSessionRef,
        workspace: PathBuf,
    ) {
        if native.provider != ProviderId::Claude || native.profile != self.config.profile {
            return self.fail(
                op,
                AgentError::NotFound {
                    what: "that native session".into(),
                },
            );
        }
        let config = self
            .sessions
            .get(&session)
            .map(|s| s.config.clone())
            .unwrap_or_else(|| convert::session_config(1, "default", "default"));
        self.register(session, native.id.clone(), workspace, config.clone(), true);
        self.emit(Some(op), session, None, AgentEvent::SessionAttached);
        let out = OperationOutput::SessionReady {
            native,
            config,
            capabilities: Box::new(self.capabilities.clone()),
        };
        self.finish(op, OperationOutcome::Succeeded(out));
    }

    fn set_config(
        &mut self,
        op: OperationId,
        session: SessionId,
        expected: ConfigRevision,
        patch: ConfigPatch,
    ) {
        let Some(record) = self.sessions.get_mut(&session) else {
            return self.fail(
                op,
                AgentError::NotFound {
                    what: "that session".into(),
                },
            );
        };
        if record.config.revision != expected {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "the configuration revision".into(),
                },
            );
        }
        let mut next = record.config.clone();
        if let Err(error) = Self::model_and_mode(&patch, &mut next) {
            return self.fail(op, error);
        }
        next.revision = ConfigRevision(next.revision.0 + 1);
        record.config = next.clone();
        self.emit(
            Some(op),
            session,
            None,
            AgentEvent::ConfigChanged(next.clone()),
        );
        self.finish(
            op,
            OperationOutcome::Succeeded(OperationOutput::Config(next)),
        );
    }

    fn launch_args(record: &Session) -> Vec<OsString> {
        let current = |id: &str| {
            record
                .config
                .get(id)
                .map(|o| o.current.clone())
                .unwrap_or_default()
        };
        let mut args: Vec<OsString> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        args.push(current("permission_mode").into());
        let model = current("model");
        if !model.is_empty() && model != "default" {
            args.push("--model".into());
            args.push(model.into());
        }
        args.push(
            if record.history {
                "--resume"
            } else {
                "--session-id"
            }
            .into(),
        );
        args.push(record.native.clone().into());
        args
    }

    /// Make sure the session has a process matching its current settings.
    fn ensure_runtime(&mut self, session: SessionId) -> Result<(), AgentError> {
        let (model, mode, stale) = {
            let record = &self.sessions[&session];
            let current = |id: &str| {
                record
                    .config
                    .get(id)
                    .map(|o| o.current.clone())
                    .unwrap_or_default()
            };
            let (model, mode) = (current("model"), current("permission_mode"));
            let stale = record.runtime.as_ref().is_some_and(|r| {
                r.active.is_none() && (r.launched_model != model || r.launched_mode != mode)
            });
            (model, mode, stale)
        };
        if stale {
            // Settings changed while idle: restart the process so they apply.
            self.retire(session);
        }
        if self.sessions[&session].runtime.is_some() {
            return Ok(());
        }
        let (args, cwd) = {
            let record = &self.sessions[&session];
            (Self::launch_args(record), record.workspace.clone())
        };
        let child = self.spawn(args, cwd)?;
        self.next_generation += 1;
        let generation = self.next_generation;
        if let Some(record) = self.sessions.get_mut(&session) {
            record.runtime = Some(Runtime {
                child,
                lines: LineBuffer::default(),
                generation,
                active: None,
                requests: HashMap::new(),
                controls: HashMap::new(),
                stream: StreamState::default(),
                launched_model: model,
                launched_mode: mode,
                stderr_tail: String::new(),
            });
        }
        Ok(())
    }

    /// Stop a session's process in the background (idle detach or config change).
    fn retire(&mut self, session: SessionId) {
        let Some(mut runtime) = self
            .sessions
            .get_mut(&session)
            .and_then(|s| s.runtime.take())
        else {
            return;
        };
        runtime.child.close_stdin();
        runtime.child.request_stop();
        self.retiring.push(Retiring {
            child: runtime.child,
            deadline: Instant::now() + RETIRE_GRACE,
        });
    }

    fn start_turn(
        &mut self,
        op: OperationId,
        session: SessionId,
        input: Vec<InputBlock>,
        expected: Option<ConfigRevision>,
    ) {
        let Some(record) = self.sessions.get(&session) else {
            return self.fail(
                op,
                AgentError::NotFound {
                    what: "that session".into(),
                },
            );
        };
        if record.runtime.as_ref().is_some_and(|r| r.active.is_some()) {
            return self.fail(op, AgentError::Busy);
        }
        if expected.is_some_and(|revision| revision != record.config.revision) {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "the configuration revision".into(),
                },
            );
        }
        let content = match convert::input_blocks(&input) {
            Ok(content) => content,
            Err(error) => return self.fail(op, error),
        };
        if let Err(error) = self.ensure_runtime(session) {
            return self.fail(op, error);
        }
        let turn = {
            let record = self.sessions.get_mut(&session).expect("checked above");
            record.turns += 1;
            TurnId(format!("turn-{}", record.turns))
        };
        let written = {
            let record = self.sessions.get_mut(&session).expect("checked above");
            let runtime = record.runtime.as_mut().expect("ensured");
            runtime
                .child
                .write(wire::user_message(content).as_bytes())
                .is_ok()
        };
        if !written {
            // The process died between turns; the next attempt relaunches it.
            if let Some(record) = self.sessions.get_mut(&session) {
                record.runtime = None;
            }
            return self.fail(
                op,
                AgentError::Transport {
                    detail: "the claude process is not running; try again".into(),
                },
            );
        }
        if let Some(record) = self.sessions.get_mut(&session) {
            record.history = true;
            if let Some(runtime) = record.runtime.as_mut() {
                runtime.active = Some(Active {
                    turn: turn.clone(),
                    interrupt_requested: false,
                    denied: HashSet::new(),
                });
            }
        }
        self.emit(None, session, Some(turn.clone()), AgentEvent::TurnStarted);
        self.finish(
            op,
            OperationOutcome::Succeeded(OperationOutput::TurnStarted(turn)),
        );
    }

    fn interrupt(&mut self, op: OperationId, session: SessionId, turn: TurnId) {
        let control = format!("lumen-{}", self.next_control);
        self.next_control += 1;
        let Some(runtime) = self
            .sessions
            .get_mut(&session)
            .and_then(|s| s.runtime.as_mut())
        else {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that turn".into(),
                },
            );
        };
        match &mut runtime.active {
            Some(active) if active.turn == turn => {
                active.interrupt_requested = true;
                if runtime
                    .child
                    .write(wire::interrupt_request(&control).as_bytes())
                    .is_err()
                {
                    return self.fail(
                        op,
                        AgentError::Transport {
                            detail: "the claude process is not running".into(),
                        },
                    );
                }
                runtime.controls.insert(control, op);
            }
            _ => self.fail(
                op,
                AgentError::Stale {
                    what: "that turn".into(),
                },
            ),
        }
    }

    fn take_request(&mut self, token: &RequestToken) -> Option<PendingRequest> {
        let runtime = self.sessions.get_mut(&token.session)?.runtime.as_mut()?;
        let matches = runtime.generation == token.generation.0
            && runtime
                .requests
                .get(&token.native)
                .is_some_and(|r| r.token == *token);
        if matches {
            runtime.requests.remove(&token.native)
        } else {
            None
        }
    }

    fn reply_approval(
        &mut self,
        op: OperationId,
        token: RequestToken,
        decision: ApprovalDecisionId,
    ) {
        let Some(request) = self.take_request(&token) else {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that approval request".into(),
                },
            );
        };
        let (line, denied) = match decision.0.as_str() {
            convert::ALLOW => (wire::allow(&request.request_id, &request.input), false),
            convert::DENY => (wire::deny(&request.request_id, DENY_MESSAGE), true),
            other => {
                let detail = format!("unknown decision {other}");
                if let Some(runtime) = self
                    .sessions
                    .get_mut(&token.session)
                    .and_then(|s| s.runtime.as_mut())
                {
                    runtime.requests.insert(token.native.clone(), request);
                }
                return self.fail(op, AgentError::InvalidConfig { detail });
            }
        };
        let session = token.session;
        let delivered = {
            let runtime = self
                .sessions
                .get_mut(&session)
                .and_then(|s| s.runtime.as_mut());
            match runtime {
                Some(runtime) => {
                    if denied
                        && let (Some(active), Some(id)) =
                            (runtime.active.as_mut(), request.tool_use_id.clone())
                    {
                        active.denied.insert(id);
                    }
                    runtime.child.write(line.as_bytes()).is_ok()
                }
                None => false,
            }
        };
        if !delivered {
            return self.fail(
                op,
                AgentError::Ambiguous {
                    detail: "the reply could not be delivered".into(),
                },
            );
        }
        self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
        self.emit(
            None,
            session,
            Some(token.turn.clone()),
            AgentEvent::ApprovalResolved { token },
        );
    }

    fn reply_question(
        &mut self,
        op: OperationId,
        _token: RequestToken,
        _answers: Vec<QuestionAnswer>,
    ) {
        // Questions are declined at the source, so there is never one to answer.
        self.fail(
            op,
            AgentError::Stale {
                what: "that question".into(),
            },
        );
    }

    fn detach(&mut self, op: OperationId, session: SessionId) {
        match self.sessions.get(&session) {
            None => self.fail(
                op,
                AgentError::NotFound {
                    what: "that session".into(),
                },
            ),
            Some(record) if record.runtime.as_ref().is_some_and(|r| r.active.is_some()) => {
                self.fail(op, AgentError::Busy)
            }
            Some(_) => {
                self.retire(session);
                self.emit(Some(op), session, None, AgentEvent::SessionDetached);
                self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
            }
        }
    }
}

// ------------------------------------------------------------ the stream

impl ClaudeBackend {
    fn runtime(&mut self, session: SessionId) -> Option<&mut Runtime> {
        self.sessions
            .get_mut(&session)
            .and_then(|s| s.runtime.as_mut())
    }

    fn active_turn(&mut self, session: SessionId) -> Option<TurnId> {
        self.runtime(session)
            .and_then(|r| r.active.as_ref().map(|a| a.turn.clone()))
    }

    fn pump_sessions(&mut self) {
        let ids: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.runtime.is_some())
            .map(|(id, _)| *id)
            .collect();
        for session in ids {
            self.pump_session(session);
        }
    }

    fn pump_session(&mut self, session: SessionId) {
        let mut lines = Vec::new();
        let mut exit = None;
        if let Some(runtime) = self.runtime(session) {
            for event in runtime.child.drain(DRAIN_BYTES) {
                match event {
                    ChildEvent::Stdout(bytes) => lines.extend(runtime.lines.push(&bytes)),
                    ChildEvent::Stderr(bytes) => {
                        runtime
                            .stderr_tail
                            .push_str(&String::from_utf8_lossy(&bytes));
                        let excess = runtime.stderr_tail.len().saturating_sub(2000);
                        if excess > 0 {
                            let cut = (excess..runtime.stderr_tail.len())
                                .find(|i| runtime.stderr_tail.is_char_boundary(*i))
                                .unwrap_or(runtime.stderr_tail.len());
                            runtime.stderr_tail.drain(..cut);
                        }
                    }
                    ChildEvent::StdoutClosed | ChildEvent::StderrClosed => {}
                }
            }
            if let Ok(Some(info)) = runtime.child.try_exit() {
                exit = Some(info);
            }
        }
        for line in lines {
            match line {
                Line::Text(text) => match wire::parse(&text) {
                    Ok(message) => self.handle_message(session, message),
                    Err(_) => self.ignored_frames += 1,
                },
                Line::TooLong | Line::InvalidUtf8 => self.ignored_frames += 1,
            }
        }
        if let Some(info) = exit {
            // Output that arrived with the exit has been handled above; anything
            // still in the pipe is read by one more pass.
            let more = self
                .runtime(session)
                .map(|r| !r.child.drain(DRAIN_BYTES).is_empty())
                .unwrap_or(false);
            if !more {
                self.runtime_exited(session, info);
            }
        }
    }

    fn runtime_exited(&mut self, session: SessionId, info: lumen_agent_core::launcher::ExitInfo) {
        let Some(runtime) = self.runtime(session) else {
            return;
        };
        let tail = runtime.stderr_tail.trim().to_string();
        let turn = runtime.active.take().map(|a| a.turn);
        let requests: Vec<PendingRequest> = runtime.requests.drain().map(|(_, r)| r).collect();
        let controls: Vec<OperationId> = runtime.controls.drain().map(|(_, op)| op).collect();
        let how = match (info.code, info.signal) {
            (Some(code), _) => format!("exited with code {code}"),
            (None, Some(signal)) => format!("was stopped by signal {signal}"),
            _ => "exited".to_string(),
        };
        let reason = if tail.is_empty() {
            format!("claude {how}")
        } else {
            format!("claude {how}: {tail}")
        };
        for request in requests {
            self.emit(
                None,
                session,
                turn.clone(),
                AgentEvent::ApprovalResolved {
                    token: request.token,
                },
            );
        }
        for op in controls {
            self.fail(
                op,
                AgentError::Transport {
                    detail: reason.clone(),
                },
            );
        }
        if let Some(turn) = turn {
            let error = AgentError::Ambiguous {
                detail: reason.clone(),
            };
            self.emit(
                None,
                session,
                Some(turn),
                AgentEvent::TurnFinished(TurnOutcome::Failed(error)),
            );
        }
        self.emit(None, session, None, AgentEvent::RuntimeExited { reason });
        if let Some(record) = self.sessions.get_mut(&session) {
            record.runtime = None;
        }
    }

    fn handle_message(&mut self, session: SessionId, message: Message) {
        match message {
            Message::StreamEvent {
                event,
                parent_tool: None,
            } => self.stream_event(session, &event),
            Message::Assistant {
                message,
                parent_tool,
            } => self.assistant_message(session, &message, parent_tool.as_deref()),
            Message::User { message, .. } => self.user_message(session, &message),
            Message::Result(result) => self.result(session, &result),
            Message::ControlRequest {
                request_id,
                request,
            } => self.control_request(session, request_id, &request),
            Message::ControlResponse {
                request_id,
                success,
                ..
            } => {
                let op = self
                    .runtime(session)
                    .and_then(|r| r.controls.remove(&request_id));
                if let Some(op) = op {
                    if success {
                        self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
                    } else {
                        self.fail(
                            op,
                            AgentError::Provider {
                                message: "claude rejected the request".into(),
                            },
                        );
                    }
                }
            }
            Message::StreamEvent { .. }
            | Message::System { .. }
            | Message::RateLimit(_)
            | Message::Other(_) => {}
        }
    }

    fn stream_event(&mut self, session: SessionId, event: &Value) {
        let Some(turn) = self.active_turn(session) else {
            return;
        };
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        match kind {
            "message_start" => {
                let id = event["message"]["id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                if let Some(runtime) = self.runtime(session) {
                    runtime.stream.message_id = id;
                    runtime.stream.blocks.clear();
                }
            }
            "content_block_start" => {
                let block = &event["content_block"];
                match block["type"].as_str() {
                    Some("text") => {
                        let Some(runtime) = self.runtime(session) else {
                            return;
                        };
                        let id = ItemId(format!("{}:{index}", runtime.stream.message_id));
                        let message_id = runtime.stream.message_id.clone();
                        runtime.stream.streamed.insert(message_id);
                        runtime.stream.blocks.insert(index, Block::Text(id.clone()));
                        self.emit(
                            None,
                            session,
                            Some(turn),
                            AgentEvent::MessageStarted {
                                item: id,
                                phase: MessagePhase::Final,
                            },
                        );
                    }
                    Some("tool_use") => {
                        let tool_id = block["id"].as_str().unwrap_or_default().to_string();
                        let name = block["name"].as_str().unwrap_or("tool");
                        let call = convert::tool_call(name, &block["input"], None);
                        if let Some(runtime) = self.runtime(session) {
                            runtime.stream.blocks.insert(index, Block::Tool);
                            runtime.stream.tools.insert(tool_id.clone());
                        }
                        self.emit(
                            None,
                            session,
                            Some(turn),
                            AgentEvent::ToolStarted {
                                item: ItemId(tool_id),
                                call,
                            },
                        );
                    }
                    _ => {
                        if let Some(runtime) = self.runtime(session) {
                            runtime.stream.blocks.insert(index, Block::Ignored);
                        }
                    }
                }
            }
            "content_block_delta" => {
                let delta = &event["delta"];
                if delta["type"].as_str() != Some("text_delta") {
                    return;
                }
                let item = match self
                    .runtime(session)
                    .and_then(|r| r.stream.blocks.get(&index))
                {
                    Some(Block::Text(id)) => id.clone(),
                    _ => return,
                };
                let text = delta["text"].as_str().unwrap_or_default().to_string();
                self.emit(
                    None,
                    session,
                    Some(turn),
                    AgentEvent::TextDelta { item, delta: text },
                );
            }
            "content_block_stop" => {
                let block = self
                    .runtime(session)
                    .and_then(|r| r.stream.blocks.remove(&index));
                if let Some(Block::Text(item)) = block {
                    self.emit(
                        None,
                        session,
                        Some(turn),
                        AgentEvent::MessageFinished { item },
                    );
                }
            }
            _ => {}
        }
    }

    fn assistant_message(&mut self, session: SessionId, message: &Value, parent: Option<&str>) {
        let Some(turn) = self.active_turn(session) else {
            return;
        };
        let message_id = message["id"].as_str().unwrap_or_default().to_string();
        let streamed = self
            .runtime(session)
            .is_some_and(|r| r.stream.streamed.contains(&message_id));
        let Some(blocks) = message["content"].as_array() else {
            return;
        };
        for (index, block) in blocks.iter().enumerate() {
            match block["type"].as_str() {
                Some("text") if !streamed || parent.is_some() => {
                    let text = block["text"].as_str().unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    let item = ItemId(format!("{message_id}:{index}"));
                    let phase = MessagePhase::Final;
                    self.emit(
                        None,
                        session,
                        Some(turn.clone()),
                        AgentEvent::MessageStarted {
                            item: item.clone(),
                            phase,
                        },
                    );
                    self.emit(
                        None,
                        session,
                        Some(turn.clone()),
                        AgentEvent::TextDelta {
                            item: item.clone(),
                            delta: text.to_string(),
                        },
                    );
                    self.emit(
                        None,
                        session,
                        Some(turn.clone()),
                        AgentEvent::MessageFinished { item },
                    );
                }
                Some("tool_use") => {
                    let tool_id = block["id"].as_str().unwrap_or_default().to_string();
                    let name = block["name"].as_str().unwrap_or("tool");
                    let call = convert::tool_call(name, &block["input"], parent);
                    let known = self
                        .runtime(session)
                        .is_some_and(|r| !r.stream.tools.insert(tool_id.clone()));
                    let item = ItemId(tool_id);
                    if known {
                        let update = ToolUpdate {
                            title: Some(call.title.clone()),
                            status: Some(ToolStatus::Running),
                            locations: Some(call.locations.clone()),
                            patch: call.patch.clone(),
                            ..ToolUpdate::default()
                        };
                        self.emit(
                            None,
                            session,
                            Some(turn.clone()),
                            AgentEvent::ToolUpdated { item, update },
                        );
                    } else {
                        self.emit(
                            None,
                            session,
                            Some(turn.clone()),
                            AgentEvent::ToolStarted {
                                item: item.clone(),
                                call,
                            },
                        );
                        let update = ToolUpdate {
                            status: Some(ToolStatus::Running),
                            ..ToolUpdate::default()
                        };
                        self.emit(
                            None,
                            session,
                            Some(turn.clone()),
                            AgentEvent::ToolUpdated { item, update },
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn user_message(&mut self, session: SessionId, message: &Value) {
        let Some(turn) = self.active_turn(session) else {
            return;
        };
        let Some(blocks) = message["content"].as_array() else {
            return;
        };
        for block in blocks {
            if block["type"].as_str() != Some("tool_result") {
                continue;
            }
            let id = block["tool_use_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let known = self
                .runtime(session)
                .is_some_and(|r| r.stream.tools.contains(&id));
            if !known {
                continue;
            }
            let denied = self
                .runtime(session)
                .and_then(|r| r.active.as_ref().map(|a| a.denied.contains(&id)))
                .unwrap_or(false);
            let failed = block["is_error"].as_bool().unwrap_or(false);
            let status = if denied {
                ToolStatus::Declined
            } else if failed {
                ToolStatus::Failed
            } else {
                ToolStatus::Completed
            };
            let preview = convert::content_text(&block["content"]);
            let item = ItemId(id);
            if !preview.is_empty() && !denied {
                let append_preview = Some(preview.chars().take(4000).collect());
                let update = ToolUpdate {
                    append_preview,
                    ..ToolUpdate::default()
                };
                self.emit(
                    None,
                    session,
                    Some(turn.clone()),
                    AgentEvent::ToolUpdated {
                        item: item.clone(),
                        update,
                    },
                );
            }
            self.emit(
                None,
                session,
                Some(turn.clone()),
                AgentEvent::ToolFinished { item, status },
            );
        }
    }

    fn result(&mut self, session: SessionId, result: &Value) {
        let Some(runtime) = self.runtime(session) else {
            return;
        };
        let Some(active) = runtime.active.take() else {
            return;
        };
        runtime.stream = StreamState::default();
        let requests: Vec<PendingRequest> = runtime.requests.drain().map(|(_, r)| r).collect();
        let turn = active.turn;
        for request in requests {
            self.emit(
                None,
                session,
                Some(turn.clone()),
                AgentEvent::ApprovalResolved {
                    token: request.token,
                },
            );
        }
        self.emit(
            None,
            session,
            Some(turn.clone()),
            AgentEvent::UsageUpdated(convert::usage(result)),
        );
        let outcome = convert::turn_outcome(result, active.interrupt_requested);
        self.emit(None, session, Some(turn), AgentEvent::TurnFinished(outcome));
    }

    fn control_request(&mut self, session: SessionId, request_id: String, request: &Value) {
        let reply = |this: &mut Self, line: String| {
            if let Some(runtime) = this.runtime(session) {
                let _ = runtime.child.write(line.as_bytes());
            }
        };
        if request["subtype"].as_str() != Some("can_use_tool") {
            return reply(
                self,
                wire::control_error(&request_id, "unsupported control request"),
            );
        }
        let Some(turn) = self.active_turn(session) else {
            return reply(self, wire::deny(&request_id, DENY_MESSAGE));
        };
        if request["tool_name"].as_str() == Some("AskUserQuestion") {
            return reply(self, wire::deny(&request_id, NO_QUESTIONS));
        }
        let generation = self.runtime(session).map(|r| r.generation).unwrap_or(0);
        let token = self.token(session, &turn, generation, &request_id);
        let Some(approval) = convert::approval_request(request, token.clone()) else {
            return reply(
                self,
                wire::control_error(&request_id, "malformed permission request"),
            );
        };
        let pending = PendingRequest {
            request_id: request_id.clone(),
            input: request.get("input").cloned().unwrap_or(Value::Null),
            tool_use_id: request["tool_use_id"].as_str().map(str::to_string),
            token,
        };
        if let Some(runtime) = self.runtime(session) {
            runtime.requests.insert(request_id, pending);
        }
        self.emit(
            None,
            session,
            Some(turn),
            AgentEvent::ApprovalRequested(approval),
        );
    }

    // ------------------------------------------------- one-shots, retiring

    fn pump_one_shots(&mut self) {
        let mut finished = Vec::new();
        for (index, shot) in self.one_shots.iter_mut().enumerate() {
            for event in shot.child.drain(DRAIN_BYTES) {
                if let ChildEvent::Stdout(bytes) = event {
                    shot.out.extend_from_slice(&bytes);
                }
            }
            if !shot.exited && matches!(shot.child.try_exit(), Ok(Some(_))) {
                // One more pass to pick up output that raced the exit.
                shot.exited = true;
                continue;
            }
            if shot.exited {
                finished.push((index, false));
            } else if Instant::now() >= shot.deadline {
                shot.child.force_stop();
                finished.push((index, true));
            }
        }
        for (index, timed_out) in finished.into_iter().rev() {
            let shot = self.one_shots.remove(index);
            if timed_out {
                self.fail(
                    shot.op,
                    AgentError::Transport {
                        detail: "claude did not answer in time".into(),
                    },
                );
            } else {
                self.complete_one_shot(shot);
            }
        }
    }

    fn complete_one_shot(&mut self, shot: OneShot) {
        let text = String::from_utf8_lossy(&shot.out).to_string();
        match shot.kind {
            OneShotKind::Version => match convert::parse_version(&text) {
                Some(version) => {
                    self.descriptor.executable_version = Some(version.clone());
                    self.start_one_shot(
                        shot.op,
                        &["auth", "status"],
                        OneShotKind::AuthStatus {
                            probe_version: Some(version),
                        },
                    );
                }
                None => self.fail(
                    shot.op,
                    AgentError::ProviderMissing {
                        detail: "`claude --version` printed no version".into(),
                    },
                ),
            },
            OneShotKind::AuthStatus { probe_version } => {
                let auth = serde_json::from_str::<Value>(text.trim())
                    .map(|v| convert::auth_state(&v))
                    .unwrap_or(AuthState::Unknown);
                self.emit_in(
                    0,
                    None,
                    None,
                    None,
                    AgentEvent::AuthStateChanged(auth.clone()),
                );
                let output = match probe_version {
                    Some(version) => OperationOutput::Probed {
                        version,
                        capabilities: Box::new(self.capabilities.clone()),
                        auth,
                    },
                    None => OperationOutput::Auth(auth),
                };
                self.finish(shot.op, OperationOutcome::Succeeded(output));
            }
        }
    }

    fn pump_retiring(&mut self) {
        let now = Instant::now();
        self.retiring.retain_mut(|retiring| {
            let _ = retiring.child.drain(DRAIN_BYTES);
            if matches!(retiring.child.try_exit(), Ok(Some(_))) {
                return false;
            }
            if now >= retiring.deadline {
                retiring.child.force_stop();
                return false;
            }
            true
        });
    }

    fn advance_shutdown(&mut self) {
        let Some(shutdown) = &self.shutdown else {
            return;
        };
        let expired = Instant::now() >= shutdown.deadline;
        if (self.retiring.is_empty() && self.one_shots.is_empty()) || expired {
            for mut retiring in self.retiring.drain(..) {
                retiring.child.force_stop();
            }
            let op = shutdown.op;
            self.shutdown = None;
            self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
            // New work stays refused after shutdown.
            self.queue.clear();
            self.closed = true;
        }
    }
}

impl AgentBackend for ClaudeBackend {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    fn try_submit(&mut self, request: Request) -> Result<OperationId, SubmitError> {
        if self.shutdown.is_some() || self.closed {
            return Err(SubmitError::ShuttingDown);
        }
        if self.queue.len() >= QUEUE_LIMIT {
            return Err(SubmitError::QueueFull);
        }
        if let AgentOperation::StartTurn { input, .. } = &request.operation
            && input.is_empty()
        {
            return Err(SubmitError::Invalid(
                "a message needs at least one block".into(),
            ));
        }
        if !self.seen.insert(request.client_request_id) {
            return Err(SubmitError::Duplicate);
        }
        let id = OperationId(self.next_operation);
        self.next_operation += 1;
        self.queue.push_back((id, request.operation));
        Ok(id)
    }

    fn poll_events(&mut self, budget: PollBudget) -> Vec<EventEnvelope> {
        self.pump_sessions();
        if self.shutdown.is_none() {
            while let Some((op, operation)) = self.queue.pop_front() {
                self.dispatch(op, operation);
            }
        }
        self.pump_one_shots();
        self.pump_retiring();
        self.advance_shutdown();
        let take = budget.max_events.min(self.out.len());
        self.out.drain(..take).collect()
    }

    fn begin_shutdown(&mut self, deadline: Duration) -> Result<OperationId, SubmitError> {
        if self.shutdown.is_some() || self.closed {
            return Err(SubmitError::ShuttingDown);
        }
        let op = OperationId(self.next_operation);
        self.next_operation += 1;
        while let Some((queued, _)) = self.queue.pop_front() {
            self.finish(queued, OperationOutcome::Cancelled);
        }
        for shot in std::mem::take(&mut self.one_shots) {
            let mut shot = shot;
            shot.child.force_stop();
            self.finish(shot.op, OperationOutcome::Cancelled);
        }
        let ids: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.runtime.is_some())
            .map(|(id, _)| *id)
            .collect();
        for session in ids {
            // Requests waiting for the user are declined so nothing stays blocked,
            // and a running turn is reported as interrupted.
            let turn = self.active_turn(session);
            let requests: Vec<PendingRequest> = self
                .runtime(session)
                .map(|r| r.requests.drain().map(|(_, p)| p).collect())
                .unwrap_or_default();
            for request in requests {
                if let Some(runtime) = self.runtime(session) {
                    let _ = runtime
                        .child
                        .write(wire::deny(&request.request_id, DENY_MESSAGE).as_bytes());
                }
                self.emit(
                    None,
                    session,
                    turn.clone(),
                    AgentEvent::ApprovalResolved {
                        token: request.token,
                    },
                );
            }
            if let Some(turn) = turn {
                if let Some(runtime) = self.runtime(session) {
                    runtime.active = None;
                }
                self.emit(
                    None,
                    session,
                    Some(turn),
                    AgentEvent::TurnFinished(TurnOutcome::Interrupted),
                );
            }
            self.emit(
                None,
                session,
                None,
                AgentEvent::RuntimeExited {
                    reason: "Lumen is shutting down".into(),
                },
            );
            self.retire(session);
        }
        self.shutdown = Some(Shutdown {
            op,
            deadline: Instant::now() + deadline,
        });
        Ok(op)
    }
}

impl Drop for ClaudeBackend {
    fn drop(&mut self) {
        // Last resort only; the app calls begin_shutdown first.
        for session in self.sessions.values_mut() {
            if let Some(runtime) = session.runtime.as_mut() {
                runtime.child.force_stop();
            }
        }
        for retiring in &mut self.retiring {
            retiring.child.force_stop();
        }
        for shot in &mut self.one_shots {
            shot.child.force_stop();
        }
    }
}
