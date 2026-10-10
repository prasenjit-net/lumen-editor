//! UI-independent document and workspace state.
mod document;
mod editor;
mod file_name;
pub(crate) use document::{Document, display_name};
pub(crate) use editor::EditorState;
pub(crate) use file_name::{validate_file_name, validate_relative_name};
