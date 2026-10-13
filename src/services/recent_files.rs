//! Small, user-scoped Quick Open history. Never stored in a project directory.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use toml_edit::{Array, DocumentMut, value};

const MAX_BYTES: u64 = 64 * 1024;

pub(crate) trait RecentFilesStore {
    fn load(&self) -> Result<Vec<PathBuf>, String>;
    fn save(&self, paths: &[PathBuf]) -> Result<(), String>;
}

pub(crate) struct LocalRecentFilesStore {
    path: PathBuf,
}

impl LocalRecentFilesStore {
    pub(crate) fn new(settings_path: &Path) -> Self {
        Self {
            path: settings_path.with_file_name("recent-files.toml"),
        }
    }

    fn reject_symlink(path: &Path) -> Result<(), String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(format!("{} is a symbolic link", path.display()))
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

impl RecentFilesStore for LocalRecentFilesStore {
    fn load(&self) -> Result<Vec<PathBuf>, String> {
        Self::reject_symlink(&self.path)?;
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        if metadata.len() > MAX_BYTES {
            return Err("Recent files history exceeds 64 KiB".into());
        }
        let text = fs::read_to_string(&self.path).map_err(|error| error.to_string())?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid recent files history: {error}"))?;
        let paths = document["paths"]
            .as_array()
            .ok_or("Recent files history has no paths array")?;
        paths
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| "Recent files history contains a non-string path".into())
            })
            .collect()
    }

    fn save(&self, paths: &[PathBuf]) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or("Recent files path has no parent")?;
        Self::reject_symlink(parent)?;
        Self::reject_symlink(&self.path)?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut array = Array::new();
        for path in paths {
            if let Some(path) = path.to_str() {
                array.push(path);
            }
        }
        let mut document = DocumentMut::new();
        document["paths"] = value(array);
        let mut staging =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        staging
            .write_all(document.to_string().as_bytes())
            .and_then(|()| staging.as_file().sync_all())
            .map_err(|error| error.to_string())?;
        Self::reject_symlink(&self.path)?;
        staging
            .persist(&self.path)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_round_trips_quoted_paths_and_rejects_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalRecentFilesStore::new(&temp.path().join("settings.toml"));
        assert!(store.load().unwrap().is_empty());
        let paths = vec![temp.path().join("a \"quoted\" file.rs")];
        store.save(&paths).unwrap();
        assert_eq!(store.load().unwrap(), paths);
        fs::write(&store.path, "paths = [4]").unwrap();
        assert!(store.load().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn history_symlink_is_not_followed_for_read_or_write() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = LocalRecentFilesStore::new(&temp.path().join("settings.toml"));
        let target = outside.path().join("target.toml");
        fs::write(&target, "paths = []").unwrap();
        std::os::unix::fs::symlink(&target, &store.path).unwrap();
        assert!(store.load().is_err());
        assert!(store.save(&[]).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "paths = []");
    }
}
