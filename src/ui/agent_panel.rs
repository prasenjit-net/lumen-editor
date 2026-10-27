//! The right-hand agent panel: one tab per provider, each showing that provider's
//! current chat or, behind the History button, its previous sessions. Chats are
//! not editor tabs; this panel is the only chat surface.
use super::{
    Workbench,
    chat::{provider_color, status_text},
    chat_controls::ChatControls,
    theme,
    widgets::{HeaderAction, PanelTab, surface, tabbed_panel_header},
};
use crate::{
    app::{
        agent_hub::SessionStatus,
        agent_panel::{HistoryGroup, RenameDraft, TabMode, history_group, title_matches},
        command::Command,
        native_agents::chat_workspace_for,
    },
    icon,
    model::agent_registry::SessionRecord,
};
use eframe::egui::{self, Key, RichText, ScrollArea, Sense, SidePanel, Stroke};
use lumen_agent_core::{ProviderId, SessionId};

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
        let provider = self.view.agent_panel.active_provider();
        let workspace = chat_workspace_for(self.editor.workspace.as_deref());
        // Every tab keeps a valid current session, so switching never lands on a stale one.
        for known in ProviderId::ALL {
            self.view.resolve_agent_session(known, &workspace);
        }
        let mode = self.view.agent_panel.mode(provider);
        let activity: Vec<_> = ProviderId::ALL
            .iter()
            .map(|known| {
                self.view
                    .native_agents
                    .as_ref()
                    .and_then(|hub| hub.provider_activity(*known))
                    .map(|(record, status)| {
                        let tooltip = format!(
                            "{} — {}\n{}",
                            status_text(&status),
                            record.title,
                            record.workspace.display()
                        );
                        let indicator = match status {
                            SessionStatus::NeedsAttention => (icon::WARNING, colors.warning),
                            SessionStatus::Failed(_) => (icon::ERROR, colors.error),
                            SessionStatus::Working => (icon::REFRESH, colors.accent),
                            SessionStatus::Waiting(_) => (icon::HISTORY, colors.muted),
                            SessionStatus::Idle => ("", colors.muted),
                        };
                        (indicator, tooltip)
                    })
                    .unwrap_or_else(|| (("", colors.muted), "No active chats".into()))
            })
            .collect();
        SidePanel::right("agent_panel")
            .default_width(DEFAULT_WIDTH)
            .width_range(MIN_WIDTH..=MAX_WIDTH)
            .resizable(true)
            .frame(super::widgets::canvas(ctx))
            .show(ctx, |ui| {
                surface(colors.panel).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let tabs: Vec<PanelTab> = ProviderId::ALL
                        .iter()
                        .zip(&activity)
                        .map(|(known, (indicator, tooltip))| PanelTab {
                            label: known.label(),
                            indicator: Some(*indicator),
                            active: *known == provider,
                            command: Command::AgentSelectProvider(*known),
                            tooltip,
                        })
                        .collect();
                    let actions = [HeaderAction {
                        glyph: icon::X,
                        tooltip: "Close agent panel",
                        command: Command::ToggleAgentPanel,
                        enabled: true,
                    }];
                    if let Some(command) = tabbed_panel_header(ui, &tabs, &actions) {
                        commands.push(command);
                    }
                    let current = self.view.agent_panel.current(provider);
                    let config = current.and_then(|id| {
                        self.view
                            .native_agents
                            .as_ref()
                            .and_then(|hub| hub.live(id))
                            .and_then(|live| live.model.config.as_ref())
                            .map(|config| (id, config))
                    });
                    // The body is laid out in a detached child that is clipped to the
                    // panel, so wide content (long lines, tool output) wraps or clips
                    // instead of widening the panel. Only dragging the edge resizes it.
                    let rect = ui.available_rect_before_wrap();
                    let mut body = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    body.set_clip_rect(rect.intersect(ui.clip_rect()));
                    let ui_body = &mut body;
                    ChatControls {
                        provider,
                        mode,
                        session: config,
                    }
                    .show(ui_body, &mut commands);
                    let missing = self.view.agent_panel.missing(provider).map(str::to_owned);
                    match mode {
                        TabMode::Chat if missing.is_some() => super::agent_install::not_installed(
                            ui_body,
                            provider,
                            missing.as_deref().unwrap_or_default(),
                            &colors,
                            &mut commands,
                        ),
                        TabMode::Chat => match current {
                            Some(id) => self.chat_body(ui_body, id),
                            None => start_chat(ui_body, provider, &colors, &mut commands),
                        },
                        TabMode::History => {
                            self.history_list(ui_body, provider, &workspace, &mut commands)
                        }
                    }
                    ui.allocate_rect(rect, Sense::hover());
                });
            });
        for command in commands {
            self.command(command);
        }
    }

    fn history_list(
        &mut self,
        ui: &mut egui::Ui,
        provider: ProviderId,
        workspace: &std::path::Path,
        commands: &mut Vec<Command>,
    ) {
        let colors = theme::palette(ui.visuals());
        // Escape returns to the chat unless a rename is capturing it.
        let searching = !self.view.agent_panel.history_query_mut().is_empty();
        if self.view.agent_panel.renaming().is_none()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            if searching {
                self.view.agent_panel.history_query_mut().clear();
            } else {
                commands.push(Command::AgentBackToChat(provider));
            }
        }
        if ui
            .add(
                egui::Button::new(RichText::new("Back to chat").size(theme::META_SIZE))
                    .frame(false),
            )
            .on_hover_text("Return to the chat without choosing another session (Esc)")
            .clicked()
        {
            commands.push(Command::AgentBackToChat(provider));
        }
        let Some(hub) = self.view.native_agents.as_ref() else {
            return;
        };
        let rows: Vec<_> = hub
            .registry
            .listing(Some(workspace), false)
            .into_iter()
            .filter(|record| record.provider == provider)
            .collect();
        if rows.is_empty() {
            self.view.agent_panel.history_query_mut().clear();
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!("No {} chats in this folder yet.", provider.label()))
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
        history_search(ui, self.view.agent_panel.history_query_mut(), &colors);
        let query = self.view.agent_panel.history_query_mut().clone();
        let total = rows.len();
        let entries = history_entries(rows, &query, now);
        if entries.is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(format!(
                        "No {} chats match “{}”.",
                        provider.label(),
                        query.trim()
                    ))
                    .size(11.5)
                    .color(colors.muted),
                );
                if ui
                    .button("Clear search")
                    .on_hover_text(format!("Show all {total} chats"))
                    .clicked()
                {
                    self.view.agent_panel.history_query_mut().clear();
                }
            });
            return;
        }
        let current = self.view.agent_panel.current(provider);
        let mut focus_first = self.view.agent_panel.take_history_focus();
        let mut renaming = self.view.agent_panel.take_rename();
        let mut begin_rename: Option<(SessionId, String)> = None;
        let mut finish_rename = false;
        ScrollArea::vertical()
            .id_salt("session_rows")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for entry in &entries {
                    let record = match entry {
                        HistoryEntry::Heading(group) => {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(group.label())
                                    .size(10.0)
                                    .strong()
                                    .color(colors.muted),
                            );
                            continue;
                        }
                        HistoryEntry::Session(record) => *record,
                    };
                    let status = hub.status(record.id);
                    let accent = provider_color(&colors, record.provider);
                    let selected = current == Some(record.id);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::click(),
                    );
                    if std::mem::take(&mut focus_first) {
                        response.request_focus();
                    }
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
                    let is_renaming = renaming.as_ref().is_some_and(|r| r.id == record.id);
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
                            if is_renaming {
                                String::new()
                            } else {
                                format!("{pin}{}", record.title)
                            },
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
                    if let Some(edit) = renaming.as_mut().filter(|r| r.id == record.id) {
                        let edit_rect = egui::Rect::from_min_size(
                            rect.min + egui::vec2(10.0, 2.0),
                            egui::vec2((rect.width() - 16.0).max(0.0), 20.0),
                        );
                        let field = ui.put(
                            edit_rect,
                            egui::TextEdit::singleline(&mut edit.text)
                                .font(egui::FontId::proportional(12.5))
                                .desired_width(f32::INFINITY),
                        );
                        if std::mem::take(&mut edit.focus) {
                            field.request_focus();
                        }
                        if field.lost_focus() {
                            let entered = ui.input(|i| i.key_pressed(Key::Enter));
                            if entered {
                                commands.push(Command::RenameChat(record.id, edit.text.clone()));
                            }
                            finish_rename = true;
                        }
                    }
                    let tooltip = format!("{}\n{}", record.title, record.workspace.display());
                    let response = response.on_hover_text(tooltip);
                    if response.clicked() && renaming.as_ref().is_none_or(|r| r.id != record.id) {
                        commands.push(Command::OpenChat(record.id));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Open").clicked() {
                            commands.push(Command::OpenChat(record.id));
                            ui.close();
                        }
                        if ui.button("Rename").clicked() {
                            begin_rename = Some((record.id, record.title.clone()));
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
                }
            });
        if finish_rename {
            renaming = None;
        }
        if let Some((id, title)) = begin_rename {
            renaming = Some(RenameDraft::new(id, title));
        }
        self.view.agent_panel.set_rename(renaming);
    }
}

