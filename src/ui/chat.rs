//! Agent chat: transcript, approvals, queue and composer for one session,
//! hosted in the agent panel. Reads the hub through `ViewState` and only
//! issues typed commands.
use super::{Workbench, agent_message, theme};
use crate::{
    app::{
        agent_hub::{QueueMove, QueuedPrompt, SessionStatus},
        agent_panel::{self, ChatSuggestion, QueueEdit, ScrollRequest},
        chat_attachments::Attachment,
        command::Command,
    },
    icon,
    model::{
        agent_scheduler::WaitReason,
        patch_view::{self, PatchFile, PatchLineKind},
        tool_groups::{self, Block},
    },
    services::source_control::DiffSide,
};
use eframe::egui::{self, Color32, Frame, Key, Margin, Modifiers, RichText, ScrollArea};
use lumen_agent_core::{
    RequestToken, SessionId,
    event::ApprovalRequest,
    model::{ItemKind, ToolCall, ToolKind, ToolStatus},
};
use std::path::PathBuf;

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
        let (title, workspace) = (record.title.clone(), record.workspace.clone());
        let status = hub.status(id);
        let mut commands: Vec<Command> = Vec::new();
        let accent = provider_color(&colors, provider);

        let width = ui.available_width();
        // Never wider than the panel: a floor larger than the pane pushed the composer off-screen.
        let column = (width - 2.0 * GUTTER).max(80.0).min(width.max(0.0));
        let focus_composer = self.view.agent_panel.take_composer_focus();
        let scroll_request = self.view.agent_panel.take_scroll_request(id);
        let draft_empty = self.view.agent_panel.draft_mut(id).trim().is_empty();
        let has_repository = self.view.source_control.snapshot.is_some();
        let mut queue_edit = self.view.agent_panel.queue_edit().cloned();
        let mut review_approval = None;

        identity_row(ui, &title, &workspace, column, &colors);

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
                            review_approval = ui
                                .push_id(("approval_navigation", id.0), |ui| {
                                    approval_navigation(ui, &live.model.pending_approvals)
                                })
                                .inner;
                            if let Some(error) = &live.error {
                                notice_card(ui, error, colors.error);
                            }
                            queue_list(ui, id, &live.prompts, &mut queue_edit, &mut commands);
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
        self.view.agent_panel.set_queue_edit(queue_edit);

        let hub = self.view.native_agents.as_ref().expect("checked above");
        let snapshot = self.view.source_control.snapshot.as_ref();
        let repo_diff = |path: &std::path::Path| {
            snapshot.and_then(|snapshot| agent_panel::changed_file_diff(snapshot, path))
        };
        let mut user_tops: Vec<(usize, f32)> = Vec::new();
        let output = ScrollArea::vertical()
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
                            empty_state(
                                ui,
                                id,
                                provider,
                                accent,
                                &Suggestions {
                                    draft_empty,
                                    has_repository,
                                },
                                &colors,
                                &mut commands,
                            );
                        }
                        let mut pass = TranscriptPass {
                            session: id,
                            provider,
                            accent,
                            colors: &colors,
                            repo_diff: &repo_diff,
                            commands: &mut commands,
                            scroll_to: match scroll_request {
                                Some(ScrollRequest::Item(index)) => Some(index),
                                _ => None,
                            },
                            user_tops: &mut user_tops,
                        };
                        let items = &live.model.items;
                        let mut previous_tool = false;
                        for block in tool_groups::blocks(items) {
                            let is_tool = match &block {
                                Block::Item(index) => {
                                    matches!(items[*index].kind, ItemKind::Tool(_))
                                }
                                Block::Explored(_) => true,
                            };
                            if !is_tool && previous_tool {
                                ui.add_space(2.0);
                            }
                            previous_tool = is_tool;
                            match block {
                                Block::Item(index) => {
                                    transcript_item(ui, index, &items[index], &mut pass)
                                }
                                Block::Explored(group) => {
                                    explored_group(ui, items, &group, &mut pass)
                                }
                            }
                        }
                        for request in &live.model.pending_approvals {
                            let response = approval_card(ui, id, request, &colors, pass.commands);
                            // Navigation is scoped to this frame and exact request token.
                            // A resolved request or another session cannot inherit it.
                            if review_approval.as_ref() == Some(&request.token) {
                                response.scroll_to_me(Some(egui::Align::Min));
                            }
                        }
                        if let SessionStatus::Working | SessionStatus::Waiting(_) = &status {
                            working_indicator(ui, &status, accent, &colors);
                        }
                        let end = ui.allocate_response(egui::vec2(1.0, 12.0), egui::Sense::hover());
                        if scroll_request == Some(ScrollRequest::Latest) {
                            end.scroll_to_me(Some(egui::Align::BOTTOM));
                        }
                    });
                });
            });

        let items = hub.live(id).map_or(0, |live| live.model.items.len());
        let viewport = output.inner_rect;
        let at_end = at_bottom(
            output.state.offset.y,
            viewport.height(),
            output.content_size.y,
        );
        if at_end {
            self.view.agent_panel.mark_seen(id, items);
        }
        let unseen = self.view.agent_panel.unseen_items(id, items);
        let overflow = output.content_size.y > viewport.height() + 1.0;
        let (previous, next) = adjacent_messages(&user_tops, viewport.top());
        let navigation = ui
            .push_id(("chat_navigation", id.0), |ui| {
                conversation_navigation(
                    ui,
                    viewport,
                    &NavigationState {
                        show_arrows: overflow && !user_tops.is_empty(),
                        previous,
                        next,
                        at_end,
                        unseen,
                    },
                    &colors,
                )
            })
            .inner;
        if let Some(request) = navigation {
            self.view.agent_panel.request_scroll(id, request);
            ui.ctx().request_repaint();
        }
        for command in commands {
            self.command(command);
        }
    }
}

