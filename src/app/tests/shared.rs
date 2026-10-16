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
    let index = app.editor.active_index().unwrap();
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
fn default_view_renders_document_core_without_an_opt_in_toggle() {
    let (mut app, _) = test_app(TestDialogs::default());
    app.view = crate::app::view_state::ViewState::default();
    assert!(app.view.shared_editor);
    app.new_file();
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    editor_frame(&mut app, &ctx, vec![]);
    let document = app.editor.active().unwrap();
    assert!(document.editing.is_some());
    assert!(egui::text_edit::TextEditState::load(&ctx, text_widget_id(document.id)).is_none());
}

#[test]
fn go_to_line_moves_shared_caret_without_history_or_text_change() {
    let (mut app, _, ctx) = fixture("one\r\ne\u{301}x\r\nthree");
    let id = app.editor.active().unwrap().id;
    let before = buffer(&app).text().to_owned();
    app.go_to_line(id, "2:2");
    editor_frame(&mut app, &ctx, vec![]);
    let selections = buffer(&app).selections();
    assert_eq!(
        selections.selections()[selections.primary_index()].head,
        "one\r\ne\u{301}".len()
    );
    assert_eq!(buffer(&app).text(), before);
    assert!(!buffer(&app).can_undo());
}

#[test]
fn save_as_cancellation_and_failed_close_save_preserve_core_history() {
    let (mut app, storage, ctx) = fixture("original");
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("x".into())]);
    let id = app.editor.active().unwrap().id;
    let original_selection = buffer(&app).selections().clone();
    let revision = buffer(&app).revision();
    app.save_as_active(); // Injected dialog cancels.
    assert_eq!(buffer(&app).revision(), revision);
    assert_eq!(buffer(&app).selections(), &original_selection);
    assert!(buffer(&app).is_dirty());
    storage.0.borrow_mut().fail_writes = true;
    app.request_close(app.editor.files[0].id);
    assert_eq!(app.view.pending_close, Some(app.editor.files[0].id));
    app.execute(Command::SaveAndClose(app.editor.files[0].id), &ctx);
    assert_eq!(app.editor.active().unwrap().id, id);
    assert!(buffer(&app).is_dirty());
    assert_eq!(buffer(&app).text(), "xoriginal");
    assert!(buffer(&app).can_undo());
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "original");
    assert!(!buffer(&app).is_dirty());
}

#[test]
fn delayed_paste_rejects_affinity_only_target_changes() {
    use lumen_editor_core::{CaretAffinity, SelectionAffinity};
    let (mut app, _, ctx) = fixture("abc אבג xyz");
    select(&mut app, vec![Selection::caret(4)], 0);
    let clipboard = TestClipboard::default();
    app.clipboard = Box::new(clipboard.clone());
    app.execute(Command::Paste, &ctx);
    app.start_clipboard_read(&ctx);
    let request = app.view.editor_paste_target.unwrap().request;
    app.editor.files[0]
        .editing
        .as_mut()
        .unwrap()
        .set_selections_with_affinity(
            vec![(
                Selection::caret(4),
                SelectionAffinity {
                    anchor: CaretAffinity::Before,
                    head: CaretAffinity::Before,
                },
            )],
            0,
        )
        .unwrap();
    clipboard.complete(request, Ok(Some("unsafe".into())));
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "abc אבג xyz");
    assert!(!buffer(&app).is_dirty());
}

#[test]
fn native_popup_undo_restores_visual_selection_and_affinity() {
    use lumen_editor_core::{CaretAffinity, SelectionAffinity};
    let (mut app, _, ctx) = fixture("abc אבג xyz");
    app.editor.files[0]
        .editing
        .as_mut()
        .unwrap()
        .set_selections_with_affinity(
            vec![(
                Selection::caret(4),
                SelectionAffinity {
                    anchor: CaretAffinity::Before,
                    head: CaretAffinity::Before,
                },
            )],
            0,
        )
        .unwrap();
    editor_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::ArrowRight, egui::Modifiers::SHIFT)],
    );
    let selection = buffer(&app).selections().clone();
    assert_eq!(selection.affinities()[0].anchor, CaretAffinity::Before);
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("x".into())]);
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).selections(), &selection);
    assert_eq!(buffer(&app).text(), "abc אבג xyz");
}

