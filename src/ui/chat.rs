//! Agent chat: transcript, approvals, queue and composer for one session,
//! hosted in the agent panel. Reads the hub through `ViewState` and only
//! issues typed commands.
use super::{Workbench, agent_message, theme};
use crate::{
    app::{
        agent_hub::{QueuedPrompt, SessionStatus},
        chat_attachments::Attachment,
        command::Command,
    },
    icon,
    model::agent_scheduler::WaitReason,
};
use eframe::egui::{self, Color32, Frame, Key, Margin, Modifiers, RichText, ScrollArea};
use lumen_agent_core::{
    SessionId,
    model::{ItemKind, ToolCall, ToolKind, ToolStatus},
};

const TEXT_SIZE: f32 = 12.5;

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
    /// The agent panel's chat: the conversation, then (pinned to the bottom) any
    /// queued messages and the composer. Model and effort live in the controls row.
    pub(super) fn chat_body(&mut self, ui: &mut egui::Ui, id: SessionId) {
        let colors = theme::palette(ui.visuals());
        let Some(hub) = self.view.native_agents.as_ref() else {
            ui.label("Native agents are not enabled.");
            return;
        };
        let Some(record) = hub.registry.get(id) else {
            ui.label("This session no longer exists.");
            return;
        };
        let provider = record.provider;
        let status = hub.status(id);
        let mut commands: Vec<Command> = Vec::new();
        let accent = provider_color(&colors, provider);

        let width = ui.available_width();
        // Never wider than the panel: a floor larger than the pane pushed the composer off-screen.
        let column = (width - 2.0 * GUTTER).max(80.0).min(width.max(0.0));
        let focus_composer = self.view.agent_panel.take_composer_focus();

        // Bottom block first so the transcript takes whatever is left.
        egui::TopBottomPanel::bottom(egui::Id::new(("chat_composer", id.0)))
            .frame(Frame::default().inner_margin(Margin::symmetric(GUTTER as i8, 8)))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(column);
                        let hub = self.view.native_agents.as_ref().expect("checked above");
                        if let Some(live) = hub.live(id) {
                            if let Some(error) = &live.error {
                                notice_card(ui, error, colors.error);
                            }
                            queue_list(ui, id, live.prompts.iter().skip(1), &mut commands);
                        }
                        let busy = matches!(
                            status,
                            SessionStatus::Working | SessionStatus::NeedsAttention
                        );
                        let attachments = self.view.agent_panel.attachments(id).to_vec();
                        let draft = self.view.agent_panel.draft_mut(id);
                        composer(
                            ui,
                            id,
                            draft,
                            &attachments,
                            busy,
                            focus_composer,
                            &mut commands,
                        );
                    });
                });
            });

        let hub = self.view.native_agents.as_ref().expect("checked above");
        ScrollArea::vertical()
            .id_salt(("chat_transcript", id.0))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(GUTTER);
                    ui.vertical(|ui| {
                        ui.set_width(column);
                        ui.spacing_mut().item_spacing.y = 10.0;
                        let Some(live) = hub.live(id) else {
                            return;
                        };
                        if live.model.items.is_empty() {
                            empty_state(ui, provider, accent, &colors);
                        }
                        let mut previous_tool = false;
                        for item in &live.model.items {
                            let is_tool = matches!(item.kind, ItemKind::Tool(_));
                            if !is_tool && previous_tool {
                                ui.add_space(2.0);
                            }
                            previous_tool = is_tool;
                            match &item.kind {
                                ItemKind::User(text) => user_bubble(ui, text, &colors),
                                ItemKind::Agent { text, phase, .. } => {
                                    agent_block(ui, provider, accent, text, *phase, &colors)
                                }
                                ItemKind::Tool(call) => tool_row(ui, &item.id.0, call, &colors),
                                ItemKind::Plan(entries) => plan_card(ui, entries, &colors),
                                ItemKind::Error(text) => notice_card(ui, text, colors.error),
                                ItemKind::Notice(text) => {
                                    ui.label(
                                        RichText::new(text)
                                            .size(crate::ui::theme::META_SIZE)
                                            .color(colors.muted),
                                    );
                                }
                            }
                        }
                        for request in &live.model.pending_approvals {
                            approval_card(ui, id, request, &colors, &mut commands);
                        }
                        if let SessionStatus::Working | SessionStatus::Waiting(_) = &status {
                            working_indicator(ui, &status, accent, &colors);
                        }
                        ui.add_space(12.0);
                    });
                });
            });
        for command in commands {
            self.command(command);
        }
    }
}

