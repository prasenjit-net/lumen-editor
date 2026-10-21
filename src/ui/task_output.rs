//! Task run output: a read-only, bounded log in the bottom panel slot beside the
//! Terminal and Output tabs. Hiding the panel or switching tabs never pauses or
//! stops a run; only Stop does.
use super::{
    Workbench,
    bottom_tabs::{ActiveBottom, as_panel_tabs},
    terminal::{TerminalStyle, cell_advance, terminal_input_bytes, terminal_line},
    theme,
    widgets::{HeaderAction, MenuItem, tabbed_panel_header_with_menu},
};
use crate::{
    app::{
        command::Command,
        task_runs::{Readiness, RunOutcome, RunState, TaskRun},
    },
    icon,
    model::task_output::{LogLine, Stream},
};
use eframe::egui::{self, FontId, RichText, ScrollArea, TopBottomPanel};
use std::time::{Duration, Instant};

const DEFAULT_HEIGHT: f32 = 220.0;
const MIN_HEIGHT: f32 = 120.0;
const MAX_HEIGHT: f32 = 520.0;
/// Below this header width the output commands move into an overflow menu.
const NARROW_HEADER_WIDTH: f32 = 520.0;

fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn status_text(run: &TaskRun, now: Instant) -> String {
    let state = match &run.state {
        RunState::Running => match &run.readiness {
            Readiness::Waiting { pattern, .. } => format!("Starting, waiting for \"{pattern}\""),
            Readiness::Ready => "Running · ready".to_string(),
            _ => "Running".to_string(),
        },
        RunState::Stopping => "Stopping…".to_string(),
        RunState::Finished(outcome) => outcome.summary(),
    };
    format!("{state} · {}", clock(run.elapsed(now)))
}

