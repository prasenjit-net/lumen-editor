use crate::{EditorSurface, SurfaceOutput};
use egui::{Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
use lumen_editor_core::{EditorBuffer, Selection};

pub(crate) fn test_syntax_palette() -> crate::SyntaxPalette {
    crate::SyntaxPalette {
        comment: egui::Color32::GREEN,
        string: egui::Color32::YELLOW,
        number: egui::Color32::LIGHT_BLUE,
        keyword: egui::Color32::RED,
        type_name: egui::Color32::BLUE,
        function: egui::Color32::GOLD,
        property: egui::Color32::KHAKI,
        tag: egui::Color32::LIGHT_GREEN,
    }
}

#[test]
fn syntax_colors_keep_editor_geometry_and_document_unchanged() {
    let (mut surface, mut buffer, ctx) = fixture("// 🌍\r\nfn main() { let x = 42; }");
    let plain = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let plain_caret = plain.geometry.caret_rect(9);
    let revision = buffer.revision();
    surface.set_syntax(Some((
        lumen_editor_core::syntax::Language::Rust,
        test_syntax_palette(),
    )));
    let colored = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(colored.geometry.caret_rect(9), plain_caret);
    assert_eq!(buffer.revision(), revision);
    assert!(!buffer.is_dirty());
    let job = &colored.geometry.galley().job;
    assert!(
        job.sections
            .iter()
            .any(|section| &job.text[section.byte_range.clone()] == "fn"
                && section.format.color == egui::Color32::RED)
    );
    assert!(
        job.sections
            .iter()
            .any(|section| &job.text[section.byte_range.clone()] == "42"
                && section.format.color == egui::Color32::LIGHT_BLUE)
    );
    surface.set_syntax(None);
    let plain_again = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(plain_again.geometry.caret_rect(9), plain_caret);
    assert_eq!(plain_again.geometry.galley().job.sections.len(), 1);
}

#[test]
fn parser_tokens_color_go_without_changing_editor_geometry() {
    let (mut surface, mut buffer, ctx) =
        fixture("// note\r\npackage main\r\nfunc main() { println(\"ok\", 42) }");
    let plain = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let plain_caret = plain.geometry.caret_rect(9);
    let revision = buffer.revision();
    surface.set_syntax(Some((
        lumen_editor_core::syntax::Language::Go,
        test_syntax_palette(),
    )));
    let colored = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let job = &colored.geometry.galley().job;
    for (source, color) in [
        ("// note", egui::Color32::GREEN),
        ("package", egui::Color32::RED),
        ("\"ok\"", egui::Color32::YELLOW),
        ("42", egui::Color32::LIGHT_BLUE),
    ] {
        assert!(job.sections.iter().any(|section| {
            &job.text[section.byte_range.clone()] == source && section.format.color == color
        }));
    }
    assert_eq!(colored.geometry.caret_rect(9), plain_caret);
    assert_eq!(buffer.revision(), revision);
    assert!(!buffer.is_dirty());
}

#[test]
fn unsupported_parser_languages_keep_lexical_coloring() {
    // Python has no parser contribution yet, so the lexical highlighter colors it.
    let (mut surface, mut buffer, ctx) = fixture("def f(): pass");
    surface.set_syntax(Some((
        lumen_editor_core::syntax::Language::Python,
        test_syntax_palette(),
    )));
    let colored = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let job = &colored.geometry.galley().job;
    assert!(job.sections.iter().any(|section| {
        &job.text[section.byte_range.clone()] == "pass"
            && section.format.color == egui::Color32::RED
    }));
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.is_dirty());
}

#[test]
fn bracket_matching_uses_syntax_spans_after_crlf_projection() {
    let text = "fn main() { // }\r\n  let x = \"}\";\r\n}";
    let (mut surface, mut buffer, ctx) = fixture(text);
    surface.set_syntax(Some((
        lumen_editor_core::syntax::Language::Rust,
        test_syntax_palette(),
    )));
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let open = text.find('{').unwrap();
    let comment = text.find("// }").unwrap() + 3;
    let string = text.find("\"}\"").unwrap() + 1;
    assert!(output.geometry.is_code_byte(open));
    assert!(!output.geometry.is_code_byte(comment));
    assert!(!output.geometry.is_code_byte(string));
    assert_eq!(
        lumen_editor_core::matching_brackets_where(text, open, |byte| {
            output.geometry.is_code_byte(byte)
        }),
        Some((open..open + 1, text.len() - 1..text.len()))
    );
    assert_eq!(buffer.revision(), 0);
    surface.set_syntax(None);
    surface.set_language(lumen_editor_core::syntax::Language::Rust);
    let uncolored = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert!(!uncolored.geometry.is_code_byte(comment));
    assert!(!uncolored.geometry.is_code_byte(string));
}

mod accessibility;
mod commands;
mod folding;
mod ime;
#[cfg(feature = "shaped")]
mod shaped;
mod typing;

