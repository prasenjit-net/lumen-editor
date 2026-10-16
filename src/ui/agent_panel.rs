//! Right-hand agent panel: connection status, transcript and composer.
//!
//! Renders `AgentsView` snapshots and emits typed commands; it never touches the
//! process, the files or the clipboard service directly.
use super::{
    Workbench, theme,
    widgets::{HeaderAction, PanelTab, tabbed_panel_header},
};
use crate::{
    app::{
        agent::{AgentKind, AgentSessionView, ConnectionState, UnsavedChoice, display_folder},
        agent_changes::{ChangeAction, RowState, TurnChanges},
        agent_context,
        agent_permission::{PendingPermission, display_path},
        command::Command,
    },
    icon,
    model::{
        line_diff::{self, DiffLine},
        markdown_lite::{self, Block, Inline},
    },
};
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Layout, Margin, RichText, ScrollArea, SidePanel,
    TextFormat, TopBottomPanel, text::LayoutJob,
};
use lumen_agent_protocol::{
    reducer::{Entry, Role},
    types::{PermissionKind, PlanStatus, ToolCall, ToolContent, ToolKind, ToolStatus},
};

const DEFAULT_WIDTH: f32 = 380.0;
const MIN_WIDTH: f32 = 280.0;
const MAX_WIDTH: f32 = 760.0;
const TEXT_SIZE: f32 = 12.5;

impl Workbench<'_> {
    pub(crate) fn agent_panel(&mut self, ctx: &egui::Context) {
        if !self.view.agents.open || !self.view.settings.boolean("agent.enabled") {
            return;
        }
        let active = self.view.agents.active;
        SidePanel::right("agent_panel")
            .default_width(DEFAULT_WIDTH)
            .width_range(MIN_WIDTH..=MAX_WIDTH)
            .resizable(true)
            .frame(Frame::default().fill(theme::PANEL))
            .show(ctx, |ui| {
                self.agent_header(ui, active);
                let root = self.editor.workspace.clone();
                let folder = self
                    .view
                    .agents
                    .session(active)
                    .cwd
                    .as_deref()
                    .map(display_folder);
                let session = self.view.agents.session(active);
                let mut commands = Vec::new();
                status_strip(ui, session, folder.as_deref(), &mut commands);
                let running = session.running;
                let sendable =
                    !session.draft.trim().is_empty() && !running && session.held_prompt.is_none();
                let paths = &self.view.navigation.paths;
                TopBottomPanel::bottom("agent_composer")
                    .frame(
                        Frame::default()
                            .fill(theme::PANEL)
                            .inner_margin(Margin::symmetric(10, 8)),
                    )
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        let session = self.view.agents.session_mut(active);
                        unsaved_card(ui, session, &mut commands);
                        changes_card(ui, session, root.as_deref(), &mut commands);
                        permission_cards(ui, session, root.as_deref(), &mut commands);
                        composer(
                            ui,
                            session,
                            running,
                            sendable,
                            root.as_deref(),
                            paths,
                            &mut commands,
                        );
                    });
                egui::CentralPanel::default()
                    .frame(
                        Frame::default()
                            .fill(theme::BG)
                            .inner_margin(Margin::symmetric(10, 6)),
                    )
                    .show_inside(ui, |ui| {
                        let session = self.view.agents.session_mut(active);
                        transcript(ui, session);
                    });
                for command in commands {
                    self.command(command);
                }
            });
    }

    fn agent_header(&mut self, ui: &mut egui::Ui, active: AgentKind) {
        let tabs = AgentKind::ALL.map(|kind| PanelTab {
            label: kind.label(),
            active: kind == active,
            command: Command::SelectAgent(kind),
        });
        let actions = [
            HeaderAction {
                glyph: icon::PLUS,
                tooltip: "New session (discards this conversation)",
                command: Command::NewAgentSession,
                enabled: true,
            },
            HeaderAction {
                glyph: icon::X,
                tooltip: "Close agent panel",
                command: Command::ToggleAgentPanel,
                enabled: true,
            },
        ];
        if let Some(command) = tabbed_panel_header(ui, &tabs, &actions) {
            self.command(command);
        }
    }
}

fn status_strip(
    ui: &mut egui::Ui,
    session: &AgentSessionView,
    folder: Option<&str>,
    commands: &mut Vec<Command>,
) {
    let (dot, text) = match &session.state {
        ConnectionState::Idle => (theme::MUTED, "Starts when you send a message".to_owned()),
        ConnectionState::Starting => (
            Color32::from_rgb(204, 167, 0),
            "Starting… the first run through npx can take several minutes".to_owned(),
        ),
        ConnectionState::Ready => (
            Color32::from_rgb(35, 209, 139),
            match (&session.agent_name, folder) {
                (Some(name), Some(folder)) => format!("{name} · {folder}"),
                (Some(name), None) => name.clone(),
                _ => "Ready".to_owned(),
            },
        ),
        ConnectionState::NeedsAuth => (
            Color32::from_rgb(204, 167, 0),
            session.kind.sign_in_hint().to_owned(),
        ),
        ConnectionState::NotFound(message) => (theme::DIFF_REMOVED_TEXT, message.clone()),
        ConnectionState::Failed(error) => (theme::DIFF_REMOVED_TEXT, error.clone()),
    };
    Frame::default()
        .inner_margin(Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("●").size(10.0).color(dot));
                ui.add(egui::Label::new(RichText::new(text).size(11.0).color(theme::MUTED)).wrap());
            });
            match &session.state {
                ConnectionState::NeedsAuth => {
                    ui.horizontal_wrapped(|ui| {
                        for method in &session.auth_methods {
                            if ui
                                .small_button(&method.name)
                                .on_hover_text(method.description.clone().unwrap_or_default())
                                .clicked()
                            {
                                commands.push(Command::AuthenticateAgent(method.id.clone()));
                            }
                        }
                        if ui.small_button("Retry").on_hover_text("Try again after signing in").clicked() {
                            commands.push(Command::RetryAgent);
                        }
                    });
                }
                ConnectionState::NotFound(_) => {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button("Download with npx…")
                            .on_hover_text("Runs npx -y <adapter>; downloads several hundred MB the first time")
                            .clicked()
                        {
                            commands.push(Command::AllowAgentNpx);
                        }
                        if ui.small_button("Copy install command").clicked() {
                            ui.ctx().copy_text(format!(
                                "npm install -g {}",
                                session
                                    .kind
                                    .npx_package()
                                    .rsplit_once('@')
                                    .map_or(session.kind.npx_package(), |(name, _)| name)
                            ));
                        }
                    });
                }
                ConnectionState::Failed(_) if ui.small_button("Retry").clicked() => {
                    commands.push(Command::RetryAgent);
                }
                _ => {}
            }
            ui.label(
                RichText::new("Edits and commands ask for your approval first.")
                    .size(10.5)
                    .color(theme::MUTED),
            );
        });
    ui.separator();
}

