use super::{
    Workbench,
    bottom_tabs::{ActiveBottom, as_panel_tabs},
    widgets::{HeaderAction, tabbed_panel_header},
};
use crate::{app::command::Command, icon};
use eframe::egui::{self, RichText, ScrollArea, TopBottomPanel};
use lumen_language_services::diagnostics::DiagnosticSeverity;

const DEFAULT_HEIGHT: f32 = 220.0;

/// Where a problem points: an open document's range (from a language server), or
/// a file location printed by a task, which may not be open.
#[derive(Clone)]
enum ProblemTarget {
    Document {
        id: crate::model::DocumentId,
        range: std::ops::Range<usize>,
    },
    Location(std::path::PathBuf),
}

#[derive(Clone)]
struct ProblemRow {
    target: ProblemTarget,
    file: String,
    line: usize,
    column: usize,
    severity: DiagnosticSeverity,
    message: String,
    /// Complete diagnostic text; `message` keeps only the first line for the row.
    full_message: String,
    detail: String,
}

impl Workbench<'_> {
    pub(crate) fn problems_panel(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.problems.open {
            return;
        }
        let rows = self.problem_rows();
        let error_count = rows
            .iter()
            .filter(|row| row.severity == DiagnosticSeverity::Error)
            .count();
        let warning_count = rows
            .iter()
            .filter(|row| row.severity == DiagnosticSeverity::Warning)
            .count();
        TopBottomPanel::bottom("terminal")
            .default_height(DEFAULT_HEIGHT)
            .height_range(120.0..=520.0)
            .resizable(true)
            .frame(super::widgets::canvas(ctx))
            .show(ctx, |ui| {
                super::widgets::surface(colors.panel).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.set_min_height(ui.available_height());
                    let has_task_problems = rows
                        .iter()
                        .any(|row| matches!(row.target, ProblemTarget::Location(_)));
                    let actions = [
                        HeaderAction {
                            glyph: icon::CLEAR_ALL,
                            tooltip: "Clear problems found in task output",
                            command: Command::ClearTaskProblems,
                            enabled: has_task_problems,
                        },
                        HeaderAction {
                            glyph: icon::X,
                            tooltip: "Close Problems",
                            command: Command::CloseProblems,
                            enabled: true,
                        },
                    ];
                    let entries = self.bottom_tab_entries(ActiveBottom::Problems);
                    let tabs = as_panel_tabs(&entries);
                    if let Some(command) = tabbed_panel_header(ui, &tabs, &actions) {
                        self.command(command);
                    }
                    // Same horizontal inset as the panel header's content.
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(10, 0))
                        .show(ui, |ui| {
                            // Wrap instead of overflowing, so every filter stays reachable at
                            // narrow widths.
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                ui.checkbox(
                                    &mut self.view.problems.show_errors,
                                    format!("{error_count} Errors"),
                                );
                                ui.checkbox(
                                    &mut self.view.problems.show_warnings,
                                    format!("{warning_count} Warnings"),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.view.problems.filter)
                                        .hint_text("Filter problems")
                                        .desired_width(
                                            190.0_f32.min(ui.available_width().max(90.0)),
                                        ),
                                );
                            });
                            ui.separator();
                            let query = self.view.problems.filter.trim().to_lowercase();
                            let show_errors = self.view.problems.show_errors;
                            let show_warnings = self.view.problems.show_warnings;
                            let mut visible_rows = 0;
                            ScrollArea::vertical()
                                .id_salt("problems_list")
                                .show(ui, |ui| {
                                    let mut previous_file = None;
                                    for row in rows.iter().filter(|row| {
                                        let severity_visible = match row.severity {
                                            DiagnosticSeverity::Error => show_errors,
                                            DiagnosticSeverity::Warning => show_warnings,
                                            DiagnosticSeverity::Information
                                            | DiagnosticSeverity::Hint => true,
                                        };
                                        severity_visible
                                            && (query.is_empty()
                                                || row.message.to_lowercase().contains(&query)
                                                || row.file.to_lowercase().contains(&query)
                                                || row.detail.to_lowercase().contains(&query))
                                    }) {
                                        visible_rows += 1;
                                        if previous_file.as_deref() != Some(row.file.as_str()) {
                                            ui.label(
                                                RichText::new(&row.file)
                                                    .strong()
                                                    .size(crate::ui::theme::META_SIZE)
                                                    .color(colors.text),
                                            );
                                            previous_file = Some(row.file.clone());
                                        }
                                        let (glyph, color) = match row.severity {
                                            DiagnosticSeverity::Error => {
                                                (icon::ERROR, colors.error)
                                            }
                                            DiagnosticSeverity::Warning => {
                                                (icon::WARNING, colors.warning)
                                            }
                                            DiagnosticSeverity::Information
                                            | DiagnosticSeverity::Hint => {
                                                (icon::DEBUG, colors.muted)
                                            }
                                        };
                                        let label = format!(
                                            "  {glyph}  {}  [{}:{}]{}",
                                            row.message, row.line, row.column, row.detail
                                        );
                                        ui.horizontal(|ui| {
                                            // Reserve the trailing action first; the message
                                            // truncates (full text on hover) instead of
                                            // pushing the action away or changing row height.
                                            let action_width = 22.0;
                                            let text_width = (ui.available_width()
                                                - action_width
                                                - ui.spacing().item_spacing.x)
                                                .max(48.0);
                                            let response = ui
                                                .allocate_ui_with_layout(
                                                    egui::vec2(text_width, 18.0),
                                                    egui::Layout::left_to_right(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        ui.add(
                                                            egui::Button::new(
                                                                RichText::new(&label)
                                                                    .size(crate::ui::theme::META_SIZE)
                                                                    .color(color),
                                                            )
                                                            .frame(false)
                                                            .truncate(),
                                                        )
                                                    },
                                                )
                                                .inner
                                                .on_hover_text(format!(
                                                    "{}\n{}:{}:{}{}",
                                                    row.full_message,
                                                    row.file,
                                                    row.line,
                                                    row.column,
                                                    row.detail
                                                ));
                                            response.context_menu(|ui| {
                                                if ui.button("Copy Message").clicked() {
                                                    ui.ctx().copy_text(row.full_message.clone());
                                                    self.view.status =
                                                        "Problem message copied".into();
                                                    ui.close();
                                                }
                                                if ui.button("Copy Location").clicked() {
                                                    ui.ctx().copy_text(format!(
                                                        "{}:{}:{}",
                                                        row.file, row.line, row.column
                                                    ));
                                                    self.view.status =
                                                        "Problem location copied".into();
                                                    ui.close();
                                                }
                                            });
                                            if response.clicked() {
                                                match &row.target {
                                                    ProblemTarget::Document { id, range } => {
                                                        self.command(Command::OpenProblem(
                                                            *id,
                                                            range.clone(),
                                                        ));
                                                    }
                                                    ProblemTarget::Location(path) => {
                                                        self.command(Command::OpenTaskProblem(
                                                            path.clone(),
                                                            row.line,
                                                            row.column,
                                                        ));
                                                    }
                                                }
                                            }
                                            // Agent fixes work on an open document's range;
                                            // a task's printed location has neither.
                                            if let ProblemTarget::Document { id, range } =
                                                &row.target
                                            {
                                                let fix = ui
                                                    .add(
                                                        egui::Button::new(icon::AGENT)
                                                            .small()
                                                            .frame(false),
                                                    )
                                                    .on_hover_text(
                                                        "Fix with agent: prefill a request in the agent panel",
                                                    );
                                                if super::widgets::named(
                                                    fix,
                                                    true,
                                                    "Fix this problem with the agent",
                                                )
                                                .clicked()
                                                {
                                                    self.command(Command::FixProblemWithAgent(
                                                        *id,
                                                        range.clone(),
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                    if visible_rows == 0 {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(if rows.is_empty() {
                                                    empty_message(
                                                        self.view.language_server_status.as_ref(),
                                                    )
                                                } else {
                                                    "No problems match the current filters."
                                                })
                                                .size(crate::ui::theme::META_SIZE)
                                                .color(colors.muted),
                                            )
                                            .wrap(),
                                        );
                                    }
                                });
                        });
                });
            });
    }

    fn problem_rows(&self) -> Vec<ProblemRow> {
        let mut rows = Vec::new();
        for document in &self.editor.files {
            let Some(diagnostics) = self.view.language_server_diagnostics.get(&document.id) else {
                continue;
            };
            let file = document
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| document.name.clone());
            for diagnostic in diagnostics {
                let start = diagnostic.range.start.min(document.text.len());
                let prefix = &document.text[..start];
                let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = prefix
                    .rsplit('\n')
                    .next()
                    .map(str::chars)
                    .map(Iterator::count)
                    .unwrap_or(0)
                    + 1;
                let detail = match (&diagnostic.source, &diagnostic.code) {
                    (Some(source), Some(code)) => format!("  {source}({code})"),
                    (Some(source), None) => format!("  {source}"),
                    (None, Some(code)) => format!("  {code}"),
                    (None, None) => String::new(),
                };
                rows.push(ProblemRow {
                    target: ProblemTarget::Document {
                        id: document.id,
                        range: diagnostic.range.clone(),
                    },
                    file: file.clone(),
                    line,
                    column,
                    severity: diagnostic.severity,
                    message: diagnostic
                        .message
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                    full_message: diagnostic.message.clone(),
                    detail,
                });
            }
        }
        rows.extend(self.task_problem_rows());
        rows.sort_by(|left, right| {
            left.file
                .cmp(&right.file)
                .then(left.line.cmp(&right.line))
                .then(left.column.cmp(&right.column))
        });
        rows
    }

    /// Problems found in task output: only the newest run of each task counts, so
    /// a rerun replaces its own older problems and nothing else's.
    fn task_problem_rows(&self) -> Vec<ProblemRow> {
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();
        for run in self.view.task_runs.runs().iter().rev() {
            if run.summary || !seen.insert(run.key.clone()) {
                continue;
            }
            for problem in &run.problems {
                let path = std::path::Path::new(&problem.path);
                let full = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    run.cwd.join(path)
                };
                let shown = self
                    .editor
                    .workspace
                    .as_deref()
                    .and_then(|root| full.strip_prefix(root).ok())
                    .unwrap_or(&full)
                    .display()
                    .to_string();
                let severity = match problem.severity {
                    lumen_task_core::Severity::Error => DiagnosticSeverity::Error,
                    lumen_task_core::Severity::Warning => DiagnosticSeverity::Warning,
                    lumen_task_core::Severity::Info => DiagnosticSeverity::Information,
                };
                rows.push(ProblemRow {
                    target: ProblemTarget::Location(full),
                    file: shown,
                    line: problem.line as usize,
                    column: problem.column as usize,
                    severity,
                    message: problem
                        .message
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                    full_message: problem.message.clone(),
                    detail: format!("  {}", run.label),
                });
            }
        }
        rows
    }
}

