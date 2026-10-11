//! Optional layout-only candidate. Embedded fonts plus an explicit macOS font
//! coverage test; no renderer replacement, editing/history or live integration.
use cosmic_text::{Attrs, Buffer, Cursor, Family, FontSystem, Metrics, Shaping, Wrap, fontdb};
use unicode_segmentation::UnicodeSegmentation;

fn shape(text: &str) -> Buffer {
    shape_with_font(text, None)
}

fn shape_with_font(text: &str, arabic_font: Option<Vec<u8>>) -> Buffer {
    let mut database = fontdb::Database::new();
    for font in egui::FontDefinitions::default().font_data.values() {
        database.load_font_data(font.font.to_vec());
    }
    if let Some(font) = arabic_font {
        database.load_font_data(font);
    }
    database.set_sans_serif_family("Ubuntu");
    let mut fonts = FontSystem::new_with_locale_and_db("en-US".into(), database);
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(14.0, 18.0));
    buffer.set_size(Some(1000.0), Some(500.0));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        text,
        &Attrs::new().family(Family::SansSerif),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut fonts, false);
    buffer
}

#[test]
fn arabic_clusters_keep_source_bytes_without_claiming_bundled_font_coverage() {
    let text = "العربية";
    let buffer = shape(text);
    let run = buffer.layout_runs().next().unwrap();
    assert!(run.rtl);
    assert!(!run.glyphs.is_empty());
    for glyph in run.glyphs {
        assert!(text.is_char_boundary(glyph.start));
        assert!(text.is_char_boundary(glyph.end));
        assert!(glyph.start < glyph.end && glyph.end <= text.len());
    }
    for byte in text
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .chain([text.len()])
    {
        let cursor = Cursor::new(0, byte);
        let (x, _) = buffer.cursor_position(&cursor).expect("source caret");
        let hit = buffer.hit(x, run.line_top + run.line_height / 2.0).unwrap();
        assert_eq!(hit.index, byte, "Arabic source byte {byte}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_arabic_font_coverage_and_caret_mapping() {
    // Native font read is confined to this opt-in platform test, not UI code.
    let bytes =
        std::fs::read("/System/Library/Fonts/Supplemental/Arial.ttf").expect("macOS Arial fixture");
    let text = "العربية";
    let buffer = shape_with_font(text, Some(bytes));
    let run = buffer.layout_runs().next().unwrap();
    assert!(run.rtl);
    assert!(
        run.glyphs.iter().all(|glyph| glyph.glyph_id != 0),
        "Missing glyph cannot establish script coverage"
    );
    for byte in text
        .char_indices()
        .map(|(byte, _)| byte)
        .chain([text.len()])
    {
        let (x, _) = buffer.cursor_position(&Cursor::new(0, byte)).unwrap();
        let hit = buffer.hit(x, run.line_top + run.line_height / 2.0).unwrap();
        assert_eq!(hit.index, byte);
    }
}

#[test]
fn mixed_bidi_ligatures_and_combining_layouts_never_invent_source_indices() {
    for text in ["office ffi", "é", "abc العربية xyz", "漢字", "👩‍💻"] {
        let buffer = shape(text);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                assert!(text.is_char_boundary(glyph.start));
                assert!(text.is_char_boundary(glyph.end));
                assert!(glyph.end <= text.len());
                let hit = buffer
                    .hit(
                        glyph.x + glyph.w / 2.0,
                        run.line_top + run.line_height / 2.0,
                    )
                    .unwrap();
                assert!(text.is_char_boundary(hit.index));
                assert!(hit.index <= text.len());
            }
        }
    }
}
