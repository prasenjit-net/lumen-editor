//! The native-agent controller: owns the provider backends, the session index and
//! the turn scheduler, and turns user actions into backend operations.
//!
//! Everything is polled from the UI thread: [`AgentHub::tick`] drains backend
//! events into each session's reducer, settles operations and starts whatever the
//! scheduler now allows. Backends never queue turns - the hub keeps each session's
//! queue - and a backend only ever sees one active turn per session.
// Replaces the ACP controller in `agent.rs` once the workbench switches over.
#![allow(dead_code)]
use crate::model::{
    agent_registry::Registry,
    agent_scheduler::{Dispatch, Phase, Scheduler, Ticket, WaitReason},
};
use lumen_agent_core::{
    AgentError, ClientRequestId, ItemId, OperationId, PollBudget, ProviderId, RequestToken,
    SessionId, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, ClientTurnId, ConfigPatch, InputBlock,
        OperationOutcome, OperationOutput, QuestionAnswer, Request, SessionConfig,
    },
    event::{AgentEvent, EventEnvelope, TurnOutcome},
    model::{SessionModel, TurnState},
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    time::Duration,
};

/// What a session list row says about a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SessionStatus {
    Idle,
    Working,
    /// Waiting for an approval or an answer from the user.
    NeedsAttention,
    /// Held back by the scheduler; the reason is shown to the user.
    Waiting(WaitReason),
    /// The last turn failed or the runtime was lost; the message explains.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QueuedPrompt {
    pub(crate) ticket: Ticket,
    pub(crate) input: Vec<InputBlock>,
}

/// Why `send` refused a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SendError {
    UnknownSession,
    Archived,
    EmptyMessage,
    ProviderUnavailable,
}

/// Per-session runtime state; the persistent facts live in the registry.
pub(crate) struct Live {
    pub(crate) model: SessionModel,
    /// The backend knows this session (created or attached).
    ready: bool,
    preparing: bool,
    /// Messages not yet finished, oldest first; the head is the current turn once
    /// it has been dispatched.
    pub(crate) prompts: VecDeque<QueuedPrompt>,
    current: Option<Ticket>,
    turn: Option<TurnId>,
    /// An answer waiting for a slot before it is sent to the provider.
    held_reply: Option<HeldReply>,
    pub(crate) error: Option<String>,
    /// Settings changes sent to the provider and not yet answered. A turn waits
    /// for them so it starts with the revision the user last saw.
    config_in_flight: usize,
    /// Changes waiting for the previous one to be answered (each needs the
    /// revision the last produced).
    config_queue: VecDeque<ConfigPatch>,
    /// `StartTurn` has been sent for the current prompt.
    starting: bool,
}

enum HeldReply {
    Approval(RequestToken, ApprovalDecisionId),
    Question(RequestToken, Vec<QuestionAnswer>),
}

impl Live {
    fn new() -> Self {
        Self {
            model: SessionModel::default(),
            ready: false,
            preparing: false,
            prompts: VecDeque::new(),
            current: None,
            turn: None,
            held_reply: None,
            error: None,
            config_in_flight: 0,
            config_queue: VecDeque::new(),
            starting: false,
        }
    }
}

enum Pending {
    /// Create or attach, after which the head prompt starts.
    Prepare(SessionId),
    Start(SessionId),
    Interrupt,
    Reply,
    /// A settings change; a refusal is shown on the session.
    Config(SessionId),
    Other,
}

pub(crate) struct AgentHub {
    backends: HashMap<ProviderId, Box<dyn AgentBackend>>,
    pub(crate) registry: Registry,
    scheduler: Scheduler,
    live: HashMap<SessionId, Live>,
    ops: HashMap<(ProviderId, OperationId), Pending>,
    next_client_id: u64,
    now: Box<dyn Fn() -> u64 + Send>,
    /// Set when the registry changed and should be saved.
    pub(crate) registry_dirty: bool,
}

