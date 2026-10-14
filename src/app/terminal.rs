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
            self.view.terminal.focus_input = false;
            return;
        }
        self.view.terminal.open = true;
        self.view.terminal.focus_input = true;
        if self.terminal_session.is_none() {
            self.start_terminal(ctx);
        }
    }

    pub(crate) fn submit_terminal_input(&mut self, ctx: &egui::Context) {
        let input = std::mem::take(&mut self.view.terminal.input);
        let Some(session) = &mut self.terminal_session else {
            self.view.terminal.error = Some("Terminal is not running".into());
            return;
        };
        if let Err(error) = session.write_line(&input) {
            self.view.terminal.error = Some(error);
        } else {
            record_terminal_history(&mut self.view.terminal.history, &input);
        }
        self.view.terminal.history_index = None;
        self.view.terminal.history_draft.clear();
        self.view.terminal.focus_input = true;
        ctx.request_repaint();
    }

    pub(crate) fn close_terminal(&mut self) {
        self.terminal_session = None;
        self.view.terminal.emulator = None;
        self.view.terminal.open = false;
        self.view.terminal.focus_input = false;
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
            self.view.terminal.scroll_to_bottom = true;
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
        let rows = self.view.terminal.rows.max(DEFAULT_ROWS);
        let columns = self.view.terminal.columns.max(DEFAULT_COLUMNS);
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

pub(crate) fn recall_terminal_history(
    terminal: &mut super::view_state::TerminalView,
    direction: std::cmp::Ordering,
) {
    let Some(last) = terminal.history.len().checked_sub(1) else {
        return;
    };
    match (terminal.history_index, direction) {
        (None, std::cmp::Ordering::Less) => {
            terminal.history_draft.clone_from(&terminal.input);
            terminal.history_index = Some(last);
        }
        (Some(index), std::cmp::Ordering::Less) if index > 0 => {
            terminal.history_index = Some(index - 1);
        }
        (Some(index), std::cmp::Ordering::Greater) if index < last => {
            terminal.history_index = Some(index + 1);
        }
        (Some(_), std::cmp::Ordering::Greater) => {
            terminal.history_index = None;
            terminal.input.clone_from(&terminal.history_draft);
            return;
        }
        _ => return,
    }
    if let Some(index) = terminal.history_index {
        terminal.input.clone_from(&terminal.history[index]);
    }
}

fn record_terminal_history(history: &mut Vec<String>, input: &str) {
    if input.is_empty() {
        return;
    }
    history.push(input.into());
    const MAX_HISTORY: usize = 200;
    if history.len() > MAX_HISTORY {
        history.drain(..history.len() - MAX_HISTORY);
    }
}

#[cfg(test)]
mod tests {
    use super::recall_terminal_history;
    use crate::app::view_state::TerminalView;
    use std::cmp::Ordering;

    #[test]
    fn history_recall_restores_the_unsubmitted_draft() {
        let mut terminal = TerminalView {
            input: "draft".into(),
            history: vec!["first".into(), "second".into()],
            ..Default::default()
        };
        recall_terminal_history(&mut terminal, Ordering::Less);
        assert_eq!(terminal.input, "second");
        recall_terminal_history(&mut terminal, Ordering::Less);
        assert_eq!(terminal.input, "first");
        recall_terminal_history(&mut terminal, Ordering::Greater);
        assert_eq!(terminal.input, "second");
        recall_terminal_history(&mut terminal, Ordering::Greater);
        assert_eq!(terminal.input, "draft");
    }
}
