//! UI-independent document and workspace state.
mod document;
mod editor;
pub(crate) use document::{Document, display_name};
pub(crate) use editor::EditorState;
