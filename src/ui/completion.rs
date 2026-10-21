//! Completion list and signature-help popups for the shared editor.
//!
//! Keys are consumed before the editor sees them, but only while a popup is open
//! and the editor owns focus. Accepting an item mutates the buffer through
//! `app::completion::accept`, before any widget input in the same frame.
use super::{
    Workbench,
    popup_place::{Side, place},
};
use crate::app::{
    completion,
    editor_action::{blocked, text_widget_id},
};
use eframe::egui::{self, Align, Color32, FontId, RichText, Sense, text::LayoutJob};
use lumen_editor_surface::SurfaceOutput;

const MAX_ROWS: usize = 8;
const HOVER_MAX_WIDTH: f32 = 520.0;
const SIGNATURE_WIDTH: f32 = 420.0;
const COMPLETION_WIDTH: f32 = 300.0;

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
        // Horizontal limits are the editor itself (so popups stay clear of an open
        // side panel); vertical limits are the window.
        let bounds = egui::Rect::from_x_y_ranges(
            output.response.rect.x_range(),
            ctx.content_rect().y_range(),
        );
        let document = self.editor.files[index].id;
        if self
            .view
            .language_hover_anchor
            .is_some_and(|anchor| anchor != (document, caret))
        {
            // Moving the caret, switching tabs or editing dismisses the hover.
            self.view.language_hover = None;
            self.view.language_hover_anchor = None;
        }
        if let (Some(hover), Some(_)) = (
            self.view.language_hover.clone(),
            self.view.language_hover_anchor,
        ) {
            let mut close = false;
            let galley = ctx.fonts_mut(|fonts| {
                fonts.layout(
                    hover.clone(),
                    FontId::monospace(11.0),
                    colors.text,
                    HOVER_MAX_WIDTH - 40.0,
                )
            });
            let size = galley.size() + egui::vec2(40.0, 14.0);
            let placed = place(caret_rect, size, bounds, Side::Above, 6.0);
            egui::Area::new(egui::Id::new("language_server_hover"))
                .order(egui::Order::Foreground)
                .fixed_pos(placed.pos)
                .pivot(placed.pivot)
                .show(ctx, |ui| {
                    popup_frame(ui, |ui| {
                        ui.set_max_width(HOVER_MAX_WIDTH);
                        ui.horizontal_top(|ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&hover)
                                        .monospace()
                                        .size(crate::ui::theme::META_SIZE)
                                        .color(colors.text),
                                )
                                .wrap(),
                            );
                            if super::widgets::named(
                                ui.small_button(crate::icon::X).on_hover_text("Close hover"),
                                true,
                                "Close hover",
                            )
                            .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                });
            if close {
                self.view.language_hover = None;
                self.view.language_hover_anchor = None;
            }
        }
        if let Some((_, _, help)) = self.view.language_signature.clone() {
            let mut close = false;
            let placed = place(
                caret_rect,
                egui::vec2(SIGNATURE_WIDTH, 64.0),
                bounds,
                Side::Above,
                6.0,
            );
            egui::Area::new(egui::Id::new("language_server_signature"))
                .order(egui::Order::Foreground)
                .fixed_pos(placed.pos)
                .pivot(placed.pivot)
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
        let rows = visible.len().min(MAX_ROWS) as f32;
        let placed = place(
            caret_rect,
            egui::vec2(COMPLETION_WIDTH, rows * 18.0 + 10.0),
            bounds,
            Side::Below,
            2.0,
        );
        egui::Area::new(egui::Id::new("language_server_completion"))
            .order(egui::Order::Foreground)
            .fixed_pos(placed.pos)
            .pivot(placed.pivot)
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
    let mut append = |text: &str, color: Color32, underline: bool| {
        if !text.is_empty() {
            job.append(
                text,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    // Underline pairs with the accent colour so the active
                    // parameter does not depend on colour alone.
                    underline: if underline {
                        egui::Stroke::new(1.0_f32, color)
                    } else {
                        egui::Stroke::NONE
                    },
                    ..Default::default()
                },
            );
        }
    };
    match &help.active_parameter {
        Some(range) => {
            append(&help.label[..range.start], colors.text, false);
            append(&help.label[range.clone()], colors.accent, true);
            append(&help.label[range.end..], colors.text, false);
        }
        None => append(&help.label, colors.text, false),
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

#[cfg(test)]
mod tests {
    use crate::app::LumenApp;
    use eframe::egui;

    fn app_with_hover(caret: usize) -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        app.view.shared_editor = true;
        app.editor.new_file();
        app.editor.files[0].text = "let value = 1;\n".into();
        app.editor.files[0].enable_shared_editing();
        app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(vec![lumen_editor_core::Selection::caret(caret)], 0)
            .unwrap();
        app.view.language_hover = Some("HOVERTEXT".into());
        app.view.language_hover_anchor = Some((app.editor.files[0].id, caret));
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        (app, ctx)
    }

    fn frame(app: &mut LumenApp, ctx: &egui::Context, width: f32) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.0),
                )),
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.editor(ctx)),
        )
    }

    fn hover_rect(output: &egui::FullOutput) -> Option<egui::Rect> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "HOVERTEXT" => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
    }

    #[test]
    fn hover_stays_on_screen_next_to_its_caret_and_goes_away_when_the_caret_moves() {
        for width in [500.0, 900.0, 1400.0] {
            let (mut app, ctx) = app_with_hover(5);
            let mut output = frame(&mut app, &ctx, width);
            for _ in 0..3 {
                output = frame(&mut app, &ctx, width);
            }
            let rect = hover_rect(&output).unwrap_or_else(|| panic!("no hover at {width}"));
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
            assert!(window.contains_rect(rect), "{rect:?} escapes {window:?}");
        }
        let (mut app, ctx) = app_with_hover(5);
        for _ in 0..3 {
            frame(&mut app, &ctx, 900.0);
        }
        app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(vec![lumen_editor_core::Selection::caret(9)], 0)
            .unwrap();
        let output = frame(&mut app, &ctx, 900.0);
        assert!(hover_rect(&output).is_none(), "hover must follow its caret");
        assert!(app.view.language_hover.is_none());
        let selections = app.editor.files[0].editing.as_ref().unwrap().selections();
        assert_eq!(selections.selections()[selections.primary_index()].head, 9);
    }
}