/// Why the list is empty. "No problems" is only honest when a server that could
/// report some is actually running.
fn empty_message(status: Option<&crate::app::view_state::LanguageServerStatus>) -> &'static str {
    use crate::app::view_state::LanguageServerStatus::*;
    match status {
        None => {
            "No language server is running for the open files, so no diagnostics are available."
        }
        Some(Starting(_)) => {
            "The language server is starting; diagnostics will appear when it is ready."
        }
        Some(Missing(_)) => {
            "The language server is not installed, so no diagnostics are available."
        }
        Some(Error(_)) => {
            "The language server stopped, so diagnostics may be out of date. Select LSP in the status bar for details."
        }
        Some(Ready(_) | Available(_)) => "No problems detected in open files.",
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{LumenApp, view_state::LanguageServerStatus};
    use eframe::egui;
    use lumen_language_services::diagnostics::{Diagnostic, DiagnosticSeverity};

    fn app(message: &str) -> LumenApp {
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.editor.files[0].text = "fn main() {}\n".into();
        app.view.language_server_diagnostics.insert(
            app.editor.files[0].id,
            vec![Diagnostic {
                range: 0..2,
                severity: DiagnosticSeverity::Error,
                message: message.into(),
                source: Some("rustc".into()),
                code: Some("E0001".into()),
            }],
        );
        app.view.problems.open = true;
        app
    }

    /// Texts with their horizontal extent, from the last of several frames.
    fn texts(app: &mut LumenApp, width: f32) -> Vec<(String, f32, f32)> {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let mut last = None;
        for _ in 0..3 {
            last = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 700.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.problems_panel(ctx)),
            ));
        }
        last.unwrap()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.job.text.clone(),
                    text.pos.x + text.galley.rect.min.x,
                    text.pos.x + text.galley.rect.max.x,
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn long_messages_truncate_and_keep_the_agent_action_visible() {
        let mut app = app(&"very long diagnostic ".repeat(40));
        for width in [320.0, 600.0, 1200.0] {
            let all = texts(&mut app, width);
            let (_, _, action_right) = all
                .iter()
                .find(|(text, ..)| text == crate::icon::AGENT)
                .unwrap_or_else(|| panic!("agent action missing at {width}"));
            assert!(*action_right <= width, "action offscreen at {width}");
            let (message, _, message_right) = all
                .iter()
                .find(|(text, ..)| text.contains("very long diagnostic"))
                .expect("message");
            assert!(
                *message_right <= width,
                "{}… overflows at {width}",
                &message[..20]
            );
        }
    }

    #[test]
    fn empty_state_explains_whether_diagnostics_can_exist() {
        let mut quiet = LumenApp::default();
        quiet.view.problems.open = true;
        let cases = [
            (None, "No language server is running"),
            (
                Some(LanguageServerStatus::Starting("rust-analyzer".into())),
                "starting",
            ),
            (
                Some(LanguageServerStatus::Missing("rust-analyzer".into())),
                "not installed",
            ),
            (
                Some(LanguageServerStatus::Error("boom".into())),
                "may be out of date",
            ),
            (
                Some(LanguageServerStatus::Ready("rust-analyzer".into())),
                "No problems detected",
            ),
        ];
        for (status, expected) in cases {
            quiet.view.language_server_status = status;
            let all = texts(&mut quiet, 900.0);
            assert!(
                all.iter().any(|(text, ..)| text.contains(expected)),
                "{expected:?} missing from {all:?}"
            );
        }
        // A filter that hides every row says so instead of claiming no problems.
        let mut filtered = app("boom");
        filtered.view.language_server_status = Some(LanguageServerStatus::Ready("ra".into()));
        filtered.view.problems.filter = "zzz".into();
        let all = texts(&mut filtered, 900.0);
        assert!(
            all.iter()
                .any(|(text, ..)| text.contains("match the current filters"))
        );
    }
}

