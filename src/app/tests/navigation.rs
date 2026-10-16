use super::*;
use crate::app::view_state::NavigationMode;

#[test]
fn quick_open_reuses_tabs_and_retains_failed_target_for_retry() {
    let (mut app, storage) = test_app(TestDialogs::default());
    let path = PathBuf::from("/project/src/main.rs");
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), "first".into());
    app.view.navigation.mode = Some(NavigationMode::QuickOpen);
    app.open_quick_path(path.clone());
    let id = app.editor.active().unwrap().id;
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.view.navigation.mode, None);
    app.view.navigation.mode = Some(NavigationMode::QuickOpen);
    app.open_quick_path(path.clone());
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.editor.active().unwrap().id, id);
    assert_eq!(app.view.navigation.recent, vec![path]);
    app.view.navigation.mode = Some(NavigationMode::QuickOpen);
    app.open_quick_path("/project/missing.rs".into());
    assert_eq!(app.view.navigation.mode, Some(NavigationMode::QuickOpen));
    assert!(
        app.view
            .navigation
            .error
            .as_deref()
            .unwrap()
            .contains("Could not open")
    );
    assert_eq!(app.editor.active().unwrap().id, id);
}

#[test]
fn go_to_line_targets_active_document_without_editing_it() {
    let (mut app, storage) = test_app(TestDialogs::default());
    let path = PathBuf::from("/project/example.rs");
    let text = "first\r\ne\u{301}x\r\nlast";
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), text.into());
    app.open_file(path);
    let document = app.editor.active().unwrap().id;
    app.view.navigation.mode = Some(NavigationMode::GoToLine);
    app.go_to_line(document, "2:2");
    assert_eq!(
        app.view.pending_goto,
        Some((document, "first\r\ne\u{301}".len()))
    );
    assert_eq!(app.editor.active().unwrap().text, text);
    assert!(!app.editor.active().unwrap().dirty);
    assert_eq!(app.view.navigation.mode, None);
    app.view.navigation.mode = Some(NavigationMode::GoToLine);
    app.go_to_line(document, "bad");
    assert!(app.view.navigation.error.is_some());
    assert_eq!(app.view.navigation.mode, Some(NavigationMode::GoToLine));

    let other = PathBuf::from("/project/other.rs");
    storage
        .0
        .borrow_mut()
        .files
        .insert(other.clone(), "other".into());
    app.open_file(other);
    app.view.pending_goto = None;
    app.go_to_line(document, "1");
    assert!(app.view.pending_goto.is_none());
    assert!(
        app.view
            .navigation
            .error
            .as_deref()
            .unwrap()
            .contains("Editor changed")
    );
}

#[test]
fn recent_files_survive_restart_and_missing_targets_are_pruned() {
    let temp = tempfile::tempdir().unwrap();
    let settings = temp.path().join(".lumen/settings.toml");
    let path = temp.path().join("main.rs");
    let (mut app, storage) = test_app(TestDialogs::default());
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), "body".into());
    app.initialize_recent_files(Some(settings.clone()));
    app.open_file(path.clone());
    assert_eq!(app.view.navigation.recent, vec![path.clone()]);

    let (mut reopened, _) = test_app(TestDialogs::default());
    reopened.initialize_recent_files(Some(settings.clone()));
    assert_eq!(reopened.view.navigation.recent, vec![path.clone()]);
    reopened.view.navigation.mode = Some(NavigationMode::QuickOpen);
    reopened.open_quick_path(path.clone());
    assert!(reopened.view.navigation.recent.is_empty());
    assert!(reopened.view.navigation.error.is_some());

    let (mut after_missing, _) = test_app(TestDialogs::default());
    after_missing.initialize_recent_files(Some(settings));
    assert!(after_missing.view.navigation.recent.is_empty());
}

fn app_with_folders(names: &[&str]) -> (crate::app::LumenApp, MemoryFileSystem, PathBuf) {
    let temp = tempfile::tempdir().unwrap().keep();
    let settings = temp.join(".lumen/settings.toml");
    let (mut app, storage) = test_app(TestDialogs::default());
    for name in names {
        storage
            .0
            .borrow_mut()
            .directories
            .insert(PathBuf::from(name));
    }
    app.initialize_recent_files(Some(settings.clone()));
    (app, storage, settings)
}

