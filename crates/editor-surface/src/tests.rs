use crate::{EditorSurface, SurfaceOutput};
use egui::{Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use lumen_editor_core::{EditorBuffer, Selection};

mod accessibility;
mod commands;
mod ime;
#[cfg(feature = "shaped")]
mod shaped;
mod typing;

fn frame(
    surface: &mut EditorSurface,
    buffer: &mut EditorBuffer,
    ctx: &Context,
    events: Vec<Event>,
    width: f32,
) -> SurfaceOutput {
    frame_with_cursor(surface, buffer, ctx, events, width).0
}

fn frame_with_cursor(
    surface: &mut EditorSurface,
    buffer: &mut EditorBuffer,
    ctx: &Context,
    events: Vec<Event>,
    width: f32,
) -> (SurfaceOutput, egui::CursorIcon) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 250.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(surface.show(ui, Id::new("test_surface"), buffer));
            });
        },
    );
    (result.unwrap(), output.platform_output.cursor_icon)
}

fn fixture(text: &str) -> (EditorSurface, EditorBuffer, Context) {
    let ctx = Context::default();
    let mut surface = EditorSurface::default();
    let mut buffer = EditorBuffer::new(text.into());
    frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    ctx.memory_mut(|memory| memory.request_focus(Id::new("test_surface")));
    frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    (surface, buffer, ctx)
}

#[test]
fn text_and_blank_editor_space_use_ibeam_without_claiming_gutter_or_scrollbars() {
    for width in [300.0, 900.0, 1400.0] {
        let (mut surface, mut buffer, ctx) = fixture("text");
        let output = frame(&mut surface, &mut buffer, &ctx, vec![], width);
        for point in [
            output.geometry.caret_rect(1).center(),
            output.text_clip.center(),
        ] {
            frame(
                &mut surface,
                &mut buffer,
                &ctx,
                vec![Event::PointerMoved(point)],
                width,
            );
            let (_, cursor) = frame_with_cursor(&mut surface, &mut buffer, &ctx, vec![], width);
            assert_eq!(cursor, egui::CursorIcon::Text);
        }
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![Event::PointerMoved(output.gutter.center())],
            width,
        );
        let (_, cursor) = frame_with_cursor(&mut surface, &mut buffer, &ctx, vec![], width);
        assert_ne!(cursor, egui::CursorIcon::Text);
        assert_eq!(buffer.revision(), 0);
    }
    let (mut surface, mut buffer, ctx) = fixture(&("long ".repeat(100) + &"\nrow".repeat(100)));
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 300.0);
    let scrollbar = pos2(output.text_clip.right() + 3.0, output.text_clip.center().y);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerMoved(scrollbar)],
        300.0,
    );
    let (_, cursor) = frame_with_cursor(&mut surface, &mut buffer, &ctx, vec![], 300.0);
    assert_ne!(cursor, egui::CursorIcon::Text);
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn disabled_editor_does_not_advertise_text_editing_cursor() {
    let (mut surface, mut buffer, ctx) = fixture("text");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let point = output.text_clip.center();
    for _ in 0..2 {
        let output = ctx.run(
            RawInput {
                events: vec![Event::PointerMoved(point)],
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 250.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(false, |ui| {
                        surface.show(ui, Id::new("test_surface"), &mut buffer);
                    });
                });
            },
        );
        assert_ne!(output.platform_output.cursor_icon, egui::CursorIcon::Text);
    }
}

#[test]
fn gutter_paints_only_visible_numbers_including_empty_rows() {
    let (mut surface, mut buffer, ctx) = fixture(&"\n".repeat(1_000));
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 250.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                surface.show(ui, Id::new("test_surface"), &mut buffer);
            });
        },
    );
    let numbers: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape {
                text.galley.text().parse::<usize>().ok()
            } else {
                None
            }
        })
        .collect();
    assert!(numbers.contains(&1));
    assert!(numbers.len() >= 10 && numbers.len() < 20, "{numbers:?}");
    assert!(!numbers.contains(&1_001));
    assert_eq!(buffer.revision(), 0);
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn click(pos: Pos2, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        },
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        },
    ]
}