/// Space between the panel edge and the conversation column.
const GUTTER: f32 = 8.0;

/// What the empty chat's starting actions may do right now.
struct Suggestions {
    draft_empty: bool,
    has_repository: bool,
}

/// The session title and its folder, always visible above the conversation.
fn identity_row(
    ui: &mut egui::Ui,
    title: &str,
    workspace: &std::path::Path,
    column: f32,
    colors: &theme::Palette,
) {
    let folder = workspace.file_name().map_or_else(
        || workspace.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let folder_text = format!("{} {folder}", icon::FOLDER);
    let font = egui::FontId::proportional(theme::META_SIZE);
    let natural = ui
        .painter()
        .layout_no_wrap(folder_text.clone(), font.clone(), colors.muted)
        .size()
        .x;
    // The folder keeps what it needs up to 40%; the title gets the rest and truncates.
    let folder_width = natural.min(column * 0.4);
    let gap = 6.0;
    let title_width = (column - folder_width - gap).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(
            egui::vec2(title_width, 18.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(RichText::new(title).size(12.0).strong().color(colors.text))
                        .truncate(),
                )
                .on_hover_text(title);
            },
        );
        ui.add_space(gap);
        ui.allocate_ui_with_layout(
            egui::vec2(folder_width, 18.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.add(
                    egui::Label::new(RichText::new(folder_text).font(font).color(colors.muted))
                        .truncate(),
                )
                .on_hover_text(workspace.display().to_string());
            },
        );
    });
}

/// Whether the viewport shows the end of the content. The slack covers the
/// padding below the last item, which a jump to the end leaves unscrolled.
fn at_bottom(offset: f32, viewport: f32, content: f32) -> bool {
    const SLACK: f32 = 16.0;
    offset + viewport >= content - SLACK
}

/// The user messages just above and just below the viewport's top edge, given
/// each message's top in the same coordinates. A message that is already at the
/// top counts as "here", so Previous/Next always move.
fn adjacent_messages(tops: &[(usize, f32)], viewport_top: f32) -> (Option<usize>, Option<usize>) {
    const SLACK: f32 = 6.0;
    let previous = tops
        .iter()
        .rev()
        .find(|(_, top)| *top < viewport_top - SLACK)
        .map(|(index, _)| *index);
    let next = tops
        .iter()
        .find(|(_, top)| *top > viewport_top + SLACK)
        .map(|(index, _)| *index);
    (previous, next)
}

