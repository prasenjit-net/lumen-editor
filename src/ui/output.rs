//! Read-only Output panel: the editor's timestamped log, selectable and copyable.
//! It shares the terminal's bottom-panel slot (same panel id, so the height the
//! user chose is kept when switching tabs).
use super::{
    Workbench, theme,
    widgets::{HeaderAction, tabbed_panel_header},
};
use crate::{app::command::Command, icon, model::output_log::LogLevel};
use eframe::egui::{
    self, Color32, FontId, ScrollArea, TextFormat, TopBottomPanel, text::LayoutJob,
};

const DEFAULT_HEIGHT: f32 = 220.0;
const MIN_HEIGHT: f32 = 120.0;
const MAX_HEIGHT: f32 = 520.0;

fn level_color(level: LogLevel, colors: theme::Palette) -> Color32 {
    match level {
        LogLevel::Info => colors.text,
        LogLevel::Warn => colors.warning,
        LogLevel::Error => colors.error,
    }
}

/// One entry as a single selectable row. Pieces are separated exactly like
/// `LogEntry::line`, so copied text matches what is on screen.
fn entry_job(
    entry: &crate::model::output_log::LogEntry,
    font: &FontId,
    colors: theme::Palette,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let piece = |text: String, color: Color32| {
        (
            text,
            TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        )
    };
    for (text, format) in [
        piece(format!("{} ", entry.timestamp), colors.muted),
        piece(
            format!("[{}] ", entry.level.label()),
            level_color(entry.level, colors),
        ),
        piece(format!("{}: ", entry.source), colors.muted),
        piece(entry.message.clone(), level_color(entry.level, colors)),
    ] {
        job.append(&text, 0.0, format);
    }
    job
}

impl Workbench<'_> {
    pub(crate) fn output_panel(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.output.open {
            return;
        }
        let font_size = self
            .view
            .settings
            .number("terminal.font_size", None)
            .min(13.0);
        let font = FontId::monospace(font_size);
        let panel_divider = self.view.settings.boolean("terminal.panel_divider");
        TopBottomPanel::bottom("terminal")
            .default_height(DEFAULT_HEIGHT)
            .height_range(MIN_HEIGHT..=MAX_HEIGHT)
            .resizable(true)
            .show_separator_line(panel_divider)
            .frame(egui::Frame::default().fill(colors.panel))
            .show(ctx, |ui| {
                ui.set_min_height(ui.available_height());
                let entries = self.bottom_tab_entries(super::bottom_tabs::ActiveBottom::Output);
                let tabs = super::bottom_tabs::as_panel_tabs(&entries);
                let actions = [
                    HeaderAction {
                        glyph: icon::COPY,
                        tooltip: "Copy all output",
                        command: Command::CopyOutput,
                        enabled: !self.view.output.entries.is_empty(),
                    },
                    HeaderAction {
                        glyph: icon::CLEAR_ALL,
                        tooltip: "Clear output",
                        command: Command::ClearOutput,
                        enabled: !self.view.output.entries.is_empty(),
                    },
                    HeaderAction {
                        glyph: icon::X,
                        tooltip: "Close output",
                        command: Command::CloseOutput,
                        enabled: true,
                    },
                ];
                if let Some(command) = tabbed_panel_header(ui, &tabs, &actions) {
                    self.command(command);
                }
                let scroll_to_bottom = std::mem::take(&mut self.view.output.scroll_to_bottom);
                egui::Frame::default()
                    .fill(colors.bg)
                    .corner_radius(egui::CornerRadius::same(3))
                    .inner_margin(egui::Margin::symmetric(7, 5))
                    .show(ui, |ui| {
                        ScrollArea::both()
                            .id_salt("output_log")
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .max_height(ui.available_height())
                            .show(ui, |ui| {
                                if self.view.output.entries.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No output yet.")
                                            .color(colors.muted)
                                            .size(11.0),
                                    );
                                }
                                ui.spacing_mut().item_spacing.y = 1.0;
                                for entry in &self.view.output.entries {
                                    // Wrapping off keeps one entry on one row;
                                    // long messages scroll horizontally.
                                    ui.add(
                                        egui::Label::new(entry_job(entry, &font, colors))
                                            .wrap_mode(egui::TextWrapMode::Extend)
                                            .selectable(true),
                                    );
                                }
                                if scroll_to_bottom {
                                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                                }
                            });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::LumenApp,
        model::output_log::{LogEntry, LogLevel},
    };

    fn entry(sequence: u64, level: LogLevel, message: &str) -> LogEntry {
        LogEntry {
            sequence,
            timestamp: format!("2026-10-16 09:00:0{sequence}.125"),
            level,
            source: "editor".into(),
            message: message.into(),
        }
    }

    fn frame(app: &mut LumenApp, events: Vec<egui::Event>) -> egui::FullOutput {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.output_panel(ctx)),
        )
    }

    fn texts(output: &egui::FullOutput) -> String {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn closed_panel_draws_nothing_and_open_panel_lists_timestamped_entries() {
        let mut app = LumenApp::default();
        app.view.output.entries = vec![
            entry(1, LogLevel::Info, "Lumen started"),
            entry(2, LogLevel::Error, "Save failed: disk full"),
        ];
        assert!(texts(&frame(&mut app, vec![])).is_empty());

        app.view.output.open = true;
        let shown = texts(&frame(&mut app, vec![]));
        assert!(shown.contains("OUTPUT") && shown.contains("TERMINAL"));
        assert!(shown.contains("2026-10-16 09:00:01.125 "));
        assert!(shown.contains("[ERROR] "));
        assert!(shown.contains("Save failed: disk full"));
    }

    #[test]
    fn empty_log_says_so_and_disables_copy_and_clear() {
        let mut app = LumenApp::default();
        app.view.output.open = true;
        assert!(texts(&frame(&mut app, vec![])).contains("No output yet."));
    }

    #[test]
    fn copy_command_puts_every_line_on_the_clipboard() {
        let mut app = LumenApp::default();
        app.view.output.entries = vec![
            entry(1, LogLevel::Info, "one"),
            entry(2, LogLevel::Warn, "two"),
        ];
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            app.execute(crate::app::command::Command::CopyOutput, ctx);
        });
        let copied = output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
        assert_eq!(
            copied.as_deref(),
            Some(
                "2026-10-16 09:00:01.125 [INFO] editor: one\n2026-10-16 09:00:02.125 [WARN] editor: two"
            )
        );
    }
}
