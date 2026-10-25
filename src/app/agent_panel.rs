//! View state of the right-hand agent panel: one tab per provider, each showing
//! either its current chat or the list of its previous sessions. Presentation
//! only: sessions live in the hub, so nothing here owns or ends one. Drafts and
//! attachments are kept per session so they outlive tab, history and panel switches.
use super::chat_attachments::{Attachment, AttachmentError, MAX_ATTACHMENTS};
use crate::model::agent_scheduler::Ticket;
use lumen_agent_core::{ProviderId, SessionId};
use std::collections::HashMap;

/// A starting prompt offered in an empty chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChatSuggestion {
    ExplainFolder,
    ReviewChanges,
}

impl ChatSuggestion {
    pub(crate) const ALL: [Self; 2] = [Self::ExplainFolder, Self::ReviewChanges];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ExplainFolder => "Explain this folder",
            Self::ReviewChanges => "Review my changes",
        }
    }

    pub(crate) fn prompt(self) -> &'static str {
        match self {
            Self::ExplainFolder => {
                "Explain what this folder contains: its purpose, structure and how to build and run it."
            }
            Self::ReviewChanges => {
                "Review my uncommitted changes and point out bugs, risks and anything missing."
            }
        }
    }

    /// Reviewing changes needs a Git repository to have any.
    pub(crate) fn needs_repository(self) -> bool {
        self == Self::ReviewChanges
    }
}

/// The Git comparison that shows an agent-edited file, when one exists: the
/// working tree first, then untracked, then staged. `None` when the file has no
/// uncommitted change in the repository (or lies outside it).
pub(crate) fn changed_file_diff(
    snapshot: &crate::services::source_control::GitSnapshot,
    path: &std::path::Path,
) -> Option<(
    std::path::PathBuf,
    crate::services::source_control::DiffSide,
)> {
    use crate::services::source_control::DiffSide;
    let relative = path.strip_prefix(&snapshot.root).ok()?;
    let change = snapshot.changes.iter().find(|c| c.path == relative)?;
    let side = if change.index == '?' && change.worktree == '?' {
        DiffSide::Untracked
    } else if change.worktree != ' ' && change.worktree != '?' {
        DiffSide::Working
    } else if change.index != ' ' && change.index != '?' {
        DiffSide::Staged
    } else {
        return None;
    };
    Some((relative.to_path_buf(), side))
}

/// Where a chat's transcript should scroll to on the next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollRequest {
    Latest,
    /// The transcript item at this index (a message the user sent).
    Item(usize),
}

/// A queued message being edited in place; saved or cancelled explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QueueEdit {
    pub(crate) session: SessionId,
    pub(crate) ticket: Ticket,
    pub(crate) text: String,
    pub(crate) focus: bool,
    /// The field had keyboard focus last frame. egui drops focus on Escape before
    /// the widget runs, so this is what tells Escape it was meant for the field.
    pub(crate) had_focus: bool,
}

/// Recency groups of the History list, newest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HistoryGroup {
    Pinned,
    Recent,
    Week,
    Earlier,
}

impl HistoryGroup {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pinned => "Pinned",
            Self::Recent => "Last 24 hours",
            Self::Week => "Last 7 days",
            Self::Earlier => "Earlier",
        }
    }
}

/// Rolling windows rather than calendar days: no time-zone guess is needed and a
/// chat from just before midnight is still "recent".
pub(crate) fn history_group(pinned: bool, now: u64, updated: u64) -> HistoryGroup {
    if pinned {
        return HistoryGroup::Pinned;
    }
    match now.saturating_sub(updated) {
        0..86_400 => HistoryGroup::Recent,
        86_400..604_800 => HistoryGroup::Week,
        _ => HistoryGroup::Earlier,
    }
}

/// Whether a title matches the search text (case-insensitive; blank matches all).
pub(crate) fn title_matches(title: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || title.to_lowercase().contains(&query.to_lowercase())
}

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
    /// Providers whose program was not found, with the name that was looked for.
    missing: HashMap<ProviderId, String>,
    /// Pending transcript scroll per session, applied on the next frame.
    scroll_requests: HashMap<SessionId, ScrollRequest>,
    /// Transcript items the user has seen at the end of each session, so content
    /// that arrives while reading older messages can be flagged.
    seen_items: HashMap<SessionId, usize>,
    /// Title filter of the History list, shared by the provider tabs.
    history_query: String,
    queue_edit: Option<QueueEdit>,
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
            missing: HashMap::new(),
            scroll_requests: HashMap::new(),
            seen_items: HashMap::new(),
            history_query: String::new(),
            queue_edit: None,
        }
    }
}

impl AgentPanel {
    /// Record whether the provider's program was found (`None`) or not (`Some(name)`).
    pub(crate) fn set_missing(&mut self, provider: ProviderId, program: Option<String>) {
        match program {
            Some(name) => self.missing.insert(provider, name),
            None => self.missing.remove(&provider),
        };
    }

