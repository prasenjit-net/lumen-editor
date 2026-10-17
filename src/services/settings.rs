//! Config storage boundary. UI and settings resolution never access disk directly.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(crate) trait SettingsStore {
    fn read(&self, path: &Path) -> Result<Option<String>, String>;
    fn write(
        &self,
        path: &Path,
        root: Option<&Path>,
        expected: Option<&str>,
        content: &str,
    ) -> Result<(), String>;
}
pub(crate) struct LocalSettingsStore;
pub(crate) fn user_path() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".lumen/settings.toml"))
}
impl SettingsStore for LocalSettingsStore {
    fn read(&self, path: &Path) -> Result<Option<String>, String> {
        match fs::metadata(path) {
            Ok(metadata) if metadata.len() > 1024 * 1024 => {
                Err("Config file exceeds 1 MiB.".into())
            }
            Ok(_) => fs::read_to_string(path)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
    fn write(
        &self,
        path: &Path,
        root: Option<&Path>,
        expected: Option<&str>,
        content: &str,
    ) -> Result<(), String> {
        let operation = || -> Result<(), String> {
            let parent = path.parent().ok_or("Config has no parent directory.")?;
            // Refuse config symlinks rather than accidentally editing their targets.
            for component in [parent, path] {
                match fs::symlink_metadata(component) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err("Config path is a symbolic link; refusing to write.".into());
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.to_string()),
                }
            }
            if let Some(root) = root {
                let canonical_root = fs::canonicalize(root).map_err(|error| error.to_string())?;
                if parent != root.join(".lumen") {
                    return Err("Project config must be inside the opened directory.".into());
                }
                if parent.exists()
                    && !fs::canonicalize(parent)
                        .map_err(|error| error.to_string())?
                        .starts_with(canonical_root)
                {
                    return Err("Project config path escapes the opened directory.".into());
                }
            }
            if self.read(path)?.as_deref() != expected {
                return Err("Config file changed on disk. Reload before applying; your draft has been retained.".into());
            }
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            let mut staging =
                tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
            staging
                .write_all(content.as_bytes())
                .and_then(|()| staging.as_file().sync_all())
                .map_err(|error| error.to_string())?;
            if self.read(path)?.as_deref() != expected {
                return Err("Config file changed during save. Reload before applying.".into());
            }
            if expected.is_none() {
                staging
                    .persist_noclobber(path)
                    .map_err(|error| error.to_string())?;
            } else {
                staging.persist(path).map_err(|error| error.to_string())?;
            }
            Ok(())
        };
        operation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_missing_create_conflict_and_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".lumen/settings.toml");
        let store = LocalSettingsStore;
        assert_eq!(store.read(&path).unwrap(), None);
        assert!(!path.parent().unwrap().exists());
        store
            .write(&path, Some(dir.path()), None, "schema_version = 1\n")
            .unwrap();
        assert!(
            store
                .write(&path, Some(dir.path()), None, "overwrite")
                .is_err()
        );
        store
            .write(
                &path,
                Some(dir.path()),
                Some("schema_version = 1\n"),
                "# replaced\n",
            )
            .unwrap();
        assert_eq!(store.read(&path).unwrap().as_deref(), Some("# replaced\n"));
    }
    #[cfg(unix)]
    #[test]
    fn project_symlinks_cannot_escape() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join(".lumen")).unwrap();
        assert!(
            LocalSettingsStore
                .write(
                    &root.path().join(".lumen/settings.toml"),
                    Some(root.path()),
                    None,
                    ""
                )
                .is_err()
        );
        assert!(!outside.path().join("settings.toml").exists());
    }
}
