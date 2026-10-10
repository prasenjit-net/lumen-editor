//! Replaceable boundaries for filesystem access and native file selection.
mod dialogs;
mod filesystem;
pub(crate) use dialogs::{FileDialogs, NativeDialogs};
pub(crate) use filesystem::{FileSystem, LocalFileSystem};
