//! The agent panel's chat controls row: model and effort selectors on the left,
//! configuration / History / New chat icon buttons anchored on the right. The
//! selectors truncate before the buttons ever move. Every other session option
//! lives in the configuration menu, generated from the session's typed options so
//! providers declare options without per-provider UI.
use super::{theme, widgets::named};
use crate::{
    app::{agent_panel::TabMode, command::Command},
    icon,
};
use eframe::egui::{self, Frame, Margin, RichText};
use lumen_agent_core::{
    ProviderId, SessionId,
    api::{ConfigKind, ConfigOption, SessionConfig},
};

const ROW_HEIGHT: f32 = 24.0;
const BUTTON_SIZE: f32 = 22.0;
const MODEL_OPTION: &str = "model";
const EFFORT_OPTION: &str = "effort";
/// Model space to reserve before offering effort alongside it.
const MIN_MODEL_WIDTH: f32 = 128.0;
const SELECTOR_GAP: f32 = 6.0;
const ACTION_GAP: f32 = 2.0;
const MENU_WIDTH: f32 = 200.0;

/// What the row acts on: the active provider and, when one exists, its session.
pub(super) struct ChatControls<'a> {
    pub(super) provider: ProviderId,
    pub(super) mode: TabMode,
    pub(super) session: Option<(SessionId, &'a SessionConfig)>,
}

/// Options shown in the row itself; all others go to the menu.
fn is_primary(option: &ConfigOption, inline_effort: bool) -> bool {
    option.kind == ConfigKind::Select
        && (option.id == MODEL_OPTION || (inline_effort && option.id == EFFORT_OPTION))
}

fn current_label(option: &ConfigOption) -> &str {
    option
        .choices
        .iter()
        .find(|choice| choice.value == option.current)
        .map_or(option.current.as_str(), |choice| choice.label.as_str())
}

/// Keep enough room for the longest effort choice and a useful model label.
/// Choice measurement makes the threshold stable as the selected effort changes.
fn effort_width(ui: &egui::Ui, config: &SessionConfig) -> Option<f32> {
    let effort = config
        .get(EFFORT_OPTION)
        .filter(|o| o.kind == ConfigKind::Select)?;
    let font = egui::FontId::proportional(theme::META_SIZE);
    let text_width = ui.fonts_mut(|fonts| {
        effort
            .choices
            .iter()
            .map(|choice| choice.label.as_str())
            .chain(std::iter::once(current_label(effort)))
            .map(|label| {
                fonts
                    .layout_no_wrap(label.into(), font.clone(), egui::Color32::PLACEHOLDER)
                    .size()
                    .x
            })
            .fold(0.0_f32, f32::max)
    });
    Some(
        text_width
            + 2.0 * ui.spacing().button_padding.x
            + ui.spacing().icon_width
            + ui.spacing().icon_spacing,
    )
}

fn at_default(option: &ConfigOption) -> bool {
    option.default.as_ref() == Some(&option.current)
}

/// Compact 22px icon button; `selected` paints the active state (History on).
fn icon_button(
    ui: &mut egui::Ui,
    glyph: &str,
    tooltip: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    let colors = theme::palette(ui.visuals());
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(BUTTON_SIZE, BUTTON_SIZE),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            colors.active
        } else if response.hovered() && enabled {
            colors.hover
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 3.0, fill);
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                3.0,
                egui::Stroke::new(1.0_f32, colors.accent),
                egui::StrokeKind::Inside,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(13.0),
            if !enabled {
                colors.muted.gamma_multiply(0.5)
            } else if selected {
                colors.text
            } else {
                colors.muted
            },
        );
    }
    let response = named(response, enabled, tooltip);
    response.on_hover_text(tooltip)
}

