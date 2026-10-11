//! Literal search results use validated UTF-8 byte ranges, never screen columns.
use std::ops::Range;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SearchOptions {
    pub(crate) match_case: bool,
    pub(crate) whole_word: bool,
}

fn word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

pub(crate) fn find_matches(text: &str, query: &str, options: SearchOptions) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let whole_word = |range: &Range<usize>| {
        !options.whole_word
            || (!text[..range.start]
                .chars()
                .next_back()
                .is_some_and(word_character)
                && !text[range.end..].chars().next().is_some_and(word_character))
    };
    if options.match_case {
        return text
            .match_indices(query)
            .map(|(start, found)| start..start + found.len())
            .filter(whole_word)
            .collect();
    }
    // Lowercasing can expand a character (e.g. İ). Keep a boundary map so a
    // match can never return an offset inside the original UTF-8 character.
    let mut folded = String::new();
    let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
    for (original, character) in text.char_indices() {
        boundaries.push((folded.len(), original));
        folded.extend(character.to_lowercase());
    }
    boundaries.push((folded.len(), text.len()));
    let needle = query.to_lowercase();
    folded
        .match_indices(&needle)
        .filter_map(|(start, found)| {
            let start = boundaries
                .binary_search_by_key(&start, |entry| entry.0)
                .ok()?;
            let end = boundaries
                .binary_search_by_key(&(boundaries[start].0 + found.len()), |entry| entry.0)
                .ok()?;
            Some(boundaries[start].1..boundaries[end].1)
        })
        .filter(whole_word)
        .collect()
}

pub(crate) fn replace_matches(
    text: &str,
    ranges: &[Range<usize>],
    replacement: &str,
) -> Option<String> {
    let mut result = String::new();
    let mut previous = 0;
    for range in ranges {
        if range.start < previous || range.start >= range.end {
            return None;
        }
        result.push_str(text.get(previous..range.start)?);
        text.get(range.clone())?;
        result.push_str(replacement);
        previous = range.end;
    }
    result.push_str(text.get(previous..)?);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_matches_and_expansion_return_original_byte_boundaries() {
        let options = SearchOptions::default();
        assert_eq!(find_matches("É é 🌍", "é", options), vec![0..2, 3..5]);
        assert_eq!(find_matches("İ", "i", options), Vec::<Range<usize>>::new());
        assert_eq!(find_matches("İ", "i\u{307}", options), vec![0..2]);
    }
    #[test]
    fn options_empty_queries_and_nonoverlap_are_explicit() {
        assert!(find_matches("text", "", SearchOptions::default()).is_empty());
        assert_eq!(
            find_matches("aaaa", "aa", SearchOptions::default()),
            vec![0..2, 2..4]
        );
        let options = SearchOptions {
            match_case: true,
            whole_word: true,
        };
        assert_eq!(
            find_matches("cat Cat cat_cat cat!", "cat", options),
            vec![0..3, 16..19]
        );
    }
    #[test]
    fn replace_all_is_snapshot_based_and_rejects_invalid_ranges() {
        assert_eq!(
            replace_matches("é é", &[0..2, 3..5], "éé"),
            Some("éé éé".into())
        );
        assert!(replace_matches("é", std::slice::from_ref(&(1..2)), "x").is_none());
        assert!(replace_matches("abc", &[0..2, 1..3], "x").is_none());
        assert_eq!(
            replace_matches("a\r\na", &[0..1, 3..4], "b"),
            Some("b\r\nb".into())
        );
    }
}
