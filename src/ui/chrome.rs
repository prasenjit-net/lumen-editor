use super::Workbench;
use crate::app::view_state::LanguageServerStatus;
use crate::{app::command::Command, icon};
use eframe::egui::{self, Color32, Frame, Margin, RichText, TopBottomPanel};
#[cfg(target_os = "macos")]
use eframe::egui::{FontId, Vec2};
/// Room the title bar reserves for the task launcher.
#[cfg(target_os = "macos")]
const LAUNCHER_WIDTH: f32 = 350.0;
/// Narrowest span worth drawing a title in.
#[cfg(target_os = "macos")]
const MIN_TITLE_WIDTH: f32 = 48.0;
/// Space AppKit keeps for the real traffic-light buttons.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS: f32 = 80.0;

/// Horizontal span available to the window title between the traffic lights and
/// the task launcher, with a small gap on each side.
#[cfg(target_os = "macos")]
fn title_span(rect: egui::Rect, launcher_width: f32) -> (f32, f32) {
    (
        rect.left() + TRAFFIC_LIGHTS + 8.0,
        rect.right() - launcher_width - 8.0,
    )
}

impl Workbench<'_> {
    pub(crate) fn title_bar(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let title = self
            .active()
            .map(|file| {
                format!(
                    "{}{} — Lumen",
                    file.name,
                    if file.dirty { " ●" } else { "" }
                )
            })
            .unwrap_or_else(|| "Lumen".into());
        if self.view.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.view.window_title = title;
        }
        #[cfg(target_os = "macos")]
        TopBottomPanel::top("mac_titlebar")
            .exact_height(32.0)
            .frame(Frame::default().fill(colors.title))
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 32.0),
                    egui::Sense::hover(),
                );
                // The launcher sits at the right edge; the centered title yields
                // when the two would overlap.
                let launcher_width = LAUNCHER_WIDTH.min(rect.width() / 2.0);
                // The title lives between the traffic lights and the launcher: centred
                // when it fits there, otherwise cut with an ellipsis rather than hidden.
                let (left, right) = title_span(rect, launcher_width);
                if right - left >= MIN_TITLE_WIDTH {
                    let title_galley = super::widgets::fit_galley(
                        ui,
                        &self.view.window_title,
                        FontId::proportional(13.0),
                        colors.text,
                        right - left,
                    );
                    let size = title_galley.size();
                    let x =
                        (rect.center().x - size.x / 2.0).clamp(left, (right - size.x).max(left));
                    ui.painter().galley(
                        egui::pos2(x, rect.center().y - size.y / 2.0),
                        title_galley,
                        colors.text,
                    );
                }
                // Leave the leftmost 80px to AppKit's real traffic-light buttons.
                let drag_rect =
                    egui::Rect::from_min_max(rect.min + Vec2::new(TRAFFIC_LIGHTS, 0.0), rect.max);
                let response = ui.interact(
                    drag_rect,
                    ui.id().with("window_drag"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                // Added after the drag area so its buttons win the hit test.
                let launcher_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.right() - launcher_width, rect.top()),
                    rect.max,
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(launcher_rect.shrink2(Vec2::new(8.0, 0.0)))
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        self.panel_toggles(ui);
                        self.task_launcher(ui);
                    },
                );
            });
        #[cfg(target_os = "windows")]
        TopBottomPanel::top("task_toolbar")
            .exact_height(28.0)
            .frame(
                Frame::default()
                    .fill(colors.title)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.panel_toggles(ui);
                    self.task_launcher(ui);
                });
            });
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        TopBottomPanel::top("titlebar")
            .exact_height(35.0)
            .frame(
                Frame::default()
                    .fill(colors.title)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                Frame::default().fill(colors.title).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if !self.view.safety_notices.is_empty() {
                            let label = format!("Safety: {}", self.view.safety_notices.len());
                            if ui
                                .small_button(label)
                                .on_hover_text("Show recovery and save warnings")
                                .clicked()
                            {
                                self.view.safety_details_open = true;
                            }
                        }
                        ui.label(
                            RichText::new(format!("{}  LUMEN", icon::BRACKETS_CURLY))
                                .strong()
                                .color(colors.muted),
                        );
                        ui.add_space(12.0);
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        {
                            self.file_menu(ui);
                            for item in
                                ["Edit", "Selection", "View", "Go", "Run", "Terminal", "Help"]
                            {
                                // Only File is wired on this platform; the rest are
                                // shown muted and say so instead of looking clickable.
                                ui.label(RichText::new(item).size(12.0).color(colors.muted))
                                    .on_hover_text("Not available in this menu yet");
                                ui.add_space(2.0);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let title = self
                                .active()
                                .map(|file| {
                                    format!(
                                        "{}{} — Lumen",
                                        file.name,
                                        if file.dirty { " ●" } else { "" }
                                    )
                                })
                                .unwrap_or_else(|| "Lumen".into());
                            self.panel_toggles(ui);
                            self.task_launcher(ui);
                            ui.add_space(8.0);
                            ui.label(RichText::new(title).color(colors.muted).size(12.0));
                        });
                    });
                });
            });
    }
    pub(crate) fn status_bar(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let info = self.status_document_info();
        let (errors, warnings) = self.status_diagnostic_counts();
        let problems_label = format!("{} {errors}  {} {warnings}", icon::ERROR, icon::WARNING);
        let position_label = info.position.map(|(line, column)| {
            let mut label = format!("Ln {line}, Col {column}");
            if info.selected > 0 {
                label.push_str(&format!(" ({} selected)", info.selected));
            }
            label
        });
        let meta_label = match info.eol {
            Some(eol) => format!(
                "{}{}   UTF-8   {}",
                info.cursors,
                info.language,
                eol.label()
            ),
            None => String::new(),
        };
        let lsp = self.view.language_server_status.as_ref().map(lsp_label);
        TopBottomPanel::bottom("status")
            .exact_height(22.0)
            .frame(
                Frame::default()
                    .fill(colors.status)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                Frame::default().fill(colors.status).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    let font = egui::FontId::proportional(11.0);
                    let width = |ui: &egui::Ui, text: &str| {
                        ui.fonts_mut(|fonts| {
                            fonts
                                .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
                                .size()
                                .x
                        })
                    };
                    ui.horizontal(|ui| {
                        let gap = ui.spacing().item_spacing.x;
                        let problems_width = width(ui, &problems_label) + 2.0 * gap;
                        // Reserve the right-hand facts first so a long transient
                        // message can never push them (or the counts) off screen.
                        // Under pressure the language/EOL facts go first, then the
                        // language-server state; the cursor position and the problem
                        // counts always stay.
                        let right_width = |ui: &egui::Ui, meta: bool, server: bool| {
                            let mut right = gap;
                            if meta {
                                right += width(ui, &meta_label) + gap;
                            }
                            if let Some(label) = &position_label {
                                right += width(ui, label) + gap;
                            }
                            if server && let Some((label, ..)) = &lsp {
                                right += width(ui, label) + gap;
                            }
                            right
                        };
                        let total = ui.available_width();
                        let budget = total * 0.7;
                        let mut show_meta = true;
                        let mut show_server = true;
                        if right_width(ui, show_meta, show_server) > budget {
                            show_meta = false;
                        }
                        if right_width(ui, show_meta, show_server) > budget {
                            show_server = false;
                        }
                        let right = right_width(ui, show_meta, show_server).min(budget);
                        let left = (total - right - gap).max(0.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(left, 22.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.scope(|ui| {
                                    ui.set_max_width((left - problems_width).max(0.0));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&self.view.status)
                                                .size(crate::ui::theme::META_SIZE)
                                                .color(Color32::WHITE),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&self.view.status);
                                });
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(&problems_label)
                                                .size(crate::ui::theme::META_SIZE)
                                                .color(Color32::WHITE),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Show Problems")
                                    .clicked()
                                {
                                    self.command(Command::ToggleProblems);
                                }
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if show_meta {
                                ui.label(
                                    RichText::new(&meta_label)
                                        .size(crate::ui::theme::META_SIZE)
                                        .color(Color32::WHITE),
                                )
                                .on_hover_text(
                                    info.eol
                                        .map(|eol| eol.describe())
                                        .unwrap_or("No document is open"),
                                );
                            }
                            if let Some(label) = &position_label
                                && ui
                                    .add(
                                        egui::Label::new(
                                            RichText::new(label)
                                                .size(crate::ui::theme::META_SIZE)
                                                .color(Color32::WHITE),
                                        )
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Go to Line…")
                                    .clicked()
                            {
                                self.command(Command::GoToLine);
                            }
                            if show_server && let Some((label, tooltip, is_error)) = &lsp {
                                let response = ui
                                    .add(
                                        egui::Label::new(
                                            RichText::new(label)
                                                .size(crate::ui::theme::META_SIZE)
                                                .color(Color32::WHITE),
                                        )
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(tooltip);
                                if response.clicked() && *is_error {
                                    self.view.language_server_details_open = true;
                                }
                            }
                        });
                    });
                });
            });
    }

    fn status_diagnostic_counts(&self) -> (usize, usize) {
        use lumen_language_services::diagnostics::DiagnosticSeverity::{Error, Warning};
        let count = |severity| {
            self.view
                .language_server_diagnostics
                .values()
                .flatten()
                .filter(|diagnostic| diagnostic.severity == severity)
                .count()
        };
        (count(Error), count(Warning))
    }

    fn status_document_info(&self) -> StatusDocumentInfo {
        let Some(file) = self.active() else {
            return StatusDocumentInfo {
                language: "Plain Text".into(),
                ..Default::default()
            };
        };
        let (position, selected, cursors) = file
            .editing
            .as_ref()
            .map(|buffer| {
                let selections = buffer.selections().selections();
                let primary = &selections[buffer.selections().primary_index()];
                let range = primary.range();
                let selected = file
                    .text
                    .get(range.clone())
                    .map_or(0, |text| text.chars().count());
                let cursors = if selections.len() > 1 {
                    let noun = if selections
                        .iter()
                        .any(|selection| !selection.range().is_empty())
                    {
                        "selections"
                    } else {
                        "cursors"
                    };
                    format!("{} {noun}   ", selections.len())
                } else {
                    String::new()
                };
                (
                    Some(crate::model::status_info::line_column(
                        &file.text,
                        primary.head,
                    )),
                    selected,
                    cursors,
                )
            })
            .unwrap_or((None, 0, String::new()));
        StatusDocumentInfo {
            language: file.language.as_str().to_owned(),
            eol: Some(crate::model::status_info::detect_line_ending(&file.text)),
            position,
            selected,
            cursors,
        }
    }
}

