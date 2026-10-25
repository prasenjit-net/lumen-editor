//! The native-agent controller: owns the provider backends, the session index and
//! the turn scheduler, and turns user actions into backend operations.
//!
//! Everything is polled from the UI thread: [`AgentHub::tick`] drains backend
//! events into each session's reducer, settles operations and starts whatever the
//! scheduler now allows. Backends never queue turns - the hub keeps each session's
//! queue - and a backend only ever sees one active turn per session.
use crate::model::{
    agent_registry::{Registry, SessionRecord},
    agent_scheduler::{Dispatch, Phase, Scheduler, Ticket, WaitReason},
};
use lumen_agent_core::{
    AgentError, ClientRequestId, ItemId, OperationId, PollBudget, ProviderId, RequestToken,
    SessionId, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, ClientTurnId, ConfigPatch, InputBlock,
        OperationOutcome, OperationOutput, Request, SessionConfig,
    },
    event::{AgentEvent, EventEnvelope, TurnOutcome},
    model::{SessionModel, TurnState},
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    time::Duration,
};

/// What a History row says about a session.
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

/// Which way a pending message moves in the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueMove {
    Up,
    Down,
}

/// Why `send` refused a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SendError {
    UnknownSession,
    Archived,
    EmptyMessage,
    ProviderUnavailable,
    /// Lumen is quitting; nothing new starts.
    ShuttingDown,
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
    /// What the provider reported when the session became ready.
    pub(crate) capabilities: Option<Box<lumen_agent_core::Capabilities>>,
    /// Settings changes sent to the provider and not yet answered. A turn waits
    /// for them so it starts with the revision the user last saw.
    config_in_flight: usize,
    /// Changes waiting for the previous one to be answered (each needs the
    /// revision the last produced).
    config_queue: VecDeque<ConfigPatch>,
    /// `StartTurn` has been sent for the current prompt.
    starting: bool,
    /// The current turn was already retried once after the provider lost the
    /// session (for example after the agent restarted).
    reattached: bool,
    /// A turn has run since the session was attached, so it may hold a process.
    had_turn: bool,
    /// When the session last became idle (seconds); drives releasing idle sessions.
    idle_since: u64,
    /// The chat was archived or left behind by a folder change: its process goes
    /// as soon as the turn is over, without waiting for the idle timeout.
    release_now: bool,
    /// A detach is on its way to the provider.
    detaching: bool,
    /// The saved conversation was looked up (whether or not one existed).
    transcript_loaded: bool,
    /// The conversation changed since it was last saved.
    transcript_dirty: bool,
}

enum HeldReply {
    Approval(RequestToken, ApprovalDecisionId),
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
            capabilities: None,
            config_in_flight: 0,
            config_queue: VecDeque::new(),
            starting: false,
            reattached: false,
            had_turn: false,
            idle_since: 0,
            release_now: false,
            detaching: false,
            transcript_loaded: false,
            transcript_dirty: false,
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
    /// Giving up the provider session; a refusal leaves it to be released later.
    Detach(SessionId),
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
    /// Shutdown operations still open, per provider; `Some` once shutdown began.
    shutdown_ops: Option<Vec<(ProviderId, OperationId)>>,
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
            shutdown_ops: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_clock(mut self, now: impl Fn() -> u64 + Send + 'static) -> Self {
        self.now = Box::new(now);
        self
    }

    #[cfg(test)]
    pub(crate) fn set_slots(&mut self, slots: usize) {
        self.scheduler.set_slots(slots);
    }

    /// Conversations that changed since they were saved, with the provider
    /// identity they are stored under. Sessions without one have nothing to save to.
    pub(crate) fn take_dirty_transcripts(
        &mut self,
    ) -> Vec<(
        ProviderId,
        String,
        Vec<lumen_agent_core::model::TranscriptItem>,
    )> {
        let mut out = Vec::new();
        for (id, live) in &mut self.live {
            if !live.transcript_dirty {
                continue;
            }
            live.transcript_dirty = false;
            if let Some(record) = self.registry.get(*id)
                && let Some(native) = &record.native
            {
                out.push((record.provider, native.clone(), live.model.items.clone()));
            }
        }
        out
    }

    pub(crate) fn transcript_loaded(&self, session: SessionId) -> bool {
        self.live.get(&session).is_some_and(|l| l.transcript_loaded)
    }