#[cfg(test)]
mod task_problem_tests {
    use crate::{
        app::{LumenApp, command::Command},
        model::task_output::Stream,
        services::task_process::{TaskEvent, fake::FakeTaskLauncher},
    };
    use eframe::egui;
    use lumen_task_core::{Scope, TaskKey};
    use std::sync::Arc;

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
                |ctx| app.render_component(ctx, |workbench| workbench.problems_panel(ctx)),
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
    fn task_output_problems_appear_with_their_task_and_clear_on_request() {
        let root = tempfile::tempdir().unwrap();
        let path = crate::services::tasks::workspace_path(root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "schema_version = 1\n[[tasks]]\nid = \"check\"\nlabel = \"Check\"\ntype = \"process\"\ncommand = \"cc\"\nsave = \"none\"\nproblem_matcher = \"gcc\"\n",
        )
        .unwrap();
        let fake = FakeTaskLauncher::default();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.path().to_path_buf());
        app.task_launcher = Arc::new(fake.clone());
        let key = TaskKey::new(Scope::Workspace, "check");
        let (def, _) = app.read_task(&key).unwrap();
        app.task_trust
            .trust(root.path(), &lumen_task_core::execution_digest(&def))
            .unwrap();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        app.execute(Command::RunTask(key), &ctx);
        fake.process(0)
            .lock()
            .unwrap()
            .queued
            .push(TaskEvent::Output(
                Stream::Stdout,
                b"src/a.c:12:5: error: expected ';'\nsrc/b.c:3:1: warning: unused\n".to_vec(),
            ));
        app.poll_task_runs(&ctx);
        app.view.problems.open = true;
        let shown = texts(&mut app, &ctx);
        assert!(
            shown.contains("src/a.c") && shown.contains("expected ';'"),
            "{shown}"
        );
        assert!(
            shown.contains("[12:5]") && shown.contains("Check"),
            "{shown}"
        );
        assert!(
            shown.contains("2 Errors") || shown.contains("1 Errors"),
            "{shown}"
        );
        // Finish the run, then clear: the rows go away.
        {
            let state = fake.process(0);
            let mut state = state.lock().unwrap();
            state.exit = Some(crate::services::task_process::ExitReport::Code(1));
            state.queued.push(TaskEvent::Closed(Stream::Stdout));
            state.queued.push(TaskEvent::Closed(Stream::Stderr));
        }
        app.poll_task_runs(&ctx);
        app.execute(Command::ClearTaskProblems, &ctx);
        assert!(!texts(&mut app, &ctx).contains("expected ';'"));
    }
}
