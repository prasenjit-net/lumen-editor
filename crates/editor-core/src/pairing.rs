//! Delimiter edits use the buffer's existing transaction and selection history.
use crate::{Direction, EditError, EditorBuffer, Selection};
use std::ops::Range;

impl EditorBuffer {
    /// Insert an opening delimiter at each caret, or wrap each selection.
    /// Carets stay inside new pairs; wrapped text remains selected for editing.
    pub fn insert_pair(&mut self, open: char, close: char) -> Result<bool, EditError> {
        let before = self.text().to_owned();
        let selections = self.selections().clone();
        let mut fragments = Vec::with_capacity(selections.selections().len());
        let mut targets = Vec::with_capacity(selections.selections().len());
        let mut shift = 0isize;
        for selected in selections.selections() {
            let range = selected.range();
            let content = &before[range.clone()];
            // Do not create a duplicate closing delimiter immediately before one.
            let existing_close = content.is_empty() && before[range.end..].starts_with(close);
            let fragment = if existing_close {
                format!("{open}")
            } else {
                format!("{open}{content}{close}")
            };
            let inner_start = (range.start as isize + shift) as usize + open.len_utf8();
            let inner_end = inner_start + content.len();
            targets.push(Selection {
                anchor: inner_start,
                head: inner_end,
            });
            shift += fragment.len() as isize - content.len() as isize;
            fragments.push(fragment);
        }
        let pieces: Vec<&str> = fragments.iter().map(String::as_str).collect();
        let changed = self.replace_selections(&pieces)?;
        if changed {
            self.set_selections(targets, selections.primary_index())?;
        }
        Ok(changed)
    }

    /// Overtype a closing delimiter when every caret already faces it.
    pub fn skip_closing(&mut self, close: char) -> bool {
        if self.selections().selections().iter().all(|selection| {
            selection.range().is_empty() && self.text()[selection.head..].starts_with(close)
        }) {
            self.move_horizontal(Direction::Forward, false);
            true
        } else {
            false
        }
    }
}

/// Find matching ASCII brackets next to a caret without changing document state.
pub fn matching_brackets(text: &str, caret: usize) -> Option<(Range<usize>, Range<usize>)> {
    matching_brackets_where(text, caret, |_| true)
}

/// The caller can exclude lexical regions such as strings and comments.
pub fn matching_brackets_where(
    text: &str,
    caret: usize,
    allowed: impl Fn(usize) -> bool,
) -> Option<(Range<usize>, Range<usize>)> {
    let at_caret = text
        .get(caret..)
        .and_then(|after| after.chars().next().map(|ch| (caret, ch)))
        .filter(|(_, ch)| is_bracket(*ch));
    let candidate = if let Some(candidate) = at_caret {
        allowed(candidate.0).then_some(candidate)
    } else {
        {
            text.get(..caret).and_then(|before| {
                before
                    .char_indices()
                    .next_back()
                    .filter(|(index, ch)| is_bracket(*ch) && allowed(*index))
            })
        }
    }?;
    let (start, bracket) = candidate;
    let (partner, forward) = match bracket {
        '(' => (')', true),
        '[' => (']', true),
        '{' => ('}', true),
        ')' => ('(', false),
        ']' => ('[', false),
        '}' => ('{', false),
        _ => return None,
    };
    let mut depth = 0usize;
    let source = if forward {
        &text[start..]
    } else {
        &text[..=start]
    };
    let positions: Box<dyn Iterator<Item = (usize, char)> + '_> = if forward {
        Box::new(source.char_indices().map(|(index, ch)| (start + index, ch)))
    } else {
        Box::new(source.char_indices().rev())
    };
    for (index, ch) in positions {
        if !allowed(index) {
            continue;
        }
        if ch == bracket {
            depth += 1;
        } else if ch == partner {
            depth -= 1;
            if depth == 0 {
                let current = start..start + bracket.len_utf8();
                let matching = index..index + partner.len_utf8();
                return Some((current, matching));
            }
        }
    }
    None
}

fn is_bracket(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_multiselections_and_undo_restores_original_ranges() {
        let mut buffer = EditorBuffer::new("é one two".into());
        buffer
            .set_selections(
                vec![
                    Selection { anchor: 3, head: 6 },
                    Selection {
                        anchor: 7,
                        head: 10,
                    },
                ],
                1,
            )
            .unwrap();
        let original = buffer.selections().clone();
        assert!(buffer.insert_pair('(', ')').unwrap());
        assert_eq!(buffer.text(), "é (one) (two)");
        assert_eq!(
            buffer.selections().selections(),
            &[
                Selection { anchor: 4, head: 7 },
                Selection {
                    anchor: 10,
                    head: 13
                }
            ]
        );
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "é one two");
        assert_eq!(buffer.selections(), &original);
    }

    #[test]
    fn paired_caret_and_existing_closer_do_not_duplicate_delimiters() {
        let mut buffer = EditorBuffer::new("x)".into());
        buffer.set_selections(vec![Selection::caret(1)], 0).unwrap();
        buffer.insert_pair('(', ')').unwrap();
        assert_eq!(buffer.text(), "x()");
        assert_eq!(buffer.selections().selections(), &[Selection::caret(2)]);
        assert!(buffer.skip_closing(')'));
        assert_eq!(buffer.selections().selections(), &[Selection::caret(3)]);
        assert_eq!(buffer.text(), "x()");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "x)");
    }

    #[test]
    fn matching_is_nested_and_read_only() {
        let text = "a([x])\r\n";
        assert_eq!(matching_brackets(text, 1), Some((1..2, 5..6)));
        assert_eq!(matching_brackets(text, 3), Some((2..3, 4..5)));
        assert_eq!(matching_brackets(text, 6), Some((5..6, 1..2)));
        assert_eq!(matching_brackets("(unfinished", 1), None);
        let text = "(\"ignored )\" + [ok])";
        let excluded = 2..12;
        assert_eq!(
            matching_brackets_where(text, 0, |byte| !excluded.contains(&byte)),
            Some((0..1, text.len() - 1..text.len()))
        );
    }
}
