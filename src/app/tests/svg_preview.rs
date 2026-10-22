//! SVG preview tabs: availability, liveness, errors, sharp zoom, lifetime.
use super::editor::editor_frame_at_width;
use super::*;
use crate::{app::preview::raster_scale, model::Document};
use std::path::PathBuf;

const RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#f00"/></svg>"##;

fn fixture() -> (LumenApp, egui::Context) {
    let (app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    (app, ctx)
}

fn open_document(app: &mut LumenApp, path: &str, text: &str) -> crate::model::DocumentId {
    let document = Document::from_path(PathBuf::from(path), text.into());
    let id = document.id;
    app.editor.files.push(document);
    app.editor.activate(id);
    id
}

fn edit(app: &mut LumenApp, index: usize, text: &str) {
    let file = &mut app.editor.files[index];
    file.enable_shared_editing();
    file.editing
        .as_mut()
        .unwrap()
        .replace_document(text)
        .unwrap();
}

#[test]
fn svg_offers_a_preview_and_stays_a_text_document() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/logo.svg", RED);
    assert!(!app.editor.files[0].is_image());
    assert!(app.editor.files[0].preview_kind().is_some());
    app.execute(Command::OpenPreview(source), &ctx);
    assert_eq!(app.editor.files.len(), 2);
    assert_eq!(app.editor.files[1].preview_of, Some(source));
    assert_eq!(app.editor.files[1].name, "Preview: logo.svg");

    app.editor.activate(source);
    app.execute(Command::OpenActivePreview, &ctx);
    assert_eq!(app.editor.files.len(), 2, "focuses the existing preview");
    assert_eq!(app.editor.active_id(), Some(app.editor.files[1].id));
}

#[test]
fn preview_rerenders_unsaved_edits_and_reports_invalid_markup() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/logo.svg", RED);
    app.execute(Command::OpenPreview(source), &ctx);
    let preview = app.editor.files[1].id;
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
    assert_eq!(
        app.view.preview.svg(preview, &app.editor.files[0]).size(),
        Ok((40, 20))
    );

    edit(&mut app, 0, &RED.replace("40", "80"));
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
    assert_eq!(
        app.view.preview.svg(preview, &app.editor.files[0]).size(),
        Ok((80, 20))
    );
    assert!(app.editor.files[0].dirty && !app.editor.files[1].dirty);

    for broken in ["<svg", "", "plain text"] {
        edit(&mut app, 0, broken);
        editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
        assert!(
            app.view
                .preview
                .svg(preview, &app.editor.files[0])
                .size()
                .is_err()
        );
    }
    edit(&mut app, 0, RED);
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
    assert!(
        app.view
            .preview
            .svg(preview, &app.editor.files[0])
            .size()
            .is_ok()
    );
}

#[test]
fn preview_rasterizes_sharper_when_zoomed_in_and_reuses_the_bitmap_otherwise() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/logo.svg", RED);
    app.execute(Command::OpenPreview(source), &ctx);
    let preview = app.editor.files[1].id;
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
    let state = app.view.preview.svg(preview, &app.editor.files[0]);
    let small = state.raster(1.0).unwrap();
    assert!(std::sync::Arc::ptr_eq(&small, &state.raster(1.0).unwrap()));
    let sharp = state.raster(8.0).unwrap();
    assert_eq!((sharp.width, sharp.height), (40, 20));
    assert_eq!(sharp.texture_width, 320);
    assert!(sharp.texture_width > small.texture_width);
    assert!(raster_scale(1.0) < raster_scale(8.0));
}

#[test]
fn closing_either_side_and_renaming_behave_like_other_previews() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/logo.svg", RED);
    app.execute(Command::OpenPreview(source), &ctx);
    app.editor.files[0].relocate(PathBuf::from("/w/mark.svg"));
    app.maintain_previews(&ctx);
    assert_eq!(app.editor.files[1].name, "Preview: mark.svg");

    let preview = app.editor.files[1].id;
    app.execute(Command::RequestClose(preview), &ctx);
    assert!(app.view.pending_close.is_none());
    assert_eq!(app.editor.files.len(), 1);

    app.execute(Command::OpenPreview(source), &ctx);
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
    app.execute(Command::RequestClose(source), &ctx);
    assert!(app.editor.files.is_empty());
    app.maintain_previews(&ctx);
}

#[test]
fn preview_draws_at_narrow_default_and_wide_widths_for_valid_and_invalid_svg() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/logo.svg", RED);
    app.execute(Command::OpenPreview(source), &ctx);
    for width in [160.0, 600.0, 1600.0] {
        editor_frame_at_width(&mut app, &ctx, vec![], width);
    }
    edit(&mut app, 0, "<svg");
    for width in [160.0, 600.0, 1600.0] {
        editor_frame_at_width(&mut app, &ctx, vec![], width);
    }
}
