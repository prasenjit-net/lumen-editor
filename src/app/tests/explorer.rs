use super::{MemoryFileSystem, TestDialogs, configure_ui, edit_active, key_event, test_app};
use crate::app::{LumenApp, command::Command, view_state::ExplorerSelection};
use eframe::egui;
use std::path::{Path, PathBuf};

fn workspace() -> (LumenApp, MemoryFileSystem) {
    let (mut app, storage) = test_app(TestDialogs::default());
    app.editor.workspace = Some("project".into());
    app.view.expanded_dirs.insert("project".into());
    storage
        .0
        .borrow_mut()
        .directories
        .extend(["project", "project/src", "project/dest"].map(PathBuf::from));
    storage
        .0
        .borrow_mut()
        .files
        .insert("project/src/main.rs".into(), "original on disk".into());
    (app, storage)
}

fn confirm_name(app: &mut LumenApp, name: &str) {
    app.view.explorer_edit.as_mut().unwrap().name = name.into();
    app.commit_explorer_edit();
}

#[test]
fn native_new_text_file_stays_untitled_even_in_a_workspace() {
    let (mut app, _) = workspace();
    app.execute(Command::NewUntitled, &egui::Context::default());
    assert!(app.editor.active().unwrap().path.is_none());
    assert!(app.view.explorer_edit.is_none());
    assert_eq!(Command::from_menu_id("new"), Some(Command::NewUntitled));
}

#[test]
fn new_folder_uses_selection_and_does_not_open_an_editor() {
    let (mut app, storage) = workspace();
    app.view.explorer_selection = Some(ExplorerSelection::Directory("project/src".into()));
    app.new_folder();
    confirm_name(&mut app, "components");
    assert!(
        storage
            .0
            .borrow()
            .directories
            .contains(Path::new("project/src/components"))
    );
    assert!(app.editor.files.is_empty());
    assert_eq!(
        app.view.explorer_selection.as_ref().unwrap().path(),
        Path::new("project/src/components")
    );
}

#[test]
fn explorer_new_file_accepts_nested_paths_and_trailing_separator_folders() {
    let (mut app, storage) = workspace();
    app.new_file();
    confirm_name(&mut app, "nested/deep/module.rs");
    assert!(
        storage
            .0
            .borrow()
            .files
            .contains_key(Path::new("project/nested/deep/module.rs"))
    );
    assert_eq!(app.editor.active().unwrap().language, "Rust");
    app.view.explorer_selection = None;
    app.new_file();
    confirm_name(&mut app, "folder-only/");
    assert!(
        storage
            .0
            .borrow()
            .directories
            .contains(Path::new("project/folder-only"))
    );
    assert_eq!(app.editor.files.len(), 1);
}

#[test]
fn rename_file_updates_metadata_but_keeps_unsaved_text_and_dirty_flag() {
    let (mut app, storage) = workspace();
    app.open_file("project/src/main.rs".into());
    edit_active(&mut app, "unsaved changes");
    app.rename_path("project/src/main.rs".into());
    confirm_name(&mut app, "notes.md");
    let file = app.editor.active().unwrap();
    assert_eq!(
        file.path.as_deref(),
        Some(Path::new("project/src/notes.md"))
    );
    assert_eq!(file.language, "Markdown");
    assert_eq!(file.text, "unsaved changes");
    assert!(file.dirty);
    assert_eq!(
        storage.0.borrow().files[Path::new("project/src/notes.md")],
        "original on disk"
    );
    assert!(
        !storage
            .0
            .borrow()
            .files
            .contains_key(Path::new("project/src/main.rs"))
    );
    app.save_active();
    assert_eq!(
        storage.0.borrow().files[Path::new("project/src/notes.md")],
        "unsaved changes"
    );
}

#[test]
fn rename_folder_relocates_descendant_tabs_and_expansion_state() {
    let (mut app, _) = workspace();
    app.open_file("project/src/main.rs".into());
    edit_active(&mut app, "keep me");
    app.view.expanded_dirs.insert("project/src".into());
    app.rename_path("project/src".into());
    confirm_name(&mut app, "lib");
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(Path::new("project/lib/main.rs"))
    );
    assert!(app.editor.active().unwrap().dirty);
    assert!(app.view.expanded_dirs.contains(Path::new("project/lib")));
    assert!(!app.view.expanded_dirs.contains(Path::new("project/src")));
}

#[test]
fn rename_collision_and_io_failure_leave_original_tree_and_buffer_intact() {
    let (mut app, storage) = workspace();
    app.open_file("project/src/main.rs".into());
    storage
        .0
        .borrow_mut()
        .files
        .insert("project/src/taken.rs".into(), "other content".into());
    app.rename_path("project/src/main.rs".into());
    confirm_name(&mut app, "taken.rs");
    assert!(app.view.explorer_edit.as_ref().unwrap().error.is_some());
    assert_eq!(
        storage.0.borrow().files[Path::new("project/src/taken.rs")],
        "other content"
    );
    storage.0.borrow_mut().fail_writes = true;
    confirm_name(&mut app, "renamed.rs");
    assert!(app.view.explorer_edit.is_some());
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(Path::new("project/src/main.rs"))
    );
}

