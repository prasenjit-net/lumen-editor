//! Public cosmic-text layout and Swash rasterization; no editing/history or I/O.
use cosmic_text::{
    Attrs, Buffer, CacheKey, Cursor, Family, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent, Wrap, fontdb,
};
use egui::{
    Color32, ColorImage, FontDefinitions, Painter, Pos2, Rect, TextureHandle, TextureOptions, Ui,
    Vec2, vec2,
};
use lumen_editor_core::CaretAffinity;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};
use unicode_segmentation::UnicodeSegmentation;

const GLYPH_CACHE_LIMIT: usize = 2048;

#[derive(Clone)]
pub struct Shaper(Arc<Mutex<Resources>>);

struct Resources {
    fonts: FontSystem,
    raster: SwashCache,
    textures: HashMap<CacheKey, GlyphTexture>,
}

struct GlyphTexture {
    texture: TextureHandle,
    offset: Vec2,
    colored: bool,
}

impl Shaper {
    /// Font bytes are supplied by platform startup, never read in a UI component.
    pub fn new(definitions: &FontDefinitions) -> Self {
        let mut database = fontdb::Database::new();
        for (name, font) in &definitions.font_data {
            if name != "codicon" {
                database.load_font_data(font.font.to_vec());
            }
        }
        database.set_monospace_family(if cfg!(target_os = "macos") {
            "Menlo"
        } else if cfg!(target_os = "windows") {
            "Consolas"
        } else {
            "Hack"
        });
        database.set_sans_serif_family(
            if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
                "Arial"
            } else {
                "Ubuntu"
            },
        );
        Self(Arc::new(Mutex::new(Resources {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), database),
            raster: SwashCache::new(),
            textures: HashMap::new(),
        })))
    }

    pub(crate) fn layout(&self, text: &str, typography: crate::Typography) -> ShapedLayout {
        let mut resources = self.0.lock().expect("shaping resource lock");
        let mut buffer = Buffer::new(
            &mut resources.fonts,
            Metrics::new(typography.font_size, typography.line_height),
        );
        buffer.set_size(None, None);
        buffer.set_wrap(Wrap::None);
        buffer.set_tab_width(4);
        buffer.set_text(
            text,
            &Attrs::new().family(if typography.bundled_font {
                Family::Name("Hack")
            } else {
                Family::Monospace
            }),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut resources.fonts, false);
        ShapedLayout {
            buffer,
            starts: std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
            shaper: self.clone(),
            character_carets: OnceLock::new(),
            visual_stops: OnceLock::new(),
        }
    }
}

pub(crate) struct ShapedLayout {
    pub(crate) buffer: Buffer,
    pub(crate) starts: Vec<usize>,
    shaper: Shaper,
    character_carets: OnceLock<Vec<Pos2>>,
    visual_stops: OnceLock<VisualStops>,
}

type VisualStop = (usize, CaretAffinity, Pos2);
struct VisualStops {
    left: Vec<VisualStop>,
    right: Vec<VisualStop>,
}

impl ShapedLayout {
    pub(crate) fn cursor(&self, byte: usize) -> Cursor {
        let line = self
            .starts
            .partition_point(|start| *start <= byte)
            .saturating_sub(1);
        Cursor::new(line, byte - self.starts[line])
    }

    pub(crate) fn caret(&self, byte: usize, origin: Pos2) -> Rect {
        let cursor = self.cursor(byte);
        let (x, y) = self
            .buffer
            .cursor_position(&cursor)
            .unwrap_or((0.0, cursor.line as f32 * self.buffer.metrics().line_height));
        Rect::from_min_size(
            origin + vec2(x, y),
            vec2(1.0, self.buffer.metrics().line_height),
        )
    }

    pub(crate) fn caret_with_affinity(
        &self,
        byte: usize,
        affinity: CaretAffinity,
        origin: Pos2,
    ) -> Rect {
        if affinity == CaretAffinity::Before {
            let cursor = self.cursor(byte);
            for run in self
                .buffer
                .layout_runs()
                .filter(|run| run.line_i == cursor.line)
            {
                if let Some(glyph) = run.glyphs.iter().find(|glyph| glyph.end == cursor.index) {
                    let x = if glyph.level.is_rtl() {
                        glyph.x
                    } else {
                        glyph.x + glyph.w
                    };
                    return Rect::from_min_size(
                        origin + vec2(x, run.line_top),
                        vec2(1.0, self.buffer.metrics().line_height),
                    );
                }
            }
        }
        self.caret(byte, origin)
    }