/// Asked when the message is held back because editor tabs have unsaved text.
fn unsaved_card(ui: &mut egui::Ui, session: &AgentSessionView, commands: &mut Vec<Command>) {
    let Some(held) = &session.held_prompt else {
        return;
    };
    Frame::default()
        .fill(theme::BG)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(204, 167, 0)))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let names = held.files.join(", ");
            ui.label(
                RichText::new(format!(
                    "⚠ {} unsaved file(s): {names}. The agent reads files from disk and will not see these edits.",
                    held.files.len()
                ))
                .size(11.0)
                .color(theme::TEXT),
            );
            ui.horizontal_wrapped(|ui| {
                let mut choice = |ui: &mut egui::Ui, label: &str, hint: &str, choice| {
                    if ui.button(label).on_hover_text(hint).clicked() {
                        commands.push(Command::ResolveUnsavedFiles(choice));
                    }
                };
                choice(ui, "Save all & send", "Save the open files, then send", UnsavedChoice::SaveAll);
                if held.can_attach {
                    choice(ui, "Attach & send", "Send the unsaved text to the agent as context", UnsavedChoice::Attach);
                }
                choice(ui, "Send anyway", "The agent will only see what is on disk", UnsavedChoice::SendAnyway);
                choice(ui, "Cancel", "Put the message back in the box", UnsavedChoice::Cancel);
            });
        });
    ui.add_space(6.0);
}

/// "Changed by the agent": a summary row that expands into per-file diffs with
/// Open / Revert, plus Revert all and Keep (dismiss).
fn changes_card(
    ui: &mut egui::Ui,
    session: &AgentSessionView,
    root: Option<&std::path::Path>,
    commands: &mut Vec<Command>,
) {
    let kind = session.kind;
    if session.preparing || session.finishing {
        ui.label(
            RichText::new(if session.preparing {
                "Preparing change tracking…"
            } else {
                "Comparing files…"
            })
            .size(10.5)
            .color(theme::MUTED),
        );
    }
    let Some(changes) = &session.changes else {
        return;
    };
    let (mut added, mut removed) = (0, 0);
    for row in &changes.rows {
        let diff = line_diff::diff(
            row.change.before.as_deref().unwrap_or(""),
            row.change.after.as_deref().unwrap_or(""),
            0,
        );
        added += diff.added;
        removed += diff.removed;
    }
    let pending = changes
        .rows
        .iter()
        .filter(|row| row.state == RowState::Pending)
        .count();
    Frame::default()
        .fill(theme::BG)
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let arrow = if changes.open {
                    icon::CARET_DOWN
                } else {
                    icon::CARET_RIGHT
                };
                if ui
                    .add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{arrow} Changed by the agent: {} file(s)  +{added} −{removed}",
                                changes.rows.len()
                            ))
                            .size(11.5)
                            .color(theme::TEXT),
                        )
                        .selectable(false)
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Show or hide the changed files")
                    .clicked()
                {
                    commands.push(Command::AgentChanges(kind, ChangeAction::ToggleOpen));
                }
                if pending > 0
                    && ui
                        .small_button("Revert all")
                        .on_hover_text("Restore every text file to how it was before this turn")
                        .clicked()
                {
                    commands.push(Command::AgentChanges(kind, ChangeAction::RevertAll));
                }
                if ui
                    .small_button("Keep")
                    .on_hover_text("Accept the changes and hide this")
                    .clicked()
                {
                    commands.push(Command::AgentChanges(kind, ChangeAction::Dismiss));
                }
            });
            if changes.truncated {
                ui.label(
                    RichText::new("⚠ This folder is large; some changes may not be listed.")
                        .size(10.5)
                        .color(Color32::from_rgb(204, 167, 0)),
                );
            }
            if changes.open {
                change_rows(ui, kind, changes, root, commands);
            }
        });
    ui.add_space(6.0);
}

fn change_rows(
    ui: &mut egui::Ui,
    kind: AgentKind,
    changes: &TurnChanges,
    root: Option<&std::path::Path>,
    commands: &mut Vec<Command>,
) {
    use crate::services::workspace_snapshot::ChangeKind;
    ScrollArea::vertical()
        .id_salt(("agent_changes", kind))
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, row) in changes.rows.iter().take(200).enumerate() {
                ui.push_id(index, |ui| {
                    let (mark, color) = match row.change.kind {
                        ChangeKind::Added => ("A", theme::DIFF_ADDED_TEXT),
                        ChangeKind::Modified => ("M", Color32::from_rgb(204, 167, 0)),
                        ChangeKind::Deleted => ("D", theme::DIFF_REMOVED_TEXT),
                    };
                    let path = row.change.path.display().to_string();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).monospace().size(11.0).color(color));
                        let label = display_path(&path, root);
                        if ui
                            .add(
                                egui::Label::new(
                                    RichText::new(label).size(11.0).color(theme::TEXT),
                                )
                                .selectable(false)
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Show the diff")
                            .clicked()
                        {
                            commands.push(Command::AgentChanges(
                                kind,
                                ChangeAction::ToggleFile(row.change.path.clone()),
                            ));
                        }
                        match &row.state {
                            RowState::Pending => {
                                if row.change.kind != ChangeKind::Deleted
                                    && ui.small_button("Open").clicked()
                                {
                                    commands.push(Command::AgentChanges(
                                        kind,
                                        ChangeAction::OpenFile(row.change.path.clone()),
                                    ));
                                }
                                if ui
                                    .add_enabled(
                                        !row.change.opaque,
                                        egui::Button::new("Revert").small(),
                                    )
                                    .on_disabled_hover_text(
                                        "Binary or large files cannot be reverted here",
                                    )
                                    .clicked()
                                {
                                    commands.push(Command::AgentChanges(
                                        kind,
                                        ChangeAction::Revert(row.change.path.clone()),
                                    ));
                                }
                            }
                            RowState::Reverted => {
                                ui.label(RichText::new("reverted").size(10.5).color(theme::MUTED));
                            }
                            RowState::Failed(reason) => {
                                ui.label(
                                    RichText::new(format!("not reverted: {reason}"))
                                        .size(10.5)
                                        .color(theme::DIFF_REMOVED_TEXT),
                                );
                            }
                        }
                    });
                    if row.expanded {
                        if row.change.opaque {
                            ui.label(
                                RichText::new("Binary or large file")
                                    .size(10.5)
                                    .color(theme::MUTED),
                            );
                        } else {
                            diff_view(
                                ui,
                                row.change.before.as_deref(),
                                row.change.after.as_deref().unwrap_or(""),
                                &path,
                            );
                        }
                    }
                });
            }
            if changes.rows.len() > 200 {
                ui.label(
                    RichText::new(format!("… and {} more files", changes.rows.len() - 200))
                        .size(10.5)
                        .color(theme::MUTED),
                );
            }
        });
}

