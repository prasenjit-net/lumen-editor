//! Task configuration workflows: opening, saving, reverting and closing task
//! editor tabs. Filesystem access goes through the injected settings store; the
//! UI only issues typed commands.
use super::task_catalog::{ScopeState, ScopeTasks};
use super::{
    LumenApp,
    task_editor::{TaskEditorId, TaskEditorState, TaskEditorTarget},
};
use crate::services::tasks::{self, SaveMode};
use lumen_task_core::{Scope, TaskKey};
use std::path::PathBuf;

impl LumenApp {
    /// The tasks file for `scope` and the workspace root guarding it (workspace only).
    pub(crate) fn task_config_path(
        &self,
        scope: Scope,
    ) -> Result<(PathBuf, Option<PathBuf>), String> {
        match scope {
            Scope::User => self
                .task_user_path
                .clone()
                .map(|path| (path, None))
                .ok_or_else(|| "The user config directory is unavailable".to_string()),
            Scope::Workspace => self
                .editor
                .workspace
                .clone()
                .map(|root| (tasks::workspace_path(&root), Some(root)))
                .ok_or_else(|| "Open a folder to use workspace tasks".to_string()),
        }
    }

    /// Keep the sidebar's task list fresh while it is visible: re-read on first
    /// show, after a change, and when the workspace changes. Called every frame.
    pub(crate) fn ensure_tasks(&mut self) {
        use super::view_state::Activity;
        if self.view.activity != Activity::RunDebug || !self.view.sidebar_visible {
            // Files can change while hidden; re-read when it is shown again.
            self.view.tasks.stale = true;
            return;
        }
        if self.view.tasks.stale || self.view.tasks.loaded_root != self.editor.workspace {
            self.refresh_tasks();
        }
    }

    pub(crate) fn refresh_tasks(&mut self) {
        for scope in [Scope::User, Scope::Workspace] {
            let loaded = self.read_scope(scope);
            *self.view.tasks.scope_mut(scope) = loaded;
        }
        self.view.tasks.loaded_root = self.editor.workspace.clone();
        self.view.tasks.stale = false;
        if let Some(key) = self.view.tasks.selected.clone()
            && self.view.tasks.get(&key).is_none()
        {
            self.view.tasks.selected = None;
        }
    }

    fn read_scope(&self, scope: Scope) -> ScopeTasks {
        let (path, _) = match self.task_config_path(scope) {
            Ok(found) => found,
            Err(message) => {
                return ScopeTasks {
                    path: None,
                    state: ScopeState::Unavailable(message),
                };
            }
        };
        let state = match tasks::load(self.settings_store.as_ref(), &path) {
            Err(message) => ScopeState::Unreadable(message),
            Ok(loaded) => match (loaded.text, loaded.parsed) {
                (None, _) => ScopeState::Missing,
                (Some(text), Ok(file)) => ScopeState::Loaded {
                    tasks: file.tasks,
                    text,
                },
                (Some(_), Err(issues)) => ScopeState::Invalid { issues },
            },
        };
        ScopeTasks {
            path: Some(path),
            state,
        }
    }

    /// Open a copy of a task as an unsaved new draft with a fresh ID.
    pub(crate) fn duplicate_task(&mut self, key: TaskKey) {
        let Some(original) = self.view.tasks.get(&key).cloned() else {
            self.view.status = "That task no longer exists; refresh the list".into();
            return;
        };
        let mut draft = original;
        draft.id = self.view.tasks.unique_copy_id(key.scope, &key.id);
        draft.label = format!("{} copy", draft.label);
        draft.is_default = false;
        let id = self
            .view
            .task_editors
            .open(TaskEditorTarget::New(key.scope), draft);
        if let Some(tab) = self.view.task_editors.get_mut(id) {
            // Unsaved by definition: the baseline is "nothing".
            tab.baseline = TaskEditorState::blank_draft();
        }
        let file = self.task_config_path(key.scope).ok().map(|(path, _)| path);
        self.view.task_editors.set_file(id, file);
    }

