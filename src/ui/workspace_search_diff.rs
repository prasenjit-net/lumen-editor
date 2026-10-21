//! Compact, read-only diff rows for workspace replacement previews.
use crate::services::workspace_search::{SearchFile, SearchHit};
use eframe::egui::{self, Color32, FontId, Frame, Margin, RichText, ScrollArea, TextFormat};
use std::ops::Range;

const MAX_FULL_HUNK_BYTES: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffKind {
    Context,
    Removed,
    Added,
}

#[derive(Debug, PartialEq, Eq)]
struct DiffSegment {
    text: String,
    changed: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct DiffRow {
    kind: DiffKind,
    segments: Vec<DiffSegment>,
}

impl DiffRow {
    fn new(kind: DiffKind, segments: Vec<DiffSegment>) -> Self {
        Self { kind, segments }
    }
}

fn segment(text: impl Into<String>, changed: bool) -> DiffSegment {
    DiffSegment {
        text: text.into(),
        changed,
    }
}

pub(super) fn show_hunk(
    ui: &mut egui::Ui,
    file: &SearchFile,
    hits: &[SearchHit],
    range: Range<usize>,
    replacement: &str,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let Some(first) = hits.first() else { return };
    let id = egui::Id::new((
        "workspace_replace_diff",
        file.path.as_path(),
        first.range.start,
    ));
    let rows = if range.end.saturating_sub(range.start) > MAX_FULL_HUNK_BYTES {
        ui.label(
            RichText::new("Long group · excerpts around each match")
                .color(colors.muted)
                .size(crate::ui::theme::META_SIZE),
        );
        let Some(rows) = hits
            .iter()
            .map(|hit| excerpt_rows(&file.original, hit, replacement))
            .collect::<Option<Vec<_>>>()
        else {
            ui.label(
                RichText::new("Preview unavailable: invalid match ranges").color(colors.muted),
            );
            return;
        };
        rows.into_iter().flatten().collect()
    } else {
        match diff_rows(&file.original, hits, range, replacement) {
            Some(rows) => rows,
            None => {
                ui.label(
                    RichText::new("Preview unavailable: invalid match ranges").color(colors.muted),
                );
                return;
            }
        }
    };
    for (index, row) in rows.iter().enumerate() {
        show_row(ui, row, id.with(index));
    }
}

fn diff_rows(
    text: &str,
    hits: &[SearchHit],
    range: Range<usize>,
    replacement: &str,
) -> Option<Vec<DiffRow>> {
    let source = text.get(range.clone())?;
    let mut previous = range.start;
    for hit in hits {
        if hit.range.start < previous || hit.range.start >= hit.range.end {
            return None;
        }
        text.get(hit.range.clone())?;
        if hit.range.end > range.end {
            return None;
        }
        previous = hit.range.end;
    }
    let mut rows = Vec::new();
    let mut offset = range.start;
    let mut next_hit = 0;
    for line in source.split_inclusive('\n') {
        let without_newline = line.strip_suffix('\n').unwrap_or(line);
        let content = without_newline
            .strip_suffix('\r')
            .unwrap_or(without_newline);
        let end = offset + content.len();
        let mut removed = Vec::new();
        let mut added = Vec::new();
        let mut cursor = offset;
        while let Some(hit) = hits.get(next_hit)
            && hit.range.start < offset + line.len()
        {
            if hit.range.start < cursor || hit.range.end > end {
                return None;
            }
            let unchanged = text.get(cursor..hit.range.start)?;
            removed.push(segment(unchanged, false));
            added.push(segment(unchanged, false));
            removed.push(segment(text.get(hit.range.clone())?, true));
            added.push(segment(replacement, true));
            cursor = hit.range.end;
            next_hit += 1;
        }
        if removed.is_empty() {
            rows.push(DiffRow::new(
                DiffKind::Context,
                vec![segment(content, false)],
            ));
        } else {
            let tail = text.get(cursor..end)?;
            removed.push(segment(tail, false));
            added.push(segment(tail, false));
            rows.push(DiffRow::new(DiffKind::Removed, removed));
            rows.push(DiffRow::new(DiffKind::Added, added));
        }
        offset += line.len();
    }
    (next_hit == hits.len()).then_some(rows)
}

fn excerpt_rows(text: &str, hit: &SearchHit, replacement: &str) -> Option<[DiffRow; 2]> {
    let found = text.get(hit.range.clone())?;
    let line_start = text[..hit.range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = text[hit.range.end..]
        .find('\n')
        .map_or(text.len(), |index| hit.range.end + index);
    let prefix = &text[line_start..hit.range.start];
    let suffix = text[hit.range.end..line_end].trim_end_matches('\r');
    let tail: String = prefix
        .chars()
        .rev()
        .take(70)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let head: String = suffix.chars().take(70).collect();
    let left = if tail.chars().count() < prefix.chars().count() {
        "…"
    } else {
        ""
    };
    let right = if head.chars().count() < suffix.chars().count() {
        "…"
    } else {
        ""
    };
    let context_before = format!("{left}{tail}");
    let context_after = format!("{head}{right}");
    let make_row = |kind, changed: &str| {
        DiffRow::new(
            kind,
            vec![
                segment(&context_before, false),
                segment(leading_excerpt(changed, 160), true),
                segment(&context_after, false),
            ],
        )
    };
    Some([
        make_row(DiffKind::Removed, found),
        make_row(DiffKind::Added, replacement),
    ])
}

fn leading_excerpt(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let excerpt: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{excerpt}…")
    } else {
        excerpt
    }
}

fn show_row(ui: &mut egui::Ui, row: &DiffRow, id: egui::Id) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let (marker, background, color) = match row.kind {
        DiffKind::Context => (" ", colors.panel, colors.muted),
        DiffKind::Removed => ("−", colors.diff_removed_bg, colors.diff_removed_text),
        DiffKind::Added => ("+", colors.diff_added_bg, colors.diff_added_text),
    };
    let width = ui.available_width();
    Frame::new()
        .fill(background)
        .inner_margin(Margin::symmetric(4, 1))
        .show(ui, |ui| {
            ui.set_min_width((width - 8.0).max(0.0));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(marker).monospace().size(12.0).color(color));
                let mut job = egui::text::LayoutJob::default();
                for part in &row.segments {
                    let (text_color, highlight) = if part.changed {
                        (
                            color,
                            match row.kind {
                                DiffKind::Removed => colors.diff_removed_highlight,
                                DiffKind::Added => colors.diff_added_highlight,
                                DiffKind::Context => Color32::TRANSPARENT,
                            },
                        )
                    } else {
                        (
                            if row.kind == DiffKind::Context {
                                colors.muted
                            } else {
                                colors.text
                            },
                            Color32::TRANSPARENT,
                        )
                    };
                    job.append(
                        &part.text,
                        0.0,
                        TextFormat {
                            font_id: FontId::monospace(12.0),
                            color: text_color,
                            background: highlight,
                            ..Default::default()
                        },
                    );
                }
                ScrollArea::horizontal()
                    .id_salt(id)
                    .max_width(ui.available_width())
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(job).extend().halign(egui::Align::Min));
                    });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(range: Range<usize>) -> SearchHit {
        SearchHit {
            range,
            line: 1,
            column: 1,
            snippet: String::new(),
        }
    }

    #[test]
    fn grouped_diff_marks_only_replaced_spans_and_keeps_crlf_context() {
        let text = "before\r\nα needle and needle\r\nafter\r\n";
        let first = text.find("needle").unwrap();
        let second = text.rfind("needle").unwrap();
        let rows = diff_rows(
            text,
            &[hit(first..first + 6), hit(second..second + 6)],
            0..text.len(),
            "tea",
        )
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            [
                DiffKind::Context,
                DiffKind::Removed,
                DiffKind::Added,
                DiffKind::Context,
            ]
        );
        assert_eq!(rows[0].segments[0].text, "before");
        assert_eq!(rows[3].segments[0].text, "after");
        assert_eq!(
            rows[1].segments.iter().filter(|part| part.changed).count(),
            2
        );
        assert_eq!(
            rows[2].segments.iter().filter(|part| part.changed).count(),
            2
        );
        assert_eq!(
            rows[1]
                .segments
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            "α needle and needle"
        );
        assert_eq!(
            rows[2]
                .segments
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            "α tea and tea"
        );
    }

    #[test]
    fn long_line_excerpt_highlights_unicode_match_and_replacement() {
        let prefix = "α".repeat(100);
        let text = format!("{prefix}needle{}\r\n", "β".repeat(100));
        let rows =
            excerpt_rows(&text, &hit(prefix.len()..prefix.len() + 6), "replacement").unwrap();
        assert!(rows[0].segments[0].text.starts_with('…'));
        assert_eq!(rows[0].segments[1], segment("needle", true));
        assert_eq!(rows[1].segments[1], segment("replacement", true));
        assert!(rows[1].segments[2].text.ends_with('…'));
        assert!(
            rows.iter()
                .all(|row| row.segments.iter().all(|part| !part.text.contains('\r')))
        );
    }

    #[test]
    fn preview_paints_distinct_added_and_removed_rows() {
        let original = "before\nneedle\nafter\n";
        let start = original.find("needle").unwrap();
        let file = SearchFile {
            path: "/workspace/example.txt".into(),
            original: original.into(),
            hits: vec![hit(start..start + "needle".len())],
        };
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 300.0),
                )),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    show_hunk(ui, &file, &file.hits, 0..original.len(), "tea");
                });
            },
        );
        let fills: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) => Some(rect.fill),
                _ => None,
            })
            .collect();
        assert!(fills.contains(&crate::ui::theme::Theme::Dark.palette().diff_removed_bg));
        assert!(fills.contains(&crate::ui::theme::Theme::Dark.palette().diff_added_bg));
    }
}
