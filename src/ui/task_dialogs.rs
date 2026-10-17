//! Prompts around running tasks: first-run review and unsaved files, closing a
//! running task's tab, changing folder with tasks running, and quitting with them.
//! Each answer is a typed command; cancelling any of them changes nothing.
use super::{
    Workbench,
    dialogs::{escape_pressed, keep_focus, primary_button},
};
use crate::app::command::Command;
use eframe::egui::{self, RichText, Vec2};

fn centered(title: &str) -> egui::Window<'_> {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
}

impl Workbench<'_> {
    /// Review a workspace task before its first run, and offer to save dirty files.
    pub(crate) fn task_run_dialog(&mut self, ctx: &egui::Context) {
        let colors = super::theme::palette(&ctx.style().visuals);
        let Some(pending) = self.view.task_pending.as_ref() else {
            return;
        };
        let title = if pending.needs_review {
            "Review task before running"
        } else {
            "Unsaved files"
        };
        let (label, command_line, cwd, needs_review, dirty, env_names) = (
            pending.label.clone(),
            pending.command_line(),
            pending.resolved.cwd.display().to_string(),
            pending.needs_review,
            pending.dirty_files.clone(),
            pending.resolved.env.keys().cloned().collect::<Vec<_>>(),
        );
        let mut answer = None;
        centered(title).show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.add(egui::Label::new(RichText::new(format!("Run '{label}'?")).strong()).wrap());
            if needs_review {
                ui.add_space(4.0);
                ui.label(RichText::new("Command").size(11.0).color(colors.muted));
                ui.add(egui::Label::new(RichText::new(&command_line).monospace()).wrap());
                ui.label(
                    RichText::new("Working directory")
                        .size(11.0)
                        .color(colors.muted),
                );
                ui.add(egui::Label::new(RichText::new(&cwd).monospace()).wrap());
                if !env_names.is_empty() {
                    ui.label(
                        RichText::new(format!("Sets: {}", env_names.join(", ")))
                            .size(11.0)
                            .color(colors.muted),
                    );
                }
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "It runs on your computer with your permissions. You will not be \
                             asked again unless the command changes.",
                        )
                        .color(colors.muted),
                    )
                    .wrap(),
                );
            }
            if !dirty.is_empty() {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Unsaved files: {}. The task sees the contents on disk.",
                            dirty.join(", ")
                        ))
                        .color(colors.warning),
                    )
                    .wrap(),
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if dirty.is_empty() {
                    let run = primary_button(ui, "Run");
                    let cancel = ui.button("Cancel");
                    keep_focus(ui.ctx(), &[&run, &cancel], &cancel);
                    if run.clicked() {
                        answer = Some(Command::ConfirmTaskRun(false));
                    } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                        answer = Some(Command::CancelTaskRun);
                    }
                } else {
                    let save = primary_button(ui, "Save All and Run");
                    let run = ui.button("Run without Saving");
                    let cancel = ui.button("Cancel");
                    keep_focus(ui.ctx(), &[&save, &run, &cancel], &save);
                    if save.clicked() {
                        answer = Some(Command::ConfirmTaskRun(true));
                    } else if run.clicked() {
                        answer = Some(Command::ConfirmTaskRun(false));
                    } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                        answer = Some(Command::CancelTaskRun);
                    }
                }
            });
        });
        if let Some(command) = answer {
            self.command(command);
        }
    }

    pub(crate) fn task_run_close_dialog(&mut self, ctx: &egui::Context) {
        let colors = super::theme::palette(&ctx.style().visuals);
        let Some(id) = self.view.task_run_close_prompt else {
            return;
        };
        let Some(title) = self
            .view
            .task_runs
            .get(id)
            .filter(|run| run.is_active())
            .map(|_| self.view.task_runs.title(id))
        else {
            self.view.task_run_close_prompt = None;
            return;
        };
        let mut answer = None;
        centered("Task is running").show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.add(
                egui::Label::new(format!(
                    "'{title}' is still running. Stop it and close its tab?"
                ))
                .wrap(),
            );
            ui.label(
                RichText::new("Its process and everything it started will be stopped.")
                    .color(colors.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let stop = primary_button(ui, "Stop and Close");
                let cancel = ui.button("Cancel");
                keep_focus(ui.ctx(), &[&stop, &cancel], &cancel);
                if stop.clicked() {
                    answer = Some(Command::StopAndCloseTaskRun(id));
                } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                    answer = Some(Command::CancelCloseTaskRun);
                }
            });
        });
        if let Some(command) = answer {
            self.command(command);
        }
    }

    pub(crate) fn task_workspace_dialog(&mut self, ctx: &egui::Context) {
        let colors = super::theme::palette(&ctx.style().visuals);
        let Some(pending) = self.view.task_workspace_switch.as_ref() else {
            return;
        };
        let stopping = pending.stopping;
        let names: Vec<String> = self.workspace_run_titles();
        let mut answer = None;
        centered("Tasks are running").show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.add(
                egui::Label::new(format!(
                    "These tasks belong to the current folder: {}.",
                    names.join(", ")
                ))
                .wrap(),
            );
            ui.label(
                RichText::new(if stopping {
                    "Stopping them…"
                } else {
                    "Stop them and switch folders, or stay where you are?"
                })
                .color(colors.muted),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!stopping, |ui| {
                    let stop = primary_button(ui, "Stop Tasks and Switch");
                    let cancel = ui.button("Cancel");
                    keep_focus(ui.ctx(), &[&stop, &cancel], &cancel);
                    if stop.clicked() {
                        answer = Some(Command::ConfirmWorkspaceSwitch);
                    } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                        answer = Some(Command::CancelWorkspaceSwitch);
                    }
                });
            });
        });
        if let Some(command) = answer {
            self.command(command);
        }
    }

    pub(crate) fn task_quit_dialog(&mut self, ctx: &egui::Context) {
        let colors = super::theme::palette(&ctx.style().visuals);
        let Some(pending) = self.view.task_quit.as_ref() else {
            return;
        };
        let stopping = pending.deadline.is_some();
        let names: Vec<String> = self
            .view
            .task_runs
            .runs()
            .iter()
            .filter(|run| run.is_active())
            .map(|run| self.view.task_runs.title(run.id))
            .collect();
        let mut answer = None;
        centered("Tasks are running").show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.add(egui::Label::new(format!("Still running: {}.", names.join(", "))).wrap());
            ui.label(
                RichText::new(if stopping {
                    "Stopping them, then quitting…"
                } else {
                    "Quitting stops them and everything they started."
                })
                .color(colors.muted),
            );
            ui.add_space(10.0);
            ui.add_enabled_ui(!stopping, |ui| {
                ui.horizontal(|ui| {
                    let stop = primary_button(ui, "Stop Tasks and Quit");
                    let cancel = ui.button("Cancel");
                    keep_focus(ui.ctx(), &[&stop, &cancel], &cancel);
                    if stop.clicked() {
                        answer = Some(Command::ConfirmTaskQuit);
                    } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                        answer = Some(Command::CancelTaskQuit);
                    }
                });
            });
        });
        if let Some(command) = answer {
            self.command(command);
        }
    }

    fn workspace_run_titles(&self) -> Vec<String> {
        self.view
            .task_runs
            .runs()
            .iter()
            .filter(|run| {
                run.is_active()
                    && run.key.scope == lumen_task_core::Scope::Workspace
                    && run.workspace == self.editor.workspace
            })
            .map(|run| self.view.task_runs.title(run.id))
            .collect()
    }
}
