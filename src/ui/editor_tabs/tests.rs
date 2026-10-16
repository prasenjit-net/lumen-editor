use super::*;
use crate::{app::LumenApp, model::Document};
use crate::{
    app::view_state::SourceDiffView,
    services::source_control::{DiffSide, GitComparison},
};
fn fixture(count: usize) -> (LumenApp, egui::Context) {
    let mut app = LumenApp::default();
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
        "Close to the Right",
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