    pub(crate) fn request_delete_task(&mut self, key: TaskKey) {
        if self.view.tasks.get(&key).is_some() {
            self.view.task_delete_prompt = Some(key);
        }
    }

    /// Delete one definition (never the files it produced). Refused while the task
    /// has an open editor tab, so unsaved edits are not stranded.
    pub(crate) fn delete_task_definition(&mut self, key: TaskKey) {
        self.view.task_delete_prompt = None;
        let target = TaskEditorTarget::Existing(key.clone());
        if self
            .view
            .task_editors
            .tabs()
            .iter()
            .any(|tab| tab.target == target)
        {
            self.view.status = format!("Close the editor tab for '{}' before deleting it", key.id);
            return;
        }
        let result = (|| -> Result<(), String> {
            let (path, root) = self.task_config_path(key.scope)?;
            let text = self
                .view
                .tasks
                .text(key.scope)
                .ok_or("The task list is out of date; refresh it first")?;
            tasks::delete_task(
                self.settings_store.as_ref(),
                &path,
                root.as_deref(),
                text,
                &key.id,
            )
            .map(|_| ())
        })();
        self.view.tasks.stale = true;
        match result {
            Ok(()) => {
                if self.view.tasks.selected.as_ref() == Some(&key) {
                    self.view.tasks.selected = None;
                }
                self.view.status = format!("Deleted task '{}'", key.id);
            }
            Err(message) => self.view.status = format!("Could not delete task: {message}"),
        }
    }

    /// Open a blank draft. Workspace drafts need an open folder.
    pub(crate) fn new_task_editor(&mut self, scope: Scope) {
        if let Err(message) = self.task_config_path(scope) {
            self.view.status = message;
            return;
        }
        let id = self
            .view
            .task_editors
            .open(TaskEditorTarget::New(scope), TaskEditorState::blank_draft());
        let file = self.task_config_path(scope).ok().map(|(path, _)| path);
        self.view.task_editors.set_file(id, file);
    }

    /// Open an existing definition for editing, focusing its tab if already open.
    pub(crate) fn open_task_editor(&mut self, key: TaskKey) -> bool {
        let target = TaskEditorTarget::Existing(key.clone());
        if let Some(tab) = self
            .view
            .task_editors
            .tabs()
            .iter()
            .find(|tab| tab.target == target)
        {
            let id = tab.id;
            return self.view.task_editors.activate(id);
        }
        let result = self.read_task(&key);
        match result {
            Ok((def, text)) => {
                let scope = key.scope;
                let id = self.view.task_editors.open_with(target, def, Some(text));
                let file = self.task_config_path(scope).ok().map(|(path, _)| path);
                self.view.task_editors.set_file(id, file);
                true
            }
            Err(message) => {
                self.view.status = message;
                false
            }
        }
    }

    pub(crate) fn read_task(
        &self,
        key: &TaskKey,
    ) -> Result<(lumen_task_core::TaskDef, String), String> {
        let (path, _) = self.task_config_path(key.scope)?;
        let loaded = tasks::load(self.settings_store.as_ref(), &path)?;
        let text = loaded
            .text
            .ok_or_else(|| format!("{} does not exist", path.display()))?;
        let file = loaded.parsed.map_err(|issues| {
            format!(
                "{} has problems; open it as TOML to fix them ({})",
                path.display(),
                issues.first().map(ToString::to_string).unwrap_or_default()
            )
        })?;
        let def = file
            .get(&key.id)
            .cloned()
            .ok_or_else(|| format!("Task '{}' no longer exists", key.id))?;
        Ok((def, text))
    }

