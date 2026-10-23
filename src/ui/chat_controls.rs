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
/// Narrowest a selector may get before it stops shrinking.
const MIN_SELECT_WIDTH: f32 = 48.0;
const MENU_WIDTH: f32 = 200.0;

/// What the row acts on: the active provider and, when one exists, its session.
pub(super) struct ChatControls<'a> {
    pub(super) provider: ProviderId,
    pub(super) mode: TabMode,
    pub(super) session: Option<(SessionId, &'a SessionConfig)>,
}

/// Options shown in the row itself; all others go to the menu.
fn is_primary(option: &ConfigOption) -> bool {
    option.id == MODEL_OPTION || option.id == EFFORT_OPTION
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
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
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
                            && config.options.iter().any(|o| !is_primary(o))
                        {
                            let button = icon_button(
                                ui,
                                icon::GEAR,
                                "Chat configuration",
                                false,
                                self.mode == TabMode::Chat,
                            );
                            if self.mode == TabMode::Chat {
                                config_menu(&button, id, config, commands);
                            }
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            match (self.mode, self.session) {
                                (TabMode::Chat, Some((id, config))) => {
                                    selectors(ui, id, config, commands);
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
    commands: &mut Vec<Command>,
) {
    let options: Vec<&ConfigOption> = [MODEL_OPTION, EFFORT_OPTION]
        .into_iter()
        .filter_map(|wanted| {
            config
                .options
                .iter()
                .find(|o| o.id == wanted && o.kind == ConfigKind::Select)
        })
        .collect();
    if options.is_empty() {
        return;
    }
    let spacing = ui.spacing().item_spacing.x;
    let room = (ui.available_width() - spacing * (options.len() as f32 - 1.0)).max(0.0);
    // The model name is the long one; effort values are short words.
    let shares: &[f32] = if options.len() == 2 {
        &[0.62, 0.38]
    } else {
        &[1.0]
    };
    for (option, share) in options.into_iter().zip(shares) {
        let width = (room * share).max(MIN_SELECT_WIDTH.min(room));
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
    let current = option
        .choices
        .iter()
        .find(|c| c.value == option.current)
        .map_or(option.current.as_str(), |c| c.label.as_str());
    let hover = match &option.disabled {
        Some(reason) => format!("{}\nUnavailable: {reason}", option.description),
        None => option.description.clone(),
    };
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
    commands: &mut Vec<Command>,
) {
    let colors = theme::palette(&button.ctx.style().visuals);
    egui::Popup::menu(button)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_min_width(MENU_WIDTH);
            ui.set_max_width(MENU_WIDTH.max(ui.available_width().min(320.0)));
            for option in config.options.iter().filter(|o| !is_primary(o)) {
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
