use super::{
    TestDialogs, configure_ui,
    editor::{editor_frame, editor_frame_at_width, pointer},
    test_app,
};
use crate::app::{LumenApp, editor_action::text_widget_id};
use eframe::egui;

fn fixture(text: &str) -> (LumenApp, egui::Context) {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    app.editor.files[0].text = text.into();
    app.editor.files[0].mark_saved("fixture.txt".into());
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

fn text_shape<'a>(output: &'a egui::FullOutput, text: &str) -> &'a egui::epaint::TextShape {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(shape) if shape.galley.text() == text => Some(shape),
            _ => None,
        })
        .expect("text shape is rendered")
}

fn wheel(position: egui::Pos2, delta: egui::Vec2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(position),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

fn editor_clip(output: &egui::FullOutput, text: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(rendered) if rendered.galley.text() == text => {
                Some(shape.clip_rect)
            }
            _ => None,
        })
        .unwrap()
}

fn selection(app: &LumenApp, ctx: &egui::Context) -> egui::text::CCursorRange {
    egui::text_edit::TextEditState::load(ctx, text_widget_id(app.editor.active().unwrap().id))
        .unwrap()
        .cursor
        .char_range()
        .unwrap()
}

#[test]
fn gutter_drag_scrolls_with_stationary_pointer_and_stops_on_release() {
    let text = (1..=120)
        .map(|row| format!("row {row:03}\n"))
        .collect::<String>();
    let (mut app, ctx) = fixture(&text);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let origin = text_shape(&output, &text).pos;
    let first = super::editor::text_position(&output, "1");
    let outside = egui::pos2(first.x, editor_clip(&output, &text).bottom() + 80.0);
    editor_frame(
        &mut app,
        &ctx,
        pointer(first, egui::PointerButton::Primary, true),
    );
    editor_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(outside)]);
    let initial = selection(&app, &ctx).primary.index;
    let mut output = editor_frame(&mut app, &ctx, vec![]);
    for _ in 0..20 {
        output = editor_frame(&mut app, &ctx, vec![]);
    }
    let current = text_shape(&output, &text).pos;
    assert!(current.y < origin.y - 100.0);
    assert_eq!(current.x, origin.x);
    let range = selection(&app, &ctx);
    assert_eq!(range.secondary.index, 0);
    assert!(range.primary.index > initial);
    assert!(range.primary.index <= text.chars().count());
    editor_frame(
        &mut app,
        &ctx,
        pointer(outside, egui::PointerButton::Primary, false),
    );
    let stopped = editor_frame(&mut app, &ctx, vec![]);
    let after = editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(
        text_shape(&stopped, &text).pos,
        text_shape(&after, &text).pos
    );
    assert_eq!(selection(&app, &ctx), range);
    assert_eq!(app.editor.files[0].text, text);
    assert!(!app.editor.files[0].dirty);
    assert_eq!(app.editor.files[0].revision, 0);
}

#[test]
fn gutter_drag_reverses_scroll_and_clamps_at_both_document_edges() {
    let text = (1..=90)
        .map(|row| format!("é 🌍 {row:03}\r\n"))
        .collect::<String>();
    let (mut app, ctx) = fixture(&text);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let first = super::editor::text_position(&output, "1");
    let top_y = text_shape(&output, &text).pos.y;
    let bounds = editor_clip(&output, &text);
    let below = egui::pos2(first.x, bounds.bottom() + 200.0);
    let above = egui::pos2(first.x, bounds.top() - 200.0);
    editor_frame(
        &mut app,
        &ctx,
        pointer(first, egui::PointerButton::Primary, true),
    );
    editor_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(below)]);
    for _ in 0..180 {
        editor_frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(selection(&app, &ctx).primary.index, text.chars().count());
    let bottom = editor_frame(&mut app, &ctx, vec![]);
    let still_bottom = editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(
        text_shape(&bottom, &text).pos,
        text_shape(&still_bottom, &text).pos
    );
    editor_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(above)]);
    let reversing = editor_frame(&mut app, &ctx, vec![]);
    assert!(text_shape(&reversing, &text).pos.y > text_shape(&bottom, &text).pos.y);
    for _ in 0..180 {
        editor_frame(&mut app, &ctx, vec![]);
    }
    let top = editor_frame(&mut app, &ctx, vec![]);
    assert!(
        (text_shape(&top, &text).pos.y - top_y).abs() < 0.1,
        "top={} expected={top_y} selection={:?} pointer={:?} blocked={}",
        text_shape(&top, &text).pos.y,
        selection(&app, &ctx),
        ctx.input(|input| (input.pointer.primary_down(), input.pointer.interact_pos())),
        crate::app::editor_action::blocked(&app.view)
    );
    assert_eq!(selection(&app, &ctx).primary.index, 9);
    assert_eq!(selection(&app, &ctx).secondary.index, 0);
    assert_eq!(app.editor.files[0].text, text);
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn pointer_outside_viewport_without_a_gutter_drag_does_not_scroll() {
    let text = "row\n".repeat(100);
    let (mut app, ctx) = fixture(&text);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let origin = text_shape(&output, &text).pos;
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(egui::pos2(25.0, 700.0))],
    );
    for _ in 0..10 {
        let output = editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(text_shape(&output, &text).pos, origin);
    }
}

