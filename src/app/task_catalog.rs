//! What the Run and Debug sidebar lists: both scopes' task definitions as last
//! read from disk, plus the local filter and selection. Pure state; the controller
//! in `app/tasks.rs` fills it and the UI only reads it.
use lumen_task_core::{Issue, Scope, TaskDef, TaskKey};
use std::path::PathBuf;

/// The result of reading one scope's tasks file.
#[derive(Clone, Debug, Default)]
pub(crate) enum ScopeState {
    /// Not read yet.
    #[default]
    Unread,
    /// The scope cannot be used (no folder open, no config directory).
    Unavailable(String),
    /// The file does not exist; there is simply nothing defined.
    Missing,
    Loaded {
        tasks: Vec<TaskDef>,
        text: String,
    },
    /// Parsed but invalid; the file is never rewritten blindly.
    Invalid {
        issues: Vec<Issue>,
    },
    Unreadable(String),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScopeTasks {
    pub(crate) path: Option<PathBuf>,
    pub(crate) state: ScopeState,
}

pub(crate) struct TaskCatalog {
    pub(crate) user: ScopeTasks,
    pub(crate) workspace: ScopeTasks,
    /// Re-read on the next frame the sidebar is visible.
    pub(crate) stale: bool,
    /// The workspace the workspace scope was read for.
    pub(crate) loaded_root: Option<PathBuf>,
    pub(crate) filter: String,
    pub(crate) selected: Option<TaskKey>,
}

impl Default for TaskCatalog {
    fn default() -> Self {
        Self {
            user: ScopeTasks::default(),
            workspace: ScopeTasks::default(),
            stale: true,
            loaded_root: None,
            filter: String::new(),
            selected: None,
        }
    }
}

impl TaskCatalog {
    pub(crate) fn scope(&self, scope: Scope) -> &ScopeTasks {
        match scope {
            Scope::User => &self.user,
            Scope::Workspace => &self.workspace,
        }
    }

    pub(crate) fn scope_mut(&mut self, scope: Scope) -> &mut ScopeTasks {
        match scope {
            Scope::User => &mut self.user,
            Scope::Workspace => &mut self.workspace,
        }
    }

    pub(crate) fn tasks(&self, scope: Scope) -> &[TaskDef] {
        match &self.scope(scope).state {
            ScopeState::Loaded { tasks, .. } => tasks,
            _ => &[],
        }
    }

    pub(crate) fn text(&self, scope: Scope) -> Option<&str> {
        match &self.scope(scope).state {
            ScopeState::Loaded { text, .. } => Some(text),
            _ => None,
        }
    }

    pub(crate) fn get(&self, key: &TaskKey) -> Option<&TaskDef> {
        self.tasks(key.scope).iter().find(|task| task.id == key.id)
    }