enum HistoryEntry<'a> {
    Heading(HistoryGroup),
    Session(&'a SessionRecord),
}

/// Filter by title, then lay the sessions out under recency headings. Sessions
/// keep the registry's order (pinned, then newest), so identity and order are
/// the same with or without a query.
fn history_entries<'a>(
    rows: Vec<&'a SessionRecord>,
    query: &str,
    now: u64,
) -> Vec<HistoryEntry<'a>> {
    let mut entries = Vec::new();
    let mut last = None;
    for record in rows
        .into_iter()
        .filter(|record| title_matches(&record.title, query))
    {
        let group = history_group(record.pinned, now, record.updated);
        if last != Some(group) {
            entries.push(HistoryEntry::Heading(group));
            last = Some(group);
        }
        entries.push(HistoryEntry::Session(record));
    }
    entries
}

/// The title filter. Escape clears it first; with nothing typed it leaves the
/// History to the caller's own Escape handling.
fn history_search(ui: &mut egui::Ui, query: &mut String, colors: &theme::Palette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let clear_width = if query.is_empty() { 0.0 } else { 20.0 };
        ui.add(
            egui::TextEdit::singleline(query)
                .id(egui::Id::new("history_search"))
                .hint_text(format!("{} Search chats", icon::MAGNIFYING_GLASS))
                .font(egui::FontId::proportional(theme::META_SIZE))
                .text_color(colors.text)
                .desired_width((ui.available_width() - clear_width - 4.0).max(40.0)),
        );
        if !query.is_empty()
            && ui
                .add(
                    egui::Button::new(RichText::new(icon::X).size(10.0).color(colors.muted))
                        .frame(false),
                )
                .on_hover_text("Clear search")
                .clicked()
        {
            query.clear();
        }
    });
    ui.add_space(4.0);
}

/// No chat yet for this provider: say what it can do and offer to start one.
fn start_chat(
    ui: &mut egui::Ui,
    provider: ProviderId,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) {
    ui.add_space(super::widgets::empty_state_top(ui).min(48.0));
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icon::AGENT)
                .size(26.0)
                .color(provider_color(colors, provider)),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Ask {} anything about this folder",
                provider.label()
            ))
            .size(13.0)
            .color(colors.text),
        );
        ui.label(
            RichText::new("It can read, edit and run commands here. You approve anything risky.")
                .size(11.5)
                .color(colors.muted),
        );
        ui.add_space(8.0);
        let button = egui::Button::new(
            RichText::new(format!("{}  Start chat", icon::PLUS))
                .size(12.0)
                .color(colors.on_accent),
        )
        .fill(colors.accent)
        .corner_radius(3);
        if ui
            .add(button)
            .on_hover_text(format!("New {} chat in this folder", provider.label()))
            .clicked()
        {
            commands.push(Command::NativeNewSession(provider));
        }
    });
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
