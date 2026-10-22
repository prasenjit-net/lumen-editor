//! Preview tabs: availability, open/focus, liveness, lifetime, rename, links.
use super::editor::editor_frame_at_width;
use super::*;
use crate::model::Document;
use std::path::PathBuf;

const SAMPLE: &str = "# Title\n\nSome *text* with a [link](other.md) and ![pic](pic.png).\n\n\
- [x] done\n- [ ] todo\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n## Later\n";

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

fn previews(app: &LumenApp) -> Vec<&Document> {
    app.editor
        .files
        .iter()
        .filter(|file| file.is_preview())
        .collect()
}

#[test]
fn only_markdown_documents_offer_a_preview() {
    let (mut app, ctx) = fixture();
    let markdown = open_document(&mut app, "/w/readme.md", SAMPLE);
    let code = open_document(&mut app, "/w/main.rs", "fn main() {}");
    assert!(app.editor.files[0].preview_kind().is_some());
    assert!(app.editor.files[1].preview_kind().is_none());
    app.execute(Command::OpenPreview(code), &ctx);
    assert!(previews(&app).is_empty());
    app.editor.activate(markdown);
    assert!(app.editor.active().unwrap().preview_kind().is_some());
}

#[test]
fn opening_a_preview_adds_one_clean_tab_after_the_source_and_refocuses_it() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/readme.md", SAMPLE);
    open_document(&mut app, "/w/other.txt", "x");
    app.execute(Command::OpenPreview(source), &ctx);
    let preview = &app.editor.files[1];
    assert_eq!(preview.preview_of, Some(source));
    assert_eq!(preview.name, "Preview: readme.md");
    assert!(!preview.dirty && preview.path.is_none());
    assert_eq!(app.editor.active_id(), Some(preview.id));
    assert!(preview.preview_kind().is_none(), "no preview of a preview");

    app.editor.activate(source);
    app.execute(Command::OpenActivePreview, &ctx);
    assert_eq!(previews(&app).len(), 1);
    assert_eq!(app.editor.active_id(), Some(app.editor.files[1].id));
}

#[test]
fn preview_follows_unsaved_edits_and_a_rename_of_its_source() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/readme.md", "# One\n");
    app.execute(Command::OpenPreview(source), &ctx);
    editor_frame_at_width(&mut app, &ctx, vec![], 900.0);
    let before = app.view.preview.blocks(&app.editor.files[0]);

    let file = &mut app.editor.files[0];
    file.enable_shared_editing();
    file.editing
        .as_mut()
        .unwrap()
        .replace_document("# Two\n")
        .unwrap();
    editor_frame_at_width(&mut app, &ctx, vec![], 900.0);
    let after = app.view.preview.blocks(&app.editor.files[0]);
    assert_ne!(before, after);
    assert!(app.editor.files[0].dirty, "the source stays unsaved");
    assert!(!app.editor.files[1].dirty);

    app.editor.files[0].relocate(PathBuf::from("/w/guide.md"));
    app.maintain_previews(&ctx);
    assert_eq!(app.editor.files[1].name, "Preview: guide.md");
}

#[test]
fn closing_the_source_closes_its_preview_without_prompting_and_closing_the_preview_keeps_the_source()
 {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/readme.md", SAMPLE);
    app.execute(Command::OpenPreview(source), &ctx);
    let preview = app.editor.files[1].id;
    app.execute(Command::RequestClose(preview), &ctx);
    assert!(app.view.pending_close.is_none());
    assert_eq!(app.editor.files.len(), 1);

    app.execute(Command::OpenPreview(source), &ctx);
    app.execute(Command::RequestClose(source), &ctx);
    assert!(app.editor.files.is_empty());
    assert!(
        app.view
            .closed_tabs
            .iter()
            .all(|tab| tab.path != PathBuf::new())
    );
}

