//! Clipboard-request snapshots, separate from ordinary direct paste events.
use super::{
    TestDialogs, configure_ui,
    editor::{editor_frame, select},
    test_app,
};
use crate::app::{
    LumenApp,
    command::Command,
    editor_action::{EditorAction, text_widget_id},
};
use eframe::egui;

fn fixture() -> (LumenApp, egui::Context) {
    let (mut app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.new_file();
    app.editor.files[0].text = "é 🌍 original".into();
    app.editor.files[0].mark_saved("test.txt".into());
    editor_frame(&mut app, &ctx, vec![]);
    select(&app, &ctx, 4, 12);
    editor_frame(&mut app, &ctx, vec![]);
    (app, ctx)
}

fn request(app: &mut LumenApp, ctx: &egui::Context, native: bool) {
    if native {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.execute(Command::Paste, ctx);
            app.handle_events(ctx);
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        });
    } else {
        app.execute(
            Command::EditorAction(app.editor.files[0].id, EditorAction::Paste),
            ctx,
        );
        editor_frame(app, ctx, vec![]);
    }
    assert!(app.view.editor_paste_target.is_some());
}

fn deliver(app: &mut LumenApp, ctx: &egui::Context) {
    editor_frame(app, ctx, vec![egui::Event::Paste("replacement".into())]);
    assert!(app.view.editor_paste_target.is_none());
}

#[test]
fn native_and_popup_paste_replace_only_the_captured_selection_and_support_undo() {
    for native in [false, true] {
        let (mut app, ctx) = fixture();
        request(&mut app, &ctx, native);
        deliver(&mut app, &ctx);
        assert_eq!(app.editor.files[0].text, "é 🌍 replacement");
        app.execute(
            Command::EditorAction(app.editor.files[0].id, EditorAction::Undo),
            &ctx,
        );
        editor_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.editor.files[0].text, "é 🌍 original");
        assert!(!app.editor.files[0].dirty);
    }
}

#[test]
fn revision_change_cancels_paste_even_if_content_is_restored() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    app.editor.files[0].record_text_change();
    deliver(&mut app, &ctx);
    assert_eq!(app.editor.files[0].text, "é 🌍 original");
    assert!(!app.editor.files[0].dirty);
    assert!(app.view.status.starts_with("Paste cancelled:"));
}

#[test]
fn selection_movement_and_focus_round_trips_do_not_reactivate_a_cancelled_request() {
    for focus in [false, true] {
        let (mut app, ctx) = fixture();
        request(&mut app, &ctx, false);
        if focus {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("another_input")));
        } else {
            select(&app, &ctx, 0, 0);
        }
        editor_frame(&mut app, &ctx, vec![]);
        select(&app, &ctx, 4, 12);
        ctx.memory_mut(|memory| memory.request_focus(text_widget_id(app.editor.files[0].id)));
        deliver(&mut app, &ctx);
        assert_eq!(app.editor.files[0].text, "é 🌍 original");
    }
}

#[test]
fn find_input_must_not_receive_a_delayed_editor_paste() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    app.execute(Command::Find, &ctx);
    let query = app.view.find[&app.editor.files[0].id].query.clone();
    deliver(&mut app, &ctx);
    assert_eq!(app.view.find[&app.editor.files[0].id].query, query);
    assert_eq!(app.editor.files[0].text, "é 🌍 original");
}

#[test]
fn text_before_clipboard_response_cancels_the_paste_but_keeps_typing() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text("typed".into()),
            egui::Event::Paste("wrong".into()),
        ],
    );
    assert_eq!(app.editor.files[0].text, "é 🌍 typed");
    assert!(app.view.editor_paste_target.is_none());
    assert!(app.view.pending_text_input.is_none());
}

#[test]
fn typing_after_valid_paste_follows_it_in_order() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    editor_frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Paste("new".into()),
            egui::Event::Text("!".into()),
        ],
    );
    editor_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.files[0].text, "é 🌍 new!");
}

#[test]
fn rename_and_successful_save_do_not_invalidate_an_unchanged_document_target() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    app.editor.files[0].mark_saved("renamed.txt".into());
    deliver(&mut app, &ctx);
    assert_eq!(app.editor.files[0].text, "é 🌍 replacement");
    assert!(app.editor.files[0].dirty);
}

#[test]
fn empty_clipboard_response_is_a_safe_noop_and_consumes_the_request() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste(String::new())]);
    assert_eq!(app.editor.files[0].text, "é 🌍 original");
    assert!(!app.editor.files[0].dirty);
    assert!(app.view.editor_paste_target.is_none());
}

#[test]
fn existing_platform_paste_event_does_not_request_a_second_clipboard_read() {
    let (mut app, ctx) = fixture();
    let output = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Paste("new".into())],
            ..Default::default()
        },
        |ctx| {
            app.execute(Command::Paste, ctx);
            app.handle_events(ctx);
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        },
    );
    assert_eq!(app.editor.files[0].text, "é 🌍 new");
    assert!(app.view.editor_paste_target.is_none());
    assert!(!output.viewport_output.values().any(|viewport| {
        viewport
            .commands
            .contains(&egui::ViewportCommand::RequestPaste)
    }));
}

#[test]
fn native_focus_command_between_paste_validation_stages_cannot_redirect_delivery() {
    let (mut app, ctx) = fixture();
    request(&mut app, &ctx, false);
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Paste("wrong".into())],
            ..Default::default()
        },
        |ctx| {
            crate::app::editor_action::validate_paste(&mut app.view, app.editor.active(), ctx);
            app.execute(Command::Find, ctx);
            crate::app::editor_action::finish_paste_validation(
                &mut app.view,
                app.editor.active(),
                ctx,
            );
            app.render_component(ctx, |workbench| workbench.editor(ctx));
        },
    );
    assert_eq!(app.editor.files[0].text, "é 🌍 original");
    assert_eq!(app.view.find[&app.editor.files[0].id].query, "original");
    assert!(app.view.editor_paste_target.is_none());
}

#[test]
fn generic_find_field_native_paste_does_not_capture_an_editor_target() {
    let (mut app, ctx) = fixture();
    app.execute(Command::Find, &ctx);
    editor_frame(&mut app, &ctx, vec![]);
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        app.execute(Command::Paste, ctx);
        app.handle_events(ctx);
        app.render_component(ctx, |workbench| workbench.editor(ctx));
    });
    assert!(app.view.editor_paste_target.is_none());
    editor_frame(&mut app, &ctx, vec![egui::Event::Paste("query".into())]);
    assert_eq!(app.view.find[&app.editor.files[0].id].query, "query");
    assert_eq!(app.editor.files[0].text, "é 🌍 original");
}
