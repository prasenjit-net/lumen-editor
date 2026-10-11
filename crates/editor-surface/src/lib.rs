//! Runnable backend spike, intentionally not linked into the Lumen workbench.
mod accessibility;
mod commands;
mod composition_ui;
mod geometry;
mod gutter;
mod input;
#[cfg(feature = "shaped")]
mod shaping;

use egui::{Align2, FontId, Id, Pos2, Rect, Response, ScrollArea, Sense, Stroke, Ui, vec2};
pub use geometry::TextGeometry;
use lumen_editor_core::{Composition, EditorBuffer, SelectionClipboard};
#[cfg(feature = "shaped")]
pub use shaping::Shaper;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct EditorSurface {
    widget: Option<Id>,
    drag_anchor: Option<DragSelection>,
    composition: Option<Composition>,
    composition_cancelled: bool,
    clipboard: Option<SelectionClipboard>,
    vertical_goal: Option<(usize, f32)>,
    reveal: Option<usize>,
    gutter_id: Option<Id>,
    gutter_drag: Option<(std::ops::Range<usize>, u64)>,
    pub notice: Option<String>,
    #[cfg(feature = "shaped")]
    shaper: Option<Shaper>,
}

#[derive(Clone, Copy)]
struct DragSelection {
    anchor: usize,
    revision: u64,
}

pub struct SurfaceOutput {
    pub response: Response,
    pub geometry: TextGeometry,
    pub text_clip: Rect,
    pub gutter: Rect,
    pub carets: Vec<Rect>,
    pub ime_cursor: Option<Rect>,
}

impl EditorSurface {
    pub fn clear_clipboard(&mut self) {
        self.clipboard = None;
    }
    pub fn reveal(&mut self, byte: usize) {
        self.reveal = Some(byte);
    }
    #[cfg(feature = "shaped")]
    pub fn with_shaper(shaper: Shaper) -> Self {
        Self {
            shaper: Some(shaper),
            ..Self::default()
        }
    }

