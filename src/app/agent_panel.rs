//! View state of the right-hand agent panel: one tab per provider, each showing
//! either its current chat or the list of its previous sessions. Presentation
//! only: sessions live in the hub, so nothing here owns or ends one. Drafts and
//! attachments are kept per session so they outlive tab, history and panel switches.
use super::chat_attachments::{Attachment, AttachmentError, MAX_ATTACHMENTS};
use lumen_agent_core::{ProviderId, SessionId};
use std::collections::HashMap;

/// What a provider tab shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TabMode {
    #[default]
    Chat,
    History,
}

/// Each provider tab remembers its own mode and current session.
#[derive(Clone, Copy, Debug, Default)]
struct ProviderTab {
    mode: TabMode,
    current: Option<SessionId>,
}

/// A History row being renamed in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenameDraft {
    pub(crate) id: SessionId,
    pub(crate) text: String,
    /// The field should take keyboard focus (first frame only).
    pub(crate) focus: bool,
}

impl RenameDraft {
    pub(crate) fn new(id: SessionId, text: String) -> Self {
        Self {
            id,
            text,
            focus: true,
        }
    }
}

pub(crate) struct AgentPanel {
    active: ProviderId,
    tabs: HashMap<ProviderId, ProviderTab>,
    /// The active provider changed since it was last saved.
    provider_dirty: bool,
    /// The composer should take keyboard focus on the next frame.
    focus_composer: bool,
    /// The History list should focus its first row on the next frame.
    focus_history: bool,
    rename: Option<RenameDraft>,
    /// Unsent composer text per session.
    drafts: HashMap<SessionId, String>,
    /// Files attached to the unsent message, per session. Paths and metadata
    /// only: the bytes are read when the message is sent.
    attachments: HashMap<SessionId, Vec<Attachment>>,
}

impl Default for AgentPanel {
    fn default() -> Self {
        Self {
            active: ProviderId::ALL[0],
            tabs: HashMap::new(),
            provider_dirty: false,
            focus_composer: false,
            focus_history: false,
            rename: None,
            drafts: HashMap::new(),
            attachments: HashMap::new(),
        }
    }
}

impl AgentPanel {
    pub(crate) fn active_provider(&self) -> ProviderId {
        self.active
    }

    /// Restore the tab chosen in an earlier run; not a user change, so not saved again.
    pub(crate) fn restore_provider(&mut self, provider: ProviderId) {
        self.active = provider;
    }

    /// Show another provider's tab, in whatever mode it was left.
    pub(crate) fn select_provider(&mut self, provider: ProviderId) {
        if self.active != provider {
            self.active = provider;
            self.provider_dirty = true;
        }
        self.focus_composer = self.mode(provider) == TabMode::Chat;
    }

    /// The provider to save, once per change.
    pub(crate) fn take_provider_change(&mut self) -> Option<ProviderId> {
        std::mem::take(&mut self.provider_dirty).then_some(self.active)
    }

    pub(crate) fn mode(&self, provider: ProviderId) -> TabMode {
        self.tabs
            .get(&provider)
            .map_or(TabMode::Chat, |tab| tab.mode)
    }

    pub(crate) fn current(&self, provider: ProviderId) -> Option<SessionId> {
        self.tabs.get(&provider).and_then(|tab| tab.current)
    }

    /// The session the visible tab is showing, when it is in chat mode.
    #[cfg(test)]
    pub(crate) fn visible_chat(&self) -> Option<SessionId> {
        (self.mode(self.active) == TabMode::Chat)
            .then(|| self.current(self.active))
            .flatten()
    }

    /// Make `id` the provider's current chat and show it.
    pub(crate) fn show_session(&mut self, provider: ProviderId, id: SessionId) {
        let tab = self.tabs.entry(provider).or_default();
        tab.current = Some(id);
        tab.mode = TabMode::Chat;
        self.select_provider(provider);
    }

    /// History button: flip the provider's tab between chat and its session list.
    pub(crate) fn toggle_history(&mut self, provider: ProviderId) {
        let tab = self.tabs.entry(provider).or_default();
        tab.mode = match tab.mode {
            TabMode::Chat => TabMode::History,
            TabMode::History => TabMode::Chat,
        };
        self.focus_composer = tab.mode == TabMode::Chat;
        self.focus_history = tab.mode == TabMode::History;
        self.rename = None;
    }

