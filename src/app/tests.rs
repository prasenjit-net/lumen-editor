use crate::ui::theme::configure_ui;

use super::LumenApp;
use super::command::Command;
use super::view_state::Activity;
use crate::services::{FileDialogs, FileSystem};
use eframe::egui::{self, Vec2};
use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Default)]
struct TestStorage {
    files: HashMap<PathBuf, String>,
    reads: usize,
    writes: Vec<(PathBuf, String)>,
    fail_writes: bool,
}

#[derive(Clone, Default)]
struct MemoryFileSystem(Rc<RefCell<TestStorage>>);

impl FileSystem for MemoryFileSystem {
    fn read_text(&self, path: &Path) -> io::Result<String> {
        let mut storage = self.0.borrow_mut();
        storage.reads += 1;
        storage
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn write_text(&self, path: &Path, text: &str) -> io::Result<()> {
        let mut storage = self.0.borrow_mut();
        if storage.fail_writes {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        storage.writes.push((path.into(), text.into()));
        storage.files.insert(path.into(), text.into());
        Ok(())
    }

    fn children(&self, _: &Path) -> Vec<PathBuf> {
        Vec::new()
    }
    fn is_directory(&self, _: &Path) -> bool {
        false
    }
}

#[derive(Clone, Default)]
struct TestDialogs {
    file: Option<PathBuf>,
    folder: Option<PathBuf>,
    save_path: Option<PathBuf>,
}

impl FileDialogs for TestDialogs {
    fn open_file(&self) -> Option<PathBuf> {
        self.file.clone()
    }
    fn open_folder(&self) -> Option<PathBuf> {
        self.folder.clone()
    }
    fn save_as(&self, _: &str) -> Option<PathBuf> {
        self.save_path.clone()
    }
}

fn test_app(dialogs: TestDialogs) -> (LumenApp, MemoryFileSystem) {
    let storage = MemoryFileSystem::default();
    let app = LumenApp {
        file_system: Box::new(storage.clone()),
        dialogs: Box::new(dialogs),
        ..Default::default()
    };
    (app, storage)
}

fn edit_active(app: &mut LumenApp, text: &str) {
    let file = &mut app.editor.files[app.editor.active_file.unwrap()];
    file.text = text.into();
    file.dirty = true;
}

#[test]
fn new_buffers_have_unique_names_and_closing_selects_a_remaining_tab() {
    let (mut app, _) = test_app(TestDialogs::default());
    app.new_file();
    app.new_file();
    assert_eq!(app.editor.files[0].name, "untitled-1.txt");
    assert_eq!(app.editor.active().unwrap().name, "untitled-2.txt");
    app.close_file(1);
    assert_eq!(app.editor.active_file, Some(0));
    app.close_file(0);
    assert_eq!(app.editor.active_file, None);
    app.new_file();
    assert_eq!(app.editor.active().unwrap().name, "untitled-3.txt");
}

#[test]
fn opening_an_existing_tab_preserves_edits_without_rereading() {
    let path = PathBuf::from("example.rs");
    let (mut app, storage) = test_app(TestDialogs {
        file: Some(path.clone()),
        ..Default::default()
    });
    storage
        .0
        .borrow_mut()
        .files
        .insert(path.clone(), "original".into());
    app.choose_file();
    assert_eq!(app.editor.active().unwrap().language, "Rust");
    edit_active(&mut app, "edited");
    app.new_file();
    app.open_file(path);
    assert_eq!(app.editor.files.len(), 2);
    assert_eq!(app.editor.active_file, Some(0));
    assert_eq!(app.editor.active().unwrap().text, "edited");
    assert!(app.editor.active().unwrap().dirty);
    assert_eq!(storage.0.borrow().reads, 1);
}

#[test]
fn failed_open_keeps_existing_buffers_and_selection() {
    let (mut app, _) = test_app(TestDialogs::default());
    app.new_file();
    edit_active(&mut app, "keep me");
    app.open_file("missing.txt".into());
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.editor.active().unwrap().text, "keep me");
    assert!(app.view.status.starts_with("Could not open file:"));
}

#[test]
fn saving_an_untitled_buffer_updates_metadata_only_after_writing() {
    let path = PathBuf::from("saved.rs");
    let (mut app, storage) = test_app(TestDialogs {
        save_path: Some(path.clone()),
        ..Default::default()
    });
    app.new_file();
    edit_active(&mut app, "fn main() {}");
    app.save_active();
    let file = app.editor.active().unwrap();
    assert_eq!(file.path.as_ref(), Some(&path));
    assert_eq!(file.name, "saved.rs");
    assert_eq!(file.language, "Rust");
    assert!(!file.dirty);
    assert_eq!(storage.0.borrow().files[&path], "fn main() {}");
    edit_active(&mut app, "updated");
    app.save_active();
    assert_eq!(storage.0.borrow().files[&path], "updated");
    assert_eq!(storage.0.borrow().writes.len(), 2);
}

#[test]
fn cancelling_dialogs_preserves_buffers_and_workspace() {
    let (mut app, storage) = test_app(TestDialogs::default());
    app.new_file();
    edit_active(&mut app, "unsaved");
    app.choose_file();
    app.choose_folder();
    app.save_as_active();
    let file = app.editor.active().unwrap();
    assert_eq!(app.editor.files.len(), 1);
    assert!(app.editor.workspace.is_none());
    assert_eq!(file.name, "untitled-1.txt");
    assert!(file.path.is_none());
    assert_eq!(file.text, "unsaved");
    assert!(file.dirty);
    assert!(storage.0.borrow().writes.is_empty());
}

#[test]
fn failed_save_as_preserves_original_path_and_dirty_content() {
    let original = PathBuf::from("original.txt");
    let (mut app, storage) = test_app(TestDialogs {
        save_path: Some("new.rs".into()),
        ..Default::default()
    });
    storage
        .0
        .borrow_mut()
        .files
        .insert(original.clone(), "original".into());
    app.open_file(original.clone());
    edit_active(&mut app, "edited");
    storage.0.borrow_mut().fail_writes = true;
    app.save_as_active();
    let file = app.editor.active().unwrap();
    assert_eq!(file.path.as_ref(), Some(&original));
    assert_eq!(file.language, "Plain Text");
    assert_eq!(file.text, "edited");
    assert!(file.dirty);
    assert!(app.view.status.starts_with("Could not save file:"));
}

#[test]
fn opening_a_folder_resets_explorer_state_but_keeps_buffers() {
    let folder = PathBuf::from("project");
    let (mut app, _) = test_app(TestDialogs {
        folder: Some(folder.clone()),
        ..Default::default()
    });
    app.new_file();
    app.view.sidebar_visible = false;
    app.view.activity = Activity::Search;
    app.view.expanded_dirs.insert("old-folder".into());
    app.choose_folder();
    assert_eq!(app.editor.workspace.as_ref(), Some(&folder));
    assert_eq!(app.editor.files.len(), 1);
    assert!(app.view.sidebar_visible);
    assert!(app.view.activity == Activity::Explorer);
    assert_eq!(app.view.expanded_dirs, [folder].into_iter().collect());
}

#[test]
fn dirty_close_requires_a_decision_and_clean_close_does_not() {
    let (mut app, _) = test_app(TestDialogs::default());
    app.new_file();
    edit_active(&mut app, "unsaved");
    app.request_close(0);
    assert_eq!(app.view.pending_close, Some(0));
    assert_eq!(app.editor.files.len(), 1);
    app.new_file();
    app.request_close(1);
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.editor.active_file, Some(0));
}

