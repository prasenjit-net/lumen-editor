use crate::{EditError, EditorBuffer, Selection, SelectionSet, grapheme::is_boundary};
use std::{collections::BTreeSet, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

impl EditorBuffer {
    /// Replace a multi-line selection with one caret at the end of each
    /// touched logical line. A selection ending at the next line's start does
    /// not include that next line. This is a selection-only command.
    pub fn select_line_ends(&mut self) -> bool {
        let text = self.text();
        let starts: Vec<_> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(byte, _)| byte + 1))
            .collect();
        let mut ends = BTreeSet::new();
        let line_at = |byte| {
            starts
                .partition_point(|start| *start <= byte)
                .saturating_sub(1)
        };
        let line_end = |index: usize| {
            let next = starts.get(index + 1).copied().unwrap_or(text.len());
            if next > 0 && text.as_bytes()[next - 1] == b'\n' {
                let before_newline = next - 1;
                if before_newline > 0 && text.as_bytes()[before_newline - 1] == b'\r' {
                    before_newline - 1
                } else {
                    before_newline
                }
            } else {
                next
            }
        };
        for selection in self.selections().selections() {
            let range = selection.range();
            let last = if range.is_empty() {
                range.end
            } else {
                range.end - 1
            };
            for index in line_at(range.start)..=line_at(last) {
                ends.insert(line_end(index));
            }
        }
        let primary_head = self.selections().selections()[self.selections().primary_index()].head;
        let primary_end = line_end(line_at(primary_head));
        let cursors: Vec<_> = ends.into_iter().map(Selection::caret).collect();
        let primary = cursors
            .iter()
            .rposition(|cursor| cursor.head <= primary_end)
            .unwrap_or(0);
        let before = self.selections().clone();
        self.set_selections(cursors, primary).unwrap();
        before != *self.selections()
    }

    /// Select the word at a lone caret first; otherwise add the next literal
    /// occurrence, wrapping once. Does not edit text or create undo history.
    pub fn select_next_occurrence(&mut self) -> bool {
        let primary = self.selections().selections()[self.selections().primary_index()];
        let range = primary.range();
        if range.is_empty() {
            if let Some(word) = word_at(self.text(), primary.head) {
                return self.set_selections(vec![selection(word)], 0).is_ok();
            }
            return false;
        }
        let matches = literal_ranges(self.text(), &self.text()[range.clone()]);
        let mut candidates = matches
            .iter()
            .filter(|candidate| candidate.start >= range.end)
            .chain(
                matches
                    .iter()
                    .filter(|candidate| candidate.start < range.end),
            );
        let next = candidates
            .find(|candidate| {
                !self
                    .selections()
                    .selections()
                    .iter()
                    .any(|existing| overlaps(&existing.range(), candidate))
            })
            .cloned();
        let Some(next) = next else {
            return false;
        };
        let mut selections = self.selections().selections().to_vec();
        let primary = selections.len();
        selections.push(selection(next));
        self.set_selections(selections, primary).is_ok()
    }

    pub fn select_all_occurrences(&mut self) -> bool {
        let primary = self.selections().selections()[self.selections().primary_index()];
        let range = if primary.range().is_empty() {
            let Some(word) = word_at(self.text(), primary.head) else {
                return false;
            };
            word
        } else {
            primary.range()
        };
        let matches = literal_ranges(self.text(), &self.text()[range.clone()]);
        let index = matches
            .iter()
            .position(|candidate| *candidate == range)
            .unwrap_or(0);
        let selections = matches.into_iter().map(selection).collect();
        let before = self.selections().clone();
        self.set_selections(selections, index).unwrap();
        before != *self.selections()
    }

    /// Deduplicate touched logical lines, exclude an end exactly on the next
    /// line, and remap both endpoints. One transaction retains selection direction.
    pub fn indent_lines(&mut self, indentation: &str, outdent: bool) -> Result<bool, EditError> {
        if indentation.is_empty() || !indentation.chars().all(|c| c == ' ' || c == '\t') {
            return Err(EditError::InvalidIndentation);
        }
        let starts: Vec<_> = std::iter::once(0)
            .chain(self.text().match_indices('\n').map(|(byte, _)| byte + 1))
            .collect();
        let mut touched = BTreeSet::new();
        for selected in self.selections().selections() {
            let range = selected.range();
            let first = starts
                .partition_point(|start| *start <= range.start)
                .saturating_sub(1);
            let last_byte = if range.is_empty() {
                range.end
            } else {
                range.end - 1
            };
            let last = starts
                .partition_point(|start| *start <= last_byte)
                .saturating_sub(1);
            touched.extend(starts[first..=last].iter().copied());
        }
        let edits: Vec<_> = touched
            .into_iter()
            .map(|start| {
                let removal = if outdent {
                    if self.text()[start..].starts_with('\t') {
                        1
                    } else {
                        self.text()[start..]
                            .bytes()
                            .take(indentation.len())
                            .take_while(|byte| *byte == b' ')
                            .count()
                    }
                } else {
                    0
                };
                (
                    start..start + removal,
                    if outdent { "" } else { indentation },
                )
            })
            .collect();
        let mut text = String::new();
        let mut previous = 0;
        for (range, inserted) in &edits {
            text.push_str(&self.text()[previous..range.start]);
            text.push_str(inserted);
            previous = range.end;
        }
        text.push_str(&self.text()[previous..]);
        let selections = self
            .selections()
            .selections()
            .iter()
            .zip(self.selections().affinities())
            .map(|(selected, affinity)| {
                (
                    Selection {
                        anchor: remap(selected.anchor, &edits),
                        head: remap(selected.head, &edits),
                    },
                    *affinity,
                )
            })
            .collect();
        let selections =
            SelectionSet::with_affinities(&text, selections, self.selections().primary_index())?;
        Ok(self.commit_snapshot(text, selections))
    }
}

fn selection(range: Range<usize>) -> Selection {
    Selection {
        anchor: range.start,
        head: range.end,
    }
}

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    if left.is_empty() {
        right.start <= left.start && left.start <= right.end
    } else {
        left.start < right.end && right.start < left.end
    }
}

fn word_at(text: &str, caret: usize) -> Option<Range<usize>> {
    let word = |grapheme: &str| {
        grapheme
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    };
    let start = text[..caret]
        .grapheme_indices(true)
        .rev()
        .take_while(|(_, grapheme)| word(grapheme))
        .last()
        .map_or(caret, |(i, _)| i);
    let end = caret
        + text[caret..]
            .graphemes(true)
            .take_while(|grapheme| word(grapheme))
            .map(str::len)
            .sum::<usize>();
    (start < end && is_boundary(text, start) && is_boundary(text, end)).then_some(start..end)
}

fn literal_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    text.match_indices(query)
        .map(|(start, _)| start..start + query.len())
        .filter(|range| is_boundary(text, range.start) && is_boundary(text, range.end))
        .collect()
}

fn remap(byte: usize, edits: &[(Range<usize>, &str)]) -> usize {
    let mut removed = 0;
    let mut inserted = 0;
    for (range, text) in edits {
        if byte < range.start {
            break;
        }
        if byte <= range.end {
            return range.start - removed + inserted + text.len();
        }
        removed += range.len();
        inserted += text.len();
    }
    byte - removed + inserted
}

#[cfg(test)]
mod tests;