#[test]
fn real_multi_location_input_and_undo_restore_the_selection_set() {
    let (mut surface, mut buffer, ctx) = fixture("one two");
    buffer
        .set_selections(
            vec![
                Selection { anchor: 0, head: 3 },
                Selection { anchor: 4, head: 7 },
            ],
            1,
        )
        .unwrap();
    let before = buffer.selections().clone();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("🌍".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "🌍 🌍");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Z, Modifiers::COMMAND)],
        500.0,
    );
    assert_eq!(buffer.text(), "one two");
    assert_eq!(buffer.selections(), &before);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)],
        500.0,
    );
    assert_eq!(buffer.text(), "🌍 🌍");
}

#[test]
fn alt_click_adds_a_real_cursor_then_typing_edits_both_locations() {
    let (mut surface, mut buffer, ctx) = fixture("abc\ndef");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let point = output.geometry.caret_rect(4).center();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(point, Modifiers::ALT),
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        [Selection::caret(0), Selection::caret(4)]
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("X".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "Xabc\nXdef");
}

#[test]
fn ordinary_click_collapses_cursors_and_blank_space_maps_to_eof() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(1)], 1)
        .unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let point = output.text_clip.center();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(point, Modifiers::NONE),
        500.0,
    );
    assert_eq!(buffer.selections().selections(), [Selection::caret(3)]);
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn scrollbar_hit_regions_do_not_move_the_caret() {
    let text = "long line ".repeat(80) + &"\nrow".repeat(80);
    let (mut surface, mut buffer, ctx) = fixture(&text);
    buffer.set_selections(vec![Selection::caret(4)], 0).unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 300.0);
    let point = pos2(output.text_clip.right() + 3.0, output.text_clip.center().y);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(point, Modifiers::NONE),
        300.0,
    );
    assert_eq!(buffer.selections().selections(), [Selection::caret(4)]);
}

#[test]
fn gutter_line_selection_uses_source_crlf_ranges_without_editing() {
    let (mut surface, mut buffer, ctx) = fixture("a\r\nb\r\nc");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let pos = pos2(
        output.gutter.center().x,
        output.geometry.caret_rect(3).center().y,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(pos, Modifiers::NONE),
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        &[Selection { anchor: 3, head: 6 }]
    );
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_dirty());
}

#[test]
fn right_click_keeps_selected_text_and_other_fields_keep_keyboard_input() {
    let (mut surface, mut buffer, ctx) = fixture("selected");
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 8 }], 0)
        .unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerButton {
            pos: output.geometry.caret_rect(4).center(),
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        [Selection { anchor: 0, head: 8 }]
    );
    ctx.memory_mut(|memory| memory.request_focus(Id::new("other_field")));
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Paste("wrong".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "selected");
}

#[test]
fn crlf_projection_and_unicode_boundaries_share_paint_and_hit_geometry() {
    let text = "é\r\ne\u{301}👩‍💻\n";
    let (mut surface, mut buffer, ctx) = fixture(text);
    for width in [180.0, 500.0, 1000.0] {
        let output = frame(&mut surface, &mut buffer, &ctx, vec![], width);
        assert_eq!(output.geometry.galley.rows.len(), 3);
        assert!(!output.geometry.galley.text().contains('\r'));
        for byte in [0, 2, 4, 7, 18, 19] {
            let point = output.geometry.caret_rect(byte).center();
            assert_eq!(output.geometry.hit_test(point), byte, "byte {byte}");
        }
        assert_eq!(buffer.text(), text);
    }
}

#[test]
fn keyboard_segments_keep_typing_paste_and_movement_order() {
    let (mut surface, mut buffer, ctx) = fixture("a");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::Text("X".into()),
            key(Key::ArrowRight, Modifiers::NONE),
            Event::Paste("é\r\n".into()),
            Event::Text("Y".into()),
        ],
        500.0,
    );
    assert_eq!(buffer.text(), "Xaé\r\nY");
    assert_eq!(buffer.revision(), 3);
}

