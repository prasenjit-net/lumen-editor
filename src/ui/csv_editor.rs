//! Table editor for CSV/TSV documents. It paints and edits the source
//! document's buffer through typed [`TableEdit`] commands; it owns no cells.
use super::{Workbench, theme};
use crate::{
    app::{
        command::Command,
        visual_editor::{CellEdit, TableEdit, TableSource, TableView},
    },
    model::{
        DocumentId,
        csv::{self, CsvTable},
    },
};
use eframe::egui::{self, Align2, Color32, FontId, Key, Modifiers, Rect, RichText, Sense, Vec2};

const ROW_HEIGHT: f32 = 22.0;
const HEADER_HEIGHT: f32 = 24.0;
const MIN_COLUMN_WIDTH: f32 = 36.0;
const RESIZE_GRAB: f32 = 5.0;
const CELL_PADDING: f32 = 6.0;
const FONT_SIZE: f32 = 12.5;
/// The grid and its cell editor keep Tab and the arrows instead of moving focus.
const GRID_FOCUS_FILTER: egui::EventFilter = egui::EventFilter {
    tab: true,
    horizontal_arrows: true,
    vertical_arrows: true,
    escape: false,
};

/// Column left edges (relative to the content origin) for one frame.
struct Layout {
    lefts: Vec<f32>,
    row_number_width: f32,
}

impl Layout {
    fn new(view: &TableView, columns: usize, rows: usize, digit_width: f32) -> Self {
        let mut lefts = Vec::with_capacity(columns + 1);
        let digits = rows.to_string().len().max(2) as f32;
        let row_number_width = digits * digit_width + 2.0 * CELL_PADDING;
        let mut left = row_number_width;
        for column in 0..columns {
            lefts.push(left);
            left += view.width(column);
        }
        lefts.push(left);
        Self {
            lefts,
            row_number_width,
        }
    }

    fn width(&self) -> f32 {
        *self.lefts.last().unwrap_or(&0.0)
    }

    /// Column under the x offset, if any.
    fn column_at(&self, x: f32) -> Option<usize> {
        let columns = self.lefts.len().saturating_sub(1);
        if x < self.row_number_width || columns == 0 || x >= self.width() {
            return None;
        }
        Some(self.lefts.partition_point(|left| *left <= x) - 1)
    }
}

/// Display text for a cell: one line, no control characters.
fn one_line(value: &str) -> std::borrow::Cow<'_, str> {
    if value.contains(['\n', '\r', '\t']) {
        std::borrow::Cow::Owned(value.replace(['\n', '\r', '\t'], " "))
    } else {
        std::borrow::Cow::Borrowed(value)
    }
}

fn column_letters(mut column: usize) -> String {
    let mut name = String::new();
    loop {
        name.insert(0, (b'A' + (column % 26) as u8) as char);
        if column < 26 {
            return name;
        }
        column = column / 26 - 1;
    }
}

fn delimiter_name(delimiter: char) -> &'static str {
    match delimiter {
        ',' => "Comma",
        ';' => "Semicolon",
        '\t' => "Tab",
        _ => "Pipe",
    }
}

