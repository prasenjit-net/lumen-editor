//! Row drawing for the Source Control panel: repository headers and file
//! changes. Rows are painted at fixed geometry so text truncates instead of
//! widening the resizable sidebar, and fixed-edge controls never move.
use super::{
    theme::{self, change_color},
    widgets::fit_galley,
};
use crate::{
    icon,
    services::source_control::{ChangeKind, GitChange, GitSnapshot},
};
use eframe::egui::{self, CornerRadius, FontId, Stroke, Vec2};

const REPO_ROW_HEIGHT: f32 = 24.0;
pub(super) const CHANGE_ROW_HEIGHT: f32 = 22.0;
const BUTTON: f32 = 20.0;

/// How a repository relates to the others found under the opened folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RepoKind {
    Worktree,
    Submodule,
    Nested,
}

impl RepoKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Submodule => "submodule",
            Self::Nested => "nested",
        }
    }
}

/// Kind of `repo` within `all`. A linked worktree is reported first because it
/// is the stronger fact; a submodule is a nested repository its parent lists
/// as a gitlink change.
pub(super) fn repository_kind(all: &[GitSnapshot], repo: &GitSnapshot) -> Option<RepoKind> {
    if repo.is_linked_worktree() {
        return Some(RepoKind::Worktree);
    }
    let outer = all
        .iter()
        .filter(|other| other.root != repo.root && repo.root.starts_with(&other.root));
    let mut nested = false;
    for other in outer {
        nested = true;
        // A submodule keeps its git directory under the parent's `modules`
        // folder, whether or not the parent currently lists a change for it.
        let stored_by_parent = !other.common_dir.as_os_str().is_empty()
            && repo.git_dir.starts_with(other.common_dir.join("modules"));
        let listed = other
            .changes
            .iter()
            .any(|change| change.is_submodule() && other.root.join(&change.path) == repo.root);
        if stored_by_parent || listed {
            return Some(RepoKind::Submodule);
        }
    }
    nested.then_some(RepoKind::Nested)
}

pub(super) struct RepoRow<'a> {
    pub(super) name: &'a str,
    pub(super) kind: Option<RepoKind>,
    pub(super) branch: &'a str,
    pub(super) ahead: usize,
    pub(super) behind: usize,
    pub(super) changes: usize,
    pub(super) conflicts: bool,
    pub(super) active: bool,
    pub(super) expanded: bool,
    /// Several repositories: show the caret and activity dot.
    pub(super) accordion: bool,
    pub(super) draft: bool,
    /// Reserve the right edge for a `⋯` menu button.
    pub(super) menu: bool,
    pub(super) tooltip: String,
}

pub(super) struct RepoRowResponse {
    pub(super) response: egui::Response,
    /// Where the caller puts the `⋯` button; always the row's right edge.
    pub(super) menu_rect: egui::Rect,
    /// The clickable branch name of the selected repository, when drawn.
    pub(super) branch_rect: Option<egui::Rect>,
}