    /// Tasks matching the filter (label, ID or group, case-insensitive; every
    /// whitespace-separated term must match).
    pub(crate) fn visible(&self, scope: Scope) -> Vec<&TaskDef> {
        let terms: Vec<String> = self
            .filter
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.tasks(scope)
            .iter()
            .filter(|task| {
                let haystack = format!(
                    "{} {} {}",
                    task.label,
                    task.id,
                    task.group.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                terms.iter().all(|term| haystack.contains(term))
            })
            .collect()
    }

    /// The default task of `group`: the workspace default wins over the user's.
    pub(crate) fn default_for_group(&self, group: &str) -> Option<TaskKey> {
        [Scope::Workspace, Scope::User]
            .into_iter()
            .find_map(|scope| {
                self.tasks(scope)
                    .iter()
                    .find(|task| task.is_default && task.group.as_deref() == Some(group))
                    .map(|task| TaskKey::new(scope, &task.id))
            })
    }

    /// An unused ID for a copy of `id`: `build-copy`, `build-copy-2`, …
    pub(crate) fn unique_copy_id(&self, scope: Scope, id: &str) -> String {
        let taken = |candidate: &str| self.tasks(scope).iter().any(|task| task.id == candidate);
        let base: String = id
            .chars()
            .take(lumen_task_core::MAX_ID_LEN - "-copy-999".len())
            .collect();
        let first = format!("{base}-copy");
        if !taken(&first) {
            return first;
        }
        (2..)
            .map(|n| format!("{first}-{n}"))
            .find(|candidate| !taken(candidate))
            .expect("an unused suffix exists")
    }

    /// Tasks whose `depends_on` would dangle if `key` were deleted: same-scope
    /// references, and `user:` references from workspace tasks to a user task.
    pub(crate) fn dependents(&self, key: &TaskKey) -> Vec<TaskKey> {
        let mut found = Vec::new();
        for scope in [Scope::User, Scope::Workspace] {
            for task in self.tasks(scope) {
                let references = task.depends_on.iter().any(|dependency| {
                    match dependency.strip_prefix("user:") {
                        Some(id) => key.scope == Scope::User && key.id == id,
                        None => scope == key.scope && key.id == *dependency,
                    }
                });
                if references && !(scope == key.scope && task.id == key.id) {
                    found.push(TaskKey::new(scope, &task.id));
                }
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_task_core::TaskKind;

    fn task(id: &str, label: &str, group: Option<&str>, deps: &[&str]) -> TaskDef {
        let mut task = TaskDef::new(id, label, TaskKind::Process);
        task.command = Some("true".into());
        task.group = group.map(str::to_string);
        task.depends_on = deps.iter().map(|d| d.to_string()).collect();
        task
    }

    fn catalog() -> TaskCatalog {
        let mut catalog = TaskCatalog::default();
        catalog.workspace.state = ScopeState::Loaded {
            tasks: vec![
                task("build", "Build all", Some("build"), &[]),
                task("test", "Run tests", Some("test"), &["build"]),
                task("ci", "CI", None, &["test", "user:lint"]),
            ],
            text: String::new(),
        };
        catalog.user.state = ScopeState::Loaded {
            tasks: vec![
                task("lint", "Lint", None, &[]),
                task("build", "Mine", None, &[]),
            ],
            text: String::new(),
        };
        catalog
    }

    #[test]
    fn filter_matches_label_id_and_group_with_every_term() {
        let mut catalog = catalog();
        assert_eq!(catalog.visible(Scope::Workspace).len(), 3);
        catalog.filter = "TEST".into();
        let ids: Vec<_> = catalog
            .visible(Scope::Workspace)
            .iter()
            .map(|t| t.id.clone())
            .collect();
        assert_eq!(ids, ["test"]);
        catalog.filter = "build all".into();
        assert_eq!(catalog.visible(Scope::Workspace).len(), 1);
        catalog.filter = "build tests".into();
        assert!(
            catalog.visible(Scope::Workspace).is_empty(),
            "all terms must match"
        );
        catalog.filter = "  ".into();
        assert_eq!(catalog.visible(Scope::User).len(), 2);
    }

    #[test]
    fn copies_get_the_first_unused_suffix_per_scope() {
        let mut catalog = catalog();
        assert_eq!(
            catalog.unique_copy_id(Scope::Workspace, "build"),
            "build-copy"
        );
        if let ScopeState::Loaded { tasks, .. } = &mut catalog.workspace.state {
            tasks.push(task("build-copy", "x", None, &[]));
            tasks.push(task("build-copy-2", "x", None, &[]));
        }
        assert_eq!(
            catalog.unique_copy_id(Scope::Workspace, "build"),
            "build-copy-3"
        );
        assert_eq!(catalog.unique_copy_id(Scope::User, "build"), "build-copy");
        let long = "x".repeat(64);
        assert!(lumen_task_core::MAX_ID_LEN >= catalog.unique_copy_id(Scope::User, &long).len());
    }

    #[test]
    fn dependents_follow_scope_rules_for_plain_and_user_references() {
        let catalog = catalog();
        let by = |scope, id: &str| -> Vec<String> {
            catalog
                .dependents(&TaskKey::new(scope, id))
                .into_iter()
                .map(|key| format!("{:?}:{}", key.scope, key.id))
                .collect()
        };
        assert_eq!(by(Scope::Workspace, "build"), ["Workspace:test"]);
        assert_eq!(by(Scope::Workspace, "test"), ["Workspace:ci"]);
        assert_eq!(by(Scope::User, "lint"), ["Workspace:ci"]);
        assert!(
            by(Scope::User, "build").is_empty(),
            "the workspace 'build' is a different task"
        );
        assert!(by(Scope::Workspace, "ci").is_empty());
    }

    #[test]
    fn unread_and_failed_scopes_list_nothing() {
        let mut catalog = TaskCatalog::default();
        assert!(catalog.tasks(Scope::User).is_empty() && catalog.text(Scope::User).is_none());
        catalog.user.state = ScopeState::Invalid { issues: vec![] };
        assert!(catalog.visible(Scope::User).is_empty());
        assert!(catalog.stale, "a fresh catalog needs its first read");
    }
}