    /// Traverse the visual edges of legal grapheme segments, not logical byte
    /// order. The arrival side belongs to the segment crossed by the arrow.
    pub(crate) fn visual_step(
        &self,
        text: &str,
        byte: usize,
        affinity: CaretAffinity,
        right: bool,
    ) -> Option<(usize, CaretAffinity)> {
        let current = self.caret_with_affinity(byte, affinity, Pos2::ZERO).min;
        let stops = self
            .visual_stops
            .get_or_init(|| self.build_visual_stops(text));
        let stops = if right { &stops.right } else { &stops.left };
        let index = if right {
            stops.partition_point(|stop| {
                stop.2.y < current.y - 0.01
                    || ((stop.2.y - current.y).abs() < 0.01 && stop.2.x <= current.x + 0.01)
            })
        } else {
            stops
                .partition_point(|stop| {
                    stop.2.y < current.y - 0.01
                        || ((stop.2.y - current.y).abs() < 0.01 && stop.2.x < current.x - 0.01)
                })
                .checked_sub(1)?
        };
        let target = stops.get(index)?;
        Some((target.0, target.1))
    }

    fn build_visual_stops(&self, text: &str) -> VisualStops {
        let positions = self.character_carets(text);
        let offsets: Vec<_> = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([text.len()])
            .collect();
        let after = |byte| positions[offsets.binary_search(&byte).unwrap()];
        let mut preceding = HashMap::new();
        for run in self.buffer.layout_runs() {
            for glyph in run.glyphs {
                let x = if glyph.level.is_rtl() {
                    glyph.x
                } else {
                    glyph.x + glyph.w
                };
                preceding
                    .entry(self.starts[run.line_i] + glyph.end)
                    .or_insert(Pos2::new(x, run.line_top));
            }
        }
        let before = |byte| preceding.get(&byte).copied().unwrap_or_else(|| after(byte));
        let mut stops = VisualStops {
            left: Vec::new(),
            right: Vec::new(),
        };
        for (start, grapheme) in text.grapheme_indices(true) {
            let end = start + grapheme.len();
            let left = after(start);
            let right_edge = before(end);
            if grapheme.contains('\n') {
                for target in [
                    (start, CaretAffinity::Before, before(start)),
                    (end, CaretAffinity::After, after(end)),
                ] {
                    stops.left.push(target);
                    stops.right.push(target);
                }
            } else {
                let forward = right_edge.x >= left.x;
                let logical_start = (start, CaretAffinity::After, left);
                let logical_end = (end, CaretAffinity::Before, right_edge);
                stops
                    .left
                    .push(if forward { logical_start } else { logical_end });
                stops
                    .right
                    .push(if forward { logical_end } else { logical_start });
            }
        }
        let ordering =
            |a: &VisualStop, b: &VisualStop| a.2.y.total_cmp(&b.2.y).then(a.2.x.total_cmp(&b.2.x));
        stops.left.sort_by(ordering);
        stops.right.sort_by(ordering);
        stops
    }

    /// Bulk character geometry for accessibility. Index runs once instead of
    /// asking Buffer::cursor_position to rescan all document rows per character.
    /// Run::cursor_position retains cosmic-text's glyph/bidi/cluster policy.
    pub(crate) fn character_carets(&self, text: &str) -> &[Pos2] {
        self.character_carets
            .get_or_init(|| self.build_character_carets(text))
    }

    fn build_character_carets(&self, text: &str) -> Vec<Pos2> {
        let mut lines: Vec<Vec<_>> = (0..self.starts.len()).map(|_| Vec::new()).collect();
        for run in self.buffer.layout_runs() {
            if let Some(line) = lines.get_mut(run.line_i) {
                line.push((run.line_top, indexed_run_positions(&run)));
            }
        }
        text.char_indices()
            .map(|(byte, _)| byte)
            .chain([text.len()])
            .map(|byte| {
                let cursor = self.cursor(byte);
                let position = lines.get(cursor.line).and_then(|runs| {
                    runs.iter().find_map(|(top, positions)| {
                        positions.get(&cursor.index).map(|x| Pos2::new(*x, *top))
                    })
                });
                position.unwrap_or(Pos2::new(
                    0.0,
                    cursor.line as f32 * self.buffer.metrics().line_height,
                ))
            })
            .collect()
    }

