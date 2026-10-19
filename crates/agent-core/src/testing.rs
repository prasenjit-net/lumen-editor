//! Test support shared by the app and the provider crates: a scriptable fake
//! backend (deterministic, no processes), a fake child launcher, and the contract
//! checks every backend must pass.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    AgentError, Capabilities, ClientRequestId, EventEnvelope, NativeSessionRef, OperationId,
    PollBudget, ProfileId, ProviderDescriptor, ProviderId, RequestToken, RuntimeGeneration,
    SessionId, SubmitError, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, AuthState, ConfigChoice, ConfigOption,
        ConfigRevision, InputBlock, OperationOutcome, OperationOutput, QuestionAnswer, Request,
        SessionConfig, SessionSummary,
    },
    event::{
        AgentEvent, ApprovalChoice, ApprovalRequest, QuestionKind, QuestionRequest, RiskLevel,
        TurnOutcome,
    },
    launcher::{Child, ChildEvent, ChildLauncher, ChildSpec, ExitInfo, LaunchError, Wake},
    model::{MessagePhase, ToolCall, ToolKind, ToolStatus},
};

const QUEUE_LIMIT: usize = 64;

struct FakeSession {
    native: NativeSessionRef,
    workspace: std::path::PathBuf,
    config: SessionConfig,
    active: Option<ActiveTurn>,
    title: Option<String>,
    archived: bool,
}

struct ActiveTurn {
    turn: TurnId,
    kind: TurnKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TurnKind {
    /// Waits for an interrupt.
    Slow,
    Approval(RequestToken),
    Question(RequestToken),
}

/// A deterministic backend. The prompt text selects the behavior:
/// `approve:` asks for an approval, `ask:` asks a question, `slow:` runs until
/// interrupted, `crash:` kills the runtime mid-turn, anything else is echoed.
pub struct FakeBackend {
    descriptor: ProviderDescriptor,
    generation: RuntimeGeneration,
    sequence: u64,
    next_operation: u64,
    next_native: u64,
    next_turn: u64,
    next_request: u64,
    seen: HashSet<ClientRequestId>,
    pending: VecDeque<(OperationId, AgentOperation)>,
    out: VecDeque<EventEnvelope>,
    sessions: HashMap<SessionId, FakeSession>,
    shutdown: bool,
    /// Every operation ever accepted, to check each ends exactly once.
    pub accepted: Vec<OperationId>,
}

impl FakeBackend {
    pub fn new(provider: ProviderId) -> Self {
        Self {
            descriptor: ProviderDescriptor::new(provider),
            generation: RuntimeGeneration(1),
            sequence: 0,
            next_operation: 1,
            next_native: 1,
            next_turn: 1,
            next_request: 1,
            seen: HashSet::new(),
            pending: VecDeque::new(),
            out: VecDeque::new(),
            sessions: HashMap::new(),
            shutdown: false,
            accepted: Vec::new(),
        }
    }

