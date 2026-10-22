//! Visual editor tabs: availability, open/focus, shared buffer, undo, lifetime.
use super::editor::editor_frame_at_width;
use super::*;
use crate::{
    app::visual_editor::{TableEdit, TableSource},
    model::{Document, DocumentId, previewable::AlternateView},
};
use std::path::PathBuf;

const SAMPLE: &str = "name,qty\r\napple,3\r\n\"pear, green\",5\r\n";

fn fixture() -> (LumenApp, egui::Context) {
    let (app, _) = test_app(TestDialogs::default());
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    (app, ctx)
}

fn open_document(app: &mut LumenApp, path: &str, text: &str) -> DocumentId {
    let document = Document::from_path(PathBuf::from(path), text.into());
    let id = document.id;
    app.editor.files.push(document);
    app.editor.activate(id);
    id
}

fn visual_tabs(app: &LumenApp) -> usize {
    app.editor
        .files
        .iter()
        .filter(|file| matches!(file.view_kind, Some(AlternateView::Visual(_))))
        .count()
}

fn text_of(app: &LumenApp, id: DocumentId) -> String {
    let index = app.editor.index_of(id).unwrap();
    app.editor.files[index].text.clone()
}

fn edit(app: &mut LumenApp, ctx: &egui::Context, source: DocumentId, edit: TableEdit) {
    app.execute(Command::EditTable(source, edit), ctx);
}

fn set(row: usize, column: usize, value: &str) -> TableEdit {
    TableEdit::SetCells {
        row,
        column,
        grid: vec![vec![value.into()]],
    }
}

#[test]
fn only_csv_and_tsv_offer_a_visual_editor_and_markdown_keeps_its_preview() {
    let (mut app, ctx) = fixture();
    open_document(&mut app, "/w/data.csv", SAMPLE);
    open_document(&mut app, "/w/data.tsv", "a\tb\n");
    let code = open_document(&mut app, "/w/main.rs", "fn main() {}");
    let markdown = open_document(&mut app, "/w/readme.md", "# Hi\n");
    assert_eq!(app.editor.files[0].alternate_views().len(), 1);
    assert!(
        app.editor.files[0].visual_kind().is_some() && app.editor.files[0].preview_kind().is_none()
    );
    assert!(app.editor.files[1].visual_kind().is_some());
    assert!(app.editor.files[2].alternate_views().is_empty());
    assert!(
        app.editor.files[3].visual_kind().is_none() && app.editor.files[3].preview_kind().is_some()
    );
    app.execute(Command::OpenVisualEditor(code), &ctx);
    app.execute(Command::OpenVisualEditor(markdown), &ctx);
    assert_eq!(visual_tabs(&app), 0);
    assert_eq!(app.view.status, "This file type has no visual editor");
}

#[test]
fn opening_twice_focuses_the_existing_tab_placed_after_its_source() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/data.csv", SAMPLE);
    open_document(&mut app, "/w/other.txt", "x");
    app.execute(Command::OpenVisualEditor(source), &ctx);
    let tab = &app.editor.files[1];
    assert_eq!(tab.view_of, Some(source));
    assert_eq!(tab.name, "Table: data.csv");
    assert!(!tab.dirty && tab.path.is_none());
    assert_eq!(app.editor.active_id(), Some(tab.id));
    assert!(tab.alternate_views().is_empty(), "no view of a view");
    app.editor.activate(source);
    app.execute(Command::OpenActiveVisualEditor, &ctx);
    assert_eq!(visual_tabs(&app), 1);
    assert_eq!(app.editor.active_id(), Some(app.editor.files[1].id));
}

#[test]
fn table_edits_change_the_shared_buffer_dirty_flag_and_undo_history() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/data.csv", SAMPLE);
    app.execute(Command::OpenVisualEditor(source), &ctx);
    edit(&mut app, &ctx, source, set(1, 1, "9"));
    let edited = "name,qty\r\napple,9\r\n\"pear, green\",5\r\n";
    assert_eq!(text_of(&app, source), edited);
    assert!(app.editor.files[0].dirty);
    assert!(
        !app.editor.files[1].dirty,
        "dirty state belongs to the document"
    );
    edit(&mut app, &ctx, source, TableEdit::InsertColumn(1));
    assert_ne!(text_of(&app, source), edited);
    edit(&mut app, &ctx, source, TableEdit::Undo);
    assert_eq!(text_of(&app, source), edited);
    edit(&mut app, &ctx, source, TableEdit::Undo);
    assert_eq!(text_of(&app, source), SAMPLE);
    assert!(!app.editor.files[0].dirty, "undo returns to the saved text");
    edit(&mut app, &ctx, source, TableEdit::Redo);
    assert!(app.editor.files[0].dirty);
}

