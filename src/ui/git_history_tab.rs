//! The History tab: a filterable, paginated commit list with a details pane,
//! shown in the editor area beside file and diff tabs. State and Git reads live
//! in `app::git_history`; this module only draws and emits typed commands.
use super::{
    Workbench,
    source_control::{commit_date, history_blame, history_comparison},
    theme::{self, change_color},
    widgets::{
        MenuEntry, MenuItem, disclosure_row, fit_galley, menu_entries, named, overflow_menu,
    },
};
use crate::{
    app::{command::Command, git_history::HistoryDialog},
    icon,
    services::source_control::{GitCommit, GitCommitFile, GitLogQuery, HistoryAction, ResetMode},
};
use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, RichText, Stroke, Vec2};

const ROW_HEIGHT: f32 = 24.0;
/// Below this width the details pane moves under the list.
const WIDE_LAYOUT: f32 = 640.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RefChip {
    pub(super) label: String,
    pub(super) kind: RefKind,
}

/// Turns Git's decorations (`HEAD -> main`, `origin/main`, `tag: v1`) into
/// labeled chips. A name is remote only when its first segment is a remote
/// that exists, so a local `feature/x` is not mistaken for one.
pub(super) fn ref_chips(refs: &[String], remotes: &[String]) -> Vec<RefChip> {
    let mut chips: Vec<RefChip> = refs
        .iter()
        .map(|name| {
            if let Some(branch) = name.strip_prefix("HEAD -> ") {
                RefChip {
                    label: branch.to_owned(),
                    kind: RefKind::Head,
                }
            } else if name == "HEAD" {
                RefChip {
                    label: "HEAD".into(),
                    kind: RefKind::Head,
                }
            } else if let Some(tag) = name.strip_prefix("tag: ") {
                RefChip {
                    label: tag.to_owned(),
                    kind: RefKind::Tag,
                }
            } else {
                let remote = name
                    .split_once('/')
                    .is_some_and(|(first, _)| remotes.iter().any(|r| r == first));
                RefChip {
                    label: name.clone(),
                    kind: if remote {
                        RefKind::Remote
                    } else {
                        RefKind::Local
                    },
                }
            }
        })
        .collect();
    chips.sort_by_key(|chip| chip.kind);
    chips
}

impl PartialOrd for RefKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RefKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |kind: &Self| match kind {
            Self::Head => 0,
            Self::Local => 1,
            Self::Remote => 2,
            Self::Tag => 3,
        };
        rank(self).cmp(&rank(other))
    }
}

/// "5 min ago", "2 d ago"; the absolute date stays in the tooltip.
pub(super) fn relative_time(now: i64, then: i64) -> String {
    let seconds = (now - then).max(0);
    let (value, unit) = match seconds {
        0..=59 => return "just now".into(),
        60..=3_599 => (seconds / 60, "min"),
        3_600..=86_399 => (seconds / 3_600, "h"),
        86_400..=604_799 => (seconds / 86_400, "d"),
        604_800..=2_591_999 => (seconds / 604_800, "w"),
        2_592_000..=31_535_999 => (seconds / 2_592_000, "mo"),
        _ => (seconds / 31_536_000, "y"),
    };
    format!("{value} {unit} ago")
}

/// Which trailing columns fit; the subject always keeps the remainder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Columns {
    pub(super) author: bool,
    pub(super) date: bool,
    pub(super) hash: bool,
}

pub(super) fn columns(width: f32) -> Columns {
    Columns {
        hash: width >= 300.0,
        date: width >= 400.0,
        author: width >= 560.0,
    }
}

/// Actions for one commit, shared by the details menu and the row context menu.
pub(super) fn commit_menu(commit: &GitCommit, parent: usize) -> Vec<MenuEntry> {
    let short = &commit.id[..commit.id.len().min(7)];
    let parent = (commit.parents.len() > 1).then_some(parent);
    vec![
        MenuEntry::Caption(format!("{short}  ·  {}", commit.subject)),
        MenuItem::new(
            "Cherry-pick onto Current Branch…",
            Command::PlanHistoryAction(
                HistoryAction::CherryPick {
                    commit: commit.id.clone(),
                    parent,
                },
                format!("Cherry-pick {short}"),
            ),
        )
        .entry(),
        MenuItem::new(
            "Revert Commit…",
            Command::PlanHistoryAction(
                HistoryAction::Revert {
                    commit: commit.id.clone(),
                    parent,
                },
                format!("Revert {short}"),
            ),
        )
        .entry(),
        MenuItem::new(
            "Create Tag…",
            Command::OpenTagsDialog(Some(commit.id.clone())),
        )
        .entry(),
        MenuEntry::Separator,
        MenuItem::new(
            "Reset Current Branch to Here…",
            Command::PlanHistoryAction(
                HistoryAction::Reset {
                    target: commit.id.clone(),
                    mode: ResetMode::Mixed,
                    backup: true,
                },
                format!("Reset to {short}"),
            ),
        )
        .danger()
        .entry(),
    ]
}