    /// Validate and write the draft. Returns whether the file now holds it; every
    /// failure leaves the draft untouched and explains itself in the tab.
    pub(crate) fn save_task_editor(&mut self, id: TaskEditorId) -> bool {
        let Some(tab) = self.view.task_editors.get_mut(id) else {
            return false;
        };
        tab.sync_texts();
        let issues = tab.issues();
        if !issues.is_empty() {
            tab.error = Some("Fix the problems below before saving.".into());
            return false;
        }
        let (scope, draft, target, loaded) = (
            tab.scope(),
            tab.draft.clone(),
            tab.target.clone(),
            tab.loaded_text.clone(),
        );
        let outcome = self.write_task(scope, &draft, &target, loaded.as_deref());
        let Some(tab) = self.view.task_editors.get_mut(id) else {
            return false;
        };
        match outcome {
            Ok(text) => {
                let key = TaskKey::new(scope, draft.id.clone());
                tab.target = TaskEditorTarget::Existing(key);
                tab.id_auto = false;
                tab.adopt_saved(draft.clone(), Some(text));
                self.view.tasks.stale = true;
                self.view.status = format!("Saved task '{}'", draft.label);
                true
            }
            Err(message) => {
                tab.error = Some(message);
                false
            }
        }
    }

    fn write_task(
        &self,
        scope: Scope,
        draft: &lumen_task_core::TaskDef,
        target: &TaskEditorTarget,
        loaded: Option<&str>,
    ) -> Result<String, String> {
        let (path, root) = self.task_config_path(scope)?;
        if self
            .editor
            .files
            .iter()
            .any(|file| file.dirty && file.path.as_deref() == Some(path.as_path()))
        {
            return Err(format!(
                "{} has unsaved changes in an editor tab. Save or revert that file first.",
                path.display()
            ));
        }
        let store = self.settings_store.as_ref();
        let current = store.read(&path)?;
        let (mode, expected) = match target {
            // A new task appends to whatever the file holds now; other tasks'
            // edits cannot conflict with it.
            TaskEditorTarget::New(_) => (SaveMode::Create, current),
            // An update must still match what the draft was loaded from.
            TaskEditorTarget::Existing(_) => {
                if current.as_deref() != loaded {
                    return Err(
                        "The tasks file changed on disk. Reload the task (discarding your edits) or copy your changes first."
                            .into(),
                    );
                }
                (SaveMode::Update, loaded.map(str::to_string))
            }
        };
        tasks::save_task(
            store,
            &path,
            root.as_deref(),
            expected.as_deref(),
            draft,
            mode,
        )
    }

    /// Discard unsaved edits, returning to the saved definition.
    pub(crate) fn revert_task_editor(&mut self, id: TaskEditorId) {
        if let Some(tab) = self.view.task_editors.get_mut(id) {
            tab.draft = tab.baseline.clone();
            tab.error = None;
            tab.rebuild_texts();
            if matches!(tab.target, TaskEditorTarget::New(_)) {
                tab.id_auto = tab.draft.id.is_empty();
            }
        }
    }

    /// Re-read an existing definition from disk, replacing the draft. Only offered
    /// after a conflict, so the user has already chosen to drop their edits.
    pub(crate) fn reload_task_editor(&mut self, id: TaskEditorId) {
        let Some(tab) = self.view.task_editors.get(id) else {
            return;
        };
        let TaskEditorTarget::Existing(key) = tab.target.clone() else {
            return;
        };
        let result = self.read_task(&key);
        if let Some(tab) = self.view.task_editors.get_mut(id) {
            match result {
                Ok((def, text)) => tab.adopt_saved(def, Some(text)),
                Err(message) => tab.error = Some(message),
            }
        }
    }

    /// Close now when clean; otherwise ask Save / Don't Save / Cancel.
    pub(crate) fn request_close_task_editor(&mut self, id: TaskEditorId) {
        let Some(tab) = self.view.task_editors.get_mut(id) else {
            return;
        };
        tab.sync_texts();
        if tab.dirty() {
            self.view.task_editors.activate(id);
            self.view.task_close_prompt = Some(id);
        } else {
            self.view.task_editors.close(id);
        }
    }

    pub(crate) fn save_and_close_task_editor(&mut self, id: TaskEditorId) {
        if self.save_task_editor(id) {
            self.view.task_close_prompt = None;
            self.view.task_editors.close(id);
        }
        // On failure the prompt closes so the tab's error banner is visible.
        self.view.task_close_prompt = None;
    }

