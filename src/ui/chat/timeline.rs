//! Finding your way around a long chat, after VS Code's prompt timeline.
//!
//! - A quiet handle in the transcript's left gutter carries one dot per message
//!   you sent (evenly sampled when they do not all fit). The dot for the message
//!   you are reading is in the accent colour.
//! - Hovering, focusing or clicking the handle opens a list of those messages
//!   beside it; hovering a dot highlights its message, and choosing one scrolls
//!   to it. With the keyboard: Enter opens, Up/Down move, Enter jumps, Escape
//!   closes.
//! - When the message you are reading is scrolled off the top, a band pinned to
//!   the top of the transcript names it; clicking it scrolls back to it.
use crate::{
    app::agent_panel::ScrollRequest,
    ui::{theme, widgets},
};
use eframe::egui::{self, CornerRadius, FontId, Rect, Sense, Stroke};

/// Extra left gutter the transcript keeps free for the handle.
pub(super) const RESERVE: f32 = 20.0;
/// Narrower panels have no room for the handle beside the conversation.
pub(super) const MIN_WIDTH: f32 = 320.0;

const HANDLE_LEFT: f32 = 4.0;
const HANDLE_WIDTH: f32 = 16.0;
const PAD_Y: f32 = 8.0;
const DOT: f32 = 4.0;
const DOT_GAP: f32 = 4.0;
/// Space kept between the handle or list and the transcript's top and bottom.
const INSET: f32 = 12.0;
const LIST_WIDTH: f32 = 260.0;
const ROW: f32 = 24.0;
const BAND: f32 = 26.0;
/// A message whose top is this close to the viewport's top counts as there.
const SLACK: f32 = 6.0;

/// One message you sent: its transcript index, where it was drawn, and its text.
pub(super) struct Prompt<'a> {
    pub(super) index: usize,
    pub(super) top: f32,
    pub(super) text: &'a str,
}

/// Position (in `tops`) of the message being read: the last one that starts at
/// or above the viewport's top, or the first when the reader is above them all.
pub(super) fn current(tops: &[f32], viewport_top: f32) -> Option<usize> {
    if tops.is_empty() {
        return None;
    }
    Some(
        tops.iter()
            .rposition(|top| *top <= viewport_top + SLACK)
            .unwrap_or(0),
    )
}

/// Which messages get a dot when only `room` fit: every one, or `room` of them
/// spread evenly, always including the first and the last.
pub(super) fn sampled(count: usize, room: usize) -> Vec<usize> {
    if count <= room {
        return (0..count).collect();
    }
    let room = room.max(2);
    let mut dots: Vec<usize> = (0..room)
        .map(|i| (i * (count - 1) + (room - 1) / 2) / (room - 1))
        .collect();
    dots.dedup();
    dots
}

/// First line of a message, for one row of the list.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

/// Persistent state of one chat's timeline.
#[derive(Clone, Copy, Default)]
struct State {
    /// Opened by a click or the keyboard (hover alone shows it transiently).
    open: bool,
    /// Row chosen with the arrow keys, as a position in the prompt list.
    selected: Option<usize>,
    /// Where the list was drawn last frame, so moving into it keeps it open.
    list: Option<Rect>,
    /// The row the list last scrolled to. It scrolls only when this changes, so
    /// a row never slides out from under the pointer.
    scrolled_to: Option<usize>,
}