struct NavigationState {
    show_arrows: bool,
    previous: Option<usize>,
    next: Option<usize>,
    at_end: bool,
    unseen: usize,
}

/// Floating controls over the bottom of the transcript: Previous/Next message
/// and Jump to latest (with a count when content arrived while reading above).
/// They only request a scroll; streaming never moves the viewport by itself.
fn conversation_navigation(
    ui: &mut egui::Ui,
    viewport: egui::Rect,
    state: &NavigationState,
    colors: &theme::Palette,
) -> Option<ScrollRequest> {
    if !state.show_arrows && state.at_end {
        return None;
    }
    let rect = egui::Rect::from_min_max(
        egui::pos2(viewport.left() + GUTTER, viewport.bottom() - 28.0),
        egui::pos2(viewport.right() - GUTTER, viewport.bottom() - 4.0),
    );
    let mut overlay = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let ui = &mut overlay;
    let mut request = None;
    let chip = |ui: &mut egui::Ui, text: String, enabled: bool, tip: &str, accent: bool| {
        let button =
            egui::Button::new(RichText::new(text).size(theme::META_SIZE).color(if accent {
                colors.on_accent
            } else {
                colors.text
            }))
            .fill(if accent { colors.accent } else { colors.panel })
            .stroke(egui::Stroke::new(1.0_f32, colors.border))
            .corner_radius(3);
        ui.add_enabled(enabled, button).on_hover_text(tip).clicked()
    };
    if !state.at_end {
        let label = if state.unseen > 0 {
            format!("{} {} new", icon::CARET_DOWN, state.unseen)
        } else {
            format!("{} Latest", icon::CARET_DOWN)
        };
        if chip(
            ui,
            label,
            true,
            "Jump to the latest message",
            state.unseen > 0,
        ) {
            request = Some(ScrollRequest::Latest);
        }
    }
    if state.show_arrows {
        if chip(
            ui,
            "Next".into(),
            state.next.is_some(),
            "Scroll to the next message you sent",
            false,
        ) {
            request = state.next.map(ScrollRequest::Item);
        }
        if chip(
            ui,
            "Prev".into(),
            state.previous.is_some(),
            "Scroll to the previous message you sent",
            false,
        ) {
            request = state.previous.map(ScrollRequest::Item);
        }
    }
    request
}

/// Everything one pass over the transcript needs to draw its items.
struct TranscriptPass<'a> {
    /// Keys expansion state, so the same item id in two chats stays independent.
    session: SessionId,
    provider: lumen_agent_core::ProviderId,
    accent: Color32,
    colors: &'a theme::Palette,
    repo_diff: &'a dyn Fn(&std::path::Path) -> Option<(PathBuf, DiffSide)>,
    commands: &'a mut Vec<Command>,
    /// A user message to bring to the top of the viewport this frame.
    scroll_to: Option<usize>,
    /// Where each user message was drawn, for Previous/Next.
    user_tops: &'a mut Vec<(usize, f32)>,
}

