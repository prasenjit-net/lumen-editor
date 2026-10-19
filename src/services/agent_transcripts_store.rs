//! Where cached conversations live on disk: one small file per provider session in
//! `~/.lumen/agent-transcripts/`, named by the provider's own session id so a lost
//! or recreated session index can never attach one conversation to another chat.
//! Writes are atomic (temp file, then rename).
// Used by the native agent glue when native agents are enabled.
#![allow(dead_code)]
use crate::model::agent_transcript;
use lumen_agent_core::{ProviderId, model::TranscriptItem};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn user_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".lumen/agent-transcripts"))
}

/// A file name for a provider session; anything unusual in the id is replaced so
/// a hostile id cannot name a path outside the folder.
pub(crate) fn file_name(provider: ProviderId, native: &str) -> String {
    let safe: String = native
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    format!("{}-{safe}.json", provider.as_str())
}

/// A missing file is simply no cached conversation.
pub(crate) fn load(
    dir: &Path,
    provider: ProviderId,
    native: &str,
) -> Result<Vec<TranscriptItem>, String> {
    let path = dir.join(file_name(provider, native));
    match fs::read_to_string(&path) {
        Ok(text) => agent_transcript::from_json(&text).map(|(items, _)| items),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

pub(crate) fn save(
    dir: &Path,
    provider: ProviderId,
    native: &str,
    items: &[TranscriptItem],
) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join(file_name(provider, native));
    let temp = path.with_extension("json.tmp");
    let mut file =
        fs::File::create(&temp).map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    file.write_all(agent_transcript::to_json(items).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("could not write {}: {e}", temp.display()))?;
    fs::rename(&temp, &path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{ItemId, model::ItemKind};

    fn dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumen-transcripts-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_saved_conversation_comes_back_and_a_missing_one_is_empty() {
        let dir = dir("roundtrip");
        assert!(load(&dir, ProviderId::Claude, "abc").unwrap().is_empty());
        let items = vec![TranscriptItem {
            id: ItemId("u".into()),
            turn: None,
            kind: ItemKind::User("hi".into()),
        }];
        save(&dir, ProviderId::Claude, "abc", &items).unwrap();
        assert_eq!(load(&dir, ProviderId::Claude, "abc").unwrap(), items);
        assert!(
            load(&dir, ProviderId::Codex, "abc").unwrap().is_empty(),
            "providers are separate"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_ids_cannot_leave_the_folder_and_damaged_files_are_errors() {
        let name = file_name(ProviderId::Codex, "../../etc/passwd");
        assert!(!name.contains('/') && !name.contains(".."), "{name}");
        let dir = dir("damaged");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(file_name(ProviderId::Codex, "x")), "garbage").unwrap();
        assert!(load(&dir, ProviderId::Codex, "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
