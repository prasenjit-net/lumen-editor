use crate::{
    EditError, Selection, SelectionAffinity, SelectionSet, selection::grapheme_boundaries,
};

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

/// Shared editing/history authority in the feature-gated workbench and prototype.
pub struct EditorBuffer {
    identity: u64,
    current: Snapshot,
    saved_text: String,
    revision: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    typing: Option<TypingGroup>,
}

struct TypingGroup {
    revision: u64,
    selections: SelectionSet,
    time: f64,
}

const TYPING_PAUSE_SECONDS: f64 = 1.0;

impl EditorBuffer {
    pub fn new(text: String) -> Self {
        let selections = SelectionSet::new(&text, vec![Selection::caret(0)], 0).unwrap();
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            identity: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            saved_text: text.clone(),
            current: Snapshot { text, selections },
            revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.current.text
    }
    pub fn selections(&self) -> &SelectionSet {
        &self.current.selections
    }
    /// Stable identity for revision-keyed caches, including after buffer replacement.
    pub fn identity(&self) -> u64 {
        self.identity
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Retained undo/redo text payload, not total allocator or process memory.
    /// Excludes current/saved text, selection vectors and allocation capacity.
    pub fn history_text_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(|snapshot| snapshot.text.len())
            .sum()
    }

    /// Replace validated ranges without changing the original undo selection.
    pub fn replace_ranges(
        &mut self,
        selections: Vec<Selection>,
        primary: usize,
        fragments: &[&str],
    ) -> Result<bool, EditError> {
        let set = SelectionSet::new(self.text(), selections, primary)?;
        self.replace_set(&set, fragments)
    }
    pub fn is_dirty(&self) -> bool {
        self.current.text != self.saved_text
    }

    /// Call only after a successful save, never on a failed/cancelled write.
    pub fn mark_saved(&mut self) {
        self.break_undo_group();
        self.saved_text.clone_from(&self.current.text);
    }

    pub fn set_selections(
        &mut self,
        selections: Vec<Selection>,
        primary: usize,
    ) -> Result<(), EditError> {
        let selections = SelectionSet::new(self.text(), selections, primary)?;
        self.break_undo_group();
        self.current.selections = selections;
        Ok(())
    }

    pub fn set_selections_with_affinity(
        &mut self,
        selections: Vec<(Selection, SelectionAffinity)>,
        primary: usize,
    ) -> Result<(), EditError> {
        let selections = SelectionSet::with_affinities(self.text(), selections, primary)?;
        self.break_undo_group();
        self.current.selections = selections;
        Ok(())
    }

    /// One shared fragment, or one per canonical selection in document order.
    /// Build everything against one snapshot before mutating; errors and text
    /// no-ops preserve text, selections, revision and both history branches.
    pub fn replace_selections(&mut self, fragments: &[&str]) -> Result<bool, EditError> {
        self.replace_set(&self.selections().clone(), fragments)
    }

    /// Consecutive single-line typing shares one undo snapshot. Time is supplied
    /// by the input adapter, keeping clocks and UI dependencies out of the core.
    /// Paste, IME and commands use ordinary atomic replacement, never this API.
    pub fn type_text(&mut self, text: &str, time: f64) -> Result<bool, EditError> {
        if text.is_empty() {
            return Ok(false);
        }
        let eligible = time.is_finite() && !text.contains(['\r', '\n']);
        let join = eligible
            && self.typing.as_ref().is_some_and(|group| {
                group.revision == self.revision
                    && group.selections == *self.selections()
                    && time >= group.time
                    && time - group.time <= TYPING_PAUSE_SECONDS
            });
        let changed = self.replace_set_with_history(&self.selections().clone(), &[text], join)?;
        if changed && eligible {
            self.typing = Some(TypingGroup {
                revision: self.revision,
                selections: self.selections().clone(),
                time,
            });
        }
        Ok(changed)
    }

    pub fn break_undo_group(&mut self) {
        self.typing = None;
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
        self.replace_set_with_history(selections, fragments, false)
    }

    fn replace_set_with_history(
        &mut self,
        selections: &SelectionSet,
        fragments: &[&str],
        join: bool,
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
        Ok(self.commit_snapshot_with_history(text, selections, join))
    }

    pub(crate) fn commit_snapshot(&mut self, text: String, selections: SelectionSet) -> bool {
        self.commit_snapshot_with_history(text, selections, false)
    }

    fn commit_snapshot_with_history(
        &mut self,
        text: String,
        selections: SelectionSet,
        join: bool,
    ) -> bool {
        if text == self.text() {
            return false;
        }
        let previous = std::mem::replace(&mut self.current, Snapshot { text, selections });
        self.break_undo_group();
        if !join {
            self.undo.push(previous);
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn undo(&mut self) -> bool {
        self.break_undo_group();
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.current, previous));
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.break_undo_group();
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