impl Workbench<'_> {
    pub(super) fn csv_editor_page(&mut self, ui: &mut egui::Ui, index: usize) {
        let colors = theme::palette(ui.visuals());
        let Some(source_index) = self.editor.files[index]
            .view_of
            .and_then(|id| self.editor.index_of(id))
        else {
            ui.label(RichText::new("The edited file was closed").color(colors.muted));
            return;
        };
        // The text view may not have rendered this frame; bring the text up to date.
        self.editor.files[source_index].sync_shared_editing();
        let source = self.editor.files[source_index].id;
        let extension = self.editor.files[source_index]
            .path
            .as_deref()
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            .map(str::to_owned);
        let revision = self.editor.files[source_index].revision;
        let table_view = self.view.visual.table_mut(source);
        let table = table_view.table(
            &self.editor.files[source_index].text,
            revision,
            extension.as_deref(),
        );
        let mut commands = Vec::new();
        let table = match table {
            TableSource::Ready(table) => table,
            TableSource::Loading => {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.spinner();
                    ui.label(RichText::new("Parsing table…").color(colors.muted));
                });
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            TableSource::TooLarge(bytes) => {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(format!(
                            "This file is {} MB, too large for the visual editor.",
                            bytes / (1024 * 1024)
                        ))
                        .color(colors.muted),
                    );
                    if ui.small_button("Open Text View").clicked() {
                        commands.push(Command::ActivateDocument(source));
                    }
                });
                for command in commands {
                    self.command(command);
                }
                return;
            }
        };
        let text = std::mem::take(&mut self.editor.files[source_index].text);
        self.csv_toolbar(ui, source, &table, &mut commands);
        self.csv_grid(ui, source, &table, &text, &mut commands);
        self.editor.files[source_index].text = text;
        for command in commands {
            self.command(command);
        }
    }

    fn csv_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        source: DocumentId,
        table: &CsvTable,
        commands: &mut Vec<Command>,
    ) {
        let colors = theme::palette(ui.visuals());
        let view = self.view.visual.table_mut(source);
        let malformed = table.unterminated.is_some();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut header = view.header;
                    if ui
                        .checkbox(&mut header, "First row is header")
                        .on_hover_text("Show the first row as column titles")
                        .changed()
                    {
                        view.header = header;
                        view.anchor = (usize::from(header), view.anchor.1);
                        view.head = view.anchor;
                    }
                    let current = view.delimiter();
                    egui::ComboBox::from_id_salt(("csv-delimiter", source))
                        .selected_text(delimiter_name(current))
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for delimiter in csv::DELIMITERS {
                                if ui
                                    .selectable_label(current == delimiter, delimiter_name(delimiter))
                                    .clicked()
                                {
                                    view.set_delimiter(delimiter);
                                }
                            }
                        })
                        .response
                        .on_hover_text("Delimiter (detected from the file; changing it only changes how the table reads the text)");
                    ui.menu_button("Table", |ui| {
                        if let Some(edit) = table_menu(ui, view, table, malformed) {
                            commands.push(Command::EditTable(source, edit));
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Insert, delete, move and sort rows and columns");
                    if ui
                        .small_button("Text View")
                        .on_hover_text("Edit the same file as text")
                        .clicked()
                    {
                        commands.push(Command::ActivateDocument(source));
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} rows × {} columns",
                            table.row_count().saturating_sub(usize::from(view.header)),
                            table.columns
                        ))
                        .size(theme::META_SIZE)
                        .color(colors.muted),
                    );
                });
                if malformed {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Row {} has a quote that never closes, so the table is read-only.",
                                table.unterminated.unwrap_or(0) + 1
                            ))
                            .size(theme::META_SIZE)
                            .color(colors.danger),
                        );
                        if ui.small_button("Fix in Text View").clicked() {
                            commands.push(Command::ActivateDocument(source));
                        }
                    });
                }
            });
    }

    fn csv_grid(
        &mut self,
        ui: &mut egui::Ui,
        source: DocumentId,
        table: &CsvTable,
        text: &str,
        commands: &mut Vec<Command>,
    ) {
        let colors = theme::palette(ui.visuals());
        let font = FontId::monospace(FONT_SIZE);
        let digit_width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let view = self.view.visual.table_mut(source);
        let malformed = table.unterminated.is_some();
        let rows = table.row_count();
        let columns = table.columns;
        let first = usize::from(view.header && rows > 0);
        // Keep the selection and edit inside a table that shrank under them.
        let (last_row, last_column) = (rows.saturating_sub(1), columns.saturating_sub(1));
        view.anchor = (view.anchor.0.min(last_row), view.anchor.1.min(last_column));
        view.head = (view.head.0.min(last_row), view.head.1.min(last_column));
        if view.editing.as_ref().is_some_and(|edit| {
            edit.row >= rows || edit.column >= columns.max(edit.column + usize::from(rows == 0))
        }) {
            view.editing = None;
        }
        let layout = Layout::new(view, columns, rows, digit_width);
        let grid_id = egui::Id::new(("csv-grid", source));
        let mut reveal = std::mem::take(&mut view.reveal);
        let id = grid_id.with("focus");

        let output = egui::ScrollArea::both()
            .id_salt(grid_id)
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let content = Vec2::new(
                    layout.width().max(ui.available_width()),
                    HEADER_HEIGHT + ROW_HEIGHT * (rows.saturating_sub(first)) as f32,
                );
                let (area, _) = ui.allocate_exact_size(content, Sense::hover());
                let response = ui.interact(area, id, Sense::click_and_drag());
                let origin = area.min;
                let painter = ui.painter_at(ui.clip_rect());
                let cell_rect = |row: usize, column: usize| -> Rect {
                    let top =
                        origin.y + HEADER_HEIGHT + (row.saturating_sub(first)) as f32 * ROW_HEIGHT;
                    Rect::from_min_max(
                        egui::pos2(origin.x + layout.lefts[column], top),
                        egui::pos2(origin.x + layout.lefts[column + 1], top + ROW_HEIGHT),
                    )
                };
                let visible_rows = {
                    let start = ((viewport.min.y - HEADER_HEIGHT) / ROW_HEIGHT)
                        .floor()
                        .max(0.0) as usize;
                    let end = ((viewport.max.y - HEADER_HEIGHT) / ROW_HEIGHT)
                        .ceil()
                        .max(0.0) as usize
                        + 1;
                    (first + start).min(rows)..(first + end).min(rows)
                };
                let visible_columns = (0..columns)
                    .filter(|column| {
                        layout.lefts[column + 1] >= viewport.min.x
                            && layout.lefts[*column] <= viewport.max.x
                    })
                    .collect::<Vec<_>>();
                let sticky_x = origin.x + viewport.min.x;
                let sticky_y = origin.y + viewport.min.y;
                let (low, high) = view.selection();
                let line = colors.border;

                // Pointer: clicks, drags, column resize.
                let mut resizing = false;
                for &column in &visible_columns {
                    let edge = origin.x + layout.lefts[column + 1];
                    let handle = Rect::from_min_max(
                        egui::pos2(edge - RESIZE_GRAB, sticky_y),
                        egui::pos2(edge + RESIZE_GRAB, sticky_y + HEADER_HEIGHT),
                    );
                    let grab = ui.interact(handle, grid_id.with(("resize", column)), Sense::drag());
                    if grab.hovered() || grab.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                    }
                    if grab.dragged() {
                        resizing = true;
                        let width =
                            (view.width(column) + grab.drag_delta().x).max(MIN_COLUMN_WIDTH);
                        view.set_width(column, width);
                    }
                }
                let to_cell = |position: egui::Pos2| -> Option<(usize, usize)> {
                    let relative = position - origin;
                    let column = layout.column_at(relative.x);
                    let band = relative.y < HEADER_HEIGHT;
                    let row = if band {
                        Some(0)
                    } else {
                        let at = first + ((relative.y - HEADER_HEIGHT) / ROW_HEIGHT) as usize;
                        (at < rows).then_some(at)
                    };
                    match (row, column) {
                        (Some(row), Some(column)) => Some((row, column)),
                        _ => None,
                    }
                };
                let shift = ui.input(|input| input.modifiers.shift);
                if !resizing && let Some(position) = response.interact_pointer_pos() {
                    let relative = position - origin;
                    if response.drag_started() || response.clicked() || response.secondary_clicked()
                    {
                        if let Some(edit) = view.editing.take() {
                            commands.push(commit(source, edit));
                        }
                        ui.memory_mut(|memory| memory.request_focus(id));
                        if relative.x < layout.row_number_width && relative.y >= HEADER_HEIGHT {
                            let row = (first
                                + ((relative.y - HEADER_HEIGHT) / ROW_HEIGHT) as usize)
                                .min(last_row);
                            view.anchor = (row, 0);
                            view.head = (row, last_column);
                        } else if relative.y < HEADER_HEIGHT && !view.header {
                            if let Some(column) = layout.column_at(relative.x) {
                                view.anchor = (0, column);
                                view.head = (last_row, column);
                            }
                        } else if let Some(cell) = to_cell(position) {
                            let inside = cell.0 >= low.0
                                && cell.0 <= high.0
                                && cell.1 >= low.1
                                && cell.1 <= high.1;
                            if shift {
                                view.head = cell;
                            } else if !(response.secondary_clicked() && inside) {
                                view.anchor = cell;
                                view.head = cell;
                            }
                        }
                    } else if response.dragged()
                        && let Some(cell) = to_cell(position)
                    {
                        view.head = cell;
                    }
                }
                if response.double_clicked()
                    && !malformed
                    && let Some(position) = response.interact_pointer_pos()
                    && let Some(cell) = to_cell(position)
                {
                    begin_edit(view, table, text, cell, None);
                }

                // Rows.
                let clip = ui.clip_rect();
                for row in visible_rows.clone() {
                    let top = cell_rect(row, 0).top();
                    for &column in &visible_columns {
                        let rect = cell_rect(row, column);
                        let selected =
                            row >= low.0 && row <= high.0 && column >= low.1 && column <= high.1;
                        let value = table.value(text, row, column);
                        if selected {
                            painter.rect_filled(rect, 0, colors.accent.gamma_multiply(0.28));
                        } else if value.is_none() {
                            painter.rect_filled(rect, 0, colors.panel);
                        }
                        painter.line_segment([rect.right_top(), rect.right_bottom()], (1.0, line));
                        painter
                            .line_segment([rect.left_bottom(), rect.right_bottom()], (1.0, line));
                        if let Some(value) = value
                            && view
                                .editing
                                .as_ref()
                                .is_none_or(|edit| (edit.row, edit.column) != (row, column))
                        {
                            let cell_painter = painter
                                .with_clip_rect(rect.shrink2(Vec2::new(0.0, 1.0)).intersect(clip));
                            cell_painter.text(
                                egui::pos2(rect.left() + CELL_PADDING, top + ROW_HEIGHT / 2.0),
                                Align2::LEFT_CENTER,
                                one_line(&value),
                                font.clone(),
                                colors.text,
                            );
                        }
                    }
                }
                if rows > 0 && view.head.0 >= first {
                    let active = cell_rect(view.head.0, view.head.1);
                    painter.rect_stroke(
                        active,
                        0,
                        egui::Stroke::new(1.5_f32, colors.accent),
                        egui::StrokeKind::Inside,
                    );
                }

                // Cell editor.
                let mut finished: Option<(CellEdit, Finish)> = None;
                if let Some(mut edit) = view.editing.take() {
                    let rect = cell_rect(edit.row, edit.column);
                    let editor_id = grid_id.with(("edit", edit.row, edit.column));
                    // Like Excel: Tab commits and moves right, Shift+Tab left. Take
                    // the key before the text widget can treat it as focus movement.
                    let tab_step = ui.input_mut(|input| {
                        if input.consume_key(Modifiers::SHIFT, Key::Tab) {
                            Some(true)
                        } else {
                            input
                                .consume_key(Modifiers::NONE, Key::Tab)
                                .then_some(false)
                        }
                    });
                    let output = ui.put(
                        rect,
                        egui::TextEdit::singleline(&mut edit.text)
                            .id(editor_id)
                            .font(font.clone())
                            .frame(false)
                            .margin(Vec2::new(CELL_PADDING, 3.0))
                            .desired_width(rect.width()),
                    );
                    ui.memory_mut(|memory| {
                        memory.set_focus_lock_filter(editor_id, GRID_FOCUS_FILTER)
                    });
                    if edit.needs_focus {
                        output.request_focus();
                        edit.needs_focus = false;
                    }
                    let (enter, escape) = ui.input(|input| {
                        (
                            input.key_pressed(Key::Enter),
                            input.key_pressed(Key::Escape),
                        )
                    });
                    let back = ui.input(|input| input.modifiers.shift);
                    painter.rect_stroke(
                        rect,
                        0,
                        egui::Stroke::new(1.5_f32, colors.accent),
                        egui::StrokeKind::Inside,
                    );
                    if escape {
                        finished = Some((edit, Finish::Cancel));
                    } else if let Some(back) = tab_step {
                        finished = Some((edit, Finish::Right(back)));
                    } else if output.lost_focus() && enter {
                        finished = Some((edit, Finish::Down(back)));
                    } else if output.lost_focus() {
                        finished = Some((edit, Finish::Stay));
                    } else {
                        view.editing = Some(edit);
                    }
                }
                if let Some((edit, finish)) = finished {
                    let (row, column) = (edit.row, edit.column);
                    if !matches!(finish, Finish::Cancel) {
                        commands.push(commit(source, edit));
                    }
                    let target = match finish {
                        Finish::Cancel | Finish::Stay => (row, column),
                        Finish::Down(false) => ((row + 1).min(last_row), column),
                        Finish::Down(true) => (row.saturating_sub(1).max(first), column),
                        Finish::Right(false) => (row, (column + 1).min(last_column)),
                        Finish::Right(true) => (row, column.saturating_sub(1)),
                    };
                    view.anchor = target;
                    view.head = target;
                    reveal = true;
                    if !matches!(finish, Finish::Stay) {
                        ui.memory_mut(|memory| memory.request_focus(id));
                    }
                }

                // Sticky header band and row numbers, over the cells.
                let band_top = sticky_y;
                painter.rect_filled(
                    Rect::from_min_max(
                        egui::pos2(origin.x, band_top),
                        egui::pos2(
                            origin.x + layout.width().max(content.x),
                            band_top + HEADER_HEIGHT,
                        ),
                    ),
                    0,
                    colors.panel,
                );
                for &column in &visible_columns {
                    let rect = Rect::from_min_max(
                        egui::pos2(origin.x + layout.lefts[column], band_top),
                        egui::pos2(
                            origin.x + layout.lefts[column + 1],
                            band_top + HEADER_HEIGHT,
                        ),
                    );
                    let title = if view.header && rows > 0 {
                        table
                            .value(text, 0, column)
                            .map(|value| one_line(&value).into_owned())
                            .unwrap_or_default()
                    } else {
                        column_letters(column)
                    };
                    let in_selection = column >= low.1 && column <= high.1;
                    let cell_painter = painter.with_clip_rect(rect.intersect(clip));
                    if view.header && rows > 0 && low.0 == 0 && in_selection {
                        cell_painter.rect_filled(rect, 0, colors.accent.gamma_multiply(0.28));
                    }
                    cell_painter.text(
                        egui::pos2(rect.left() + CELL_PADDING, rect.center().y),
                        Align2::LEFT_CENTER,
                        title,
                        FontId::monospace(FONT_SIZE),
                        if view.header {
                            colors.text
                        } else {
                            colors.muted
                        },
                    );
                    painter.line_segment([rect.right_top(), rect.right_bottom()], (1.0, line));
                }
                painter.line_segment(
                    [
                        egui::pos2(origin.x, band_top + HEADER_HEIGHT),
                        egui::pos2(
                            origin.x + layout.width().max(content.x),
                            band_top + HEADER_HEIGHT,
                        ),
                    ],
                    (1.0, line),
                );
                let gutter = Rect::from_min_max(
                    egui::pos2(sticky_x, sticky_y + HEADER_HEIGHT),
                    egui::pos2(
                        sticky_x + layout.row_number_width,
                        origin.y + viewport.max.y,
                    ),
                );
                painter.rect_filled(gutter, 0, colors.panel);
                for row in visible_rows {
                    let top = cell_rect(row, 0).top();
                    let selected = row >= low.0 && row <= high.0;
                    painter.text(
                        egui::pos2(
                            sticky_x + layout.row_number_width - CELL_PADDING,
                            top + ROW_HEIGHT / 2.0,
                        ),
                        Align2::RIGHT_CENTER,
                        (row + 1 - first).to_string(),
                        FontId::monospace(FONT_SIZE),
                        if selected { colors.text } else { colors.muted },
                    );
                }
                painter.line_segment(
                    [
                        egui::pos2(
                            sticky_x + layout.row_number_width,
                            origin.y + viewport.min.y,
                        ),
                        egui::pos2(
                            sticky_x + layout.row_number_width,
                            origin.y + viewport.max.y,
                        ),
                    ],
                    (1.0, line),
                );
                painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(sticky_x, band_top),
                        Vec2::new(layout.row_number_width, HEADER_HEIGHT),
                    ),
                    0,
                    colors.panel,
                );

                if reveal && rows > 0 {
                    let target = cell_rect(view.head.0.max(first), view.head.1.min(last_column));
                    let padded = Rect::from_min_max(
                        target.min - Vec2::new(layout.row_number_width, HEADER_HEIGHT),
                        target.max,
                    );
                    ui.scroll_to_rect(padded, None);
                }
                response
            });
        let response = output.inner;
        let focused = ui.memory(|memory| memory.has_focus(id));
        ui.memory_mut(|memory| memory.set_focus_lock_filter(id, GRID_FOCUS_FILTER));
        let view = self.view.visual.table_mut(source);
        if focused && view.editing.is_none() {
            handle_keys(ui, source, view, table, text, malformed, commands);
        }
        if !malformed {
            response.context_menu(|ui| {
                if let Some(edit) = table_menu(ui, view, table, malformed) {
                    commands.push(Command::EditTable(source, edit));
                    ui.close();
                }
            });
        }
        if view.editing.is_some() {
            // Keep the cell editor alive across the frame it was created in.
            ui.ctx().request_repaint();
        }
    }
}