fn transcript_item(
    ui: &mut egui::Ui,
    index: usize,
    item: &lumen_agent_core::model::TranscriptItem,
    pass: &mut TranscriptPass,
) {
    let colors = pass.colors;
    match &item.kind {
        ItemKind::User(text) => {
            let response = user_bubble(ui, text, colors);
            pass.user_tops.push((index, response.rect.top()));
            if pass.scroll_to == Some(index) {
                response.scroll_to_me(Some(egui::Align::Min));
            }
        }
        ItemKind::Agent { text, phase, .. } => {
            agent_block(ui, pass.provider, pass.accent, text, *phase, colors)
        }
        ItemKind::Tool(call) => tool_row(ui, &item.id.0, call, pass),
        ItemKind::Plan(entries) => plan_card(ui, entries, colors),
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

/// A run of finished reads and searches as one expandable summary. Each call
/// keeps its own row (and its own expansion state) inside the group.
fn explored_group(
    ui: &mut egui::Ui,
    items: &[lumen_agent_core::model::TranscriptItem],
    group: &[usize],
    pass: &mut TranscriptPass,
) {
    let colors = pass.colors;
    let id = egui::Id::new(("chat_explored", pass.session.0, &items[group[0]].id.0));
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let summary = tool_groups::summary(items, group);
    let header = Frame::default()
        .fill(colors.panel)
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if open {
                        icon::CARET_DOWN
                    } else {
                        icon::CARET_RIGHT
                    })
                    .size(theme::META_SIZE)
                    .color(colors.muted),
                );
                ui.add(
                    egui::Label::new(RichText::new(&summary).size(11.5).color(colors.text))
                        .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("done").size(10.5).color(colors.success));
                });
            });
        });
    let toggle = ui
        .interact(
            header.response.rect,
            id.with("toggle"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if open {
            "Hide these calls"
        } else {
            "Show every call in this group"
        });
    toggle.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::CollapsingHeader,
            true,
            open,
            summary.clone(),
        )
    });
    if toggle.clicked() {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }
    if open {
        ui.spacing_mut().item_spacing.y = 4.0;
        for &index in group {
            if let ItemKind::Tool(call) = &items[index].kind {
                tool_row(ui, &items[index].id.0, call, pass);
            }
        }
    }
}

fn empty_state(
    ui: &mut egui::Ui,
    id: SessionId,
    provider: lumen_agent_core::ProviderId,
    accent: Color32,
    suggestions: &Suggestions,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
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
        ui.add_space(6.0);
        // Starting points fill the message box for editing; nothing is sent.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
            for suggestion in ChatSuggestion::ALL {
                let blocked = if !suggestion.needs_repository() || suggestions.has_repository {
                    None
                } else {
                    Some("Open a folder that is a Git repository to review its changes")
                }
                .or(if suggestions.draft_empty {
                    None
                } else {
                    Some("Clear the message box first; your draft is kept as it is")
                });
                let button = egui::Button::new(
                    RichText::new(suggestion.label())
                        .size(theme::META_SIZE)
                        .color(colors.text),
                )
                .fill(colors.input)
                .stroke(egui::Stroke::new(1.0_f32, colors.border))
                .corner_radius(3);
                let response = ui
                    .add_enabled(blocked.is_none(), button)
                    .on_hover_text(format!("Fill the message box: {}", suggestion.prompt()))
                    .on_disabled_hover_text(blocked.unwrap_or_default());
                if response.clicked() {
                    commands.push(Command::ChatSuggest(id, suggestion));
                }
            }
        });
    });
}

