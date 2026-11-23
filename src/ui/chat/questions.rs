//! The agent's questions, docked above the message box: one question at a
//! time with its options, an answer in your own words where the agent accepts
//! one, and Back / Next / Submit. Skipping tells the agent to carry on.
use super::{TEXT_SIZE, docked};
use crate::{
    icon,
    model::question_form::QuestionForm,
    ui::{theme, widgets},
};
use eframe::egui::{self, Frame, Key, Margin, RichText, Sense, Stroke};
use lumen_agent_core::{Question, QuestionRequest, TypedAnswer, api::QuestionReply};

/// Header, typed answer and footer rows, kept outside the scrolling part.
const CHROME_HEIGHT: f32 = 104.0;
const OPTION_GAP: f32 = 2.0;

/// Draw the card for `request`. Returns the reply once the user submits or
/// skips; until then the choices live in `form`.
pub(super) fn question_card(
    ui: &mut egui::Ui,
    request: &QuestionRequest,
    form: &mut QuestionForm,
    max_height: f32,
    colors: &theme::Palette,
) -> Option<QuestionReply> {
    let questions = request.questions.as_slice();
    let base = ui.id().with(("question_card", &request.token));
    // Shortcuts first, so a key is used before a focused option or button
    // could also take it as a click. Focus is matched against the card's
    // widgets as they were in the frame before.
    let holders_key = base.with("keyboard");
    let holders: Vec<egui::Id> = ui
        .data(|data| data.get_temp(holders_key))
        .unwrap_or_default();
    let mut reply = keys(ui, base, questions, form, &holders);
    let question = questions.get(form.step())?;
    let mut keyboard = KeyboardScope::default();
    docked::card(ui, colors.accent, colors, |ui| {
        ui.add_enabled_ui(!form.is_sent(), |ui| {
            if header(ui, question, form, questions.len(), colors) {
                reply = reply.take().or_else(|| form.skip());
            }
            let limit = (max_height - CHROME_HEIGHT).max(80.0);
            docked::scrolling(ui, base.with("body"), limit, |ui| {
                body(ui, base, questions, question, form, colors, &mut keyboard);
            });
            typed_answer(ui, base, questions, question, form, &mut keyboard);
            if let Some(footer_reply) = footer(ui, questions, form, colors, &mut keyboard) {
                reply = reply.take().or(Some(footer_reply));
            }
        });
    });
    let holders: Vec<egui::Id> = keyboard
        .options
        .iter()
        .chain(&keyboard.buttons)
        .copied()
        .collect();
    ui.data_mut(|data| data.insert_temp(holders_key, holders));
    arrive(ui, base, form, &keyboard);
    reply
}

/// Widgets of the card that take keys for it: its options and buttons, never
/// its text field, so typing elsewhere is left alone.
#[derive(Default)]
struct KeyboardScope {
    options: Vec<egui::Id>,
    buttons: Vec<egui::Id>,
    typed: Option<egui::Id>,
    /// The typed field gave up focus with Enter: move on.
    typed_enter: bool,
}

/// Icon, the question's short label, then the step and Skip at the right.
/// Returns whether Skip was pressed.
fn header(
    ui: &mut egui::Ui,
    question: &Question,
    form: &QuestionForm,
    count: usize,
    colors: &theme::Palette,
) -> bool {
    let mut skip = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = widgets::tool_button(ui, false, 0.0, |ui| {
                ui.add(
                    egui::Button::new(RichText::new(icon::X).size(12.0).color(colors.muted))
                        .frame(false)
                        .min_size(egui::vec2(20.0, 20.0)),
                )
            })
            .on_hover_text("Skip all questions (Esc): the agent carries on without your answers");
            skip = widgets::named(close, true, "Skip all questions").clicked();
            if count > 1 {
                ui.label(
                    RichText::new(format!("{}/{count}", form.step() + 1))
                        .size(theme::META_SIZE)
                        .color(colors.muted),
                );
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(icon::QUESTION)
                        .size(12.0)
                        .color(colors.accent),
                );
                let title = if question.header.trim().is_empty() {
                    "Question"
                } else {
                    question.header.as_str()
                };
                let font = egui::FontId::proportional(11.5);
                let fitted = widgets::ellipsize(ui, title, &font, ui.available_width());
                ui.label(
                    RichText::new(fitted)
                        .font(font)
                        .strong()
                        .color(colors.accent),
                )
                .on_hover_text(title);
            });
        });
    });
    skip
}