/// Space between the panel edge and the conversation column.
const GUTTER: f32 = 8.0;

fn empty_state(
    ui: &mut egui::Ui,
    provider: lumen_agent_core::ProviderId,
    accent: Color32,
    colors: &theme::Palette,
) {
    ui.add_space(super::widgets::empty_state_top(ui).min(48.0));
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icon::AGENT).size(26.0).color(accent));
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Ask {} anything about this folder",
                provider.label()
            ))
            .size(14.0)
            .color(colors.text),
        );
        ui.label(
            RichText::new("It can read, edit and run commands here. You approve anything risky.")
                .size(11.5)
                .color(colors.muted),
        );
    });
}

/// What you said: a quiet rounded card, labelled.
fn user_bubble(ui: &mut egui::Ui, text: &str, colors: &theme::Palette) {
    Frame::default()
        .fill(colors.input)
        .corner_radius(3)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("You").size(10.5).strong().color(colors.muted));
            ui.add(
                egui::Label::new(RichText::new(text).size(TEXT_SIZE).color(colors.text))
                    .selectable(true),
            );
        });
}

/// What the agent said: no card, a coloured name, then the markdown body.
/// Working notes between steps are smaller and muted so the answer stands out.
fn agent_block(
    ui: &mut egui::Ui,
    provider: lumen_agent_core::ProviderId,
    accent: Color32,
    text: &str,
    phase: lumen_agent_core::model::MessagePhase,
    colors: &theme::Palette,
) {
    if text.trim().is_empty() {
        return;
    }
    let note = phase == lumen_agent_core::model::MessagePhase::Commentary;
    ui.horizontal(|ui| {
        ui.label(RichText::new("●").size(9.0).color(accent));
        ui.label(
            RichText::new(provider.label())
                .size(10.5)
                .strong()
                .color(accent),
        );
        if note {
            ui.label(
                RichText::new("working notes")
                    .size(10.0)
                    .color(colors.muted),
            );
        }
    });
    if note {
        ui.add(
            egui::Label::new(RichText::new(text).size(11.5).color(colors.muted)).selectable(true),
        );
    } else {
        agent_message::agent_message(ui, text);
    }
}

fn tool_glyph(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Command => icon::TERMINAL,
        ToolKind::FileEdit => icon::FILE_CODE,
        ToolKind::Read => icon::FILE,
        ToolKind::Search => icon::MAGNIFYING_GLASS,
        ToolKind::Fetch => icon::REPO_PULL,
        ToolKind::Other => icon::GEAR,
    }
}

/// One tool call as a compact row: kind icon, what it did, and its state at the
/// right. Output opens beneath it on demand.
fn tool_row(ui: &mut egui::Ui, item_id: &str, call: &ToolCall, colors: &theme::Palette) {
    let (state, color) = match call.status {
        ToolStatus::Pending | ToolStatus::Running => ("running", colors.accent),
        ToolStatus::Completed => ("done", colors.success),
        ToolStatus::Failed => ("failed", colors.error),
        ToolStatus::Declined => ("declined", colors.warning),
        ToolStatus::Cancelled => ("cancelled", colors.warning),
    };
    let has_detail = !call.preview.is_empty() || call.patch.is_some();
    let id = ui.make_persistent_id(("chat_tool", item_id));
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let row = Frame::default()
        .fill(colors.panel)
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(tool_glyph(call.kind))
                        .size(12.0)
                        .color(colors.muted),
                );
                ui.add(
                    egui::Label::new(RichText::new(&call.title).size(11.5).color(colors.text))
                        .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if has_detail {
                        ui.label(
                            RichText::new(if open {
                                icon::CARET_DOWN
                            } else {
                                icon::CARET_RIGHT
                            })
                            .size(crate::ui::theme::META_SIZE)
                            .color(colors.muted),
                        );
                    }
                    ui.label(RichText::new(state).size(10.5).color(color));
                    if matches!(call.status, ToolStatus::Running | ToolStatus::Pending) {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(250));
                    }
                });
            });
            if open && has_detail {
                let text = call.patch.as_deref().unwrap_or(&call.preview);
                ui.add_space(2.0);
                ScrollArea::vertical()
                    .id_salt(("chat_tool_out", item_id))
                    .max_height(220.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(text)
                                    .monospace()
                                    .size(crate::ui::theme::META_SIZE)
                                    .color(colors.muted),
                            )
                            .selectable(true),
                        );
                    });
            }
        });
    if has_detail
        && ui
            .interact(row.response.rect, id.with("toggle"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }
}