enum Finish {
    Cancel,
    Stay,
    Down(bool),
    Right(bool),
}

fn commit(source: DocumentId, edit: CellEdit) -> Command {
    Command::EditTable(
        source,
        TableEdit::SetCells {
            row: edit.row,
            column: edit.column,
            grid: vec![vec![edit.text]],
        },
    )
}

fn begin_edit(
    view: &mut TableView,
    table: &CsvTable,
    text: &str,
    cell: (usize, usize),
    replace_with: Option<String>,
) {
    let current = table
        .value(text, cell.0, cell.1)
        .map(|value| value.into_owned())
        .unwrap_or_default();
    view.anchor = cell;
    view.head = cell;
    view.editing = Some(CellEdit {
        row: cell.0,
        column: cell.1,
        text: replace_with.unwrap_or(current),
        needs_focus: true,
    });
}

/// Selected cells as rows of text; missing cells in short records are empty.
fn selected_grid(view: &TableView, table: &CsvTable, text: &str) -> Vec<Vec<String>> {
    let (low, high) = view.selection();
    (low.0..=high.0)
        .map(|row| {
            (low.1..=high.1)
                .map(|column| {
                    table
                        .value(text, row, column)
                        .map(|value| value.into_owned())
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect()
}

fn handle_keys(
    ui: &mut egui::Ui,
    source: DocumentId,
    view: &mut TableView,
    table: &CsvTable,
    text: &str,
    malformed: bool,
    commands: &mut Vec<Command>,
) {
    let rows = table.row_count();
    if rows == 0 {
        return;
    }
    let first = usize::from(view.header);
    let (last_row, last_column) = (rows - 1, table.columns.saturating_sub(1));
    let command = Modifiers::COMMAND;
    let mut moved = false;
    let mut step = |view: &mut TableView, row: isize, column: isize, extend: bool| {
        let target = (
            (view.head.0 as isize + row).clamp(first as isize, last_row as isize) as usize,
            (view.head.1 as isize + column).clamp(0, last_column as isize) as usize,
        );
        view.head = target;
        if !extend {
            view.anchor = target;
        }
        moved = true;
    };
    for (key, row, column) in [
        (Key::ArrowUp, -1, 0),
        (Key::ArrowDown, 1, 0),
        (Key::ArrowLeft, 0, -1),
        (Key::ArrowRight, 0, 1),
    ] {
        if ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, key)) {
            step(view, row, column, true);
        } else if ui.input_mut(|input| input.consume_key(Modifiers::NONE, key)) {
            step(view, row, column, false);
        }
    }
    // Shift variants first: a shortcut without Shift also matches a Shift press.
    if ui.input_mut(|input| input.consume_key(Modifiers::SHIFT, Key::Tab)) {
        step(view, 0, -1, false);
    } else if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab)) {
        step(view, 0, 1, false);
    }
    if moved {
        view.reveal = true;
        ui.ctx().request_repaint();
        view.head = (view.head.0.max(first), view.head.1);
    }
    if !malformed
        && (ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
            || ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::F2)))
    {
        begin_edit(view, table, text, view.head, None);
    }
    if !malformed
        && (ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Delete))
            || ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Backspace)))
    {
        let grid = selected_grid(view, table, text)
            .into_iter()
            .map(|row| row.into_iter().map(|_| String::new()).collect())
            .collect();
        let (low, _) = view.selection();
        commands.push(Command::EditTable(
            source,
            TableEdit::SetCells {
                row: low.0,
                column: low.1,
                grid,
            },
        ));
    }
    if ui.input_mut(|input| input.consume_key(command, Key::A)) {
        view.anchor = (first, 0);
        view.head = (last_row, last_column);
    }
    if !malformed
        && (ui.input_mut(|input| input.consume_key(command | Modifiers::SHIFT, Key::Z))
            || ui.input_mut(|input| input.consume_key(command, Key::Y)))
    {
        commands.push(Command::EditTable(source, TableEdit::Redo));
    } else if !malformed && ui.input_mut(|input| input.consume_key(command, Key::Z)) {
        commands.push(Command::EditTable(source, TableEdit::Undo));
    }
    let events = ui.input(|input| input.events.clone());
    for event in events {
        match event {
            egui::Event::Copy | egui::Event::Cut => {
                let grid = selected_grid(view, table, text);
                ui.ctx().copy_text(csv::clipboard_text(&grid));
                if matches!(event, egui::Event::Cut) && !malformed {
                    let (low, _) = view.selection();
                    let cleared = grid
                        .iter()
                        .map(|row| row.iter().map(|_| String::new()).collect())
                        .collect();
                    commands.push(Command::EditTable(
                        source,
                        TableEdit::SetCells {
                            row: low.0,
                            column: low.1,
                            grid: cleared,
                        },
                    ));
                }
            }
            egui::Event::Paste(pasted) if !malformed => {
                let grid = csv::parse_clipboard(&pasted);
                if !grid.is_empty() {
                    let (low, _) = view.selection();
                    let (rows, columns) =
                        (grid.len(), grid.iter().map(Vec::len).max().unwrap_or(1));
                    commands.push(Command::EditTable(
                        source,
                        TableEdit::SetCells {
                            row: low.0,
                            column: low.1,
                            grid,
                        },
                    ));
                    view.head = (low.0 + rows - 1, low.1 + columns - 1);
                    view.anchor = low;
                }
            }
            egui::Event::Text(typed)
                if !malformed
                    && !ui.input(|input| input.modifiers.command)
                    && typed.chars().all(|character| !character.is_control()) =>
            {
                begin_edit(view, table, text, view.head, Some(typed));
            }
            _ => {}
        }
    }
}