impl ChatControls<'_> {
    pub(super) fn show(&self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        let colors = theme::palette(ui.visuals());
        Frame::default()
            .fill(colors.panel)
            .inner_margin(Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Reserve the gear even when effort is inline, so resizing never
                // moves History or New chat, or leaves a hidden effort unreachable.
                let has_menu = self.session.is_some_and(|(_, config)| {
                    config
                        .options
                        .iter()
                        .any(|option| !is_primary(option, false))
                });
                let action_count = if has_menu { 3.0 } else { 2.0 };
                let selector_room =
                    ui.available_width() - action_count * (BUTTON_SIZE + ACTION_GAP);
                let effort_width = self
                    .session
                    .and_then(|(_, config)| effort_width(ui, config));
                let inline_effort = self.session.is_none_or(|(_, config)| {
                    config
                        .get(MODEL_OPTION)
                        .is_none_or(|o| o.kind != ConfigKind::Select)
                        || effort_width.is_none_or(|width| {
                            selector_room >= MIN_MODEL_WIDTH + SELECTOR_GAP + width
                        })
                });
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = ACTION_GAP;
                        // Buttons first, so the selectors only get what is left.
                        let new_chat = icon_button(
                            ui,
                            icon::PLUS,
                            &format!("New {} chat in this folder", self.provider.label()),
                            false,
                            true,
                        );
                        if new_chat.clicked() {
                            commands.push(Command::NativeNewSession(self.provider));
                        }
                        let history = icon_button(
                            ui,
                            icon::HISTORY,
                            if self.mode == TabMode::History {
                                "Back to the chat"
                            } else {
                                "Show previous chats"
                            },
                            self.mode == TabMode::History,
                            true,
                        );
                        if history.clicked() {
                            commands.push(Command::AgentToggleHistory(self.provider));
                        }
                        if let Some((id, config)) = self.session
                            && has_menu
                        {
                            let tooltip = if !inline_effort {
                                config
                                    .get(EFFORT_OPTION)
                                    .map(|effort| {
                                        format!(
                                            "Chat configuration\n{}: {}",
                                            effort.label,
                                            current_label(effort)
                                        )
                                    })
                                    .unwrap_or_else(|| "Chat configuration".into())
                            } else {
                                "Chat configuration".into()
                            };
                            let button = icon_button(
                                ui,
                                icon::GEAR,
                                &tooltip,
                                false,
                                self.mode == TabMode::Chat,
                            );
                            if self.mode == TabMode::Chat {
                                config_menu(&button, id, config, inline_effort, commands);
                            }
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = SELECTOR_GAP;
                            match (self.mode, self.session) {
                                (TabMode::Chat, Some((id, config))) => {
                                    selectors(ui, id, config, inline_effort, commands);
                                }
                                (TabMode::History, _) => {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new("Previous chats")
                                                .size(theme::META_SIZE)
                                                .color(colors.muted),
                                        )
                                        .truncate(),
                                    );
                                }
                                (TabMode::Chat, None) => {}
                            }
                        });
                    },
                );
            });
    }
}

/// Model and effort as compact drop-downs sharing the room left of the buttons.
/// A provider without one of them simply has no selector for it.
fn selectors(
    ui: &mut egui::Ui,
    id: SessionId,
    config: &SessionConfig,
    inline_effort: bool,
    commands: &mut Vec<Command>,
) {
    let options: Vec<&ConfigOption> = [MODEL_OPTION, EFFORT_OPTION]
        .into_iter()
        .filter_map(|wanted| config.get(wanted).filter(|o| is_primary(o, inline_effort)))
        .collect();
    let room =
        (ui.available_width() - SELECTOR_GAP * options.len().saturating_sub(1) as f32).max(0.0);
    let effort = effort_width(ui, config).unwrap_or(0.0).min(room);
    let both = options.len() == 2;
    for option in options {
        let width = if !both {
            room
        } else if option.id == EFFORT_OPTION {
            effort
        } else {
            room - effort
        };
        select_option(ui, id, option, width, commands);
    }
}

fn select_option(
    ui: &mut egui::Ui,
    id: SessionId,
    option: &ConfigOption,
    width: f32,
    commands: &mut Vec<Command>,
) {
    let enabled = option.disabled.is_none();
    let current = current_label(option);
    let mut hover = format!("{}: {}\n{}", option.label, current, option.description);
    if let Some(reason) = &option.disabled {
        hover.push_str(&format!("\nUnavailable: {reason}"));
    }
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(("chat_option", id.0, &option.id))
            .width(width)
            .truncate()
            .selected_text(RichText::new(current).size(theme::META_SIZE))
            .show_ui(ui, |ui| {
                for choice in &option.choices {
                    let selected = choice.value == option.current;
                    let response = ui.selectable_label(selected, &choice.label);
                    let response = if choice.description.is_empty() {
                        response
                    } else {
                        response.on_hover_text(&choice.description)
                    };
                    if response.clicked() && !selected {
                        commands.push(Command::ChatSetConfig(
                            id,
                            option.id.clone(),
                            choice.value.clone(),
                        ));
                    }
                }
            })
            .response
            .on_hover_text(&hover)
            .on_disabled_hover_text(&hover)
    });
}

