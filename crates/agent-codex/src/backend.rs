//! `CodexBackend`: the common interface over one `codex app-server` child.
//!
//! One app-server process serves every Codex session of a profile (it multiplexes
//! threads, verified in the spike); the process is started lazily through the
//! injected launcher, handshaken, and relaunched (with a new runtime generation)
//! after it dies. All I/O is non-blocking and driven by `poll_events`.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use lumen_agent_core::{
    AgentError, Capabilities, ClientRequestId, EventEnvelope, ItemId, OperationId, PollBudget,
    ProfileId, ProviderDescriptor, ProviderId, RequestToken, RuntimeGeneration, SessionId,
    SubmitError, Support, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, AuthState, ConfigPatch, ConfigRevision,
        InputBlock, OperationOutcome, OperationOutput, QuestionAnswer, Request, SessionConfig,
    },
    event::{AgentEvent, DiagnosticLevel},
    launcher::{Child, ChildEvent, ChildLauncher, ChildSpec, Wake},
    lines::{Line, LineBuffer},
    model::ToolUpdate,
};
use serde_json::{Value, json};

use crate::{
    convert,
    wire::{self, Frame},
};

const QUEUE_LIMIT: usize = 64;
/// Output processed per poll, so a flood cannot stall a frame.
const DRAIN_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct CodexConfig {
    /// The `codex` executable to run (`app-server` is appended).
    pub executable: PathBuf,
    pub client_name: String,
    pub client_version: String,
    pub profile: ProfileId,
}

impl CodexConfig {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            client_name: "lumen".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            profile: ProfileId::default_profile(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Down,
    Initializing,
    Ready,
}

/// What an in-flight JSON-RPC request is for.
enum Pending {
    Initialize,
    Models,
    Op(OperationId, Kind),
}

enum Kind {
    Probe,
    Auth,
    List,
    Create { session: SessionId },
    Attach { session: SessionId },
    History,
    Rename { session: SessionId, title: String },
    StartTurn { session: SessionId },
    Interrupt,
    Detach { session: SessionId },
    Archive,
}

struct Session {
    thread: String,
    config: SessionConfig,
    /// Native turn currently active (set by `turn/started`, cleared by completion).
    active: Option<TurnId>,
    /// A `turn/start` is in flight but the server has not reported the turn yet.
    starting: bool,
    loaded: bool,
}

struct ServerRequest {
    session: SessionId,
    turn: TurnId,
    method: String,
    raw_id: Value,
    /// The question id to answer, for `requestUserInput`.
    question: Option<String>,
    token: RequestToken,
}

struct Shutdown {
    op: OperationId,
    deadline: Instant,
    stopped: bool,
}

pub struct CodexBackend {
    launcher: Arc<dyn ChildLauncher>,
    wake: Wake,
    config: CodexConfig,
    descriptor: ProviderDescriptor,
    capabilities: Capabilities,
    generation: u64,
    sequence: u64,
    link: Link,
    child: Option<Box<dyn Child>>,
    lines: LineBuffer,
    next_rpc: u64,
    pending: HashMap<u64, Pending>,
    queue: VecDeque<(OperationId, AgentOperation)>,
    out: VecDeque<EventEnvelope>,
    sessions: HashMap<SessionId, Session>,
    threads: HashMap<String, SessionId>,
    server_requests: HashMap<String, ServerRequest>,
    models: Vec<convert::ModelInfo>,
    seen: HashSet<ClientRequestId>,
    next_operation: u64,
    shutdown: Option<Shutdown>,
    stderr_tail: String,
    /// Frames we could not make sense of (counted, never fatal).
    pub ignored_frames: u64,
}

fn capabilities() -> Capabilities {
    let mut caps = Capabilities::none();
    let yes = || Support::Supported;
    caps.streaming = yes();
    caps.list_history = yes();
    caps.read_history = yes();
    caps.resume = yes();
    caps.rename = yes();
    caps.archive = yes();
    caps.attachments_text = yes();
    caps.attachments_image = Support::unsupported("not implemented for Codex yet");
    caps.approvals = yes();
    caps.questions = Support::VersionGated {
        since: "0.157.0".into(),
        reason: "request_user_input is experimental in the app-server schema; one question per request is supported".into(),
    };
    caps.config_mutation = yes();
    caps.command_discovery = Support::unsupported("not implemented for Codex yet");
    caps.usage = yes();
    caps.enforced_read_only = yes();
    caps.interrupt = yes();
    caps.native_multi_session = yes();
    caps
}

impl CodexBackend {
    pub fn new(launcher: Arc<dyn ChildLauncher>, wake: Wake, config: CodexConfig) -> Self {
        let mut descriptor = ProviderDescriptor::new(ProviderId::Codex);
        descriptor.profile = config.profile.clone();
        Self {
            launcher,
            wake,
            config,
            descriptor,
            capabilities: capabilities(),
            generation: 0,
            sequence: 0,
            link: Link::Down,
            child: None,
            lines: LineBuffer::default(),
            next_rpc: 0,
            pending: HashMap::new(),
            queue: VecDeque::new(),
            out: VecDeque::new(),
            sessions: HashMap::new(),
            threads: HashMap::new(),
            server_requests: HashMap::new(),
            models: Vec::new(),
            seen: HashSet::new(),
            next_operation: 1,
            shutdown: None,
            stderr_tail: String::new(),
            ignored_frames: 0,
        }
    }