pub(super) fn repo_row(ui: &mut egui::Ui, row: &RepoRow) -> RepoRowResponse {
    let colors = theme::palette(ui.visuals());
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), REPO_ROW_HEIGHT),
        egui::Sense::click(),
    );
    let hovered = ui.rect_contains_pointer(rect);
    if row.active {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.active);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
            CornerRadius::ZERO,
            colors.accent,
        );
    } else if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.hover);
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
            row.active,
            row.name,
        )
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let center = rect.center().y;
    let menu_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 4.0 - BUTTON / 2.0, center),
        Vec2::splat(BUTTON),
    );

    // Right edge first, so the left text can only take what is left over.
    let mut right = if row.menu {
        menu_rect.left() - 2.0
    } else {
        rect.right() - 6.0
    };
    if row.changes > 0 {
        let text = if row.conflicts {
            format!("{} {}", icon::WARNING, row.changes)
        } else {
            row.changes.to_string()
        };
        let galley = painter.layout_no_wrap(
            text,
            FontId::proportional(10.0),
            if row.conflicts {
                colors.warning
            } else {
                colors.muted
            },
        );
        let badge = egui::Rect::from_min_size(
            egui::pos2(right - galley.size().x - 10.0, center - 7.5),
            Vec2::new(galley.size().x + 10.0, 15.0),
        );
        painter.rect_filled(badge, CornerRadius::same(8), colors.input);
        painter.galley(
            egui::pos2(badge.left() + 5.0, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
        right = badge.left() - 5.0;
    }
    for (count, glyph, color) in [
        (row.ahead, icon::REPO_PUSH, colors.success),
        (row.behind, icon::REPO_PULL, colors.warning),
    ] {
        if count == 0 {
            continue;
        }
        let galley =
            painter.layout_no_wrap(format!("{glyph}{count}"), FontId::proportional(11.0), color);
        let x = right - galley.size().x;
        painter.galley(egui::pos2(x, center - galley.size().y / 2.0), galley, color);
        right = x - 6.0;
    }
    if row.draft {
        let galley =
            painter.layout_no_wrap("draft".into(), FontId::proportional(10.0), colors.muted);
        let x = right - galley.size().x;
        painter.galley(
            egui::pos2(x, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
        right = x - 6.0;
    }

    let mut x = rect.left() + if row.accordion { 4.0 } else { 10.0 };
    if row.accordion {
        painter.text(
            egui::pos2(x, center),
            egui::Align2::LEFT_CENTER,
            if row.expanded {
                icon::CARET_DOWN
            } else {
                icon::CARET_RIGHT
            },
            FontId::proportional(12.0),
            colors.muted,
        );
        x += 14.0;
        let dot = egui::pos2(x + 3.5, center);
        if row.active {
            painter.circle_filled(dot, 3.5, colors.accent);
        } else {
            painter.circle_stroke(dot, 3.0, Stroke::new(1.2_f32, colors.muted));
        }
        x += 14.0;
    }
    let budget = (right - x - 6.0).max(0.0);
    let name_color = if row.active {
        colors.text
    } else {
        colors.muted
    };
    let full = painter.layout_no_wrap(row.name.to_owned(), FontId::proportional(13.0), name_color);
    let name_width = full.size().x.min(if row.branch.is_empty() {
        budget
    } else {
        budget * 0.55
    });
    let name = fit_galley(
        ui,
        row.name,
        FontId::proportional(13.0),
        name_color,
        name_width,
    );
    painter.galley(
        egui::pos2(x, center - name.size().y / 2.0),
        name,
        name_color,
    );
    x += name_width + 6.0;
    if let Some(kind) = row.kind {
        let galley = painter.layout_no_wrap(
            kind.label().to_owned(),
            FontId::proportional(10.0),
            colors.muted,
        );
        let width = galley.size().x + 8.0;
        if x + width < right - 40.0 {
            let tag =
                egui::Rect::from_min_size(egui::pos2(x, center - 7.5), Vec2::new(width, 15.0));
            painter.rect_stroke(
                tag,
                CornerRadius::same(3),
                Stroke::new(1.0_f32, colors.muted.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
            painter.galley(
                egui::pos2(tag.left() + 4.0, center - galley.size().y / 2.0),
                galley,
                colors.muted,
            );
            x += width + 6.0;
        }
    }
    let remaining = right - x;
    let mut branch_rect = None;
    if remaining > 24.0 && !row.branch.is_empty() {
        // The selected repository's branch is a button that opens the switcher;
        // its caret says so and is kept even when the name is truncated.
        let caret = if row.active { 14.0 } else { 0.0 };
        let galley = fit_galley(
            ui,
            &format!("{} {}", icon::GIT_BRANCH, row.branch),
            FontId::proportional(12.0),
            colors.muted,
            (remaining - caret).max(0.0),
        );
        let top_left = egui::pos2(x, center - galley.size().y / 2.0);
        let text_width = galley.size().x;
        painter.galley(top_left, galley, colors.muted);
        if row.active {
            painter.text(
                egui::pos2(x + text_width + 2.0, center),
                egui::Align2::LEFT_CENTER,
                icon::CARET_DOWN,
                FontId::proportional(10.0),
                colors.muted,
            );
            branch_rect = Some(egui::Rect::from_min_max(
                egui::pos2(x - 3.0, rect.top() + 2.0),
                egui::pos2(x + text_width + caret + 3.0, rect.bottom() - 2.0),
            ));
        }
    }
    RepoRowResponse {
        response: response.on_hover_text(row.tooltip.clone()),
        menu_rect,
        branch_rect,
    }
}

pub(super) struct ChangeRowResponse {
    pub(super) response: egui::Response,
    /// Stage, unstage or mark resolved.
    pub(super) primary: bool,
    pub(super) discard: bool,
}

pub(super) struct ChangeRow<'a> {
    pub(super) change: &'a GitChange,
    pub(super) marker: char,
    pub(super) discardable: bool,
    pub(super) primary_glyph: &'static str,
    pub(super) primary_tip: &'static str,
    pub(super) hover: String,
}

/// File icon, bold name, dim directory and a colored status letter. Stage and
/// discard buttons replace the letter while the row is hovered.
pub(super) fn change_row(ui: &mut egui::Ui, row: &ChangeRow) -> ChangeRowResponse {
    let colors = theme::palette(ui.visuals());
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), CHANGE_ROW_HEIGHT),
        egui::Sense::click(),
    );
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.hover);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            CornerRadius::same(2),
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    let path = &row.change.path;
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, row.hover.clone())
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let center = rect.center().y;
    let letter_left = rect.right() - 22.0;
    let mut x = rect.left() + 14.0;
    let file = super::file_icons::for_file(path);
    painter.text(
        egui::pos2(x, center),
        egui::Align2::LEFT_CENTER,
        file.glyph,
        FontId::new(14.0, super::file_icons::family()),
        file.color,
    );
    x += 20.0;
    let name_galley = fit_galley(
        ui,
        &name,
        FontId::proportional(13.0),
        colors.text,
        letter_left - x - 4.0,
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
    if matches!(row.change.kind, ChangeKind::Submodule(_)) {
        if !detail.is_empty() {
            detail.push_str("  ");
        }
        detail.push_str("submodule");
    }
    if !detail.is_empty() && letter_left - x > 24.0 {
        let galley = fit_galley(
            ui,
            &detail,
            FontId::proportional(11.5),
            colors.muted,
            letter_left - x - 4.0,
        );
        painter.galley(
            egui::pos2(x, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
    }
    let mut primary = false;
    let mut discard = false;
    // The buttons stay while the row or one of them has keyboard focus so Tab
    // can reach them. Remembered across frames because they only exist once shown.
    let focus_key = response.id.with("actions_focus");
    let focus_within = ui
        .data(|data| data.get_temp::<bool>(focus_key))
        .unwrap_or(false);
    let mut focused = response.has_focus();
    if hovered || focus_within {
        let slots = if row.discardable { 2.0 } else { 1.0 };
        let area = egui::Rect::from_min_max(
            egui::pos2(rect.right() - 4.0 - slots * BUTTON - 2.0, rect.top()),
            rect.right_bottom(),
        );
        ui.painter()
            .rect_filled(area, CornerRadius::ZERO, colors.hover);
        let button_rect = |index: f32| {
            egui::Rect::from_min_size(
                egui::pos2(
                    rect.right() - 4.0 - (index + 1.0) * BUTTON,
                    center - BUTTON / 2.0,
                ),
                Vec2::splat(BUTTON),
            )
        };
        let primary_button = ui
            .put(
                button_rect(0.0),
                egui::Button::new(row.primary_glyph).frame(false),
            )
            .on_hover_text(row.primary_tip);
        focused |= primary_button.has_focus();
        primary = super::widgets::named(primary_button, true, row.primary_tip).clicked();
        if row.discardable {
            let discard_button = ui
                .put(
                    button_rect(1.0),
                    egui::Button::new(icon::DISCARD).frame(false),
                )
                .on_hover_text("Discard changes to this file");
            focused |= discard_button.has_focus();
            discard = super::widgets::named(discard_button, true, "Discard changes to this file")
                .clicked();
        }
    } else {
        painter.text(
            egui::pos2(rect.right() - 12.0, center),
            egui::Align2::CENTER_CENTER,
            row.marker,
            FontId::monospace(11.0),
            change_color(&colors, row.marker),
        );
    }
    ui.data_mut(|data| data.insert_temp(focus_key, focused));
    ChangeRowResponse {
        response: response.on_hover_text(row.hover.clone()),
        primary,
        discard,
    }
}

pub(super) struct ListRow<'a> {
    /// Muted monospace text before the label (a short commit id, a stash name).
    pub(super) lead: &'a str,
    pub(super) text: &'a str,
    /// Muted text at the right edge (a relative date); may be empty.
    pub(super) trail: &'a str,
    pub(super) tooltip: String,
    pub(super) selected: bool,
}

/// A compact one-line row for secondary lists. Text truncates; the trailing
/// text keeps the right edge.
pub(super) fn list_row(ui: &mut egui::Ui, row: &ListRow) -> egui::Response {
    let colors = theme::palette(ui.visuals());
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), CHANGE_ROW_HEIGHT),
        egui::Sense::click(),
    );
    if row.selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.active);
    } else if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, colors.hover);
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
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, row.text.to_owned())
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let center = rect.center().y;
    let mut right = rect.right() - 8.0;
    if !row.trail.is_empty() {
        let galley = painter.layout_no_wrap(
            row.trail.to_owned(),
            FontId::proportional(11.0),
            colors.muted,
        );
        right -= galley.size().x;
        painter.galley(
            egui::pos2(right, center - galley.size().y / 2.0),
            galley,
            colors.muted,
        );
        right -= 8.0;
    }
    let mut x = rect.left() + 20.0;
    let lead = painter.layout_no_wrap(row.lead.to_owned(), FontId::monospace(11.0), colors.muted);
    let lead_width = lead.size().x;
    painter.galley(
        egui::pos2(x, center - lead.size().y / 2.0),
        lead,
        colors.muted,
    );
    x += lead_width + 8.0;
    let text = fit_galley(
        ui,
        row.text,
        FontId::proportional(13.0),
        colors.text,
        (right - x).max(0.0),
    );
    painter.galley(
        egui::pos2(x, center - text.size().y / 2.0),
        text,
        colors.text,
    );
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(row.tooltip.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo(root: &str) -> GitSnapshot {
        GitSnapshot {
            root: PathBuf::from(root),
            ..Default::default()
        }
    }

    #[test]
    fn kinds_distinguish_worktrees_submodules_and_nested_repositories() {
        let mut gitlink = GitChange::new("vendor/x", ' ', 'M');
        gitlink.kind = ChangeKind::Submodule(Default::default());
        let mut outer = repo("/w");
        outer.changes = vec![gitlink];
        let sub = repo("/w/vendor/x");
        let mut clean_sub = repo("/w/vendor/y");
        clean_sub.git_dir = PathBuf::from("/w/.git/modules/y");
        outer.common_dir = PathBuf::from("/w/.git");
        outer.git_dir = outer.common_dir.clone();
        let nested = repo("/w/crates/core");
        let mut worktree = repo("/other/w");
        worktree.git_dir = PathBuf::from("/w/.git/worktrees/w");
        worktree.common_dir = PathBuf::from("/w/.git");
        let all = vec![
            outer.clone(),
            sub.clone(),
            clean_sub.clone(),
            nested.clone(),
            worktree.clone(),
        ];

        assert_eq!(repository_kind(&all, &outer), None);
        assert_eq!(
            repository_kind(&all, &clean_sub),
            Some(RepoKind::Submodule),
            "a submodule with no pending change is still a submodule"
        );
        assert_eq!(repository_kind(&all, &sub), Some(RepoKind::Submodule));
        assert_eq!(repository_kind(&all, &nested), Some(RepoKind::Nested));
        assert_eq!(repository_kind(&all, &worktree), Some(RepoKind::Worktree));
    }
}