fn system_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl AgentHub {
    pub(crate) fn new(
        backends: Vec<(ProviderId, Box<dyn AgentBackend>)>,
        registry: Registry,
    ) -> Self {
        Self {
            backends: backends.into_iter().collect(),
            registry,
            scheduler: Scheduler::default(),
            live: HashMap::new(),
            ops: HashMap::new(),
            next_client_id: 1,
            now: Box::new(system_seconds),
            registry_dirty: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_clock(mut self, now: impl Fn() -> u64 + Send + 'static) -> Self {
        self.now = Box::new(now);
        self
    }

    pub(crate) fn set_slots(&mut self, slots: usize) {
        self.scheduler.set_slots(slots);
    }

    pub(crate) fn live(&self, session: SessionId) -> Option<&Live> {
        self.live.get(&session)
    }

    fn live_mut(&mut self, session: SessionId) -> &mut Live {
        self.live.entry(session).or_insert_with(Live::new)
    }

    // -------------------------------------------------------------- actions

    pub(crate) fn create_session(&mut self, provider: ProviderId, workspace: PathBuf) -> SessionId {
        let profile = self
            .backends
            .get(&provider)
            .map(|b| b.descriptor().profile.clone())
            .unwrap_or_else(lumen_agent_core::ProfileId::default_profile);
        let id = self
            .registry
            .create(provider, profile, workspace, (self.now)());
        self.live.insert(id, Live::new());
        self.registry_dirty = true;
        self.prepare(id, provider);
        id
    }

    /// Make sure a listed session is attached (used when its chat is opened).
    pub(crate) fn ensure_ready(&mut self, session: SessionId) {
        if let Some(provider) = self.registry.get(session).map(|r| r.provider) {
            self.prepare(session, provider);
        }
    }

    /// Queue a message. It starts as soon as the session is free and the scheduler
    /// has a slot (and the folder lease, when the turn may write).
    pub(crate) fn send(
        &mut self,
        session: SessionId,
        input: Vec<InputBlock>,
    ) -> Result<Ticket, SendError> {
        let record = self
            .registry
            .get(session)
            .ok_or(SendError::UnknownSession)?;
        if record.archived {
            return Err(SendError::Archived);
        }
        if input.is_empty() {
            return Err(SendError::EmptyMessage);
        }
        if !self.backends.contains_key(&record.provider) {
            return Err(SendError::ProviderUnavailable);
        }
        let workspace = record.workspace.clone();
        let title_hint = input.iter().find_map(|b| match b {
            InputBlock::Text(text) => Some(text.clone()),
            _ => None,
        });
        // Unknown execution modes count as potentially writing.
        let ticket = self.scheduler.submit(session, workspace, true);
        let live = self.live_mut(session);
        live.error = None;
        live.prompts.push_back(QueuedPrompt { ticket, input });
        if let Some(text) = title_hint {
            self.registry.suggest_title(session, &text);
            self.registry_dirty = true;
        }
        self.run_scheduler();
        Ok(ticket)
    }

    /// Drop a message that has not been dispatched yet.
    pub(crate) fn cancel_queued(&mut self, session: SessionId, ticket: Ticket) -> bool {
        let Some(live) = self.live.get_mut(&session) else {
            return false;
        };
        if live.current == Some(ticket) || self.scheduler.phase(ticket) != Some(Phase::Queued) {
            return false;
        }
        live.prompts.retain(|p| p.ticket != ticket);
        self.scheduler.finish(ticket);
        true
    }

    pub(crate) fn interrupt(&mut self, session: SessionId) -> bool {
        let Some(live) = self.live.get(&session) else {
            return false;
        };
        let Some(turn) = live.turn.clone() else {
            return false;
        };
        let Some(provider) = self.registry.get(session).map(|r| r.provider) else {
            return false;
        };
        self.submit(
            provider,
            AgentOperation::InterruptTurn { session, turn },
            Pending::Interrupt,
        );
        true
    }

    /// Answer an approval. The reply goes out once the turn has a slot again.
    pub(crate) fn reply_approval(
        &mut self,
        session: SessionId,
        token: RequestToken,
        decision: ApprovalDecisionId,
    ) {
        self.hold_reply(session, HeldReply::Approval(token, decision));
    }

    pub(crate) fn reply_question(
        &mut self,
        session: SessionId,
        token: RequestToken,
        answers: Vec<QuestionAnswer>,
    ) {
        self.hold_reply(session, HeldReply::Question(token, answers));
    }

    fn hold_reply(&mut self, session: SessionId, reply: HeldReply) {
        let Some(live) = self.live.get_mut(&session) else {
            return;
        };
        let Some(ticket) = live.current else {
            return;
        };
        live.held_reply = Some(reply);
        self.scheduler.user_answered(ticket);
        self.run_scheduler();
    }

    /// Queue a settings change. Changes go to the provider one at a time, each
    /// against the revision the previous one produced, so quick edits in a row
    /// all apply instead of colliding.
    pub(crate) fn set_config(&mut self, session: SessionId, patch: ConfigPatch) -> bool {
        let Some(live) = self.live.get_mut(&session).filter(|l| l.ready) else {
            return false;
        };
        if patch.is_empty() {
            return false;
        }
        live.config_queue.push_back(patch);
        self.pump_config(session);
        true
    }

    fn pump_config(&mut self, session: SessionId) {
        let Some(provider) = self.registry.get(session).map(|r| r.provider) else {
            return;
        };
        let Some(live) = self.live.get_mut(&session) else {
            return;
        };
        if live.config_in_flight > 0 {
            return;
        }
        let Some(revision) = live.model.config.as_ref().map(|c| c.revision) else {
            return;
        };
        let Some(patch) = live.config_queue.pop_front() else {
            return;
        };
        live.config_in_flight = 1;
        let operation = AgentOperation::SetConfig {
            session,
            expected_revision: revision,
            patch,
        };
        if !self.submit(provider, operation, Pending::Config(session))
            && let Some(live) = self.live.get_mut(&session)
        {
            live.config_in_flight = 0;
        }
    }

    /// Re-apply the user's saved choices to a resumed session.
    fn restore_prefs(&mut self, session: SessionId) {
        let saved = self
            .registry
            .get(session)
            .map(|r| r.prefs.clone())
            .unwrap_or_default();
        let Some(live) = self.live.get_mut(&session) else {
            return;
        };
        let Some(config) = live.model.config.as_ref() else {
            return;
        };
        let values: Vec<(String, String)> = saved
            .into_iter()
            .filter(|(id, value)| config.get(id).is_some_and(|o| &o.current != value))
            .collect();
        if !values.is_empty() {
            live.config_queue.push_back(ConfigPatch {
                values,
                ..Default::default()
            });
            self.pump_config(session);
        }
    }

    /// Remember the choices that differ from the provider's defaults.
    fn remember_prefs(&mut self, session: SessionId, config: &SessionConfig) {
        let prefs: Vec<(String, String)> = config
            .options
            .iter()
            .filter(|o| o.default.as_ref() != Some(&o.current))
            .map(|o| (o.id.clone(), o.current.clone()))
            .collect();
        self.registry.set_prefs(session, prefs);
        self.registry_dirty = true;
    }

    pub(crate) fn archive(&mut self, session: SessionId) {
        if self.status(session) == SessionStatus::Working {
            return;
        }
        for ticket in self.scheduler.drop_session(session) {
            let _ = ticket;
        }
        if let Some(live) = self.live.get_mut(&session) {
            live.prompts.clear();
            live.current = None;
        }
        self.registry.set_archived(session, true);
        self.registry_dirty = true;
    }

    // ---------------------------------------------------------------- status

    pub(crate) fn status(&self, session: SessionId) -> SessionStatus {
        let Some(live) = self.live.get(&session) else {
            return SessionStatus::Idle;
        };
        if live.model.needs_attention() {
            return SessionStatus::NeedsAttention;
        }
        for prompt in &live.prompts {
            match self.scheduler.phase(prompt.ticket) {
                Some(Phase::Executing | Phase::Finishing) => return SessionStatus::Working,
                Some(Phase::WaitingForUser) => return SessionStatus::NeedsAttention,
                Some(Phase::Resuming | Phase::Queued) => {
                    return match self.scheduler.wait_reason(prompt.ticket) {
                        Some(reason) => SessionStatus::Waiting(reason),
                        None => SessionStatus::Working,
                    };
                }
                None => {}
            }
        }
        if live.preparing && live.current.is_some() {
            return SessionStatus::Working;
        }
        if let Some(error) = &live.error {
            return SessionStatus::Failed(error.clone());
        }
        if live.model.state == TurnState::Unknown {
            return SessionStatus::Failed("The agent stopped unexpectedly".into());
        }
        SessionStatus::Idle
    }

    pub(crate) fn wants_repaint(&self) -> bool {
        !self.ops.is_empty() || !self.scheduler.is_idle()
    }

    // ------------------------------------------------------------- plumbing

    fn submit(
        &mut self,
        provider: ProviderId,
        operation: AgentOperation,
        pending: Pending,
    ) -> bool {
        let id = ClientRequestId(self.next_client_id);
        self.next_client_id += 1;
        let Some(backend) = self.backends.get_mut(&provider) else {
            return false;
        };
        match backend.try_submit(Request {
            client_request_id: id,
            operation,
        }) {
            Ok(op) => {
                self.ops.insert((provider, op), pending);
                true
            }
            Err(_) => false,
        }
    }

    /// Start whatever the scheduler now lets run.
    fn run_scheduler(&mut self) {
        for dispatch in self.scheduler.schedule() {
            self.dispatch(dispatch);
        }
    }

    fn dispatch(&mut self, dispatch: Dispatch) {
        let Dispatch {
            session,
            ticket,
            fresh,
        } = dispatch;
        let Some(provider) = self.registry.get(session).map(|r| r.provider) else {
            self.scheduler.finish(ticket);
            return;
        };
        if !fresh {
            let held = self
                .live
                .get_mut(&session)
                .and_then(|l| l.held_reply.take());
            let operation = match held {
                Some(HeldReply::Approval(token, decision)) => {
                    AgentOperation::ReplyApproval { token, decision }
                }
                Some(HeldReply::Question(token, answers)) => {
                    AgentOperation::ReplyQuestion { token, answers }
                }
                None => return,
            };
            self.submit(provider, operation, Pending::Reply);
            return;
        }
        let live = self.live_mut(session);
        live.current = Some(ticket);
        if live.ready {
            self.start_current(session, provider);
            return;
        }
        if live.preparing {
            // The session is being created; its completion starts this turn.
            return;
        }
        self.prepare(session, provider);
    }

    /// Create or attach the provider session so its settings are known and
    /// editable before the first message.
    pub(crate) fn prepare(&mut self, session: SessionId, provider: ProviderId) {
        let live = self.live_mut(session);
        if live.ready || live.preparing {
            return;
        }
        live.preparing = true;
        let Some(record) = self.registry.get(session) else {
            return;
        };
        let (workspace, native) = (record.workspace.clone(), record.native_ref());
        let operation = match native {
            Some(native) => AgentOperation::AttachSession {
                session,
                native,
                workspace,
            },
            None => AgentOperation::CreateSession {
                session,
                workspace,
                config: ConfigPatch::default(),
            },
        };
        if !self.submit(provider, operation, Pending::Prepare(session)) {
            self.fail_turn(session, "The agent could not be reached".into());
        }
    }

    fn start_current(&mut self, session: SessionId, provider: ProviderId) {
        let Some(live) = self.live.get(&session) else {
            return;
        };
        let (Some(ticket), Some(prompt)) = (live.current, live.prompts.front()) else {
            return;
        };
        if live.starting || live.config_in_flight > 0 {
            return;
        }
        let expected_revision = live.model.config.as_ref().map(|c| c.revision);
        debug_assert_eq!(prompt.ticket, ticket);
        let shown: Vec<String> = prompt.input.iter().map(describe_block).collect();
        let (item, shown) = (ItemId(format!("user-{}", ticket.0)), shown.join("\n"));
        let operation = AgentOperation::StartTurn {
            session,
            client_turn: ClientTurnId(format!("t{}", ticket.0)),
            input: prompt.input.clone(),
            expected_revision,
        };
        if let Some(live) = self.live.get_mut(&session) {
            live.model.record_user_message(item, shown);
            live.starting = true;
        }
        if !self.submit(provider, operation, Pending::Start(session)) {
            self.fail_turn(session, "The agent could not be reached".into());
        }
    }

    /// End the current turn with an error and let the next one run.
    fn fail_turn(&mut self, session: SessionId, message: String) {
        self.end_turn(session, Some(message), "failed");
    }

    fn end_turn(&mut self, session: SessionId, error: Option<String>, outcome: &str) {
        let now = (self.now)();
        if let Some(live) = self.live.get_mut(&session) {
            if let Some(ticket) = live.current.take() {
                self.scheduler.finish(ticket);
                live.prompts.retain(|p| p.ticket != ticket);
            }
            live.turn = None;
            live.starting = false;
            live.preparing = false;
            live.held_reply = None;
            live.error = error;
        }
        self.registry.touch(session, now, Some(outcome));
        self.registry_dirty = true;
        self.run_scheduler();
    }

    /// Drain every backend and settle what arrived. Call once per frame.
    pub(crate) fn tick(&mut self) {
        let providers: Vec<ProviderId> = self.backends.keys().copied().collect();
        for provider in providers {
            let events = match self.backends.get_mut(&provider) {
                Some(backend) => backend.poll_events(PollBudget::FRAME),
                None => continue,
            };
            for event in events {
                self.handle(provider, event);
            }
        }
        self.run_scheduler();
    }

    fn handle(&mut self, provider: ProviderId, envelope: EventEnvelope) {
        if let Some(session) = envelope.session
            && let Some(live) = self.live.get_mut(&session)
        {
            live.model.apply(&envelope);
        }
        let session = envelope.session;
        match &envelope.event {
            AgentEvent::OperationFinished(outcome) => {
                if let Some(op) = envelope.operation
                    && let Some(pending) = self.ops.remove(&(provider, op))
                {
                    self.settle(pending, outcome);
                }
            }
            AgentEvent::ApprovalRequested(_) | AgentEvent::QuestionRequested(_) => {
                if let Some(ticket) = session
                    .and_then(|s| self.live.get(&s))
                    .and_then(|l| l.current)
                {
                    self.scheduler.waiting_for_user(ticket);
                }
            }
            AgentEvent::TurnFinished(outcome) => {
                if let Some(session) = session {
                    let (error, label) = match outcome {
                        TurnOutcome::Completed => (None, "completed"),
                        TurnOutcome::Interrupted => (None, "interrupted"),
                        TurnOutcome::Failed(error) => (Some(describe(error)), "failed"),
                        TurnOutcome::Limited(note) => (
                            Some(
                                note.clone()
                                    .unwrap_or_else(|| "A usage limit was reached".into()),
                            ),
                            "limited",
                        ),
                        TurnOutcome::Refused(note) => (
                            Some(
                                note.clone()
                                    .unwrap_or_else(|| "The agent declined to continue".into()),
                            ),
                            "refused",
                        ),
                    };
                    self.end_turn(session, error, label);
                }
            }
            AgentEvent::MetadataChanged { title } => {
                if let Some(session) = session {
                    self.registry.suggest_title(session, title);
                    self.registry_dirty = true;
                }
            }
            _ => {}
        }
    }

    /// A settings change was answered: send the next one, or let a waiting turn go.
    fn config_settled(&mut self, session: SessionId) {
        if let Some(live) = self.live.get_mut(&session) {
            live.config_in_flight = 0;
        }
        self.pump_config(session);
        if let Some(provider) = self.registry.get(session).map(|r| r.provider) {
            self.start_current(session, provider);
        }
    }

    fn settle(&mut self, pending: Pending, outcome: &OperationOutcome) {
        match (pending, outcome) {
            (
                Pending::Prepare(session),
                OperationOutcome::Succeeded(OperationOutput::SessionReady {
                    native, config, ..
                }),
            ) => {
                self.registry.set_native(session, native);
                self.registry_dirty = true;
                let provider = native.provider;
                if let Some(live) = self.live.get_mut(&session) {
                    live.ready = true;
                    live.preparing = false;
                    if live.model.config.is_none() {
                        live.model.config = Some(config.clone());
                    }
                }
                self.restore_prefs(session);
                self.start_current(session, provider);
            }
            (Pending::Prepare(session), OperationOutcome::Succeeded(_)) => {
                self.fail_turn(session, "The agent answered unexpectedly".into());
            }
            (
                Pending::Prepare(session) | Pending::Start(session),
                OperationOutcome::Failed(error),
            ) => {
                self.fail_turn(session, describe(error));
            }
            (Pending::Prepare(session) | Pending::Start(session), OperationOutcome::Cancelled) => {
                self.fail_turn(session, "The request was cancelled".into());
            }
            (
                Pending::Start(session),
                OperationOutcome::Succeeded(OperationOutput::TurnStarted(turn)),
            ) => {
                if let Some(live) = self.live.get_mut(&session) {
                    live.turn = Some(turn.clone());
                }
            }
            (
                Pending::Config(session),
                OperationOutcome::Succeeded(OperationOutput::Config(config)),
            ) => {
                self.remember_prefs(session, config);
                self.config_settled(session);
            }
            (Pending::Config(session), OperationOutcome::Failed(error)) => {
                if let Some(live) = self.live.get_mut(&session) {
                    live.error = Some(format!("Could not change the setting: {}", describe(error)));
                }
                self.config_settled(session);
            }
            (Pending::Config(session), _) => self.config_settled(session),
            // Interrupt, reply and housekeeping results need no follow-up: their
            // effects arrive as events. A failed reply is shown on the session.
            (Pending::Reply, OperationOutcome::Failed(error)) => {
                let message = format!("The answer was not delivered: {}", describe(error));
                for live in self.live.values_mut().filter(|l| l.model.needs_attention()) {
                    live.error = Some(message.clone());
                }
            }
            _ => {}
        }
    }

    /// Stop everything: cancel queued work, ask backends to shut down. The caller
    /// keeps ticking until [`AgentHub::is_shut_down`].
    pub(crate) fn begin_shutdown(&mut self) {
        for backend in self.backends.values_mut() {
            let _ = backend.begin_shutdown(Duration::from_secs(3));
        }
    }
}

/// How a message block reads in the transcript.
fn describe_block(block: &InputBlock) -> String {
    match block {
        InputBlock::Text(text) => text.clone(),
        InputBlock::FileReference { path } => format!("@{}", path.display()),
        InputBlock::TextSnapshot { origin, .. } => format!("[file: {origin}]"),
        InputBlock::SelectionSnapshot { origin, .. } => format!("[selection: {origin}]"),
        InputBlock::Image { media_type, .. } => format!("[{media_type} image]"),
    }
}

fn describe(error: &AgentError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::testing::FakeBackend;

    fn hub() -> AgentHub {
        let backends: Vec<(ProviderId, Box<dyn AgentBackend>)> = vec![
            (
                ProviderId::Codex,
                Box::new(FakeBackend::new(ProviderId::Codex)),
            ),
            (
                ProviderId::Claude,
                Box::new(FakeBackend::new(ProviderId::Claude)),
            ),
        ];
        AgentHub::new(backends, Registry::new()).with_clock(|| 1000)
    }

    fn text(message: &str) -> Vec<InputBlock> {
        vec![InputBlock::Text(message.into())]
    }

    fn settle(hub: &mut AgentHub, until: impl Fn(&AgentHub) -> bool) {
        for _ in 0..200 {
            hub.tick();
            if until(hub) {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("hub did not settle");
    }

    fn idle(hub: &AgentHub, session: SessionId) -> bool {
        hub.status(session) == SessionStatus::Idle
            && hub.live(session).is_some_and(|l| l.prompts.is_empty())
    }

    #[test]
    fn a_message_creates_the_native_session_runs_the_turn_and_titles_the_chat() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        hub.send(session, text("hello there")).unwrap();
        assert!(matches!(hub.status(session), SessionStatus::Working));
        settle(&mut hub, |h| idle(h, session));
        let record = hub.registry.get(session).unwrap();
        assert!(record.native.is_some(), "native identity recorded");
        assert_eq!(record.title, "hello there");
        assert_eq!(record.last_outcome.as_deref(), Some("completed"));
        let live = hub.live(session).unwrap();
        assert_eq!(live.model.last_outcome, Some(TurnOutcome::Completed));
        assert!(!live.model.items.is_empty());
        assert!(hub.registry_dirty);
    }

    #[test]
    fn queued_messages_run_in_order_and_a_queued_one_can_be_cancelled() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        hub.send(session, text("one")).unwrap();
        let second = hub.send(session, text("two")).unwrap();
        let third = hub.send(session, text("three")).unwrap();
        assert_eq!(
            hub.status(session),
            SessionStatus::Working,
            "the head is running; the rest wait their turn"
        );
        assert!(hub.cancel_queued(session, second));
        assert!(!hub.cancel_queued(session, second), "already gone");
        settle(&mut hub, |h| idle(h, session));
        let users: Vec<_> = hub
            .live(session)
            .unwrap()
            .model
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                lumen_agent_core::model::ItemKind::User(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(users, ["one", "three"]);
        let _ = third;
    }

    #[test]
    fn the_second_writer_in_one_folder_waits_with_a_reason_and_other_folders_do_not() {
        let mut hub = hub();
        let a = hub.create_session(ProviderId::Codex, "/repo".into());
        let b = hub.create_session(ProviderId::Claude, "/repo".into());
        let c = hub.create_session(ProviderId::Claude, "/elsewhere".into());
        hub.set_slots(3);
        hub.send(a, text("slow: working")).unwrap();
        hub.send(b, text("hi")).unwrap();
        hub.send(c, text("hi")).unwrap();
        assert_eq!(
            hub.status(b),
            SessionStatus::Waiting(WaitReason::Lease { holder: a })
        );
        settle(&mut hub, |h| idle(h, c));
        assert!(matches!(hub.status(b), SessionStatus::Waiting(_)));
        assert!(hub.interrupt(a));
        settle(&mut hub, |h| idle(h, a) && idle(h, b));
        assert_eq!(
            hub.live(a).unwrap().model.last_outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert_eq!(
            hub.live(b).unwrap().model.last_outcome,
            Some(TurnOutcome::Completed)
        );
    }

    #[test]
    fn an_approval_needs_attention_releases_the_slot_and_resumes_after_the_answer() {
        let mut hub = hub();
        hub.set_slots(1);
        let a = hub.create_session(ProviderId::Codex, "/repo".into());
        let b = hub.create_session(ProviderId::Codex, "/other".into());
        hub.send(a, text("approve: touch file")).unwrap();
        settle(&mut hub, |h| h.status(a) == SessionStatus::NeedsAttention);
        // The slot is free for another folder while the user decides.
        hub.send(b, text("hi")).unwrap();
        settle(&mut hub, |h| idle(h, b));
        let request = hub.live(a).unwrap().model.pending_approvals[0].clone();
        let allow = request
            .choices
            .iter()
            .find(|c| c.allows)
            .unwrap()
            .id
            .clone();
        hub.reply_approval(a, request.token, allow);
        settle(&mut hub, |h| idle(h, a));
        assert_eq!(
            hub.live(a).unwrap().model.last_outcome,
            Some(TurnOutcome::Completed)
        );
    }

    #[test]
    fn a_failed_backend_operation_ends_the_turn_with_a_message_and_frees_the_queue() {
        let mut hub = hub();
        let session = hub.registry.create(
            ProviderId::Codex,
            lumen_agent_core::ProfileId::default_profile(),
            "/w".into(),
            1,
        );
        // Pretend the index knows a native session the backend has never heard of.
        let native = lumen_agent_core::NativeSessionRef {
            provider: ProviderId::Codex,
            profile: lumen_agent_core::ProfileId::default_profile(),
            id: "gone".into(),
        };
        hub.registry.set_native(session, &native);
        hub.send(session, text("hello")).unwrap();
        settle(&mut hub, |h| {
            matches!(h.status(session), SessionStatus::Failed(_))
        });
        assert!(hub.scheduler.is_idle(), "nothing keeps a slot or lease");
        // The next message is accepted again.
        assert!(hub.send(session, text("again")).is_ok());
    }

    #[test]
    fn sending_to_unknown_archived_or_empty_is_refused_without_side_effects() {
        let mut hub = hub();
        assert_eq!(
            hub.send(SessionId(42), text("x")),
            Err(SendError::UnknownSession)
        );
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        assert_eq!(hub.send(session, Vec::new()), Err(SendError::EmptyMessage));
        hub.archive(session);
        assert_eq!(hub.send(session, text("x")), Err(SendError::Archived));
        assert!(hub.scheduler.is_idle());
    }

    #[test]
    fn two_providers_run_at_once_within_the_slot_limit() {
        let mut hub = hub();
        let a = hub.create_session(ProviderId::Codex, "/one".into());
        let b = hub.create_session(ProviderId::Claude, "/two".into());
        hub.send(a, text("slow: a")).unwrap();
        hub.send(b, text("slow: b")).unwrap();
        settle(&mut hub, |h| {
            h.live(a).is_some_and(|l| l.model.state != TurnState::Idle)
                && h.live(b).is_some_and(|l| l.model.state != TurnState::Idle)
        });
        assert!(hub.interrupt(a) && hub.interrupt(b));
        settle(&mut hub, |h| idle(h, a) && idle(h, b));
    }

    #[test]
    fn a_new_session_is_ready_with_editable_settings_before_any_message() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        assert_eq!(
            hub.status(session),
            SessionStatus::Idle,
            "preparing is not working"
        );
        settle(&mut hub, |h| {
            h.live(session).is_some_and(|l| l.model.config.is_some())
        });
        let config = hub.live(session).unwrap().model.config.clone().unwrap();
        assert_eq!(config.get("model").unwrap().current, "fast");
        assert!(hub.set_config(
            session,
            ConfigPatch {
                values: vec![("model".into(), "smart".into())],
                ..Default::default()
            }
        ));
        settle(&mut hub, |h| {
            h.live(session)
                .and_then(|l| l.model.config.as_ref())
                .is_some_and(|c| c.get("model").unwrap().current == "smart")
        });
        // A message sent now reuses the prepared session instead of creating another.
        hub.send(session, text("hi")).unwrap();
        settle(&mut hub, |h| idle(h, session));
        assert_eq!(
            hub.live(session).unwrap().model.last_outcome,
            Some(TurnOutcome::Completed)
        );
    }

    #[test]
    fn a_message_sent_while_the_session_is_still_preparing_waits_for_it() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        hub.send(session, text("immediately")).unwrap();
        settle(&mut hub, |h| idle(h, session));
        assert_eq!(
            hub.live(session).unwrap().model.last_outcome,
            Some(TurnOutcome::Completed)
        );
    }

    #[test]
    fn a_refused_setting_is_reported_on_the_session() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        settle(&mut hub, |h| {
            h.live(session).is_some_and(|l| l.model.config.is_some())
        });
        hub.set_config(
            session,
            ConfigPatch {
                values: vec![("model".into(), "no-such-model".into())],
                ..Default::default()
            },
        );
        settle(&mut hub, |h| {
            matches!(h.status(session), SessionStatus::Failed(_))
        });
    }

    fn current(hub: &AgentHub, session: SessionId, option: &str) -> String {
        hub.live(session)
            .and_then(|l| l.model.config.as_ref())
            .and_then(|c| c.get(option))
            .map(|o| o.current.clone())
            .unwrap_or_default()
    }

    #[test]
    fn quick_edits_in_a_row_all_apply_instead_of_colliding_on_the_revision() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        settle(&mut hub, |h| {
            h.live(session).is_some_and(|l| l.model.config.is_some())
        });
        assert!(hub.set_config(session, ConfigPatch::set("model", "smart")));
        assert!(hub.set_config(session, ConfigPatch::set("verbose", "true")));
        settle(&mut hub, |h| current(h, session, "verbose") == "true");
        assert_eq!(current(&hub, session, "model"), "smart");
        assert!(hub.live(session).unwrap().error.is_none());
    }

    #[test]
    fn a_turn_waits_for_pending_settings_and_starts_with_the_latest_revision() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        settle(&mut hub, |h| {
            h.live(session).is_some_and(|l| l.model.config.is_some())
        });
        hub.set_config(session, ConfigPatch::set("model", "smart"));
        hub.send(session, text("hi")).unwrap();
        settle(&mut hub, |h| idle(h, session));
        let live = hub.live(session).unwrap();
        assert_eq!(live.model.last_outcome, Some(TurnOutcome::Completed));
        assert!(
            live.error.is_none(),
            "the revision guard did not reject the turn"
        );
    }

    #[test]
    fn choices_are_remembered_per_session_and_reapplied_when_it_is_resumed() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        let other = hub.create_session(ProviderId::Codex, "/w".into());
        settle(&mut hub, |h| {
            h.live(session).is_some_and(|l| l.model.config.is_some())
                && h.live(other).is_some_and(|l| l.model.config.is_some())
        });
        hub.set_config(session, ConfigPatch::set("verbose", "true"));
        settle(&mut hub, |h| current(h, session, "verbose") == "true");
        assert_eq!(
            hub.registry.get(session).unwrap().prefs,
            vec![("verbose".to_string(), "true".to_string())],
            "only the non-default choice is stored"
        );
        assert!(
            hub.registry.get(other).unwrap().prefs.is_empty(),
            "other sessions are untouched"
        );
        assert_eq!(current(&hub, other, "verbose"), "false");
        // A resume starts from the provider's own state; the saved choice is re-applied.
        hub.registry
            .set_prefs(session, vec![("verbose".to_string(), "true".to_string())]);
        let live = hub.live.get_mut(&session).unwrap();
        live.ready = false;
        live.model.config = None;
        hub.ensure_ready(session);
        settle(&mut hub, |h| current(h, session, "verbose") == "true");
        // Reset returns to the default and clears the stored choice.
        hub.set_config(session, ConfigPatch::reset("verbose"));
        settle(&mut hub, |h| current(h, session, "verbose") == "false");
        assert!(hub.registry.get(session).unwrap().prefs.is_empty());
    }
}
