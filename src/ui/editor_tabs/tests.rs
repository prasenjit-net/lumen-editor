use super::*;
use crate::{app::LumenApp, model::Document};
use crate::{
    app::view_state::SourceDiffView,
    services::source_control::{DiffSide, GitComparison},
};
fn fixture(count: usize) -> (LumenApp, egui::Context) {
    let mut app = LumenApp::default();
    app.editor.workspace = Some("/workspace".into());
    app.task_user_path = Some("/user/.lumen/tasks.toml".into());
    for index in 0..count {
        app.editor.files.push(Document::from_path(
            format!("file_{index}.rs").into(),
            "untouched".into(),
        ));
    }
    app.editor.set_active_index(count.checked_sub(1));
    let ctx = egui::Context::default();
    crate::ui::theme::configure_ui(&ctx);
    (app, ctx)
}
fn frame(
    app: &mut LumenApp,
    ctx: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> (egui::Rect, egui::FullOutput) {
    let mut rect = egui::Rect::NOTHING;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            app.render_component(ctx, |workbench| {
                egui::CentralPanel::default()
                    .frame(Frame::NONE)
                    .show(ctx, |ui| {
                        rect = workbench.editor_tabs(ui);
                    });
            })
        },
    );
    (rect, output)
}
fn label_rect(output: &egui::FullOutput, label: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {label}"))
}
fn click(app: &mut LumenApp, ctx: &egui::Context, width: f32, pos: egui::Pos2) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            width,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}
#[test]
fn sizing_shrinks_before_hiding_and_reserves_fixed_actions() {
    let sizing = TabSizing::new(600.0, &[220.0; 3], Some(2));
    assert!(!sizing.overflow);
    assert!(
        sizing
            .widths
            .iter()
            .all(|width| *width < 220.0 && *width >= MIN_WIDTH)
    );
    assert!(sizing.widths.iter().sum::<f32>() + 4.0 <= sizing.viewport + 0.01);
    let sizing = TabSizing::new(300.0, &[220.0; 20], None);
    assert!(sizing.overflow);
    assert_eq!(sizing.viewport, 248.0);
    assert!(
        sizing
            .widths
            .iter()
            .all(|width| (*width - MIN_WIDTH).abs() < 0.001)
    );
}
#[test]
fn overflow_never_adds_a_scrollbar_row_and_actions_stay_visible() {
    for width in [180.0, 300.0, 900.0, 1400.0] {
        let (mut app, ctx) = fixture(40);
        for _ in 0..3 {
            let (rect, output) = frame(&mut app, &ctx, width, vec![]);
            assert!(rect.height() <= HEIGHT + 0.1, "height {}", rect.height());
            let plus = label_rect(&output, icon::PLUS);
            assert!(
                plus.left() >= width - 30.0 && plus.right() <= width,
                "plus {plus:?} width {width}"
            );
            assert!(label_rect(&output, icon::CARET_DOWN).right() < plus.left());
        }
        let (_, output) = frame(&mut app, &ctx, width, vec![]);
        let selected = label_rect(&output, "file_39.rs");
        assert!(
            selected.left() >= -0.5 && selected.right() <= width - 52.0 + 0.5,
            "selected {selected:?} width {width}"
        );
        assert!(app.editor.files.iter().all(|file| file.text == "untouched"));
    }
}
#[test]
fn dropdown_opens_hidden_editor_and_new_file_button_keeps_its_behavior() {
    let (mut app, ctx) = fixture(30);
    for _ in 0..3 {
        frame(&mut app, &ctx, 300.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 300.0, vec![]);
    click(
        &mut app,
        &ctx,
        300.0,
        label_rect(&output, icon::CARET_DOWN).center(),
    );
    let (_, output) = frame(&mut app, &ctx, 300.0, vec![]);
    let position = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if text.galley.job.text.contains("file_0.rs") && text.pos.y > 22.0 =>
            {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .expect("hidden editor is available in dropdown");
    click(&mut app, &ctx, 300.0, position);
    assert_eq!(app.editor.active_index(), Some(0));
    for _ in 0..3 {
        frame(&mut app, &ctx, 300.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 300.0, vec![]);
    assert!(label_rect(&output, "file_0.rs").left() >= -0.5);
    click(
        &mut app,
        &ctx,
        300.0,
        label_rect(&output, icon::PLUS).center(),
    );
    assert_eq!(app.editor.files.len(), 31);
    assert!(app.editor.active().unwrap().path.is_none());
}

#[test]
fn settings_is_revealed_in_overflow_and_dirty_markers_keep_close_confirmation() {
    let (mut app, ctx) = fixture(30);
    app.execute(Command::Settings, &ctx);
    for _ in 0..3 {
        frame(&mut app, &ctx, 300.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 300.0, vec![]);
    let settings = label_rect(&output, "Settings");
    assert!(settings.left() >= 0.0 && settings.right() <= 248.0);
    assert!(app.view.settings.tab_open && app.view.settings.open);
    app.execute(Command::CloseEditor, &ctx);
    app.editor.files[0].name = "a_very_long_filename_that_must_not_expand_the_editor_tab.rs".into();
    app.editor.files[0].dirty = true;
    app.editor.set_active_index(Some(0));
    for _ in 0..3 {
        frame(&mut app, &ctx, 300.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 300.0, vec![]);
    let dirty = label_rect(&output, "●");
    assert!(dirty.left() >= 0.0 && dirty.right() <= 248.0);
    click(&mut app, &ctx, 300.0, dirty.center());
    assert_eq!(app.view.pending_close, Some(app.editor.files[0].id));
    assert_eq!(app.editor.files.len(), 30);
    assert!(app.editor.files[0].dirty);
    assert_eq!(app.editor.files[0].text, "untouched");
}

#[test]
fn diff_only_document_has_one_visible_tab_until_explorer_reveals_it() {
    let (mut app, ctx) = fixture(1);
    let id = app.editor.files[0].id;
    app.view.source_control.diff = Some(SourceDiffView {
        root: "/workspace".into(),
        path: "file_0.rs".into(),
        side: DiffSide::Working,
        comparison: Ok(std::sync::Arc::new(GitComparison {
            base: String::new(),
            changed: Some("untouched".into()),
            old_lines: Default::default(),
            new_lines: Default::default(),
            hunks: Vec::new(),
        })),
        document: Some(id),
        diff_only_document: Some(id),
        opened_revision: Some(0),
        active: true,
    });
    let (_, output) = frame(&mut app, &ctx, 800.0, vec![]);
    let labels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().contains("file_0.rs") => {
                Some(text.galley.text().to_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(labels.len(), 1, "{labels:?}");
    assert!(labels[0].contains("(Diff)"));

    app.view
        .source_control
        .diff
        .as_mut()
        .unwrap()
        .diff_only_document = None;
    let (_, output) = frame(&mut app, &ctx, 800.0, vec![]);
    assert_eq!(
        output
            .shapes
            .iter()
            .filter(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("file_0.rs")))
            .count(),
        2
    );
}

fn button(
    app: &mut LumenApp,
    ctx: &egui::Context,
    width: f32,
    pos: egui::Pos2,
    button: egui::PointerButton,
) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            width,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn right_click_opens_a_tab_menu_without_activating_the_tab_or_changing_content() {
    let (mut app, ctx) = fixture(3);
    for _ in 0..3 {
        frame(&mut app, &ctx, 900.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 900.0, vec![]);
    let title = label_rect(&output, "file_0.rs");
    app.editor.files[0].dirty = true;
    button(
        &mut app,
        &ctx,
        900.0,
        title.center(),
        egui::PointerButton::Secondary,
    );
    let (_, output) = frame(&mut app, &ctx, 900.0, vec![]);
    for entry in [
        "Close",
        "Close Others",
        "Close to the Left",
        "Close to the Right",
        "Close Saved",
        "Close All",
        "Copy Path",
        "Copy Relative Path",
    ] {
        label_rect(&output, entry);
    }
    assert_eq!(
        app.editor.active_index(),
        Some(2),
        "menu must not switch tabs"
    );
    assert_eq!(app.editor.files[0].text, "untouched");
    assert_eq!(app.editor.files.len(), 3);
}

#[test]
fn close_others_from_the_menu_prompts_for_a_dirty_tab_and_keeps_its_text() {
    let (mut app, ctx) = fixture(3);
    app.editor.files[0].dirty = true;
    app.editor.files[0].text = "precious".into();
    for _ in 0..3 {
        frame(&mut app, &ctx, 900.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 900.0, vec![]);
    let title = label_rect(&output, "file_2.rs");
    button(
        &mut app,
        &ctx,
        900.0,
        title.center(),
        egui::PointerButton::Secondary,
    );
    let (_, output) = frame(&mut app, &ctx, 900.0, vec![]);
    let others = label_rect(&output, "Close Others");
    click(&mut app, &ctx, 900.0, others.center());
    app.advance_bulk_close();
    assert_eq!(
        app.view.pending_close,
        Some(app.editor.files[0].id),
        "dirty tab is not dropped"
    );
    assert_eq!(app.editor.files[0].text, "precious");
    assert_eq!(app.editor.files.len(), 2, "the clean other tab closed");
}

#[test]
fn middle_click_closes_a_clean_tab_and_prompts_for_a_dirty_one() {
    let (mut app, ctx) = fixture(2);
    app.editor.files[1].dirty = true;
    for _ in 0..3 {
        frame(&mut app, &ctx, 900.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 900.0, vec![]);
    let dirty = label_rect(&output, "file_1.rs");
    button(
        &mut app,
        &ctx,
        900.0,
        dirty.center(),
        egui::PointerButton::Middle,
    );
    assert_eq!(app.view.pending_close, Some(app.editor.files[1].id));
    app.view.pending_close = None;
    let clean = label_rect(&output, "file_0.rs");
    button(
        &mut app,
        &ctx,
        900.0,
        clean.center(),
        egui::PointerButton::Middle,
    );
    assert_eq!(app.editor.files.len(), 1);
}

fn pointer(pos: egui::Pos2) -> egui::Event {
    egui::Event::PointerMoved(pos)
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn order(app: &LumenApp) -> Vec<String> {
    app.editor
        .files
        .iter()
        .map(|file| file.name.clone())
        .collect()
}

/// Drag a tab's title to `to`, optionally pressing Escape before releasing, and
/// leave the pointer there for `release_at` (which may differ from `to`).
fn drag_tab(
    app: &mut LumenApp,
    ctx: &egui::Context,
    from: &str,
    to: egui::Pos2,
    escape: bool,
    release_at: egui::Pos2,
) {
    let (_, output) = frame(app, ctx, 900.0, vec![]);
    let start = label_rect(&output, from).center();
    frame(app, ctx, 900.0, vec![pointer(start), press(start, true)]);
    frame(
        app,
        ctx,
        900.0,
        vec![pointer(start + egui::vec2(24.0, 0.0))],
    );
    frame(app, ctx, 900.0, vec![pointer(to)]);
    frame(app, ctx, 900.0, vec![pointer(to)]);
    if escape {
        frame(
            app,
            ctx,
            900.0,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    frame(
        app,
        ctx,
        900.0,
        vec![pointer(release_at), press(release_at, false)],
    );
    frame(app, ctx, 900.0, vec![]);
}

fn settled(count: usize) -> (LumenApp, egui::Context, egui::FullOutput) {
    let (mut app, ctx) = fixture(count);
    let mut output = frame(&mut app, &ctx, 900.0, vec![]).1;
    for _ in 0..3 {
        output = frame(&mut app, &ctx, 900.0, vec![]).1;
    }
    (app, ctx, output)
}

#[test]
fn dragging_a_tab_past_its_neighbours_reorders_without_touching_content() {
    let (mut app, ctx, output) = settled(3);
    app.editor.files[0].dirty = true;
    let target = label_rect(&output, "file_2.rs").center() + egui::vec2(60.0, 0.0);
    let active = app.editor.active_id();
    drag_tab(&mut app, &ctx, "file_0.rs", target, false, target);
    assert_eq!(order(&app), ["file_1.rs", "file_2.rs", "file_0.rs"]);
    assert_eq!(
        app.editor.active_id(),
        active,
        "dragging does not change the active tab"
    );
    let moved = app
        .editor
        .files
        .iter()
        .find(|f| f.name == "file_0.rs")
        .unwrap();
    assert!(moved.dirty);
    assert_eq!(moved.text, "untouched");
}

#[test]
fn dragging_left_moves_a_later_tab_to_the_front() {
    let (mut app, ctx, output) = settled(3);
    let target = label_rect(&output, "file_0.rs").center() - egui::vec2(40.0, 0.0);
    drag_tab(&mut app, &ctx, "file_2.rs", target, false, target);
    assert_eq!(order(&app), ["file_2.rs", "file_0.rs", "file_1.rs"]);
}

#[test]
fn releasing_over_the_tabs_own_slot_changes_nothing() {
    let (mut app, ctx, output) = settled(3);
    let own = label_rect(&output, "file_1.rs").center() + egui::vec2(30.0, 0.0);
    drag_tab(&mut app, &ctx, "file_1.rs", own, false, own);
    assert_eq!(order(&app), ["file_0.rs", "file_1.rs", "file_2.rs"]);
}

#[test]
fn escape_while_dragging_cancels_the_move() {
    let (mut app, ctx, output) = settled(3);
    let target = label_rect(&output, "file_2.rs").center() + egui::vec2(60.0, 0.0);
    drag_tab(&mut app, &ctx, "file_0.rs", target, true, target);
    assert_eq!(order(&app), ["file_0.rs", "file_1.rs", "file_2.rs"]);
    // The cancel does not leak into the next drag.
    drag_tab(&mut app, &ctx, "file_0.rs", target, false, target);
    assert_eq!(order(&app), ["file_1.rs", "file_2.rs", "file_0.rs"]);
}

#[test]
fn dropping_well_outside_the_strip_abandons_the_move() {
    let (mut app, ctx, output) = settled(3);
    let target = label_rect(&output, "file_2.rs").center() + egui::vec2(60.0, 0.0);
    let far_below = egui::pos2(target.x, 400.0);
    drag_tab(&mut app, &ctx, "file_0.rs", target, false, far_below);
    assert_eq!(order(&app), ["file_0.rs", "file_1.rs", "file_2.rs"]);
}

#[test]
fn a_plain_click_still_selects_and_never_reorders() {
    let (mut app, ctx, output) = settled(3);
    let title = label_rect(&output, "file_0.rs").center();
    frame(
        &mut app,
        &ctx,
        900.0,
        vec![pointer(title), press(title, true)],
    );
    frame(&mut app, &ctx, 900.0, vec![press(title, false)]);
    frame(&mut app, &ctx, 900.0, vec![]);
    assert_eq!(order(&app), ["file_0.rs", "file_1.rs", "file_2.rs"]);
    assert_eq!(app.editor.active_index(), Some(0));
}

#[test]
fn the_tab_menu_offers_move_left_and_right_with_end_states() {
    let (mut app, ctx, output) = settled(3);
    let first = label_rect(&output, "file_0.rs").center();
    for pressed in [true, false] {
        frame(
            &mut app,
            &ctx,
            900.0,
            vec![
                pointer(first),
                egui::Event::PointerButton {
                    pos: first,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let (_, menu) = frame(&mut app, &ctx, 900.0, vec![]);
    label_rect(&menu, "Move Left");
    let right = label_rect(&menu, "Move Right");
    click(&mut app, &ctx, 900.0, right.center());
    assert_eq!(order(&app), ["file_1.rs", "file_0.rs", "file_2.rs"]);
}

fn texts_containing(output: &egui::FullOutput, needle: &str) -> usize {
    output
        .shapes
        .iter()
        .filter(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(needle)))
        .count()
}

#[test]
fn a_task_editor_tab_shows_its_page_and_hands_back_to_documents() {
    use lumen_task_core::Scope;
    let (mut app, ctx) = fixture(2);
    app.execute(Command::NewTaskEditor(Scope::Workspace), &ctx);
    assert!(app.view.page_tab_active());
    let (_, output) = frame(&mut app, &ctx, 800.0, vec![]);
    assert_eq!(texts_containing(&output, "New task"), 1, "one strip tab");

    // Choosing a document tab, creating a file or opening Settings leaves the page.
    app.execute(Command::NewTaskEditor(Scope::User), &ctx);
    assert_eq!(app.view.task_editors.tabs().len(), 2);
    app.execute(Command::NewUntitled, &ctx);
    assert!(app.view.task_editors.active().is_none());
    assert_eq!(app.view.task_editors.tabs().len(), 2, "tabs stay open");
    let first = app.view.task_editors.tabs()[0].id;
    app.execute(Command::ActivateTaskEditor(first), &ctx);
    app.execute(Command::Settings, &ctx);
    assert!(app.view.settings.open && app.view.task_editors.active().is_none());
    app.execute(Command::ActivateTaskEditor(first), &ctx);
    assert!(!app.view.settings.open, "the task page replaces Settings");
    let doc = app.editor.files[0].id;
    app.show_document_tab(doc);
    assert!(!app.view.page_tab_active());
}

#[test]
fn closing_task_editors_by_command_and_by_the_tab_close_button() {
    use lumen_task_core::Scope;
    let (mut app, ctx) = fixture(1);
    app.execute(Command::NewTaskEditor(Scope::Workspace), &ctx);
    app.execute(Command::CloseEditor, &ctx);
    assert!(app.view.task_editors.tabs().is_empty());
    assert_eq!(
        app.editor.files.len(),
        1,
        "Close Editor did not close a document"
    );

    app.execute(Command::NewTaskEditor(Scope::Workspace), &ctx);
    let id = app.view.task_editors.tabs()[0].id;
    app.execute(Command::CloseTaskEditor(id), &ctx);
    app.execute(Command::CloseTaskEditor(id), &ctx);
    app.execute(Command::ActivateTaskEditor(id), &ctx);
    assert!(
        app.view.task_editors.active().is_none(),
        "stale ids do nothing"
    );

    app.execute(Command::NewTaskEditor(Scope::User), &ctx);
    frame(&mut app, &ctx, 800.0, vec![]);
    let (_, output) = frame(&mut app, &ctx, 800.0, vec![]);
    let tab = label_rect(&output, "New task");
    // The close button sits at the right edge of the tab's cell.
    click(
        &mut app,
        &ctx,
        800.0,
        egui::pos2(tab.right() + 40.0, tab.center().y),
    );
    assert!(
        app.view.task_editors.tabs().is_empty(),
        "close button closed it"
    );
}

#[test]
fn commands_that_need_a_document_are_unavailable_while_a_task_editor_shows() {
    use lumen_task_core::Scope;
    let (mut app, ctx) = fixture(1);
    let reason = |app: &mut LumenApp, command: Command| {
        Workbench::new(&mut app.editor, &mut app.view, app.file_system.as_ref())
            .palette_disabled_reason(&command)
    };
    assert!(reason(&mut app, Command::Find).is_none());
    app.execute(Command::NewTaskEditor(Scope::Workspace), &ctx);
    assert!(reason(&mut app, Command::Find).is_some());
    assert!(reason(&mut app, Command::CloseEditor).is_none());
}

#[test]
fn a_task_editor_tab_is_reachable_through_overflow() {
    use lumen_task_core::Scope;
    let (mut app, ctx) = fixture(30);
    app.execute(Command::NewTaskEditor(Scope::Workspace), &ctx);
    for _ in 0..3 {
        frame(&mut app, &ctx, 300.0, vec![]);
    }
    let (_, output) = frame(&mut app, &ctx, 300.0, vec![]);
    let tab = label_rect(&output, "New task");
    assert!(tab.left() >= 0.0 && tab.right() <= 248.0, "{tab:?}");
}
