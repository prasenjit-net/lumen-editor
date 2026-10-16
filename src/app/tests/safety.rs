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
fn second_external_edit_requires_a_new_explicit_review() {
    let (mut app, storage) = dirty_file();
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "first external".into());
    app.save_active();
    let id = app.editor.files[0].id;
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "second external".into());
    app.overwrite_conflict(id);
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "second external"
    );
    assert_eq!(
        app.view
            .save_conflict
            .as_ref()
            .unwrap()
            .disk_text
            .as_deref(),
        Some("second external")
    );
    assert!(app.view.save_conflict.as_ref().unwrap().show_comparison);
    app.overwrite_conflict(id);
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "edited"
    );
}

#[test]
fn removed_file_cannot_overwrite_a_new_file_at_its_path() {
    let (mut app, storage) = dirty_file();
    storage
        .0
        .borrow_mut()
        .files
        .remove(&PathBuf::from("note.txt"));
    app.save_active();
    let id = app.editor.files[0].id;
    assert!(app.view.save_conflict.as_ref().unwrap().disk_text.is_none());
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "new owner".into());
    app.overwrite_conflict(id);
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "new owner"
    );
    assert!(app.editor.files[0].dirty);
}

#[test]
fn every_open_tab_gets_a_persistent_external_change_marker() {
    let (mut app, storage) = dirty_file();
    storage
        .0
        .borrow_mut()
        .files
        .insert("other.txt".into(), "old".into());
    app.open_file("other.txt".into());
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "external".into());
    storage
        .0
        .borrow_mut()
        .files
        .insert("other.txt".into(), "new".into());
    app.check_open_disk_changes();
    assert!(app.editor.files.iter().all(|file| file.disk_changed));
    assert!(app.editor.files[0].dirty);
    assert_eq!(app.editor.files[1].text, "old");
    let id = app.editor.files[1].id;
    app.review_disk_change(id);
    assert_eq!(
        app.view
            .save_conflict
            .as_ref()
            .unwrap()
            .disk_text
            .as_deref(),
        Some("new")
    );
    assert!(app.view.save_conflict.as_ref().unwrap().show_comparison);
}

#[test]
fn focus_change_policy_saves_only_the_document_that_lost_editor_focus() {
    let (mut app, storage) = dirty_file();
    app.view.settings.user.persisted.insert(
        "files.auto_save".into(),
        Value::Choice("onFocusChange".into()),
    );
    let ctx = egui::Context::default();
    let id = crate::app::editor_action::text_widget_id(app.editor.files[0].id);
    ctx.memory_mut(|memory| memory.request_focus(id));
    app.maintain_documents(&ctx);
    assert!(storage.0.borrow().writes.is_empty());
    ctx.memory_mut(|memory| memory.surrender_focus(id));
    app.maintain_documents(&ctx);
    assert_eq!(storage.0.borrow().writes.len(), 1);
}

#[test]
fn window_change_policy_saves_only_on_a_real_focused_to_unfocused_transition() {
    let (mut app, storage) = dirty_file();
    app.view.settings.user.persisted.insert(
        "files.auto_save".into(),
        Value::Choice("onWindowChange".into()),
    );
    let ctx = egui::Context::default();
    app.maintain_documents(&ctx);
    assert!(storage.0.borrow().writes.is_empty());
    let _ = ctx.run(
        egui::RawInput {
            focused: true,
            ..Default::default()
        },
        |_| {},
    );
    app.maintain_documents(&ctx);
    assert!(storage.0.borrow().writes.is_empty());
    let _ = ctx.run(
        egui::RawInput {
            focused: false,
            ..Default::default()
        },
        |_| {},
    );
    app.maintain_documents(&ctx);
    assert_eq!(storage.0.borrow().writes.len(), 1);
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
    assert_eq!(app.editor.files.len(), 2);
    assert_eq!(app.editor.files[1].text, "unsaved");
    assert_eq!(app.editor.files[1].path, None);
    assert!(app.editor.files[1].dirty);
    app.save_active();
    assert!(app.editor.files[1].dirty); // Save As was cancelled.
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
fn divergent_recovered_buffers_for_one_path_remain_distinct() {
    let (mut app, _) = test_app(TestDialogs::default());
    let recovery = MemoryRecoveryStore::default();
    recovery.0.borrow_mut().extend([
        RecoveryDocument {
            path: Some("same.txt".into()),
            name: "same.txt".into(),
            text: "first".into(),
            saved_text: "base".into(),
        },
        RecoveryDocument {
            path: Some("same.txt".into()),
            name: "same.txt".into(),
            text: "second".into(),
            saved_text: "base".into(),
        },
    ]);
    app.recovery_store = Box::new(recovery);
    app.restore_recovery();
    assert_eq!(app.editor.files.len(), 2);
    assert_eq!(app.editor.files[0].path, Some("same.txt".into()));
    assert_eq!(app.editor.files[1].path, None);
    assert_eq!(app.editor.files[0].text, "first");
    assert_eq!(app.editor.files[1].text, "second");
    assert!(app.editor.files.iter().all(|file| file.dirty));
}

#[test]
fn conflict_save_as_keeps_external_version_and_saves_other_path() {
    let (mut app, storage) = test_app(TestDialogs {
        save_path: Some("copy.txt".into()),
        ..Default::default()
    });
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "original".into());
    app.open_file("note.txt".into());
    app.editor.files[0].text = "mine".into();
    app.editor.files[0].record_text_change();
    storage
        .0
        .borrow_mut()
        .files
        .insert("note.txt".into(), "external".into());
    app.save_active();
    let id = app.editor.files[0].id;
    app.save_conflict_as(id);
    assert_eq!(
        storage.0.borrow().files[&PathBuf::from("note.txt")],
        "external"
    );
    assert_eq!(storage.0.borrow().files[&PathBuf::from("copy.txt")], "mine");
    assert!(!app.editor.files[0].dirty);
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
    let file = app.editor.active_index().unwrap();
    app.editor.files[file].text = "untitled edit".into();
    app.editor.files[file].record_text_change();
    app.maintain_documents(&ctx);
    assert!(app.editor.files[file].dirty);
    assert_eq!(storage.0.borrow().writes.len(), 1);
}
