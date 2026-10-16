//! Output panel controller: panel switching and mirroring editor activity into
//! the process log.
use super::LumenApp;
use crate::{
    model::output_log::{self, LogLevel},
    services::logger,
};
use eframe::egui;

impl LumenApp {
    /// Open the Output tab in the bottom panel (the terminal session keeps running).
    pub(crate) fn show_output(&mut self) {
        self.view.output.open = true;
        self.view.terminal.open = false;
        self.view.problems.open = false;
        self.view.output.scroll_to_bottom = true;
    }

    pub(crate) fn toggle_output(&mut self) {
        if self.view.output.open {
            self.view.output.open = false;
        } else {
            self.show_output();
        }
    }

    pub(crate) fn clear_output(&mut self) {
        logger::clear();
        self.view.output.entries.clear();
        logger::info("output", "Output cleared");
    }

    pub(crate) fn copy_output(&mut self, ctx: &egui::Context) {
        ctx.copy_text(output_log::text_of(&self.view.output.entries));
        self.view.status = "Copied output log".into();
    }

    /// Refresh the panel's snapshot when anything was logged since the last frame.
    pub(crate) fn sync_output(&mut self) {
        let latest = logger::last_sequence();
        if latest != self.view.output.seen {
            self.view.output.seen = latest;
            self.view.output.entries = logger::snapshot();
        }
    }

    /// Record status-bar messages and terminal errors as they change, so every
    /// user-visible outcome (including failures) is also in the log.
    pub(crate) fn log_status_changes(&mut self) {
        if self.view.status != self.logged_status {
            self.logged_status.clone_from(&self.view.status);
            let message = self.logged_status.clone();
            logger::log(output_log::classify_status(&message), "editor", &message);
        }
        if self.view.terminal.error != self.logged_terminal_error {
            self.logged_terminal_error
                .clone_from(&self.view.terminal.error);
            if let Some(error) = self.logged_terminal_error.clone() {
                logger::log(LogLevel::Error, "terminal", &error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::Command;

    #[test]
    fn bottom_panel_slot_holds_one_view_at_a_time() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        app.view.terminal.open = true;
        app.execute(Command::ShowOutput, &ctx);
        assert!(app.view.output.open && !app.view.terminal.open);

        app.execute(Command::ToggleProblems, &ctx);
        assert!(app.view.problems.open && !app.view.output.open);
        app.execute(Command::ToggleOutput, &ctx);
        assert!(app.view.output.open && !app.view.problems.open);
        app.execute(Command::ToggleOutput, &ctx);
        assert!(!app.view.output.open);
        app.execute(Command::ToggleOutput, &ctx);
        app.execute(Command::CloseOutput, &ctx);
        assert!(!app.view.output.open);
    }

    #[test]
    fn status_and_terminal_changes_are_logged_once_with_a_level() {
        let mut app = LumenApp::default();
        app.view.status = "Save failed: output-test-unique-1".into();
        app.log_status_changes();
        app.log_status_changes();
        app.view.terminal.error = Some("output-test-unique-2".into());
        app.log_status_changes();
        app.sync_output();
        let find = |text: &str| {
            app.view
                .output
                .entries
                .iter()
                .filter(|entry| entry.message == text)
                .collect::<Vec<_>>()
        };
        let saved = find("Save failed: output-test-unique-1");
        assert_eq!(saved.len(), 1, "an unchanged status is not logged again");
        assert_eq!(
            (saved[0].level, saved[0].source.as_str()),
            (LogLevel::Error, "editor")
        );
        let terminal = find("output-test-unique-2");
        assert_eq!(
            (terminal[0].level, terminal[0].source.as_str()),
            (LogLevel::Error, "terminal")
        );
    }

    #[test]
    fn clearing_empties_the_panel_and_records_that_it_happened() {
        let mut app = LumenApp::default();
        logger::info("test", "output-test-before-clear");
        app.sync_output();
        assert!(
            app.view
                .output
                .entries
                .iter()
                .any(|e| e.message == "output-test-before-clear")
        );
        app.clear_output();
        app.sync_output();
        assert!(
            !app.view
                .output
                .entries
                .iter()
                .any(|e| e.message == "output-test-before-clear")
        );
        assert!(
            app.view
                .output
                .entries
                .iter()
                .any(|e| e.message == "Output cleared")
        );
    }
}
