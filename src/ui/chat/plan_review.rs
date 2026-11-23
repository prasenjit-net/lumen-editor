//! A plan the agent proposes, docked above the message box so it can be read
//! before it is approved: the plan itself, optional feedback, and the
//! provider's choices (approve and accept edits, approve, keep planning).
use super::{docked, markdown_render};
use crate::{
    app::command::Command,
    icon,
    ui::{
        markdown_render::{Host, Metrics},
        theme, widgets,
    },
};
use eframe::egui::{self, RichText};
use lumen_agent_core::{ApprovalRequest, PlanReview, SessionId};

/// Header, feedback and buttons, kept outside the plan's scrolling.
const CHROME_HEIGHT: f32 = 132.0;

/// Draw the review of `plan`, the plan of approval `request`. `note` is the
/// feedback being written. Choosing pushes the approval reply.
#[allow(clippy::too_many_arguments)]
pub(super) fn plan_card(
    ui: &mut egui::Ui,
    session: SessionId,
    request: &ApprovalRequest,
    plan: &PlanReview,
    note: &mut String,
    max_height: f32,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) {
    let base = ui.id().with(("plan_card", &request.token));
    let collapsed_key = base.with("collapsed");
    let sent_key = base.with("sent");
    let mut collapsed = ui
        .data(|data| data.get_temp::<bool>(collapsed_key))
        .unwrap_or(false);
    let sent = ui
        .data(|data| data.get_temp::<bool>(sent_key))
        .unwrap_or(false);
    let mut chosen = None;
    docked::card(ui, colors.accent, colors, |ui| {
        if header(ui, session, plan, collapsed, colors, commands) {
            collapsed = !collapsed;
        }
        ui.add_enabled_ui(!sent, |ui| {
            if !collapsed {
                let limit = (max_height - CHROME_HEIGHT).max(80.0);
                docked::scrolling(ui, base.with("body"), limit, |ui| {
                    markdown_render::show(
                        ui,
                        plan.body(),
                        Metrics::CHAT,
                        Host::Chat {
                            session,
                            item: &request.token.native,
                        },
                        false,
                        commands,
                    );
                });
                ui.separator();
                ui.add(
                    egui::TextEdit::multiline(note)
                        .id(base.with("note"))
                        .hint_text("Feedback for the agent (optional)")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
            }
            chosen = choices(ui, request, note, sent, colors);
        });
    });
    ui.data_mut(|data| data.insert_temp(collapsed_key, collapsed));
    if let Some((decision, with_note)) = chosen {
        ui.data_mut(|data| data.insert_temp(sent_key, true));
        let note = with_note
            .then(|| note.trim().to_string())
            .filter(|n| !n.is_empty());
        commands.push(Command::ChatApproval(
            session,
            request.token.clone(),
            decision,
            note,
        ));
    }
}

/// The plan's name, then Open (when it has a file) and the collapse toggle at
/// the right. Returns whether the toggle was pressed.
fn header(
    ui: &mut egui::Ui,
    session: SessionId,
    plan: &PlanReview,
    collapsed: bool,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) -> bool {
    let mut toggled = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (glyph, name) = if collapsed {
                (icon::CARET_RIGHT, "Show the plan")
            } else {
                (icon::CARET_DOWN, "Hide the plan")
            };
            let toggle = widgets::tool_button(ui, false, 0.0, |ui| {
                ui.add(
                    egui::Button::new(RichText::new(glyph).size(12.0).color(colors.muted))
                        .frame(false)
                        .min_size(egui::vec2(20.0, 20.0)),
                )
            })
            .on_hover_text(name);
            toggled = widgets::named(toggle, true, name).clicked();
            if let Some(file) = &plan.file {
                let open = widgets::tool_button(ui, false, 0.0, |ui| {
                    ui.add(
                        egui::Button::new(RichText::new("Open").size(11.5).color(colors.text))
                            .frame(false)
                            .min_size(egui::vec2(0.0, 20.0)),
                    )
                })
                .on_hover_text(format!("Open the plan file: {}", file.display()));
                if open.clicked() {
                    commands.push(Command::OpenChatLink(session, file.display().to_string()));
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(icon::CHECKLIST)
                        .size(12.0)
                        .color(colors.accent),
                );
                let title = plan
                    .title()
                    .unwrap_or_else(|| "Plan ready for review".to_string());
                let font = egui::FontId::proportional(11.5);
                let fitted = widgets::ellipsize(ui, &title, &font, ui.available_width());
                ui.label(
                    RichText::new(fitted)
                        .font(font)
                        .strong()
                        .color(colors.accent),
                )
                .on_hover_text(title);
            });
        });
    });
    toggled
}

