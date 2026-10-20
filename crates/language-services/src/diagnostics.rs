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

impl Diagnostic {
    /// The span to underline in `text`. Servers report many syntax errors as
    /// zero-width positions (rust-analyzer's "expected expression"); those are
    /// widened to the character after the position, or the one before it at the
    /// end of a line, so they stay visible. A position with no neighbour on its
    /// line (an empty line) stays empty.
    #[must_use]
    pub fn display_range(&self, text: &str) -> Range<usize> {
        let at = self.range.start;
        if self.range.end > at || at > text.len() || !text.is_char_boundary(at) {
            return self.range.clone();
        }
        if let Some(next) = text[at..]
            .chars()
            .next()
            .filter(|c| *c != '\n' && *c != '\r')
        {
            return at..at + next.len_utf8();
        }
        if let Some(previous) = text[..at]
            .chars()
            .next_back()
            .filter(|c| *c != '\n' && *c != '\r')
        {
            return at - previous.len_utf8()..at;
        }
        self.range.clone()
    }
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

    fn at(range: Range<usize>) -> Diagnostic {
        Diagnostic {
            range,
            severity: DiagnosticSeverity::Error,
            message: "expected expression".into(),
            source: None,
            code: None,
        }
    }

    #[test]
    fn zero_width_diagnostics_are_widened_to_a_visible_character() {
        let text = "let x = ;\nfoo(\nlet é";
        assert_eq!(at(4..4).display_range(text), 4..5, "next character");
        assert_eq!(
            at(9..9).display_range(text),
            8..9,
            "end of line uses previous"
        );
        assert_eq!(
            at(text.len()..text.len()).display_range(text),
            text.len() - 2..text.len(),
            "multibyte at EOF"
        );
        assert_eq!(
            at(2..6).display_range(text),
            2..6,
            "real ranges are untouched"
        );
        assert_eq!(at(0..0).display_range(""), 0..0, "empty document");
        assert_eq!(
            at(9..9).display_range("a\n\nb"),
            9..9,
            "out of range is untouched"
        );
        assert_eq!(
            at(2..2).display_range("a\n\nb"),
            2..2,
            "empty line stays empty"
        );
    }
}
