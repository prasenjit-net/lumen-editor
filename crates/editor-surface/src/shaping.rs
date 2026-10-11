//! Public cosmic-text layout and Swash rasterization; no editing/history or I/O.
use cosmic_text::{
    Attrs, Buffer, CacheKey, Cursor, Family, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent, Wrap, fontdb,
};
use egui::{
    Color32, ColorImage, FontDefinitions, Painter, Pos2, Rect, TextureHandle, TextureOptions, Ui,
    Vec2, vec2,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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

    pub(crate) fn layout(&self, text: &str) -> ShapedLayout {
        let mut resources = self.0.lock().expect("shaping resource lock");
        let mut buffer = Buffer::new(&mut resources.fonts, Metrics::new(14.0, 18.0));
        buffer.set_size(None, None);
        buffer.set_wrap(Wrap::None);
        buffer.set_tab_width(4);
        buffer.set_text(
            text,
            &Attrs::new().family(Family::Monospace),
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
        }
    }
}

pub(crate) struct ShapedLayout {
    pub(crate) buffer: Buffer,
    pub(crate) starts: Vec<usize>,
    shaper: Shaper,
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
            .unwrap_or((0.0, cursor.line as f32 * 18.0));
        Rect::from_min_size(origin + vec2(x, y), vec2(1.0, 18.0))
    }

    pub(crate) fn hit(&self, point: Vec2) -> usize {
        let Some(cursor) = self.buffer.hit(point.x, point.y) else {
            return 0;
        };
        self.starts[cursor.line] + cursor.index
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

    pub(crate) fn paint(&self, ui: &Ui, painter: &Painter, origin: Pos2) {
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
                                ui.visuals().text_color()
                            },
                        );
                    }
                }
            }
        }
    }
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
    fn retina_glyph_baselines_match_logical_layout_rows() {
        let layout = Shaper::default().layout("one\ntwo\nthree");
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