    fn emit(
        &mut self,
        operation: Option<OperationId>,
        session: Option<SessionId>,
        turn: Option<TurnId>,
        event: AgentEvent,
    ) {
        self.sequence += 1;
        self.out.push_back(EventEnvelope {
            provider: self.descriptor.provider,
            profile: self.descriptor.profile.clone(),
            generation: self.generation,
            operation,
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

    fn config(revision: u64) -> SessionConfig {
        SessionConfig {
            revision: ConfigRevision(revision),
            options: vec![
                ConfigOption::select(
                    "model",
                    "Model",
                    "Which model answers",
                    vec![
                        ConfigChoice::new("fast", "Fast"),
                        ConfigChoice::new("smart", "Smart"),
                    ],
                    "fast",
                )
                .with_default("fast"),
                ConfigOption::toggle("verbose", "Verbose", "Explain more", false),
            ],
        }
    }

    fn token(&mut self, session: SessionId, turn: &TurnId) -> RequestToken {
        let native = format!("req-{}", self.next_request);
        self.next_request += 1;
        RequestToken {
            provider: self.descriptor.provider,
            profile: self.descriptor.profile.clone(),
            session,
            turn: turn.clone(),
            generation: self.generation,
            native,
        }
    }

    fn run(&mut self, op: OperationId, operation: AgentOperation) {
        use AgentOperation as O;
        match operation {
            O::Probe => {
                let out = OperationOutput::Probed {
                    version: "fake-1".into(),
                    capabilities: Box::new(Capabilities::all()),
                    auth: AuthState::SignedIn {
                        label: "fake account".into(),
                    },
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            O::ReadAuthState => {
                let out = OperationOutput::Auth(AuthState::SignedIn {
                    label: "fake account".into(),
                });
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            O::BeginAuthentication { .. } | O::CancelAuthentication { .. } => {
                self.fail(
                    op,
                    AgentError::Unsupported {
                        reason: "the fake is always signed in".into(),
                    },
                );
            }
            O::ListSessions {
                workspace, limit, ..
            } => {
                let mut summaries: Vec<SessionSummary> = self
                    .sessions
                    .values()
                    .filter(|s| workspace.as_ref().is_none_or(|w| *w == s.workspace))
                    .map(|s| SessionSummary {
                        native: s.native.clone(),
                        title: s.title.clone(),
                        workspace: Some(s.workspace.clone()),
                        last_activity: None,
                        archived: s.archived,
                    })
                    .collect();
                summaries.sort_by(|a, b| a.native.id.cmp(&b.native.id));
                summaries.truncate(limit.max(1));
                let out = OperationOutput::Sessions {
                    summaries,
                    next: None,
                    complete: true,
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            O::CreateSession {
                session, workspace, ..
            } => {
                if self.sessions.contains_key(&session) {
                    return self.fail(
                        op,
                        AgentError::Stale {
                            what: "that session id".into(),
                        },
                    );
                }
                let native = NativeSessionRef {
                    provider: self.descriptor.provider,
                    profile: self.descriptor.profile.clone(),
                    id: format!("fake-{}", self.next_native),
                };
                self.next_native += 1;
                let config = Self::config(1);
                self.sessions.insert(
                    session,
                    FakeSession {
                        native: native.clone(),
                        workspace,
                        config: config.clone(),
                        active: None,
                        title: None,
                        archived: false,
                    },
                );
                self.emit(Some(op), Some(session), None, AgentEvent::SessionCreated);
                let out = OperationOutput::SessionReady {
                    native,
                    config,
                    capabilities: Box::new(Capabilities::all()),
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            O::AttachSession {
                session,
                native,
                workspace,
            } => {
                let known = self
                    .sessions
                    .values()
                    .find(|s| s.native == native)
                    .is_some();
                if native.provider != self.descriptor.provider || !known {
                    return self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that native session".into(),
                        },
                    );
                }
                let existing = self
                    .sessions
                    .iter()
                    .find(|(_, s)| s.native == native)
                    .map(|(id, _)| *id);
                if let Some(old) = existing
                    && old != session
                    && let Some(record) = self.sessions.remove(&old)
                {
                    self.sessions.insert(session, record);
                }
                let record = self
                    .sessions
                    .get_mut(&session)
                    .expect("just inserted or present");
                record.workspace = workspace;
                let (native, config) = (record.native.clone(), record.config.clone());
                self.emit(Some(op), Some(session), None, AgentEvent::SessionAttached);
                let out = OperationOutput::SessionReady {
                    native,
                    config,
                    capabilities: Box::new(Capabilities::all()),
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            O::ReadHistory { session, .. } => {
                if !self.sessions.contains_key(&session) {
                    return self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that session".into(),
                        },
                    );
                }
                let out = OperationOutput::History {
                    items: Vec::new(),
                    next: None,
                };
                self.finish(op, OperationOutcome::Succeeded(out));
            }
            O::UpdateSessionMetadata { session, title } => match self.sessions.get_mut(&session) {
                Some(record) => {
                    record.title = Some(title.clone());
                    self.emit(
                        Some(op),
                        Some(session),
                        None,
                        AgentEvent::MetadataChanged { title },
                    );
                    self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
                }
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
            } => {
                let Some(record) = self.sessions.get_mut(&session) else {
                    return self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that session".into(),
                        },
                    );
                };
                if record.config.revision != expected_revision {
                    return self.fail(
                        op,
                        AgentError::Stale {
                            what: "the configuration revision".into(),
                        },
                    );
                }
                match record.config.applying(&patch) {
                    Ok(next) => record.config = next,
                    Err(error) => return self.fail(op, error),
                }
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
            O::ListCommands { session } => {
                if self.sessions.contains_key(&session) {
                    self.finish(
                        op,
                        OperationOutcome::Succeeded(OperationOutput::Commands(Vec::new())),
                    );
                } else {
                    self.fail(
                        op,
                        AgentError::NotFound {
                            what: "that session".into(),
                        },
                    );
                }
            }
            O::StartTurn {
                session,
                input,
                expected_revision,
                ..
            } => self.start_turn(op, session, input, expected_revision),
            O::InterruptTurn { session, turn } => {
                let matches = self
                    .sessions
                    .get(&session)
                    .and_then(|s| s.active.as_ref())
                    .is_some_and(|a| a.turn == turn);
                if !matches {
                    return self.fail(
                        op,
                        AgentError::Stale {
                            what: "that turn".into(),
                        },
                    );
                }
                self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
                self.end_turn(session, TurnOutcome::Interrupted);
            }
            O::ReplyApproval { token, decision } => self.reply_approval(op, token, decision),
            O::ReplyQuestion { token, answers } => self.reply_question(op, token, answers),
            O::DetachSession { session } => match self.sessions.get(&session) {
                None => self.fail(
                    op,
                    AgentError::NotFound {
                        what: "that session".into(),
                    },
                ),
                Some(record) if record.active.is_some() => self.fail(op, AgentError::Busy),
                Some(_) => {
                    self.emit(Some(op), Some(session), None, AgentEvent::SessionDetached);
                    self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
                }
            },
            O::ArchiveSession { session } => match self.sessions.get_mut(&session) {
                None => self.fail(
                    op,
                    AgentError::NotFound {
                        what: "that session".into(),
                    },
                ),
                Some(record) if record.active.is_some() => self.fail(op, AgentError::Busy),
                Some(record) => {
                    record.archived = true;
                    self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
                }
            },
        }
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
        if record.active.is_some() {
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
        let text: String = input
            .iter()
            .filter_map(|block| match block {
                InputBlock::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let turn = TurnId(format!("turn-{}", self.next_turn));
        self.next_turn += 1;
        self.finish(
            op,
            OperationOutcome::Succeeded(OperationOutput::TurnStarted(turn.clone())),
        );
        self.emit(
            None,
            Some(session),
            Some(turn.clone()),
            AgentEvent::TurnStarted,
        );
        if text.starts_with("crash:") {
            self.sessions.get_mut(&session).expect("exists").active = Some(ActiveTurn {
                turn: turn.clone(),
                kind: TurnKind::Slow,
            });
            self.emit(
                None,
                Some(session),
                Some(turn),
                AgentEvent::RuntimeExited {
                    reason: "the fake runtime crashed".into(),
                },
            );
            self.generation = RuntimeGeneration(self.generation.0 + 1);
            self.sequence = 0;
            if let Some(record) = self.sessions.get_mut(&session) {
                record.active = None;
            }
            return;
        }
        let turn_name = turn.0.clone();
        let item = move |n: u64| crate::ItemId(format!("{turn_name}-{n}"));
        if text.starts_with("approve:") {
            let token = self.token(session, &turn);
            let mut call = ToolCall::new(ToolKind::Command, "touch note.txt");
            call.status = ToolStatus::Pending;
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::ToolStarted {
                    item: item(1),
                    call,
                },
            );
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::ApprovalRequested(ApprovalRequest {
                    token: token.clone(),
                    summary: "Run a command".into(),
                    command: Some("touch note.txt".into()),
                    paths: vec![],
                    choices: vec![
                        ApprovalChoice {
                            id: ApprovalDecisionId("accept".into()),
                            label: "Allow once".into(),
                            scope: "this command".into(),
                            allows: true,
                            risk: RiskLevel::Medium,
                        },
                        ApprovalChoice {
                            id: ApprovalDecisionId("decline".into()),
                            label: "Decline".into(),
                            scope: "this command".into(),
                            allows: false,
                            risk: RiskLevel::Low,
                        },
                    ],
                }),
            );
            self.sessions.get_mut(&session).expect("exists").active = Some(ActiveTurn {
                turn,
                kind: TurnKind::Approval(token),
            });
        } else if text.starts_with("ask:") {
            let token = self.token(session, &turn);
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::QuestionRequested(QuestionRequest {
                    token: token.clone(),
                    prompt: "Which one?".into(),
                    kind: QuestionKind::SingleChoice(vec!["a".into(), "b".into()]),
                }),
            );
            self.sessions.get_mut(&session).expect("exists").active = Some(ActiveTurn {
                turn,
                kind: TurnKind::Question(token),
            });
        } else if text.starts_with("slow:") {
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::MessageStarted {
                    item: item(1),
                    phase: MessagePhase::Final,
                },
            );
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::TextDelta {
                    item: item(1),
                    delta: "working".into(),
                },
            );
            self.sessions.get_mut(&session).expect("exists").active = Some(ActiveTurn {
                turn,
                kind: TurnKind::Slow,
            });
        } else {
            self.sessions.get_mut(&session).expect("exists").active = Some(ActiveTurn {
                turn: turn.clone(),
                kind: TurnKind::Slow,
            });
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::MessageStarted {
                    item: item(1),
                    phase: MessagePhase::Final,
                },
            );
            self.emit(
                None,
                Some(session),
                Some(turn.clone()),
                AgentEvent::TextDelta {
                    item: item(1),
                    delta: format!("echo: {text}"),
                },
            );
            self.emit(
                None,
                Some(session),
                Some(turn),
                AgentEvent::MessageFinished { item: item(1) },
            );
            self.end_turn(session, TurnOutcome::Completed);
        }
    }

    fn end_turn(&mut self, session: SessionId, outcome: TurnOutcome) {
        let Some(active) = self
            .sessions
            .get_mut(&session)
            .and_then(|s| s.active.take())
        else {
            return;
        };
        self.emit(
            None,
            Some(session),
            Some(active.turn),
            AgentEvent::TurnFinished(outcome),
        );
    }

    fn find_request(&self, token: &RequestToken) -> Option<SessionId> {
        let record = self.sessions.get(&token.session)?;
        let active = record.active.as_ref()?;
        let matches = match &active.kind {
            TurnKind::Approval(t) | TurnKind::Question(t) => t == token,
            TurnKind::Slow => false,
        };
        (matches && token.generation == self.generation && active.turn == token.turn)
            .then_some(token.session)
    }

    fn reply_approval(
        &mut self,
        op: OperationId,
        token: RequestToken,
        decision: ApprovalDecisionId,
    ) {
        let Some(session) = self.find_request(&token) else {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that approval request".into(),
                },
            );
        };
        let is_approval = matches!(
            self.sessions[&session].active.as_ref().map(|a| &a.kind),
            Some(TurnKind::Approval(_))
        );
        if !is_approval {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that approval request".into(),
                },
            );
        }
        self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
        let turn = token.turn.clone();
        self.emit(
            None,
            Some(session),
            Some(turn.clone()),
            AgentEvent::ApprovalResolved { token },
        );
        let allowed = decision.0 == "accept";
        self.emit(
            None,
            Some(session),
            Some(turn.clone()),
            AgentEvent::ToolFinished {
                item: crate::ItemId(format!("{}-1", turn.0)),
                status: if allowed {
                    ToolStatus::Completed
                } else {
                    ToolStatus::Declined
                },
            },
        );
        self.end_turn(session, TurnOutcome::Completed);
    }

