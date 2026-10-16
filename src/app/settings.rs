//! Scope lifecycle, drafts and persistence orchestration.
use super::LumenApp;
use lumen_settings_core::{
    self as settings, Layer, Scope, Setting, Value, catalog, parse, serialize, setting,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(crate) struct ConfigDraft {
    pub(crate) path: Option<PathBuf>,
    pub(crate) original: Option<String>,
    pub(crate) persisted: Layer,
    pub(crate) draft: Layer,
    pub(crate) error: Option<String>,
    pub(crate) warnings: Vec<String>,
}
impl ConfigDraft {
    pub(crate) fn pending(&self) -> usize {
        catalog()
            .filter(|setting| self.draft.get(setting.id) != self.persisted.get(setting.id))
            .count()
    }
}
#[derive(Default)]
pub(crate) struct SettingsState {
    /// Presence in the tab strip is independent of which editor is active.
    pub(crate) tab_open: bool,
    /// Whether the Settings view is currently active.
    pub(crate) open: bool,
    pub(crate) pending_quit: bool,
    pub(crate) focus_search: bool,
    pub(crate) query: String,
    pub(crate) category: String,
    pub(crate) scope: Scope,
    pub(crate) user: ConfigDraft,
    pub(crate) project: ConfigDraft,
    pub(crate) root: Option<PathBuf>,
    pub(crate) initialized: bool,
    projects: BTreeMap<PathBuf, ConfigDraft>,
    applied_sidebar: Option<bool>,
    applied_scale: Option<f64>,
    applied_theme: Option<(crate::ui::theme::Theme, bool)>,
}
impl SettingsState {
    pub(crate) fn has_drafts(&self) -> bool {
        self.user.pending() > 0
            || self.project.pending() > 0
            || self.projects.values().any(|config| config.pending() > 0)
    }
    pub(crate) fn discard_drafts(&mut self) {
        self.user.draft = self.user.persisted.clone();
        self.project.draft = self.project.persisted.clone();
        for config in self.projects.values_mut() {
            config.draft = config.persisted.clone();
        }
        self.pending_quit = false;
    }
    pub(crate) fn selected(&self) -> &ConfigDraft {
        match self.scope {
            Scope::User => &self.user,
            Scope::Project => &self.project,
        }
    }
    pub(crate) fn selected_mut(&mut self) -> &mut ConfigDraft {
        match self.scope {
            Scope::User => &mut self.user,
            Scope::Project => &mut self.project,
        }
    }
    pub(crate) fn effective(
        &self,
        setting: &Setting,
        document: Option<&Path>,
    ) -> (Value, &'static str) {
        let project = self
            .root
            .as_ref()
            .filter(|root| !setting.document || document.is_none_or(|path| path.starts_with(root)));
        settings::resolve(
            setting,
            &self.user.persisted,
            project.map(|_| &self.project.persisted),
        )
    }
    pub(crate) fn number(&self, id: &str, document: Option<&Path>) -> f32 {
        match self.effective(setting(id), document).0 {
            Value::Number(value) => value as f32,
            _ => unreachable!(),
        }
    }
    pub(crate) fn boolean(&self, id: &str) -> bool {
        matches!(self.effective(setting(id), None).0, Value::Bool(true))
    }
    pub(crate) fn document_boolean(&self, id: &str, document: Option<&Path>) -> bool {
        matches!(self.effective(setting(id), document).0, Value::Bool(true))
    }
    pub(crate) fn font_family(&self, document: Option<&Path>) -> String {
        match self.effective(setting("editor.font_family"), document).0 {
            Value::Choice(value) => value,
            _ => unreachable!(),
        }
    }
    pub(crate) fn document_choice(&self, id: &str, document: Option<&Path>) -> String {
        match self.effective(setting(id), document).0 {
            Value::Choice(value) => value,
            _ => unreachable!(),
        }
    }
    pub(crate) fn choice(&self, id: &str) -> String {
        match self.effective(setting(id), None).0 {
            Value::Choice(value) => value,
            _ => unreachable!(),
        }
    }
    /// User-scope value only. Executable paths never come from a project file,
    /// so opening an untrusted folder cannot choose which program Lumen runs.
    pub(crate) fn user_text(&self, id: &str) -> String {
        match settings::resolve(setting(id), &self.user.persisted, None).0 {
            Value::Text(value) => value,
            _ => unreachable!(),
        }
    }
    /// Whether the open project's settings file sets a non-empty text value.
    pub(crate) fn project_text_override(&self, id: &str) -> bool {
        self.root.is_some()
            && matches!(self.project.persisted.get(id), Some(Value::Text(value)) if !value.trim().is_empty())
    }
    pub(crate) fn search_exclude(&self) -> String {
        match self.effective(setting("search.exclude"), None).0 {
            Value::Text(value) => value,
            _ => unreachable!(),
        }
    }
}
impl LumenApp {
    pub(crate) fn initialize_settings(&mut self, user_path: Option<PathBuf>) {
        self.view.settings.initialized = true;
        self.view.settings.user = ConfigDraft {
            path: user_path,
            ..Default::default()
        };
        self.load_settings(Scope::User);
        self.sync_project_settings();
    }
    pub(crate) fn sync_project_settings(&mut self) {
        if !self.view.settings.initialized || self.view.settings.root == self.editor.workspace {
            return;
        }
        let state = &mut self.view.settings;
        if let Some(previous) = state.root.take() {
            state
                .projects
                .insert(previous, std::mem::take(&mut state.project));
        }
        state.root = self.editor.workspace.clone();
        if let Some(root) = &state.root {
            if let Some(draft) = state.projects.remove(root) {
                state.project = draft;
                if state.project.pending() == 0 {
                    self.load_settings(Scope::Project);
                }
            } else {
                state.project = ConfigDraft {
                    path: Some(root.join(".lumen/settings.toml")),
                    ..Default::default()
                };
                self.load_settings(Scope::Project);
            }
        } else {
            state.project = ConfigDraft::default();
            state.scope = Scope::User;
        }
    }
    fn load_settings(&mut self, scope: Scope) {
        let config = match scope {
            Scope::User => &self.view.settings.user,
            Scope::Project => &self.view.settings.project,
        };
        let result = config
            .path
            .as_ref()
            .ok_or_else(|| "User home directory is unavailable.".to_string())
            .and_then(|path| self.settings_store.read(path))
            .and_then(|content| {
                let (layer, warnings) = parse(content.as_deref())?;
                match scope {
                    Scope::User => {
                        settings::validate_layers(&layer, None)?;
                        settings::validate_layers(
                            &layer,
                            self.view
                                .settings
                                .root
                                .as_ref()
                                .map(|_| &self.view.settings.project.persisted),
                        )?;
                    }
                    Scope::Project => {
                        settings::validate_layers(&self.view.settings.user.persisted, Some(&layer))?
                    }
                }
                Ok((content, layer, warnings))
            });
        let config = match scope {
            Scope::User => &mut self.view.settings.user,
            Scope::Project => &mut self.view.settings.project,
        };
        match result {
            Ok((content, layer, warnings)) => {
                config.original = content;
                config.persisted = layer.clone();
                config.draft = layer;
                config.error = None;
                config.warnings = warnings;
            }
            Err(error) => config.error = Some(error), // Keep the last valid snapshot.
        }
    }
    pub(crate) fn apply_settings(&mut self) {
        let state = &self.view.settings;
        let scope = state.scope;
        let config = state.selected();
        let result = (|| {
            match scope {
                Scope::User => {
                    settings::validate_layers(&config.draft, None)?;
                    settings::validate_layers(
                        &config.draft,
                        state.root.as_ref().map(|_| &state.project.persisted),
                    )?;
                }
                Scope::Project => {
                    settings::validate_layers(&state.user.persisted, Some(&config.draft))?
                }
            }
            let path = config
                .path
                .as_ref()
                .ok_or("No settings path is available for this scope.")?;
            // Avoid overwriting an unsaved config document through a second surface.
            if self
                .editor
                .files
                .iter()
                .any(|file| file.path.as_ref() == Some(path))
            {
                return Err("The config file is open in the editor. Edit and save it there, or close its tab before applying settings here. Its text and history have been preserved.".into());
            }
            let content = serialize(config.original.as_deref(), &config.draft)?;
            self.settings_store.write(
                path,
                if scope == Scope::Project {
                    state.root.as_deref()
                } else {
                    None
                },
                config.original.as_deref(),
                &content,
            )?;
            Ok(content)
        })();
        let config = self.view.settings.selected_mut();
        match result {
            Ok(content) => {
                config.original = Some(content.clone());
                config.persisted = config.draft.clone();
                config.error = None;
                self.view.status = format!("{} settings saved", scope.label());
            }
            Err(error) => config.error = Some(error),
        }
    }
    pub(crate) fn reload_settings(&mut self) {
        if self.view.settings.selected().pending() != 0 {
            self.view.settings.selected_mut().error = Some(
                "Cancel pending changes before reloading. Your draft has been retained.".into(),
            );
        } else {
            self.load_settings(self.view.settings.scope);
        }
    }
    pub(crate) fn settings_file_saved(&mut self, path: &Path) {
        for scope in [Scope::User, Scope::Project] {
            let config = match scope {
                Scope::User => &self.view.settings.user,
                Scope::Project => &self.view.settings.project,
            };
            if config.path.as_deref() == Some(path) {
                if config.pending() == 0 {
                    self.load_settings(scope);
                } else {
                    self.view.status = "Config saved; settings form has pending edits. Cancel and Reload to load the file.".into();
                }
            }
        }
    }
    pub(crate) fn apply_workbench_settings(&mut self, ctx: &eframe::egui::Context) {
        if !self.view.settings.initialized {
            return;
        }
        let choice = self.view.settings.choice("workbench.theme");
        let theme = crate::ui::theme::Theme::resolve(&choice, ctx.system_theme());
        let follow_system = choice == "system";
        if self.view.settings.applied_theme != Some((theme, follow_system)) {
            crate::ui::theme::configure_theme(ctx, theme, follow_system);
            self.view.settings.applied_theme = Some((theme, follow_system));
        }
        let sidebar = self.view.settings.boolean("workbench.sidebar_visible");
        if self.view.settings.applied_sidebar != Some(sidebar) {
            self.view.sidebar_visible = sidebar;
            self.view.settings.applied_sidebar = Some(sidebar);
        }
        let scale = f64::from(self.view.settings.number("workbench.ui_scale", None));
        if self.view.settings.applied_scale != Some(scale) {
            ctx.set_zoom_factor(scale as f32);
            self.view.settings.applied_scale = Some(scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::settings::SettingsStore;
    use std::{cell::RefCell, rc::Rc};
    #[derive(Default)]
    struct MemoryStore {
        files: BTreeMap<PathBuf, String>,
        writes: usize,
        fail: bool,
    }
    #[derive(Clone, Default)]
    struct Store(Rc<RefCell<MemoryStore>>);
    impl SettingsStore for Store {
        fn read(&self, path: &Path) -> Result<Option<String>, String> {
            Ok(self.0.borrow().files.get(path).cloned())
        }
        fn write(
            &self,
            path: &Path,
            _: Option<&Path>,
            expected: Option<&str>,
            content: &str,
        ) -> Result<(), String> {
            let mut state = self.0.borrow_mut();
            if state.fail {
                return Err("permission denied".into());
            }
            if state.files.get(path).map(String::as_str) != expected {
                return Err("external conflict".into());
            }
            state.files.insert(path.into(), content.into());
            state.writes += 1;
            Ok(())
        }
    }
    fn fixture() -> (LumenApp, Store) {
        let store = Store::default();
        let mut app = LumenApp {
            settings_store: Box::new(store.clone()),
            ..Default::default()
        };
        app.initialize_settings(Some(PathBuf::from("/user/.lumen/settings.toml")));
        (app, store)
    }
    #[test]
    fn theme_apply_failure_cancel_and_system_changes_preserve_editor_and_drafts() {
        use crate::ui::theme::{self, Theme};
        let (mut app, store) = fixture();
        let ctx = eframe::egui::Context::default();
        theme::configure_ui(&ctx);
        app.editor.new_file();
        app.editor.files[0].text = "unsaved text".into();
        app.editor.files[0].dirty = true;
        let document = app.editor.files[0].id;
        let focus = super::super::editor_action::text_widget_id(document);
        ctx.memory_mut(|memory| memory.request_focus(focus));
        app.view
            .settings
            .user
            .draft
            .insert("workbench.theme".into(), Value::Choice("light".into()));
        app.apply_workbench_settings(&ctx);
        assert_eq!(theme::current(&ctx.style().visuals), Theme::Dark);
        store.0.borrow_mut().fail = true;
        app.apply_settings();
        app.apply_workbench_settings(&ctx);
        assert_eq!(theme::current(&ctx.style().visuals), Theme::Dark);
        assert!(app.view.settings.has_drafts());
        store.0.borrow_mut().fail = false;
        app.apply_settings();
        app.apply_workbench_settings(&ctx);
        assert_eq!(theme::current(&ctx.style().visuals), Theme::Light);
        assert!(ctx.memory(|memory| memory.has_focus(focus)));
        app.view.settings.user.draft.insert(
            "workbench.theme".into(),
            Value::Choice("high-contrast".into()),
        );
        app.view.settings.discard_drafts();
        app.apply_workbench_settings(&ctx);
        assert_eq!(theme::current(&ctx.style().visuals), Theme::Light);
        app.view
            .settings
            .user
            .draft
            .insert("workbench.theme".into(), Value::Choice("system".into()));
        app.apply_settings();
        app.view
            .settings
            .user
            .draft
            .insert("editor.font_size".into(), Value::Number(15.0));
        for (system, expected) in [
            (eframe::egui::Theme::Dark, Theme::Dark),
            (eframe::egui::Theme::Light, Theme::Light),
        ] {
            let _ = ctx.run(
                eframe::egui::RawInput {
                    system_theme: Some(system),
                    ..Default::default()
                },
                |ctx| app.apply_workbench_settings(ctx),
            );
            assert_eq!(theme::current(&ctx.style().visuals), expected);
            assert_eq!(app.view.settings.user.pending(), 1);
        }
        assert_eq!(app.editor.files[0].id, document);
        assert_eq!(app.editor.files[0].text, "unsaved text");
        assert!(app.editor.files[0].dirty);
    }

    #[test]
    fn project_editor_scheme_is_local_to_workspace_documents() {
        let (mut app, store) = fixture();
        store.0.borrow_mut().files.insert(
            "/project/.lumen/settings.toml".into(),
            "[workbench]\ntheme = 'high-contrast'\n[editor]\ncolor_scheme = 'lumen-light'\n".into(),
        );
        app.editor.workspace = Some("/project".into());
        app.sync_project_settings();
        let ctx = eframe::egui::Context::default();
        app.apply_workbench_settings(&ctx);
        assert_eq!(
            crate::ui::theme::current(&ctx.style().visuals),
            crate::ui::theme::Theme::HighContrast
        );
        assert_eq!(
            app.view
                .settings
                .document_choice("editor.color_scheme", Some(Path::new("/project/main.rs"))),
            "lumen-light"
        );
        assert_eq!(
            app.view
                .settings
                .document_choice("editor.color_scheme", Some(Path::new("/other/main.rs"))),
            "workbench"
        );
    }

    #[test]
    fn project_priority_outside_files_and_no_discovery_or_implicit_creation() {
        let (mut app, store) = fixture();
        store.0.borrow_mut().files.insert(
            "/project/.lumen/settings.toml".into(),
            "[editor]\nfont_size = 16\n".into(),
        );
        app.editor.workspace = Some("/project".into());
        app.sync_project_settings();
        assert_eq!(
            app.view
                .settings
                .number("editor.font_size", Some(Path::new("/project/src/a.rs"))),
            16.0
        );
        assert_eq!(app.view.settings.number("editor.font_size", None), 16.0);
        assert_eq!(
            app.view
                .settings
                .number("editor.font_size", Some(Path::new("/elsewhere/a.rs"))),
            14.0
        );
        assert_eq!(store.0.borrow().writes, 0);
        app.editor.workspace = None;
        app.sync_project_settings();
        assert_eq!(
            app.view
                .settings
                .number("editor.font_size", Some(Path::new("/project/src/a.rs"))),
            14.0
        );
    }
    #[test]
    fn drafts_are_separate_and_project_switch_does_not_discard_them() {
        let (mut app, _) = fixture();
        app.view
            .settings
            .user
            .draft
            .insert("editor.font_size".into(), Value::Number(15.0));
        app.editor.workspace = Some("/first".into());
        app.sync_project_settings();
        app.view
            .settings
            .project
            .draft
            .insert("editor.font_size".into(), Value::Number(16.0));
        app.editor.workspace = Some("/second".into());
        app.sync_project_settings();
        assert_eq!(app.view.settings.project.pending(), 0);
        app.editor.workspace = Some("/first".into());
        app.sync_project_settings();
        assert_eq!(app.view.settings.project.pending(), 1);
        assert_eq!(app.view.settings.user.pending(), 1);
    }
    #[test]
    fn failed_save_reload_and_invalid_layers_keep_valid_state_and_drafts() {
        let (mut app, store) = fixture();
        app.view
            .settings
            .user
            .draft
            .insert("editor.font_size".into(), Value::Number(16.0));
        store.0.borrow_mut().fail = true;
        app.apply_settings();
        assert_eq!(app.view.settings.number("editor.font_size", None), 14.0);
        assert_eq!(app.view.settings.user.pending(), 1);
        app.reload_settings();
        assert_eq!(app.view.settings.user.pending(), 1);
        store.0.borrow_mut().fail = false;
        app.apply_settings();
        assert_eq!(app.view.settings.number("editor.font_size", None), 16.0);
        assert_eq!(app.view.settings.user.pending(), 0);
        store.0.borrow_mut().files.insert(
            "/user/.lumen/settings.toml".into(),
            "[editor]\nfont_size = 'invalid'".into(),
        );
        app.reload_settings();
        assert_eq!(app.view.settings.number("editor.font_size", None), 16.0);
        assert!(app.view.settings.user.error.is_some());
    }
    #[test]
    fn config_tabs_and_geometry_errors_never_overwrite_or_mutate_documents() {
        let (mut app, store) = fixture();
        let path = app.view.settings.user.path.clone().unwrap();
        let mut document = crate::model::Document::from_path(path, "# unsaved config".into());
        document.dirty = true;
        let id = document.id;
        app.editor.files.push(document);
        app.view
            .settings
            .user
            .draft
            .insert("editor.font_size".into(), Value::Number(16.0));
        app.apply_settings();
        assert_eq!(store.0.borrow().writes, 0);
        assert_eq!(app.editor.files[0].text, "# unsaved config");
        assert!(app.editor.files[0].dirty);
        assert_eq!(app.editor.files[0].id, id);
        app.editor.files.clear();
        app.view
            .settings
            .user
            .draft
            .insert("editor.font_size".into(), Value::Number(30.0));
        app.apply_settings();
        assert_eq!(store.0.borrow().writes, 0);
        assert_eq!(app.view.settings.number("editor.font_size", None), 14.0);
    }
}
