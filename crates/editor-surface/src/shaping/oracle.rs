//! Test-only copy of the original whole-document layout. The production layout
//! shapes lines lazily; this one shapes everything up front exactly as before, so
//! randomized tests can prove both agree on carets, bidirectional stepping, hit
//! testing and selection.
use super::*;

impl Shaper {
    pub(crate) fn oracle_layout(&self, text: &str, typography: crate::Typography) -> OracleLayout {
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
        OracleLayout {
            buffer,
            starts: std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
            character_carets: OnceLock::new(),
            visual_stops: OnceLock::new(),
        }
    }
}

pub(crate) struct OracleLayout {
    pub(crate) buffer: Buffer,
    pub(crate) starts: Vec<usize>,
    character_carets: OnceLock<Vec<Pos2>>,
    visual_stops: OnceLock<VisualStops>,
}

type VisualStop = (usize, CaretAffinity, Pos2);
struct VisualStops {
    left: Vec<VisualStop>,
    right: Vec<VisualStop>,
}

impl OracleLayout {
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
}