    fn reply_question(
        &mut self,
        op: OperationId,
        token: RequestToken,
        answers: Vec<QuestionAnswer>,
    ) {
        let Some(session) = self.find_request(&token) else {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that question".into(),
                },
            );
        };
        if !matches!(
            self.sessions[&session].active.as_ref().map(|a| &a.kind),
            Some(TurnKind::Question(_))
        ) {
            return self.fail(
                op,
                AgentError::Stale {
                    what: "that question".into(),
                },
            );
        }
        if answers.is_empty() {
            return self.fail(
                op,
                AgentError::Provider {
                    message: "no answer given".into(),
                },
            );
        }
        self.finish(op, OperationOutcome::Succeeded(OperationOutput::Ack));
        let turn = token.turn.clone();
        self.emit(
            None,
            Some(session),
            Some(turn),
            AgentEvent::QuestionResolved { token },
        );
        self.end_turn(session, TurnOutcome::Completed);
    }
}

impl AgentBackend for FakeBackend {
    fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    fn try_submit(&mut self, request: Request) -> Result<OperationId, SubmitError> {
        if self.shutdown {
            return Err(SubmitError::ShuttingDown);
        }
        if self.pending.len() >= QUEUE_LIMIT {
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
        self.accepted.push(id);
        self.pending.push_back((id, request.operation));
        Ok(id)
    }

    fn poll_events(&mut self, budget: PollBudget) -> Vec<EventEnvelope> {
        while let Some((op, operation)) = self.pending.pop_front() {
            self.run(op, operation);
        }
        let take = budget.max_events.min(self.out.len());
        self.out.drain(..take).collect()
    }

    fn begin_shutdown(&mut self, _deadline: Duration) -> Result<OperationId, SubmitError> {
        if self.shutdown {
            return Err(SubmitError::ShuttingDown);
        }
        self.shutdown = true;
        let id = OperationId(self.next_operation);
        self.next_operation += 1;
        self.accepted.push(id);
        // Pending requests are settled and active turns interrupted before the end.
        let sessions: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.active.is_some())
            .map(|(id, _)| *id)
            .collect();
        for session in sessions {
            let request = self.sessions[&session]
                .active
                .as_ref()
                .and_then(|a| match &a.kind {
                    TurnKind::Approval(t) => Some((t.clone(), true)),
                    TurnKind::Question(t) => Some((t.clone(), false)),
                    TurnKind::Slow => None,
                });
            if let Some((token, approval)) = request {
                let event = if approval {
                    AgentEvent::ApprovalResolved {
                        token: token.clone(),
                    }
                } else {
                    AgentEvent::QuestionResolved {
                        token: token.clone(),
                    }
                };
                self.emit(None, Some(session), Some(token.turn), event);
            }
            self.end_turn(session, TurnOutcome::Interrupted);
        }
        self.finish(id, OperationOutcome::Succeeded(OperationOutput::Ack));
        Ok(id)
    }
}

