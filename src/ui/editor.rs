use super::Workbench;
use super::theme::{BG, MUTED};
use super::widgets::{canvas, surface};
use crate::app::command::Command;
use crate::icon;
use crate::model::display_name;
use eframe::egui::{self, RichText};
impl Workbench<'_> {
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
                    if self
                        .view
                        .source_control
                        .diff
                        .as_ref()
                        .is_some_and(|diff| diff.active)
                    {
                        self.source_diff_editor(ui);
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
                    if self.editor.files[index].disk_changed {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("File changed on disk")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                            if ui.small_button("Review Changes").clicked() {
                                self.command(Command::ReviewDiskChange(document_id));
                            }
                        });
                    }
                    self.editor_find_bar(ui, document_id);
                    let hover_position = ui.cursor().min + egui::vec2(56.0, 22.0);
                    self.editor_viewport(ui, index);
                    if let Some(hover) = self.view.language_hover.clone() {
                        egui::Area::new(egui::Id::new("language_server_hover"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(hover_position)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::new()
                                    .fill(super::theme::PANEL)
                                    .stroke(egui::Stroke::new(1.0_f32, super::theme::BORDER))
                                    .corner_radius(3)
                                    .inner_margin(egui::Margin::symmetric(8, 5))
                                    .show(ui, |ui| {
                                        ui.set_max_width(520.0);
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(hover)
                                                        .monospace()
                                                        .size(11.0)
                                                        .color(super::theme::TEXT),
                                                )
                                                .wrap(),
                                            );
                                            if ui
                                                .small_button(icon::X)
                                                .on_hover_text("Close hover")
                                                .clicked()
                                            {
                                                self.view.language_hover = None;
                                            }
                                        });
                                    });
                            });
                    }
                });
            });
    }
}
