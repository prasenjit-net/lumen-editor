//! Orientation cues drawn behind the text: the current-line band and indent
//! guides. Display only; nothing here reads or changes selections or history.
//!
//! The functions work on rows (one per visible source line, since the surface
//! does not wrap), through a closure that gives each row's raw indent level.
//! They know nothing about egui, so the rules are testable on plain strings.

/// Which cues to draw. Both are off by default for library users; the
/// application turns them on from its settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cues {
    /// A faint band behind each caret's line, and a brighter line number.
    pub current_line: bool,
    /// Vertical guides at each indentation step.
    pub indent_guides: bool,
}

/// Rows searched above and below a blank row for the indentation it continues.
const BLANK_SEARCH: usize = 200;
/// Rows searched above and below the caret for the end of its block.
const BLOCK_SEARCH: usize = 2000;

/// Leading whitespace of `row` in columns, with tabs advancing to the next
/// multiple of `width`. `None` for a blank row (empty or only whitespace).
pub(crate) fn leading_columns(row: &str, width: usize) -> Option<usize> {
    let width = width.max(1);
    let mut columns = 0;
    for ch in row.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => columns += width - columns % width,
            '\r' => {}
            _ => return Some(columns),
        }
    }
    None
}

/// Indentation steps of a row: its columns divided by the indent width.
pub(crate) fn level_of(row: &str, width: usize) -> Option<usize> {
    leading_columns(row, width).map(|columns| columns / width.max(1))
}

/// The level to draw guides to for `row`. A blank row continues the smaller of
/// the indentation above and below it, so a guide runs unbroken through blank
/// lines inside a block and stops at the block's end.
pub(crate) fn resolved_level(
    raw: &dyn Fn(usize) -> Option<usize>,
    rows: usize,
    row: usize,
) -> usize {
    if let Some(level) = raw(row) {
        return level;
    }
    let above = (row.saturating_sub(BLANK_SEARCH)..row).rev().find_map(raw);
    let below = (row + 1..rows.min(row + 1 + BLANK_SEARCH)).find_map(raw);
    match (above, below) {
        (Some(above), Some(below)) => above.min(below),
        _ => 0,
    }
}

/// The block around the caret: the guide index to emphasise and the rows it
/// covers. `None` when the caret's row is not indented.
pub(crate) fn enclosing_block(
    raw: &dyn Fn(usize) -> Option<usize>,
    rows: usize,
    caret_row: usize,
) -> Option<(usize, std::ops::RangeInclusive<usize>)> {
    if caret_row >= rows {
        return None;
    }
    let level = resolved_level(raw, rows, caret_row);
    if level == 0 {
        return None;
    }
    let mut first = caret_row;
    while first > 0
        && caret_row - first < BLOCK_SEARCH
        && resolved_level(raw, rows, first - 1) >= level
    {
        first -= 1;
    }
    let mut last = caret_row;
    while last + 1 < rows
        && last - caret_row < BLOCK_SEARCH
        && resolved_level(raw, rows, last + 1) >= level
    {
        last += 1;
    }
    Some((level - 1, first..=last))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Vec<&str> {
        text.split('\n').collect()
    }

    #[test]
    fn leading_columns_count_spaces_and_tab_stops() {
        assert_eq!(leading_columns("    x", 4), Some(4));
        assert_eq!(leading_columns("\tx", 4), Some(4));
        assert_eq!(
            leading_columns("  \tx", 4),
            Some(4),
            "a tab reaches the next stop"
        );
        assert_eq!(leading_columns("x", 4), Some(0));
        assert_eq!(leading_columns("", 4), None);
        assert_eq!(leading_columns("    ", 4), None, "whitespace only is blank");
        assert_eq!(level_of("        x", 4), Some(2));
        assert_eq!(
            level_of("      x", 4),
            Some(1),
            "a partial step is not a level"
        );
        assert_eq!(level_of("  x", 2), Some(1), "the width is the caller's");
    }

    #[test]
    fn a_blank_row_continues_the_smaller_neighbouring_indent() {
        let text = rows("fn f() {\n    a\n\n    b\n}\n\nfn g() {\n    c\n}");
        let raw = |row: usize| level_of(text[row], 4);
        let level = |row| resolved_level(&raw, text.len(), row);
        assert_eq!(level(2), 1, "inside the body");
        assert_eq!(level(5), 0, "between items, after a closing brace");
        assert_eq!(level(1), 1);
    }

    #[test]
    fn blank_rows_at_the_edges_have_no_guides() {
        let text = rows("\n    a\n");
        let raw = |row: usize| level_of(text[row], 4);
        assert_eq!(resolved_level(&raw, text.len(), 0), 0);
        assert_eq!(resolved_level(&raw, text.len(), 2), 0);
    }

    #[test]
    fn the_enclosing_block_runs_through_blank_rows_and_stops_at_its_end() {
        let text = rows("fn f() {\n    if x {\n        a\n\n        b\n    }\n}");
        let raw = |row: usize| level_of(text[row], 4);
        // Caret on `a` (row 2, level 2): the inner guide spans rows 2..=4.
        assert_eq!(enclosing_block(&raw, text.len(), 2), Some((1, 2..=4)));
        // Caret on `if x {` (row 1, level 1): the outer guide spans rows 1..=5.
        assert_eq!(enclosing_block(&raw, text.len(), 1), Some((0, 1..=5)));
        // Caret on a row at level 0 has no block.
        assert_eq!(enclosing_block(&raw, text.len(), 0), None);
        assert_eq!(enclosing_block(&raw, text.len(), 99), None);
    }

    #[test]
    fn long_blocks_are_searched_only_a_bounded_distance() {
        let body = "    x\n".repeat(BLOCK_SEARCH * 3);
        let text = rows(&body);
        let raw = |row: usize| level_of(text[row], 4);
        let (_, block) = enclosing_block(&raw, text.len(), BLOCK_SEARCH * 3 / 2).unwrap();
        assert!(block.end() - block.start() <= BLOCK_SEARCH * 2 + 1);
    }
}