// ----------------------------------------------------------------- fake child

/// Reacts to a line written to a fake child (typically by queueing a reply).
pub type Responder = Arc<dyn Fn(&str, &FakeChildHandle) + Send + Sync>;

/// A scripted child process for provider-crate tests: queue stdout lines, read
/// what the library wrote, and decide when it "exits".
#[derive(Clone, Default)]
pub struct FakeChildHandle {
    pub state: Arc<Mutex<FakeChildState>>,
}

#[derive(Default)]
pub struct FakeChildState {
    /// Called with each complete line the library writes, to script a server.
    pub responder: Option<Responder>,
    partial: String,
    pub queued: VecDeque<ChildEvent>,
    pub written: Vec<u8>,
    pub exit: Option<ExitInfo>,
    pub stop_requests: usize,
    pub force_stops: usize,
    pub stdin_closed: bool,
    pub dropped: bool,
}

impl FakeChildHandle {
    pub fn stdout_line(&self, line: &str) {
        let mut state = self.state.lock().unwrap();
        state
            .queued
            .push_back(ChildEvent::Stdout(format!("{line}\n").into_bytes()));
    }

    pub fn exit(&self, code: i32) {
        let mut state = self.state.lock().unwrap();
        state.exit = Some(ExitInfo {
            code: Some(code),
            signal: None,
        });
        state.queued.push_back(ChildEvent::StdoutClosed);
        state.queued.push_back(ChildEvent::StderrClosed);
    }

    /// What the library wrote to the child, as text.
    pub fn written_text(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().unwrap().written).into_owned()
    }
}

struct FakeChild(FakeChildHandle);

impl Child for FakeChild {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (responder, lines) = {
            let mut state = self.0.state.lock().unwrap();
            if state.exit.is_some() {
                return Err("the process has exited".into());
            }
            state.written.extend_from_slice(bytes);
            state.partial.push_str(&String::from_utf8_lossy(bytes));
            let mut lines = Vec::new();
            while let Some(newline) = state.partial.find('\n') {
                let line: String = state.partial.drain(..=newline).collect();
                lines.push(line.trim_end().to_string());
            }
            (state.responder.clone(), lines)
        };
        if let Some(responder) = responder {
            for line in lines {
                responder(&line, &self.0);
            }
        }
        Ok(())
    }
    fn close_stdin(&mut self) {
        self.0.state.lock().unwrap().stdin_closed = true;
    }
    fn drain(&mut self, max_bytes: usize) -> Vec<ChildEvent> {
        let mut state = self.0.state.lock().unwrap();
        let mut out = Vec::new();
        let mut bytes = 0;
        while let Some(event) = state.queued.pop_front() {
            if let ChildEvent::Stdout(data) | ChildEvent::Stderr(data) = &event {
                bytes += data.len();
            }
            out.push(event);
            if bytes >= max_bytes {
                break;
            }
        }
        out
    }
    fn try_exit(&mut self) -> Result<Option<ExitInfo>, String> {
        Ok(self.0.state.lock().unwrap().exit)
    }
    fn request_stop(&mut self) -> bool {
        let mut state = self.0.state.lock().unwrap();
        state.stop_requests += 1;
        if state.exit.is_none() {
            state.exit = Some(ExitInfo {
                code: None,
                signal: Some(15),
            });
            state.queued.push_back(ChildEvent::StdoutClosed);
            state.queued.push_back(ChildEvent::StderrClosed);
        }
        true
    }
    fn force_stop(&mut self) -> bool {
        let mut state = self.0.state.lock().unwrap();
        state.force_stops += 1;
        if state.exit.is_none() {
            state.exit = Some(ExitInfo {
                code: None,
                signal: Some(9),
            });
        }
        true
    }
}

