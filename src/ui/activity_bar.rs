use super::Workbench;
use super::theme::{ACCENT, SIDEBAR};
use super::widgets::{self, canvas, surface};
use crate::app::command::Command;
use crate::app::view_state::Activity;
use crate::icon;
use eframe::egui::{self, Stroke};
impl Workbench<'_> {
    pub(crate) fn activity_bar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("activity")
            .exact_width(52.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(canvas())
            .show(ctx, |ui| {
                surface(SIDEBAR).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.vertical_centered(|ui| {
                        for (activity, glyph) in [
                            (Activity::Explorer, icon::FILES),
                            (Activity::Search, icon::MAGNIFYING_GLASS),
                            (Activity::SourceControl, icon::GIT_BRANCH),
                            (Activity::RunDebug, icon::DEBUG),
                            (Activity::Extensions, icon::PUZZLE_PIECE),
                        ] {
                            let selected =
                                self.view.sidebar_visible && self.view.activity == activity;
                            let response = widgets::icon_button(ui, glyph, selected)
                                .on_hover_text(activity.title());
                            if selected {
                                ui.painter().vline(
                                    response.rect.left(),
                                    response.rect.y_range(),
                                    Stroke::new(2.0_f32, ACCENT),
                                );
                            }
                            if response.clicked() {
                                self.command(Command::ActivateActivity(activity));
                            }
                        }
                        // Bottom-aligned group: Terminal, Output, Settings.
                        ui.add_space((ui.available_height() - 50.0 * 3.0).max(0.0));
                        let has_workspace = self.editor.workspace.is_some();
                        for (glyph, selected, enabled, tooltip, command) in [
                            (
                                icon::TERMINAL,
                                self.view.terminal.open,
                                has_workspace,
                                "Terminal (Ctrl+`)",
                                Command::ToggleTerminal,
                            ),
                            (
                                icon::OUTPUT,
                                self.view.output.open,
                                true,
                                "Output",
                                Command::ToggleOutput,
                            ),
                        ] {
                            let response = ui
                                .add_enabled_ui(enabled, |ui| {
                                    widgets::icon_button(ui, glyph, selected)
                                })
                                .inner
                                .on_hover_text(tooltip);
                            if selected {
                                ui.painter().vline(
                                    response.rect.left(),
                                    response.rect.y_range(),
                                    Stroke::new(2.0_f32, ACCENT),
                                );
                            }
                            if response.clicked() {
                                self.command(command);
                            }
                        }
                        if widgets::icon_button(ui, icon::GEAR, self.view.settings.open)
                            .on_hover_text("Settings (Cmd/Ctrl+,)")
                            .clicked()
                        {
                            self.command(Command::Settings);
                        }
                    });
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{LumenApp, view_state::ExplorerEdit};

    fn frame(
        app: &mut LumenApp,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.activity_bar(ctx)),
        )
    }

    fn click(app: &mut LumenApp, ctx: &egui::Context, width: f32, glyph: &str) {
        let output = frame(app, ctx, width, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == glyph => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                }
                _ => None,
            })
            .expect("activity icon is visible");
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                width,
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

    fn active_stripes(output: &egui::FullOutput) -> usize {
        output.shapes.iter().filter(|shape| matches!(&shape.shape,
            egui::Shape::LineSegment { stroke, .. } if stroke.color == ACCENT && stroke.width == 2.0
        )).count()
    }

    #[test]
    fn activity_clicks_toggle_and_switch_without_losing_work() {
        for width in [300.0, 900.0, 1400.0] {
            let mut app = LumenApp::default();
            app.editor.new_file();
            app.editor.files[0].text = "unsaved work".into();
            app.editor.files[0].dirty = true;
            let document_id = app.editor.files[0].id;
            app.view.explorer_edit = Some(ExplorerEdit::new("/workspace".into()));
            app.view.explorer_edit.as_mut().unwrap().name = "draft.rs".into();
            app.view.expanded_dirs.insert("/workspace".into());
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            frame(&mut app, &ctx, width, vec![]);
            for (activity, glyph) in [
                (Activity::Explorer, icon::FILES),
                (Activity::Search, icon::MAGNIFYING_GLASS),
                (Activity::SourceControl, icon::GIT_BRANCH),
                (Activity::RunDebug, icon::DEBUG),
                (Activity::Extensions, icon::PUZZLE_PIECE),
            ] {
                app.view.activity = Activity::Explorer;
                app.view.sidebar_visible = false;
                app.view.explorer_focused = true;
                click(&mut app, &ctx, width, glyph);
                assert_eq!(app.view.activity, activity);
                assert!(app.view.sidebar_visible);
                assert!(!app.view.explorer_focused);
                assert_eq!(active_stripes(&frame(&mut app, &ctx, width, vec![])), 1);
                click(&mut app, &ctx, width, glyph);
                assert!(!app.view.sidebar_visible);
                assert_eq!(active_stripes(&frame(&mut app, &ctx, width, vec![])), 0);
                click(&mut app, &ctx, width, glyph);
                assert!(app.view.sidebar_visible);
                let other = if activity == Activity::Explorer {
                    icon::MAGNIFYING_GLASS
                } else {
                    icon::FILES
                };
                click(&mut app, &ctx, width, other);
                assert!(app.view.sidebar_visible);
                assert_ne!(app.view.activity, activity);
            }
            assert_eq!(app.editor.files[0].id, document_id);
            assert_eq!(app.editor.files[0].text, "unsaved work");
            assert!(app.editor.files[0].dirty);
            assert_eq!(app.view.explorer_edit.as_ref().unwrap().name, "draft.rs");
            assert!(
                app.view
                    .expanded_dirs
                    .contains(std::path::Path::new("/workspace"))
            );
        }
    }

    fn glyph_y(output: &egui::FullOutput, glyph: &str) -> f32 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == glyph => Some(text.pos.y),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{glyph:?} is visible"))
    }

    #[test]
    fn terminal_and_output_buttons_sit_above_settings_at_the_bottom() {
        for height in [420.0_f32, 600.0, 900.0] {
            let mut app = LumenApp::default();
            app.editor.workspace = Some("/workspace".into());
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, height),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.activity_bar(ctx)),
            );
            let (terminal, log, gear) = (
                glyph_y(&output, icon::TERMINAL),
                glyph_y(&output, icon::OUTPUT),
                glyph_y(&output, icon::GEAR),
            );
            assert!(
                terminal < log && log < gear,
                "{height}: {terminal} {log} {gear}"
            );
            assert!(
                gear > height - 70.0,
                "settings stays bottom-aligned at {height}"
            );
            assert!(
                terminal > glyph_y(&output, icon::PUZZLE_PIECE),
                "below the activities"
            );
        }
    }

    #[test]
    fn output_button_toggles_the_panel_and_terminal_needs_a_folder() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let baseline = active_stripes(&frame(&mut app, &ctx, 900.0, vec![]));
        click(&mut app, &ctx, 900.0, icon::OUTPUT);
        assert!(app.view.output.open);
        assert_eq!(
            active_stripes(&frame(&mut app, &ctx, 900.0, vec![])),
            baseline + 1,
            "the open Output button is marked like an active activity"
        );
        click(&mut app, &ctx, 900.0, icon::OUTPUT);
        assert!(!app.view.output.open);

        // No folder open: the terminal button is disabled and does nothing.
        click(&mut app, &ctx, 900.0, icon::TERMINAL);
        assert!(!app.view.terminal.open);
    }
}