#[test]
fn composition_preedit_is_transient_and_commit_is_one_multi_cursor_edit() {
    let (mut surface, mut buffer, ctx) = fixture("ab");
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(2)], 1)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::Ime(egui::ImeEvent::Enabled),
            Event::Ime(egui::ImeEvent::Preedit("漢".into())),
            Event::Text("漢".into()),
        ],
        500.0,
    );
    assert_eq!(buffer.text(), "ab");
    assert_eq!(buffer.revision(), 0);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::Commit("漢".into()))],
        500.0,
    );
    assert_eq!(buffer.text(), "漢ab漢");
    assert_eq!(buffer.revision(), 1);
    buffer.undo();
    assert_eq!(buffer.text(), "ab");
    assert_eq!(
        buffer.selections().selections(),
        [Selection::caret(0), Selection::caret(2)]
    );
}

#[test]
fn two_axis_scroll_keeps_gutter_fixed_and_pointer_mapping_on_painted_text() {
    let text = "long code ".repeat(70) + &"\nmore rows".repeat(60);
    let (mut surface, mut buffer, ctx) = fixture(&text);
    let before = frame(&mut surface, &mut buffer, &ctx, vec![], 300.0);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::PointerMoved(before.text_clip.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(-90.0, -70.0),
                modifiers: Modifiers::NONE,
            },
        ],
        300.0,
    );
    let after = frame(&mut surface, &mut buffer, &ctx, vec![], 300.0);
    assert!(after.geometry.origin.x < before.geometry.origin.x);
    assert!(after.geometry.origin.y < before.geometry.origin.y);
    assert_eq!(after.gutter.left(), before.gutter.left());
    assert_eq!(after.gutter.right(), before.gutter.right());
    let byte = after.geometry.hit_test(after.text_clip.center());
    let point = after.geometry.caret_rect(byte).center();
    assert!(after.text_clip.contains(point));
    assert_eq!(after.geometry.hit_test(point), byte);
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn shift_click_drag_and_release_keep_one_authoritative_selection() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef");
    buffer.set_selections(vec![Selection::caret(1)], 0).unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let point = output.geometry.caret_rect(4).center();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(point, Modifiers::SHIFT),
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        [Selection { anchor: 1, head: 4 }]
    );
    let start = output.geometry.caret_rect(2).center();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::PointerMoved(start),
            Event::PointerButton {
                pos: start,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        500.0,
    );
    let end = output.geometry.caret_rect(5).center();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerMoved(end)],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        [Selection { anchor: 2, head: 5 }]
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::PointerButton {
                pos: end,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
            Event::PointerMoved(start),
        ],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        [Selection { anchor: 2, head: 5 }]
    );
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn losing_native_focus_rejects_input_and_cancels_drag() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    let _ = ctx.run(
        RawInput {
            focused: false,
            events: vec![Event::Text("wrong".into())],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                surface.show(ui, Id::new("test_surface"), &mut buffer);
            });
        },
    );
    assert_eq!(buffer.text(), "abc");
    assert_eq!(buffer.revision(), 0);
    assert!(surface.drag_anchor.is_none());
}

#[test]
fn escape_removes_only_secondary_cursors_without_editing_text() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(2)], 1)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Escape, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), [Selection::caret(2)]);
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn text_revision_change_cancels_an_old_drag_without_reapplying_its_anchor() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let point = output.geometry.caret_rect(5).center();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::PointerMoved(point),
            Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
        500.0,
    );
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 6 }], 0)
        .unwrap();
    buffer.replace_selections(&["X"]).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerMoved(point)],
        500.0,
    );
    assert_eq!(buffer.text(), "X");
    assert_eq!(buffer.selections().selections(), [Selection::caret(1)]);
    assert!(surface.drag_anchor.is_none());
}

#[test]
fn layout_output_exposes_every_caret_at_the_same_positions_used_for_hit_tests() {
    let (mut surface, mut buffer, ctx) = fixture("abc\ndef");
    buffer
        .set_selections(vec![Selection::caret(1), Selection::caret(5)], 1)
        .unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(output.carets.len(), 2);
    for (byte, rect) in [1, 5].into_iter().zip(output.carets) {
        assert_eq!(output.geometry.caret_rect(byte), rect);
        assert_eq!(output.geometry.hit_test(rect.center()), byte);
    }
}
