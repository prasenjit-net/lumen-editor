use egui::{Galley, Pos2, Rect, Ui, Vec2, text::CCursor};
use lumen_editor_core::syntax;
use lumen_editor_core::{CaretAffinity, Composition};
use lumen_language_services::{DocumentVersion, TokenClass};
use std::ops::Range;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct LayoutConfig {
    pub(crate) typography: crate::Typography,
    pub(crate) language: Option<syntax::Language>,
    pub(crate) syntax: Option<(syntax::Language, crate::SyntaxPalette)>,
    /// Validated language-server spans in source byte coordinates.
    pub(crate) semantic: Option<Arc<Vec<lumen_language_services::TokenSpan>>>,
}

/// Syntax spans consumed by the renderer. Rust and Go use parser-backed
/// language-service tokens; the existing lightweight lexer remains the safe
/// fallback for every other language and for parser failures.
#[derive(Clone, Debug)]
pub(crate) struct SyntaxSpan {
    pub(crate) range: Range<usize>,
    class: SyntaxClass,
}

#[derive(Clone, Copy, Debug)]
enum SyntaxClass {
    Legacy(syntax::Kind),
    Language(TokenClass),
}

impl SyntaxSpan {
    fn legacy(span: syntax::Span) -> Self {
        Self {
            range: span.range,
            class: SyntaxClass::Legacy(span.kind),
        }
    }

    fn language(range: Range<usize>, class: TokenClass) -> Self {
        Self {
            range,
            class: SyntaxClass::Language(class),
        }
    }

    pub(crate) fn color(&self, palette: crate::SyntaxPalette) -> egui::Color32 {
        match self.class {
            SyntaxClass::Legacy(kind) => palette.color(kind),
            SyntaxClass::Language(class) => palette.language_token_color(class),
        }
    }

    fn excludes_bracket_matching(&self) -> bool {
        matches!(
            self.class,
            SyntaxClass::Legacy(syntax::Kind::Comment | syntax::Kind::String)
                | SyntaxClass::Language(
                    TokenClass::Comment | TokenClass::String | TokenClass::Escape
                )
        )
    }
}

pub(crate) fn highlight(
    visual: &str,
    language: syntax::Language,
    semantic: &[lumen_language_services::TokenSpan],
) -> Vec<SyntaxSpan> {
    if let Ok(Some(snapshot)) = lumen_language_services::highlight::highlight_registered(
        visual,
        language.label(),
        DocumentVersion::default(),
    ) {
        return lumen_language_services::overlay_semantic(snapshot.spans, semantic)
            .into_iter()
            .map(|span| SyntaxSpan::language(span.range, span.class))
            .collect();
    }
    syntax::highlight(visual, language)
        .into_iter()
        .map(SyntaxSpan::legacy)
        .collect()
}

/// Project source-coordinate spans into the visual (CRLF-normalized, folded)
/// text. Spans that start or end inside a fold, or cross a visual line, are dropped.
fn visual_semantic_spans(
    spans: &[lumen_language_services::TokenSpan],
    byte_offsets: &[usize],
    visual_offsets: &[usize],
    visual: &str,
) -> Vec<lumen_language_services::TokenSpan> {
    spans
        .iter()
        .filter_map(|span| {
            let start = visual_offsets[byte_offsets.binary_search(&span.range.start).ok()?];
            let end = visual_offsets[byte_offsets.binary_search(&span.range.end).ok()?];
            (start < end && !visual[start..end].contains('\n')).then_some({
                lumen_language_services::TokenSpan {
                    range: start..end,
                    class: span.class,
                }
            })
        })
        .collect()
}

/// One source-to-layout mapping for painting and pointer/caret geometry.
/// CRLF projects to one visual newline without altering the stored document.
///
/// Cheap to clone: the per-character tables are shared, so an unchanged document
/// reuses one layout across frames instead of rebuilding them every frame.
#[derive(Clone)]
pub struct TextGeometry {
    pub galley: Arc<Galley>,
    pub origin: Pos2,
    byte_offsets: Arc<Vec<usize>>,
    graphemes: Arc<Vec<usize>>,
    preedit: Option<(Range<usize>, usize)>,
    syntax_spans: Arc<Vec<SyntaxSpan>>,
    #[cfg(feature = "shaped")]
    syntax_palette: Option<crate::SyntaxPalette>,
    #[cfg(feature = "shaped")]
    pub(crate) shaped: Option<Arc<crate::shaping::ShapedLayout>>,
    visual_offsets: Arc<Vec<usize>>,
    folded: Arc<Vec<Range<usize>>>,
    source_lines: usize,
    row_lines: Arc<Vec<usize>>,
}