/// The oldest unanswered request, with the rest counted. Keyboard: Tab / Enter
/// on the options, Esc declines.
fn permission_cards(
    ui: &mut egui::Ui,
    session: &mut AgentSessionView,
    root: Option<&std::path::Path>,
    commands: &mut Vec<Command>,
) {
    let kind = session.kind;
    let waiting = session.pending_permissions.len();
    let Some(pending) = session.pending_permissions.first().cloned() else {
        session.permission_focus = None;
        return;
    };
    let focus_first = session.permission_focus.as_ref() != Some(&pending.id);
    session.permission_focus = Some(pending.id.clone());
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        commands.push(Command::AnswerAgentPermission(
            kind,
            pending.id.clone(),
            None,
        ));
        return;
    }
    Frame::default()
        .fill(theme::BG)
        .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(204, 167, 0)))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            permission_body(ui, &pending, root, waiting);
            let mut options = pending.request.options.clone();
            options.sort_by_key(|option| match option.kind {
                PermissionKind::AllowOnce => 0,
                PermissionKind::AllowAlways => 1,
                PermissionKind::RejectOnce => 2,
                PermissionKind::RejectAlways => 3,
            });
            ui.horizontal_wrapped(|ui| {
                for (index, option) in options.iter().enumerate() {
                    let label = match option.kind {
                        PermissionKind::AllowOnce | PermissionKind::AllowAlways => {
                            format!("{} {}", icon::CHECK, option.name)
                        }
                        _ => format!("{} {}", icon::X, option.name),
                    };
                    let response = ui.button(label);
                    if index == 0 && focus_first {
                        response.request_focus();
                    }
                    if response.clicked() {
                        commands.push(Command::AnswerAgentPermission(
                            kind,
                            pending.id.clone(),
                            Some(option.id.clone()),
                        ));
                    }
                }
            });
        });
    ui.add_space(6.0);
}

fn permission_body(
    ui: &mut egui::Ui,
    pending: &PendingPermission,
    root: Option<&std::path::Path>,
    waiting: usize,
) {
    let call = &pending.request.tool_call;
    let title = if call.title.is_empty() {
        "Permission requested"
    } else {
        &call.title
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(kind_glyph(call.kind))
                .size(12.0)
                .color(theme::MUTED),
        );
        ui.label(RichText::new(title).strong().size(12.0).color(theme::TEXT));
        if waiting > 1 {
            ui.label(
                RichText::new(format!("(+{} waiting)", waiting - 1))
                    .size(10.5)
                    .color(theme::MUTED),
            );
        }
    });
    for warning in &pending.warnings {
        ui.label(
            RichText::new(format!("⚠ {warning}"))
                .size(11.0)
                .color(Color32::from_rgb(204, 167, 0)),
        );
    }
    for location in &call.locations {
        let line = location
            .line
            .map_or(String::new(), |line| format!(":{line}"));
        ui.label(
            RichText::new(format!("{}{line}", display_path(&location.path, root)))
                .size(10.5)
                .color(theme::MUTED),
        );
    }
    if let Some(command) = call
        .raw_input
        .as_ref()
        .and_then(|input| input.get("command"))
        .and_then(|command| command.as_str())
    {
        ui.add(
            egui::Label::new(
                RichText::new(format!("$ {command}"))
                    .monospace()
                    .size(11.0)
                    .color(theme::TEXT),
            )
            .selectable(true),
        );
    }
    for content in &call.content {
        if let ToolContent::Diff {
            path,
            old_text,
            new_text,
        } = content
        {
            ui.label(
                RichText::new(display_path(path, root))
                    .size(10.5)
                    .color(theme::MUTED),
            );
            diff_view(ui, old_text.as_deref(), new_text, path);
        }
    }
}

/// Unified diff with green/red rows; long diffs scroll and are capped.
fn diff_view(ui: &mut egui::Ui, old: Option<&str>, new: &str, salt: &str) {
    const MAX_ROWS: usize = 400;
    let diff = line_diff::diff(old.unwrap_or(""), new, 2);
    ui.label(
        RichText::new(format!("+{} −{}", diff.added, diff.removed))
            .size(10.5)
            .color(theme::MUTED),
    );
    if diff.lines.is_empty() {
        ui.label(RichText::new("No changes").size(10.5).color(theme::MUTED));
        return;
    }
    let font = FontId::monospace(11.0);
    ScrollArea::both()
        .id_salt(("agent_diff", salt))
        .max_height(180.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for line in diff.lines.iter().take(MAX_ROWS) {
                let (text, color, background) = match line {
                    DiffLine::Added(text) => (
                        format!("+ {text}"),
                        theme::DIFF_ADDED_TEXT,
                        Some(theme::DIFF_ADDED_BG),
                    ),
                    DiffLine::Removed(text) => (
                        format!("- {text}"),
                        theme::DIFF_REMOVED_TEXT,
                        Some(theme::DIFF_REMOVED_BG),
                    ),
                    DiffLine::Context(text) => (format!("  {text}"), theme::MUTED, None),
                    DiffLine::Gap(count) => {
                        (format!("  … {count} unchanged lines"), theme::MUTED, None)
                    }
                };
                let galley = ui.painter().layout_no_wrap(text, font.clone(), color);
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(
                        galley.size().x.max(ui.available_width()),
                        galley.size().y + 1.0,
                    ),
                    egui::Sense::hover(),
                );
                if let Some(background) = background {
                    ui.painter().rect_filled(rect, 0, background);
                }
                ui.painter().galley(rect.left_top(), galley, color);
            }
            if diff.lines.len() > MAX_ROWS {
                ui.label(
                    RichText::new(format!("… {} more lines", diff.lines.len() - MAX_ROWS))
                        .size(10.5)
                        .color(theme::MUTED),
                );
            }
        });
}

/// Suggestions shown above the box while an `@file` or `/command` is being typed.
enum Popup {
    None,
    /// Folder-relative paths.
    Mentions(Vec<String>),
    /// (name, description).
    Commands(Vec<(String, String)>),
}

impl Popup {
    fn len(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Mentions(items) => items.len(),
            Self::Commands(items) => items.len(),
        }
    }
}

fn popup_for(
    session: &AgentSessionView,
    root: Option<&std::path::Path>,
    paths: &[std::path::PathBuf],
) -> Popup {
    if let (Some((_, query)), Some(root)) = (agent_context::active_mention(&session.draft), root) {
        let found = agent_context::mention_candidates(paths, root, query, 8);
        if !found.is_empty() {
            return Popup::Mentions(found);
        }
    }
    let commands = agent_context::slash_matches(&session.conversation.commands, &session.draft);
    if commands.is_empty() {
        Popup::None
    } else {
        Popup::Commands(
            commands
                .into_iter()
                .take(8)
                .map(|command| (command.name.clone(), command.description.clone()))
                .collect(),
        )
    }
}