    pub(crate) fn discard_task_editor(&mut self, id: TaskEditorId) {
        self.view.task_close_prompt = None;
        self.view.task_editors.close(id);
    }

    /// Open the raw tasks file as an ordinary document.
    pub(crate) fn open_task_toml(&mut self, scope: Scope) {
        match self.task_config_path(scope) {
            Ok((path, _)) if path.is_file() => self.open_file(path),
            Ok((path, _)) => {
                self.view.status = format!(
                    "{} does not exist yet; save a task to create it",
                    path.display()
                )
            }
            Err(message) => self.view.status = message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::Command;
    use eframe::egui;

    fn app() -> (LumenApp, tempfile::TempDir, egui::Context) {
        let root = tempfile::tempdir().unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.path().to_path_buf());
        app.task_user_path = Some(root.path().join("home/.lumen/tasks.toml"));
        (app, root, egui::Context::default())
    }

    fn fill(app: &mut LumenApp, id: TaskEditorId, label: &str, command: &str) {
        let tab = app.view.task_editors.get_mut(id).unwrap();
        tab.draft.label = label.into();
        tab.follow_label();
        tab.draft.command = Some(command.into());
    }

    fn open_new(app: &mut LumenApp, ctx: &egui::Context, scope: Scope) -> TaskEditorId {
        app.execute(Command::NewTaskEditor(scope), ctx);
        app.view.task_editors.active().expect("a tab opened")
    }

    fn workspace_file(root: &tempfile::TempDir) -> PathBuf {
        tasks::workspace_path(root.path())
    }

    #[test]
    fn a_new_workspace_task_is_saved_with_a_label_derived_id_and_becomes_an_existing_one() {
        let (mut app, root, ctx) = app();
        let id = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, id, "Build all", "cargo");
        app.view.task_editors.get_mut(id).unwrap().args_text = "build\n--release".into();
        assert!(app.view.task_editors.get(id).unwrap().dirty());
        assert!(
            !workspace_file(&root).exists(),
            "nothing is written before Save"
        );
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let text = std::fs::read_to_string(workspace_file(&root)).unwrap();
        assert!(
            text.contains("id = \"build-all\"") && text.contains("--release"),
            "{text}"
        );
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(!tab.dirty() && tab.error.is_none());
        assert_eq!(
            tab.target,
            TaskEditorTarget::Existing(TaskKey::new(Scope::Workspace, "build-all"))
        );
        assert_eq!(app.view.status, "Saved task 'Build all'");
        // A second save with a change updates the same task rather than duplicating it.
        app.view.task_editors.get_mut(id).unwrap().draft.detail = Some("All crates".into());
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let parsed =
            lumen_task_core::parse(&std::fs::read_to_string(workspace_file(&root)).unwrap())
                .unwrap();
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(parsed.tasks[0].detail.as_deref(), Some("All crates"));
    }

    #[test]
    fn invalid_drafts_and_duplicate_ids_write_nothing_and_say_why() {
        let (mut app, root, ctx) = app();
        let id = open_new(&mut app, &ctx, Scope::Workspace);
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(tab.error.as_deref().unwrap().contains("Fix the problems"));
        assert!(!workspace_file(&root).exists());

        fill(&mut app, id, "Build", "cargo");
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let other = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, other, "Build", "make");
        app.execute(Command::SaveTaskEditor(other), &ctx);
        let tab = app.view.task_editors.get(other).unwrap();
        assert!(
            tab.error.as_deref().unwrap().contains("already exists"),
            "{:?}",
            tab.error
        );
        assert!(tab.dirty(), "the draft survives the failed save");
        let text = std::fs::read_to_string(workspace_file(&root)).unwrap();
        assert!(text.contains("cargo") && !text.contains("make"));