    pub(crate) fn geometry(
        &self,
        ui: &Ui,
        text: &str,
        origin: Pos2,
        composition: Option<&Composition>,
    ) -> TextGeometry {
        let geometry = TextGeometry::layout_composition(ui, text, origin, composition);
        #[cfg(feature = "shaped")]
        let geometry = {
            let mut geometry = geometry;
            if let Some(shaper) = &self.shaper {
                geometry.shape(shaper);
            }
            geometry
        };
        geometry
    }
    pub fn show(&mut self, ui: &mut Ui, id: Id, buffer: &mut EditorBuffer) -> SurfaceOutput {
        if self.widget.is_some_and(|widget| widget != id) {
            self.composition = None;
            self.composition_cancelled = true;
            self.drag_anchor = None;
            self.vertical_goal = None;
            self.gutter_drag = None;
            self.notice = Some("Composition cancelled: editor identity changed".into());
        }
        self.widget = Some(id);
        let viewport_origin = ui.cursor().min;
        ui.spacing_mut().scroll.floating = false;
        ScrollArea::both()
            .id_salt(id.with("scroll"))
            .animated(false)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let initial = self.geometry(
                    ui,
                    buffer.text(),
                    Pos2::ZERO,
                    self.composition
                        .as_ref()
                        .filter(|composition| composition.is_current(buffer)),
                );
                let gutter_width =
                    ui.fonts_mut(|fonts| {
                        fonts.layout_no_wrap(
                            initial.row_count().to_string(),
                            FontId::monospace(14.0),
                            ui.visuals().weak_text_color(),
                        )
                    })
                    .size()
                    .x + 24.0;
                let (_, content) = ui.allocate_space(vec2(
                    (initial.size().x + gutter_width + 16.0).max(viewport.width()),
                    (initial.size().y + 8.0).max(viewport.height()),
                ));
                let gutter =
                    Rect::from_min_size(viewport_origin, vec2(gutter_width, viewport.height()));
                let text_clip = ui.clip_rect().intersect(Rect::from_min_max(
                    Pos2::new(gutter.right(), ui.clip_rect().top()),
                    ui.clip_rect().max,
                ));
                let origin = content.min + vec2(gutter_width + 8.0, 4.0);
                let mut response =
                    ui.interact(content.intersect(text_clip), id, Sense::click_and_drag());
                let before_selection = buffer.selections().clone();
                let before_preedit = self
                    .composition
                    .as_ref()
                    .map(|composition| composition.preedit().to_owned());
                if self.handle_input(ui, &response, text_clip, origin, buffer) {
                    response.mark_changed();
                    ui.ctx().request_repaint();
                }
                let geometry = self.geometry(ui, buffer.text(), origin, self.composition.as_ref());
                let painter = ui.painter().with_clip_rect(text_clip);
                for (index, selection) in buffer.selections().selections().iter().enumerate() {
                    if geometry.preedit_rects().is_some()
                        && index == buffer.selections().primary_index()
                    {
                        continue;
                    }
                    for rect in geometry.selection_rects(selection.range()) {
                        painter.rect_filled(rect, 0, ui.visuals().selection.bg_fill);
                    }
                }
                geometry.paint(ui, &painter);
                let carets: Vec<_> = buffer
                    .selections()
                    .selections()
                    .iter()
                    .map(|selection| geometry.caret_rect(selection.head))
                    .collect();
                accessibility::publish(ui, &response, &geometry, buffer);
                if let Some(byte) = self.reveal.take().filter(|byte| {
                    *byte <= buffer.text().len() && buffer.text().is_char_boundary(*byte)
                }) {
                    // Find can point inside a grapheme; reveal its nearest legal boundary.
                    let byte = buffer
                        .text()
                        .grapheme_indices(true)
                        .map(|(offset, _)| offset)
                        .chain([buffer.text().len()])
                        .rfind(|offset| *offset <= byte)
                        .unwrap_or(0);
                    ui.scroll_to_rect(geometry.caret_rect(byte), None);
                }
                let mut ime_cursor = None;
                if response.changed()
                    || before_selection != *buffer.selections()
                    || before_preedit.as_deref()
                        != self.composition.as_ref().map(Composition::preedit)
                {
                    ui.scroll_to_rect(
                        geometry
                            .preedit_rects()
                            .map_or(carets[buffer.selections().primary_index()], |(_, end)| end)
                            .expand(2.0),
                        None,
                    );
                }
                if response.has_focus() && ui.is_enabled() {
                    ui.memory_mut(|memory| {
                        memory.set_focus_lock_filter(
                            id,
                            egui::EventFilter {
                                escape: true,
                                horizontal_arrows: true,
                                vertical_arrows: true,
                                tab: true,
                            },
                        )
                    });
                    for (index, caret) in carets.iter().enumerate() {
                        if geometry.preedit_rects().is_some()
                            && index == buffer.selections().primary_index()
                        {
                            continue;
                        }
                        let width: f32 = if index == buffer.selections().primary_index() {
                            2.0
                        } else {
                            1.0
                        };
                        painter.line_segment(
                            [caret.left_top(), caret.left_bottom()],
                            Stroke::new(
                                width,
                                if self.composition.is_some() {
                                    ui.visuals().weak_text_color()
                                } else {
                                    ui.visuals().text_color()
                                },
                            ),
                        );
                    }
                    let cursor = self
                        .paint_preedit(ui, &geometry, text_clip)
                        .unwrap_or(carets[buffer.selections().primary_index()]);
                    let transform = ui
                        .ctx()
                        .layer_transform_to_global(ui.layer_id())
                        .unwrap_or_default();
                    ui.output_mut(|output| {
                        output.ime =
                            text_clip
                                .intersects(cursor)
                                .then_some(egui::output::IMEOutput {
                                    rect: transform * text_clip,
                                    cursor_rect: transform * cursor,
                                });
                        output.mutable_text_under_cursor = response.hovered();
                    });
                    ime_cursor = text_clip.intersects(cursor).then_some(cursor);
                }
                let gutter_painter = ui
                    .painter()
                    .with_clip_rect(gutter.intersect(ui.clip_rect()));
                gutter_painter.rect_filled(gutter, 0, ui.visuals().panel_fill);
                for (index, row) in geometry.row_rects().iter().enumerate() {
                    gutter_painter.text(
                        Pos2::new(gutter.right() - 8.0, row.top()),
                        Align2::RIGHT_TOP,
                        (index + 1).to_string(),
                        FontId::monospace(14.0),
                        ui.visuals().weak_text_color(),
                    );
                }
                self.gutter_input(ui, id, gutter, &geometry, buffer);
                SurfaceOutput {
                    response,
                    geometry,
                    text_clip,
                    gutter,
                    carets,
                    ime_cursor,
                }
            })
            .inner
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "shaping-probe"))]
mod shaping_probe;

#[cfg(all(test, feature = "cosmic-probe"))]
mod cosmic_probe;
