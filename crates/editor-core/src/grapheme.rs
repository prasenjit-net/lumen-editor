//! Grapheme boundary queries that look only around one offset.
//!
//! Building a table of every boundary is linear in the document; asking about a
//! single position is not, which matters for each arrow key, delete and edit in a
//! large file. Results match the full table (see the tests).
use unicode_segmentation::GraphemeCursor;

/// Whether `offset` is a legal extended grapheme boundary of `text`.
pub(crate) fn is_boundary(text: &str, offset: usize) -> bool {
    offset <= text.len()
        && text.is_char_boundary(offset)
        && GraphemeCursor::new(offset, text.len(), true)
            .is_boundary(text, 0)
            .unwrap_or(false)
}

/// The first boundary strictly after `offset`, or the end of the text.
pub(crate) fn next_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    GraphemeCursor::new(offset, text.len(), true)
        .next_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(text.len())
}

/// The last boundary strictly before `offset`, or the start of the text.
pub(crate) fn previous_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    GraphemeCursor::new(offset.min(text.len()), text.len(), true)
        .prev_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// `offset` if it is a boundary, else the next boundary after it.
pub(crate) fn snap_forward(text: &str, offset: usize) -> usize {
    if is_boundary(text, offset) {
        offset
    } else {
        next_boundary(text, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    fn table(text: &str) -> Vec<usize> {
        text.grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len()))
            .collect()
    }

    const PIECES: &[&str] = &[
        "a",
        "b",
        " ",
        "\n",
        "\r\n",
        "é",
        "e\u{301}",
        "👩\u{200d}💻",
        "🇯🇵",
        "🇺🇸",
        "漢",
        "אב",
        "\u{200d}",
        "\u{1f3fd}",
        "😀",
        "ก\u{e33}",
        "한",
        "\u{1100}\u{1161}\u{11a8}",
        "\t",
    ];

    fn texts() -> Vec<String> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut texts = vec![String::new()];
        for _ in 0..200 {
            let count = (next() % 10) as usize;
            texts.push(
                (0..count)
                    .map(|_| PIECES[(next() % PIECES.len() as u64) as usize])
                    .collect(),
            );
        }
        texts
    }

    #[test]
    fn local_queries_match_the_full_boundary_table_on_random_unicode() {
        for text in texts() {
            let boundaries = table(&text);
            for offset in 0..=text.len() + 1 {
                let expected = boundaries.binary_search(&offset).is_ok();
                assert_eq!(is_boundary(&text, offset), expected, "{text:?} @ {offset}");
                if offset > text.len() || !text.is_char_boundary(offset) {
                    continue;
                }
                let next = boundaries.iter().copied().find(|b| *b > offset);
                assert_eq!(
                    next_boundary(&text, offset),
                    next.unwrap_or(text.len()),
                    "next {text:?} @ {offset}"
                );
                let previous = boundaries.iter().copied().rev().find(|b| *b < offset);
                assert_eq!(
                    previous_boundary(&text, offset),
                    previous.unwrap_or(0),
                    "previous {text:?} @ {offset}"
                );
                let snapped = boundaries.iter().copied().find(|b| *b >= offset).unwrap();
                assert_eq!(
                    snap_forward(&text, offset),
                    snapped,
                    "snap {text:?} @ {offset}"
                );
            }
        }
    }

    #[test]
    fn queries_clamp_at_the_ends_of_the_text() {
        assert_eq!(next_boundary("ab", 2), 2);
        assert_eq!(next_boundary("ab", 9), 2);
        assert_eq!(previous_boundary("ab", 0), 0);
        assert_eq!(previous_boundary("", 0), 0);
        assert!(is_boundary("", 0));
        assert!(
            !is_boundary("é", 1),
            "inside a character is never a boundary"
        );
        assert!(!is_boundary("ab", 3));
    }
}
