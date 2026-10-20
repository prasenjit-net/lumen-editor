//! Output panel controller: panel switching and mirroring editor activity into
//! the process log.
use super::LumenApp;
use crate::{
    model::output_log::{self, LogLevel},
    services::logger,
};
use eframe::egui;

/// Drop any text selection made in the previous bottom-panel view.
pub(crate) fn clear_text_selection(ctx: &egui::Context) {
    ctx.plugin::<egui::text_selection::LabelSelectionState>()
        .lock()
        .clear_selection();
}

/// A view of the shared bottom slot, remembered across hiding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BottomView {
    Terminal,
    Output,
    Problems,
    Run(super::task_runs::RunId),
}

impl LumenApp {
    /// The view the bottom slot is drawing now, if any.
    pub(crate) fn visible_bottom_view(&self) -> Option<BottomView> {
        let view = &self.view;
        if view.terminal.open {
            Some(BottomView::Terminal)
        } else if view.output.open {
            Some(BottomView::Output)
        } else if view.problems.open {
            Some(BottomView::Problems)
        } else if view.task_runs.panel_open {
            view.task_runs
                .selected_run()
                .map(|run| BottomView::Run(run.id))
        } else {
            None
        }
    }

    /// Hide the bottom panel (terminal sessions and task runs keep going), or
    /// bring back the view it last showed, falling back to the terminal.
    pub(crate) fn toggle_bottom_panel(&mut self, ctx: &egui::Context) {
        if let Some(shown) = self.visible_bottom_view() {
            self.view.bottom_restore = Some(shown);
            self.view.terminal.open = false;
            self.view.output.open = false;
            self.view.problems.open = false;
            self.view.task_runs.panel_open = false;
            return;
        }
        match self.view.bottom_restore {
            Some(BottomView::Output) => self.show_output(),
            Some(BottomView::Problems) => self.show_problems(),
            Some(BottomView::Run(id)) if self.view.task_runs.get(id).is_some() => {
                self.show_task_run(id);
            }
            _ => self.toggle_terminal(ctx),
        }
    }

    /// Open the Output tab in the bottom panel (the terminal session keeps running).
    pub(crate) fn show_output(&mut self) {
        self.view.output.open = true;
        self.view.terminal.open = false;
        self.view.problems.open = false;
        self.view.task_runs.panel_open = false;
        self.view.output.scroll_to_bottom = true;
    }

    /// Open the Problems tab in the bottom panel (the terminal session keeps running).
    pub(crate) fn show_problems(&mut self) {
        self.view.problems.open = true;
        self.view.terminal.open = false;
        self.view.output.open = false;
        self.view.task_runs.panel_open = false;
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

    /// The log is process-wide; tests that read it back or clear it take turns.
    static LOG_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    fn the_bottom_toggle_hides_and_restores_the_last_view() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        app.execute(Command::ShowProblems, &ctx);
        app.execute(Command::ToggleBottomPanel, &ctx);
        assert_eq!(app.visible_bottom_view(), None);
        app.execute(Command::ToggleBottomPanel, &ctx);
        assert_eq!(app.visible_bottom_view(), Some(BottomView::Problems));
        app.execute(Command::ShowOutput, &ctx);
        app.execute(Command::ToggleBottomPanel, &ctx);
        app.execute(Command::ToggleBottomPanel, &ctx);
        assert_eq!(app.visible_bottom_view(), Some(BottomView::Output));
    }

    #[test]
    fn problems_shares_the_bottom_slot_with_terminal_output_and_task_runs() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        app.view.output.open = true;
        app.execute(Command::ShowProblems, &ctx);
        assert!(app.view.problems.open && !app.view.output.open);
        // Showing it again keeps it open (a tab click), unlike the toggle.
        app.execute(Command::ShowProblems, &ctx);
        assert!(app.view.problems.open);
        app.execute(Command::ShowOutput, &ctx);
        assert!(app.view.output.open && !app.view.problems.open);
        app.view.task_runs.panel_open = true;
        app.execute(Command::ShowProblems, &ctx);
        assert!(app.view.problems.open && !app.view.task_runs.panel_open);
        app.execute(Command::CloseProblems, &ctx);
        assert!(!app.view.problems.open);
    }

    #[test]
    fn status_and_terminal_changes_are_logged_once_with_a_level() {
        let _turn = LOG_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let _turn = LOG_TESTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
