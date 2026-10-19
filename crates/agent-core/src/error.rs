//! Typed errors. A failed or unknown outcome is always surfaced to the user; the
//! library never retries a side-effecting request on its own.

use std::{fmt, time::Duration};

/// Why a request was not even accepted into a backend's queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmitError {
    /// The bounded command queue is full; try again later.
    QueueFull,
    /// The request is malformed (empty input, unknown id, ...).
    Invalid(String),
    /// `begin_shutdown` was called; nothing new is accepted.
    ShuttingDown,
    /// The same `client_request_id` was already submitted.
    Duplicate,
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => write!(f, "the agent is busy; try again in a moment"),
            Self::Invalid(why) => write!(f, "invalid request: {why}"),
            Self::ShuttingDown => write!(f, "the agent is shutting down"),
            Self::Duplicate => write!(f, "that request was already submitted"),
        }
    }
}

/// How an accepted operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    /// The capability is unsupported or version-gated; `reason` says which.
    Unsupported {
        reason: String,
    },
    /// The provider's executable is not installed or not runnable.
    ProviderMissing {
        detail: String,
    },
    /// The installed version is outside the tested range.
    IncompatibleVersion {
        found: String,
        detail: String,
    },
    /// Sign in with the provider first.
    AuthenticationRequired,
    /// The session already has an active turn.
    Busy,
    NotFound {
        what: String,
    },
    InvalidConfig {
        detail: String,
    },
    RateLimited {
        retry_after: Option<Duration>,
    },
    /// The connection or child failed, or sent something we cannot interpret.
    Transport {
        detail: String,
    },
    Timeout,
    /// The target (turn, request, config revision) is no longer current.
    Stale {
        what: String,
    },
    PermissionDenied {
        detail: String,
    },
    /// The outcome of a side-effecting request is unknown (for example the child
    /// died after we sent it). Never resent automatically.
    Ambiguous {
        detail: String,
    },
    /// Anything the provider reported that has no better category.
    Provider {
        message: String,
    },
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { reason } => write!(f, "not supported: {reason}"),
            Self::ProviderMissing { detail } => write!(f, "provider not found: {detail}"),
            Self::IncompatibleVersion { found, detail } => {
                write!(f, "unsupported version {found}: {detail}")
            }
            Self::AuthenticationRequired => write!(f, "sign in required"),
            Self::Busy => write!(f, "the session is already working on a message"),
            Self::NotFound { what } => write!(f, "{what} was not found"),
            Self::InvalidConfig { detail } => write!(f, "invalid configuration: {detail}"),
            Self::RateLimited {
                retry_after: Some(wait),
            } => {
                write!(f, "rate limited; retry in {}s", wait.as_secs())
            }
            Self::RateLimited { retry_after: None } => write!(f, "rate limited"),
            Self::Transport { detail } => write!(f, "connection problem: {detail}"),
            Self::Timeout => write!(f, "timed out"),
            Self::Stale { what } => write!(f, "{what} is no longer current"),
            Self::PermissionDenied { detail } => write!(f, "permission denied: {detail}"),
            Self::Ambiguous { detail } => {
                write!(f, "outcome unknown ({detail}); check before retrying")
            }
            Self::Provider { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AgentError {}
impl std::error::Error for SubmitError {}
