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