    /// The program that was looked for and not found, when it is not installed.
    pub(crate) fn missing(&self, provider: ProviderId) -> Option<&str> {
        self.missing.get(&provider).map(String::as_str)
    }

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
        self.history_query.clear();
    }

    pub(crate) fn request_composer_focus(&mut self) {
        self.focus_composer = true;
    }

    pub(crate) fn history_query_mut(&mut self) -> &mut String {
        &mut self.history_query
    }

    pub(crate) fn request_scroll(&mut self, id: SessionId, request: ScrollRequest) {
        self.scroll_requests.insert(id, request);
    }

    pub(crate) fn take_scroll_request(&mut self, id: SessionId) -> Option<ScrollRequest> {
        self.scroll_requests.remove(&id)
    }

    /// Record how many transcript items the user has caught up with.
    pub(crate) fn mark_seen(&mut self, id: SessionId, items: usize) {
        self.seen_items.insert(id, items);
    }

    /// Items that arrived since the user last saw the end of the transcript. A
    /// session never viewed has nothing "new": its whole history is not news.
    pub(crate) fn unseen_items(&self, id: SessionId, items: usize) -> usize {
        self.seen_items
            .get(&id)
            .map_or(0, |seen| items.saturating_sub(*seen))
    }

    pub(crate) fn queue_edit(&self) -> Option<&QueueEdit> {
        self.queue_edit.as_ref()
    }

    pub(crate) fn set_queue_edit(&mut self, edit: Option<QueueEdit>) {
        self.queue_edit = edit;
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
    fn agent_edits_open_the_matching_git_comparison_or_nothing() {
        use crate::services::source_control::{DiffSide, GitChange, GitSnapshot};
        let snapshot = GitSnapshot {
            root: "/repo".into(),
            changes: vec![
                GitChange::new("a.rs", ' ', 'M'),
                GitChange::new("new.rs", '?', '?'),
                GitChange::new("staged.rs", 'M', ' '),
            ],
            ..Default::default()
        };
        let find = |p: &str| changed_file_diff(&snapshot, std::path::Path::new(p));
        assert_eq!(find("/repo/a.rs"), Some(("a.rs".into(), DiffSide::Working)));
        assert_eq!(
            find("/repo/new.rs"),
            Some(("new.rs".into(), DiffSide::Untracked))
        );
        assert_eq!(
            find("/repo/staged.rs"),
            Some(("staged.rs".into(), DiffSide::Staged))
        );
        assert_eq!(find("/repo/clean.rs"), None);
        assert_eq!(find("/elsewhere/a.rs"), None);
    }

    #[test]
    fn history_groups_are_rolling_windows_with_pinned_first() {
        let now = 1_000_000;
        assert_eq!(history_group(true, now, 0), HistoryGroup::Pinned);
        assert_eq!(
            history_group(false, now, now - 86_399),
            HistoryGroup::Recent
        );
        assert_eq!(history_group(false, now, now - 86_400), HistoryGroup::Week);
        assert_eq!(
            history_group(false, now, now - 604_800),
            HistoryGroup::Earlier
        );
        assert_eq!(history_group(false, now, now + 5), HistoryGroup::Recent);
        assert!(HistoryGroup::Pinned < HistoryGroup::Recent);
    }

    #[test]
    fn title_search_ignores_case_and_surrounding_space() {
        assert!(title_matches("Fix Login Bug", "  login "));
        assert!(title_matches("anything", ""));
        assert!(!title_matches("Fix Login Bug", "logout"));
    }

    #[test]
    fn unseen_items_count_only_what_arrived_after_the_last_look() {
        let mut panel = AgentPanel::default();
        let id = SessionId(1);
        assert_eq!(panel.unseen_items(id, 40), 0, "never viewed is not news");
        panel.mark_seen(id, 10);
        assert_eq!(panel.unseen_items(id, 14), 4);
        assert_eq!(
            panel.unseen_items(id, 8),
            0,
            "a shorter transcript is not new"
        );
        assert_eq!(panel.unseen_items(SessionId(2), 14), 0);
    }

    #[test]
    fn scroll_requests_are_per_session_and_taken_once() {
        let mut panel = AgentPanel::default();
        panel.request_scroll(SessionId(1), ScrollRequest::Item(3));
        assert_eq!(panel.take_scroll_request(SessionId(2)), None);
        assert_eq!(
            panel.take_scroll_request(SessionId(1)),
            Some(ScrollRequest::Item(3))
        );
        assert_eq!(panel.take_scroll_request(SessionId(1)), None);
    }

    #[test]
    fn going_back_to_the_chat_clears_the_history_search() {
        let mut panel = AgentPanel::default();
        panel.history_query_mut().push_str("bug");
        panel.toggle_history(CODEX);
        assert_eq!(panel.history_query_mut(), "bug", "kept while in History");
        panel.back_to_chat(CODEX);
        assert!(panel.history_query_mut().is_empty());
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
