//! The session model: transcript items plus the turn state machine, and the
//! reducer that folds `EventEnvelope`s into them. The same reducer serves live
//! events and replayed history, rejects impossible transitions and old runtime
//! generations, and never lets a later partial update erase earlier values.

use std::{collections::HashMap, path::PathBuf};

use crate::{
    EventEnvelope, ItemId, RuntimeGeneration, TurnId,
    api::SessionConfig,
    event::{AgentEvent, ApprovalRequest, FileChange, QuestionRequest, TurnOutcome, Usage},
};

/// Longest tool preview kept per call; more is cut with a marker.
pub const MAX_PREVIEW_BYTES: usize = 64 * 1024;
const TRUNCATION_MARKER: &str = "\n… output truncated";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessagePhase {
    /// Working notes between steps.
    Commentary,
    /// The answer to the user.
    Final,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Command,
    FileEdit,
    Read,
    Search,
    Fetch,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Declined,
    Cancelled,
}

impl ToolStatus {
    pub fn is_finished(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub kind: ToolKind,
    pub title: String,
    pub status: ToolStatus,
    /// Bounded text (command output, a diff summary, ...).
    pub preview: String,
    pub locations: Vec<PathBuf>,
    /// A patch the tool proposed or applied, when the provider gave one.
    pub patch: Option<String>,
    /// The tool call this one was started from (a sub-activity), if any.
    pub parent: Option<ItemId>,
    pub exit_code: Option<i32>,
}

impl ToolCall {
    pub fn new(kind: ToolKind, title: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            status: ToolStatus::Running,
            preview: String::new(),
            locations: Vec::new(),
            patch: None,
            parent: None,
            exit_code: None,
        }
    }
}

/// A partial update: only the fields that are `Some` change, so a later update
/// that omits a field never erases an earlier value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolUpdate {
    pub title: Option<String>,
    pub status: Option<ToolStatus>,
    pub append_preview: Option<String>,
    pub locations: Option<Vec<PathBuf>>,
    pub patch: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEntry {
    pub text: String,
    pub status: PlanStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    User(String),
    Agent {
        text: String,
        phase: MessagePhase,
        finished: bool,
    },
    Tool(ToolCall),
    Plan(Vec<PlanEntry>),
    Error(String),
    Notice(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptItem {
    pub id: ItemId,
    pub turn: Option<TurnId>,
    pub kind: ItemKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnPhase {
    Running,
    /// Blocked on an approval or question; nothing else runs for this session.
    WaitingForUser,
}

/// Exactly one turn is active per session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnState {
    Idle,
    Active {
        turn: Option<TurnId>,
        phase: TurnPhase,
    },
    /// The runtime died mid-turn: what happened is unknown. Needs an explicit new
    /// action; it is never assumed completed or cancelled.
    Unknown,
}

/// What the reducer did with an event (for logging and tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    Applied,
    /// The event does not concern the session model (operation results, auth...).
    NotModelEvent,
    StaleGeneration,
    Duplicate,
    /// The event does not fit the current state; ignored and counted.
    Impossible(String),
}

#[derive(Clone, Debug)]
pub struct SessionModel {
    pub items: Vec<TranscriptItem>,
    index: HashMap<ItemId, usize>,
    pub state: TurnState,
    pub pending_approvals: Vec<ApprovalRequest>,
    pub pending_questions: Vec<QuestionRequest>,
    pub title: Option<String>,
    pub config: Option<SessionConfig>,
    pub usage: Option<Usage>,
    pub file_changes: Vec<FileChange>,
    pub last_outcome: Option<TurnOutcome>,
    generation: Option<RuntimeGeneration>,
    last_sequence: u64,
    /// Events ignored for being stale, duplicate or impossible.
    pub ignored: u64,
}

impl Default for SessionModel {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            index: HashMap::new(),
            state: TurnState::Idle,
            pending_approvals: Vec::new(),
            pending_questions: Vec::new(),
            title: None,
            config: None,
            usage: None,
            file_changes: Vec::new(),
            last_outcome: None,
            generation: None,
            last_sequence: 0,
            ignored: 0,
        }
    }
}

