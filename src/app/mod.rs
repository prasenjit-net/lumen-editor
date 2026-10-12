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
pub(crate) mod settings;
#[cfg(test)]
mod tests;
pub(crate) mod view_state;
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
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_menu: None,
        }
    }
}
impl eframe::App for LumenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_project_settings();
        self.apply_workbench_settings(ctx);
        self.handle_events(ctx);
        // Apply actions after each component, retaining the original frame order:
        // an explorer click can still open a file in the editor this same frame.
        self.render_component(ctx, |workbench| workbench.title_bar(ctx));
        self.render_component(ctx, |workbench| workbench.status_bar(ctx));
        self.render_component(ctx, |workbench| workbench.activity_bar(ctx));
        self.render_component(ctx, |workbench| workbench.sidebar(ctx));
        self.render_component(ctx, |workbench| workbench.editor(ctx));
        self.render_component(ctx, |workbench| workbench.palette(ctx));
        self.render_component(ctx, |workbench| workbench.unsaved_changes_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.settings_quit_dialog(ctx));
        self.render_component(ctx, |workbench| workbench.delete_dialog(ctx));
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
