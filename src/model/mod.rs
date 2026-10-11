//! UI-independent document and workspace state.
mod document;
mod document_id;
mod editor;
mod file_name;
pub(crate) mod search;
pub(crate) use document::{Document, display_name};
pub(crate) use document_id::DocumentId;
pub(crate) use editor::EditorState;
pub(crate) use file_name::{validate_file_name, validate_relative_name};
