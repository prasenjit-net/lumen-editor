use super::*;
use crate::{
    app::workspace_search::ReplaceScope,
    model::search::SearchOptions,
    services::workspace_search::{SearchFile, SearchHit, SearchRequest, SearchResult},
};

#[cfg(feature = "shared-editor")]
#[test]
fn opening_workspace_hit_selects_match_on_click_release() {
    use lumen_editor_core::Selection;

    let (mut app, _, path) = prepared();
    app.view.shared_editor = true;
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.open_workspace_hit(path.clone(), 7..12);
    assert_eq!(app.editor.active().unwrap().path.as_ref(), Some(&path));
    super::editor::editor_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: egui::pos2(30.0, 180.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let document = app.editor.active().unwrap();
    assert_eq!(
        document.editing.as_ref().unwrap().selections().selections(),
        &[Selection {
            anchor: 7,
            head: 12
        }]
    );
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(crate::app::editor_action::text_widget_id(document.id))
    );
    assert!(!document.dirty);
}

#[cfg(feature = "shared-editor")]
#[test]
fn opening_distant_workspace_hit_reveals_it_in_editor_viewport() {
    let (mut app, storage, path) = prepared();
    app.view.shared_editor = true;
    let text = format!("{}target", "line\n".repeat(100));
    let start = text.len() - "target".len();
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), text.clone());
    app.view.workspace_search.query = "target".into();
    app.view
        .workspace_search
        .completed_request
        .as_mut()
        .unwrap()
        .query = "target".into();
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.open_workspace_hit(path, start..text.len());
    super::editor::editor_frame(&mut app, &ctx, vec![]);
    super::editor::editor_frame(&mut app, &ctx, vec![]);
    let output = app.view.shared_output.as_ref().unwrap();
    assert!(
        output
            .text_clip
            .intersects(output.geometry.caret_rect(start)),
        "distant match must be scrolled into view"
    );
}

fn prepared() -> (LumenApp, MemoryFileSystem, PathBuf) {
    let (mut app, storage) = test_app(TestDialogs::default());
    let path = PathBuf::from("root/a.txt");
    app.editor.workspace = Some("root".into());
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), "café\r\ncafé".into());
    app.view.workspace_search.query = "café".into();
    app.view.workspace_search.replacement = "tea".into();
    app.view.workspace_search.completed_request = Some(SearchRequest {
        root: "root".into(),
        query: "café".into(),
        include: String::new(),
        exclude: String::new(),
        options: SearchOptions::default(),
    });
    app.view.workspace_search.results = Some(SearchResult {
        files: vec![SearchFile {
            path: path.clone(),
            original: "café\r\ncafé".into(),
            hits: vec![
                SearchHit {
                    range: 0..5,
                    line: 1,
                    column: 1,
                    snippet: "café".into(),
                },
                SearchHit {
                    range: 7..12,
                    line: 2,
                    column: 1,
                    snippet: "café".into(),
                },
            ],
        }],
        ..Default::default()
    });
    app.view.workspace_search.selected.insert(path.clone());
    app.view
        .workspace_search
        .preview_selected
        .insert(path.clone());
    app.view.workspace_search.preview_replacement = Some("tea".into());
    app.view.workspace_search.confirmation_open = true;
    (app, storage, path)
}

#[test]
fn clearing_search_resets_results_without_changing_open_files_or_filters() {
    let (mut app, storage, path) = prepared();
    app.open_file(path.clone());
    let document_id = app.editor.active().unwrap().id;
    let search = &mut app.view.workspace_search;
    search.running = true;
    search.include = "*.rs".into();
    search.exclude = "target".into();
    search.match_case = true;
    search.expanded_files.insert(path.clone());
    search.expanded_folders.insert(PathBuf::from("root"));
    search.report.push("earlier error".into());
    search.replaced_files = 2;

    app.clear_workspace_search();

    let search = &app.view.workspace_search;
    assert!(search.query.is_empty());
    assert!(search.results.is_none());
    assert!(search.completed_request.is_none());
    assert!(!search.running);
    assert!(search.progress.is_none());
    assert!(search.selected.is_empty());
    assert!(search.expanded_files.is_empty());
    assert!(search.expanded_folders.is_empty());
    assert!(!search.preview_open);
    assert!(!search.confirmation_open);
    assert!(search.preview_selected.is_empty());
    assert!(search.report.is_empty());
    assert_eq!(search.replaced_files, 0);
    assert_eq!(search.replacement, "tea");
    assert_eq!(search.include, "*.rs");
    assert_eq!(search.exclude, "target");
    assert!(search.match_case);
    assert!(search.focus_query);
    assert_eq!(app.editor.active().unwrap().id, document_id);
    assert_eq!(storage.0.borrow().files[&path], "café\r\ncafé");
}