/// The gutter handle and its list. Returns a scroll to a chosen message.
pub(super) fn gutter(
    ui: &mut egui::Ui,
    viewport: Rect,
    prompts: &[Prompt],
    key: egui::Id,
    colors: &theme::Palette,
) -> Option<ScrollRequest> {
    if prompts.len() < 2 || viewport.width() < MIN_WIDTH {
        return None;
    }
    let state_key = key.with("state");
    let mut state: State = ui.data(|data| data.get_temp(state_key)).unwrap_or_default();
    let tops: Vec<f32> = prompts.iter().map(|prompt| prompt.top).collect();
    let active = current(&tops, viewport.top());
    let room = ((viewport.height() - 2.0 * INSET - 2.0 * PAD_Y + DOT_GAP) / (DOT + DOT_GAP))
        .floor()
        .max(2.0) as usize;
    let dots = sampled(prompts.len(), room);
    let nearest = |position: usize| (0..dots.len()).min_by_key(|&dot| dots[dot].abs_diff(position));
    let height = 2.0 * PAD_Y + dots.len() as f32 * DOT + (dots.len() - 1) as f32 * DOT_GAP;
    let handle = Rect::from_min_size(
        egui::pos2(
            viewport.left() + HANDLE_LEFT,
            viewport.center().y - height / 2.0,
        ),
        egui::vec2(HANDLE_WIDTH, height),
    );
    let dot_center = |dot: usize| {
        egui::pos2(
            handle.center().x,
            handle.top() + PAD_Y + DOT / 2.0 + dot as f32 * (DOT + DOT_GAP),
        )
    };
    let response = widgets::named(
        ui.interact(handle, key.with("handle"), Sense::click()),
        true,
        "Messages in this chat",
    );
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    // The list is its own foreground layer, so test the raw pointer position:
    // `rect_contains_pointer` would ask whether this layer is under the pointer.
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let over_list = state
        .list
        .zip(pointer)
        .is_some_and(|(list, pointer)| list.expand(2.0).contains(pointer));
    // The handle is tested the same way: while the list first lays itself out,
    // egui can take it to cover the handle, which would close it again.
    let over_handle = pointer.is_some_and(|pointer| handle.contains(pointer));
    // The dot under the pointer previews its message.
    let hovered_dot = pointer.filter(|_| over_handle).and_then(|pointer| {
        (0..dots.len()).min_by(|a, b| {
            (dot_center(*a).y - pointer.y)
                .abs()
                .total_cmp(&(dot_center(*b).y - pointer.y).abs())
        })
    });
    let focused = response.has_focus();
    let mut jump = None;

    if focused {
        // Arrow keys move through the list instead of moving focus.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                },
            );
        });
        let pressed =
            |key: egui::Key| ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        if pressed(egui::Key::Escape) {
            state.open = false;
            state.selected = None;
        }
        if pressed(egui::Key::ArrowDown) {
            state.open = true;
            let from = state.selected.or(active).unwrap_or(0);
            state.selected = Some((from + 1).min(prompts.len() - 1));
        }
        if pressed(egui::Key::ArrowUp) {
            state.open = true;
            let from = state.selected.or(active).unwrap_or(0);
            state.selected = Some(from.saturating_sub(1));
        }
        if pressed(egui::Key::Enter) || pressed(egui::Key::Space) {
            match (state.open, state.selected) {
                (true, Some(position)) => jump = Some(position),
                _ => {
                    state.open = !state.open;
                    state.selected = active;
                }
            }
        }
    }
    if response.clicked() {
        match hovered_dot {
            Some(dot) => jump = Some(dots[dot]),
            None => state.open = !state.open,
        }
    }
    // A click anywhere else closes a list that was opened on purpose.
    let clicked_elsewhere =
        ui.input(|input| input.pointer.any_click()) && !over_handle && !over_list;
    if clicked_elsewhere {
        state.open = false;
    }

    let revealed = state.open || over_handle || over_list || focused;
    let painter = ui.painter();
    if revealed {
        painter.rect_filled(handle, CornerRadius::same(3), colors.control_hover);
    }
    if focused {
        painter.rect_stroke(
            handle.expand(1.0),
            CornerRadius::same(3),
            Stroke::new(1.0_f32, colors.accent),
            egui::StrokeKind::Outside,
        );
    }
    let active_dot = active.and_then(nearest);
    for dot in 0..dots.len() {
        let is_active = active_dot == Some(dot);
        let is_preview = hovered_dot == Some(dot);
        let color = if is_active {
            colors.accent
        } else if revealed {
            colors.muted
        } else {
            colors.muted.gamma_multiply(0.55)
        };
        let radius = if is_active || is_preview {
            DOT * 0.75
        } else {
            DOT / 2.0
        };
        painter.circle_filled(dot_center(dot), radius, color);
    }

    let laid_out = state.list.is_some();
    state.list = None;
    if revealed {
        let highlight = hovered_dot.map(|dot| dots[dot]).or(state.selected);
        // Scroll the list to a newly highlighted row once the list has a layout.
        let scroll_to = highlight.filter(|_| laid_out && highlight != state.scrolled_to);
        if laid_out {
            state.scrolled_to = highlight;
        }
        let rows = Rows {
            prompts,
            active,
            highlight,
            scroll_to,
        };
        let (rect, chosen) = list(ui, viewport, handle, &rows, key, colors);
        state.list = Some(rect);
        jump = jump.or(chosen);
    } else {
        state.scrolled_to = None;
    }
    if jump.is_some() {
        // Choosing a message closes the list; it opens again from the handle.
        state.open = false;
        state.selected = None;
        state.list = None;
        state.scrolled_to = None;
    }
    ui.data_mut(|data| data.insert_temp(state_key, state));
    jump.map(|position| ScrollRequest::Item(prompts[position].index))
}

