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
            if name != "lumen-icons" {
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

    #[cfg(test)]
    pub(crate) fn layout(&self, text: &str, typography: crate::Typography) -> ShapedLayout {
        self.layout_after(text, typography, None)
    }

    /// Lay out `text`, reusing every already-shaped line of `previous` that is
    /// unchanged. Small documents then shape whatever is still missing, so their
    /// widths and row geometry are exact; large ones shape only what is on screen
    /// (see `prepare`).
    pub(crate) fn layout_after(
        &self,
        text: &str,
        typography: crate::Typography,
        previous: Option<&ShapedLayout>,
    ) -> ShapedLayout {
        let layout = self.build_layout(text, typography);
        if let Some(previous) = previous {
            layout.inherit_lines_from(previous);
        }
        if text.len() <= EAGER_BYTES {
            for line in 0..layout.starts.len() {
                layout.line(line);
            }
            *layout.visible.lock().expect("visible lock") = (0, layout.starts.len());
        }
        layout
    }

    /// Test hook: lazy shaping regardless of size, to exercise the large-document path.
    #[cfg(test)]
    pub(crate) fn layout_lazy(&self, text: &str, typography: crate::Typography) -> ShapedLayout {
        self.build_layout(text, typography)
    }

    fn build_layout(&self, text: &str, typography: crate::Typography) -> ShapedLayout {
        let starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        ShapedLayout {
            text: Arc::from(text),
            typography,
            starts,
            shaper: self.clone(),
            lines: Mutex::new(HashMap::new()),
            row_stops: Mutex::new(HashMap::new()),
            visible: Mutex::new((0, 0)),
            widths: OnceLock::new(),
            character_carets: OnceLock::new(),
        }
    }
}

/// Documents up to this many bytes are shaped completely up front.
const EAGER_BYTES: usize = 256 * 1024;
/// Shaped lines kept at once; the cache restarts when it fills.
const LINE_CACHE_LIMIT: usize = 4096;

/// One source line, shaped on its own. A buffer holds a single line, so its first
/// (and only) layout run sits at the top; the owner offsets it by its row.
struct LineShape {
    buffer: Buffer,
    /// Per run: its top and the x position of every legal caret index, built on
    /// demand for caret and visual-stop queries.
    positions: OnceLock<Vec<(f32, HashMap<usize, f32>)>>,
    /// Right edge of the glyph ending at each local index (the "before" side).
    preceding: OnceLock<HashMap<usize, Pos2>>,
}

impl LineShape {
    fn width(&self) -> f32 {
        self.buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max)
    }

    fn positions(&self) -> &[(f32, HashMap<usize, f32>)] {
        self.positions.get_or_init(|| {
            self.buffer
                .layout_runs()
                .map(|run| (run.line_top, indexed_run_positions(&run)))
                .collect()
        })
    }

    fn preceding(&self) -> &HashMap<usize, Pos2> {
        self.preceding.get_or_init(|| {
            let mut preceding = HashMap::new();
            for run in self.buffer.layout_runs() {
                for glyph in run.glyphs {
                    let x = if glyph.level.is_rtl() {
                        glyph.x
                    } else {
                        glyph.x + glyph.w
                    };
                    preceding
                        .entry(glyph.end)
                        .or_insert(Pos2::new(x, run.line_top));
                }
            }
            preceding
        })
    }
}

/// Lazily shaped text. Lines are shaped when something needs them (painting what
/// is visible, a caret, a click) and kept in a bounded cache, so opening or
/// editing a large document costs the lines involved, not the whole document.
pub(crate) struct ShapedLayout {
    text: Arc<str>,
    typography: crate::Typography,
    pub(crate) starts: Vec<usize>,
    shaper: Shaper,
    lines: Mutex<HashMap<usize, Arc<LineShape>>>,
    row_stops: Mutex<HashMap<usize, Arc<RowStops>>>,
    /// Lines worth shaping exactly for selection and sizing; the rest use estimates.
    visible: Mutex<(usize, usize)>,
    widths: OnceLock<Vec<f32>>,
    character_carets: OnceLock<Vec<Pos2>>,
}

type VisualStop = (usize, CaretAffinity, Pos2);
/// The caret stops on one visual row, ordered by x.
struct RowStops {
    left: Vec<VisualStop>,
    right: Vec<VisualStop>,
}