impl Drop for FakeChild {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().dropped = true;
    }
}

/// Launches `FakeChild`ren and records what it was asked to start.
#[derive(Clone, Default)]
pub struct FakeLauncher {
    /// Installed on every child this launcher starts.
    pub responder: Arc<Mutex<Option<Responder>>>,
    pub launched: Arc<Mutex<Vec<ChildSpec>>>,
    pub children: Arc<Mutex<Vec<FakeChildHandle>>>,
    pub fail_with: Arc<Mutex<Option<LaunchError>>>,
}

impl FakeLauncher {
    pub fn child(&self, index: usize) -> FakeChildHandle {
        self.children.lock().unwrap()[index].clone()
    }
}

impl ChildLauncher for FakeLauncher {
    fn launch(&self, spec: &ChildSpec, _wake: Wake) -> Result<Box<dyn Child>, LaunchError> {
        if let Some(error) = self.fail_with.lock().unwrap().clone() {
            return Err(error);
        }
        self.launched.lock().unwrap().push(spec.clone());
        let handle = FakeChildHandle::default();
        handle.state.lock().unwrap().responder = self.responder.lock().unwrap().clone();
        self.children.lock().unwrap().push(handle.clone());
        Ok(Box::new(FakeChild(handle)))
    }
}

// ------------------------------------------------------------------- contract

/// Checks every backend must pass without needing a model: they use only
/// operations that behave the same for any provider, and cannot depend on the
/// fake's magic prompts. Provider crates call these with their own backend wired
/// to a fake launcher.
pub mod contract {
    use super::*;

    pub fn request(id: u64, operation: AgentOperation) -> Request {
        Request {
            client_request_id: ClientRequestId(id),
            operation,
        }
    }

