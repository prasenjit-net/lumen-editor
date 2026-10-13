use super::*;

#[test]
fn configured_enter_and_tab_keep_multicursor_edits_atomic() {
    let (mut surface, mut buffer, ctx) = fixture("  one {\r\n  two {");
    surface.set_indentation(lumen_editor_core::Indentation {
        width: 2,
        insert_spaces: true,
        detect: false,
        auto_indent: true,
    });
    buffer
        .set_selections(
            vec![
                Selection::caret("  one {".len()),
                Selection::caret(buffer.text().len()),
            ],
            1,
        )
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Enter, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.text(), "  one {\r\n    \r\n  two {\r\n    ");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "  one {\r\n  two {");
    surface.set_indentation(lumen_editor_core::Indentation {
        width: 2,
        insert_spaces: false,
        detect: false,
        auto_indent: false,
    });
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Tab, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.text(), "  one {\t\r\n  two {\t");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "  one {\r\n  two {");
}

#[test]
fn selected_lines_gain_end_cursors_and_type_as_one_edit() {
    let (mut surface, mut buffer, ctx) = fixture("one\ntwo\nthree");
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 8 }], 0)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            key(Key::I, Modifiers::SHIFT | Modifiers::ALT),
            Event::Text("ˆ".into()),
        ],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        &[Selection::caret(3), Selection::caret(7)]
    );
    assert_eq!(buffer.text(), "one\ntwo\nthree");
    let before = buffer.selections().clone();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("!".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "one!\ntwo!\nthree");
    assert!(buffer.undo());
    assert_eq!(buffer.selections(), &before);
}

#[test]
fn line_end_shortcut_ignores_delayed_or_earlier_dead_key_text() {
    for events in [
        vec![
            key(Key::I, Modifiers::SHIFT | Modifiers::ALT),
            Event::Text("ˆ".into()),
        ],
        vec![
            Event::Text("ˆ".into()),
            key(Key::I, Modifiers::SHIFT | Modifiers::ALT),
        ],
    ] {
        let (mut surface, mut buffer, ctx) = fixture("a\nb");
        buffer
            .set_selections(vec![Selection { anchor: 0, head: 3 }], 0)
            .unwrap();
        frame(&mut surface, &mut buffer, &ctx, events, 500.0);
        assert_eq!(buffer.text(), "a\nb");
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![Event::Text("!".into())],
            500.0,
        );
        assert_eq!(buffer.text(), "a!\nb!");
    }

    let (mut surface, mut buffer, ctx) = fixture("a\nb");
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 3 }], 0)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::I, Modifiers::SHIFT | Modifiers::ALT)],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("ˆ".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "a\nb");
}

#[test]
fn every_cursor_retains_column_through_short_lines_and_passive_events() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef\nx\nabcdef\nabcdef\nx\nabcdef");
    buffer
        .set_selections(vec![Selection::caret(3), Selection::caret(19)], 1)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        &[Selection::caret(8), Selection::caret(24)]
    );
    let release = Event::Key {
        key: Key::ArrowDown,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![release, Event::PointerMoved(pos2(490.0, 240.0))],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        &[Selection::caret(12), Selection::caret(28)]
    );
    assert_eq!(buffer.selections().primary_index(), 1);
    assert!(!buffer.is_dirty());
    assert!(!buffer.can_undo());
}

#[test]
fn vertical_selection_extension_preserves_each_anchor_and_column() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef\nx\nabcdef\nabcdef\nx\nabcdef");
    buffer
        .set_selections(vec![Selection::caret(3), Selection::caret(19)], 0)
        .unwrap();
    for _ in 0..2 {
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![key(Key::ArrowDown, Modifiers::SHIFT)],
            500.0,
        );
    }
    assert_eq!(
        buffer.selections().selections(),
        &[
            Selection {
                anchor: 3,
                head: 12
            },
            Selection {
                anchor: 19,
                head: 28
            }
        ]
    );
    for _ in 0..2 {
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![key(Key::ArrowUp, Modifiers::SHIFT)],
            500.0,
        );
    }
    assert_eq!(
        buffer.selections().selections(),
        &[Selection::caret(3), Selection::caret(19)]
    );
}

