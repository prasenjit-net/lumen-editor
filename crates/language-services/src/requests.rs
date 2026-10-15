//! Correlation for asynchronous LSP requests.
use crate::RequestStamp;
use std::collections::BTreeMap;

#[derive(Default, Debug)]
pub struct RequestTracker {
    next_id: u64,
    pending: BTreeMap<u64, RequestStamp>,
}

impl RequestTracker {
    pub fn register(&mut self, stamp: RequestStamp) -> u64 {
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.pending.insert(self.next_id, stamp);
        self.next_id
    }

    pub fn take_current(&mut self, id: u64, current: &RequestStamp) -> Option<RequestStamp> {
        let stamp = self.pending.remove(&id)?;
        stamp.is_current_for(current).then_some(stamp)
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DocumentId, DocumentKey, DocumentUri, DocumentVersion, ServerDocumentVersion,
        SessionGeneration,
    };
    fn stamp(version: u64) -> RequestStamp {
        RequestStamp {
            document: DocumentKey {
                id: DocumentId(1),
                uri: DocumentUri::new("file:///x").unwrap(),
            },
            document_version: DocumentVersion(version),
            server_version: ServerDocumentVersion(version),
            session: SessionGeneration(1),
        }
    }
    #[test]
    fn stale_or_duplicate_responses_are_discarded() {
        let mut tracker = RequestTracker::default();
        let id = tracker.register(stamp(1));
        assert!(tracker.take_current(id, &stamp(2)).is_none());
        assert!(tracker.take_current(id, &stamp(1)).is_none());
    }
}
