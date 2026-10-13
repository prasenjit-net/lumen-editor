use crate::model::display_name;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub(crate) trait FileSystem {
    fn read_text(&self, path: &Path) -> io::Result<String>;
    fn write_text(&self, path: &Path, text: &str) -> io::Result<()>;
    /// Refuse to replace a file whose contents changed since it was opened/saved.
    fn write_text_if_unchanged(&self, path: &Path, expected: &str, text: &str) -> io::Result<()> {
        if self.read_text(path)? != expected {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "File changed on disk",
            ));
        }
        self.write_text(path, text)
    }
    fn write_text_if_missing(&self, path: &Path, text: &str) -> io::Result<()> {
        if self.entry_exists(path) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "File appeared on disk",
            ));
        }
        self.write_text(path, text)
    }
    /// Guard a workspace replacement against paths that escape the opened root.
    fn write_workspace_replace(
        &self,
        root: &Path,
        path: &Path,
        expected: &str,
        text: &str,
    ) -> io::Result<()> {
        if !path.starts_with(root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Path is outside the workspace",
            ));
        }
        self.write_text_if_unchanged(path, expected, text)
    }
    /// Atomically create an empty file; never replace an existing entry.
    fn create_file(&self, path: &Path) -> io::Result<()>;
    fn create_directory(&self, path: &Path) -> io::Result<()>;
    fn rename_entry(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy_entry(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn trash_entry(&self, path: &Path) -> io::Result<()>;
    fn entry_exists(&self, path: &Path) -> bool;
    fn children(&self, path: &Path) -> Vec<PathBuf>;
    fn is_directory(&self, path: &Path) -> bool;
}
pub(crate) struct LocalFileSystem;
impl FileSystem for LocalFileSystem {
    fn read_text(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }
    fn write_text(&self, path: &Path, text: &str) -> io::Result<()> {
        atomic_replace(path, text, None)
    }
    fn write_text_if_unchanged(&self, path: &Path, expected: &str, text: &str) -> io::Result<()> {
        atomic_replace(path, text, Some(expected))
    }
    fn write_text_if_missing(&self, path: &Path, text: &str) -> io::Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut staging = tempfile::NamedTempFile::new_in(parent)?;
        use io::Write;
        staging.write_all(text.as_bytes())?;
        staging.as_file().sync_all()?;
        staging
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
        Ok(())
    }
    fn write_workspace_replace(
        &self,
        root: &Path,
        path: &Path,
        expected: &str,
        text: &str,
    ) -> io::Result<()> {
        let workspace = fs::canonicalize(root)?;
        let target = fs::canonicalize(path)?;
        if !target.starts_with(&workspace) || !path.starts_with(root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Path escapes the workspace",
            ));
        }
        let mut component = root.to_path_buf();
        for part in path.strip_prefix(root).unwrap().components() {
            component.push(part);
            if fs::symlink_metadata(&component)?.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Workspace path became a symbolic link",
                ));
            }
        }
        self.write_text_if_unchanged(path, expected, text)
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
    fn create_directory(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }
    fn rename_entry(&self, from: &Path, to: &Path) -> io::Result<()> {
        super::file_operations::rename_no_replace(from, to)
    }
    fn copy_entry(&self, from: &Path, to: &Path) -> io::Result<()> {
        super::file_operations::copy_no_replace(from, to)
    }
    fn trash_entry(&self, path: &Path) -> io::Result<()> {
        trash::delete(path).map_err(io::Error::other)
    }
    fn entry_exists(&self, path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok()
    }
}

fn atomic_replace(path: &Path, text: &str, expected: Option<&str>) -> io::Result<()> {
    if let Some(expected) = expected
        && fs::read_to_string(path)? != expected
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "File changed on disk",
        ));
    }
    // Follow an opened symlink rather than replacing the link itself.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut staging = tempfile::NamedTempFile::new_in(parent)?;
    use io::Write;
    staging.write_all(text.as_bytes())?;
    if let Ok(metadata) = fs::metadata(&target) {
        staging.as_file().set_permissions(metadata.permissions())?;
    }
    staging.as_file().sync_all()?;
    if let Some(expected) = expected
        && fs::read_to_string(path)? != expected
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "File changed during save",
        ));
    }
    staging.persist(&target).map_err(|error| error.error)?;
    Ok(())
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
        let directory_error = LocalFileSystem.create_file(&directory).unwrap_err();
        // Windows reports access denied when create_new targets a directory.
        assert!(matches!(
            directory_error.kind(),
            io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
        ));
        assert!(directory.is_dir());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn guarded_save_preserves_external_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        fs::write(&path, "external").unwrap();
        let error = LocalFileSystem
            .write_text_if_unchanged(&path, "original", "mine")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        LocalFileSystem
            .write_text_if_unchanged(&path, "external", "mine")
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine");
    }

    #[cfg(unix)]
    #[test]
    fn workspace_replace_rejects_retargeted_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secret.txt");
        fs::write(&target, "same text").unwrap();
        let linked = root.path().join("result.txt");
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let error = LocalFileSystem
            .write_workspace_replace(root.path(), &linked, "same text", "replacement")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(target).unwrap(), "same text");
    }
}
