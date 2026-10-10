use crate::model::display_name;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub(crate) trait FileSystem {
    fn read_text(&self, path: &Path) -> io::Result<String>;
    fn write_text(&self, path: &Path, text: &str) -> io::Result<()>;
    fn children(&self, path: &Path) -> Vec<PathBuf>;
    fn is_directory(&self, path: &Path) -> bool;
}
pub(crate) struct LocalFileSystem;
impl FileSystem for LocalFileSystem {
    fn read_text(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }
    fn write_text(&self, path: &Path, text: &str) -> io::Result<()> {
        fs::write(path, text)
    }
    fn children(&self, path: &Path) -> Vec<PathBuf> {
        sorted_children(path)
    }
    fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }
}
fn sorted_children(path: &Path) -> Vec<PathBuf> {
    let mut children: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".git") | Some("target") | Some(".DS_Store")
            )
        })
        .collect();
    children.sort_by(|left, right| {
        right.is_dir().cmp(&left.is_dir()).then_with(|| {
            display_name(left)
                .to_lowercase()
                .cmp(&display_name(right).to_lowercase())
        })
    });
    children
}
