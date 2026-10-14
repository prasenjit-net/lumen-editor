//! Replaceable filesystem, dialog and clipboard boundaries.
mod clipboard;
mod dialogs;
pub(crate) mod file_index;
mod file_operations;
mod filesystem;
pub(crate) mod recent_files;
pub(crate) mod recovery;
mod reveal;
pub(crate) mod settings;
pub(crate) mod source_control;
pub(crate) mod workspace_search;
pub(crate) use clipboard::{ClipboardRead, ClipboardRequestId, ClipboardService, NativeClipboard};
pub(crate) use dialogs::{FileDialogs, NativeDialogs};
pub(crate) use filesystem::{FileSystem, LocalFileSystem};
pub(crate) use reveal::reveal_in_file_manager;