        let tab = app.view.task_editors.get_mut(other).unwrap();
        tab.env_text = "not-an-assignment".into();
        app.execute(Command::SaveTaskEditor(other), &ctx);
        let tab = app.view.task_editors.get(other).unwrap();
        assert!(tab.issues().iter().any(|i| i.field == "env"));
    }

    #[test]
    fn workspace_tasks_need_a_folder_and_user_tasks_need_a_config_dir() {
        let (mut app, _root, ctx) = app();
        app.editor.workspace = None;
        app.execute(Command::NewTaskEditor(Scope::Workspace), &ctx);
        assert!(app.view.task_editors.tabs().is_empty());
        assert!(app.view.status.contains("Open a folder"));
        app.task_user_path = None;
        app.execute(Command::NewTaskEditor(Scope::User), &ctx);
        assert!(app.view.task_editors.tabs().is_empty());
        assert!(app.view.status.contains("unavailable"));
    }

    #[test]
    fn user_tasks_save_to_the_user_file_not_the_workspace() {
        let (mut app, root, ctx) = app();
        let id = open_new(&mut app, &ctx, Scope::User);
        fill(&mut app, id, "Lint", "cargo");
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let path = root.path().join("home/.lumen/tasks.toml");
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("id = \"lint\"")
        );
        assert!(!workspace_file(&root).exists());
    }

    fn seed(root: &tempfile::TempDir) -> TaskKey {
        let path = workspace_file(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "schema_version = 1\n\n# keep me\n[[tasks]]\nid = \"build\"\nlabel = \"Build\"\ntype = \"process\"\ncommand = \"cargo\"\nargs = [\"build\"]\n",
        )
        .unwrap();
        TaskKey::new(Scope::Workspace, "build")
    }

    #[test]
    fn editing_an_existing_task_keeps_comments_and_detects_external_changes() {
        let (mut app, root, ctx) = app();
        let key = seed(&root);
        app.execute(Command::OpenTaskEditor(key.clone()), &ctx);
        let id = app.view.task_editors.active().unwrap();
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(!tab.dirty() && tab.draft.command.as_deref() == Some("cargo"));
        assert_eq!(tab.args_text, "build");
        app.execute(Command::OpenTaskEditor(key.clone()), &ctx);
        assert_eq!(
            app.view.task_editors.tabs().len(),
            1,
            "focused, not duplicated"
        );

        app.view.task_editors.get_mut(id).unwrap().args_text = "build\n--release".into();
        app.execute(Command::Save, &ctx);
        let text = std::fs::read_to_string(workspace_file(&root)).unwrap();
        assert!(
            text.contains("# keep me") && text.contains("--release"),
            "{text}"
        );

        // Someone else edits the file; our next save must not overwrite it.
        std::fs::write(workspace_file(&root), format!("{text}# external\n")).unwrap();
        app.view.task_editors.get_mut(id).unwrap().draft.detail = Some("x".into());
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(tab.error.as_deref().unwrap().contains("changed on disk"));
        assert!(
            std::fs::read_to_string(workspace_file(&root))
                .unwrap()
                .contains("# external")
        );
        assert!(
            !std::fs::read_to_string(workspace_file(&root))
                .unwrap()
                .contains("detail")
        );
        // Reload drops the edits and adopts the file's version.
        app.execute(Command::ReloadTaskEditor(id), &ctx);
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(tab.error.is_none() && !tab.dirty() && tab.draft.detail.is_none());
    }

    #[test]
    fn opening_a_missing_or_broken_definition_reports_and_opens_nothing() {
        let (mut app, root, ctx) = app();
        app.execute(
            Command::OpenTaskEditor(TaskKey::new(Scope::Workspace, "x")),
            &ctx,
        );
        assert!(app.view.task_editors.tabs().is_empty());
        assert!(app.view.status.contains("does not exist"));
        let key = seed(&root);
        app.execute(
            Command::OpenTaskEditor(TaskKey::new(Scope::Workspace, "ghost")),
            &ctx,
        );
        assert!(app.view.status.contains("no longer exists"));
        std::fs::write(workspace_file(&root), "not [ toml").unwrap();
        app.execute(Command::OpenTaskEditor(key), &ctx);
        assert!(app.view.status.contains("has problems"));
        assert!(app.view.task_editors.tabs().is_empty());
    }

    #[test]
    fn a_dirty_open_tasks_file_blocks_the_form_from_writing_over_it() {
        let (mut app, root, ctx) = app();
        let key = seed(&root);
        app.open_file(workspace_file(&root));
        app.editor.files[0].dirty = true;
        app.execute(Command::OpenTaskEditor(key), &ctx);
        let id = app.view.task_editors.active().unwrap();
        app.view.task_editors.get_mut(id).unwrap().draft.detail = Some("x".into());
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(
            tab.error
                .as_deref()
                .unwrap()
                .contains("unsaved changes in an editor tab")
        );
        assert!(
            !std::fs::read_to_string(workspace_file(&root))
                .unwrap()
                .contains("detail")
        );
    }

    #[test]
    fn closing_a_dirty_task_asks_and_each_answer_does_what_it_says() {
        let (mut app, root, ctx) = app();
        let clean = open_new(&mut app, &ctx, Scope::Workspace);
        app.execute(Command::CloseTaskEditor(clean), &ctx);
        assert!(
            app.view.task_editors.tabs().is_empty(),
            "clean tabs close at once"
        );

        let id = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, id, "Build", "cargo");
        app.execute(Command::CloseTaskEditor(id), &ctx);
        assert_eq!(app.view.task_close_prompt, Some(id));
        assert_eq!(app.view.task_editors.tabs().len(), 1);
        app.execute(Command::CancelTaskEditorClose, &ctx);
        assert!(app.view.task_close_prompt.is_none() && app.view.task_editors.tabs().len() == 1);

        app.execute(Command::CloseEditor, &ctx);
        assert_eq!(
            app.view.task_close_prompt,
            Some(id),
            "Close Editor asks too"
        );
        app.execute(Command::DiscardTaskEditor(id), &ctx);
        assert!(app.view.task_editors.tabs().is_empty());
        assert!(!workspace_file(&root).exists(), "Don't Save writes nothing");

        let id = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, id, "Build", "cargo");
        app.execute(Command::CloseTaskEditor(id), &ctx);
        app.execute(Command::SaveAndCloseTaskEditor(id), &ctx);
        assert!(app.view.task_editors.tabs().is_empty());
        assert!(workspace_file(&root).exists());

        // A save that fails keeps the tab (and the work) with its error visible.
        let id = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, id, "Build", "make");
        app.execute(Command::CloseTaskEditor(id), &ctx);
        app.execute(Command::SaveAndCloseTaskEditor(id), &ctx);
        assert_eq!(app.view.task_editors.tabs().len(), 1);
        assert!(app.view.task_close_prompt.is_none());
        assert!(app.view.task_editors.get(id).unwrap().error.is_some());
    }

    #[test]
    fn quitting_with_an_unsaved_task_asks_instead_of_exiting() {
        let (mut app, _root, ctx) = app();
        let id = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, id, "Build", "cargo");
        app.execute(Command::Settings, &ctx);
        app.request_quit(&ctx);
        assert_eq!(app.view.task_close_prompt, Some(id));
        assert!(!app.view.allow_quit);
        assert!(app.view.status.contains("quit again"));
        assert_eq!(
            app.view.task_editors.active(),
            Some(id),
            "the draft is shown"
        );
    }

    #[test]
    fn revert_restores_the_saved_definition_and_clears_errors() {
        let (mut app, root, ctx) = app();
        let key = seed(&root);
        app.execute(Command::OpenTaskEditor(key), &ctx);
        let id = app.view.task_editors.active().unwrap();
        let tab = app.view.task_editors.get_mut(id).unwrap();
        tab.draft.command = Some("make".into());
        tab.args_text = "other".into();
        tab.error = Some("old".into());
        app.execute(Command::RevertTaskEditor(id), &ctx);
        let tab = app.view.task_editors.get(id).unwrap();
        assert!(!tab.dirty() && tab.error.is_none());
        assert_eq!(
            (tab.draft.command.as_deref(), tab.args_text.as_str()),
            (Some("cargo"), "build")
        );
    }

    #[test]
    fn open_in_toml_opens_the_file_as_a_document_or_explains_it_is_missing() {
        let (mut app, root, ctx) = app();
        app.execute(Command::OpenTaskToml(Scope::Workspace), &ctx);
        assert!(app.editor.files.is_empty() && app.view.status.contains("does not exist yet"));
        seed(&root);
        let id = open_new(&mut app, &ctx, Scope::Workspace);
        app.execute(Command::OpenTaskToml(Scope::Workspace), &ctx);
        assert_eq!(app.editor.files.len(), 1);
        assert_eq!(
            app.editor.active().unwrap().path.as_deref(),
            Some(workspace_file(&root).as_path())
        );
        assert!(
            app.view.task_editors.active().is_none(),
            "the document is shown"
        );
        assert!(
            app.view.task_editors.get(id).is_some(),
            "the draft tab stays open"
        );
    }

    fn show_sidebar(app: &mut LumenApp) {
        app.view.activity = crate::app::view_state::Activity::RunDebug;
        app.view.sidebar_visible = true;
        app.ensure_tasks();
    }

    #[test]
    fn the_catalog_reads_each_scope_only_while_the_sidebar_is_visible() {
        let (mut app, root, ctx) = app();
        seed(&root);
        app.ensure_tasks();
        assert!(
            app.view.tasks.stale,
            "nothing is read while another activity shows"
        );
        assert!(matches!(app.view.tasks.workspace.state, ScopeState::Unread));
        show_sidebar(&mut app);
        assert!(!app.view.tasks.stale);
        assert_eq!(app.view.tasks.tasks(Scope::Workspace).len(), 1);
        assert!(matches!(app.view.tasks.user.state, ScopeState::Missing));
        assert_eq!(app.view.tasks.loaded_root, app.editor.workspace);

        // Hiding and showing again re-reads, picking up outside edits.
        app.view.sidebar_visible = false;
        app.ensure_tasks();
        let path = workspace_file(&root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n[[tasks]]\nid = \"t\"\nlabel = \"T\"\ntype = \"process\"\ncommand = \"x\"\n")).unwrap();
        show_sidebar(&mut app);
        assert_eq!(app.view.tasks.tasks(Scope::Workspace).len(), 2);

        // A different workspace forces a re-read without being hidden.
        let other = tempfile::tempdir().unwrap();
        app.editor.workspace = Some(other.path().to_path_buf());
        app.ensure_tasks();
        assert!(matches!(
            app.view.tasks.workspace.state,
            ScopeState::Missing
        ));
        app.editor.workspace = None;
        app.ensure_tasks();
        assert!(matches!(
            app.view.tasks.workspace.state,
            ScopeState::Unavailable(_)
        ));
        let _ = ctx;
    }

    #[test]
    fn invalid_and_unreadable_files_are_reported_not_listed_or_rewritten() {
        let (mut app, root, _ctx) = app();
        let path = workspace_file(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "schema_version = 1\n[[tasks]]\nid = \"a\"\nlabel = \"A\"\ntype = \"process\"\n",
        )
        .unwrap();
        show_sidebar(&mut app);
        assert!(
            matches!(&app.view.tasks.workspace.state, ScopeState::Invalid { issues } if !issues.is_empty())
        );
        assert!(app.view.tasks.tasks(Scope::Workspace).is_empty());
        std::fs::write(&path, "not [ toml").unwrap();
        app.execute(Command::RefreshTasks, &egui::Context::default());
        assert!(matches!(
            app.view.tasks.workspace.state,
            ScopeState::Invalid { .. }
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not [ toml");
    }

    #[test]
    fn saving_a_task_refreshes_the_list_and_selection_survives_only_while_it_exists() {
        let (mut app, root, ctx) = app();
        show_sidebar(&mut app);
        let id = open_new(&mut app, &ctx, Scope::Workspace);
        fill(&mut app, id, "Build", "cargo");
        app.execute(Command::SaveTaskEditor(id), &ctx);
        assert!(app.view.tasks.stale);
        show_sidebar(&mut app);
        assert_eq!(app.view.tasks.tasks(Scope::Workspace).len(), 1);
        let key = TaskKey::new(Scope::Workspace, "build");
        app.execute(Command::SelectTask(key.clone()), &ctx);
        assert_eq!(app.view.tasks.selected, Some(key));
        std::fs::remove_file(workspace_file(&root)).unwrap();
        app.execute(Command::RefreshTasks, &ctx);
        assert_eq!(app.view.tasks.selected, None);
    }

    #[test]
    fn duplicate_opens_an_unsaved_copy_with_a_fresh_id_and_saving_keeps_the_original() {
        let (mut app, root, ctx) = app();
        let key = seed(&root);
        show_sidebar(&mut app);
        app.execute(Command::DuplicateTask(key), &ctx);
        let id = app.view.task_editors.active().unwrap();
        let tab = app.view.task_editors.get(id).unwrap();
        assert_eq!(
            (tab.draft.id.as_str(), tab.draft.label.as_str()),
            ("build-copy", "Build copy")
        );
        assert!(tab.dirty(), "a copy is unsaved until saved");
        assert!(!tab.draft.is_default);
        assert!(app.view.page_tab_active());
        app.execute(Command::SaveTaskEditor(id), &ctx);
        let parsed =
            lumen_task_core::parse(&std::fs::read_to_string(workspace_file(&root)).unwrap())
                .unwrap();
        let ids: Vec<_> = parsed.tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["build", "build-copy"]);
        // Duplicating something that vanished explains itself.
        app.execute(
            Command::DuplicateTask(TaskKey::new(Scope::Workspace, "ghost")),
            &ctx,
        );
        assert!(app.view.status.contains("no longer exists"));
    }

    #[test]
    fn deleting_asks_first_edits_only_that_definition_and_keeps_comments() {
        let (mut app, root, ctx) = app();
        let key = seed(&root);
        let path = workspace_file(&root);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\n# about keep\n[[tasks]]\nid = \"keep\"\nlabel = \"Keep\"\ntype = \"process\"\ncommand = \"x\"\n")).unwrap();
        show_sidebar(&mut app);
        app.execute(Command::RequestDeleteTask(key.clone()), &ctx);
        assert_eq!(app.view.task_delete_prompt, Some(key.clone()));
        app.execute(Command::CancelDeleteTask, &ctx);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("\"build\""),
            "cancel deletes nothing"
        );
        app.execute(Command::RequestDeleteTask(key.clone()), &ctx);
        app.execute(Command::ConfirmDeleteTask(key), &ctx);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            !after.contains("id = \"build\"")
                && after.contains("id = \"keep\"")
                && after.contains("# about keep"),
            "{after}"
        );
        assert!(app.view.task_delete_prompt.is_none() && app.view.tasks.stale);
        assert!(app.view.status.contains("Deleted task 'build'"));
        // Unknown targets never open a prompt.
        app.execute(
            Command::RequestDeleteTask(TaskKey::new(Scope::Workspace, "ghost")),
            &ctx,
        );
        assert!(app.view.task_delete_prompt.is_none());
    }

    #[test]
    fn delete_is_refused_with_an_open_editor_or_a_changed_file() {
        let (mut app, root, ctx) = app();
        let key = seed(&root);
        let path = workspace_file(&root);
        show_sidebar(&mut app);
        app.execute(Command::OpenTaskEditor(key.clone()), &ctx);
        app.execute(Command::ConfirmDeleteTask(key.clone()), &ctx);
        assert!(app.view.status.contains("Close the editor tab"));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("\"build\"")
        );
        let id = app.view.task_editors.active().unwrap();
        app.execute(Command::CloseTaskEditor(id), &ctx);

        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}# external\n")).unwrap();
        app.execute(Command::ConfirmDeleteTask(key), &ctx);
        assert!(
            app.view.status.contains("Could not delete"),
            "{}",
            app.view.status
        );
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("# external") && after.contains("\"build\""));
        assert!(
            app.view.tasks.stale,
            "the list is refreshed after a failure"
        );
    }
}
