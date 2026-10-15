//! Application orchestration: shared commands, workflows and frame ordering.
mod clipboard;
pub(crate) mod command;
mod dispatch;
mod documents;
pub(crate) mod editor_action;
pub(crate) mod editor_clipboard;
pub(crate) mod editor_input;
mod events;
mod explorer_ops;
pub(crate) mod find;
mod folding;
mod language_services;
mod navigation;
mod safety;
pub(crate) mod settings;
pub(crate) mod source_control;
pub(crate) mod terminal;
#[cfg(test)]
mod tests;
pub(crate) mod view_state;
pub(crate) mod workspace_search;
use crate::{
    model::EditorState,
    services::{
        ClipboardService, FileDialogs, FileSystem, LocalFileSystem, NativeClipboard, NativeDialogs,
    },
};
use eframe::egui;
use view_state::ViewState;
pub(crate) struct LumenApp {
    pub(crate) editor: EditorState,
    pub(crate) view: ViewState,
    pub(crate) file_system: Box<dyn FileSystem>,
    pub(crate) dialogs: Box<dyn FileDialogs>,
    pub(crate) clipboard: Box<dyn ClipboardService>,
    pub(crate) settings_store: Box<dyn crate::services::settings::SettingsStore>,
    pub(crate) recovery_store: Box<dyn crate::services::recovery::RecoveryStore>,
    pub(crate) recent_files_store: Option<Box<dyn crate::services::recent_files::RecentFilesStore>>,
    pub(crate) safety: safety::SafetyState,
    pub(crate) search_job: Option<workspace_search::SearchJob>,
    pub(crate) file_index_job: Option<navigation::FileIndexJob>,
    pub(crate) source_control_job: Option<source_control::SourceControlJob>,
    pub(crate) source_control_service:
        std::sync::Arc<dyn crate::services::source_control::SourceControlService>,
    pub(crate) language_server_service:
        std::sync::Arc<dyn crate::services::language_server::LanguageServerService>,
    pub(crate) language_server_process: Option<crate::services::language_server::ServerProcess>,
    pub(crate) language_server_workspace: Option<std::path::PathBuf>,
    pub(crate) language_server_language: Option<lumen_language_services::LanguageId>,
    pub(crate) language_server_document: Option<(
        crate::model::DocumentId,
        u64,
        lumen_language_services::DocumentUri,
    )>,
    pub(crate) terminal_session: Option<crate::services::terminal::TerminalSession>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) native_menu: Option<crate::platform::native_menu::NativeMenu>,
}
impl Default for LumenApp {
    fn default() -> Self {
        Self {
            editor: EditorState::default(),
            view: ViewState::default(),
            file_system: Box::new(LocalFileSystem),
            dialogs: Box::new(NativeDialogs),
            clipboard: Box::<NativeClipboard>::default(),
            settings_store: Box::new(crate::services::settings::LocalSettingsStore),
            recovery_store: Box::new(crate::services::recovery::LocalRecoveryStore::new()),
            recent_files_store: None,
            safety: safety::SafetyState::default(),
            search_job: None,
            file_index_job: None,
            source_control_job: None,
            source_control_service: std::sync::Arc::new(crate::services::source_control::LocalGit),
            language_server_service: std::sync::Arc::new(
                crate::services::language_server::LocalLanguageServer,
            ),
            language_server_process: None,
            language_server_workspace: None,
            language_server_language: None,
            language_server_document: None,
            terminal_session: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_menu: None,
        }
    }
}
impl eframe::App for LumenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_project_settings();
        self.apply_workbench_settings(ctx);
        self.refresh_language_server_status();
        self.poll_language_server();
        self.handle_events(ctx);
        // Apply actions after each component, retaining the original frame order:
        // an explorer click can still open a file in the editor this same frame.
        self.render_component(ctx, |workbench| workbench.title_bar(ctx));
        self.render_component(ctx, |workbench| workbench.status_bar(ctx));
        self.render_component(ctx, |workbench| workbench.activity_bar(ctx));
        self.ensure_source_control(ctx);
        self.render_component(ctx, |workbench| workbench.sidebar(ctx));
        self.render_component(ctx, |workbench| workbench.terminal_panel(ctx));
        self.resize_terminal(self.view.terminal.rows, self.view.terminal.columns);
        self.render_component(ctx, |workbench| workbench.editor(ctx));
        self.render_component(ctx, |workbench| workbench.palette(ctx));
        self.render_component(ctx, |workbench| workbench.navigation_palette(ctx));
        self.render_component(ctx, |workbench| workbench.unsaved_changes_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.settings_quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.delete_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.source_revert_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.save_conflict_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.safety_details_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.workspace_replace_preview(ctx));
        self.maintain_documents(ctx);
        self.poll_workspace_search(ctx);
        self.poll_file_index(ctx);
        self.poll_source_control(ctx);
        self.poll_terminal(ctx);
    }
}

impl LumenApp {
    pub(crate) fn render_component(
        &mut self,
        ctx: &egui::Context,
        render: impl FnOnce(&mut crate::ui::Workbench<'_>),
    ) {
        let mut workbench =
            crate::ui::Workbench::new(&mut self.editor, &mut self.view, self.file_system.as_ref());
        render(&mut workbench);
        for command in workbench.take_commands() {
            self.execute(command, ctx);
        }
        self.start_clipboard_read(ctx);
    }
}
