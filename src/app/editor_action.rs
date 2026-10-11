//! Document-targeted actions for editor popups, processed before TextEdit input.
use super::view_state::ViewState;
use crate::model::DocumentId;
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingEditorAction {
    pub(crate) document: DocumentId,
    pub(crate) action: EditorAction,
}

pub(crate) fn text_widget_id(document: DocumentId) -> egui::Id {
    egui::Id::new(("editor_text", document))
}

pub(crate) fn blocked(view: &ViewState) -> bool {
    view.command_palette
        || view.pending_close.is_some()
        || view.pending_quit
        || view.delete_prompt.is_some()
        || view.explorer_edit.is_some()
}

/// Reject delayed popup paste before any other input widget can consume it.
pub(crate) fn validate_paste(
    view: &mut ViewState,
    active: Option<DocumentId>,
    ctx: &egui::Context,
) {
    if let Some(target) = view.editor_paste_target
        && ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)))
        })
    {
        let focused = ctx.memory(|memory| memory.has_focus(text_widget_id(target)));
        if active != Some(target) || blocked(view) || !focused {
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
        }
        view.editor_paste_target = None;
    }
}

pub(crate) fn prepare_action(view: &mut ViewState, document: DocumentId, ctx: &egui::Context) {
    if let Some(pending) = view.pending_editor_action.take()
        && pending.document == document
        && !blocked(view)
    {
        view.explorer_focused = false;
        ctx.memory_mut(|memory| memory.request_focus(text_widget_id(document)));
        if pending.action == EditorAction::Paste {
            view.editor_paste_target = Some(document);
        }
        pending.action.send(ctx);
    }
}

impl EditorAction {
    /// The widget remains the sole mutation/history owner during migration.
    pub(crate) fn send(self, ctx: &egui::Context) {
        match self {
            Self::Copy => ctx.input_mut(|input| input.events.push(egui::Event::Copy)),
            Self::Cut => ctx.input_mut(|input| input.events.push(egui::Event::Cut)),
            Self::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Self::Undo | Self::Redo | Self::SelectAll => ctx.input_mut(|input| {
                input.events.push(egui::Event::Key {
                    key: if self == Self::SelectAll {
                        egui::Key::A
                    } else {
                        egui::Key::Z
                    },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        command: true,
                        mac_cmd: cfg!(target_os = "macos"),
                        ctrl: !cfg!(target_os = "macos"),
                        shift: self == Self::Redo,
                        ..Default::default()
                    },
                });
            }),
        }
    }
}