#[test]
fn workspace_root_and_outside_paths_cannot_be_renamed_or_deleted() {
    let (mut app, _) = workspace();
    for path in ["project", "outside.txt", "project/../outside"] {
        app.rename_path(path.into());
        assert!(app.view.explorer_edit.is_none());
        app.request_delete(path.into());
        assert!(app.view.delete_prompt.is_none());
    }
}

#[test]
fn deletion_requires_confirmation_and_preserves_dirty_buffers_as_untitled() {
    let (mut app, storage) = workspace();
    app.open_file("project/src/main.rs".into());
    edit_active(&mut app, "irreplaceable edits");
    app.request_delete("project/src".into());
    assert!(
        storage
            .0
            .borrow()
            .files
            .contains_key(Path::new("project/src/main.rs"))
    );
    app.confirm_delete();
    assert!(
        !storage
            .0
            .borrow()
            .directories
            .contains(Path::new("project/src"))
    );
    let file = app.editor.active().unwrap();
    assert!(file.path.is_none());
    assert_eq!(file.text, "irreplaceable edits");
    assert!(file.dirty);
    assert!(app.view.delete_prompt.is_none());
}

#[test]
fn deletion_failure_leaves_confirmation_and_open_buffers_unchanged() {
    let (mut app, storage) = workspace();
    app.open_file("project/src/main.rs".into());
    app.request_delete("project/src/main.rs".into());
    storage.0.borrow_mut().fail_writes = true;
    app.confirm_delete();
    assert!(app.view.delete_prompt.as_ref().unwrap().error.is_some());
    assert_eq!(app.editor.files.len(), 1);
    assert!(
        storage
            .0
            .borrow()
            .files
            .contains_key(Path::new("project/src/main.rs"))
    );
}

#[test]
fn copied_entries_use_unique_names_and_never_replace_an_existing_copy() {
    let (mut app, storage) = workspace();
    app.view.explorer_selection = Some(ExplorerSelection::Directory("project/src".into()));
    app.explorer_copy("project/src/main.rs".into(), false);
    app.explorer_paste();
    app.view.explorer_selection = Some(ExplorerSelection::Directory("project/src".into()));
    app.explorer_paste();
    for path in [
        "project/src/main.rs",
        "project/src/main copy.rs",
        "project/src/main copy 2.rs",
    ] {
        assert_eq!(
            storage.0.borrow().files[Path::new(path)],
            "original on disk"
        );
    }
}

#[test]
fn cut_paste_moves_files_and_updates_dirty_editor_paths() {
    let (mut app, storage) = workspace();
    app.open_file("project/src/main.rs".into());
    edit_active(&mut app, "dirty text");
    app.explorer_copy("project/src/main.rs".into(), true);
    app.view.explorer_selection = Some(ExplorerSelection::Directory("project/dest".into()));
    app.explorer_paste();
    assert_eq!(
        app.editor.active().unwrap().path.as_deref(),
        Some(Path::new("project/dest/main.rs"))
    );
    assert!(app.editor.active().unwrap().dirty);
    assert!(app.view.explorer_clipboard.is_none());
    assert!(
        !storage
            .0
            .borrow()
            .files
            .contains_key(Path::new("project/src/main.rs"))
    );
}

#[test]
fn paste_rejects_folder_into_itself_and_move_collisions() {
    let (mut app, storage) = workspace();
    app.explorer_copy("project/src".into(), false);
    app.view.explorer_selection = Some(ExplorerSelection::Directory("project/src".into()));
    app.explorer_paste();
    assert!(app.view.status.contains("inside itself"));
    storage
        .0
        .borrow_mut()
        .files
        .insert("project/dest/main.rs".into(), "destination".into());
    app.explorer_copy("project/src/main.rs".into(), true);
    app.view.explorer_selection = Some(ExplorerSelection::Directory("project/dest".into()));
    app.explorer_paste();
    assert_eq!(
        storage.0.borrow().files[Path::new("project/dest/main.rs")],
        "destination"
    );
    assert!(app.view.explorer_clipboard.is_some());
}

#[test]
fn escape_cancels_delete_without_calling_storage() {
    let (mut app, storage) = workspace();
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    app.request_delete("project/src".into());
    for events in [vec![], vec![key_event(egui::Key::Escape)]] {
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| app.render_component(ctx, |workbench| workbench.delete_dialog(ctx)),
        );
    }
    assert!(app.view.delete_prompt.is_none());
    assert!(
        storage
            .0
            .borrow()
            .directories
            .contains(Path::new("project/src"))
    );
}
