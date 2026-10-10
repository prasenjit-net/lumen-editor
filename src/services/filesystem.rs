use crate::model::display_name;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub(crate) trait FileSystem {
    fn read_text(&self, path: &Path) -> io::Result<String>;
    fn write_text(&self, path: &Path, text: &str) -> io::Result<()>;
    /// Atomically create an empty file; never replace an existing entry.
    fn create_file(&self, path: &Path) -> io::Result<()>;
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
    fn create_file(&self, path: &Path) -> io::Result<()> {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|_| ())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_create_is_empty_and_never_overwrites_files_or_directories() {
        let directory = std::env::temp_dir().join(format!(
            "lumen-create-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("created.txt");
        LocalFileSystem.create_file(&path).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        fs::write(&path, "must not be overwritten").unwrap();
        assert_eq!(
            LocalFileSystem.create_file(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "must not be overwritten"
        );
        assert_eq!(
            LocalFileSystem.create_file(&directory).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