struct ChipStyle {
    fill: Option<Color32>,
    stroke: Color32,
    text: Color32,
}

fn chip_style(colors: &theme::Palette, kind: RefKind) -> ChipStyle {
    match kind {
        RefKind::Head => ChipStyle {
            fill: Some(colors.accent),
            stroke: colors.accent,
            text: Color32::WHITE,
        },
        RefKind::Local => ChipStyle {
            fill: None,
            stroke: colors.accent,
            text: colors.info,
        },
        RefKind::Remote => ChipStyle {
            fill: None,
            stroke: colors.muted.gamma_multiply(0.5),
            text: colors.muted,
        },
        RefKind::Tag => ChipStyle {
            fill: None,
            stroke: colors.warning.gamma_multiply(0.6),
            text: colors.warning,
        },
    }
}

impl Workbench<'_> {
    pub(crate) fn history_page(&mut self, ui: &mut egui::Ui) {
        let font = FontId::monospace(self.view.settings.number("editor.font_size", None));
        let line_height = self.view.settings.number("editor.line_height", None);
        // Keys belong to the list only while nothing modal is open over it.
        let keys_enabled = !crate::app::editor_action::blocked(self.view)
            && !self.view.git_modal_open()
            && self.view.task_editors.active().is_none();
        let Some(root) = self
            .view
            .history_dialog
            .as_ref()
            .map(|dialog| dialog.root.clone())
        else {
            return;
        };
        let remotes = shown_remotes(self.view, &root);
        let Some(dialog) = &mut self.view.history_dialog else {
            return;
        };
        let now = chrono::Utc::now().timestamp();
        let mut actions: Vec<Command> = Vec::new();
        history_view(
            ui,
            dialog,
            &HistoryContext {
                remotes: &remotes,
                font: &font,
                line_height,
                now,
                keys_enabled,
            },
            &mut actions,
        );
        for action in actions {
            self.command(action);
        }
    }
}

/// Remote names of the repository the tab shows; only these make a ref remote.
fn shown_remotes(view: &crate::app::view_state::ViewState, root: &std::path::Path) -> Vec<String> {
    view.source_control
        .repositories
        .iter()
        .chain(view.source_control.snapshot.iter())
        .filter(|repo| repo.root == root)
        .flat_map(|repo| repo.remotes.iter().map(|remote| remote.name.clone()))
        .collect()
}

struct HistoryContext<'a> {
    remotes: &'a [String],
    font: &'a FontId,
    line_height: f32,
    now: i64,
    keys_enabled: bool,
}

fn history_view(
    ui: &mut egui::Ui,
    dialog: &mut HistoryDialog,
    context: &HistoryContext,
    actions: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    if dialog.compare.is_some() || dialog.blame.is_some() {
        egui::Frame::NONE
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                if let Some(compare) = &mut dialog.compare {
                    history_comparison(ui, compare, context.font, context.line_height, actions);
                } else if let Some(blame) = &dialog.blame {
                    history_blame(ui, blame, actions);
                }
                if let Some(error) = &dialog.error {
                    ui.label(RichText::new(error).color(colors.error));
                }
            });
        return;
    }
    toolbar(ui, dialog, actions);
    if let Some(error) = &dialog.error {
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(10, 2))
            .show(ui, |ui| {
                ui.label(RichText::new(error).size(12.0).color(colors.error));
            });
    }
    let dialog: &HistoryDialog = dialog;
    let selected = dialog.current_commit();
    let frame = egui::Frame::NONE
        .fill(colors.panel)
        .inner_margin(Margin::symmetric(10, 8));
    if ui.available_width() >= WIDE_LAYOUT {
        egui::SidePanel::right("git_history_details")
            .resizable(true)
            .default_width(300.0)
            .width_range(220.0..=520.0)
            .show_separator_line(false)
            .frame(frame)
            .show_inside(ui, |ui| {
                details(ui, dialog, selected, context, actions);
            });
    } else if selected.is_some() {
        let limit = (ui.available_height() * 0.7).max(120.0);
        egui::TopBottomPanel::bottom("git_history_details_below")
            .resizable(true)
            .default_height(200.0_f32.min(limit))
            .height_range(100.0..=limit)
            .show_separator_line(false)
            .frame(frame)
            .show_inside(ui, |ui| {
                details(ui, dialog, selected, context, actions);
            });
    }
    commit_list(ui, dialog, context, actions);
}

