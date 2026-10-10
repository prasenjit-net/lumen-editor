//! Workbench components and shared drawing primitives.
mod activity_bar;
mod chrome;
mod dialogs;
mod editor;
mod explorer;
mod palette;
pub(crate) mod theme;
mod widgets;

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
