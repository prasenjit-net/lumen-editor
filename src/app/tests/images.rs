//! Image tabs: opening, loading, fallback, reload and read-only behavior.
use super::editor::editor_frame_at_width;
use super::*;
use crate::model::image_asset::ImageLoad;
use std::time::{Duration, Instant};

fn png(color: [u8; 4], width: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, 2, image::Rgba(color));
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn settle(app: &mut LumenApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app
        .editor
        .files
        .iter()
        .any(|file| matches!(file.image, Some(ImageLoad::Loading)))
        || !app.images.is_idle()
    {
        assert!(Instant::now() < deadline, "image load did not finish");
        std::thread::sleep(Duration::from_millis(5));
        app.poll_images(ctx);
    }
}

fn fixture() -> (LumenApp, egui::Context, tempfile::TempDir) {
    let (app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    (app, ctx, tempfile::tempdir().unwrap())
}

#[test]
fn image_opens_in_an_unpinned_free_clean_tab_and_reopening_focuses_it() {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("pic.png");
    std::fs::write(&path, png([1, 2, 3, 255], 3)).unwrap();
    app.open_file(path.clone());
    assert!(matches!(
        app.editor.files[0].image,
        Some(ImageLoad::Loading)
    ));
    settle(&mut app, &ctx);
    let Some(ImageLoad::Ready(image)) = &app.editor.files[0].image else {
        panic!("image should be ready");
    };
    assert_eq!((image.width, image.height), (3, 2));
    assert!(!app.editor.files[0].dirty);
    app.open_file(path);
    assert_eq!(app.editor.files.len(), 1);
}

#[test]
fn image_tab_renders_loading_ready_and_failed_states_without_panicking() {
    let (mut app, ctx, dir) = fixture();
    let good = dir.path().join("good.png");
    std::fs::write(&good, png([9, 9, 9, 100], 4)).unwrap();
    app.open_file(good);
    for width in [180.0, 600.0, 1400.0] {
        editor_frame_at_width(&mut app, &ctx, vec![], width);
    }
    settle(&mut app, &ctx);
    for width in [180.0, 600.0, 1400.0] {
        editor_frame_at_width(&mut app, &ctx, vec![], width);
    }
    assert_eq!(app.view.image_views.len(), 1);
    let bad = dir.path().join("bad.png");
    std::fs::write(&bad, b"just text").unwrap();
    app.open_file(bad);
    settle(&mut app, &ctx);
    assert!(matches!(
        app.editor.files[1].image,
        Some(ImageLoad::Failed(_))
    ));
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
}

#[test]
fn corrupt_image_falls_back_to_text_in_the_same_tab() {
    let (mut app, ctx, dir) = fixture();
    app.file_system = Box::new(crate::services::LocalFileSystem);
    let path = dir.path().join("notes.png");
    std::fs::write(&path, "hello").unwrap();
    app.open_file(path);
    settle(&mut app, &ctx);
    let id = app.editor.files[0].id;
    app.execute(Command::OpenImageAsText(id), &ctx);
    let file = &app.editor.files[0];
    assert_eq!(
        (file.id, file.text.as_str(), file.is_image()),
        (id, "hello", false)
    );
    assert!(!file.dirty);
}

#[test]
fn changed_file_reloads_and_deleted_file_is_flagged_but_keeps_its_pixels() {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("live.png");
    std::fs::write(&path, png([1, 1, 1, 255], 3)).unwrap();
    app.open_file(path.clone());
    settle(&mut app, &ctx);
    std::fs::write(&path, png([1, 1, 1, 255], 5)).unwrap();
    app.images.force_check();
    app.poll_images(&ctx);
    settle(&mut app, &ctx);
    let Some(ImageLoad::Ready(image)) = &app.editor.files[0].image else {
        panic!("image should be ready");
    };
    assert_eq!(image.width, 5);
    std::fs::remove_file(&path).unwrap();
    app.images.force_check();
    app.poll_images(&ctx);
    assert!(app.editor.files[0].disk_changed);
    assert!(matches!(
        app.editor.files[0].image,
        Some(ImageLoad::Ready(_))
    ));
}

#[test]
fn saving_an_image_tab_never_writes_to_the_file() {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("keep.png");
    let bytes = png([5, 5, 5, 255], 3);
    std::fs::write(&path, &bytes).unwrap();
    app.open_file(path.clone());
    settle(&mut app, &ctx);
    app.save_active();
    app.save_file_at(0, path.clone());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!app.editor.has_unsaved_changes());
}

#[test]
fn text_commands_do_not_open_for_image_tabs_and_closing_needs_no_prompt() {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("a.png");
    std::fs::write(&path, png([5, 5, 5, 255], 3)).unwrap();
    app.open_file(path);
    settle(&mut app, &ctx);
    app.open_find(false, &ctx);
    assert!(app.view.find.is_empty());
    app.open_go_to_line();
    assert!(app.view.navigation.mode.is_none());
    let id = app.editor.files[0].id;
    app.request_close(id);
    assert!(app.editor.files.is_empty());
    assert!(app.view.pending_close.is_none());
}