#[test]
fn save_all_stops_on_cancellation_and_restores_selection() {
    let (mut app, storage) = test_app(TestDialogs::default());
    app.new_file();
    edit_active(&mut app, "first");
    app.new_file();
    edit_active(&mut app, "second");
    app.view.pending_quit = true;
    app.execute(Command::SaveAllAndQuit, &egui::Context::default());
    assert_eq!(app.editor.active_file, Some(1));
    assert!(app.editor.files.iter().all(|file| file.dirty));
    assert!(app.view.pending_quit);
    assert!(!app.view.allow_quit);
    assert!(storage.0.borrow().writes.is_empty());
}

#[test]
fn save_all_writes_dirty_files_before_allowing_quit() {
    let (mut app, storage) = test_app(TestDialogs::default());
    for name in ["first.txt", "second.txt"] {
        storage
            .0
            .borrow_mut()
            .files
            .insert(name.into(), "original".into());
        app.open_file(name.into());
        edit_active(&mut app, name);
    }
    app.view.pending_quit = true;
    app.execute(Command::SaveAllAndQuit, &egui::Context::default());
    assert_eq!(app.editor.active_file, Some(1));
    assert!(!app.editor.has_unsaved_changes());
    assert!(app.view.allow_quit);
    assert!(!app.view.pending_quit);
    assert_eq!(storage.0.borrow().writes.len(), 2);
}

