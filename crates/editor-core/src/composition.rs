use crate::{EditError, EditorBuffer, SelectionSet};

/// Transient composition: no document mutation or history until one commit.
/// Preedit is shown at the primary selection; commit broadcasts once to the
/// captured set. The surface must cancel on focus/document/pointer changes.
pub struct Composition {
    revision: u64,
    selections: SelectionSet,
    preedit: String,
}

impl Composition {
    pub fn begin(buffer: &EditorBuffer) -> Self {
        Self {
            revision: buffer.revision(),
            selections: buffer.selections().clone(),
            preedit: String::new(),
        }
    }

    pub fn is_current(&self, buffer: &EditorBuffer) -> bool {
        self.revision == buffer.revision() && self.selections == *buffer.selections()
    }

    pub fn update(&mut self, preedit: String) {
        self.preedit = preedit;
    }
    pub fn preedit(&self) -> &str {
        &self.preedit
    }
    pub fn insertion_byte(&self) -> usize {
        self.selections.selections()[self.selections.primary_index()]
            .range()
            .start
    }

    pub fn commit(self, buffer: &mut EditorBuffer, text: &str) -> Result<bool, EditError> {
        if !self.is_current(buffer) {
            return Err(EditError::StaleComposition);
        }
        // Empty commit is cancellation, not deletion of captured selections.
        if text.is_empty() {
            return Ok(false);
        }
        buffer.replace_selections(&[text])
    }
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;
