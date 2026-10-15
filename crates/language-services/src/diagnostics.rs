//! Validated diagnostic snapshots for later LSP notifications and requests.
use crate::RequestStamp;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticError {
    InvalidBoundary,
    EmptyMessage,
}

/// Diagnostics tied to the exact document/session state that requested them.
/// Callers must check `stamp.is_current_for(active)` before painting a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticSnapshot {
    pub stamp: RequestStamp,
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticSnapshot {
    pub fn new(
        text: &str,
        stamp: RequestStamp,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Self, DiagnosticError> {
        for diagnostic in &diagnostics {
            if diagnostic.range.start > diagnostic.range.end
                || diagnostic.range.end > text.len()
                || !text.is_char_boundary(diagnostic.range.start)
                || !text.is_char_boundary(diagnostic.range.end)
            {
                return Err(DiagnosticError::InvalidBoundary);
            }
            if diagnostic.message.trim().is_empty() {
                return Err(DiagnosticError::EmptyMessage);
            }
        }
        Ok(Self { stamp, diagnostics })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DocumentId, DocumentKey, DocumentUri, DocumentVersion, ServerDocumentVersion,
        SessionGeneration,
    };

    fn stamp() -> RequestStamp {
        RequestStamp {
            document: DocumentKey {
                id: DocumentId(1),
                uri: DocumentUri::new("file:///workspace/main.rs").unwrap(),
            },
            document_version: DocumentVersion(2),
            server_version: ServerDocumentVersion(3),
            session: SessionGeneration(1),
        }
    }

    #[test]
    fn diagnostics_require_utf8_boundaries_and_nonempty_messages() {
        let text = "let 🌍 = 1;";
        let snapshot = DiagnosticSnapshot::new(
            text,
            stamp(),
            vec![Diagnostic {
                range: 4..8,
                severity: DiagnosticSeverity::Warning,
                message: "unused binding".into(),
                source: Some("rust-analyzer".into()),
                code: Some("unused_variables".into()),
            }],
        )
        .unwrap();
        assert_eq!(snapshot.diagnostics.len(), 1);
        assert_eq!(
            DiagnosticSnapshot::new(
                text,
                stamp(),
                vec![Diagnostic {
                    range: 5..8,
                    severity: DiagnosticSeverity::Error,
                    message: "bad".into(),
                    source: None,
                    code: None,
                }],
            ),
            Err(DiagnosticError::InvalidBoundary)
        );
    }
}
