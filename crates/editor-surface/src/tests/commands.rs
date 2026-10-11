use super::*;

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
    let modifiers = if cfg!(target_os = "macos") {
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
    };
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
