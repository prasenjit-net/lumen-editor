use super::{TestDialogs, configure_ui, test_app};
use crate::app::{command::Command, editor_action::EditorAction};
use crate::{app::LumenApp, model::DocumentId};
use eframe::egui;

fn text_id(document: DocumentId) -> egui::Id {
    egui::Id::new(("editor_text", document))
}

pub(super) fn editor_frame(
    app: &mut LumenApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    editor_frame_at_width(app, ctx, events, 1000.0)
}

pub(super) fn editor_frame_at_width(
    app: &mut LumenApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    width: f32,
) -> egui::FullOutput {
    // Let undo groups settle between text frames, but keep pointer gestures
    // within egui's click-duration threshold.
    let elapsed = if events
        .iter()
        .any(|event| matches!(event, egui::Event::PointerButton { .. }))
    {
        0.01
    } else {
        1.0
    };
    let time = ctx.input(|input| input.time) + elapsed;
    let modifiers = events
        .iter()
        .rev()
        .find_map(|event| match event {
            egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } => {
                Some(*modifiers)
            }
            _ => None,
        })
        .unwrap_or_default();
    ctx.run(
        egui::RawInput {
            time: Some(time),
            modifiers,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            app.handle_events(ctx);
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        },
    )
}

fn undo() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            mac_cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            ..Default::default()
        },
    }
}

#[test]
fn tabs_keep_independent_text_cursor_and_undo_state() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    let first = app.editor.active().unwrap().id;
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("alpha".into())]);
    editor_frame(&mut app, &ctx, vec![]);
    let mut state = egui::text_edit::TextEditState::load(&ctx, text_id(first)).unwrap();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(2),
        )));
    state.store(&ctx, text_id(first));

    app.new_file();
    let second = app.editor.active().unwrap().id;
    assert_ne!(first, second);
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("beta".into())]);
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![undo()]);
    assert_eq!(app.editor.files[0].text, "alpha");
    assert_eq!(app.editor.files[1].text, "");

    app.editor.set_active_index(Some(0));
    app.view.focus_editor = true;
    editor_frame(&mut app, &ctx, vec![]);
    let state = egui::text_edit::TextEditState::load(&ctx, text_id(first)).unwrap();
    assert_eq!(state.cursor.char_range().unwrap().primary.index, 2);
    editor_frame(&mut app, &ctx, vec![undo()]);
    // egui includes cursor moves in history: first restore the pre-move cursor,
    // then undo the text insertion. Both entries must belong to this document.
    assert_eq!(app.editor.files[0].text, "alpha");
    editor_frame(&mut app, &ctx, vec![undo()]);
    assert_eq!(app.editor.files[0].text, "");
    assert_eq!(app.editor.files[1].text, "");
}

#[test]
fn undo_to_saved_contents_clears_dirty_and_redirty_on_edit() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("saved".into())]);
    editor_frame(&mut app, &ctx, vec![]);
    app.editor.files[0].mark_saved("saved.txt".into());
    let revision = app.editor.files[0].revision;
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("!".into())]);
    editor_frame(&mut app, &ctx, vec![]);
    assert!(app.editor.files[0].dirty);
    editor_frame(&mut app, &ctx, vec![undo()]);
    assert_eq!(app.editor.files[0].text, "saved");
    assert!(!app.editor.files[0].dirty);
    assert!(app.editor.files[0].revision > revision);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text(" again".into())]);
    assert!(app.editor.files[0].dirty);
}

#[test]
fn document_identity_survives_rename_and_tab_index_changes() {
    let (mut app, _) = test_app(TestDialogs::default());
    app.new_file();
    app.new_file();
    let second = app.editor.files[1].id;
    app.editor.files[1].relocate("renamed.rs".into());
    assert_eq!(app.editor.files[1].id, second);
    app.close_file(app.editor.files[0].id);
    assert_eq!(app.editor.files[0].id, second);
    app.new_file();
    assert_ne!(app.editor.files[1].id, second);
}

#[test]
fn clicking_a_tab_transfers_keyboard_input_to_its_document() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    let output = editor_frame(&mut app, &ctx, vec![]);

    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text().contains("untitled-1") => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("first tab title is rendered");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(app.editor.active_index(), Some(0));
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("first".into())]);
    assert_eq!(app.editor.files[0].text, "first");
    assert_eq!(app.editor.files[1].text, "");
}

pub(super) fn select(app: &LumenApp, ctx: &egui::Context, start: usize, end: usize) {
    let id = text_id(app.editor.active().unwrap().id);
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(start),
            egui::text::CCursor::new(end),
        )));
    state.store(ctx, id);
}

