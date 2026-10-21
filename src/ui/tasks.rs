//! Run and Debug sidebar: the task list for both scopes. It only reads the catalog
//! the controller filled and issues typed commands; nothing here touches disk.
use super::Workbench;
use crate::{
    app::{
        command::Command,
        task_catalog::{ScopeState, TaskCatalog},
    },
    icon,
};
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use lumen_task_core::{Scope, TaskDef, TaskKey};

impl Workbench<'_> {
    /// Where New Task and Open Tasks File go by default: the workspace when a
    /// folder is open, otherwise the user's file.
    pub(super) fn default_task_scope(&self) -> Scope {
        if self.editor.workspace.is_some() {
            Scope::Workspace
        } else {
            Scope::User
        }
    }

    pub(super) fn tasks_panel(&mut self, ui: &mut egui::Ui) {
        let colors = super::theme::palette(ui.visuals());
        ui.add_space(4.0);
        ui.add(
            TextEdit::singleline(&mut self.view.tasks.filter)
                .id_salt("task_filter")
                .hint_text("Filter tasks")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let mut commands = Vec::new();
        let running: Vec<_> = self
            .view
            .task_runs
            .runs()
            .iter()
            .filter(|run| run.is_active())
            .map(|run| {
                (
                    run.id,
                    self.view.task_runs.title(run.id),
                    run.elapsed(std::time::Instant::now()),
                )
            })
            .collect();
        ScrollArea::vertical()
            .id_salt("tasks_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                running_section(ui, &running, &colors, &mut commands);
                for scope in [Scope::Workspace, Scope::User] {
                    scope_section(ui, &self.view.tasks, scope, &colors, &mut commands);
                }
            });
        for command in commands {
            self.command(command);
        }
    }
}

fn running_section(
    ui: &mut egui::Ui,
    running: &[(crate::app::task_runs::RunId, String, std::time::Duration)],
    colors: &super::theme::Palette,
    commands: &mut Vec<Command>,
) {
    if running.is_empty() {
        return;
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("RUNNING ({})", running.len()))
            .size(crate::ui::theme::META_SIZE)
            .color(colors.muted),
    );
    for (id, title, elapsed) in running {
        ui.horizontal(|ui| {
            let seconds = elapsed.as_secs();
            let stop = ui
                .add(egui::Button::new(icon::STOP).frame(false))
                .on_hover_text("Stop task");
            if stop.clicked() {
                commands.push(Command::StopTaskRun(*id));
            }
            let show = ui
                .add(
                    egui::Label::new(
                        RichText::new(format!("{title}   {:02}:{:02}", seconds / 60, seconds % 60))
                            .size(12.0),
                    )
                    .sense(egui::Sense::click())
                    .truncate(),
                )
                .on_hover_text("Show output");
            if show.clicked() {
                commands.push(Command::ShowTaskRun(*id));
            }
        });
    }
}

fn scope_section(
    ui: &mut egui::Ui,
    catalog: &TaskCatalog,
    scope: Scope,
    colors: &super::theme::Palette,
    commands: &mut Vec<Command>,
) {
    let heading = match scope {
        Scope::Workspace => "WORKSPACE TASKS",
        Scope::User => "USER TASKS",
    };
    let tasks = catalog.scope(scope);
    let count = catalog.tasks(scope).len();
    ui.add_space(6.0);
    let title = ui.label(
        RichText::new(if count > 0 {
            format!("{heading} ({count})")
        } else {
            heading.to_string()
        })
        .size(crate::ui::theme::META_SIZE)
        .color(colors.muted),
    );
    if let Some(path) = &tasks.path {
        title.on_hover_text(path.display().to_string());
    }
    let muted = |text: &str| {
        RichText::new(text.to_string())
            .size(crate::ui::theme::META_SIZE)
            .color(colors.muted)
    };
    match &tasks.state {
        ScopeState::Unread => {
            ui.label(muted("Loading…"));
        }
        ScopeState::Unavailable(message) => {
            ui.add(egui::Label::new(muted(message)).wrap());
        }
        ScopeState::Missing => {
            ui.add(egui::Label::new(muted("No tasks defined yet.")).wrap());
            if ui
                .small_button(format!("{}  Create Task", icon::PLUS))
                .clicked()
            {
                commands.push(Command::NewTaskEditor(scope));
            }
        }
        ScopeState::Unreadable(message) => {
            ui.add(egui::Label::new(RichText::new(message).color(colors.error)).wrap());
        }
        ScopeState::Invalid { issues } => {
            ui.add(
                egui::Label::new(RichText::new("The tasks file has problems:").color(colors.error))
                    .wrap(),
            );
            for issue in issues.iter().take(3) {
                ui.add(egui::Label::new(muted(&issue.to_string())).wrap());
            }
            if issues.len() > 3 {
                ui.label(muted(&format!("…and {} more", issues.len() - 3)));
            }
            if ui
                .small_button(format!("{}  Open as TOML", icon::FILE_CODE))
                .clicked()
            {
                commands.push(Command::OpenTaskToml(scope));
            }
        }
        ScopeState::Loaded { .. } => {
            let visible = catalog.visible(scope);
            if count == 0 {
                ui.label(muted("No tasks defined yet."));
                if ui
                    .small_button(format!("{}  Create Task", icon::PLUS))
                    .clicked()
                {
                    commands.push(Command::NewTaskEditor(scope));
                }
            } else if visible.is_empty() {
                ui.label(muted("No matching tasks."));
            }
            for task in visible {
                task_row(ui, catalog, scope, task, colors, commands);
            }
        }
    }
}

