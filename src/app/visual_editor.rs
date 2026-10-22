//! Visual editor state for structured files. The table is a view over the
//! source document's buffer: it keeps spans, selection and layout, never a copy
//! of the cells, and every edit goes through the document's own undo history.
use super::view_state::ViewState;
use crate::model::{
    DocumentId, EditorState,
    csv::{self, CsvTable, TextChange},
};
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
};

/// A change requested by the table editor, applied to the source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TableEdit {
    /// Replace a block of cells with its top-left cell at (`row`, `column`).
    SetCells {
        row: usize,
        column: usize,
        grid: Vec<Vec<String>>,
    },
    InsertRow(usize),
    DeleteRows(Range<usize>),
    InsertColumn(usize),
    DeleteColumn(usize),
    MoveRow {
        from: usize,
        to: usize,
    },
    MoveColumn {
        from: usize,
        to: usize,
    },
    /// Rewrite the buffer with rows from `first_row` down ordered by a column.
    SortRows {
        first_row: usize,
        column: usize,
        descending: bool,
    },
    Undo,
    Redo,
}

/// A cell being typed into; committed on Enter, dropped on Escape.
pub(crate) struct CellEdit {
    pub(crate) row: usize,
    pub(crate) column: usize,
    pub(crate) text: String,
    pub(crate) needs_focus: bool,
}

pub(crate) const DEFAULT_COLUMN_WIDTH: f32 = 120.0;

/// Parse result for one revision of the source text.
enum Parsed {
    Ready {
        revision: u64,
        delimiter: char,
        table: Arc<CsvTable>,
    },
    Loading {
        revision: u64,
        delimiter: char,
        receiver: Receiver<CsvTable>,
    },
}

/// What the table editor can show this frame.
pub(crate) enum TableSource {
    Ready(Arc<CsvTable>),
    Loading,
    TooLarge(usize),
}

pub(crate) struct TableView {
    /// Chosen by the user; `None` follows the detected delimiter.
    pub(crate) delimiter_override: Option<char>,
    detected: Option<char>,
    pub(crate) header: bool,
    pub(crate) anchor: (usize, usize),
    pub(crate) head: (usize, usize),
    pub(crate) editing: Option<CellEdit>,
    pub(crate) widths: Vec<f32>,
    parsed: Option<Parsed>,
}

impl Default for TableView {
    fn default() -> Self {
        Self {
            delimiter_override: None,
            detected: None,
            header: true,
            anchor: (0, 0),
            head: (0, 0),
            editing: None,
            widths: Vec::new(),
            parsed: None,
        }
    }
}

impl TableView {
    pub(crate) fn width(&self, column: usize) -> f32 {
        self.widths
            .get(column)
            .copied()
            .unwrap_or(DEFAULT_COLUMN_WIDTH)
    }

    pub(crate) fn set_width(&mut self, column: usize, width: f32) {
        if self.widths.len() <= column {
            self.widths.resize(column + 1, DEFAULT_COLUMN_WIDTH);
        }
        self.widths[column] = width;
    }

    pub(crate) fn delimiter(&self) -> char {
        self.delimiter_override.or(self.detected).unwrap_or(',')
    }

    pub(crate) fn set_delimiter(&mut self, delimiter: char) {
        self.delimiter_override = Some(delimiter);
        self.editing = None;
        self.parsed = None;
    }

    /// Selected cells as inclusive (top-left, bottom-right) corners.
    pub(crate) fn selection(&self) -> ((usize, usize), (usize, usize)) {
        (
            (
                self.anchor.0.min(self.head.0),
                self.anchor.1.min(self.head.1),
            ),
            (
                self.anchor.0.max(self.head.0),
                self.anchor.1.max(self.head.1),
            ),
        )
    }

