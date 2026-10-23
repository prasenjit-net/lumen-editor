//! The agent panel's remembered provider tab, stored next to the session index.
//! Losing or damaging it only forgets which tab was open, so errors fall back to
//! the default tab instead of being reported.
use lumen_agent_core::ProviderId;
use std::{
    fs,
    path::{Path, PathBuf},
};

const FILE_NAME: &str = "agent-panel.json";

/// The preferences file that lives beside the session index.
pub(crate) fn path_beside(index: &Path) -> PathBuf {
    index.with_file_name(FILE_NAME)
}

pub(crate) fn load(path: &Path) -> Option<ProviderId> {
    let text = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    ProviderId::parse(value.get("provider")?.as_str()?)
}

pub(crate) fn save(path: &Path, provider: ProviderId) -> Result<(), String> {
    let text = serde_json::json!({ "provider": provider.as_str() }).to_string();
    fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chosen_tab_round_trips_and_damage_means_the_default() {
        let dir = std::env::temp_dir().join(format!("lumen-panel-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = path_beside(&dir.join("agent-sessions.json"));
        assert_eq!(load(&path), None, "no file yet");
        save(&path, ProviderId::Claude).unwrap();
        assert_eq!(load(&path), Some(ProviderId::Claude));
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), None);
        fs::write(&path, r#"{"provider":"gemini"}"#).unwrap();
        assert_eq!(load(&path), None, "unknown providers are ignored");
        fs::remove_dir_all(dir).ok();
    }
}