fn toolbar(ui: &mut egui::Ui, dialog: &mut HistoryDialog, actions: &mut Vec<Command>) {
    let colors = theme::palette(ui.visuals());
    let filtering = dialog.active != GitLogQuery::default();
    let author_or_path = !dialog.filter_author.trim().is_empty()
        || !dialog.filter_path.trim().is_empty()
        || dialog.active.path.is_some()
        || !dialog.active.author.is_empty();
    let mut apply = false;
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let spacing = ui.spacing().item_spacing.x;
                let combo_width = (ui.available_width() * 0.3).clamp(90.0, 180.0);
                let current = dialog.filter_ref.clone().unwrap_or_else(|| "HEAD".into());
                egui::ComboBox::from_id_salt("git_history_ref")
                    .selected_text(format!("{} {current}", icon::GIT_BRANCH))
                    .width(combo_width)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(dialog.filter_ref.is_none(), "HEAD")
                            .clicked()
                        {
                            actions.push(Command::SetHistoryRef(None));
                        }
                        for name in &dialog.refs {
                            let chosen = dialog.filter_ref.as_deref() == Some(name.as_str());
                            if ui.selectable_label(chosen, name).clicked() {
                                actions.push(Command::SetHistoryRef(Some(name.clone())));
                            }
                        }
                    })
                    .response
                    .on_hover_text("Show the history of a branch or tag");
                let buttons = 22.0 * if filtering { 2.0 } else { 1.0 } + spacing * 2.0 + 4.0;
                let width = (ui.available_width() - buttons).max(40.0);
                let search = ui.add_sized(
                    [width, 22.0],
                    egui::TextEdit::singleline(&mut dialog.filter_text)
                        .id_salt("git_history_text")
                        .hint_text("Search messages, or paste a commit id"),
                );
                apply |= search.lost_focus()
                    && search
                        .ctx
                        .input(|input| input.key_pressed(egui::Key::Enter));
                if named(
                    ui.add_sized(
                        [22.0, 22.0],
                        egui::Button::new(icon::MAGNIFYING_GLASS).frame(false),
                    )
                    .on_hover_text("Search"),
                    true,
                    "Search history",
                )
                .clicked()
                {
                    apply = true;
                }
                if filtering
                    && named(
                        ui.add_sized([22.0, 22.0], egui::Button::new(icon::X).frame(false))
                            .on_hover_text("Clear every filter"),
                        true,
                        "Clear history filters",
                    )
                    .clicked()
                {
                    actions.push(Command::ClearHistoryFilter);
                }
            });
        });
    let key = egui::Id::new("git_history_filters_open");
    let open = ui
        .data(|data| data.get_temp::<bool>(key))
        .unwrap_or(author_or_path);
    let active_filters = usize::from(!dialog.filter_author.trim().is_empty())
        + usize::from(!dialog.filter_path.trim().is_empty());
    let outcome = disclosure_row(ui, !open, "FILTERS", active_filters, &[]);
    if outcome.toggled {
        ui.data_mut(|data| data.insert_temp(key, !open));
    }
    if open {
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let half = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0 - 2.0)
                        .max(40.0);
                    let author = ui.add_sized(
                        [half, 22.0],
                        egui::TextEdit::singleline(&mut dialog.filter_author)
                            .id_salt("git_history_author")
                            .hint_text("Author"),
                    );
                    let path = ui.add_sized(
                        [half, 22.0],
                        egui::TextEdit::singleline(&mut dialog.filter_path)
                            .id_salt("git_history_path")
                            .hint_text("File path (follows renames)"),
                    );
                    for response in [author, path] {
                        apply |= response.lost_focus()
                            && response
                                .ctx
                                .input(|input| input.key_pressed(egui::Key::Enter));
                    }
                });
            });
    }
    if apply {
        actions.push(Command::ApplyHistoryFilter);
    }
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, colors.border);
}

