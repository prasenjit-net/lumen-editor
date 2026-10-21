use super::Workbench;
use crate::app::{command::Command, editor_action::EditorAction};
use crate::model::DocumentId;
use eframe::egui;

impl Workbench<'_> {
    pub(super) fn editor_popup(
        &mut self,
        response: &egui::Response,
        document: DocumentId,
        can_undo: bool,
        can_redo: bool,
    ) {
        response.context_menu(|ui| {
            for (action, title, shortcut, enabled) in [
                (
                    EditorAction::Undo,
                    "Undo",
                    super::shortcut::primary("Z"),
                    can_undo,
                ),
                (
                    EditorAction::Redo,
                    "Redo",
                    super::shortcut::primary("Shift+Z"),
                    can_redo,
                ),
                (
                    EditorAction::Cut,
                    "Cut",
                    super::shortcut::primary("X"),
                    true,
                ),
                (
                    EditorAction::Copy,
                    "Copy",
                    super::shortcut::primary("C"),
                    true,
                ),
                (
                    EditorAction::Paste,
                    "Paste",
                    super::shortcut::primary("V"),
                    true,
                ),
                (
                    EditorAction::SelectAll,
                    "Select All",
                    super::shortcut::primary("A"),
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
                (Command::Find, "Find", super::shortcut::primary("F")),
                (
                    Command::Replace,
                    "Replace",
                    if cfg!(target_os = "macos") {
                        super::shortcut::primary("Alt+F")
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
                    super::shortcut::primary("Shift+O"),
                ),
            ] {
                // Same availability rules as the command palette, with the reason.
                let reason = self.palette_disabled_reason(&command);
                let response = ui.add_enabled(
                    reason.is_none(),
                    egui::Button::new(title).shortcut_text(shortcut),
                );
                let response = match reason {
                    Some(reason) => response.on_disabled_hover_text(reason),
                    None => response,
                };
                if response.clicked() {
                    self.command(command);
                    ui.close();
                }
            }
        });
    }
}
