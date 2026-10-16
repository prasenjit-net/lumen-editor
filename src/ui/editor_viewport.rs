//! One scroll/layout authority for text, gutter, search highlights and hit tests.
use super::{
    Workbench,
    theme::{BG, MUTED, TEXT},
};
use crate::app::{
    editor_action::{blocked, prepare_action, text_widget_id},
    editor_clipboard::{finish_clipboard, prepare_clipboard},
    editor_input::{cancel_input, finish_input, prepare_input},
    find::{finish_replace, prepare_find},
};
use eframe::egui::{self, FontId, Frame, Margin, ScrollArea, TextEdit};

impl Workbench<'_> {
    pub(super) fn editor_viewport(&mut self, ui: &mut egui::Ui, index: usize) {
        #[cfg(feature = "shared-editor")]
        if self.view.shared_editor || self.editor.files[index].editing.is_some() {
            self.editor_shared(ui, index);
            return;
        }
        let ctx = ui.ctx().clone();
        let document = self.editor.files[index].id;
        prepare_action(self.view, &self.editor.files[index], &ctx);
        let reveal_find = self
            .view
            .find
            .get(&document)
            .is_some_and(|find| find.reveal);
        let blocked = blocked(self.view);
        let replacing = self
            .view
            .find
            .get_mut(&document)
            .is_some_and(|find| prepare_find(find, &self.editor.files[index], &ctx, blocked));
        let clipboard_edit = if replacing {
            cancel_input(self.view);
            None
        } else {
            prepare_input(self.view, &self.editor.files[index], &ctx);
            self.view.text_clipboard.observe_input(&ctx);
            prepare_clipboard(self.view, &self.editor.files[index], &ctx)
        };
        let height = ui.available_height();
        ui.push_id(("editor_viewport", document), |ui| {
            Frame::new()
                .fill(BG)
                .corner_radius(4)
                .inner_margin(Margin::ZERO)
                .show(ui, |ui| {
                    // Reserve scrollbar hit regions instead of overlaying editable text.
                    ui.spacing_mut().scroll.floating = false;
                    let path = self.editor.files[index].path.as_deref();
                    let font_size = self.view.settings.number("editor.font_size", path);
                    let line_height = self.view.settings.number("editor.line_height", path);
                    let font = if self.view.settings.font_family(path) == "Hack" {
                        FontId::new(font_size, egui::FontFamily::Name("lumen-hack".into()))
                    } else {
                        FontId::monospace(font_size)
                    };
                    let line_count = self.editor.files[index].text.split('\n').count();
                    let number_width = ui.fonts_mut(|fonts| {
                        fonts
                            .layout_no_wrap(line_count.to_string(), font.clone(), MUTED)
                            .size()
                            .x
                    });
                    let gutter_width = (number_width + 24.0).max(48.0);
                    let viewport_origin = ui.cursor().min;
                    let diff_hook = self.view.source_control.diff_scroll_hook;
                    let mut area = ScrollArea::both()
                        .id_salt(("code_editor", document))
                        // Drag scrolling is already time-based. Animating its
                        // per-frame deltas accumulates targets beyond EOF and
                        // delays reversal; keep normal reveal animation otherwise.
                        .animated(!ctx.is_being_dragged(super::editor_gutter::gutter_id(document)))
                        .auto_shrink([false, false])
                        .max_height(height);
                    if diff_hook
                        && let Some(line) = self.view.source_control.diff_scroll.take_pending(1)
                    {
                        area = area.vertical_scroll_offset(line * line_height.max(1.0));
                    }
                    let scrolled = area.show_viewport(ui, |ui, viewport| {
                        let gutter = egui::Rect::from_min_size(
                            viewport_origin,
                            egui::vec2(gutter_width, viewport.height()),
                        );
                        let clip = ui.clip_rect();
                        let text_clip = clip.intersect(egui::Rect::from_min_max(
                            egui::pos2(gutter.right(), clip.top()),
                            clip.max,
                        ));
                        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                            // Code lines do not wrap implicitly. Horizontal scrolling owns overflow.
                            ui.fonts_mut(|fonts| {
                                let mut job = egui::text::LayoutJob::simple(
                                    text.as_str().into(),
                                    font.clone(),
                                    TEXT,
                                    f32::INFINITY,
                                );
                                job.sections[0].format.line_height = Some(line_height);
                                fonts.layout_job(job)
                            })
                        };
                        let output = ui
                            .horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.allocate_space(egui::vec2(gutter_width + 6.0, 0.0));
                                ui.set_clip_rect(text_clip);
                                let id = text_widget_id(document);
                                let edit = TextEdit::multiline(&mut self.editor.files[index].text)
                                    .id(id)
                                    .font(font.clone())
                                    .text_color(TEXT)
                                    .desired_width(f32::INFINITY)
                                    .min_size(egui::vec2(0.0, viewport.height()))
                                    .frame(false)
                                    .code_editor()
                                    .layouter(&mut layouter);
                                let mut output = super::editor_menu::show_editor_text(ui, edit, id);
                                if self.view.focus_editor {
                                    output.response.request_focus();
                                    self.view.focus_editor = false;
                                }
                                if output.response.changed() {
                                    self.editor.files[index].record_text_change();
                                }
                                if let Some((target, byte)) = self.view.pending_goto.take()
                                    && target == document
                                {
                                    let character =
                                        self.editor.files[index].text[..byte].chars().count();
                                    let cursor = egui::text::CCursor::new(character);
                                    let mut widget = output.state.clone();
                                    widget.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::one(cursor),
                                    ));
                                    widget.store(&ctx, text_widget_id(document));
                                    ui.scroll_to_rect(
                                        output
                                            .galley
                                            .pos_from_cursor(cursor)
                                            .translate(output.galley_pos.to_vec2()),
                                        None,
                                    );
                                    output.response.request_focus();
                                }
                                if let Some((target, range)) =
                                    self.view.pending_problem_selection.take()
                                    && target == document
                                {
                                    let start = self.editor.files[index].text[..range.start]
                                        .chars()
                                        .count();
                                    let end =
                                        self.editor.files[index].text[..range.end].chars().count();
                                    let mut widget = output.state.clone();
                                    widget.cursor.set_char_range(Some(
                                        egui::text::CCursorRange::two(
                                            egui::text::CCursor::new(start),
                                            egui::text::CCursor::new(end),
                                        ),
                                    ));
                                    widget.store(&ctx, text_widget_id(document));
                                    output.response.request_focus();
                                }
                                if replacing {
                                    finish_replace(&output, &self.editor.files[index], &ctx);
                                }
                                if let Some(edit) = &clipboard_edit {
                                    finish_clipboard(
                                        &mut output,
                                        &self.editor.files[index],
                                        &ctx,
                                        edit,
                                    );
                                }
                                finish_input(self.view, &self.editor.files[index], &ctx);
                                if let Some(find) =
                                    self.view.find.get_mut(&document).filter(|find| find.open)
                                {
                                    find.refresh(&self.editor.files[index]);
                                    super::editor_find::paint_find(ui, &output, find, reveal_find);
                                }
                                self.editor_context_menu(&output, document);
                                output
                            })
                            .inner;
                        ui.set_clip_rect(clip);
                        super::editor_gutter::show_gutter(
                            ui, &output, gutter, &font, document, !blocked,
                        );
                    });
                    if diff_hook {
                        self.view
                            .source_control
                            .diff_scroll
                            .observe(1, scrolled.state.offset.y / line_height.max(1.0));
                    }
                })
        });
    }
}