/// The question, its options and the typed answer.
fn body(
    ui: &mut egui::Ui,
    base: egui::Id,
    questions: &[Question],
    question: &Question,
    form: &mut QuestionForm,
    colors: &theme::Palette,
    keyboard: &mut KeyboardScope,
) {
    ui.add(
        egui::Label::new(
            RichText::new(&question.prompt)
                .size(TEXT_SIZE)
                .color(colors.text),
        )
        .selectable(true),
    );
    if question.multiple {
        ui.label(
            RichText::new("Choose any that apply")
                .size(theme::META_SIZE)
                .color(colors.muted),
        );
    }
    ui.spacing_mut().item_spacing.y = OPTION_GAP;
    let step = form.step();
    for (index, option) in question.options.iter().enumerate() {
        let id = base.with(("option", step, index));
        let chosen = form.is_chosen(step, &option.label);
        let response = option_row(ui, id, index, option, question.multiple, chosen, colors);
        keyboard.options.push(response.id);
        if response.clicked() {
            form.choose(questions, index);
            // Like a radio group: the clicked option keeps the keyboard.
            hold_focus(ui, response.id);
        }
    }
}

/// The answer in the user's own words, below the options and outside their
/// scrolling, so it is never hidden behind a long list.
fn typed_answer(
    ui: &mut egui::Ui,
    base: egui::Id,
    questions: &[Question],
    question: &Question,
    form: &mut QuestionForm,
    keyboard: &mut KeyboardScope,
) {
    let step = form.step();
    if question.typed != TypedAnswer::None {
        let hint = if question.options.is_empty() {
            "Type your answer"
        } else {
            "Or type your own answer"
        };
        let response = ui.add(
            egui::TextEdit::singleline(form.typed_mut())
                .id(base.with(("typed", step)))
                .hint_text(hint)
                .password(question.typed == TypedAnswer::Secret)
                .desired_width(f32::INFINITY),
        );
        if response.changed() {
            form.typed_changed(questions);
        }
        keyboard.typed = Some(response.id);
        keyboard.typed_enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    }
}

/// One option: its number, a radio dot or check box, the label and what it
/// means. The whole row is the click target and takes keyboard focus.
fn option_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    index: usize,
    option: &lumen_agent_core::QuestionOption,
    multiple: bool,
    chosen: bool,
    colors: &theme::Palette,
) -> egui::Response {
    // The row's fill goes into a slot reserved before its text.
    let slot = ui.painter().add(egui::Shape::Noop);
    let inner = Frame::default()
        .inner_margin(Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let number = if index < 9 {
                    format!("{}", index + 1)
                } else {
                    String::new()
                };
                ui.add_sized(
                    egui::vec2(10.0, 16.0),
                    egui::Label::new(
                        RichText::new(number)
                            .monospace()
                            .size(theme::META_SIZE)
                            .color(colors.muted),
                    ),
                );
                let (mark, _) = ui.allocate_exact_size(egui::vec2(14.0, 16.0), Sense::hover());
                paint_mark(ui, mark, multiple, chosen, colors);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(
                        RichText::new(&option.label)
                            .size(TEXT_SIZE - 0.5)
                            .color(colors.text),
                    );
                    if !option.description.trim().is_empty() {
                        ui.label(
                            RichText::new(&option.description)
                                .size(theme::META_SIZE + 0.5)
                                .color(colors.muted),
                        );
                    }
                });
            });
        });
    let rect = inner.response.rect;
    let response = ui.interact(rect, id, Sense::click());
    if response.has_focus() {
        hold_focus(ui, id);
    }
    let kind = if multiple {
        egui::WidgetType::Checkbox
    } else {
        egui::WidgetType::RadioButton
    };
    response.widget_info(|| egui::WidgetInfo::selected(kind, true, chosen, &option.label));
    let mut shapes = Vec::new();
    if chosen || response.hovered() {
        shapes.push(egui::Shape::rect_filled(rect, 3.0, colors.control_hover));
    }
    let keyboard_focus = response.has_focus() && !response.is_pointer_button_down_on();
    if chosen || keyboard_focus {
        shapes.push(egui::Shape::rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Inside,
        ));
    }
    ui.painter().set(slot, egui::Shape::Vec(shapes));
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A radio dot for one answer, a check box for several.
fn paint_mark(
    ui: &egui::Ui,
    rect: egui::Rect,
    multiple: bool,
    chosen: bool,
    colors: &theme::Palette,
) {
    let painter = ui.painter();
    let center = egui::pos2(rect.center().x, rect.top() + 8.0);
    let edge = if chosen {
        colors.accent
    } else {
        colors.checkbox_border
    };
    if multiple {
        let square = egui::Rect::from_center_size(center, egui::vec2(12.0, 12.0));
        if chosen {
            painter.rect_filled(square, 2.0, colors.accent);
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                icon::CHECK,
                egui::FontId::proportional(10.0),
                colors.on_accent,
            );
        } else {
            painter.rect_stroke(
                square,
                2.0,
                Stroke::new(1.0_f32, edge),
                egui::StrokeKind::Inside,
            );
        }
    } else {
        painter.circle_stroke(center, 5.5, Stroke::new(1.0_f32, edge));
        if chosen {
            painter.circle_filled(center, 3.0, colors.accent);
        }
    }
}

