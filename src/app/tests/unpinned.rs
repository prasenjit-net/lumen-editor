use super::*;

fn app_with(files: &[&str]) -> LumenApp {
    let (app, storage) = test_app(TestDialogs::default());
    for name in files {
        storage
            .0
            .borrow_mut()
            .files
            .insert(PathBuf::from(name), format!("text of {name}"));
    }
    app
}

fn paths(app: &LumenApp) -> Vec<String> {
    app.editor
        .files
        .iter()
        .map(|file| file.path.as_ref().unwrap().display().to_string())
        .collect()
}

#[test]
fn single_click_replaces_the_unpinned_tab_in_place() {
    let mut app = app_with(&["/w/a", "/w/b", "/w/c"]);
    app.open_file_unpinned("/w/a".into());
    app.open_file("/w/b".into());
    app.open_file_unpinned("/w/c".into());
    assert_eq!(paths(&app), ["/w/c", "/w/b"], "replaced at the same index");
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(Path::new("/w/c"))
    );
    assert!(app.editor.is_unpinned(app.editor.files[0].id));
    assert!(!app.editor.is_unpinned(app.editor.files[1].id));
}

#[test]
fn double_click_pins_and_a_pinned_tab_is_never_demoted() {
    let mut app = app_with(&["/w/a", "/w/b"]);
    app.open_file_unpinned("/w/a".into());
    app.open_file("/w/a".into());
    assert!(!app.editor.is_unpinned(app.editor.files[0].id));
    app.open_file_unpinned("/w/a".into());
    assert!(!app.editor.is_unpinned(app.editor.files[0].id));
    app.open_file_unpinned("/w/b".into());
    assert_eq!(paths(&app), ["/w/a", "/w/b"], "pinned tab was kept");
    assert_eq!(app.editor.files.len(), 2);
}

#[test]
fn reopening_the_unpinned_file_keeps_one_unpinned_tab() {
    let mut app = app_with(&["/w/a"]);
    app.open_file_unpinned("/w/a".into());
    app.open_file_unpinned("/w/a".into());
    assert_eq!(app.editor.files.len(), 1);
    assert!(app.editor.is_unpinned(app.editor.files[0].id));
}

#[test]
fn editing_pins_before_the_next_single_click() {
    let mut app = app_with(&["/w/a", "/w/b"]);
    app.open_file_unpinned("/w/a".into());
    app.editor.files[0].text.push('!');
    app.editor.files[0].record_text_change();
    app.open_file_unpinned("/w/b".into());
    assert_eq!(paths(&app), ["/w/a", "/w/b"], "dirty tab is never replaced");
    assert!(app.editor.files[0].dirty);
}

#[test]
fn navigation_does_not_pin() {
    let mut app = app_with(&["/w/a", "/w/b"]);
    app.open_file_unpinned("/w/a".into());
    app.editor.pin_touched();
    app.editor.set_active_index(Some(0));
    assert!(app.editor.is_unpinned(app.editor.files[0].id));
}

#[test]
fn the_setting_off_makes_every_open_pinned() {
    let mut app = app_with(&["/w/a", "/w/b"]);
    app.view.settings.user.persisted =
        lumen_settings_core::parse(Some("[workbench]\nenable_preview = false\n"))
            .unwrap()
            .0;
    app.open_file_unpinned("/w/a".into());
    app.open_file_unpinned("/w/b".into());
    assert_eq!(app.editor.files.len(), 2);
    assert!(
        app.editor
            .files
            .iter()
            .all(|f| !app.editor.is_unpinned(f.id))
    );
}

#[test]
fn pin_commands_make_the_tab_permanent() {
    let mut app = app_with(&["/w/a", "/w/b"]);
    app.open_file_unpinned("/w/a".into());
    let id = app.editor.files[0].id;
    Command::PinTab(id).apply_local(&mut app.editor, &mut app.view);
    assert!(!app.editor.is_unpinned(id));
    app.open_file_unpinned("/w/b".into());
    Command::PinActiveTab.apply_local(&mut app.editor, &mut app.view);
    assert!(
        app.editor
            .files
            .iter()
            .all(|f| !app.editor.is_unpinned(f.id))
    );
}
