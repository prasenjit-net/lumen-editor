//! Explicit, per-feature capability reporting. Unproven features are visibly
//! unavailable, never silently simulated.

/// Whether one feature works with this provider and version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unsupported {
        reason: String,
    },
    /// Works only from `since` onward (or only with a flag); `reason` says what.
    VersionGated {
        since: String,
        reason: String,
    },
}

impl Support {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self::Unsupported {
            reason: reason.into(),
        }
    }

    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }
}

/// Everything Lumen may need to know before offering a feature. Separate fields on
/// purpose: for example Lumen can resume a session it already knows about without
/// being able to enumerate every native session (`resume` versus `list_history`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub streaming: Support,
    pub list_history: Support,
    pub read_history: Support,
    pub resume: Support,
    pub rename: Support,
    pub archive: Support,
    pub attachments_text: Support,
    pub attachments_image: Support,
    pub approvals: Support,
    pub questions: Support,
    pub config_mutation: Support,
    pub command_discovery: Support,
    pub usage: Support,
    /// The provider itself enforces a read-only mode (a UI label is not enough).
    pub enforced_read_only: Support,
    pub interrupt: Support,
    /// Several sessions share one native connection.
    pub native_multi_session: Support,
}

impl Capabilities {
    /// Everything unsupported: the starting point a provider crate fills in.
    pub fn none() -> Self {
        let no = || Support::unsupported("not implemented");
        Self {
            streaming: no(),
            list_history: no(),
            read_history: no(),
            resume: no(),
            rename: no(),
            archive: no(),
            attachments_text: no(),
            attachments_image: no(),
            approvals: no(),
            questions: no(),
            config_mutation: no(),
            command_discovery: no(),
            usage: no(),
            enforced_read_only: no(),
            interrupt: no(),
            native_multi_session: no(),
        }
    }

    /// Everything supported (used by the fake backend).
    pub fn all() -> Self {
        let yes = Support::Supported;
        Self {
            streaming: yes.clone(),
            list_history: yes.clone(),
            read_history: yes.clone(),
            resume: yes.clone(),
            rename: yes.clone(),
            archive: yes.clone(),
            attachments_text: yes.clone(),
            attachments_image: yes.clone(),
            approvals: yes.clone(),
            questions: yes.clone(),
            config_mutation: yes.clone(),
            command_discovery: yes.clone(),
            usage: yes.clone(),
            enforced_read_only: yes.clone(),
            interrupt: yes.clone(),
            native_multi_session: yes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_all_unsupported_with_a_reason_and_all_is_supported() {
        let none = Capabilities::none();
        assert!(!none.resume.is_supported() && !none.interrupt.is_supported());
        assert!(matches!(&none.resume, Support::Unsupported { reason } if !reason.is_empty()));
        let all = Capabilities::all();
        assert!(all.resume.is_supported() && all.native_multi_session.is_supported());
        assert_ne!(none, all);
    }

    #[test]
    fn resume_and_listing_are_separate_capabilities() {
        let mut caps = Capabilities::none();
        caps.resume = Support::Supported;
        assert!(caps.resume.is_supported() && !caps.list_history.is_supported());
    }
}