/// Back and Next, or Submit on the last question; "Sending…" once sent.
fn footer(
    ui: &mut egui::Ui,
    questions: &[Question],
    form: &mut QuestionForm,
    colors: &theme::Palette,
    keyboard: &mut KeyboardScope,
) -> Option<QuestionReply> {
    let mut reply = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if form.is_sent() {
                ui.label(
                    RichText::new("Sending…")
                        .size(theme::META_SIZE)
                        .color(colors.muted),
                );
                return;
            }
            let last = form.is_last();
            let (label, allowed) = if last {
                ("Submit", form.can_submit())
            } else {
                ("Next", form.can_advance())
            };
            let primary = ui
                .add_enabled(
                    allowed,
                    egui::Button::new(RichText::new(label).size(11.5).color(colors.on_accent))
                        .fill(colors.accent)
                        .min_size(egui::vec2(64.0, 0.0)),
                )
                .on_hover_text(if last {
                    "Send your answers to the agent (Ctrl+Enter)"
                } else {
                    "Go to the next question (Ctrl+Enter)"
                })
                .on_disabled_hover_text("Choose an option or type an answer first");
            keyboard.buttons.push(primary.id);
            if primary.clicked() || (keyboard.typed_enter && allowed) {
                if last {
                    reply = form.submit();
                } else {
                    form.next(questions);
                }
            }
            if form.step() > 0 {
                let back = ui
                    .add(egui::Button::new(
                        RichText::new("Back").size(11.5).color(colors.text),
                    ))
                    .on_hover_text("Go to the previous question");
                keyboard.buttons.push(back.id);
                if back.clicked() {
                    form.back(questions);
                }
            }
        });
    });
    reply
}

/// Keys for the card while one of its options or buttons has focus: 1-9 pick
/// an option, Up/Down move (and choose, for one answer), Ctrl/Cmd+Enter moves
/// on or submits, Escape skips every question.
fn keys(
    ui: &mut egui::Ui,
    base: egui::Id,
    questions: &[Question],
    form: &mut QuestionForm,
    holders: &[egui::Id],
) -> Option<QuestionReply> {
    let focused = ui.memory(|memory| memory.focused())?;
    if !holders.contains(&focused) || form.is_sent() {
        return None;
    }
    let step = form.step();
    let count = questions.get(step).map_or(0, |q| q.options.len());
    let pressed = |ui: &mut egui::Ui, key: Key| {
        ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key))
    };
    if pressed(ui, Key::Escape) {
        return form.skip();
    }
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, Key::Enter)) {
        if form.is_last() {
            return form.submit();
        }
        form.next(questions);
        focus_option(ui, base, form);
        return None;
    }
    const DIGITS: [Key; 9] = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    for (index, key) in DIGITS.into_iter().enumerate().take(count) {
        if pressed(ui, key) {
            form.choose(questions, index);
            focus_option(ui, base, form);
            return None;
        }
    }
    let on_option = (0..count).position(|index| base.with(("option", step, index)) == focused);
    if let Some(index) = on_option {
        for (key, delta) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
            if pressed(ui, key) {
                // Move from the focused row, which is where the user is.
                form.point_at(index);
                form.move_cursor(questions, delta);
                focus_option(ui, base, form);
            }
        }
    }
    None
}

fn focus_option(ui: &egui::Ui, base: egui::Id, form: &QuestionForm) {
    hold_focus(ui, base.with(("option", form.step(), form.cursor())));
    ui.ctx().request_repaint();
}

