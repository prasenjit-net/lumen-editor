use std::ops::Range;
#[cfg(test)]
use unicode_segmentation::UnicodeSegmentation;

/// Which logical side of a byte boundary owns its visual caret in bidi text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaretAffinity {
    Before,
    #[default]
    After,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionAffinity {
    pub anchor: CaretAffinity,
    pub head: CaretAffinity,
}

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
    InvalidIndentation,
}

/// Document-ordered, non-overlapping selections and a stable primary member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionSet {
    selections: Vec<Selection>,
    primary: usize,
    affinities: Vec<SelectionAffinity>,
}

impl SelectionSet {
    pub fn new(text: &str, selections: Vec<Selection>, primary: usize) -> Result<Self, EditError> {
        Self::with_affinities(
            text,
            selections
                .into_iter()
                .map(|selection| (selection, SelectionAffinity::default()))
                .collect(),
            primary,
        )
    }

    pub fn with_affinities(
        text: &str,
        selections: Vec<(Selection, SelectionAffinity)>,
        primary: usize,
    ) -> Result<Self, EditError> {
        if selections.is_empty() {
            return Err(EditError::EmptySelections);
        }
        if primary >= selections.len() {
            return Err(EditError::InvalidPrimary);
        }
        for (selection, _) in &selections {
            for offset in [selection.anchor, selection.head] {
                if !crate::grapheme::is_boundary(text, offset) {
                    return Err(EditError::InvalidBoundary(offset));
                }
            }
        }
        let mut ordered: Vec<_> = selections
            .into_iter()
            .enumerate()
            .map(|(index, (selection, affinity))| (selection, index == primary, affinity))
            .collect();
        ordered.sort_by_key(|(selection, _, _)| {
            let range = selection.range();
            (range.start, range.end)
        });
        let mut merged: Vec<(Selection, bool, SelectionAffinity)> =
            Vec::with_capacity(ordered.len());
        for (selection, is_primary, affinity) in ordered {
            if let Some((previous, previous_primary, previous_affinity)) = merged.last_mut() {
                let left = previous.range();
                let right = selection.range();
                // Touching nonempty ranges are distinct. A caret on a selection
                // edge belongs to it, preventing an extra insertion at that edge.
                let overlaps = right.start < left.end
                    || (right.start == left.end && (left.is_empty() || right.is_empty()));
                if overlaps {
                    let old_selection = *previous;
                    let old_affinity = *previous_affinity;
                    let reversed = if is_primary {
                        selection.reversed()
                    } else {
                        previous.reversed()
                    };
                    // Preserve the owning endpoint when it survives a merge;
                    // new union endpoints inherit their original logical side.
                    let endpoint = |byte| {
                        if is_primary && byte == selection.head {
                            affinity.head
                        } else if is_primary && byte == selection.anchor {
                            affinity.anchor
                        } else if byte == old_selection.head {
                            old_affinity.head
                        } else if byte == old_selection.anchor {
                            old_affinity.anchor
                        } else if byte == selection.head {
                            affinity.head
                        } else {
                            affinity.anchor
                        }
                    };
                    *previous_affinity = SelectionAffinity {
                        anchor: endpoint(previous.anchor),
                        head: endpoint(previous.head),
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
            merged.push((selection, is_primary, affinity));
        }
        let primary = merged.iter().position(|(_, primary, _)| *primary).unwrap();
        let affinities = merged.iter().map(|(_, _, affinity)| *affinity).collect();
        Ok(Self {
            selections: merged
                .into_iter()
                .map(|(selection, _, _)| selection)
                .collect(),
            primary,
            affinities,
        })
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }
    pub fn primary_index(&self) -> usize {
        self.primary
    }
    pub fn affinities(&self) -> &[SelectionAffinity] {
        &self.affinities
    }
}

#[cfg(test)]
pub(crate) fn grapheme_boundaries(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .collect()
}