/// What you said: a quiet rounded card, labelled.
fn user_bubble(ui: &mut egui::Ui, text: &str, colors: &theme::Palette) -> egui::Response {
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
        })
        .response
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
/// right. Output opens beneath it on demand; the header alone toggles it so the
/// buttons in the detail stay clickable.
fn tool_row(ui: &mut egui::Ui, item_id: &str, call: &ToolCall, pass: &mut TranscriptPass) {
    let colors = pass.colors;
    let (state, color) = match call.status {
        ToolStatus::Pending | ToolStatus::Running => ("running", colors.accent),
        ToolStatus::Completed => ("done", colors.success),
        ToolStatus::Failed => ("failed", colors.error),
        ToolStatus::Declined => ("declined", colors.warning),
        ToolStatus::Cancelled => ("cancelled", colors.warning),
    };
    let has_detail = !call.preview.is_empty() || call.patch.is_some();
    let id = egui::Id::new(("chat_tool", pass.session.0, item_id));
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let mut toggle_rect = egui::Rect::NOTHING;
    Frame::default()
        .fill(colors.panel)
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let header = ui.horizontal(|ui| {
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
            toggle_rect = header.response.rect;
            toggle_rect.max.x = ui.max_rect().right();
            if open && has_detail {
                ui.add_space(2.0);
                tool_detail(ui, item_id, call, pass);
            }
        });
    if has_detail
        && ui
            .interact(toggle_rect, id.with("toggle"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    {
        open = !open;
        ui.ctx().data_mut(|d| d.insert_temp(id, open));
    }
}

/// The opened part of a tool row: a readable patch when the provider sent one,
/// the plain output otherwise, and file actions for edits.
fn tool_detail(ui: &mut egui::Ui, item_id: &str, call: &ToolCall, pass: &mut TranscriptPass) {
    let colors = pass.colors;
    let plain = |ui: &mut egui::Ui, text: &str| {
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
    };
    match call.patch.as_deref() {
        Some(patch) => match patch_view::parse(patch, &call.locations) {
            Some(files) => patch_files(ui, item_id, &files, colors),
            None => {
                ui.label(
                    RichText::new("This change could not be read as a diff; showing the raw text.")
                        .size(10.0)
                        .color(colors.warning),
                );
                plain(ui, patch);
            }
        },
        None => plain(ui, &call.preview),
    }
    if call.kind == ToolKind::FileEdit && !call.locations.is_empty() {
        file_actions(ui, item_id, &call.locations, pass);
    }
}

/// Per-file headers with +/- counts, then the changed lines tinted by kind.
fn patch_files(ui: &mut egui::Ui, item_id: &str, files: &[PatchFile], colors: &theme::Palette) {
    for (index, file) in files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icon::FILE_CODE)
                    .size(12.0)
                    .color(colors.muted),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(file.display_name())
                        .size(11.5)
                        .strong()
                        .color(colors.text),
                )
                .truncate(),
            )
            .on_hover_text(&file.path);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{}{}", icon::MINUS, file.removed))
                        .size(theme::META_SIZE)
                        .color(colors.error),
                );
                ui.label(
                    RichText::new(format!("{}{}", icon::PLUS, file.added))
                        .size(theme::META_SIZE)
                        .color(colors.success),
                );
            });
        });
        let font = egui::FontId::monospace(theme::META_SIZE);
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = f32::INFINITY;
        for line in &file.lines {
            let (mark, text_color, background) = match line.kind {
                PatchLineKind::Added => ("+ ", colors.success, colors.success.gamma_multiply(0.15)),
                PatchLineKind::Removed => ("- ", colors.error, colors.error.gamma_multiply(0.15)),
                PatchLineKind::Hunk => ("", colors.accent, Color32::TRANSPARENT),
                PatchLineKind::Context => ("  ", colors.muted, Color32::TRANSPARENT),
            };
            job.append(
                &format!("{mark}{}\n", line.text),
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: text_color,
                    background,
                    ..Default::default()
                },
            );
        }
        ScrollArea::both()
            .id_salt(("chat_patch", item_id, index))
            .max_height(220.0)
            .show(ui, |ui| {
                ui.add(egui::Label::new(job).selectable(true));
            });
    }
}

/// Open an edited file, or its uncommitted Git diff when the repository has one.
fn file_actions(
    ui: &mut egui::Ui,
    item_id: &str,
    locations: &[PathBuf],
    pass: &mut TranscriptPass,
) {
    const SHOWN: usize = 4;
    let colors = pass.colors;
    ui.add_space(2.0);
    for (index, path) in locations.iter().take(SHOWN).enumerate() {
        ui.push_id((item_id, "file_action", index), |ui| {
            ui.horizontal(|ui| {
                let name = path
                    .file_name()
                    .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
                ui.add(
                    egui::Label::new(
                        RichText::new(name)
                            .size(theme::META_SIZE)
                            .color(colors.muted),
                    )
                    .truncate(),
                )
                .on_hover_text(path.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Open")
                        .on_hover_text("Open this file in the editor")
                        .clicked()
                    {
                        pass.commands.push(Command::OpenPath(path.clone()));
                    }
                    let diff = (pass.repo_diff)(path);
                    let response = ui
                        .add_enabled(diff.is_some(), egui::Button::new("Diff").small())
                        .on_hover_text("Show this file's uncommitted changes from Git")
                        .on_disabled_hover_text(
                            "No uncommitted Git change for this file (it may be committed, reverted or outside the repository)",
                        );
                    if let (true, Some((relative, side))) = (response.clicked(), diff) {
                        pass.commands.push(Command::ShowSourceDiff(relative, side));
                    }
                });
            });
        });
    }
    if locations.len() > SHOWN {
        ui.label(
            RichText::new(format!("+{} more files", locations.len() - SHOWN))
                .size(10.0)
                .color(colors.muted),
        );
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
    request: &ApprovalRequest,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) -> egui::Response {
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
    card.response
}

