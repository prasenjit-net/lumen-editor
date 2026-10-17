//! The tab strip shared by the Terminal, Output and task-run panels. They draw in
//! the same bottom slot, so each lists every sibling: TERMINAL, OUTPUT, then one
//! tab per task run, keyed by `RunId`.
use super::{Workbench, widgets::PanelTab};
use crate::{
    app::{
        command::Command,
        task_runs::{RunOutcome, RunState},
    },
    icon,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveBottom {
    Terminal,
    Output,
    /// A task run tab is showing; which one is `task_runs.selected`.
    Run,
}

pub(super) struct BottomTabEntry {
    pub(super) label: String,
    pub(super) tooltip: String,
    pub(super) active: bool,
    pub(super) command: Command,
}

pub(super) fn as_panel_tabs(entries: &[BottomTabEntry]) -> Vec<PanelTab<'_>> {
    entries
        .iter()
        .map(|entry| PanelTab {
            label: &entry.label,
            active: entry.active,
            command: entry.command.clone(),
            tooltip: &entry.tooltip,
        })
        .collect()
}

impl Workbench<'_> {
    pub(super) fn bottom_tab_entries(&self, active: ActiveBottom) -> Vec<BottomTabEntry> {
        let mut entries = vec![
            BottomTabEntry {
                label: "TERMINAL".into(),
                tooltip: String::new(),
                active: active == ActiveBottom::Terminal,
                command: Command::ShowTerminal,
            },
            BottomTabEntry {
                label: "OUTPUT".into(),
                tooltip: String::new(),
                active: active == ActiveBottom::Output,
                command: Command::ShowOutput,
            },
        ];
        let runs = &self.view.task_runs;
        for run in runs.runs() {
            let glyph = match &run.state {
                RunState::Running | RunState::Stopping => icon::PLAY,
                RunState::Finished(RunOutcome::Succeeded(_) | RunOutcome::Completed) => icon::CHECK,
                RunState::Finished(RunOutcome::Cancelled) => icon::STOP,
                RunState::Finished(RunOutcome::TimedOut) => icon::WARNING,
                RunState::Finished(_) => icon::ERROR,
            };
            entries.push(BottomTabEntry {
                label: format!("{glyph} {}", runs.title(run.id)),
                tooltip: format!(
                    "{}{} task {} · {}\n{}\nStarted {} in {}",
                    if run.summary {
                        "Task group: ".to_string()
                    } else {
                        runs.parent_label(run.id)
                            .map(|parent| format!("Part of '{parent}': "))
                            .unwrap_or_default()
                    },
                    run.key.scope.label(),
                    run.key.id,
                    match &run.state {
                        RunState::Running => "running".to_string(),
                        RunState::Stopping => "stopping".to_string(),
                        RunState::Finished(outcome) => outcome.summary(),
                    },
                    run.command_line,
                    run.started_at,
                    run.cwd.display()
                ),
                active: active == ActiveBottom::Run && runs.selected == Some(run.id),
                command: Command::ShowTaskRun(run.id),
            });
        }
        entries
    }
}