    // ------------------------------------------------------------ plumbing

    fn emit(
        &mut self,
        op: Option<OperationId>,
        session: Option<SessionId>,
        turn: Option<TurnId>,
        event: AgentEvent,
    ) {
        self.sequence += 1;
        self.out.push_back(EventEnvelope {
            provider: ProviderId::Codex,
            profile: self.config.profile.clone(),
            generation: RuntimeGeneration(self.generation.max(1)),
            operation: op,
            session,
            turn,
            sequence: self.sequence,
            event,
        });
    }

    fn finish(&mut self, op: OperationId, outcome: OperationOutcome) {
        self.emit(Some(op), None, None, AgentEvent::OperationFinished(outcome));
    }

    fn fail(&mut self, op: OperationId, error: AgentError) {
        self.finish(op, OperationOutcome::Failed(error));
    }

    fn diagnostic(&mut self, level: DiagnosticLevel, message: impl Into<String>) {
        self.emit(
            None,
            None,
            None,
            AgentEvent::Diagnostic {
                level,
                message: message.into(),
            },
        );
    }

    fn send_raw(&mut self, text: &str) -> bool {
        match self.child.as_mut() {
            Some(child) => child.write(text.as_bytes()).is_ok(),
            None => false,
        }
    }

    /// Send a request; returns false if the child is gone.
    fn rpc(&mut self, method: &str, params: Value, pending: Pending) -> bool {
        self.next_rpc += 1;
        let id = self.next_rpc;
        if !self.send_raw(&wire::request(id, method, params)) {
            return false;
        }
        self.pending.insert(id, pending);
        true
    }

    fn token(&self, session: SessionId, turn: &TurnId, native: &str) -> RequestToken {
        RequestToken {
            provider: ProviderId::Codex,
            profile: self.config.profile.clone(),
            session,
            turn: turn.clone(),
            generation: RuntimeGeneration(self.generation.max(1)),
            native: native.to_string(),
        }
    }

    fn launch(&mut self) -> Result<(), AgentError> {
        let spec = ChildSpec {
            program: self.config.executable.clone(),
            args: vec!["app-server".into()],
            env: Vec::new(),
            env_remove: Vec::new(),
            cwd: std::env::temp_dir(),
        };
        let child =
            self.launcher
                .launch(&spec, self.wake.clone())
                .map_err(|error| match error {
                    lumen_agent_core::launcher::LaunchError::NotFound(detail) => {
                        AgentError::ProviderMissing { detail }
                    }
                    lumen_agent_core::launcher::LaunchError::Failed(detail) => {
                        AgentError::Transport { detail }
                    }
                })?;
        self.child = Some(child);
        self.generation += 1;
        self.sequence = 0;
        self.lines = LineBuffer::default();
        self.link = Link::Initializing;
        let params = json!({"clientInfo": {
            "name": self.config.client_name,
            "title": "Lumen",
            "version": self.config.client_version,
        }});
        if !self.rpc("initialize", params, Pending::Initialize) {
            return Err(AgentError::Transport {
                detail: "could not talk to codex app-server".into(),
            });
        }
        Ok(())
    }

    // ---------------------------------------------------------- dispatching