fn task_row(
    ui: &mut egui::Ui,
    catalog: &TaskCatalog,
    scope: Scope,
    task: &TaskDef,
    colors: &super::theme::Palette,
    commands: &mut Vec<Command>,
) {
    let key = TaskKey::new(scope, &task.id);
    let selected = catalog.selected.as_ref() == Some(&key);
    // Group (and a check for the group default) sit at the right edge.
    let tag = task.group.as_ref().map(|group| {
        if task.is_default {
            format!("{group} {}", icon::CHECK)
        } else {
            group.clone()
        }
    });
    let hover = format!(
        "{} · {}\nDouble-click to run",
        task.id,
        task.command.as_deref().unwrap_or("compound task")
    );
    let response = ui
        .with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
            let mut button =
                egui::Button::selectable(selected, RichText::new(&task.label).size(12.0))
                    .frame_when_inactive(selected)
                    .min_size(egui::vec2(ui.available_width(), 22.0));
            if let Some(tag) = tag {
                button = button.right_text(
                    RichText::new(tag)
                        .size(crate::ui::theme::META_SIZE)
                        .color(colors.muted),
                );
            }
            ui.add(button)
        })
        .inner
        .on_hover_text(hover);
    let enter = response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    if response.double_clicked() || enter {
        commands.push(Command::RunTask(key.clone()));
    } else if response.clicked() {
        commands.push(Command::SelectTask(key.clone()));
    }
    response.context_menu(|ui| {
        if ui.button(format!("{}  Run", icon::PLAY)).clicked() {
            commands.push(Command::RunTask(key.clone()));
            ui.close();
        }
        if ui.button(format!("{}  Edit", icon::FILE_CODE)).clicked() {
            commands.push(Command::OpenTaskEditor(key.clone()));
            ui.close();
        }
        if ui.button(format!("{}  Duplicate", icon::COPY)).clicked() {
            commands.push(Command::DuplicateTask(key.clone()));
            ui.close();
        }
        ui.separator();
        if ui
            .button(RichText::new(format!("{}  Delete Definition", icon::X)).color(colors.error))
            .clicked()
        {
            commands.push(Command::RequestDeleteTask(key.clone()));
            ui.close();
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::app::{
        LumenApp,
        task_catalog::{ScopeState, ScopeTasks},
        view_state::Activity,
    };
    use eframe::egui;
    use lumen_task_core::{Issue, Scope, TaskDef, TaskKey, TaskKind};

    fn task(id: &str, label: &str, group: Option<&str>) -> TaskDef {
        let mut task = TaskDef::new(id, label, TaskKind::Process);
        task.command = Some("cargo".into());
        task.group = group.map(str::to_string);
        task
    }

    fn app() -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        app.view.activity = Activity::RunDebug;
        app.view.sidebar_visible = true;
        app.editor.workspace = Some("/workspace".into());
        app.view.tasks.stale = false;
        app.view.tasks.loaded_root = app.editor.workspace.clone();
        app.view.tasks.workspace = ScopeTasks {
            path: Some("/workspace/.lumen/tasks.toml".into()),
            state: ScopeState::Loaded {
                tasks: vec![
                    task("build", "Build all", Some("build")),
                    task("test", "Run tests", Some("test")),
                ],
                text: String::new(),
            },
        };
        app.view.tasks.user = ScopeTasks {
            path: None,
            state: ScopeState::Missing,
        };
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        (app, ctx)
    }

    fn frame(
        app: &mut LumenApp,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.sidebar(ctx)),
        )
    }

    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn settle(app: &mut LumenApp, ctx: &egui::Context) -> egui::FullOutput {
        let mut output = frame(app, ctx, 0.0, vec![]);
        for step in 1..4 {
            output = frame(app, ctx, f64::from(step) * 0.05, vec![]);
        }
        output
    }

    fn find(output: &egui::FullOutput, needle: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {needle}"))
    }

    fn click(app: &mut LumenApp, ctx: &egui::Context, pos: egui::Pos2, time: &mut f64) {
        for pressed in [true, false] {
            *time += 0.02;
            frame(
                app,
                ctx,
                *time,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn both_scopes_list_with_counts_rows_and_their_empty_states() {
        let (mut app, ctx) = app();
        let output = settle(&mut app, &ctx);
        let all = texts(&output).join("\n");
        assert!(all.contains("WORKSPACE TASKS (2)"), "{all}");
        assert!(all.contains("Build all") && all.contains("Run tests"));
        assert!(all.contains("USER TASKS"));
        assert!(all.contains("No tasks defined yet."));
        assert!(all.contains("Create Task"));
        assert!(all.contains("Filter tasks"));
        assert!(
            !all.contains("not available yet"),
            "the placeholder is gone"
        );
    }

    #[test]
    fn unavailable_invalid_and_filtered_states_explain_themselves() {
        let (mut app, ctx) = app();
        app.view.tasks.workspace = ScopeTasks {
            path: None,
            state: ScopeState::Unavailable("Open a folder to use workspace tasks".into()),
        };
        app.view.tasks.user = ScopeTasks {
            path: None,
            state: ScopeState::Invalid {
                issues: vec![Issue {
                    task: Some("a".into()),
                    field: "command".into(),
                    message: "must not be empty".into(),
                }],
            },
        };
        let all = texts(&settle(&mut app, &ctx)).join("\n");
        assert!(all.contains("Open a folder to use workspace tasks"));
        assert!(all.contains("The tasks file has problems:"));
        assert!(all.contains("must not be empty") && all.contains("Open as TOML"));

        let (mut app, ctx) = self::app();
        app.view.tasks.filter = "zzz".into();
        assert!(
            texts(&settle(&mut app, &ctx))
                .join("\n")
                .contains("No matching tasks.")
        );
        app.view.tasks.filter = "tests".into();
        let all = texts(&settle(&mut app, &ctx)).join("\n");
        assert!(all.contains("Run tests") && !all.contains("Build all"));
    }

    #[test]
    fn clicking_selects_and_double_clicking_runs_the_task_after_review() {
        let (mut app, ctx) = app();
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join(".lumen")).unwrap();
        std::fs::write(
            root.path().join(".lumen/tasks.toml"),
            "schema_version = 1\n[[tasks]]\nid = \"build\"\nlabel = \"Build all\"\ntype = \"process\"\ncommand = \"cargo\"\n[[tasks]]\nid = \"test\"\nlabel = \"Run tests\"\ntype = \"process\"\ncommand = \"cargo\"\n",
        )
        .unwrap();
        app.editor.workspace = Some(root.path().to_path_buf());
        app.refresh_tasks();
        let output = settle(&mut app, &ctx);
        let row = find(&output, "Run tests").center();
        let mut time = 1.0;
        click(&mut app, &ctx, row, &mut time);
        assert_eq!(
            app.view.tasks.selected,
            Some(TaskKey::new(Scope::Workspace, "test"))
        );
        assert!(
            app.view.task_editors.tabs().is_empty(),
            "one click only selects"
        );
        time += 0.05;
        click(&mut app, &ctx, row, &mut time);
        // A workspace task is reviewed before its first run; nothing has started.
        let pending = app
            .view
            .task_pending
            .as_ref()
            .expect("double click asks to run it");
        assert_eq!(
            (pending.plan.root.id.as_str(), pending.plan.label.as_str()),
            ("test", "Run tests")
        );
        assert!(pending.needs_review());
        assert!(app.view.task_runs.runs().is_empty());
    }
}