/// What the list shows: every message, which one is being read, which one is
/// highlighted, and which one to bring into view this frame.
struct Rows<'p, 'a> {
    prompts: &'p [Prompt<'a>],
    active: Option<usize>,
    highlight: Option<usize>,
    scroll_to: Option<usize>,
}

/// The list beside the handle: one row per message, the current one marked.
fn list(
    ui: &egui::Ui,
    viewport: Rect,
    handle: Rect,
    rows: &Rows,
    key: egui::Id,
    colors: &theme::Palette,
) -> (Rect, Option<usize>) {
    let Rows {
        prompts,
        active,
        highlight,
        scroll_to,
    } = *rows;
    let wanted = prompts.len() as f32 * ROW + 8.0;
    let height = wanted.min(viewport.height() - 2.0 * INSET);
    let top = (handle.center().y - height / 2.0)
        .clamp(viewport.top() + INSET, viewport.bottom() - INSET - height);
    let mut chosen = None;
    let area = egui::Area::new(key.with("list"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(handle.right(), top))
        .show(ui.ctx(), |ui| {
            widgets::floating_frame(ui.ctx())
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(LIST_WIDTH - 8.0);
                    egui::ScrollArea::vertical()
                        .id_salt(key.with("rows"))
                        .max_height(height - 8.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (position, prompt) in prompts.iter().enumerate() {
                                let response = row(
                                    ui,
                                    prompt,
                                    position + 1,
                                    active == Some(position),
                                    highlight == Some(position),
                                    colors,
                                );
                                if scroll_to == Some(position) {
                                    response.scroll_to_me(None);
                                }
                                if response.clicked() {
                                    chosen = Some(position);
                                }
                            }
                        });
                });
        });
    (area.response.rect, chosen)
}

fn row(
    ui: &mut egui::Ui,
    prompt: &Prompt,
    number: usize,
    active: bool,
    highlighted: bool,
    colors: &theme::Palette,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW - 2.0), Sense::click());
    if highlighted || response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), colors.control_hover);
    }
    let marker = egui::pos2(rect.left() + 9.0, rect.center().y);
    if active {
        ui.painter().circle_filled(marker, 3.0, colors.accent);
    } else {
        ui.painter()
            .circle_filled(marker, 2.0, colors.muted.gamma_multiply(0.7));
    }
    let text = first_line(prompt.text);
    let galley = widgets::fit_galley(
        ui,
        text,
        FontId::proportional(12.0),
        if active { colors.text } else { colors.muted },
        rect.width() - 24.0,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0),
        galley,
        colors.text,
    );
    let spoken = format!("Message {number}: {text}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, &spoken)
    });
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(prompt.text)
}

