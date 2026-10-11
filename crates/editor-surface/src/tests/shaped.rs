use super::*;
use crate::Shaper;
use unicode_segmentation::UnicodeSegmentation;

fn shaped_fixture(text: &str) -> (EditorSurface, EditorBuffer, Context) {
    let (_, buffer, ctx) = fixture(text);
    let mut surface = EditorSurface::with_shaper(Shaper::default());
    let mut buffer = buffer;
    frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    (surface, buffer, ctx)
}

#[test]
fn enabling_accessibility_after_idle_frames_still_publishes_complete_rows() {
    let (mut surface, mut buffer, ctx) = shaped_fixture("one\ntwo");
    ctx.enable_accesskit();
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            surface.show(ui, Id::new("test_surface"), &mut buffer);
        });
    });
    let tree = output.platform_output.accesskit_update.unwrap();
    for (line, expected) in [(0, "one\n"), (1, "two")] {
        let id = egui::accesskit::NodeId(Id::new("test_surface").with(line).value());
        let (_, node) = tree
            .nodes
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .unwrap();
        assert_eq!(node.value(), Some(expected));
        assert_eq!(
            node.character_positions().unwrap().len(),
            expected.chars().count()
        );
    }
    assert_eq!(buffer.revision(), 0);
}

#[test]
fn bulk_accessible_positions_match_painted_geometry_including_preedit_and_crlf() {
    for preedit in [false, true] {
        let (mut surface, mut buffer, ctx) = shaped_fixture("office é\r\nabc العربية 👩‍💻\n\nend");
        ctx.enable_accesskit();
        let mut geometry = None;
        let output = ctx.run(
            RawInput {
                events: if preedit {
                    vec![
                        Event::Ime(egui::ImeEvent::Enabled),
                        Event::Ime(egui::ImeEvent::Preedit("漢".into())),
                    ]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    geometry = Some(
                        surface
                            .show(ui, Id::new("test_surface"), &mut buffer)
                            .geometry,
                    );
                });
            },
        );
        let geometry = geometry.unwrap();
        let shaped = geometry.shaped.as_ref().unwrap();
        let text = geometry.galley.text();
        let tree = output.platform_output.accesskit_update.unwrap();
        let mut character = 0;
        for (line, start) in shaped.starts.iter().copied().enumerate() {
            let end = shaped.starts.get(line + 1).copied().unwrap_or(text.len());
            let value = &text[start..end];
            let id = egui::accesskit::NodeId(Id::new("test_surface").with(line).value());
            let node = &tree
                .nodes
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .unwrap()
                .1;
            assert_eq!(node.value(), Some(value));
            let positions = node.character_positions().unwrap();
            let widths = node.character_widths().unwrap();
            assert_eq!(positions.len(), value.chars().count());
            for offset in 0..positions.len() {
                let left = geometry.visual_caret(character + offset).left();
                let right = geometry.visual_caret(character + offset + 1).left();
                assert!((positions[offset] - (left.min(right) - geometry.origin.x)).abs() < 0.0001);
                assert!((widths[offset] - (right - left).abs()).abs() < 0.0001);
            }
            character += value.chars().count();
        }
        assert_eq!(buffer.revision(), 0);
    }
}

#[test]
fn shaped_geometry_maps_source_crlf_and_extended_graphemes() {
    let text = "office ffi é\r\n👩‍💻 漢";
    let (mut surface, mut buffer, ctx) = shaped_fixture(text);
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(output.geometry.row_count(), 2);
    assert_eq!(
        output.geometry.row_rects()[1].top() - output.geometry.row_rects()[0].top(),
        18.0
    );
    for byte in text
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .chain([text.len()])
    {
        let caret = output.geometry.caret_rect(byte);
        assert!(caret.is_finite());
        let hit = output
            .geometry
            .hit_test(egui::pos2(caret.left(), caret.center().y));
        assert_eq!(hit, byte, "source byte {byte}");
    }
}

