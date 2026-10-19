//! Opaque identities. Nothing here is an index or a provider-vector slot: a tab, a
//! queue entry or a request is always addressed by one of these.

use std::fmt;

/// The two supported providers. Fixed product support, not a plugin point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProviderId {
    Codex,
    Claude,
}

impl ProviderId {
    pub const ALL: [ProviderId; 2] = [Self::Codex, Self::Claude];

    /// Stable machine name used in saved records.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    /// Name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.as_str() == text)
    }
}

macro_rules! counter_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

counter_id!(
    /// Lumen-local conversation identity. Exists before any native session does and
    /// keeps tab identity stable across restarts of the runtime.
    SessionId
);
counter_id!(
    /// One accepted operation; it ends exactly once.
    OperationId
);
counter_id!(
    /// Chosen by the caller to deduplicate submissions locally.
    ClientRequestId
);
counter_id!(
    /// One incarnation of a runtime (child process or connection). Events and
    /// request tokens from an older generation are rejected.
    RuntimeGeneration
);
counter_id!(
    /// A sign-in attempt that can be cancelled.
    AuthAttemptId
);

/// An executable/install and authentication context. One per provider initially.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProfileId(pub String);

impl ProfileId {
    pub fn default_profile() -> Self {
        Self("default".into())
    }
}

/// Provider-assigned turn identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TurnId(pub String);

/// A provider's own session/thread identity, namespaced so it can never be mixed up
/// with another provider's or another profile's.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeSessionRef {
    pub provider: ProviderId,
    pub profile: ProfileId,
    pub id: String,
}

/// Identity of a message, tool call, plan or other transcript item within a session.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId(pub String);

/// Binds an approval or question to the exact provider/profile/session/turn and
/// runtime generation it was raised in. A reply with a stale token is rejected.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestToken {
    pub provider: ProviderId,
    pub profile: ProfileId,
    pub session: SessionId,
    pub turn: TurnId,
    pub generation: RuntimeGeneration,
    /// The provider's own request id (opaque to the app).
    pub native: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_round_trip_through_their_stable_names() {
        for provider in ProviderId::ALL {
            assert_eq!(ProviderId::parse(provider.as_str()), Some(provider));
        }
        assert_eq!(ProviderId::parse("gemini"), None);
        assert_eq!(ProviderId::Claude.label(), "Claude Code");
    }

    #[test]
    fn native_refs_of_different_providers_or_profiles_never_compare_equal() {
        let make = |provider, profile: &str| NativeSessionRef {
            provider,
            profile: ProfileId(profile.into()),
            id: "abc".into(),
        };
        assert_ne!(
            make(ProviderId::Codex, "default"),
            make(ProviderId::Claude, "default")
        );
        assert_ne!(
            make(ProviderId::Codex, "default"),
            make(ProviderId::Codex, "work")
        );
        assert_eq!(
            make(ProviderId::Codex, "default"),
            make(ProviderId::Codex, "default")
        );
    }
}
