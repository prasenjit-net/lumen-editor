//! The session index: every conversation Lumen knows about, whether or not it has
//! a tab, a running process or even a native identity yet.
//!
//! A [`SessionId`] is Lumen's own and stable; the provider's identity is attached
//! once it exists and is namespaced by provider and profile. Discovery merges
//! provider listings into the index by native identity, so history that was
//! started in a terminal appears without duplicating a session Lumen already has.
//! Serialization is JSON with a schema version; unknown fields and unreadable
//! rows are tolerated and counted, never fatal, and no secrets are stored.
use lumen_agent_core::{NativeSessionRef, ProfileId, ProviderId, SessionId, api::SessionSummary};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(crate) const SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SessionRecord {
    pub(crate) id: SessionId,
    pub(crate) provider: ProviderId,
    pub(crate) profile: ProfileId,
    /// The provider's id, once the session exists natively.
    pub(crate) native: Option<String>,
    /// Canonical working directory the session was created for.
    pub(crate) workspace: PathBuf,
    pub(crate) title: String,
    /// The user chose this title, so discovery must not overwrite it.
    pub(crate) title_is_custom: bool,
    pub(crate) pinned: bool,
    pub(crate) archived: bool,
    /// Seconds since the Unix epoch.
    pub(crate) updated: u64,
    /// Short note of how the last turn ended ("completed", "failed", ...).
    pub(crate) last_outcome: Option<String>,
    /// The user's explicit setting choices (model, effort, ...) that differ from
    /// the provider defaults, re-applied when the session is resumed. Provider
    /// observations are never stored here.
    pub(crate) prefs: Vec<(String, String)>,
}