    fn dispatch(&mut self, op: OperationId, operation: AgentOperation) {
        use AgentOperation as O;
        match operation {
            O::Probe => self.send_op(
                op,
                "account/read",
                json!({"refreshToken": false}),
                Kind::Probe,
            ),
            O::ReadAuthState => self.send_op(
                op,
                "account/read",
                json!({"refreshToken": false}),
                Kind::Auth,
            ),
            O::BeginAuthentication { .. } | O::CancelAuthentication { .. } => self.fail(
                op,
                AgentError::Unsupported {
                    reason: "sign in with `codex login` in a terminal, then retry".into(),
                },
            ),
            O::ListSessions {
                workspace,
                cursor,
                limit,
            } => {
                let mut params = json!({"limit": limit.clamp(1, 100), "archived": false});
                if let Some(workspace) = workspace {
                    params["cwd"] = json!(workspace.to_string_lossy());
                }
                if let Some(cursor) = cursor {
                    params["cursor"] = json!(cursor.0);
                }
                self.send_op(op, "thread/list", params, Kind::List);
            }
            O::CreateSession {
                session,
                workspace,
                config,
            } => {
                let mut params = json!({"cwd": workspace.to_string_lossy(), "ephemeral": false});
                if let Err(error) = apply_patch_to_start(&mut params, &config) {
                    return self.fail(op, error);
                }
                // A model from the user's own config can be one this account
                // cannot use, so a session always names a model the server listed.
                if params.get("model").is_none()
                    && let Some(model) = self
                        .models
                        .iter()
                        .find(|m| m.is_default)
                        .or(self.models.first())
                {
                    params["model"] = json!(model.id);
                }
                self.send_op(op, "thread/start", params, Kind::Create { session });
            }
            O::AttachSession {
                session,
                native,
                workspace,
            } => {
                if native.provider != ProviderId::Codex || native.profile != self.config.profile {
                    return self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that native session".into(),
                        },
                    );
                }
                let params = json!({"threadId": native.id, "cwd": workspace.to_string_lossy()});
                self.threads.insert(native.id.clone(), session);
                self.send_op(op, "thread/resume", params, Kind::Attach { session });
            }
            O::ReadHistory { session, .. } => match self.thread_of(session) {
                Some(thread) => self.send_op(
                    op,
                    "thread/read",
                    json!({"threadId": thread, "includeTurns": true}),
                    Kind::History,
                ),
                None => self.fail(
                    op,
                    AgentError::NotFound {
                        what: "that session".into(),
                    },
                ),
            },
            O::UpdateSessionMetadata { session, title } => match self.thread_of(session) {
                Some(thread) => self.send_op(
                    op,
                    "thread/name/set",
                    json!({"threadId": thread, "name": title}),
                    Kind::Rename { session, title },
                ),
                None => self.fail(
                    op,
                    AgentError::NotFound {
                        what: "that session".into(),
                    },
                ),
            },
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
            O::ListCommands { .. } => self.fail(
                op,
                AgentError::Unsupported {
                    reason: "command discovery is not implemented for Codex yet".into(),
                },
            ),
            O::StartTurn {
                session,
                input,
                expected_revision,
                ..
            } => self.start_turn(op, session, input, expected_revision),
            O::InterruptTurn { session, turn } => {
                let current = self.sessions.get(&session).and_then(|s| s.active.clone());
                match (self.thread_of(session), current) {
                    (Some(thread), Some(active)) if active == turn => {
                        self.send_op(
                            op,
                            "turn/interrupt",
                            json!({"threadId": thread, "turnId": turn.0}),
                            Kind::Interrupt,
                        );
                    }
                    _ => self.fail(
                        op,
                        AgentError::Stale {
                            what: "that turn".into(),
                        },
                    ),
                }
            }
            O::ReplyApproval { token, decision } => self.reply_approval(op, token, decision),
            O::ReplyQuestion { token, answers } => self.reply_question(op, token, answers),
            O::DetachSession { session } => {
                match (self.thread_of(session), self.sessions.get(&session)) {
                    (Some(_), Some(record)) if record.active.is_some() || record.starting => {
                        self.fail(op, AgentError::Busy)
                    }
                    (Some(thread), _) => self.send_op(
                        op,
                        "thread/unsubscribe",
                        json!({"threadId": thread}),
                        Kind::Detach { session },
                    ),
                    _ => self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that session".into(),
                        },
                    ),
                }
            }
            O::ArchiveSession { session } => {
                match (self.thread_of(session), self.sessions.get(&session)) {
                    (Some(_), Some(record)) if record.active.is_some() || record.starting => {
                        self.fail(op, AgentError::Busy)
                    }
                    (Some(thread), _) => self.send_op(
                        op,
                        "thread/archive",
                        json!({"threadId": thread}),
                        Kind::Archive,
                    ),
                    _ => self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that session".into(),
                        },
                    ),
                }
            }
        }
    }

    fn thread_of(&self, session: SessionId) -> Option<String> {
        self.sessions.get(&session).map(|s| s.thread.clone())
    }

    fn send_op(&mut self, op: OperationId, method: &str, params: Value, kind: Kind) {
        if !self.rpc(method, params, Pending::Op(op, kind)) {
            self.fail(
                op,
                AgentError::Transport {
                    detail: "the codex app-server is not running".into(),
                },
            );
        }
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
        for (id, value) in &patch.values {
            let valid = record
                .config
                .get(id)
                .is_some_and(|option| option.choices.iter().any(|c| c.value == *value));
            if !valid {
                return self.fail(
                    op,
                    AgentError::InvalidConfig {
                        detail: format!("{id} cannot be {value}"),
                    },
                );
            }
        }
        for (id, value) in &patch.values {
            if let Some(option) = record.config.options.iter_mut().find(|o| o.id == *id) {
                option.current = value.clone();
            }
        }
        record.config.revision = ConfigRevision(record.config.revision.0 + 1);
        let config = record.config.clone();
        self.emit(
            Some(op),
            Some(session),
            None,
            AgentEvent::ConfigChanged(config.clone()),
        );
        self.finish(
            op,
            OperationOutcome::Succeeded(OperationOutput::Config(config)),
        );
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
        if record.active.is_some() || record.starting {
            return self.fail(op, AgentError::Busy);
        }
        if !record.loaded {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "this session after the agent restarted; attach it again".into(),
                },
            );
        }
        if expected.is_some_and(|revision| revision != record.config.revision) {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "the configuration revision".into(),
                },
            );
        }
        let items = match convert::input_items(&input) {
            Ok(items) => items,
            Err(error) => return self.fail(op, error),
        };
        let mut params = json!({"threadId": record.thread, "input": items});
        let current = |id: &str| {
            record
                .config
                .get(id)
                .map(|o| o.current.clone())
                .filter(|v| !v.is_empty())
        };
        if let Some(model) = current("model") {
            params["model"] = json!(model);
        }
        if let Some(policy) = current("sandbox").and_then(|mode| convert::sandbox_policy(&mode)) {
            params["sandboxPolicy"] = policy;
        }
        if let Some(approval) = current("approval") {
            params["approvalPolicy"] = json!(approval);
        }
        if let Some(record) = self.sessions.get_mut(&session) {
            record.starting = true;
        }
        if !self.rpc(
            "turn/start",
            params,
            Pending::Op(op, Kind::StartTurn { session }),
        ) {
            if let Some(record) = self.sessions.get_mut(&session) {
                record.starting = false;
            }
            self.fail(
                op,
                AgentError::Transport {
                    detail: "the codex app-server is not running".into(),
                },
            );
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
        if !convert::is_approval_method(&request.method) {
            self.server_requests.insert(token.native.clone(), request);
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that approval request".into(),
                },
            );
        }
        if !convert::valid_decision(&decision.0) {
            self.server_requests.insert(token.native.clone(), request);
            return self.fail(
                op,
                AgentError::InvalidConfig {
                    detail: format!("unknown decision {}", decision.0),
                },
            );
        }
        let line = wire::reply(&request.raw_id, json!({"decision": decision.0}));
        if !self.send_raw(&line) {
            return self.fail(
                op,
                AgentError::Ambiguous {
                    detail: "the reply could not be delivered".into(),
                },
            );
        }
        self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
        self.settle(&request, true);
    }

    fn reply_question(
        &mut self,
        op: OperationId,
        token: RequestToken,
        answers: Vec<QuestionAnswer>,
    ) {
        let Some(request) = self.take_request(&token) else {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that question".into(),
                },
            );
        };
        let Some(question) = request.question.clone() else {
            self.server_requests.insert(token.native.clone(), request);
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that question".into(),
                },
            );
        };
        let texts: Vec<String> = answers
            .iter()
            .flat_map(|answer| match answer {
                QuestionAnswer::Text(text) | QuestionAnswer::Choice(text) => vec![text.clone()],
                QuestionAnswer::Choices(many) => many.clone(),
                QuestionAnswer::Cancelled => Vec::new(),
            })
            .collect();
        let line = wire::reply(
            &request.raw_id,
            json!({"answers": {question: {"answers": texts}}}),
        );
        if !self.send_raw(&line) {
            return self.fail(
                op,
                AgentError::Ambiguous {
                    detail: "the answer could not be delivered".into(),
                },
            );
        }
        self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
        self.settle(&request, false);
    }

    /// Remove a pending server request only if the token matches it exactly and
    /// belongs to the current runtime generation.
    fn take_request(&mut self, token: &RequestToken) -> Option<ServerRequest> {
        let current = RuntimeGeneration(self.generation.max(1));
        let matches = token.generation == current
            && self
                .server_requests
                .get(&token.native)
                .is_some_and(|r| r.token == *token);
        if matches {
            self.server_requests.remove(&token.native)
        } else {
            None
        }
    }

    fn settle(&mut self, request: &ServerRequest, approval: bool) {
        let event = if approval {
            AgentEvent::ApprovalResolved {
                token: request.token.clone(),
            }
        } else {
            AgentEvent::QuestionResolved {
                token: request.token.clone(),
            }
        };
        self.emit(
            None,
            Some(request.session),
            Some(request.turn.clone()),
            event,
        );
    }

    // ------------------------------------------------------------- frames

    fn handle_line(&mut self, line: Line) {
        match line {
            Line::Text(text) => match wire::parse(&text) {
                Ok(frame) => self.handle_frame(frame),
                Err(_) => self.ignored_frames += 1,
            },
            Line::TooLong | Line::InvalidUtf8 => self.ignored_frames += 1,
        }
    }

    fn handle_frame(&mut self, frame: Frame) {
        match frame {
            Frame::Response { id, result } => self.on_response(id, result),
            Frame::Error { id, code, message } => self.on_error(id, code, &message),
            Frame::Notification { method, params } => self.on_notification(&method, &params),
            Frame::ServerRequest {
                id,
                raw_id,
                method,
                params,
            } => self.on_server_request(id, raw_id, &method, &params),
        }
    }

    fn on_error(&mut self, id: u64, code: i64, message: &str) {
        let Some(pending) = self.pending.remove(&id) else {
            return;
        };
        let lower = message.to_lowercase();
        let error = if lower.contains("not found") || lower.contains("no such thread") {
            AgentError::NotFound {
                what: message.to_string(),
            }
        } else if lower.contains("not logged in") || lower.contains("unauthorized") {
            AgentError::AuthenticationRequired
        } else {
            AgentError::Provider {
                message: format!("{message} (code {code})"),
            }
        };
        match pending {
            Pending::Initialize => {
                self.diagnostic(
                    DiagnosticLevel::Error,
                    format!("The Codex app-server refused to start: {message}"),
                );
                self.fail_all_queued(error);
                self.link = Link::Down;
            }
            Pending::Models => self.diagnostic(
                DiagnosticLevel::Warning,
                format!("Could not list Codex models: {message}"),
            ),
            Pending::Op(op, kind) => {
                if let Kind::StartTurn { session } = kind
                    && let Some(record) = self.sessions.get_mut(&session)
                {
                    record.starting = false;
                }
                self.fail(op, error);
            }
        }
    }

    fn on_response(&mut self, id: u64, result: Value) {
        let Some(pending) = self.pending.remove(&id) else {
            self.ignored_frames += 1;
            return;
        };
        match pending {
            Pending::Initialize => {
                if let Some(agent) = result.get("userAgent").and_then(Value::as_str) {
                    // "<client>/<codex version> (os; arch) ..."
                    let version = agent.split(['/', ' ']).nth(1).unwrap_or("unknown");
                    self.descriptor.executable_version = Some(version.to_string());
                }
                self.send_raw(&wire::notification("initialized"));
                self.link = Link::Ready;
                self.rpc("model/list", json!({}), Pending::Models);
            }
            Pending::Models => self.models = convert::models(&result),
            Pending::Op(op, kind) => self.on_op_response(op, kind, &result),
        }
    }

    fn session_config(&self, thread: &Value, revision: u64) -> SessionConfig {
        let model = thread
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let sandbox = thread
            .get("sandbox")
            .and_then(|s| s.get("type"))
            .and_then(Value::as_str)
            .map(sandbox_name)
            .unwrap_or("workspace-write");
        let approval = thread
            .get("approvalPolicy")
            .and_then(Value::as_str)
            .unwrap_or("on-request");
        convert::session_config(revision, &self.models, model, sandbox, approval)
    }

    fn register(&mut self, session: SessionId, thread_id: &str, config: SessionConfig) {
        self.threads.insert(thread_id.to_string(), session);
        let existing = self.sessions.get(&session).map(|s| s.config.revision.0);
        let mut config = config;
        if let Some(previous) = existing {
            config.revision = ConfigRevision(previous + 1);
        }
        self.sessions.insert(
            session,
            Session {
                thread: thread_id.to_string(),
                config,
                active: None,
                starting: false,
                loaded: true,
            },
        );
    }

    /// A created or resumed thread is usable: remember it and report its config.
    fn session_ready(
        &mut self,
        op: OperationId,
        session: SessionId,
        result: &Value,
        created: bool,
    ) {
        let thread = result.get("thread").cloned().unwrap_or(Value::Null);
        let Some(id) = thread.get("id").and_then(Value::as_str).map(str::to_string) else {
            return self.fail(
                op,
                AgentError::Transport {
                    detail: "the app-server did not return a thread".into(),
                },
            );
        };
        let mut merged = result.clone();
        if merged.get("model").is_none() {
            merged["model"] = thread.get("model").cloned().unwrap_or(Value::Null);
        }
        let config = self.session_config(&merged, 1);
        self.register(session, &id, config);
        let config = self.sessions[&session].config.clone();
        let event = if created {
            AgentEvent::SessionCreated
        } else {
            AgentEvent::SessionAttached
        };
        self.emit(Some(op), Some(session), None, event);
        self.emit(
            Some(op),
            Some(session),
            None,
            AgentEvent::ConfigChanged(config.clone()),
        );
        let native = convert::native_ref(&self.config.profile, &id);
        let out = OperationOutput::SessionReady {
            native,
            config,
            capabilities: Box::new(self.capabilities.clone()),
        };
        self.finish(op, OperationOutcome::Succeeded(out));
    }

    fn on_op_response(&mut self, op: OperationId, kind: Kind, result: &Value) {
        match kind {
            Kind::Probe => {
                let version = self
                    .descriptor
                    .executable_version
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                let out = OperationOutput::Probed {
                    version,
                    capabilities: Box::new(self.capabilities.clone()),
                    auth: convert::auth_state(result),
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            Kind::Auth => {
                let auth = convert::auth_state(result);
                self.finish(op, OperationOutcome::Succeeded(OperationOutput::Auth(auth)));
            }
            Kind::List => {
                let profile = self.config.profile.clone();
                let summaries = result
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|thread| convert::thread_summary(thread, &profile))
                    .collect();
                let next = result
                    .get("nextCursor")
                    .and_then(Value::as_str)
                    .map(|c| lumen_agent_core::api::Cursor(c.to_string()));
                let out = OperationOutput::Sessions {
                    summaries,
                    complete: next.is_none(),
                    next,
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            Kind::Create { session } => self.session_ready(op, session, result, true),
            Kind::Attach { session } => self.session_ready(op, session, result, false),
            Kind::History => {
                let thread = result.get("thread").cloned().unwrap_or(Value::Null);
                let mut items = Vec::new();
                for turn in thread
                    .get("turns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let turn_id = TurnId(
                        turn.get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    );
                    for item in turn
                        .get("items")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(item) = convert::transcript_item(item, &turn_id) {
                            items.push(item);
                        }
                    }
                }
                self.finish(
                    op,
                    OperationOutcome::Succeeded(OperationOutput::History { items, next: None }),
                );
            }
            Kind::Rename { session, title } => {
                self.emit(
                    Some(op),
                    Some(session),
                    None,
                    AgentEvent::MetadataChanged { title },
                );
                self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
            }
            Kind::StartTurn { session } => {
                let turn = result
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .map(|id| TurnId(id.to_string()));
                if let Some(record) = self.sessions.get_mut(&session) {
                    record.starting = false;
                    if let Some(turn) = &turn
                        && record.active.is_none()
                    {
                        record.active = Some(turn.clone());
                    }
                }
                match turn {
                    Some(turn) => self.finish(
                        op,
                        OperationOutcome::Succeeded(OperationOutput::TurnStarted(turn)),
                    ),
                    None => self.fail(
                        op,
                        AgentError::Transport {
                            detail: "the app-server did not return a turn".into(),
                        },
                    ),
                }
            }
            Kind::Interrupt => self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack)),
            Kind::Detach { session } => {
                if let Some(record) = self.sessions.get_mut(&session) {
                    record.loaded = false;
                }
                self.emit(Some(op), Some(session), None, AgentEvent::SessionDetached);
                self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
            }
            Kind::Archive => self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack)),
        }
    }
}