fn composer(
    ui: &mut egui::Ui,
    session: &mut AgentSessionView,
    running: bool,
    sendable: bool,
    root: Option<&std::path::Path>,
    paths: &[std::path::PathBuf],
    commands: &mut Vec<Command>,
) {
    let kind = session.kind;
    attachment_chips(ui, session, commands);
    config_row(ui, session, commands);
    let id = egui::Id::new(("agent_composer_input", kind));
    let focused = ui.ctx().memory(|memory| memory.has_focus(id));
    let popup = popup_for(session, root, paths);
    let popup_len = popup.len();
    let selected_slot = match popup {
        Popup::Mentions(_) => &mut session.mention_selected,
        _ => &mut session.slash_selected,
    };
    *selected_slot = (*selected_slot).min(popup_len.saturating_sub(1));
    let mut chosen = None;
    if focused && popup_len > 0 {
        // Navigation and completion keys belong to the suggestion list while it is open.
        let key =
            |ui: &mut egui::Ui, key| ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
        if key(ui, egui::Key::ArrowDown) {
            *selected_slot = (*selected_slot + 1) % popup_len;
        }
        if key(ui, egui::Key::ArrowUp) {
            *selected_slot = (*selected_slot + popup_len - 1) % popup_len;
        }
        if key(ui, egui::Key::Tab) || take_plain_enter(ui) {
            chosen = Some(*selected_slot);
        }
    } else if focused && sendable && take_plain_enter(ui) {
        // Enter sends; Shift+Enter inserts a newline. Consume before the widget sees it.
        commands.push(Command::SendAgentPrompt);
    }
    let selected = *selected_slot;
    suggestion_list(ui, kind, &popup, selected, &mut chosen);
    if let Some(index) = chosen {
        match &popup {
            Popup::Mentions(items) => {
                commands.push(Command::PickAgentMention(kind, items[index].clone()));
            }
            Popup::Commands(items) => {
                commands.push(Command::PickSlash(kind, items[index].0.clone()));
            }
            Popup::None => {}
        }
    }
    let response = ui.add(
        egui::TextEdit::multiline(&mut session.draft)
            .id(id)
            .hint_text(format!(
                "Message {}… (@ to mention a file, / for commands)",
                kind.label()
            ))
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .font(FontId::proportional(TEXT_SIZE)),
    );
    if session.focus_composer {
        response.request_focus();
        session.focus_composer = false;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Enter to send · Shift+Enter for a new line")
                .size(10.5)
                .color(theme::MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if running {
                if ui
                    .button(format!("{} Stop", icon::STOP))
                    .on_hover_text("Cancel the current turn")
                    .clicked()
                {
                    commands.push(Command::CancelAgentTurn);
                }
            } else if ui
                .add_enabled(sendable, egui::Button::new(format!("{} Send", icon::SEND)))
                .on_hover_text("Send message (Enter)")
                .clicked()
            {
                commands.push(Command::SendAgentPrompt);
            }
        });
    });
}

fn suggestion_list(
    ui: &mut egui::Ui,
    kind: AgentKind,
    popup: &Popup,
    selected: usize,
    chosen: &mut Option<usize>,
) {
    let rows: Vec<(String, String)> = match popup {
        Popup::None => return,
        Popup::Mentions(items) => items
            .iter()
            .map(|item| (format!("@{item}"), String::new()))
            .collect(),
        Popup::Commands(items) => items
            .iter()
            .map(|(name, description)| (format!("/{name}"), description.clone()))
            .collect(),
    };
    Frame::default()
        .fill(theme::BG)
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(4, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, (label, hint)) in rows.iter().enumerate() {
                let text = if hint.is_empty() {
                    label.clone()
                } else {
                    format!("{label}  {hint}")
                };
                let response = ui.add(
                    egui::Button::new(
                        RichText::new(text)
                            .monospace()
                            .size(11.0)
                            .color(theme::TEXT),
                    )
                    .fill(if index == selected {
                        theme::ACTIVE
                    } else {
                        Color32::TRANSPARENT
                    })
                    .frame(index == selected)
                    .min_size(egui::vec2(ui.available_width(), 0.0)),
                );
                if response.clicked() {
                    *chosen = Some(index);
                }
            }
        });
    let _ = kind;
    ui.add_space(4.0);
}

/// Selections and files waiting to go with the next message.
fn attachment_chips(ui: &mut egui::Ui, session: &AgentSessionView, commands: &mut Vec<Command>) {
    if session.attachments.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (index, chip) in session.attachments.iter().enumerate() {
            Frame::default()
                .fill(theme::INPUT)
                .corner_radius(3)
                .inner_margin(Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} {}", icon::FILE, chip.label))
                                .size(11.0)
                                .color(theme::TEXT),
                        );
                        if ui
                            .add(egui::Button::new(icon::X).small().frame(false))
                            .on_hover_text("Remove from this message")
                            .clicked()
                        {
                            commands.push(Command::RemoveAgentAttachment(session.kind, index));
                        }
                    });
                });
        }
    });
    ui.add_space(4.0);
}

/// Mode / model / effort dropdowns from the agent's own `configOptions`.
fn config_row(ui: &mut egui::Ui, session: &AgentSessionView, commands: &mut Vec<Command>) {
    let options: Vec<_> = session
        .conversation
        .config_options
        .iter()
        .filter(|option| option.kind == "select" && !option.values.is_empty())
        .collect();
    if options.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for option in options {
            let current = option
                .values
                .iter()
                .find(|value| value.value == option.current)
                .map_or(option.current.as_str(), |value| value.name.as_str());
            egui::ComboBox::from_id_salt(("agent_config", session.kind, &option.id))
                .selected_text(RichText::new(format!("{}: {current}", option.name)).size(10.5))
                .width(130.0)
                .show_ui(ui, |ui| {
                    for value in &option.values {
                        let response =
                            ui.selectable_label(value.value == option.current, &value.name);
                        let response = match &value.description {
                            Some(description) => response.on_hover_text(description),
                            None => response,
                        };
                        if response.clicked() {
                            commands.push(Command::SetAgentConfig(
                                session.kind,
                                option.id.clone(),
                                value.value.clone(),
                            ));
                        }
                    }
                });
        }
    });
    if let Some((_, _, label)) = &session.pending_config {
        Frame::default()
            .fill(theme::BG)
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(204, 167, 0)))
            .corner_radius(3)
            .inner_margin(Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!(
                        "⚠ “{label}” lets the agent act without asking first. Edits still show up under “Changed by the agent”."
                    ))
                    .size(11.0)
                    .color(theme::TEXT),
                );
                ui.horizontal(|ui| {
                    if ui.button("Enable").clicked() {
                        commands.push(Command::ConfirmAgentConfig(true));
                    }
                    if ui.button("Cancel").clicked() {
                        commands.push(Command::ConfirmAgentConfig(false));
                    }
                });
            });
    }
    ui.add_space(4.0);
}

/// Remove an unmodified Enter key press. `consume_key` would also match
/// Shift+Enter (egui treats an extra Shift as compatible), and that must stay a
/// newline.
fn take_plain_enter(ui: &mut egui::Ui) -> bool {
    ui.input_mut(|input| {
        let position = input.events.iter().position(|event| {
            matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. }
                if !modifiers.shift && !modifiers.alt && !modifiers.command && !modifiers.ctrl)
        });
        position.map(|index| input.events.remove(index)).is_some()
    })
}

