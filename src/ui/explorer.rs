use super::Workbench;
use super::theme::{MUTED, PANEL};
use super::widgets::{self, HeaderAction, canvas, explorer_row, panel_header, surface};
use crate::app::command::Command;
use crate::app::view_state::ExplorerEditKind;
use crate::app::view_state::{Activity, ExplorerSelection};
use crate::icon;
use crate::model::display_name;
use eframe::egui::{self, RichText, ScrollArea, TextEdit};
use std::path::Path;
impl Workbench<'_> {
    pub(crate) fn explorer(&mut self, ui: &mut egui::Ui) {
        let Some(root) = self.editor.workspace.clone() else {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No folder opened").color(MUTED));
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

    pub(crate) fn explorer_entry(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
        if depth > 0
            && !self.view.settings.boolean("explorer.show_hidden")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
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
                    "{}  {}  {}",
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
                    display_name(path)
                ),
                depth * 14 + 4,
                self.view
                    .explorer_selection
                    .as_ref()
                    .is_some_and(|selection| selection.path() == path),
            );
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
                format!("{}  {}", widgets::file_icon(path), display_name(path)),
                depth * 14 + 20,
                open,
            );
            self.explorer_row_actions(&response, path, false);
            if response.clicked() {
                self.view.explorer_selection = Some(ExplorerSelection::File(path.to_path_buf()));
                self.command(Command::OpenPath(path.to_path_buf()));
            }
        }
    }

    pub(crate) fn sidebar(&mut self, ctx: &egui::Context) {
        if !self.view.sidebar_visible {
            return;
        }
        egui::SidePanel::left("explorer")
            .default_width(240.0)
            .width_range(180.0..=600.0)
            .resizable(true)
            .show_separator_line(false)
            .frame(canvas())
            .show(ctx, |ui| {
                surface(PANEL).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
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
                    let actions = if self.view.activity == Activity::Explorer {
                        &explorer_actions[..]
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
                        Activity::Search => {
                            ui.label(RichText::new("Search").size(18.0));
                            ui.add_space(4.0);
                            ui.add(
                                TextEdit::singleline(&mut self.view.search)
                                    .hint_text("Search files")
                                    .desired_width(f32::INFINITY),
                            );
                        }
                        Activity::SourceControl => {
                            ui.label(RichText::new("Source Control").size(18.0));
                            ui.label(RichText::new("Git integration is coming next.").color(MUTED));
                        }
                        Activity::RunDebug => {
                            ui.label(RichText::new("Run and Debug").size(18.0));
                            ui.add_sized(
                                [ui.available_width(), 32.0],
                                egui::Button::new(format!("{}  Run project", icon::PLAY)),
                            );
                        }
                        Activity::Extensions => {
                            ui.label(RichText::new("Extensions").size(18.0));
                            ui.add(
                                TextEdit::singleline(&mut self.view.search)
                                    .hint_text("Search extensions")
                                    .desired_width(f32::INFINITY),
                            );
                        }
                    }
                });
            });
    }
}