#[test]
fn effective_settings_and_panel_exclusions_invalidate_old_replace_preview() {
    use lumen_settings_core::Value;

    let (mut app, _, _) = prepared();
    app.view
        .settings
        .user
        .persisted
        .insert("search.exclude".into(), Value::Text("*.tmp".into()));
    app.view.workspace_search.exclude = "*.log".into();
    assert_eq!(app.combined_workspace_exclude(), "*.tmp,*.log");
    app.view.settings.root = Some(PathBuf::from("root"));
    app.view
        .settings
        .project
        .persisted
        .insert("search.exclude".into(), Value::Text("generated".into()));
    assert_eq!(app.combined_workspace_exclude(), "generated,*.log");
    app.view
        .workspace_search
        .completed_request
        .as_mut()
        .unwrap()
        .exclude = app.combined_workspace_exclude();
    app.preview_workspace_replace();
    assert!(app.view.workspace_search.preview_open);

    app.view
        .settings
        .project
        .persisted
        .insert("search.exclude".into(), Value::Text("*.cache".into()));
    app.preview_workspace_replace();
    assert!(!app.view.workspace_search.preview_open);
    assert!(app.view.status.contains("Search inputs changed"));
}

#[test]
fn file_and_folder_replace_actions_limit_the_preview_and_write_scope() {
    let (mut app, storage, root_file) = prepared();
    let nested = PathBuf::from("root/sub/b.txt");
    let deeper = PathBuf::from("root/sub/deep/c.txt");
    for path in [&nested, &deeper] {
        storage
            .0
            .borrow_mut()
            .files
            .insert(path.clone(), "café".into());
        app.view
            .workspace_search
            .results
            .as_mut()
            .unwrap()
            .files
            .push(SearchFile {
                path: path.clone(),
                original: "café".into(),
                hits: vec![SearchHit {
                    range: 0..5,
                    line: 1,
                    column: 1,
                    snippet: "café".into(),
                }],
            });
    }
    app.preview_workspace_replace_scope(ReplaceScope::File(nested.clone()));
    assert_eq!(app.view.workspace_search.selected, [nested.clone()].into());
    app.view.workspace_search.confirmation_open = true;
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&nested], "tea");
    assert_eq!(storage.0.borrow().files[&deeper], "café");
    assert_eq!(storage.0.borrow().files[&root_file], "café\r\ncafé");

    app.preview_workspace_replace_scope(ReplaceScope::Folder(PathBuf::from("root/sub")));
    assert_eq!(
        app.view.workspace_search.selected,
        [nested.clone(), deeper.clone()].into()
    );
    app.view.workspace_search.confirmation_open = true;
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&deeper], "tea");
    assert_eq!(storage.0.borrow().files[&root_file], "café\r\ncafé");
    assert_eq!(app.view.workspace_search.report.len(), 1); // Already replaced file is now stale.
}

#[test]
fn confirmed_replacement_preserves_crlf_and_reports_success() {
    let (mut app, storage, path) = prepared();
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&path], "tea\r\ntea");
    assert_eq!(app.view.workspace_search.replaced_files, 1);
    assert!(app.view.workspace_search.report.is_empty());
}

#[test]
fn stale_disk_and_dirty_open_tab_are_never_overwritten() {
    let (mut app, storage, path) = prepared();
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), "external".into());
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&path], "external");
    assert_eq!(app.view.workspace_search.report.len(), 1);
    let (mut app, storage, path) = prepared();
    app.open_file(path.clone());
    app.editor.files[0].text = "unsaved".into();
    app.editor.files[0].record_text_change();
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&path], "café\r\ncafé");
    assert!(app.editor.files[0].dirty);
    assert_eq!(app.view.workspace_search.report.len(), 1);
}

#[test]
fn failed_write_and_changed_search_inputs_leave_files_intact() {
    let (mut app, storage, path) = prepared();
    storage.0.borrow_mut().fail_writes = true;
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&path], "café\r\ncafé");
    assert_eq!(app.view.workspace_search.report.len(), 1);
    let (mut app, storage, path) = prepared();
    app.view.workspace_search.query = "other".into();
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&path], "café\r\ncafé");
    assert_eq!(app.view.workspace_search.replaced_files, 0);
    let (mut app, storage, path) = prepared();
    app.view.workspace_search.replacement = "surprise".into();
    app.apply_workspace_replace();
    assert_eq!(storage.0.borrow().files[&path], "café\r\ncafé");
    assert_eq!(app.view.workspace_search.replaced_files, 0);
}
