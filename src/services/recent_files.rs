//! Small, user-scoped history of recently opened files and folders. Never stored
//! in a project directory.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use toml_edit::{Array, DocumentMut, value};

const MAX_BYTES: u64 = 64 * 1024;

/// What the history file holds. Both lists are most-recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecentHistory {
    pub(crate) files: Vec<PathBuf>,
    pub(crate) folders: Vec<PathBuf>,
}

pub(crate) trait RecentFilesStore {
    fn load(&self) -> Result<RecentHistory, String>;
    fn save(&self, history: &RecentHistory) -> Result<(), String>;
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
    fn load(&self) -> Result<RecentHistory, String> {
        Self::reject_symlink(&self.path)?;
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(RecentHistory::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        if metadata.len() > MAX_BYTES {
            return Err("Recent files history exceeds 64 KiB".into());
        }
        let text = fs::read_to_string(&self.path).map_err(|error| error.to_string())?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|error| format!("Invalid recent files history: {error}"))?;
        let files = paths(
            document
                .get("paths")
                .ok_or("Recent files history has no paths array")?,
            "paths",
        )?;
        // Files written before folders were remembered simply lack this key.
        let folders = match document.get("folders") {
            Some(item) => paths(item, "folders")?,
            None => Vec::new(),
        };
        Ok(RecentHistory { files, folders })
    }

    fn save(&self, history: &RecentHistory) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or("Recent files path has no parent")?;
        Self::reject_symlink(parent)?;
        Self::reject_symlink(&self.path)?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        // Start from the existing document so keys this version does not know
        // about (written by a newer one) survive; a corrupt file is replaced.
        let mut document = fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| text.parse::<DocumentMut>().ok())
            .unwrap_or_default();
        document["paths"] = value(array(&history.files));
        document["folders"] = value(array(&history.folders));
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

fn paths(item: &toml_edit::Item, key: &str) -> Result<Vec<PathBuf>, String> {
    item.as_array()
        .ok_or_else(|| format!("Recent files history `{key}` is not an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| "Recent files history contains a non-string path".into())
        })
        .collect()
}

fn array(paths: &[PathBuf]) -> Array {
    let mut array = Array::new();
    for path in paths {
        if let Some(path) = path.to_str() {
            array.push(path);
        }
    }
    array
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(temp: &tempfile::TempDir) -> LocalRecentFilesStore {
        LocalRecentFilesStore::new(&temp.path().join("settings.toml"))
    }

    #[test]
    fn history_round_trips_quoted_paths_and_rejects_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        assert_eq!(store.load().unwrap(), RecentHistory::default());
        let history = RecentHistory {
            files: vec![temp.path().join("a \"quoted\" file.rs")],
            folders: vec![temp.path().join("my project"), temp.path().join("other")],
        };
        store.save(&history).unwrap();
        assert_eq!(store.load().unwrap(), history);
        fs::write(&store.path, "paths = [4]").unwrap();
        assert!(store.load().is_err());
        fs::write(&store.path, "paths = []\nfolders = 'nope'").unwrap();
        assert!(store.load().is_err());
        fs::write(&store.path, "paths = []\nfolders = [3]").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn files_written_before_folders_existed_still_load() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        fs::write(&store.path, "paths = [\"/a/b.rs\"]").unwrap();
        let history = store.load().unwrap();
        assert_eq!(history.files, [PathBuf::from("/a/b.rs")]);
        assert!(history.folders.is_empty());
    }

    #[test]
    fn saving_keeps_keys_this_version_does_not_know_and_replaces_corrupt_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        fs::write(&store.path, "paths = []\nfuture_key = 7\n").unwrap();
        store
            .save(&RecentHistory {
                files: vec![PathBuf::from("/x.rs")],
                folders: Vec::new(),
            })
            .unwrap();
        let text = fs::read_to_string(&store.path).unwrap();
        assert!(text.contains("future_key = 7"), "{text}");
        assert_eq!(store.load().unwrap().files, [PathBuf::from("/x.rs")]);
        fs::write(&store.path, "this is { not toml").unwrap();
        store.save(&RecentHistory::default()).unwrap();
        assert_eq!(store.load().unwrap(), RecentHistory::default());
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
        assert!(store.save(&RecentHistory::default()).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "paths = []");
    }
}
