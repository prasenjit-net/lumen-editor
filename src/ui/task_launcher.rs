//! Title-bar task launcher: a dropdown that picks the task and a run button
//! that starts it. It shares the Run and Debug sidebar's selection, so a task
//! picked in either place is the one the other shows.
use super::Workbench;
use crate::{app::command::Command, icon};
use eframe::egui::{self, RichText};
use lumen_task_core::{Scope, TaskKey};

/// Space between the last toggle and the window edge.
const EDGE_MARGIN: f32 = 6.0;
/// Extra space between neighbouring toggles; their padded hit areas already
/// provide the visual gap, so adding more would double it.
const TOGGLE_SPACING: f32 = 0.0;
/// Width and height of a toggle's hit and hover area.
const TOGGLE_SIZE: (f32, f32) = (26.0, 22.0);

/// Gap between the launcher and the panel toggles.
const TOGGLE_GAP: f32 = 16.0;

/// Longest the selected task's name may be, in characters, before it is cut.
const MAX_LABEL_CHARS: usize = 24;

impl Workbench<'_> {
    /// Panel visibility toggles: left sidebar, right agent panel, bottom panel.
    /// Call inside a `right_to_left` layout; they sit at the edge in reverse
    /// order, so they read left, right, bottom with the launcher before them.
    pub(crate) fn panel_toggles(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let toggles = [
            (
                icon::PANEL_BOTTOM,
                self.visible_bottom_panel(),
                "Toggle Bottom Panel",
                Command::ToggleBottomPanel,
            ),
            (
                icon::SIDEBAR_RIGHT,
                self.view.agents.open,
                "Toggle Right Panel (Coding Agents)",
                Command::ToggleAgentPanel,
            ),
            (
                icon::SIDEBAR_LEFT,
                self.view.sidebar_visible,
                "Toggle Left Sidebar",
                Command::ToggleSidebar,
            ),
        ];
        // Breathing room at the outer edge, then evenly spaced buttons.
        ui.add_space(EDGE_MARGIN);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = TOGGLE_SPACING;
            for (glyph, shown, tooltip, command) in toggles {
                let response = ui
                    .add(
                        egui::Button::new(RichText::new(glyph).size(14.0).color(if shown {
                            colors.text
                        } else {
                            colors.muted
                        }))
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(3)
                        .min_size(egui::vec2(TOGGLE_SIZE.0, TOGGLE_SIZE.1)),
                    )
                    .on_hover_text(tooltip);
                if response.clicked() {
                    self.command(command);
                }
            }
        });
        ui.add_space(TOGGLE_GAP);
    }

    fn visible_bottom_panel(&self) -> bool {
        self.view.terminal.open
            || self.view.output.open
            || self.view.problems.open
            || (self.view.task_runs.panel_open && self.view.task_runs.selected_run().is_some())
    }

    /// Draw the launcher inside a `right_to_left` layout: the run button at the
    /// edge, with the dropdown to its left.
    pub(crate) fn task_launcher(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let selected = self.view.tasks.selected.clone();
        let selected_label = selected
            .as_ref()
            .and_then(|key| self.view.tasks.get(key))
            .map(|task| task.label.clone());
        let label = match &selected_label {
            Some(name) if name.chars().count() > MAX_LABEL_CHARS => {
                format!(
                    "{}…",
                    name.chars().take(MAX_LABEL_CHARS).collect::<String>()
                )
            }
            Some(name) => name.clone(),
            None => "Select task".into(),
        };
        let mut commands = Vec::new();
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let run = ui
                .add_enabled(
                    selected_label.is_some(),
                    egui::Button::new(RichText::new(icon::PLAY).size(12.0).color(colors.success))
                        .frame(false),
                )
                .on_hover_text(match &selected_label {
                    Some(name) => format!("Run task '{name}'"),
                    None => "Choose a task to run first".into(),
                })
                .on_disabled_hover_text("Choose a task to run first");
            if run.clicked() {
                commands.push(Command::RunSelectedTask);
            }
            // Stop the selected task's active run, else the run shown in the panel.
            let active = selected
                .as_ref()
                .and_then(|key| self.view.task_runs.active_for(key))
                .or_else(|| {
                    self.view
                        .task_runs
                        .selected_run()
                        .filter(|run| run.is_active())
                })
                .map(|run| run.id);
            if let Some(run_id) = active
                && ui
                    .add(
                        egui::Button::new(RichText::new(icon::STOP).size(12.0).color(colors.error))
                            .frame(false),
                    )
                    .on_hover_text("Stop the running task")
                    .clicked()
            {
                commands.push(Command::StopTaskRun(run_id));
            }
            let menu = ui.menu_button(
                RichText::new(format!("{label}  {}", icon::CARET_DOWN))
                    .size(11.0)
                    .color(if selected_label.is_some() {
                        colors.text
                    } else {
                        colors.muted
                    }),
                |ui| {
                    ui.set_max_width(260.0);
                    task_menu(ui, &self.view.tasks, selected.as_ref(), &mut commands);
                },
            );
            // The catalog is otherwise only read while the sidebar shows it.
            if menu.response.clicked() {
                commands.push(Command::RefreshTasks);
            }
            menu.response.on_hover_text("Choose the task to run");
        });
        for command in commands {
            self.command(command);
        }
    }
}

