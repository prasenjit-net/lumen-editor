use super::*;
use crate::services::recovery::RecoveryDocument;
use lumen_settings_core::Value;

fn dirty_file() -> (LumenApp, MemoryFileSystem) {
    let (mut app, storage) = test_app(TestDialogs::default());
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "saved".into());
    app.open_file("note.txt".into());
    app.editor.files[0].text = "edited".into();
    app.editor.files[0].record_text_change();
    (app, storage)
}

#[test]
fn external_change_blocks_save_until_explicit_overwrite() {
    let (mut app, storage) = dirty_file();
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "external".into());
    app.save_active();
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "external"
    );
    assert!(storage.0.borrow().writes.is_empty());
    assert!(app.editor.files[0].dirty);
    let conflict = app.view.save_conflict.as_ref().unwrap();
    assert_eq!(conflict.path, PathBuf::from("note.txt"));
    let id = conflict.document;
    app.overwrite_conflict(id);
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "edited"
    );
    assert!(!app.editor.files[0].dirty);
    assert!(app.view.save_conflict.is_none());
}

#[test]
fn changed_document_cannot_use_a_stale_overwrite_confirmation() {
    let (mut app, storage) = dirty_file();
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "external".into());
    app.save_active();
    let id = app.editor.files[0].id;
    app.editor.files[0].text.push('!');
    app.editor.files[0].record_text_change();
    app.overwrite_conflict(id);
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "external"
    );
    assert!(app.editor.files[0].dirty);
}

#[test]
fn explicit_reload_uses_external_text_and_clears_dirty_state() {
    let (mut app, storage) = dirty_file();
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "external".into());
    app.save_active();
    let id = app.editor.files[0].id;
    app.reload_conflict(id);
    assert_eq!(app.editor.files[0].id, id);
    assert_eq!(app.editor.files[0].text, "external");
    assert!(!app.editor.files[0].dirty);
    assert!(!app.editor.files[0].disk_changed);
    assert!(app.view.save_conflict.is_none());
}

#[test]
fn recovery_restores_dirty_text_but_keeps_original_save_checkpoint() {
    let (mut app, storage) = test_app(TestDialogs::default());
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "external".into());
    let recovery = MemoryRecoveryStore::default();
    recovery.0.borrow_mut().push(RecoveryDocument {
        path: Some("note.txt".into()),
        name: "note.txt".into(),
        text: "unsaved".into(),
        saved_text: "original".into(),
    });
    app.recovery_store = Box::new(recovery);
    app.open_file("note.txt".into());
    app.restore_recovery();
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.editor.files[0].text, "unsaved");
    assert!(app.editor.files[0].dirty);
    app.save_active();
    assert!(app.view.save_conflict.is_some());
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "external"
    );
}

#[test]
fn recovered_untitled_name_is_not_reused() {
    let (mut app, _) = test_app(TestDialogs::default());
    let recovery = MemoryRecoveryStore::default();
    recovery.0.borrow_mut().push(RecoveryDocument {
        path: None,
        name: "untitled-4.txt".into(),
        text: "work".into(),
        saved_text: String::new(),
    });
    app.recovery_store = Box::new(recovery);
    app.restore_recovery();
    app.editor.new_file();
    assert_eq!(app.editor.files.last().unwrap().name, "untitled-5.txt");
}

#[test]
fn backup_tracks_unsaved_edits_and_clears_after_saving() {
    let (mut app, _) = dirty_file();
    let recovery = MemoryRecoveryStore::default();
    app.recovery_store = Box::new(recovery.clone());
    let ctx = egui::Context::default();
    app.maintain_documents(&ctx);
    assert_eq!(recovery.0.borrow()[0].text, "edited");
    app.save_active();
    app.maintain_documents(&ctx);
    assert!(recovery.0.borrow().is_empty());
}

#[test]
fn delay_autosave_is_opt_in_and_skips_untitled_buffers() {
    let (mut app, storage) = dirty_file();
    let ctx = egui::Context::default();
    app.maintain_documents(&ctx);
    std::thread::sleep(std::time::Duration::from_millis(120));
    app.maintain_documents(&ctx);
    assert!(storage.0.borrow().writes.is_empty());
    app.view
        .settings
        .user
        .persisted
        .insert("files.auto_save".into(), Value::Choice("afterDelay".into()));
    app.view
        .settings
        .user
        .persisted
        .insert("files.auto_save_delay".into(), Value::Number(100.0));
    app.maintain_documents(&ctx);
    assert!(!app.editor.files[0].dirty);
    assert_eq!(storage.0.borrow().writes.len(), 1);
    app.editor.new_file();
    let file = app.editor.active_file.unwrap();
    app.editor.files[file].text = "untitled edit".into();
    app.editor.files[file].record_text_change();
    app.maintain_documents(&ctx);
    assert!(app.editor.files[file].dirty);
    assert_eq!(storage.0.borrow().writes.len(), 1);
}
