use super::editor::{editor_frame, pointer, select, text_position};
use super::{TestDialogs, configure_ui, test_app};
use crate::app::{
    LumenApp,
    command::Command,
    editor_action::{EditorAction, text_widget_id},
};
use eframe::egui;

fn fixture(text: &str, caret: usize) -> (LumenApp, egui::Context) {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    app.editor.files[0].text = text.into();
    app.editor.files[0].mark_saved("test.txt".into());
    editor_frame(&mut app, &ctx, vec![]);
    select(&app, &ctx, caret, caret);
    editor_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

fn copied(output: &egui::FullOutput) -> Option<&str> {
    output
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.as_str()),
            _ => None,
        })
}

fn caret(app: &LumenApp, ctx: &egui::Context) -> egui::text::CCursorRange {
    egui::text_edit::TextEditState::load(ctx, text_widget_id(app.editor.files[0].id))
        .unwrap()
        .cursor
        .char_range()
        .unwrap()
}

#[test]
fn empty_selection_copy_preserves_caret_revision_and_saved_state() {
    let (mut app, ctx) = fixture("é 🌍\r\nlast", 2);
    let before = caret(&app, &ctx);
    let revision = app.editor.files[0].revision;
    let output = editor_frame(&mut app, &ctx, vec![egui::Event::Copy]);
    assert_eq!(copied(&output), Some("é 🌍\r\n"));
    assert_eq!(caret(&app, &ctx), before);
    assert_eq!(app.editor.files[0].revision, revision);
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn whole_line_cut_is_one_widget_undo_transaction_with_redo() {
    let original = "é 🌍\r\nlast";
    let (mut app, ctx) = fixture(original, 2);
    let output = editor_frame(&mut app, &ctx, vec![egui::Event::Cut]);
    assert_eq!(copied(&output), Some("é 🌍\r\n"));
    assert_eq!(app.editor.files[0].text, "last");
    assert!(app.editor.files[0].dirty);
    assert_eq!(caret(&app, &ctx).primary.index, 0);
    for (action, expected, dirty) in [
        (EditorAction::Undo, original, false),
        (EditorAction::Redo, "last", true),
        (EditorAction::Undo, original, false),
    ] {
        let document = app.editor.files[0].id;
        app.execute(Command::EditorAction(document, action), &ctx);
        editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.editor.files[0].text, expected);
        assert_eq!(app.editor.files[0].dirty, dirty);
    }
    assert_eq!(caret(&app, &ctx).primary.index, 2);
    assert!(caret(&app, &ctx).is_empty());
}

#[test]
fn popup_cut_last_line_copies_its_own_eol_not_the_preceding_newline() {
    let (mut app, ctx) = fixture("first\r\n尾🌍", 8);
    let document = app.editor.files[0].id;
    app.execute(Command::EditorAction(document, EditorAction::Cut), &ctx);
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(copied(&output), Some("尾🌍\r\n"));
    assert_eq!(app.editor.files[0].text, "first");
    assert_eq!(caret(&app, &ctx).primary.index, 5);
    app.execute(Command::EditorAction(document, EditorAction::Undo), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "first\r\n尾🌍");
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn blank_eof_and_empty_document_cuts_are_safe() {
    for (text, position, clipboard, remaining) in [
        ("", 0, "\n", ""),
        ("solo", 2, "solo\n", ""),
        ("a\n\nb", 2, "\n", "a\nb"),
        ("a\r\n", 3, "\r\n", "a"),
    ] {
        let (mut app, ctx) = fixture(text, position);
        let output = editor_frame(&mut app, &ctx, vec![egui::Event::Cut]);
        assert_eq!(copied(&output), Some(clipboard));
        assert_eq!(app.editor.files[0].text, remaining);
        assert_eq!(app.editor.files[0].dirty, text != remaining);
    }
}

#[test]
fn native_commands_and_popup_copy_share_whole_line_behavior() {
    for targeted in [false, true] {
        let (mut app, ctx) = fixture("first\nlast", 9);
        let command = if targeted {
            Command::EditorAction(app.editor.files[0].id, EditorAction::Copy)
        } else {
            Command::Copy
        };
        // Native menu commands run inside the input frame. Events injected
        // outside Context::run are replaced by the next RawInput.
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            app.execute(command.clone(), ctx);
            app.handle_events(ctx);
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        });
        assert_eq!(copied(&output), Some("last\n"));
        assert!(caret(&app, &ctx).is_empty());
    }
}

