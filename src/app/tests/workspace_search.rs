use super::*;
use crate::{
    app::workspace_search::ReplaceScope,
    model::search::SearchOptions,
    services::workspace_search::{SearchFile, SearchHit, SearchRequest, SearchResult},
};

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
