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
