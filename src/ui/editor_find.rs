use super::{
    Workbench,
    theme::{ACCENT, MUTED, PANEL},
};
use crate::{
    app::find::{FindState, find_input_id},
    icon,
    model::DocumentId,
};
use eframe::egui::{self, Frame, Margin, Stroke, StrokeKind, TextEdit};

impl Workbench<'_> {
    pub(super) fn editor_find_bar(&mut self, ui: &mut egui::Ui, document: DocumentId) {
        let blocked = self.view.command_palette
            || self.view.pending_close.is_some()
            || self.view.pending_quit
            || self.view.delete_prompt.is_some()
            || self.view.explorer_edit.is_some();
        let Some(state) = self.view.find.get_mut(&document).filter(|state| state.open) else {
            return;
        };
        let file = self
            .editor
            .active()
            .expect("find belongs to active document");
        state.refresh(file);
        let focused = ui.memory(|memory| {
            memory.has_focus(find_input_id(document))
                || memory.has_focus(egui::Id::new(("editor_replace", document)))
        });
        if !blocked {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                state.open = false;
                state.reveal = true;
                self.view.focus_editor = true;
                return;
            }
            let previous = focused
                && ui
                    .input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter));
            let next = focused
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            if previous || next {
                state.navigate(previous);
            }
        }
        let width = ui.available_width().min(620.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - width).max(0.0));
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    Frame::new()
                        .fill(PANEL)
                        .inner_margin(Margin::same(4))
                        .corner_radius(3)
                        .show(ui, |ui| {
                            ui.set_width((width - 8.0).max(0.0));
                            ui.add_enabled_ui(!blocked, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .small_button(if state.replace_visible {
                                            icon::CARET_DOWN
                                        } else {
                                            icon::CARET_RIGHT
                                        })
                                        .on_hover_text("Toggle Replace")
                                        .clicked()
                                    {
                                        state.replace_visible = !state.replace_visible;
                                    }
                                    let mut input = TextEdit::singleline(&mut state.query)
                                        .id(find_input_id(document))
                                        .hint_text("Find")
                                        .desired_width((width - 340.0).max(40.0))
                                        .show(ui);
                                    if state.focus {
                                        input.response.request_focus();
                                        input.state.cursor.set_char_range(Some(
                                            egui::text::CCursorRange::two(
                                                egui::text::CCursor::new(0),
                                                egui::text::CCursor::new(
                                                    state.query.chars().count(),
                                                ),
                                            ),
                                        ));
                                        input.state.store(ui.ctx(), find_input_id(document));
                                        state.focus = false;
                                    }
                                    if input.response.changed() {
                                        state.current = 0;
                                        state.refresh(file);
                                    }
                                    let changed = ui
                                        .selectable_label(state.options.match_case, "Aa")
                                        .on_hover_text("Match Case")
                                        .clicked();
                                    if changed {
                                        state.options.match_case = !state.options.match_case;
                                    }
                                    if ui
                                        .selectable_label(state.options.whole_word, "ab")
                                        .on_hover_text("Match Whole Word")
                                        .clicked()
                                    {
                                        state.options.whole_word = !state.options.whole_word;
                                    }
                                    state.refresh(file);
                                    ui.label(
                                        egui::RichText::new(if state.matches.is_empty() {
                                            "No results".into()
                                        } else {
                                            format!(
                                                "{} of {}",
                                                state.current + 1,
                                                state.matches.len()
                                            )
                                        })
                                        .size(11.0)
                                        .color(MUTED),
                                    );
                                    let matches = !state.matches.is_empty();
                                    if ui
                                        .add_enabled(matches, egui::Button::new("Prev"))
                                        .on_hover_text("Previous Match (Shift+Enter)")
                                        .clicked()
                                    {
                                        state.navigate(true);
                                    }
                                    if ui
                                        .add_enabled(matches, egui::Button::new("Next"))
                                        .on_hover_text("Next Match (Enter)")
                                        .clicked()
                                    {
                                        state.navigate(false);
                                    }
                                    if ui
                                        .small_button(icon::X)
                                        .on_hover_text("Close Find (Escape)")
                                        .clicked()
                                    {
                                        state.open = false;
                                        state.reveal = true;
                                        self.view.focus_editor = true;
                                    }
                                });
                                if state.replace_visible {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.add_space(24.0);
                                        ui.add(
                                            TextEdit::singleline(&mut state.replacement)
                                                .id(egui::Id::new(("editor_replace", document)))
                                                .hint_text("Replace")
                                                .desired_width((width - 140.0).max(40.0)),
                                        );
                                        let matches = !state.matches.is_empty();
                                        if ui
                                            .add_enabled(matches, egui::Button::new("Replace"))
                                            .clicked()
                                        {
                                            state.queue_replace(file, false);
                                        }
                                        if ui
                                            .add_enabled(matches, egui::Button::new("All"))
                                            .on_hover_text("Replace All")
                                            .clicked()
                                        {
                                            state.queue_replace(file, true);
                                        }
                                    });
                                }
                            });
                        });
                },
            );
        });
    }
}

pub(super) fn paint_find(
    ui: &egui::Ui,
    output: &egui::text_edit::TextEditOutput,
    state: &FindState,
    reveal: bool,
) {
    if !state.open {
        return;
    }
    let painter = ui.painter_at(output.text_clip_rect.intersect(ui.clip_rect()));
    let mut row_start = 0;
    let rows: Vec<_> = output
        .galley
        .rows
        .iter()
        .map(|row| {
            let start = row_start;
            let end = start + row.char_count_excluding_newline();
            row_start += row.char_count_including_newline();
            (row, start, end)
        })
        .collect();
    for (index, range) in state.character_matches.iter().enumerate() {
        let first_row = rows.partition_point(|(_, _, end)| *end <= range.start);
        let mut revealed = false;
        for (row, row_start, row_end) in &rows[first_row..] {
            if *row_start >= range.end {
                break;
            }
            let left = range.start.max(*row_start);
            let right = range.end.min(*row_end);
            if left < right {
                let rect = egui::Rect::from_min_max(
                    egui::pos2(row.x_offset(left - *row_start), row.min_y()),
                    egui::pos2(row.x_offset(right - *row_start), row.max_y()),
                )
                .translate(output.galley_pos.to_vec2());
                if painter.clip_rect().intersects(rect) {
                    painter.rect_stroke(
                        rect,
                        1,
                        Stroke::new(
                            if index == state.current {
                                2.0_f32
                            } else {
                                1.0_f32
                            },
                            ACCENT,
                        ),
                        StrokeKind::Inside,
                    );
                }
                if reveal && index == state.current && !revealed {
                    ui.scroll_to_rect(rect, None);
                    revealed = true;
                }
            }
        }
    }
}
