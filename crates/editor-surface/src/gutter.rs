use crate::{EditorSurface, TextGeometry};
use egui::{Id, PointerButton, Rect, Sense, Ui};
use lumen_editor_core::{EditorBuffer, Selection};
use std::ops::Range;

impl EditorSurface {
    pub fn set_gutter_id(&mut self, id: Id) {
        self.gutter_id = Some(id);
    }

    pub(crate) fn gutter_input(
        &mut self,
        ui: &Ui,
        id: Id,
        gutter: Rect,
        geometry: &TextGeometry,
        buffer: &mut EditorBuffer,
    ) {
        let fold_at_pointer = ui
            .input(|input| input.pointer.hover_pos())
            .and_then(|pointer| {
                if pointer.x < gutter.left() + 2.0 || pointer.x >= gutter.left() + 20.0 {
                    return None;
                }
                let row = geometry
                    .row_rects()
                    .iter()
                    .position(|rect| rect.top() <= pointer.y && pointer.y < rect.bottom())?;
                self.fold_ranges
                    .iter()
                    .find(|range| range.header + 1 == geometry.source_line_for_row(row))
                    .copied()
            });
        let response = ui
            .interact(
                gutter.intersect(ui.clip_rect()),
                self.gutter_id.unwrap_or(id.with("gutter")),
                Sense::click_and_drag(),
            )
            .on_hover_text(if let Some(range) = fold_at_pointer {
                if self.collapsed.contains(&range.header) {
                    "Expand folded lines"
                } else {
                    "Collapse lines"
                }
            } else {
                "Select line; Shift-click to extend"
            });
        if fold_at_pointer.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if !ui.is_enabled() || !ui.input(|input| input.focused) {
            self.gutter_drag = None;
            if ui.ctx().is_being_dragged(response.id) {
                ui.ctx().stop_dragging();
            }
            return;
        }
        let start = response.hovered() && ui.input(|input| input.pointer.primary_pressed());
        if start && let Some(range) = fold_at_pointer {
            self.gutter_drag = None;
            self.toggle_fold(range.header, buffer);
            ui.memory_mut(|memory| memory.request_focus(id));
            ui.ctx().request_repaint();
            return;
        }
        if !start && !response.dragged_by(PointerButton::Primary) {
            if !ui.input(|input| input.pointer.primary_down()) {
                self.gutter_drag = None;
            }
            return;
        }
        if self
            .gutter_drag
            .as_ref()
            .is_some_and(|(_, revision)| *revision != buffer.revision())
        {
            self.gutter_drag = None;
            return;
        }
        let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
            return;
        };
        self.cancel_composition();
        let byte = geometry.hit_test(egui::pos2(
            geometry.origin.x,
            pointer.y.clamp(gutter.top(), gutter.bottom()),
        ));
        let current = line_at(buffer.text(), byte);
        let anchor = if start {
            let anchor = if ui.input(|input| input.modifiers.shift) {
                self.gutter_drag
                    .as_ref()
                    .map(|(range, _)| range.clone())
                    .unwrap_or_else(|| {
                        let set = buffer.selections();
                        line_at(buffer.text(), set.selections()[set.primary_index()].anchor)
                    })
            } else {
                current.clone()
            };
            self.gutter_drag = Some((anchor.clone(), buffer.revision()));
            anchor
        } else {
            self.gutter_drag
                .as_ref()
                .map(|(range, _)| range.clone())
                .unwrap_or_else(|| current.clone())
        };
        let selection = if current.start < anchor.start {
            Selection {
                anchor: anchor.end,
                head: current.start,
            }
        } else {
            Selection {
                anchor: anchor.start,
                head: current.end,
            }
        };
        if buffer.set_selections(vec![selection], 0).is_err() {
            self.gutter_drag = None;
            return;
        }
        ui.memory_mut(|memory| memory.request_focus(id));
        if response.dragged_by(PointerButton::Primary) {
            let distance = if pointer.y < gutter.top() {
                pointer.y - gutter.top()
            } else if pointer.y > gutter.bottom() {
                pointer.y - gutter.bottom()
            } else {
                0.0
            };
            let elapsed = ui.input(|input| input.stable_dt).clamp(0.0, 0.05);
            let delta = distance.signum() * (80.0 + distance.abs() * 8.0).min(800.0) * elapsed;
            if distance != 0.0 {
                ui.scroll_with_delta_animation(
                    egui::vec2(0.0, -delta),
                    egui::style::ScrollAnimation::none(),
                );
            }
        }
        ui.ctx().request_repaint();
    }
}

fn line_at(text: &str, byte: usize) -> Range<usize> {
    let start = text[..byte].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[byte..]
        .find('\n')
        .map_or(text.len(), |offset| byte + offset + 1);
    start..end
}