fn plan_card(
    ui: &mut egui::Ui,
    entries: &[lumen_agent_core::model::PlanEntry],
    colors: &theme::Palette,
) {
    use lumen_agent_core::model::PlanStatus;
    Frame::default()
        .fill(colors.panel)
        .corner_radius(3)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Plan")
                    .size(10.5)
                    .strong()
                    .color(colors.muted),
            );
            for entry in entries {
                let (mark, color) = match entry.status {
                    PlanStatus::Done => (icon::CHECK, colors.success),
                    PlanStatus::InProgress => ("▸", colors.accent),
                    PlanStatus::Pending => ("○", colors.muted),
                };
                ui.horizontal_top(|ui| {
                    ui.label(
                        RichText::new(mark)
                            .size(crate::ui::theme::META_SIZE)
                            .color(color),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(&entry.text).size(11.5).color(colors.text))
                            .selectable(true),
                    );
                });
            }
        });
}

/// An error or warning that must not be missed, in a tinted card.
fn notice_card(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::default()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0_f32, color.gamma_multiply(0.6)))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(color)).selectable(true));
        });
}

/// A permission request: what the agent wants, the exact command or paths, and
/// the answers the provider offers - allowing buttons filled, refusals outlined.
fn approval_card(
    ui: &mut egui::Ui,
    id: SessionId,
    request: &lumen_agent_core::event::ApprovalRequest,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) {
    // A warning edge marks the request; the rest of the card stays a quiet panel.
    let card = Frame::default()
        .fill(colors.panel)
        .corner_radius(3)
        .inner_margin(Margin {
            left: 14,
            right: 12,
            top: 9,
            bottom: 9,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon::WARNING)
                        .size(12.0)
                        .color(colors.warning),
                );
                ui.label(
                    RichText::new("Permission needed")
                        .size(11.5)
                        .strong()
                        .color(colors.warning),
                );
            });
            ui.add(
                egui::Label::new(
                    RichText::new(&request.summary)
                        .size(TEXT_SIZE)
                        .color(colors.text),
                )
                .selectable(true),
            );
            if let Some(command) = &request.command {
                Frame::default()
                    .fill(colors.bg)
                    .corner_radius(3)
                    .inner_margin(Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("$ {command}"))
                                    .monospace()
                                    .size(11.5)
                                    .color(colors.text),
                            )
                            .selectable(true),
                        );
                    });
            }
            for path in &request.paths {
                ui.label(
                    RichText::new(format!("{} {}", icon::FILE, path.display()))
                        .size(crate::ui::theme::META_SIZE)
                        .color(colors.muted),
                );
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                for choice in &request.choices {
                    let label = RichText::new(&choice.label).size(11.5);
                    let button = if choice.allows {
                        egui::Button::new(label.color(colors.on_accent)).fill(colors.accent)
                    } else {
                        egui::Button::new(label.color(colors.text))
                    };
                    if ui
                        .add(button)
                        .on_hover_text(format!("Applies to: {}", choice.scope))
                        .clicked()
                    {
                        commands.push(Command::ChatApproval(
                            id,
                            request.token.clone(),
                            choice.id.clone(),
                        ));
                    }
                }
            });
        });
    let rect = card.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(rect.left() + 3.0, rect.bottom()),
        ),
        egui::CornerRadius {
            nw: 3,
            sw: 3,
            ne: 0,
            se: 0,
        },
        colors.warning,
    );
}

/// "Working…" with animated dots, or why the turn has not started.
fn working_indicator(
    ui: &mut egui::Ui,
    status: &SessionStatus,
    accent: Color32,
    colors: &theme::Palette,
) {
    let time = ui.input(|i| i.time);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(300));
    let dots = ".".repeat(1 + (time * 2.5) as usize % 3);
    let (text, color) = match status {
        SessionStatus::Waiting(_) => (status_text(status), colors.warning),
        _ => (format!("Working{dots}"), accent),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("●").size(9.0).color(color));
        ui.label(RichText::new(text).size(11.5).italics().color(color));
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
        Frame::default()
            .fill(colors.panel)
            .corner_radius(3)
            .inner_margin(Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Queued").size(10.0).color(colors.muted));
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .size(crate::ui::theme::META_SIZE)
                                .color(colors.text),
                        )
                        .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(icon::X)
                            .on_hover_text("Remove this queued message")
                            .clicked()
                        {
                            commands.push(Command::ChatCancelQueued(id, prompt.ticket));
                        }
                    });
                });
            });
    }
}

