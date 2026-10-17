use super::{TestDialogs, test_app};
use crate::cli::Target;
use std::path::PathBuf;

fn file(path: &str, line: Option<usize>, column: Option<usize>) -> Target {
    Target {
        path: path.into(),
        line,
        column,
    }
}

#[test]
fn a_file_argument_opens_directly_and_the_last_one_named_is_shown() {
    let (mut app, storage) = test_app(TestDialogs::default());
    for (name, text) in [("one.txt", "1"), ("two.txt", "2")] {
        storage
            .0
            .borrow_mut()
            .files
            .insert(name.into(), text.into());
    }
    app.open_command_line_targets(vec![
        file("one.txt", None, None),
        file("two.txt", None, None),
    ]);
    assert_eq!(app.editor.files.len(), 2);
    assert_eq!(app.editor.active().unwrap().name, "two.txt");
    assert_eq!(app.view.status, "Opened 2 items from the command line");
    assert!(app.editor.workspace.is_none());
}

#[test]
fn a_folder_argument_becomes_the_workspace_before_files_open() {
    let (mut app, storage) = test_app(TestDialogs::default());
    storage.0.borrow_mut().directories.insert("proj".into());
    storage
        .0
        .borrow_mut()
        .files
        .insert("proj/a.txt".into(), "A".into());
    // The file is named first, but the workspace is still set up first.
    app.open_command_line_targets(vec![
        file("proj/a.txt", None, None),
        file("proj", None, None),
    ]);
    assert_eq!(app.editor.workspace, Some(PathBuf::from("proj")));
    assert_eq!(app.editor.files.len(), 1);
    assert!(app.view.expanded_dirs.contains(&PathBuf::from("proj")));
}

#[test]
fn only_the_first_folder_opens_and_extra_ones_are_reported() {
    let (mut app, storage) = test_app(TestDialogs::default());
    for dir in ["first", "second"] {
        storage.0.borrow_mut().directories.insert(dir.into());
    }
    app.open_command_line_targets(vec![file("first", None, None), file("second", None, None)]);
    assert_eq!(app.editor.workspace, Some(PathBuf::from("first")));
    assert!(
        app.view.status.contains("only one folder"),
        "{}",
        app.view.status
    );
    assert!(app.view.status.contains("second"));
}

#[test]
fn a_missing_file_is_reported_but_does_not_stop_the_others() {
    let (mut app, storage) = test_app(TestDialogs::default());
    storage
        .0
        .borrow_mut()
        .files
        .insert("real.txt".into(), "ok".into());
    app.open_command_line_targets(vec![
        file("ghost.txt", None, None),
        file("real.txt", None, None),
    ]);
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.editor.active().unwrap().name, "real.txt");
    assert!(
        app.view.status.starts_with("Could not open"),
        "{}",
        app.view.status
    );
    assert!(app.view.status.contains("ghost.txt"));
    assert!(!app.view.status.contains("real.txt"));
}

#[test]
fn a_line_and_column_place_the_caret_in_the_opened_file() {
    let (mut app, storage) = test_app(TestDialogs::default());
    storage
        .0
        .borrow_mut()
        .files
        .insert("code.rs".into(), "fn a() {}\nlet é = 1;\n".into());
    app.open_command_line_targets(vec![file("code.rs", Some(2), Some(6))]);
    let document = app.editor.active().unwrap();
    // Line 2 starts at byte 10; column 6 is after `let é` (the é is two bytes).
    assert_eq!(app.view.pending_goto, Some((document.id, 10 + 6)));
    assert!(document.text.is_char_boundary(16));
}

#[test]
fn opening_an_already_open_file_again_just_activates_it_at_the_position() {
    let (mut app, storage) = test_app(TestDialogs::default());
    storage
        .0
        .borrow_mut()
        .files
        .insert("a.txt".into(), "x\ny\n".into());
    storage
        .0
        .borrow_mut()
        .files
        .insert("b.txt".into(), "z".into());
    app.open_command_line_targets(vec![
        file("a.txt", None, None),
        file("b.txt", None, None),
        file("a.txt", Some(2), None),
    ]);
    assert_eq!(app.editor.files.len(), 2, "no duplicate tab");
    assert_eq!(app.editor.active().unwrap().name, "a.txt");
    assert_eq!(app.view.pending_goto.map(|(_, byte)| byte), Some(2));
}

#[test]
fn no_targets_changes_nothing() {
    let (mut app, _) = test_app(TestDialogs::default());
    let before = app.view.status.clone();
    app.open_command_line_targets(Vec::new());
    assert!(app.editor.files.is_empty());
    assert_eq!(app.view.status, before);
}