impl Workbench<'_> {
    pub(crate) fn task_output_panel(&mut self, ctx: &egui::Context) {
        if !self.view.task_runs.panel_open {
            return;
        }
        let Some(run_id) = self.view.task_runs.selected_run().map(|run| run.id) else {
            // The selected run is gone (closed or evicted): the slot has nothing to show.
            self.view.task_runs.panel_open = false;
            return;
        };
        let colors = theme::palette(&ctx.style().visuals);
        let font_size = self.view.settings.number("terminal.font_size", None);
        let font = FontId::monospace(font_size);
        let panel_divider = self.view.settings.boolean("terminal.panel_divider");
        let entries = self.bottom_tab_entries(ActiveBottom::Run);
        let (active, has_output) = self
            .view
            .task_runs
            .get(run_id)
            .map(|run| (run.is_active(), run.has_output()))
            .unwrap_or_default();
        let style = self.terminal_style(ctx);
        let focus_request = std::mem::take(&mut self.view.task_runs.focus_canvas);
        let now = Instant::now();
        let mut commands = Vec::new();
        TopBottomPanel::bottom("terminal")
            .default_height(DEFAULT_HEIGHT)
            .height_range(MIN_HEIGHT..=MAX_HEIGHT)
            .resizable(true)
            .show_separator_line(panel_divider)
            .frame(super::widgets::canvas(ctx))
            .show(ctx, |ui| {
                super::widgets::surface(colors.panel).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    // Claim the allocated height up-front so the log cannot resize the
                    // persisted panel.
                    ui.set_min_height(ui.available_height());
                    let tabs = as_panel_tabs(&entries);
                    let mut actions = vec![
                        HeaderAction {
                            glyph: icon::STOP,
                            tooltip: "Stop task",
                            command: Command::StopTaskRun(run_id),
                            enabled: active
                                && !matches!(
                                    self.view.task_runs.get(run_id).map(|run| &run.state),
                                    Some(RunState::Stopping)
                                ),
                        },
                        HeaderAction {
                            glyph: icon::REFRESH,
                            tooltip: "Restart task",
                            command: Command::RestartTaskRun(run_id),
                            enabled: true,
                        },
                        HeaderAction {
                            glyph: icon::CLEAR_ALL,
                            tooltip: "Clear this view (the task keeps running)",
                            command: Command::ClearTaskOutput(run_id),
                            enabled: has_output,
                        },
                        HeaderAction {
                            glyph: icon::COPY,
                            tooltip: "Copy output",
                            command: Command::CopyTaskOutput(run_id),
                            enabled: has_output,
                        },
                        HeaderAction {
                            glyph: icon::FLOPPY_DISK,
                            tooltip: "Save output to a file (what Lumen retained)",
                            command: Command::SaveTaskOutput(run_id),
                            enabled: has_output,
                        },
                        HeaderAction {
                            glyph: icon::X,
                            tooltip: "Close this task tab",
                            command: Command::CloseTaskRun(run_id),
                            enabled: true,
                        },
                        HeaderAction {
                            glyph: icon::CARET_DOWN,
                            tooltip: "Hide panel (tasks keep running)",
                            command: Command::HideTaskPanel,
                            enabled: true,
                        },
                    ];
                    // A narrow header keeps Stop, Restart, Close and Hide at the edge and moves
                    // the output commands into one overflow menu in a stable order.
                    let mut overflow = Vec::new();
                    if ui.available_width() < NARROW_HEADER_WIDTH {
                        let moved: Vec<HeaderAction> = actions
                            .iter()
                            .filter(|action| {
                                matches!(
                                    action.command,
                                    Command::ClearTaskOutput(_)
                                        | Command::CopyTaskOutput(_)
                                        | Command::SaveTaskOutput(_)
                                )
                            })
                            .map(|action| HeaderAction {
                                glyph: action.glyph,
                                tooltip: action.tooltip,
                                command: action.command.clone(),
                                enabled: action.enabled,
                            })
                            .collect();
                        actions.retain(|action| {
                            !matches!(
                                action.command,
                                Command::ClearTaskOutput(_)
                                    | Command::CopyTaskOutput(_)
                                    | Command::SaveTaskOutput(_)
                            )
                        });
                        for action in moved {
                            let mut item = MenuItem::new(action.tooltip, action.command);
                            if !action.enabled {
                                item = item.unless(Some("No output yet"));
                            }
                            overflow.push(item.entry());
                        }
                    }
                    let menu = (!overflow.is_empty())
                        .then_some(("More task output actions", &overflow[..]));
                    if let Some(command) = tabbed_panel_header_with_menu(ui, &tabs, &actions, menu)
                    {
                        commands.push(command);
                    }
                    let Some(run) = self.view.task_runs.get(run_id) else {
                        return;
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        let color = match run.outcome() {
                            Some(outcome) if outcome.is_success() => colors.success,
                            Some(RunOutcome::Cancelled) => colors.muted,
                            Some(_) => colors.error,
                            None => colors.text,
                        };
                        ui.label(RichText::new(status_text(run, now)).size(11.0).color(color));
                        ui.label(
                            RichText::new(run.command_line.clone())
                                .size(11.0)
                                .color(colors.muted),
                        );
                    });
                    egui::Frame::default()
                        .fill(colors.bg)
                        .corner_radius(egui::CornerRadius::same(3))
                        .inner_margin(egui::Margin::symmetric(7, 5))
                        .show(ui, |ui| {
                            if run.is_pty() {
                                pty_view(ui, ctx, run, &style, &mut commands, focus_request);
                            } else {
                                log_view(ui, run, &font, &colors, run_id);
                            }
                        });
                });
            });
        for command in commands {
            self.command(command);
        }
    }
}

