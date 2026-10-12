//! Runnable backend spike, intentionally not linked into the Lumen workbench.
mod accessibility;
mod commands;
mod composition_ui;
mod geometry;
mod gutter;
mod input;
#[cfg(feature = "shaped")]
mod shaping;
mod visual_navigation;

use egui::{Align2, FontId, Id, Pos2, Rect, Response, ScrollArea, Sense, Stroke, Ui, vec2};
pub use geometry::TextGeometry;
use lumen_editor_core::{CaretAffinity, Composition, EditorBuffer, SelectionClipboard};
#[cfg(feature = "shaped")]
pub use shaping::Shaper;
#[cfg(feature = "shaped")]
use std::{cell::RefCell, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct EditorSurface {
    typography: Typography,
    widget: Option<Id>,
    drag_anchor: Option<DragSelection>,
    pending_shortcut_text: bool,
    composition: Option<Composition>,
    composition_cancelled: bool,
    clipboard: Option<SelectionClipboard>,
    vertical_goals: Option<commands::VerticalGoals>,
    reveal: Option<usize>,
    gutter_id: Option<Id>,
    gutter_drag: Option<(std::ops::Range<usize>, u64)>,
    pub notice: Option<String>,
    #[cfg(feature = "shaped")]
    shaper: Option<Shaper>,
    #[cfg(feature = "shaped")]
    shape_cache: RefCell<Option<CachedShape>>,
}

/// Display-only configuration; never changes text, selections or history.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub font_size: f32,
    pub line_height: f32,
    pub bundled_font: bool,
}
impl Default for Typography {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            line_height: 18.0,
            bundled_font: false,
        }
    }
}
impl Typography {
    pub fn font_id(self) -> FontId {
        if self.bundled_font {
            FontId::new(self.font_size, egui::FontFamily::Name("lumen-hack".into()))
        } else {
            FontId::monospace(self.font_size)
        }
    }
}

#[cfg(feature = "shaped")]
struct CachedShape {
    galley: Arc<egui::Galley>,
    layout: Arc<shaping::ShapedLayout>,
}

#[derive(Clone)]
enum DragSelection {
    Linear {
        anchor: usize,
        revision: u64,
        affinity: CaretAffinity,
    },
    Additive {
        anchor: usize,
        revision: u64,
        affinity: CaretAffinity,
        previous: Vec<(
            lumen_editor_core::Selection,
            lumen_editor_core::SelectionAffinity,
        )>,
    },
    Column {
        origin: Pos2,
        revision: u64,
    },
}

impl DragSelection {
    fn revision(&self) -> u64 {
        match self {
            Self::Linear { revision, .. }
            | Self::Additive { revision, .. }
            | Self::Column { revision, .. } => *revision,
        }
    }
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
    /// Defer relayout during composition/drag to preserve their layout authority.
    pub fn set_typography(&mut self, typography: Typography) -> bool {
        if !typography.font_size.is_finite()
            || !typography.line_height.is_finite()
            || !(8.0..=48.0).contains(&typography.font_size)
            || !(typography.font_size..=80.0).contains(&typography.line_height)
            || self.composition.is_some()
            || self.drag_anchor.is_some()
            || self.gutter_drag.is_some()
        {
            return false;
        }
        if self.typography != typography {
            self.typography = typography;
            self.vertical_goals = None;
            #[cfg(feature = "shaped")]
            self.shape_cache.borrow_mut().take();
        }
        true
    }
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
        let geometry =
            TextGeometry::layout_typography(ui, text, origin, composition, self.typography);
        #[cfg(feature = "shaped")]
        let geometry = {
            let mut geometry = geometry;
            if let Some(shaper) = &self.shaper {
                // egui invalidates galley identity for text, fonts, scale and
                // layout style changes. Cache only the matching immutable shape;
                // source/preedit mappings remain owned by this geometry instance.
                let mut cache = self.shape_cache.borrow_mut();
                let layout = if let Some(cached) = &*cache
                    && Arc::ptr_eq(&cached.galley, &geometry.galley)
                {
                    cached.layout.clone()
                } else {
                    let layout = Arc::new(shaper.layout(geometry.galley.text(), self.typography));
                    *cache = Some(CachedShape {
                        galley: geometry.galley.clone(),
                        layout: layout.clone(),
                    });
                    layout
                };
                geometry.shaped = Some(layout);
            }
            geometry
        };
        geometry
    }
    pub fn show(&mut self, ui: &mut Ui, id: Id, buffer: &mut EditorBuffer) -> SurfaceOutput {
        if self.widget.is_some_and(|widget| widget != id) {
            #[cfg(feature = "shaped")]
            self.shape_cache.borrow_mut().take();
            buffer.break_undo_group();
            self.composition = None;
            self.composition_cancelled = true;
            self.drag_anchor = None;
            self.vertical_goals = None;
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
                            self.typography.font_id(),
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
                let mut response = ui
                    .interact(content.intersect(text_clip), id, Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::Text);
                let before_selection = buffer.selections().clone();
                let before_preedit = self
                    .composition
                    .as_ref()
                    .map(|composition| composition.preedit().to_owned());
                let text_changed = self.handle_input(ui, &response, text_clip, origin, buffer);
                if text_changed {
                    response.mark_changed();
                    ui.ctx().request_repaint();
                }
                let geometry = if !text_changed
                    && before_preedit.as_deref()
                        == self.composition.as_ref().map(Composition::preedit)
                {
                    // The sizing pass already laid out this exact source and
                    // preedit. Reuse it for painting and pointer authority;
                    // translation changes no byte/character mapping.
                    let mut geometry = initial;
                    geometry.origin = origin;
                    geometry
                } else {
                    self.geometry(ui, buffer.text(), origin, self.composition.as_ref())
                };
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
                    .zip(buffer.selections().affinities())
                    .map(|(selection, affinity)| {
                        geometry.caret_with_affinity(selection.head, affinity.head)
                    })
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
                    // Gutter labels share text row geometry, but only visible
                    // rows need paint commands. Use vertical overlap so empty
                    // lines (zero-width text rows) keep their numbers.
                    if row.bottom() < gutter_painter.clip_rect().top()
                        || row.top() > gutter_painter.clip_rect().bottom()
                    {
                        continue;
                    }
                    gutter_painter.text(
                        Pos2::new(gutter.right() - 8.0, row.top()),
                        Align2::RIGHT_TOP,
                        (index + 1).to_string(),
                        self.typography.font_id(),
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
