//! Isolated multi-selection backend spike. No UI, I/O, or active TextEdit history.
mod buffer;
mod composition;
mod selection;

pub use buffer::{Direction, EditorBuffer};
pub use composition::Composition;
pub use selection::{EditError, Selection, SelectionSet};

#[cfg(test)]
mod tests;
