use super::{
    Workbench,
    git_history_tab::relative_time,
    source_control_menu::repository_menu,
    source_control_rows::{
        CHANGE_ROW_HEIGHT, ChangeRow, ListRow, RepoKind, RepoRow, change_row, list_row, repo_row,
        repository_kind,
    },
    widgets::{RowAction, disclosure_row, explorer_row, overflow_menu},
};
use crate::app::view_state::{ChangeGroup, ExtraSection};
use crate::{
    app::command::Command,
    services::source_control::{
        ChangeKind, ConflictSide, DiffSide, GitChange, GitSnapshot, HistoryAction, OperationAction,
        RepoState,
    },
};
use eframe::egui::{self, FontId, RichText, ScrollArea, TextEdit};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum SourceRow<'a> {
    Heading(ChangeGroup, usize),
    Change(&'a GitChange, DiffSide),
}

fn source_rows(snapshot: &GitSnapshot) -> Vec<SourceRow<'_>> {
    let mut rows = Vec::with_capacity(snapshot.changes.len() * 2 + 3);
    // Unresolved conflicts get their own group and never appear as ordinary
    // staged or working rows.
    let conflicts: Vec<_> = snapshot
        .changes
        .iter()
        .filter(|change| change.is_conflict())
        .collect();
    if !conflicts.is_empty() {
        rows.push(SourceRow::Heading(ChangeGroup::Conflicts, conflicts.len()));
        rows.extend(
            conflicts
                .into_iter()
                .map(|change| SourceRow::Change(change, DiffSide::Working)),
        );
    }
    let staged: Vec<_> = snapshot
        .changes
        .iter()
        .filter(|change| !change.is_conflict() && change.index != ' ' && change.index != '?')
        .collect();
    if !staged.is_empty() {
        rows.push(SourceRow::Heading(ChangeGroup::Staged, staged.len()));
        rows.extend(
            staged
                .into_iter()
                .map(|change| SourceRow::Change(change, DiffSide::Staged)),
        );
    }
    let working: Vec<_> = snapshot
        .changes
        .iter()
        .filter(|change| !change.is_conflict() && change.worktree != ' ')
        .collect();
    if !working.is_empty() {
        rows.push(SourceRow::Heading(ChangeGroup::Working, working.len()));
        rows.extend(working.into_iter().map(|change| {
            SourceRow::Change(
                change,
                if change.index == '?' {
                    DiffSide::Untracked
                } else {
                    DiffSide::Working
                },
            )
        }));
    }
    rows
}

/// Blame of the saved file; a line leads to its commit.
pub(super) fn history_blame(
    ui: &mut egui::Ui,
    blame: &crate::app::git_history::BlameView,
    actions: &mut Vec<Command>,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            actions.push(Command::CloseBlame);
        }
        ui.label(RichText::new(format!("Blame · {}", blame.path)).color(colors.muted));
    });
    ui.label(
        RichText::new("Based on the saved file; unsaved editor text is not included.")
            .small()
            .color(colors.muted),
    );
    match &blame.lines {
        None => {
            ui.label(RichText::new("Reading blame…").color(colors.muted));
        }
        Some(Err(error)) => {
            ui.label(RichText::new(error).color(colors.error));
        }
        Some(Ok(lines)) => {
            ScrollArea::vertical()
                .id_salt("git_history_blame")
                .auto_shrink([false, false])
                .show_rows(ui, 22.0, lines.len(), |ui, visible| {
                    for index in visible {
                        let line = &lines[index];
                        let label = if line.is_uncommitted() {
                            format!("{:>4}  Not committed yet  {}", index + 1, line.text)
                        } else {
                            format!(
                                "{:>4}  {}  {}  {}  {}",
                                index + 1,
                                &line.commit[..line.commit.len().min(7)],
                                line.author,
                                commit_date(line.time),
                                line.text
                            )
                        };
                        let response = explorer_row(ui, label, 4, false);
                        if line.is_uncommitted() {
                            response.on_hover_text("This line is not in any commit yet");
                        } else if response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(format!("{}\n{}", line.commit, line.summary))
                            .clicked()
                        {
                            actions.push(Command::ShowBlamedCommit(line.commit.clone()));
                        }
                    }
                });
        }
    }
}

/// Side-by-side, read-only before/after for one file of a commit.
pub(super) fn history_comparison(
    ui: &mut egui::Ui,
    compare: &mut crate::app::git_history::CommitCompare,
    font: &FontId,
    line_height: f32,
    actions: &mut Vec<Command>,
) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.horizontal(|ui| {
        if ui.button("Back").clicked() {
            actions.push(Command::CloseCommitFile);
        }
        ui.label(RichText::new(compare.file.label()).color(colors.muted));
    });
    match &compare.result {
        None => {
            ui.label(RichText::new("Reading file…").color(colors.muted));
        }
        Some(Err(error)) => {
            ui.label(RichText::new(error).color(colors.error));
        }
        Some(Ok(comparison)) => {
            let comparison = comparison.clone();
            let mut sync = compare.scroll;
            ui.columns(2, |columns| {
                columns[0].label(
                    RichText::new("Parent · read-only")
                        .small()
                        .color(colors.muted),
                );
                let observed = read_only_source(
                    &mut columns[0],
                    &ReadOnlySource {
                        text: &comparison.base,
                        changed: &comparison.old_lines,
                        changed_background: colors.diff_removed_bg,
                        id: "git_history_base",
                        font,
                        line_height,
                    },
                    sync.take_pending(0),
                );
                sync.observe(0, observed);
                columns[1].label(
                    RichText::new("Commit · read-only")
                        .small()
                        .color(colors.muted),
                );
                let observed = read_only_source(
                    &mut columns[1],
                    &ReadOnlySource {
                        text: comparison.changed.as_deref().unwrap_or_default(),
                        changed: &comparison.new_lines,
                        changed_background: colors.diff_added_bg,
                        id: "git_history_changed",
                        font,
                        line_height,
                    },
                    sync.take_pending(1),
                );
                sync.observe(1, observed);
            });
            if sync.finish_frame() {
                ui.ctx().request_repaint();
            }
            compare.scroll = sync;
        }
    }
}

