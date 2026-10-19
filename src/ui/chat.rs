//! Agent chat tab: header, transcript, approvals, queue and composer for one
//! session. Reads the hub through `ViewState` and only issues typed commands.
use super::{Workbench, agent_panel, theme};
use crate::{
    app::{
        agent_hub::{QueuedPrompt, SessionStatus},
        command::Command,
    },
    icon,
    model::agent_scheduler::WaitReason,
};
use eframe::egui::{self, Frame, Key, Margin, Modifiers, RichText, ScrollArea};
use lumen_agent_core::{
    SessionId,
    model::{ItemKind, ToolCall, ToolStatus},
};

const TEXT_SIZE: f32 = 12.5;
/// Widest the conversation grows; wider windows centre it for readable lines.
const MAX_CONTENT_WIDTH: f32 = 860.0;

/// The subtle identity colour of a provider's sessions.
pub(super) fn provider_color(
    colors: &theme::Palette,
    provider: lumen_agent_core::ProviderId,
) -> eframe::egui::Color32 {
    match provider {
        lumen_agent_core::ProviderId::Codex => colors.codex,
        lumen_agent_core::ProviderId::Claude => colors.claude,
    }
}

/// One-line wording for a session's status (shared by the list and the chat).
pub(super) fn status_text(status: &SessionStatus) -> String {
    match status {
        SessionStatus::Idle => "Idle".into(),
        SessionStatus::Working => "Working".into(),
        SessionStatus::NeedsAttention => "Needs your answer".into(),
        SessionStatus::Failed(message) => format!("Failed: {message}"),
        SessionStatus::Waiting(reason) => match reason {
            WaitReason::PreviousMessage => "Waiting for the previous message".into(),
            WaitReason::NoSlot { executing } => {
                format!("Waiting: {executing} sessions are working")
            }
            WaitReason::Lease { .. } => "Waiting: another session is editing this folder".into(),
            WaitReason::ApprovedWaitingForSlot => "Approved, waiting to continue".into(),
        },
    }
}

