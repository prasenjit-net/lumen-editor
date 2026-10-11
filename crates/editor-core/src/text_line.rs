//! Whole-line clipboard rules, expressed in Unicode scalar cursor coordinates.
use std::ops::Range;

pub struct LineClipboard {
    pub text: String,
    pub deletion: Range<usize>,
}

pub struct LinePaste {
    pub insertion: usize,
    pub caret_after: usize,
}

/// A proven whole-line clipboard payload is inserted before the current line.
/// Keep the caret on the original text, at its original Unicode scalar column.
pub fn line_paste(text: &str, caret: usize, copied: &str) -> Option<LinePaste> {
    if copied.find('\n') != Some(copied.len().checked_sub(1)?) {
        return None;
    }
    let byte = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain([text.len()])
        .nth(caret)?;
    let start = text[..byte].rfind('\n').map_or(0, |newline| newline + 1);
    Some(LinePaste {
        insertion: text[..start].chars().count(),
        caret_after: caret.checked_add(copied.chars().count())?,
    })
}

/// Copy always includes a terminator; cutting the last line removes its preceding
/// terminator instead. This follows VS Code's empty-selection clipboard behavior.
pub fn line_clipboard(text: &str, caret: usize) -> Option<LineClipboard> {
    let byte = text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain([text.len()])
        .nth(caret)?;
    let start = text[..byte].rfind('\n').map_or(0, |newline| newline + 1);
    let next = text[start..].find('\n').map(|offset| start + offset + 1);
    let end = next.unwrap_or(text.len());
    let content_end = if next.is_some() { end - 1 } else { end };
    let content_end = if next.is_some() && text[..content_end].ends_with('\r') {
        content_end - 1
    } else {
        content_end
    };
    let eol = if let Some(next) = next {
        &text[content_end..next]
    } else if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let deletion_start = if next.is_none() && start > 0 {
        let newline = start - 1;
        if text[..newline].ends_with('\r') {
            newline - 1
        } else {
            newline
        }
    } else {
        start
    };
    Some(LineClipboard {
        text: format!("{}{eol}", &text[start..content_end]),
        deletion: text[..deletion_start].chars().count()..text[..end].chars().count(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_crlf_and_last_line_have_distinct_copy_and_delete_ranges() {
        let text = "é 🌍\r\nlast";
        let first = line_clipboard(text, 2).unwrap();
        assert_eq!(first.text, "é 🌍\r\n");
        assert_eq!(first.deletion, 0..5);
        let last = line_clipboard(text, 8).unwrap();
        assert_eq!(last.text, "last\r\n");
        assert_eq!(last.deletion, 3..9);
    }

    #[test]
    fn blank_empty_and_eof_lines_are_valid_but_stale_cursors_are_not() {
        for (text, caret, copy, deletion) in [
            ("", 0, "\n", 0..0),
            ("solo", 2, "solo\n", 0..4),
            ("a\n\nb", 2, "\n", 2..3),
            ("a\n", 2, "\n", 1..2),
            ("a\r\n", 3, "\r\n", 1..3),
        ] {
            let line = line_clipboard(text, caret).unwrap();
            assert_eq!(line.text, copy);
            assert_eq!(line.deletion, deletion);
        }
        assert!(line_clipboard("é", 2).is_none());
    }

    #[test]
    fn line_paste_maps_unicode_columns_and_empty_eof_lines() {
        for (text, caret, copied, insertion, after) in [
            ("é 🌍\r\nnext", 7, "é 🌍\r\n", 5, 12),
            ("", 0, "\n", 0, 1),
            ("a\r\n", 3, "b\r\n", 3, 6),
        ] {
            let paste = line_paste(text, caret, copied).unwrap();
            assert_eq!(paste.insertion, insertion);
            assert_eq!(paste.caret_after, after);
        }
    }

    #[test]
    fn line_paste_rejects_stale_cursors_and_non_line_payloads() {
        assert!(line_paste("é", 2, "x\n").is_none());
        for copied in ["", "x", "x\ny\n", "x\ny"] {
            assert!(line_paste("text", 1, copied).is_none());
        }
    }
}
