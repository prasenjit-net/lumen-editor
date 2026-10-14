use super::{Workbench, theme, widgets::explorer_row};
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
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
    ui.painter().text(
        rect.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{title}  {count}"),
        FontId::proportional(11.5),
        theme::MUTED,
    );
}

impl Workbench<'_> {
    pub(crate) fn source_control_panel(&mut self, ui: &mut egui::Ui) {
        let Some(workspace) = &self.editor.workspace else {
            ui.label(RichText::new("Open a folder to view source control").color(theme::MUTED));
            return;
        };
        if self.view.source_control.loading && self.view.source_control.snapshot.is_none() {
            ui.label(RichText::new("Reading Git status…").color(theme::MUTED));
            return;
        }
        if let Some(error) = &self.view.source_control.error {
            ui.label(RichText::new(error).color(theme::DIFF_REMOVED_TEXT));
            return;
        }
        let Some(snapshot) = self.view.source_control.snapshot.clone() else {
            ui.label(RichText::new("No Git repository found").color(theme::MUTED));
            ui.label(
                RichText::new(workspace.display().to_string())
                    .small()
                    .color(theme::MUTED),
            );
            return;
        };
        ui.label(
            RichText::new(format!("{}  {}", crate::icon::GIT_BRANCH, snapshot.branch))
                .color(theme::TEXT),
        )
        .on_hover_text(snapshot.root.display().to_string());
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
        ui.horizontal(|ui| {
            ui.label(RichText::new("SOURCE CONTROL").small().color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(format!("{} Stage All", crate::icon::PLUS))
                    .on_hover_text("Stage all working changes")
                    .clicked()
                {
                    self.command(Command::StagePaths(working_paths.clone()));
                }
                if ui
                    .small_button(format!("{} Unstage All", crate::icon::X))
                    .on_hover_text("Unstage all staged changes")
                    .clicked()
                {
                    self.command(Command::UnstagePaths(staged_paths.clone()));
                }
            });
        });
        let has_staged = !staged_paths.is_empty();
        ui.horizontal(|ui| {
            let response = ui.add_enabled(
                has_staged && !self.view.source_control.loading,
                TextEdit::singleline(&mut self.view.source_control.commit_message)
                    .id_salt("source_control_commit_message")
                    .hint_text("Message (press Commit to submit)"),
            );
            if self.view.source_control.commit_focus {
                response.request_focus();
                self.view.source_control.commit_focus = false;
            }
            if ui
                .add_enabled(
                    has_staged && !self.view.source_control.loading,
                    egui::Button::new("Commit"),
                )
                .clicked()
            {
                self.command(Command::CommitSourceControl);
            }
        });
        if let Some(error) = &self.view.source_control.commit_error {
            ui.label(RichText::new(error).small().color(theme::DIFF_REMOVED_TEXT));
        }
        if snapshot.changes.is_empty() {
            ui.label(RichText::new("No changes").color(theme::MUTED));
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
                    } else if response.clicked() {
                        requested = Some((change.path.clone(), side));
                    }
                }
            });
        if let Some((path, side)) = requested {
            self.command(Command::ShowSourceDiff(path, side));
        }
    }

    pub(crate) fn source_diff_editor(&mut self, ui: &mut egui::Ui) {
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
                        ui.label(RichText::new("Hunks").small().color(theme::MUTED));
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
                                    .color(theme::MUTED),
                            );
                        }
                    });
                }
                ui.columns(2, |columns| {
                    columns[0].label(
                        RichText::new(match side {
                            DiffSide::Staged => "HEAD · read-only",
                            DiffSide::Working => "Index · read-only",
                            DiffSide::Untracked => "New file · no previous content",
                        })
                        .small()
                        .color(theme::MUTED),
                    );
                    read_only_source(
                        &mut columns[0],
                        &comparison.base,
                        &comparison.old_lines,
                        theme::DIFF_REMOVED_BG,
                        "git_diff_base",
                        &font,
                        line_height,
                    );
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
                            .color(theme::MUTED),
                        );
                        let document = self.editor.files[index].id;
                        self.editor_find_bar(&mut columns[1], document);
                        self.editor_viewport(&mut columns[1], index);
                    } else {
                        columns[1].label(
                            RichText::new(if side == DiffSide::Staged {
                                "Index · read-only"
                            } else {
                                "Working Tree · file missing"
                            })
                            .small()
                            .color(theme::MUTED),
                        );
                        read_only_source(
                            &mut columns[1],
                            comparison.changed.as_deref().unwrap_or_default(),
                            &comparison.new_lines,
                            theme::DIFF_ADDED_BG,
                            "git_diff_changed",
                            &font,
                            line_height,
                        );
                    }
                });
            }
            Err(error) => {
                ui.label(RichText::new(error).color(theme::DIFF_REMOVED_TEXT));
            }
        }
    }
}

fn read_only_source(
    ui: &mut egui::Ui,
    text: &str,
    changed: &BTreeSet<usize>,
    changed_background: egui::Color32,
    id: &str,
    font: &FontId,
    line_height: f32,
) {
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
    ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false])
        .show_rows(ui, height, lines.len(), |ui, rows| {
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
                                .color(theme::MUTED),
                        );
                        ui.label(
                            RichText::new(lines[index])
                                .font(font.clone())
                                .color(theme::TEXT),
                        );
                    });
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::view_state::{SourceDiffView, ViewState},
        model::EditorState,
        services::{
            LocalFileSystem,
            source_control::{GitChange, GitComparison, GitHunk, GitSnapshot},
        },
    };
    use std::path::PathBuf;

    #[test]
    fn source_rows_stay_left_aligned_at_panel_widths() {
        for width in [180.0, 300.0, 600.0] {
            let ctx = egui::Context::default();
            theme::configure_ui(&ctx);
            let mut editor = EditorState {
                workspace: Some(PathBuf::from("/workspace")),
                ..Default::default()
            };
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
                assert!(x.is_some_and(|x| x < 30.0), "{width}: {expected} at {x:?}");
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
                    },
                    GitHunk {
                        header: "@@ -8 +8 @@".into(),
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
}