/// The provider's choices: approving ones filled, the refusal outlined. With
/// feedback written, the refusal sends it. Returns the choice and whether the
/// feedback goes with it.
fn choices(
    ui: &mut egui::Ui,
    request: &ApprovalRequest,
    note: &str,
    sent: bool,
    colors: &theme::Palette,
) -> Option<(lumen_agent_core::api::ApprovalDecisionId, bool)> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        if sent {
            ui.label(
                RichText::new("Sending…")
                    .size(theme::META_SIZE)
                    .color(colors.muted),
            );
            return;
        }
        let feedback = !note.trim().is_empty();
        for choice in &request.choices {
            let label = if !choice.allows && feedback {
                "Send feedback"
            } else {
                choice.label.as_str()
            };
            let text = RichText::new(label).size(11.5);
            let button = if choice.allows {
                egui::Button::new(text.color(colors.on_accent)).fill(colors.accent)
            } else {
                egui::Button::new(text.color(colors.text))
            };
            let hover = if !choice.allows && feedback {
                "Send your feedback; the plan stays unapproved and the agent keeps planning"
                    .to_string()
            } else {
                format!("Applies to: {}", choice.scope)
            };
            if ui.add(button).on_hover_text(hover).clicked() {
                chosen = Some((choice.id.clone(), !choice.allows));
            }
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{
        ApprovalChoice, ProfileId, ProviderId, RequestToken, RiskLevel, RuntimeGeneration, TurnId,
        api::ApprovalDecisionId,
    };

    fn request(sections: usize) -> ApprovalRequest {
        let choice = |id: &str, label: &str, allows: bool| ApprovalChoice {
            id: ApprovalDecisionId(id.into()),
            label: label.into(),
            scope: "the plan".into(),
            allows,
            risk: RiskLevel::Low,
        };
        let text = std::iter::once("# Plan: A scanner with a rather long name for its card".into())
            .chain((0..sections).map(|n| {
                format!("## Step {n}\nRead the build files and list every declared version.")
            }))
            .collect::<Vec<String>>()
            .join("\n\n");
        ApprovalRequest {
            token: RequestToken {
                provider: ProviderId::Claude,
                profile: ProfileId::default_profile(),
                session: SessionId(1),
                turn: TurnId("turn".into()),
                generation: RuntimeGeneration(1),
                native: "p".into(),
            },
            summary: "Plan ready for review".into(),
            command: None,
            paths: vec![],
            choices: vec![
                choice("accept", "Approve and accept edits", true),
                choice("approve", "Approve", true),
                choice("keep", "Keep planning", false),
            ],
            plan: Some(PlanReview {
                text,
                file: Some("/plans/p.md".into()),
            }),
        }
    }

    fn visible_text(width: f32, max_height: f32, sections: usize) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let request = request(sections);
        let plan = request.plan.clone().unwrap();
        let mut note = String::new();
        let mut output = None;
        for _ in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let colors = theme::palette(ui.visuals());
                        let mut commands = Vec::new();
                        plan_card(
                            ui,
                            SessionId(1),
                            &request,
                            &plan,
                            &mut note,
                            max_height,
                            &colors,
                            &mut commands,
                        );
                    });
                },
            ));
        }
        output
            .unwrap()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    shape
                        .clip_rect
                        .contains_rect(rect)
                        .then(|| (text.galley.job.text.clone(), rect))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_long_plan_scrolls_inside_a_card_that_keeps_its_height() {
        let texts = visible_text(480.0, 320.0, 20);
        let top = texts
            .iter()
            .map(|(_, r)| r.top())
            .fold(f32::INFINITY, f32::min);
        let keep = texts
            .iter()
            .find(|(text, _)| text == "Keep planning")
            .expect("the choices stay visible");
        assert!(keep.1.bottom() - top <= 320.0, "{}", keep.1.bottom() - top);
        assert!(texts.iter().any(|(text, _)| text == "Step 0"));
        assert!(
            !texts.iter().any(|(text, _)| text == "Step 19"),
            "later steps scroll"
        );
    }

    #[test]
    fn a_narrow_panel_wraps_the_plan_and_its_choices() {
        let texts = visible_text(260.0, 420.0, 2);
        for (text, rect) in &texts {
            assert!(rect.right() <= 260.0, "{text:?} ends at {}", rect.right());
        }
        for needle in ["Approve and accept edits", "Keep planning", "Open"] {
            assert!(texts.iter().any(|(text, _)| text == needle), "{needle}");
        }
    }
}