fn commit_list(
    ui: &mut egui::Ui,
    dialog: &HistoryDialog,
    context: &HistoryContext,
    actions: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    let commits = &dialog.commits;
    if commits.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let text = if dialog.loading {
                "Reading history…"
            } else if dialog.active == GitLogQuery::default() {
                "No commits yet"
            } else {
                "No commits match these filters"
            };
            ui.label(RichText::new(text).color(colors.muted));
        });
        return;
    }
    let cols = columns(ui.available_width());
    let list_rect = ui.available_rect_before_wrap();
    let selected_index = dialog
        .selected
        .as_ref()
        .and_then(|id| commits.iter().position(|commit| &commit.id == id));
    // Arrow keys move the selection while the pointer is over the list, so
    // typing in a filter box is never intercepted.
    let scroll_key = egui::Id::new("git_history_scroll_to");
    let mut scroll_to: Option<f32> = ui.data_mut(|data| data.remove_temp::<f32>(scroll_key));
    if context.keys_enabled
        && ui.rect_contains_pointer(list_rect)
        && !ui.ctx().wants_keyboard_input()
    {
        let step = ui.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                1
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                -1
            } else {
                0
            }
        });
        if step != 0 {
            let next = selected_index.map_or(0, |index| {
                (index as isize + step).clamp(0, commits.len() as isize - 1) as usize
            });
            actions.push(Command::SelectCommit(commits[next].id.clone()));
            ui.data_mut(|data| data.insert_temp(egui::Id::new("git_history_keep_visible"), next));
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            && dialog.details.as_ref().is_some_and(|d| !d.files.is_empty())
        {
            actions.push(Command::OpenCommitFile(0));
        }
    }
    let rows = commits.len() + usize::from(dialog.has_more || dialog.loading);
    let mut area = egui::ScrollArea::vertical()
        .id_salt("git_history_list")
        .auto_shrink([false, false]);
    if let Some(offset) = scroll_to.take() {
        area = area.vertical_scroll_offset(offset);
    }
    let output = area.show_rows(ui, ROW_HEIGHT, rows, |ui, range| {
        for index in range.clone() {
            let Some(commit) = commits.get(index) else {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add_space(26.0);
                        if dialog.page_failed {
                            ui.label(
                                RichText::new("Could not load more commits")
                                    .size(12.0)
                                    .color(colors.error),
                            );
                            if ui.small_button("Retry").clicked() {
                                actions.push(Command::LoadMoreHistory);
                            }
                        } else {
                            ui.label(
                                RichText::new("Loading more…")
                                    .size(12.0)
                                    .color(colors.muted),
                            );
                        }
                    },
                );
                continue;
            };
            let shown = commit_row(ui, commit, selected_index == Some(index), cols, context);
            if shown.clicked() {
                actions.push(Command::SelectCommit(commit.id.clone()));
            }
            shown.context_menu(|ui| {
                let parent = if dialog.selected.as_ref() == Some(&commit.id) {
                    dialog.parent
                } else {
                    0
                };
                if let Some(command) = menu_entries(ui, &commit_menu(commit, parent)) {
                    actions.push(command);
                }
            });
        }
        // Fetch the next page before the user reaches the end.
        // A failed page waits for Retry; it must not be re-requested every frame.
        if dialog.has_more
            && !dialog.loading
            && !dialog.page_failed
            && range.end + 10 >= commits.len()
        {
            actions.push(Command::LoadMoreHistory);
        }
    });
    // After a keyboard move, bring the row into view on the next frame.
    let keep: Option<usize> =
        ui.data_mut(|data| data.remove_temp(egui::Id::new("git_history_keep_visible")));
    if let Some(index) = keep {
        let top = index as f32 * ROW_HEIGHT;
        let offset = output.state.offset.y;
        let view = output.inner_rect.height();
        let target = if top < offset {
            Some(top)
        } else if top + ROW_HEIGHT > offset + view {
            Some(top + ROW_HEIGHT - view)
        } else {
            None
        };
        if let Some(target) = target {
            ui.data_mut(|data| data.insert_temp(scroll_key, target.max(0.0)));
            ui.ctx().request_repaint();
        }
    }
}