fn transcript(ui: &mut egui::Ui, session: &mut AgentSessionView) {
    let follow = std::mem::take(&mut session.follow_output);
    ScrollArea::vertical()
        .id_salt(("agent_transcript", session.kind))
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            if session.conversation.entries.is_empty() {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("Ask {} about your code.", session.kind.label()))
                        .size(11.5)
                        .color(theme::MUTED),
                );
            }
            plan(ui, session);
            for (index, entry) in session.conversation.entries.iter().enumerate() {
                ui.push_id(index, |ui| match entry {
                    Entry::Message {
                        role: Role::User,
                        text,
                    } => user_message(ui, text),
                    Entry::Message {
                        role: Role::Agent,
                        text,
                    } => agent_message(ui, text),
                    Entry::Message {
                        role: Role::Thought,
                        text,
                    } => thought(ui, text),
                    Entry::ToolCall(call) => tool_call(ui, call),
                    Entry::Notice(text) => {
                        ui.label(RichText::new(text).italics().size(11.0).color(theme::MUTED));
                    }
                });
            }
            if session.running {
                ui.label(RichText::new("Working…").size(11.0).color(theme::MUTED));
            }
            if follow {
                ui.scroll_to_cursor(Some(Align::BOTTOM));
            }
        });
}

fn plan(ui: &mut egui::Ui, session: &AgentSessionView) {
    if session.conversation.plan.is_empty() {
        return;
    }
    ui.label(RichText::new("PLAN").size(10.5).color(theme::MUTED));
    for entry in &session.conversation.plan {
        let mark = match entry.status {
            PlanStatus::Completed => "☑",
            PlanStatus::InProgress => "▶",
            PlanStatus::Pending => "☐",
        };
        ui.label(
            RichText::new(format!("{mark} {}", entry.content))
                .size(11.5)
                .color(theme::TEXT),
        );
    }
    ui.separator();
}

fn user_message(ui: &mut egui::Ui, text: &str) {
    Frame::default()
        .fill(theme::INPUT)
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(RichText::new(text).size(TEXT_SIZE).color(theme::TEXT))
                    .selectable(true),
            );
        });
}

fn agent_message(ui: &mut egui::Ui, text: &str) {
    for block in markdown_lite::parse(text) {
        match block {
            Block::Paragraph(inlines) => {
                ui.add(egui::Label::new(job(&inlines, TEXT_SIZE, theme::TEXT)).selectable(true));
            }
            Block::Heading(inlines) => {
                ui.add(
                    egui::Label::new(job(&inlines, TEXT_SIZE + 1.5, Color32::WHITE))
                        .selectable(true),
                );
            }
            Block::Item { marker, content } => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(marker).size(TEXT_SIZE).color(theme::MUTED));
                    ui.add(
                        egui::Label::new(job(&content, TEXT_SIZE, theme::TEXT)).selectable(true),
                    );
                });
            }
            Block::Code { language, text } => code_block(ui, &language, &text),
        }
    }
}

fn code_block(ui: &mut egui::Ui, language: &str, text: &str) {
    Frame::default()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if language.is_empty() {
                        "code"
                    } else {
                        language
                    })
                    .size(10.0)
                    .color(theme::MUTED),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button(icon::COPY)
                        .on_hover_text("Copy code")
                        .clicked()
                    {
                        ui.ctx().copy_text(text.to_owned());
                    }
                });
            });
            ScrollArea::horizontal()
                .id_salt(("code", text.len(), language))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .monospace()
                                .size(11.5)
                                .color(theme::TEXT),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(true),
                    );
                });
        });
}

fn thought(ui: &mut egui::Ui, text: &str) {
    egui::CollapsingHeader::new(RichText::new("Thinking").size(11.0).color(theme::MUTED))
        .id_salt("thought")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(text).italics().size(11.0).color(theme::MUTED))
                    .selectable(true),
            );
        });
}

fn tool_call(ui: &mut egui::Ui, call: &ToolCall) {
    let (status, color) = match call.status {
        ToolStatus::Pending => ("waiting", theme::MUTED),
        ToolStatus::InProgress => ("running", Color32::from_rgb(204, 167, 0)),
        ToolStatus::Completed => ("done", theme::MUTED),
        ToolStatus::Failed => ("failed", theme::DIFF_REMOVED_TEXT),
    };
    let title = if call.title.is_empty() {
        "Tool call"
    } else {
        &call.title
    };
    egui::CollapsingHeader::new(
        RichText::new(format!("{} {title}  ·  {status}", kind_glyph(call.kind)))
            .size(11.0)
            .color(color),
    )
    .id_salt("tool")
    .default_open(false)
    .show(ui, |ui| {
        for location in &call.locations {
            let line = location
                .line
                .map_or(String::new(), |line| format!(":{line}"));
            ui.label(
                RichText::new(format!("{}{line}", location.path))
                    .size(10.5)
                    .color(theme::MUTED),
            );
        }
        for content in &call.content {
            match content {
                ToolContent::Text(text) => {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .monospace()
                                .size(11.0)
                                .color(theme::TEXT),
                        )
                        .selectable(true),
                    );
                }
                ToolContent::Diff {
                    path,
                    old_text,
                    new_text,
                } => {
                    let added = new_text.lines().count();
                    let removed = old_text.as_deref().map_or(0, |old| old.lines().count());
                    ui.label(
                        RichText::new(format!("Edit {path}: +{added} −{removed} lines"))
                            .size(11.0)
                            .color(theme::TEXT),
                    );
                }
                ToolContent::Terminal { .. } => {
                    ui.label(
                        RichText::new("Command output is shown by the agent.")
                            .size(10.5)
                            .color(theme::MUTED),
                    );
                }
                ToolContent::Other(kind) => {
                    ui.label(
                        RichText::new(format!("[{kind}]"))
                            .size(10.5)
                            .color(theme::MUTED),
                    );
                }
            }
        }
        if let Some(command) = call
            .raw_input
            .as_ref()
            .and_then(|input| input.get("command"))
            .and_then(|command| command.as_str())
        {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("$ {command}"))
                        .monospace()
                        .size(11.0)
                        .color(theme::TEXT),
                )
                .selectable(true),
            );
        }
    });
}

fn kind_glyph(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read | ToolKind::Search | ToolKind::Fetch => icon::MAGNIFYING_GLASS,
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => icon::FILE_CODE,
        ToolKind::Execute => icon::TERMINAL,
        ToolKind::Think | ToolKind::SwitchMode | ToolKind::Other => icon::GEAR,
    }
}