    pub(crate) fn hit(&self, text: &str, point: Vec2) -> usize {
        let Some(cursor) = self.buffer.hit(point.x, point.y) else {
            return 0;
        };
        let fallback = self.starts[cursor.line] + cursor.index;
        // Use the same legal grapheme edges as visual arrows. The backend's
        // cluster hit interpolation does not reverse multi-grapheme RTL clusters.
        // Preserve its above/below-document policy and tie preference at seams.
        let rows = self.rows(Pos2::ZERO);
        let Some(row) = rows
            .iter()
            .find(|row| row.top() <= point.y && point.y < row.bottom())
        else {
            return fallback;
        };
        let stops = self
            .visual_stops
            .get_or_init(|| self.build_visual_stops(text));
        let on_row = |stop: &&VisualStop| (stop.2.y - row.top()).abs() < 0.01;
        stops
            .left
            .iter()
            .chain(&stops.right)
            .filter(on_row)
            .min_by(|a, b| {
                (a.2.x - point.x)
                    .abs()
                    .total_cmp(&(b.2.x - point.x).abs())
                    .then((a.0 != fallback).cmp(&(b.0 != fallback)))
            })
            .map_or(fallback, |stop| stop.0)
    }

    pub(crate) fn rows(&self, origin: Pos2) -> Vec<Rect> {
        self.buffer
            .layout_runs()
            .map(|run| {
                Rect::from_min_size(
                    origin + vec2(0.0, run.line_top),
                    vec2(run.line_w, run.line_height),
                )
            })
            .collect()
    }

    pub(crate) fn selection(&self, range: std::ops::Range<usize>, origin: Pos2) -> Vec<Rect> {
        if range.is_empty() {
            return Vec::new();
        }
        let start = self.cursor(range.start);
        let end = self.cursor(range.end);
        self.buffer
            .layout_runs()
            .filter(|run| start.line <= run.line_i && run.line_i <= end.line)
            .flat_map(|run| {
                run.highlight(start, end).map(move |(x, width)| {
                    Rect::from_min_size(
                        origin + vec2(x, run.line_top),
                        vec2(width, run.line_height),
                    )
                })
            })
            .collect()
    }

    pub(crate) fn paint(
        &self,
        ui: &Ui,
        painter: &Painter,
        origin: Pos2,
        spans: &[lumen_editor_core::syntax::Span],
        palette: Option<crate::SyntaxPalette>,
    ) {
        let scale = ui.ctx().pixels_per_point();
        let mut resources = self.shaper.0.lock().expect("shaping resource lock");
        if resources.textures.len() >= GLYPH_CACHE_LIMIT {
            resources.textures.clear();
            resources.raster.image_cache.clear();
        }
        for run in self.buffer.layout_runs() {
            if origin.y + run.line_top > painter.clip_rect().bottom()
                || origin.y + run.line_top + run.line_height < painter.clip_rect().top()
            {
                continue;
            }
            for glyph in run.glyphs {
                if origin.x + glyph.x + glyph.w < painter.clip_rect().left()
                    || origin.x + glyph.x > painter.clip_rect().right()
                {
                    continue;
                }
                let physical = physical_glyph(glyph, run.line_y, scale);
                let byte = self.starts[run.line_i] + glyph.start;
                let color = syntax_color(spans, palette, byte, ui.visuals().text_color());
                if !resources.textures.contains_key(&physical.cache_key) {
                    let Resources {
                        fonts,
                        raster,
                        textures,
                    } = &mut *resources;
                    if let Some(image) = raster.get_image(fonts, physical.cache_key) {
                        let size = [
                            image.placement.width as usize,
                            image.placement.height as usize,
                        ];
                        if size[0] == 0 || size[1] == 0 {
                            continue;
                        }
                        let colored = matches!(image.content, SwashContent::Color);
                        let pixels = match image.content {
                            SwashContent::Mask => image
                                .data
                                .iter()
                                .map(|alpha| Color32::from_white_alpha(*alpha))
                                .collect(),
                            SwashContent::Color => image
                                .data
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .map(|rgba| {
                                    Color32::from_rgba_unmultiplied(
                                        rgba[0], rgba[1], rgba[2], rgba[3],
                                    )
                                })
                                .collect(),
                            SwashContent::SubpixelMask => image
                                .data
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .map(|rgba| {
                                    Color32::from_white_alpha(rgba[0].max(rgba[1]).max(rgba[2]))
                                })
                                .collect(),
                        };
                        textures.insert(
                            physical.cache_key,
                            GlyphTexture {
                                texture: ui.ctx().load_texture(
                                    "editor-shaped-glyph",
                                    ColorImage::new(size, pixels),
                                    TextureOptions::LINEAR,
                                ),
                                offset: vec2(
                                    image.placement.left as f32,
                                    -image.placement.top as f32,
                                ),
                                colored,
                            },
                        );
                    }
                }
                if let Some(texture) = resources.textures.get(&physical.cache_key) {
                    let min = origin
                        + (vec2(physical.x as f32, physical.y as f32) + texture.offset) / scale;
                    let rect = Rect::from_min_size(min, texture.texture.size_vec2() / scale);
                    if painter.clip_rect().intersects(rect) {
                        painter.image(
                            texture.texture.id(),
                            rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            if texture.colored {
                                Color32::WHITE
                            } else {
                                color
                            },
                        );
                    }
                }
            }
        }
    }
}