/// A pseudo-terminal run: the emulator's screen, with keyboard input going to the
/// process while the canvas has focus. It shares the integrated terminal's canvas
/// id, so global shortcuts stay out of the way exactly as they do there.
fn pty_view(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    run: &TaskRun,
    style: &TerminalStyle,
    commands: &mut Vec<Command>,
    focus_request: bool,
) {
    let colors = style.colors;
    let canvas_id = egui::Id::new("terminal_canvas");
    let cell_width = cell_advance(ui, &style.font);
    let rows = ((ui.available_height() - 10.0) / style.line_height)
        .floor()
        .max(1.0) as u16;
    let columns = ((ui.available_width() - 14.0) / cell_width)
        .floor()
        .max(1.0) as u16;
    if run.is_active() && run.pty_size != (rows, columns) {
        commands.push(Command::ResizeTaskTerminal(run.id, rows, columns));
    }
    let focused = ctx.memory(|memory| memory.has_focus(canvas_id));
    let cursor_visible = run.accepts_input() && focused;
    // The canvas takes focus and input even before the first output arrives: a
    // quiet program waiting for a line must still be typeable.
    let canvas_rect = ui.available_rect_before_wrap();
    match run.screen.as_ref() {
        None => {
            ui.label(
                RichText::new(if run.is_active() {
                    "Waiting for output…"
                } else {
                    "The task produced no output."
                })
                .size(11.0)
                .color(colors.muted),
            );
        }
        Some(screen) => {
            ScrollArea::vertical()
                .id_salt(("task_pty", run.id))
                .auto_shrink(false)
                .stick_to_bottom(true)
                .show_rows(ui, style.line_height, screen.rows, |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for row in range {
                        if let Some(cells) = screen.row(row) {
                            ui.add(
                                egui::Label::new(terminal_line(
                                    cells,
                                    &style.font,
                                    style.line_height,
                                    &style.cursor_style,
                                    cursor_visible,
                                    style.color_scheme,
                                    colors,
                                ))
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    }
                });
        }
    }
    let canvas = ui.interact(canvas_rect, canvas_id, egui::Sense::click());
    if canvas.clicked() || focus_request {
        canvas.request_focus();
    }
    if canvas.has_focus() && run.accepts_input() {
        for event in ctx.input(|input| input.events.clone()) {
            if let Some(bytes) = terminal_input_bytes(event) {
                commands.push(Command::SendTaskInput(run.id, bytes));
            }
        }
    }
}

fn log_view(
    ui: &mut egui::Ui,
    run: &TaskRun,
    font: &FontId,
    colors: &theme::Palette,
    run_id: crate::app::task_runs::RunId,
) {
    let row_height = ui.fonts_mut(|fonts| fonts.row_height(font)) + 1.0;
    let notice = run.log.truncation_notice();
    let partial: Vec<LogLine> = run.log.partial();
    let lines = run.log.lines();
    let total = usize::from(notice.is_some()) + lines.len() + partial.len();
    if total == 0 {
        ui.label(
            RichText::new(if run.is_active() {
                "Waiting for output…"
            } else {
                "The task produced no output."
            })
            .size(11.0)
            .color(colors.muted),
        );
        return;
    }
    ScrollArea::both()
        .id_salt(("task_log", run_id))
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show_rows(ui, row_height, total, |ui, range| {
            let offset = usize::from(notice.is_some());
            for row in range {
                if row < offset {
                    ui.add(
                        egui::Label::new(
                            RichText::new(notice.clone().unwrap_or_default())
                                .font(font.clone())
                                .color(colors.warning),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                    continue;
                }
                let index = row - offset;
                let line = if index < lines.len() {
                    &lines[index]
                } else {
                    &partial[index - lines.len()]
                };
                let color = match line.stream {
                    Stream::Stdout => colors.text,
                    Stream::Stderr => colors.warning,
                };
                ui.add(
                    egui::Label::new(RichText::new(&line.text).font(font.clone()).color(color))
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{LumenApp, command::Command},
        model::task_output::Stream,
        services::task_process::{ExitReport, TaskEvent, fake::FakeTaskLauncher},
    };
    use eframe::egui;
    use lumen_task_core::{Scope, TaskKey};
    use std::sync::Arc;

    struct Fixture {
        app: LumenApp,
        fake: FakeTaskLauncher,
        ctx: egui::Context,
        _root: tempfile::TempDir,
    }

    fn fixture(tasks: usize) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let path = crate::services::tasks::workspace_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = String::from("schema_version = 1\n");
        for n in 0..tasks {
            text.push_str(&format!(
                "\n[[tasks]]\nid = \"t{n}\"\nlabel = \"Task number {n}\"\ntype = \"process\"\ncommand = \"echo\"\nargs = [\"hi\"]\nsave = \"none\"\ninstance_policy = \"parallel\"\n"
            ));
        }
        text.push_str(
            "\n[[tasks]]\nid = \"term\"\nlabel = \"Shell task\"\ntype = \"shell\"\ncommand = \"echo hi\"\nsave = \"none\"\n",
        );
        std::fs::write(&path, text).unwrap();
        let fake = FakeTaskLauncher::default();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.path().to_path_buf());
        app.task_launcher = Arc::new(fake.clone());
        let ids: Vec<String> = (0..tasks)
            .map(|n| format!("t{n}"))
            .chain(["term".to_string()])
            .collect();
        for id in ids {
            let (def, _) = app.read_task(&TaskKey::new(Scope::Workspace, id)).unwrap();
            app.task_trust
                .trust(root.path(), &lumen_task_core::execution_digest(&def))
                .unwrap();
        }
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        // Headless frames do not advance time, so a smooth scroll would not finish.
        ctx.style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        Fixture {
            app,
            fake,
            ctx,
            _root: root,
        }
    }

    impl Fixture {
        fn run(&mut self, n: usize) {
            self.app.execute(
                Command::RunTask(TaskKey::new(Scope::Workspace, format!("t{n}"))),
                &self.ctx,
            );
        }

        fn frame(&mut self, width: f32) -> egui::FullOutput {
            self.frame_with(width, Vec::new())
        }

        fn frame_with(&mut self, width: f32, events: Vec<egui::Event>) -> egui::FullOutput {
            let mut output = None;
            for round in 0..3 {
                output = Some(self.ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 600.0),
                        )),
                        // Input events belong to the first pass only.
                        events: if round == 0 {
                            events.clone()
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    },
                    |ctx| {
                        self.app
                            .render_component(ctx, |workbench| workbench.terminal_panel(ctx));
                        self.app
                            .render_component(ctx, |workbench| workbench.output_panel(ctx));
                        self.app
                            .render_component(ctx, |workbench| workbench.task_output_panel(ctx));
                    },
                ));
            }
            output.unwrap()
        }
    }

    fn rect_of(output: &egui::FullOutput, needle: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
    }

    #[test]
    fn the_panel_shows_status_command_output_and_stderr_separately() {
        let mut f = fixture(1);
        f.run(0);
        {
            let state = f.fake.process(0);
            let mut state = state.lock().unwrap();
            state
                .queued
                .push(TaskEvent::Output(Stream::Stdout, b"hello out\n".to_vec()));
            state
                .queued
                .push(TaskEvent::Output(Stream::Stderr, b"careful err\n".to_vec()));
        }
        f.app.poll_task_runs(&f.ctx);
        let output = f.frame(900.0);
        assert!(rect_of(&output, "Running").is_some());
        assert!(rect_of(&output, "echo hi").is_some(), "the command line");
        assert!(rect_of(&output, "hello out").is_some());
        assert!(rect_of(&output, "careful err").is_some());
        assert!(rect_of(&output, "Task number 0").is_some(), "its tab");
        assert!(rect_of(&output, "TERMINAL").is_some() && rect_of(&output, "OUTPUT").is_some());
        // Finishing updates the status; hiding the panel never stops the run.
        {
            let state = f.fake.process(0);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Code(0));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }
        f.app.poll_task_runs(&f.ctx);
        assert!(rect_of(&f.frame(900.0), "Succeeded").is_some());
        f.app.execute(Command::HideTaskPanel, &f.ctx);
        assert!(rect_of(&f.frame(900.0), "hello out").is_none());
        assert!(
            !f.app.view.task_runs.runs().is_empty(),
            "hiding keeps the run"
        );
    }

    #[test]
    fn terminal_and_output_headers_list_the_runs_and_switching_works() {
        let mut f = fixture(2);
        f.run(0);
        f.run(1);
        f.app.execute(Command::ShowOutput, &f.ctx);
        assert!(!f.app.view.task_runs.panel_open);
        let output = f.frame(900.0);
        assert!(
            rect_of(&output, "Task number 0").is_some()
                && rect_of(&output, "Task number 1").is_some()
        );
        let first = f.app.view.task_runs.runs()[0].id;
        f.app.execute(Command::ShowTaskRun(first), &f.ctx);
        assert!(
            f.app.view.task_runs.panel_open && !f.app.view.output.open && !f.app.view.terminal.open
        );
        f.app.execute(Command::ShowOutput, &f.ctx);
        assert!(!f.app.view.task_runs.panel_open && f.app.view.output.open);
    }

    #[test]
    fn many_tabs_stay_inside_the_header_and_the_selected_one_is_reachable() {
        let mut f = fixture(12);
        for n in 0..12 {
            f.run(n);
            // Finish each so the concurrency limit never blocks the next one.
            let state = f.fake.process(n);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Code(0));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
            drop(state);
            f.app.poll_task_runs(&f.ctx);
        }
        let last = f.app.view.task_runs.runs()[11].id;
        f.app.execute(Command::ShowTaskRun(last), &f.ctx);
        let width = 600.0;
        let output = f.frame(width);
        let hide = rect_of(&output, crate::icon::CARET_DOWN).expect("the hide action");
        assert!(
            hide.right() <= width && hide.left() > width - 40.0,
            "actions stay at the right edge: {hide:?}"
        );
        let selected = rect_of(&output, "Task number 11").expect("the selected tab is drawn");
        assert!(
            selected.left() >= 0.0 && selected.right() <= hide.left(),
            "the selected tab was scrolled into view, clear of the actions: {selected:?}"
        );
    }

    #[test]
    fn a_narrow_header_moves_output_commands_into_one_overflow_menu() {
        let mut f = fixture(1);
        f.run(0);
        let wide = f.frame(900.0);
        for glyph in [
            crate::icon::CLEAR_ALL,
            crate::icon::COPY,
            crate::icon::FLOPPY_DISK,
        ] {
            assert!(
                rect_of(&wide, glyph).is_some(),
                "{glyph:?} is a button at 900px"
            );
        }
        let narrow = f.frame(420.0);
        for glyph in [
            crate::icon::CLEAR_ALL,
            crate::icon::COPY,
            crate::icon::FLOPPY_DISK,
        ] {
            assert!(
                rect_of(&narrow, glyph).is_none(),
                "{glyph:?} moved into the menu at 420px"
            );
        }
        // Stop, Restart, Close and Hide stay at the edge.
        for glyph in [
            crate::icon::STOP,
            crate::icon::REFRESH,
            crate::icon::X,
            crate::icon::CARET_DOWN,
        ] {
            let rect = rect_of(&narrow, glyph).unwrap_or_else(|| panic!("{glyph:?} stays"));
            assert!(rect.right() <= 420.0, "{glyph:?} inside the header");
        }
    }

    #[test]
    fn closing_the_selected_run_hides_an_empty_panel() {
        let mut f = fixture(1);
        f.run(0);
        let id = f.app.view.task_runs.runs()[0].id;
        {
            let state = f.fake.process(0);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Code(0));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }
        f.app.poll_task_runs(&f.ctx);
        f.app.execute(Command::CloseTaskRun(id), &f.ctx);
        let _ = f.frame(900.0);
        assert!(!f.app.view.task_runs.panel_open, "no run left to show");
    }

    fn run_terminal_task(f: &mut Fixture) {
        f.app.execute(
            Command::RunTask(TaskKey::new(Scope::Workspace, "term")),
            &f.ctx,
        );
    }

    #[test]
    fn a_terminal_run_draws_the_emulator_screen_and_sizes_the_pty_to_the_panel() {
        let mut f = fixture(0);
        run_terminal_task(&mut f);
        f.fake
            .process(0)
            .lock()
            .unwrap()
            .queued
            .push(TaskEvent::Output(
                Stream::Stdout,
                b"\x1b[32mgreen\x1b[0m line\r\nprompt$ ".to_vec(),
            ));
        f.app.poll_task_runs(&f.ctx);
        let output = f.frame(900.0);
        assert!(rect_of(&output, "green line").is_some() || rect_of(&output, "line").is_some());
        assert!(rect_of(&output, "prompt$").is_some());
        assert!(
            rect_of(&output, "\u{1b}").is_none(),
            "no raw escape bytes are shown"
        );
        let size = f
            .fake
            .process(0)
            .lock()
            .unwrap()
            .size
            .expect("the PTY was sized");
        assert!(
            size.0 >= 1 && size.1 > 40,
            "columns follow the panel width: {size:?}"
        );
        let wide = size.1;
        f.frame(500.0);
        let narrow = f.fake.process(0).lock().unwrap().size.unwrap().1;
        assert!(
            narrow < wide,
            "a narrower panel gives fewer columns ({narrow} < {wide})"
        );
    }

    #[test]
    fn typing_in_a_focused_terminal_run_is_sent_to_the_process() {
        let mut f = fixture(0);
        run_terminal_task(&mut f);
        // Showing an interactive run gives its canvas the keyboard.
        f.frame(900.0);
        f.frame_with(900.0, vec![egui::Event::Text("ls".into())]);
        f.frame_with(
            900.0,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(f.fake.process(0).lock().unwrap().written, b"ls\r");
        // A finished run takes no more input.
        {
            let state = f.fake.process(0);
            let mut state = state.lock().unwrap();
            state.exit = Some(ExitReport::Code(0));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }
        f.app.poll_task_runs(&f.ctx);
        f.frame_with(900.0, vec![egui::Event::Text("late".into())]);
        assert_eq!(f.fake.process(0).lock().unwrap().written, b"ls\r");
    }

    #[test]
    fn a_pipe_run_never_takes_the_keyboard() {
        let mut f = fixture(1);
        f.run(0);
        f.frame(900.0);
        f.frame_with(900.0, vec![egui::Event::Text("zzz".into())]);
        assert!(f.fake.process(0).lock().unwrap().written.is_empty());
        assert!(
            !f.ctx
                .memory(|m| m.has_focus(egui::Id::new("terminal_canvas"))),
            "a log view does not claim the terminal canvas"
        );
    }
}
