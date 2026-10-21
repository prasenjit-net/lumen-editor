//! Task editor page, shown in the editor area for a task editor tab: a compact
//! form over a draft. Edits only change the draft; Save, Revert and Open in TOML
//! are typed commands handled by the controller, and nothing here touches disk.
use super::{
    Workbench,
    widgets::{HeaderAction, panel_header},
};
use crate::{
    app::{command::Command, task_editor::TaskEditorTab},
    icon,
};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use lumen_task_core::{Issue, SavePolicy, TaskKind, Transport};

const LABEL_WIDTH: f32 = 130.0;
/// Narrowest control column worth keeping beside the label; below this the
/// label stacks above its control.
const MIN_CONTROL_WIDTH: f32 = 220.0;

impl Workbench<'_> {
    pub(super) fn task_editor_page(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.view.task_editors.active_tab_mut() else {
            return;
        };
        let colors = super::theme::palette(ui.visuals());
        let (id, scope, dirty) = (tab.id, tab.scope(), tab.dirty());
        let actions = [
            HeaderAction {
                glyph: icon::FLOPPY_DISK,
                tooltip: "Save task",
                command: Command::SaveTaskEditor(id),
                enabled: dirty,
            },
            HeaderAction {
                glyph: icon::DISCARD,
                tooltip: "Revert unsaved changes",
                command: Command::RevertTaskEditor(id),
                enabled: dirty,
            },
            HeaderAction {
                glyph: icon::FILE_CODE,
                tooltip: "Open the tasks file as TOML",
                command: Command::OpenTaskToml(scope),
                enabled: true,
            },
            HeaderAction {
                glyph: icon::X,
                tooltip: "Close task editor",
                command: Command::CloseTaskEditor(id),
                enabled: true,
            },
        ];
        let title = format!("TASK · {}{}", tab.title(), if dirty { " ●" } else { "" });
        let mut commands = Vec::new();
        if let Some(command) = panel_header(ui, &title, &actions) {
            commands.push(command);
        }
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ScrollArea::vertical()
                    .id_salt(("task_editor_scroll", id))
                    .auto_shrink(false)
                    .show(ui, |ui| form(ui, tab, &colors, &mut commands));
            });
        // The form changed the draft: fold list buffers in and follow the label.
        if let Some(tab) = self.view.task_editors.active_tab_mut() {
            tab.sync_texts();
        }
        for command in commands {
            self.command(command);
        }
    }
}