    /// Return to the provider's chat without choosing another session.
    pub(crate) fn back_to_chat(&mut self, provider: ProviderId) {
        self.tabs.entry(provider).or_default().mode = TabMode::Chat;
        self.focus_composer = true;
        self.rename = None;
    }

    pub(crate) fn take_history_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus_history)
    }

    pub(crate) fn renaming(&self) -> Option<&RenameDraft> {
        self.rename.as_ref()
    }

    /// Take the rename in progress for this frame's drawing; put it back with `set_rename`.
    pub(crate) fn take_rename(&mut self) -> Option<RenameDraft> {
        self.rename.take()
    }

    pub(crate) fn set_rename(&mut self, rename: Option<RenameDraft>) {
        self.rename = rename;
    }

    /// Keep the provider's current session valid. `sessions` are its live
    /// (unarchived) sessions, most recent first: a current session that is gone
    /// falls back to the most recent one, else to none (the empty state).
    pub(crate) fn resolve_current(
        &mut self,
        provider: ProviderId,
        sessions: &[SessionId],
    ) -> Option<SessionId> {
        let tab = self.tabs.entry(provider).or_default();
        if !tab.current.is_some_and(|id| sessions.contains(&id)) {
            tab.current = sessions.first().copied();
        }
        tab.current
    }

    pub(crate) fn take_composer_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus_composer)
    }

    pub(crate) fn draft_mut(&mut self, id: SessionId) -> &mut String {
        self.drafts.entry(id).or_default()
    }

    #[cfg(test)]
    pub(crate) fn draft(&self, id: SessionId) -> &str {
        self.drafts.get(&id).map_or("", String::as_str)
    }

    pub(crate) fn take_draft(&mut self, id: SessionId) -> String {
        self.drafts.remove(&id).unwrap_or_default()
    }

    pub(crate) fn restore_draft(&mut self, id: SessionId, text: String) {
        if !text.is_empty() {
            self.drafts.insert(id, text);
        }
    }

    pub(crate) fn attachments(&self, id: SessionId) -> &[Attachment] {
        self.attachments.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Add an attachment; a file already attached is refused rather than doubled.
    pub(crate) fn attach(
        &mut self,
        id: SessionId,
        attachment: Attachment,
    ) -> Result<(), AttachmentError> {
        let list = self.attachments.entry(id).or_default();
        if list.iter().any(|a| a.path == attachment.path) {
            return Err(AttachmentError::Duplicate(attachment.name()));
        }
        if list.len() >= MAX_ATTACHMENTS {
            return Err(AttachmentError::TooMany);
        }
        list.push(attachment);
        Ok(())
    }

    pub(crate) fn detach(&mut self, id: SessionId, path: &std::path::Path) {
        if let Some(list) = self.attachments.get_mut(&id) {
            list.retain(|a| a.path != path);
        }
    }

    pub(crate) fn take_attachments(&mut self, id: SessionId) -> Vec<Attachment> {
        self.attachments.remove(&id).unwrap_or_default()
    }

    pub(crate) fn restore_attachments(&mut self, id: SessionId, list: Vec<Attachment>) {
        if !list.is_empty() {
            self.attachments.insert(id, list);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Attachment {
        Attachment::new(name.into(), None, 10)
    }

    const CLAUDE: ProviderId = ProviderId::Claude;
    const CODEX: ProviderId = ProviderId::Codex;

    #[test]
    fn attachments_are_per_session_unique_and_bounded() {
        let mut panel = AgentPanel::default();
        let id = SessionId(1);
        assert!(panel.attach(id, file("/w/a.rs")).is_ok());
        assert_eq!(
            panel.attach(id, file("/w/a.rs")),
            Err(AttachmentError::Duplicate("a.rs".into()))
        );
        assert!(panel.attachments(SessionId(2)).is_empty());
        for n in 1..MAX_ATTACHMENTS {
            panel.attach(id, file(&format!("/w/{n}.rs"))).unwrap();
        }
        assert_eq!(
            panel.attach(id, file("/w/extra.rs")),
            Err(AttachmentError::TooMany)
        );
        panel.detach(id, std::path::Path::new("/w/a.rs"));
        assert_eq!(panel.attachments(id).len(), MAX_ATTACHMENTS - 1);
        let taken = panel.take_attachments(id);
        assert!(panel.attachments(id).is_empty());
        panel.restore_attachments(id, taken);
        assert_eq!(panel.attachments(id).len(), MAX_ATTACHMENTS - 1);
    }

    #[test]
    fn showing_a_session_selects_its_provider_and_returns_to_chat() {
        let mut panel = AgentPanel::default();
        panel.toggle_history(CLAUDE);
        panel.show_session(CLAUDE, SessionId(4));
        assert_eq!(panel.active_provider(), CLAUDE);
        assert_eq!(panel.mode(CLAUDE), TabMode::Chat);
        assert_eq!(panel.visible_chat(), Some(SessionId(4)));
        assert!(panel.take_composer_focus());
        assert!(!panel.take_composer_focus(), "focus is requested once");
    }

    #[test]
    fn history_toggles_and_goes_back_without_changing_the_session() {
        let mut panel = AgentPanel::default();
        panel.show_session(CODEX, SessionId(1));
        panel.toggle_history(CODEX);
        assert_eq!(panel.mode(CODEX), TabMode::History);
        assert_eq!(panel.visible_chat(), None);
        panel.back_to_chat(CODEX);
        assert_eq!(panel.mode(CODEX), TabMode::Chat);
        assert_eq!(panel.current(CODEX), Some(SessionId(1)));
        panel.toggle_history(CODEX);
        panel.toggle_history(CODEX);
        assert_eq!(panel.mode(CODEX), TabMode::Chat);
    }

    #[test]
    fn each_provider_keeps_its_own_mode_and_session() {
        let mut panel = AgentPanel::default();
        panel.show_session(CODEX, SessionId(1));
        panel.show_session(CLAUDE, SessionId(2));
        panel.toggle_history(CODEX);
        panel.select_provider(CLAUDE);
        assert_eq!(panel.visible_chat(), Some(SessionId(2)));
        assert_eq!(panel.mode(CODEX), TabMode::History);
        panel.select_provider(CODEX);
        assert_eq!(panel.visible_chat(), None, "Codex was left in History");
        assert_eq!(panel.current(CODEX), Some(SessionId(1)));
        assert_eq!(panel.current(CLAUDE), Some(SessionId(2)));
    }

    #[test]
    fn a_lost_current_session_falls_back_to_the_most_recent_then_to_empty() {
        let mut panel = AgentPanel::default();
        panel.show_session(CLAUDE, SessionId(5));
        let sessions = [SessionId(9), SessionId(5), SessionId(2)];
        assert_eq!(panel.resolve_current(CLAUDE, &sessions), Some(SessionId(5)));
        let after_archive = [SessionId(9), SessionId(2)];
        assert_eq!(
            panel.resolve_current(CLAUDE, &after_archive),
            Some(SessionId(9))
        );
        assert_eq!(panel.resolve_current(CLAUDE, &[]), None);
        assert_eq!(panel.visible_chat(), None);
        assert_eq!(
            panel.resolve_current(CODEX, &[SessionId(3)]),
            Some(SessionId(3)),
            "a provider with no current session adopts its most recent"
        );
    }

    #[test]
    fn the_provider_choice_is_reported_once_per_change() {
        let mut panel = AgentPanel::default();
        panel.restore_provider(CLAUDE);
        assert_eq!(
            panel.take_provider_change(),
            None,
            "restoring is not a change"
        );
        panel.select_provider(CODEX);
        assert_eq!(panel.take_provider_change(), Some(CODEX));
        assert_eq!(panel.take_provider_change(), None);
        panel.select_provider(CODEX);
        assert_eq!(panel.take_provider_change(), None);
    }

    #[test]
    fn drafts_are_per_session_and_survive_switching_tabs() {
        let mut panel = AgentPanel::default();
        panel.show_session(CLAUDE, SessionId(1));
        panel.draft_mut(SessionId(1)).push_str("half typed");
        panel.show_session(CODEX, SessionId(2));
        panel.toggle_history(CLAUDE);
        assert_eq!(panel.draft(SessionId(1)), "half typed");
        assert_eq!(panel.draft(SessionId(2)), "");
        assert_eq!(panel.take_draft(SessionId(1)), "half typed");
        assert_eq!(panel.draft(SessionId(1)), "");
    }
}
