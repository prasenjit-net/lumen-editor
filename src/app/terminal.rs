//! Controller for the workspace-rooted integrated terminal.
use super::LumenApp;
use crate::services::terminal::{
    DEFAULT_COLUMNS, DEFAULT_ROWS, TerminalSession, default_working_directory,
};
use eframe::egui;

impl LumenApp {
    pub(crate) fn toggle_terminal(&mut self, ctx: &egui::Context) {
        if self.view.terminal.open {
            self.view.terminal.open = false;
            return;
        }
        self.view.terminal.open = true;
        self.view.problems.open = false;
        self.view.output.open = false;
        self.view.task_runs.panel_open = false;
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("terminal_canvas")));
        if self.terminal_session.is_none() {
            self.start_terminal(ctx);
        }
    }

    pub(crate) fn send_terminal_input(&mut self, input: &[u8], ctx: &egui::Context) {
        let Some(session) = &mut self.terminal_session else {
            self.view.terminal.error = Some("Terminal is not running".into());
            return;
        };
        if let Err(error) = session.write(input) {
            self.view.terminal.error = Some(error);
        }
        ctx.request_repaint();
    }

    pub(crate) fn close_terminal(&mut self) {
        self.terminal_session = None;
        self.view.terminal.emulator = None;
        self.view.terminal.open = false;
    }

    pub(crate) fn clear_terminal(&mut self) {
        if let Some(emulator) = &mut self.view.terminal.emulator {
            emulator.clear();
        }
        self.view.terminal.error = None;
    }

    pub(crate) fn resize_terminal(&mut self, rows: u16, columns: u16) {
        if self.view.terminal.applied_rows == rows && self.view.terminal.applied_columns == columns
        {
            return;
        }
        if let Some(session) = &self.terminal_session
            && let Err(error) = session.resize(rows, columns)
        {
            self.view.terminal.error = Some(error);
        }
        self.view.terminal.applied_rows = rows;
        self.view.terminal.applied_columns = columns;
        if let Some(emulator) = &mut self.view.terminal.emulator {
            emulator.resize(rows as usize, columns as usize);
        }
    }

    pub(crate) fn poll_terminal(&mut self, ctx: &egui::Context) {
        let Some(session) = &mut self.terminal_session else {
            return;
        };
        let chunks = session.drain_output();
        if !chunks.is_empty() {
            if let Some(emulator) = &mut self.view.terminal.emulator {
                for chunk in chunks {
                    emulator.feed(&chunk);
                }
            }
            self.view.terminal.scroll_to_bottom =
                self.view.settings.boolean("terminal.auto_scroll");
            ctx.request_repaint();
        }
        match session.has_exited() {
            Ok(true) => {
                self.terminal_session = None;
                self.view.terminal.error = Some("Terminal shell exited".into());
            }
            Ok(false) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(error) => self.view.terminal.error = Some(error),
        }
    }

    fn start_terminal(&mut self, ctx: &egui::Context) {
        let root = self
            .editor
            .workspace
            .clone()
            .unwrap_or_else(default_working_directory);
        // Start at the size the panel last measured so the shell's first prompt
        // is drawn for the visible rows; fall back to defaults before any layout.
        let measured = |value: u16, default: u16| if value == 0 { default } else { value };
        let rows = measured(self.view.terminal.rows, DEFAULT_ROWS);
        let columns = measured(self.view.terminal.columns, DEFAULT_COLUMNS);
        match TerminalSession::start(&root, rows, columns) {
            Ok(session) => {
                self.terminal_session = Some(session);
                self.view.terminal.error = None;
                self.view.terminal.rows = rows;
                self.view.terminal.columns = columns;
                self.view.terminal.applied_rows = rows;
                self.view.terminal.applied_columns = columns;
                self.view.terminal.emulator = Some(lumen_terminal_core::Terminal::new(
                    rows as usize,
                    columns as usize,
                ));
                self.view.terminal.scroll_to_bottom = true;
                self.view.status = format!("Terminal started in {}", root.display());
                ctx.request_repaint();
            }
            Err(error) => self.view.terminal.error = Some(error),
        }
    }
}