#[test]
fn external_selection_and_revision_changes_invalidate_columns() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef\nx\nabcdef");
    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    buffer.set_selections(vec![Selection::caret(7)], 0).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(9)]);

    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    buffer.replace_document("abcdef\ny\nabcdef").unwrap();
    buffer.set_selections(vec![Selection::caret(8)], 0).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(10)]);
}

#[test]
fn adding_cursor_above_remaps_columns_after_document_order_normalization() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef\nx\nabcdef");
    buffer
        .set_selections(vec![Selection::caret(12)], 0)
        .unwrap();
    let modifiers = cursor_modifiers();
    for _ in 0..2 {
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![key(Key::ArrowUp, modifiers)],
            500.0,
        );
    }
    assert_eq!(
        buffer.selections().selections(),
        &[
            Selection::caret(3),
            Selection::caret(8),
            Selection::caret(12)
        ]
    );
    assert_eq!(buffer.selections().primary_index(), 0);
}

fn cursor_modifiers() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers {
            alt: true,
            mac_cmd: true,
            ..Modifiers::COMMAND
        }
    } else if cfg!(target_os = "windows") {
        Modifiers {
            alt: true,
            ctrl: true,
            ..Modifiers::NONE
        }
    } else {
        Modifiers {
            alt: true,
            shift: true,
            ..Modifiers::NONE
        }
    }
}

#[test]
fn merged_cursors_reset_columns_without_reusing_stale_indices() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef\nx\nabcdef");
    buffer
        .set_selections(vec![Selection::caret(2), Selection::caret(4)], 1)
        .unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(8)]);
    assert!(surface.vertical_goals.is_none());
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(10)]);
}

#[test]
fn horizontal_movement_starts_a_new_preferred_column_sequence() {
    let (mut surface, mut buffer, ctx) = fixture("abcdef\nx\nabcdef");
    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowLeft, Modifiers::NONE)],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(9)]);
}

#[test]
fn occurrence_shortcuts_edit_all_locations_and_restore_set_on_undo() {
    let (mut surface, mut buffer, ctx) = fixture("name name name");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::D, Modifiers::COMMAND)],
        500.0,
    );
    let all = Modifiers {
        shift: true,
        ..Modifiers::COMMAND
    };
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::L, all)],
        500.0,
    );
    assert_eq!(buffer.selections().selections().len(), 3);
    let before = buffer.selections().clone();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("other".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "other other other");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Z, Modifiers::COMMAND)],
        500.0,
    );
    assert_eq!(buffer.selections(), &before);
}

#[test]
fn native_cursor_shortcut_uses_adjacent_row_geometry_and_stops_at_edges() {
    let (mut surface, mut buffer, ctx) = fixture("abcd\nx\n漢xyz");
    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    let modifiers = cursor_modifiers();
    let first = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, modifiers)],
        500.0,
    );
    let expected = first.geometry.hit_test(Pos2::new(
        first.geometry.caret_rect(3).left(),
        first.geometry.caret_rect(7).center().y,
    ));
    assert_eq!(
        buffer.selections().selections(),
        &[Selection::caret(3), Selection::caret(6)]
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, modifiers)],
        500.0,
    );
    assert_eq!(buffer.selections().selections().len(), 3);
    let set = buffer.selections();
    assert_eq!(set.selections()[set.primary_index()].head, expected);
    let before = buffer.selections().clone();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, modifiers)],
        500.0,
    );
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
}

#[test]
fn copy_paste_distributes_only_recorded_fragments_and_cut_is_one_undo_step() {
    let (mut surface, mut buffer, ctx) = fixture("a b");
    buffer
        .set_selections(
            vec![
                Selection { anchor: 0, head: 1 },
                Selection { anchor: 2, head: 3 },
            ],
            0,
        )
        .unwrap();
    frame(&mut surface, &mut buffer, &ctx, vec![Event::Copy], 500.0);
    assert_eq!(surface.clipboard.as_ref().unwrap().text(), "a\nb");
    assert!(!buffer.is_dirty());
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Cut, Event::Paste("a\nb".into())],
        500.0,
    );
    assert_eq!(buffer.text(), "a b");
    buffer.undo();
    assert_eq!(buffer.text(), " ");
    buffer.undo();
    assert_eq!(buffer.text(), "a b");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Tab, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.text(), "    a b"); // Same logical line is indented once.
}
