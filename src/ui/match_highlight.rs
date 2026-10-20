//! Matched-text highlighting for quick-input result rows.
use eframe::egui::{
    self, Color32, FontId,
    text::{LayoutJob, TextFormat},
};
use std::ops::Range;

/// Byte ranges of `text` covered by the first case-insensitive occurrence of
/// each whitespace-separated word in `query`, merged and in order.
pub(crate) fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let folded: Vec<char> = chars
        .iter()
        .map(|(_, c)| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for word in query.split_whitespace() {
        let word: Vec<char> = word
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        if word.is_empty() || word.len() > folded.len() {
            continue;
        }
        let Some(start) =
            (0..=folded.len() - word.len()).find(|&i| folded[i..i + word.len()] == word[..])
        else {
            continue;
        };
        let end = chars
            .get(start + word.len())
            .map_or(text.len(), |(byte, _)| *byte);
        ranges.push(chars[start].0..end);
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// Append `text` to `job`, coloring the parts of it that match `query`.
pub(crate) fn append_highlighted(
    job: &mut LayoutJob,
    text: &str,
    query: &str,
    font: FontId,
    color: Color32,
    match_color: Color32,
) {
    let mut cursor = 0;
    let mut push = |part: &str, color: Color32| {
        if !part.is_empty() {
            job.append(
                part,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                },
            );
        }
    };
    for range in match_ranges(text, query) {
        push(&text[cursor..range.start], color);
        push(&text[range.clone()], match_color);
        cursor = range.end;
    }
    push(&text[cursor..], color);
}

/// Highlight color for matched characters; brighter than body text in each theme.
pub(crate) fn match_color(ctx: &egui::Context) -> Color32 {
    crate::ui::theme::palette(&ctx.style().visuals).info
}

#[cfg(test)]
mod tests {
    use super::match_ranges;

    #[test]
    fn highlights_each_word_once_case_insensitively() {
        assert_eq!(match_ranges("Save As…", "sav as"), vec![0..3, 5..7]);
        assert_eq!(match_ranges("editor_gutter.rs", "GUT"), vec![7..10]);
    }

    #[test]
    fn overlapping_words_merge_and_misses_are_ignored() {
        assert_eq!(match_ranges("gutter", "gut utt"), vec![0..4]);
        assert!(match_ranges("gutter", "zzz").is_empty());
        assert!(match_ranges("gutter", "  ").is_empty());
    }

    #[test]
    fn multibyte_text_stays_on_char_boundaries() {
        assert_eq!(match_ranges("é save", "save"), vec![3..7]);
    }
}