pub(super) fn commit_date(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// Display name of a repository: its path relative to the opened folder, or
/// its directory name.
fn repository_name(repo: &GitSnapshot, workspace: Option<&std::path::Path>) -> String {
    workspace
        .and_then(|workspace| repo.root.strip_prefix(workspace).ok())
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(|relative| relative.display().to_string())
        .or_else(|| {
            repo.root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| repo.root.display().to_string())
}

/// What a repository's accordion row shows, taken from its snapshot.
struct RepoSummary {
    root: std::path::PathBuf,
    name: String,
    kind: Option<RepoKind>,
    branch: String,
    ahead: usize,
    behind: usize,
    changes: usize,
    conflicts: bool,
    draft: bool,
    tooltip: String,
}

fn remote_details(snapshot: &GitSnapshot) -> String {
    if snapshot.remotes.is_empty() {
        return "Add a remote with `git remote add <name> <url>`".into();
    }
    snapshot
        .remotes
        .iter()
        .map(|remote| match &remote.push_url {
            Some(push) => format!("{}\nfetch {}\npush  {push}", remote.name, remote.fetch_url),
            None => format!("{}\n{}", remote.name, remote.fetch_url),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn branch_details(snapshot: &GitSnapshot) -> String {
    let mut lines = vec![snapshot.root.display().to_string()];
    match &snapshot.upstream {
        Some(upstream) => match upstream.ahead_behind {
            Some((ahead, behind)) => lines.push(format!(
                "Tracking {}: {ahead} ahead, {behind} behind (as of the last fetch)",
                upstream.name
            )),
            None => lines.push(format!(
                "Tracking {}, which no longer exists on the remote",
                upstream.name
            )),
        },
        None if matches!(
            snapshot.head,
            crate::services::source_control::GitHead::Attached(_)
        ) =>
        {
            lines.push("No upstream branch is configured".into())
        }
        None => {}
    }
    if let Some(state) = snapshot.state.label() {
        lines.push(format!("{state} is in progress"));
    }
    lines.join("\n")
}

fn source_heading(ui: &mut egui::Ui, title: &str, count: usize) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        if count == 0 {
            title.to_owned()
        } else {
            format!("{title}  {count}")
        },
        FontId::proportional(11.5),
        colors.muted,
    );
}

/// Source-control labels must never contribute their natural width to the
/// resizable SidePanel. Git branch names and failures routinely contain long
/// remote paths, so every ordinary text row is clipped to the current pane.
fn source_label(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(0.0), 22.0),
        egui::Sense::hover(),
    );
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(13.0), color);
    let clipped = galley.size().x > rect.width();
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(
            egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    // Clipped Git errors must stay reachable at narrow sidebar widths.
    if clipped {
        response.on_hover_text(text)
    } else {
        response
    }
}

impl Workbench<'_> {
    pub(crate) fn source_control_panel(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        self.view.source_control.commit_focused = false;
        let Some(workspace) = &self.editor.workspace else {
            source_label(ui, "Open a folder to view source control", colors.muted);
            return;
        };
        if self.view.source_control.loading && self.view.source_control.snapshot.is_none() {
            source_label(ui, "Reading Git status…", colors.muted);
            return;
        }
        if let Some(error) = &self.view.source_control.error {
            source_label(ui, error, colors.error);
            if self.view.source_control.snapshot.is_none() {
                return;
            }
        }
        if self.view.source_control.snapshot.is_none() {
            self.repository_errors(ui);
        }
        let Some(snapshot) = self.view.source_control.snapshot.clone() else {
            source_label(ui, "No Git repository found", colors.muted);
            source_label(ui, &workspace.display().to_string(), colors.muted);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Initial branch").small().color(colors.muted));
                let width = (ui.available_width() - 4.0).max(24.0);
                ui.add_sized(
                    [width, 22.0],
                    TextEdit::singleline(&mut self.view.source_control.init_branch)
                        .id_salt("source_control_init_branch")
                        .hint_text("main"),
                );
            });
            if ui
                .add_enabled(
                    !self.view.source_control.loading,
                    egui::Button::new("Initialize Repository"),
                )
                .on_hover_text("Run git init in the opened folder")
                .clicked()
            {
                self.command(Command::InitSourceRepository);
            }
            if ui
                .button("Clone Repository…")
                .on_hover_text("Clone a repository into a new folder and open it")
                .clicked()
            {
                self.command(Command::OpenCloneDialog);
            }
            return;
        };
        self.repository_sections(ui, &snapshot);
    }

    /// Every repository as an accordion section. One repository is selected and
    /// expanded; it owns the commit box and the change groups, so no action is
    /// ever ambiguous. Collapsed repositories show a one-line summary.
    fn repository_sections(&mut self, ui: &mut egui::Ui, snapshot: &GitSnapshot) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let summaries = self.repository_summaries(snapshot);
        let multi = summaries.len() > 1;
        let idle = !self.view.source_control.loading;
        let has_conflicts = snapshot.changes.iter().any(|change| change.is_conflict());
        if multi {
            let dirty = summaries.iter().filter(|s| s.changes > 0).count();
            let mut text = format!("{} repositories · {dirty} with changes", summaries.len());
            let broken = self.view.source_control.repository_errors.len();
            if broken > 0 {
                text.push_str(&format!(" · {broken} unreadable"));
            }
            source_label(ui, &text, colors.muted);
        }
        let mut commands: Vec<Command> = Vec::new();
        ScrollArea::vertical()
            .id_salt("source_control_changes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for summary in &summaries {
                    let active = summary.root == snapshot.root;
                    // A lone repository has no caret to expand it again.
                    let collapsed = multi
                        && active
                        && self
                            .view
                            .source_control
                            .collapsed_repos
                            .contains(&summary.root);
                    let row = RepoRow {
                        name: &summary.name,
                        kind: summary.kind,
                        branch: &summary.branch,
                        ahead: summary.ahead,
                        behind: summary.behind,
                        changes: summary.changes,
                        conflicts: summary.conflicts,
                        active,
                        expanded: active && !collapsed,
                        accordion: multi,
                        draft: summary.draft,
                        menu: active,
                        tooltip: summary.tooltip.clone(),
                    };
                    let shown = repo_row(ui, &row);
                    if active {
                        let entries = repository_menu(&summary.name, snapshot, idle, has_conflicts);
                        if let Some(command) = overflow_menu(
                            ui,
                            shown.menu_rect,
                            &format!("Actions for {}", summary.name),
                            &entries,
                        ) {
                            commands.push(command);
                        }
                    }
                    // The branch name of the selected repository opens the switcher.
                    let mut chip_clicked = false;
                    if let Some(rect) = shown.branch_rect {
                        // Remember where the chip was drawn and when, so the
                        // switcher can tell if its anchor is still on screen.
                        let frame = ui.ctx().cumulative_frame_nr();
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(egui::Id::new("branch_chip_rect"), (rect, frame))
                        });
                        let chip = ui
                            .interact(
                                rect,
                                egui::Id::new(("branch_chip", &summary.root)),
                                egui::Sense::click(),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("Switch branch");
                        if chip.clicked() {
                            chip_clicked = true;
                            commands.push(Command::OpenBranchPicker);
                        }
                    }
                    if shown.response.clicked() && !chip_clicked {
                        if !active {
                            // A repository selected on purpose is shown open.
                            self.view
                                .source_control
                                .collapsed_repos
                                .remove(&summary.root);
                            commands.push(Command::SelectRepository(summary.root.clone()));
                        } else if multi
                            && !self
                                .view
                                .source_control
                                .collapsed_repos
                                .remove(&summary.root)
                        {
                            self.view
                                .source_control
                                .collapsed_repos
                                .insert(summary.root.clone());
                        }
                    }
                    if active && !collapsed {
                        self.repository_body(ui, snapshot, &mut commands);
                    }
                }
                for (root, error) in &self.view.source_control.repository_errors {
                    let name = root.file_name().map_or_else(
                        || root.display().to_string(),
                        |n| n.to_string_lossy().into(),
                    );
                    source_label(
                        ui,
                        &format!("{} {name}: {error}", crate::icon::ERROR),
                        colors.error,
                    )
                    .on_hover_text(format!(
                        "{}\n{error}\nOther repositories are not affected.",
                        root.display()
                    ));
                }
            });
        for command in commands {
            self.command(command);
        }
    }

    /// One summary per repository; the selected one is read from `snapshot`,
    /// which is fresher than its entry in the scanned list.
    fn repository_summaries(&self, snapshot: &GitSnapshot) -> Vec<RepoSummary> {
        let view = &self.view.source_control;
        let workspace = self.editor.workspace.as_deref();
        let all = &view.repositories;
        let summarize = |repo: &GitSnapshot| {
            let (ahead, behind) = repo
                .upstream
                .as_ref()
                .and_then(|upstream| upstream.ahead_behind)
                .unwrap_or((0, 0));
            let shared = all
                .iter()
                .filter(|other| other.root != repo.root && other.shares_storage_with(repo))
                .count();
            let mut tooltip = branch_details(repo);
            if !repo.remotes.is_empty() {
                tooltip.push_str(&format!("\n{}", remote_details(repo)));
            }
            if shared > 0 {
                tooltip.push_str(&format!(
                    "\nShares branches and objects with {shared} other listed worktree(s)"
                ));
            }
            RepoSummary {
                root: repo.root.clone(),
                name: repository_name(repo, workspace),
                kind: repository_kind(all, repo),
                branch: repo.branch.clone(),
                ahead: ahead as usize,
                behind: behind as usize,
                changes: repo.changes.len(),
                conflicts: repo.changes.iter().any(|change| change.is_conflict()),
                draft: view.parked_drafts.contains_key(&repo.root),
                tooltip,
            }
        };
        let mut summaries: Vec<_> = all
            .iter()
            .map(|repo| {
                if repo.root == snapshot.root {
                    summarize(snapshot)
                } else {
                    summarize(repo)
                }
            })
            .collect();
        if !summaries
            .iter()
            .any(|summary| summary.root == snapshot.root)
        {
            summaries.insert(0, summarize(snapshot));
        }
        summaries
    }

    /// The selected repository's operation banner, commit box and change groups.
    fn repository_body(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GitSnapshot,
        commands: &mut Vec<Command>,
    ) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let idle = !self.view.source_control.loading;
        let conflicts = snapshot.changes.iter().filter(|c| c.is_conflict()).count();
        if let Some(label) = snapshot.state.label() {
            let banner = egui::Frame::NONE
                .fill(colors.warning.gamma_multiply(0.14))
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let text = if conflicts > 0 {
                            format!("{label} · {conflicts} left to resolve")
                        } else {
                            label.to_owned()
                        };
                        ui.label(RichText::new(text).small().color(colors.text));
                        let continue_button =
                            ui.add_enabled(idle && conflicts == 0, egui::Button::new("Continue"));
                        let continue_button = if conflicts > 0 {
                            continue_button
                                .on_disabled_hover_text("Resolve and stage every conflict first")
                        } else {
                            continue_button
                                .on_hover_text(format!("Continue {}", label.to_lowercase()))
                        };
                        if continue_button.clicked() {
                            commands.push(Command::SourceOperation(OperationAction::Continue));
                        }
                        if snapshot.state != RepoState::Merge
                            && ui
                                .add_enabled(idle, egui::Button::new("Skip"))
                                .on_hover_text("Skip the current step")
                                .clicked()
                        {
                            commands.push(Command::SourceOperation(OperationAction::Skip));
                        }
                        if ui
                            .add_enabled(idle, egui::Button::new("Abort…"))
                            .on_hover_text("Stop and return to where you started")
                            .clicked()
                        {
                            commands.push(Command::SourceOperation(OperationAction::Abort));
                        }
                    });
                });
            let rect = banner.response.rect;
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                egui::CornerRadius::ZERO,
                colors.warning,
            );
            ui.add_space(4.0);
        }

        let staged_paths: Vec<_> = snapshot
            .changes
            .iter()
            .filter(|change| !change.is_conflict() && change.index != ' ' && change.index != '?')
            .map(|change| change.path.clone())
            .collect();
        let working_paths: Vec<_> = snapshot
            .changes
            .iter()
            .filter(|change| !change.is_conflict() && change.worktree != ' ')
            .map(|change| change.path.clone())
            .collect();
        let revertible_paths: Vec<_> = snapshot
            .changes
            .iter()
            .filter(|change| !change.is_conflict() && change.index != '?' && change.index != 'A')
            .map(|change| change.path.clone())
            .collect();

        ui.add_space(4.0);
        // The previous frame's focus decides whether Cmd/Ctrl+Enter commits;
        // consume it before the multiline editor can turn it into a newline.
        let commit_shortcut = self.view.source_control.commit_focused
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        let message_width = ui.available_width().max(24.0);
        let response = ui.add_sized(
            [message_width, 56.0],
            TextEdit::multiline(&mut self.view.source_control.commit_message)
                .id_salt("source_control_commit_message")
                .desired_rows(3)
                .hint_text("Message (Cmd/Ctrl+Enter to commit)"),
        );
        if self.view.source_control.commit_focus {
            response.request_focus();
            self.view.source_control.commit_focus = false;
        }
        self.view.source_control.commit_focused = response.has_focus();
        ui.add_space(6.0);
        let can_commit =
            (!staged_paths.is_empty() || self.view.source_control.amend_commit) && idle;
        if self.commit_button(ui, can_commit) || (commit_shortcut && can_commit) {
            commands.push(Command::CommitSourceControl);
        }
        ui.add_space(4.0);
        if self.view.source_control.amend_commit {
            ui.horizontal(|ui| {
                // Leave room for the 22px button and the item spacing between.
                let width = (ui.available_width() - 22.0 - ui.spacing().item_spacing.x).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 22.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let subject = snapshot.last_commit.as_deref().unwrap_or("the last commit");
                        source_label(ui, &format!("Amending: {subject}"), colors.muted);
                    },
                );
                if ui
                    .add_sized([22.0, 22.0], egui::Button::new(crate::icon::X).frame(false))
                    .on_hover_text("Stop amending")
                    .clicked()
                {
                    self.view.source_control.amend_commit = false;
                }
            });
        }
        if let Some(error) = &self.view.source_control.commit_error {
            source_label(ui, error, colors.error);
        }
        if snapshot.changes.is_empty() {
            source_label(
                ui,
                &format!("{} No changes", crate::icon::CHECK),
                colors.muted,
            );
            self.repository_extras(ui, snapshot, commands);
            return;
        }

        let rows = source_rows(snapshot);
        let mut collapsed_group = false;
        for row in &rows {
            let (change, side) = match *row {
                SourceRow::Heading(group, count) => {
                    let key = (snapshot.root.clone(), group);
                    collapsed_group = self.view.source_control.collapsed_groups.contains(&key);
                    // Each action carries its own command, so order and
                    // labels can change without rewiring anything.
                    let group_actions: Vec<(RowAction, Command)> = match group {
                        ChangeGroup::Staged => vec![(
                            RowAction {
                                glyph: crate::icon::X,
                                tooltip: "Unstage all staged changes",
                                enabled: true,
                            },
                            Command::UnstagePaths(staged_paths.clone()),
                        )],
                        ChangeGroup::Working => vec![
                            (
                                RowAction {
                                    glyph: crate::icon::DISCARD,
                                    tooltip: "Discard all tracked changes",
                                    enabled: !revertible_paths.is_empty(),
                                },
                                Command::RequestSourceRevert(revertible_paths.clone()),
                            ),
                            (
                                RowAction {
                                    glyph: crate::icon::PLUS,
                                    tooltip: "Stage all working changes",
                                    enabled: true,
                                },
                                Command::StagePaths(working_paths.clone()),
                            ),
                        ],
                        ChangeGroup::Conflicts => Vec::new(),
                    };
                    let (specs, group_commands): (Vec<_>, Vec<_>) =
                        group_actions.into_iter().unzip();
                    let outcome = disclosure_row(ui, collapsed_group, group.title(), count, &specs);
                    if outcome.toggled && !self.view.source_control.collapsed_groups.remove(&key) {
                        self.view.source_control.collapsed_groups.insert(key);
                    }
                    if let Some(command) = outcome.action.and_then(|i| group_commands.get(i)) {
                        commands.push(command.clone());
                    }
                    continue;
                }
                SourceRow::Change(change, side) => (change, side),
            };
            if collapsed_group {
                continue;
            }
            // Only rows in view are drawn; the rest keep their height.
            let top = ui.cursor().top();
            if top + CHANGE_ROW_HEIGHT < ui.clip_rect().top() || top > ui.clip_rect().bottom() {
                ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), CHANGE_ROW_HEIGHT),
                    egui::Sense::hover(),
                );
                continue;
            }
            self.change_entry(ui, snapshot, change, side, commands);
        }
        self.repository_extras(ui, snapshot, commands);
    }

    /// Stashes and the commits that differ from the upstream, as collapsed
    /// sections that appear only when they have something to show.
    fn repository_extras(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GitSnapshot,
        commands: &mut Vec<Command>,
    ) {
        let colors = crate::ui::theme::palette(ui.visuals());
        // Taken for the frame so toggling a section can borrow the view too.
        let Some(extras) = self.view.source_control.extras.loaded.take() else {
            return;
        };
        if extras.root() != snapshot.root {
            self.view.source_control.extras.loaded = Some(extras);
            return;
        }
        if let Some(error) = &extras.error {
            source_label(ui, error, colors.error);
        }
        let now = chrono::Utc::now().timestamp();
        let sections = [
            (ExtraSection::Stashes, "STASHES", extras.stashes.len()),
            (
                ExtraSection::Outgoing,
                "OUTGOING",
                extras.sync.outgoing.len(),
            ),
            (
                ExtraSection::Incoming,
                "INCOMING",
                extras.sync.incoming.len(),
            ),
        ];
        for (section, title, count) in sections {
            if count == 0 {
                continue;
            }
            let key = (snapshot.root.clone(), section);
            let open = self.view.source_control.expanded_extras.contains(&key);
            if disclosure_row(ui, !open, title, count, &[]).toggled
                && !self.view.source_control.expanded_extras.remove(&key)
            {
                self.view.source_control.expanded_extras.insert(key);
            }
            if !open {
                continue;
            }
            match section {
                ExtraSection::Stashes => {
                    for stash in &extras.stashes {
                        let tip = "Open the stash list to apply, pop or drop it";
                        if list_row(
                            ui,
                            &ListRow {
                                lead: &format!("stash@{{{}}}", stash.index),
                                text: &stash.message,
                                trail: "",
                                tooltip: tip.to_owned(),
                                selected: false,
                            },
                        )
                        .clicked()
                        {
                            commands.push(Command::OpenStashDialog);
                        }
                    }
                }
                ExtraSection::Outgoing | ExtraSection::Incoming => {
                    let commits = if section == ExtraSection::Outgoing {
                        &extras.sync.outgoing
                    } else {
                        &extras.sync.incoming
                    };
                    let note = if section == ExtraSection::Outgoing {
                        "Not on the upstream yet"
                    } else {
                        "On the upstream, not here yet (as of the last fetch)"
                    };
                    for commit in commits {
                        let response = list_row(
                            ui,
                            &ListRow {
                                lead: &commit.id[..commit.id.len().min(7)],
                                text: &commit.subject,
                                trail: &relative_time(now, commit.time),
                                tooltip: format!("{note}\n{}\n{}", commit.id, commit.author),
                                selected: false,
                            },
                        );
                        if response.clicked() {
                            commands.push(Command::OpenHistoryAt(commit.id.clone()));
                        }
                    }
                    if commits.len() >= crate::services::source_control::SYNC_LIMIT {
                        source_label(
                            ui,
                            &format!(
                                "Showing the newest {}",
                                crate::services::source_control::SYNC_LIMIT
                            ),
                            colors.muted,
                        );
                    }
                }
            }
        }
        self.view.source_control.extras.loaded = Some(extras);
    }

    /// The split Commit button: commit on the left, options on the right.
    /// Returns true when the primary part was clicked.
    fn commit_button(&mut self, ui: &mut egui::Ui, can_commit: bool) -> bool {
        const DROP: f32 = 24.0;
        let colors = crate::ui::theme::palette(ui.visuals());
        let fill = if can_commit {
            colors.accent
        } else {
            colors.input
        };
        let text_color = if can_commit {
            egui::Color32::WHITE
        } else {
            colors.muted
        };
        let label = if self.view.source_control.amend_commit {
            format!("{} Amend", crate::icon::CHECK)
        } else {
            format!("{} Commit", crate::icon::CHECK)
        };
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            let width = (ui.available_width() - DROP - 1.0).max(24.0);
            clicked = ui
                .add_enabled(
                    can_commit,
                    egui::Button::new(RichText::new(label).color(text_color))
                        .fill(fill)
                        .corner_radius(egui::CornerRadius {
                            nw: 3,
                            sw: 3,
                            ne: 0,
                            se: 0,
                        })
                        .min_size(egui::vec2(width, 24.0)),
                )
                .on_hover_text(if self.view.source_control.amend_commit {
                    "Amend the most recent commit"
                } else {
                    "Commit the staged changes"
                })
                .clicked();
            let drop = egui::Button::new(RichText::new(crate::icon::CARET_DOWN).color(text_color))
                .fill(fill)
                .corner_radius(egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: 3,
                    se: 3,
                })
                .min_size(egui::vec2(DROP, 24.0));
            // The caret must fit its fixed slot; default padding would make it
            // wider than DROP and widen the pane by the difference every frame.
            ui.spacing_mut().button_padding.x = 2.0;
            let (response, _) =
                egui::containers::menu::MenuButton::from_button(drop).ui(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.checkbox(
                        &mut self.view.source_control.amend_commit,
                        "Amend Last Commit",
                    )
                    .on_hover_text(
                        "Amend the most recent commit; an empty message keeps its current message",
                    );
                });
            response.on_hover_text("Commit options");
        });
        clicked
    }

    /// One file row with its actions and context menu.
    fn change_entry(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GitSnapshot,
        change: &GitChange,
        side: DiffSide,
        commands: &mut Vec<Command>,
    ) {
        let marker = if change.is_conflict() {
            '!'
        } else if side == DiffSide::Untracked {
            'U'
        } else if side == DiffSide::Staged {
            change.index
        } else {
            change.worktree
        };
        let submodule = match &change.kind {
            ChangeKind::Submodule(sub) => Some(*sub),
            _ => None,
        };
        let hover = match submodule {
            Some(sub) => format!("{}\n{}", change.path.display(), sub.describe()),
            None => change.path.display().to_string(),
        };
        let hover = if change.is_conflict() {
            format!("{hover}\nRight-click to choose a side")
        } else {
            hover
        };
        let revertible = !change.is_conflict()
            && !change.is_submodule()
            && change.index != '?'
            && change.index != 'A';
        let (primary_glyph, primary_tip) = if side == DiffSide::Staged {
            (crate::icon::X, "Unstage change")
        } else if change.is_conflict() {
            (
                crate::icon::PLUS,
                "Mark resolved (stage the file as it is on disk)",
            )
        } else {
            (crate::icon::PLUS, "Stage change")
        };
        let shown = change_row(
            ui,
            &ChangeRow {
                change,
                marker,
                discardable: revertible,
                primary_glyph,
                primary_tip,
                hover,
            },
        );
        let response = shown
            .response
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let path = change.path.clone();
        let (ours, theirs) = snapshot.state.side_labels();
        response.context_menu(|ui| {
            if change.is_conflict() {
                if ui.button(ours).clicked() {
                    commands.push(Command::ResolveConflict(path.clone(), ConflictSide::Ours));
                    ui.close();
                }
                if ui.button(theirs).clicked() {
                    commands.push(Command::ResolveConflict(path.clone(), ConflictSide::Theirs));
                    ui.close();
                }
                ui.separator();
            }
            let deleted = change.index == 'D' || change.worktree == 'D';
            if !change.is_submodule()
                && ui
                    .add_enabled(!deleted, egui::Button::new("Open File"))
                    .on_disabled_hover_text("The file no longer exists on disk")
                    .clicked()
            {
                commands.push(Command::OpenPath(snapshot.root.join(&path)));
                ui.close();
            }
            let primary_label = match (side, change.is_conflict()) {
                (DiffSide::Staged, _) => "Unstage",
                (_, true) => "Mark Resolved",
                _ => "Stage",
            };
            if ui.button(primary_label).clicked() {
                commands.push(match side {
                    DiffSide::Staged => Command::UnstagePaths(vec![path.clone()]),
                    _ => Command::StagePaths(vec![path.clone()]),
                });
                ui.close();
            }
            if revertible && ui.button("Discard Changes…").clicked() {
                commands.push(Command::RequestSourceRevert(vec![path.clone()]));
                ui.close();
            }
        });
        if shown.primary {
            commands.push(match side {
                DiffSide::Staged => Command::UnstagePaths(vec![change.path.clone()]),
                _ => Command::StagePaths(vec![change.path.clone()]),
            });
        } else if shown.discard {
            commands.push(Command::RequestSourceRevert(vec![change.path.clone()]));
        } else if response.clicked() && change.is_submodule() {
            // A gitlink has no text comparison; jump to the child repository
            // when it was discovered.
            let child = snapshot.root.join(&change.path);
            if self
                .view
                .source_control
                .repositories
                .iter()
                .any(|repo| repo.root == child)
            {
                commands.push(Command::SelectRepository(child));
            }
        } else if response.clicked() && change.is_conflict() {
            // A comparison of a conflicted file is not meaningful; open it so
            // the markers can be resolved in the editor.
            commands.push(Command::OpenPath(snapshot.root.join(&change.path)));
        } else if response.clicked() {
            commands.push(Command::ShowSourceDiff(change.path.clone(), side));
        }
    }

    fn repository_errors(&self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        for (root, error) in &self.view.source_control.repository_errors {
            let name = root.file_name().map_or_else(
                || root.display().to_string(),
                |n| n.to_string_lossy().into(),
            );
            source_label(ui, &format!("{name}: {error}"), colors.error)
                .on_hover_text(format!("{}\n{error}", root.display()));
        }
    }

    pub(crate) fn history_op_dialog(&mut self, ctx: &egui::Context) {
        use crate::app::git_history_ops::HistoryOption;
        use crate::services::source_control::{HistoryAction, ResetMode};
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(dialog) = &self.view.history_op_dialog else {
            return;
        };
        let mut actions: Vec<Command> = Vec::new();
        let modal = egui::Modal::new(egui::Id::new("git_history_op_dialog")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(&dialog.title);
            ui.add_space(4.0);
            match &dialog.action {
                HistoryAction::Merge { no_ff, .. } => {
                    let mut value = *no_ff;
                    if ui
                        .checkbox(&mut value, "Always create a merge commit")
                        .changed()
                    {
                        actions.push(Command::SetHistoryOption(HistoryOption::NoFastForward(
                            value,
                        )));
                    }
                }
                HistoryAction::Reset { mode, backup, .. } => {
                    ui.horizontal(|ui| {
                        for (label, value) in [
                            ("Soft", ResetMode::Soft),
                            ("Mixed", ResetMode::Mixed),
                            ("Hard", ResetMode::Hard),
                        ] {
                            if ui.selectable_label(*mode == value, label).clicked()
                                && *mode != value
                            {
                                actions.push(Command::SetHistoryOption(HistoryOption::ResetMode(
                                    value,
                                )));
                            }
                        }
                    });
                    let mut value = *backup;
                    if ui
                        .checkbox(&mut value, "Keep the current tip on a backup branch")
                        .changed()
                    {
                        actions.push(Command::SetHistoryOption(HistoryOption::Backup(value)));
                    }
                }
                HistoryAction::CherryPick { parent, .. } | HistoryAction::Revert { parent, .. }
                    if dialog.parent_count > 1 =>
                {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Merge commit: use parent").color(colors.muted));
                        for index in 0..dialog.parent_count {
                            if ui
                                .selectable_label(*parent == Some(index), format!("{}", index + 1))
                                .clicked()
                            {
                                actions
                                    .push(Command::SetHistoryOption(HistoryOption::Parent(index)));
                            }
                        }
                    });
                }
                _ => {}
            }
            ui.add_space(4.0);
            match &dialog.plan {
                None if dialog.loading => {
                    ui.label(RichText::new("Checking the repository…").color(colors.muted));
                }
                None => {}
                Some(plan) => {
                    for line in &plan.lines {
                        ui.label(line);
                    }
                    for warning in &plan.warnings {
                        ui.label(RichText::new(warning).color(colors.warning));
                    }
                    if let Some(reason) = &plan.blocked {
                        ui.label(RichText::new(reason).color(colors.error));
                    }
                }
            }
            if let Some(error) = &dialog.error {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ready = dialog
                    .plan
                    .as_ref()
                    .is_some_and(|plan| plan.blocked.is_none())
                    && !dialog.loading;
                let label = match &dialog.action {
                    HistoryAction::Merge { .. } => "Merge",
                    HistoryAction::CherryPick { .. } => "Cherry-pick",
                    HistoryAction::Revert { .. } => "Revert",
                    HistoryAction::Reset {
                        mode: ResetMode::Hard,
                        ..
                    } => "Reset (discards changes)",
                    HistoryAction::Reset { .. } => "Reset",
                    HistoryAction::ForcePush { .. } => "Force push",
                };
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    actions.push(Command::ConfirmHistoryAction);
                }
                if ui.button("Cancel").clicked() {
                    actions.push(Command::CancelHistoryAction);
                }
            });
        });
        if modal.should_close() {
            actions.push(Command::CancelHistoryAction);
        }
        for action in actions {
            self.command(action);
        }
    }

    pub(crate) fn tags_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(dialog) = &mut self.view.tags_dialog else {
            return;
        };
        let mut actions: Vec<Command> = Vec::new();
        let modal = egui::Modal::new(egui::Id::new("git_tags_dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Tags");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 70.0).max(40.0);
                ui.add(
                    TextEdit::singleline(&mut dialog.name)
                        .id_salt("git_tag_name")
                        .desired_width(width)
                        .hint_text("v1.0.0"),
                );
                if ui
                    .add_enabled(!dialog.name.trim().is_empty(), egui::Button::new("Create"))
                    .clicked()
                {
                    actions.push(Command::CreateTag);
                }
            });
            ui.add(
                TextEdit::singleline(&mut dialog.message)
                    .id_salt("git_tag_message")
                    .desired_width(f32::INFINITY)
                    .hint_text("Message (makes it an annotated tag)"),
            );
            ui.label(
                RichText::new(match &dialog.target {
                    Some(id) => format!("On commit {}", &id[..id.len().min(7)]),
                    None => "On the current commit".to_owned(),
                })
                .small()
                .color(colors.muted),
            );
            ui.add_space(4.0);
            let confirming = dialog.confirm_delete.clone();
            ScrollArea::vertical()
                .id_salt("git_tags_list")
                .max_height(180.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if dialog.tags.is_empty() {
                        let text = if dialog.loading {
                            "Reading tags…"
                        } else {
                            "No tags"
                        };
                        ui.label(RichText::new(text).color(colors.muted));
                    }
                    for tag in &dialog.tags {
                        let kind = if tag.annotated {
                            "annotated"
                        } else {
                            "lightweight"
                        };
                        let mut label = format!(
                            "{}  {}  {kind}",
                            tag.name,
                            &tag.commit[..tag.commit.len().min(7)]
                        );
                        if !tag.message.is_empty() {
                            label.push_str(&format!("  {}", tag.message));
                        }
                        let response = explorer_row(ui, label, 4, false)
                            .on_hover_text(format!("{}\n{}", tag.name, tag.commit));
                        if ui
                            .put(
                                egui::Rect::from_min_size(
                                    response.rect.right_top() - egui::vec2(22.0, 0.0),
                                    egui::vec2(22.0, response.rect.height()),
                                ),
                                egui::Button::new(crate::icon::X).frame(false),
                            )
                            .on_hover_text("Delete this tag locally")
                            .clicked()
                        {
                            actions.push(Command::RequestTagDelete(tag.name.clone()));
                        } else if ui
                            .put(
                                egui::Rect::from_min_size(
                                    response.rect.right_top() - egui::vec2(44.0, 0.0),
                                    egui::vec2(22.0, response.rect.height()),
                                ),
                                egui::Button::new(crate::icon::REPO_PUSH).frame(false),
                            )
                            .on_hover_text("Push this tag to the chosen remote")
                            .clicked()
                        {
                            actions.push(Command::PushTag(tag.name.clone()));
                        }
                    }
                });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Push to").small().color(colors.muted));
                if dialog.remotes.is_empty() {
                    ui.label(
                        RichText::new("no remote configured")
                            .small()
                            .color(colors.muted),
                    );
                }
                for remote in &dialog.remotes {
                    if ui
                        .selectable_label(dialog.remote.as_deref() == Some(remote.as_str()), remote)
                        .clicked()
                    {
                        actions.push(Command::ChooseTagRemote(remote.clone()));
                    }
                }
            });
            if let Some(name) = &confirming {
                ui.label(
                    RichText::new(format!(
                        "Delete tag {name}? Copies on remotes are not removed."
                    ))
                    .color(colors.muted),
                );
                ui.horizontal(|ui| {
                    if ui.button("Delete Tag").clicked() {
                        actions.push(Command::ConfirmTagDelete);
                    }
                    if ui.button("Keep").clicked() {
                        actions.push(Command::CancelTagDelete);
                    }
                });
            }
            if let Some(error) = &dialog.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                actions.push(Command::CloseTagsDialog);
            }
        });
        if modal.should_close() {
            actions.push(Command::CloseTagsDialog);
        }
        for action in actions {
            self.command(action);
        }
    }

    pub(crate) fn stash_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(dialog) = &mut self.view.stash_dialog else {
            return;
        };
        let mut actions: Vec<Command> = Vec::new();
        let modal = egui::Modal::new(egui::Id::new("git_stash_dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Stashes");
            ui.label(
                RichText::new(
                    "A stash saves what is on disk. Unsaved editor text is not included.",
                )
                .color(colors.muted),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 70.0).max(40.0);
                ui.add(
                    TextEdit::singleline(&mut dialog.message)
                        .id_salt("git_stash_message")
                        .desired_width(width)
                        .hint_text("Message (optional)"),
                );
                if ui
                    .button("Stash")
                    .on_hover_text("Stash tracked changes and reset them")
                    .clicked()
                {
                    actions.push(Command::CreateStash);
                }
            });
            ui.checkbox(&mut dialog.include_untracked, "Include untracked files");
            ui.checkbox(
                &mut dialog.restore_index,
                "Restore staged state when applying",
            );
            ui.add_space(4.0);
            let selected = dialog.selected.clone();
            let confirming = dialog.confirm_drop.clone();
            ScrollArea::vertical()
                .id_salt("git_stash_list")
                .max_height(180.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if dialog.stashes.is_empty() {
                        let text = if dialog.loading {
                            "Reading stashes…"
                        } else {
                            "No stashes"
                        };
                        ui.label(RichText::new(text).color(colors.muted));
                    }
                    for stash in &dialog.stashes {
                        let label = format!("{{{}}}  {}", stash.index, stash.message);
                        let response =
                            explorer_row(ui, label, 4, selected.as_deref() == Some(&stash.id))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Show the files in this stash");
                        if response.clicked() {
                            actions.push(Command::SelectStash(stash.id.clone()));
                        }
                    }
                });
            if let Some(id) = &selected {
                if !dialog.files.is_empty() {
                    ui.add_space(2.0);
                    ScrollArea::vertical()
                        .id_salt("git_stash_files")
                        .max_height(90.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for (status, path) in &dialog.files {
                                ui.label(RichText::new(format!("{status}  {path}")).small());
                            }
                        });
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button("Apply")
                        .on_hover_text("Restore the changes and keep the stash")
                        .clicked()
                    {
                        actions.push(Command::ApplyStash(id.clone(), false));
                    }
                    if ui
                        .button("Pop")
                        .on_hover_text(
                            "Restore the changes and remove the stash (kept if it conflicts)",
                        )
                        .clicked()
                    {
                        actions.push(Command::ApplyStash(id.clone(), true));
                    }
                    if ui
                        .button("Drop…")
                        .on_hover_text("Delete this stash")
                        .clicked()
                    {
                        actions.push(Command::RequestStashDrop(id.clone()));
                    }
                });
            }
            if confirming.is_some() {
                ui.label(
                    RichText::new("Delete this stash? Its changes cannot be recovered afterwards.")
                        .color(colors.muted),
                );
                ui.horizontal(|ui| {
                    if ui.button("Delete Stash").clicked() {
                        actions.push(Command::ConfirmStashDrop);
                    }
                    if ui.button("Keep").clicked() {
                        actions.push(Command::CancelStashDrop);
                    }
                });
            }
            if let Some(error) = &dialog.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                actions.push(Command::CloseStashDialog);
            }
        });
        if modal.should_close() {
            actions.push(Command::CloseStashDialog);
        }
        for action in actions {
            self.command(action);
        }
    }

    /// The quick branch switcher: a popover under the branch name with a
    /// filter, the local and remote branches, and shortcuts to create or manage.
    pub(crate) fn branch_picker(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(dialog) = &mut self.view.branch_dialog else {
            return;
        };
        if !dialog.quick {
            return;
        }
        const WIDTH: f32 = 280.0;
        // The switcher hangs from the branch name. When that is not drawn this
        // frame (sidebar hidden, another activity), it has no anchor: close it.
        let drawn: Option<(egui::Rect, u64)> =
            ctx.data(|data| data.get_temp(egui::Id::new("branch_chip_rect")));
        let Some(anchor) = drawn
            .filter(|(_, frame)| *frame == ctx.cumulative_frame_nr())
            .map(|(rect, _)| rect)
        else {
            self.command(Command::CloseBranchDialog);
            return;
        };
        let screen = ctx.content_rect();
        let left = anchor
            .left()
            .min(screen.right() - WIDTH - 8.0)
            .max(screen.left() + 8.0);
        let mut actions: Vec<Command> = Vec::new();
        let selection_key = egui::Id::new("git_branch_picker_selection");
        let output = egui::Area::new(egui::Id::new("git_branch_picker"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(left, anchor.bottom() + 2.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    let filter = ui.add(
                        TextEdit::singleline(&mut dialog.filter)
                            .id_salt("git_branch_picker_filter")
                            .desired_width(f32::INFINITY)
                            .hint_text("Switch to a branch…"),
                    );
                    if std::mem::take(&mut dialog.focus_filter) {
                        filter.request_focus();
                    }
                    if let Some(error) = &dialog.error {
                        ui.label(RichText::new(error).size(12.0).color(colors.error));
                    }
                    let needle = dialog.filter.trim().to_lowercase();
                    let shown: Vec<&crate::services::source_control::GitBranch> = [false, true]
                        .into_iter()
                        .flat_map(|remote| {
                            dialog.branches.iter().filter(move |b| b.remote == remote)
                        })
                        .filter(|branch| branch.name.to_lowercase().contains(&needle))
                        .collect();
                    // Up/Down move the highlight and Enter switches, while typing.
                    let mut selected: usize =
                        ui.data(|data| data.get_temp(selection_key)).unwrap_or(0);
                    selected = selected.min(shown.len().saturating_sub(1));
                    // Keys are taken only while the filter box owns the keyboard.
                    let (step, enter) = if filter.has_focus() || filter.lost_focus() {
                        ui.input_mut(|input| {
                            let step = if input
                                .consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                            {
                                1
                            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                                -1
                            } else {
                                0
                            };
                            (
                                step,
                                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                            )
                        })
                    } else {
                        (0, false)
                    };
                    if step != 0 && !shown.is_empty() {
                        selected =
                            (selected as isize + step).clamp(0, shown.len() as isize - 1) as usize;
                    }
                    if filter.changed() {
                        selected = 0;
                    }
                    ui.data_mut(|data| data.insert_temp(selection_key, selected));
                    if enter && let Some(branch) = shown.get(selected) {
                        actions.push(Command::SwitchBranch(branch.name.clone(), branch.remote));
                    }
                    ui.add_space(4.0);
                    ScrollArea::vertical()
                        .id_salt("git_branch_picker_list")
                        .max_height(240.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            if dialog.loading && dialog.branches.is_empty() {
                                ui.label(RichText::new("Reading branches…").color(colors.muted));
                            } else if shown.is_empty() {
                                ui.label(RichText::new("No matching branch").color(colors.muted));
                            }
                            let mut heading: Option<bool> = None;
                            for (index, branch) in shown.iter().enumerate() {
                                if heading != Some(branch.remote) {
                                    source_heading(
                                        ui,
                                        if branch.remote { "REMOTE" } else { "LOCAL" },
                                        0,
                                    );
                                    heading = Some(branch.remote);
                                }
                                let trail = if branch.current {
                                    "current"
                                } else if branch.checked_out_at.is_some() {
                                    "in a worktree"
                                } else {
                                    ""
                                };
                                let tip = match (&branch.upstream, &branch.checked_out_at) {
                                    (_, Some(path)) => {
                                        format!("Checked out in {}", path.display())
                                    }
                                    (Some(upstream), None) => format!("Tracks {upstream}"),
                                    (None, None) => branch.name.clone(),
                                };
                                let row = list_row(
                                    ui,
                                    &ListRow {
                                        lead: "",
                                        text: &branch.name,
                                        trail,
                                        tooltip: tip,
                                        selected: index == selected,
                                    },
                                );
                                if row.clicked() {
                                    actions.push(Command::SwitchBranch(
                                        branch.name.clone(),
                                        branch.remote,
                                    ));
                                }
                            }
                        });
                    ui.separator();
                    let typed = dialog.filter.trim().to_owned();
                    if !typed.is_empty()
                        && !dialog
                            .branches
                            .iter()
                            .any(|branch| !branch.remote && branch.name == typed)
                        && list_row(
                            ui,
                            &ListRow {
                                lead: "+",
                                text: &format!("Create branch “{typed}”"),
                                trail: "",
                                tooltip: "Create it from the current commit and switch to it"
                                    .into(),
                                selected: false,
                            },
                        )
                        .clicked()
                    {
                        dialog.new_name = typed;
                        actions.push(Command::CreateBranch);
                    }
                    if list_row(
                        ui,
                        &ListRow {
                            lead: "",
                            text: "Manage branches…",
                            trail: "",
                            tooltip: "Create, rename or delete branches".into(),
                            selected: false,
                        },
                    )
                    .clicked()
                    {
                        actions.push(Command::CloseBranchDialog);
                        actions.push(Command::OpenBranchDialog);
                    }
                });
            });
        let inside = output.response.rect;
        let (escape, pressed_at) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Escape),
                input
                    .pointer
                    .any_pressed()
                    .then(|| input.pointer.interact_pos())
                    .flatten(),
            )
        });
        // A press on the branch name itself toggles through its own command.
        let outside =
            pressed_at.is_some_and(|point| !inside.contains(point) && !anchor.contains(point));
        if escape || outside {
            actions.push(Command::CloseBranchDialog);
        }
        for action in actions {
            self.command(action);
        }
    }

    pub(crate) fn branch_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(dialog) = &mut self.view.branch_dialog else {
            return;
        };
        // The quick switcher is drawn by `branch_picker`.
        if dialog.quick {
            return;
        }
        let mut actions: Vec<Command> = Vec::new();
        let modal = egui::Modal::new(egui::Id::new("git_branch_dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Branches");
            ui.add_space(4.0);
            ui.add(
                TextEdit::singleline(&mut dialog.filter)
                    .id_salt("git_branch_filter")
                    .desired_width(f32::INFINITY)
                    .hint_text("Filter branches"),
            );
            let filter = dialog.filter.to_lowercase();
            let confirming = dialog.confirm_delete.clone();
            ScrollArea::vertical()
                .id_salt("git_branch_list")
                .max_height(240.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if dialog.loading && dialog.branches.is_empty() {
                        ui.label(RichText::new("Reading branches…").color(colors.muted));
                    }
                    for remote in [false, true] {
                        let mut heading_shown = false;
                        for branch in dialog.branches.iter().filter(|b| {
                            b.remote == remote && b.name.to_lowercase().contains(&filter)
                        }) {
                            if !heading_shown {
                                source_heading(ui, if remote { "REMOTE" } else { "LOCAL" }, 0);
                                heading_shown = true;
                            }
                            let mut label = branch.name.clone();
                            if branch.current {
                                label.push_str("  (current)");
                            }
                            if let Some(upstream) = &branch.upstream {
                                label.push_str(&format!("  tracks {upstream}"));
                            }
                            if branch.checked_out_at.is_some() {
                                label.push_str("  (in another worktree)");
                            }
                            let response = explorer_row(ui, label, 4, branch.current)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(match &branch.checked_out_at {
                                    Some(path) => format!("Checked out at {}", path.display()),
                                    None if remote => format!(
                                        "Create a local branch tracking {} and switch to it",
                                        branch.name
                                    ),
                                    None => format!("Switch to {}", branch.name),
                                });
                            if !branch.current {
                                let name = branch.name.clone();
                                let commit = branch.commit.clone();
                                response.context_menu(|ui| {
                                    if ui.button("Merge into current branch…").clicked() {
                                        actions.push(Command::PlanHistoryAction(
                                            HistoryAction::Merge {
                                                target: commit.clone(),
                                                no_ff: false,
                                            },
                                            format!("Merge {name}"),
                                        ));
                                        ui.close();
                                    }
                                });
                            }
                            let deletable =
                                !remote && !branch.current && branch.checked_out_at.is_none();
                            let delete_clicked = deletable
                                && ui
                                    .put(
                                        egui::Rect::from_min_size(
                                            response.rect.right_top() - egui::vec2(22.0, 0.0),
                                            egui::vec2(22.0, response.rect.height()),
                                        ),
                                        egui::Button::new(crate::icon::X).frame(false),
                                    )
                                    .on_hover_text("Delete this branch (only if fully merged)")
                                    .clicked();
                            if delete_clicked {
                                actions.push(Command::RequestDeleteBranch(branch.name.clone()));
                            } else if response.clicked() {
                                actions.push(Command::SwitchBranch(branch.name.clone(), remote));
                            }
                        }
                    }
                });
            if let Some(name) = &confirming {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Delete {name}? Git refuses if it has unmerged commits."
                    ))
                    .color(colors.muted),
                );
                ui.horizontal(|ui| {
                    if ui.button("Delete Branch").clicked() {
                        actions.push(Command::ConfirmDeleteBranch);
                    }
                    if ui.button("Keep").clicked() {
                        dialog.confirm_delete = None;
                    }
                });
            }
            ui.add_space(6.0);
            ui.label(RichText::new("New branch from the current commit").color(colors.muted));
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 110.0).max(40.0);
                ui.add(
                    TextEdit::singleline(&mut dialog.new_name)
                        .id_salt("git_branch_new")
                        .desired_width(width)
                        .hint_text("feature/name"),
                );
                if ui
                    .add_enabled(
                        !dialog.new_name.trim().is_empty(),
                        egui::Button::new("Create"),
                    )
                    .on_hover_text("Create the branch and switch to it")
                    .clicked()
                {
                    actions.push(Command::CreateBranch);
                }
            });
            ui.label(RichText::new("Rename current branch").color(colors.muted));
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 110.0).max(40.0);
                ui.add(
                    TextEdit::singleline(&mut dialog.rename_to)
                        .id_salt("git_branch_rename")
                        .desired_width(width)
                        .hint_text("new name"),
                );
                if ui
                    .add_enabled(
                        !dialog.rename_to.trim().is_empty(),
                        egui::Button::new("Rename"),
                    )
                    .clicked()
                {
                    actions.push(Command::RenameBranch);
                }
            });
            if let Some(error) = &dialog.error {
                ui.add_space(4.0);
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                actions.push(Command::CloseBranchDialog);
            }
        });
        if modal.should_close() {
            actions.push(Command::CloseBranchDialog);
        }
        for action in actions {
            self.command(action);
        }
    }

    pub(crate) fn clone_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(dialog) = &mut self.view.clone_dialog else {
            return;
        };
        let running = dialog.run.is_some();
        let progress = dialog.run.as_ref().map(|run| run.progress.clone());
        let error = dialog.error.clone();
        let target = dialog.target();
        let (mut browse, mut start, mut cancel) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("git_clone_dialog")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Clone Repository");
            ui.add_space(4.0);
            ui.label(RichText::new("Repository URL or path").color(colors.muted));
            let url = ui.add_enabled(
                !running,
                TextEdit::singleline(&mut dialog.url)
                    .id_salt("git_clone_url")
                    .desired_width(f32::INFINITY)
                    .hint_text("https://host/team/project.git"),
            );
            if url.changed() {
                dialog.sync_name();
            }
            ui.label(RichText::new("Destination folder").color(colors.muted));
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 80.0).max(40.0);
                ui.add_enabled(
                    !running,
                    TextEdit::singleline(&mut dialog.parent)
                        .id_salt("git_clone_parent")
                        .desired_width(width),
                );
                browse = ui
                    .add_enabled(!running, egui::Button::new("Browse…"))
                    .clicked();
            });
            ui.label(RichText::new("Folder name").color(colors.muted));
            let name = ui.add_enabled(
                !running,
                TextEdit::singleline(&mut dialog.name)
                    .id_salt("git_clone_name")
                    .desired_width(f32::INFINITY),
            );
            if name.changed() {
                dialog.name_edited = true;
            }
            ui.add_space(4.0);
            match (&progress, &error, &target) {
                (Some(progress), _, _) => {
                    ui.label(RichText::new(progress).color(colors.muted));
                }
                (None, Some(error), _) => {
                    ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                }
                (None, None, Ok(target)) => {
                    ui.label(
                        RichText::new(format!("Creates {}", target.display())).color(colors.muted),
                    );
                }
                (None, None, Err(reason)) if !dialog.url.is_empty() => {
                    ui.label(RichText::new(reason).color(colors.muted));
                }
                _ => {}
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                start = ui
                    .add_enabled(!running && target.is_ok(), egui::Button::new("Clone"))
                    .clicked();
                cancel = ui
                    .button(if running { "Cancel Clone" } else { "Cancel" })
                    .clicked();
            });
        });
        if browse {
            self.command(Command::PickCloneDestination);
        } else if start {
            self.command(Command::StartClone);
        } else if cancel || modal.should_close() {
            self.command(Command::CancelClone);
        }
    }

    pub(crate) fn source_abort_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.source_control.abort_prompt {
            return;
        }
        let label = self
            .view
            .source_control
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.state.label())
            .unwrap_or("The operation");
        let (mut confirm, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("source_control_abort")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading("Abort and go back?");
            ui.label(
                RichText::new(format!(
                    "{label} will be stopped and Git will try to return to the state before it began. Changes you made while resolving conflicts are discarded, and Git may be unable to restore uncommitted changes that existed beforehand."
                ))
                .color(colors.muted),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                confirm = ui.button("Abort").clicked();
                cancel = ui.button("Keep Going").clicked();
            });
        });
        if cancel || modal.should_close() {
            self.command(Command::CancelOperationAbort);
        } else if confirm {
            self.command(Command::ConfirmOperationAbort);
        }
    }

    pub(crate) fn source_revert_dialog(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(prompt) = &self.view.source_control.revert_prompt else {
            return;
        };
        let paths = prompt.paths.clone();
        let error = prompt.error.clone();
        let mut confirm = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("source_control_revert")).show(ctx, |ui| {
            ui.set_width(400.0);
            let count = paths.len();
            ui.heading(if count == 1 { "Discard changes to this file?" } else { "Discard changes to these files?" });
            ui.label(RichText::new("This restores the tracked file and its staged state from HEAD. This cannot be undone in Lumen.").color(colors.muted));
            ui.label(RichText::new("Untracked files are not removed.").color(colors.muted));
            ui.add_space(6.0);
            for path in paths.iter().take(4) {
                ui.label(path.display().to_string());
            }
            if count > 4 {
                ui.label(RichText::new(format!("…and {} more", count - 4)).color(colors.muted));
            }
            if let Some(error) = &error {
                ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                confirm = ui.add_enabled(!self.view.source_control.loading, egui::Button::new("Discard Changes")).clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || modal.should_close() {
            self.view.source_control.revert_prompt = None;
        } else if confirm {
            self.command(Command::ConfirmSourceRevert);
        }
    }

    pub(crate) fn source_diff_editor(&mut self, ui: &mut egui::Ui) {
        let path = self
            .view
            .source_control
            .diff
            .as_ref()
            .map(|diff| diff.root.join(&diff.path));
        let path = path.as_deref();
        let scheme = self
            .view
            .settings
            .document_choice("editor.color_scheme", path);
        let theme = crate::ui::theme::editor_theme(&scheme, ui.visuals());
        let original = ui.style().clone();
        *ui.visuals_mut() = crate::ui::theme::visuals(theme);
        self.source_diff_editor_themed(ui);
        ui.set_style(original);
    }

    pub(crate) fn source_diff_editor_themed(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let Some(diff) = &self.view.source_control.diff else {
            return;
        };
        let side = diff.side;
        let relative_path = diff.path.clone();
        let document = diff.document;
        let opened_revision = diff.opened_revision;
        let comparison = diff.comparison.clone();
        let path = diff.root.join(&diff.path);
        let document_index = document.and_then(|document| {
            self.editor
                .files
                .iter()
                .position(|file| file.id == document)
        });
        let font_size = self.view.settings.number("editor.font_size", Some(&path));
        let line_height = self.view.settings.number("editor.line_height", Some(&path));
        let font = if self.view.settings.font_family(Some(&path)) == "Hack" {
            FontId::new(font_size, egui::FontFamily::Name("lumen-hack".into()))
        } else {
            FontId::monospace(font_size)
        };
        match &comparison {
            Ok(comparison) => {
                if side == DiffSide::Working && !comparison.hunks.is_empty() {
                    let editable_is_dirty =
                        document_index.is_some_and(|index| self.editor.files[index].dirty);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Hunks").small().color(colors.muted));
                        for (index, hunk) in comparison.hunks.iter().enumerate() {
                            if ui
                                .add_enabled(
                                    !editable_is_dirty && !self.view.source_control.loading,
                                    egui::Button::new(format!(
                                        "{} Stage {}",
                                        crate::icon::PLUS,
                                        index + 1
                                    ))
                                    .small(),
                                )
                                .on_hover_text(format!("Stage only this hunk\n{}", hunk.header))
                                .clicked()
                            {
                                self.command(Command::StageHunk(relative_path.clone(), index));
                            }
                        }
                        if editable_is_dirty {
                            ui.label(
                                RichText::new("Save or discard editor edits before staging a hunk")
                                    .small()
                                    .color(colors.muted),
                            );
                        }
                    });
                }
                let key = (relative_path.clone(), side);
                if self.view.source_control.diff_scroll_key.as_ref() != Some(&key) {
                    self.view.source_control.diff_scroll =
                        crate::app::scroll_sync::ScrollSync::at_top();
                    self.view.source_control.diff_scroll_key = Some(key);
                }
                let mut sync = self.view.source_control.diff_scroll;
                ui.columns(2, |columns| {
                    columns[0].label(
                        RichText::new(match side {
                            DiffSide::Staged => "HEAD · read-only",
                            DiffSide::Working => "Index · read-only",
                            DiffSide::Untracked => "New file · no previous content",
                        })
                        .small()
                        .color(colors.muted),
                    );
                    let observed = read_only_source(
                        &mut columns[0],
                        &ReadOnlySource {
                            text: &comparison.base,
                            changed: &comparison.old_lines,
                            changed_background: colors.diff_removed_bg,
                            id: "git_diff_base",
                            font: &font,
                            line_height,
                        },
                        sync.take_pending(0),
                    );
                    sync.observe(0, observed);
                    if let Some(index) = document_index {
                        let dirty = self.editor.files[index].dirty;
                        let changed_since_open =
                            opened_revision != Some(self.editor.files[index].revision);
                        columns[1].label(
                            RichText::new(if changed_since_open {
                                "Working Tree · editable · unsaved edits; diff markers hidden"
                            } else if dirty {
                                "Working Tree · editable · unsaved changes"
                            } else {
                                "Working Tree · editable"
                            })
                            .small()
                            .color(colors.muted),
                        );
                        let document = self.editor.files[index].id;
                        self.editor_find_bar(&mut columns[1], document);
                        // The editor pane takes and reports its scroll position
                        // through the shared sync state while this flag is set.
                        self.view.source_control.diff_scroll = sync;
                        self.view.source_control.diff_scroll_hook = true;
                        self.editor_viewport(&mut columns[1], index);
                        self.view.source_control.diff_scroll_hook = false;
                        sync = self.view.source_control.diff_scroll;
                    } else {
                        columns[1].label(
                            RichText::new(if side == DiffSide::Staged {
                                "Index · read-only"
                            } else {
                                "Working Tree · file missing"
                            })
                            .small()
                            .color(colors.muted),
                        );
                        let observed = read_only_source(
                            &mut columns[1],
                            &ReadOnlySource {
                                text: comparison.changed.as_deref().unwrap_or_default(),
                                changed: &comparison.new_lines,
                                changed_background: colors.diff_added_bg,
                                id: "git_diff_changed",
                                font: &font,
                                line_height,
                            },
                            sync.take_pending(1),
                        );
                        sync.observe(1, observed);
                    }
                });
                let needs_frame = sync.finish_frame();
                self.view.source_control.diff_scroll = sync;
                if needs_frame {
                    ui.ctx().request_repaint();
                }
            }
            Err(error) => {
                ui.label(RichText::new(error).color(colors.error));
            }
        }
    }
}

