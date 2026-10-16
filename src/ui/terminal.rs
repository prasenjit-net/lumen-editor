use super::{
    Workbench, theme,
    widgets::{HeaderAction, PanelTab, tabbed_panel_header},
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
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
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
        let configured_scheme = self.view.settings.choice("terminal.color_scheme");
        let terminal_theme = match configured_scheme.as_str() {
            "workbench" => theme::current(&ctx.style().visuals),
            "light" => theme::Theme::Light,
            "high_contrast" => theme::Theme::HighContrast,
            _ => theme::Theme::Dark,
        };
        let color_scheme = match terminal_theme {
            theme::Theme::Light => "light",
            theme::Theme::HighContrast => "high_contrast",
            _ => "vscode_dark",
        };
        let terminal_colors = terminal_theme.palette();
        let cursor_blink = self.view.settings.boolean("terminal.cursor_blink");
        let auto_scroll = self.view.settings.boolean("terminal.auto_scroll");
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
            .frame(super::widgets::canvas(ctx))
            .show(ctx, |ui| {
                super::widgets::surface(colors.panel).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
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
                    let tabs = [
                        PanelTab {
                            label: "TERMINAL",
                            active: true,
                            command: Command::ShowTerminal,
                        },
                        PanelTab {
                            label: "OUTPUT",
                            active: false,
                            command: Command::ShowOutput,
                        },
                    ];
                    if let Some(command) = tabbed_panel_header(ui, &tabs, &actions) {
                        self.command(command);
                    }
                    if let Some(error) = &self.view.terminal.error {
                        ui.label(RichText::new(error).size(11.0).color(colors.error));
                    }
                    // The emulator owns both output and the active prompt. Keep a
                    // single fixed canvas so its scrollback cannot affect panel
                    // geometry, and derive PTY dimensions from that canvas.
                    let terminal_height = ui.available_height();
                    self.view.terminal.rows =
                        ((terminal_height - 10.0) / line_height).floor().max(1.0) as u16;
                    let cell_width = cell_advance(ui, &font);
                    self.view.terminal.columns = ((ui.available_width() - 14.0) / cell_width)
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
                            let colors = terminal_colors;
                            *ui.visuals_mut() = theme::visuals(terminal_theme);
                            let frame = egui::Frame::default()
                                .fill(colors.bg)
                                .corner_radius(egui::CornerRadius::same(3))
                                .inner_margin(egui::Margin::symmetric(7, 5))
                                .show(ui, |ui| {
                                    ScrollArea::vertical()
                                        .id_salt("terminal_output")
                                        // Stay pinned to the live prompt until the user
                                        // scrolls back, instead of only on the frame
                                        // that new output arrived.
                                        .stick_to_bottom(
                                            auto_scroll || self.view.terminal.scroll_to_bottom,
                                        )
                                        .max_height(ui.available_height())
                                        .show(ui, |ui| {
                                            // Rows touch: N rows of `line_height` must
                                            // exactly fill the N rows the PTY was sized
                                            // for, or the prompt scrolls out of view.
                                            ui.spacing_mut().item_spacing.y = 0.0;
                                            if let Some(screen) = screen {
                                                for row in 0..screen.rows {
                                                    if let Some(cells) = screen.row(row) {
                                                        ui.add(
                                                            egui::Label::new(terminal_line(
                                                                cells,
                                                                &font,
                                                                line_height,
                                                                &cursor_style,
                                                                cursor_visible,
                                                                color_scheme,
                                                                terminal_colors,
                                                            ))
                                                            .wrap_mode(egui::TextWrapMode::Extend),
                                                        );
                                                    }
                                                }
                                            } else {
                                                ui.label(
                                                    RichText::new("Starting terminal…")
                                                        .color(colors.muted),
                                                );
                                            }
                                        });
                                });
                            let canvas = ui.interact(
                                frame.response.rect,
                                egui::Id::new("terminal_canvas"),
                                egui::Sense::click(),
                            );
                            if canvas.clicked() || canvas.secondary_clicked() {
                                canvas.request_focus();
                            }
                            canvas.context_menu(|ui| {
                                // Copy and Paste ask the platform for its own clipboard
                                // events, so they follow the same paths as the keyboard
                                // shortcuts and never bypass the terminal's focus rules.
                                if ui.button("Copy").clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy);
                                    ui.close();
                                }
                                if ui.button("Paste").clicked() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .add_enabled(
                                        self.view.terminal.emulator.is_some(),
                                        egui::Button::new("Clear"),
                                    )
                                    .clicked()
                                {
                                    self.command(Command::ClearTerminal);
                                    ui.close();
                                }
                            });
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
            });
    }
}

