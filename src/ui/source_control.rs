use super::{Workbench, widgets::explorer_row};
use crate::{
    app::command::Command,
    services::source_control::{DiffSide, GitChange, GitSnapshot},
};
use eframe::egui::{self, FontId, RichText, ScrollArea, TextEdit};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum SourceRow<'a> {
    Heading(&'static str, usize),
    Change(&'a GitChange, DiffSide),
}

fn source_rows(snapshot: &GitSnapshot) -> Vec<SourceRow<'_>> {
    let mut rows = Vec::with_capacity(snapshot.changes.len() * 2 + 2);
    let staged: Vec<_> = snapshot
        .changes
        .iter()
        .filter(|change| change.index != ' ' && change.index != '?')
        .collect();
    if !staged.is_empty() {
        rows.push(SourceRow::Heading("STAGED CHANGES", staged.len()));
        rows.extend(
            staged
                .into_iter()
                .map(|change| SourceRow::Change(change, DiffSide::Staged)),
        );
    }
    let working: Vec<_> = snapshot
        .changes
        .iter()
        .filter(|change| change.worktree != ' ')
        .collect();
    if !working.is_empty() {
        rows.push(SourceRow::Heading("CHANGES", working.len()));
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

fn source_heading(ui: &mut egui::Ui, title: &str, count: usize) {
    let colors = crate::ui::theme::palette(ui.visuals());
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{title}  {count}"),
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
        let Some(snapshot) = self.view.source_control.snapshot.clone() else {
            source_label(ui, "No Git repository found", colors.muted);
            source_label(ui, &workspace.display().to_string(), colors.muted);
            return;
        };
        let branch_text = format!("{}  {}", crate::icon::GIT_BRANCH, snapshot.branch);
        let branch = source_label(ui, &branch_text, colors.text);
        branch.on_hover_text(snapshot.root.display().to_string());
        let staged_paths: Vec<_> = snapshot
            .changes
            .iter()
            .filter(|change| change.index != ' ' && change.index != '?')
            .map(|change| change.path.clone())
            .collect();
        let working_paths: Vec<_> = snapshot
            .changes
            .iter()
            .filter(|change| change.worktree != ' ')
            .map(|change| change.path.clone())
            .collect();
        let revertible_paths: Vec<_> = snapshot
            .changes
            .iter()
            .filter(|change| change.index != '?' && change.index != 'A')
            .map(|change| change.path.clone())
            .collect();
        ui.horizontal(|ui| {
            ui.label(RichText::new("SOURCE CONTROL").small().color(colors.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_sized(
                        [22.0, 22.0],
                        egui::Button::new(crate::icon::PLUS).frame(false),
                    )
                    .on_hover_text("Stage all working changes")
                    .clicked()
                {
                    self.command(Command::StagePaths(working_paths.clone()));
                }
                if ui
                    .add_sized([22.0, 22.0], egui::Button::new(crate::icon::X).frame(false))
                    .on_hover_text("Unstage all staged changes")
                    .clicked()
                {
                    self.command(Command::UnstagePaths(staged_paths.clone()));
                }
                if ui
                    .add_enabled(
                        !revertible_paths.is_empty(),
                        egui::Button::new(crate::icon::DISCARD).frame(false),
                    )
                    .on_hover_text("Discard all tracked changes")
                    .clicked()
                {
                    self.command(Command::RequestSourceRevert(revertible_paths.clone()));
                }
            });
        });
        let has_staged = !staged_paths.is_empty();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.view.source_control.amend_commit, "Amend")
                .on_hover_text(
                    "Amend the most recent commit; an empty message keeps its current message",
                );
            let commit_label = if self.view.source_control.amend_commit {
                "Amend"
            } else {
                "Commit"
            };
            // Reserve the full Commit/Amend button and row spacing. Reserving
            // only the button's text width makes the row a few pixels too wide;
            // SidePanel then persists that overflow as its next frame's width.
            const COMMIT_BUTTON_AND_SPACING: f32 = 70.0;
            let message_width = (ui.available_width() - COMMIT_BUTTON_AND_SPACING).max(24.0);
            let response = ui.add_sized(
                [message_width, 22.0],
                TextEdit::singleline(&mut self.view.source_control.commit_message)
                    .id_salt("source_control_commit_message")
                    .hint_text("Message"),
            );
            if self.view.source_control.commit_focus {
                response.request_focus();
                self.view.source_control.commit_focus = false;
            }
            self.view.source_control.commit_focused = response.has_focus();
            if ui
                .add_enabled(
                    (has_staged || self.view.source_control.amend_commit)
                        && !self.view.source_control.loading,
                    egui::Button::new(commit_label),
                )
                .clicked()
            {
                self.command(Command::CommitSourceControl);
            }
        });
        if let Some(error) = &self.view.source_control.commit_error {
            source_label(ui, error, colors.error);
        }
        if snapshot.changes.is_empty() {
            source_label(ui, "No changes", colors.muted);
            return;
        }
        let rows = source_rows(&snapshot);
        let mut requested = None;
        ScrollArea::vertical()
            .id_salt("source_control_changes")
            .show_rows(ui, 22.0, rows.len(), |ui, visible| {
                for index in visible {
                    let SourceRow::Change(change, side) = rows[index] else {
                        if let SourceRow::Heading(title, count) = rows[index] {
                            source_heading(ui, title, count);
                        }
                        continue;
                    };
                    let marker = if side == DiffSide::Untracked {
                        'U'
                    } else if side == DiffSide::Staged {
                        change.index
                    } else {
                        change.worktree
                    };
                    let label = format!("{marker}  {}", change.path.display());
                    let response = explorer_row(ui, label, 4, false)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(change.path.display().to_string());
                    let action = match side {
                        DiffSide::Staged => Command::UnstagePaths(vec![change.path.clone()]),
                        DiffSide::Working | DiffSide::Untracked => {
                            Command::StagePaths(vec![change.path.clone()])
                        }
                    };
                    let action_label = if side == DiffSide::Staged {
                        crate::icon::X
                    } else {
                        crate::icon::PLUS
                    };
                    let revertible = change.index != '?' && change.index != 'A';
                    let revert_clicked = revertible
                        && ui
                            .put(
                                egui::Rect::from_min_size(
                                    response.rect.right_top() - egui::vec2(44.0, 0.0),
                                    egui::vec2(22.0, response.rect.height()),
                                ),
                                egui::Button::new(crate::icon::DISCARD).frame(false),
                            )
                            .on_hover_text("Discard changes to this file")
                            .clicked();
                    let action_clicked = ui
                        .put(
                            egui::Rect::from_min_size(
                                response.rect.right_top() - egui::vec2(22.0, 0.0),
                                egui::vec2(22.0, response.rect.height()),
                            ),
                            egui::Button::new(action_label).frame(false),
                        )
                        .on_hover_text(if side == DiffSide::Staged {
                            "Unstage change"
                        } else {
                            "Stage change"
                        })
                        .clicked();
                    if action_clicked {
                        self.command(action);
                    } else if revert_clicked {
                        self.command(Command::RequestSourceRevert(vec![change.path.clone()]));
                    } else if response.clicked() {
                        requested = Some((change.path.clone(), side));
                    }
                }
            });
        if let Some((path, side)) = requested {
            self.command(Command::ShowSourceDiff(path, side));
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
        let mut widths = vec![source_sidebar_frame(&mut app, &ctx, vec![])];
        app.view.source_control.snapshot = Some(GitSnapshot {
            root: PathBuf::from("/workspace"),
            branch: "feature/a-branch-name-long-enough-to-previously-grow-the-sidebar".into(),
            changes: vec![GitChange {
                path: PathBuf::from("src/long-file.rs"),
                index: 'M',
                worktree: 'M',
            }],
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
            changes: vec![GitChange {
                path: PathBuf::from("src/main.rs"),
                index: 'M',
                worktree: ' ',
            }],
        });
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
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
            changes: vec![GitChange {
                path: PathBuf::from("src/a-very-long-file-name-that-must-stay-clipped.rs"),
                index: ' ',
                worktree: 'M',
            }],
        });
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
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
            let mut editor = EditorState::with_workspace(PathBuf::from("/workspace"));
            let mut view = ViewState::default();
            view.source_control.snapshot = Some(GitSnapshot {
                root: PathBuf::from("/workspace"),
                branch: "main".into(),
                changes: vec![GitChange {
                    path: PathBuf::from("src/file.rs"),
                    index: ' ',
                    worktree: 'M',
                }],
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
            for expected in ["main", "src/file.rs"] {
                let x = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text().contains(expected) => {
                        Some(text.pos.x)
                    }
                    _ => None,
                });
                // The branch text follows its icon; both labels still begin
                // in the left portion of the panel rather than being centred.
                assert!(x.is_some_and(|x| x < 80.0), "{width}: {expected} at {x:?}");
            }
        }
    }

    #[test]
    fn staged_working_and_untracked_changes_get_distinct_rows() {
        let snapshot = GitSnapshot {
            root: PathBuf::from("/workspace"),
            branch: "main".into(),
            changes: vec![
                GitChange {
                    path: PathBuf::from("both.rs"),
                    index: 'M',
                    worktree: 'M',
                },
                GitChange {
                    path: PathBuf::from("new.rs"),
                    index: '?',
                    worktree: '?',
                },
            ],
        };
        let rows = source_rows(&snapshot);
        assert!(matches!(rows[0], SourceRow::Heading("STAGED CHANGES", 1)));
        assert!(matches!(rows[1], SourceRow::Change(_, DiffSide::Staged)));
        assert!(matches!(rows[2], SourceRow::Heading("CHANGES", 2)));
        assert!(matches!(rows[3], SourceRow::Change(_, DiffSide::Working)));
        assert!(matches!(rows[4], SourceRow::Change(_, DiffSide::Untracked)));
    }

    #[test]
    fn staged_diff_uses_both_editor_columns_at_multiple_widths() {
        for width in [620.0, 1100.0] {
            let ctx = egui::Context::default();
            theme::configure_ui(&ctx);
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
    fn working_diff_exposes_compact_stage_buttons_for_each_hunk() {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
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
}