#[test]
fn typography_relayouts_geometry_without_mutating_document_or_history() {
    let (mut surface, mut buffer, ctx) = fixture("one\r\ntwo e\u{301} text");
    buffer.replace_selections(&["x"]).unwrap();
    let text = buffer.text().to_string();
    let selections = buffer.selections().clone();
    let revision = buffer.revision();
    let before = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert!(surface.set_typography(crate::Typography {
        font_size: 24.0,
        line_height: 32.0,
        bundled_font: false
    }));
    let after = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert!(after.geometry.size().y > before.geometry.size().y);
    assert_eq!(buffer.text(), text);
    assert_eq!(buffer.selections(), &selections);
    assert_eq!(buffer.revision(), revision);
    assert!(buffer.can_undo());
    assert!(!surface.set_typography(crate::Typography {
        font_size: f32::NAN,
        ..Default::default()
    }));
}

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
fn alt_drag_adds_a_range_and_clipboard_edits_every_selection_once() {
    let (mut surface, mut buffer, ctx) = fixture("alpha beta gamma");
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 5 }], 0)
        .unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let start = output.geometry.caret_rect(11).center();
    let end = output.geometry.caret_rect(16).center();
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
                modifiers: Modifiers::ALT,
            },
        ],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerMoved(end)],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::ALT,
        }],
        500.0,
    );
    let selections = buffer.selections().clone();
    assert_eq!(
        selections.selections(),
        &[
            Selection { anchor: 0, head: 5 },
            Selection {
                anchor: 11,
                head: 16
            }
        ]
    );
    frame(&mut surface, &mut buffer, &ctx, vec![Event::Copy], 500.0);
    assert_eq!(surface.clipboard.as_ref().unwrap().text(), "alpha\ngamma");
    assert!(!buffer.is_dirty());
    frame(&mut surface, &mut buffer, &ctx, vec![Event::Cut], 500.0);
    assert_eq!(buffer.text(), " beta ");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Paste("alpha\ngamma".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "alpha beta gamma");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), " beta ");
    assert!(buffer.undo());
    assert_eq!(buffer.selections(), &selections);
}

#[test]
fn shift_alt_drag_selects_columns_and_distributes_cut_copy_paste() {
    let (mut surface, mut buffer, ctx) = fixture("abc\nxy\npqr");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let start = output.geometry.caret_rect(1).center();
    let end = output.geometry.caret_rect(10).center();
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
                modifiers: Modifiers::ALT | Modifiers::SHIFT,
            },
        ],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerMoved(end)],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerButton {
            pos: end,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::ALT | Modifiers::SHIFT,
        }],
        500.0,
    );
    let selections = buffer.selections().clone();
    assert_eq!(
        selections.selections(),
        &[
            Selection { anchor: 1, head: 3 },
            Selection { anchor: 5, head: 6 },
            Selection {
                anchor: 8,
                head: 10
            },
        ]
    );
    assert_eq!(selections.primary_index(), 2);
    frame(&mut surface, &mut buffer, &ctx, vec![Event::Copy], 500.0);
    assert_eq!(surface.clipboard.as_ref().unwrap().text(), "bc\ny\nqr");
    frame(&mut surface, &mut buffer, &ctx, vec![Event::Cut], 500.0);
    assert_eq!(buffer.text(), "a\nx\np");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Paste("bc\ny\nqr".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "abc\nxy\npqr");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "a\nx\np");
    assert!(buffer.undo());
    assert_eq!(buffer.selections(), &selections);
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
        assert_eq!(output.geometry.galley().rows.len(), 3);
        assert!(!output.geometry.visual_text().contains('\r'));
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

#[test]
fn geometry_cache_retains_recent_tabs_and_rejects_replaced_buffers() {
    let ctx = Context::default();
    let mut surface = EditorSurface::default();
    let mut a = EditorBuffer::new("alpha\r\n🌍".into());
    let mut b = EditorBuffer::new("bravo\r\n🌍".into());
    let render = |surface: &mut EditorSurface, buffer: &mut EditorBuffer, id: u64| {
        let mut result = None;
        let _ = ctx.run(RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(surface.show(ui, Id::new(id), buffer));
            });
        });
        result.unwrap().geometry
    };
    let first = render(&mut surface, &mut a, 1);
    render(&mut surface, &mut b, 2);
    let back = render(&mut surface, &mut a, 1);
    assert!(std::sync::Arc::ptr_eq(first.galley(), back.galley()));
    assert_eq!(first.galley().text(), back.galley().text());
    // Same widget, length and revision, but a fresh buffer (e.g. disk reload).
    a = EditorBuffer::new("other\r\n🌍".into());
    let replaced = render(&mut surface, &mut a, 1);
    assert_eq!(replaced.galley().text(), "other\n🌍");
    assert!(!std::sync::Arc::ptr_eq(first.galley(), replaced.galley()));
    for id in 3..(crate::GEOMETRY_CACHE_LIMIT as u64 + 5) {
        render(&mut surface, &mut b, id);
    }
    assert_eq!(
        surface.geometry_cache.borrow().len(),
        crate::GEOMETRY_CACHE_LIMIT
    );
}
