use std::sync::atomic::{AtomicU64, Ordering};

/// Runtime identity for an open document. Paths and tab indices can change;
/// widget/session ownership must not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DocumentId(u64);

impl DocumentId {
    pub(super) fn allocate() -> Self {
        static NEXT_DOCUMENT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_DOCUMENT.fetch_add(1, Ordering::Relaxed))
    }
}
