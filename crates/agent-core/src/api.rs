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

/// A requested settings change. Evaluated as a whole: either every part applies
/// or none does. `resets` clear an override so the setting follows the provider's
/// own default again, which is different from choosing a literal value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigPatch {
    pub values: Vec<(String, String)>,
    pub resets: Vec<String>,
}

impl ConfigPatch {
    pub fn set(id: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            values: vec![(id.into(), value.into())],
            resets: Vec::new(),
        }
    }

    pub fn reset(id: impl Into<String>) -> Self {
        Self {
            values: Vec::new(),
            resets: vec![id.into()],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.resets.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigScope {
    Profile,
    Session,
    Turn,
}

/// When a change takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutability {
    /// Only while no turn is running.
    IdleOnly,
    /// Staged now, used from the next turn.
    NextTurn,
    Live,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfigKind {
    #[default]
    Select,
    /// On/off; the choices are `"true"` and `"false"`.
    Toggle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigChoice {
    pub value: String,
    pub label: String,
    pub description: String,
}

impl ConfigChoice {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: String::new(),
        }
    }

    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }
}

/// One advertised setting (model, reasoning effort, permission profile, ...).
/// Provider mode names are not assumed equal across providers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub kind: ConfigKind,
    pub choices: Vec<ConfigChoice>,
    /// What Lumen will use on the next turn (the user's choice, or the default).
    pub current: String,
    /// What a reset returns to; `None` means the setting cannot be reset.
    pub default: Option<String>,
    /// What the provider reported actually using, when known. It can differ from
    /// `current` (a staged change, or a value outside the offered choices).
    pub effective: Option<String>,
    /// Why the setting cannot be changed right now, if it cannot.
    pub disabled: Option<String>,
    pub scope: ConfigScope,
    pub mutability: Mutability,
}

impl ConfigOption {
    pub fn select(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        choices: Vec<ConfigChoice>,
        current: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            kind: ConfigKind::Select,
            choices,
            current: current.into(),
            default: None,
            effective: None,
            disabled: None,
            scope: ConfigScope::Session,
            mutability: Mutability::NextTurn,
        }
    }

    pub fn toggle(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        on: bool,
    ) -> Self {
        let mut option = Self::select(
            id,
            label,
            description,
            vec![
                ConfigChoice::new("true", "On"),
                ConfigChoice::new("false", "Off"),
            ],
            if on { "true" } else { "false" },
        );
        option.kind = ConfigKind::Toggle;
        option.default = Some("false".into());
        option
    }

    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    pub fn is_on(&self) -> bool {
        self.current == "true"
    }
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

    /// Check a patch against the offered options without changing anything. Every
    /// part must be valid and editable, so a bad part rejects the whole patch.
    pub fn validate(&self, patch: &ConfigPatch) -> Result<(), crate::AgentError> {
        let invalid = |detail: String| crate::AgentError::InvalidConfig { detail };
        for (id, value) in &patch.values {
            let option = self
                .get(id)
                .ok_or_else(|| invalid(format!("there is no setting called {id}")))?;
            if let Some(reason) = &option.disabled {
                return Err(invalid(format!(
                    "{} cannot be changed: {reason}",
                    option.label
                )));
            }
            if !option.choices.iter().any(|choice| choice.value == *value) {
                return Err(invalid(format!("{} cannot be {value}", option.label)));
            }
        }
        for id in &patch.resets {
            let option = self
                .get(id)
                .ok_or_else(|| invalid(format!("there is no setting called {id}")))?;
            if let Some(reason) = &option.disabled {
                return Err(invalid(format!(
                    "{} cannot be changed: {reason}",
                    option.label
                )));
            }
            if option.default.is_none() {
                return Err(invalid(format!(
                    "{} has no default to return to",
                    option.label
                )));
            }
        }
        Ok(())
    }

    /// The configuration after a patch, with a higher revision. Nothing changes
    /// when the patch is invalid.
    pub fn applying(&self, patch: &ConfigPatch) -> Result<SessionConfig, crate::AgentError> {
        self.validate(patch)?;
        let mut next = self.clone();
        for id in &patch.resets {
            if let Some(option) = next.options.iter_mut().find(|o| &o.id == id)
                && let Some(default) = option.default.clone()
            {
                option.current = default;
            }
        }
        for (id, value) in &patch.values {
            if let Some(option) = next.options.iter_mut().find(|o| &o.id == id) {
                option.current = value.clone();
            }
        }
        next.revision = ConfigRevision(self.revision.0 + 1);
        Ok(next)
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

#[cfg(test)]
mod config_tests {
    use super::*;

    fn config() -> SessionConfig {
        let mut effort = ConfigOption::select(
            "effort",
            "Effort",
            "",
            vec![
                ConfigChoice::new("low", "Low"),
                ConfigChoice::new("high", "High"),
            ],
            "low",
        );
        effort.default = None;
        let mut locked = ConfigOption::toggle("fast", "Fast", "", false);
        locked.disabled = Some("this model has no fast mode".into());
        SessionConfig {
            revision: ConfigRevision(3),
            options: vec![
                ConfigOption::select(
                    "model",
                    "Model",
                    "",
                    vec![ConfigChoice::new("a", "A"), ConfigChoice::new("b", "B")],
                    "a",
                )
                .with_default("a"),
                effort,
                locked,
            ],
        }
    }

    #[test]
    fn a_valid_patch_applies_everything_and_bumps_the_revision() {
        let next = config()
            .applying(&ConfigPatch {
                values: vec![
                    ("model".into(), "b".into()),
                    ("effort".into(), "high".into()),
                ],
                resets: Vec::new(),
            })
            .unwrap();
        assert_eq!(next.revision, ConfigRevision(4));
        assert_eq!(next.get("model").unwrap().current, "b");
        assert_eq!(next.get("effort").unwrap().current, "high");
    }

    #[test]
    fn one_bad_part_rejects_the_whole_patch_and_changes_nothing() {
        let before = config();
        for patch in [
            ConfigPatch {
                values: vec![
                    ("model".into(), "b".into()),
                    ("effort".into(), "ultra".into()),
                ],
                resets: Vec::new(),
            },
            ConfigPatch::set("nope", "x"),
            ConfigPatch::set("fast", "true"),
            ConfigPatch::reset("effort"),
            ConfigPatch::reset("fast"),
        ] {
            assert!(
                matches!(
                    before.applying(&patch),
                    Err(crate::AgentError::InvalidConfig { .. })
                ),
                "{patch:?}"
            );
        }
        assert_eq!(before, config());
    }

    #[test]
    fn reset_returns_to_the_default_and_differs_from_choosing_a_value() {
        let changed = config().applying(&ConfigPatch::set("model", "b")).unwrap();
        let reset = changed.applying(&ConfigPatch::reset("model")).unwrap();
        assert_eq!(reset.get("model").unwrap().current, "a");
        assert_eq!(reset.revision, ConfigRevision(5));
        assert!(ConfigPatch::default().is_empty());
    }
}
