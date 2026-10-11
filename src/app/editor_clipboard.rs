//! Expand empty-selection clipboard actions without taking ownership of edits.
use super::{
    editor_action::{blocked, text_widget_id},
    view_state::ViewState,
};
use crate::model::{
    Document,
    text_line::{line_clipboard, line_paste},
};
use eframe::egui;

/// Explicit session-only provenance, not a heuristic based on a trailing newline.
/// egui cannot carry native custom MIME metadata, so exact text matching is our
/// fallback. Identical text copied outside Lumen is inherently indistinguishable.
#[derive(Default)]
pub(crate) struct TextClipboard {
    copied_line: Option<String>,
}

impl TextClipboard {
    pub(crate) fn clear(&mut self) {
        self.copied_line = None;
    }

    pub(crate) fn observe_input(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Copy | egui::Event::Cut => self.clear(),
                    egui::Event::Paste(text) if self.copied_line.as_ref() != Some(text) => {
                        self.clear()
                    }
                    _ => {}
                }
            }
        });
    }
}

pub(crate) enum ClipboardEdit {
    Cut,
    Paste { caret_after: usize },
}

/// Called immediately before the document widget consumes input. Copy does not
/// move the caret; Cut selects the deletion range and lets TextEdit delete it.
/// Returns an edit requiring a closing widget undo boundary.
pub(crate) fn prepare_clipboard(
    view: &mut ViewState,
    document: &Document,
    ctx: &egui::Context,
) -> Option<ClipboardEdit> {
    let id = text_widget_id(document.id);
    if blocked(view) || !ctx.memory(|memory| memory.has_focus(id)) {
        return None;
    }
    let mut widget = egui::text_edit::TextEditState::load(ctx, id)?;
    let cursor = widget
        .cursor
        .char_range()
        .filter(|cursor| cursor.is_empty())?;
    // Never guess a caret that a preceding event or pointer gesture will change.
    // Such mixed-input frames retain the widget's ordinary clipboard semantics.
    let event = ctx.input(|input| {
        if input.pointer.any_pressed() || input.pointer.any_released() {
            return None;
        }
        for (index, event) in input.events.iter().enumerate() {
            match event {
                egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => {
                    return Some((index, event.clone()));
                }
                egui::Event::Text(_) | egui::Event::Ime(_) => return None,
                egui::Event::Key {
                    key, pressed: true, ..
                } if !matches!(key, egui::Key::C | egui::Key::X | egui::Key::V) => return None,
                _ => {}
            }
        }
        None
    });
    let (index, event) = event?;
    if let egui::Event::Paste(text) = event {
        if view.text_clipboard.copied_line.as_ref() != Some(&text) {
            return None;
        }
        // Restoring the caret after widget input must not override later input.
        let isolated = ctx.input(|input| {
            input.events[index + 1..].iter().all(|event| {
                !matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Ime(_)
                        | egui::Event::Key { pressed: true, .. }
                )
            })
        });
        if !isolated {
            return None;
        }
        let paste = line_paste(&document.text, cursor.primary.index, &text)?;
        let mut history = widget.undoer();
        history.add_undo(&(cursor, document.text.clone()));
        widget.set_undoer(history);
        widget
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(paste.insertion),
            )));
        widget.store(ctx, id);
        return Some(ClipboardEdit::Paste {
            caret_after: paste.caret_after,
        });
    }
    view.text_clipboard.clear();
    let line = line_clipboard(&document.text, cursor.primary.index)?;
    view.text_clipboard.copied_line = Some(line.text.clone());
    ctx.copy_text(line.text);
    if matches!(event, egui::Event::Copy) || line.deletion.is_empty() {
        ctx.input_mut(|input| {
            input.events.remove(index);
        });
        return None;
    }
    let mut history = widget.undoer();
    history.add_undo(&(cursor, document.text.clone()));
    widget.set_undoer(history);
    widget
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(line.deletion.start),
            egui::text::CCursor::new(line.deletion.end),
        )));
    widget.store(ctx, id);
    // Cut's clipboard text may differ from its deletion range at EOF. Delete
    // through the widget without overwriting the clipboard with that range.
    ctx.input_mut(|input| {
        input.events[index] = egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    });
    Some(ClipboardEdit::Cut)
}

pub(crate) fn finish_clipboard(
    output: &mut egui::text_edit::TextEditOutput,
    document: &Document,
    ctx: &egui::Context,
    edit: &ClipboardEdit,
) {
    let mut widget = output.state.clone();
    if let ClipboardEdit::Paste { caret_after } = edit {
        let cursor = egui::text::CCursorRange::one(egui::text::CCursor::new(*caret_after));
        widget.cursor.set_char_range(Some(cursor));
        output.cursor_range = Some(cursor);
        ctx.request_repaint();
    }
    let mut history = widget.undoer();
    history.add_undo(&(
        widget.cursor.char_range().unwrap_or_default(),
        document.text.clone(),
    ));
    widget.set_undoer(history);
    widget.clone().store(ctx, text_widget_id(document.id));
    output.state = widget;
}
