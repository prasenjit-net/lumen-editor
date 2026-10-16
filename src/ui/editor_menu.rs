use super::Workbench;
use crate::app::{command::Command, editor_action::EditorAction};
use crate::model::DocumentId;
use eframe::egui;

/// egui 0.33 treats any pointer button press as a new text selection anchor.
/// Hide only a secondary-button gesture from TextEdit while a selection exists.
/// Restore pointer input before the popup (or any other widget) processes it.
pub(super) fn show_editor_text(
    ui: &mut egui::Ui,
    edit: egui::TextEdit<'_>,
    id: egui::Id,
) -> egui::text_edit::TextEditOutput {
    let selected = egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .is_some_and(|range| !range.is_empty());
    let over_editor = ui.rect_contains_pointer(ui.clip_rect());
    let pointer = ui.input_mut(|input| {
        let secondary = input.pointer.secondary_pressed()
            || input.pointer.secondary_down()
            || input.pointer.secondary_released();
        let primary = input.pointer.primary_pressed()
            || input.pointer.primary_down()
            || input.pointer.primary_released();
        (selected && over_editor && secondary && !primary)
            .then(|| std::mem::take(&mut input.pointer))
    });
    if pointer.is_some() {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    let mut output = edit.show(ui);
    if let Some(pointer) = pointer {
        ui.input_mut(|input| input.pointer = pointer);
        // Recover hover/secondary-click response flags with the real pointer,
        // without running the widget's selection handler a second time.
        output.response |= ui.interact(output.response.rect, id, output.response.sense);
    }
    output
}

impl Workbench<'_> {
    pub(super) fn editor_context_menu(
        &mut self,
        output: &egui::text_edit::TextEditOutput,
        document: DocumentId,
    ) {
        let cursor = output
            .state
            .cursor
            .range(&output.galley)
            .unwrap_or_default();
        let text = self
            .editor
            .active()
            .map(|file| file.text.clone())
            .unwrap_or_default();
        let history = output.state.undoer();
        self.editor_popup(
            &output.response,
            document,
            history.has_undo(&(cursor, text.clone())),
            history.has_redo(&(cursor, text)),
        );
    }

    pub(super) fn editor_popup(
        &mut self,
        response: &egui::Response,
        document: DocumentId,
        can_undo: bool,
        can_redo: bool,
    ) {
        response.context_menu(|ui| {
            let command_key = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            for (action, title, shortcut, enabled) in [
                (
                    EditorAction::Undo,
                    "Undo",
                    format!("{command_key}Z"),
                    can_undo,
                ),
                (
                    EditorAction::Redo,
                    "Redo",
                    if cfg!(target_os = "macos") {
                        "Shift+⌘Z".into()
                    } else {
                        "Ctrl+Shift+Z".into()
                    },
                    can_redo,
                ),
                (EditorAction::Cut, "Cut", format!("{command_key}X"), true),
                (EditorAction::Copy, "Copy", format!("{command_key}C"), true),
                (
                    EditorAction::Paste,
                    "Paste",
                    format!("{command_key}V"),
                    true,
                ),
                (
                    EditorAction::SelectAll,
                    "Select All",
                    format!("{command_key}A"),
                    true,
                ),
            ] {
                if matches!(action, EditorAction::Cut | EditorAction::SelectAll) {
                    ui.separator();
                }
                if ui
                    .add_enabled(enabled, egui::Button::new(title).shortcut_text(shortcut))
                    .clicked()
                {
                    self.command(Command::EditorAction(document, action));
                    ui.close();
                }
            }
            ui.separator();
            for (command, title, shortcut) in [
                (Command::Find, "Find", format!("{command_key}F")),
                (
                    Command::Replace,
                    "Replace",
                    if cfg!(target_os = "macos") {
                        "Alt+⌘F".into()
                    } else {
                        "Ctrl+H".into()
                    },
                ),
                (Command::GoToDefinition, "Go to Definition", "F12".into()),
                (Command::ShowHover, "Show Hover", String::new()),
                (
                    Command::TriggerCompletion,
                    "Trigger Suggest",
                    "Ctrl+Space".into(),
                ),
                (
                    Command::ShowSignatureHelp,
                    "Show Signature Help",
                    "Ctrl+Shift+Space".into(),
                ),
                (
                    Command::ShowDocumentSymbols,
                    "Go to Symbol in Editor",
                    format!("Shift+{command_key}O"),
                ),
                (
                    Command::AskAgentAboutSelection,
                    "Ask Agent About Selection",
                    String::new(),
                ),
            ] {
                if ui
                    .add(egui::Button::new(title).shortcut_text(shortcut))
                    .clicked()
                {
                    self.command(command);
                    ui.close();
                }
            }
        });
    }
}
