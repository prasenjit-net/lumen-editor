use super::LumenApp;
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
        // A Search result opens on pointer release. Only defer replacement input;
        // the result selection must reach the editor in that same frame.
        let pointer_active =
            ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_released());
        if pointer_active && self.pending.is_some() {
            ctx.request_repaint();
        }
        if let Some(edit) = self.pending.take_if(|_| !pointer_active) {
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

impl LumenApp {
    pub(crate) fn open_find(&mut self, replace: bool, ctx: &egui::Context) {
        if self.view.command_palette
            || self.view.pending_close.is_some()
            || self.view.pending_quit
            || self.view.delete_prompt.is_some()
            || self.view.explorer_edit.is_some()
            || self
                .editor
                .active()
                .is_some_and(|file| file.is_image() || file.is_view_tab())
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
