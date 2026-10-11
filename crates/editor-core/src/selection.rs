use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// UTF-8 byte endpoints, validated against extended grapheme boundaries by a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn caret(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn reversed(self) -> bool {
        self.head < self.anchor
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    EmptySelections,
    InvalidPrimary,
    InvalidBoundary(usize),
    FragmentCount,
    StaleComposition,
}

/// Document-ordered, non-overlapping selections and a stable primary member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionSet {
    selections: Vec<Selection>,
    primary: usize,
}

impl SelectionSet {
    pub fn new(text: &str, selections: Vec<Selection>, primary: usize) -> Result<Self, EditError> {
        if selections.is_empty() {
            return Err(EditError::EmptySelections);
        }
        if primary >= selections.len() {
            return Err(EditError::InvalidPrimary);
        }
        let boundaries = grapheme_boundaries(text);
        for selection in &selections {
            for offset in [selection.anchor, selection.head] {
                if boundaries.binary_search(&offset).is_err() {
                    return Err(EditError::InvalidBoundary(offset));
                }
            }
        }
        let mut ordered: Vec<_> = selections
            .into_iter()
            .enumerate()
            .map(|(index, selection)| (selection, index == primary))
            .collect();
        ordered.sort_by_key(|(selection, _)| {
            let range = selection.range();
            (range.start, range.end)
        });
        let mut merged: Vec<(Selection, bool)> = Vec::with_capacity(ordered.len());
        for (selection, is_primary) in ordered {
            if let Some((previous, previous_primary)) = merged.last_mut() {
                let left = previous.range();
                let right = selection.range();
                // Touching nonempty ranges are distinct. A caret on a selection
                // edge belongs to it, preventing an extra insertion at that edge.
                let overlaps = right.start < left.end
                    || (right.start == left.end && (left.is_empty() || right.is_empty()));
                if overlaps {
                    let reversed = if is_primary {
                        selection.reversed()
                    } else {
                        previous.reversed()
                    };
                    let start = left.start.min(right.start);
                    let end = left.end.max(right.end);
                    *previous = if reversed {
                        Selection {
                            anchor: end,
                            head: start,
                        }
                    } else {
                        Selection {
                            anchor: start,
                            head: end,
                        }
                    };
                    *previous_primary |= is_primary;
                    continue;
                }
            }
            merged.push((selection, is_primary));
        }
        let primary = merged.iter().position(|(_, primary)| *primary).unwrap();
        Ok(Self {
            selections: merged.into_iter().map(|(selection, _)| selection).collect(),
            primary,
        })
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }
    pub fn primary_index(&self) -> usize {
        self.primary
    }
}

pub(crate) fn grapheme_boundaries(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect()
}
