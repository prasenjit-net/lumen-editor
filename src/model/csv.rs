//! CSV/TSV tokenizer, cell model and minimal text edits (no egui, no I/O).
//!
//! The table never owns cell text. It records the exact byte span of every
//! field in the source buffer, so a read is a slice of the document and an
//! edit is a list of small [`TextChange`]s against it. Cells that are not
//! edited are therefore byte-for-byte what they were: quoting, line endings,
//! ragged rows, a missing final newline and a BOM all survive untouched.
use std::{borrow::Cow, cmp::Ordering, ops::Range};

/// Candidates for delimiter detection, in tie-break order.
pub(crate) const DELIMITERS: [char; 4] = [',', ';', '\t', '|'];

/// Files larger than this are not parsed for the visual editor.
pub(crate) const MAX_VISUAL_BYTES: usize = 64 * 1024 * 1024;
/// Parsing at least this much text happens off the UI thread.
pub(crate) const BACKGROUND_PARSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Field {
    /// Raw bytes in the source, including any surrounding quotes.
    pub(crate) span: Range<usize>,
    pub(crate) quoted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) fields: Vec<Field>,
    /// First byte of the record.
    pub(crate) start: usize,
    /// End of the record's content, before its line terminator.
    pub(crate) end: usize,
    /// The record's `\n` or `\r\n`; empty for a final record without one.
    pub(crate) terminator: Range<usize>,
}

/// One replacement against the source text. A set of changes returned by an
/// operation is sorted and non-overlapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextChange {
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

impl TextChange {
    fn insert(at: usize, text: impl Into<String>) -> Self {
        Self {
            range: at..at,
            text: text.into(),
        }
    }

    fn replace(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }
}

