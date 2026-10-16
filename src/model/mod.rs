//! UI-independent document and workspace state.
pub(crate) mod breadcrumbs;
mod document;
mod document_id;
mod editor;
mod file_name;
pub(crate) mod line_diff;
pub(crate) mod markdown_lite;
pub(crate) mod output_log;
pub(crate) mod search;
pub(crate) mod status_info;
pub(crate) mod text_line;
pub(crate) use document::{Document, display_name};
pub(crate) use document_id::DocumentId;
pub(crate) use editor::EditorState;
pub(crate) use file_name::{validate_file_name, validate_relative_name};