fn sandbox_name(kind: &str) -> &'static str {
    match kind {
        "readOnly" => "read-only",
        "dangerFullAccess" => "danger-full-access",
        _ => "workspace-write",
    }
}

/// Apply create-time config choices to `thread/start` parameters.
fn apply_patch_to_start(params: &mut Value, patch: &ConfigPatch) -> Result<(), AgentError> {
    for (id, value) in &patch.values {
        match id.as_str() {
            "model" => params["model"] = json!(value),
            "sandbox" => {
                if !convert::SANDBOXES.iter().any(|(v, _)| v == value) {
                    return Err(AgentError::InvalidConfig {
                        detail: format!("sandbox cannot be {value}"),
                    });
                }
                params["sandbox"] = json!(value);
            }
            "approval" => {
                if !convert::APPROVALS.iter().any(|(v, _)| v == value) {
                    return Err(AgentError::InvalidConfig {
                        detail: format!("approvals cannot be {value}"),
                    });
                }
                params["approvalPolicy"] = json!(value);
            }
            other => {
                return Err(AgentError::InvalidConfig {
                    detail: format!("unknown setting {other}"),
                });
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------ notifications

impl CodexBackend {
    fn session_for(&self, params: &Value) -> Option<(SessionId, TurnId)> {
        let thread = params.get("threadId").and_then(Value::as_str)?;
        let session = *self.threads.get(thread)?;
        let turn = params
            .get("turnId")
            .and_then(Value::as_str)
            .or_else(|| {
                params
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
            })
            .map(|id| TurnId(id.to_string()))
            .or_else(|| self.sessions.get(&session).and_then(|s| s.active.clone()))
            .unwrap_or_else(|| TurnId(String::new()));
        Some((session, turn))
    }

    fn on_notification(&mut self, method: &str, params: &Value) {
        match method {
            "turn/started" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                if let Some(record) = self.sessions.get_mut(&session) {
                    record.active = Some(turn.clone());
                }
                self.emit(None, Some(session), Some(turn), AgentEvent::TurnStarted);
            }
            "turn/completed" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                let outcome = convert::turn_outcome(params.get("turn").unwrap_or(&Value::Null));
                if let Some(record) = self.sessions.get_mut(&session) {
                    record.active = None;
                    record.starting = false;
                }
                // Requests of a finished turn can no longer be answered.
                let stale: Vec<String> = self
                    .server_requests
                    .iter()
                    .filter(|(_, r)| r.session == session)
                    .map(|(k, _)| k.clone())
                    .collect();
                for key in stale {
                    self.server_requests.remove(&key);
                }
                self.emit(
                    None,
                    Some(session),
                    Some(turn),
                    AgentEvent::TurnFinished(outcome),
                );
            }
            "item/started" | "item/completed" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                let Some(item) = params.get("item") else {
                    return;
                };
                let events = if method == "item/started" {
                    convert::item_started(item)
                } else {
                    convert::item_completed(item)
                };
                for event in events {
                    self.emit(None, Some(session), Some(turn.clone()), event);
                }
            }
            "item/agentMessage/delta" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                let (Some(item), Some(delta)) = (
                    params.get("itemId").and_then(Value::as_str),
                    params.get("delta").and_then(Value::as_str),
                ) else {
                    return;
                };
                self.emit(
                    None,
                    Some(session),
                    Some(turn),
                    AgentEvent::TextDelta {
                        item: ItemId(item.into()),
                        delta: delta.into(),
                    },
                );
            }
            "item/commandExecution/outputDelta" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                let (Some(item), Some(delta)) = (
                    params.get("itemId").and_then(Value::as_str),
                    params.get("delta").and_then(Value::as_str),
                ) else {
                    return;
                };
                let update = ToolUpdate {
                    append_preview: Some(delta.into()),
                    ..Default::default()
                };
                self.emit(
                    None,
                    Some(session),
                    Some(turn),
                    AgentEvent::ToolUpdated {
                        item: ItemId(item.into()),
                        update,
                    },
                );
            }
            "turn/plan/updated" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                let item = ItemId(format!("plan-{}", turn.0));
                self.emit(
                    None,
                    Some(session),
                    Some(turn),
                    AgentEvent::PlanUpdated {
                        item,
                        entries: convert::plan_entries(params),
                    },
                );
            }
            "thread/tokenUsage/updated" => {
                let Some((session, turn)) = self.session_for(params) else {
                    return;
                };
                self.emit(
                    None,
                    Some(session),
                    Some(turn),
                    AgentEvent::UsageUpdated(convert::usage(params)),
                );
            }
            "thread/name/updated" => {
                let Some((session, _)) = self.session_for(params) else {
                    return;
                };
                if let Some(name) = params.get("threadName").and_then(Value::as_str) {
                    self.emit(
                        None,
                        Some(session),
                        None,
                        AgentEvent::MetadataChanged { title: name.into() },
                    );
                }
            }
            "account/updated" => {
                let label = match params.get("authMode").and_then(Value::as_str) {
                    Some("chatgpt") => format!(
                        "ChatGPT ({})",
                        params
                            .get("planType")
                            .and_then(Value::as_str)
                            .unwrap_or("signed in")
                    ),
                    Some(other) => other.to_string(),
                    None => return,
                };
                self.emit(
                    None,
                    None,
                    None,
                    AgentEvent::AuthStateChanged(AuthState::SignedIn { label }),
                );
            }
            "serverRequest/resolved" => {
                let Some(id) = params.get("requestId") else {
                    return;
                };
                let key = match id {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                // Resolved elsewhere (or by the app-server itself): clear it here too.
                if let Some(request) = self.server_requests.remove(&key) {
                    let approval = convert::is_approval_method(&request.method);
                    self.settle(&request, approval);
                }
            }
            "error" => {
                let message = params
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("The agent reported an error");
                let retry = params
                    .get("willRetry")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let level = if retry {
                    DiagnosticLevel::Warning
                } else {
                    DiagnosticLevel::Error
                };
                self.diagnostic(level, message.to_string());
            }
            "warning" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    self.diagnostic(DiagnosticLevel::Warning, message.to_string());
                }
            }
            // Status, MCP startup, rate limits, reasoning and the many other
            // notifications do not affect the transcript.
            _ => {}
        }
    }

    fn on_server_request(&mut self, id: String, raw_id: Value, method: &str, params: &Value) {
        let Some((session, turn)) = self.session_for(params) else {
            self.send_raw(&wire::reply_error(&raw_id, -32602, "unknown thread"));
            return;
        };
        let token = self.token(session, &turn, &id);
        if convert::is_approval_method(method) {
            let Some(request) = convert::approval_request(method, params, token.clone()) else {
                return;
            };
            self.server_requests.insert(
                id,
                ServerRequest {
                    session,
                    turn: turn.clone(),
                    method: method.into(),
                    raw_id,
                    question: None,
                    token,
                },
            );
            self.emit(
                None,
                Some(session),
                Some(turn),
                AgentEvent::ApprovalRequested(request),
            );
        } else if method == "item/tool/requestUserInput" {
            match convert::question_request(params, token.clone()) {
                Some((request, question)) => {
                    self.server_requests.insert(
                        id,
                        ServerRequest {
                            session,
                            turn: turn.clone(),
                            method: method.into(),
                            raw_id,
                            question: Some(question),
                            token,
                        },
                    );
                    self.emit(
                        None,
                        Some(session),
                        Some(turn),
                        AgentEvent::QuestionRequested(request),
                    );
                }
                None => {
                    self.send_raw(&wire::reply_error(
                        &raw_id,
                        -32602,
                        "Lumen supports one question per request",
                    ));
                    self.diagnostic(DiagnosticLevel::Warning, "Codex asked several questions at once, which Lumen does not support yet; the request was declined.");
                }
            }
        } else {
            // Unknown or unsupported server requests (permission profiles, tool
            // calls, token refresh, ...) are refused, never auto-approved.
            self.send_raw(&wire::reply_error(
                &raw_id,
                -32601,
                "not supported by this client",
            ));
            self.diagnostic(DiagnosticLevel::Warning, format!("Codex asked for something Lumen does not support ({method}); the request was declined."));
        }
    }
}