/// A pinned entry point to approvals, even when the transcript is scrolled away.
/// Choosing a request only reveals it; decisions stay with its full details.
fn approval_navigation(ui: &mut egui::Ui, requests: &[ApprovalRequest]) -> Option<RequestToken> {
    if requests.is_empty() {
        return None;
    }
    let colors = theme::palette(ui.visuals());
    let mut selected = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if requests.len() == 1 {
                if ui
                    .push_id(&requests[0].token, |ui| ui.small_button("Review"))
                    .inner
                    .on_hover_text(format!("Show approval request: {}", requests[0].summary))
                    .clicked()
                {
                    selected = Some(requests[0].token.clone());
                }
            } else {
                ui.menu_button("Review", |ui| {
                    ui.set_max_width(280.0);
                    ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for (index, request) in requests.iter().enumerate() {
                            if ui
                                .push_id(&request.token, |ui| {
                                    ui.add(
                                        egui::Button::new(format!(
                                            "{}. {}",
                                            index + 1,
                                            request.summary
                                        ))
                                        .truncate(),
                                    )
                                })
                                .inner
                                .on_hover_text(&request.summary)
                                .clicked()
                            {
                                selected = Some(request.token.clone());
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Choose an approval request to review");
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let label = if requests.len() == 1 {
                    "1 approval needed".into()
                } else {
                    format!("{} approvals needed", requests.len())
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{} {label}", icon::WARNING))
                            .size(theme::META_SIZE)
                            .color(colors.warning),
                    )
                    .truncate(),
                )
                .on_hover_text(label);
            });
        });
    });
    selected
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