fn commit_row(
    ui: &mut egui::Ui,
    commit: &GitCommit,
    selected: bool,
    cols: Columns,
    context: &HistoryContext,
) -> egui::Response {
    let colors = theme::palette(ui.visuals());
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    let hovered = ui.rect_contains_pointer(rect);
    if selected || hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            if selected {
                colors.active
            } else {
                colors.hover
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            CornerRadius::same(2),
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            selected,
            &commit.subject,
        )
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let center = rect.center().y;

    // Right-hand columns first, so the subject takes only what remains.
    let mut right = rect.right() - 10.0;
    let mut column = |text: String, width: f32, font: FontId, align_right: bool| {
        let galley = fit_galley(ui, &text, font, colors.muted, width);
        let x = if align_right {
            right - galley.size().x
        } else {
            right - width
        };
        painter.galley(
            egui::pos2(x, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
        right -= width + 12.0;
    };
    if cols.hash {
        column(
            commit.id[..commit.id.len().min(7)].to_owned(),
            54.0,
            FontId::monospace(11.0),
            false,
        );
    }
    if cols.date {
        column(
            relative_time(context.now, commit.time),
            62.0,
            FontId::proportional(12.0),
            true,
        );
    }
    if cols.author {
        column(
            commit.author.clone(),
            110.0,
            FontId::proportional(12.0),
            false,
        );
    }

    // Graph column: a plain dot, hollow for merges.
    let dot = egui::pos2(rect.left() + 14.0, center);
    if commit.parents.len() > 1 {
        painter.circle_stroke(dot, 3.5, Stroke::new(1.4_f32, colors.text));
    } else {
        painter.circle_filled(dot, 3.5, colors.text);
    }
    let mut x = rect.left() + 28.0;
    let chips = ref_chips(&commit.refs, context.remotes);
    let chip_budget = ((right - x) * 0.45).max(0.0);
    let mut used = 0.0;
    let mut hidden = 0;
    for chip in &chips {
        let style = chip_style(&colors, chip.kind);
        let galley = fit_galley(
            ui,
            &chip.label,
            FontId::proportional(10.5),
            style.text,
            120.0,
        );
        let width = galley.size().x + 10.0;
        if used + width > chip_budget {
            hidden += 1;
            continue;
        }
        let chip_rect =
            egui::Rect::from_min_size(egui::pos2(x, center - 8.0), Vec2::new(width, 16.0));
        if let Some(fill) = style.fill {
            painter.rect_filled(chip_rect, CornerRadius::same(3), fill);
        }
        painter.rect_stroke(
            chip_rect,
            CornerRadius::same(3),
            Stroke::new(1.0_f32, style.stroke),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(chip_rect.left() + 5.0, center - galley.size().y / 2.0),
            galley,
            style.text,
        );
        x += width + 4.0;
        used += width + 4.0;
    }
    if hidden > 0 {
        let galley = painter.layout_no_wrap(
            format!("+{hidden}"),
            FontId::proportional(10.5),
            colors.muted,
        );
        let width = galley.size().x;
        painter.galley(
            egui::pos2(x, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
        x += width + 4.0;
    }
    let subject = fit_galley(
        ui,
        &commit.subject,
        FontId::proportional(13.0),
        colors.text,
        (right - x - 4.0).max(0.0),
    );
    painter.galley(
        egui::pos2(x, center - subject.size().y / 2.0),
        subject,
        colors.text,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{}\n{}  ·  {}",
            commit.id,
            commit.author,
            commit_date(commit.time)
        ))
}

fn details(
    ui: &mut egui::Ui,
    dialog: &HistoryDialog,
    selected: Option<&GitCommit>,
    context: &HistoryContext,
    actions: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    let Some(commit) = selected else {
        ui.label(RichText::new("Select a commit to see its details").color(colors.muted));
        return;
    };
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        &commit.id[..commit.id.len().min(10)],
        FontId::monospace(12.0),
        colors.muted,
    );
    let menu_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 11.0, rect.center().y),
        Vec2::splat(22.0),
    );
    let copy_rect = menu_rect.translate(egui::vec2(-24.0, 0.0));
    if named(
        ui.put(copy_rect, egui::Button::new(icon::COPY).frame(false))
            .on_hover_text("Copy the full commit hash"),
        true,
        "Copy commit hash",
    )
    .clicked()
    {
        ui.ctx().copy_text(commit.id.clone());
    }
    if let Some(command) = overflow_menu(
        ui,
        menu_rect,
        "Commit actions",
        &commit_menu(commit, dialog.parent),
    ) {
        actions.push(command);
    }
    egui::ScrollArea::vertical()
        .id_salt("git_history_details")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(&commit.subject).strong().size(14.0)).wrap());
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{}  ·  {}",
                    commit.author,
                    relative_time(context.now, commit.time)
                ))
                .size(12.0)
                .color(colors.muted),
            )
            .on_hover_text(commit_date(commit.time));
            let chips = ref_chips(&commit.refs, context.remotes);
            if !chips.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                    for chip in &chips {
                        let style = chip_style(&colors, chip.kind);
                        let mut frame = egui::Frame::NONE
                            .stroke(Stroke::new(1.0_f32, style.stroke))
                            .corner_radius(CornerRadius::same(3))
                            .inner_margin(Margin::symmetric(5, 1));
                        if let Some(fill) = style.fill {
                            frame = frame.fill(fill);
                        }
                        frame.show(ui, |ui| {
                            ui.label(RichText::new(&chip.label).size(10.5).color(style.text));
                        });
                    }
                });
            }
            ui.add_space(6.0);
            parents(ui, dialog, commit, actions);
            match &dialog.details {
                Some(info) => {
                    if info.committer != commit.author || info.committed != commit.time {
                        ui.label(
                            RichText::new(format!(
                                "Committed by {} on {}",
                                info.committer,
                                commit_date(info.committed)
                            ))
                            .size(12.0)
                            .color(colors.muted),
                        );
                    }
                    let body = info
                        .message
                        .split_once('\n')
                        .map_or("", |(_, rest)| rest.trim());
                    if !body.is_empty() {
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(RichText::new(body).size(12.5)).wrap());
                    }
                    ui.add_space(8.0);
                    let key = egui::Id::new("git_history_files_collapsed");
                    let collapsed = ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false);
                    let outcome =
                        disclosure_row(ui, collapsed, "FILES CHANGED", info.files.len(), &[]);
                    if outcome.toggled {
                        ui.data_mut(|data| data.insert_temp(key, !collapsed));
                    }
                    if !collapsed {
                        for (index, file) in info.files.iter().enumerate() {
                            if commit_file_row(ui, file).clicked() {
                                actions.push(Command::OpenCommitFile(index));
                            }
                        }
                    }
                }
                None => {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Reading commit…").color(colors.muted));
                }
            }
        });
}

/// Parents as short ids; a merge offers which parent the files compare against.
fn parents(
    ui: &mut egui::Ui,
    dialog: &HistoryDialog,
    commit: &GitCommit,
    actions: &mut Vec<Command>,
) {
    let colors = theme::palette(ui.visuals());
    let short = |id: &str| id[..id.len().min(7)].to_owned();
    if commit.parents.len() > 1 {
        ui.label(
            RichText::new("Compare files with")
                .size(11.5)
                .color(colors.muted),
        );
        ui.horizontal_wrapped(|ui| {
            for (index, parent) in commit.parents.iter().enumerate() {
                if ui
                    .selectable_label(
                        dialog.parent == index,
                        format!("parent {} ({})", index + 1, short(parent)),
                    )
                    .clicked()
                {
                    actions.push(Command::ChooseHistoryParent(index));
                }
            }
        });
    } else if let Some(parent) = commit.parents.first() {
        let parent_known = dialog.commits.iter().any(|c| &c.id == parent);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Parent").size(11.5).color(colors.muted));
            let link = ui.add(
                egui::Label::new(RichText::new(short(parent)).monospace().size(12.0).color(
                    if parent_known {
                        colors.info
                    } else {
                        colors.muted
                    },
                ))
                .sense(egui::Sense::click()),
            );
            if parent_known
                && link
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Select the parent commit")
                    .clicked()
            {
                actions.push(Command::SelectCommit(parent.clone()));
            }
        });
    }
}

