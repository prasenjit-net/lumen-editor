use super::Workbench;
use super::widgets::{self, HeaderAction, canvas, explorer_row, panel_header, surface};
use crate::app::command::Command;
use crate::app::view_state::ExplorerEditKind;
use crate::app::view_state::{Activity, ExplorerSelection};
use crate::icon;
use crate::model::display_name;
use eframe::egui::{self, RichText, ScrollArea};
use std::path::Path;
impl Workbench<'_> {
    fn git_badge(&self, path: &Path, directory: bool) -> Option<char> {
        let snapshot = self.view.source_control.snapshot.as_ref()?;
        let relative = path.strip_prefix(&snapshot.root).ok()?;
        snapshot
            .changes
            .iter()
            .filter(|change| {
                if directory {
                    change.path.starts_with(relative)
                } else {
                    change.path == relative
                }
            })
            .map(|change| {
                if change.index == '?' {
                    'U'
                } else if change.worktree == 'D' || change.index == 'D' {
                    'D'
                } else if change.index != ' ' {
                    'S'
                } else {
                    change.worktree
                }
            })
            .next()
    }

    pub(crate) fn explorer(&mut self, ui: &mut egui::Ui) {
        let colors = crate::ui::theme::palette(ui.visuals());
        let Some(root) = self.editor.workspace.clone() else {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No folder opened").color(colors.muted));
                if ui
                    .button(format!("{}  Open Folder", icon::FOLDER_OPEN))
                    .clicked()
                {
                    self.command(Command::OpenFolder);
                }
            });
            return;
        };
        self.explorer_entry(ui, &root, 0);
        self.explorer_keyboard(ui, &root);
    }

    /// One rule for drawing and keyboard traversal: a hidden child is neither
    /// shown nor selectable. The workspace root is never hidden.
    pub(crate) fn explorer_entry_hidden(&self, path: &Path) -> bool {
        !self.view.settings.boolean("explorer.show_hidden")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
    }

    pub(crate) fn explorer_entry(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
        if depth > 0 && self.explorer_entry_hidden(path) {
            return;
        }
        let renaming = self.view.explorer_edit.as_ref().is_some_and(
            |edit| matches!(&edit.kind, ExplorerEditKind::Rename(source) if source == path),
        );
        if renaming {
            self.explorer_edit_row(ui, depth);
            if self.file_system.is_directory(path) && self.view.expanded_dirs.contains(path) {
                for child in self.file_system.children(path) {
                    self.explorer_entry(ui, &child, depth + 1);
                }
            }
            return;
        }
        if self.file_system.is_directory(path) {
            let expanded = self.view.expanded_dirs.contains(path);
            let response = explorer_row(
                ui,
                format!(
                    "{}  {}  {}{}",
                    if expanded {
                        icon::CARET_DOWN
                    } else {
                        icon::CARET_RIGHT
                    },
                    if expanded {
                        icon::FOLDER_OPEN
                    } else {
                        icon::FOLDER
                    },
                    display_name(path),
                    self.git_badge(path, true)
                        .map(|badge| format!("  {badge}"))
                        .unwrap_or_default()
                ),
                depth * 14 + 4,
                self.view
                    .explorer_selection
                    .as_ref()
                    .is_some_and(|selection| selection.path() == path),
            );
            let response = response.on_hover_text(path.display().to_string());
            self.explorer_row_actions(&response, path, true);
            if response.clicked() {
                self.view.explorer_selection =
                    Some(ExplorerSelection::Directory(path.to_path_buf()));
                if expanded {
                    self.view.expanded_dirs.remove(path);
                } else {
                    self.view.expanded_dirs.insert(path.to_path_buf());
                }
            }
            if self.view.expanded_dirs.contains(path) {
                if self.view.explorer_edit.as_ref().is_some_and(|edit| {
                    edit.directory == path && !matches!(edit.kind, ExplorerEditKind::Rename(_))
                }) {
                    self.explorer_edit_row(ui, depth + 1);
                }
                for child in self.file_system.children(path) {
                    self.explorer_entry(ui, &child, depth + 1);
                }
            }
        } else {
            let open = if let Some(selection) = &self.view.explorer_selection {
                selection.path() == path
            } else {
                self.active()
                    .and_then(|file| file.path.as_deref())
                    .is_some_and(|open_path| open_path == path)
            };
            let response = explorer_row(
                ui,
                format!(
                    "{}  {}{}",
                    widgets::file_icon(path),
                    display_name(path),
                    self.git_badge(path, false)
                        .map(|badge| format!("  {badge}"))
                        .unwrap_or_default()
                ),
                depth * 14 + 20,
                open,
            );
            let response = response.on_hover_text(path.display().to_string());
            self.explorer_row_actions(&response, path, false);
            if response.clicked() {
                self.view.explorer_selection = Some(ExplorerSelection::File(path.to_path_buf()));
                self.command(Command::OpenPath(path.to_path_buf()));
            }
        }
    }

    pub(crate) fn sidebar(&mut self, ctx: &egui::Context) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        if !self.view.sidebar_visible {
            return;
        }
        egui::SidePanel::left("explorer")
            .default_width(240.0)
            .width_range(180.0..=600.0)
            .resizable(true)
            .show_separator_line(false)
            .frame(canvas(ctx))
            .show(ctx, |ui| {
                surface(colors.panel).show(ui, |ui| {
                    // Width belongs to SidePanel's drag state. Setting a content
                    // minimum from `available_size` can grow it on hover/pass 2.
                    ui.set_min_height(ui.available_height());
                    let explorer_actions = [
                        HeaderAction {
                            glyph: icon::FILE_PLUS,
                            tooltip: "New file",
                            command: Command::NewFile,
                            enabled: true,
                        },
                        HeaderAction {
                            glyph: icon::NEW_FOLDER,
                            tooltip: "New folder",
                            command: Command::NewFolder,
                            enabled: self.editor.workspace.is_some(),
                        },
                        HeaderAction {
                            glyph: icon::REFRESH,
                            tooltip: "Refresh explorer",
                            command: Command::RefreshExplorer,
                            enabled: self.editor.workspace.is_some(),
                        },
                        HeaderAction {
                            glyph: icon::COLLAPSE_ALL,
                            tooltip: "Collapse folders",
                            command: Command::CollapseFolders,
                            enabled: self.editor.workspace.is_some(),
                        },
                    ];
                    let source_actions = [
                        HeaderAction {
                            glyph: icon::REPO_PULL,
                            tooltip: "Pull remote changes (fast-forward only)",
                            command: Command::PullSourceControl,
                            enabled: self.view.source_control.snapshot.is_some()
                                && !self.view.source_control.loading,
                        },
                        HeaderAction {
                            glyph: icon::REPO_PUSH,
                            tooltip: "Push local commits",
                            command: Command::PushSourceControl,
                            enabled: self.view.source_control.snapshot.is_some()
                                && !self.view.source_control.loading,
                        },
                        HeaderAction {
                            glyph: icon::REFRESH,
                            tooltip: "Refresh source control",
                            command: Command::RefreshSourceControl,
                            enabled: self.editor.workspace.is_some(),
                        },
                    ];
                    let task_scope = self.default_task_scope();
                    let task_actions = [
                        HeaderAction {
                            glyph: icon::PLAY,
                            tooltip: "Run the selected task",
                            command: Command::RunSelectedTask,
                            enabled: self.view.tasks.selected.is_some(),
                        },
                        HeaderAction {
                            glyph: icon::PLUS,
                            tooltip: "New task",
                            command: Command::NewTaskEditor(task_scope),
                            enabled: true,
                        },
                        HeaderAction {
                            glyph: icon::REFRESH,
                            tooltip: "Refresh tasks",
                            command: Command::RefreshTasks,
                            enabled: true,
                        },
                        HeaderAction {
                            glyph: icon::FILE_CODE,
                            tooltip: "Open the tasks file as TOML",
                            command: Command::OpenTaskToml(task_scope),
                            enabled: true,
                        },
                    ];
                    let actions = if self.view.activity == Activity::Explorer {
                        &explorer_actions[..]
                    } else if self.view.activity == Activity::SourceControl {
                        &source_actions[..]
                    } else if self.view.activity == Activity::RunDebug {
                        &task_actions[..]
                    } else {
                        &[]
                    };
                    if let Some(command) = panel_header(ui, self.view.activity.title(), actions) {
                        self.command(command);
                    }
                    match self.view.activity {
                        Activity::Explorer => {
                            ScrollArea::vertical()
                                .id_salt("explorer_scroll")
                                .show(ui, |ui| self.explorer(ui));
                        }
                        // Inset like the header's content so inputs and labels do
                        // not touch the pane edge.
                        Activity::Search => {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(8, 0))
                                .show(ui, |ui| self.workspace_search_panel(ui));
                        }
                        Activity::SourceControl => {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(8, 0))
                                .show(ui, |ui| self.source_control_panel(ui));
                        }
                        Activity::RunDebug => {
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(8, 0))
                                .show(ui, |ui| self.tasks_panel(ui));
                        }
                        Activity::Extensions => {
                            unavailable_panel(
                                ui,
                                "Extensions",
                                "Extensions are not supported yet. Language support is built in; see Settings for language servers.",
                                None,
                            );
                        }
                    }
                });
            });
    }
}

