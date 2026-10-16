//! Completion list and signature-help popups for the shared editor.
//!
//! Keys are consumed before the editor sees them, but only while a popup is open
//! and the editor owns focus. Accepting an item mutates the buffer through
//! `app::completion::accept`, before any widget input in the same frame.
use super::Workbench;
use crate::app::{
    completion,
    editor_action::{blocked, text_widget_id},
};
use eframe::egui::{self, Align, Color32, FontId, RichText, Sense, text::LayoutJob};
use lumen_editor_surface::SurfaceOutput;

const MAX_ROWS: usize = 8;

impl Workbench<'_> {
    /// Validate the session against the buffer, then apply navigation and accept
    /// keys. Must run before the editor widget reads input this frame.
    pub(super) fn completion_before_input(&mut self, ctx: &egui::Context, index: usize) {
        let document = self.editor.files[index].id;
        if self
            .view
            .language_signature
            .as_ref()
            .is_some_and(|(id, revision, _)| {
                *id != document || *revision != self.editor.files[index].revision
            })
        {
            self.view.language_signature = None;
        }
        let Some(session) = self.view.language_completion.as_mut() else {
            return;
        };
        let Some(buffer) = self.editor.files[index].editing.as_mut() else {
            return;
        };
        let selections = buffer.selections();
        let primary = selections.selections()[selections.primary_index()];
        let visible = (session.document == document
            && selections.selections().len() == 1
            && primary.anchor == primary.head)
            .then(|| completion::visible(session, buffer.text(), primary.head))
            .flatten()
            .filter(|visible| !visible.is_empty());
        let Some(visible) = visible else {
            self.view.language_completion = None;
            return;
        };
        session.selected = session.selected.min(visible.len() - 1);
        let focused = ctx.memory(|memory| memory.has_focus(text_widget_id(document)));
        let mut accept = session.pending_accept.take();
        if focused {
            let none = egui::Modifiers::NONE;
            ctx.input_mut(|input| {
                if input.consume_key(none, egui::Key::ArrowDown) {
                    session.selected = (session.selected + 1) % visible.len();
                }
                if input.consume_key(none, egui::Key::ArrowUp) {
                    session.selected = (session.selected + visible.len() - 1) % visible.len();
                }
                if input.consume_key(none, egui::Key::Enter)
                    || input.consume_key(none, egui::Key::Tab)
                {
                    accept = accept.or(Some(visible[session.selected]));
                }
            });
        }
        // egui drops widget focus on Escape before this runs, so Escape closes the
        // popup whether or not the editor still looks focused, then restores focus.
        if !blocked(self.view)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.view.language_completion = None;
            ctx.memory_mut(|memory| memory.request_focus(text_widget_id(document)));
            return;
        }
        let Some(item) = accept else {
            return;
        };
        let session = self.view.language_completion.take().unwrap();
        let Some(chosen) = session.items.get(item) else {
            return;
        };
        match completion::accept(buffer, &session, chosen) {
            Ok(note) => {
                self.editor.files[index].sync_shared_editing();
                self.view.status = note.unwrap_or("Completion applied").into();
            }
            Err(reason) => self.view.status = reason.into(),
        }
        ctx.memory_mut(|memory| memory.request_focus(text_widget_id(document)));
    }

    /// Paint the open list and signature help below/above the caret.
    pub(super) fn completion_popups(
        &mut self,
        ctx: &egui::Context,
        output: &SurfaceOutput,
        index: usize,
    ) {
        let colors = crate::ui::theme::palette(&ctx.style().visuals);
        let Some(buffer) = self.editor.files[index].editing.as_ref() else {
            return;
        };
        let caret = buffer.selections().selections()[buffer.selections().primary_index()].head;
        let caret_rect = output.geometry.caret_rect(caret);
        if let Some((_, _, help)) = self.view.language_signature.clone() {
            let mut close = false;
            egui::Area::new(egui::Id::new("language_server_signature"))
                .order(egui::Order::Foreground)
                .fixed_pos(caret_rect.left_top() - egui::vec2(0.0, 6.0))
                .pivot(egui::Align2::LEFT_BOTTOM)
                .show(ctx, |ui| {
                    popup_frame(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.horizontal(|ui| {
                            ui.label(signature_label(&help, colors));
                            if super::widgets::named(
                                ui.small_button(crate::icon::X).on_hover_text("Close"),
                                true,
                                "Close signature help",
                            )
                            .clicked()
                            {
                                close = true;
                            }
                        });
                        if help.count > 1 {
                            ui.label(
                                RichText::new(format!("{} of {}", help.index, help.count))
                                    .small()
                                    .color(colors.muted),
                            );
                        }
                        if let Some(documentation) = &help.documentation {
                            ui.label(
                                RichText::new(summary(documentation))
                                    .small()
                                    .color(colors.muted),
                            );
                        }
                    });
                });
            if close {
                self.view.language_signature = None;
            }
        }
        let Some(session) = self.view.language_completion.as_mut() else {
            return;
        };
        let Some(visible) = completion::visible(session, buffer.text(), caret) else {
            return;
        };
        let mut chosen = None;
        egui::Area::new(egui::Id::new("language_server_completion"))
            .order(egui::Order::Foreground)
            .fixed_pos(caret_rect.left_bottom() + egui::vec2(0.0, 2.0))
            .show(ctx, |ui| {
                popup_frame(ui, |ui| {
                    ui.set_min_width(260.0);
                    ui.set_max_width(520.0);
                    let row_height = 18.0;
                    egui::ScrollArea::vertical()
                        .max_height(row_height * MAX_ROWS as f32)
                        .auto_shrink([true, true])
                        .show(ui, |ui| {
                            for (position, item_index) in visible.iter().enumerate() {
                                let item = &session.items[*item_index];
                                let selected = position == session.selected;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), row_height),
                                    Sense::click(),
                                );
                                if selected {
                                    ui.painter().rect_filled(rect, 2, colors.active);
                                    ui.scroll_to_rect(rect, Some(Align::Center));
                                } else if response.hovered() {
                                    ui.painter().rect_filled(rect, 2, colors.hover);
                                }
                                let font = FontId::monospace(12.0);
                                let label_end = ui.painter().text(
                                    rect.left_center() + egui::vec2(6.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    &item.label,
                                    font,
                                    colors.text,
                                );
                                if let Some(detail) = &item.detail {
                                    ui.painter().text(
                                        egui::pos2(label_end.right() + 10.0, rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        detail,
                                        FontId::proportional(11.0),
                                        colors.muted,
                                    );
                                }
                                if response.clicked() {
                                    chosen = Some(*item_index);
                                }
                            }
                        });
                });
            });
        if let Some(item) = chosen {
            session.pending_accept = Some(item);
            ctx.request_repaint();
        }
    }
}

fn popup_frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let colors = crate::ui::theme::palette(ui.visuals());
    egui::Frame::new()
        .fill(colors.panel)
        .stroke(egui::Stroke::new(1.0_f32, colors.border))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(4, 4))
        .show(ui, add);
}

/// Signature text with the active parameter highlighted.
fn signature_label(
    help: &lumen_language_services::incoming::SignatureHelp,
    colors: crate::ui::theme::Palette,
) -> LayoutJob {
    let font = FontId::monospace(11.0);
    let mut job = LayoutJob::default();
    let mut append = |text: &str, color: Color32| {
        if !text.is_empty() {
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                },
            );
        }
    };
    match &help.active_parameter {
        Some(range) => {
            append(&help.label[..range.start], colors.text);
            append(&help.label[range.clone()], colors.accent);
            append(&help.label[range.end..], colors.text);
        }
        None => append(&help.label, colors.text),
    }
    job
}

/// First paragraph of server documentation, capped so the popup stays compact.
fn summary(documentation: &str) -> String {
    let first = documentation
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .trim();
    let mut shortened: String = first.chars().take(240).collect();
    if shortened.len() < first.len() {
        shortened.push('…');
    }
    shortened
}
