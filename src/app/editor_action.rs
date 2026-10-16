//! Document-targeted actions for editor popups, processed before TextEdit input.
use super::view_state::ViewState;
use crate::model::{Document, DocumentId};
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

#[derive(Clone, Copy)]
pub(crate) struct PasteTarget {
    pub(crate) request: crate::services::ClipboardRequestId,
    document: DocumentId,
    revision: u64,
    selection: egui::text::CCursorRange,
    cancelled: bool,
}

pub(crate) fn request_paste(view: &mut ViewState, document: &Document, ctx: &egui::Context) {
    // An accelerator may already have delivered its platform paste event.
    if ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
    }) {
        return;
    }
    let Some(selection) = current_selection(document, ctx) else {
        return;
    };
    if blocked(view) || view.pending_text_input.is_some() {
        view.status = "Paste cancelled: finish the current editor input first".into();
        return;
    }
    let request = crate::services::ClipboardRequestId::allocate();
    view.editor_paste_target = Some(PasteTarget {
        request,
        document: document.id,
        revision: document.revision,
        selection,
        cancelled: false,
    });
    view.clipboard_read = Some(request);
    #[cfg(feature = "shared-editor")]
    {
        view.shared_paste_selection = document
            .editing
            .as_ref()
            .map(|buffer| buffer.selections().clone());
    }
}

pub(crate) fn current_selection(
    document: &Document,
    ctx: &egui::Context,
) -> Option<egui::text::CCursorRange> {
    #[cfg(feature = "shared-editor")]
    if let Some(buffer) = &document.editing {
        let set = buffer.selections();
        let selection = set.selections()[set.primary_index()];
        return Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(buffer.text()[..selection.anchor].chars().count()),
            egui::text::CCursor::new(buffer.text()[..selection.head].chars().count()),
        ));
    }
    egui::text_edit::TextEditState::load(ctx, text_widget_id(document.id))
        .and_then(|widget| widget.cursor.char_range())
}

pub(crate) fn text_widget_id(document: DocumentId) -> egui::Id {
    egui::Id::new(("editor_text", document))
}

pub(crate) fn blocked(view: &ViewState) -> bool {
    view.settings.open
        || view.settings.pending_quit
        || view.command_palette
        || view.navigation.mode.is_some()
        || view.pending_close.is_some()
        || view.pending_quit
        || view.delete_prompt.is_some()
        || view.explorer_edit.is_some()
        || view.rename_prompt.is_some()
        || view.language_edit_plan.is_some()
}

/// Reject delayed popup paste before any other input widget can consume it.
pub(crate) fn validate_paste(view: &mut ViewState, active: Option<&Document>, ctx: &egui::Context) {
    if let Some(mut target) = view.editor_paste_target {
        let id = text_widget_id(target.document);
        let selection = active
            .filter(|document| document.id == target.document)
            .and_then(|document| current_selection(document, ctx));
        #[cfg(feature = "shared-editor")]
        {
            target.cancelled |= view
                .shared_paste_selection
                .as_ref()
                .is_some_and(|snapshot| {
                    !active
                        .and_then(|document| document.editing.as_ref())
                        .is_some_and(|buffer| buffer.selections() == snapshot)
                });
        }
        let (delivered, interrupted) = ctx.input(|input| {
            let delivered = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::Paste(_)));
            let interrupted = input.pointer.any_pressed()
                || input.pointer.any_released()
                || input.pointer.any_down()
                || !input.focused
                || input
                    .events
                    .iter()
                    .take_while(|event| !matches!(event, egui::Event::Paste(_)))
                    .any(|event| match event {
                        egui::Event::Text(text) => !text.is_empty(),
                        egui::Event::Cut | egui::Event::Ime(_) => true,
                        egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => !(*key == egui::Key::V && modifiers.command),
                        _ => false,
                    });
            (delivered, interrupted)
        });
        target.cancelled |= !active.is_some_and(|document| {
            document.id == target.document && document.revision == target.revision
        }) || selection != Some(target.selection)
            || blocked(view)
            || view.explorer_focused
            || view
                .find
                .get(&target.document)
                .is_some_and(|find| find.focus)
            || !ctx.memory(|memory| memory.has_focus(id))
            || interrupted
            || view.pending_text_input.is_some();
        if delivered && target.cancelled {
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
            view.status = "Paste cancelled: editor target changed while reading clipboard".into();
        }
        // Keep cancellation until delivery, even if focus/caret later returns.
        // A valid delivery remains guarded through native command dispatch.
        view.editor_paste_target = (!(delivered && target.cancelled)).then_some(target);
    }
}

/// Final check after commands may have changed the active document or focus.
pub(crate) fn finish_paste_validation(
    view: &mut ViewState,
    active: Option<&Document>,
    ctx: &egui::Context,
) {
    validate_paste(view, active, ctx);
    if ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
    }) {
        view.editor_paste_target = None;
    }
}

pub(crate) fn prepare_action(view: &mut ViewState, document: &Document, ctx: &egui::Context) {
    if let Some(pending) = view.pending_editor_action.take()
        && pending.document == document.id
        && !blocked(view)
    {
        view.explorer_focused = false;
        if matches!(pending.action, EditorAction::Copy | EditorAction::Cut) {
            view.text_clipboard.clear();
        }
        ctx.memory_mut(|memory| memory.request_focus(text_widget_id(document.id)));
        if pending.action == EditorAction::Paste {
            request_paste(view, document, ctx);
        } else {
            pending.action.send(ctx);
        }
    }
}

impl EditorAction {
    /// The widget remains the sole mutation/history owner during migration.
    pub(crate) fn send(self, ctx: &egui::Context) {
        match self {
            Self::Copy | Self::Cut => ctx.input_mut(|input| {
                let event = if self == Self::Copy {
                    egui::Event::Copy
                } else {
                    egui::Event::Cut
                };
                // Native accelerators can accompany the window's clipboard
                // event. Inject once; do not turn one Cut into two queued cuts.
                if !input.events.contains(&event) {
                    input.events.push(event);
                }
            }),
            Self::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Self::Undo | Self::Redo | Self::SelectAll => ctx.input_mut(|input| {
                let key = if self == Self::SelectAll {
                    egui::Key::A
                } else {
                    egui::Key::Z
                };
                if input.events.iter().any(|event| {
                    matches!(event,egui::Event::Key {key:existing,pressed:true,modifiers,..}
                    if *existing==key && modifiers.command && modifiers.shift==(self==Self::Redo))
                }) {
                    return;
                }
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