/// Dropdown anchored under the configuration button: one submenu per select
/// option (current value checked), a toggle row per boolean, then a reset.
/// Choices apply immediately; values off their default carry a marker.
fn config_menu(
    button: &egui::Response,
    id: SessionId,
    config: &SessionConfig,
    inline_effort: bool,
    commands: &mut Vec<Command>,
) {
    let colors = theme::palette(&button.ctx.style().visuals);
    egui::Popup::menu(button)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_min_width(MENU_WIDTH);
            ui.set_max_width(MENU_WIDTH.max(ui.available_width().min(320.0)));
            for option in config
                .options
                .iter()
                .filter(|o| !is_primary(o, inline_effort))
            {
                let enabled = option.disabled.is_none();
                let hover = match &option.disabled {
                    Some(reason) => format!("{}\nUnavailable: {reason}", option.description),
                    None => option.description.clone(),
                };
                let changed = option.default.is_some() && !at_default(option);
                match option.kind {
                    ConfigKind::Toggle => {
                        let mut on = option.is_on();
                        let label = marked_label(&option.label, changed, colors.accent);
                        let response = ui
                            .add_enabled(enabled, egui::Checkbox::new(&mut on, label))
                            .on_hover_text(&hover)
                            .on_disabled_hover_text(&hover);
                        if response.changed() {
                            commands.push(Command::ChatSetConfig(
                                id,
                                option.id.clone(),
                                on.to_string(),
                            ));
                        }
                    }
                    ConfigKind::Select => {
                        let current = option
                            .choices
                            .iter()
                            .find(|c| c.value == option.current)
                            .map_or(option.current.as_str(), |c| c.label.as_str());
                        let title = format!("{}: {current}", option.label);
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.menu_button(marked_label(&title, changed, colors.accent), |ui| {
                                for choice in &option.choices {
                                    let selected = choice.value == option.current;
                                    let text = if selected {
                                        format!("{}  {}", icon::CHECK, choice.label)
                                    } else {
                                        format!("     {}", choice.label)
                                    };
                                    let response = ui.button(text);
                                    let response = if choice.description.is_empty() {
                                        response
                                    } else {
                                        response.on_hover_text(&choice.description)
                                    };
                                    if response.clicked() {
                                        if !selected {
                                            commands.push(Command::ChatSetConfig(
                                                id,
                                                option.id.clone(),
                                                choice.value.clone(),
                                            ));
                                        }
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text(&hover)
                        })
                        .response
                        .on_disabled_hover_text(&hover);
                    }
                }
            }
            ui.separator();
            let resettable: Vec<&ConfigOption> = config
                .options
                .iter()
                .filter(|o| o.disabled.is_none() && o.default.is_some() && !at_default(o))
                .collect();
            if ui
                .add_enabled(
                    !resettable.is_empty(),
                    egui::Button::new("Reset to defaults"),
                )
                .on_hover_text("Return every option, including model and effort, to its default")
                .on_disabled_hover_text("Everything is already at its default")
                .clicked()
            {
                for option in resettable {
                    commands.push(Command::ChatResetConfig(id, option.id.clone()));
                }
                ui.close();
            }
        });
}

