use super::*;

fn ime(event: egui::ImeEvent) -> Event {
    Event::Ime(event)
}

#[test]
fn repeated_preedit_and_empty_commit_leave_history_and_saved_checkpoint_untouched() {
    let (mut surface, mut buffer, ctx) = fixture("selected");
    buffer
        .set_selections(vec![Selection { anchor: 0, head: 8 }], 0)
        .unwrap();
    let before = buffer.selections().clone();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Preedit("h".into())),
            ime(egui::ImeEvent::Preedit("漢".into())),
            ime(egui::ImeEvent::Preedit(String::new())),
        ],
        500.0,
    );
    assert_eq!(buffer.text(), "selected");
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
    assert_eq!(surface.composition.as_ref().unwrap().preedit(), "");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![ime(egui::ImeEvent::Commit(String::new()))],
        500.0,
    );
    assert!(!buffer.undo());
    assert_eq!(buffer.text(), "selected");
}

#[test]
fn pointer_caret_revision_and_focus_changes_reject_late_composition_commit() {
    for reason in ["pointer", "caret", "revision", "focus"] {
        let (mut surface, mut buffer, ctx) = fixture("abc");
        let output = frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![
                ime(egui::ImeEvent::Enabled),
                ime(egui::ImeEvent::Preedit("漢".into())),
            ],
            500.0,
        );
        match reason {
            "pointer" => {
                frame(
                    &mut surface,
                    &mut buffer,
                    &ctx,
                    click(output.geometry.caret_rect(0).center(), Modifiers::NONE),
                    500.0,
                );
            }
            "caret" => {
                buffer.set_selections(vec![Selection::caret(1)], 0).unwrap();
            }
            "revision" => {
                buffer.replace_document("different").unwrap();
                buffer.undo();
            }
            "focus" => {
                ctx.memory_mut(|memory| memory.request_focus(Id::new("other")));
            }
            _ => unreachable!(),
        }
        frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
        ctx.memory_mut(|memory| memory.request_focus(Id::new("test_surface")));
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![ime(egui::ImeEvent::Commit("wrong".into()))],
            500.0,
        );
        assert_eq!(buffer.text(), "abc", "{reason}");
        assert!(surface.composition.is_none());
    }
}

#[test]
fn duplicate_commit_is_ignored_and_a_fresh_session_can_commit_again() {
    let (mut surface, mut buffer, ctx) = fixture("ab");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Commit("漢".into())),
            ime(egui::ImeEvent::Commit("漢".into())),
        ],
        500.0,
    );
    assert_eq!(buffer.text(), "漢ab");
    assert_eq!(buffer.revision(), 1);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Commit("字".into())),
        ],
        500.0,
    );
    assert_eq!(buffer.text(), "漢字ab");
    assert_eq!(buffer.revision(), 2);
}

#[test]
fn escape_and_disable_cancel_preedit_and_dead_key_text_still_edits_normally() {
    for end in [
        key(Key::Escape, Modifiers::NONE),
        ime(egui::ImeEvent::Disabled),
    ] {
        let (mut surface, mut buffer, ctx) = fixture("ab");
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![
                ime(egui::ImeEvent::Enabled),
                ime(egui::ImeEvent::Preedit("漢".into())),
                end,
            ],
            500.0,
        );
        assert_eq!(buffer.text(), "ab");
        assert!(surface.composition.is_none());
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![Event::Text("é".into())],
            500.0,
        );
        assert_eq!(buffer.text(), "éab");
    }
}

#[test]
fn preedit_candidate_rect_uses_primary_geometry() {
    let (mut surface, mut buffer, ctx) = fixture("abc\ndef");
    buffer
        .set_selections(vec![Selection::caret(0), Selection::caret(4)], 1)
        .unwrap();
    let output = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Preedit("候補".into())),
        ],
        500.0,
    );
    let candidate = output.ime_cursor.unwrap();
    assert_eq!(candidate.top(), output.geometry.caret_rect(4).top());
    assert!(candidate.left() > output.geometry.caret_rect(4).left());
    assert_eq!(buffer.text(), "abc\ndef");
}

#[test]
fn changing_widget_identity_rejects_a_previous_widgets_commit() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Preedit("漢".into())),
        ],
        500.0,
    );
    ctx.memory_mut(|memory| memory.request_focus(Id::new("new_document")));
    let _ = ctx.run(
        RawInput {
            events: vec![ime(egui::ImeEvent::Commit("wrong".into()))],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                surface.show(ui, Id::new("new_document"), &mut buffer);
            });
        },
    );
    assert_eq!(buffer.text(), "abc");
    assert!(surface.composition.is_none());
}

#[test]
fn native_focus_loss_cancels_preedit_and_publishes_no_ime_output() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Preedit("漢".into())),
        ],
        500.0,
    );
    let output = ctx.run(
        RawInput {
            focused: false,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                surface.show(ui, Id::new("test_surface"), &mut buffer);
            });
        },
    );
    assert!(output.platform_output.ime.is_none());
    assert!(surface.composition.is_none());
    assert_eq!(buffer.text(), "abc");
}

#[test]
fn composition_candidate_moves_with_the_same_two_axis_scroll_as_source_text() {
    let text = "long code ".repeat(70) + &"\nrows".repeat(60);
    let (mut surface, mut buffer, ctx) = fixture(&text);
    buffer
        .set_selections(vec![Selection::caret(15)], 0)
        .unwrap();
    let before = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            ime(egui::ImeEvent::Enabled),
            ime(egui::ImeEvent::Preedit("preview".into())),
        ],
        300.0,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::PointerMoved(before.text_clip.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(-40.0, 0.0),
                modifiers: Modifiers::NONE,
            },
        ],
        300.0,
    );
    let after = frame(&mut surface, &mut buffer, &ctx, vec![], 300.0);
    let candidate_delta = after.ime_cursor.unwrap().left() - before.ime_cursor.unwrap().left();
    let source_delta = after.geometry.origin.x - before.geometry.origin.x;
    assert!((candidate_delta - source_delta).abs() < 0.01);
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn disabled_surface_rejects_keyboard_and_composition_edits() {
    let (mut surface, mut buffer, ctx) = fixture("abc");
    let output = ctx.run(
        RawInput {
            events: vec![
                Event::Text("wrong".into()),
                ime(egui::ImeEvent::Enabled),
                ime(egui::ImeEvent::Commit("wrong".into())),
            ],
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
    assert_eq!(buffer.text(), "abc");
    assert!(output.platform_output.ime.is_none());
    assert!(surface.composition.is_none());
}