fn clip(text: &mut String) {
    if text.len() > MAX_PREVIEW_BYTES {
        let mut end = MAX_PREVIEW_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(TRUNCATION_MARKER);
    }
}

impl SessionModel {
    pub fn item(&self, id: &ItemId) -> Option<&TranscriptItem> {
        self.index.get(id).map(|position| &self.items[*position])
    }

    /// Whether a new turn may start (one active turn per session).
    /// Add the user's own message, which the providers never echo back during a
    /// live turn. Returns false when an item with that id already exists.
    pub fn record_user_message(&mut self, id: ItemId, text: String) -> bool {
        self.push(TranscriptItem {
            id,
            turn: None,
            kind: ItemKind::User(text),
        })
    }

    pub fn can_start_turn(&self) -> bool {
        matches!(self.state, TurnState::Idle | TurnState::Unknown)
    }

    pub fn needs_attention(&self) -> bool {
        !self.pending_approvals.is_empty() || !self.pending_questions.is_empty()
    }

    fn push(&mut self, item: TranscriptItem) -> bool {
        if self.index.contains_key(&item.id) {
            return false;
        }
        self.index.insert(item.id.clone(), self.items.len());
        self.items.push(item);
        true
    }

    fn reject(&mut self, why: impl Into<String>) -> Applied {
        self.ignored += 1;
        Applied::Impossible(why.into())
    }

    /// Fold one event in. Replayed and live events take the same path.
    pub fn apply(&mut self, envelope: &EventEnvelope) -> Applied {
        match self.generation {
            Some(current) if envelope.generation < current => {
                self.ignored += 1;
                return Applied::StaleGeneration;
            }
            Some(current)
                if envelope.generation == current && envelope.sequence <= self.last_sequence =>
            {
                self.ignored += 1;
                return Applied::Duplicate;
            }
            // First event, or a new runtime generation: its sequence restarts.
            _ => {}
        }
        self.generation = Some(envelope.generation);
        self.last_sequence = envelope.sequence;
        self.apply_event(envelope)
    }

