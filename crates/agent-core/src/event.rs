//! Events a backend delivers. Ordered per runtime generation and tagged with the
//! session/turn they belong to; there is no promised chronology across providers.

use std::path::PathBuf;

use crate::{
    AgentError, Capabilities, ItemId, OperationId, ProfileId, ProviderId, RequestToken,
    RuntimeGeneration, SessionId, TurnId,
    api::{AuthChallenge, AuthState, CommandDescriptor, OperationOutcome, SessionConfig},
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
    /// Set when the approval is for a plan the agent wants to start on.
    pub plan: Option<PlanReview>,
}

/// A plan the agent proposes, to be read before it is approved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanReview {
    /// The plan, as Markdown.
    pub text: String,
    /// Where the agent saved it, when it says.
    pub file: Option<PathBuf>,
}

impl PlanReview {
    /// The plan's name, from its first heading.
    pub fn title(&self) -> Option<String> {
        plan_title(&self.text)
    }

    /// The plan without a heading that opens it: that heading is its title,
    /// shown on its own.
    pub fn body(&self) -> &str {
        let text = self.text.trim_start();
        match text.split_once('\n') {
            Some((first, rest)) if first.starts_with('#') => rest.trim_start(),
            None if text.starts_with('#') => "",
            _ => text,
        }
    }
}

/// A Markdown plan's name: its first heading, without a leading "Plan:" or
/// code and bold marks, and at most 120 characters.
pub fn plan_title(plan: &str) -> Option<String> {
    let heading = plan
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with('#'))?
        .trim_start_matches('#')
        .replace("**", "")
        .replace('`', "");
    let heading = heading.trim();
    let name = heading
        .get(..5)
        .filter(|start| start.eq_ignore_ascii_case("plan:"))
        .map_or(heading, |_| heading[5..].trim());
    (!name.is_empty()).then(|| name.chars().take(120).collect())
}

/// Questions the agent asks together; they are answered (or skipped) as one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionRequest {
    pub token: RequestToken,
    pub questions: Vec<Question>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// What the provider uses to match the answer to the question.
    pub id: String,
    /// A short label for the question, possibly empty.
    pub header: String,
    pub prompt: String,
    /// Suggested answers; empty when only a typed answer makes sense.
    pub options: Vec<QuestionOption>,
    /// Whether several options can be chosen.
    pub multiple: bool,
    pub typed: TypedAnswer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    /// What choosing it means, possibly empty.
    pub description: String,
}

/// Whether a question also takes an answer in the user's own words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedAnswer {
    None,
    Text,
    /// Typed, but hidden as it is entered (a password or token).
    Secret,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_is_named_by_its_first_heading() {
        assert_eq!(
            plan_title("intro\n# Plan: `scan` – a **CLI**\n## Context").as_deref(),
            Some("scan – a CLI"),
            "code and bold marks are not shown as text"
        );
        assert_eq!(
            plan_title("## Rollout steps").as_deref(),
            Some("Rollout steps")
        );
        assert_eq!(plan_title("no heading at all"), None);
        assert_eq!(plan_title("# Plan:"), None);
        let review = PlanReview {
            text: format!("# {}", "x".repeat(300)),
            file: None,
        };
        assert_eq!(review.title().unwrap().chars().count(), 120);
    }

    #[test]
    fn the_body_starts_after_a_heading_that_names_the_plan() {
        let plan = |text: &str| PlanReview {
            text: text.into(),
            file: None,
        };
        assert_eq!(
            plan("\n# Plan: x\n\n## Context\nWhy.").body(),
            "## Context\nWhy."
        );
        assert_eq!(
            plan("Intro first.\n# Heading").body(),
            "Intro first.\n# Heading"
        );
        assert_eq!(plan("# Only a title").body(), "");
    }
}
