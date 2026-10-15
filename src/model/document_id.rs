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

    /// Stable language-service identity. UI code continues to use the opaque
    /// model ID, while the app adapter converts it at the service boundary.
    pub(crate) const fn language_service_id(self) -> lumen_language_services::DocumentId {
        lumen_language_services::DocumentId(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_service_adapter_preserves_the_stable_identity() {
        let id = DocumentId::allocate();
        assert!(id.language_service_id().0 > 0);
    }
}
