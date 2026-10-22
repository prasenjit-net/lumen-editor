use rfd::FileDialog;
use std::path::PathBuf;
pub(crate) trait FileDialogs {
    fn open_file(&self) -> Option<PathBuf>;
    fn open_folder(&self) -> Option<PathBuf>;
    fn save_as(&self, name: &str) -> Option<PathBuf>;
    /// Files to attach to a chat message; empty when cancelled.
    fn pick_attachments(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}
pub(crate) struct NativeDialogs;
impl FileDialogs for NativeDialogs {
    fn open_file(&self) -> Option<PathBuf> {
        FileDialog::new().set_title("Open File").pick_file()
    }
    fn open_folder(&self) -> Option<PathBuf> {
        FileDialog::new().set_title("Open Folder").pick_folder()
    }
    fn pick_attachments(&self) -> Vec<PathBuf> {
        FileDialog::new()
            .set_title("Attach Files")
            .pick_files()
            .unwrap_or_default()
    }
    fn save_as(&self, name: &str) -> Option<PathBuf> {
        FileDialog::new()
            .set_title("Save As")
            .set_file_name(name)
            .save_file()
    }
}
