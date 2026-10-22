//! Which alternate views a document type offers: read-only previews and
//! editable visual editors share one tab mechanism. A new type adds a variant
//! here; the editor button, command and palette entry are driven by it.
use lumen_editor_core::syntax::Language;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewKind {
    Markdown,
}

impl PreviewKind {
    pub(crate) fn of(language: Language) -> Option<Self> {
        match language {
            Language::Markdown => Some(Self::Markdown),
            _ => None,
        }
    }

    /// `.markdown` and `.mdx` are previewed as Markdown even where the editor's
    /// highlighter does not classify them as such.
    pub(crate) fn of_path(path: &Path, language: Language) -> Option<Self> {
        Self::of(language).or_else(|| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .filter(|extension| {
                    ["markdown", "mdx"]
                        .iter()
                        .any(|known| extension.eq_ignore_ascii_case(known))
                })
                .map(|_| Self::Markdown)
        })
    }
}

/// Editable table view of a structured text file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VisualKind {
    Csv,
}

impl VisualKind {
    pub(crate) fn of_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        ["csv", "tsv"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
            .then_some(Self::Csv)
    }
}

/// A way to look at a document other than its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlternateView {
    Preview(PreviewKind),
    Visual(VisualKind),
}

impl AlternateView {
    pub(crate) fn tab_title(self, source_name: &str) -> String {
        match self {
            Self::Preview(_) => format!("Preview: {source_name}"),
            Self::Visual(_) => format!("Table: {source_name}"),
        }
    }

    pub(crate) fn opened_message(self) -> &'static str {
        match self {
            Self::Preview(_) => "Opened preview",
            Self::Visual(_) => "Opened visual editor",
        }
    }

    pub(crate) fn unsupported_message() -> &'static str {
        "This file type has no preview or visual editor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_family_is_previewable_and_code_is_not() {
        for name in ["a.md", "a.markdown", "README.md", "a.mdx"] {
            let path = Path::new(name);
            assert_eq!(
                PreviewKind::of_path(path, Language::from_path(path)),
                Some(PreviewKind::Markdown),
                "{name}"
            );
        }
        let path = Path::new("a.rs");
        assert_eq!(PreviewKind::of_path(path, Language::from_path(path)), None);
    }

    #[test]
    fn csv_and_tsv_offer_a_visual_editor_and_nothing_else_does() {
        for name in ["a.csv", "A.CSV", "data.tsv"] {
            assert_eq!(VisualKind::of_path(Path::new(name)), Some(VisualKind::Csv));
        }
        for name in ["a.md", "a.rs", "csv", "a.csv.bak"] {
            assert_eq!(VisualKind::of_path(Path::new(name)), None, "{name}");
        }
    }
}
