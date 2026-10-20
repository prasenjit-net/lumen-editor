use super::Workbench;
use super::widgets::{self, HeaderAction, canvas, panel_header_with_menu, surface};
use crate::app::command::Command;
use crate::app::view_state::ExplorerEditKind;
use crate::app::view_state::{Activity, ExplorerSelection};
use crate::icon;
use crate::model::display_name;
use crate::ui::file_icons;
use eframe::egui::{self, RichText, ScrollArea};
use std::path::Path;
impl Workbench<'_> {
    fn git_badge(&self, path: &Path, directory: bool) -> Option<char> {
        let snapshot = self.view.source_control.owning_snapshot(path)?;
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
                if change.is_conflict() {
                    '!'
                } else if change.index == '?' {
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
            let response = widgets::explorer_row_with_icon(
                ui,
                Some(if expanded {
                    icon::CARET_DOWN
                } else {
                    icon::CARET_RIGHT
                }),
                Some(file_icons::for_folder(path, expanded)),
                format!(
                    "{}{}",
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
            let response = widgets::explorer_row_with_icon(
                ui,
                None,
                Some(file_icons::for_file(path)),
                format!(
                    "{}{}",
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
                    let source_actions = [HeaderAction {
                        glyph: icon::REFRESH,
                        tooltip: "Refresh source control and rescan repositories",
                        command: Command::RefreshSourceControl,
                        enabled: self.editor.workspace.is_some(),
                    }];
                    let source_menu = crate::ui::source_control_menu::panel_menu(
                        self.editor.workspace.is_some(),
                        !self.view.source_control.loading,
                    );
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
                    let menu = (self.view.activity == Activity::SourceControl)
                        .then_some(("Source Control actions", &source_menu[..]));
                    if let Some(command) =
                        panel_header_with_menu(ui, self.view.activity.title(), actions, menu)
                    {
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
                    }
                });
            });
    }
}
