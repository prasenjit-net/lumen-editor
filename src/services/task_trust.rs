//! Remembered "I reviewed this workspace task" decisions, keyed by workspace and
//! the definition's execution digest. A changed command or dependency has a new
//! digest and is reviewed again; this is a disclosure record, not a sandbox.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

const MAX_WORKSPACES: usize = 200;
const MAX_DIGESTS: usize = 200;

pub(crate) trait TaskTrustStore {
    fn is_trusted(&self, workspace: &Path, digest: &str) -> bool;
    fn trust(&mut self, workspace: &Path, digest: &str) -> Result<(), String>;
}

/// In-memory store: the default, so tests and unconfigured apps never touch the
/// real home directory.
#[derive(Default)]
pub(crate) struct MemoryTrust(Vec<(PathBuf, String)>);

impl TaskTrustStore for MemoryTrust {
    fn is_trusted(&self, workspace: &Path, digest: &str) -> bool {
        self.0
            .iter()
            .any(|(path, d)| path == workspace && d == digest)
    }
    fn trust(&mut self, workspace: &Path, digest: &str) -> Result<(), String> {
        if !self.is_trusted(workspace, digest) {
            self.0.push((workspace.to_path_buf(), digest.into()));
        }
        Ok(())
    }
}

pub(crate) struct FileTrust {
    path: PathBuf,
}

impl FileTrust {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn user_path() -> Option<PathBuf> {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|home| !home.is_empty())
            .map(|home| PathBuf::from(home).join(".lumen/task-trust.toml"))
    }

    fn read(&self) -> Result<DocumentMut, String> {
        match fs::read_to_string(&self.path) {
            Ok(text) => text.parse().map_err(|error| format!("{error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok("schema_version = 1\n".parse().unwrap())
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

impl TaskTrustStore for FileTrust {
    fn is_trusted(&self, workspace: &Path, digest: &str) -> bool {
        let Ok(document) = self.read() else {
            // An unreadable record means "not reviewed": ask again rather than trust.
            return false;
        };
        let Some(Item::ArrayOfTables(tables)) = document.get("workspaces") else {
            return false;
        };
        tables.iter().any(|table| {
            table.get("path").and_then(Item::as_str) == workspace.to_str()
                && table
                    .get("digests")
                    .and_then(Item::as_array)
                    .is_some_and(|digests| digests.iter().any(|d| d.as_str() == Some(digest)))
        })
    }

    fn trust(&mut self, workspace: &Path, digest: &str) -> Result<(), String> {
        let workspace = workspace
            .to_str()
            .ok_or("The workspace path is not UTF-8")?;
        let mut document = self.read()?;
        if document.get("workspaces").is_none() {
            document["workspaces"] = Item::ArrayOfTables(ArrayOfTables::new());
        }
        let Some(Item::ArrayOfTables(tables)) = document.get_mut("workspaces") else {
            return Err("The trust file is malformed".into());
        };
        let position = tables
            .iter()
            .position(|table| table.get("path").and_then(Item::as_str) == Some(workspace));
        let table = match position {
            Some(position) => tables.get_mut(position).expect("position is valid"),
            None => {
                let mut table = Table::new();
                table["path"] = value(workspace);
                table["digests"] = value(Array::new());
                tables.push(table);
                tables.get_mut(tables.len() - 1).expect("just pushed")
            }
        };
        let digests = table
            .get_mut("digests")
            .and_then(Item::as_array_mut)
            .ok_or("The trust file is malformed")?;
        if digests.iter().any(|d| d.as_str() == Some(digest)) {
            return Ok(());
        }
        digests.push(digest);
        while digests.len() > MAX_DIGESTS {
            digests.remove(0);
        }
        while tables.len() > MAX_WORKSPACES {
            tables.remove(0);
        }
        let parent = self
            .path
            .parent()
            .ok_or("The trust file has no directory")?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let mut staging = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        staging
            .write_all(document.to_string().as_bytes())
            .and_then(|()| staging.as_file().sync_all())
            .map_err(|error| error.to_string())?;
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
    fn trust_is_per_workspace_and_digest() {
        let mut store = MemoryTrust::default();
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        assert!(!store.is_trusted(a, "d1"));
        store.trust(a, "d1").unwrap();
        assert!(store.is_trusted(a, "d1"));
        assert!(
            !store.is_trusted(a, "d2"),
            "a changed definition is reviewed again"
        );
        assert!(
            !store.is_trusted(b, "d1"),
            "trust never leaks to another workspace"
        );
    }

    #[test]
    fn the_file_store_persists_survives_reopening_and_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".lumen/task-trust.toml");
        let mut store = FileTrust::new(path.clone());
        let workspace = dir.path().join("proj");
        assert!(!store.is_trusted(&workspace, "x"));
        store.trust(&workspace, "x").unwrap();
        store.trust(&workspace, "x").unwrap();
        assert!(FileTrust::new(path.clone()).is_trusted(&workspace, "x"));
        assert!(!FileTrust::new(path.clone()).is_trusted(&workspace, "y"));
        for n in 0..(MAX_DIGESTS + 5) {
            store.trust(&workspace, &format!("d{n}")).unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("\"x\""),
            "oldest digests are evicted: {}",
            text.len()
        );
        assert!(store.is_trusted(&workspace, &format!("d{}", MAX_DIGESTS + 4)));
    }

    #[test]
    fn an_unreadable_file_means_not_trusted_and_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trust.toml");
        fs::write(&path, "not [ toml").unwrap();
        let mut store = FileTrust::new(path.clone());
        assert!(!store.is_trusted(Path::new("/w"), "x"));
        assert!(store.trust(Path::new("/w"), "x").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not [ toml");
    }
}
