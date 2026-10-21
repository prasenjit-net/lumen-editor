//! The right-hand agent panel: one list of sessions from every
//! provider, subtly colour-coded, that opens chats as editor tabs. It starts and
//! lists conversations; the conversation itself lives in the chat tab.
use super::{
    Workbench,
    chat::{provider_color, status_text},
    theme,
    widgets::{HeaderAction, panel_header, surface},
};
use crate::{
    app::{agent_hub::SessionStatus, command::Command},
    icon,
};
use eframe::egui::{self, Frame, Margin, RichText, ScrollArea, Sense, SidePanel, Stroke};
use lumen_agent_core::ProviderId;

const DEFAULT_WIDTH: f32 = 300.0;
const MIN_WIDTH: f32 = 240.0;
const MAX_WIDTH: f32 = 600.0;
const ROW_HEIGHT: f32 = 40.0;
/// Width of the provider colour bar on a row's left edge.
const ACCENT_BAR: f32 = 3.0;

/// "5m", "3h", "2d": compact age for the second line of a row.
pub(super) fn age_text(now: u64, then: u64) -> String {
    if then == 0 || then > now {
        return String::new();
    }
    let seconds = now - then;
    match seconds {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

impl Workbench<'_> {
    pub(crate) fn agent_panel(&mut self, ctx: &egui::Context) {
        if !self.view.agents.open
            || !self.view.settings.boolean("agent.enabled")
            || self.view.native_agents.is_none()
        {
            return;
        }
        let colors = theme::palette(&ctx.style().visuals);
        let mut commands: Vec<Command> = Vec::new();
        SidePanel::right("agent_panel")
            .default_width(DEFAULT_WIDTH)
            .width_range(MIN_WIDTH..=MAX_WIDTH)
            .resizable(true)
            .frame(super::widgets::canvas(ctx))
            .show(ctx, |ui| {
                surface(colors.panel).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let actions = [HeaderAction {
                        glyph: icon::X,
                        tooltip: "Close agent panel",
                        command: Command::ToggleAgentPanel,
                        enabled: true,
                    }];
                    if let Some(command) = panel_header(ui, "AGENT SESSIONS", &actions) {
                        commands.push(command);
                    }
                    Frame::default()
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for provider in ProviderId::ALL {
                                    let label = RichText::new(format!(
                                        "{} {}",
                                        icon::PLUS,
                                        provider.label()
                                    ))
                                    .size(11.5)
                                    .color(provider_color(&colors, provider));
                                    if ui
                                        .button(label)
                                        .on_hover_text(format!(
                                            "New {} chat in this folder",
                                            provider.label()
                                        ))
                                        .clicked()
                                    {
                                        commands.push(Command::NativeNewSession(provider));
                                    }
                                }
                            });
                        });
                    self.session_rows(ui, &mut commands);
                });
            });
        for command in commands {
            self.command(command);
        }
    }

    fn session_rows(&mut self, ui: &mut egui::Ui, commands: &mut Vec<Command>) {
        let colors = theme::palette(ui.visuals());
        let Some(hub) = self.view.native_agents.as_ref() else {
            return;
        };
        let workspace = self.editor.workspace.clone();
        let canonical = workspace
            .as_ref()
            .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()));
        let rows = hub.registry.listing(canonical.as_deref(), false);
        if rows.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No chats in this folder yet.")
                        .size(11.5)
                        .color(colors.muted),
                );
            });
            return;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let open_chat = self.view.chats.active();
        ScrollArea::vertical()
            .id_salt("session_rows")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for record in &rows[range] {
                    let status = hub.status(record.id);
                    let accent = provider_color(&colors, record.provider);
                    let selected = open_chat == Some(record.id);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    // Custom-painted rows still need a spoken name and state, and a
                    // keyboard focus ring that is not the selection fill.
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            true,
                            selected,
                            format!("{}, {}", record.title, status_text(&status)),
                        )
                    });
                    if ui.is_rect_visible(rect) {
                        let fill = if selected {
                            colors.active
                        } else if response.hovered() {
                            colors.hover
                        } else {
                            egui::Color32::TRANSPARENT
                        };
                        ui.painter().rect_filled(rect, 3.0, fill);
                        if response.has_focus() {
                            ui.painter().rect_stroke(
                                rect.shrink(1.0),
                                3.0,
                                Stroke::new(1.0_f32, colors.accent),
                                egui::StrokeKind::Inside,
                            );
                        }
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.min + egui::vec2(2.0, 4.0),
                                egui::vec2(ACCENT_BAR, rect.height() - 8.0),
                            ),
                            1.5,
                            accent,
                        );
                        let status_color = match &status {
                            SessionStatus::Failed(_) => colors.error,
                            SessionStatus::NeedsAttention => colors.warning,
                            SessionStatus::Working => colors.accent,
                            _ => colors.muted,
                        };
                        let left = rect.left() + 12.0;
                        let pin = if record.pinned {
                            format!("{} ", icon::CHECK)
                        } else {
                            String::new()
                        };
                        let title = ui.painter().layout_no_wrap(
                            format!("{pin}{}", record.title),
                            egui::FontId::proportional(12.5),
                            colors.text,
                        );
                        let age = age_text(now, record.updated);
                        let age_galley = ui.painter().layout_no_wrap(
                            age,
                            egui::FontId::proportional(10.5),
                            colors.muted,
                        );
                        let title_max = rect.width() - 12.0 - age_galley.size().x - 14.0;
                        ui.painter()
                            .with_clip_rect(egui::Rect::from_min_size(
                                egui::pos2(left, rect.top()),
                                egui::vec2(title_max.max(0.0), 20.0),
                            ))
                            .galley(egui::pos2(left, rect.top() + 4.0), title, colors.text);
                        ui.painter().galley(
                            egui::pos2(rect.right() - age_galley.size().x - 8.0, rect.top() + 5.0),
                            age_galley,
                            colors.muted,
                        );
                        let second = ui.painter().layout_no_wrap(
                            format!("{} · {}", record.provider.label(), status_text(&status)),
                            egui::FontId::proportional(10.5),
                            status_color,
                        );
                        ui.painter()
                            .with_clip_rect(egui::Rect::from_min_size(
                                egui::pos2(left, rect.top() + 20.0),
                                egui::vec2((rect.width() - 20.0).max(0.0), 18.0),
                            ))
                            .galley(egui::pos2(left, rect.top() + 22.0), second, status_color);
                    }
                    let tooltip = format!("{}\n{}", record.title, record.workspace.display());
                    let response = response.on_hover_text(tooltip);
                    if response.clicked() {
                        commands.push(Command::OpenChat(record.id));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            commands.push(Command::OpenChat(record.id));
                            ui.close();
                        }
                        let pin_label = if record.pinned { "Unpin" } else { "Pin" };
                        if ui.button(pin_label).clicked() {
                            commands.push(Command::PinChat(record.id, !record.pinned));
                            ui.close();
                        }
                        let can_archive = status == SessionStatus::Idle
                            || matches!(status, SessionStatus::Failed(_));
                        if ui
                            .add_enabled(can_archive, egui::Button::new("Archive"))
                            .on_disabled_hover_text("Stop the running turn first")
                            .clicked()
                        {
                            commands.push(Command::ArchiveChat(record.id));
                            ui.close();
                        }
                    });
                    let _ = Stroke::NONE;
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::age_text;

    #[test]
    fn ages_are_compact_and_blank_when_unknown() {
        assert_eq!(age_text(1000, 0), "");
        assert_eq!(age_text(1000, 2000), "");
        assert_eq!(age_text(1000, 990), "now");
        assert_eq!(age_text(10_000, 10_000 - 300), "5m");
        assert_eq!(age_text(100_000, 100_000 - 7_300), "2h");
        assert_eq!(age_text(1_000_000, 1_000_000 - 200_000), "2d");
    }
}
