//! Runnable backend spike, intentionally not linked into the Lumen workbench.
mod accessibility;
mod commands;
mod composition_ui;
mod folding;
mod geometry;
mod gutter;
mod input;
#[cfg(feature = "shaped")]
mod shaping;
mod visual_navigation;

use egui::{Align2, FontId, Id, Pos2, Rect, Response, ScrollArea, Sense, Stroke, Ui, vec2};
use geometry::LayoutConfig;
pub use geometry::TextGeometry;
use lumen_editor_core::syntax::{Kind, Language};
use lumen_editor_core::{
    CaretAffinity, Composition, EditorBuffer, FoldRange, Indentation, SelectionClipboard,
};
#[cfg(feature = "shaped")]
pub use shaping::Shaper;
#[cfg(feature = "shaped")]
use std::{cell::RefCell, sync::Arc};
use std::{collections::BTreeSet, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct EditorSurface {
    typography: Typography,
    indentation: Indentation,
    pairing: PairingOptions,
    syntax: Option<(Language, SyntaxPalette)>,
    language: Option<Language>,
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
    fold_ranges: Vec<FoldRange>,
    collapsed: BTreeSet<usize>,
    fold_intervals: Vec<(usize, Range<usize>)>,
    fold_icons: Option<(&'static str, &'static str)>,
    folds_changed: bool,
    fold_suspended: bool,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PairingOptions {
    pub auto_closing_brackets: bool,
    pub auto_closing_quotes: bool,
    pub bracket_matching: bool,
}

impl Default for PairingOptions {
    fn default() -> Self {
        Self {
            auto_closing_brackets: true,
            auto_closing_quotes: true,
            bracket_matching: true,
        }
    }
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
    pub folded_headers: Option<BTreeSet<usize>>,
}

impl EditorSurface {
    pub fn set_language(&mut self, language: Language) {
        self.language = (language != Language::Plain).then_some(language);
    }
    pub fn set_indentation(&mut self, indentation: Indentation) {
        self.indentation = indentation;
    }
    pub fn set_pairing(&mut self, pairing: PairingOptions) {
        self.pairing = pairing;
    }
    pub fn set_syntax(&mut self, syntax: Option<(Language, SyntaxPalette)>) {
        self.syntax = syntax.filter(|(language, _)| *language != Language::Plain);
    }
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
        let folds = if composition.is_some() {
            Vec::new()
        } else {
            self.active_fold_intervals()
        };
        let geometry = TextGeometry::layout_typography(
            ui,
            text,
            origin,
            composition,
            LayoutConfig {
                typography: self.typography,
                language: self.language,
                syntax: self.syntax,
            },
            &folds,
        );
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
        self.fold_suspended = false;
        self.folds_changed = false;
        self.reveal_hidden_selections(buffer);
        self.reveal_hidden_target(buffer.text());
        if self.widget.is_some_and(|widget| widget != id) {
            #[cfg(feature = "shaped")]
            self.shape_cache.borrow_mut().take();
            buffer.break_undo_group();
            if self.composition.is_some() {
                self.notice = Some("Composition cancelled: editor identity changed".into());
            }
            self.composition = None;
            self.composition_cancelled = true;
            self.drag_anchor = None;
            self.vertical_goals = None;
            self.gutter_drag = None;
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
                            initial.source_line_count().to_string(),
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
                    self.fold_suspended = true;
                } else {
                    self.reveal_hidden_selections(buffer);
                }
                if text_changed {
                    response.mark_changed();
                    ui.ctx().request_repaint();
                }
                let geometry = if !text_changed
                    && !self.fold_suspended
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
                if self.pairing.bracket_matching
                    && self.composition.is_none()
                    && response.has_focus()
                {
                    let primary =
                        buffer.selections().selections()[buffer.selections().primary_index()];
                    if primary.range().is_empty()
                        && let Some((left, right)) = lumen_editor_core::matching_brackets_where(
                            buffer.text(),
                            primary.head,
                            |byte| geometry.is_code_byte(byte),
                        )
                    {
                        for range in [left, right] {
                            for rect in geometry.selection_rects(range) {
                                painter.rect_stroke(
                                    rect,
                                    1,
                                    Stroke::new(1.0_f32, ui.visuals().selection.stroke.color),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }
                }
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
                        geometry.source_line_for_row(index).to_string(),
                        self.typography.font_id(),
                        ui.visuals().weak_text_color(),
                    );
                    if let Some(range) = self
                        .fold_ranges
                        .iter()
                        .find(|range| range.header + 1 == geometry.source_line_for_row(index))
                        && let Some((collapsed_icon, expanded_icon)) = self.fold_icons
                    {
                        let collapsed = self.collapsed.contains(&range.header);
                        let hovered = ui.input(|input| {
                            input.pointer.hover_pos().is_some_and(|pointer| {
                                pointer.x >= gutter.left() + 2.0
                                    && pointer.x < gutter.left() + 20.0
                                    && row.top() <= pointer.y
                                    && pointer.y < row.bottom()
                            })
                        });
                        if collapsed || hovered {
                            gutter_painter.text(
                                Pos2::new(gutter.left() + 3.0, row.center().y),
                                Align2::LEFT_CENTER,
                                if collapsed {
                                    collapsed_icon
                                } else {
                                    expanded_icon
                                },
                                FontId::proportional(13.0),
                                ui.visuals().weak_text_color(),
                            );
                        }
                    }
                }
                self.gutter_input(ui, id, gutter, &geometry, buffer);
                SurfaceOutput {
                    response,
                    geometry,
                    text_clip,
                    gutter,
                    carets,
                    ime_cursor,
                    folded_headers: self.folds_changed.then(|| self.collapsed.clone()),
                }
            })
            .inner
    }
}

#[derive(Clone, Copy)]
pub struct SyntaxPalette {
    pub comment: egui::Color32,
    pub string: egui::Color32,
    pub number: egui::Color32,
    pub keyword: egui::Color32,
    pub type_name: egui::Color32,
    pub function: egui::Color32,
    pub property: egui::Color32,
    pub tag: egui::Color32,
}
impl SyntaxPalette {
    pub(crate) fn color(self, kind: Kind) -> egui::Color32 {
        match kind {
            Kind::Comment => self.comment,
            Kind::String => self.string,
            Kind::Number => self.number,
            Kind::Keyword => self.keyword,
            Kind::Type => self.type_name,
            Kind::Function => self.function,
            Kind::Property => self.property,
            Kind::Tag => self.tag,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "shaping-probe"))]
mod shaping_probe;

#[cfg(all(test, feature = "cosmic-probe"))]
mod cosmic_probe;
