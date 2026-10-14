use super::{
    Workbench, theme,
    widgets::{HeaderAction, panel_header},
};
use crate::{
    app::{command::Command, terminal::recall_terminal_history},
    icon,
};
use eframe::egui::{
    self, Color32, FontId, RichText, ScrollArea, TextEdit, TextFormat, TopBottomPanel,
    text::LayoutJob,
};

const TERMINAL_HEIGHT: f32 = 220.0;
const MIN_TERMINAL_HEIGHT: f32 = 120.0;
const MAX_TERMINAL_HEIGHT: f32 = 520.0;
const CELL_WIDTH: f32 = 7.2;
const CELL_HEIGHT: f32 = 17.0;

impl Workbench<'_> {
    pub(crate) fn terminal_panel(&mut self, ctx: &egui::Context) {
        if !self.view.terminal.open {
            return;
        }
        TopBottomPanel::bottom("terminal")
            .default_height(TERMINAL_HEIGHT)
            .height_range(MIN_TERMINAL_HEIGHT..=MAX_TERMINAL_HEIGHT)
            .resizable(true)
            .show_separator_line(false)
            .frame(egui::Frame::default().fill(theme::PANEL))
            .show(ctx, |ui| {
                // A resizable TopBottomPanel stores the size of its rendered
                // response. Claim the allocated height up-front so scrollback
                // content can neither grow nor shrink the persisted panel.
                ui.set_min_height(ui.available_height());
                let actions = [
                    HeaderAction {
                        glyph: icon::CLEAR_ALL,
                        tooltip: "Clear terminal output",
                        command: Command::ClearTerminal,
                        enabled: self.view.terminal.emulator.is_some(),
                    },
                    HeaderAction {
                        glyph: icon::X,
                        tooltip: "Close terminal",
                        command: Command::CloseTerminal,
                        enabled: true,
                    },
                ];
                if let Some(command) = panel_header(ui, "TERMINAL", &actions) {
                    self.command(command);
                }
                if let Some(error) = &self.view.terminal.error {
                    ui.label(
                        RichText::new(error)
                            .size(11.0)
                            .color(theme::DIFF_REMOVED_TEXT),
                    );
                }
                // Reserve fixed-height controls before deriving the PTY size.
                // Letting the scrollback determine a panel's desired height
                // feeds its output back into egui's persisted panel geometry.
                let terminal_height = ui.available_height();
                let output_height = (terminal_height - 27.0).max(CELL_HEIGHT);
                self.view.terminal.rows =
                    ((output_height - 10.0) / CELL_HEIGHT).floor().max(1.0) as u16;
                self.view.terminal.columns = ((ui.available_width() - 14.0) / CELL_WIDTH)
                    .floor()
                    .max(1.0) as u16;
                let screen = self
                    .view
                    .terminal
                    .emulator
                    .as_ref()
                    .map(lumen_terminal_core::Terminal::screen);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), terminal_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::default()
                            .fill(theme::BG)
                            .corner_radius(egui::CornerRadius::same(3))
                            .inner_margin(egui::Margin::symmetric(7, 5))
                            .show(ui, |ui| {
                                ScrollArea::vertical()
                                    .id_salt("terminal_output")
                                    .stick_to_bottom(self.view.terminal.scroll_to_bottom)
                                    .max_height(output_height - 10.0)
                                    .show(ui, |ui| {
                                        if let Some(screen) = screen {
                                            for row in 0..screen.rows {
                                                if let Some(cells) = screen.row(row) {
                                                    ui.label(terminal_line(cells));
                                                }
                                            }
                                        } else {
                                            ui.label(
                                                RichText::new("Starting terminal…")
                                                    .color(theme::MUTED),
                                            );
                                        }
                                    });
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(">")
                                            .font(FontId::monospace(12.5))
                                            .color(theme::MUTED),
                                    );
                                    let input_width = (ui.available_width() - 42.0).max(0.0);
                                    let response = ui.add_sized(
                                        [input_width, 18.0],
                                        TextEdit::singleline(&mut self.view.terminal.input)
                                            .id_salt("terminal_input")
                                            .frame(false)
                                            .font(FontId::monospace(12.5))
                                            .hint_text("Enter a command"),
                                    );
                                    if self.view.terminal.focus_input {
                                        response.request_focus();
                                        self.view.terminal.focus_input = false;
                                    }
                                    if response.has_focus() {
                                        let (submit, up, down) = ui.input(|input| {
                                            (
                                                input.key_pressed(egui::Key::Enter),
                                                input.key_pressed(egui::Key::ArrowUp)
                                                    && input.modifiers.is_none(),
                                                input.key_pressed(egui::Key::ArrowDown)
                                                    && input.modifiers.is_none(),
                                            )
                                        });
                                        if up {
                                            recall_terminal_history(
                                                &mut self.view.terminal,
                                                std::cmp::Ordering::Less,
                                            );
                                        } else if down {
                                            recall_terminal_history(
                                                &mut self.view.terminal,
                                                std::cmp::Ordering::Greater,
                                            );
                                        } else if submit {
                                            self.command(Command::SubmitTerminalInput);
                                        }
                                    }
                                    if ui
                                        .add_sized(
                                            [18.0, 18.0],
                                            egui::Button::new(icon::PLAY).frame(false),
                                        )
                                        .on_hover_text("Run command")
                                        .clicked()
                                    {
                                        self.command(Command::SubmitTerminalInput);
                                    }
                                });
                            });
                    },
                );
                self.view.terminal.scroll_to_bottom = false;
            });
    }
}

fn terminal_line(cells: &[lumen_terminal_core::Cell]) -> LayoutJob {
    let mut job = LayoutJob::default();
    let font = FontId::monospace(12.5);
    for cell in cells {
        job.append(
            &cell.character.to_string(),
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: color(cell.foreground, theme::TEXT),
                background: color(cell.background, theme::BG),
                ..Default::default()
            },
        );
    }
    job
}

fn color(color: lumen_terminal_core::TerminalColor, fallback: Color32) -> Color32 {
    use lumen_terminal_core::TerminalColor;
    match color {
        TerminalColor::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        TerminalColor::Indexed(index) => indexed_color(index),
        // Alacritty uses these special indexes for the active default colours.
        // Keeping the caller's fallback lets foreground and background retain
        // Lumen's terminal palette without hard-coding either one here.
        TerminalColor::Named(256) => fallback,
        TerminalColor::Named(257) => theme::BG,
        TerminalColor::Named(index) => indexed_color(index as u8),
    }
}

fn indexed_color(index: u8) -> Color32 {
    const ANSI: [Color32; 16] = [
        Color32::from_rgb(0, 0, 0),
        Color32::from_rgb(205, 49, 49),
        Color32::from_rgb(13, 188, 121),
        Color32::from_rgb(229, 229, 16),
        Color32::from_rgb(36, 114, 200),
        Color32::from_rgb(188, 63, 188),
        Color32::from_rgb(17, 168, 205),
        Color32::from_rgb(229, 229, 229),
        Color32::from_rgb(102, 102, 102),
        Color32::from_rgb(241, 76, 76),
        Color32::from_rgb(35, 209, 139),
        Color32::from_rgb(245, 245, 67),
        Color32::from_rgb(59, 142, 234),
        Color32::from_rgb(214, 112, 214),
        Color32::from_rgb(41, 184, 219),
        Color32::from_rgb(255, 255, 255),
    ];
    ANSI.get(index as usize).copied().unwrap_or(theme::TEXT)
}
