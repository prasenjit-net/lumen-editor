//! Application orchestration: shared commands, workflows and frame ordering.
pub(crate) mod agent;
pub(crate) mod agent_changes;
pub(crate) mod agent_context;
pub(crate) mod agent_history;
pub(crate) mod agent_permission;
mod cli_open;
mod clipboard;
pub(crate) mod command;
pub(crate) mod completion;
mod dispatch;
mod documents;
pub(crate) mod editor_action;
pub(crate) mod editor_clipboard;
pub(crate) mod editor_input;
mod events;
mod explorer_ops;
pub(crate) mod find;
mod folding;
mod language_edits;
mod language_services;
mod navigation;
pub(crate) mod output;
mod safety;
pub(crate) mod scroll_sync;
pub(crate) mod settings;
pub(crate) mod source_control;
pub(crate) mod tab_close;
pub(crate) mod tab_commands;
pub(crate) mod task_catalog;
pub(crate) mod task_editor;
pub(crate) mod task_graph;
pub(crate) mod task_runs;
mod tasks;
pub(crate) mod terminal;
#[cfg(test)]
mod tests;
pub(crate) mod view_state;
pub(crate) mod workspace_edit;
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
    /// Location of the user tasks file; `None` in tests so they never touch the real home.
    pub(crate) task_user_path: Option<std::path::PathBuf>,
    pub(crate) task_launcher: std::sync::Arc<dyn crate::services::task_process::TaskLauncher>,
    pub(crate) task_shell: crate::services::task_process::ShellSpec,
    pub(crate) task_trust: Box<dyn crate::services::task_trust::TaskTrustStore>,
    /// Use the login shell's PATH when starting tasks (GUI apps do not inherit it).
    pub(crate) task_login_path: bool,
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
    pub(crate) language_server_process:
        Option<Box<dyn crate::services::language_server::LanguageServerConnection>>,
    pub(crate) retiring_language_servers: Vec<language_services::RetiringLanguageServer>,
    pub(crate) language_server_workspace: Option<std::path::PathBuf>,
    pub(crate) language_server_language: Option<lumen_language_services::LanguageId>,
    pub(crate) language_server_generation: lumen_language_services::SessionGeneration,
    pub(crate) language_server_initialized: bool,
    pub(crate) language_server_capabilities:
        Option<lumen_language_services::capabilities::ServerCapabilities>,
    pub(crate) language_server_requests: lumen_language_services::requests::RequestTracker,
    pub(crate) language_server_request_kinds:
        std::collections::HashMap<u64, language_services::PendingLanguageRequest>,
    pub(crate) language_server_documents: std::collections::HashMap<
        crate::model::DocumentId,
        language_services::SyncedLanguageDocument,
    >,
    pub(crate) language_server_restart: language_services::LanguageServerRestart,
    pub(crate) semantic_token_request: Option<language_services::SemanticTokenAttempt>,
    pub(crate) language_server_started_at: Option<std::time::Instant>,
    pub(crate) language_server_ready_at: Option<std::time::Instant>,
    /// Last status/terminal text mirrored into the log, to log changes once.
    pub(crate) logged_status: String,
    pub(crate) logged_terminal_error: Option<String>,
    pub(crate) agent_runtime: agent::AgentRuntime,
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
            task_user_path: None,
            task_launcher: std::sync::Arc::new(crate::services::task_process::LocalTaskLauncher),
            task_shell: crate::services::task_process::ShellSpec::platform_default(),
            task_trust: Box::<crate::services::task_trust::MemoryTrust>::default(),
            task_login_path: false,
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
            retiring_language_servers: Vec::new(),
            language_server_workspace: None,
            language_server_language: None,
            language_server_generation: Default::default(),
            language_server_initialized: false,
            language_server_capabilities: None,
            language_server_requests: Default::default(),
            language_server_request_kinds: Default::default(),
            language_server_documents: Default::default(),
            language_server_restart: Default::default(),
            semantic_token_request: None,
            language_server_started_at: None,
            language_server_ready_at: None,
            logged_status: String::new(),
            logged_terminal_error: None,
            agent_runtime: Default::default(),
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
        self.refresh_language_server_status(ctx);
        self.poll_language_server(ctx);
        self.handle_events(ctx);
        // Apply actions after each component, retaining the original frame order:
        // an explorer click can still open a file in the editor this same frame.
        self.render_component(ctx, |workbench| workbench.title_bar(ctx));
        self.render_component(ctx, |workbench| workbench.status_bar(ctx));
        self.render_component(ctx, |workbench| workbench.activity_bar(ctx));
        self.ensure_source_control(ctx);
        self.ensure_tasks();
        self.poll_task_runs(ctx);
        self.render_component(ctx, |workbench| workbench.sidebar(ctx));
        self.render_component(ctx, |workbench| workbench.agent_panel(ctx));
        self.render_component(ctx, |workbench| workbench.problems_panel(ctx));
        self.render_component(ctx, |workbench| workbench.output_panel(ctx));
        self.render_component(ctx, |workbench| workbench.task_output_panel(ctx));
        self.render_component(ctx, |workbench| workbench.terminal_panel(ctx));
        self.resize_terminal(self.view.terminal.rows, self.view.terminal.columns);
        self.render_component(ctx, |workbench| workbench.editor(ctx));
        self.render_component(ctx, |workbench| workbench.palette(ctx));
        self.render_component(ctx, |workbench| workbench.navigation_palette(ctx));
        self.render_component(ctx, |workbench| workbench.language_navigation(ctx));
        self.render_component(ctx, |workbench| workbench.rename_prompt(ctx));
        self.render_component(ctx, |workbench| workbench.language_edit_preview(ctx));
        self.advance_bulk_close();
        self.render_component(ctx, |workbench| workbench.unsaved_changes_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.task_close_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.task_delete_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.task_run_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.task_run_close_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.task_workspace_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.task_quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.settings_quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.delete_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.source_revert_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.save_conflict_dialog(ctx));
        self.render_component(ctx, |workbench| {
            workbench.language_server_details_dialog(ctx)
        });
        self.render_component(ctx, |workbench| workbench.safety_details_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.workspace_replace_preview(ctx));
        self.maintain_documents(ctx);
        self.poll_workspace_search(ctx);
        self.poll_file_index(ctx);
        self.poll_source_control(ctx);
        self.poll_terminal(ctx);
        self.poll_agents(ctx);
        self.log_status_changes();
        self.sync_output();
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
