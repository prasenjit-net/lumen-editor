//! Events a backend delivers. Ordered per runtime generation and tagged with the
//! session/turn they belong to; there is no promised chronology across providers.

use std::path::PathBuf;

use crate::{
    AgentError, Capabilities, ItemId, OperationId, ProfileId, ProviderId, RequestToken,
    RuntimeGeneration, SessionId, TurnId,
    api::{
        AuthChallenge, AuthState, CommandDescriptor, OperationOutcome, QuestionAnswer,
        SessionConfig,
    },
    model::{MessagePhase, PlanEntry, ToolCall, ToolStatus, ToolUpdate},
};

#[derive(Clone, Debug, PartialEq)]
pub struct EventEnvelope {
    pub provider: ProviderId,
    pub profile: ProfileId,
    pub generation: RuntimeGeneration,
    pub operation: Option<OperationId>,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    /// Strictly increasing within one generation of one backend.
    pub sequence: u64,
    pub event: AgentEvent,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    OperationFinished(OperationOutcome),
    AuthStateChanged(AuthState),
    AuthenticationChallenge {
        attempt: crate::AuthAttemptId,
        challenge: AuthChallenge,
    },

    SessionCreated,
    SessionAttached,
    SessionDetached,
    MetadataChanged {
        title: String,
    },
    ConfigChanged(SessionConfig),
    CapabilitiesChanged(Box<Capabilities>),
    CommandsChanged(Vec<CommandDescriptor>),

    TurnQueued,
    TurnStarted,
    MessageStarted {
        item: ItemId,
        phase: MessagePhase,
    },
    TextDelta {
        item: ItemId,
        delta: String,
    },
    MessageFinished {
        item: ItemId,
    },
    ToolStarted {
        item: ItemId,
        call: ToolCall,
    },
    ToolUpdated {
        item: ItemId,
        update: ToolUpdate,
    },
    ToolFinished {
        item: ItemId,
        status: ToolStatus,
    },
    PlanUpdated {
        item: ItemId,
        entries: Vec<PlanEntry>,
    },
    UsageUpdated(Usage),

    ApprovalRequested(ApprovalRequest),
    ApprovalResolved {
        token: RequestToken,
    },
    QuestionRequested(QuestionRequest),
    QuestionResolved {
        token: RequestToken,
    },

    FileChangesReported(Vec<FileChange>),
    TurnFinished(TurnOutcome),
    Diagnostic {
        level: DiagnosticLevel,
        message: String,
    },
    RuntimeExited {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

/// Provider-reported usage, never normalized into an invented price.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub context_window: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Unknown,
}

/// One answer the provider offers for an approval, with its scope spelled out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalChoice {
    pub id: crate::api::ApprovalDecisionId,
    pub label: String,
    /// What approving this covers ("this command", "this session", ...).
    pub scope: String,
    pub allows: bool,
    pub risk: RiskLevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRequest {
    pub token: RequestToken,
    pub summary: String,
    pub command: Option<String>,
    pub paths: Vec<PathBuf>,
    pub choices: Vec<ApprovalChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionKind {
    Text,
    SingleChoice(Vec<String>),
    MultiChoice(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionRequest {
    pub token: RequestToken,
    pub prompt: String,
    pub kind: QuestionKind,
}

/// A file change as reported by the provider (not Lumen's own disk comparison).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
    /// Proposed or already applied, as the provider says.
    pub applied: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

/// How a turn ended. A runtime failure may leave the real state unknown; that is
/// `Failed` with an ambiguous error, never assumed completed or cancelled.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed(AgentError),
    /// A usage or rate limit stopped it.
    Limited(Option<String>),
    /// The provider declined to continue.
    Refused(Option<String>),
}

impl TurnOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// An answer set a UI collected, kept next to the event types it answers.
pub type Answers = Vec<QuestionAnswer>;
