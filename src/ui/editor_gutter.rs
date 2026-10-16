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
    interaction_enabled: bool,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let painter = ui.painter_at(gutter.intersect(ui.clip_rect()));
    painter.rect_filled(gutter, 0, colors.bg);
    for (index, row) in output.galley.rows.iter().enumerate() {
        let y = output.galley_pos.y + (row.min_y() + row.max_y()) * 0.5;
        if y >= gutter.top() - row.size.y && y <= gutter.bottom() + row.size.y {
            painter.text(
                egui::pos2(gutter.right() - 10.0, y),
                egui::Align2::RIGHT_CENTER,
                (index + 1).to_string(),
                font.clone(),
                colors.muted,
            );
        }
    }
    let id = gutter_id(document);
    if !interaction_enabled {
        if ui.ctx().is_being_dragged(id) {
            ui.ctx().stop_dragging();
        }
        return;
    }
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
    // Select the visible edge line, then keep extending as the shared viewport
    // scrolls. An offscreen pointer must not jump directly to an unseen row.
    let position = egui::pos2(pointer.x, pointer.y.clamp(gutter.top(), gutter.bottom()));
    let cursor = output.galley.cursor_from_pos(position - output.galley_pos);
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
    if response.dragged_by(egui::PointerButton::Primary) {
        let delta = ui.input(|input| edge_scroll_delta(pointer.y, gutter, input.stable_dt));
        if delta != 0.0 {
            ui.scroll_with_delta_animation(
                egui::vec2(0.0, -delta),
                egui::style::ScrollAnimation::none(),
            );
        }
    }
    ui.ctx().request_repaint();
}

/// Speed grows with distance beyond the edge, with a bounded frame interval
/// so a stalled frame cannot cause a large jump. Inside the viewport is idle.
fn edge_scroll_delta(pointer_y: f32, viewport: egui::Rect, elapsed: f32) -> f32 {
    let distance = if pointer_y < viewport.top() {
        pointer_y - viewport.top()
    } else if pointer_y > viewport.bottom() {
        pointer_y - viewport.bottom()
    } else {
        return 0.0;
    };
    let speed = (80.0 + distance.abs() * 8.0).min(800.0);
    distance.signum() * speed * elapsed.clamp(0.0, 0.05)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_scroll_is_directional_bounded_and_frame_rate_independent() {
        let viewport = egui::Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(50.0, 110.0));
        assert_eq!(edge_scroll_delta(60.0, viewport, 0.02), 0.0);
        assert_eq!(edge_scroll_delta(10.0, viewport, 0.02), 0.0);
        assert_eq!(edge_scroll_delta(110.0, viewport, 0.02), 0.0);
        assert!(edge_scroll_delta(0.0, viewport, 0.02) < 0.0);
        assert!(edge_scroll_delta(120.0, viewport, 0.02) > 0.0);
        assert_eq!(
            edge_scroll_delta(130.0, viewport, 0.02),
            2.0 * edge_scroll_delta(130.0, viewport, 0.01)
        );
        assert_eq!(edge_scroll_delta(10000.0, viewport, 10.0), 40.0);
        assert_eq!(edge_scroll_delta(-10000.0, viewport, -1.0), 0.0);
    }
}
