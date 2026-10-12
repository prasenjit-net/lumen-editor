use super::Workbench;
use super::theme::{ACCENT, BG, MUTED, TAB, TEXT};
use super::widgets::{self, canvas, surface};
use crate::app::command::Command;
use crate::icon;
use crate::model::display_name;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke};
use std::path::Path;
impl Workbench<'_> {
    pub(crate) fn editor_tabs(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        let mut close = None;
        ScrollArea::horizontal()
            .id_salt("editor_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (index, file) in self.editor.files.iter().enumerate() {
                        let selected =
                            !self.view.settings.open && self.editor.active_file == Some(index);
                        Frame::default()
                            .fill(if selected { BG } else { TAB })
                            .inner_margin(Margin::symmetric(10, 6))
                            .corner_radius(CornerRadius {
                                nw: 3,
                                ne: 3,
                                sw: 0,
                                se: 0,
                            })
                            .show(ui, |ui| {
                                let start = ui.cursor().min;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let path =
                                        file.path.as_deref().unwrap_or(Path::new(&file.name));
                                    let title = format!(
                                        "{} {}{}",
                                        widgets::file_icon(path),
                                        file.name,
                                        if file.dirty { " ●" } else { "" }
                                    );
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(title).color(if selected {
                                                    TEXT
                                                } else {
                                                    MUTED
                                                }),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        select = Some(index);
                                    }
                                    if ui
                                        .add(egui::Button::new(icon::X).frame(false))
                                        .on_hover_text("Close editor")
                                        .clicked()
                                    {
                                        close = Some(index);
                                    }
                                });
                                let rect = ui.min_rect();
                                if selected {
                                    ui.painter().hline(
                                        (start.x - 7.0)..=(rect.right() + 7.0),
                                        start.y - 6.0,
                                        Stroke::new(1.0_f32, ACCENT),
                                    );
                                }
                            });
                    }
                    if self.view.settings.tab_open {
                        let selected = self.view.settings.open;
                        Frame::new()
                            .fill(if selected { BG } else { TAB })
                            .inner_margin(Margin::symmetric(10, 6))
                            .corner_radius(CornerRadius {
                                nw: 3,
                                ne: 3,
                                sw: 0,
                                se: 0,
                            })
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!("{} Settings", icon::GEAR))
                                                    .color(if selected { TEXT } else { MUTED }),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        self.command(Command::Settings);
                                    }
                                    if ui
                                        .small_button(icon::X)
                                        .on_hover_text("Close Settings (drafts retained)")
                                        .clicked()
                                    {
                                        self.view.settings.tab_open = false;
                                        self.view.settings.open = false;
                                    }
                                });
                            });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(icon::PLUS)
                            .on_hover_text("New file")
                            .clicked()
                        {
                            self.command(Command::NewUntitled);
                        }
                    });
                });
            });
        if let Some(index) = select {
            self.view.settings.open = false;
            self.editor.active_file = Some(index);
            self.view.focus_editor = true;
        }
        if let Some(index) = close {
            self.command(Command::RequestClose(index));
        }
    }

    pub(crate) fn editor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(canvas())
            .show(ctx, |ui| {
                if ui.rect_contains_pointer(ui.max_rect())
                    && ui.input(|input| input.pointer.any_pressed())
                {
                    self.view.explorer_focused = false;
                }
                surface(BG).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.editor_tabs(ui);
                    if self.view.settings.open {
                        self.settings_page(ui);
                        return;
                    }
                    let Some(index) = self.editor.active_file else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(130.0);
                            ui.label(RichText::new("LUMEN").size(28.0).strong().color(MUTED));
                            ui.label(RichText::new("A focused local file editor").color(MUTED));
                            ui.add_space(20.0);
                            if ui
                                .button(format!("{}  Open File", icon::FILE_ARROW_UP))
                                .clicked()
                            {
                                self.command(Command::OpenFile);
                            }
                            if ui
                                .button(format!("{}  Open Folder", icon::FOLDER_OPEN))
                                .clicked()
                            {
                                self.command(Command::OpenFolder);
                            }
                            ui.label(
                                RichText::new("⌘O to open a file  •  ⌘N for a new file")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        });
                        return;
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        if let Some(root) = &self.editor.workspace {
                            ui.label(RichText::new(display_name(root)).size(11.0).color(MUTED));
                            ui.label(RichText::new(icon::CARET_RIGHT).color(MUTED));
                        }
                        ui.label(
                            RichText::new(&self.editor.files[index].name)
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                    ui.add_space(2.0);
                    let document_id = self.editor.files[index].id;
                    self.editor_find_bar(ui, document_id);
                    self.editor_viewport(ui, index);
                });
            });
    }
}
