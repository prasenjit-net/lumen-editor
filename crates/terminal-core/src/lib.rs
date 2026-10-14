//! Terminal-emulator state for Lumen.
//!
//! This crate adapts the Alacritty-backed state model used by `egui_term`, but
//! deliberately contains no egui or PTY code. Lumen's service layer owns the
//! native shell, while the workbench renders the snapshots exposed here.

use alacritty_terminal::{
    event::VoidListener,
    grid::Dimensions,
    index::{Column, Line},
    term::{self, Term},
};
use vte::ansi::Color;

/// A terminal screen cell ready for a GUI renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub character: char,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
}

/// Portable color information from the terminal's ANSI state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalColor {
    Named(u16),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The visible screen, in row-major order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub rows: usize,
    pub columns: usize,
    pub cells: Vec<Cell>,
}

impl Screen {
    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        (row < self.rows && column < self.columns).then(|| &self.cells[row * self.columns + column])
    }

    pub fn line(&self, row: usize) -> String {
        if row >= self.rows {
            return String::new();
        }
        let start = row * self.columns;
        self.cells[start..start + self.columns]
            .iter()
            .map(|cell| cell.character)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    pub fn row(&self, row: usize) -> Option<&[Cell]> {
        (row < self.rows).then(|| {
            let start = row * self.columns;
            &self.cells[start..start + self.columns]
        })
    }
}

/// ANSI/VT parser and screen grid.
pub struct Terminal {
    term: Term<VoidListener>,
    parser: vte::ansi::Processor,
    size: Size,
}

impl Terminal {
    pub fn new(rows: usize, columns: usize) -> Self {
        let size = Size::new(rows, columns);
        Self {
            term: Term::new(term::Config::default(), &size, VoidListener),
            parser: vte::ansi::Processor::new(),
            size,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, rows: usize, columns: usize) {
        let size = Size::new(rows, columns);
        if size == self.size {
            return;
        }
        self.term.resize(size);
        self.size = size;
    }

    pub fn clear(&mut self) {
        // The workbench Clear action is an output reset, not the shell's
        // escape-sequence-only erase command: discard retained scrollback too.
        self.term.grid_mut().reset::<Color>();
    }

    pub fn screen(&self) -> Screen {
        // Preserve Alacritty's scrollback rather than presenting only the
        // viewport. The egui panel owns scrolling, while the emulator stays at
        // the live bottom of its buffer.
        let columns = self.size.columns;
        let grid = self.term.grid();
        let history = grid.history_size();
        let rows = grid.total_lines();
        let mut cells = Vec::with_capacity(rows * columns);
        for row in 0..rows {
            for column in 0..columns {
                let cell = &grid[Line(row as i32 - history as i32)][Column(column)];
                cells.push(Cell {
                    character: cell.c,
                    foreground: terminal_color(cell.fg),
                    background: terminal_color(cell.bg),
                });
            }
        }
        Screen {
            rows,
            columns,
            cells,
        }
    }
}

fn terminal_color(color: Color) -> TerminalColor {
    match color {
        Color::Named(named) => TerminalColor::Named(named as u16),
        Color::Indexed(index) => TerminalColor::Indexed(index),
        Color::Spec(rgb) => TerminalColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    screen_lines: usize,
    columns: usize,
}

impl Size {
    fn new(rows: usize, columns: usize) -> Self {
        Self {
            screen_lines: rows.max(1),
            columns: columns.max(1),
        }
    }
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[cfg(test)]
mod tests {
    use super::{Terminal, TerminalColor};

    #[test]
    fn ansi_sequences_update_the_screen_without_leaking_control_text() {
        let mut terminal = Terminal::new(2, 8);
        terminal.feed(b"one\r\n\x1b[31mtwo\x1b[0m");
        let screen = terminal.screen();
        assert_eq!(screen.line(0), "one");
        assert_eq!(screen.line(1), "two");
    }

    #[test]
    fn clear_screen_and_resize_are_emulated() {
        let mut terminal = Terminal::new(2, 4);
        terminal.feed(b"abcd");
        terminal.clear();
        terminal.feed(b"ok");
        assert_eq!(terminal.screen().line(0), "ok");
        terminal.resize(3, 6);
        let screen = terminal.screen();
        assert_eq!(screen.rows, 3);
        assert_eq!(screen.columns, 6);
    }

    #[test]
    fn ansi_cell_colours_are_retained_for_the_renderer() {
        let mut terminal = Terminal::new(1, 4);
        terminal.feed(b"\x1b[31mR\x1b[0m");
        let screen = terminal.screen();
        let cell = screen.cell(0, 0).expect("first cell");
        assert_eq!(cell.character, 'R');
        assert_eq!(cell.foreground, TerminalColor::Named(1));
    }

    #[test]
    fn screen_includes_scrollback_before_the_visible_rows() {
        let mut terminal = Terminal::new(2, 8);
        terminal.feed(b"one\r\ntwo\r\nthree");
        let screen = terminal.screen();
        assert_eq!(screen.rows, 3);
        assert_eq!(screen.line(0), "one");
        assert_eq!(screen.line(2), "three");
    }
}
