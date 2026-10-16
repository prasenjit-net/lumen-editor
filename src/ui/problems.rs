use super::{
    Workbench, theme,
    widgets::{HeaderAction, panel_header},
};
use crate::{app::command::Command, icon};
use eframe::egui::{self, RichText, ScrollArea, TopBottomPanel};
use lumen_language_services::diagnostics::DiagnosticSeverity;

const DEFAULT_HEIGHT: f32 = 210.0;

#[derive(Clone)]
struct ProblemRow {
    document: crate::model::DocumentId,
    file: String,
    range: std::ops::Range<usize>,
    line: usize,
    column: usize,
    severity: DiagnosticSeverity,
    message: String,
    detail: String,
}

impl Workbench<'_> {
    pub(crate) fn problems_panel(&mut self, ctx: &egui::Context) {
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
        TopBottomPanel::bottom("problems")
            .default_height(DEFAULT_HEIGHT)
            .height_range(120.0..=520.0)
            .resizable(true)
            .frame(egui::Frame::default().fill(theme::PANEL))
            .show(ctx, |ui| {
                ui.set_min_height(ui.available_height());
                let actions = [HeaderAction {
                    glyph: icon::X,
                    tooltip: "Close Problems",
                    command: Command::CloseProblems,
                    enabled: true,
                }];
                if let Some(command) = panel_header(ui, "PROBLEMS", &actions) {
                    self.command(command);
                }
                ui.horizontal(|ui| {
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
                            .desired_width(190.0),
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
                                DiagnosticSeverity::Information | DiagnosticSeverity::Hint => true,
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
                                        .size(11.0)
                                        .color(theme::TEXT),
                                );
                                previous_file = Some(row.file.clone());
                            }
                            let (glyph, color) = match row.severity {
                                DiagnosticSeverity::Error => {
                                    (icon::ERROR, theme::DIFF_REMOVED_TEXT)
                                }
                                DiagnosticSeverity::Warning => {
                                    (icon::WARNING, egui::Color32::from_rgb(204, 167, 0))
                                }
                                DiagnosticSeverity::Information | DiagnosticSeverity::Hint => {
                                    (icon::DEBUG, theme::MUTED)
                                }
                            };
                            let label = format!(
                                "  {glyph}  {}  [{}:{}]{}",
                                row.message, row.line, row.column, row.detail
                            );
                            let response = ui.add(
                                egui::Button::new(RichText::new(label).size(11.0).color(color))
                                    .frame(false),
                            );
                            if response.clicked() {
                                self.command(Command::OpenProblem(row.document, row.range.clone()));
                            }
                        }
                        if visible_rows == 0 {
                            ui.label(
                                RichText::new(if rows.is_empty() {
                                    "No problems detected in open files."
                                } else {
                                    "No problems match the current filters."
                                })
                                .size(11.0)
                                .color(theme::MUTED),
                            );
                        }
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
                    document: document.id,
                    file: file.clone(),
                    range: diagnostic.range.clone(),
                    line,
                    column,
                    severity: diagnostic.severity,
                    message: diagnostic
                        .message
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                    detail,
                });
            }
        }
        rows.sort_by(|left, right| {
            left.file
                .cmp(&right.file)
                .then(left.range.start.cmp(&right.range.start))
        });
        rows
    }
}