/// The dropdown body: tasks grouped by scope, the selected one marked.
fn task_menu(
    ui: &mut egui::Ui,
    catalog: &crate::app::task_catalog::TaskCatalog,
    selected: Option<&TaskKey>,
    commands: &mut Vec<Command>,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let mut any = false;
    for scope in [Scope::Workspace, Scope::User] {
        let tasks = catalog.tasks(scope);
        if tasks.is_empty() {
            continue;
        }
        any = true;
        ui.label(
            RichText::new(scope.label().to_uppercase())
                .size(10.0)
                .color(colors.muted),
        );
        for task in tasks {
            let key = TaskKey::new(scope, &task.id);
            let mut response = ui.selectable_label(selected == Some(&key), &task.label);
            if let Some(detail) = &task.detail {
                response = response.on_hover_text(detail);
            }
            if response.clicked() {
                commands.push(Command::SelectTask(key));
                ui.close();
            }
        }
    }
    if !any {
        ui.label(RichText::new("No tasks defined").color(colors.muted));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        LumenApp,
        task_catalog::{ScopeState, ScopeTasks},
    };
    use crate::ui::theme::configure_ui;

    fn render(app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 200.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |workbench| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            workbench.task_launcher(ui)
                        });
                    });
                })
            },
        );
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn toggles_read_left_right_bottom_after_a_gap_following_the_launcher() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app_with_task();
        app.view.tasks.selected = Some(TaskKey::new(Scope::Workspace, "build"));
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 200.0),
                )),
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |workbench| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            workbench.panel_toggles(ui);
                            workbench.task_launcher(ui);
                        });
                    });
                })
            },
        );
        let rect_of = |needle: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{needle} not drawn"))
        };
        let left = rect_of(icon::SIDEBAR_LEFT);
        let right = rect_of(icon::SIDEBAR_RIGHT);
        let bottom = rect_of(icon::PANEL_BOTTOM);
        let run = rect_of(icon::PLAY);
        assert!(left.right() < right.left() && right.right() < bottom.left());
        assert!(
            run.right() + TOGGLE_GAP - 1.0 <= left.left(),
            "gap before the toggles"
        );
    }

    fn app_with_task() -> LumenApp {
        let mut app = LumenApp::default();
        let file = lumen_task_core::parse(
            "schema_version = 1\n[[tasks]]\nid = 'build'\ntype = 'shell'\nlabel = 'Build app'\ncommand = 'true'\n",
        )
        .unwrap_or_else(|issues| panic!("{issues:?}"));
        app.view.tasks.workspace = ScopeTasks {
            path: None,
            state: ScopeState::Loaded {
                tasks: file.tasks,
                text: String::new(),
            },
        };
        app
    }

    #[test]
    fn without_a_selection_the_launcher_prompts_and_the_run_button_is_disabled() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app_with_task();
        let text = render(&mut app, &ctx, vec![]).join(" ");
        assert!(text.contains("Select task"), "{text}");
        assert!(!text.contains("Build app"));
    }

    #[test]
    fn the_selected_task_names_the_dropdown() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = app_with_task();
        app.view.tasks.selected = Some(TaskKey::new(Scope::Workspace, "build"));
        let text = render(&mut app, &ctx, vec![]).join(" ");
        assert!(text.contains("Build app"), "{text}");
    }
}
