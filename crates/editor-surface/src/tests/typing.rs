use super::*;

#[test]
fn text_events_across_frames_and_key_releases_share_one_undo_group() {
    let (mut surface, mut buffer, ctx) = fixture("");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::A, Modifiers::NONE), Event::Text("a".into())],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            Event::Text("b".into()),
        ],
        500.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Z, Modifiers::COMMAND)],
        500.0,
    );
    assert_eq!(buffer.text(), "");
    assert!(!buffer.can_undo());
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)],
        500.0,
    );
    assert_eq!(buffer.text(), "ab");
}

#[test]
fn copy_and_paste_split_typing_without_reordering_events() {
    let (mut surface, mut buffer, ctx) = fixture("");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::Text("a".into()),
            Event::Copy,
            Event::Text("b".into()),
            Event::Paste("P".into()),
            Event::Text("c".into()),
        ],
        500.0,
    );
    for expected in ["abP", "ab", "a", ""] {
        buffer.undo();
        assert_eq!(buffer.text(), expected);
    }
}

#[test]
fn focus_loss_splits_typing_even_without_a_selection_change() {
    let (mut surface, mut buffer, ctx) = fixture("");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("a".into())],
        500.0,
    );
    ctx.memory_mut(|memory| memory.surrender_focus(Id::new("test_surface")));
    frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    ctx.memory_mut(|memory| memory.request_focus(Id::new("test_surface")));
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("b".into())],
        500.0,
    );
    buffer.undo();
    assert_eq!(buffer.text(), "a");
}
