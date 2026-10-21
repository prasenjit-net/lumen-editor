//! Prompts around running tasks: first-run review and unsaved files, closing a
//! running task's tab, changing folder with tasks running, and quitting with them.
//! Each answer is a typed command; cancelling any of them changes nothing.
use super::{
    Workbench,
    dialog::{Dialog, danger_button, footer},
    dialogs::{escape_pressed, keep_focus, primary_button},
};
use crate::app::command::Command;
use eframe::egui::{self, RichText};

impl Workbench<'_> {
    /// Review a workspace task before its first run, and offer to save dirty files.
    pub(crate) fn task_run_dialog(&mut self, ctx: &egui::Context) {
        let colors = super::theme::palette(&ctx.style().visuals);
        let Some(pending) = self.view.task_pending.as_ref() else {
            return;
        };
        let title = if pending.needs_review() {
            "Review task before running"
        } else {
            "Unsaved files"
        };
        let label = pending.plan.label.clone();
        let task_count = pending
            .plan
            .nodes
            .iter()
            .filter(|n| n.resolved.is_some())
            .count();
        let items: Vec<_> = pending
            .review
            .iter()
            .map(|item| {
                (
                    item.label.clone(),
                    item.command_line.clone(),
                    item.cwd.clone(),
                    item.env_names.clone(),
                )
            })
            .collect();
        let dirty = pending.dirty_files.clone();
        let mut answer = None;
        let dialog = Dialog::new(title).width(480.0).show(ctx, |ui| {
            ui.add(egui::Label::new(RichText::new(format!("Run '{label}'?")).strong()).wrap());
            if task_count > 1 {
                ui.label(
                    RichText::new(format!("This starts {task_count} tasks."))
                        .size(11.0)
                        .color(colors.muted),
                );
            }
            for (name, command_line, cwd, env_names) in &items {
                ui.add_space(4.0);
                if items.len() > 1 {
                    ui.label(RichText::new(name).strong().size(11.0));
                }
                ui.label(RichText::new("Command").size(11.0).color(colors.muted));
                ui.add(egui::Label::new(RichText::new(command_line).monospace()).wrap());
                ui.label(
                    RichText::new("Working directory")
                        .size(11.0)
                        .color(colors.muted),
                );
                ui.add(egui::Label::new(RichText::new(cwd).monospace()).wrap());
                if !env_names.is_empty() {
                    ui.label(
                        RichText::new(format!("Sets: {}", env_names.join(", ")))
                            .size(11.0)
                            .color(colors.muted),
                    );
                }
            }
            if !items.is_empty() {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "These run on your computer with your permissions. You will not be \
                             asked again unless a command changes.",
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
            footer(ui, |ui| {
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
                    let cancel = ui.button("Cancel");
                    let run = ui.button("Run without Saving");
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
        if dialog.close_clicked {
            answer = Some(Command::CancelTaskRun);
        }
        if let Some(command) = answer {
            self.command(command);
        }
    }

    /// Ask for a run's declared inputs. Secrets are masked; the answers live only in
    /// this prompt until submitted, and cancelling starts nothing.
    pub(crate) fn task_inputs_dialog(&mut self, ctx: &egui::Context) {
        let colors = super::theme::palette(&ctx.style().visuals);
        let Some(pending) = self.view.task_inputs.as_mut() else {
            return;
        };
        let title = pending.key.id.clone();
        let complete = pending.complete();
        let mut answer = None;
        let dialog = Dialog::new("Run task").width(440.0).show(ctx, |ui| {
            ui.add(
                egui::Label::new(RichText::new(format!("Inputs for '{title}'")).strong()).wrap(),
            );
            ui.add_space(6.0);
            for (index, prompt) in pending.prompts.iter_mut().enumerate() {
                ui.label(RichText::new(&prompt.prompt).size(11.0).color(colors.muted));
                match prompt.kind {
                    lumen_task_core::InputKind::Choice => {
                        egui::ComboBox::from_id_salt(("task_input", index))
                            .selected_text(prompt.value.clone())
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for option in &prompt.options {
                                    ui.selectable_value(&mut prompt.value, option.clone(), option);
                                }
                            });
                    }
                    kind => {
                        let edit = egui::TextEdit::singleline(&mut prompt.value)
                            .id_salt(("task_input", index))
                            .password(kind == lumen_task_core::InputKind::Secret)
                            .desired_width(f32::INFINITY);
                        let response = ui.add(edit);
                        if index == 0
                            && !response.has_focus()
                            && ui.memory(|m| m.focused().is_none())
                        {
                            response.request_focus();
                        }
                        if kind == lumen_task_core::InputKind::Secret {
                            ui.label(
                                RichText::new("Used for this run only; never saved or shown.")
                                    .size(11.0)
                                    .color(colors.muted),
                            );
                        }
                    }
                }
                ui.add_space(4.0);
            }
            footer(ui, |ui| {
                ui.add_enabled_ui(complete, |ui| {
                    if primary_button(ui, "Run").clicked() {
                        answer = Some(Command::SubmitTaskInputs);
                    }
                });
                if ui.button("Cancel").clicked() || escape_pressed(ui.ctx()) {
                    answer = Some(Command::CancelTaskInputs);
                }
            });
        });
        if dialog.close_clicked {
            answer = Some(Command::CancelTaskInputs);
        }
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
        let dialog = Dialog::new("Task is running").show(ctx, |ui| {
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
            footer(ui, |ui| {
                let stop = danger_button(ui, "Stop and Close");
                let cancel = ui.button("Cancel");
                keep_focus(ui.ctx(), &[&stop, &cancel], &cancel);
                if stop.clicked() {
                    answer = Some(Command::StopAndCloseTaskRun(id));
                } else if cancel.clicked() || escape_pressed(ui.ctx()) {
                    answer = Some(Command::CancelCloseTaskRun);
                }
            });
        });
        if dialog.close_clicked {
            answer = Some(Command::CancelCloseTaskRun);
        }
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
        let dialog = Dialog::new("Tasks are running")
            .width(440.0)
            .show(ctx, |ui| {
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
                footer(ui, |ui| {
                    ui.add_enabled_ui(!stopping, |ui| {
                        let stop = danger_button(ui, "Stop Tasks and Switch");
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
        if dialog.close_clicked && !stopping {
            answer = Some(Command::CancelWorkspaceSwitch);
        }
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
        let dialog = Dialog::new("Tasks are running")
            .width(440.0)
            .show(ctx, |ui| {
                ui.add(egui::Label::new(format!("Still running: {}.", names.join(", "))).wrap());
                ui.label(
                    RichText::new(if stopping {
                        "Stopping them, then quitting…"
                    } else {
                        "Quitting stops them and everything they started."
                    })
                    .color(colors.muted),
                );
                ui.add_enabled_ui(!stopping, |ui| {
                    footer(ui, |ui| {
                        let stop = danger_button(ui, "Stop Tasks and Quit");
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
        if dialog.close_clicked && !stopping {
            answer = Some(Command::CancelTaskQuit);
        }
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

#[cfg(test)]
mod tests {
    use crate::app::{
        LumenApp,
        task_runs::{InputPrompt, PendingInputs},
    };
    use eframe::egui;
    use lumen_task_core::{InputKind, Scope, TaskKey};

    fn app_with_prompts() -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        app.view.task_inputs = Some(PendingInputs {
            key: TaskKey::new(Scope::Workspace, "deploy"),
            prompts: vec![
                InputPrompt {
                    scope: Scope::Workspace,
                    id: "env".into(),
                    prompt: "Environment".into(),
                    kind: InputKind::Choice,
                    options: vec!["staging".into(), "prod".into()],
                    value: "staging".into(),
                },
                InputPrompt {
                    scope: Scope::Workspace,
                    id: "token".into(),
                    prompt: "API token".into(),
                    kind: InputKind::Secret,
                    options: Vec::new(),
                    value: "hunter2".into(),
                },
            ],
            collected: Default::default(),
        });
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        (app, ctx)
    }

    fn texts(app: &mut LumenApp, ctx: &egui::Context) -> String {
        let mut all = String::new();
        for _ in 0..3 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.task_inputs_dialog(ctx)),
            );
            all = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
        all
    }

    #[test]
    fn the_prompt_lists_every_input_and_never_draws_a_secret() {
        let (mut app, ctx) = app_with_prompts();
        let shown = texts(&mut app, &ctx);
        assert!(shown.contains("Inputs for 'deploy'"), "{shown}");
        assert!(shown.contains("Environment") && shown.contains("staging"));
        assert!(shown.contains("API token"));
        assert!(shown.contains("never saved or shown"));
        assert!(
            !shown.contains("hunter2"),
            "a secret is masked while typing: {shown}"
        );
    }

    #[test]
    fn escape_cancels_the_prompt_and_drops_what_was_typed() {
        let (mut app, ctx) = app_with_prompts();
        texts(&mut app, &ctx);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events: vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.task_inputs_dialog(ctx)),
        );
        assert!(app.view.task_inputs.is_none());
        assert!(app.view.task_runs.runs().is_empty());
    }
}
