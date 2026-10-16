//! Workbench components and shared drawing primitives.
mod activity_bar;
mod chrome;
mod completion;
mod dialogs;
mod editor;
mod editor_find;
mod editor_gutter;
mod editor_menu;
#[cfg(feature = "shared-editor")]
mod editor_shared;
mod editor_tabs;
mod editor_viewport;
mod explorer;
mod explorer_actions;
mod explorer_edit;
mod language_edit;
mod language_navigation;
mod navigation;
mod palette;
mod problems;
mod settings;
mod source_control;
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