impl Workbench<'_> {
    pub(super) fn chat_title(&self, id: SessionId) -> String {
        self.view
            .native_agents
            .as_ref()
            .and_then(|hub| hub.registry.get(id))
            .map_or_else(|| "Chat".into(), |record| record.title.clone())
    }

    pub(super) fn chat_needs_attention(&self, id: SessionId) -> bool {
        self.view
            .native_agents
            .as_ref()
            .is_some_and(|hub| hub.status(id) == SessionStatus::NeedsAttention)
    }

    pub(super) fn chat_tooltip(&self, id: SessionId) -> String {
        let Some(hub) = self.view.native_agents.as_ref() else {
            return String::new();
        };
        hub.registry.get(id).map_or_else(String::new, |record| {
            format!(
                "{} · {}\n{}",
                record.provider.label(),
                status_text(&hub.status(id)),
                record.workspace.display()
            )
        })
    }

    /// The editor area's content while a chat tab is the visible page.
    pub(super) fn chat_page(&mut self, ui: &mut egui::Ui, id: SessionId) {
        let colors = theme::palette(ui.visuals());
        let Some(hub) = self.view.native_agents.as_ref() else {
            ui.label("Native agents are not enabled.");
            return;
        };
        let Some(record) = hub.registry.get(id) else {
            ui.label("This session no longer exists.");
            return;
        };
        let (provider, title, folder) = (
            record.provider,
            record.title.clone(),
            record.workspace.clone(),
        );
        let status = hub.status(id);
        let mut commands: Vec<Command> = Vec::new();
        let accent = provider_color(&colors, provider);

        // Header: provider chip and title on the left, state on the right.
        Frame::default()
            .inner_margin(Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::AGENT).color(accent));
                    ui.label(RichText::new(provider.label()).size(11.0).color(accent));
                    ui.add(
                        egui::Label::new(RichText::new(&title).size(12.5).color(colors.text))
                            .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let color = match status {
                            SessionStatus::Failed(_) => colors.error,
                            SessionStatus::NeedsAttention => colors.warning,
                            SessionStatus::Working => colors.accent,
                            _ => colors.muted,
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(status_text(&status)).size(11.0).color(color),
                            )
                            .truncate(),
                        );
                    });
                });
                ui.label(
                    RichText::new(folder.display().to_string())
                        .size(10.5)
                        .color(colors.muted),
                );
            });
        ui.separator();

        let width = ui.available_width().min(MAX_CONTENT_WIDTH);
        let side = ((ui.available_width() - width) / 2.0).max(0.0);

        // Composer pinned to the bottom, then the transcript fills the rest.
        egui::TopBottomPanel::bottom(egui::Id::new(("chat_composer", id.0)))
            .frame(Frame::default().inner_margin(Margin::symmetric(12, 8)))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_max_width((width - 24.0).max(200.0));
                        let hub = self.view.native_agents.as_ref().expect("checked above");
                        if let Some(live) = hub.live(id) {
                            queue_list(ui, id, live.prompts.iter().skip(1), &mut commands);
                            if let Some(error) = &live.error {
                                ui.label(RichText::new(error).size(11.0).color(colors.error));
                            }
                        }
                        let busy = matches!(
                            status,
                            SessionStatus::Working | SessionStatus::NeedsAttention
                        );
                        let draft = self.view.chats.draft_mut(id);
                        let editor_id = egui::Id::new(("chat_draft", id.0));
                        // Enter sends, Shift+Enter inserts a newline; consumed before the
                        // text box sees it, and only while the box has focus.
                        let focused = ui.ctx().memory(|m| m.has_focus(editor_id));
                        let composing = ui.ctx().input(|i| {
                            i.events
                                .iter()
                                .any(|e| matches!(e, egui::Event::Ime(egui::ImeEvent::Preedit(_))))
                        });
                        let send_now = focused
                            && !composing
                            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                        ui.horizontal_top(|ui| {
                            let send_width = 28.0;
                            let response = ui.add_sized(
                        [ui.available_width() - send_width - 6.0, 54.0],
                        egui::TextEdit::multiline(draft)
                            .id(editor_id)
                            .hint_text(if busy {
                                "Queue another message…"
                            } else {
                                "Message the agent… (Enter to send, Shift+Enter for a new line)"
                            })
                            .desired_rows(2)
                            .lock_focus(true),
                    );
                            let _ = response;
                            ui.vertical(|ui| {
                                let can_send = !draft.trim().is_empty();
                                let send = ui
                                    .add_enabled(can_send, egui::Button::new(icon::SEND))
                                    .on_hover_text("Send message");
                                if send.clicked() {
                                    commands.push(Command::ChatSend(id));
                                }
                                if matches!(
                                    status,
                                    SessionStatus::Working | SessionStatus::NeedsAttention
                                ) && ui
                                    .button(icon::STOP)
                                    .on_hover_text("Stop the current turn")
                                    .clicked()
                                {
                                    commands.push(Command::ChatStop(id));
                                }
                            });
                        });
                        if send_now {
                            commands.push(Command::ChatSend(id));
                        }
                    });
                });
            });

        let hub = self.view.native_agents.as_ref().expect("checked above");
        ScrollArea::vertical()
            .id_salt(("chat_transcript", id.0))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(side + 12.0);
                    ui.vertical(|ui| {
                        ui.set_max_width((width - 24.0).max(200.0));
                        ui.spacing_mut().item_spacing.y = 8.0;
                        let Some(live) = hub.live(id) else {
                            return;
                        };
                        if live.model.items.is_empty() {
                            ui.label(
                                RichText::new("Start the conversation below.")
                                    .size(TEXT_SIZE)
                                    .color(colors.muted),
                            );
                        }
                        for item in &live.model.items {
                            match &item.kind {
                                ItemKind::User(text) => agent_panel::user_message(ui, text),
                                ItemKind::Agent { text, .. } => {
                                    agent_panel::agent_message(ui, text)
                                }
                                ItemKind::Tool(call) => tool_row(ui, &item.id.0, call),
                                ItemKind::Plan(entries) => {
                                    for entry in entries {
                                        ui.label(
                                            RichText::new(format!("• {}", entry.text))
                                                .size(TEXT_SIZE)
                                                .color(colors.muted),
                                        );
                                    }
                                }
                                ItemKind::Error(text) => {
                                    ui.label(
                                        RichText::new(text).size(TEXT_SIZE).color(colors.error),
                                    );
                                }
                                ItemKind::Notice(text) => {
                                    ui.label(
                                        RichText::new(text).size(TEXT_SIZE).color(colors.muted),
                                    );
                                }
                            }
                        }
                        for request in &live.model.pending_approvals {
                            approval_card(ui, id, request, &mut commands);
                        }
                        ui.add_space(8.0);
                    });
                });
            });
        for command in commands {
            self.command(command);
        }
    }
}