fn form(
    ui: &mut egui::Ui,
    tab: &mut TaskEditorTab,
    colors: &super::theme::Palette,
    commands: &mut Vec<Command>,
) {
    let id = tab.id;
    // Give fields a visible well and border like other input surfaces.
    {
        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = colors.input;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, colors.border);
    }
    ui.label(
        RichText::new(format!(
            "{} scope · {}",
            tab.scope().label(),
            tab.file
                .as_ref()
                .map_or_else(|| "no file".to_string(), |path| path.display().to_string())
        ))
        .size(crate::ui::theme::META_SIZE)
        .color(colors.muted),
    );
    if let Some(error) = tab.error.clone() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&error).color(colors.error));
            if error.contains("changed on disk")
                && matches!(
                    tab.target,
                    crate::app::task_editor::TaskEditorTarget::Existing(_)
                )
                && ui.button("Reload from disk").clicked()
            {
                commands.push(Command::ReloadTaskEditor(id));
            }
        });
    }
    ui.add_space(6.0);
    // Core validation, shown beside the field it belongs to.
    tab.sync_texts();
    let issues = tab.issues();
    let is_new = matches!(
        tab.target,
        crate::app::task_editor::TaskEditorTarget::New(_)
    );

    if is_new {
        row(ui, "Template", &issues, &["template"], colors, |ui| {
            egui::ComboBox::from_id_salt(("task_template", id))
                .selected_text("Choose a starting point…")
                .show_ui(ui, |ui| {
                    for template in crate::app::task_editor::TaskTemplate::ALL {
                        if ui.selectable_label(false, template.label()).clicked() {
                            template.apply(&mut tab.draft);
                            tab.follow_label();
                            tab.rebuild_texts();
                        }
                    }
                });
        });
    }
    row(ui, "Label", &issues, &["label"], colors, |ui| {
        let response = ui.add(
            TextEdit::singleline(&mut tab.draft.label)
                .hint_text("Build")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            tab.follow_label();
        }
    });
    row(ui, "ID", &issues, &["id"], colors, |ui| {
        let response = ui.add_enabled(
            is_new,
            TextEdit::singleline(&mut tab.draft.id)
                .hint_text("build")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            tab.id_auto = false;
        }
    });
    if tab.draft.kind == TaskKind::Compound {
        ui.label(
            RichText::new("Compound tasks are edited in the TOML file for now.")
                .color(colors.muted),
        );
        return;
    }
    row(ui, "Type", &issues, &["type"], colors, |ui| {
        egui::ComboBox::from_id_salt(("task_kind", id))
            .selected_text(match tab.draft.kind {
                TaskKind::Shell => "Shell command",
                _ => "Process",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut tab.draft.kind, TaskKind::Process, "Process")
                    .on_hover_text("Run an executable directly; arguments are literal");
                ui.selectable_value(&mut tab.draft.kind, TaskKind::Shell, "Shell command")
                    .on_hover_text("Run a command string through the shell");
            });
    });
    let shell = tab.draft.kind == TaskKind::Shell;
    row(
        ui,
        if shell { "Command" } else { "Executable" },
        &issues,
        &["command"],
        colors,
        |ui| {
            let mut command = tab.draft.command.clone().unwrap_or_default();
            let hint = if shell { "npm run dev" } else { "cargo" };
            let response = ui.add(
                TextEdit::singleline(&mut command)
                    .hint_text(hint)
                    .desired_width(f32::INFINITY),
            );
            if response.changed() {
                tab.draft.command = Some(command).filter(|text| !text.is_empty());
            }
        },
    );
    if !shell {
        row(ui, "Arguments", &issues, &["args"], colors, |ui| {
            ui.add(
                TextEdit::multiline(&mut tab.args_text)
                    .hint_text("One argument per line")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if crate::app::task_editor::has_joined_args(&tab.args_text) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(
                            "Each line is one argument, so a line like \"build --release\" is \
                             passed as a single word.",
                        )
                        .size(crate::ui::theme::META_SIZE)
                        .color(colors.warning),
                    );
                    if ui.small_button("Split on spaces").clicked() {
                        tab.args_text = crate::app::task_editor::split_joined_args(&tab.args_text);
                    }
                });
            }
        });
    }
    row(ui, "Working directory", &issues, &["cwd"], colors, |ui| {
        let mut cwd = tab.draft.cwd.clone().unwrap_or_default();
        let response = ui.add(
            TextEdit::singleline(&mut cwd)
                .hint_text("${workspaceFolder}")
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            tab.draft.cwd = Some(cwd).filter(|text| !text.is_empty());
        }
    });
    row(ui, "Environment", &issues, &["env"], colors, |ui| {
        ui.add(
            TextEdit::multiline(&mut tab.env_text)
                .hint_text("NAME=VALUE, one per line")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
    });
    row(
        ui,
        "Group",
        &issues,
        &["group", "is_default"],
        colors,
        |ui| {
            let mut group = tab.draft.group.clone().unwrap_or_default();
            let response = ui.add(
                TextEdit::singleline(&mut group)
                    .hint_text("build, test…")
                    .desired_width(120.0),
            );
            if response.changed() {
                tab.draft.group = Some(group).filter(|text| !text.is_empty());
            }
            ui.checkbox(&mut tab.draft.is_default, "Default for group");
        },
    );
    row(
        ui,
        "Problem matcher",
        &issues,
        &["problem_matcher"],
        colors,
        |ui| {
            use lumen_task_core::{BuiltinMatcher, ProblemMatcherSpec};
            let current = match &tab.draft.problem_matcher {
                None => "None".to_string(),
                Some(ProblemMatcherSpec::Builtin(builtin)) => builtin.as_str().to_string(),
                Some(ProblemMatcherSpec::Pattern { .. }) => "Custom pattern (edit in TOML)".into(),
            };
            egui::ComboBox::from_id_salt(("task_matcher", id))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(tab.draft.problem_matcher.is_none(), "None")
                        .clicked()
                    {
                        tab.draft.problem_matcher = None;
                    }
                    for builtin in BuiltinMatcher::ALL {
                        let selected =
                            tab.draft.problem_matcher == Some(ProblemMatcherSpec::Builtin(builtin));
                        if ui.selectable_label(selected, builtin.as_str()).clicked() {
                            tab.draft.problem_matcher = Some(ProblemMatcherSpec::Builtin(builtin));
                        }
                    }
                })
                .response
                .on_hover_text("Finds file:line problems in the output and lists them in Problems");
        },
    );
    row(ui, "Save before run", &issues, &["save"], colors, |ui| {
        egui::ComboBox::from_id_salt(("task_save", id))
            .selected_text(match tab.draft.save {
                None => "Default (prompt)",
                Some(SavePolicy::Prompt) => "Prompt",
                Some(SavePolicy::Save) => "Save all",
                Some(SavePolicy::None) => "Do not save",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut tab.draft.save, None, "Default (prompt)");
                ui.selectable_value(&mut tab.draft.save, Some(SavePolicy::Prompt), "Prompt");
                ui.selectable_value(&mut tab.draft.save, Some(SavePolicy::Save), "Save all");
                ui.selectable_value(&mut tab.draft.save, Some(SavePolicy::None), "Do not save")
                    .on_hover_text("The task sees the contents on disk");
            });
    });
    row(ui, "Output", &issues, &["transport"], colors, |ui| {
        egui::ComboBox::from_id_salt(("task_transport", id))
            .selected_text(match tab.draft.transport {
                None => "Default",
                Some(Transport::Pipe) => "Captured output",
                Some(Transport::Pty) => "Terminal (PTY)",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut tab.draft.transport, None, "Default");
                ui.selectable_value(
                    &mut tab.draft.transport,
                    Some(Transport::Pipe),
                    "Captured output",
                );
                ui.selectable_value(
                    &mut tab.draft.transport,
                    Some(Transport::Pty),
                    "Terminal (PTY)",
                );
            });
    });
    // Problems that belong to no visible field (for example dependencies).
    let shown = [
        "label",
        "id",
        "type",
        "command",
        "args",
        "cwd",
        "env",
        "group",
        "is_default",
        "save",
        "transport",
    ];
    for issue in issues
        .iter()
        .filter(|issue| !shown.iter().any(|name| issue.field.starts_with(name)))
    {
        ui.label(RichText::new(format!("{}: {}", issue.field, issue.message)).color(colors.error));
    }
    ui.add_space(8.0);
    let summary = if issues.is_empty() {
        "No problems.".to_string()
    } else {
        format!("{} problem(s); fix them to save.", issues.len())
    };
    ui.label(
        RichText::new(summary)
            .size(crate::ui::theme::META_SIZE)
            .color(colors.muted),
    );
    if issues.is_empty()
        && let Ok(text) = lumen_task_core::new_file(&tab.draft)
    {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Will be written as")
                .size(crate::ui::theme::META_SIZE)
                .color(colors.muted),
        );
        ui.label(
            RichText::new(text)
                .monospace()
                .size(crate::ui::theme::META_SIZE),
        );
    }
}

