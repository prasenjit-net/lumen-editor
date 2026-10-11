//! Optional newer-egui evaluation; the app/surface still use pinned egui 0.33.
use egui_shaping::{Context, FontId, RawInput, text::CCursor};

#[test]
fn newer_shaper_retains_source_characters_and_usable_caret_geometry() {
    let ctx = Context::default();
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        for text in ["office ffi", "e\u{301}", "👩‍💻", "é\n漢字"] {
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    text.into(),
                    FontId::proportional(14.0),
                    egui_shaping::Color32::WHITE,
                )
            });
            assert_eq!(galley.text(), text);
            assert_eq!(galley.end().index, text.chars().count().into());
            for index in 0..=text.chars().count() {
                let rect = galley.pos_from_cursor(CCursor::new(index));
                assert!(rect.is_finite());
                assert!(
                    galley.cursor_from_pos(rect.center().to_vec2()).index
                        <= text.chars().count().into()
                );
            }
        }
    });
    output.textures_delta.clear(); // No renderer in this headless capability probe.
}

#[test]
fn supported_ligature_cluster_renders_fewer_glyph_quads_than_characters() {
    let ctx = Context::default();
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        let galley = ui.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                "ffi".into(),
                FontId::proportional(20.0),
                egui_shaping::Color32::WHITE,
            )
        });
        assert_eq!(galley.rows[0].glyphs.len(), 3); // source positions survive shaping
        let vertices = galley.rows[0].visuals.glyph_vertex_range.len();
        assert!(
            vertices > 0 && vertices < 12,
            "expected a ligature, got {vertices} vertices"
        );
    });
    output.textures_delta.clear();
}

#[test]
fn arabic_candidate_geometry_fails_the_source_index_gate() {
    // A pinned 0.36.2/bundled-font probe currently reports 13 char positions for
    // seven source characters. Record rejection, not a successful RTL claim.
    let ctx = Context::default();
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        let text = "العربية";
        let galley = ui.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                text.into(),
                FontId::proportional(14.0),
                egui_shaping::Color32::WHITE,
            )
        });
        assert_eq!(galley.text(), text);
        assert_ne!(
            galley.end().index,
            text.chars().count().into(),
            "Reevaluate the backend gate: the pinned Arabic defect no longer reproduces"
        );
    });
    output.textures_delta.clear();
}
