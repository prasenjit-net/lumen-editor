//! Where the native-agent session index lives on disk. Writes are atomic
//! (temp file, then rename) so a crash never leaves a half-written index, and a
//! damaged or newer-format file is reported instead of being overwritten silently.
// Used by the native agent glue when native agents are enabled.
#![allow(dead_code)]
use crate::model::agent_registry::Registry;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn user_path() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".lumen/agent-sessions.json"))
}

/// A missing file is a fresh index. An unreadable one is an error so the caller
/// can keep the file untouched rather than replace it.
pub(crate) fn load(path: &Path) -> Result<Registry, String> {
    match fs::read_to_string(path) {
        Ok(text) => Registry::from_json(&text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Registry::new()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

pub(crate) fn save(path: &Path, registry: &Registry) -> Result<(), String> {
    let parent = path.parent().ok_or("the session index has no folder")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    let temp = path.with_extension("json.tmp");
    let mut file =
        fs::File::create(&temp).map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    file.write_all(registry.to_json().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    fs::rename(&temp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{ProfileId, ProviderId};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-sessions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_missing_file_is_empty_and_saved_data_round_trips_atomically() {
        let dir = scratch("roundtrip");
        let path = dir.join("nested/agent-sessions.json");
        assert!(load(&path).unwrap().records().is_empty());
        let mut registry = Registry::new();
        registry.create(
            ProviderId::Claude,
            ProfileId::default_profile(),
            "/w".into(),
            5,
        );
        save(&path, &registry).unwrap();
        assert!(!path.with_extension("json.tmp").exists());
        assert_eq!(load(&path).unwrap().records(), registry.records());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_newer_file_is_an_error_and_stays_untouched() {
        let dir = scratch("damaged");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-sessions.json");
        fs::write(&path, r#"{"schema_version":99,"sessions":[]}"#).unwrap();
        assert!(load(&path).is_err());
        assert!(fs::read_to_string(&path).unwrap().contains("99"));
        let _ = fs::remove_dir_all(&dir);
    }
}
