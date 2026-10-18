//! Task editor tabs: virtual (non-document) tabs hosted beside the editor tabs,
//! like Settings. They own no document history and never dirty an editor buffer.
//! Tabs are addressed by `TaskEditorId`, never by strip position or label.
// The sidebar and palette that open these tabs arrive with the editable form.
#![allow(dead_code)]
use lumen_task_core::{Issue, Scope, TaskDef, TaskKey, TaskKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TaskEditorId(u64);

/// What a tab edits: an existing definition, or a not-yet-saved new one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TaskEditorTarget {
    New(Scope),
    Existing(TaskKey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskEditorTab {
    pub(crate) id: TaskEditorId,
    pub(crate) target: TaskEditorTarget,
    pub(crate) draft: TaskDef,
    /// The saved definition (blank for a new task); the draft is dirty when it differs.
    pub(crate) baseline: TaskDef,
    /// Editing buffers for list-like fields, folded into `draft` by `sync_texts`.
    pub(crate) args_text: String,
    pub(crate) env_text: String,
    env_errors: Vec<String>,
    /// While true the ID follows the label (new tasks only, until the ID is edited).
    pub(crate) id_auto: bool,
    /// The config file text the draft was loaded against, for the conflict check.
    pub(crate) loaded_text: Option<String>,
    /// The tasks file this tab saves to, for display.
    pub(crate) file: Option<std::path::PathBuf>,
    /// The last save/reload failure, shown in a banner above the form.
    pub(crate) error: Option<String>,
}

impl TaskEditorTab {
    fn new(
        id: TaskEditorId,
        target: TaskEditorTarget,
        draft: TaskDef,
        loaded_text: Option<String>,
    ) -> Self {
        let id_auto = matches!(target, TaskEditorTarget::New(_)) && draft.id.is_empty();
        let mut tab = Self {
            id,
            target,
            baseline: draft.clone(),
            args_text: String::new(),
            env_text: String::new(),
            env_errors: Vec::new(),
            id_auto,
            loaded_text,
            file: None,
            error: None,
            draft,
        };
        tab.rebuild_texts();
        tab
    }

    pub(crate) fn scope(&self) -> Scope {
        match &self.target {
            TaskEditorTarget::New(scope) => *scope,
            TaskEditorTarget::Existing(key) => key.scope,
        }
    }

    /// The label for the strip: the task's label, or `New task`.
    pub(crate) fn title(&self) -> String {
        match &self.target {
            TaskEditorTarget::New(_) if self.draft.label.trim().is_empty() => "New task".into(),
            _ if self.draft.label.trim().is_empty() => self.draft.id.clone(),
            _ => self.draft.label.clone(),
        }
    }

    pub(crate) fn dirty(&self) -> bool {
        self.draft != self.baseline
    }

    /// Regenerate the editing buffers from the draft (after a revert or reload).
    pub(crate) fn rebuild_texts(&mut self) {
        self.args_text = self.draft.args.join("\n");
        self.env_text = self
            .draft
            .env
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        self.env_errors.clear();
    }

    /// Fold the editing buffers into the draft. Blank argument lines are dropped;
    /// an environment line must be `NAME=VALUE`.
    pub(crate) fn sync_texts(&mut self) {
        self.draft.args = self
            .args_text
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        self.env_errors.clear();
        let mut env = std::collections::BTreeMap::new();
        for (index, line) in self.env_text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match line.split_once('=') {
                Some((name, value)) => {
                    env.insert(name.trim().to_string(), value.to_string());
                }
                None => self
                    .env_errors
                    .push(format!("line {}: expected NAME=VALUE", index + 1)),
            }
        }
        if self.env_errors.is_empty() {
            self.draft.env = env;
        }
    }

    /// Keep the ID in step with the label while it is automatic.
    pub(crate) fn follow_label(&mut self) {
        if self.id_auto {
            self.draft.id = slug(&self.draft.label);
        }
    }

    /// Everything blocking a save: core validation plus form-level parse errors.
    pub(crate) fn issues(&self) -> Vec<Issue> {
        let mut issues = lumen_task_core::validate_task(&self.draft);
        issues.extend(self.env_errors.iter().map(|message| Issue {
            task: Some(self.draft.id.clone()),
            field: "env".into(),
            message: message.clone(),
        }));
        issues
    }

    /// Replace the draft and baseline with a saved definition.
    pub(crate) fn adopt_saved(&mut self, saved: TaskDef, loaded_text: Option<String>) {
        self.baseline = saved.clone();
        self.draft = saved;
        self.loaded_text = loaded_text;
        self.error = None;
        self.rebuild_texts();
    }
}

/// Whether an argument line looks like several arguments typed on one line, such
/// as `build --release`. Process tasks pass each line as exactly one argument, so
/// `cargo` would be asked for a command literally named `build --release`. Lines
/// with path separators, quotes or `$` are left alone: spaces there are usually real.
pub(crate) fn looks_like_joined_args(line: &str) -> bool {
    let line = line.trim();
    line.contains(char::is_whitespace) && !line.contains(['/', '\\', '"', '\'', '$'])
}

/// Does any line of the argument buffer look like joined arguments?
pub(crate) fn has_joined_args(args_text: &str) -> bool {
    args_text.lines().any(looks_like_joined_args)
}

/// Put each word of every suspicious line on its own line.
pub(crate) fn split_joined_args(args_text: &str) -> String {
    args_text
        .lines()
        .map(|line| {
            if looks_like_joined_args(line) {
                line.split_whitespace().collect::<Vec<_>>().join("\n")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `Build & Test!` -> `build-test`: letters/digits joined by single dashes.
pub(crate) fn slug(label: &str) -> String {
    let mut slug = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-')
        .chars()
        .take(lumen_task_core::MAX_ID_LEN)
        .collect()
}

#[derive(Default)]
pub(crate) struct TaskEditorState {
    tabs: Vec<TaskEditorTab>,
    active: Option<TaskEditorId>,
    next_id: u64,
}

impl TaskEditorState {
    pub(crate) fn tabs(&self) -> &[TaskEditorTab] {
        &self.tabs
    }

    pub(crate) fn active(&self) -> Option<TaskEditorId> {
        self.active
    }

    pub(crate) fn active_tab(&self) -> Option<&TaskEditorTab> {
        self.get(self.active?)
    }

    pub(crate) fn get_mut(&mut self, id: TaskEditorId) -> Option<&mut TaskEditorTab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    pub(crate) fn active_tab_mut(&mut self) -> Option<&mut TaskEditorTab> {
        let id = self.active?;
        self.get_mut(id)
    }

    /// The first tab with unsaved changes, for the quit guard.
    pub(crate) fn first_dirty(&self) -> Option<TaskEditorId> {
        self.tabs.iter().find(|tab| tab.dirty()).map(|tab| tab.id)
    }

    pub(crate) fn get(&self, id: TaskEditorId) -> Option<&TaskEditorTab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    /// Open (and show) a tab. An existing definition already open is focused
    /// instead of duplicated; New always starts a fresh draft.
    pub(crate) fn open(&mut self, target: TaskEditorTarget, draft: TaskDef) -> TaskEditorId {
        self.open_with(target, draft, None)
    }

    /// `open`, remembering the file text the draft came from for conflict checks.
    pub(crate) fn open_with(
        &mut self,
        target: TaskEditorTarget,
        draft: TaskDef,
        loaded_text: Option<String>,
    ) -> TaskEditorId {
        if let TaskEditorTarget::Existing(_) = &target
            && let Some(tab) = self.tabs.iter().find(|tab| tab.target == target)
        {
            self.active = Some(tab.id);
            return tab.id;
        }
        self.next_id += 1;
        let id = TaskEditorId(self.next_id);
        self.tabs
            .push(TaskEditorTab::new(id, target, draft, loaded_text));
        self.active = Some(id);
        id
    }

    /// A blank process-task draft for `New(scope)`.
    pub(crate) fn blank_draft() -> TaskDef {
        TaskDef::new("", "", TaskKind::Process)
    }

    /// Show an open tab; false for a stale id.
    pub(crate) fn set_file(&mut self, id: TaskEditorId, file: Option<std::path::PathBuf>) {
        if let Some(tab) = self.get_mut(id) {
            tab.file = file;
        }
    }

    pub(crate) fn activate(&mut self, id: TaskEditorId) -> bool {
        let known = self.get(id).is_some();
        if known {
            self.active = Some(id);
        }
        known
    }

    /// Stop showing the task editor page without closing any tab.
    pub(crate) fn deactivate(&mut self) {
        self.active = None;
    }

    /// Close a tab. Closing the visible one hands the page back to the editor;
    /// a stale id changes nothing.
    pub(crate) fn close(&mut self, id: TaskEditorId) -> bool {
        let Some(position) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.tabs.remove(position);
        if self.active == Some(id) {
            self.active = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing(id: &str) -> TaskEditorTarget {
        TaskEditorTarget::Existing(TaskKey::new(Scope::Workspace, id))
    }

    #[test]
    fn opening_an_open_definition_focuses_it_but_new_drafts_never_merge() {
        let mut state = TaskEditorState::default();
        let a = state.open(existing("build"), TaskEditorState::blank_draft());
        let b = state.open(existing("test"), TaskEditorState::blank_draft());
        assert_eq!(state.active(), Some(b));
        assert_eq!(
            state.open(existing("build"), TaskEditorState::blank_draft()),
            a
        );
        assert_eq!(state.tabs().len(), 2);
        assert_eq!(state.active(), Some(a));
        let n1 = state.open(
            TaskEditorTarget::New(Scope::User),
            TaskEditorState::blank_draft(),
        );
        let n2 = state.open(
            TaskEditorTarget::New(Scope::User),
            TaskEditorState::blank_draft(),
        );
        assert_ne!(n1, n2);
        assert_eq!(state.tabs().len(), 4);
    }

    #[test]
    fn ids_are_never_reused_and_stale_ids_change_nothing() {
        let mut state = TaskEditorState::default();
        let a = state.open(existing("a"), TaskEditorState::blank_draft());
        assert!(state.close(a));
        assert!(!state.close(a), "already closed");
        assert!(!state.activate(a));
        let b = state.open(existing("a"), TaskEditorState::blank_draft());
        assert_ne!(a, b, "a reopened task is a new tab identity");
        assert_eq!(state.active(), Some(b));
        assert!(!state.close(a));
        assert_eq!(state.active(), Some(b));
    }

    #[test]
    fn closing_the_visible_tab_releases_the_page_and_closing_another_keeps_it() {
        let mut state = TaskEditorState::default();
        let a = state.open(existing("a"), TaskEditorState::blank_draft());
        let b = state.open(existing("b"), TaskEditorState::blank_draft());
        assert!(state.close(a));
        assert_eq!(state.active(), Some(b));
        assert!(state.close(b));
        assert_eq!(state.active(), None);
        assert!(state.tabs().is_empty());
    }

    #[test]
    fn deactivating_keeps_tabs_and_activate_restores_one() {
        let mut state = TaskEditorState::default();
        let a = state.open(existing("a"), TaskEditorState::blank_draft());
        state.deactivate();
        assert_eq!(state.active(), None);
        assert_eq!(state.tabs().len(), 1);
        assert!(state.activate(a));
        assert_eq!(state.active_tab().map(|tab| tab.id), Some(a));
    }

    #[test]
    fn titles_prefer_the_label_then_the_id_then_new_task() {
        let mut state = TaskEditorState::default();
        let id = state.open(
            TaskEditorTarget::New(Scope::User),
            TaskEditorState::blank_draft(),
        );
        assert_eq!(state.get(id).unwrap().title(), "New task");
        let mut def = TaskEditorState::blank_draft();
        def.id = "build".into();
        let id = state.open(existing("build"), def.clone());
        assert_eq!(state.get(id).unwrap().title(), "build");
        def.label = "Build".into();
        let id = state.open(existing("other"), def);
        assert_eq!(state.get(id).unwrap().title(), "Build");
    }

    #[test]
    fn sync_folds_args_and_env_and_reports_malformed_env_lines() {
        let mut state = TaskEditorState::default();
        let id = state.open(
            TaskEditorTarget::New(Scope::User),
            TaskEditorState::blank_draft(),
        );
        let tab = state.get_mut(id).unwrap();
        tab.args_text = "build\n\n--release\n".into();
        tab.env_text = "A=1\nB = x y\n\n".into();
        tab.sync_texts();
        assert_eq!(tab.draft.args, ["build", "--release"]);
        assert_eq!(tab.draft.env["A"], "1");
        assert_eq!(tab.draft.env["B"], " x y", "value text is kept verbatim");
        assert!(tab.dirty());
        tab.env_text = "A=1\nbroken".into();
        tab.sync_texts();
        assert!(
            tab.issues()
                .iter()
                .any(|i| i.field == "env" && i.message.contains("line 2"))
        );
        assert_eq!(
            tab.draft.env.len(),
            2,
            "a bad buffer never replaces the env"
        );
    }

    #[test]
    fn dirty_tracks_the_baseline_and_adopting_a_save_clears_it() {
        let mut state = TaskEditorState::default();
        let id = state.open(
            TaskEditorTarget::New(Scope::User),
            TaskEditorState::blank_draft(),
        );
        assert!(state.first_dirty().is_none());
        let tab = state.get_mut(id).unwrap();
        tab.draft.label = "Build".into();
        assert_eq!(state.first_dirty(), Some(id));
        let tab = state.get_mut(id).unwrap();
        let saved = tab.draft.clone();
        tab.adopt_saved(saved, Some("text".into()));
        assert!(!tab.dirty() && tab.loaded_text.as_deref() == Some("text"));
        tab.draft = tab.baseline.clone();
        assert!(!tab.dirty(), "reverting by hand is clean again");
    }

    #[test]
    fn the_id_follows_the_label_until_it_is_taken_over() {
        assert_eq!(slug("Build & Test!"), "build-test");
        assert_eq!(slug("  --  "), "");
        let mut state = TaskEditorState::default();
        let id = state.open(
            TaskEditorTarget::New(Scope::User),
            TaskEditorState::blank_draft(),
        );
        let tab = state.get_mut(id).unwrap();
        tab.draft.label = "Dev server".into();
        tab.follow_label();
        assert_eq!(tab.draft.id, "dev-server");
        tab.id_auto = false;
        tab.draft.label = "Other".into();
        tab.follow_label();
        assert_eq!(tab.draft.id, "dev-server");
        let id = state.open(existing("x"), TaskEditorState::blank_draft());
        assert!(
            !state.get(id).unwrap().id_auto,
            "existing ids never follow labels"
        );
    }

    #[test]
    fn joined_arguments_are_recognised_and_split_but_paths_and_quotes_are_left_alone() {
        assert!(looks_like_joined_args("build --release"));
        assert!(looks_like_joined_args("  test --workspace  "));
        assert!(!looks_like_joined_args("build"));
        assert!(!looks_like_joined_args("--release"));
        assert!(!looks_like_joined_args("/Users/me/My Docs/file.txt"));
        assert!(!looks_like_joined_args("C:\\Program Files\\x"));
        assert!(!looks_like_joined_args("say \"hello world\""));
        assert!(!looks_like_joined_args("price is $5 today"));
        assert!(has_joined_args("clippy\nbuild --release"));
        assert!(!has_joined_args("clippy\n--all-targets"));
        assert_eq!(
            split_joined_args("build --release\n--target x86\n/My Docs/a b"),
            "build\n--release\n--target\nx86\n/My Docs/a b"
        );
        assert_eq!(split_joined_args(""), "");
    }
}