// ------------------------------------------------------------------ polling

impl CodexBackend {
    fn fail_all_queued(&mut self, error: AgentError) {
        while let Some((op, _)) = self.queue.pop_front() {
            self.fail(op, error.clone());
        }
    }

    /// The child exited: settle everything that depended on it.
    fn on_runtime_exit(&mut self, reason: String) {
        self.child = None;
        self.link = Link::Down;
        let sessions: Vec<(SessionId, bool)> = self
            .sessions
            .iter()
            .map(|(id, s)| (*id, s.active.is_some() || s.starting))
            .collect();
        for (session, was_busy) in sessions {
            let turn = self.sessions.get(&session).and_then(|s| s.active.clone());
            if let Some(record) = self.sessions.get_mut(&session) {
                record.active = None;
                record.starting = false;
                record.loaded = false;
            }
            if was_busy {
                self.emit(
                    None,
                    Some(session),
                    turn,
                    AgentEvent::RuntimeExited {
                        reason: reason.clone(),
                    },
                );
            }
        }
        self.server_requests.clear();
        // Requests that were in flight have an unknown outcome and are not resent.
        let pending: Vec<_> = self.pending.drain().collect();
        for (_, pending) in pending {
            if let Pending::Op(op, kind) = pending {
                let error = match kind {
                    Kind::StartTurn { .. }
                    | Kind::Interrupt
                    | Kind::Rename { .. }
                    | Kind::Archive => AgentError::Ambiguous {
                        detail: reason.clone(),
                    },
                    _ => AgentError::Transport {
                        detail: reason.clone(),
                    },
                };
                self.fail(op, error);
            }
        }
        self.threads.clear();
        self.sequence = 0;
        if !self.stderr_tail.is_empty() {
            let tail = std::mem::take(&mut self.stderr_tail);
            self.diagnostic(
                DiagnosticLevel::Warning,
                format!("codex app-server said: {tail}"),
            );
        }
    }

