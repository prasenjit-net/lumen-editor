use super::editor::{editor_frame, select};
use super::{TestDialogs, configure_ui, test_app};
use crate::app::{LumenApp, command::Command, editor_action::EditorAction};
use eframe::egui;

fn fixture(text: &str, caret: usize) -> (LumenApp, egui::Context) {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    app.editor.files[0].text = text.into();
    app.editor.files[0].mark_saved("test.txt".into());
    editor_frame(&mut app, &ctx, vec![]);
    select(&mut app, &ctx, caret, caret);
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
fn selected_text_retains_normal_clipboard_behavior() {
    let (mut app, ctx) = fixture("é 🌍\r\nlast", 0);
    select(&mut app, &ctx, 2, 3);
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

fn copy_line(app: &mut LumenApp, ctx: &egui::Context) -> String {
    copied(&editor_frame(app, ctx, vec![egui::Event::Copy]))
        .unwrap()
        .to_owned()
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
    select(&mut app, &ctx, 0, 7);
    assert_eq!(
        copied(&editor_frame(&mut app, &ctx, vec![egui::Event::Copy])),
        Some(line.as_str())
    );
    select(&mut app, &ctx, 9, 9);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\ntasource\nrget");
}

#[test]
fn selected_paste_replaces_the_selection_instead_of_inserting_above_it() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&mut app, &ctx, 7, 13);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\nsource\n");
}

#[test]
fn mismatched_paste_invalidates_metadata_for_later_matching_text() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    select(&mut app, &ctx, 9, 9);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste("!".into())]);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(line)]);
    assert_eq!(app.editor.files[0].text, "source\nta!source\nrget");
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
fn copy_path_clears_line_provenance_without_changing_documents() {
    let (mut app, ctx) = fixture("source\ntarget", 2);
    let line = copy_line(&mut app, &ctx);
    app.execute(Command::CopyPath("some/path".into(), false), &ctx);
    select(&mut app, &ctx, 9, 9);
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
