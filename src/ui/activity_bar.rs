use super::Workbench;
use super::widgets::{self, canvas, surface};
use crate::app::command::Command;
use crate::app::view_state::Activity;
use crate::icon;
use eframe::egui::{self, Stroke};
impl Workbench<'_> {
    pub(crate) fn activity_bar(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        egui::SidePanel::left("activity")
            .exact_width(52.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(canvas(ctx))
            .show(ctx, |ui| {
                surface(colors.sidebar).show(ui, |ui| {
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
                            let response = widgets::named(
                                widgets::icon_button(ui, glyph, selected)
                                    .on_hover_text(activity.title()),
                                true,
                                activity.title(),
                            );
                            if selected {
                                ui.painter().vline(
                                    response.rect.left(),
                                    response.rect.y_range(),
                                    Stroke::new(2.0_f32, colors.accent),
                                );
                            }
                            if response.clicked() {
                                self.command(Command::ActivateActivity(activity));
                            }
                        }
                        // Bottom-aligned group: Agent, Terminal, Output, Settings.
                        let agents_enabled = self.view.settings.boolean("agent.enabled");
                        let buttons = if agents_enabled { 4.0 } else { 3.0 };
                        ui.add_space((ui.available_height() - 50.0 * buttons).max(0.0));
                        for (glyph, selected, tooltip, command) in [
                            (
                                icon::AGENT,
                                self.view.agents.open,
                                format!("Coding Agents ({})", super::shortcut::primary("Shift+A")),
                                Command::ToggleAgentPanel,
                            ),
                            (
                                icon::TERMINAL,
                                self.view.terminal.open,
                                "Terminal (Ctrl+`); opens in your home folder when no folder is open".into(),
                                Command::ToggleTerminal,
                            ),
                            (
                                icon::OUTPUT,
                                self.view.output.open,
                                "Output".into(),
                                Command::ToggleOutput,
                            ),
                        ]
                        .into_iter()
                        .filter(|(glyph, ..)| agents_enabled || *glyph != icon::AGENT)
                        {
                            let waiting =
                                glyph == icon::AGENT && self.view.agents.needs_attention();
                            let name = if waiting {
                                format!("{tooltip} - waiting for your approval")
                            } else {
                                tooltip.clone()
                            };
                            let response = widgets::named(
                                widgets::icon_button(ui, glyph, selected).on_hover_text(&name),
                                true,
                                &name,
                            );
                            if waiting {
                                // An agent is blocked on a permission request.
                                ui.painter().circle_filled(
                                    response.rect.right_top() + egui::vec2(-12.0, 12.0),
                                    4.5,
                                    colors.warning,
                                );
                            }
                            if selected {
                                ui.painter().vline(
                                    response.rect.left(),
                                    response.rect.y_range(),
                                    Stroke::new(2.0_f32, colors.accent),
                                );
                            }
                            if response.clicked() {
                                self.command(command);
                            }
                        }
                        if widgets::named(
                            widgets::icon_button(ui, icon::GEAR, self.view.settings.open)
                                .on_hover_text(format!("Settings ({})", super::shortcut::primary(","))),
                            true,
                            "Settings",
                        )
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
            egui::Shape::LineSegment { stroke, .. } if stroke.color == crate::ui::theme::Theme::Dark.palette().accent && stroke.width == 2.0
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
    fn output_button_toggles_the_panel() {
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
    }

    #[test]
    fn terminal_button_works_without_a_folder_and_starts_in_the_home_directory() {
        let mut app = LumenApp::default();
        assert!(app.editor.workspace.is_none());
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        click(&mut app, &ctx, 900.0, icon::TERMINAL);
        assert!(app.view.terminal.open, "terminal opens with no folder");
        assert!(
            app.view.status.contains(
                &crate::services::terminal::default_working_directory()
                    .display()
                    .to_string()
            ) || app.view.terminal.error.is_some(),
            "started in the user directory: {}",
            app.view.status
        );
        app.close_terminal();
    }

    #[test]
    fn agent_button_toggles_the_panel_and_disappears_when_agents_are_disabled() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        click(&mut app, &ctx, 900.0, icon::AGENT);
        assert!(app.view.agents.open);
        click(&mut app, &ctx, 900.0, icon::AGENT);
        assert!(!app.view.agents.open);

        app.view.settings.user.persisted.insert(
            "agent.enabled".into(),
            lumen_settings_core::Value::Bool(false),
        );
        let output = frame(&mut app, &ctx, 900.0, vec![]);
        assert!(
            !output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.job.text == icon::AGENT)),
            "no agent button when disabled"
        );
        app.execute(Command::ToggleAgentPanel, &ctx);
        assert!(!app.view.agents.open);
        assert!(app.view.status.contains("turned off"));
    }

    #[test]
    fn a_pending_agent_permission_shows_a_badge_on_the_closed_agent_button() {
        use crate::app::{agent::AgentKind, agent_permission::PendingPermission};
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let warning = crate::ui::theme::Theme::Dark.palette().warning;
        let badges = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape,
                    egui::Shape::Circle(circle) if circle.fill == warning)
                })
                .count()
        };
        assert!(!app.view.agents.open);
        assert_eq!(badges(&frame(&mut app, &ctx, 900.0, vec![])), 0);
        let params = serde_json::json!({"sessionId":"s1",
            "toolCall":{"toolCallId":"t1","title":"Write","kind":"edit"},
            "options":[{"optionId":"o","name":"Yes","kind":"allow_once"}]});
        let request = lumen_agent_protocol::types::parse_permission_request(&params).unwrap();
        app.view
            .agents
            .session_mut(AgentKind::Claude)
            .pending_permissions
            .push(PendingPermission {
                id: lumen_agent_protocol::rpc::RequestId::Number(1),
                request,
                warnings: vec![],
            });
        assert_eq!(badges(&frame(&mut app, &ctx, 900.0, vec![])), 1);
        app.view
            .agents
            .session_mut(AgentKind::Claude)
            .pending_permissions
            .clear();
        assert_eq!(badges(&frame(&mut app, &ctx, 900.0, vec![])), 0);
    }
}
