use super::{LumenApp, editor_action::text_widget_id};
use crate::model::{
    Document, DocumentId,
    search::{SearchOptions, find_matches, replace_matches},
};
use eframe::egui;
use std::ops::Range;

#[derive(Default)]
pub(crate) struct FindState {
    pub(crate) open: bool,
    pub(crate) replace_visible: bool,
    pub(crate) query: String,
    pub(crate) replacement: String,
    pub(crate) options: SearchOptions,
    pub(crate) matches: Vec<Range<usize>>,
    pub(crate) character_matches: Vec<Range<usize>>,
    pub(crate) current: usize,
    pub(crate) focus: bool,
    pub(crate) reveal: bool,
    searched: Option<(u64, String, SearchOptions)>,
    pending: Option<FindEdit>,
}

struct FindEdit {
    revision: u64,
    range: Range<usize>,
    replacement: String,
}

impl FindState {
    #[cfg(feature = "shared-editor")]
    pub(crate) fn prepare_shared(
        &mut self,
        document: &mut Document,
        ctx: &egui::Context,
        blocked: bool,
    ) -> Option<String> {
        if blocked {
            self.pending = None;
            return None;
        }
        if ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_released()) {
            if self.pending.is_some() {
                ctx.request_repaint();
            }
            return None;
        }
        if let Some(edit) = self.pending.take() {
            if edit.revision != document.revision {
                return Some("Replace cancelled: document changed".into());
            }
            if let Some(buffer) = document.editing.as_mut() {
                if buffer
                    .replace_ranges(
                        vec![lumen_editor_core::Selection {
                            anchor: edit.range.start,
                            head: edit.range.end,
                        }],
                        0,
                        &[&edit.replacement],
                    )
                    .is_err()
                {
                    return Some("Replace cancelled: range splits a grapheme".into());
                }
                document.sync_shared_editing();
                ctx.memory_mut(|memory| memory.request_focus(find_input_id(document.id)));
            }
        }
        if self.reveal
            && !self.open
            && let Some(range) = self.matches.get(self.current)
            && let Some(buffer) = document.editing.as_mut()
        {
            let _ = buffer.set_selections(
                vec![lumen_editor_core::Selection {
                    anchor: range.start,
                    head: range.end,
                }],
                0,
            );
        }
        None
    }
    pub(crate) fn refresh(&mut self, document: &Document) {
        let key = (document.revision, self.query.clone(), self.options);
        if self.searched.as_ref() != Some(&key) {
            self.matches = find_matches(&document.text, &self.query, self.options);
            let mut byte_offset = 0;
            let mut character_offset = 0;
            self.character_matches = self
                .matches
                .iter()
                .map(|range| {
                    character_offset += document.text[byte_offset..range.start].chars().count();
                    let start = character_offset;
                    character_offset += document.text[range.clone()].chars().count();
                    byte_offset = range.end;
                    start..character_offset
                })
                .collect();
            self.current = self.current.min(self.matches.len().saturating_sub(1));
            self.searched = Some(key);
            self.reveal = self.open;
        }
    }

    pub(crate) fn navigate(&mut self, previous: bool) {
        if !self.matches.is_empty() {
            self.current = if previous {
                (self.current + self.matches.len() - 1) % self.matches.len()
            } else {
                (self.current + 1) % self.matches.len()
            };
            self.reveal = true;
        }
    }

    pub(crate) fn queue_replace(&mut self, document: &Document, all: bool) {
        self.refresh(document);
        let Some(range) = self.matches.get(self.current).cloned() else {
            return;
        };
        let (range, replacement) = if all {
            let Some(text) = replace_matches(&document.text, &self.matches, &self.replacement)
            else {
                return;
            };
            (0..document.text.len(), text)
        } else {
            (range, self.replacement.clone())
        };
        if document.text.get(range.clone()) == Some(replacement.as_str()) {
            return;
        }
        self.pending = Some(FindEdit {
            revision: document.revision,
            range,
            replacement,
        });
    }
}