#[test]
fn context_menu_copy_and_cut_are_enabled_without_a_selection() {
    for (title, remaining) in [("Copy", "line"), ("Cut", "")] {
        let (mut app, ctx) = fixture("line", 2);
        let output = editor_frame(&mut app, &ctx, vec![]);
        let position = text_position(&output, "line");
        for pressed in [true, false] {
            editor_frame(
                &mut app,
                &ctx,
                pointer(position, egui::PointerButton::Secondary, pressed),
            );
        }
        let output = editor_frame(&mut app, &ctx, vec![]);
        let position = text_position(&output, title);
        for pressed in [true, false] {
            editor_frame(
                &mut app,
                &ctx,
                pointer(position, egui::PointerButton::Primary, pressed),
            );
        }
        let output = editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(copied(&output), Some("line\n"));
        assert_eq!(app.editor.files[0].text, remaining);
    }
}

#[test]
fn selected_text_retains_normal_clipboard_behavior() {
    let (mut app, ctx) = fixture("é 🌍\r\nlast", 0);
    select(&app, &ctx, 2, 3);
    let output = editor_frame(&mut app, &ctx, vec![egui::Event::Cut]);
    assert_eq!(copied(&output), Some("🌍"));
    assert_eq!(app.editor.files[0].text, "é \r\nlast");
}

#[test]
fn find_field_clipboard_is_not_expanded_into_document_lines() {
    let (mut app, ctx) = fixture("hello\nworld", 2);
    app.execute(Command::Find, &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    let output = editor_frame(&mut app, &ctx, vec![egui::Event::Cut]);
    assert_eq!(app.editor.files[0].text, "hello\nworld");
    assert!(!app.editor.files[0].dirty);
    assert_ne!(copied(&output), Some("hello\n"));
}

#[test]
fn mixed_navigation_clipboard_frame_cuts_at_the_updated_caret() {
    let (mut app, ctx) = fixture("first\nlast", 0);
    let output = editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Cut,
        ],
    );
    assert_eq!(app.editor.files[0].text, "first\nlast");
    assert_eq!(copied(&output), None);
    assert!(app.view.pending_text_input.is_some());
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(copied(&output), Some("last\n"));
    assert_eq!(app.editor.files[0].text, "first");
    assert!(app.view.pending_text_input.is_none());
}

fn copy_line(app: &mut LumenApp, ctx: &egui::Context) -> String {
    copied(&editor_frame(app, ctx, vec![egui::Event::Copy]))
        .unwrap()
        .to_owned()
}

#[test]
fn whole_line_paste_preserves_original_column_and_is_one_undo_redo_step() {
    let original = "é 🌍\r\nsecond";
    let (mut app, ctx) = fixture(original, 2);
    let line = copy_line(&mut app, &ctx);
    select(&app, &ctx, 8, 8);
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    let pasted = "é 🌍\r\né 🌍\r\nsecond";
    assert_eq!(app.editor.files[0].text, pasted);
    assert_eq!(caret(&app, &ctx).primary.index, 13);
    assert!(app.editor.files[0].dirty);
    let document = app.editor.files[0].id;
    for (action, expected, position, dirty) in [
        (EditorAction::Undo, original, 8, false),
        (EditorAction::Redo, pasted, 13, true),
    ] {
        app.execute(Command::EditorAction(document, action), &ctx);
        editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.editor.files[0].text, expected);
        assert_eq!(caret(&app, &ctx).primary.index, position);
        assert!(caret(&app, &ctx).is_empty());
        assert_eq!(app.editor.files[0].dirty, dirty);
    }
}

#[test]
fn copied_line_can_be_pasted_repeatedly_in_another_document() {
    let (mut app, ctx) = fixture("source", 3);
    let line = copy_line(&mut app, &ctx);
    app.new_file();
    app.editor.files[1].text = "target".into();
    editor_frame(&mut app, &ctx, vec![]);
    select(&app, &ctx, 2, 2);
    for count in 1..=2 {
        editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line.clone())]);
        assert_eq!(
            app.editor.files[1].text,
            format!("{}target", "source\n".repeat(count))
        );
        let range =
            egui::text_edit::TextEditState::load(&ctx, text_widget_id(app.editor.files[1].id))
                .unwrap()
                .cursor
                .char_range()
                .unwrap();
        assert_eq!(range.primary.index, 2 + 7 * count);
        assert!(range.is_empty());
    }
    assert_eq!(app.editor.files[0].text, "source");
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn external_multiline_text_without_provenance_is_inserted_at_the_caret() {
    let (mut app, ctx) = fixture("target", 2);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste("source\n".into())]);
    assert_eq!(app.editor.files[0].text, "tasource\nrget");
}

