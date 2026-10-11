use crate::{EditError, Selection, SelectionSet, selection::grapheme_boundaries};

// Full snapshots are deliberately bounded for this correctness spike; measured
// storage/history performance is a separate gate before production adoption.
const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy)]
pub enum Direction {
    Backward,
    Forward,
}

#[derive(Clone)]
struct Snapshot {
    text: String,
    selections: SelectionSet,
}

/// Candidate editing authority. The application does not instantiate this yet.
pub struct EditorBuffer {
    current: Snapshot,
    saved_text: String,
    revision: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl EditorBuffer {
    pub fn new(text: String) -> Self {
        let selections = SelectionSet::new(&text, vec![Selection::caret(0)], 0).unwrap();
        Self {
            saved_text: text.clone(),
            current: Snapshot { text, selections },
            revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.current.text
    }
    pub fn selections(&self) -> &SelectionSet {
        &self.current.selections
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_dirty(&self) -> bool {
        self.current.text != self.saved_text
    }

    /// Call only after a successful save, never on a failed/cancelled write.
    pub fn mark_saved(&mut self) {
        self.saved_text.clone_from(&self.current.text);
    }

    pub fn set_selections(
        &mut self,
        selections: Vec<Selection>,
        primary: usize,
    ) -> Result<(), EditError> {
        self.current.selections = SelectionSet::new(self.text(), selections, primary)?;
        Ok(())
    }

    /// One shared fragment, or one per canonical selection in document order.
    /// Build everything against one snapshot before mutating; errors and text
    /// no-ops preserve text, selections, revision and both history branches.
    pub fn replace_selections(&mut self, fragments: &[&str]) -> Result<bool, EditError> {
        self.replace_set(&self.selections().clone(), fragments)
    }

    pub fn replace_document(&mut self, text: &str) -> Result<bool, EditError> {
        let set = SelectionSet::new(
            self.text(),
            vec![Selection {
                anchor: 0,
                head: self.text().len(),
            }],
            0,
        )?;
        self.replace_set(&set, &[text])
    }

    pub fn move_horizontal(&mut self, direction: Direction, extend: bool) {
        let boundaries = grapheme_boundaries(self.text());
        let selections = self
            .selections()
            .selections()
            .iter()
            .map(|selection| {
                let range = selection.range();
                let head = if !extend && !range.is_empty() {
                    match direction {
                        Direction::Backward => range.start,
                        Direction::Forward => range.end,
                    }
                } else {
                    neighbor(&boundaries, selection.head, direction)
                };
                Selection {
                    anchor: if extend { selection.anchor } else { head },
                    head,
                }
            })
            .collect();
        self.set_selections(selections, self.selections().primary_index())
            .unwrap();
    }

    pub fn delete_graphemes(&mut self, direction: Direction) -> Result<bool, EditError> {
        let boundaries = grapheme_boundaries(self.text());
        let selections = self
            .selections()
            .selections()
            .iter()
            .map(|selection| {
                if selection.range().is_empty() {
                    Selection {
                        anchor: selection.anchor,
                        head: neighbor(&boundaries, selection.head, direction),
                    }
                } else {
                    *selection
                }
            })
            .collect();
        let set = SelectionSet::new(self.text(), selections, self.selections().primary_index())?;
        // The history records the original carets, not temporary deletion ranges.
        self.replace_set(&set, &[""])
    }

    fn replace_set(
        &mut self,
        selections: &SelectionSet,
        fragments: &[&str],
    ) -> Result<bool, EditError> {
        if fragments.len() != 1 && fragments.len() != selections.selections().len() {
            return Err(EditError::FragmentCount);
        }
        let mut text = String::new();
        let mut carets = Vec::with_capacity(selections.selections().len());
        let mut previous_end = 0;
        for (index, selection) in selections.selections().iter().enumerate() {
            let range = selection.range();
            text.push_str(&self.text()[previous_end..range.start]);
            text.push_str(fragments[if fragments.len() == 1 { 0 } else { index }]);
            carets.push(Selection::caret(text.len()));
            previous_end = range.end;
        }
        text.push_str(&self.text()[previous_end..]);
        if text == self.text() {
            return Ok(false);
        }
        // Insertion can join a combining mark/ZWJ with neighboring text. Snap
        // forward in the final document instead of leaving a caret inside it.
        let boundaries = grapheme_boundaries(&text);
        for caret in &mut carets {
            let index = boundaries.partition_point(|boundary| *boundary < caret.head);
            *caret = Selection::caret(boundaries[index]);
        }
        let selections = SelectionSet::new(&text, carets, selections.primary_index())?;
        let previous = std::mem::replace(&mut self.current, Snapshot { text, selections });
        self.undo.push(previous);
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.current, previous));
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.current, next));
        self.revision = self.revision.wrapping_add(1);
        true
    }
}

fn neighbor(boundaries: &[usize], offset: usize, direction: Direction) -> usize {
    let index = boundaries.binary_search(&offset).unwrap();
    boundaries[match direction {
        Direction::Backward => index.saturating_sub(1),
        Direction::Forward => (index + 1).min(boundaries.len() - 1),
    }]
}