fn commit_file_row(ui: &mut egui::Ui, file: &GitCommitFile) -> egui::Response {
    let colors = theme::palette(ui.visuals());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.hover);
    }
    let path = std::path::Path::new(&file.path);
    let name = path
        .file_name()
        .map_or_else(|| file.path.clone(), |n| n.to_string_lossy().into_owned());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let center = rect.center().y;
    let letter = file.status.chars().next().unwrap_or('M');
    let limit = rect.right() - 24.0;
    let icon = super::file_icons::for_file(path);
    painter.text(
        egui::pos2(rect.left() + 2.0, center),
        egui::Align2::LEFT_CENTER,
        icon.glyph,
        FontId::new(14.0, super::file_icons::family()),
        icon.color,
    );
    let mut x = rect.left() + 22.0;
    let name_galley = fit_galley(
        ui,
        &name,
        FontId::proportional(13.0),
        colors.text,
        limit - x,
    );
    let name_width = name_galley.size().x;
    painter.galley(
        egui::pos2(x, center - name_galley.size().y / 2.0),
        name_galley,
        colors.text,
    );
    x += name_width + 6.0;
    let mut detail = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.display().to_string())
        .unwrap_or_default();
    if let Some(old) = &file.old_path {
        detail = format!("{detail}  ← {old}");
    }
    if !detail.is_empty() && limit - x > 24.0 {
        let galley = fit_galley(
            ui,
            detail.trim(),
            FontId::proportional(11.5),
            colors.muted,
            limit - x,
        );
        painter.galley(
            egui::pos2(x, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
    }
    painter.text(
        egui::pos2(rect.right() - 10.0, center),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::monospace(11.0),
        change_color(&colors, letter),
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "{}\nCompare with its parent (read-only)",
            file.label()
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(chips: &[RefChip]) -> Vec<(&str, RefKind)> {
        chips
            .iter()
            .map(|chip| (chip.label.as_str(), chip.kind))
            .collect()
    }

    #[test]
    fn decorations_become_ordered_chips_and_only_real_remotes_are_remote() {
        let refs: Vec<String> = ["tag: v1", "origin/main", "feature/x", "HEAD -> main"]
            .map(String::from)
            .to_vec();
        let chips = ref_chips(&refs, &["origin".to_owned()]);
        assert_eq!(
            names(&chips),
            [
                ("main", RefKind::Head),
                ("feature/x", RefKind::Local),
                ("origin/main", RefKind::Remote),
                ("v1", RefKind::Tag),
            ]
        );
        assert_eq!(
            names(&ref_chips(&["HEAD".to_owned()], &[])),
            [("HEAD", RefKind::Head)]
        );
    }

    #[test]
    fn relative_time_uses_the_largest_whole_unit() {
        assert_eq!(relative_time(1_000, 990), "just now");
        assert_eq!(
            relative_time(1_000, 1_000 + 50),
            "just now",
            "future clocks clamp"
        );
        assert_eq!(relative_time(10_000, 10_000 - 300), "5 min ago");
        assert_eq!(relative_time(100_000, 100_000 - 7_300), "2 h ago");
        assert_eq!(relative_time(1_000_000, 1_000_000 - 3 * 86_400), "3 d ago");
        assert_eq!(
            relative_time(10_000_000, 10_000_000 - 2 * 604_800),
            "2 w ago"
        );
        assert_eq!(
            relative_time(100_000_000, 100_000_000 - 4 * 2_592_000),
            "4 mo ago"
        );
        assert_eq!(relative_time(100_000_000, 0), "3 y ago");
    }

    #[test]
    fn narrow_lists_drop_author_then_date_then_hash() {
        assert_eq!(
            columns(700.0),
            Columns {
                author: true,
                date: true,
                hash: true
            }
        );
        assert_eq!(
            columns(450.0),
            Columns {
                author: false,
                date: true,
                hash: true
            }
        );
        assert_eq!(
            columns(320.0),
            Columns {
                author: false,
                date: false,
                hash: true
            }
        );
        assert!(!columns(200.0).hash);
    }

    #[test]
    fn commit_menu_carries_the_chosen_parent_only_for_merges() {
        let plain = GitCommit {
            id: "abcdef1234567".into(),
            parents: vec!["p1".into()],
            subject: "fix".into(),
            ..Default::default()
        };
        let merge = GitCommit {
            parents: vec!["p1".into(), "p2".into()],
            ..plain.clone()
        };
        let parent_of = |entries: &[MenuEntry]| {
            entries.iter().find_map(|entry| match entry {
                MenuEntry::Item(MenuItem {
                    command:
                        Command::PlanHistoryAction(HistoryAction::CherryPick { parent, .. }, _),
                    ..
                }) => Some(*parent),
                _ => None,
            })
        };
        assert_eq!(parent_of(&commit_menu(&plain, 1)), Some(None));
        assert_eq!(parent_of(&commit_menu(&merge, 1)), Some(Some(1)));
        let entries = commit_menu(&plain, 0);
        assert!(
            matches!(entries.last(), Some(MenuEntry::Item(item)) if item.danger),
            "the reset entry is last and dangerous"
        );
    }

    use crate::{
        app::LumenApp,
        services::source_control::{GitCommitDetails, GitSnapshot},
    };
    use std::path::PathBuf;

    fn commit(n: usize, subject: &str, refs: &[&str], parents: usize) -> GitCommit {
        GitCommit {
            id: format!("{n:07x}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            parents: (0..parents).map(|p| format!("{p:07x}bbbb")).collect(),
            author: "A Very Long Author Name".into(),
            time: 1_700_000_000 - n as i64 * 3_600,
            refs: refs.iter().map(|r| r.to_string()).collect(),
            subject: subject.into(),
        }
    }

    fn app_with_history() -> LumenApp {
        let mut app = LumenApp::default();
        app.editor.workspace = Some(PathBuf::from("/w"));
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from("/w/repo"),
            ..Default::default()
        });
        app.open_history_dialog(&egui::Context::default());
        let dialog = app.view.history_dialog.as_mut().unwrap();
        dialog.commits = (0..400)
            .map(|n| {
                commit(
                    n,
                    if n == 0 {
                        "the selected commit"
                    } else {
                        "a subject long enough that it must be truncated instead of widening the page or covering the columns"
                    },
                    if n == 0 {
                        &["HEAD -> main", "origin/main", "tag: v1", "tag: v2", "feature/long-branch-name"]
                    } else {
                        &[]
                    },
                    1 + usize::from(n == 3),
                )
            })
            .collect();
        dialog.has_more = true;
        dialog.selected = Some(dialog.commits[0].id.clone());
        dialog.details = Some(GitCommitDetails {
            message: "subject\n\nA body line.".into(),
            committer: "Someone Else".into(),
            committed: 1_700_000_100,
            files: vec![GitCommitFile {
                status: "M".into(),
                path: "src/some/deeply/nested/directory/with/a/long_file_name.rs".into(),
                old_path: None,
            }],
        });
        app
    }

    fn run_page(
        app: &mut LumenApp,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Vec<Command>) {
        let mut taken = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut workbench = Workbench::new(
                        &mut app.editor,
                        &mut app.view,
                        &crate::services::LocalFileSystem,
                    );
                    workbench.history_page(ui);
                    taken = workbench.take_commands();
                });
            },
        );
        (output, taken)
    }

    fn fixture_context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        crate::platform::fonts::configure(&ctx);
        ctx
    }

    fn text_bounds(output: &egui::FullOutput, needle: &str) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
    }

    #[test]
    fn the_page_keeps_every_column_inside_the_tab_at_every_width() {
        let mut app = app_with_history();
        let ctx = fixture_context();
        for width in [320.0, 560.0, 900.0, 1400.0] {
            // Two frames so panel sizes settle.
            run_page(&mut app, &ctx, width, vec![]);
            let (output, _) = run_page(&mut app, &ctx, width, vec![]);
            // Hint text is clipped by its field, so only drawn rows are measured.
            for needle in ["a subject long enough", "0000001"] {
                if let Some(rect) = text_bounds(&output, needle) {
                    assert!(
                        rect.right() <= width + 1.0,
                        "{width}: {needle:?} reaches {}",
                        rect.right()
                    );
                }
            }
            let subject = text_bounds(&output, "a subject long enough").expect("rows are drawn");
            assert!(
                subject.left() < width / 2.0,
                "{width}: subject at {}",
                subject.left()
            );
        }
    }

    #[test]
    fn wide_pages_show_the_details_beside_the_list_and_narrow_ones_below() {
        let mut app = app_with_history();
        let ctx = fixture_context();
        // Panels settle over a couple of frames.
        run_page(&mut app, &ctx, 1000.0, vec![]);
        run_page(&mut app, &ctx, 1000.0, vec![]);
        let (wide, _) = run_page(&mut app, &ctx, 1000.0, vec![]);
        let list = text_bounds(&wide, "a subject long enough").unwrap();
        let file = text_bounds(&wide, "long_file_name.rs").expect("details list files");
        assert!(
            file.left() > list.left() + 300.0,
            "beside: {file:?} vs {list:?}"
        );

        let mut app = app_with_history();
        let ctx = fixture_context();
        run_page(&mut app, &ctx, 500.0, vec![]);
        let (narrow, _) = run_page(&mut app, &ctx, 500.0, vec![]);
        let list = text_bounds(&narrow, "a subject long enough").unwrap();
        let file = text_bounds(&narrow, "long_file_name.rs").expect("details list files");
        assert!(file.top() > list.top(), "below: {file:?} vs {list:?}");
    }

    #[test]
    fn clicking_a_commit_row_selects_it_and_a_merge_row_is_marked() {
        let mut app = app_with_history();
        let ctx = fixture_context();
        run_page(&mut app, &ctx, 1000.0, vec![]);
        let (output, _) = run_page(&mut app, &ctx, 1000.0, vec![]);
        let subject = text_bounds(&output, "a subject long enough").unwrap();
        // The first row is the selected commit, so this text is the second row.
        let at = egui::pos2(subject.left() + 20.0, subject.center().y);
        run_page(&mut app, &ctx, 1000.0, vec![egui::Event::PointerMoved(at)]);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run_page(&mut app, &ctx, 1000.0, vec![button(true)]);
        let (_, commands) = run_page(&mut app, &ctx, 1000.0, vec![button(false)]);
        let second = app.view.history_dialog.as_ref().unwrap().commits[1]
            .id
            .clone();
        assert!(
            commands.contains(&Command::SelectCommit(second)),
            "{commands:?}"
        );
    }

    #[test]
    fn the_history_tab_is_listed_in_the_tab_strip_with_its_repository_name() {
        let mut app = app_with_history();
        app.view.history_active = true;
        let ctx = fixture_context();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut workbench = Workbench::new(
                        &mut app.editor,
                        &mut app.view,
                        &crate::services::LocalFileSystem,
                    );
                    workbench.editor_tabs(ui);
                });
            },
        );
        assert!(
            text_bounds(&output, "History · repo").is_some(),
            "the History tab is drawn"
        );
    }

    #[test]
    fn only_the_shown_repositorys_remotes_make_a_ref_remote() {
        use crate::services::source_control::GitRemote;
        let remote = |name: &str| GitRemote {
            name: name.into(),
            fetch_url: "https://example.com/r.git".into(),
            push_url: None,
        };
        let mut view = crate::app::view_state::ViewState::default();
        view.source_control.repositories = vec![
            GitSnapshot {
                root: PathBuf::from("/w/a"),
                ..Default::default()
            },
            GitSnapshot {
                root: PathBuf::from("/w/b"),
                remotes: vec![remote("origin")],
                ..Default::default()
            },
        ];
        let remotes = shown_remotes(&view, std::path::Path::new("/w/a"));
        assert!(remotes.is_empty());
        let chips = ref_chips(&["origin/hotfix".to_owned()], &remotes);
        assert_eq!(
            chips[0].kind,
            RefKind::Local,
            "a local branch named like a remote"
        );
        assert_eq!(
            shown_remotes(&view, std::path::Path::new("/w/b")),
            ["origin".to_owned()]
        );
    }

    #[test]
    fn list_keys_are_ignored_while_a_dialog_is_open_over_the_tab() {
        let mut app = app_with_history();
        let ctx = fixture_context();
        run_page(&mut app, &ctx, 1000.0, vec![]);
        let (output, _) = run_page(&mut app, &ctx, 1000.0, vec![]);
        let row = text_bounds(&output, "a subject long enough").unwrap();
        let over_list = egui::Event::PointerMoved(egui::pos2(row.left() + 10.0, row.center().y));
        let down = egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: Some(egui::Key::ArrowDown),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        run_page(&mut app, &ctx, 1000.0, vec![over_list.clone()]);
        let (_, commands) = run_page(
            &mut app,
            &ctx,
            1000.0,
            vec![over_list.clone(), down.clone()],
        );
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::SelectCommit(_))),
            "the list moves its selection when nothing is open: {commands:?}"
        );

        app.view.tags_dialog = Some(Default::default());
        let (_, commands) = run_page(&mut app, &ctx, 1000.0, vec![over_list, down]);
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, Command::SelectCommit(_))),
            "a dialog owns the keyboard: {commands:?}"
        );
    }

    #[test]
    fn a_failed_page_shows_retry_and_is_not_requested_again_every_frame() {
        let mut app = app_with_history();
        let ctx = fixture_context();
        run_page(&mut app, &ctx, 1000.0, vec![]);
        let (_, commands) = run_page(&mut app, &ctx, 1000.0, vec![]);
        // Only 400 rows in view range at the top: nothing is near the end yet.
        assert!(!commands.contains(&Command::LoadMoreHistory));

        // Scrolled to the end, a healthy list asks for the next page...
        let dialog = app.view.history_dialog.as_mut().unwrap();
        dialog.commits.truncate(8);
        dialog.has_more = true;
        dialog.loading = false;
        let (_, commands) = run_page(&mut app, &ctx, 1000.0, vec![]);
        assert!(commands.contains(&Command::LoadMoreHistory), "{commands:?}");

        // ...but after a failure it waits for an explicit retry.
        let dialog = app.view.history_dialog.as_mut().unwrap();
        dialog.page_failed = true;
        for _ in 0..3 {
            let (output, commands) = run_page(&mut app, &ctx, 1000.0, vec![]);
            assert!(
                !commands.contains(&Command::LoadMoreHistory),
                "{commands:?}"
            );
            assert!(text_bounds(&output, "Retry").is_some());
        }
    }
}