pub(crate) fn find_input_id(document: DocumentId) -> egui::Id {
    egui::Id::new(("editor_find", document))
}

pub(crate) fn prepare_find(
    state: &mut FindState,
    document: &Document,
    ctx: &egui::Context,
    blocked: bool,
) -> bool {
    if blocked {
        state.pending = None;
        return false;
    }
    // A button-release frame surrenders TextEdit focus as an outside click.
    // Deliver replacement on the next frame, just like popup edit commands.
    if state.pending.is_some()
        && ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_released())
    {
        ctx.request_repaint();
        return false;
    }
    let id = text_widget_id(document.id);
    let Some(mut widget) = egui::text_edit::TextEditState::load(ctx, id) else {
        return false;
    };
    if let Some(edit) = state.pending.take() {
        if edit.revision != document.revision || document.text.get(edit.range.clone()).is_none() {
            return false;
        }
        let before = widget.cursor.char_range().unwrap_or_default();
        let mut history = widget.undoer();
        history.add_undo(&(before, document.text.clone()));
        widget.set_undoer(history);
        widget
            .cursor
            .set_char_range(Some(character_range(&document.text, &edit.range)));
        widget.store(ctx, id);
        ctx.memory_mut(|memory| memory.request_focus(id));
        ctx.input_mut(|input| input.events.push(egui::Event::Paste(edit.replacement)));
        return true;
    }
    if state.reveal {
        state.reveal = false;
        // While the find input owns focus, highlights are a preview, not edits
        // to TextEdit's selection/history. Adopt the result when Find closes.
        if !state.open
            && let Some(range) = state.matches.get(state.current)
        {
            widget
                .cursor
                .set_char_range(Some(character_range(&document.text, range)));
            widget.store(ctx, id);
        }
    }
    false
}

pub(crate) fn finish_replace(
    output: &egui::text_edit::TextEditOutput,
    document: &Document,
    ctx: &egui::Context,
) {
    let mut widget = output.state.clone();
    let mut history = widget.undoer();
    history.add_undo(&(
        widget.cursor.char_range().unwrap_or_default(),
        document.text.clone(),
    ));
    widget.set_undoer(history);
    widget.store(ctx, text_widget_id(document.id));
    ctx.memory_mut(|memory| memory.request_focus(find_input_id(document.id)));
}

fn character_range(text: &str, range: &Range<usize>) -> egui::text::CCursorRange {
    egui::text::CCursorRange::two(
        egui::text::CCursor::new(text[..range.start].chars().count()),
        egui::text::CCursor::new(text[..range.end].chars().count()),
    )
}

impl LumenApp {
    pub(crate) fn open_find(&mut self, replace: bool, ctx: &egui::Context) {
        if self.view.command_palette
            || self.view.pending_close.is_some()
            || self.view.pending_quit
            || self.view.delete_prompt.is_some()
            || self.view.explorer_edit.is_some()
        {
            return;
        }
        let Some(document) = self.editor.active() else {
            return;
        };
        let state = self.view.find.entry(document.id).or_default();
        let selection = super::editor_action::current_selection(document, ctx);
        if let Some(selection) = selection.filter(|selection| !selection.is_empty()) {
            let [start, end] = selection.sorted_cursors();
            state.query = document
                .text
                .chars()
                .skip(start.index)
                .take(end.index - start.index)
                .collect();
        } else if let Some(selection) = selection {
            let characters: Vec<_> = document.text.chars().collect();
            let word = |character: &char| character.is_alphanumeric() || *character == '_';
            let mut start = selection.primary.index.min(characters.len());
            let mut end = start;
            while start > 0 && word(&characters[start - 1]) {
                start -= 1;
            }
            while end < characters.len() && word(&characters[end]) {
                end += 1;
            }
            if start < end {
                state.query = characters[start..end].iter().collect();
            }
        }
        state.open = true;
        state.replace_visible |= replace;
        state.focus = true;
        state.refresh(document);
        self.view.explorer_focused = false;
    }
}