fn syntax_color(
    spans: &[lumen_editor_core::syntax::Span],
    palette: Option<crate::SyntaxPalette>,
    byte: usize,
    default: Color32,
) -> Color32 {
    palette
        .and_then(|palette| {
            spans
                .get(
                    spans
                        .partition_point(|span| span.range.start <= byte)
                        .checked_sub(1)?,
                )
                .filter(|span| byte < span.range.end)
                .map(|span| palette.color(span.kind))
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod syntax_tests {
    use super::*;
    #[test]
    fn glyph_color_tracks_visual_byte_spans_and_defaults() {
        let palette = crate::tests::test_syntax_palette();
        let spans = lumen_editor_core::syntax::highlight(
            "fn main()",
            lumen_editor_core::syntax::Language::Rust,
        );
        assert_eq!(
            syntax_color(&spans, Some(palette), 0, Color32::WHITE),
            palette.keyword
        );
        assert_eq!(
            syntax_color(&spans, Some(palette), 3, Color32::WHITE),
            palette.function
        );
        assert_eq!(
            syntax_color(&spans, Some(palette), 2, Color32::WHITE),
            Color32::WHITE
        );
        assert_eq!(
            syntax_color(&spans, None, 0, Color32::WHITE),
            Color32::WHITE
        );
    }
}

/// Index the public LayoutRun::cursor_position policy: first containing glyph,
/// then first matching end edge. Preserve that priority for overlapping glyphs
/// and mixed bidi; do not sort glyphs or infer positions from scalar widths.
fn indexed_run_positions(run: &cosmic_text::LayoutRun<'_>) -> HashMap<usize, f32> {
    let offsets: Vec<_> = run
        .text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain([run.text.len()])
        .collect();
    let mut positions = HashMap::with_capacity(offsets.len());
    if run.glyphs.is_empty() {
        positions.extend(offsets.into_iter().map(|byte| (byte, 0.0)));
        return positions;
    }
    for glyph in run.glyphs {
        let first = offsets.partition_point(|byte| *byte < glyph.start);
        let last = offsets.partition_point(|byte| *byte < glyph.end);
        let cluster = &run.text[glyph.start..glyph.end];
        let graphemes: Vec<_> = cluster
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .collect();
        for byte in &offsets[first..last] {
            positions.entry(*byte).or_insert_with(|| {
                let offset = if *byte == glyph.start {
                    0.0
                } else {
                    let before = graphemes.partition_point(|start| glyph.start + start < *byte);
                    glyph.w * before as f32 / graphemes.len() as f32
                };
                if glyph.level.is_rtl() {
                    glyph.x + glyph.w - offset
                } else {
                    glyph.x + offset
                }
            });
        }
    }
    for glyph in run.glyphs {
        positions.entry(glyph.end).or_insert_with(|| {
            if glyph.level.is_rtl() {
                glyph.x + glyph.w - glyph.w
            } else {
                glyph.x + glyph.w
            }
        });
    }
    positions
}

fn physical_glyph(
    glyph: &cosmic_text::LayoutGlyph,
    baseline: f32,
    scale: f32,
) -> cosmic_text::PhysicalGlyph {
    // cosmic's offset parameter is already physical, unlike glyph x/y.
    glyph.physical((0.0, baseline * scale), scale)
}

impl Default for Shaper {
    fn default() -> Self {
        Self::new(&FontDefinitions::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_direction_arrows_follow_visual_order_and_retain_boundary_side() {
        let text = "abc אבג xyz";
        let layout = Shaper::default().layout(text, crate::Typography::default());
        let before = layout.caret_with_affinity(4, CaretAffinity::Before, Pos2::ZERO);
        let after = layout.caret_with_affinity(4, CaretAffinity::After, Pos2::ZERO);
        assert!((before.left() - after.left()).abs() > 1.0);
        let mut cursor = (0, CaretAffinity::After);
        let mut bytes = vec![0];
        let mut point = layout
            .caret_with_affinity(cursor.0, cursor.1, Pos2::ZERO)
            .min;
        for _ in 0..40 {
            let Some(next) = layout.visual_step(text, cursor.0, cursor.1, true) else {
                break;
            };
            let next_point = layout.caret_with_affinity(next.0, next.1, Pos2::ZERO).min;
            assert!(next_point.x > point.x, "{cursor:?} -> {next:?}");
            assert!(
                text.grapheme_indices(true).any(|(byte, _)| byte == next.0) || next.0 == text.len()
            );
            bytes.push(next.0);
            cursor = next;
            point = next_point;
        }
        assert_eq!(cursor.0, text.len());
        assert!(bytes.windows(2).any(|pair| pair[1] < pair[0]));
        for _ in 0..40 {
            let Some(next) = layout.visual_step(text, cursor.0, cursor.1, false) else {
                break;
            };
            let next_point = layout.caret_with_affinity(next.0, next.1, Pos2::ZERO).min;
            assert!(next_point.x < point.x);
            cursor = next;
            point = next_point;
        }
        assert_eq!(cursor.0, 0);
    }

    #[test]
    fn visual_arrows_cross_empty_lines_and_keep_extended_graphemes_atomic() {
        let text = "é👩‍💻\n\n漢";
        let layout = Shaper::default().layout(text, crate::Typography::default());
        let mut cursor = (0, CaretAffinity::After);
        let boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain([text.len()])
            .collect();
        for _ in 0..20 {
            let Some(next) = layout.visual_step(text, cursor.0, cursor.1, true) else {
                break;
            };
            assert!(boundaries.contains(&next.0));
            cursor = next;
        }
        assert_eq!(cursor.0, text.len());
        for _ in 0..20 {
            let Some(next) = layout.visual_step(text, cursor.0, cursor.1, false) else {
                break;
            };
            assert!(boundaries.contains(&next.0));
            cursor = next;
        }
        assert_eq!(cursor.0, 0);
    }

    #[test]
    fn bulk_character_geometry_matches_renderer_at_every_boundary() {
        for text in [
            "",
            "\n\n",
            "one\ntwo\n",
            "office ffi é\t👩‍💻\n漢字",
            "abc العربية xyz\nالعربية\n123",
            "a\u{2028}b\nend",
        ] {
            let layout = Shaper::default().layout(text, crate::Typography::default());
            let positions = layout.character_carets(text);
            let boundaries: Vec<_> = text
                .char_indices()
                .map(|(byte, _)| byte)
                .chain([text.len()])
                .collect();
            assert_eq!(positions.len(), boundaries.len());
            for (position, byte) in positions.iter().zip(boundaries) {
                assert_eq!(
                    *position,
                    layout.caret(byte, Pos2::ZERO).min,
                    "{text:?}, byte {byte}"
                );
            }
        }
    }
    #[test]
    fn retina_glyph_baselines_match_logical_layout_rows() {
        let layout = Shaper::default().layout("one\ntwo\nthree", crate::Typography::default());
        for run in layout.buffer.layout_runs() {
            let glyph = run.glyphs.first().unwrap();
            for scale in [1.0, 1.5, 2.0] {
                let physical = physical_glyph(glyph, run.line_y, scale);
                assert!(
                    (physical.y as f32 / scale
                        - (run.line_y + glyph.y - glyph.font_size * glyph.y_offset))
                        .abs()
                        <= 1.0 / scale
                );
            }
        }
    }
}