/// Give option `id` the keyboard, keeping Up/Down for moving between options
/// (see `keys`) and Escape for skipping, instead of egui moving focus or
/// letting it go. Focus is only requested when it moves: a new request would
/// reset the filter before the next frame reads its keys.
fn hold_focus(ui: &egui::Ui, id: egui::Id) {
    ui.memory_mut(|memory| {
        if !memory.has_focus(id) {
            memory.request_focus(id);
        }
        memory.set_focus_lock_filter(
            id,
            egui::EventFilter {
                vertical_arrows: true,
                escape: true,
                ..Default::default()
            },
        );
    });
}

/// When a card first shows and nothing is being typed, put the keyboard on
/// it, so the questions can be answered without reaching for the pointer.
fn arrive(ui: &egui::Ui, base: egui::Id, form: &QuestionForm, keyboard: &KeyboardScope) {
    let key = base.with("arrived");
    if ui.data(|data| data.get_temp::<bool>(key)).unwrap_or(false) {
        return;
    }
    ui.data_mut(|data| data.insert_temp(key, true));
    if ui.ctx().wants_keyboard_input() || form.is_sent() {
        return;
    }
    let target = keyboard
        .options
        .get(form.cursor())
        .copied()
        .or(keyboard.typed);
    if let Some(id) = target {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{
        ProfileId, ProviderId, QuestionOption, RequestToken, RuntimeGeneration, SessionId, TurnId,
    };

    fn request(options: usize) -> QuestionRequest {
        QuestionRequest {
            token: RequestToken {
                provider: ProviderId::Claude,
                profile: ProfileId::default_profile(),
                session: SessionId(1),
                turn: TurnId("turn".into()),
                generation: RuntimeGeneration(1),
                native: "q".into(),
            },
            questions: vec![Question {
                id: "q".into(),
                header: "A rather long header that will not fit".into(),
                prompt: "Which of these many choices should the agent go with for this change?"
                    .into(),
                options: (0..options)
                    .map(|n| QuestionOption {
                        label: format!("Option {n} with a long label"),
                        description: "What choosing it means, explained at some length so it wraps"
                            .into(),
                    })
                    .collect(),
                multiple: false,
                typed: TypedAnswer::Text,
            }],
        }
    }

    /// Text drawn and left visible by its clip, after the card has settled.
    fn visible_text(width: f32, max_height: f32, options: usize) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        let request = request(options);
        let mut form = QuestionForm::new(&request.questions);
        let mut output = None;
        for _ in 0..4 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 800.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let colors = theme::palette(ui.visuals());
                        question_card(ui, &request, &mut form, max_height, &colors);
                    });
                },
            ));
        }
        output
            .unwrap()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => {
                    let rect = text.galley.rect.translate(text.pos.to_vec2());
                    shape
                        .clip_rect
                        .contains_rect(rect)
                        .then(|| (text.galley.job.text.clone(), rect))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn many_options_scroll_inside_a_card_that_keeps_its_height() {
        let texts = visible_text(420.0, 260.0, 9);
        let top = texts
            .iter()
            .map(|(_, r)| r.top())
            .fold(f32::INFINITY, f32::min);
        let next = texts
            .iter()
            .find(|(text, _)| text == "Submit")
            .expect("the footer stays visible");
        assert!(
            next.1.bottom() - top <= 260.0,
            "the card stays within its height: {}",
            next.1.bottom() - top
        );
        assert!(
            texts.iter().any(|(text, _)| text.starts_with("Option 0")),
            "the first option shows"
        );
        assert!(
            !texts.iter().any(|(text, _)| text.starts_with("Option 8")),
            "later options scroll"
        );
    }

    #[test]
    fn a_short_question_takes_only_the_height_it_needs_and_wraps_in_a_narrow_panel() {
        let texts = visible_text(240.0, 420.0, 2);
        for (text, rect) in &texts {
            assert!(rect.right() <= 240.0, "{text:?} ends at {}", rect.right());
        }
        for needle in ["Option 1 with a long label", "Submit"] {
            assert!(
                texts.iter().any(|(text, _)| text == needle),
                "{needle} is visible"
            );
        }
        let top = texts
            .iter()
            .map(|(_, r)| r.top())
            .fold(f32::INFINITY, f32::min);
        let bottom = texts.iter().map(|(_, r)| r.bottom()).fold(0.0, f32::max);
        assert!(bottom - top < 420.0, "no empty space below a short card");
    }
}