    fn apply_event(&mut self, envelope: &EventEnvelope) -> Applied {
        use AgentEvent as E;
        match &envelope.event {
            E::OperationFinished(_)
            | E::AuthStateChanged(_)
            | E::AuthenticationChallenge { .. }
            | E::SessionCreated
            | E::SessionAttached
            | E::SessionDetached
            | E::CapabilitiesChanged(_)
            | E::CommandsChanged(_)
            | E::Diagnostic { .. } => Applied::NotModelEvent,
            E::MetadataChanged { title } => {
                self.title = Some(title.clone());
                Applied::Applied
            }
            E::ConfigChanged(config) => {
                self.config = Some(config.clone());
                Applied::Applied
            }
            E::UsageUpdated(usage) => {
                self.usage = Some(usage.clone());
                Applied::Applied
            }
            E::FileChangesReported(changes) => {
                self.file_changes.extend(changes.iter().cloned());
                Applied::Applied
            }
            E::TurnQueued => Applied::Applied,
            E::TurnStarted => {
                if !self.can_start_turn() {
                    return self.reject("a turn is already active");
                }
                self.state = TurnState::Active {
                    turn: envelope.turn.clone(),
                    phase: TurnPhase::Running,
                };
                self.last_outcome = None;
                Applied::Applied
            }
            E::TurnFinished(outcome) => {
                if matches!(self.state, TurnState::Idle) {
                    return self.reject("no turn is active");
                }
                self.state = TurnState::Idle;
                // Anything still waiting for the user is settled by the end of its turn.
                self.pending_approvals.clear();
                self.pending_questions.clear();
                self.last_outcome = Some(outcome.clone());
                Applied::Applied
            }
            E::MessageStarted { item, phase } => {
                let added = self.push(TranscriptItem {
                    id: item.clone(),
                    turn: envelope.turn.clone(),
                    kind: ItemKind::Agent {
                        text: String::new(),
                        phase: *phase,
                        finished: false,
                    },
                });
                if added { Applied::Applied } else { self.dup() }
            }
            E::TextDelta { item, delta } => match self.index.get(item).copied() {
                Some(position) => match &mut self.items[position].kind {
                    ItemKind::Agent { text, .. } => {
                        text.push_str(delta);
                        Applied::Applied
                    }
                    _ => self.reject("a text delta targets a non-message item"),
                },
                None => self.reject("a text delta targets an unknown message"),
            },
            E::MessageFinished { item } => match self.index.get(item).copied() {
                Some(position) => match &mut self.items[position].kind {
                    ItemKind::Agent { finished, .. } => {
                        *finished = true;
                        Applied::Applied
                    }
                    _ => self.reject("only a message can finish"),
                },
                None => self.reject("an unknown message finished"),
            },
            E::ToolStarted { item, call } => {
                let mut call = call.clone();
                clip(&mut call.preview);
                let added = self.push(TranscriptItem {
                    id: item.clone(),
                    turn: envelope.turn.clone(),
                    kind: ItemKind::Tool(call),
                });
                if added { Applied::Applied } else { self.dup() }
            }
            E::ToolUpdated { item, update } => match self.index.get(item).copied() {
                Some(position) => match &mut self.items[position].kind {
                    ItemKind::Tool(call) => {
                        if let Some(title) = &update.title {
                            call.title = title.clone();
                        }
                        if let Some(status) = update.status {
                            call.status = status;
                        }
                        if let Some(more) = &update.append_preview {
                            call.preview.push_str(more);
                            clip(&mut call.preview);
                        }
                        if let Some(locations) = &update.locations {
                            call.locations = locations.clone();
                        }
                        if let Some(patch) = &update.patch {
                            call.patch = Some(patch.clone());
                        }
                        if let Some(code) = update.exit_code {
                            call.exit_code = Some(code);
                        }
                        Applied::Applied
                    }
                    _ => self.reject("a tool update targets a non-tool item"),
                },
                None => self.reject("a tool update targets an unknown tool call"),
            },
            E::ToolFinished { item, status } => match self.index.get(item).copied() {
                Some(position) => match &mut self.items[position].kind {
                    ItemKind::Tool(call) => {
                        call.status = *status;
                        Applied::Applied
                    }
                    _ => self.reject("only a tool call can finish"),
                },
                None => self.reject("an unknown tool call finished"),
            },
            E::PlanUpdated { item, entries } => match self.index.get(item).copied() {
                Some(position) => {
                    self.items[position].kind = ItemKind::Plan(entries.clone());
                    Applied::Applied
                }
                None => {
                    self.push(TranscriptItem {
                        id: item.clone(),
                        turn: envelope.turn.clone(),
                        kind: ItemKind::Plan(entries.clone()),
                    });
                    Applied::Applied
                }
            },
            E::ApprovalRequested(request) => {
                if request.token.generation != envelope.generation {
                    return self.reject("an approval from another runtime generation");
                }
                if !matches!(self.state, TurnState::Active { .. }) {
                    return self.reject("an approval outside an active turn");
                }
                if self
                    .pending_approvals
                    .iter()
                    .any(|p| p.token == request.token)
                {
                    return self.dup();
                }
                self.pending_approvals.push(request.clone());
                self.set_phase(TurnPhase::WaitingForUser);
                Applied::Applied
            }
            E::ApprovalResolved { token } => {
                let before = self.pending_approvals.len();
                self.pending_approvals.retain(|p| p.token != *token);
                if self.pending_approvals.len() == before {
                    return self.reject("an unknown approval was resolved");
                }
                self.settle_phase();
                Applied::Applied
            }
            E::QuestionRequested(request) => {
                if request.token.generation != envelope.generation {
                    return self.reject("a question from another runtime generation");
                }
                if !matches!(self.state, TurnState::Active { .. }) {
                    return self.reject("a question outside an active turn");
                }
                if self
                    .pending_questions
                    .iter()
                    .any(|p| p.token == request.token)
                {
                    return self.dup();
                }
                self.pending_questions.push(request.clone());
                self.set_phase(TurnPhase::WaitingForUser);
                Applied::Applied
            }
            E::QuestionResolved { token } => {
                let before = self.pending_questions.len();
                self.pending_questions.retain(|p| p.token != *token);
                if self.pending_questions.len() == before {
                    return self.reject("an unknown question was resolved");
                }
                self.settle_phase();
                Applied::Applied
            }
            E::RuntimeExited { reason } => {
                if matches!(self.state, TurnState::Active { .. }) {
                    self.state = TurnState::Unknown;
                }
                self.pending_approvals.clear();
                self.pending_questions.clear();
                let id = ItemId(format!("runtime-exit-{}", envelope.sequence));
                self.push(TranscriptItem {
                    id,
                    turn: envelope.turn.clone(),
                    kind: ItemKind::Notice(format!("The agent process ended: {reason}")),
                });
                Applied::Applied
            }
        }
    }