#[test]
fn shaped_workbench_retains_all_cursor_columns_without_creating_history() {
    let (mut app, _, ctx) = fixture("abcdef\nx\nabcdef\nabcdef\nx\nabcdef");
    select(&mut app, vec![Selection::caret(3), Selection::caret(19)], 1);
    editor_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
    );
    assert_eq!(
        buffer(&app).selections().selections(),
        &[Selection::caret(8), Selection::caret(24)]
    );
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::ArrowDown,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    editor_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)],
    );
    assert_eq!(
        buffer(&app).selections().selections(),
        &[Selection::caret(12), Selection::caret(28)]
    );
    assert_eq!(buffer(&app).selections().primary_index(), 1);
    assert!(!app.editor.active().unwrap().dirty);
    assert!(!buffer(&app).can_undo());
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
fn popup_cut_copy_and_tagged_paste_edit_each_selected_range() {
    let (mut app, _, ctx) = fixture("aa bb");
    let clipboard = TestClipboard::default();
    app.clipboard = Box::new(clipboard.clone());
    select(
        &mut app,
        vec![
            Selection { anchor: 0, head: 2 },
            Selection { anchor: 3, head: 5 },
        ],
        1,
    );
    let original = buffer(&app).selections().clone();
    edit_command(&mut app, &ctx, EditorAction::Copy);
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(
        output.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(text) if text == "aa\nbb")
        )
    );
    assert_eq!(buffer(&app).selections(), &original);
    assert!(!buffer(&app).is_dirty());

    edit_command(&mut app, &ctx, EditorAction::Cut);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), " ");
    edit_command(&mut app, &ctx, EditorAction::Paste);
    editor_frame(&mut app, &ctx, vec![]);
    app.start_clipboard_read(&ctx);
    let request = app.view.editor_paste_target.unwrap().request;
    clipboard.complete(request, Ok(Some("aa\nbb".into())));
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "aa bb");
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), " ");
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).selections(), &original);
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
    app.close_file(app.editor.files[1].id);
    app.view.focus_editor = true;
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.active().unwrap().id, first);
    assert_eq!(buffer(&app).text(), "Afirst");
    editor_frame(&mut app, &ctx, vec![egui::Event::Text("C".into())]);
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "Afirst");
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "first");
}

#[test]
fn shaped_tabs_restore_two_axis_scroll_selection_and_focus() {
    let text = "long code ".repeat(160) + &"\nmore rows".repeat(100);
    let (mut app, _, ctx) = fixture(&text);
    let first = app.editor.active().unwrap().id;
    select(&mut app, vec![Selection::caret(3), Selection::caret(10)], 1);
    editor_frame(&mut app, &ctx, vec![]);
    let before = app.view.shared_output.as_ref().unwrap().geometry.origin;
    let pointer = app.view.shared_output.as_ref().unwrap().text_clip.center();
    editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(-90.0, -70.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    for _ in 0..4 {
        editor_frame(&mut app, &ctx, vec![]);
    }
    let scrolled = app.view.shared_output.as_ref().unwrap().geometry.origin;
    assert!(scrolled.x < before.x && scrolled.y < before.y);
    let selection = buffer(&app).selections().clone();
    app.new_file();
    editor_frame(&mut app, &ctx, vec![]);
    app.close_file(app.editor.files[1].id);
    app.view.focus_editor = true;
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.active().unwrap().id, first);
    assert_eq!(buffer(&app).selections(), &selection);
    let restored = app.view.shared_output.as_ref().unwrap().geometry.origin;
    assert!((restored - scrolled).length() < 0.01);
    assert!(ctx.memory(|memory| memory.has_focus(text_widget_id(first))));
    assert!(!buffer(&app).can_undo());
}

