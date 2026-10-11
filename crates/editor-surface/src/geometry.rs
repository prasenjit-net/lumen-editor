use egui::{FontId, Galley, Pos2, Rect, Ui, Vec2, text::CCursor};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// One source-to-layout mapping for painting and pointer/caret geometry.
/// CRLF projects to one visual newline without altering the stored document.
pub struct TextGeometry {
    pub galley: Arc<Galley>,
    pub origin: Pos2,
    byte_offsets: Vec<usize>,
    graphemes: Vec<usize>,
}

impl TextGeometry {
    pub fn layout(ui: &Ui, text: &str, origin: Pos2) -> Self {
        let mut visual = String::new();
        let mut byte_offsets = vec![0];
        let mut characters = text.char_indices().peekable();
        while let Some((offset, character)) = characters.next() {
            if character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n') {
                characters.next();
                visual.push('\n');
                byte_offsets.push(offset + 2);
            } else {
                visual.push(character);
                byte_offsets.push(offset + character.len_utf8());
            }
        }
        let galley = ui.fonts_mut(|fonts| {
            fonts.layout_no_wrap(visual, FontId::monospace(14.0), ui.visuals().text_color())
        });
        Self {
            galley,
            origin,
            byte_offsets,
            graphemes: text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .chain(std::iter::once(text.len()))
                .collect(),
        }
    }

    pub fn cursor(&self, byte: usize) -> CCursor {
        CCursor::new(
            self.byte_offsets
                .binary_search(&byte)
                .expect("validated source boundary"),
        )
    }

    pub fn caret_rect(&self, byte: usize) -> Rect {
        self.galley
            .pos_from_cursor(self.cursor(byte))
            .translate(self.origin.to_vec2())
    }

    pub fn hit_test(&self, pointer: Pos2) -> usize {
        let cursor = self.galley.cursor_from_pos(pointer - self.origin);
        let byte = self.byte_offsets[cursor.index.min(self.byte_offsets.len() - 1)];
        match self.graphemes.binary_search(&byte) {
            Ok(_) => byte,
            Err(index) => {
                let before = self.graphemes[index.saturating_sub(1)];
                let after = self.graphemes[index.min(self.graphemes.len() - 1)];
                if self.caret_rect(before).center().distance_sq(pointer)
                    < self.caret_rect(after).center().distance_sq(pointer)
                {
                    before
                } else {
                    after
                }
            }
        }
    }

    pub fn size(&self) -> Vec2 {
        self.galley.size()
    }

    pub(crate) fn accessibility_byte(
        &self,
        widget: egui::Id,
        position: &egui::accesskit::TextPosition,
    ) -> Option<usize> {
        let mut index = 0;
        for (row_index, row) in self.galley.rows.iter().enumerate() {
            let count = row.char_count_including_newline();
            if egui::accesskit::NodeId(widget.with(row_index).value()) == position.node {
                if position.character_index > count {
                    return None;
                }
                let byte = *self.byte_offsets.get(index + position.character_index)?;
                return self.graphemes.binary_search(&byte).is_ok().then_some(byte);
            }
            index += count;
        }
        None
    }
}
