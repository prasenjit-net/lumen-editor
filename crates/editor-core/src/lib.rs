//! Isolated multi-selection backend spike. No UI, I/O, or active TextEdit history.
mod buffer;
mod clipboard;
mod commands;
mod composition;
mod navigation;
mod selection;
pub mod syntax;
pub mod text_line;

pub use buffer::{Direction, EditorBuffer};
pub use clipboard::SelectionClipboard;
pub use composition::Composition;
pub use selection::{CaretAffinity, EditError, Selection, SelectionAffinity, SelectionSet};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod typing_tests;
