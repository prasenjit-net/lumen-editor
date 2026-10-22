//! Which document types can be shown in a preview tab. New previewable types
//! add a variant here; the button, command and palette entry are driven by it.
use lumen_editor_core::syntax::Language;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewKind {
    Markdown,
    /// SVG stays a text document; its preview rasterizes the buffer.
    Svg,
}

impl PreviewKind {
    pub(crate) fn of(language: Language) -> Option<Self> {
        match language {
            Language::Markdown => Some(Self::Markdown),
            _ => None,
        }
    }

    /// Extensions the editor's highlighter does not classify are matched here:
    /// `.markdown` and `.mdx` preview as Markdown, `.svg` as SVG.
    pub(crate) fn of_path(path: &Path, language: Language) -> Option<Self> {
        Self::of(language).or_else(|| {
            let extension = path.extension()?.to_str()?;
            let is = |known: &[&str]| {
                known
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            };
            if is(&["markdown", "mdx"]) {
                Some(Self::Markdown)
            } else if is(&["svg"]) {
                Some(Self::Svg)
            } else {
                None
            }
        })
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
    fn svg_is_previewable_regardless_of_case() {
        for name in ["a.svg", "logo.SVG"] {
            let path = Path::new(name);
            assert_eq!(
                PreviewKind::of_path(path, Language::from_path(path)),
                Some(PreviewKind::Svg),
                "{name}"
            );
        }
    }
}
