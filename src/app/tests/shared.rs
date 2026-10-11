//! Explicitly runs the shaped workbench, alongside the unchanged legacy suite.
use super::editor::editor_frame;
use super::*;
use crate::app::editor_action::{EditorAction, text_widget_id};
use lumen_editor_core::{EditorBuffer, Selection};

fn fixture(text: &str) -> (LumenApp, MemoryFileSystem, egui::Context) {
    let (mut app, storage) = test_app(TestDialogs::default());
    app.view.shared_editor = true;
    storage
        .0
        .borrow_mut()
        .files
        .insert("fixture.txt".into(), text.into());
    app.open_file("fixture.txt".into());
    app.view.focus_editor = true;
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    editor_frame(&mut app, &ctx, vec![]);
    (app, storage, ctx)
}

fn buffer(app: &LumenApp) -> &EditorBuffer {
    app.editor.active().unwrap().editing.as_ref().unwrap()
}
fn select(app: &mut LumenApp, selections: Vec<Selection>, primary: usize) {
    let index = app.editor.active_file.unwrap();
    app.editor.files[index]
        .editing
        .as_mut()
        .unwrap()
        .set_selections(selections, primary)
        .unwrap();
}
fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        modifiers,
        physical_key: None,
        pressed: true,
        repeat: false,
    }
}

fn edit_command(app: &mut LumenApp, ctx: &egui::Context, action: EditorAction) {
    let id = app.editor.active().unwrap().id;
    app.execute(Command::EditorAction(id, action), ctx);
}

#[test]
fn workbench_multi_cursor_history_is_core_owned_and_save_checkpoint_survives_undo() {
    let (mut app, storage, ctx) = fixture("one two");
    let id = app.editor.active().unwrap().id;
    assert!(egui::text_edit::TextEditState::load(&ctx, text_widget_id(id)).is_none());
    select(
        &mut app,
        vec![
            Selection { anchor: 0, head: 3 },
            Selection { anchor: 4, head: 7 },
        ],
        1,
    );
    let before = buffer(&app).selections().clone();
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("漢".into())]);
    assert_eq!(app.editor.active().unwrap().text, "漢 漢");
    app.save_active();
    assert_eq!(storage.0.borrow().files[Path::new("fixture.txt")], "漢 漢");
    assert!(!app.editor.active().unwrap().dirty);
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).selections(), &before);
    assert_eq!(app.editor.active().unwrap().text, "one two");
    assert!(app.editor.active().unwrap().dirty);
    edit_command(&mut app, &ctx, EditorAction::Redo);
    editor_frame(&mut app, &ctx, vec![]);
    assert!(!app.editor.active().unwrap().dirty);
}

#[test]
fn failed_save_and_rename_preserve_shared_history_and_unsaved_content() {
    let (mut app, storage, ctx) = fixture("original");
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("changed".into())]);
    storage.0.borrow_mut().fail_writes = true;
    app.save_active();
    assert!(app.editor.active().unwrap().dirty);
    assert!(app.view.status.starts_with("Could not save file:"));
    app.editor.files[0].relocate("renamed.rs".into());
    assert!(buffer(&app).can_undo());
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "original");
    assert!(!app.editor.active().unwrap().dirty);
}

#[test]
fn find_replace_all_uses_core_history_and_does_not_change_secondary_undo_selection() {
    let (mut app, _, ctx) = fixture("name name");
    select(&mut app, vec![Selection::caret(0), Selection::caret(5)], 1);
    let before = buffer(&app).selections().clone();
    app.open_find(true, &ctx);
    let id = app.editor.active().unwrap().id;
    let document = &app.editor.files[0];
    let find = app.view.find.get_mut(&id).unwrap();
    find.query = "name".into();
    find.replacement = "longer".into();
    find.queue_replace(document, true);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "longer longer");
    app.view.find.get_mut(&id).unwrap().open = false;
    ctx.memory_mut(|memory| memory.request_focus(text_widget_id(id)));
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "name name");
    assert_eq!(buffer(&app).selections(), &before);
}

#[test]
fn tabs_have_independent_shared_history_and_stable_document_identity() {
    let (mut app, _, ctx) = fixture("first");
    let first = app.editor.active().unwrap().id;
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("A".into())]);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("B".into())]);
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "");
    app.close_file(1);
    app.view.focus_editor = true;
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.active().unwrap().id, first);
    assert_eq!(buffer(&app).text(), "Afirst");
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "first");
}