/// Horizontal advance of one terminal cell, measured from the selected font so
/// the PTY column count matches what is drawn at any font size or family.
fn cell_advance(ui: &egui::Ui, font: &FontId) -> f32 {
    let measured = ui.fonts_mut(|fonts| fonts.glyph_width(font, 'M'));
    if measured.is_finite() && measured > 1.0 {
        measured
    } else {
        font.size * CELL_WIDTH_FACTOR
    }
}

fn terminal_line(
    cells: &[lumen_terminal_core::Cell],
    font: &FontId,
    line_height: f32,
    cursor_style: &str,
    cursor_visible: bool,
    color_scheme: &str,
    colors: theme::Palette,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    for cell in cells {
        let foreground = color(cell.foreground, colors.text, color_scheme, colors);
        let background = color(cell.background, colors.bg, color_scheme, colors);
        let cursor = cell.cursor && cursor_visible;
        let block_cursor = cursor && cursor_style == "block";
        job.append(
            &cell.character.to_string(),
            0.0,
            TextFormat {
                font_id: font.clone(),
                line_height: Some(line_height),
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
    colors: theme::Palette,
) -> Color32 {
    use lumen_terminal_core::TerminalColor;
    match color {
        TerminalColor::Rgb(red, green, blue) => Color32::from_rgb(red, green, blue),
        TerminalColor::Indexed(index) => theme::terminal_indexed_color(index, color_scheme, colors),
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
                colors.bg
            }
        }
        TerminalColor::Named(index) => {
            theme::terminal_indexed_color(index as u8, color_scheme, colors)
        }
    }
}

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

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::app::LumenApp;

    const SCREEN_HEIGHT: f32 = 600.0;

    fn frame(app: &mut LumenApp, ctx: &egui::Context) -> egui::FullOutput {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, SCREEN_HEIGHT),
                )),
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.terminal_panel(ctx)),
        );
        // The real app applies the measured size to the PTY/emulator after drawing.
        app.resize_terminal(app.view.terminal.rows, app.view.terminal.columns);
        output
    }

    /// (top, bottom, text) of every terminal row drawn.
    fn rows(output: &egui::FullOutput, marker: &str) -> Vec<(f32, f32, String)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.contains(marker) => Some((
                    text.pos.y,
                    text.pos.y + text.galley.size().y,
                    text.galley.job.text.trim().to_owned(),
                )),
                _ => None,
            })
            .collect()
    }

    fn app_with_screen(initial_rows: usize, text: &str) -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        app.view.terminal.open = true;
        let mut emulator = lumen_terminal_core::Terminal::new(initial_rows, 100);
        emulator.feed(text.as_bytes());
        app.view.terminal.emulator = Some(emulator);
        app.view.terminal.applied_rows = initial_rows as u16;
        app.view.terminal.applied_columns = 100;
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        (app, ctx)
    }

    #[test]
    fn pty_columns_follow_the_measured_cell_width_of_the_font() {
        let (mut app, ctx) = app_with_screen(24, "$ ");
        let mut columns = |size: f64| {
            app.view.settings.user.persisted.insert(
                "terminal.font_size".into(),
                lumen_settings_core::Value::Number(size),
            );
            for _ in 0..3 {
                frame(&mut app, &ctx);
            }
            app.view.terminal.columns
        };
        let small = columns(12.0);
        let large = columns(24.0);
        assert!(small > large, "{small} columns at 12px vs {large} at 24px");
        let ratio = f32::from(small) / f32::from(large);
        assert!(
            (1.7..2.3).contains(&ratio),
            "columns scale with font size: {ratio}"
        );
    }

    #[test]
    fn rows_are_exactly_line_height_apart_and_fit_the_measured_panel() {
        let (mut app, ctx) = app_with_screen(24, "alpha\r\nbeta\r\ngamma\r\n$ ");
        for _ in 0..3 {
            frame(&mut app, &ctx);
        }
        let output = frame(&mut app, &ctx);
        let line_height = app.view.settings.number("terminal.line_height", None);
        let drawn = rows(&output, "");
        let terminal_rows: Vec<_> = drawn
            .iter()
            .filter(|(_, _, text)| !text.is_empty() || true)
            .collect();
        // Pitch between consecutive terminal rows equals the configured height.
        let pitches: Vec<f32> = terminal_rows
            .windows(2)
            .map(|pair| pair[1].0 - pair[0].0)
            .filter(|pitch| *pitch > 0.0 && *pitch < line_height * 1.5)
            .collect();
        assert!(!pitches.is_empty());
        assert!(
            pitches
                .iter()
                .all(|pitch| (pitch - line_height).abs() < 0.01),
            "row pitch {pitches:?} vs {line_height}"
        );
        // Nothing is drawn below the window, and the live prompt is on screen.
        assert!(
            terminal_rows
                .iter()
                .all(|(_, bottom, _)| *bottom <= SCREEN_HEIGHT + 0.5)
        );
        let prompt = rows(&output, "$ ");
        assert!(!prompt.is_empty(), "prompt row is visible");
    }

    #[test]
    fn shrinking_a_tall_initial_terminal_keeps_the_prompt_visible_and_pinned() {
        // The shell starts with 24 rows, but the panel only fits about eleven.
        let lines: String = (1..=30).map(|n| format!("out{n}\r\n")).collect();
        let (mut app, ctx) = app_with_screen(24, &format!("{lines}$ "));
        for _ in 0..6 {
            frame(&mut app, &ctx);
        }
        let output = frame(&mut app, &ctx);
        assert!(
            app.view.terminal.rows < 24,
            "rows follow the panel: {}",
            app.view.terminal.rows
        );
        let prompt = rows(&output, "$ ");
        assert_eq!(prompt.len(), 1);
        let line_height = app.view.settings.number("terminal.line_height", None);
        // Pinned to the bottom edge of the panel, not scrolled out above it.
        assert!(
            SCREEN_HEIGHT - prompt[0].1 < line_height * 2.0 + 10.0,
            "prompt bottom {} too far above {SCREEN_HEIGHT}",
            prompt[0].1
        );
        assert!(
            prompt[0].0 > SCREEN_HEIGHT - 220.0,
            "prompt is inside the panel"
        );
    }

    #[test]
    fn a_short_session_shows_its_first_prompt_at_the_top_of_the_canvas() {
        let (mut app, ctx) = app_with_screen(24, "$ ");
        for _ in 0..4 {
            frame(&mut app, &ctx);
        }
        let output = frame(&mut app, &ctx);
        let prompt = rows(&output, "$ ");
        assert_eq!(prompt.len(), 1, "the initial prompt is not hidden");
        assert!(
            prompt[0].0 > SCREEN_HEIGHT - 220.0 + 30.0,
            "below the header"
        );
        assert!(prompt[0].1 < SCREEN_HEIGHT, "inside the panel");
    }

    #[test]
    fn dragging_from_a_panel_tab_over_the_terminal_selects_nothing() {
        let (mut app, ctx) = app_with_screen(11, "one\r\ntwo\r\n$ ");
        for _ in 0..3 {
            frame(&mut app, &ctx);
        }
        let output = frame(&mut app, &ctx);
        let tab = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "OUTPUT" => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                }
                _ => None,
            })
            .expect("output tab is drawn");
        let inside_terminal = egui::pos2(tab.x + 40.0, tab.y + 60.0);
        let run = |app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, SCREEN_HEIGHT),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.terminal_panel(ctx)),
            );
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(tab), button(tab, true)],
        );
        run(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(inside_terminal)],
        );
        run(&mut app, &ctx, vec![button(inside_terminal, false)]);
        run(&mut app, &ctx, vec![]);
        assert!(
            !ctx.plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection(),
            "tab clicks must not start a text selection"
        );
    }
}