fn queued_text(prompt: &QueuedPrompt) -> &str {
    prompt
        .input
        .iter()
        .find_map(|b| match b {
            lumen_agent_core::api::InputBlock::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

/// Messages waiting behind the active one (the head of `prompts`). Each can be
/// edited in place (Save or Cancel), moved up or down, or removed; the active
/// prompt is never listed, so no action can reach it.
fn queue_list(
    ui: &mut egui::Ui,
    id: SessionId,
    prompts: &std::collections::VecDeque<QueuedPrompt>,
    edit: &mut Option<QueueEdit>,
    commands: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    let queued: Vec<&QueuedPrompt> = prompts.iter().skip(1).collect();
    // An edit whose message started or was removed has nothing left to change.
    if edit
        .as_ref()
        .is_some_and(|e| e.session == id && !queued.iter().any(|p| p.ticket == e.ticket))
    {
        *edit = None;
    }
    if queued.is_empty() {
        return;
    }
    ui.label(
        RichText::new(format!("Queue · {} pending", queued.len()))
            .size(10.0)
            .strong()
            .color(colors.muted),
    );
    for (position, prompt) in queued.iter().enumerate() {
        let editing = edit
            .as_ref()
            .is_some_and(|e| e.session == id && e.ticket == prompt.ticket);
        let mut finished = false;
        Frame::default()
            .fill(colors.panel)
            .corner_radius(3)
            .inner_margin(Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.push_id(("queued", id.0, prompt.ticket.0), |ui| {
                    if editing {
                        if let Some(e) = edit.as_mut() {
                            finished = queue_edit_row(ui, e, &colors, commands);
                        }
                    } else {
                        let text = queued_text(prompt);
                        let attachment_only = text.is_empty();
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{}.", position + 1))
                                    .size(10.0)
                                    .color(colors.muted),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if attachment_only {
                                        "(attachment)"
                                    } else {
                                        text
                                    })
                                    .size(crate::ui::theme::META_SIZE)
                                    .color(colors.text),
                                )
                                .truncate(),
                            )
                            .on_hover_text(text);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.menu_button(
                                        RichText::new(icon::ELLIPSIS).size(11.0),
                                        |ui| {
                                            if ui.button("Edit").clicked() {
                                                *edit = Some(QueueEdit {
                                                    session: id,
                                                    ticket: prompt.ticket,
                                                    text: text.to_string(),
                                                    focus: true,
                                                    had_focus: false,
                                                });
                                                ui.close();
                                            }
                                            if ui
                                                .add_enabled(
                                                    position > 0,
                                                    egui::Button::new("Move up"),
                                                )
                                                .clicked()
                                            {
                                                commands.push(Command::ChatMoveQueued(
                                                    id,
                                                    prompt.ticket,
                                                    QueueMove::Up,
                                                ));
                                                ui.close();
                                            }
                                            if ui
                                                .add_enabled(
                                                    position + 1 < queued.len(),
                                                    egui::Button::new("Move down"),
                                                )
                                                .clicked()
                                            {
                                                commands.push(Command::ChatMoveQueued(
                                                    id,
                                                    prompt.ticket,
                                                    QueueMove::Down,
                                                ));
                                                ui.close();
                                            }
                                            if ui.button("Remove").clicked() {
                                                commands.push(Command::ChatCancelQueued(
                                                    id,
                                                    prompt.ticket,
                                                ));
                                                ui.close();
                                            }
                                        },
                                    )
                                    .response
                                    .on_hover_text("Edit, reorder or remove this queued message");
                                },
                            );
                        });
                    }
                });
            });
        // Save and Cancel both end the edit; only Save sent a command.
        if finished {
            *edit = None;
        }
    }
}