/// Apply sorted, non-overlapping changes to `text`.
#[cfg(test)]
pub(crate) fn apply_changes(text: &str, changes: &[TextChange]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for change in changes {
        out.push_str(&text[at..change.range.start]);
        out.push_str(&change.text);
        at = change.range.end;
    }
    out.push_str(&text[at..]);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CsvTable {
    pub(crate) delimiter: char,
    pub(crate) records: Vec<Record>,
    /// Widest record; shorter records are kept as they are.
    pub(crate) columns: usize,
    /// Record holding a quote that never closes, if any.
    pub(crate) unterminated: Option<usize>,
    pub(crate) line_ending: &'static str,
    /// Length of the source the table was parsed from.
    pub(crate) source_len: usize,
}

impl CsvTable {
    pub(crate) fn parse(text: &str, delimiter: char) -> Self {
        debug_assert!(delimiter.is_ascii());
        let bytes = text.as_bytes();
        let delimiter_byte = delimiter as u8;
        let length = bytes.len();
        let mut position = if text.starts_with('\u{feff}') { 3 } else { 0 };
        let mut records = Vec::new();
        let mut columns = 0;
        let mut unterminated = None;
        let mut crlf = false;
        while position < length {
            let start = position;
            let mut fields = Vec::new();
            loop {
                let field_start = position;
                let mut quoted = false;
                if position < length && bytes[position] == b'"' {
                    quoted = true;
                    position += 1;
                    loop {
                        match bytes[position.min(length)..]
                            .iter()
                            .position(|byte| *byte == b'"')
                        {
                            None => {
                                position = length;
                                unterminated.get_or_insert(records.len());
                                break;
                            }
                            Some(offset) => {
                                position += offset;
                                if bytes.get(position + 1) == Some(&b'"') {
                                    position += 2;
                                } else {
                                    position += 1;
                                    break;
                                }
                            }
                        }
                    }
                }
                while position < length {
                    let byte = bytes[position];
                    if byte == delimiter_byte
                        || byte == b'\n'
                        || (byte == b'\r' && bytes.get(position + 1) == Some(&b'\n'))
                    {
                        break;
                    }
                    position += 1;
                }
                fields.push(Field {
                    span: field_start..position,
                    quoted,
                });
                if position < length && bytes[position] == delimiter_byte {
                    position += 1;
                } else {
                    break;
                }
            }
            let end = position;
            if position < length {
                if bytes[position] == b'\r' {
                    crlf = true;
                    position += 2;
                } else {
                    position += 1;
                }
            }
            columns = columns.max(fields.len());
            records.push(Record {
                fields,
                start,
                end,
                terminator: end..position,
            });
        }
        Self {
            delimiter,
            records,
            columns,
            unterminated,
            line_ending: if crlf { "\r\n" } else { "\n" },
            source_len: length,
        }
    }

    pub(crate) fn row_count(&self) -> usize {
        self.records.len()
    }

    fn ends_with_newline(&self) -> bool {
        self.records
            .last()
            .is_none_or(|record| !record.terminator.is_empty())
    }

    /// Unescaped text of a cell; `None` where a ragged record has no such cell.
    pub(crate) fn value<'a>(
        &self,
        text: &'a str,
        row: usize,
        column: usize,
    ) -> Option<Cow<'a, str>> {
        let field = self.records.get(row)?.fields.get(column)?;
        Some(unescape(&text[field.span.clone()], field.quoted))
    }

    /// Replace `grid` of values with its top-left cell at (`row`, `column`).
    /// Cells whose value already matches are left alone; cells past the end of
    /// a record or the table are appended.
    pub(crate) fn set_cells(
        &self,
        text: &str,
        row: usize,
        column: usize,
        grid: &[Vec<String>],
    ) -> Vec<TextChange> {
        let mut changes = Vec::new();
        let mut appended: Vec<String> = Vec::new();
        let delimiter = self.delimiter.to_string();
        for (offset, values) in grid.iter().enumerate() {
            let index = row + offset;
            let Some(record) = self.records.get(index) else {
                let mut line = delimiter.repeat(column);
                line.push_str(
                    &values
                        .iter()
                        .map(|value| encode(value, self.delimiter, false))
                        .collect::<Vec<_>>()
                        .join(&delimiter),
                );
                appended.push(line);
                continue;
            };
            let mut tail = String::new();
            for (position, value) in values.iter().enumerate() {
                if let Some(field) = record.fields.get(column + position)
                    && unescape(&text[field.span.clone()], field.quoted) != value.as_str()
                {
                    changes.push(TextChange::replace(
                        field.span.clone(),
                        encode(value, self.delimiter, field.quoted),
                    ));
                }
            }
            // Cells past the end of a short record are appended, padding any gap.
            for target in record.fields.len()..column + values.len() {
                tail.push_str(&delimiter);
                if let Some(value) = target.checked_sub(column).and_then(|at| values.get(at)) {
                    tail.push_str(&encode(value, self.delimiter, false));
                }
            }
            if !tail.is_empty() {
                changes.push(TextChange::insert(record.end, tail));
            }
        }
        if !appended.is_empty() {
            changes.push(self.append_lines(text, &appended));
        }
        changes.sort_by_key(|change| change.range.start);
        changes
    }

    #[cfg(test)]
    pub(crate) fn set_cell(
        &self,
        text: &str,
        row: usize,
        column: usize,
        value: &str,
    ) -> Vec<TextChange> {
        self.set_cells(text, row, column, &[vec![value.to_owned()]])
    }

    /// Lines added at the end, keeping the file's final-newline state.
    fn append_lines(&self, text: &str, lines: &[String]) -> TextChange {
        let eol = self.line_ending;
        let mut added = String::new();
        if !self.ends_with_newline() {
            added.push_str(eol);
        }
        added.push_str(&lines.join(eol));
        if self.ends_with_newline() {
            added.push_str(eol);
        }
        TextChange::insert(text.len(), added)
    }

    pub(crate) fn insert_row(&self, text: &str, at: usize) -> Vec<TextChange> {
        let blank = self
            .delimiter
            .to_string()
            .repeat(self.columns.saturating_sub(1));
        match self.records.get(at) {
            Some(record) => vec![TextChange::insert(
                record.start,
                format!("{blank}{}", self.line_ending),
            )],
            None => vec![self.append_lines(text, &[blank])],
        }
    }

    /// Remove `rows` (a contiguous range of record indices).
    pub(crate) fn delete_rows(&self, rows: Range<usize>) -> Vec<TextChange> {
        let end = rows.end.min(self.records.len());
        if rows.start >= end {
            return Vec::new();
        }
        let first = &self.records[rows.start];
        let last = &self.records[end - 1];
        let range = if last.terminator.is_empty() && rows.start > 0 {
            // No newline follows the final record, so drop the one before it.
            self.records[rows.start - 1].end..last.end
        } else {
            first.start..last.terminator.end
        };
        vec![TextChange::replace(range, "")]
    }

    pub(crate) fn insert_column(&self, at: usize) -> Vec<TextChange> {
        let delimiter = self.delimiter.to_string();
        self.records
            .iter()
            .filter_map(|record| match record.fields.get(at) {
                Some(field) => Some(TextChange::insert(field.span.start, delimiter.clone())),
                None if at == record.fields.len() => {
                    Some(TextChange::insert(record.end, delimiter.clone()))
                }
                None => None,
            })
            .collect()
    }

    pub(crate) fn delete_column(&self, at: usize) -> Vec<TextChange> {
        self.records
            .iter()
            .filter_map(|record| {
                let field = record.fields.get(at)?;
                let range = if let Some(next) = record.fields.get(at + 1) {
                    field.span.start..next.span.start
                } else if at > 0 {
                    record.fields[at - 1].span.end..field.span.end
                } else {
                    field.span.clone()
                };
                Some(TextChange::replace(range, ""))
            })
            .collect()
    }

    pub(crate) fn move_column(&self, text: &str, from: usize, to: usize) -> Vec<TextChange> {
        if from == to {
            return Vec::new();
        }
        let (low, high) = (from.min(to), from.max(to));
        let delimiter = self.delimiter.to_string();
        self.records
            .iter()
            .filter(|record| record.fields.len() > high)
            .map(|record| {
                let mut raw: Vec<&str> = record.fields[low..=high]
                    .iter()
                    .map(|field| &text[field.span.clone()])
                    .collect();
                let moved = raw.remove(from - low);
                raw.insert(to - low, moved);
                TextChange::replace(
                    record.fields[low].span.start..record.fields[high].span.end,
                    raw.join(&delimiter),
                )
            })
            .collect()
    }

    /// Reorder records so that `order[i]` is the old index of new record `i`.
    /// Line terminators stay where they are; only record content moves.
    fn permute_rows(&self, text: &str, order: &[usize]) -> Vec<TextChange> {
        order
            .iter()
            .enumerate()
            .filter(|(position, old)| position != *old)
            .map(|(position, old)| {
                let target = &self.records[position];
                let source = &self.records[*old];
                TextChange::replace(target.start..target.end, &text[source.start..source.end])
            })
            .collect()
    }

    pub(crate) fn move_row(&self, text: &str, from: usize, to: usize) -> Vec<TextChange> {
        if from == to || from >= self.records.len() || to >= self.records.len() {
            return Vec::new();
        }
        let mut order: Vec<usize> = (0..self.records.len()).collect();
        let moved = order.remove(from);
        order.insert(to, moved);
        self.permute_rows(text, &order)
    }

    /// Stable sort of the records from `first_row` down by one column. Numbers
    /// compare numerically; everything else compares case-insensitively.
    pub(crate) fn sort_rows(
        &self,
        text: &str,
        first_row: usize,
        column: usize,
        descending: bool,
    ) -> Vec<TextChange> {
        let mut order: Vec<usize> = (0..self.records.len()).collect();
        let keys: Vec<Cow<str>> = (0..self.records.len())
            .map(|row| self.value(text, row, column).unwrap_or(Cow::Borrowed("")))
            .collect();
        order[first_row.min(self.records.len())..].sort_by(|a, b| {
            let ordering = compare_cells(&keys[*a], &keys[*b]);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
        self.permute_rows(text, &order)
    }
}

fn compare_cells(a: &str, b: &str) -> Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

fn unescape(raw: &str, quoted: bool) -> Cow<'_, str> {
    if !quoted {
        return Cow::Borrowed(raw);
    }
    let inner = &raw[1..];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    let mut closed = false;
    while let Some(character) = chars.next() {
        if closed {
            out.push(character);
        } else if character == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                out.push('"');
            } else {
                closed = true;
            }
        } else {
            out.push(character);
        }
    }
    Cow::Owned(out)
}