fn tool_row(ui: &mut egui::Ui, item_id: &str, call: &ToolCall) {
    let colors = theme::palette(ui.visuals());
    let (mark, color) = match call.status {
        ToolStatus::Pending | ToolStatus::Running => ("…", colors.muted),
        ToolStatus::Completed => (icon::CHECK, colors.success),
        ToolStatus::Failed => (icon::ERROR, colors.error),
        ToolStatus::Declined | ToolStatus::Cancelled => (icon::X, colors.warning),
    };
    let header = RichText::new(format!("{mark}  {}", call.title))
        .size(TEXT_SIZE - 1.0)
        .color(color);
    if call.preview.is_empty() && call.patch.is_none() {
        ui.label(header);
        return;
    }
    egui::CollapsingHeader::new(header)
        .id_salt(("chat_tool", item_id))
        .default_open(false)
        .show(ui, |ui| {
            let text = call.patch.as_deref().unwrap_or(&call.preview);
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .monospace()
                        .size(11.0)
                        .color(colors.muted),
                )
                .selectable(true),
            );
        });
}

fn approval_card(
    ui: &mut egui::Ui,
    id: SessionId,
    request: &lumen_agent_core::event::ApprovalRequest,
    commands: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    Frame::default()
        .fill(colors.input)
        .stroke(egui::Stroke::new(1.0_f32, colors.warning))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(&request.summary)
                    .size(TEXT_SIZE)
                    .color(colors.text),
            );
            if let Some(command) = &request.command {
                ui.add(
                    egui::Label::new(
                        RichText::new(command)
                            .monospace()
                            .size(11.0)
                            .color(colors.muted),
                    )
                    .selectable(true),
                );
            }
            for path in &request.paths {
                ui.label(
                    RichText::new(path.display().to_string())
                        .size(11.0)
                        .color(colors.muted),
                );
            }
            ui.horizontal_wrapped(|ui| {
                for choice in &request.choices {
                    let response = ui
                        .button(&choice.label)
                        .on_hover_text(format!("Applies to: {}", choice.scope));
                    if response.clicked() {
                        commands.push(Command::ChatApproval(
                            id,
                            request.token.clone(),
                            choice.id.clone(),
                        ));
                    }
                }
            });
        });
}

fn queue_list<'a>(
    ui: &mut egui::Ui,
    id: SessionId,
    queued: impl Iterator<Item = &'a QueuedPrompt>,
    commands: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    for prompt in queued {
        let text = prompt
            .input
            .iter()
            .find_map(|b| match b {
                lumen_agent_core::api::InputBlock::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .unwrap_or("(attachment)");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Queued").size(10.5).color(colors.muted));
            ui.add(egui::Label::new(RichText::new(text).size(11.0).color(colors.text)).truncate());
            if ui
                .small_button(icon::X)
                .on_hover_text("Remove this queued message")
                .clicked()
            {
                commands.push(Command::ChatCancelQueued(id, prompt.ticket));
            }
        });
    }
}
