use super::{
    Workbench, theme,
    widgets::{HeaderAction, panel_header},
};
use crate::{app::command::Command, icon};
use eframe::egui::{
    self, Color32, FontId, RichText, ScrollArea, TextFormat, TopBottomPanel, text::LayoutJob,
};

const TERMINAL_HEIGHT: f32 = 220.0;
const MIN_TERMINAL_HEIGHT: f32 = 120.0;
const MAX_TERMINAL_HEIGHT: f32 = 520.0;
const CELL_WIDTH_FACTOR: f32 = 0.58;

impl Workbench<'_> {
    pub(crate) fn terminal_panel(&mut self, ctx: &egui::Context) {
        if !self.view.terminal.open {
            return;
        }
        let font_size = self.view.settings.number("terminal.font_size", None);
        let line_height = self.view.settings.number("terminal.line_height", None);
        let font = if self.view.settings.choice("terminal.font_family") == "Hack" {
            FontId::new(font_size, egui::FontFamily::Name("lumen-hack".into()))
        } else {
            FontId::monospace(font_size)
        };
        let cursor_style = self.view.settings.choice("terminal.cursor_style");
        let color_scheme = self.view.settings.choice("terminal.color_scheme");
        let cursor_blink = self.view.settings.boolean("terminal.cursor_blink");
        let panel_divider = self.view.settings.boolean("terminal.panel_divider");
        let canvas_focused =
            ctx.memory(|memory| memory.has_focus(egui::Id::new("terminal_canvas")));
        let cursor_visible =
            !cursor_blink || !canvas_focused || ctx.input(|input| input.time % 1.0 < 0.5);
        TopBottomPanel::bottom("terminal")
            .default_height(TERMINAL_HEIGHT)
            .height_range(MIN_TERMINAL_HEIGHT..=MAX_TERMINAL_HEIGHT)
            .resizable(true)
            .show_separator_line(panel_divider)
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
                // The emulator owns both output and the active prompt. Keep a
                // single fixed canvas so its scrollback cannot affect panel
                // geometry, and derive PTY dimensions from that canvas.
                let terminal_height = ui.available_height();
                self.view.terminal.rows =
                    ((terminal_height - 10.0) / line_height).floor().max(1.0) as u16;
                self.view.terminal.columns = ((ui.available_width() - 14.0)
                    / (font_size * CELL_WIDTH_FACTOR))
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
                        let frame = egui::Frame::default()
                            .fill(theme::BG)
                            .corner_radius(egui::CornerRadius::same(3))
                            .inner_margin(egui::Margin::symmetric(7, 5))
                            .show(ui, |ui| {
                                ScrollArea::vertical()
                                    .id_salt("terminal_output")
                                    .stick_to_bottom(self.view.terminal.scroll_to_bottom)
                                    .max_height(ui.available_height())
                                    .show(ui, |ui| {
                                        if let Some(screen) = screen {
                                            for row in 0..screen.rows {
                                                if let Some(cells) = screen.row(row) {
                                                    ui.label(terminal_line(
                                                        cells,
                                                        &font,
                                                        &cursor_style,
                                                        cursor_visible,
                                                        &color_scheme,
                                                    ));
                                                }
                                            }
                                        } else {
                                            ui.label(
                                                RichText::new("Starting terminal…")
                                                    .color(theme::MUTED),
                                            );
                                        }
                                    });
                            });
                        let canvas = ui.interact(
                            frame.response.rect,
                            egui::Id::new("terminal_canvas"),
                            egui::Sense::click(),
                        );
                        if canvas.clicked() {
                            canvas.request_focus();
                        }
                        if canvas.has_focus() {
                            for event in ctx.input(|input| input.events.clone()) {
                                if let Some(bytes) = terminal_input_bytes(event) {
                                    self.command(Command::SendTerminalInput(bytes));
                                }
                            }
                        }
                    },
                );
                self.view.terminal.scroll_to_bottom = false;
            });
    }
}

fn terminal_line(
    cells: &[lumen_terminal_core::Cell],
    font: &FontId,
    cursor_style: &str,
    cursor_visible: bool,
    color_scheme: &str,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    for cell in cells {
        let foreground = color(cell.foreground, theme::TEXT, color_scheme);
        let background = color(cell.background, theme::BG, color_scheme);
        let cursor = cell.cursor && cursor_visible;
        let block_cursor = cursor && cursor_style == "block";
        job.append(
            &cell.character.to_string(),
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: if block_cursor { background } else { foreground },
                background: if block_cursor { foreground } else { background },
                underline: if cursor && cursor_style == "underline" {
                    egui::Stroke::new(1.0_f32, foreground)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            },
        );
    }
    job
}