impl SessionRecord {
    pub(crate) fn native_ref(&self) -> Option<NativeSessionRef> {
        self.native.as_ref().map(|id| NativeSessionRef {
            provider: self.provider,
            profile: self.profile.clone(),
            id: id.clone(),
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Registry {
    records: Vec<SessionRecord>,
    next_id: u64,
    /// Rows that could not be read when loading (kept for a diagnostic).
    pub(crate) skipped_rows: usize,
}

pub(crate) const DEFAULT_TITLE: &str = "New chat";

impl Registry {
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn records(&self) -> &[SessionRecord] {
        &self.records
    }

    pub(crate) fn get(&self, id: SessionId) -> Option<&SessionRecord> {
        self.records.iter().find(|r| r.id == id)
    }

    fn get_mut(&mut self, id: SessionId) -> Option<&mut SessionRecord> {
        self.records.iter_mut().find(|r| r.id == id)
    }

    /// A new local session with no native identity yet.
    pub(crate) fn create(
        &mut self,
        provider: ProviderId,
        profile: ProfileId,
        workspace: PathBuf,
        now: u64,
    ) -> SessionId {
        let id = SessionId(self.next_id.max(1));
        self.next_id = id.0 + 1;
        self.records.push(SessionRecord {
            id,
            provider,
            profile,
            native: None,
            workspace,
            title: DEFAULT_TITLE.into(),
            title_is_custom: false,
            pinned: false,
            archived: false,
            updated: now,
            last_outcome: None,
            prefs: Vec::new(),
        });
        id
    }

    pub(crate) fn find_native(&self, native: &NativeSessionRef) -> Option<SessionId> {
        self.records
            .iter()
            .find(|r| {
                r.provider == native.provider
                    && r.profile == native.profile
                    && r.native.as_deref() == Some(native.id.as_str())
            })
            .map(|r| r.id)
    }

    /// Record the provider's identity. Refuses to attach one native session to two
    /// Lumen sessions, and never replaces an identity already set.
    pub(crate) fn set_native(&mut self, id: SessionId, native: &NativeSessionRef) -> bool {
        if self.find_native(native).is_some_and(|owner| owner != id) {
            return false;
        }
        match self.get_mut(id) {
            Some(record)
                if record.provider == native.provider
                    && record.profile == native.profile
                    && record.native.as_ref().is_none_or(|n| n == &native.id) =>
            {
                record.native = Some(native.id.clone());
                true
            }
            _ => false,
        }
    }

    /// A first message names an untitled session; a custom title is never replaced.
    pub(crate) fn suggest_title(&mut self, id: SessionId, text: &str) {
        let Some(record) = self.get_mut(id) else {
            return;
        };
        if record.title_is_custom || record.title != DEFAULT_TITLE {
            return;
        }
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let title: String = line.trim().chars().take(60).collect();
        if !title.is_empty() {
            record.title = title;
        }
    }

    // Not reachable from the UI yet (session rename, remove and discovery).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn rename(&mut self, id: SessionId, title: &str) -> bool {
        let title = title.trim();
        match self.get_mut(id) {
            Some(record) if !title.is_empty() => {
                record.title = title.chars().take(120).collect();
                record.title_is_custom = true;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn set_pinned(&mut self, id: SessionId, pinned: bool) {
        if let Some(record) = self.get_mut(id) {
            record.pinned = pinned;
        }
    }

    pub(crate) fn set_archived(&mut self, id: SessionId, archived: bool) {
        if let Some(record) = self.get_mut(id) {
            record.archived = archived;
        }
    }

    pub(crate) fn touch(&mut self, id: SessionId, now: u64, outcome: Option<&str>) {
        if let Some(record) = self.get_mut(id) {
            record.updated = record.updated.max(now);
            if let Some(outcome) = outcome {
                record.last_outcome = Some(outcome.into());
            }
        }
    }

    pub(crate) fn set_prefs(&mut self, id: SessionId, prefs: Vec<(String, String)>) {
        if let Some(record) = self.get_mut(id)
            && record.prefs != prefs
        {
            record.prefs = prefs;
        }
    }

    // Not reachable from the UI yet (session rename, remove and discovery).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn remove(&mut self, id: SessionId) -> bool {
        let before = self.records.len();
        self.records.retain(|r| r.id != id);
        self.records.len() != before
    }

    /// Fold a provider listing into the index. Known sessions only refresh their
    /// time and (non-custom) title; unknown ones are added. Returns how many were
    /// added. A listing says nothing about rows it does not contain.
    // Not reachable from the UI yet (session rename, remove and discovery).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn merge_discovered(
        &mut self,
        summaries: &[SessionSummary],
        fallback_workspace: &Path,
    ) -> usize {
        let mut added = 0;
        for summary in summaries {
            match self.find_native(&summary.native) {
                Some(id) => {
                    if let Some(record) = self.get_mut(id) {
                        if let Some(time) = summary.last_activity {
                            record.updated = record.updated.max(time);
                        }
                        if !record.title_is_custom
                            && let Some(title) = summary.title.as_ref().filter(|t| !t.is_empty())
                        {
                            record.title = title.clone();
                        }
                        record.archived = summary.archived;
                    }
                }
                None => {
                    let id = self.create(
                        summary.native.provider,
                        summary.native.profile.clone(),
                        summary
                            .workspace
                            .clone()
                            .unwrap_or_else(|| fallback_workspace.to_path_buf()),
                        summary.last_activity.unwrap_or(0),
                    );
                    if let Some(record) = self.get_mut(id) {
                        record.native = Some(summary.native.id.clone());
                        record.archived = summary.archived;
                        if let Some(title) = summary.title.as_ref().filter(|t| !t.is_empty()) {
                            record.title = title.clone();
                        }
                    }
                    added += 1;
                }
            }
        }
        added
    }

    /// Visible rows: pinned first, then most recently active; optionally only one
    /// workspace and with or without archived sessions.
    pub(crate) fn listing(
        &self,
        workspace: Option<&Path>,
        include_archived: bool,
    ) -> Vec<&SessionRecord> {
        let mut rows: Vec<&SessionRecord> = self
            .records
            .iter()
            .filter(|r| include_archived || !r.archived)
            .filter(|r| workspace.is_none_or(|w| r.workspace == w))
            .collect();
        rows.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.updated.cmp(&a.updated))
                .then(b.id.cmp(&a.id))
        });
        rows
    }

    // ---------------------------------------------------------- persistence

    pub(crate) fn to_json(&self) -> String {
        let rows: Vec<Value> = self
            .records
            .iter()
            .map(|r| {
                json!({
                    "id": r.id.0,
                    "provider": r.provider.as_str(),
                    "profile": r.profile.0,
                    "native": r.native,
                    "workspace": r.workspace.to_string_lossy(),
                    "title": r.title,
                    "title_is_custom": r.title_is_custom,
                    "pinned": r.pinned,
                    "archived": r.archived,
                    "updated": r.updated,
                    "last_outcome": r.last_outcome,
                    "prefs": r.prefs.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({"schema_version": SCHEMA_VERSION, "sessions": rows}))
            .unwrap_or_default()
    }

    /// A newer schema or damaged file yields an empty index and a diagnostic
    /// string rather than overwriting data this build cannot understand.
    pub(crate) fn from_json(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| format!("unreadable: {e}"))?;
        let version = value.get("schema_version").and_then(Value::as_u64);
        if version.is_none_or(|v| v > SCHEMA_VERSION) {
            return Err(format!(
                "session index has an unsupported version ({version:?})"
            ));
        }
        let mut registry = Self::new();
        for row in value
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match read_record(row) {
                Some(record) if registry.get(record.id).is_none() => {
                    registry.next_id = registry.next_id.max(record.id.0 + 1);
                    registry.records.push(record);
                }
                _ => registry.skipped_rows += 1,
            }
        }
        Ok(registry)
    }
}

fn read_record(row: &Value) -> Option<SessionRecord> {
    let text = |key: &str| row.get(key).and_then(Value::as_str);
    let flag = |key: &str| row.get(key).and_then(Value::as_bool).unwrap_or(false);
    Some(SessionRecord {
        id: SessionId(row.get("id")?.as_u64().filter(|id| *id > 0)?),
        provider: ProviderId::parse(text("provider")?)?,
        profile: ProfileId(text("profile").unwrap_or("default").to_string()),
        native: text("native").map(str::to_string),
        workspace: PathBuf::from(text("workspace")?),
        title: text("title").unwrap_or(DEFAULT_TITLE).to_string(),
        title_is_custom: flag("title_is_custom"),
        pinned: flag("pinned"),
        archived: flag("archived"),
        updated: row.get("updated").and_then(Value::as_u64).unwrap_or(0),
        last_outcome: text("last_outcome").map(str::to_string),
        prefs: row
            .get("prefs")
            .and_then(Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .filter_map(|pair| {
                        let pair = pair.as_array()?;
                        Some((
                            pair.first()?.as_str()?.to_string(),
                            pair.get(1)?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(provider: ProviderId, id: &str) -> NativeSessionRef {
        NativeSessionRef {
            provider,
            profile: ProfileId::default_profile(),
            id: id.into(),
        }
    }

    fn summary(provider: ProviderId, id: &str, title: &str, at: u64) -> SessionSummary {
        SessionSummary {
            native: native(provider, id),
            title: Some(title.into()),
            workspace: Some("/w".into()),
            last_activity: Some(at),
            archived: false,
        }
    }

    fn registry() -> (Registry, SessionId, SessionId) {
        let mut registry = Registry::new();
        let a = registry.create(
            ProviderId::Codex,
            ProfileId::default_profile(),
            "/w".into(),
            10,
        );
        let b = registry.create(
            ProviderId::Claude,
            ProfileId::default_profile(),
            "/w".into(),
            20,
        );
        (registry, a, b)
    }

    #[test]
    fn local_ids_are_unique_and_native_identity_is_exclusive_and_namespaced() {
        let (mut registry, a, b) = registry();
        assert_ne!(a, b);
        assert!(registry.set_native(a, &native(ProviderId::Codex, "t1")));
        assert!(
            !registry.set_native(b, &native(ProviderId::Codex, "t1")),
            "wrong provider"
        );
        assert!(
            !registry.set_native(a, &native(ProviderId::Codex, "t2")),
            "never replaced"
        );
        assert!(
            registry.set_native(a, &native(ProviderId::Codex, "t1")),
            "idempotent"
        );
        assert_eq!(
            registry.find_native(&native(ProviderId::Codex, "t1")),
            Some(a)
        );
        assert_eq!(
            registry.find_native(&native(ProviderId::Claude, "t1")),
            None
        );
        let c = registry.create(
            ProviderId::Codex,
            ProfileId::default_profile(),
            "/w".into(),
            1,
        );
        assert!(
            !registry.set_native(c, &native(ProviderId::Codex, "t1")),
            "one owner per native id"
        );
    }

    #[test]
    fn the_first_message_titles_a_session_but_never_over_a_custom_title() {
        let (mut registry, a, b) = registry();
        registry.suggest_title(a, "\n  Fix the parser bug\nmore detail");
        assert_eq!(registry.get(a).unwrap().title, "Fix the parser bug");
        registry.suggest_title(a, "something else");
        assert_eq!(registry.get(a).unwrap().title, "Fix the parser bug");
        assert!(registry.rename(b, "  My chat "));
        registry.suggest_title(b, "ignored");
        assert_eq!(registry.get(b).unwrap().title, "My chat");
        assert!(!registry.rename(b, "   "));
    }

    #[test]
    fn discovery_adds_unknown_sessions_and_refreshes_known_ones_without_duplicates() {
        let (mut registry, a, _) = registry();
        registry.set_native(a, &native(ProviderId::Codex, "t1"));
        registry.rename(a, "Mine");
        let listing = [
            summary(ProviderId::Codex, "t1", "Provider title", 99),
            summary(ProviderId::Codex, "t9", "From terminal", 50),
        ];
        let w = Path::new("/w");
        assert_eq!(registry.merge_discovered(&listing, w), 1);
        assert_eq!(registry.merge_discovered(&listing, w), 0, "idempotent");
        assert_eq!(registry.records().len(), 3);
        let known = registry.get(a).unwrap();
        assert_eq!((known.title.as_str(), known.updated), ("Mine", 99));
        let found = registry
            .find_native(&native(ProviderId::Codex, "t9"))
            .unwrap();
        assert_eq!(registry.get(found).unwrap().title, "From terminal");
    }

    #[test]
    fn listing_puts_pinned_first_then_newest_and_hides_archived() {
        let (mut registry, a, b) = registry();
        let c = registry.create(
            ProviderId::Claude,
            ProfileId::default_profile(),
            "/other".into(),
            30,
        );
        registry.set_pinned(a, true);
        let ids = |rows: Vec<&SessionRecord>| rows.iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids(registry.listing(None, false)), [a, c, b]);
        registry.set_archived(c, true);
        assert_eq!(ids(registry.listing(None, false)), [a, b]);
        assert_eq!(ids(registry.listing(None, true)), [a, c, b]);
        assert_eq!(ids(registry.listing(Some(Path::new("/w")), false)), [a, b]);
    }

    #[test]
    fn the_index_round_trips_and_tolerates_bad_rows_but_refuses_newer_schemas() {
        let (mut registry, a, _) = registry();
        registry.set_native(a, &native(ProviderId::Codex, "t1"));
        registry.rename(a, "Titled");
        registry.touch(a, 77, Some("completed"));
        registry.set_prefs(
            a,
            vec![
                ("model".into(), "opus".into()),
                ("effort".into(), "high".into()),
            ],
        );
        let loaded = Registry::from_json(&registry.to_json()).unwrap();
        assert_eq!(loaded.records(), registry.records());
        let mut next = loaded;
        assert_eq!(
            next.create(
                ProviderId::Codex,
                ProfileId::default_profile(),
                "/w".into(),
                1
            )
            .0,
            3
        );

        let damaged = r#"{"schema_version":1,"sessions":[
            {"id":5,"provider":"codex","workspace":"/w","future_field":true},
            {"id":0,"provider":"codex","workspace":"/w"},
            {"id":6,"provider":"gemini","workspace":"/w"},
            {"id":5,"provider":"claude","workspace":"/w"},
            "junk"]}"#;
        let loaded = Registry::from_json(damaged).unwrap();
        assert_eq!(loaded.records().len(), 1);
        assert_eq!(loaded.skipped_rows, 4);
        assert!(Registry::from_json(r#"{"schema_version":2,"sessions":[]}"#).is_err());
        assert!(Registry::from_json("not json").is_err());
        assert!(Registry::from_json("{}").is_err());
    }

    #[test]
    fn removal_and_touch_behave() {
        let (mut registry, a, _) = registry();
        registry.touch(a, 5, None);
        assert_eq!(
            registry.get(a).unwrap().updated,
            10,
            "time never goes backwards"
        );
        assert!(registry.remove(a));
        assert!(!registry.remove(a));
    }
}