    fn pump_child(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        for event in child.drain(DRAIN_BYTES) {
            match event {
                ChildEvent::Stdout(bytes) => {
                    for line in self.lines.push(&bytes) {
                        self.handle_line(line);
                    }
                }
                ChildEvent::Stderr(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    self.stderr_tail.push_str(text.trim());
                    if self.stderr_tail.len() > 300 {
                        let keep = self.stderr_tail.len() - 300;
                        let boundary = (keep..self.stderr_tail.len())
                            .find(|i| self.stderr_tail.is_char_boundary(*i))
                            .unwrap_or(0);
                        self.stderr_tail.drain(..boundary);
                    }
                }
                ChildEvent::StdoutClosed | ChildEvent::StderrClosed => {}
            }
            if self.child.is_none() {
                break;
            }
        }
        let exit = self
            .child
            .as_mut()
            .and_then(|child| child.try_exit().ok().flatten());
        if let Some(exit) = exit {
            let reason = match (exit.code, exit.signal) {
                (Some(code), _) => format!("exited with code {code}"),
                (None, Some(signal)) => format!("killed by signal {signal}"),
                _ => "exited".to_string(),
            };
            if self.shutdown.is_none() {
                self.on_runtime_exit(reason);
            } else {
                self.child = None;
                self.link = Link::Down;
            }
        }
    }

