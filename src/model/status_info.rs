//! Facts the status bar reports about a document, derived from its text.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineEnding {
    /// No line break yet, so the text carries no convention to report.
    None,
    Lf,
    CrLf,
    Cr,
    Mixed,
}

impl LineEnding {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "No EOL",
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
            Self::Mixed => "Mixed EOL",
        }
    }

    pub(crate) fn describe(self) -> &'static str {
        match self {
            Self::None => "The text has no line breaks yet",
            Self::Lf => "Lines end with LF (\\n)",
            Self::CrLf => "Lines end with CRLF (\\r\\n)",
            Self::Cr => "Lines end with CR (\\r)",
            Self::Mixed => "The text mixes different line endings",
        }
    }
}

pub(crate) fn detect_line_ending(text: &str) -> LineEnding {
    let (mut lf, mut crlf, mut cr) = (false, false, false);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf = true;
                index += 1;
            }
            b'\r' => cr = true,
            b'\n' => lf = true,
            _ => {}
        }
        index += 1;
    }
    match (lf, crlf, cr) {
        (false, false, false) => LineEnding::None,
        (true, false, false) => LineEnding::Lf,
        (false, true, false) => LineEnding::CrLf,
        (false, false, true) => LineEnding::Cr,
        _ => LineEnding::Mixed,
    }
}

/// One-based line and column (counted in characters) of a byte offset.
/// An offset past the end or inside a character is clamped to a boundary.
pub(crate) fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = &text[..end];
    let mut line = 1;
    let mut line_start = 0;
    // Look ahead in the whole text so a CR directly before an LF is one break.
    let bytes = text.as_bytes();
    for (index, byte) in bytes[..end].iter().enumerate() {
        let breaks = match byte {
            b'\n' => true,
            b'\r' => bytes.get(index + 1) != Some(&b'\n'),
            _ => false,
        };
        if breaks {
            line += 1;
            line_start = index + 1;
        }
    }
    (line, before[line_start..].chars().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_are_reported_honestly() {
        assert_eq!(detect_line_ending(""), LineEnding::None);
        assert_eq!(detect_line_ending("one line"), LineEnding::None);
        assert_eq!(detect_line_ending("a\nb\n"), LineEnding::Lf);
        assert_eq!(detect_line_ending("a\r\nb\r\n"), LineEnding::CrLf);
        assert_eq!(detect_line_ending("a\rb"), LineEnding::Cr);
        assert_eq!(detect_line_ending("a\r\nb\n"), LineEnding::Mixed);
        assert_eq!(detect_line_ending("a\nb\rc"), LineEnding::Mixed);
    }

    #[test]
    fn positions_are_one_based_and_count_characters() {
        assert_eq!(line_column("", 0), (1, 1));
        assert_eq!(line_column("abc\ndef", 5), (2, 2));
        assert_eq!(line_column("a\r\nb", 3), (2, 1));
        assert_eq!(line_column("a\r\nb", 2), (1, 3));
        assert_eq!(line_column("é\nx", 2), (1, 2));
        assert_eq!(
            line_column("é", 1),
            (1, 1),
            "inside a character clamps back"
        );
        assert_eq!(line_column("ab", 99), (1, 3));
    }
}
