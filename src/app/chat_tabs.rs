//! Chat tabs: virtual (non-document) tabs in the editor area, one per open agent
//! session. Closing a tab only hides the view; the session and its history stay in
//! the session list. Tabs are addressed by `SessionId`, never by strip position.
// Hosted in the first editor group for now, like Settings and task editors.
use super::chat_attachments::{Attachment, AttachmentError, MAX_ATTACHMENTS};
use lumen_agent_core::SessionId;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct ChatTabs {
    tabs: Vec<SessionId>,
    active: Option<SessionId>,
    /// Unsent composer text per session, kept while the tab is open or hidden.
    drafts: HashMap<SessionId, String>,
    /// Files attached to the unsent message, per session. Paths and metadata
    /// only: the bytes are read when the message is sent.
    attachments: HashMap<SessionId, Vec<Attachment>>,
}

impl ChatTabs {
    pub(crate) fn tabs(&self) -> &[SessionId] {
        &self.tabs
    }

    pub(crate) fn active(&self) -> Option<SessionId> {
        self.active
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self, id: SessionId) -> bool {
        self.tabs.contains(&id)
    }

    /// Open the chat (or focus it when already open) and make it the visible page.
    pub(crate) fn open(&mut self, id: SessionId) {
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
        self.active = Some(id);
    }

    /// Show another open chat; false when it is not open.
    #[cfg(test)]
    pub(crate) fn activate(&mut self, id: SessionId) -> bool {
        if self.tabs.contains(&id) {
            self.active = Some(id);
            true
        } else {
            false
        }
    }

    /// Stop showing a chat page (the tabs stay open).
    pub(crate) fn deactivate(&mut self) {
        self.active = None;
    }

    /// Close the tab. When it was the visible page, the neighbouring chat takes
    /// over, otherwise the editor shows its documents again.
    pub(crate) fn close(&mut self, id: SessionId) {
        let Some(position) = self.tabs.iter().position(|tab| *tab == id) else {
            return;
        };
        self.tabs.remove(position);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(position)
                .or_else(|| position.checked_sub(1).and_then(|p| self.tabs.get(p)))
                .copied();
        }
    }

    #[cfg(test)]
    pub(crate) fn draft(&self, id: SessionId) -> &str {
        self.drafts.get(&id).map_or("", String::as_str)
    }

    pub(crate) fn draft_mut(&mut self, id: SessionId) -> &mut String {
        self.drafts.entry(id).or_default()
    }

    pub(crate) fn take_draft(&mut self, id: SessionId) -> String {
        self.drafts.remove(&id).unwrap_or_default()
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

    pub(crate) fn restore_draft(&mut self, id: SessionId, text: String) {
        if !text.is_empty() {
            self.drafts.insert(id, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> Attachment {
        Attachment::new(name.into(), None, 10)
    }

    #[test]
    fn attachments_are_per_session_unique_and_bounded() {
        let mut tabs = ChatTabs::default();
        let id = SessionId(1);
        assert!(tabs.attach(id, file("/w/a.rs")).is_ok());
        assert_eq!(
            tabs.attach(id, file("/w/a.rs")),
            Err(AttachmentError::Duplicate("a.rs".into()))
        );
        assert!(tabs.attachments(SessionId(2)).is_empty());
        for n in 1..MAX_ATTACHMENTS {
            tabs.attach(id, file(&format!("/w/{n}.rs"))).unwrap();
        }
        assert_eq!(
            tabs.attach(id, file("/w/extra.rs")),
            Err(AttachmentError::TooMany)
        );
        tabs.detach(id, std::path::Path::new("/w/a.rs"));
        assert_eq!(tabs.attachments(id).len(), MAX_ATTACHMENTS - 1);
        let taken = tabs.take_attachments(id);
        assert!(tabs.attachments(id).is_empty());
        tabs.restore_attachments(id, taken);
        assert_eq!(tabs.attachments(id).len(), MAX_ATTACHMENTS - 1);
    }

    #[test]
    fn opening_twice_focuses_instead_of_duplicating() {
        let mut tabs = ChatTabs::default();
        tabs.open(SessionId(1));
        tabs.open(SessionId(2));
        tabs.open(SessionId(1));
        assert_eq!(tabs.tabs(), [SessionId(1), SessionId(2)]);
        assert_eq!(tabs.active(), Some(SessionId(1)));
    }

    #[test]
    fn closing_the_visible_chat_shows_a_neighbour_then_the_documents() {
        let mut tabs = ChatTabs::default();
        for n in 1..=3 {
            tabs.open(SessionId(n));
        }
        tabs.activate(SessionId(2));
        tabs.close(SessionId(2));
        assert_eq!(tabs.active(), Some(SessionId(3)));
        tabs.close(SessionId(3));
        assert_eq!(tabs.active(), Some(SessionId(1)));
        tabs.close(SessionId(1));
        assert_eq!(tabs.active(), None);
        tabs.close(SessionId(9));
    }

    #[test]
    fn closing_a_background_chat_keeps_the_visible_page_and_drafts_survive() {
        let mut tabs = ChatTabs::default();
        tabs.open(SessionId(1));
        tabs.open(SessionId(2));
        tabs.draft_mut(SessionId(1)).push_str("half typed");
        tabs.close(SessionId(1));
        assert_eq!(tabs.active(), Some(SessionId(2)));
        assert_eq!(
            tabs.draft(SessionId(1)),
            "half typed",
            "the draft outlives the tab"
        );
        assert_eq!(tabs.take_draft(SessionId(1)), "half typed");
        assert_eq!(tabs.draft(SessionId(1)), "");
        tabs.deactivate();
        assert!(!tabs.activate(SessionId(7)));
    }
}