#[test]
fn sorting_rewrites_the_buffer_and_keeps_the_header() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/data.csv", "n,v\nb,2\na,1\nc,3\n");
    edit(
        &mut app,
        &ctx,
        source,
        TableEdit::SortRows {
            first_row: 1,
            column: 0,
            descending: false,
        },
    );
    assert_eq!(text_of(&app, source), "n,v\na,1\nb,2\nc,3\n");
}

#[test]
fn text_edits_show_up_in_the_table_on_the_next_frame() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/data.csv", "a,b\n");
    app.execute(Command::OpenVisualEditor(source), &ctx);
    editor_frame_at_width(&mut app, &ctx, vec![], 900.0);
    let rows = |app: &mut LumenApp| {
        let file = &app.editor.files[0];
        match app
            .view
            .visual
            .table_mut(source)
            .table(&file.text, file.revision, Some("csv"))
        {
            TableSource::Ready(table) => table.row_count(),
            _ => panic!("small tables parse immediately"),
        }
    };
    assert_eq!(rows(&mut app), 1);
    let file = &mut app.editor.files[0];
    file.enable_shared_editing();
    file.editing
        .as_mut()
        .unwrap()
        .replace_document("a,b\nc,d\ne,f\n")
        .unwrap();
    editor_frame_at_width(&mut app, &ctx, vec![], 900.0);
    assert_eq!(rows(&mut app), 3);
}

#[test]
fn rename_retitles_and_closing_either_side_never_prompts() {
    let (mut app, ctx) = fixture();
    let source = open_document(&mut app, "/w/data.csv", SAMPLE);
    app.execute(Command::OpenVisualEditor(source), &ctx);
    app.editor.files[0].relocate(PathBuf::from("/w/people.csv"));
    app.maintain_previews(&ctx);
    assert_eq!(app.editor.files[1].name, "Table: people.csv");

    let tab = app.editor.files[1].id;
    edit(&mut app, &ctx, source, set(0, 0, "changed"));
    app.execute(Command::RequestClose(tab), &ctx);
    assert!(app.view.pending_close.is_none());
    assert_eq!(app.editor.files.len(), 1);
    assert!(
        app.editor.files[0].dirty,
        "closing the table keeps the unsaved source"
    );

    app.execute(Command::OpenVisualEditor(source), &ctx);
    app.editor.close_document(source);
    app.maintain_previews(&ctx);
    assert!(app.editor.files.is_empty());
}

#[test]
fn malformed_files_refuse_table_edits_and_lose_nothing() {
    let (mut app, ctx) = fixture();
    let text = "a,b\n\"oops,1\n";
    let source = open_document(&mut app, "/w/bad.csv", text);
    app.execute(Command::OpenVisualEditor(source), &ctx);
    edit(&mut app, &ctx, source, set(0, 0, "x"));
    assert_eq!(text_of(&app, source), text);
    assert!(!app.editor.files[0].dirty);
    assert!(app.view.status.contains("unterminated quote"));
    // Rendering the read-only table must still work.
    editor_frame_at_width(&mut app, &ctx, vec![], 600.0);
}

#[test]
fn the_table_renders_at_narrow_default_and_wide_widths() {
    let (mut app, ctx) = fixture();
    let big: String = (0..500)
        .map(|row| format!("{row},name {row},\"quoted, {row}\",{}\n", row * 3))
        .collect();
    let source = open_document(&mut app, "/w/big.csv", &big);
    app.execute(Command::OpenVisualEditor(source), &ctx);
    for width in [320.0, 900.0, 1800.0] {
        editor_frame_at_width(&mut app, &ctx, vec![], width);
    }
    assert_eq!(visual_tabs(&app), 1);
}