    fn advance_shutdown(&mut self) {
        let Some(shutdown) = &self.shutdown else {
            return;
        };
        let (op, deadline, stopped) = (shutdown.op, shutdown.deadline, shutdown.stopped);
        let gone = self.child.is_none();
        if gone || Instant::now() >= deadline {
            if let Some(child) = self.child.as_mut() {
                if !stopped {
                    child.request_stop();
                }
                child.force_stop();
            }
            self.child = None;
            self.shutdown = None;
            self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
        } else if !stopped {
            if let Some(child) = self.child.as_mut() {
                child.close_stdin();
                child.request_stop();
            }
            if let Some(shutdown) = self.shutdown.as_mut() {
                shutdown.stopped = true;
            }
        }
    }
}

impl AgentBackend for CodexBackend {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    fn try_submit(&mut self, request: Request) -> Result<OperationId, SubmitError> {
        if self.shutdown.is_some() {
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
        self.pump_child();
        if self.child.is_none()
            && self.shutdown.is_none()
            && !self.queue.is_empty()
            && self.link == Link::Down
            && let Err(error) = self.launch()
        {
            self.fail_all_queued(error);
        }
        // Work waits for the model list so new sessions can name a usable model.
        let models_pending = self.pending.values().any(|p| matches!(p, Pending::Models));
        if self.link == Link::Ready && !models_pending {
            while let Some((op, operation)) = self.queue.pop_front() {
                self.dispatch(op, operation);
                // A dispatch can discover the child is gone.
                if self.link != Link::Ready {
                    break;
                }
            }
        }
        self.advance_shutdown();
        let take = budget.max_events.min(self.out.len());
        self.out.drain(..take).collect()
    }

