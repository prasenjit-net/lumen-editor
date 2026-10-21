//! Markdown-lite rendering of agent replies in chat tabs.
use super::theme;
use crate::{
    icon,
    model::markdown_lite::{self, Block, Inline},
};
use eframe::egui::{
    self, Align, Color32, FontId, Frame, Layout, Margin, RichText, ScrollArea, TextFormat,
    text::LayoutJob,
};

const TEXT_SIZE: f32 = 12.5;

pub(super) fn agent_message(ui: &mut egui::Ui, text: &str) {
    let colors = crate::ui::theme::palette(ui.visuals());
    for block in markdown_lite::parse(text) {
        match block {
            Block::Paragraph(inlines) => {
                ui.add(
                    egui::Label::new(job(&inlines, TEXT_SIZE, colors.text, colors))
                        .selectable(true),
                );
            }
            Block::Heading(inlines) => {
                ui.add(
                    egui::Label::new(job(&inlines, TEXT_SIZE + 1.5, colors.text, colors))
                        .selectable(true),
                );
            }
            Block::Item { marker, content } => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(marker).size(TEXT_SIZE).color(colors.muted));
                    ui.add(
                        egui::Label::new(job(&content, TEXT_SIZE, colors.text, colors))
                            .selectable(true),
                    );
                });
            }
            Block::Code { language, text } => code_block(ui, &language, &text),
        }
    }
}

fn code_block(ui: &mut egui::Ui, language: &str, text: &str) {
    let colors = crate::ui::theme::palette(ui.visuals());
    Frame::default()
        .fill(colors.panel)
        .stroke(egui::Stroke::new(1.0_f32, colors.border))
        .corner_radius(3)
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if language.is_empty() {
                        "code"
                    } else {
                        language
                    })
                    .size(10.0)
                    .color(colors.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button(icon::COPY)
                        .on_hover_text("Copy code")
                        .clicked()
                    {
                        ui.ctx().copy_text(text.to_owned());
                    }
                });
            });
            ScrollArea::horizontal()
                .id_salt(("code", text.len(), language))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(text)
                                .monospace()
                                .size(11.5)
                                .color(colors.text),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(true),
                    );
                });
        });
}

fn job(inlines: &[Inline], size: f32, color: Color32, colors: theme::Palette) -> LayoutJob {
    let mut job = LayoutJob::default();
    for inline in inlines {
        let (text, format) = match inline {
            Inline::Text(text) => (
                text,
                TextFormat {
                    font_id: FontId::proportional(size),
                    color,
                    ..Default::default()
                },
            ),
            Inline::Bold(text) => (
                text,
                TextFormat {
                    font_id: FontId::proportional(size),
                    color: colors.text,
                    ..Default::default()
                },
            ),
            Inline::Code(text) => (
                text,
                TextFormat {
                    font_id: FontId::monospace(size - 1.0),
                    color,
                    background: colors.input,
                    ..Default::default()
                },
            ),
        };
        job.append(text, 0.0, format);
    }
    job
}