#[test]
fn previews_cannot_be_saved_or_counted_as_unsaved_work() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/readme.md", SAMPLE);
    app.execute(Command::OpenPreview(source), &ctx);
    app.execute(Command::Save, &ctx);
    app.execute(Command::SaveAs, &ctx);
    assert!(app.view.status.contains("read-only"));
    assert!(app.editor.files[1].path.is_none());
    assert!(!app.editor.has_unsaved_changes());
}

#[test]
fn preview_renders_every_construct_at_narrow_default_and_wide_widths() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/readme.md", SAMPLE);
    app.execute(Command::OpenPreview(source), &ctx);
    for width in [160.0, 600.0, 1600.0] {
        editor_frame_at_width(&mut app, &ctx, vec![], width);
    }
}

#[test]
fn preview_images_resolve_against_the_source_and_load_in_the_background() {
    let dir = tempfile::tempdir().unwrap();
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
    image.save(dir.path().join("pic.png")).unwrap();
    let (mut app, ctx) = fixture();
    let source = open_document(
        &mut app,
        dir.path().join("readme.md").to_str().unwrap(),
        "![pic](pic.png) ![remote](https://example.com/x.png) ![gone](missing.png)",
    );
    app.execute(Command::OpenPreview(source), &ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
        app.maintain_previews(&ctx);
        let loaded = app
            .view
            .preview
            .images
            .values()
            .all(|image| !matches!(image, crate::app::preview::PreviewImage::Loading));
        if loaded && app.view.preview.images.len() == 2 {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "images never loaded");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let states: Vec<_> = app.view.preview.images.iter().collect();
    assert!(states.iter().any(|(path, image)| path.ends_with("pic.png")
        && matches!(image, crate::app::preview::PreviewImage::Ready { .. })));
    assert!(
        states
            .iter()
            .any(|(path, image)| path.ends_with("missing.png")
                && matches!(image, crate::app::preview::PreviewImage::Failed))
    );
    assert!(
        !states
            .iter()
            .any(|(path, _)| path.to_string_lossy().contains("example.com")),
        "remote images are never fetched"
    );
}

#[test]
fn links_open_workspace_files_scroll_to_anchors_and_refuse_unknown_schemes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs/other.md"), "# Other\n").unwrap();
    let (mut app, ctx) = fixture();
    app.file_system = Box::new(crate::services::LocalFileSystem);
    let source = open_document(
        &mut app,
        dir.path().join("readme.md").to_str().unwrap(),
        "[a](docs/other.md#x) [b](#later)\n\n## Later\n",
    );
    app.execute(Command::OpenPreview(source), &ctx);
    let preview = app.editor.files[1].id;

    app.execute(Command::OpenPreviewLink(preview, "#later".into()), &ctx);
    assert_eq!(
        app.view.preview.anchor_request,
        Some((preview, "later".into()))
    );
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
    assert!(app.view.preview.anchor_request.is_none(), "anchor consumed");

    app.execute(
        Command::OpenPreviewLink(preview, "docs/other.md#x".into()),
        &ctx,
    );
    assert!(
        app.editor
            .files
            .iter()
            .any(|file| file.path.as_deref() == Some(&dir.path().join("docs/other.md")))
    );

    let count = app.editor.files.len();
    app.execute(Command::OpenPreviewLink(preview, "missing.md".into()), &ctx);
    assert!(app.view.status.contains("does not exist"));
    app.execute(
        Command::OpenPreviewLink(preview, "javascript:alert(1)".into()),
        &ctx,
    );
    assert!(app.view.status.contains("Could not open"));
    assert_eq!(app.editor.files.len(), count);
}

#[test]
fn relative_paths_resolve_against_the_source_directory() {
    use crate::app::preview::resolve_relative;
    let source = std::path::Path::new("/w/docs/a.md");
    assert_eq!(
        resolve_relative(source, "img/x.png"),
        Some(PathBuf::from("/w/docs/img/x.png"))
    );
    assert_eq!(
        resolve_relative(source, "../x.png"),
        Some(PathBuf::from("/w/x.png"))
    );
    assert_eq!(resolve_relative(source, ""), None);
}