#[test]
fn tagged_paste_checks_the_entire_shared_selection_set() {
    for change in [false, true] {
        let (mut app, _, ctx) = fixture("a b");
        let clipboard = TestClipboard::default();
        app.clipboard = Box::new(clipboard.clone());
        select(&mut app, vec![Selection::caret(0), Selection::caret(2)], 0);
        app.execute(Command::Paste, &ctx);
        app.start_clipboard_read(&ctx);
        let request = app.view.editor_paste_target.unwrap().request;
        if change {
            select(&mut app, vec![Selection::caret(0), Selection::caret(3)], 0);
        }
        clipboard.complete(request, Ok(Some("漢\r\n".into())));
        editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(
            buffer(&app).text(),
            if change { "a b" } else { "漢\r\na 漢\r\nb" }
        );
    }
}

#[test]
fn pointer_and_modal_input_are_safe_and_right_click_preserves_selection() {
    let (mut app, _, ctx) = fixture("abc");
    select(&mut app, vec![Selection { anchor: 0, head: 3 }], 0);
    let before = buffer(&app).selections().clone();
    let pos = app
        .view
        .shared_output
        .as_ref()
        .unwrap()
        .geometry
        .caret_rect(1)
        .center();
    editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    assert_eq!(buffer(&app).selections(), &before);
    app.view.command_palette = true;
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("blocked".into())]);
    assert_eq!(buffer(&app).text(), "abc");
}

#[test]
fn ordered_line_clipboard_after_typing_navigation_and_paste_is_not_deferred() {
    let (mut app, _, ctx) = fixture("é\r\nnext");
    select(&mut app, vec![Selection::caret(2)], 0);
    editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text(" 🌍".into()),
            egui::Event::Copy,
            key(egui::Key::ArrowDown, egui::Modifiers::NONE),
            egui::Event::Paste("é 🌍\r\n".into()),
            egui::Event::Text("!".into()),
        ],
    );
    assert_eq!(buffer(&app).text(), "é 🌍\r\né 🌍\r\nnext!");
    assert!(app.view.pending_text_input.is_none());
}

#[test]
fn shaped_ime_commit_and_clipboard_failures_are_non_duplicating() {
    let (mut app, _, ctx) = fixture("a b");
    select(&mut app, vec![Selection::caret(0), Selection::caret(2)], 1);
    editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Ime(egui::ImeEvent::Enabled),
            egui::Event::Ime(egui::ImeEvent::Preedit("漢".into())),
        ],
    );
    assert_eq!(buffer(&app).revision(), 0);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Commit("漢".into()))],
    );
    assert_eq!(buffer(&app).text(), "漢a 漢b");
    assert_eq!(buffer(&app).revision(), 1);
    let clipboard = TestClipboard::default();
    app.clipboard = Box::new(clipboard.clone());
    app.execute(Command::Paste, &ctx);
    app.start_clipboard_read(&ctx);
    let request = app.view.editor_paste_target.unwrap().request;
    clipboard.complete(request, Err("test failure".into()));
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).revision(), 1);
    assert!(app.view.status.contains("test failure"));
}

#[test]
fn native_accelerator_and_existing_undo_key_do_not_undo_twice() {
    let (mut app, _, ctx) = fixture("text");
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("A".into())]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("B".into())]);
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 600.0),
            )),
            events: vec![key(egui::Key::Z, egui::Modifiers::COMMAND)],
            ..Default::default()
        },
        |ctx| {
            app.execute(Command::Undo, ctx);
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        },
    );
    assert_eq!(buffer(&app).text(), "Atext");
}

#[test]
fn stale_find_replace_and_stale_popup_actions_cannot_mutate_another_document() {
    let (mut app, _, ctx) = fixture("name");
    let id = app.editor.active().unwrap().id;
    app.open_find(true, &ctx);
    let find = app.view.find.get_mut(&id).unwrap();
    find.query = "name".into();
    find.replacement = "changed".into();
    find.queue_replace(&app.editor.files[0], true);
    app.editor.files[0]
        .editing
        .as_mut()
        .unwrap()
        .replace_document("new revision")
        .unwrap();
    app.editor.files[0].sync_shared_editing();
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "new revision");
    app.execute(Command::EditorAction(id, EditorAction::Cut), &ctx);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "");
    assert_eq!(app.editor.files[0].text, "new revision");
}

#[test]
fn replace_that_splits_a_grapheme_reports_failure_without_history_or_text_changes() {
    let (mut app, _, ctx) = fixture("é");
    let id = app.editor.active().unwrap().id;
    app.open_find(true, &ctx);
    let find = app.view.find.get_mut(&id).unwrap();
    find.query = "e".into();
    find.replacement = "x".into();
    find.queue_replace(&app.editor.files[0], false);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "é");
    assert!(!buffer(&app).can_undo());
    assert!(app.view.status.contains("splits a grapheme"));
}