    /// Poll until `done` accepts the collected events or the budget is exhausted.
    pub fn pump(
        backend: &mut dyn AgentBackend,
        mut done: impl FnMut(&[EventEnvelope]) -> bool,
    ) -> Vec<EventEnvelope> {
        let mut all = Vec::new();
        for _ in 0..200 {
            all.extend(backend.poll_events(PollBudget::FRAME));
            if done(&all) {
                return all;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("backend did not produce the expected events; got {all:#?}");
    }

    pub fn outcome_of(events: &[EventEnvelope], op: OperationId) -> Option<&OperationOutcome> {
        events.iter().find_map(|e| match (&e.event, e.operation) {
            (AgentEvent::OperationFinished(outcome), Some(o)) if o == op => Some(outcome),
            _ => None,
        })
    }

    /// Every accepted operation ends exactly once, even after shutdown.
    pub fn every_operation_ends_exactly_once(
        backend: &mut dyn AgentBackend,
        ops: &[OperationId],
        events: &[EventEnvelope],
    ) {
        for op in ops {
            let count = events
                .iter()
                .filter(|e| {
                    e.operation == Some(*op) && matches!(e.event, AgentEvent::OperationFinished(_))
                })
                .count();
            assert_eq!(count, 1, "operation {op} finished {count} times");
        }
        let _ = backend.poll_events(PollBudget::FRAME);
    }

    /// Duplicate ids are rejected; sequences strictly increase within a generation.
    pub fn basic(backend: &mut dyn AgentBackend) {
        let first = backend
            .try_submit(request(1, AgentOperation::Probe))
            .expect("probe accepted");
        assert_eq!(
            backend.try_submit(request(1, AgentOperation::Probe)),
            Err(SubmitError::Duplicate)
        );
        let second = backend
            .try_submit(request(2, AgentOperation::ReadAuthState))
            .expect("auth accepted");
        let events = pump(backend, |e| {
            outcome_of(e, first).is_some() && outcome_of(e, second).is_some()
        });
        assert!(matches!(
            outcome_of(&events, first),
            Some(OperationOutcome::Succeeded(OperationOutput::Probed { .. }))
        ));
        let mut last: HashMap<RuntimeGeneration, u64> = HashMap::new();
        for event in &events {
            let previous = last.insert(event.generation, event.sequence).unwrap_or(0);
            assert!(
                event.sequence > previous,
                "sequence must strictly increase within a generation"
            );
        }
        every_operation_ends_exactly_once(backend, &[first, second], &events);
    }

    /// Unknown sessions and stale targets fail with typed errors, never panic or hang.
    pub fn unknown_targets_fail_typed(backend: &mut dyn AgentBackend, provider: ProviderId) {
        let ghost = SessionId(9999);
        let a = backend
            .try_submit(request(10, AgentOperation::ReadConfig { session: ghost }))
            .unwrap();
        let b = backend
            .try_submit(request(
                11,
                AgentOperation::InterruptTurn {
                    session: ghost,
                    turn: TurnId("nope".into()),
                },
            ))
            .unwrap();
        let stale = RequestToken {
            provider,
            profile: ProfileId::default_profile(),
            session: ghost,
            turn: TurnId("nope".into()),
            generation: RuntimeGeneration(1),
            native: "x".into(),
        };
        let c = backend
            .try_submit(request(
                12,
                AgentOperation::ReplyApproval {
                    token: stale,
                    decision: ApprovalDecisionId("accept".into()),
                },
            ))
            .unwrap();
        let events = pump(backend, |e| {
            [a, b, c].iter().all(|op| outcome_of(e, *op).is_some())
        });
        for op in [a, b, c] {
            assert!(
                matches!(outcome_of(&events, op), Some(OperationOutcome::Failed(_))),
                "operation {op}"
            );
        }
        assert!(matches!(
            outcome_of(&events, a),
            Some(OperationOutcome::Failed(AgentError::NotFound { .. }))
        ));
        assert!(matches!(
            outcome_of(&events, c),
            Some(OperationOutcome::Failed(AgentError::Stale { .. }))
        ));
    }

    /// Nothing is accepted after shutdown begins, and shutdown itself ends once.
    pub fn shutdown_is_final(backend: &mut dyn AgentBackend) {
        let shutdown = backend
            .begin_shutdown(Duration::from_secs(1))
            .expect("shutdown accepted");
        assert_eq!(
            backend.try_submit(request(100, AgentOperation::Probe)),
            Err(SubmitError::ShuttingDown)
        );
        assert_eq!(
            backend.begin_shutdown(Duration::from_secs(1)),
            Err(SubmitError::ShuttingDown)
        );
        let events = pump(backend, |e| outcome_of(e, shutdown).is_some());
        assert!(matches!(
            outcome_of(&events, shutdown),
            Some(OperationOutcome::Succeeded(_))
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::contract::*;
    use super::*;
    use crate::{
        ItemId,
        model::{Applied, SessionModel, TurnState},
    };

    fn fake() -> FakeBackend {
        FakeBackend::new(ProviderId::Codex)
    }

    #[test]
    fn the_fake_passes_the_shared_contract() {
        basic(&mut fake());
        unknown_targets_fail_typed(&mut fake(), ProviderId::Codex);
        shutdown_is_final(&mut fake());
    }

    fn create(backend: &mut FakeBackend, id: u64, session: u64) -> Vec<EventEnvelope> {
        let op = backend
            .try_submit(request(
                id,
                AgentOperation::CreateSession {
                    session: SessionId(session),
                    workspace: "/w".into(),
                    config: Default::default(),
                },
            ))
            .unwrap();
        pump(backend, |e| outcome_of(e, op).is_some())
    }

    fn start(
        backend: &mut FakeBackend,
        id: u64,
        session: u64,
        text: &str,
    ) -> (OperationId, Vec<EventEnvelope>) {
        let op = backend
            .try_submit(request(
                id,
                AgentOperation::StartTurn {
                    session: SessionId(session),
                    client_turn: crate::api::ClientTurnId(format!("c{id}")),
                    input: vec![InputBlock::Text(text.into())],
                    expected_revision: None,
                },
            ))
            .unwrap();
        let events = pump(backend, |e| outcome_of(e, op).is_some());
        (op, events)
    }

    fn fold(model: &mut SessionModel, events: &[EventEnvelope]) {
        for event in events {
            if event.session == Some(SessionId(1)) {
                model.apply(event);
            }
        }
    }

    #[test]
    fn an_echo_turn_completes_through_the_reducer_and_a_second_start_while_busy_is_refused() {
        let mut b = fake();
        create(&mut b, 1, 1);
        let mut model = SessionModel::default();
        let (_, events) = start(&mut b, 2, 1, "hello");
        fold(&mut model, &events);
        assert!(model.can_start_turn() && model.last_outcome == Some(TurnOutcome::Completed));
        assert!(
            matches!(&model.items[0].kind, crate::model::ItemKind::Agent { text, .. } if text == "echo: hello")
        );
        let (_, _) = start(&mut b, 3, 1, "slow: wait");
        let busy = b
            .try_submit(request(
                4,
                AgentOperation::StartTurn {
                    session: SessionId(1),
                    client_turn: crate::api::ClientTurnId("c4".into()),
                    input: vec![InputBlock::Text("again".into())],
                    expected_revision: None,
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, busy).is_some());
        assert_eq!(
            outcome_of(&events, busy),
            Some(&OperationOutcome::Failed(AgentError::Busy))
        );
    }

    #[test]
    fn interrupt_ends_a_slow_turn_as_interrupted_and_a_stale_interrupt_is_refused() {
        let mut b = fake();
        create(&mut b, 1, 1);
        let mut model = SessionModel::default();
        let (_, events) = start(&mut b, 2, 1, "slow: x");
        fold(&mut model, &events);
        assert!(!model.can_start_turn());
        let TurnState::Active {
            turn: Some(turn), ..
        } = model.state.clone()
        else {
            panic!()
        };
        let wrong = b
            .try_submit(request(
                3,
                AgentOperation::InterruptTurn {
                    session: SessionId(1),
                    turn: TurnId("other".into()),
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, wrong).is_some());
        assert!(matches!(
            outcome_of(&events, wrong),
            Some(OperationOutcome::Failed(AgentError::Stale { .. }))
        ));
        let ok = b
            .try_submit(request(
                4,
                AgentOperation::InterruptTurn {
                    session: SessionId(1),
                    turn,
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| {
            outcome_of(e, ok).is_some()
                && e.iter()
                    .any(|x| matches!(x.event, AgentEvent::TurnFinished(_)))
        });
        fold(&mut model, &events);
        assert_eq!(model.last_outcome, Some(TurnOutcome::Interrupted));
        assert!(model.can_start_turn());
    }

    #[test]
    fn an_approval_round_trip_resolves_once_and_a_reused_token_is_stale() {
        let mut b = fake();
        create(&mut b, 1, 1);
        let mut model = SessionModel::default();
        let (_, events) = start(&mut b, 2, 1, "approve: touch");
        fold(&mut model, &events);
        assert!(model.needs_attention());
        let token = model.pending_approvals[0].token.clone();
        let reply = b
            .try_submit(request(
                3,
                AgentOperation::ReplyApproval {
                    token: token.clone(),
                    decision: ApprovalDecisionId("accept".into()),
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| {
            outcome_of(e, reply).is_some()
                && e.iter()
                    .any(|x| matches!(x.event, AgentEvent::TurnFinished(_)))
        });
        fold(&mut model, &events);
        assert!(!model.needs_attention() && model.last_outcome == Some(TurnOutcome::Completed));
        let tool = model.item(&ItemId("turn-1-1".into())).unwrap();
        assert!(
            matches!(&tool.kind, crate::model::ItemKind::Tool(c) if c.status == ToolStatus::Completed)
        );
        let again = b
            .try_submit(request(
                4,
                AgentOperation::ReplyApproval {
                    token,
                    decision: ApprovalDecisionId("accept".into()),
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, again).is_some());
        assert!(matches!(
            outcome_of(&events, again),
            Some(OperationOutcome::Failed(AgentError::Stale { .. }))
        ));
    }

    #[test]
    fn two_sessions_are_independent_and_a_question_in_one_does_not_resolve_the_other() {
        let mut b = fake();
        create(&mut b, 1, 1);
        create(&mut b, 2, 2);
        let (_, e1) = start(&mut b, 3, 1, "ask: pick");
        let (_, e2) = start(&mut b, 4, 2, "approve: touch");
        let mut m1 = SessionModel::default();
        let mut m2 = SessionModel::default();
        for e in e1.iter().chain(&e2) {
            match e.session {
                Some(SessionId(1)) => {
                    m1.apply(e);
                }
                Some(SessionId(2)) => {
                    m2.apply(e);
                }
                _ => {}
            }
        }
        assert_eq!(
            (m1.pending_questions.len(), m1.pending_approvals.len()),
            (1, 0)
        );
        assert_eq!(
            (m2.pending_questions.len(), m2.pending_approvals.len()),
            (0, 1)
        );
        // Answering session 1's question with session 2's token is refused.
        let wrong = m2.pending_approvals[0].token.clone();
        let op = b
            .try_submit(request(
                5,
                AgentOperation::ReplyQuestion {
                    token: wrong,
                    answers: vec![QuestionAnswer::Choice("a".into())],
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, op).is_some());
        assert!(matches!(
            outcome_of(&events, op),
            Some(OperationOutcome::Failed(AgentError::Stale { .. }))
        ));
        let token = m1.pending_questions[0].token.clone();
        let op = b
            .try_submit(request(
                6,
                AgentOperation::ReplyQuestion {
                    token,
                    answers: vec![QuestionAnswer::Choice("a".into())],
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, op).is_some());
        assert!(matches!(
            outcome_of(&events, op),
            Some(OperationOutcome::Succeeded(_))
        ));
        for e in &events {
            if e.session == Some(SessionId(1)) {
                m1.apply(e);
            }
            if e.session == Some(SessionId(2)) {
                m2.apply(e);
            }
        }
        assert!(m1.can_start_turn());
        assert!(
            !m2.can_start_turn() && m2.needs_attention(),
            "the other session is untouched"
        );
    }

    #[test]
    fn a_crash_bumps_the_generation_leaves_the_turn_unknown_and_old_tokens_go_stale() {
        let mut b = fake();
        create(&mut b, 1, 1);
        let mut model = SessionModel::default();
        let (_, events) = start(&mut b, 2, 1, "crash: now");
        fold(&mut model, &events);
        assert_eq!(model.state, TurnState::Unknown);
        let (_, events) = start(&mut b, 3, 1, "after the crash");
        assert!(
            events.iter().all(|e| e.generation == RuntimeGeneration(2)),
            "new events use the new generation"
        );
        assert!(events.iter().all(|e| e.sequence >= 1));
        fold(&mut model, &events);
        assert_eq!(model.last_outcome, Some(TurnOutcome::Completed));
        // Late events from the old generation are ignored.
        let late = EventEnvelope {
            generation: RuntimeGeneration(1),
            sequence: 99,
            ..events[0].clone()
        };
        assert_eq!(model.apply(&late), Applied::StaleGeneration);
    }

    #[test]
    fn config_changes_check_the_revision_and_values() {
        let mut b = fake();
        create(&mut b, 1, 1);
        let set = |b: &mut FakeBackend, id, rev, value: &str| {
            let op = b
                .try_submit(request(
                    id,
                    AgentOperation::SetConfig {
                        session: SessionId(1),
                        expected_revision: ConfigRevision(rev),
                        patch: crate::api::ConfigPatch {
                            values: vec![("model".into(), value.into())],
                            ..Default::default()
                        },
                    },
                ))
                .unwrap();
            let events = pump(b, |e| outcome_of(e, op).is_some());
            outcome_of(&events, op).cloned().unwrap()
        };
        assert!(
            matches!(set(&mut b, 2, 1, "smart"), OperationOutcome::Succeeded(OperationOutput::Config(c)) if c.revision == ConfigRevision(2))
        );
        assert!(
            matches!(
                set(&mut b, 3, 1, "fast"),
                OperationOutcome::Failed(AgentError::Stale { .. })
            ),
            "an old revision is refused"
        );
        assert!(matches!(
            set(&mut b, 4, 2, "bogus"),
            OperationOutcome::Failed(AgentError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn shutdown_settles_pending_requests_and_interrupts_active_turns() {
        let mut b = fake();
        let mut all = create(&mut b, 1, 1);
        let mut model = SessionModel::default();
        let (_, events) = start(&mut b, 2, 1, "approve: touch");
        fold(&mut model, &events);
        all.extend(events);
        assert!(model.needs_attention());
        let shutdown = b.begin_shutdown(Duration::from_secs(1)).unwrap();
        let events = pump(&mut b, |e| outcome_of(e, shutdown).is_some());
        fold(&mut model, &events);
        all.extend(events);
        assert!(!model.needs_attention());
        assert_eq!(model.last_outcome, Some(TurnOutcome::Interrupted));
        let accepted = b.accepted.clone();
        every_operation_ends_exactly_once(&mut b, &accepted, &all);
    }

    #[test]
    fn queue_saturation_is_reported_not_dropped_and_empty_messages_are_invalid() {
        let mut b = fake();
        for n in 0..QUEUE_LIMIT as u64 {
            b.try_submit(request(1000 + n, AgentOperation::Probe))
                .unwrap();
        }
        assert_eq!(
            b.try_submit(request(5000, AgentOperation::Probe)),
            Err(SubmitError::QueueFull)
        );
        let events = pump(&mut b, |e| e.len() >= QUEUE_LIMIT);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.event, AgentEvent::OperationFinished(_)))
                .count(),
            QUEUE_LIMIT
        );
        assert!(matches!(
            b.try_submit(request(
                5001,
                AgentOperation::StartTurn {
                    session: SessionId(1),
                    client_turn: crate::api::ClientTurnId("c".into()),
                    input: vec![],
                    expected_revision: None,
                }
            )),
            Err(SubmitError::Invalid(_))
        ));
    }

    #[test]
    fn list_attach_rename_and_archive_work_and_attaching_a_foreign_native_ref_fails() {
        let mut b = fake();
        let events = create(&mut b, 1, 1);
        let native = events
            .iter()
            .find_map(|e| match &e.event {
                AgentEvent::OperationFinished(OperationOutcome::Succeeded(
                    OperationOutput::SessionReady { native, .. },
                )) => Some(native.clone()),
                _ => None,
            })
            .unwrap();
        let list = b
            .try_submit(request(
                2,
                AgentOperation::ListSessions {
                    workspace: Some("/w".into()),
                    cursor: None,
                    limit: 10,
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, list).is_some());
        assert!(
            matches!(outcome_of(&events, list), Some(OperationOutcome::Succeeded(OperationOutput::Sessions { summaries, complete: true, .. })) if summaries.len() == 1)
        );
        let rename = b
            .try_submit(request(
                3,
                AgentOperation::UpdateSessionMetadata {
                    session: SessionId(1),
                    title: "Hi".into(),
                },
            ))
            .unwrap();
        pump(&mut b, |e| outcome_of(e, rename).is_some());
        let attach = b
            .try_submit(request(
                4,
                AgentOperation::AttachSession {
                    session: SessionId(5),
                    native: native.clone(),
                    workspace: "/w".into(),
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, attach).is_some());
        assert!(matches!(
            outcome_of(&events, attach),
            Some(OperationOutcome::Succeeded(
                OperationOutput::SessionReady { .. }
            ))
        ));
        let foreign = NativeSessionRef {
            provider: ProviderId::Claude,
            ..native
        };
        let bad = b
            .try_submit(request(
                5,
                AgentOperation::AttachSession {
                    session: SessionId(6),
                    native: foreign,
                    workspace: "/w".into(),
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, bad).is_some());
        assert!(matches!(
            outcome_of(&events, bad),
            Some(OperationOutcome::Failed(AgentError::NotFound { .. }))
        ));
        let archive = b
            .try_submit(request(
                6,
                AgentOperation::ArchiveSession {
                    session: SessionId(5),
                },
            ))
            .unwrap();
        let events = pump(&mut b, |e| outcome_of(e, archive).is_some());
        assert!(matches!(
            outcome_of(&events, archive),
            Some(OperationOutcome::Succeeded(_))
        ));
    }

    #[test]
    fn the_fake_child_records_writes_scripts_output_and_stops() {
        let launcher = FakeLauncher::default();
        let spec = ChildSpec {
            program: "claude".into(),
            args: vec![],
            env: vec![],
            env_remove: vec![],
            cwd: "/w".into(),
        };
        let mut child = launcher.launch(&spec, Arc::new(|| {})).unwrap();
        let handle = launcher.child(0);
        child.write(b"hi\n").unwrap();
        handle.stdout_line("{\"a\":1}");
        assert_eq!(handle.written_text(), "hi\n");
        assert_eq!(
            child.drain(usize::MAX),
            vec![ChildEvent::Stdout(b"{\"a\":1}\n".to_vec())]
        );
        assert!(child.request_stop());
        assert_eq!(
            child.try_exit().unwrap(),
            Some(ExitInfo {
                code: None,
                signal: Some(15)
            })
        );
        assert!(child.write(b"late").is_err());
        drop(child);
        assert!(handle.state.lock().unwrap().dropped);
        *launcher.fail_with.lock().unwrap() = Some(LaunchError::NotFound("claude".into()));
        assert!(launcher.launch(&spec, Arc::new(|| {})).is_err());
    }
}