fn marked_label(text: &str, changed: bool, accent: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let size = theme::META_SIZE + 1.0;
    let font = egui::FontId::proportional(size);
    job.append(
        text,
        0.0,
        egui::TextFormat::simple(font.clone(), egui::Color32::PLACEHOLDER),
    );
    if changed {
        job.append(
            "  ●",
            0.0,
            egui::TextFormat::simple(egui::FontId::proportional(8.0), accent),
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{ConfigRevision, api::ConfigChoice};

    fn config() -> SessionConfig {
        SessionConfig {
            revision: ConfigRevision(1),
            options: vec![
                ConfigOption::select(
                    "model",
                    "Model",
                    "Choose the model",
                    vec![ConfigChoice::new("model", "A long model name for coding")],
                    "model",
                ),
                ConfigOption::select(
                    "effort",
                    "Effort",
                    "Reasoning effort",
                    vec![
                        ConfigChoice::new("high", "High"),
                        ConfigChoice::new("extra", "Extra high"),
                    ],
                    "high",
                ),
            ],
        }
    }

    fn frame(
        ctx: &egui::Context,
        config: &SessionConfig,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<Command>) {
        let mut commands = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 400.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        ChatControls {
                            provider: ProviderId::Codex,
                            mode: TabMode::Chat,
                            session: Some((SessionId(7), config)),
                        }
                        .show(ui, &mut commands);
                    });
            },
        );
        (output, commands)
    }

    fn text_rect(output: &egui::FullOutput, needle: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == needle => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
    }

    fn click(
        ctx: &egui::Context,
        config: &SessionConfig,
        width: f32,
        pos: egui::Pos2,
    ) -> (egui::FullOutput, Vec<Command>) {
        frame(ctx, config, width, vec![egui::Event::PointerMoved(pos)]);
        frame(
            ctx,
            config,
            width,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            ctx,
            config,
            width,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
    }

    #[test]
    fn resizing_preserves_values_and_right_anchored_actions_and_relocates_effort() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        let config = config();
        let original = config.clone();
        let mut gaps = Vec::new();
        for width in [240.0, 300.0, 600.0, 240.0] {
            frame(&ctx, &config, width, vec![]);
            let (output, commands) = frame(&ctx, &config, width, vec![]);
            assert!(commands.is_empty());
            assert_eq!(config, original);
            let model = text_rect(&output, "A long model name for coding").unwrap();
            let gear = text_rect(&output, icon::GEAR).unwrap();
            let history = text_rect(&output, icon::HISTORY).unwrap();
            let new = text_rect(&output, icon::PLUS).unwrap();
            assert!(
                model.right() < gear.left(),
                "model overlaps actions at {width}"
            );
            assert!(gear.right() < history.left() && history.right() < new.left());
            assert!(new.right() <= width);
            gaps.push(width - new.right());
            assert_eq!(
                text_rect(&output, "High").is_some(),
                width == 600.0,
                "effort at {width}"
            );
        }
        assert!(gaps.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.5));
    }

    #[test]
    fn compact_configuration_exposes_effort_and_dispatches_its_session_and_value() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        let config = config();
        frame(&ctx, &config, 240.0, vec![]);
        let (output, _) = frame(&ctx, &config, 240.0, vec![]);
        click(
            &ctx,
            &config,
            240.0,
            text_rect(&output, icon::GEAR).unwrap().center(),
        );
        let (output, _) = frame(&ctx, &config, 240.0, vec![]);
        let effort = text_rect(&output, "Effort: High").expect("effort moved into menu");
        click(&ctx, &config, 240.0, effort.center());
        let (output, _) = frame(&ctx, &config, 240.0, vec![]);
        let choice = text_rect(&output, "     Extra high").expect("effort choices reachable");
        let (_, commands) = click(&ctx, &config, 240.0, choice.center());
        assert!(
            matches!(commands.as_slice(), [Command::ChatSetConfig(SessionId(7), name, value)] if name == "effort" && value == "extra")
        );
    }

    #[test]
    fn disabled_effort_stays_disabled_in_the_compact_menu() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        let mut config = config();
        config.options[1].disabled = Some("The provider is reconnecting".into());
        frame(&ctx, &config, 240.0, vec![]);
        let (output, _) = frame(&ctx, &config, 240.0, vec![]);
        click(
            &ctx,
            &config,
            240.0,
            text_rect(&output, icon::GEAR).unwrap().center(),
        );
        let (output, _) = frame(&ctx, &config, 240.0, vec![]);
        let effort = text_rect(&output, "Effort: High").unwrap();
        let (_, commands) = click(&ctx, &config, 240.0, effort.center());
        assert!(commands.is_empty());
        let (output, _) = frame(&ctx, &config, 240.0, vec![]);
        assert!(text_rect(&output, "     Extra high").is_none());
        assert_eq!(config.get("effort").unwrap().current, "high");
    }
}