/// An honest placeholder for an activity that is not built yet: say so, and keep
/// any control it would have had visibly disabled with the reason on hover.
fn unavailable_panel(ui: &mut egui::Ui, title: &str, message: &str, action: Option<String>) {
    let colors = crate::ui::theme::palette(ui.visuals());
    ui.label(RichText::new(title).size(18.0));
    ui.add_space(4.0);
    ui.add(egui::Label::new(RichText::new(message).size(12.0).color(colors.muted)).wrap());
    if let Some(action) = action {
        ui.add_space(8.0);
        ui.add_enabled_ui(false, |ui| {
            ui.add_sized([ui.available_width(), 32.0], egui::Button::new(action))
                .on_disabled_hover_text("Not available yet");
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{LumenApp, view_state::Activity};
    use eframe::egui;

    #[test]
    fn unfinished_activities_explain_themselves_and_offer_no_live_controls() {
        for (activity, phrase) in [(Activity::Extensions, "not supported yet")] {
            let mut app = LumenApp::default();
            app.view.activity = activity;
            app.view.sidebar_visible = true;
            let ctx = egui::Context::default();
            crate::ui::theme::configure_ui(&ctx);
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(900.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.render_component(ctx, |workbench| workbench.sidebar(ctx)),
                ));
            }
            let text: String = output
                .unwrap()
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains(phrase), "{activity:?}: {text}");
            assert!(!text.contains("Search extensions"));
        }
    }
}