fn terminal_input_bytes(event: egui::Event) -> Option<Vec<u8>> {
    match event {
        egui::Event::Text(text) => (!text.is_empty()).then(|| text.into_bytes()),
        egui::Event::Paste(text) => Some(text.into_bytes()),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            if modifiers.ctrl && !modifiers.mac_cmd {
                ctrl_byte(key).map(|byte| vec![byte])
            } else {
                match key {
                    egui::Key::Enter => Some(vec![b'\r']),
                    egui::Key::Backspace => Some(vec![0x7f]),
                    egui::Key::Tab => Some(vec![b'\t']),
                    egui::Key::Escape => Some(vec![0x1b]),
                    egui::Key::ArrowUp => Some(b"\x1b[A".to_vec()),
                    egui::Key::ArrowDown => Some(b"\x1b[B".to_vec()),
                    egui::Key::ArrowRight => Some(b"\x1b[C".to_vec()),
                    egui::Key::ArrowLeft => Some(b"\x1b[D".to_vec()),
                    egui::Key::Home => Some(b"\x1b[H".to_vec()),
                    egui::Key::End => Some(b"\x1b[F".to_vec()),
                    egui::Key::Delete => Some(b"\x1b[3~".to_vec()),
                    egui::Key::PageUp => Some(b"\x1b[5~".to_vec()),
                    egui::Key::PageDown => Some(b"\x1b[6~".to_vec()),
                    _ => None,
                }
            }
        }
        _ => None,
    }
}

fn ctrl_byte(key: egui::Key) -> Option<u8> {
    use egui::Key;
    let byte = match key {
        Key::A => 1,
        Key::B => 2,
        Key::C => 3,
        Key::D => 4,
        Key::E => 5,
        Key::F => 6,
        Key::G => 7,
        Key::H => 8,
        Key::I => 9,
        Key::J => 10,
        Key::K => 11,
        Key::L => 12,
        Key::M => 13,
        Key::N => 14,
        Key::O => 15,
        Key::P => 16,
        Key::Q => 17,
        Key::R => 18,
        Key::S => 19,
        Key::T => 20,
        Key::U => 21,
        Key::V => 22,
        Key::W => 23,
        Key::X => 24,
        Key::Y => 25,
        Key::Z => 26,
        _ => return None,
    };
    Some(byte)
}

fn color(
    color: lumen_terminal_core::TerminalColor,
    fallback: Color32,
    color_scheme: &str,
) -> Color32 {
    use lumen_terminal_core::TerminalColor;
    match color {
        TerminalColor::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        TerminalColor::Indexed(index) => indexed_color(index, color_scheme),
        // Alacritty uses these special indexes for the active default colours.
        // Keeping the caller's fallback lets foreground and background retain
        // Lumen's terminal palette without hard-coding either one here.
        TerminalColor::Named(256) => {
            if color_scheme == "high_contrast" {
                Color32::WHITE
            } else {
                fallback
            }
        }
        TerminalColor::Named(257) => {
            if color_scheme == "high_contrast" {
                Color32::BLACK
            } else {
                theme::BG
            }
        }
        TerminalColor::Named(index) => indexed_color(index as u8, color_scheme),
    }
}

fn indexed_color(index: u8, color_scheme: &str) -> Color32 {
    if color_scheme == "high_contrast" {
        return HIGH_CONTRAST
            .get(index as usize)
            .copied()
            .unwrap_or(theme::TEXT);
    }
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

const HIGH_CONTRAST: [Color32; 16] = [
    Color32::BLACK,
    Color32::from_rgb(255, 85, 85),
    Color32::from_rgb(80, 255, 128),
    Color32::from_rgb(255, 255, 85),
    Color32::from_rgb(100, 170, 255),
    Color32::from_rgb(255, 110, 255),
    Color32::from_rgb(85, 255, 255),
    Color32::from_rgb(235, 235, 235),
    Color32::from_rgb(125, 125, 125),
    Color32::from_rgb(255, 120, 120),
    Color32::from_rgb(120, 255, 160),
    Color32::from_rgb(255, 255, 120),
    Color32::from_rgb(130, 190, 255),
    Color32::from_rgb(255, 150, 255),
    Color32::from_rgb(120, 255, 255),
    Color32::WHITE,
];

#[cfg(test)]
mod tests {
    use super::terminal_input_bytes;
    use eframe::egui;

    #[test]
    fn terminal_keys_map_to_the_bytes_expected_by_a_pty() {
        let enter = terminal_input_bytes(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let up = terminal_input_bytes(egui::Event::Key {
            key: egui::Key::ArrowUp,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        assert_eq!(enter, Some(vec![b'\r']));
        assert_eq!(up, Some(b"\x1b[A".to_vec()));
        assert_eq!(
            terminal_input_bytes(egui::Event::Text("echo ok".into())),
            Some(b"echo ok".to_vec())
        );
    }
}