    /// The table for `text` at `revision`, parsing in the background when large.
    pub(crate) fn table(
        &mut self,
        text: &str,
        revision: u64,
        extension: Option<&str>,
    ) -> TableSource {
        if text.len() > csv::MAX_VISUAL_BYTES {
            return TableSource::TooLarge(text.len());
        }
        if self.detected.is_none() {
            self.detected = Some(csv::detect_delimiter(text, extension));
        }
        let delimiter = self.delimiter();
        if let Some(Parsed::Loading {
            revision: loading,
            delimiter: loading_delimiter,
            receiver,
        }) = &self.parsed
        {
            match receiver.try_recv() {
                Ok(table) => {
                    self.parsed = Some(Parsed::Ready {
                        revision: *loading,
                        delimiter: *loading_delimiter,
                        table: Arc::new(table),
                    });
                }
                Err(TryRecvError::Empty) => {
                    if *loading == revision && *loading_delimiter == delimiter {
                        return TableSource::Loading;
                    }
                    // Superseded; the stale result is dropped with the receiver.
                    self.parsed = None;
                }
                Err(TryRecvError::Disconnected) => self.parsed = None,
            }
        }
        if let Some(Parsed::Ready {
            revision: parsed,
            delimiter: parsed_delimiter,
            table,
        }) = &self.parsed
            && *parsed == revision
            && *parsed_delimiter == delimiter
        {
            return TableSource::Ready(Arc::clone(table));
        }
        if text.len() >= csv::BACKGROUND_PARSE_BYTES {
            let (sender, receiver) = std::sync::mpsc::channel();
            let owned = text.to_owned();
            std::thread::spawn(move || {
                let _ = sender.send(CsvTable::parse(&owned, delimiter));
            });
            self.parsed = Some(Parsed::Loading {
                revision,
                delimiter,
                receiver,
            });
            return TableSource::Loading;
        }
        let table = Arc::new(CsvTable::parse(text, delimiter));
        self.parsed = Some(Parsed::Ready {
            revision,
            delimiter,
            table: Arc::clone(&table),
        });
        TableSource::Ready(table)
    }
}

/// Table-editor state that is not part of any document, keyed by source.
#[derive(Default)]
pub(crate) struct VisualState {
    tables: HashMap<DocumentId, TableView>,
}

impl VisualState {
    pub(crate) fn table_mut(&mut self, source: DocumentId) -> &mut TableView {
        self.tables.entry(source).or_default()
    }

    pub(crate) fn retain_sources(&mut self, alive: impl Fn(&DocumentId) -> bool) {
        self.tables.retain(|id, _| alive(id));
    }
}

/// Apply `edit` to the source document's buffer as one undo step.
pub(crate) fn apply_table_edit(
    editor: &mut EditorState,
    view: &mut ViewState,
    source: DocumentId,
    edit: &TableEdit,
) {
    let Some(index) = editor.index_of(source) else {
        return;
    };
    let document = &mut editor.files[index];
    document.enable_shared_editing();
    document.sync_shared_editing();
    if matches!(edit, TableEdit::Undo | TableEdit::Redo) {
        let buffer = document.editing.as_mut().expect("shared editing enabled");
        let moved = if *edit == TableEdit::Undo {
            buffer.undo()
        } else {
            buffer.redo()
        };
        document.sync_shared_editing();
        view.status = match (moved, edit) {
            (true, TableEdit::Undo) => "Undo".into(),
            (true, _) => "Redo".into(),
            (false, TableEdit::Undo) => "Nothing to undo".into(),
            (false, _) => "Nothing to redo".into(),
        };
        return;
    }
    let delimiter = view.visual.table_mut(source).delimiter();
    let table = CsvTable::parse(&document.text, delimiter);
    if table.unterminated.is_some() {
        view.status = "Fix the unterminated quote in the text view before editing the table".into();
        return;
    }
    let text = &document.text;
    let changes: Vec<TextChange> = match edit {
        TableEdit::SetCells { row, column, grid } => table.set_cells(text, *row, *column, grid),
        TableEdit::InsertRow(at) => table.insert_row(text, *at),
        TableEdit::DeleteRows(rows) => table.delete_rows(rows.clone()),
        TableEdit::InsertColumn(at) => table.insert_column(*at),
        TableEdit::DeleteColumn(at) => table.delete_column(*at),
        TableEdit::MoveRow { from, to } => table.move_row(text, *from, *to),
        TableEdit::MoveColumn { from, to } => table.move_column(text, *from, *to),
        TableEdit::SortRows {
            first_row,
            column,
            descending,
        } => table.sort_rows(text, *first_row, *column, *descending),
        TableEdit::Undo | TableEdit::Redo => unreachable!("handled above"),
    };
    if changes.is_empty() {
        return;
    }
    let changes: Vec<(Range<usize>, String)> = changes
        .into_iter()
        .map(|change| (change.range, change.text))
        .collect();
    match document.apply_changes(&changes) {
        Ok(_) => view.status = "Edited table".into(),
        Err(message) => view.status = message.into(),
    }
}