/// Structure and sort actions for the current selection.
fn table_menu(
    ui: &mut egui::Ui,
    view: &TableView,
    table: &CsvTable,
    malformed: bool,
) -> Option<TableEdit> {
    let (low, high) = view.selection();
    let first = usize::from(view.header && table.row_count() > 0);
    let mut chosen = None;
    let mut entry = |ui: &mut egui::Ui, label: &str, enabled: bool, edit: TableEdit| {
        if ui
            .add_enabled(enabled && !malformed, egui::Button::new(label))
            .clicked()
        {
            chosen = Some(edit);
        }
    };
    let rows = high.0 + 1 - low.0;
    let data = low.0 >= first;
    entry(ui, "Insert Row Above", data, TableEdit::InsertRow(low.0));
    entry(
        ui,
        "Insert Row Below",
        data || table.row_count() == 0,
        TableEdit::InsertRow(high.0 + 1),
    );
    entry(
        ui,
        if rows == 1 {
            "Delete Row"
        } else {
            "Delete Rows"
        },
        data && table.row_count() > 0,
        TableEdit::DeleteRows(low.0..high.0 + 1),
    );
    ui.separator();
    entry(
        ui,
        "Insert Column Left",
        true,
        TableEdit::InsertColumn(low.1),
    );
    entry(
        ui,
        "Insert Column Right",
        true,
        TableEdit::InsertColumn(high.1 + 1),
    );
    entry(
        ui,
        "Delete Column",
        table.columns > 0,
        TableEdit::DeleteColumn(low.1),
    );
    ui.separator();
    let last_row = table.row_count().saturating_sub(1);
    entry(
        ui,
        "Move Row Up",
        data && low.0 > first && rows == 1,
        TableEdit::MoveRow {
            from: low.0,
            to: low.0.saturating_sub(1),
        },
    );
    entry(
        ui,
        "Move Row Down",
        data && low.0 < last_row && rows == 1,
        TableEdit::MoveRow {
            from: low.0,
            to: low.0 + 1,
        },
    );
    entry(
        ui,
        "Move Column Left",
        low.1 > 0,
        TableEdit::MoveColumn {
            from: low.1,
            to: low.1.saturating_sub(1),
        },
    );
    entry(
        ui,
        "Move Column Right",
        low.1 + 1 < table.columns,
        TableEdit::MoveColumn {
            from: low.1,
            to: low.1 + 1,
        },
    );
    ui.separator();
    entry(
        ui,
        "Sort Rows by Column, A to Z",
        true,
        TableEdit::SortRows {
            first_row: first,
            column: low.1,
            descending: false,
        },
    );
    entry(
        ui,
        "Sort Rows by Column, Z to A",
        true,
        TableEdit::SortRows {
            first_row: first,
            column: low.1,
            descending: true,
        },
    );
    ui.label(
        RichText::new("Sorting rewrites the text.")
            .size(theme::META_SIZE)
            .color(Color32::GRAY),
    );
    chosen
}
