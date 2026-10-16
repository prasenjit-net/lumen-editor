use super::{
    TestDialogs, configure_ui,
    editor::{editor_frame, pointer, select, text_position},
    test_app,
};
use crate::app::{command::Command, find::find_input_id};
use eframe::egui;

fn key(key: egui::Key, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            shift,
            ..Default::default()
        },
    }
}

#[test]
fn find_seeds_selection_navigates_wraps_and_escape_returns_focus() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("é É é".into())]);
    select(&app, &ctx, 0, 1);
    app.execute(Command::Find, &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    let id = app.editor.active().unwrap().id;
    assert_eq!(app.view.find[&id].query, "é");
    assert_eq!(app.view.find[&id].matches.len(), 3);
    assert!(ctx.memory(|memory| memory.has_focus(find_input_id(id))));
    editor_frame(&mut app, &ctx, vec![key(egui::Key::Enter, false)]);
    assert_eq!(app.view.find[&id].current, 1);
    editor_frame(&mut app, &ctx, vec![key(egui::Key::Enter, true)]);
    assert_eq!(app.view.find[&id].current, 0);
    editor_frame(&mut app, &ctx, vec![key(egui::Key::Enter, true)]);
    assert_eq!(app.view.find[&id].current, 2);
    editor_frame(&mut app, &ctx, vec![key(egui::Key::Escape, false)]);
    assert!(!app.view.find[&id].open);
    assert!(ctx.memory(|memory| memory.has_focus(crate::app::editor_action::text_widget_id(id))));
}

#[test]
fn replace_all_is_one_undoable_edit_and_does_not_touch_other_documents() {
    let (mut app, storage) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("é é é\r\n".into())]);
    editor_frame(&mut app, &ctx, vec![]);
    let id = app.editor.active().unwrap().id;
    app.editor.files[0].mark_saved("fixture.txt".into());
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("other buffer".into())],
    );
    app.editor.set_active_index(Some(0));
    app.view.focus_editor = true;
    editor_frame(&mut app, &ctx, vec![]);
    app.execute(Command::Replace, &ctx);
    {
        let state = app.view.find.get_mut(&id).unwrap();
        state.query = "é".into();
        state.replacement = "éé".into();
        state.queue_replace(&app.editor.files[0], true);
    }
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "éé éé éé\r\n");
    assert!(app.editor.files[0].dirty);
    assert_eq!(app.view.find[&id].matches.len(), 6);
    assert!(ctx.memory(|memory| memory.has_focus(find_input_id(id))));
    app.execute(
        Command::EditorAction(id, crate::app::editor_action::EditorAction::Undo),
        &ctx,
    );
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "é é é\r\n");
    assert!(!app.editor.files[0].dirty);
    app.execute(
        Command::EditorAction(id, crate::app::editor_action::EditorAction::Redo),
        &ctx,
    );
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "éé éé éé\r\n");
    assert!(app.editor.files[0].dirty);
    app.editor.set_active_index(Some(1));
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[1].text, "other buffer");
    assert!(!app.view.find.contains_key(&app.editor.files[1].id));
    assert!(storage.0.borrow().writes.is_empty());
}

#[test]
fn stale_or_blocked_replacement_is_cancelled_without_mutation() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("keep".into())]);
    app.execute(Command::Replace, &ctx);
    let id = app.editor.active().unwrap().id;
    let state = app.view.find.get_mut(&id).unwrap();
    state.query = "keep".into();
    state.replacement = "bad".into();
    state.queue_replace(&app.editor.files[0], true);
    app.editor.files[0].text.push('!');
    app.editor.files[0].record_text_change();
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "keep!");
    app.view
        .find
        .get_mut(&id)
        .unwrap()
        .queue_replace(&app.editor.files[0], true);
    app.view.pending_close = Some(app.editor.files[0].id);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "keep!");
    app.view.pending_close = None;
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "keep!");
}

#[test]
fn find_state_is_per_document_and_replace_button_changes_only_current_match() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("cat cat".into())]);
    app.execute(Command::Replace, &ctx);
    let id = app.editor.active().unwrap().id;
    let state = app.view.find.get_mut(&id).unwrap();
    state.query = "cat".into();
    state.replacement = "dog".into();
    editor_frame(&mut app, &ctx, vec![]);
    let output = editor_frame(&mut app, &ctx, vec![]);
    let position = text_position(&output, "Replace");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "dog cat");
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    app.execute(Command::Find, &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    let second = app.editor.active().unwrap().id;
    app.view.find.get_mut(&second).unwrap().query = "other".into();
    app.editor.set_active_index(Some(0));
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.view.find[&id].query, "cat");
    assert_eq!(app.view.find[&second].query, "other");
    app.close_file(app.editor.files[1].id);
    assert!(!app.view.find.contains_key(&second));
}

#[test]
fn find_controls_remain_visible_at_narrow_default_and_wide_widths() {
    for width in [320.0, 700.0, 1400.0] {
        let (mut app, _) = test_app(TestDialogs::default());
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        app.new_file();
        super::editor::editor_frame_at_width(&mut app, &ctx, vec![], width);
        app.execute(Command::Replace, &ctx);
        super::editor::editor_frame_at_width(&mut app, &ctx, vec![], width);
        let output = super::editor::editor_frame_at_width(&mut app, &ctx, vec![], width);
        for title in ["Prev", "Next", "Replace", "All", "Aa", "ab"] {
            let position = text_position(&output, title);
            assert!(
                position.x > 0.0 && position.x < width,
                "{title} clipped at width {width}"
            );
        }
    }
}

#[test]
fn opening_find_selects_seeded_query_and_typing_does_not_edit_document() {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("cat dog".into())]);
    select(&app, &ctx, 1, 1);
    app.execute(Command::Find, &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    let id = app.editor.active().unwrap().id;
    assert_eq!(app.view.find[&id].query, "cat");
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("dog".into())]);
    assert_eq!(app.view.find[&id].query, "dog");
    assert_eq!(app.editor.files[0].text, "cat dog");
}
