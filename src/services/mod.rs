//! Replaceable filesystem, dialog and clipboard boundaries.
mod clipboard;
mod dialogs;
mod file_operations;
mod filesystem;
pub(crate) mod recovery;
mod reveal;
pub(crate) mod settings;
pub(crate) use clipboard::{ClipboardRead, ClipboardRequestId, ClipboardService, NativeClipboard};
pub(crate) use dialogs::{FileDialogs, NativeDialogs};
pub(crate) use filesystem::{FileSystem, LocalFileSystem};
pub(crate) use reveal::reveal_in_file_manager;