#[test]
fn targeted_clipboard_and_undo_use_the_widget_history() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("héllo 🌍".into())]);
    editor_frame(&mut app, &ctx, vec![]);
    select(&app, &ctx, 0, 5);
    editor_frame(&mut app, &ctx, vec![]);
    let document = app.editor.active().unwrap().id;
    app.execute(Command::EditorAction(document, EditorAction::Copy), &ctx);
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "héllo")
    }));
    assert_eq!(app.editor.active().unwrap().text, "héllo 🌍");
    app.execute(Command::EditorAction(document, EditorAction::Cut), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.active().unwrap().text, " 🌍");
    app.execute(Command::EditorAction(document, EditorAction::Undo), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.active().unwrap().text, "héllo 🌍");
}

#[test]
fn queued_editor_action_cannot_edit_a_different_tab_or_modal() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("keep".into())]);
    select(&app, &ctx, 0, 4);
    let document = app.editor.active().unwrap().id;
    app.execute(Command::EditorAction(document, EditorAction::Cut), &ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "keep");
    assert_eq!(app.editor.files[1].text, "");
    assert!(app.view.pending_editor_action.is_none());
    app.editor.set_active_index(Some(0));
    app.execute(Command::EditorAction(document, EditorAction::Cut), &ctx);
    app.view.pending_close = Some(app.editor.files[0].id);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "keep");
}

#[test]
fn popup_paste_is_delivered_only_to_its_original_focused_document() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    let document = app.editor.active().unwrap().id;
    app.execute(Command::EditorAction(document, EditorAction::Paste), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste("first".into())]);
    assert_eq!(app.editor.files[0].text, "first");
    app.execute(Command::EditorAction(document, EditorAction::Paste), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste("wrong tab".into())]);
    assert_eq!(app.editor.files[0].text, "first");
    assert_eq!(app.editor.files[1].text, "");
    assert!(app.view.editor_paste_target.is_none());

    let second = app.editor.active().unwrap().id;
    app.execute(Command::EditorAction(second, EditorAction::Paste), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    app.view.command_palette = true;
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("modal must not receive this".into())],
    );
    assert_eq!(app.editor.files[1].text, "");
    assert!(!ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
    }));
}

pub(super) fn text_position(output: &egui::FullOutput, title: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == title => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("requested text is rendered")
}

pub(super) fn pointer(
    position: egui::Pos2,
    button: egui::PointerButton,
    pressed: bool,
) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(position),
        egui::Event::PointerButton {
            pos: position,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

#[test]
fn context_menu_select_all_reaches_the_editor_after_popup_closes() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    let output = editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("menu fixture".into())],
    );
    let position = text_position(&output, "menu fixture");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Secondary, pressed),
        );
    }
    let output = editor_frame(&mut app, &ctx, vec![]);
    let position = text_position(&output, "Select All");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("replacement".into())],
    );
    assert_eq!(app.editor.active().unwrap().text, "replacement");
}

#[test]
fn blank_editor_space_accepts_clicks_and_opens_the_context_menu() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("first line".into())]);
    select(&app, &ctx, 0, 0);
    let position = egui::pos2(900.0, 500.0);
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    editor_frame(&mut app, &ctx, vec![egui::Event::Text(" appended".into())]);
    assert_eq!(app.editor.active().unwrap().text, "first line appended");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Secondary, pressed),
        );
    }
    let output = editor_frame(&mut app, &ctx, vec![]);
    text_position(&output, "Select All");
}

#[test]
fn right_click_preserves_selection_and_context_copy_uses_it() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("héllo 🌍 and more".into())],
    );
    select(&app, &ctx, 7, 0);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let id = text_id(app.editor.active().unwrap().id);
    let original = egui::text_edit::TextEditState::load(&ctx, id)
        .unwrap()
        .cursor
        .char_range()
        .unwrap();
    let position = text_position(&output, "héllo 🌍 and more");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Secondary, pressed),
        );
        let current = egui::text_edit::TextEditState::load(&ctx, id)
            .unwrap()
            .cursor
            .char_range()
            .unwrap();
        assert_eq!(
            current, original,
            "right-click must preserve selection on press and release"
        );
    }
    let output = editor_frame(&mut app, &ctx, vec![]);
    let position = text_position(&output, "Copy");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "héllo 🌍")
    }));
    assert_eq!(app.editor.active().unwrap().text, "héllo 🌍 and more");
    let position = text_position(&output, "héllo 🌍 and more");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    assert!(
        egui::text_edit::TextEditState::load(&ctx, id)
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .is_empty(),
        "ordinary left-click must still move the caret"
    );
}
