use super::theme::{BG, MUTED};
use crate::{app::editor_action::text_widget_id, model::DocumentId};
use eframe::egui::{self, FontId};
use std::ops::Range;

pub(super) fn gutter_id(document: DocumentId) -> egui::Id {
    egui::Id::new(("editor_gutter", document))
}

pub(super) fn show_gutter(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    gutter: egui::Rect,
    font: &FontId,
    document: DocumentId,
) {
    let painter = ui.painter_at(gutter.intersect(ui.clip_rect()));
    painter.rect_filled(gutter, 0, BG);
    for (index, row) in output.galley.rows.iter().enumerate() {
        let y = output.galley_pos.y + (row.min_y() + row.max_y()) * 0.5;
        if y >= gutter.top() - row.size.y && y <= gutter.bottom() + row.size.y {
            painter.text(
                egui::pos2(gutter.right() - 10.0, y),
                egui::Align2::RIGHT_CENTER,
                (index + 1).to_string(),
                font.clone(),
                MUTED,
            );
        }
    }
    let id = gutter_id(document);
    let response = ui
        .interact(
            gutter.intersect(ui.clip_rect()),
            id,
            egui::Sense::click_and_drag(),
        )
        .on_hover_text("Select line; Shift-click to extend");
    let start = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
    if !start && !response.dragged_by(egui::PointerButton::Primary) {
        return;
    }
    let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
        return;
    };
    let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
    let current = line_at(output, cursor.index);
    let anchor = if start {
        let shift = ui.input(|input| input.modifiers.shift);
        let anchor = if shift {
            let character = output
                .state
                .cursor
                .char_range()
                .unwrap_or_default()
                .secondary
                .index;
            // A reversed line selection anchors at the line's end, which is
            // also the next line's start. Keep the original gutter anchor.
            ui.ctx()
                .data(|data| data.get_temp::<Range<usize>>(id))
                .filter(|range| character == range.start || character == range.end)
                .unwrap_or_else(|| line_at(output, character))
        } else {
            current.clone()
        };
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, anchor.clone()));
        anchor
    } else {
        ui.ctx()
            .data(|data| data.get_temp::<Range<usize>>(id))
            .unwrap_or_else(|| current.clone())
    };
    let length = output.galley.end().index;
    let anchor = anchor.start.min(length)..anchor.end.min(length);
    let (anchor, caret) = if current.start < anchor.start {
        (anchor.end, current.start)
    } else {
        (anchor.start, current.end)
    };
    let mut state = output.state.clone();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(anchor),
            egui::text::CCursor::new(caret),
        )));
    state.store(ui.ctx(), text_widget_id(document));
    ui.memory_mut(|memory| memory.request_focus(text_widget_id(document)));
    ui.ctx().request_repaint();
}

fn line_at(output: &egui::text_edit::TextEditOutput, character: usize) -> Range<usize> {
    let mut start = 0;
    for row in &output.galley.rows {
        let end = start + row.char_count_including_newline();
        if character < end {
            return start..end;
        }
        start = end;
    }
    // The empty final line (including an empty document) has a legal EOF caret.
    start..start
}