/// Inline editor for one queued message: Enter or Save keeps the change, Escape
/// or Cancel leaves the message exactly as it was.
fn queue_edit_row(
    ui: &mut egui::Ui,
    edit: &mut QueueEdit,
    colors: &theme::Palette,
    commands: &mut Vec<Command>,
) -> bool {
    let field_id = egui::Id::new(("queue_edit_field", edit.ticket.0));
    let focused = ui.ctx().memory(|m| m.has_focus(field_id));
    let save = focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let cancel = edit.had_focus && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let field = ui.add(
        egui::TextEdit::multiline(&mut edit.text)
            .id(field_id)
            .desired_width(f32::INFINITY)
            .desired_rows(2)
            .font(egui::FontId::proportional(theme::META_SIZE))
            .text_color(colors.text),
    );
    if std::mem::take(&mut edit.focus) {
        ui.memory_mut(|m| m.request_focus(field_id));
    }
    edit.had_focus = field.has_focus();
    let (mut save, mut cancel) = (save, cancel);
    ui.horizontal(|ui| {
        save |= ui
            .small_button("Save")
            .on_hover_text("Keep the edited text (Enter)")
            .clicked();
        cancel |= ui
            .small_button("Cancel")
            .on_hover_text("Leave the message unchanged (Esc)")
            .clicked();
    });
    if save {
        commands.push(Command::ChatEditQueued(
            edit.session,
            edit.ticket,
            edit.text.clone(),
        ));
    }
    save || cancel
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
                    if busy {
                        // Stop ends the active turn and Queue only adds to the line:
                        // a gap and divider keep them from being mistaken for each other.
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{}  Stop", icon::STOP))
                                        .size(12.0)
                                        .color(colors.error),
                                )
                                .stroke(egui::Stroke::new(1.0_f32, colors.error.gamma_multiply(0.6)))
                                .corner_radius(3),
                            )
                            .on_hover_text("Stop the current turn (queued messages are kept)")
                            .clicked()
                        {
                            commands.push(Command::ChatStop(id));
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{ProfileId, ProviderId, RuntimeGeneration, TurnId};

    fn request(native: &str, summary: &str) -> ApprovalRequest {
        ApprovalRequest {
            token: RequestToken {
                provider: ProviderId::Codex,
                profile: ProfileId::default_profile(),
                session: SessionId(9),
                turn: TurnId("turn".into()),
                generation: RuntimeGeneration(1),
                native: native.into(),
            },
            summary: summary.into(),
            command: None,
            paths: vec![],
            choices: vec![],
        }
    }

    #[test]
    fn the_end_of_the_transcript_is_detected_with_slack_for_padding() {
        assert!(at_bottom(2118.0, 500.0, 2620.0));
        assert!(
            at_bottom(2111.0, 500.0, 2620.0),
            "a jump leaves a few pixels"
        );
        assert!(!at_bottom(1900.0, 500.0, 2620.0));
        assert!(
            at_bottom(0.0, 500.0, 300.0),
            "short content is always at the end"
        );
    }

    #[test]
    fn previous_and_next_skip_the_message_already_at_the_top() {
        let tops = [(0, -400.0), (3, -100.0), (5, 100.0), (8, 400.0)];
        assert_eq!(adjacent_messages(&tops, 100.0), (Some(3), Some(8)));
        assert_eq!(
            adjacent_messages(&tops, 103.0),
            (Some(3), Some(8)),
            "within slack"
        );
        assert_eq!(adjacent_messages(&tops, -500.0), (None, Some(0)));
        assert_eq!(adjacent_messages(&tops, 900.0), (Some(8), None));
        assert_eq!(adjacent_messages(&[], 0.0), (None, None));
    }

    fn frame(
        ctx: &egui::Context,
        requests: &[ApprovalRequest],
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<RequestToken>) {
        let mut target = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    target = approval_navigation(ui, requests);
                });
            },
        );
        (output, target)
    }

    fn text_center(output: &egui::FullOutput, needle: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == needle => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{needle} missing"))
    }

    fn click(
        ctx: &egui::Context,
        requests: &[ApprovalRequest],
        pos: egui::Pos2,
    ) -> Option<RequestToken> {
        frame(ctx, requests, vec![egui::Event::PointerMoved(pos)]);
        frame(
            ctx,
            requests,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            ctx,
            requests,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        )
        .1
    }

    #[test]
    fn multiple_approvals_choose_the_exact_token_and_do_not_repeat_navigation() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        let requests = [
            request("first", "First command"),
            request("second", "Second command"),
        ];
        frame(&ctx, &requests, vec![]);
        let (output, _) = frame(&ctx, &requests, vec![]);
        assert!(text_center(&output, &format!("{} 2 approvals needed", icon::WARNING)).x < 240.0);
        assert_eq!(click(&ctx, &requests, text_center(&output, "Review")), None);
        let (output, _) = frame(&ctx, &requests, vec![]);
        let chosen = click(&ctx, &requests, text_center(&output, "2. Second command"));
        assert_eq!(chosen, Some(requests[1].token.clone()));
        assert_eq!(frame(&ctx, &requests, vec![]).1, None);
        assert_eq!(
            frame(&ctx, &[], vec![]).1,
            None,
            "resolved requests cannot retain a target"
        );
    }

    #[test]
    fn review_is_reachable_with_tab_and_enter() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        let requests = [request("first", "First command")];
        frame(&ctx, &requests, vec![]);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &requests, vec![key(egui::Key::Tab)]);
        let (_, target) = frame(&ctx, &requests, vec![key(egui::Key::Enter)]);
        assert_eq!(target, Some(requests[0].token.clone()));
    }

    #[test]
    fn disappearing_approval_between_press_and_release_cannot_navigate() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        let requests = [request("first", "First command")];
        frame(&ctx, &requests, vec![]);
        let (output, _) = frame(&ctx, &requests, vec![]);
        let pos = text_center(&output, "Review");
        frame(&ctx, &requests, vec![egui::Event::PointerMoved(pos)]);
        frame(
            &ctx,
            &requests,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let (_, target) = frame(
            &ctx,
            &[],
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert_eq!(target, None);
    }
}