/// The message box: the biggest control on the page. Enter sends and Shift+Enter
/// adds a line (consumed before the text box sees it, only while it has focus
/// and no IME composition is in progress). Send turns into Stop while the agent
/// works; typing still queues the next message.
fn composer(
    ui: &mut egui::Ui,
    id: SessionId,
    draft: &mut String,
    attachments: &[Attachment],
    busy: bool,
    focus: bool,
    commands: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    let editor_id = egui::Id::new(("chat_draft", id.0));
    if focus {
        ui.memory_mut(|m| m.request_focus(editor_id));
    }
    let focused = ui.ctx().memory(|m| m.has_focus(editor_id));
    let composing = ui.ctx().input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::Ime(egui::ImeEvent::Preedit(_))))
    });
    let send_now =
        focused && !composing && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let can_send = !draft.trim().is_empty() || !attachments.is_empty();
    Frame::default()
        .fill(colors.input)
        .stroke(egui::Stroke::new(
            1.0_f32,
            if focused {
                colors.accent
            } else {
                colors.border
            },
        ))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            attachment_chips(ui, id, attachments, &colors, commands);
            ui.add(
                egui::TextEdit::multiline(draft)
                    .id(editor_id)
                    .frame(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .font(egui::FontId::proportional(13.5))
                    .hint_text(if busy {
                        "The agent is working - type to queue the next message…"
                    } else {
                        "Message the agent…"
                    })
                    .lock_focus(true),
            );
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 24.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    let send = egui::Button::new(
                        RichText::new(format!(
                            "{}  {}",
                            icon::SEND,
                            if busy { "Queue" } else { "Send" }
                        ))
                        .size(12.0)
                        .color(if can_send {
                            colors.on_accent
                        } else {
                            colors.muted
                        }),
                    )
                    .fill(if can_send {
                        colors.accent
                    } else {
                        colors.panel
                    })
                    .corner_radius(3);
                    if ui
                        .add_enabled(can_send, send)
                        .on_hover_text(if busy {
                            "Queue this message"
                        } else {
                            "Send message"
                        })
                        .clicked()
                    {
                        commands.push(Command::ChatSend(id));
                    }
                    if busy
                        && ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{}  Stop", icon::STOP))
                                        .size(12.0)
                                        .color(colors.error),
                                )
                                .corner_radius(3),
                            )
                            .on_hover_text("Stop the current turn")
                            .clicked()
                    {
                        commands.push(Command::ChatStop(id));
                    }
                    // The hint takes whatever the buttons leave and gives way first.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let attach = egui::Button::new(
                            RichText::new(icon::PAPERCLIP)
                                .size(13.0)
                                .color(colors.muted),
                        )
                        .frame(false);
                        if ui.add(attach).on_hover_text("Attach files").clicked() {
                            commands.push(Command::ChatAttach(id));
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(if busy {
                                    "Enter to queue · sends after the current turn · Shift+Enter for a new line"
                                } else {
                                    "Enter to send · Shift+Enter for a new line"
                                })
                                .size(10.0)
                                .color(colors.muted),
                            )
                            .truncate(),
                        );
                    });
                },
            );
        });
    if send_now {
        commands.push(Command::ChatSend(id));
    }
}

/// One removable chip per attached file, above the message box.
fn attachment_chips(
    ui: &mut egui::Ui,
    id: SessionId,
    attachments: &[Attachment],
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) {
    if attachments.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for attachment in attachments {
            Frame::default()
                .fill(colors.panel)
                .corner_radius(3)
                .inner_margin(Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let name = attachment.name();
                        let kind = if attachment.image_type.is_some() {
                            "image"
                        } else {
                            "file"
                        };
                        ui.label(RichText::new(icon::FILE).size(12.0).color(colors.muted));
                        ui.add(
                            egui::Label::new(
                                RichText::new(&name)
                                    .size(theme::META_SIZE)
                                    .color(colors.text),
                            )
                            .truncate(),
                        )
                        .on_hover_text(format!(
                            "{} ({kind}, {})",
                            attachment.path.display(),
                            human_size(attachment.size)
                        ));
                        ui.label(
                            RichText::new(human_size(attachment.size))
                                .size(10.0)
                                .color(colors.muted),
                        );
                        let remove = ui
                            .add(
                                egui::Button::new(
                                    RichText::new(icon::X).size(10.0).color(colors.muted),
                                )
                                .frame(false),
                            )
                            .on_hover_text(format!("Remove {name}"));
                        if remove.clicked() {
                            commands.push(Command::ChatDetach(id, attachment.path.clone()));
                        }
                    });
                });
        }
    });
    ui.add_space(4.0);
}

fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.0} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}
