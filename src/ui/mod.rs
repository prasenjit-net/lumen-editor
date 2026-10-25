//! Workbench components and shared drawing primitives.
mod activity_bar;
pub(crate) mod agent_install;
mod agent_message;
mod agent_panel;
mod bottom_tabs;
mod breadcrumbs;
mod chat;
mod chat_controls;
mod chrome;
mod completion;
mod csv_editor;
mod dialog;
mod dialogs;
mod editor;
mod editor_find;
mod editor_menu;
mod editor_shared;
mod editor_split;
mod editor_tabs;
mod editor_viewport;
mod explorer;
mod explorer_actions;
mod explorer_edit;
mod file_icons;
mod git_history_tab;
mod image_viewer;
mod language_edit;
mod language_navigation;
mod markdown_preview;
mod match_highlight;
mod navigation;
mod output;
mod palette;
mod popup_place;
mod problems;
mod settings;
pub(crate) mod shortcut;
mod source_control;
mod source_control_menu;
mod source_control_rows;
mod svg_preview;
mod tab_drag;
mod task_dialogs;
mod task_editor;
mod task_launcher;
mod task_output;
mod tasks;
mod terminal;
pub(crate) mod theme;
mod widgets;
mod workspace_search;
mod workspace_search_diff;

use crate::{
    app::{command::Command, view_state::ViewState},
    model::{Document, EditorState},
    services::FileSystem,
};

/// Borrowed presentation context. Components render state and emit commands;
/// they never open dialogs, write files, or own the application controller.
pub(crate) struct Workbench<'a> {
    editor: &'a mut EditorState,
    view: &'a mut ViewState,
    file_system: &'a dyn FileSystem,
    commands: Vec<Command>,
}

impl<'a> Workbench<'a> {
    pub(crate) fn new(
        editor: &'a mut EditorState,
        view: &'a mut ViewState,
        file_system: &'a dyn FileSystem,
    ) -> Self {
        Self {
            editor,
            view,
            file_system,
            commands: Vec::new(),
        }
    }
    pub(crate) fn take_commands(self) -> Vec<Command> {
        self.commands
    }
    fn active(&self) -> Option<&Document> {
        self.editor.active()
    }
    fn command(&mut self, command: Command) {
        if !command.apply_local(self.editor, self.view) {
            self.commands.push(command);
        }
    }
}
