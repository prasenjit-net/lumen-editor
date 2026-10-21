use super::{
    TestDialogs, configure_ui,
    editor::{editor_frame, pointer},
    test_app,
};
use crate::app::LumenApp;
use eframe::egui;

fn fixture(text: &str) -> (LumenApp, egui::Context) {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    app.editor.files[0].text = text.into();
    app.editor.files[0].mark_saved("fixture.txt".into());
    editor_frame(&mut app, &ctx, vec![]);
    editor_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

#[test]
fn gutter_selected_line_copy_preserves_crlf_and_unicode() {
    let (mut app, ctx) = fixture("é 🌍\r\nsecond\r\n");
    let output = editor_frame(&mut app, &ctx, vec![]);
    let position = super::editor::text_position(&output, "1");
    for pressed in [true, false] {
        editor_frame(
            &mut app,
            &ctx,
            pointer(position, egui::PointerButton::Primary, pressed),
        );
    }
    let id = app.editor.files[0].id;
    app.execute(
        crate::app::command::Command::EditorAction(
            id,
            crate::app::editor_action::EditorAction::Copy,
        ),
        &ctx,
    );
    let output = editor_frame(&mut app, &ctx, vec![]);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "é 🌍\r\n")
    }));
    assert!(!app.editor.files[0].dirty);
}