    fn dup(&mut self) -> Applied {
        self.ignored += 1;
        Applied::Duplicate
    }

    fn set_phase(&mut self, new: TurnPhase) {
        if let TurnState::Active { phase, .. } = &mut self.state {
            *phase = new;
        }
    }

    /// Back to Running once nothing waits for the user.
    fn settle_phase(&mut self) {
        if !self.needs_attention() {
            self.set_phase(TurnPhase::Running);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ProfileId, ProviderId, RequestToken, SessionId,
        api::ApprovalDecisionId,
        event::{ApprovalChoice, RiskLevel},
    };

    fn envelope(generation: u64, sequence: u64, event: AgentEvent) -> EventEnvelope {
        EventEnvelope {
            provider: ProviderId::Codex,
            profile: ProfileId::default_profile(),
            generation: RuntimeGeneration(generation),
            operation: None,
            session: Some(SessionId(1)),
            turn: Some(TurnId("t1".into())),
            sequence,
            event,
        }
    }

    fn token(generation: u64, native: &str) -> RequestToken {
        RequestToken {
            provider: ProviderId::Codex,
            profile: ProfileId::default_profile(),
            session: SessionId(1),
            turn: TurnId("t1".into()),
            generation: RuntimeGeneration(generation),
            native: native.into(),
        }
    }

    fn approval(generation: u64, native: &str) -> AgentEvent {
        AgentEvent::ApprovalRequested(ApprovalRequest {
            token: token(generation, native),
            summary: "run a command".into(),
            command: Some("touch x".into()),
            paths: vec![],
            choices: vec![ApprovalChoice {
                id: ApprovalDecisionId("accept".into()),
                label: "Allow once".into(),
                scope: "this command".into(),
                allows: true,
                risk: RiskLevel::Medium,
            }],
        })
    }

    fn started(model: &mut SessionModel) {
        assert_eq!(
            model.apply(&envelope(1, 1, AgentEvent::TurnStarted)),
            Applied::Applied
        );
    }

    #[test]
    fn a_turn_streams_a_message_and_finishes_once() {
        let mut m = SessionModel::default();
        assert!(m.can_start_turn());
        started(&mut m);
        assert!(!m.can_start_turn(), "one active turn per session");
        let id = ItemId("m1".into());
        m.apply(&envelope(
            1,
            2,
            AgentEvent::MessageStarted {
                item: id.clone(),
                phase: MessagePhase::Final,
            },
        ));
        m.apply(&envelope(
            1,
            3,
            AgentEvent::TextDelta {
                item: id.clone(),
                delta: "he".into(),
            },
        ));
        m.apply(&envelope(
            1,
            4,
            AgentEvent::TextDelta {
                item: id.clone(),
                delta: "llo".into(),
            },
        ));
        m.apply(&envelope(
            1,
            5,
            AgentEvent::MessageFinished { item: id.clone() },
        ));
        assert!(
            matches!(&m.item(&id).unwrap().kind, ItemKind::Agent { text, finished: true, .. } if text == "hello")
        );
        m.apply(&envelope(
            1,
            6,
            AgentEvent::TurnFinished(TurnOutcome::Completed),
        ));
        assert!(m.can_start_turn() && m.last_outcome == Some(TurnOutcome::Completed));
        assert!(matches!(
            m.apply(&envelope(
                1,
                7,
                AgentEvent::TurnFinished(TurnOutcome::Completed)
            )),
            Applied::Impossible(_)
        ));
    }

    #[test]
    fn stale_generations_and_duplicate_sequences_are_ignored_and_counted() {
        let mut m = SessionModel::default();
        m.apply(&envelope(2, 5, AgentEvent::TurnStarted));
        assert_eq!(
            m.apply(&envelope(
                1,
                99,
                AgentEvent::TurnFinished(TurnOutcome::Completed)
            )),
            Applied::StaleGeneration
        );
        assert_eq!(
            m.apply(&envelope(
                2,
                5,
                AgentEvent::TurnFinished(TurnOutcome::Completed)
            )),
            Applied::Duplicate
        );
        assert_eq!(
            m.apply(&envelope(
                2,
                4,
                AgentEvent::TurnFinished(TurnOutcome::Completed)
            )),
            Applied::Duplicate
        );
        assert!(!m.can_start_turn(), "ignored events changed nothing");
        assert_eq!(m.ignored, 3);
        // A newer generation restarts the sequence.
        assert_eq!(
            m.apply(&envelope(
                3,
                1,
                AgentEvent::TurnFinished(TurnOutcome::Completed)
            )),
            Applied::Applied
        );
    }

    #[test]
    fn replayed_items_do_not_duplicate_and_deltas_to_unknown_items_are_rejected() {
        let mut m = SessionModel::default();
        started(&mut m);
        let id = ItemId("m1".into());
        let event = AgentEvent::MessageStarted {
            item: id.clone(),
            phase: MessagePhase::Final,
        };
        assert_eq!(m.apply(&envelope(1, 2, event.clone())), Applied::Applied);
        assert_eq!(m.apply(&envelope(1, 3, event)), Applied::Duplicate);
        assert_eq!(m.items.len(), 1);
        assert!(matches!(
            m.apply(&envelope(
                1,
                4,
                AgentEvent::TextDelta {
                    item: ItemId("nope".into()),
                    delta: "x".into()
                }
            )),
            Applied::Impossible(_)
        ));
    }

    #[test]
    fn tool_updates_merge_without_erasing_earlier_values_and_previews_are_bounded() {
        let mut m = SessionModel::default();
        started(&mut m);
        let id = ItemId("tool1".into());
        let mut call = ToolCall::new(ToolKind::Command, "Preparing…");
        call.locations = vec![PathBuf::from("a.rs")];
        m.apply(&envelope(
            1,
            2,
            AgentEvent::ToolStarted {
                item: id.clone(),
                call,
            },
        ));
        m.apply(&envelope(
            1,
            3,
            AgentEvent::ToolUpdated {
                item: id.clone(),
                update: ToolUpdate {
                    title: Some("cargo build".into()),
                    append_preview: Some("compiling\n".into()),
                    ..Default::default()
                },
            },
        ));
        m.apply(&envelope(
            1,
            4,
            AgentEvent::ToolUpdated {
                item: id.clone(),
                update: ToolUpdate {
                    exit_code: Some(0),
                    ..Default::default()
                },
            },
        ));
        let ItemKind::Tool(call) = &m.item(&id).unwrap().kind else {
            panic!()
        };
        assert_eq!(call.title, "cargo build");
        assert_eq!(
            call.locations,
            vec![PathBuf::from("a.rs")],
            "an update without locations keeps them"
        );
        assert_eq!(
            (call.preview.as_str(), call.exit_code),
            ("compiling\n", Some(0))
        );
        m.apply(&envelope(
            1,
            5,
            AgentEvent::ToolUpdated {
                item: id.clone(),
                update: ToolUpdate {
                    append_preview: Some("x".repeat(MAX_PREVIEW_BYTES * 2)),
                    ..Default::default()
                },
            },
        ));
        let ItemKind::Tool(call) = &m.item(&id).unwrap().kind else {
            panic!()
        };
        assert!(call.preview.len() <= MAX_PREVIEW_BYTES + TRUNCATION_MARKER.len());
        assert!(call.preview.ends_with("truncated"));
        m.apply(&envelope(
            1,
            6,
            AgentEvent::ToolFinished {
                item: id.clone(),
                status: ToolStatus::Completed,
            },
        ));
        let ItemKind::Tool(call) = &m.item(&id).unwrap().kind else {
            panic!()
        };
        assert!(call.status.is_finished());
    }

    #[test]
    fn approvals_pause_the_turn_resolve_individually_and_do_not_outlive_it() {
        let mut m = SessionModel::default();
        started(&mut m);
        assert_eq!(
            m.apply(&envelope(1, 2, approval(1, "r1"))),
            Applied::Applied
        );
        assert_eq!(
            m.apply(&envelope(1, 3, approval(1, "r1"))),
            Applied::Duplicate
        );
        m.apply(&envelope(1, 4, approval(1, "r2")));
        assert!(m.needs_attention());
        assert_eq!(
            m.state,
            TurnState::Active {
                turn: Some(TurnId("t1".into())),
                phase: TurnPhase::WaitingForUser
            }
        );
        m.apply(&envelope(
            1,
            5,
            AgentEvent::ApprovalResolved {
                token: token(1, "r1"),
            },
        ));
        assert!(
            matches!(
                m.state,
                TurnState::Active {
                    phase: TurnPhase::WaitingForUser,
                    ..
                }
            ),
            "r2 still waits"
        );
        assert!(matches!(
            m.apply(&envelope(
                1,
                6,
                AgentEvent::ApprovalResolved {
                    token: token(1, "zzz")
                }
            )),
            Applied::Impossible(_)
        ));
        m.apply(&envelope(
            1,
            7,
            AgentEvent::ApprovalResolved {
                token: token(1, "r2"),
            },
        ));
        assert!(matches!(
            m.state,
            TurnState::Active {
                phase: TurnPhase::Running,
                ..
            }
        ));
        m.apply(&envelope(1, 8, approval(1, "r3")));
        m.apply(&envelope(
            1,
            9,
            AgentEvent::TurnFinished(TurnOutcome::Interrupted),
        ));
        assert!(
            !m.needs_attention(),
            "the end of a turn settles its requests"
        );
    }

    #[test]
    fn approvals_from_the_wrong_generation_or_outside_a_turn_are_rejected() {
        let mut m = SessionModel::default();
        assert!(
            matches!(
                m.apply(&envelope(1, 1, approval(1, "r1"))),
                Applied::Impossible(_)
            ),
            "no active turn"
        );
        assert_eq!(
            m.apply(&envelope(1, 2, AgentEvent::TurnStarted)),
            Applied::Applied
        );
        assert!(
            matches!(
                m.apply(&envelope(1, 3, approval(7, "r2"))),
                Applied::Impossible(_)
            ),
            "token generation differs"
        );
        assert!(!m.needs_attention());
    }

    #[test]
    fn a_runtime_exit_mid_turn_leaves_the_outcome_unknown_not_completed() {
        let mut m = SessionModel::default();
        started(&mut m);
        m.apply(&envelope(1, 2, approval(1, "r1")));
        m.apply(&envelope(
            1,
            3,
            AgentEvent::RuntimeExited {
                reason: "killed".into(),
            },
        ));
        assert_eq!(m.state, TurnState::Unknown);
        assert!(!m.needs_attention());
        assert!(
            m.can_start_turn(),
            "an explicit new action may start a fresh turn"
        );
        assert!(
            matches!(&m.items.last().unwrap().kind, ItemKind::Notice(text) if text.contains("killed"))
        );
        assert_eq!(m.last_outcome, None);
    }

    #[test]
    fn metadata_config_usage_and_file_changes_are_recorded() {
        use crate::event::{FileChange, FileChangeKind};
        let mut m = SessionModel::default();
        m.apply(&envelope(
            1,
            1,
            AgentEvent::MetadataChanged {
                title: "Fix bug".into(),
            },
        ));
        m.apply(&envelope(
            1,
            2,
            AgentEvent::UsageUpdated(Usage {
                total_tokens: Some(10),
                ..Default::default()
            }),
        ));
        m.apply(&envelope(
            1,
            3,
            AgentEvent::FileChangesReported(vec![FileChange {
                path: PathBuf::from("a.rs"),
                kind: FileChangeKind::Modified,
                applied: true,
            }]),
        ));
        assert_eq!(m.title.as_deref(), Some("Fix bug"));
        assert_eq!(m.usage.as_ref().unwrap().total_tokens, Some(10));
        assert_eq!(m.file_changes.len(), 1);
        assert_eq!(
            m.apply(&envelope(1, 4, AgentEvent::SessionCreated)),
            Applied::NotModelEvent
        );
    }
}
