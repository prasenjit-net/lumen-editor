//! The common backend interface: a nonblocking, object-safe command/event API for
//! egui's polling controller. It is an ACP-inspired Rust API, not the ACP wire
//! protocol: there are no JSON messages, method names or provider handles here.

use std::{path::PathBuf, time::Duration};

use crate::{
    AgentError, Capabilities, ClientRequestId, EventEnvelope, NativeSessionRef, OperationId,
    ProfileId, ProviderId, RequestToken, SessionId, SubmitError, TurnId, model::TranscriptItem,
};

/// Bumped when this interface changes incompatibly (ordinary Cargo semver also applies).
pub const CORE_API_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider: ProviderId,
    pub profile: ProfileId,
    /// The installed executable's version, once probed.
    pub executable_version: Option<String>,
    pub core_api_version: u32,
}

impl ProviderDescriptor {
    pub fn new(provider: ProviderId) -> Self {
        Self {
            provider,
            profile: ProfileId::default_profile(),
            executable_version: None,
            core_api_version: CORE_API_VERSION,
        }
    }
}

/// How much work one `poll_events` call may do, so a flood cannot stall a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollBudget {
    pub max_events: usize,
    pub max_bytes: usize,
}

impl PollBudget {
    pub const FRAME: PollBudget = PollBudget {
        max_events: 256,
        max_bytes: 256 * 1024,
    };
}

pub trait AgentBackend: Send {
    fn descriptor(&self) -> &ProviderDescriptor;

    /// Accept a request into the backend's bounded queue. `Ok` means *accepted*,
    /// not executed: every accepted operation later produces exactly one
    /// `AgentEvent::OperationFinished`. Never blocks on I/O.
    fn try_submit(&mut self, request: Request) -> Result<OperationId, SubmitError>;

    /// Collect pending events, at most `budget` worth. Never blocks.
    fn poll_events(&mut self, budget: PollBudget) -> Vec<EventEnvelope>;

    /// Start an asynchronous shutdown: pending requests are settled, owned
    /// processes are stopped, and a final `OperationFinished` arrives. Destructors
    /// are only a last-resort cleanup, never the primary lifecycle.
    fn begin_shutdown(&mut self, deadline: Duration) -> Result<OperationId, SubmitError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub client_request_id: ClientRequestId,
    pub operation: AgentOperation,
}

/// Opaque pagination position; only the backend that issued it can read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRevision(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClientTurnId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ApprovalDecisionId(pub String);

#[derive(Clone, Debug, PartialEq)]
pub enum AgentOperation {
    /// Version, availability, capabilities; never sends a prompt.
    Probe,
    /// Auth state and a safe display label; never tokens.
    ReadAuthState,
    BeginAuthentication {
        method: String,
    },
    CancelAuthentication {
        attempt: crate::AuthAttemptId,
    },
    ListSessions {
        workspace: Option<PathBuf>,
        cursor: Option<Cursor>,
        limit: usize,
    },
    CreateSession {
        session: SessionId,
        workspace: PathBuf,
        config: ConfigPatch,
    },
    AttachSession {
        session: SessionId,
        native: NativeSessionRef,
        workspace: PathBuf,
    },
    ReadHistory {
        session: SessionId,
        cursor: Option<Cursor>,
        limit: usize,
    },
    UpdateSessionMetadata {
        session: SessionId,
        title: String,
    },
    ReadConfig {
        session: SessionId,
    },
    SetConfig {
        session: SessionId,
        expected_revision: ConfigRevision,
        patch: ConfigPatch,
    },
    ListCommands {
        session: SessionId,
    },
    StartTurn {
        session: SessionId,
        client_turn: ClientTurnId,
        input: Vec<InputBlock>,
        expected_revision: Option<ConfigRevision>,
    },
    InterruptTurn {
        session: SessionId,
        turn: TurnId,
    },
    ReplyApproval {
        token: RequestToken,
        decision: ApprovalDecisionId,
    },
    ReplyQuestion {
        token: RequestToken,
        answers: Vec<QuestionAnswer>,
    },
    /// Release an idle runtime/subscriptions; native history is retained.
    DetachSession {
        session: SessionId,
    },
    /// Native archive where supported; never a destructive delete.
    ArchiveSession {
        session: SessionId,
    },
}

/// Content of a user message. Unsupported blocks fail before the prompt is sent;
/// nothing is silently dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputBlock {
    Text(String),
    /// A file the agent may read from disk itself.
    FileReference {
        path: PathBuf,
    },
    /// Text of a file as the user sees it, with where it came from. Context only:
    /// not authority to overwrite the original.
    TextSnapshot {
        origin: String,
        revision: Option<u64>,
        text: String,
    },
    SelectionSnapshot {
        origin: String,
        revision: Option<u64>,
        text: String,
    },
    Image {
        media_type: String,
        bytes: Vec<u8>,
    },
}

/// What a requested config change touches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigPatch {
    pub values: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigScope {
    Profile,
    Session,
    Turn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutability {
    IdleOnly,
    NextTurn,
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigChoice {
    pub value: String,
    pub label: String,
}

/// One advertised setting (model, reasoning effort, permission profile, ...).
/// Provider mode names are not assumed equal across providers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub choices: Vec<ConfigChoice>,
    pub current: String,
    pub scope: ConfigScope,
    pub mutability: Mutability,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub revision: ConfigRevision,
    pub options: Vec<ConfigOption>,
}

impl SessionConfig {
    pub fn get(&self, id: &str) -> Option<&ConfigOption> {
        self.options.iter().find(|option| option.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionAnswer {
    Text(String),
    Choice(String),
    Choices(Vec<String>),
    /// The user dismissed the question.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthState {
    SignedIn { label: String },
    SignedOut,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthChallenge {
    OpenUrl { url: String },
    DeviceCode { url: String, code: String },
    Instructions { text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSummary {
    pub native: NativeSessionRef,
    pub title: Option<String>,
    pub workspace: Option<PathBuf>,
    /// Seconds since the Unix epoch, when the provider reports it.
    pub last_activity: Option<u64>,
    pub archived: bool,
}

/// What an accepted operation produced.
#[derive(Clone, Debug, PartialEq)]
pub enum OperationOutput {
    Ack,
    Probed {
        version: String,
        capabilities: Box<Capabilities>,
        auth: AuthState,
    },
    Auth(AuthState),
    Sessions {
        summaries: Vec<SessionSummary>,
        next: Option<Cursor>,
        /// False when discovery was cut short; absence of a row is then unknown.
        complete: bool,
    },
    SessionReady {
        native: NativeSessionRef,
        config: SessionConfig,
        capabilities: Box<Capabilities>,
    },
    History {
        items: Vec<TranscriptItem>,
        next: Option<Cursor>,
    },
    Config(SessionConfig),
    Commands(Vec<CommandDescriptor>),
    TurnStarted(TurnId),
}

/// The single terminal result of an accepted operation.
#[derive(Clone, Debug, PartialEq)]
pub enum OperationOutcome {
    Succeeded(OperationOutput),
    Failed(AgentError),
    Cancelled,
}