/// One labelled form row with its problems underneath.
/// Whether a form row is too narrow for a label column beside its control.
fn stacked_labels(available_width: f32) -> bool {
    available_width < LABEL_WIDTH + MIN_CONTROL_WIDTH
}

fn row(
    ui: &mut egui::Ui,
    label: &str,
    issues: &[Issue],
    fields: &[&str],
    colors: &super::theme::Palette,
    add: impl FnOnce(&mut egui::Ui),
) {
    let control = |ui: &mut egui::Ui| {
        add(ui);
        for issue in issues
            .iter()
            .filter(|issue| fields.iter().any(|field| issue.field.starts_with(field)))
        {
            ui.add(
                egui::Label::new(
                    RichText::new(&issue.message)
                        .size(crate::ui::theme::META_SIZE)
                        .color(colors.error),
                )
                .wrap(),
            );
        }
    };
    if stacked_labels(ui.available_width()) {
        ui.add(egui::Label::new(RichText::new(label).color(colors.muted)).truncate());
        ui.vertical(control);
    } else {
        ui.horizontal_top(|ui| {
            ui.add_sized(
                [LABEL_WIDTH, 20.0],
                egui::Label::new(RichText::new(label).color(colors.muted)).truncate(),
            );
            ui.vertical(control);
        });
    }
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_labels_stack_when_the_control_column_would_be_too_narrow() {
        assert!(stacked_labels(240.0));
        assert!(stacked_labels(LABEL_WIDTH + MIN_CONTROL_WIDTH - 1.0));
        assert!(!stacked_labels(LABEL_WIDTH + MIN_CONTROL_WIDTH));
        assert!(!stacked_labels(900.0));
    }
}