#[test]
fn shaped_arabic_indices_and_selection_spans_remain_in_source_domain() {
    let (mut surface, mut buffer, ctx) = shaped_fixture("العربية");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    for byte in buffer
        .text()
        .char_indices()
        .map(|(byte, _)| byte)
        .chain([buffer.text().len()])
    {
        let caret = output.geometry.caret_rect(byte);
        assert_eq!(
            output
                .geometry
                .hit_test(egui::pos2(caret.left(), caret.center().y)),
            byte
        );
    }
    let rects = output.geometry.selection_rects(0..buffer.text().len());
    assert!(!rects.is_empty());
    assert!(
        rects
            .iter()
            .all(|rect| rect.is_finite() && rect.width() >= 0.0)
    );
    // Font-coverage proof is separate: embedded egui fonts lack Arabic glyphs.
}

#[test]
fn shaped_carets_have_no_selection_fill_and_ranges_do_not_select_other_rows() {
    let (mut surface, mut buffer, ctx) = shaped_fixture("one\ntwo\nthree");
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert!(output.geometry.selection_rects(0..0).is_empty());
    let rects = output.geometry.selection_rects(4..7);
    assert!(!rects.is_empty());
    let row = output.geometry.row_rects()[1];
    assert!(
        rects
            .iter()
            .all(|rect| rect.top() == row.top() && rect.bottom() == row.bottom())
    );
}

#[test]
fn shaped_render_uploads_glyph_textures_and_preserves_accessible_tree_ownership() {
    let (mut surface, mut buffer, ctx) = shaped_fixture("office\r\ntext");
    ctx.enable_accesskit();
    let output = ctx.run(RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            surface.show(ui, Id::new("test_surface"), &mut buffer);
        });
    });
    assert!(output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Mesh(mesh) if mesh.texture_id!=egui::TextureId::default())));
    let tree = output.platform_output.accesskit_update.unwrap();
    let row = egui::accesskit::NodeId(Id::new("test_surface").with(0usize).value());
    let parents = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.children().contains(&row))
        .count();
    assert_eq!(parents, 1);
    let parent = tree
        .nodes
        .iter()
        .find(|(id, _)| *id == egui::accesskit::NodeId(Id::new("test_surface").value()))
        .unwrap();
    assert!(parent.1.children().contains(&row));
    assert_eq!(parent.1.role(), egui::accesskit::Role::MultilineTextInput);
}

#[test]
fn shaped_inline_preedit_uses_rendered_caret_and_no_document_history() {
    let (mut surface, mut buffer, ctx) = shaped_fixture("abcd\r\nnext");
    buffer
        .set_selections(vec![Selection { anchor: 1, head: 2 }], 0)
        .unwrap();
    let output = frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![
            Event::Ime(egui::ImeEvent::Enabled),
            Event::Ime(egui::ImeEvent::Preedit("compose".into())),
        ],
        300.0,
    );
    assert_eq!(output.geometry.galley.text(), "acomposecd\nnext");
    assert_eq!(output.geometry.caret_rect(2), output.ime_cursor.unwrap());
    assert_eq!(
        output
            .geometry
            .hit_test(output.geometry.caret_rect(3).center()),
        3
    );
    assert_eq!(buffer.revision(), 0);
    assert!(!buffer.undo());
}

#[test]
fn shaped_gutter_selection_and_vertical_navigation_share_row_geometry() {
    let (mut surface, mut buffer, ctx) = shaped_fixture("abcd\r\nx\r\nabcd");
    buffer.set_selections(vec![Selection::caret(3)], 0).unwrap();
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(7)]);
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        vec![key(Key::ArrowDown, Modifiers::NONE)],
        500.0,
    );
    assert_eq!(buffer.selections().selections(), &[Selection::caret(12)]);
    let output = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let pos = pos2(
        output.gutter.center().x,
        output.geometry.caret_rect(6).center().y,
    );
    frame(
        &mut surface,
        &mut buffer,
        &ctx,
        click(pos, Modifiers::NONE),
        500.0,
    );
    assert_eq!(
        buffer.selections().selections(),
        &[Selection { anchor: 6, head: 9 }]
    );
    assert!(!buffer.is_dirty());
}
