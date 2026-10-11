use super::Workbench;
use super::theme::{ACCENT, BG, MUTED, TAB, TEXT};
use super::widgets::{self, canvas, surface};
use crate::app::command::Command;
use crate::icon;
use crate::model::display_name;
use eframe::egui::{
    self, CornerRadius, FontId, Frame, Margin, RichText, ScrollArea, Stroke, TextEdit, Vec2,
};
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
                        let selected = self.editor.active_file == Some(index);
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
                    let line_count = self.editor.files[index].text.split('\n').count();
                    let document_id = self.editor.files[index].id;
                    self.editor_find_bar(ui, document_id);
                    crate::app::editor_action::prepare_action(self.view, document_id, ctx);
                    let reveal_find = self
                        .view
                        .find
                        .get(&document_id)
                        .is_some_and(|find| find.reveal);
                    let blocked = crate::app::editor_action::blocked(self.view);
                    let replacing = self.view.find.get_mut(&document_id).is_some_and(|find| {
                        crate::app::find::prepare_find(
                            find,
                            &self.editor.files[index],
                            ctx,
                            blocked,
                        )
                    });
                    let content_height = ui.available_height();
                    Frame::default()
                        .fill(BG)
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::ZERO)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(48.0, content_height),
                                    egui::Layout::top_down(egui::Align::Max),
                                    |ui| {
                                        ScrollArea::vertical()
                                            .id_salt(("line_numbers", document_id))
                                            .max_height(content_height)
                                            .show(ui, |ui| {
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                for number in 1..=line_count {
                                                    ui.label(
                                                        RichText::new(format!("{number}"))
                                                            .font(FontId::monospace(14.0))
                                                            .color(MUTED),
                                                    );
                                                }
                                            });
                                    },
                                );
                                let width = ui.available_width();
                                ui.allocate_ui_with_layout(
                                    Vec2::new(width, content_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ScrollArea::vertical()
                                            .id_salt(("code_editor", document_id))
                                            .max_height(content_height)
                                            .show(ui, |ui| {
                                                let text_id =
                                                    crate::app::editor_action::text_widget_id(
                                                        document_id,
                                                    );
                                                let edit = TextEdit::multiline(
                                                    &mut self.editor.files[index].text,
                                                )
                                                .id(text_id)
                                                .font(FontId::monospace(14.0))
                                                .text_color(TEXT)
                                                .desired_width(f32::INFINITY)
                                                .min_size(Vec2::new(0.0, content_height))
                                                .frame(false)
                                                .code_editor();
                                                let output = super::editor_menu::show_editor_text(
                                                    ui, edit, text_id,
                                                );
                                                let response = &output.response;
                                                if self.view.focus_editor {
                                                    response.request_focus();
                                                    self.view.focus_editor = false;
                                                }
                                                if response.changed() {
                                                    self.editor.files[index].record_text_change();
                                                }
                                                if replacing {
                                                    crate::app::find::finish_replace(
                                                        &output,
                                                        &self.editor.files[index],
                                                        ctx,
                                                    );
                                                }
                                                if let Some(find) = self
                                                    .view
                                                    .find
                                                    .get_mut(&document_id)
                                                    .filter(|find| find.open)
                                                {
                                                    find.refresh(&self.editor.files[index]);
                                                    super::editor_find::paint_find(
                                                        ui,
                                                        &output,
                                                        find,
                                                        reveal_find,
                                                    );
                                                }
                                                self.editor_context_menu(&output, document_id);
                                            });
                                    },
                                );
                            });
                        });
                });
            });
    }
}