    fn begin_shutdown(&mut self, deadline: Duration) -> Result<OperationId, SubmitError> {
        if self.shutdown.is_some() {
            return Err(SubmitError::ShuttingDown);
        }
        let op = OperationId(self.next_operation);
        self.next_operation += 1;
        // Operations that never started are cancelled; requests waiting for the
        // user are declined so the agent is not left blocked.
        while let Some((queued, _)) = self.queue.pop_front() {
            self.finish(queued, OperationOutcome::Cancelled);
        }
        let requests: Vec<ServerRequest> = self.server_requests.drain().map(|(_, r)| r).collect();
        for request in requests {
            let decision = if convert::is_approval_method(&request.method) {
                wire::reply(&request.raw_id, json!({"decision": "cancel"}))
            } else {
                wire::reply_error(&request.raw_id, -32603, "the client is shutting down")
            };
            self.send_raw(&decision);
            let approval = convert::is_approval_method(&request.method);
            self.settle(&request, approval);
        }
        let active: Vec<(SessionId, String, TurnId)> = self
            .sessions
            .iter()
            .filter_map(|(id, s)| s.active.clone().map(|turn| (*id, s.thread.clone(), turn)))
            .collect();
        for (_, thread, turn) in active {
            let line = wire::request(
                self.next_rpc + 1,
                "turn/interrupt",
                json!({"threadId": thread, "turnId": turn.0}),
            );
            self.next_rpc += 1;
            self.send_raw(&line);
        }
        let pending: Vec<_> = self.pending.drain().collect();
        for (_, pending) in pending {
            if let Pending::Op(waiting, _) = pending {
                self.finish(waiting, OperationOutcome::Cancelled);
            }
        }
        self.shutdown = Some(Shutdown {
            op,
            deadline: Instant::now() + deadline,
            stopped: false,
        });
        Ok(op)
    }
}

impl Drop for CodexBackend {
    fn drop(&mut self) {
        // Last resort only; the app calls begin_shutdown first.
        if let Some(child) = self.child.as_mut() {
            child.force_stop();
        }
    }
}