impl TextGeometry {
    pub fn layout(ui: &Ui, text: &str, origin: Pos2) -> Self {
        Self::layout_typography(
            ui,
            text,
            origin,
            None,
            LayoutConfig {
                typography: crate::Typography::default(),
                language: None,
                syntax: None,
                semantic: None,
            },
            &[],
        )
    }

    pub(crate) fn layout_typography(
        ui: &Ui,
        text: &str,
        origin: Pos2,
        composition: Option<&Composition>,
        config: LayoutConfig,
        folded: &[Range<usize>],
    ) -> Self {
        let LayoutConfig {
            typography,
            language,
            syntax,
            semantic,
        } = config;
        // One transient layout replaces the primary range. Stored source and
        // history remain untouched; neighboring text participates in this galley.
        let preedit = composition.filter(|composition| !composition.preedit().is_empty());
        let projected = preedit.map(|composition| {
            let range = composition.replacement_range();
            format!(
                "{}{}{}",
                &text[..range.start],
                composition.preedit(),
                &text[range.end..]
            )
        });
        let layout_text = projected.as_deref().unwrap_or(text);
        let mut visual = String::new();
        let mut byte_offsets = vec![0];
        let mut characters = layout_text.char_indices().peekable();
        let mut folds = folded.iter().peekable();
        while let Some((offset, character)) = characters.next() {
            if folds.peek().is_some_and(|range| range.start == offset) {
                let end = folds.next().unwrap().end;
                while characters.peek().is_some_and(|(byte, _)| *byte < end) {
                    characters.next();
                }
                if end < layout_text.len() {
                    visual.push('\n');
                    byte_offsets.push(end);
                } else {
                    *byte_offsets.last_mut().unwrap() = end;
                }
                continue;
            }
            if character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n') {
                characters.next();
                visual.push('\n');
                byte_offsets.push(offset + 2);
            } else {
                visual.push(character);
                byte_offsets.push(offset + character.len_utf8());
            }
        }
        let visual_offsets: Vec<usize> = visual
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([visual.len()])
            .collect();
        // Source offsets from the server only line up with unprojected text.
        let semantic_spans = match (&semantic, preedit) {
            (Some(spans), None) => {
                visual_semantic_spans(spans, &byte_offsets, &visual_offsets, &visual)
            }
            _ => Vec::new(),
        };
        let syntax_spans = language
            .or_else(|| syntax.map(|(language, _)| language))
            .map_or_else(Vec::new, |language| {
                highlight(&visual, language, &semantic_spans)
            });
        let galley = ui.fonts_mut(|fonts| {
            let mut job = egui::text::LayoutJob::simple(
                visual,
                typography.font_id(),
                ui.visuals().text_color(),
                f32::INFINITY,
            );
            job.sections[0].format.line_height = Some(typography.line_height);
            if let Some((_, palette)) = syntax {
                let base = job.sections[0].format.clone();
                job.sections.clear();
                let mut position = 0;
                for span in &syntax_spans {
                    if position < span.range.start {
                        job.sections.push(egui::text::LayoutSection {
                            leading_space: 0.0,
                            byte_range: position..span.range.start,
                            format: base.clone(),
                        });
                    }
                    let mut format = base.clone();
                    format.color = span.color(palette);
                    job.sections.push(egui::text::LayoutSection {
                        leading_space: 0.0,
                        byte_range: span.range.clone(),
                        format,
                    });
                    position = span.range.end;
                }
                if position < job.text.len() || job.sections.is_empty() {
                    job.sections.push(egui::text::LayoutSection {
                        leading_space: 0.0,
                        byte_range: position..job.text.len(),
                        format: base,
                    });
                }
            }
            fonts.layout_job(job)
        });
        let line_starts: Vec<_> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(byte, _)| byte + 1))
            .collect();
        let mut character = 0;
        let row_lines = galley
            .rows
            .iter()
            .map(|row| {
                let byte = byte_offsets[character.min(byte_offsets.len() - 1)];
                let line = line_starts.partition_point(|start| *start <= byte);
                character += row.char_count_including_newline();
                line
            })
            .collect();
        Self {
            galley,
            origin,
            byte_offsets: Arc::new(byte_offsets),
            graphemes: Arc::new(
                text.grapheme_indices(true)
                    .map(|(offset, _)| offset)
                    .chain(std::iter::once(text.len()))
                    .collect(),
            ),
            preedit: preedit
                .map(|composition| (composition.replacement_range(), composition.preedit().len())),
            syntax_spans: Arc::new(syntax_spans),
            #[cfg(feature = "shaped")]
            syntax_palette: syntax.map(|(_, palette)| palette),
            #[cfg(feature = "shaped")]
            shaped: None,
            visual_offsets: Arc::new(visual_offsets),
            folded: Arc::new(folded.to_vec()),
            source_lines: line_starts.len(),
            row_lines: Arc::new(row_lines),
        }
    }

    pub fn cursor(&self, byte: usize) -> CCursor {
        self.visual_cursor(self.project_byte(byte))
    }

    fn project_byte(&self, byte: usize) -> usize {
        let byte = match &self.preedit {
            Some((range, length)) if byte >= range.end && byte > range.start => {
                byte - range.len() + length
            }
            Some((range, _)) if byte >= range.start => range.start,
            _ => byte,
        };
        for range in self.folded.iter() {
            if range.start <= byte && byte < range.end {
                return if self.byte_offsets.binary_search(&range.start).is_ok() {
                    range.start
                } else {
                    range.end
                };
            }
        }
        byte
    }

    fn visual_cursor(&self, byte: usize) -> CCursor {
        CCursor::new(
            self.byte_offsets
                .binary_search(&byte)
                .expect("validated source boundary"),
        )
    }

    pub fn caret_rect(&self, byte: usize) -> Rect {
        self.visual_caret(self.cursor(byte).index)
    }

    pub fn caret_with_affinity(&self, byte: usize, affinity: CaretAffinity) -> Rect {
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            return shaped.caret_with_affinity(
                self.visual_byte(self.cursor(byte).index),
                affinity,
                self.origin,
            );
        }
        let _ = affinity;
        self.caret_rect(byte)
    }

    pub(crate) fn pointer_affinity(&self, pointer: Pos2, byte: usize) -> CaretAffinity {
        let before = self.caret_with_affinity(byte, CaretAffinity::Before);
        let after = self.caret_with_affinity(byte, CaretAffinity::After);
        if before.center().distance_sq(pointer) < after.center().distance_sq(pointer) {
            CaretAffinity::Before
        } else {
            CaretAffinity::After
        }
    }

    pub(crate) fn visual_step(
        &self,
        byte: usize,
        affinity: CaretAffinity,
        right: bool,
    ) -> Option<(usize, CaretAffinity)> {
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            let (target, affinity) = shaped.visual_step(
                self.galley.text(),
                self.visual_byte(self.cursor(byte).index),
                affinity,
                right,
            )?;
            let index = self.visual_offsets.binary_search(&target).ok()?;
            let source = self.source_byte(self.byte_offsets[index])?;
            return self
                .graphemes
                .binary_search(&source)
                .is_ok()
                .then_some((source, affinity));
        }
        let _ = (byte, affinity, right);
        None
    }

    pub(crate) fn is_shaped(&self) -> bool {
        #[cfg(feature = "shaped")]
        {
            self.shaped.is_some()
        }
        #[cfg(not(feature = "shaped"))]
        {
            false
        }
    }

    pub(crate) fn visual_caret(&self, index: usize) -> Rect {
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            return shaped.caret(self.visual_byte(index), self.origin);
        }
        self.galley
            .pos_from_cursor(CCursor::new(index))
            .translate(self.origin.to_vec2())
    }

    pub fn hit_test(&self, pointer: Pos2) -> usize {
        let index = self.hit_character(pointer);
        let byte = self.byte_offsets[index.min(self.byte_offsets.len() - 1)];
        let byte = self.source_byte(byte).unwrap_or_else(|| {
            let (range, _) = self.preedit.as_ref().unwrap();
            if self.caret_rect(range.start).center().distance_sq(pointer)
                < self
                    .preedit_rects()
                    .unwrap()
                    .1
                    .center()
                    .distance_sq(pointer)
            {
                range.start
            } else {
                range.end
            }
        });
        match self.graphemes.binary_search(&byte) {
            Ok(_) => byte,
            Err(index) => {
                let before = self.graphemes[index.saturating_sub(1)];
                let after = self.graphemes[index.min(self.graphemes.len() - 1)];
                if self.caret_rect(before).center().distance_sq(pointer)
                    < self.caret_rect(after).center().distance_sq(pointer)
                {
                    before
                } else {
                    after
                }
            }
        }
    }

    pub fn size(&self) -> Vec2 {
        #[cfg(feature = "shaped")]
        if self.shaped.is_some() {
            return self.row_rects().iter().fold(Vec2::ZERO, |size, row| {
                egui::vec2(
                    size.x.max(row.width()),
                    size.y.max(row.bottom() - self.origin.y),
                )
            });
        }
        self.galley.size()
    }

    pub fn row_count(&self) -> usize {
        self.row_rects().len()
    }

    pub(crate) fn source_line_count(&self) -> usize {
        self.source_lines
    }

    pub(crate) fn source_line_for_row(&self, row: usize) -> usize {
        self.row_lines[row]
    }

    pub fn row_rects(&self) -> Vec<Rect> {
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            return shaped.rows(self.origin);
        }
        self.galley
            .rows
            .iter()
            .map(|row| {
                Rect::from_min_max(
                    self.origin + egui::vec2(0.0, row.min_y()),
                    self.origin + egui::vec2(row.size.x, row.max_y()),
                )
            })
            .collect()
    }

    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let index = |byte| {
            self.byte_offsets
                .partition_point(|offset| *offset <= self.project_byte(byte))
                .saturating_sub(1)
        };
        let start = index(range.start);
        let end = index(range.end);
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            return shaped.selection(self.visual_byte(start)..self.visual_byte(end), self.origin);
        }
        let mut offset = 0;
        self.galley
            .rows
            .iter()
            .filter_map(|row| {
                let left = start.max(offset);
                let right = end.min(offset + row.char_count_excluding_newline());
                let rect = (left < right).then(|| {
                    Rect::from_min_max(
                        self.origin + egui::vec2(row.x_offset(left - offset), row.min_y()),
                        self.origin + egui::vec2(row.x_offset(right - offset), row.max_y()),
                    )
                });
                offset += row.char_count_including_newline();
                rect
            })
            .collect()
    }

    pub(crate) fn is_code_byte(&self, source: usize) -> bool {
        let projected = self.project_byte(source);
        let Ok(index) = self.byte_offsets.binary_search(&projected) else {
            return false;
        };
        let visual = self.visual_offsets[index];
        let span = self.syntax_spans.get(
            self.syntax_spans
                .partition_point(|span| span.range.end <= visual),
        );
        !span.is_some_and(|span| span.range.contains(&visual) && span.excludes_bracket_matching())
    }

    pub(crate) fn paint(&self, ui: &Ui, painter: &egui::Painter) {
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            shaped.paint(
                ui,
                painter,
                self.origin,
                &self.syntax_spans,
                self.syntax_palette,
            );
            return;
        }
        painter.galley(self.origin, self.galley.clone(), ui.visuals().text_color());
    }

    fn hit_character(&self, pointer: Pos2) -> usize {
        #[cfg(feature = "shaped")]
        if let Some(shaped) = &self.shaped {
            let byte = shaped.hit(self.galley.text(), pointer - self.origin);
            return self
                .visual_offsets
                .binary_search(&byte)
                .expect("shaped source character");
        }
        self.galley.cursor_from_pos(pointer - self.origin).index
    }

    #[cfg(feature = "shaped")]
    pub(crate) fn visual_byte(&self, character: usize) -> usize {
        self.visual_offsets[character]
    }

    fn source_byte(&self, byte: usize) -> Option<usize> {
        match &self.preedit {
            Some((range, length)) if byte > range.start && byte < range.start + length => None,
            Some((range, length)) if byte >= range.start + length => {
                Some(byte - length + range.len())
            }
            _ => Some(byte),
        }
    }

    pub(crate) fn preedit_rects(&self) -> Option<(Rect, Rect)> {
        let (range, length) = self.preedit.as_ref()?;
        let start = self.visual_caret(self.visual_cursor(range.start).index);
        let end = self.visual_caret(self.visual_cursor(range.start + length).index);
        Some((start, end))
    }

    pub(crate) fn accessibility_byte(
        &self,
        widget: egui::Id,
        position: &egui::accesskit::TextPosition,
    ) -> Option<usize> {
        let mut index = 0;
        for (row_index, row) in self.galley.rows.iter().enumerate() {
            let count = row.char_count_including_newline();
            if egui::accesskit::NodeId(widget.with(row_index).value()) == position.node {
                if position.character_index > count {
                    return None;
                }
                let byte =
                    self.source_byte(*self.byte_offsets.get(index + position.character_index)?)?;
                return self.graphemes.binary_search(&byte).is_ok().then_some(byte);
            }
            index += count;
        }
        None
    }
}

#[cfg(test)]
mod semantic_projection_tests {
    use super::*;
    use lumen_language_services::TokenSpan;

    fn span(range: Range<usize>) -> TokenSpan {
        TokenSpan {
            range,
            class: TokenClass::Function,
        }
    }

    /// Mirrors the visual/source tables built by `layout_typography` for "a\r\nbc".
    #[test]
    fn crlf_and_unmappable_spans_project_to_visual_text() {
        let visual = "a\nbc";
        // source boundaries after each visual char: a=1, \r\n=3, b=4, c=5
        let byte_offsets = [0, 1, 3, 4, 5];
        let visual_offsets: Vec<usize> = visual
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([visual.len()])
            .collect();
        let projected = visual_semantic_spans(
            &[span(3..5), span(0..1), span(1..3), span(2..4), span(0..5)],
            &byte_offsets,
            &visual_offsets,
            visual,
        );
        // 1..3 is the CRLF itself (a newline), 2..4 splits the CRLF, 0..5 crosses lines.
        assert_eq!(
            projected
                .iter()
                .map(|s| s.range.clone())
                .collect::<Vec<_>>(),
            vec![2..4, 0..1]
        );
    }
}
