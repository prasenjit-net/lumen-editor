//! Ordered keyboard batches while TextEdit remains the mutation/history owner.
use super::{
    editor_action::{blocked, text_widget_id},
    view_state::ViewState,
};
use crate::model::{Document, DocumentId};
use eframe::egui;

pub(crate) struct PendingTextInput {
    document: DocumentId,
    revision: u64,
    cursor: egui::text::CCursorRange,
    events: Vec<egui::Event>,
}

fn keyboard_event(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Text(_)
            | egui::Event::Key { .. }
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_)
    )
}

fn clipboard_event(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
    )
}

/// Deliver a keyboard prefix or one clipboard action, never both. The next
/// repaint then computes line rules from the widget's updated caret and text.
pub(crate) fn prepare_input(view: &mut ViewState, document: &Document, ctx: &egui::Context) {
    let id = text_widget_id(document.id);
    let cursor =
        egui::text_edit::TextEditState::load(ctx, id).and_then(|widget| widget.cursor.char_range());
    let interrupted = blocked(view)
        || !ctx.memory(|memory| memory.has_focus(id))
        || ctx.input(|input| {
            input.pointer.any_pressed()
                || input.pointer.any_released()
                || input.pointer.any_down()
                || input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Ime(_)))
        });
    let Some(cursor) = cursor.filter(|_| !interrupted) else {
        cancel_input(view);
        return;
    };
    let pending = view.pending_text_input.take();
    let mut events = if let Some(pending) = pending {
        if pending.document == document.id
            && pending.revision == document.revision
            && pending.cursor == cursor
        {
            pending.events
        } else {
            view.status = "Pending editor input cancelled: document or caret changed".into();
            Vec::new()
        }
    } else {
        Vec::new()
    };
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if keyboard_event(event) {
                events.push(event.clone());
                false
            } else {
                true
            }
        });
        let count = match events.iter().position(clipboard_event) {
            Some(0) => 1,
            Some(index) => index,
            None => events.len(),
        };
        input.events.extend(events.drain(..count));
    });
    if !events.is_empty() {
        view.pending_text_input = Some(PendingTextInput {
            document: document.id,
            revision: document.revision,
            cursor,
            events,
        });
        ctx.request_repaint();
    }
}

/// Record the widget's actual result, not a predicted selection or mutation.
pub(crate) fn finish_input(view: &mut ViewState, document: &Document, ctx: &egui::Context) {
    if let Some(pending) = view.pending_text_input.as_mut()
        && pending.document == document.id
        && let Some(cursor) = egui::text_edit::TextEditState::load(ctx, text_widget_id(document.id))
            .and_then(|widget| widget.cursor.char_range())
    {
        pending.revision = document.revision;
        pending.cursor = cursor;
    }
}

pub(crate) fn cancel_input(view: &mut ViewState) {
    if view.pending_text_input.take().is_some() {
        view.status = "Pending editor input cancelled: focus or input mode changed".into();
    }
}