impl ShapedLayout {
    fn line_height(&self) -> f32 {
        self.typography.line_height
    }

    fn line_text(&self, line: usize) -> &str {
        let start = self.starts[line];
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1);
        &self.text[start..end]
    }

    fn shape_text(&self, text: &str) -> Buffer {
        let mut resources = self.shaper.0.lock().expect("shaping resource lock");
        let mut buffer = Buffer::new(
            &mut resources.fonts,
            Metrics::new(self.typography.font_size, self.typography.line_height),
        );
        buffer.set_size(None, None);
        buffer.set_wrap(Wrap::None);
        buffer.set_tab_width(4);
        buffer.set_text(
            text,
            &Attrs::new().family(if self.typography.bundled_font {
                Family::Name("Hack")
            } else {
                Family::Monospace
            }),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut resources.fonts, false);
        buffer
    }

    fn line(&self, line: usize) -> Arc<LineShape> {
        if let Some(shape) = self.lines.lock().expect("line cache lock").get(&line) {
            return shape.clone();
        }
        let shape = Arc::new(LineShape {
            buffer: self.shape_text(self.line_text(line)),
            positions: OnceLock::new(),
            preceding: OnceLock::new(),
        });
        let mut cache = self.lines.lock().expect("line cache lock");
        if cache.len() >= LINE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line, shape.clone());
        shape
    }

    /// Shape the rows an upcoming paint will show, plus a margin, and remember
    /// them as the window for exact selection and sizing.
    pub(crate) fn prepare(&self, origin_y: f32, top: f32, bottom: f32) {
        let lines = self.starts.len();
        let height = self.line_height().max(1.0);
        let first = (((top - origin_y) / height).floor().max(0.0) as usize)
            .saturating_sub(2)
            .min(lines);
        let last = ((((bottom - origin_y) / height).ceil().max(0.0) as usize) + 2).min(lines);
        *self.visible.lock().expect("visible lock") = (first, last);
        for line in first..last {
            self.line(line);
        }
    }

    /// Carry over every line already shaped in `previous` whose text is unchanged
    /// (same lines at the start, same lines at the end, shifted by the edit), so a
    /// small edit re-shapes only the lines it touched.
    pub(crate) fn inherit_lines_from(&self, previous: &ShapedLayout) {
        if previous.typography != self.typography {
            return;
        }
        let (old_n, new_n) = (previous.starts.len(), self.starts.len());
        let shared = old_n.min(new_n);
        let mut prefix = 0;
        while prefix < shared && previous.line_text(prefix) == self.line_text(prefix) {
            prefix += 1;
        }
        let mut suffix = 0;
        while suffix < shared - prefix
            && previous.line_text(old_n - 1 - suffix) == self.line_text(new_n - 1 - suffix)
        {
            suffix += 1;
        }
        let mut lines = self.lines.lock().expect("line cache lock");
        for (&line, shape) in previous.lines.lock().expect("line cache lock").iter() {
            let moved = if line < prefix {
                Some(line)
            } else if line >= old_n - suffix {
                Some(line + new_n - old_n)
            } else {
                None
            };
            if let Some(moved) = moved {
                lines.insert(moved, shape.clone());
            }
        }
        let window = *previous.visible.lock().expect("visible lock");
        *self.visible.lock().expect("visible lock") = window;
    }

    /// Estimated width of every line, used for lines that are not shaped: the
    /// character count times the monospace advance. Exact widths replace these as
    /// lines are shaped.
    fn estimated_widths(&self) -> &[f32] {
        self.widths.get_or_init(|| {
            let reference = self.shape_text("0000000000");
            let advance = reference
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0, f32::max)
                / 10.0;
            (0..self.starts.len())
                .map(|line| {
                    let columns: usize = self
                        .line_text(line)
                        .chars()
                        .map(|character| if character == '\t' { 4 } else { 1 })
                        .sum();
                    columns as f32 * advance
                })
                .collect()
        })
    }

    fn width_of(&self, line: usize) -> f32 {
        match self.lines.lock().expect("line cache lock").get(&line) {
            Some(shape) => shape.width(),
            None => self.estimated_widths()[line],
        }
    }

    pub(crate) fn is_rtl(&self, line: usize) -> bool {
        self.line(line).buffer.is_rtl(0).unwrap_or(false)
    }

    pub(crate) fn cursor(&self, byte: usize) -> Cursor {
        let line = self
            .starts
            .partition_point(|start| *start <= byte)
            .saturating_sub(1);
        Cursor::new(line, byte - self.starts[line])
    }

    pub(crate) fn caret(&self, byte: usize, origin: Pos2) -> Rect {
        let cursor = self.cursor(byte);
        let top = cursor.line as f32 * self.line_height();
        let (x, y) = self
            .line(cursor.line)
            .buffer
            .cursor_position(&Cursor::new(0, cursor.index))
            .unwrap_or((0.0, 0.0));
        Rect::from_min_size(origin + vec2(x, top + y), vec2(1.0, self.line_height()))
    }

    pub(crate) fn caret_with_affinity(
        &self,
        byte: usize,
        affinity: CaretAffinity,
        origin: Pos2,
    ) -> Rect {
        if affinity == CaretAffinity::Before {
            let cursor = self.cursor(byte);
            let top = cursor.line as f32 * self.line_height();
            let shape = self.line(cursor.line);
            for run in shape.buffer.layout_runs() {
                if let Some(glyph) = run.glyphs.iter().find(|glyph| glyph.end == cursor.index) {
                    let x = if glyph.level.is_rtl() {
                        glyph.x
                    } else {
                        glyph.x + glyph.w
                    };
                    return Rect::from_min_size(
                        origin + vec2(x, top + run.line_top),
                        vec2(1.0, self.line_height()),
                    );
                }
            }
        }
        self.caret(byte, origin)
    }

    /// Where a caret at `byte` sits (the "after" side), or the line start when
    /// the line has no glyph for it.
    fn after(&self, byte: usize) -> Pos2 {
        let cursor = self.cursor(byte);
        let top = cursor.line as f32 * self.line_height();
        let shape = self.line(cursor.line);
        shape
            .positions()
            .iter()
            .find_map(|(run_top, positions)| {
                positions
                    .get(&cursor.index)
                    .map(|x| Pos2::new(*x, top + *run_top))
            })
            .unwrap_or(Pos2::new(0.0, top))
    }

    /// Right edge of the glyph that ends at `byte`, else the "after" position.
    fn before(&self, byte: usize) -> Pos2 {
        let cursor = self.cursor(byte);
        let top = cursor.line as f32 * self.line_height();
        self.line(cursor.line)
            .preceding()
            .get(&cursor.index)
            .map(|position| Pos2::new(position.x, top + position.y))
            .unwrap_or_else(|| self.after(byte))
    }

    /// Caret stops of one visual row, in the order the whole-document layout
    /// produced them: the arrival side of the previous line's line break, then each
    /// grapheme of this line, then this line's own break.
    fn row_stops(&self, row: usize) -> Arc<RowStops> {
        if let Some(stops) = self.row_stops.lock().expect("row cache lock").get(&row) {
            return stops.clone();
        }
        let mut stops = RowStops {
            left: Vec::new(),
            right: Vec::new(),
        };
        let start = self.starts[row];
        if row > 0 {
            let target = (start, CaretAffinity::After, self.after(start));
            stops.left.push(target);
            stops.right.push(target);
        }
        for (offset, grapheme) in self.line_text(row).grapheme_indices(true) {
            let from = start + offset;
            let end = from + grapheme.len();
            let left = self.after(from);
            let right_edge = self.before(end);
            let forward = right_edge.x >= left.x;
            let logical_start = (from, CaretAffinity::After, left);
            let logical_end = (end, CaretAffinity::Before, right_edge);
            stops
                .left
                .push(if forward { logical_start } else { logical_end });
            stops
                .right
                .push(if forward { logical_end } else { logical_start });
        }
        if row + 1 < self.starts.len() {
            let newline = self.starts[row + 1] - 1;
            let target = (newline, CaretAffinity::Before, self.before(newline));
            stops.left.push(target);
            stops.right.push(target);
        }
        let ordering =
            |a: &VisualStop, b: &VisualStop| a.2.y.total_cmp(&b.2.y).then(a.2.x.total_cmp(&b.2.x));
        stops.left.sort_by(ordering);
        stops.right.sort_by(ordering);
        let stops = Arc::new(stops);
        let mut cache = self.row_stops.lock().expect("row cache lock");
        if cache.len() >= LINE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(row, stops.clone());
        stops
    }

    fn row_of(&self, y: f32) -> usize {
        ((y / self.line_height()).round().max(0.0) as usize).min(self.starts.len() - 1)
    }

    /// Traverse the visual edges of legal grapheme segments, not logical byte
    /// order. The arrival side belongs to the segment crossed by the arrow.
    pub(crate) fn visual_step(
        &self,
        _text: &str,
        byte: usize,
        affinity: CaretAffinity,
        right: bool,
    ) -> Option<(usize, CaretAffinity)> {
        let current = self.caret_with_affinity(byte, affinity, Pos2::ZERO).min;
        let row = self.row_of(current.y);
        if right {
            let ahead = |stop: &VisualStop| {
                stop.2.y < current.y - 0.01
                    || ((stop.2.y - current.y).abs() < 0.01 && stop.2.x <= current.x + 0.01)
            };
            for row in row..self.starts.len() {
                let stops = self.row_stops(row);
                let index = stops.right.partition_point(ahead);
                if let Some(target) = stops.right.get(index) {
                    return Some((target.0, target.1));
                }
            }
            None
        } else {
            let behind = |stop: &VisualStop| {
                stop.2.y < current.y - 0.01
                    || ((stop.2.y - current.y).abs() < 0.01 && stop.2.x < current.x - 0.01)
            };
            for row in (0..=row).rev() {
                let stops = self.row_stops(row);
                let index = stops.left.partition_point(behind);
                if let Some(target) = index.checked_sub(1).and_then(|i| stops.left.get(i)) {
                    return Some((target.0, target.1));
                }
            }
            None
        }
    }

    /// Bulk character geometry for accessibility. Shapes every line, so it is only
    /// built when assistive technology asks for the whole document.
    pub(crate) fn character_carets(&self, text: &str) -> &[Pos2] {
        self.character_carets.get_or_init(|| {
            text.char_indices()
                .map(|(byte, _)| byte)
                .chain([text.len()])
                .map(|byte| self.after(byte))
                .collect()
        })
    }

    pub(crate) fn hit(&self, _text: &str, point: Vec2) -> usize {
        let height = self.line_height();
        let lines = self.starts.len();
        // Same policy as the backend's whole-buffer hit: above the document is the
        // start, below it is the end of the last line, otherwise the row hit.
        let (line, index) = if point.y < 0.0 {
            (0, 0)
        } else {
            let row = (point.y / height).floor() as usize;
            if row >= lines {
                (lines - 1, self.line_text(lines - 1).len())
            } else {
                let local = point.y - row as f32 * height;
                match self.line(row).buffer.hit(point.x, local) {
                    Some(cursor) => (row, cursor.index),
                    None => return 0,
                }
            }
        };
        let fallback = self.starts[line] + index;
        let row_top = line as f32 * height;
        if !(row_top <= point.y && point.y < row_top + height) {
            return fallback;
        }
        let stops = self.row_stops(line);
        let on_row = |stop: &&VisualStop| (stop.2.y - row_top).abs() < 0.01;
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
        let height = self.line_height();
        (0..self.starts.len())
            .map(|line| {
                Rect::from_min_size(
                    origin + vec2(0.0, line as f32 * height),
                    vec2(self.width_of(line), height),
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
        let height = self.line_height();
        let (visible_start, visible_end) = *self.visible.lock().expect("visible lock");
        let visible = visible_start..visible_end;
        let mut rects = Vec::new();
        for line in start.line..=end.line.min(self.starts.len() - 1) {
            let top = origin.y + line as f32 * height;
            let edge = line == start.line || line == end.line;
            if edge
                || visible.contains(&line)
                || self
                    .lines
                    .lock()
                    .expect("line cache lock")
                    .contains_key(&line)
            {
                let shape = self.line(line);
                // Only the first and last lines are partly selected; a cursor on a
                // different line (line `usize::MAX`) means "selected from the edge".
                let from = if start.line == line {
                    Cursor::new(0, start.index)
                } else {
                    Cursor::new(usize::MAX, 0)
                };
                let to = if end.line == line {
                    Cursor::new(0, end.index)
                } else {
                    Cursor::new(usize::MAX, 0)
                };
                for run in shape.buffer.layout_runs() {
                    for (x, width) in run.highlight(from, to) {
                        rects.push(Rect::from_min_size(
                            Pos2::new(origin.x + x, top),
                            vec2(width, height),
                        ));
                    }
                }
            } else if !self.line_text(line).is_empty() {
                // A fully selected line off screen: its estimated extent is enough
                // until it scrolls in and is shaped exactly.
                rects.push(Rect::from_min_size(
                    Pos2::new(origin.x, top),
                    vec2(self.width_of(line), height),
                ));
            }
        }
        rects
    }

    pub(crate) fn paint(
        &self,
        ui: &Ui,
        painter: &Painter,
        origin: Pos2,
        spans: &[crate::geometry::SyntaxSpan],
        palette: Option<crate::SyntaxPalette>,
    ) {
        let scale = ui.ctx().pixels_per_point();
        let height = self.line_height().max(1.0);
        let clip = painter.clip_rect();
        let lines = self.starts.len();
        let first = (((clip.top() - origin.y) / height).floor().max(0.0) as usize).min(lines);
        let last =
            ((((clip.bottom() - origin.y) / height).ceil().max(0.0) as usize) + 1).min(lines);
        for line in first..last {
            let shape = self.line(line);
            let line_offset = line as f32 * height;
            let mut resources = self.shaper.0.lock().expect("shaping resource lock");
            if resources.textures.len() >= GLYPH_CACHE_LIMIT {
                resources.textures.clear();
                resources.raster.image_cache.clear();
            }
            for run in shape.buffer.layout_runs() {
                let line_top = line_offset + run.line_top;
                if origin.y + line_top > clip.bottom()
                    || origin.y + line_top + run.line_height < clip.top()
                {
                    continue;
                }
                let line_y = line_offset + run.line_y;
                for glyph in run.glyphs {
                    if origin.x + glyph.x + glyph.w < clip.left()
                        || origin.x + glyph.x > clip.right()
                    {
                        continue;
                    }
                    let physical = physical_glyph(glyph, line_y, scale);
                    let byte = self.starts[line] + glyph.start;
                    let color = syntax_color(spans, palette, byte, ui.visuals().text_color());
                    self.paint_glyph(ui, painter, &mut resources, physical, origin, scale, color);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_glyph(
        &self,
        ui: &Ui,
        painter: &Painter,
        resources: &mut Resources,
        physical: cosmic_text::PhysicalGlyph,
        origin: Pos2,
        scale: f32,
        color: Color32,
    ) {
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
                    return;
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
                            Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
                        })
                        .collect(),
                    SwashContent::SubpixelMask => image
                        .data
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|rgba| Color32::from_white_alpha(rgba[0].max(rgba[1]).max(rgba[2])))
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
                        offset: vec2(image.placement.left as f32, -image.placement.top as f32),
                        colored,
                    },
                );
            }
        }
        if let Some(texture) = resources.textures.get(&physical.cache_key) {
            let min =
                origin + (vec2(physical.x as f32, physical.y as f32) + texture.offset) / scale;
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

fn syntax_color(
    spans: &[crate::geometry::SyntaxSpan],
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
                .map(|span| span.color(palette))
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod syntax_tests {
    use super::*;
    #[test]
    fn glyph_color_tracks_visual_byte_spans_and_defaults() {
        let palette = crate::tests::test_syntax_palette();
        let spans =
            crate::geometry::highlight("fn main()", lumen_editor_core::syntax::Language::Rust, &[]);
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
mod equivalence;
#[cfg(test)]
mod oracle;

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
        for line in 0..layout.starts.len() {
            let shape = layout.line(line);
            let run = shape.buffer.layout_runs().next().unwrap();
            let line_y = line as f32 * layout.line_height() + run.line_y;
            let glyph = run.glyphs.first().unwrap();
            for scale in [1.0, 1.5, 2.0] {
                let physical = physical_glyph(glyph, line_y, scale);
                assert!(
                    (physical.y as f32 / scale
                        - (line_y + glyph.y - glyph.font_size * glyph.y_offset))
                        .abs()
                        <= 1.0 / scale
                );
            }
        }
    }
}
