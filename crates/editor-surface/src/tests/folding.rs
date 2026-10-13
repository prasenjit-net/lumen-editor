use super::*;
use std::collections::BTreeSet;

const TEXT: &str = "fn main() {\r\n  one();\r\n  two();\r\n}\r\nnext();";
const RANGE: lumen_editor_core::FoldRange = lumen_editor_core::FoldRange { header: 0, end: 3 };

fn folding_fixture() -> (EditorSurface, EditorBuffer, Context) {
    let (mut surface, buffer, ctx) = fixture(TEXT);
    surface.set_folding(TEXT, &[RANGE], &BTreeSet::from([0]), ">", "v");
    (surface, buffer, ctx)
}

#[test]
fn folded_projection_keeps_source_bytes_rows_and_accessibility_aligned() {
    for width in [320.0, 900.0] {
        let (mut surface, mut buffer, ctx) = folding_fixture();
        let output = frame(&mut surface, &mut buffer, &ctx, vec![], width);
        assert_eq!(output.geometry.galley.text(), "fn main() {\nnext();");
        assert_eq!(output.geometry.row_count(), 2);
        assert_eq!(output.geometry.source_line_for_row(0), 1);
        assert_eq!(output.geometry.source_line_for_row(1), 5);
        let next = TEXT.find("next").unwrap();
        let caret = output.geometry.caret_rect(next);
        assert_eq!(output.geometry.hit_test(caret.center()), next);
        assert_eq!(buffer.text(), TEXT);
        assert!(!buffer.is_dirty());
        assert_eq!(buffer.revision(), 0);
        let position = egui::accesskit::TextPosition {
            node: egui::accesskit::NodeId(Id::new("test_surface").with(1usize).value()),
            character_index: 0,
        };
        assert_eq!(
            output
                .geometry
                .accessibility_byte(Id::new("test_surface"), &position),
            Some(next)
        );
        frame(
            &mut surface,
            &mut buffer,
            &ctx,
            vec![key(Key::ArrowDown, Modifiers::NONE)],
            width,
        );
        assert!(buffer.selections().selections()[0].head >= next);
    }
}

#[test]
fn ime_preedit_temporarily_shows_all_rows_without_unfolding_state_or_text() {
    let (mut surface, mut buffer, ctx) = folding_fixture();
    let output = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::Ime(egui::ImeEvent::Enabled),
            Event::Ime(egui::ImeEvent::Preedit("漢".into())),
        ],
        500.0,
    );
    assert_eq!(output.geometry.row_count(), 5);
    assert!(output.folded_headers.is_none());
    assert_eq!(buffer.text(), TEXT);
    assert!(!buffer.can_undo());
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::Disabled)],
        500.0,
    );
    let folded = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(folded.geometry.row_count(), 2);
}

#[test]
fn fold_gutter_click_toggles_without_editing_or_selecting_text() {
    let (mut surface, mut buffer, ctx) = fixture(TEXT);
    surface.set_folding(TEXT, &[RANGE], &BTreeSet::new(), ">", "v");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let point = pos2(
        output.gutter.left() + 8.0,
        output.geometry.row_rects()[0].center().y,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::PointerMoved(point)],
        500.0,
    );
    let output = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(point, Modifiers::NONE),
        500.0,
    );
    assert_eq!(output.folded_headers, Some(BTreeSet::from([0])));
    assert_eq!(buffer.text(), TEXT);
    assert_eq!(buffer.revision(), 0);
    surface.set_folding(TEXT, &[RANGE], &BTreeSet::from([0]), ">", "v");
    let collapsed = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(collapsed.geometry.row_count(), 2);
    let output = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(point, Modifiers::NONE),
        500.0,
    );
    assert_eq!(output.folded_headers, Some(BTreeSet::new()));
}

#[test]
fn hidden_caret_unfolds_before_paint_and_typing_remains_undoable() {
    let (mut surface, mut buffer, ctx) = folding_fixture();
    let inside = TEXT.find("two").unwrap();
    buffer
        .set_selections(vec![Selection::caret(inside)], 0)
        .unwrap();
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(output.folded_headers, Some(BTreeSet::new()));
    assert_eq!(output.geometry.row_count(), 5);
    assert_eq!(buffer.text(), TEXT);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![Event::Text("X".into())],
        500.0,
    );
    assert!(buffer.text().contains("Xtwo"));
    assert!(buffer.undo());
    assert_eq!(buffer.text(), TEXT);
}

#[test]
fn revealing_a_find_target_opens_the_fold_without_changing_selection() {
    let (mut surface, mut buffer, ctx) = folding_fixture();
    let before = buffer.selections().clone();
    surface.reveal(TEXT.find("two").unwrap());
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(output.folded_headers, Some(BTreeSet::new()));
    assert_eq!(output.geometry.row_count(), 5);
    assert_eq!(buffer.selections(), &before);
    assert!(!buffer.is_dirty());
}

#[cfg(feature = "shaped")]
#[test]
fn shaped_folding_uses_the_same_source_to_visual_mapping() {
    let (_, mut buffer, ctx) = fixture(TEXT);
    let mut surface = EditorSurface::with_shaper(crate::Shaper::default());
    surface.set_folding(TEXT, &[RANGE], &BTreeSet::from([0]), ">", "v");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(output.geometry.row_count(), 2);
    assert_eq!(output.geometry.source_line_for_row(1), 5);
    let byte = TEXT.find("next").unwrap();
    assert_eq!(
        output
            .geometry
            .hit_test(output.geometry.caret_rect(byte).center()),
        byte
    );
    assert_eq!(buffer.text(), TEXT);
}