#[test]
fn shaped_typing_coalesces_but_save_is_a_separate_step() {
    let (mut app, _, ctx) = fixture("x");
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("a".into()), egui::Event::Text("b".into())],
    );
    app.save_active();
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("c".into()), egui::Event::Text("d".into())],
    );
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "abx");
    assert!(!app.editor.active().unwrap().dirty);
    edit_command(&mut app, &ctx, EditorAction::Undo);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "x");
    assert!(app.editor.active().unwrap().dirty);
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
        assert_eq!(buffer(&app).text(), if change { "a b" } else { "漢a b" });
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
    // This test needs two undo groups to detect a duplicated accelerator.
    app.editor.files[0]
        .editing
        .as_mut()
        .unwrap()
        .break_undo_group();
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

fn open_completion(app: &mut LumenApp, word_start: usize, origin: usize, labels: &[&str]) {
    let document = app.editor.active().unwrap().id;
    app.view.language_completion = Some(crate::app::view_state::CompletionSession {
        document,
        origin,
        word_start,
        items: labels
            .iter()
            .map(|label| lumen_language_services::incoming::CompletionItem {
                label: (*label).into(),
                detail: None,
                kind: None,
                filter_text: (*label).into(),
                insert_text: (*label).into(),
                edit_range: None,
                additional_edits: Vec::new(),
            })
            .collect(),
        selected: 0,
        pending_accept: None,
    });
}

#[test]
fn completion_keys_navigate_accept_and_undo_without_reaching_the_editor() {
    let (mut app, _, ctx) = fixture("x.pu");
    select(&mut app, vec![Selection::caret(4)], 0);
    open_completion(&mut app, 2, 4, &["pull", "push"]);
    let none = egui::Modifiers::NONE;
    editor_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown, none)]);
    assert_eq!(app.view.language_completion.as_ref().unwrap().selected, 1);
    // Arrow keys were consumed: the caret did not move.
    assert_eq!(
        buffer(&app).selections().selections()[0],
        Selection::caret(4)
    );
    editor_frame(&mut app, &ctx, vec![key(egui::Key::Enter, none)]);
    assert_eq!(
        buffer(&app).text(),
        "x.push",
        "Enter accepts instead of inserting a newline"
    );
    assert!(app.view.language_completion.is_none());
    assert_eq!(app.editor.active().unwrap().text, "x.push");
    let id = app.editor.active().unwrap().id;
    app.execute(Command::EditorAction(id, EditorAction::Undo), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "x.pu");
}

#[test]
fn completion_closes_on_escape_caret_exit_and_foreign_documents() {
    let none = egui::Modifiers::NONE;
    let (mut app, _, ctx) = fixture("x.pu");
    select(&mut app, vec![Selection::caret(4)], 0);
    open_completion(&mut app, 2, 4, &["push"]);
    editor_frame(&mut app, &ctx, vec![key(egui::Key::Escape, none)]);
    assert!(app.view.language_completion.is_none());
    assert_eq!(buffer(&app).text(), "x.pu");

    open_completion(&mut app, 2, 4, &["push"]);
    select(&mut app, vec![Selection::caret(1)], 0);
    editor_frame(&mut app, &ctx, vec![]);
    assert!(app.view.language_completion.is_none());

    select(&mut app, vec![Selection::caret(4)], 0);
    open_completion(&mut app, 2, 4, &["push"]);
    app.view.language_completion.as_mut().unwrap().document =
        crate::model::Document::untitled(0).id;
    editor_frame(&mut app, &ctx, vec![]);
    assert!(app.view.language_completion.is_none());
}

#[test]
fn clicked_completion_applies_before_editor_input_and_typing_filters() {
    let (mut app, _, ctx) = fixture("x.pu");
    select(&mut app, vec![Selection::caret(4)], 0);
    open_completion(&mut app, 2, 4, &["pull", "push", "len"]);
    app.view
        .language_completion
        .as_mut()
        .unwrap()
        .pending_accept = Some(0);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(buffer(&app).text(), "x.pull");
    assert!(app.view.language_completion.is_none());
}