#[test]
fn opened_folders_are_remembered_newest_first_without_duplicates_and_survive_restart() {
    let (mut app, _, settings) = app_with_folders(&["/a", "/b", "/c"]);
    for folder in ["/a", "/b", "/a", "/c"] {
        app.open_folder_path(folder.into());
    }
    let expected: Vec<PathBuf> = ["/c", "/a", "/b"].iter().map(PathBuf::from).collect();
    assert_eq!(app.view.navigation.recent_folders, expected);

    let (mut reopened, _) = test_app(TestDialogs::default());
    reopened.initialize_recent_files(Some(settings));
    assert_eq!(reopened.view.navigation.recent_folders, expected);
    assert!(
        reopened.editor.workspace.is_none(),
        "history never auto-opens"
    );
}

#[test]
fn the_folder_history_is_capped() {
    let names: Vec<String> = (0..20).map(|n| format!("/f{n}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (mut app, _, _) = app_with_folders(&refs);
    for name in &names {
        app.open_folder_path(name.into());
    }
    assert_eq!(app.view.navigation.recent_folders.len(), 15);
    assert_eq!(app.view.navigation.recent_folders[0], PathBuf::from("/f19"));
    assert_eq!(app.view.navigation.recent_folders[14], PathBuf::from("/f5"));
}

#[test]
fn a_missing_folder_reports_prunes_and_changes_nothing_else() {
    let (mut app, _, settings) = app_with_folders(&["/here"]);
    app.open_folder_path("/here".into());
    app.editor.new_file();
    app.view.expanded_dirs.insert("/here/sub".into());
    app.view.navigation.recent_folders.push("/gone".into());
    app.open_folder_path("/gone".into());
    assert!(app.view.status.starts_with("Folder not found"));
    assert_eq!(app.editor.workspace, Some(PathBuf::from("/here")));
    assert_eq!(app.editor.files.len(), 1);
    assert!(app.view.expanded_dirs.contains(&PathBuf::from("/here/sub")));
    assert_eq!(
        app.view.navigation.recent_folders,
        vec![PathBuf::from("/here")]
    );
    let (mut reopened, _) = test_app(TestDialogs::default());
    reopened.initialize_recent_files(Some(settings));
    assert_eq!(
        reopened.view.navigation.recent_folders,
        vec![PathBuf::from("/here")],
        "the pruned entry stays gone after a restart"
    );
}

#[test]
fn clearing_history_forgets_files_and_folders_on_disk_too() {
    let (mut app, _, settings) = app_with_folders(&["/a"]);
    app.open_folder_path("/a".into());
    app.remember_recent_file(&PathBuf::from("/a/x.rs"));
    app.clear_recent_history();
    assert!(app.view.navigation.recent.is_empty());
    assert!(app.view.navigation.recent_folders.is_empty());
    let (mut reopened, _) = test_app(TestDialogs::default());
    reopened.initialize_recent_files(Some(settings));
    assert!(reopened.view.navigation.recent.is_empty());
    assert!(reopened.view.navigation.recent_folders.is_empty());
}

#[test]
fn reopen_last_folder_is_opt_in_and_skips_missing_or_already_open_workspaces() {
    let (mut app, _, _) = app_with_folders(&["/last"]);
    app.open_folder_path("/last".into());
    app.close_folder();
    assert!(app.editor.workspace.is_none());
    // Off by default.
    app.reopen_last_folder_if_enabled();
    assert!(app.editor.workspace.is_none());
    app.view.settings.user.persisted.insert(
        "workbench.reopen_last_folder".into(),
        lumen_settings_core::Value::Bool(true),
    );
    app.reopen_last_folder_if_enabled();
    assert_eq!(app.editor.workspace, Some(PathBuf::from("/last")));
    // An already-open workspace is never replaced.
    app.view
        .navigation
        .remember_folder(&PathBuf::from("/other"));
    app.reopen_last_folder_if_enabled();
    assert_eq!(app.editor.workspace, Some(PathBuf::from("/last")));
    // A vanished folder is reported, pruned, and opens nothing.
    app.close_folder();
    app.view.navigation.recent_folders = vec!["/vanished".into()];
    app.reopen_last_folder_if_enabled();
    assert!(app.editor.workspace.is_none());
    assert!(app.view.status.starts_with("Folder not found"));
    assert!(app.view.navigation.recent_folders.is_empty());
}