/// The band pinned to the top of the transcript naming the message being read,
/// shown once that message has scrolled off the top. `span` is the column's
/// horizontal extent.
pub(super) fn sticky(
    ui: &mut egui::Ui,
    viewport: Rect,
    span: egui::Rangef,
    prompts: &[Prompt],
    key: egui::Id,
    colors: &theme::Palette,
) -> Option<ScrollRequest> {
    let tops: Vec<f32> = prompts.iter().map(|prompt| prompt.top).collect();
    let position = current(&tops, viewport.top())?;
    let prompt = &prompts[position];
    if prompt.top >= viewport.top() - SLACK {
        return None;
    }
    let band = Rect::from_x_y_ranges(span, viewport.top()..=viewport.top() + BAND);
    let response = widgets::named(
        ui.interact(band, key.with("sticky"), Sense::click()),
        true,
        &format!("Scroll to your message: {}", first_line(prompt.text)),
    );
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect_filled(band, CornerRadius::ZERO, ui.visuals().panel_fill);
    if hovered {
        painter.rect_filled(band, CornerRadius::same(3), colors.control_hover);
    }
    painter.hline(
        band.x_range(),
        band.bottom(),
        Stroke::new(1.0_f32, colors.border),
    );
    let ink = if hovered { colors.text } else { colors.muted };
    painter.text(
        egui::pos2(band.left() + 4.0, band.center().y),
        egui::Align2::LEFT_CENTER,
        crate::icon::CARET_RIGHT,
        FontId::proportional(11.0),
        ink,
    );
    let galley = widgets::fit_galley(
        ui,
        first_line(prompt.text),
        FontId::proportional(12.0),
        ink,
        band.width() - 24.0,
    );
    painter.galley(
        egui::pos2(band.left() + 18.0, band.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Scroll to this message");
    response
        .clicked()
        .then_some(ScrollRequest::Item(prompt.index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_message_is_the_last_one_that_started_above_the_top() {
        let tops = [-400.0, -100.0, 100.0, 400.0];
        assert_eq!(current(&tops, 0.0), Some(1));
        assert_eq!(current(&tops, 104.0), Some(2), "within slack");
        assert_eq!(current(&tops, -500.0), Some(0), "above them all: the first");
        assert_eq!(current(&tops, 900.0), Some(3));
        assert_eq!(current(&[], 0.0), None);
    }

    #[test]
    fn dots_are_sampled_evenly_and_keep_the_first_and_last() {
        assert_eq!(sampled(3, 10), [0, 1, 2]);
        assert_eq!(sampled(10, 4), [0, 3, 6, 9]);
        let many = sampled(500, 30);
        assert_eq!(many.len(), 30);
        assert_eq!((many[0], many[29]), (0, 499));
        assert!(many.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn a_row_shows_the_first_line_that_has_text() {
        assert_eq!(
            first_line("\n  \nFix the build\nsecond line"),
            "Fix the build"
        );
        assert_eq!(first_line(""), "");
    }
    const TEXTS: [&str; 4] = [
        "First question about tables",
        "Make the preview full width",
        "Add a prompt timeline",
        "Fold the steps when done",
    ];

    fn prompts(tops: &[f32]) -> Vec<Prompt<'static>> {
        tops.iter()
            .enumerate()
            .map(|(n, top)| Prompt {
                index: n * 10,
                top: *top,
                text: TEXTS[n % TEXTS.len()],
            })
            .collect()
    }

    /// One frame of the gutter over a 400×500 viewport.
    fn frame(
        ctx: &egui::Context,
        tops: &[f32],
        width: f32,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<ScrollRequest>) {
        let mut request = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ctx, |ui| {
                        let viewport =
                            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 500.0));
                        let colors = theme::Theme::Dark.palette();
                        request = gutter(ui, viewport, &prompts(tops), egui::Id::new("t"), &colors);
                    });
            },
        );
        (output, request)
    }

    fn circles(output: &egui::FullOutput) -> Vec<egui::epaint::CircleShape> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) => Some(*circle),
                _ => None,
            })
            .collect()
    }

    fn texts(output: &egui::FullOutput) -> Vec<(String, Rect)> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.job.text.clone(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::configure_ui(&ctx);
        ctx
    }

    #[test]
    fn one_dot_per_message_and_the_current_one_is_in_the_accent() {
        let ctx = context();
        let tops = [-600.0, -200.0, 120.0, 700.0];
        let (output, request) = frame(&ctx, &tops, 400.0, vec![]);
        assert!(request.is_none());
        let dots = circles(&output);
        assert_eq!(dots.len(), 4);
        let accent = theme::Theme::Dark.palette().accent;
        let active: Vec<_> = dots.iter().filter(|dot| dot.fill == accent).collect();
        assert_eq!(active.len(), 1, "exactly one current message");
        assert_eq!(
            active[0].center.y, dots[1].center.y,
            "the second message is being read"
        );
        assert!(
            dots.iter().all(|dot| dot.center.x < RESERVE),
            "in the left gutter"
        );
        assert!(texts(&output).is_empty(), "the list stays hidden at rest");
    }

    #[test]
    fn nothing_is_shown_for_one_message_or_a_narrow_panel() {
        let ctx = context();
        let (output, _) = frame(&ctx, &[0.0], 400.0, vec![]);
        assert!(circles(&output).is_empty());
        let (output, _) = frame(&ctx, &[0.0, 100.0], MIN_WIDTH - 1.0, vec![]);
        assert!(circles(&output).is_empty());
    }

    #[test]
    fn hovering_the_handle_lists_messages_and_a_click_scrolls_to_one() {
        let ctx = context();
        let tops = [-600.0, -200.0, 120.0, 700.0];
        let (output, _) = frame(&ctx, &tops, 400.0, vec![]);
        let handle = circles(&output)[0].center;
        frame(&ctx, &tops, 400.0, vec![egui::Event::PointerMoved(handle)]);
        let (output, _) = frame(&ctx, &tops, 400.0, vec![]);
        let shown = texts(&output);
        for text in &TEXTS {
            assert!(
                shown.iter().any(|(t, _)| t == text),
                "{text} listed: {shown:?}"
            );
        }
        let target = shown
            .iter()
            .find(|(t, _)| t == "Add a prompt timeline")
            .unwrap()
            .1
            .center();
        // Move along the list, then click the row.
        frame(&ctx, &tops, 400.0, vec![egui::Event::PointerMoved(target)]);
        let press = |pressed| egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &tops, 400.0, vec![press(true)]);
        let (_, request) = frame(&ctx, &tops, 400.0, vec![press(false)]);
        assert_eq!(request, Some(ScrollRequest::Item(20)));
    }

    #[test]
    fn the_keyboard_opens_moves_and_jumps() {
        let ctx = context();
        let tops = [-600.0, -200.0, 120.0, 700.0];
        frame(&ctx, &tops, 400.0, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("t").with("handle")));
        frame(&ctx, &tops, 400.0, vec![]);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        // From the current message (the second), down once is the third.
        frame(&ctx, &tops, 400.0, vec![key(egui::Key::ArrowDown)]);
        let (output, _) = frame(&ctx, &tops, 400.0, vec![]);
        assert!(!texts(&output).is_empty(), "the list opened");
        let (_, request) = frame(&ctx, &tops, 400.0, vec![key(egui::Key::Enter)]);
        assert_eq!(request, Some(ScrollRequest::Item(20)));
        // Escape closes an open list without jumping.
        frame(&ctx, &tops, 400.0, vec![key(egui::Key::Enter)]);
        let (_, request) = frame(&ctx, &tops, 400.0, vec![key(egui::Key::Escape)]);
        assert!(request.is_none());
    }

    #[test]
    fn the_sticky_band_names_a_message_only_once_it_has_scrolled_off() {
        let ctx = context();
        let run = |tops: &[f32]| {
            let mut request = None;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            let viewport = Rect::from_min_size(
                                egui::pos2(0.0, 40.0),
                                egui::vec2(400.0, 460.0),
                            );
                            let colors = theme::Theme::Dark.palette();
                            request = sticky(
                                ui,
                                viewport,
                                egui::Rangef::new(28.0, 392.0),
                                &prompts(tops),
                                egui::Id::new("s"),
                                &colors,
                            );
                        });
                },
            );
            (output, request)
        };
        // The current message starts right at the top: no band.
        let (output, _) = run(&[40.0, 600.0]);
        assert!(texts(&output).is_empty());
        // It has scrolled above the top: the band names it, inside the column.
        let (output, _) = run(&[-300.0, 600.0]);
        let shown = texts(&output);
        let (_, rect) = shown
            .iter()
            .find(|(t, _)| t == TEXTS[0])
            .expect("band names the message");
        assert!(rect.left() >= 28.0 && rect.top() >= 40.0 && rect.bottom() <= 40.0 + BAND);
    }
}