/// Field text for `value`: quoted only when it has to be, or when the cell it
/// replaces already was.
pub(crate) fn encode(value: &str, delimiter: char, was_quoted: bool) -> String {
    let needs_quotes = was_quoted || value.contains(delimiter) || value.contains(['"', '\n', '\r']);
    if needs_quotes {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// Parse a clipboard payload of tab-separated columns and newline-separated
/// rows, as spreadsheets copy it.
pub(crate) fn parse_clipboard(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .map(|line| {
            line.trim_end_matches('\r')
                .split('\t')
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

/// Clipboard text for a block of cells.
pub(crate) fn clipboard_text(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|row| row.join("\t"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pick the delimiter that splits the start of `text` most consistently.
pub(crate) fn detect_delimiter(text: &str, extension: Option<&str>) -> char {
    if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("tsv")) {
        return '\t';
    }
    let mut end = text.len().min(64 * 1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let sample = &text[..end];
    let mut best = (',', 0usize);
    for delimiter in DELIMITERS {
        let table = CsvTable::parse(sample, delimiter);
        let counts: Vec<usize> = table
            .records
            .iter()
            .take(50)
            .map(|record| record.fields.len())
            .collect();
        let Some(mode) = counts
            .iter()
            .copied()
            .max_by_key(|count| counts.iter().filter(|other| *other == count).count() * 64 + count)
        else {
            continue;
        };
        if mode < 2 {
            continue;
        }
        let score = counts.iter().filter(|count| **count == mode).count() * 1000 + mode;
        if score > best.1 {
            best = (delimiter, score);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> CsvTable {
        CsvTable::parse(text, ',')
    }

    fn cells(text: &str) -> Vec<Vec<String>> {
        let table = table(text);
        (0..table.row_count())
            .map(|row| {
                (0..table.records[row].fields.len())
                    .map(|column| table.value(text, row, column).unwrap().into_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn quoting_embedded_newlines_and_escaped_quotes_parse_per_rfc_4180() {
        let text = "a,\"b,c\",\"line1\nline2\"\n\"say \"\"hi\"\"\",,x\n";
        assert_eq!(
            cells(text),
            vec![
                vec!["a", "b,c", "line1\nline2"],
                vec!["say \"hi\"", "", "x"]
            ]
        );
        assert_eq!(table(text).unterminated, None);
    }

    #[test]
    fn line_endings_final_newline_bom_and_ragged_rows_are_reported_faithfully() {
        let text = "\u{feff}a,b\r\nc\r\n\r\nd,e,f";
        let parsed = table(text);
        assert_eq!(parsed.line_ending, "\r\n");
        assert_eq!(parsed.row_count(), 4);
        assert_eq!(parsed.columns, 3);
        assert!(!parsed.ends_with_newline());
        assert_eq!(parsed.value(text, 0, 0).unwrap(), "a");
        assert_eq!(parsed.value(text, 1, 1), None);
        assert_eq!(parsed.value(text, 2, 0).unwrap(), "");
        assert!(table("a\n").ends_with_newline());
        assert_eq!(table("").row_count(), 0);
    }

    #[test]
    fn a_trailing_delimiter_is_an_empty_last_cell() {
        assert_eq!(cells("a,b,\n"), vec![vec!["a", "b", ""]]);
    }

    #[test]
    fn unterminated_quotes_are_flagged_and_keep_the_rest_of_the_file() {
        let text = "a,b\nc,\"oops\nd,e\n";
        let parsed = table(text);
        assert_eq!(parsed.unterminated, Some(1));
        assert_eq!(parsed.value(text, 1, 1).unwrap(), "oops\nd,e\n");
    }

    #[test]
    fn every_delimiter_round_trips_and_is_detected() {
        for (delimiter, name) in [(',', "c"), (';', "s"), ('\t', "t"), ('|', "p")] {
            let text = format!("h1{d}h2{d}h3\n1{d}2{d}3\n4{d}5{d}6\n", d = delimiter);
            assert_eq!(detect_delimiter(&text, None), delimiter, "{name}");
            let parsed = CsvTable::parse(&text, delimiter);
            assert_eq!(parsed.columns, 3);
            let edited = apply_changes(&text, &parsed.set_cell(&text, 1, 1, "x"));
            let d = delimiter;
            assert_eq!(
                edited,
                format!("h1{d}h2{d}h3\n1{d}x{d}3\n4{d}5{d}6\n"),
                "{name}"
            );
        }
        assert_eq!(detect_delimiter("a b c\n", Some("tsv")), '\t');
        assert_eq!(detect_delimiter("just one column\nsecond\n", None), ',');
    }

    #[test]
    fn editing_a_cell_changes_only_that_cell_and_quotes_only_when_needed() {
        let text = "a,\"keep, quoted\",c\r\n1,2,3\r\n";
        let parsed = table(text);
        let changes = parsed.set_cell(text, 1, 1, "plain");
        let at = text.find(",2,").unwrap() + 1;
        assert_eq!(changes, vec![TextChange::replace(at..at + 1, "plain")]);
        assert_eq!(
            apply_changes(text, &changes),
            "a,\"keep, quoted\",c\r\n1,plain,3\r\n"
        );
        let comma = apply_changes(text, &parsed.set_cell(text, 1, 0, "x,y"));
        assert_eq!(comma, "a,\"keep, quoted\",c\r\n\"x,y\",2,3\r\n");
        let quote = apply_changes(text, &parsed.set_cell(text, 0, 0, "say \"hi\""));
        assert!(quote.starts_with("\"say \"\"hi\"\"\",\"keep"));
        // A cell that was quoted stays quoted when it is edited.
        let stays = apply_changes(text, &parsed.set_cell(text, 0, 1, "plain"));
        assert!(stays.starts_with("a,\"plain\",c"));
        assert!(parsed.set_cell(text, 0, 1, "keep, quoted").is_empty());
    }

    #[test]
    fn setting_cells_past_the_end_pads_short_rows_and_appends_rows() {
        let text = "a,b\nc\n";
        let parsed = table(text);
        let out = apply_changes(text, &parsed.set_cell(text, 1, 2, "z"));
        assert_eq!(out, "a,b\nc,,z\n");
        let out = apply_changes(
            text,
            &parsed.set_cells(
                text,
                1,
                0,
                &[vec!["x".into()], vec!["y".into(), "w".into()]],
            ),
        );
        assert_eq!(out, "a,b\nx\ny,w\n");
        let bare = "a,b";
        let out = apply_changes(bare, &table(bare).set_cell(bare, 1, 0, "q"));
        assert_eq!(out, "a,b\nq");
    }

    #[test]
    fn row_insert_and_delete_keep_line_endings_and_the_missing_final_newline() {
        let text = "a,b\r\nc,d\r\ne,f";
        let parsed = table(text);
        assert_eq!(
            apply_changes(text, &parsed.insert_row(text, 1)),
            "a,b\r\n,\r\nc,d\r\ne,f"
        );
        assert_eq!(
            apply_changes(text, &parsed.insert_row(text, 3)),
            "a,b\r\nc,d\r\ne,f\r\n,"
        );
        assert_eq!(apply_changes(text, &parsed.delete_rows(0..1)), "c,d\r\ne,f");
        assert_eq!(apply_changes(text, &parsed.delete_rows(2..3)), "a,b\r\nc,d");
        assert_eq!(apply_changes(text, &parsed.delete_rows(0..3)), "");
        let newline = "a\nb\n";
        assert_eq!(
            apply_changes(newline, &table(newline).insert_row(newline, 2)),
            "a\nb\n\n"
        );
        assert_eq!(
            apply_changes(newline, &table(newline).delete_rows(1..2)),
            "a\n"
        );
    }

    #[test]
    fn column_operations_skip_short_rows_instead_of_padding_them() {
        let text = "a,b,c\nd\ne,f,g\n";
        let parsed = table(text);
        assert_eq!(
            apply_changes(text, &parsed.insert_column(1)),
            "a,,b,c\nd,\ne,,f,g\n"
        );
        assert_eq!(
            apply_changes(text, &parsed.insert_column(3)),
            "a,b,c,\nd\ne,f,g,\n"
        );
        assert_eq!(
            apply_changes(text, &parsed.delete_column(1)),
            "a,c\nd\ne,g\n"
        );
        assert_eq!(
            apply_changes(text, &parsed.delete_column(2)),
            "a,b\nd\ne,f\n"
        );
        assert_eq!(
            apply_changes(text, &parsed.delete_column(0)),
            "b,c\n\nf,g\n"
        );
        assert_eq!(
            apply_changes(text, &parsed.move_column(text, 0, 2)),
            "b,c,a\nd\nf,g,e\n"
        );
        assert_eq!(
            apply_changes(text, &parsed.move_column(text, 2, 0)),
            "c,a,b\nd\ng,e,f\n"
        );
    }

    #[test]
    fn moving_and_sorting_rows_carry_raw_text_and_keep_terminators() {
        let text = "h\n\"b, 2\"\nc\na";
        let parsed = table(text);
        assert_eq!(
            apply_changes(text, &parsed.move_row(text, 1, 3)),
            "h\nc\na\n\"b, 2\""
        );
        assert_eq!(
            apply_changes(text, &parsed.sort_rows(text, 1, 0, false)),
            "h\na\n\"b, 2\"\nc"
        );
        assert_eq!(
            apply_changes(text, &parsed.sort_rows(text, 1, 0, true)),
            "h\nc\n\"b, 2\"\na"
        );
        let numbers = "n\n10\n9\n100\n";
        assert_eq!(
            apply_changes(numbers, &table(numbers).sort_rows(numbers, 1, 0, false)),
            "n\n9\n10\n100\n"
        );
    }

    #[test]
    fn edits_are_minimal_and_sorted() {
        let text = "a,b\nc,d\ne,f\n";
        let parsed = table(text);
        let changes = parsed.set_cells(
            text,
            0,
            0,
            &[vec!["a".into(), "B".into()], vec!["C".into(), "d".into()]],
        );
        assert_eq!(changes.len(), 2);
        assert!(
            changes
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
        assert_eq!(apply_changes(text, &changes), "a,B\nC,d\ne,f\n");
    }

    #[test]
    fn clipboard_text_round_trips_through_tabs_and_newlines() {
        let rows = vec![
            vec!["a".to_owned(), "b".into()],
            vec!["c".into(), "d".into()],
        ];
        assert_eq!(clipboard_text(&rows), "a\tb\nc\td");
        assert_eq!(parse_clipboard("a\tb\r\nc\td\r\n"), rows);
        assert!(parse_clipboard("").is_empty());
    }

    #[test]
    fn a_large_file_parses_quickly_and_edits_stay_small() {
        let text: String = (0..100_000)
            .map(|row| format!("{row},name{row},{}\n", row * 2))
            .collect();
        let started = std::time::Instant::now();
        let parsed = table(&text);
        assert_eq!(parsed.row_count(), 100_000);
        let changes = parsed.set_cell(&text, 99_999, 1, "edited");
        assert_eq!(changes.len(), 1);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "parse+edit took {:?}",
            started.elapsed()
        );
    }
}