/// One read-only side of a diff.
struct ReadOnlySource<'a> {
    text: &'a str,
    changed: &'a BTreeSet<usize>,
    changed_background: egui::Color32,
    id: &'a str,
    font: &'a FontId,
    line_height: f32,
}

/// Draws the pane, first jumping to `scroll_to_line` when given, and returns
/// the line now at the top of the viewport.
fn read_only_source(
    ui: &mut egui::Ui,
    source: &ReadOnlySource,
    scroll_to_line: Option<f32>,
) -> f32 {
    let colors = crate::ui::theme::palette(ui.visuals());
    let ReadOnlySource {
        text,
        changed,
        changed_background,
        id,
        font,
        line_height,
    } = *source;
    let lines: Vec<_> = text.split('\n').collect();
    let width = lines
        .iter()
        .map(|line| line.len().min(2_000))
        .max()
        .unwrap_or(0) as f32
        * font.size
        * 0.65
        + 70.0;
    let height = line_height.max(16.0);
    let mut area = ScrollArea::both().id_salt(id).auto_shrink([false, false]);
    if let Some(line) = scroll_to_line {
        area = area.vertical_scroll_offset(line * (height + ui.spacing().item_spacing.y));
    }
    let output = area.show_rows(ui, height, lines.len(), |ui, rows| {
        ui.set_min_width(width.max(ui.available_width()));
        for index in rows {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(width.max(ui.available_width()), height),
                egui::Sense::hover(),
            );
            if changed.contains(&(index + 1)) {
                ui.painter().rect_filled(rect, 0.0, changed_background);
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(height);
                    ui.label(
                        RichText::new(format!("{:>4}", index + 1))
                            .font(font.clone())
                            .color(colors.muted),
                    );
                    ui.label(
                        RichText::new(lines[index])
                            .font(font.clone())
                            .color(colors.text),
                    );
                });
            });
        }
    });
    // Report the position in lines so differently sized panes can be compared.
    output.state.offset.y / (height + ui.spacing().item_spacing.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme;
    use crate::{
        app::{
            LumenApp,
            view_state::{Activity, SourceDiffView, ViewState},
        },
        model::EditorState,
        services::{
            LocalFileSystem,
            source_control::{GitChange, GitComparison, GitHunk, GitSnapshot},
        },
    };
    use std::{cell::Cell, path::PathBuf};

    fn source_sidebar_width(
        app: &mut LumenApp,
        ctx: &egui::Context,
        requested: f32,
        pointer: egui::Pos2,
    ) -> f32 {
        ctx.data_mut(|data| {
            data.insert_persisted(
                egui::Id::new("explorer"),
                egui::containers::panel::PanelState {
                    rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(requested, 600.0)),
                },
            );
        });
        let width = Cell::new(0.0);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |workbench| workbench.sidebar(ctx));
                width.set(ctx.available_rect().left());
            },
        );
        width.get()
    }

    fn source_sidebar_frame(
        app: &mut LumenApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> f32 {
        let width = Cell::new(0.0);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |workbench| workbench.sidebar(ctx));
                width.set(ctx.available_rect().left());
            },
        );
        width.get()
    }

    #[test]
    fn source_sidebar_does_not_grow_as_git_status_arrives() {
        let mut app = LumenApp::default();
        app.view.activity = Activity::SourceControl;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        let mut widths = vec![source_sidebar_frame(&mut app, &ctx, vec![])];
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from("/workspace"),
            branch: "feature/a-branch-name-long-enough-to-previously-grow-the-sidebar".into(),
            changes: vec![GitChange::new(PathBuf::from("src/long-file.rs"), 'M', 'M')],
            ..Default::default()
        });
        for point in [
            egui::pos2(850.0, 400.0),
            egui::pos2(120.0, 95.0),
            egui::pos2(120.0, 180.0),
            egui::pos2(850.0, 400.0),
        ] {
            widths.push(source_sidebar_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(point)],
            ));
        }
        app.view.source_control.amend_commit = true;
        app.view.source_control.commit_message = "a long commit message which must clip".into();
        app.view.source_control.commit_error = Some("a long Git error which must clip".into());
        for _ in 0..3 {
            widths.push(source_sidebar_frame(&mut app, &ctx, vec![]));
        }
        assert!(
            widths.iter().all(|width| (*width - widths[0]).abs() < 2.0),
            "source sidebar changed width: {widths:?}"
        );

        let edge = egui::pos2(widths[0] - 1.0, 300.0);
        source_sidebar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
        source_sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: edge,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let narrow = source_sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(edge - egui::vec2(58.0, 0.0))],
        );
        assert!(
            narrow < widths[0] - 40.0,
            "sidebar could not shrink: {widths:?} -> {narrow}"
        );
        source_sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: edge - egui::vec2(58.0, 0.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let retained: Vec<_> = (0..4)
            .map(|_| source_sidebar_frame(&mut app, &ctx, vec![]))
            .collect();
        assert!(
            retained.iter().all(|width| (*width - narrow).abs() < 2.0),
            "narrow width drifted: {narrow} -> {retained:?}"
        );
    }

    #[test]
    fn commit_message_keeps_focus_during_background_refresh_and_error() {
        let mut app = LumenApp::default();
        app.view.activity = Activity::SourceControl;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from("/workspace"),
            branch: "main".into(),
            changes: vec![GitChange::new(PathBuf::from("src/main.rs"), 'M', ' ')],
            ..Default::default()
        });
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        app.view.source_control.commit_focus = true;
        source_sidebar_frame(&mut app, &ctx, vec![]);
        assert!(app.view.source_control.commit_focused);

        app.view.source_control.loading = true;
        source_sidebar_frame(&mut app, &ctx, vec![]);
        assert!(app.view.source_control.commit_focused);

        app.view.source_control.loading = false;
        app.view.source_control.error = Some("Git status temporarily failed".into());
        source_sidebar_frame(&mut app, &ctx, vec![]);
        assert!(app.view.source_control.commit_focused);

        app.view.source_control.error = None;
        app.view
            .source_control
            .snapshot
            .as_mut()
            .unwrap()
            .changes
            .clear();
        source_sidebar_frame(&mut app, &ctx, vec![]);
        assert!(
            app.view.source_control.commit_focused,
            "an empty staged list must not disable the message field"
        );
    }

    #[test]
    fn source_control_content_and_hover_never_expand_the_sidebar() {
        let mut app = LumenApp::default();
        app.view.activity = Activity::SourceControl;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from("/workspace"),
            branch: "feature/a-branch-name-long-enough-to-previously-grow-the-sidebar".into(),
            remotes: vec![crate::services::source_control::GitRemote {
                name: "origin".into(),
                fetch_url: "https://example.com/a/remarkably/long/repository/path/that/clips.git"
                    .into(),
                push_url: None,
            }],
            last_commit: Some("A very long last commit subject that must clip in the pane".into()),
            changes: vec![GitChange::new(
                PathBuf::from("src/a-very-long-file-name-that-must-stay-clipped.rs"),
                ' ',
                'M',
            )],
            ..Default::default()
        });
        app.view.source_control.amend_commit = true;
        app.view.source_control.commit_message = "line one\nline two\nline three\nfour".into();
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        for requested in [180.0, 240.0, 600.0] {
            let widths = [
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(850.0, 400.0)),
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(120.0, 95.0)),
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(120.0, 180.0)),
            ];
            assert!(
                widths.iter().all(|width| (*width - requested).abs() < 2.0),
                "source content widened sidebar {requested}: {widths:?}"
            );
        }
        let long_repo = |name: &str| GitSnapshot {
            root: PathBuf::from(format!("/workspace/{name}")),
            branch: "feature/a-branch-name-long-enough-to-previously-grow-the-sidebar".into(),
            ..Default::default()
        };
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.source_control.repositories = vec![
            long_repo("a-very-long-repository-directory-name"),
            long_repo("another-quite-long-nested-repository"),
        ];
        app.view.source_control.repository_errors = vec![(
            PathBuf::from("/workspace/broken-repository-with-a-long-name"),
            "could not read status because of an unusually long failure".into(),
        )];
        for requested in [180.0, 240.0, 600.0] {
            let width = source_sidebar_width(&mut app, &ctx, requested, egui::pos2(850.0, 400.0));
            assert!(
                (width - requested).abs() < 2.0,
                "repository list widened sidebar {requested}: {width}"
            );
        }
        app.view.source_control.repositories.clear();
        app.view.source_control.repository_errors.clear();
        app.view.source_control.snapshot = None;
        app.view.source_control.error = Some(
            "Git status failed because an unusually long error must remain inside the pane".into(),
        );
        let width = source_sidebar_width(&mut app, &ctx, 180.0, egui::pos2(120.0, 95.0));
        assert!(
            (width - 180.0).abs() < 2.0,
            "source error widened sidebar: {width}"
        );
    }

    #[test]
    fn source_rows_stay_left_aligned_at_panel_widths() {
        for width in [180.0, 300.0, 600.0] {
            let ctx = egui::Context::default();
            theme::configure_ui(&ctx);
            crate::platform::fonts::configure(&ctx);
            let mut editor = EditorState::with_workspace(PathBuf::from("/workspace"));
            let mut view = ViewState::default();
            view.source_control.snapshot = Some(GitSnapshot {
                root: PathBuf::from("/workspace"),
                branch: "main".into(),
                changes: vec![GitChange::new(PathBuf::from("src/file.rs"), ' ', 'M')],
                ..Default::default()
            });
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 200.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut workbench =
                            Workbench::new(&mut editor, &mut view, &LocalFileSystem);
                        workbench.source_control_panel(ui);
                    });
                },
            );
            for expected in ["main", "file.rs"] {
                let x = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().contains(expected) => {
                        Some(text.pos.x)
                    }
                    _ => None,
                });
                // The branch text follows its icon; both labels still begin
                // in the left portion of the panel rather than being centred.
                assert!(
                    x.is_some_and(|x| x < width * 0.55),
                    "{width}: {expected} at {x:?}"
                );
            }
        }
    }

    #[test]
    fn staged_working_and_untracked_changes_get_distinct_rows() {
        let snapshot = GitSnapshot {
            root: PathBuf::from("/workspace"),
            branch: "main".into(),
            changes: vec![
                GitChange::new(PathBuf::from("both.rs"), 'M', 'M'),
                GitChange::new(PathBuf::from("new.rs"), '?', '?'),
            ],
            ..Default::default()
        };
        let rows = source_rows(&snapshot);
        assert!(matches!(
            rows[0],
            SourceRow::Heading(ChangeGroup::Staged, 1)
        ));
        assert!(matches!(rows[1], SourceRow::Change(_, DiffSide::Staged)));
        assert!(matches!(
            rows[2],
            SourceRow::Heading(ChangeGroup::Working, 2)
        ));
        assert!(matches!(rows[3], SourceRow::Change(_, DiffSide::Working)));
        assert!(matches!(rows[4], SourceRow::Change(_, DiffSide::Untracked)));
    }

    #[test]
    fn staged_diff_uses_both_editor_columns_at_multiple_widths() {
        for width in [620.0, 1100.0] {
            let ctx = egui::Context::default();
            theme::configure_ui(&ctx);
            crate::platform::fonts::configure(&ctx);
            let mut editor = EditorState::default();
            let mut view = ViewState::default();
            view.source_control.diff = Some(SourceDiffView {
                root: PathBuf::from("/workspace"),
                path: PathBuf::from("file.rs"),
                side: DiffSide::Staged,
                comparison: Ok(std::sync::Arc::new(GitComparison {
                    base: "before\n".into(),
                    changed: Some("after\n".into()),
                    old_lines: [1].into(),
                    new_lines: [1].into(),
                    hunks: Vec::new(),
                })),
                document: None,
                diff_only_document: None,
                opened_revision: None,
                active: true,
            });
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 400.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut workbench =
                            Workbench::new(&mut editor, &mut view, &LocalFileSystem);
                        workbench.source_diff_editor(ui);
                    });
                },
            );
            let positions: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if ["before", "after"].contains(&text.galley.text()) =>
                    {
                        Some((text.galley.text().to_owned(), text.pos.x))
                    }
                    _ => None,
                })
                .collect();
            let before = positions
                .iter()
                .find(|(text, _)| text == "before")
                .unwrap()
                .1;
            let after = positions
                .iter()
                .find(|(text, _)| text == "after")
                .unwrap()
                .1;
            assert!(
                before < width / 2.0 && after > width / 2.0,
                "{width}: {positions:?}"
            );
        }
    }

    #[test]
    fn conflicts_form_their_own_group_and_leave_staged_and_working_lists() {
        use crate::services::source_control::ChangeKind;
        let mut conflict = GitChange::new("both.rs", 'U', 'U');
        conflict.kind = ChangeKind::Conflict;
        let snapshot = GitSnapshot {
            changes: vec![conflict, GitChange::new("ok.rs", 'M', ' ')],
            ..Default::default()
        };
        let rows = source_rows(&snapshot);
        let headings: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                SourceRow::Heading(group, count) => Some((group.title(), *count)),
                _ => None,
            })
            .collect();
        assert_eq!(headings, [("MERGE CONFLICTS", 1), ("STAGED CHANGES", 1)]);
    }

    #[test]
    fn tooltip_shows_ahead_behind_and_operation_state() {
        use crate::services::source_control::{GitUpstream, RepoState};
        let snapshot = GitSnapshot {
            branch: "main".into(),
            upstream: Some(GitUpstream {
                name: "origin/main".into(),
                ahead_behind: Some((2, 1)),
            }),
            state: RepoState::Merge,
            ..Default::default()
        };
        let details = branch_details(&snapshot);
        assert!(details.contains("2 ahead, 1 behind"), "{details}");
        assert!(details.contains("Merging is in progress"), "{details}");
    }

    #[test]
    fn working_diff_exposes_compact_stage_buttons_for_each_hunk() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        let mut editor = EditorState::default();
        let mut view = ViewState::default();
        view.source_control.diff = Some(SourceDiffView {
            root: PathBuf::from("/workspace"),
            path: PathBuf::from("file.rs"),
            side: DiffSide::Working,
            comparison: Ok(std::sync::Arc::new(GitComparison {
                base: "before\n".into(),
                changed: Some("after\n".into()),
                old_lines: [1].into(),
                new_lines: [1].into(),
                hunks: vec![
                    GitHunk {
                        header: "@@ -1 +1 @@".into(),
                        patch: String::new(),
                    },
                    GitHunk {
                        header: "@@ -8 +8 @@".into(),
                        patch: String::new(),
                    },
                ],
            })),
            document: None,
            diff_only_document: None,
            opened_revision: None,
            active: true,
        });
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut workbench = Workbench::new(&mut editor, &mut view, &LocalFileSystem);
                workbench.source_diff_editor(ui);
            });
        });
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(labels.iter().any(|text| text.contains("Stage 1")));
        assert!(labels.iter().any(|text| text.contains("Stage 2")));
    }

    fn numbered_text(prefix: &str, count: usize) -> String {
        (1..=count).map(|n| format!("{prefix}{n}\n")).collect()
    }

    struct DiffHarness {
        ctx: egui::Context,
        editor: EditorState,
        view: ViewState,
    }

    impl DiffHarness {
        fn staged(lines: usize) -> Self {
            let ctx = egui::Context::default();
            theme::configure_ui(&ctx);
            crate::platform::fonts::configure(&ctx);
            let mut view = ViewState::default();
            view.source_control.diff = Some(SourceDiffView {
                root: PathBuf::from("/workspace"),
                path: PathBuf::from("file.rs"),
                side: DiffSide::Staged,
                comparison: Ok(std::sync::Arc::new(GitComparison {
                    base: numbered_text("old", lines),
                    changed: Some(numbered_text("new", lines)),
                    old_lines: Default::default(),
                    new_lines: Default::default(),
                    hunks: Vec::new(),
                })),
                document: None,
                diff_only_document: None,
                opened_revision: None,
                active: true,
            });
            Self {
                ctx,
                editor: EditorState::default(),
                view,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let (editor, view) = (&mut self.editor, &mut self.view);
            self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut workbench = Workbench::new(editor, view, &LocalFileSystem);
                        workbench.source_diff_editor(ui);
                    });
                },
            )
        }

        fn settle(&mut self) {
            // Wheel scrolling is smoothed over several frames; let it finish.
            for _ in 0..40 {
                self.frame(vec![]);
            }
        }

        fn wheel(&mut self, x: f32, lines: f32) {
            let pos = egui::pos2(x, 200.0);
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -lines),
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            self.settle();
        }

        /// Smallest visible line number in the left and right pane.
        fn first_lines(&mut self) -> (usize, usize) {
            let output = self.frame(vec![]);
            let (mut left, mut right) = (usize::MAX, usize::MAX);
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape
                    && let Some(number) = text
                        .galley
                        .text()
                        .strip_prefix("old")
                        .map(|n| (n, true))
                        .or_else(|| text.galley.text().strip_prefix("new").map(|n| (n, false)))
                    && let Ok(n) = number.0.trim().parse::<usize>()
                {
                    if number.1 {
                        left = left.min(n);
                    } else {
                        right = right.min(n);
                    }
                }
            }
            (left, right)
        }
    }

    #[test]
    fn scrolling_either_diff_pane_moves_the_other_to_the_same_line() {
        let mut diff = DiffHarness::staged(400);
        diff.settle();
        assert_eq!(diff.first_lines(), (1, 1));

        diff.wheel(250.0, 30.0);
        let (left, right) = diff.first_lines();
        assert!(left > 5, "left pane scrolled: {left}");
        assert_eq!(left, right, "right pane follows the left");

        let before = left;
        diff.wheel(750.0, -12.0);
        let (left, right) = diff.first_lines();
        assert!(
            left < before,
            "scrolling the right pane moved the left: {left}"
        );
        assert_eq!(left, right, "left pane follows the right");
    }

    #[test]
    fn a_different_diff_resets_the_sync_memory() {
        let mut diff = DiffHarness::staged(400);
        diff.settle();
        diff.wheel(250.0, 40.0);
        assert!(diff.view.source_control.diff_scroll.positions()[0] > 5.0);
        diff.view.source_control.diff.as_mut().unwrap().path = PathBuf::from("other.rs");
        diff.settle();
        assert_eq!(
            diff.view.source_control.diff_scroll_key,
            Some((PathBuf::from("other.rs"), DiffSide::Staged))
        );
        assert_eq!(
            diff.first_lines(),
            (1, 1),
            "a different file's diff opens at the top, not at the old scroll position"
        );
    }

    #[cfg(feature = "shared-editor")]
    #[test]
    fn the_editable_working_tree_pane_follows_the_base_pane_too() {
        let mut diff = DiffHarness::staged(400);
        let text = numbered_text("new", 400);
        let mut document =
            crate::model::Document::from_path(PathBuf::from("/workspace/file.rs"), text.clone());
        document.enable_shared_editing();
        let id = document.id;
        diff.editor.files.push(document);
        diff.editor.set_active_index(Some(0));
        let state = diff.view.source_control.diff.as_mut().unwrap();
        state.side = DiffSide::Working;
        state.document = Some(id);
        state.opened_revision = Some(0);
        diff.settle();

        diff.wheel(250.0, 35.0);
        let [left, right] = diff.view.source_control.diff_scroll.positions();
        assert!(left > 5.0, "base pane scrolled: {left}");
        assert!(
            (left - right).abs() < 0.5,
            "editor pane at line {right}, base pane at line {left}"
        );

        // And the other way: wheel over the editor, base pane follows.
        diff.wheel(750.0, -20.0);
        let [left_after, right_after] = diff.view.source_control.diff_scroll.positions();
        assert!(
            left_after < left,
            "base pane followed the editor: {left_after}"
        );
        assert!((left_after - right_after).abs() < 1.0);
    }

    fn repo(root: &str, branch: &str, changes: Vec<GitChange>) -> GitSnapshot {
        GitSnapshot {
            root: PathBuf::from(root),
            branch: branch.into(),
            changes,
            ..Default::default()
        }
    }

    fn text_x(output: &egui::FullOutput, needle: &str) -> Option<f32> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains(needle) => Some(text.pos.x),
            _ => None,
        })
    }

    fn several_repositories() -> (GitSnapshot, Vec<GitSnapshot>) {
        let selected = repo(
            "/workspace",
            "feature/a-branch-name-long-enough-to-previously-grow-the-sidebar",
            vec![
                GitChange::new(PathBuf::from("src/a-long-file-name.rs"), 'M', 'M'),
                GitChange::new(PathBuf::from("notes.md"), '?', '?'),
            ],
        );
        let others = vec![
            selected.clone(),
            repo(
                "/workspace/crates/settings-core-with-a-long-name",
                "main",
                vec![],
            ),
            repo(
                "/workspace/tools/vendor/a-submodule",
                "detached",
                vec![GitChange::new(PathBuf::from("lib.c"), ' ', 'M')],
            ),
        ];
        (selected, others)
    }

    #[test]
    fn several_repositories_never_widen_the_sidebar_in_any_state() {
        let (selected, all) = several_repositories();
        let mut app = LumenApp::default();
        app.view.activity = Activity::SourceControl;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.source_control.snapshot = Some(selected);
        app.view.source_control.repositories = all;
        app.view.source_control.repository_errors = vec![(
            PathBuf::from("/workspace/archive/legacy-with-a-long-name"),
            "cannot read status: dubious ownership in repository".into(),
        )];
        app.view.source_control.amend_commit = true;
        app.view.source_control.commit_message = "message\nover\nlines".into();
        app.view.source_control.parked_drafts.insert(
            PathBuf::from("/workspace/tools/vendor/a-submodule"),
            Default::default(),
        );
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        for requested in [180.0, 240.0, 600.0] {
            let widths = [
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(850.0, 400.0)),
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(120.0, 95.0)),
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(120.0, 180.0)),
                source_sidebar_width(&mut app, &ctx, requested, egui::pos2(120.0, 260.0)),
            ];
            assert!(
                widths.iter().all(|width| (*width - requested).abs() < 2.0),
                "{requested}: {widths:?}"
            );
        }
        // Collapsed sections and groups change height, never width.
        app.view
            .source_control
            .collapsed_repos
            .insert(PathBuf::from("/workspace"));
        let width = source_sidebar_width(&mut app, &ctx, 240.0, egui::pos2(850.0, 400.0));
        assert!((width - 240.0).abs() < 2.0, "collapsed: {width}");
    }

    #[test]
    fn selecting_a_collapsed_repository_issues_a_typed_selection_command() {
        let (selected, all) = several_repositories();
        let mut editor = EditorState::with_workspace(PathBuf::from("/workspace"));
        let mut view = ViewState::default();
        view.source_control.snapshot = Some(selected);
        view.source_control.repositories = all;
        let target = PathBuf::from("/workspace/crates/settings-core-with-a-long-name");
        view.source_control.collapsed_repos.insert(target.clone());
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        let frame = |ctx: &egui::Context,
                     editor: &mut EditorState,
                     view: &mut ViewState,
                     events: Vec<egui::Event>| {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut workbench = Workbench::new(editor, view, &LocalFileSystem);
                        workbench.source_control_panel(ui);
                        let taken = workbench.take_commands();
                        ui.data_mut(|data| data.insert_temp(egui::Id::new("taken"), taken));
                    });
                },
            );
            let taken: Vec<Command> = ctx
                .data(|data| data.get_temp(egui::Id::new("taken")))
                .unwrap_or_default();
            (output, taken)
        };
        let (output, _) = frame(&ctx, &mut editor, &mut view, vec![]);
        let x = text_x(&output, "settings-core").expect("collapsed repository row is drawn");
        let y = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("settings-core") => {
                    Some(text.pos.y + 6.0)
                }
                _ => None,
            })
            .unwrap();
        let at = egui::pos2(x + 10.0, y);
        frame(
            &ctx,
            &mut editor,
            &mut view,
            vec![egui::Event::PointerMoved(at)],
        );
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut editor, &mut view, vec![button(true)]);
        let (_, commands) = frame(&ctx, &mut editor, &mut view, vec![button(false)]);
        assert!(
            commands.contains(&Command::SelectRepository(PathBuf::from(
                "/workspace/crates/settings-core-with-a-long-name"
            ))),
            "{commands:?}"
        );
        assert!(
            !view.source_control.collapsed_repos.contains(&target),
            "choosing a repository must show it open"
        );
    }

    #[test]
    fn a_lone_repository_cannot_stay_collapsed() {
        let mut app = LumenApp::default();
        app.view.activity = Activity::SourceControl;
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.source_control.snapshot = Some(repo(
            "/workspace",
            "main",
            vec![GitChange::new(PathBuf::from("a.rs"), ' ', 'M')],
        ));
        app.view
            .source_control
            .collapsed_repos
            .insert(PathBuf::from("/workspace"));
        app.view.source_control.commit_focus = true;
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        source_sidebar_frame(&mut app, &ctx, vec![]);
        assert!(
            app.view.source_control.commit_focused,
            "the commit box must be drawn for a single repository"
        );
    }

    #[test]
    fn the_header_menu_stays_at_the_right_edge_at_every_width() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        for requested in [180.0, 240.0, 600.0] {
            let mut app = LumenApp::default();
            app.view.activity = Activity::SourceControl;
            app.editor.workspace = Some(PathBuf::from("/workspace"));
            app.view.source_control.snapshot = Some(repo("/workspace", "main", vec![]));
            ctx.data_mut(|data| {
                data.insert_persisted(
                    egui::Id::new("explorer"),
                    egui::containers::panel::PanelState {
                        rect: egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(requested, 600.0),
                        ),
                    },
                );
            });
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_component(ctx, |workbench| workbench.sidebar(ctx)),
            );
            let menu = text_x(&output, crate::icon::ELLIPSIS).expect("header menu button");
            let title = text_x(&output, "SOURCE CONTROL").expect("header title");
            assert!(
                menu > requested - 40.0 && menu < requested,
                "{requested}: menu at {menu}"
            );
            assert!(title < 40.0, "{requested}: title at {title}");
        }
    }

    fn quick_branches() -> Vec<crate::services::source_control::GitBranch> {
        use crate::services::source_control::GitBranch;
        let branch = |name: &str, remote: bool, current: bool| GitBranch {
            name: name.into(),
            remote,
            current,
            ..Default::default()
        };
        vec![
            branch("main", false, true),
            branch("feature/login", false, false),
            branch("origin/main", true, false),
        ]
    }

    fn picker_app(filter: &str) -> LumenApp {
        let mut app = LumenApp::default();
        app.editor.workspace = Some(PathBuf::from("/workspace"));
        app.view.source_control.snapshot = Some(repo("/workspace", "main", vec![]));
        let mut dialog = crate::app::git_branches::BranchDialog::default();
        dialog.root = PathBuf::from("/workspace");
        dialog.branches = quick_branches();
        dialog.filter = filter.into();
        dialog.quick = true;
        app.view.branch_dialog = Some(dialog);
        app
    }

    fn run_picker(
        app: &mut LumenApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<Command>) {
        run_picker_with_chip(app, ctx, events, true)
    }

    /// `chip` stands in for the sidebar drawing the branch name this frame.
    fn run_picker_with_chip(
        app: &mut LumenApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        chip: bool,
    ) -> (egui::FullOutput, Vec<Command>) {
        let mut taken = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                if chip {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(20.0, 60.0), egui::vec2(90.0, 20.0));
                    let frame = ctx.cumulative_frame_nr();
                    ctx.data_mut(|data| {
                        data.insert_temp(egui::Id::new("branch_chip_rect"), (rect, frame))
                    });
                }
                let mut workbench =
                    Workbench::new(&mut app.editor, &mut app.view, &LocalFileSystem);
                workbench.branch_picker(ctx);
                taken = workbench.take_commands();
            },
        );
        (output, taken)
    }

    fn fixture() -> egui::Context {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        ctx
    }

    fn click(at: egui::Pos2) -> [egui::Event; 2] {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        [button(true), button(false)]
    }

    #[test]
    fn the_quick_switcher_lists_filters_and_switches_branches() {
        let ctx = fixture();
        let mut app = picker_app("");
        run_picker(&mut app, &ctx, vec![]);
        let (output, _) = run_picker(&mut app, &ctx, vec![]);
        for name in ["main", "feature/login", "origin/main", "LOCAL", "REMOTE"] {
            assert!(text_x(&output, name).is_some(), "{name} is listed");
        }

        // Filtering keeps only matches, so the others disappear.
        let mut app = picker_app("login");
        run_picker(&mut app, &ctx, vec![]);
        let (output, _) = run_picker(&mut app, &ctx, vec![]);
        assert!(text_x(&output, "feature/login").is_some());
        assert!(text_x(&output, "origin/main").is_none());
        assert!(
            text_x(&output, "Create branch").is_some(),
            "an unmatched name offers creation"
        );

        // Clicking a branch asks to switch to it.
        let bounds = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "feature/login" => Some(text.pos),
                _ => None,
            })
            .unwrap();
        let at = bounds + egui::vec2(10.0, 6.0);
        run_picker(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        let mut commands = Vec::new();
        for event in click(at) {
            commands.extend(run_picker(&mut app, &ctx, vec![event]).1);
        }
        assert!(
            commands.contains(&Command::SwitchBranch("feature/login".into(), false)),
            "{commands:?}"
        );
    }

    #[test]
    fn the_quick_switcher_creates_from_the_typed_name_and_closes_on_escape_or_outside() {
        let ctx = fixture();
        let mut app = picker_app("brand-new");
        run_picker(&mut app, &ctx, vec![]);
        let (output, _) = run_picker(&mut app, &ctx, vec![]);
        let at = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("Create branch") => {
                    Some(text.pos + egui::vec2(10.0, 6.0))
                }
                _ => None,
            })
            .expect("a create row");
        run_picker(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        let mut commands = Vec::new();
        for event in click(at) {
            commands.extend(run_picker(&mut app, &ctx, vec![event]).1);
        }
        assert!(commands.contains(&Command::CreateBranch), "{commands:?}");
        assert_eq!(
            app.view.branch_dialog.as_ref().unwrap().new_name,
            "brand-new"
        );

        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (_, commands) = run_picker(&mut app, &ctx, vec![escape]);
        assert!(commands.contains(&Command::CloseBranchDialog));

        let far = egui::pos2(850.0, 550.0);
        run_picker(&mut app, &ctx, vec![egui::Event::PointerMoved(far)]);
        let (_, commands) = run_picker(&mut app, &ctx, vec![click(far)[0].clone()]);
        assert!(
            commands.contains(&Command::CloseBranchDialog),
            "a press outside closes it: {commands:?}"
        );
    }

    #[test]
    fn the_full_dialog_is_not_drawn_while_the_quick_switcher_is_open() {
        let ctx = fixture();
        let mut app = picker_app("");
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                let mut workbench =
                    Workbench::new(&mut app.editor, &mut app.view, &LocalFileSystem);
                workbench.branch_dialog(ctx);
            },
        );
        assert!(text_x(&output, "Rename current branch").is_none());
    }

    fn extras(
        stashes: &[&str],
        outgoing: &[&str],
        incoming: &[&str],
    ) -> crate::app::repo_extras::RepoExtras {
        use crate::services::source_control::{GitCommit, GitStash, SyncCommits};
        let commit = |n: usize, subject: &&str| GitCommit {
            id: format!("{n:07x}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            subject: (*subject).into(),
            author: "A".into(),
            time: 1_700_000_000,
            ..Default::default()
        };
        crate::app::repo_extras::RepoExtras::for_test(
            "/workspace",
            stashes
                .iter()
                .enumerate()
                .map(|(index, message)| GitStash {
                    id: format!("{index}"),
                    index,
                    message: (*message).into(),
                })
                .collect(),
            SyncCommits {
                outgoing: outgoing
                    .iter()
                    .enumerate()
                    .map(|(n, s)| commit(n, s))
                    .collect(),
                incoming: incoming
                    .iter()
                    .enumerate()
                    .map(|(n, s)| commit(n + 50, s))
                    .collect(),
            },
        )
    }

    fn render_panel(
        editor: &mut EditorState,
        view: &mut ViewState,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<Command>) {
        let mut taken = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut workbench = Workbench::new(editor, view, &LocalFileSystem);
                    workbench.source_control_panel(ui);
                    taken = workbench.take_commands();
                });
            },
        );
        (output, taken)
    }

    #[test]
    fn stashes_and_sync_sections_show_counts_open_on_demand_and_lead_to_their_tools() {
        let ctx = fixture();
        let mut editor = EditorState::with_workspace(PathBuf::from("/workspace"));
        let mut view = ViewState::default();
        view.source_control.snapshot = Some(repo("/workspace", "main", vec![]));
        view.source_control.extras.loaded =
            Some(extras(&["WIP on login"], &["add feature", "fix typo"], &[]));
        let (output, _) = render_panel(&mut editor, &mut view, &ctx, 320.0, vec![]);
        for title in ["STASHES", "OUTGOING"] {
            assert!(text_x(&output, title).is_some(), "{title} header");
        }
        assert!(
            text_x(&output, "INCOMING").is_none(),
            "empty lists have no section"
        );
        assert!(
            text_x(&output, "add feature").is_none(),
            "sections start collapsed"
        );

        view.source_control.expanded_extras.insert((
            PathBuf::from("/workspace"),
            crate::app::view_state::ExtraSection::Outgoing,
        ));
        view.source_control.expanded_extras.insert((
            PathBuf::from("/workspace"),
            crate::app::view_state::ExtraSection::Stashes,
        ));
        let (output, _) = render_panel(&mut editor, &mut view, &ctx, 320.0, vec![]);
        assert!(text_x(&output, "add feature").is_some());
        assert!(text_x(&output, "WIP on login").is_some());

        let position = |needle: &str| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                    Some(text.pos + egui::vec2(8.0, 6.0))
                }
                _ => None,
            })
        };
        let commit_at = position("add feature").unwrap();
        let stash_at = position("WIP on login").unwrap();
        for (at, expected) in [
            (
                commit_at,
                Command::OpenHistoryAt("0000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            ),
            (stash_at, Command::OpenStashDialog),
        ] {
            render_panel(
                &mut editor,
                &mut view,
                &ctx,
                320.0,
                vec![egui::Event::PointerMoved(at)],
            );
            let mut commands = Vec::new();
            for event in click(at) {
                commands.extend(render_panel(&mut editor, &mut view, &ctx, 320.0, vec![event]).1);
            }
            assert!(commands.contains(&expected), "{expected:?} in {commands:?}");
        }
    }

    #[test]
    fn extras_of_another_repository_are_never_shown_and_long_subjects_stay_inside() {
        let ctx = fixture();
        let mut editor = EditorState::with_workspace(PathBuf::from("/workspace"));
        let mut view = ViewState::default();
        view.source_control.snapshot = Some(repo("/workspace", "main", vec![]));
        let long = "a subject far too long to fit in a narrow sidebar without being truncated";
        let mut loaded = extras(&[], &[long], &[]);
        view.source_control.expanded_extras.insert((
            PathBuf::from("/workspace"),
            crate::app::view_state::ExtraSection::Outgoing,
        ));
        view.source_control.extras.loaded = Some(loaded);
        let (output, _) = render_panel(&mut editor, &mut view, &ctx, 200.0, vec![]);
        let drawn = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("a subject far") => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                }
                _ => None,
            })
            .expect("the subject is drawn truncated");
        assert!(drawn.right() <= 200.0, "{drawn:?}");

        loaded = crate::app::repo_extras::RepoExtras::for_test(
            "/elsewhere",
            Vec::new(),
            crate::services::source_control::SyncCommits::default(),
        );
        view.source_control.extras.loaded = Some(loaded);
        let (output, _) = render_panel(&mut editor, &mut view, &ctx, 200.0, vec![]);
        assert!(text_x(&output, "OUTGOING").is_none());
    }

    #[test]
    fn clicking_the_selected_branch_opens_the_switcher_instead_of_collapsing_the_section() {
        let ctx = fixture();
        let (selected, all) = several_repositories();
        let mut editor = EditorState::with_workspace(PathBuf::from("/workspace"));
        let mut view = ViewState::default();
        view.source_control.snapshot = Some(selected);
        view.source_control.repositories = all;
        let (output, _) = render_panel(&mut editor, &mut view, &ctx, 420.0, vec![]);
        let at = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("feature/a-branch") => {
                    Some(text.pos + egui::vec2(10.0, 6.0))
                }
                _ => None,
            })
            .expect("the selected repository's branch is drawn");
        render_panel(
            &mut editor,
            &mut view,
            &ctx,
            420.0,
            vec![egui::Event::PointerMoved(at)],
        );
        let mut commands = Vec::new();
        for event in click(at) {
            commands.extend(render_panel(&mut editor, &mut view, &ctx, 420.0, vec![event]).1);
        }
        assert!(
            commands.contains(&Command::OpenBranchPicker),
            "{commands:?}"
        );
        assert!(
            view.source_control.collapsed_repos.is_empty(),
            "the row must not toggle as well"
        );
    }

    #[test]
    fn the_quick_switcher_closes_when_its_anchor_is_gone_and_ignores_keys_without_focus() {
        let ctx = fixture();
        let mut app = picker_app("");
        run_picker(&mut app, &ctx, vec![]);
        let (_, commands) = run_picker_with_chip(&mut app, &ctx, vec![], false);
        assert!(
            commands.contains(&Command::CloseBranchDialog),
            "no branch name on screen means nothing to hang from: {commands:?}"
        );

        // With the filter box not focused the keyboard belongs to someone else.
        let mut app = picker_app("");
        app.view.branch_dialog.as_mut().unwrap().focus_filter = false;
        let key = |key| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        run_picker(&mut app, &ctx, vec![]);
        let (_, commands) = run_picker(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown), key(egui::Key::Enter)],
        );
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, Command::SwitchBranch(..))),
            "{commands:?}"
        );
    }
}