    /// Show a saved conversation in a session that has none yet.
    pub(crate) fn restore_transcript(
        &mut self,
        session: SessionId,
        items: Vec<lumen_agent_core::model::TranscriptItem>,
    ) {
        let live = self.live_mut(session);
        live.transcript_loaded = true;
        live.model.restore_items(items);
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
        if self.shutdown_ops.is_some() {
            return Err(SendError::ShuttingDown);
        }
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
        live.release_now = false;
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

    /// Position of a message that can still be changed: waiting behind the head
    /// of the session's list, not dispatched, and still queued in the scheduler.
    /// The head is the active prompt and is never editable or movable.
    fn editable_position(&self, session: SessionId, ticket: Ticket) -> Option<usize> {
        let live = self.live.get(&session)?;
        if live.current == Some(ticket) || self.scheduler.phase(ticket) != Some(Phase::Queued) {
            return None;
        }
        live.prompts
            .iter()
            .position(|p| p.ticket == ticket)
            .filter(|&index| index > 0)
    }

    /// Replace the text of a pending message, keeping its attachments and place.
    /// Refused once the message started, and for an empty result.
    pub(crate) fn edit_queued(&mut self, session: SessionId, ticket: Ticket, text: &str) -> bool {
        let Some(index) = self.editable_position(session, ticket) else {
            return false;
        };
        let Some(live) = self.live.get_mut(&session) else {
            return false;
        };
        let input = &mut live.prompts[index].input;
        let has_other = input.iter().any(|b| !matches!(b, InputBlock::Text(_)));
        let text = text.trim();
        if text.is_empty() && !has_other {
            return false;
        }
        input.retain(|b| !matches!(b, InputBlock::Text(_)));
        if !text.is_empty() {
            input.insert(0, InputBlock::Text(text.to_string()));
        }
        true
    }

    /// Swap a pending message with its neighbour. Tickets keep their scheduler
    /// order, so the contents move and the identities stay put.
    pub(crate) fn move_queued(
        &mut self,
        session: SessionId,
        ticket: Ticket,
        direction: QueueMove,
    ) -> bool {
        let Some(index) = self.editable_position(session, ticket) else {
            return false;
        };
        let other = match direction {
            QueueMove::Up => index - 1,
            QueueMove::Down => index + 1,
        };
        let Some(neighbour) = self
            .live
            .get(&session)
            .and_then(|live| live.prompts.get(other))
            .map(|p| p.ticket)
        else {
            return false;
        };
        if self.editable_position(session, neighbour).is_none() {
            return false;
        }
        let Some(live) = self.live.get_mut(&session) else {
            return false;
        };
        let (a, b) = (index.min(other), index.max(other));
        let (left, right) = live.prompts.make_contiguous().split_at_mut(b);
        std::mem::swap(&mut left[a].input, &mut right[0].input);
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
        if patch.is_empty() || self.shutdown_ops.is_some() {
            return false;
        }
        let Some(live) = self.live.get_mut(&session) else {
            return false;
        };
        live.config_queue.push_back(patch);
        if live.ready {
            self.pump_config(session);
        } else {
            // Released or lost: attach again, then the change goes out.
            self.ensure_ready(session);
        }
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
        self.release_session(session);
        self.registry.set_archived(session, true);
        self.registry_dirty = true;
    }

    /// Stop everything a chat has going and give up its process once it is
    /// quiet: queued messages are dropped, a running turn (including one waiting
    /// for an answer) is interrupted, and the provider session is detached.
    fn release_session(&mut self, session: SessionId) {
        for ticket in self.scheduler.drop_session(session) {
            let _ = ticket;
        }
        let Some(live) = self.live.get_mut(&session) else {
            return;
        };
        live.prompts.clear();
        live.current = None;
        live.held_reply = None;
        live.release_now = true;
        if live.turn.is_some() {
            self.interrupt(session);
        }
    }

    /// The open folder changed: chats of any other folder stop and let go of
    /// their processes, so nothing keeps working in a folder that is no longer open.
    pub(crate) fn release_outside(&mut self, workspace: &std::path::Path) {
        let leaving: Vec<SessionId> = self
            .live
            .keys()
            .copied()
            .filter(|id| {
                self.registry
                    .get(*id)
                    .is_some_and(|record| record.workspace != workspace)
            })
            .collect();
        for session in leaving {
            self.release_session(session);
        }
    }

    // ---------------------------------------------------------------- status

    /// The most actionable unarchived session, across folders. Approval requests
    /// take precedence over failures, running turns, then scheduler waits. Ties
    /// use recency and identity, independent of pins or HashMap iteration order.
    pub(crate) fn provider_activity(
        &self,
        provider: ProviderId,
    ) -> Option<(&SessionRecord, SessionStatus)> {
        self.registry
            .listing(None, false)
            .into_iter()
            .filter(|record| record.provider == provider)
            .map(|record| (record, self.status(record.id)))
            .filter(|(_, status)| *status != SessionStatus::Idle)
            .max_by_key(|(record, status)| {
                let priority = match status {
                    SessionStatus::NeedsAttention => 4,
                    SessionStatus::Failed(_) => 3,
                    SessionStatus::Working => 2,
                    SessionStatus::Waiting(_) => 1,
                    SessionStatus::Idle => 0,
                };
                (priority, record.updated, std::cmp::Reverse(record.id.0))
            })
    }

    pub(crate) fn status(&self, session: SessionId) -> SessionStatus {
        let Some(live) = self.live.get(&session) else {
            return SessionStatus::Idle;
        };
        if live.model.needs_attention() {
            return SessionStatus::NeedsAttention;
        }
        for prompt in &live.prompts {
            match self.scheduler.phase(prompt.ticket) {
                Some(Phase::Executing) => return SessionStatus::Working,
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

    /// Some unarchived chat of this provider is waiting for the user's answer.
    pub(crate) fn provider_needs_attention(&self, provider: ProviderId) -> bool {
        self.registry
            .listing(None, false)
            .into_iter()
            .any(|record| {
                record.provider == provider
                    && self.status(record.id) == SessionStatus::NeedsAttention
            })
    }

    pub(crate) fn any_needs_attention(&self) -> bool {
        ProviderId::ALL
            .into_iter()
            .any(|provider| self.provider_needs_attention(provider))
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
        let (item, shown) = (
            ItemId(format!("user-{}-{}", (self.now)(), ticket.0)),
            shown.join("\n"),
        );
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
            live.reattached = false;
            live.had_turn = true;
            live.idle_since = now;
            live.transcript_dirty = true;
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
        self.release_idle();
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
                if let (Some(open), Some(op)) = (self.shutdown_ops.as_mut(), envelope.operation) {
                    open.retain(|entry| *entry != (provider, op));
                }
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
            AgentEvent::RuntimeExited { .. } => {
                // A shared agent process (Codex) took its sessions with it; they
                // must be attached again before the next turn.
                if provider == ProviderId::Codex
                    && self.shutdown_ops.is_none()
                    && let Some(live) = session.and_then(|s| self.live.get_mut(&s))
                {
                    live.ready = false;
                    live.preparing = false;
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

    /// The provider no longer knows the session (its agent restarted): attach it
    /// again and let the turn go through once. False when already retried.
    fn reattach_once(&mut self, session: SessionId) -> bool {
        let Some(provider) = self.registry.get(session).map(|r| r.provider) else {
            return false;
        };
        let Some(live) = self.live.get_mut(&session) else {
            return false;
        };
        if live.reattached || live.current.is_none() {
            return false;
        }
        live.reattached = true;
        live.ready = false;
        live.preparing = false;
        live.starting = false;
        self.prepare(session, provider);
        true
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
                    native,
                    config,
                    capabilities,
                }),
            ) => {
                self.registry.set_native(session, native);
                self.registry_dirty = true;
                let provider = native.provider;
                if let Some(live) = self.live.get_mut(&session) {
                    live.ready = true;
                    live.preparing = false;
                    live.capabilities = Some(capabilities.clone());
                    if live.model.config.is_none() {
                        live.model.config = Some(config.clone());
                    }
                }
                self.restore_prefs(session);
                self.pump_config(session);
                self.start_current(session, provider);
            }
            (Pending::Prepare(session), OperationOutcome::Succeeded(_)) => {
                self.fail_turn(session, "The agent answered unexpectedly".into());
            }
            (Pending::Start(session), OperationOutcome::Failed(AgentError::Stale { .. }))
                if self.reattach_once(session) => {}
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
                let release = self.live.get_mut(&session).is_some_and(|live| {
                    live.turn = Some(turn.clone());
                    live.release_now
                });
                // Released while the turn was still starting.
                if release {
                    self.interrupt(session);
                }
            }
            (Pending::Detach(session), outcome) => {
                let succeeded = matches!(outcome, OperationOutcome::Succeeded(_));
                if let Some(live) = self.live.get_mut(&session) {
                    live.detaching = false;
                    // Only forget the provider session when nothing started in
                    // the meantime; a refusal keeps it so a later pass retries.
                    if succeeded && live.current.is_none() && live.turn.is_none() && !live.starting
                    {
                        live.ready = false;
                        live.had_turn = false;
                        live.release_now = false;
                    }
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

    /// Stop everything: queued work is cancelled, waiting approvals declined,
    /// running turns interrupted and every process stopped. Keep ticking until
    /// [`AgentHub::is_shut_down`]. Safe to call twice.
    pub(crate) fn begin_shutdown(&mut self) {
        if self.shutdown_ops.is_some() {
            return;
        }
        for live in self.live.values_mut().filter(|l| !l.model.items.is_empty()) {
            live.transcript_dirty = true;
        }
        let mut open = Vec::new();
        for (provider, backend) in &mut self.backends {
            if let Ok(op) = backend.begin_shutdown(Duration::from_secs(3)) {
                open.push((*provider, op));
            }
        }
        self.shutdown_ops = Some(open);
    }

    #[cfg(test)]
    pub(crate) fn shutdown_started(&self) -> bool {
        self.shutdown_ops.is_some()
    }

    /// Every backend has finished shutting down.
    pub(crate) fn is_shut_down(&self) -> bool {
        self.shutdown_ops.as_ref().is_some_and(Vec::is_empty)
    }

    /// Sessions idle this long give up their provider session (and with it any
    /// process). They attach again on the next message.
    pub(crate) const IDLE_RELEASE_SECONDS: u64 = 300;
    /// At most this many idle sessions keep a warm provider session.
    pub(crate) const WARM_IDLE_SESSIONS: usize = 2;

    /// Release idle sessions: those past the timeout, and the oldest beyond the
    /// warm limit. Never touches a session that is working, waiting, has queued
    /// messages or unanswered settings changes.
    pub(crate) fn release_idle(&mut self) {
        if self.shutdown_ops.is_some() {
            return;
        }
        let now = (self.now)();
        let mut idle: Vec<(u64, SessionId, bool)> = self
            .live
            .iter()
            .filter(|(id, live)| {
                live.ready
                    && live.had_turn
                    && !live.detaching
                    && live.current.is_none()
                    && live.prompts.is_empty()
                    && live.config_in_flight == 0
                    && live.config_queue.is_empty()
                    && !live.model.needs_attention()
                    && live.model.state == TurnState::Idle
                    && self.status(**id) != SessionStatus::Working
            })
            .map(|(id, live)| (live.idle_since, *id, live.release_now))
            .collect();
        idle.sort();
        let excess = idle.len().saturating_sub(Self::WARM_IDLE_SESSIONS);
        let release: Vec<SessionId> = idle
            .iter()
            .enumerate()
            .filter(|(index, (since, _, now_release))| {
                *now_release
                    || *index < excess
                    || now.saturating_sub(*since) >= Self::IDLE_RELEASE_SECONDS
            })
            .map(|(_, (_, id, _))| *id)
            .collect();
        for session in release {
            let Some(provider) = self.registry.get(session).map(|r| r.provider) else {
                continue;
            };
            if self.submit(
                provider,
                AgentOperation::DetachSession { session },
                Pending::Detach(session),
            ) && let Some(live) = self.live.get_mut(&session)
            {
                live.detaching = true;
            }
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
    fn provider_activity_prioritizes_attention_failure_work_and_waiting_across_folders() {
        let mut hub = hub();
        hub.set_slots(1);
        let working = hub.create_session(ProviderId::Codex, "/working".into());
        let waiting = hub.create_session(ProviderId::Codex, "/waiting".into());
        let failed = hub.create_session(ProviderId::Codex, "/failed".into());
        let approval = hub.create_session(ProviderId::Codex, "/approval".into());
        let other = hub.create_session(ProviderId::Claude, "/other".into());
        assert!(hub.provider_activity(ProviderId::Codex).is_none());
        hub.live_mut(other).error = Some("other provider".into());
        hub.send(approval, text("approve: touch file")).unwrap();
        settle(&mut hub, |hub| {
            hub.status(approval) == SessionStatus::NeedsAttention
        });
        hub.live_mut(failed).error = Some("connection lost".into());
        hub.send(working, text("slow: running")).unwrap();
        hub.send(waiting, text("waiting for a slot")).unwrap();
        let activity = |hub: &AgentHub| {
            hub.provider_activity(ProviderId::Codex)
                .map(|(r, s)| (r.id, s))
        };
        assert_eq!(
            activity(&hub),
            Some((approval, SessionStatus::NeedsAttention))
        );
        hub.registry.set_archived(approval, true);
        assert_eq!(
            activity(&hub),
            Some((failed, SessionStatus::Failed("connection lost".into())))
        );
        hub.registry.set_archived(failed, true);
        assert_eq!(activity(&hub), Some((working, SessionStatus::Working)));
        hub.registry.set_archived(working, true);
        assert!(matches!(activity(&hub), Some((id, SessionStatus::Waiting(_))) if id == waiting));
        hub.registry.set_archived(waiting, true);
        assert_eq!(activity(&hub), None);
        assert_eq!(
            hub.provider_activity(ProviderId::Claude).unwrap().0.id,
            other
        );
    }

    #[test]
    fn provider_activity_ties_use_recency_then_identity_and_ignore_pins() {
        let mut hub = hub();
        let first = hub.create_session(ProviderId::Codex, "/a".into());
        let second = hub.create_session(ProviderId::Codex, "/b".into());
        hub.live_mut(first).error = Some("failed".into());
        hub.live_mut(second).error = Some("failed".into());
        hub.registry.set_pinned(second, true);
        assert_eq!(
            hub.provider_activity(ProviderId::Codex).unwrap().0.id,
            first
        );
        hub.registry.touch(second, 1001, None);
        assert_eq!(
            hub.provider_activity(ProviderId::Codex).unwrap().0.id,
            second
        );
        hub.live_mut(second).error = None;
        assert_eq!(
            hub.provider_activity(ProviderId::Codex).unwrap().0.id,
            first
        );
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

    fn user_texts(hub: &AgentHub, session: SessionId) -> Vec<String> {
        hub.live(session)
            .unwrap()
            .model
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                lumen_agent_core::model::ItemKind::User(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pending_messages_can_be_edited_and_reordered_but_not_the_active_one() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        let head = hub.send(session, text("one")).unwrap();
        let second = hub.send(session, text("two")).unwrap();
        let third = hub.send(session, text("three")).unwrap();
        let fourth = hub.send(session, text("four")).unwrap();
        assert!(!hub.edit_queued(session, head, "changed"), "active prompt");
        assert!(!hub.move_queued(session, head, QueueMove::Down));
        assert!(
            !hub.move_queued(session, second, QueueMove::Up),
            "head stays first"
        );
        assert!(
            !hub.move_queued(session, fourth, QueueMove::Down),
            "last stays last"
        );
        assert!(
            !hub.edit_queued(session, second, "  "),
            "empty edit is refused"
        );
        assert!(hub.edit_queued(session, second, " two edited "));
        assert!(hub.move_queued(session, third, QueueMove::Up));
        let order: Vec<_> = hub
            .live(session)
            .unwrap()
            .prompts
            .iter()
            .map(|p| (p.ticket, p.input.clone()))
            .collect();
        assert_eq!(order[1], (second, text("three")));
        assert_eq!(order[2], (third, text("two edited")));
        settle(&mut hub, |h| idle(h, session));
        assert_eq!(
            user_texts(&hub, session),
            ["one", "three", "two edited", "four"]
        );
        assert!(
            !hub.edit_queued(session, second, "late"),
            "finished message"
        );
    }

    #[test]
    fn editing_replaces_the_text_and_stale_tickets_change_nothing() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        hub.send(session, text("head")).unwrap();
        let mut blocks = text("see file");
        blocks.push(InputBlock::Text("second block".into()));
        let ticket = hub.send(session, blocks).unwrap();
        assert!(hub.edit_queued(session, ticket, "revised"));
        assert_eq!(
            hub.live(session).unwrap().prompts[1].input,
            text("revised"),
            "text blocks are replaced by the edited text"
        );
        assert!(hub.cancel_queued(session, ticket));
        assert!(!hub.edit_queued(session, ticket, "gone"));
        assert!(!hub.move_queued(session, ticket, QueueMove::Up));
        let other = hub.create_session(ProviderId::Codex, "/x".into());
        assert!(!hub.edit_queued(other, ticket, "wrong session"));
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

    /// Fails the first `StartTurn` the way Codex does after its agent restarted.
    struct LosesSessionOnce {
        inner: FakeBackend,
        failed_once: bool,
    }

    impl AgentBackend for LosesSessionOnce {
        fn descriptor(&self) -> &lumen_agent_core::ProviderDescriptor {
            self.inner.descriptor()
        }

        fn try_submit(
            &mut self,
            request: Request,
        ) -> Result<OperationId, lumen_agent_core::SubmitError> {
            if matches!(request.operation, AgentOperation::StartTurn { .. }) && !self.failed_once {
                self.failed_once = true;
                // A request the fake answers with Stale stands in for the real one.
                return self.inner.try_submit(Request {
                    client_request_id: request.client_request_id,
                    operation: AgentOperation::InterruptTurn {
                        session: SessionId(9999),
                        turn: TurnId("lost".into()),
                    },
                });
            }
            self.inner.try_submit(request)
        }

        fn poll_events(&mut self, budget: PollBudget) -> Vec<EventEnvelope> {
            self.inner.poll_events(budget)
        }

        fn begin_shutdown(
            &mut self,
            deadline: Duration,
        ) -> Result<OperationId, lumen_agent_core::SubmitError> {
            self.inner.begin_shutdown(deadline)
        }
    }

    #[test]
    fn a_session_the_provider_lost_is_attached_again_and_the_turn_still_runs() {
        let backends: Vec<(ProviderId, Box<dyn AgentBackend>)> = vec![(
            ProviderId::Codex,
            Box::new(LosesSessionOnce {
                inner: FakeBackend::new(ProviderId::Codex),
                failed_once: false,
            }),
        )];
        let mut hub = AgentHub::new(backends, Registry::new()).with_clock(|| 1000);
        let session = hub.create_session(ProviderId::Codex, "/w".into());
        hub.send(session, text("hello")).unwrap();
        settle(&mut hub, |h| idle(h, session));
        let live = hub.live(session).unwrap();
        assert_eq!(live.model.last_outcome, Some(TurnOutcome::Completed));
        assert!(live.error.is_none(), "{:?}", live.error);
    }

    #[test]
    fn shutdown_stops_work_refuses_new_messages_and_finishes() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        hub.send(session, text("slow: long task")).unwrap();
        settle(&mut hub, |h| {
            h.live(session)
                .is_some_and(|l| l.model.state != TurnState::Idle)
        });
        assert!(!hub.is_shut_down());
        hub.begin_shutdown();
        hub.begin_shutdown();
        assert_eq!(
            hub.send(session, text("more")),
            Err(SendError::ShuttingDown)
        );
        settle(&mut hub, |h| h.is_shut_down());
        assert!(hub.shutdown_started());
        assert!(!hub.set_config(session, ConfigPatch::set("model", "smart")));
    }

    #[test]
    fn idle_sessions_are_released_after_a_timeout_and_beyond_the_warm_limit_and_come_back() {
        let clock = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1000));
        let reader = clock.clone();
        let backends: Vec<(ProviderId, Box<dyn AgentBackend>)> = vec![(
            ProviderId::Codex,
            Box::new(FakeBackend::new(ProviderId::Codex)),
        )];
        let mut hub = AgentHub::new(backends, Registry::new())
            .with_clock(move || reader.load(std::sync::atomic::Ordering::SeqCst));
        let ids: Vec<SessionId> = (0..4)
            .map(|n| hub.create_session(ProviderId::Codex, format!("/w{n}").into()))
            .collect();
        for (n, id) in ids.iter().enumerate() {
            clock.store(1000 + n as u64 * 10, std::sync::atomic::Ordering::SeqCst);
            hub.send(*id, text("hi")).unwrap();
            settle(&mut hub, |h| idle(h, *id));
        }
        // Four idle sessions: only the two most recent stay warm.
        let ready = |hub: &AgentHub| -> Vec<bool> {
            ids.iter().map(|id| hub.live(*id).unwrap().ready).collect()
        };
        settle(&mut hub, |h| ready(h) == [false, false, true, true]);
        // Past the timeout the rest are released too.
        clock.store(1000 + 400, std::sync::atomic::Ordering::SeqCst);
        settle(&mut hub, |h| ready(h) == [false; 4]);
        assert!(ids.iter().all(|id| !hub.live(*id).unwrap().ready));
        // A message to a released session attaches it again with its history intact.
        hub.send(ids[0], text("back again")).unwrap();
        settle(&mut hub, |h| idle(h, ids[0]));
        let live = hub.live(ids[0]).unwrap();
        assert_eq!(live.model.last_outcome, Some(TurnOutcome::Completed));
        assert!(live.error.is_none(), "{:?}", live.error);
    }

    #[test]
    fn archiving_a_chat_stops_its_turn_and_releases_its_process_at_once() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        hub.send(session, text("slow: working")).unwrap();
        settle(&mut hub, |h| {
            h.live(session).is_some_and(|l| l.turn.is_some())
        });
        // A running turn is not archived out from under the user.
        hub.archive(session);
        assert!(!hub.registry.get(session).unwrap().archived);
        assert!(hub.interrupt(session));
        settle(&mut hub, |h| idle(h, session));
        hub.archive(session);
        settle(&mut hub, |h| !h.live(session).unwrap().ready);
        assert!(hub.registry.get(session).unwrap().archived);
    }

    #[test]
    fn changing_folders_stops_chats_of_the_old_folder_and_keeps_the_new_ones() {
        let mut hub = hub();
        let old = hub.create_session(ProviderId::Claude, "/old".into());
        let kept = hub.create_session(ProviderId::Claude, "/new".into());
        hub.set_slots(2);
        hub.send(old, text("slow: working")).unwrap();
        hub.send(kept, text("hi")).unwrap();
        settle(&mut hub, |h| h.live(old).is_some_and(|l| l.turn.is_some()));
        hub.release_outside(std::path::Path::new("/new"));
        settle(&mut hub, |h| {
            idle(h, old) && !h.live(old).unwrap().ready && idle(h, kept)
        });
        assert_eq!(
            hub.live(old).unwrap().model.last_outcome,
            Some(TurnOutcome::Interrupted)
        );
        assert!(
            hub.live(kept).unwrap().ready,
            "the open folder's chat stays warm"
        );
    }

    #[test]
    fn a_refused_detach_keeps_the_session_for_a_later_release() {
        let mut hub = hub();
        let session = hub.create_session(ProviderId::Claude, "/w".into());
        hub.send(session, text("hi")).unwrap();
        settle(&mut hub, |h| idle(h, session));
        let ready_before = hub.live(session).unwrap().ready;
        hub.settle(
            Pending::Detach(session),
            &OperationOutcome::Failed(AgentError::Busy),
        );
        let live = hub.live(session).unwrap();
        assert_eq!(live.ready, ready_before);
        assert!(!live.detaching);
    }

    #[test]
    fn a_working_or_waiting_session_is_never_released() {
        let clock = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1000));
        let reader = clock.clone();
        let backends: Vec<(ProviderId, Box<dyn AgentBackend>)> = vec![(
            ProviderId::Codex,
            Box::new(FakeBackend::new(ProviderId::Codex)),
        )];
        let mut hub = AgentHub::new(backends, Registry::new())
            .with_clock(move || reader.load(std::sync::atomic::Ordering::SeqCst));
        let waiting = hub.create_session(ProviderId::Codex, "/a".into());
        hub.send(waiting, text("approve: touch")).unwrap();
        settle(&mut hub, |h| {
            h.status(waiting) == SessionStatus::NeedsAttention
        });
        clock.store(1000 + 10_000, std::sync::atomic::Ordering::SeqCst);
        hub.tick();
        assert!(
            hub.live(waiting).unwrap().ready,
            "an unanswered approval keeps its session"
        );
    }
}