#[test]
fn save_and_close_writes_the_buffer_before_removing_it() {
    let path = PathBuf::from("saved.txt");
    let (mut app, storage) = test_app(TestDialogs {
        save_path: Some(path.clone()),
        ..Default::default()
    });
    app.new_file();
    edit_active(&mut app, "keep this text");
    app.request_close(0);
    app.execute(Command::SaveAndClose(0), &egui::Context::default());
    assert_eq!(storage.0.borrow().files[&path], "keep this text");
    assert!(app.editor.files.is_empty());
    assert_eq!(app.view.pending_close, None);
}

#[test]
fn save_and_close_cancellation_keeps_the_dirty_buffer() {
    let (mut app, _) = test_app(TestDialogs::default());
    app.new_file();
    edit_active(&mut app, "unsaved");
    app.request_close(0);
    app.execute(Command::SaveAndClose(0), &egui::Context::default());
    assert_eq!(app.editor.files.len(), 1);
    assert_eq!(app.editor.active().unwrap().text, "unsaved");
    assert!(app.editor.active().unwrap().dirty);
    // The existing dialog dismisses after Save, including a cancelled Save As.
    assert_eq!(app.view.pending_close, None);
}

#[test]
fn save_all_stops_on_write_failure_without_quitting() {
    let (mut app, storage) = test_app(TestDialogs::default());
    for name in ["first.txt", "second.txt"] {
        storage
            .0
            .borrow_mut()
            .files
            .insert(name.into(), "original".into());
        app.open_file(name.into());
        edit_active(&mut app, name);
    }
    storage.0.borrow_mut().fail_writes = true;
    app.view.pending_quit = true;
    app.execute(Command::SaveAllAndQuit, &egui::Context::default());
    assert_eq!(app.editor.active_file, Some(1));
    assert!(app.editor.files.iter().all(|file| file.dirty));
    assert!(app.view.pending_quit);
    assert!(!app.view.allow_quit);
    assert!(app.view.status.starts_with("Could not save file:"));
}

fn sidebar_frame(app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>) -> f32 {
    let mut width = 0.0;
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1000.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            app.render_component(ctx, |workbench| workbench.sidebar(ctx));
            width = ctx.available_rect().left();
        },
    );
    width
}

#[test]
fn explorer_sash_resizes_both_ways_and_retains_width() {
    let ctx = egui::Context::default();
    configure_ui(&ctx);
    let mut app = LumenApp::default();
    let initial = sidebar_frame(&mut app, &ctx, vec![]);
    let edge = egui::pos2(initial - 1.0, 200.0);
    sidebar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
    sidebar_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: edge,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let wider = sidebar_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(edge + Vec2::new(100.0, 0.0))],
    );
    assert!(
        wider > initial + 70.0,
        "drag should expand sidebar: {initial} -> {wider}"
    );
    let release = edge + Vec2::new(100.0, 0.0);
    sidebar_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: release,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let retained = sidebar_frame(&mut app, &ctx, vec![]);
    assert!(
        (retained - wider).abs() < 2.0,
        "width should persist after release: {wider} -> {retained}"
    );

    let edge = egui::pos2(retained - 1.0, 200.0);
    sidebar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
    sidebar_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerButton {
            pos: edge,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let narrower = sidebar_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(edge - Vec2::new(100.0, 0.0))],
    );
    assert!(
        narrower < wider - 70.0,
        "drag should shrink sidebar: {wider} -> {narrower}"
    );
}