#[test]
fn blocking_dialog_cancels_gutter_auto_scroll_without_hiding_line_numbers() {
    let text = "row\n".repeat(100);
    let (mut app, ctx) = fixture(&text);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let first = super::editor::text_position(&output, "1");
    let outside = egui::pos2(first.x, editor_clip(&output, &text).bottom() + 80.0);
    editor_frame(
        &mut app,
        &ctx,
        pointer(first, egui::PointerButton::Primary, true),
    );
    editor_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(outside)]);
    editor_frame(&mut app, &ctx, vec![]);
    let range = selection(&app, &ctx);
    app.view.pending_quit = true;
    let blocked = editor_frame(&mut app, &ctx, vec![]);
    let position = text_shape(&blocked, &text).pos;
    assert!(blocked.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::epaint::Shape::Text(text) if text.galley.text().parse::<usize>().is_ok())));
    assert!(ctx.dragged_id().is_none());
    app.view.pending_quit = false;
    for _ in 0..5 {
        let output = editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(text_shape(&output, &text).pos, position);
        assert_eq!(selection(&app, &ctx), range);
    }
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn gutter_and_text_share_vertical_geometry_and_scroll_from_either_region() {
    let text = (1..=120)
        .map(|number| format!("row {number:03}\n"))
        .collect::<String>();
    let (mut app, ctx) = fixture(&text);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let original_y = text_shape(&output, &text).pos.y;
    for position in [egui::pos2(25.0, 200.0), egui::pos2(200.0, 200.0)] {
        editor_frame(&mut app, &ctx, wheel(position, egui::vec2(0.0, -180.0)));
        let output = editor_frame(&mut app, &ctx, vec![]);
        let text_shape = text_shape(&output, &text);
        assert!(text_shape.pos.y < original_y - 50.0);
        for clipped in &output.shapes {
            if let egui::epaint::Shape::Text(number) = &clipped.shape
                && let Ok(line) = number.galley.text().parse::<usize>()
            {
                let row = &text_shape.galley.rows[line - 1];
                let expected = text_shape.pos.y + (row.min_y() + row.max_y()) * 0.5;
                let actual = number.pos.y + number.galley.size().y * 0.5;
                assert!((expected - actual).abs() < 1.0, "line {line} drifted");
            }
        }
    }
    assert_eq!(app.editor.files[0].text, text);
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn horizontal_scroll_keeps_gutter_fixed_and_long_lines_unwrapped() {
    let text = format!("{}\nsecond line", "long_code_".repeat(100));
    let (mut app, ctx) = fixture(&text);
    let output = editor_frame_at_width(&mut app, &ctx, vec![], 700.0);
    let original = text_shape(&output, &text);
    assert_eq!(original.galley.rows.len(), 2);
    let original_x = original.pos.x;
    let gutter_x = text_shape(&output, "1").pos.x;
    editor_frame_at_width(
        &mut app,
        &ctx,
        wheel(egui::pos2(200.0, 200.0), egui::vec2(-300.0, 0.0)),
        700.0,
    );
    let output = editor_frame_at_width(&mut app, &ctx, vec![], 700.0);
    assert!(text_shape(&output, &text).pos.x < original_x - 50.0);
    assert!((text_shape(&output, "1").pos.x - gutter_x).abs() < 0.1);
    assert_eq!(text_shape(&output, &text).galley.rows.len(), 2);
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn gutter_click_shift_click_and_drag_select_whole_unicode_lines() {
    let (mut app, ctx) = fixture("é 🌍\nsecond\nthird\n");
    let output = editor_frame(&mut app, &ctx, vec![]);
    let first = super::editor::text_position(&output, "1");
    let third = super::editor::text_position(&output, "3");
    let fourth = super::editor::text_position(&output, "4");
    let id = text_widget_id(app.editor.files[0].id);
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(first, egui::PointerButton::Primary, pressed),
        );
    }
    let range = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let [start, end] = range.sorted_cursors();
    assert_eq!((start.index, end.index), (0, 4));
    for pressed in [true, false] {
        let mut events = pointer(third, egui::PointerButton::Primary, pressed);
        if let egui::Event::PointerButton { modifiers, .. } = &mut events[1] {
            modifiers.shift = true;
        }
        editor_frame(&mut app, &ctx, events);
    }
    let range = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let [start, end] = range.sorted_cursors();
    assert_eq!((start.index, end.index), (0, 17));
    for pressed in [true, false] {
        let mut events = pointer(fourth, egui::PointerButton::Primary, pressed);
        if let egui::Event::PointerButton { modifiers, .. } = &mut events[1] {
            modifiers.shift = true;
        }
        editor_frame(&mut app, &ctx, events);
    }
    let range = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let [start, end] = range.sorted_cursors();
    assert_eq!((start.index, end.index), (0, 17));
    editor_frame(
        &mut app,
        &ctx,
        pointer(third, egui::PointerButton::Primary, true),
    );
    editor_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(first)]);
    editor_frame(
        &mut app,
        &ctx,
        pointer(first, egui::PointerButton::Primary, false),
    );
    let range = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let [start, end] = range.sorted_cursors();
    assert_eq!((start.index, end.index), (0, 17));
    for pressed in [true, false] {
        let mut events = pointer(fourth, egui::PointerButton::Primary, pressed);
        if let egui::Event::PointerButton { modifiers, .. } = &mut events[1] {
            modifiers.shift = true;
        }
        editor_frame(&mut app, &ctx, events);
    }
    let range = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let [start, end] = range.sorted_cursors();
    assert_eq!(
        (start.index, end.index),
        (11, 17),
        "reverse drag must retain its original line anchor"
    );
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn scrollbar_click_does_not_change_document_selection() {
    let text = "content\n".repeat(200);
    let (mut app, ctx) = fixture(&text);
    let id = text_widget_id(app.editor.files[0].id);
    super::editor::select(&app, &ctx, 1, 4);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let before = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let before_y = text_shape(&output, &text).pos.y;
    let position = egui::pos2(
        ctx.read_response(id).unwrap().interact_rect.right() + 3.0,
        300.0,
    );
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    let after = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    assert_eq!(after, before);
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(
        text_shape(&output, &text).pos.y < before_y,
        "scrollbar gesture must actually scroll"
    );
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn tabs_keep_independent_scroll_offsets_in_the_shared_viewport() {
    let text = "line\n".repeat(150);
    let (mut app, ctx) = fixture(&text);
    editor_frame(
        &mut app,
        &ctx,
        wheel(egui::pos2(200.0, 200.0), egui::vec2(0.0, -200.0)),
    );
    // Let egui's wheel smoothing settle before comparing persisted offsets.
    for _ in 0..4 {
        editor_frame(&mut app, &ctx, vec![]);
    }
    let output = editor_frame(&mut app, &ctx, vec![]);
    let first_y = text_shape(&output, &text).pos.y;
    app.new_file();
    app.editor.files[1].text = text.clone();
    app.editor.files[1].mark_saved("second.txt".into());
    editor_frame(&mut app, &ctx, vec![]);
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(text_shape(&output, &text).pos.y > first_y + 50.0);
    app.editor.active_file = Some(0);
    app.view.focus_editor = true;
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!((text_shape(&output, &text).pos.y - first_y).abs() < 1.0);
}

#[test]
fn gutter_selected_line_copy_preserves_crlf_and_unicode() {
    let (mut app, ctx) = fixture("é 🌍\r\nsecond\r\n");
    let output = editor_frame(&mut app, &ctx, vec![]);
    let position = super::editor::text_position(&output, "1");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    let id = app.editor.files[0].id;
    app.execute(
        crate::app::command::Command::EditorAction(
            id,
            crate::app::editor_action::EditorAction::Copy,
        ),
        &ctx,
    );
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "é 🌍\r\n")
    }));
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn find_reveals_a_match_offscreen_on_both_axes() {
    let text = format!("{}{}target", "short\n".repeat(119), "long_".repeat(300));
    let (mut app, ctx) = fixture(&text);
    app.execute(crate::app::command::Command::Find, &ctx);
    let id = app.editor.files[0].id;
    app.view.find.get_mut(&id).unwrap().query = "target".into();
    for _ in 0..8 {
        editor_frame(&mut app, &ctx, vec![]);
    }
    let output = editor_frame(&mut app, &ctx, vec![]);
    let shape = text_shape(&output, &text);
    let cursor = egui::text::CCursor::new(text.chars().count() - 6);
    let rect = shape
        .galley
        .pos_from_cursor(cursor)
        .translate(shape.pos.to_vec2());
    assert!(
        rect.left() > 50.0 && rect.right() < 1000.0,
        "match must be visible horizontally: {rect:?}"
    );
    assert!(
        rect.top() > 80.0 && rect.bottom() < 600.0,
        "match must be visible vertically: {rect:?}"
    );
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn go_to_line_moves_legacy_caret_without_changing_text() {
    let (mut app, ctx) = fixture("one\r\ntwo\r\nthree");
    let id = app.editor.active().unwrap().id;
    app.go_to_line(id, "2:2");
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(
        selection(&app, &ctx).primary.index,
        "one\r\nt".chars().count()
    );
    assert_eq!(app.editor.active().unwrap().text, "one\r\ntwo\r\nthree");
    assert!(!app.editor.active().unwrap().dirty);
}