fn job(inlines: &[Inline], size: f32, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for inline in inlines {
        let (text, format) = match inline {
            Inline::Text(text) => (
                text,
                TextFormat {
                    font_id: FontId::proportional(size),
                    color,
                    ..Default::default()
                },
            ),
            Inline::Bold(text) => (
                text,
                TextFormat {
                    font_id: FontId::proportional(size),
                    color: Color32::WHITE,
                    ..Default::default()
                },
            ),
            Inline::Code(text) => (
                text,
                TextFormat {
                    font_id: FontId::monospace(size - 1.0),
                    color,
                    background: theme::INPUT,
                    ..Default::default()
                },
            ),
        };
        job.append(text, 0.0, format);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{LumenApp, agent::AgentKind, agent_changes::RowState},
        services::agent_process::fake::FakeAgentLauncher,
    };
    use lumen_agent_protocol::{
        reducer::Entry,
        types::{ToolCall, ToolStatus, Update},
    };
    use std::sync::Arc;

    struct Panel {
        app: LumenApp,
        fake: FakeAgentLauncher,
        ctx: egui::Context,
        width: f32,
    }

    impl Panel {
        fn new() -> Self {
            let fake = FakeAgentLauncher::default();
            let mut app = LumenApp::default();
            app.agent_runtime.launcher = Arc::new(fake.clone());
            let program = std::env::current_exe().unwrap().display().to_string();
            for kind in AgentKind::ALL {
                app.view.settings.user.persisted.insert(
                    format!("agent.{}.command", kind.id()),
                    lumen_settings_core::Value::Text(program.clone()),
                );
            }
            app.view.agents.open = true;
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            Self {
                app,
                fake,
                ctx,
                width: 1000.0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let app = &mut self.app;
            self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(self.width, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.agent_panel(ctx)),
            )
        }

        fn texts(&mut self) -> String {
            // Panels size themselves from the previous frame; let layout settle.
            self.frame(vec![]);
            self.frame(vec![]);
            let output = self.frame(vec![]);
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// Like `click_text`, but picks the lowest match on screen (header buttons
        /// share glyphs with chip buttons).
        fn click_last_text(&mut self, label: &str) {
            self.frame(vec![]);
            self.frame(vec![]);
            let output = self.frame(vec![]);
            let pos = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                    }
                    _ => None,
                })
                .max_by(|a, b| a.y.total_cmp(&b.y))
                .unwrap_or_else(|| panic!("{label:?} is visible"));
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        }

        fn click_text(&mut self, label: &str) {
            self.frame(vec![]);
            self.frame(vec![]);
            let output = self.frame(vec![]);
            let pos = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label:?} is visible"));
            for pressed in [true, false] {
                self.frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
        }

        fn state(&self) -> crate::app::agent::ConnectionState {
            self.app
                .view
                .agents
                .session(self.app.view.agents.active)
                .state
                .clone()
        }
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn closed_panel_draws_nothing_and_open_panel_shows_tabs_composer_and_state() {
        let mut panel = Panel::new();
        panel.app.view.agents.open = false;
        assert!(panel.texts().is_empty());
        panel.app.view.agents.open = true;
        let shown = panel.texts();
        for expected in [
            "Claude Code",
            "Codex",
            "Starts when you send a message",
            "Enter to send",
            "Ask Claude Code about your code.",
        ] {
            assert!(shown.contains(expected), "missing {expected:?} in {shown}");
        }
        assert!(shown.contains("ask for your approval"));
    }

    #[test]
    fn clicking_a_tab_switches_agents_and_keeps_each_draft() {
        let mut panel = Panel::new();
        panel.app.view.agents.session_mut(AgentKind::Claude).draft = "for claude".into();
        panel.frame(vec![]);
        panel.click_text("Codex");
        assert_eq!(panel.app.view.agents.active, AgentKind::Codex);
        assert!(panel.texts().contains("Ask Codex about your code."));
        panel.click_text("Claude Code");
        assert_eq!(
            panel.app.view.agents.session(AgentKind::Claude).draft,
            "for claude"
        );
    }

    #[test]
    fn enter_sends_and_shift_enter_does_not() {
        let mut panel = Panel::new();
        panel
            .app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .focus_composer = true;
        panel.frame(vec![]);
        panel.app.view.agents.session_mut(AgentKind::Claude).draft = "hello agent".into();
        panel.frame(vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)]);
        assert!(
            panel.fake.launched.lock().unwrap().is_empty(),
            "Shift+Enter is a newline"
        );
        panel.frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(panel.fake.launched.lock().unwrap().len(), 1);
        let session = panel.app.view.agents.session(AgentKind::Claude);
        assert!(session.draft.is_empty(), "the message left the composer");
        assert!(
            matches!(session.conversation.entries.first(), Some(Entry::Message { text, .. }) if text == "hello agent")
        );
    }

    #[test]
    fn the_send_button_is_disabled_for_blank_drafts_and_stop_replaces_it_while_running() {
        let mut panel = Panel::new();
        let shown = panel.texts();
        assert!(shown.contains("Send") && !shown.contains("Stop"));
        panel.click_text("Send");
        assert!(
            panel.fake.launched.lock().unwrap().is_empty(),
            "a blank draft cannot be sent"
        );
        panel.app.view.agents.session_mut(AgentKind::Claude).running = true;
        let shown = panel.texts();
        assert!(shown.contains("Stop") && !shown.contains(&format!("{} Send", crate::icon::SEND)));
        assert!(shown.contains("Working…"));
    }

    #[test]
    fn transcript_renders_messages_markdown_tool_calls_plans_and_notices() {
        let mut panel = Panel::new();
        let conversation = &mut panel
            .app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .conversation;
        conversation.push_user_message("what does main do?");
        for update in [
            Update::AgentMessage(lumen_agent_protocol::types::ContentBlock::Text(
                "It calls `run()`.\n\n```rust\nfn main() { run(); }\n```\n- first\n- second".into(),
            )),
            Update::ToolCall(ToolCall {
                id: "t1".into(),
                title: "Read main.rs".into(),
                status: ToolStatus::Completed,
                ..ToolCall::default()
            }),
            Update::Plan(vec![lumen_agent_protocol::types::PlanEntry {
                content: "Summarise".into(),
                status: lumen_agent_protocol::types::PlanStatus::InProgress,
            }]),
        ] {
            conversation.apply(update);
        }
        conversation.push_notice("Cancelled.");
        let shown = panel.texts();
        for expected in [
            "what does main do?",
            "run()",
            "fn main() { run(); }",
            "rust",
            "first",
            "Read main.rs",
            "done",
            "Summarise",
            "PLAN",
            "Cancelled.",
        ] {
            assert!(shown.contains(expected), "missing {expected:?} in {shown}");
        }
    }

    #[test]
    fn setup_problems_offer_actions_that_do_something() {
        let mut panel = Panel::new();
        panel.app.view.agents.session_mut(AgentKind::Claude).state =
            crate::app::agent::ConnectionState::NotFound("claude-agent-acp was not found.".into());
        let shown = panel.texts();
        assert!(shown.contains("was not found") && shown.contains("Copy install command"));
        panel.click_text("Download with npx");
        // Consent was recorded and a start attempted: launched through the fake,
        // or reported missing when this machine has no npx.
        let launched = panel.fake.launched.lock().unwrap().len();
        let state = panel.state();
        assert!(
            launched == 1
                || matches!(&state, crate::app::agent::ConnectionState::NotFound(m) if m.contains("npx")),
            "{state:?}"
        );

        let mut panel = Panel::new();
        panel.app.view.agents.active = AgentKind::Codex;
        let session = panel.app.view.agents.session_mut(AgentKind::Codex);
        session.state = crate::app::agent::ConnectionState::NeedsAuth;
        session.auth_methods = vec![lumen_agent_protocol::types::AuthMethod {
            id: "api-key".into(),
            name: "API Key".into(),
            description: None,
        }];
        let shown = panel.texts();
        assert!(
            shown.contains("API Key") && shown.contains("Retry") && shown.contains("Not signed in")
        );
    }

    #[test]
    fn panel_has_a_resizable_width_that_never_overlaps_the_editor_at_small_windows() {
        for width in [700.0, 1000.0, 1600.0] {
            let mut panel = Panel::new();
            panel.width = width;
            let left = std::cell::Cell::new(0.0);
            let app = &mut panel.app;
            let _ = panel.ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 700.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.render_component(ctx, |workbench| workbench.agent_panel(ctx));
                    left.set(ctx.available_rect().right());
                },
            );
            let panel_width = width - left.get();
            assert!(
                (MIN_WIDTH - 1.0..=MAX_WIDTH + 1.0).contains(&panel_width),
                "{width}: {panel_width}"
            );
        }
    }

    use crate::services::agent_process::AgentEvent;
    use serde_json::json;

    impl Panel {
        /// Start the agent via the fake launcher, finish the handshake, and have it
        /// ask to write `notes.txt` (diff included), like Claude does.
        fn with_permission_request(&mut self) {
            self.app.editor.workspace = Some("/w".into());
            self.app.view.agents.session_mut(AgentKind::Claude).draft = "go".into();
            self.app
                .execute(crate::app::command::Command::SendAgentPrompt, &self.ctx);
            let id_of = |fake: &FakeAgentLauncher, method: &str| {
                fake.sent
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|m| m["method"] == method)
                    .unwrap()["id"]
                    .clone()
            };
            let init = id_of(&self.fake, "initialize");
            self.deliver(json!({"jsonrpc":"2.0","id":init,"result":{"protocolVersion":1}}));
            let new = id_of(&self.fake, "session/new");
            self.deliver(json!({"jsonrpc":"2.0","id":new,"result":{"sessionId":"s1"}}));
            self.deliver(json!({"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{
                "sessionId":"s1",
                "toolCall":{"toolCallId":"t","title":"Write notes.txt","kind":"edit",
                    "locations":[{"path":"/w/notes.txt"}],
                    "content":[{"type":"diff","path":"/w/notes.txt","oldText":"keep\nold line\n","newText":"keep\nnew line\n"}]},
                "options":[{"optionId":"yes","name":"Yes","kind":"allow_once"},
                           {"optionId":"always","name":"Always allow","kind":"allow_always"},
                           {"optionId":"no","name":"No","kind":"reject_once"}]}}));
        }

        fn deliver(&mut self, message: serde_json::Value) {
            self.fake
                .queued
                .lock()
                .unwrap()
                .push(AgentEvent::Message(message));
            self.app.poll_agents(&self.ctx);
        }

        fn reply_option(&self, id: i64) -> Option<String> {
            self.fake
                .sent
                .lock()
                .unwrap()
                .iter()
                .find(|m| m["id"] == id && m.get("result").is_some())
                .map(|m| {
                    m["result"]["outcome"]["optionId"]
                        .as_str()
                        .unwrap_or("cancelled")
                        .to_owned()
                })
        }
    }

    #[test]
    fn permission_card_shows_what_will_change_and_clicking_an_option_answers_the_agent() {
        let mut panel = Panel::new();
        panel.with_permission_request();
        let shown = panel.texts();
        for expected in [
            "Write notes.txt",
            "notes.txt",
            "+1 −1",
            "- old line",
            "+ new line",
            "Yes",
            "Always allow",
            "No",
        ] {
            assert!(shown.contains(expected), "missing {expected:?} in {shown}");
        }
        assert!(
            panel.reply_option(7).is_none(),
            "nothing is answered until the user clicks"
        );
        panel.click_text("Always allow");
        assert_eq!(panel.reply_option(7).as_deref(), Some("always"));
        assert!(
            !panel.texts().contains("Always allow"),
            "the card is gone once answered"
        );
    }

    #[test]
    fn escape_declines_the_open_request() {
        let mut panel = Panel::new();
        panel.with_permission_request();
        panel.texts();
        panel.frame(vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
        assert_eq!(panel.reply_option(7).as_deref(), Some("no"));
    }

    #[test]
    fn queued_requests_show_one_at_a_time_with_a_count() {
        let mut panel = Panel::new();
        panel.with_permission_request();
        panel.deliver(
            json!({"jsonrpc":"2.0","id":8,"method":"session/request_permission","params":{
            "sessionId":"s1","toolCall":{"toolCallId":"u","title":"Run tests","kind":"execute",
                "rawInput":{"command":"cargo test"}},
            "options":[{"optionId":"yes","name":"Yes","kind":"allow_once"}]}}),
        );
        let shown = panel.texts();
        assert!(shown.contains("Write notes.txt") && shown.contains("(+1 waiting)"));
        assert!(
            !shown.contains("cargo test"),
            "the second card waits its turn"
        );
        panel.click_text("Yes");
        let shown = panel.texts();
        assert!(
            shown.contains("Run tests") && shown.contains("$ cargo test"),
            "{shown}"
        );
    }

    use crate::app::agent_changes::{ChangeRow, TurnChanges};
    use crate::services::workspace_snapshot::{ChangeKind, FileChange};

    fn change(
        path: &str,
        kind: ChangeKind,
        before: Option<&str>,
        after: Option<&str>,
    ) -> ChangeRow {
        ChangeRow {
            change: FileChange {
                path: path.into(),
                kind,
                before: before.map(Arc::from),
                after: after.map(Arc::from),
                opaque: false,
            },
            state: RowState::Pending,
            expanded: false,
        }
    }

    impl Panel {
        fn with_changes(&mut self) {
            self.app.editor.workspace = Some("/w".into());
            self.app.view.agents.session_mut(AgentKind::Claude).changes = Some(TurnChanges {
                root: "/w".into(),
                rows: vec![
                    change(
                        "/w/src/a.rs",
                        ChangeKind::Modified,
                        Some("old\n"),
                        Some("new\nmore\n"),
                    ),
                    change("/w/b.txt", ChangeKind::Added, None, Some("hello\n")),
                ],
                truncated: false,
                open: false,
            });
        }
    }

    #[test]
    fn the_change_card_summarises_expands_and_offers_revert_and_keep() {
        let mut panel = Panel::new();
        panel.with_changes();
        let shown = panel.texts();
        assert!(shown.contains("Changed by the agent: 2 file(s)"), "{shown}");
        assert!(
            shown.contains("+3 −1"),
            "added/removed totals across files: {shown}"
        );
        assert!(shown.contains("Revert all") && shown.contains("Keep"));
        assert!(!shown.contains("src/a.rs"), "collapsed until opened");

        panel.click_text("Changed by the agent");
        let shown = panel.texts();
        assert!(shown.contains("src/a.rs") && shown.contains("b.txt"));
        assert!(shown.contains("Open") && shown.contains("Revert"));

        panel.click_text("src/a.rs");
        let shown = panel.texts();
        assert!(
            shown.contains("- old") && shown.contains("+ new"),
            "inline diff: {shown}"
        );
    }

    #[test]
    fn keep_hides_the_card_and_reverted_rows_say_so() {
        let mut panel = Panel::new();
        panel.with_changes();
        if let Some(changes) = &mut panel.app.view.agents.session_mut(AgentKind::Claude).changes {
            changes.open = true;
            changes.rows[0].state = RowState::Reverted;
            changes.rows[1].state = RowState::Failed("it changed again".into());
        }
        let shown = panel.texts();
        assert!(shown.contains("reverted") && shown.contains("not reverted: it changed again"));
        panel.click_text("Keep");
        assert!(
            panel
                .app
                .view
                .agents
                .session(AgentKind::Claude)
                .changes
                .is_none()
        );
        assert!(!panel.texts().contains("Changed by the agent"));
    }

    #[test]
    fn tracking_progress_is_visible_while_a_snapshot_runs() {
        let mut panel = Panel::new();
        panel
            .app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .preparing = true;
        assert!(panel.texts().contains("Preparing change tracking…"));
        let session = panel.app.view.agents.session_mut(AgentKind::Claude);
        session.preparing = false;
        session.finishing = true;
        assert!(panel.texts().contains("Comparing files…"));
    }

    #[test]
    fn the_unsaved_files_card_offers_the_four_choices_and_cancel_returns_the_text() {
        let mut panel = Panel::new();
        let mut document = crate::model::Document::from_path("/w/a.rs".into(), "x".into());
        document.dirty = true;
        panel.app.editor.files.push(document);
        panel.app.view.agents.session_mut(AgentKind::Claude).draft = "explain".into();
        panel
            .app
            .execute(crate::app::command::Command::SendAgentPrompt, &panel.ctx);
        let shown = panel.texts();
        for expected in [
            "1 unsaved file(s): a.rs",
            "will not see these edits",
            "Save all & send",
            "Attach & send",
            "Send anyway",
            "Cancel",
        ] {
            assert!(shown.contains(expected), "missing {expected:?} in {shown}");
        }
        assert!(panel.fake.launched.lock().unwrap().is_empty());
        panel.click_text("Cancel");
        let session = panel.app.view.agents.session(AgentKind::Claude);
        assert!(session.held_prompt.is_none());
        assert_eq!(session.draft, "explain");
        assert!(!panel.texts().contains("Attach & send"));

        panel
            .app
            .execute(crate::app::command::Command::SendAgentPrompt, &panel.ctx);
        panel.click_text("Send anyway");
        assert_eq!(
            panel.fake.launched.lock().unwrap().len(),
            1,
            "the agent starts after the choice"
        );
    }

    #[test]
    fn chips_show_what_will_be_sent_and_can_be_removed() {
        let mut panel = Panel::new();
        panel
            .app
            .add_attachment(crate::app::agent_context::file_attachment(
                std::path::Path::new("/w/src/a.rs"),
                Some(std::path::Path::new("/w")),
            ));
        assert!(panel.texts().contains("src/a.rs"));
        panel.click_last_text(crate::icon::X);
        assert!(
            panel
                .app
                .view
                .agents
                .session(AgentKind::Claude)
                .attachments
                .is_empty()
        );
        assert!(!panel.texts().contains("src/a.rs"));
    }

    #[test]
    fn typing_at_lists_matching_files_and_tab_or_click_completes_the_mention() {
        let mut panel = Panel::new();
        panel.app.editor.workspace = Some("/w".into());
        panel.app.view.navigation.paths = vec![
            "/w/src/main.rs".into(),
            "/w/src/lib.rs".into(),
            "/w/README.md".into(),
        ];
        panel.app.view.agents.session_mut(AgentKind::Claude).draft = "see @ma".into();
        panel
            .app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .focus_composer = true;
        let shown = panel.texts();
        assert!(shown.contains("@src/main.rs"), "{shown}");
        assert!(
            !shown.contains("@README.md") && !shown.contains("@src/lib.rs"),
            "{shown}"
        );
        panel.frame(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        panel.frame(vec![]);
        assert_eq!(
            panel.app.view.agents.session(AgentKind::Claude).draft,
            "see @src/main.rs "
        );
        assert!(
            !panel.texts().contains("@src/lib.rs"),
            "the list closes after completing"
        );

        // Clicking a row works too.
        panel.app.view.agents.session_mut(AgentKind::Claude).draft = "@".into();
        panel.click_text("@README.md");
        assert_eq!(
            panel.app.view.agents.session(AgentKind::Claude).draft,
            "@README.md "
        );
    }

    #[test]
    fn enter_completes_a_slash_command_instead_of_sending() {
        let mut panel = Panel::new();
        let session = panel.app.view.agents.session_mut(AgentKind::Claude);
        session.conversation.commands = vec![
            lumen_agent_protocol::types::AvailableCommand {
                name: "review".into(),
                description: "Review changes".into(),
                input_hint: None,
            },
            lumen_agent_protocol::types::AvailableCommand {
                name: "init".into(),
                description: "Create AGENTS.md".into(),
                input_hint: None,
            },
        ];
        session.draft = "/re".into();
        session.focus_composer = true;
        let shown = panel.texts();
        assert!(
            shown.contains("/review  Review changes") && !shown.contains("/init"),
            "{shown}"
        );
        panel.frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(
            panel.app.view.agents.session(AgentKind::Claude).draft,
            "/review "
        );
        assert!(
            panel.fake.launched.lock().unwrap().is_empty(),
            "completing is not sending"
        );
        // With arguments started, Enter sends.
        panel.app.view.agents.session_mut(AgentKind::Claude).draft = "/review now".into();
        panel.texts();
        panel.frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(panel.fake.launched.lock().unwrap().len(), 1);
    }

    #[test]
    fn option_dropdowns_come_from_the_agent_and_risky_choices_show_a_warning() {
        let mut panel = Panel::new();
        let session = panel.app.view.agents.session_mut(AgentKind::Claude);
        session.conversation.config_options = lumen_agent_protocol::types::parse_config_options(
            &json!([
            {"id":"mode","name":"Mode","currentValue":"default","options":[
                {"value":"default","name":"Manual"},{"value":"bypassPermissions","name":"Bypass permissions"}]},
            {"id":"fast","name":"Fast","type":"boolean","currentValue":false}]),
        );
        let shown = panel.texts();
        assert!(shown.contains("Mode: Manual"), "{shown}");
        assert!(!shown.contains("Fast"), "only select options are shown");
        panel
            .app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .pending_config = Some((
            "mode".into(),
            "bypassPermissions".into(),
            "Bypass permissions".into(),
        ));
        let shown = panel.texts();
        assert!(
            shown.contains("lets the agent act without asking") && shown.contains("Enable"),
            "{shown}"
        );
        panel.click_text("Cancel");
        assert!(
            panel
                .app
                .view
                .agents
                .session(AgentKind::Claude)
                .pending_config
                .is_none()
        );
    }
}