#[derive(Default)]
struct StatusDocumentInfo {
    language: String,
    eol: Option<crate::model::status_info::LineEnding>,
    position: Option<(usize, usize)>,
    selected: usize,
    cursors: String,
}

/// Label, tooltip and whether a click opens the failure details.
fn lsp_label(status: &LanguageServerStatus) -> (String, String, bool) {
    match status {
        LanguageServerStatus::Starting(name) => (
            format!("LSP: {name} starting"),
            "Language server is initializing".into(),
            false,
        ),
        LanguageServerStatus::Ready(name) => (
            format!("LSP: {name}"),
            "Language server is ready".into(),
            false,
        ),
        LanguageServerStatus::Available(name) => (
            format!("LSP: {name}"),
            "Language server available".into(),
            false,
        ),
        LanguageServerStatus::Missing(name) => (
            format!("LSP: {name} missing"),
            "Language server is not installed; editing and parser highlighting remain available"
                .into(),
            false,
        ),
        LanguageServerStatus::Error(_) => (
            "LSP: unavailable".into(),
            "Language server failed. Click to view and copy details.".into(),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::app::LumenApp;
    use eframe::egui;

    /// Every text drawn by the status bar with its horizontal extent.
    fn status_texts(app: &mut LumenApp, width: f32) -> Vec<(String, f32, f32)> {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let mut last = None;
        for _ in 0..3 {
            last = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.status_bar(ctx)),
            ));
        }
        last.unwrap()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.job.text.clone(),
                    // Right-aligned labels anchor at their right edge.
                    text.pos.x + text.galley.rect.min.x,
                    text.pos.x + text.galley.rect.max.x,
                )),
                _ => None,
            })
            .collect()
    }

    fn app_with(text: &str) -> LumenApp {
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.editor.files[0].text = text.into();
        app
    }

    fn meta(texts: &[(String, f32, f32)]) -> String {
        texts
            .iter()
            .find(|(text, ..)| text.contains("UTF-8"))
            .map(|(text, ..)| text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn line_ending_comes_from_the_document_not_a_constant() {
        for (text, expected) in [
            ("a\nb\n", "LF"),
            ("a\r\nb\r\n", "CRLF"),
            ("a\r\nb\n", "Mixed EOL"),
            ("single line", "No EOL"),
        ] {
            let mut app = app_with(text);
            let shown = meta(&status_texts(&mut app, 1000.0));
            assert!(shown.ends_with(expected), "{text:?} showed {shown:?}");
        }
    }

    #[test]
    fn without_a_document_no_encoding_or_ending_is_claimed() {
        let mut app = LumenApp::default();
        let texts = status_texts(&mut app, 1000.0);
        assert!(texts.iter().all(|(text, ..)| !text.contains("LF")));
        assert!(texts.iter().all(|(text, ..)| !text.starts_with("Ln ")));
    }

    #[test]
    fn a_long_message_cannot_push_the_right_hand_facts_offscreen() {
        let mut app = app_with("a\nb\n");
        app.view.status = "x".repeat(600);
        for width in [600.0, 1000.0] {
            let texts = status_texts(&mut app, width);
            let (shown, _, right) = texts
                .iter()
                .find(|(text, ..)| text.contains("UTF-8"))
                .expect("meta label");
            assert!(*right <= width + 0.5, "{shown:?} ends at {right} > {width}");
        }
    }

    #[test]
    fn narrow_status_bars_drop_language_facts_but_keep_the_cursor_position() {
        let mut app = app_with("a\nb\n");
        // The cursor position comes from the shared editing buffer.
        app.editor.files[0].enable_shared_editing();
        let wide = status_texts(&mut app, 1000.0);
        assert!(wide.iter().any(|(text, ..)| text.starts_with("Ln ")));
        assert!(wide.iter().any(|(text, ..)| text.contains("UTF-8")));
        let narrow = status_texts(&mut app, 240.0);
        assert!(
            narrow.iter().any(|(text, ..)| text.starts_with("Ln ")),
            "position stays: {narrow:?}"
        );
        assert!(
            narrow.iter().all(|(text, ..)| !text.contains("UTF-8")),
            "language and line-ending facts go first"
        );
        for (text, _, right) in &narrow {
            assert!(*right <= 240.5, "{text:?} ends at {right}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_mac_title_is_cut_with_an_ellipsis_instead_of_hidden() {
        let mut app = LumenApp::default();
        app.editor.new_file();
        app.editor.files[0].name =
            "a_remarkably_long_document_name_that_cannot_fit_beside_the_launcher.rs".into();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let width = 640.0_f32;
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 200.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.title_bar(ctx)),
            ));
        }
        let shown = output
            .unwrap()
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains("a_remarkably") => {
                    Some((text.galley.elided, text.pos.x + text.galley.rect.max.x))
                }
                _ => None,
            });
        let (elided, right) = shown.expect("the title is drawn, not hidden");
        assert!(elided, "a title that does not fit ends in an ellipsis");
        assert!(
            right <= width - super::LAUNCHER_WIDTH.min(width / 2.0),
            "{right}"
        );
    }
}
