use super::Workbench;
use super::theme::{MUTED, PANEL, TEXT};
use super::widgets::{self, canvas, explorer_row, surface};
use crate::app::command::Command;
use crate::app::view_state::Activity;
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
    }

    pub(crate) fn explorer_entry(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
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
                false,
            );
            if response.clicked() {
                if expanded {
                    self.view.expanded_dirs.remove(path);
                } else {
                    self.view.expanded_dirs.insert(path.to_path_buf());
                }
            }
            if expanded {
                for child in self.file_system.children(path) {
                    self.explorer_entry(ui, &child, depth + 1);
                }
            }
        } else {
            let open = self
                .active()
                .and_then(|file| file.path.as_deref())
                .is_some_and(|open_path| open_path == path);
            let response = explorer_row(
                ui,
                format!("{}  {}", widgets::file_icon(path), display_name(path)),
                depth * 14 + 20,
                open,
            );
            if response.clicked() {
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
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.add_sized(
                            [(ui.available_width() - 114.0).max(0.0), 35.0],
                            egui::Label::new(
                                RichText::new(self.view.activity.title())
                                    .size(11.0)
                                    .color(TEXT),
                            ),
                        );
                        if self.view.activity == Activity::Explorer {
                            if ui
                                .button(icon::FILE_PLUS)
                                .on_hover_text("New file")
                                .clicked()
                            {
                                self.command(Command::NewFile);
                            }
                            if ui
                                .button(icon::FILE_ARROW_UP)
                                .on_hover_text("Open file")
                                .clicked()
                            {
                                self.command(Command::OpenFile);
                            }
                            if ui
                                .button(icon::FOLDER_OPEN)
                                .on_hover_text("Open folder")
                                .clicked()
                            {
                                self.command(Command::OpenFolder);
                            }
                        }
                    });
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