#[test]
fn selected_text_copy_invalidates_line_metadata_even_when_text_is_identical() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&app, &ctx, 0, 7);
    assert_eq!(
        copied(&editor_frame(&mut app, &ctx, vec![egui::Event::Copy])),
        Some(line.as_str())
    );
    select(&app, &ctx, 9, 9);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\ntasource\nrget");
}

#[test]
fn selected_paste_replaces_the_selection_instead_of_inserting_above_it() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&app, &ctx, 7, 13);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\nsource\n");
}

#[test]
fn mismatched_paste_invalidates_metadata_for_later_matching_text() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&app, &ctx, 9, 9);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste("!".into())]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\nta!source\nrget");
}

#[test]
fn cut_line_and_empty_line_paste_keep_provenance_and_line_endings() {
    for (original, caret, clipboard, remaining, expected, after) in [
        ("é 🌍\r\nnext", 2, "é 🌍\r\n", "next", "é 🌍\r\nnext", 5),
        ("a\r\n", 3, "\r\n", "a", "\r\na", 3),
        ("", 0, "\n", "", "\n", 1),
    ] {
        let (mut app, ctx) = fixture(original, caret);
        let output = editor_frame(&mut app, &ctx, vec![egui::Event::Cut]);
        assert_eq!(copied(&output), Some(clipboard));
        assert_eq!(app.editor.files[0].text, remaining);
        editor_frame(&mut app, &ctx, vec![egui::Event::Paste(clipboard.into())]);
        assert_eq!(app.editor.files[0].text, expected);
        assert_eq!(self::caret(&app, &ctx).primary.index, after);
    }
}

#[test]
fn delayed_popup_line_paste_is_cancelled_after_switching_tabs() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    let document = app.editor.files[0].id;
    app.execute(Command::EditorAction(document, EditorAction::Paste), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    app.new_file();
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\ntarget");
    assert!(app.editor.files[1].text.is_empty());
}

#[test]
fn delayed_popup_line_paste_uses_the_original_focused_document() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&app, &ctx, 9, 9);
    let document = app.editor.files[0].id;
    app.execute(Command::EditorAction(document, EditorAction::Paste), &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\nsource\ntarget");
    assert_eq!(caret(&app, &ctx).primary.index, 16);
}

#[test]
fn copy_path_clears_line_provenance_without_changing_documents() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    app.execute(Command::CopyPath("some/path".into(), false), &ctx);
    select(&app, &ctx, 9, 9);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\ntasource\nrget");
}

#[test]
fn paste_in_find_field_keeps_document_unchanged_even_with_line_metadata() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    app.execute(Command::Find, &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\ntarget");
    assert!(!app.editor.files[0].dirty);
    assert_eq!(app.view.find[&app.editor.files[0].id].query, "source ");
}

#[test]
fn paste_batched_with_later_typing_does_not_override_the_final_caret() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&app, &ctx, 9, 9);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Paste(line), egui::Event::Text("!".into())],
    );
    assert_eq!(app.editor.files[0].text, "source\nsource\ntarget");
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "source\nsource\nta!rget");
    assert_eq!(caret(&app, &ctx).primary.index, 17);
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn drain_input(app: &mut LumenApp, ctx: &egui::Context) -> Vec<String> {
    let mut copied_text = Vec::new();
    for _ in 0..20 {
        if app.view.pending_text_input.is_none() {
            return copied_text;
        }
        let output = editor_frame(app, ctx, vec![]);
        if let Some(text) = copied(&output) {
            copied_text.push(text.into());
        }
    }
    panic!("input batch did not drain");
}

#[test]
fn typing_before_copy_copies_the_updated_unicode_line() {
    let (mut app, ctx) = fixture("é 🌍\r\nlast", 2);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("new ".into()), egui::Event::Copy],
    );
    assert_eq!(drain_input(&mut app, &ctx), ["é new 🌍\r\n"]);
    assert_eq!(app.editor.files[0].text, "é new 🌍\r\nlast");
    assert_eq!(caret(&app, &ctx).primary.index, 6);
}

#[test]
fn multiple_clipboard_actions_preserve_metadata_and_order_in_one_raw_batch() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let output = editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Copy,
            egui::Event::Paste("source\n".into()),
            egui::Event::Text("!".into()),
        ],
    );
    assert_eq!(copied(&output), Some("source\n"));
    drain_input(&mut app, &ctx);
    assert_eq!(app.editor.files[0].text, "source\nso!urce\ntarget");
    assert_eq!(caret(&app, &ctx).primary.index, 10);
}

#[test]
fn arriving_input_stays_after_an_older_deferred_clipboard_event() {
    let (mut app, ctx) = fixture("source", 0);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("A".into()), egui::Event::Copy],
    );
    let output = editor_frame(&mut app, &ctx, vec![egui::Event::Text("B".into())]);
    assert_eq!(copied(&output), Some("Asource\n"));
    assert_eq!(app.editor.files[0].text, "Asource");
    drain_input(&mut app, &ctx);
    assert_eq!(app.editor.files[0].text, "ABsource");
    assert_eq!(caret(&app, &ctx).primary.index, 2);
}

#[test]
fn cut_then_paste_in_a_batch_keep_independent_widget_undo_boundaries() {
    let (mut app, ctx) = fixture("first\nlast", 6);
    editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Cut, egui::Event::Paste("last\n".into())],
    );
    drain_input(&mut app, &ctx);
    assert_eq!(app.editor.files[0].text, "last\nfirst");
    let document = app.editor.files[0].id;
    for expected in ["first", "first\nlast"] {
        app.execute(Command::EditorAction(document, EditorAction::Undo), &ctx);
        editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.editor.files[0].text, expected);
    }
    assert!(!app.editor.files[0].dirty);
}

#[test]
fn queued_input_is_cancelled_after_tab_switch_or_document_revision_change() {
    for switch_tab in [false, true] {
        let (mut app, ctx) = fixture("first\nlast", 0);
        editor_frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown), egui::Event::Cut],
        );
        if switch_tab {
            app.new_file();
        } else {
            app.editor.files[0].text.push('!');
            app.editor.files[0].record_text_change();
        }
        let output = editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(copied(&output), None);
        assert!(app.view.pending_text_input.is_none());
        assert_eq!(
            app.editor.files[0].text,
            if switch_tab {
                "first\nlast"
            } else {
                "first\nlast!"
            }
        );
        if switch_tab {
            assert!(app.editor.files[1].text.is_empty());
        }
    }
}

#[test]
fn queued_input_is_cancelled_if_caret_changes_or_find_takes_focus() {
    for find in [false, true] {
        let (mut app, ctx) = fixture("first\nlast", 0);
        editor_frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown), egui::Event::Cut],
        );
        if find {
            app.execute(Command::Find, &ctx);
        } else {
            select(&app, &ctx, 0, 0);
        }
        editor_frame(&mut app, &ctx, vec![]);
        assert!(app.view.pending_text_input.is_none());
        assert_eq!(app.editor.files[0].text, "first\nlast");
    }
}

#[test]
fn pointer_gesture_cancels_a_deferred_keyboard_batch() {
    let (mut app, ctx) = fixture("first\nlast", 0);
    editor_frame(
        &mut app,
        &ctx,
        vec![key(egui::Key::ArrowDown), egui::Event::Cut],
    );
    editor_frame(
        &mut app,
        &ctx,
        pointer(egui::pos2(700.0, 400.0), egui::PointerButton::Primary, true),
    );
    assert!(app.view.pending_text_input.is_none());
    assert_eq!(app.editor.files[0].text, "first\nlast");
}

#[test]
fn modal_or_ime_input_cancels_deferred_clipboard_without_editing_the_buffer() {
    for ime in [false, true] {
        let (mut app, ctx) = fixture("first\nlast", 0);
        editor_frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::ArrowDown), egui::Event::Cut],
        );
        let events = if ime {
            vec![egui::Event::Ime(egui::ImeEvent::Enabled)]
        } else {
            app.view.pending_quit = true;
            Vec::new()
        };
        editor_frame(&mut app, &ctx, events);
        assert!(app.view.pending_text_input.is_none());
        assert_eq!(app.editor.files[0].text, "first\nlast");
        assert!(!app.editor.files[0].dirty);
    }
}

#[test]
fn native_cut_does_not_duplicate_an_existing_platform_clipboard_event() {
    let (mut app, ctx) = fixture("first\nlast", 2);
    let output = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Cut],
            ..Default::default()
        },
        |ctx| {
            app.execute(Command::Cut, ctx);
            app.handle_events(ctx);
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        },
    );
    assert_eq!(copied(&output), Some("first\n"));
    assert_eq!(app.editor.files[0].text, "last");
    assert!(app.view.pending_text_input.is_none());
}
