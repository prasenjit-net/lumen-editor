//! The current-line band and indent guides: drawn behind the text from the
//! shared geometry, off by default, and never touching text or selections.
use super::{fixture, frame};
use crate::Cues;
use egui::{Context, Event, Id, Key, Modifiers, Pos2, RawInput, Rect, Shape, vec2};
use lumen_editor_core::EditorBuffer;

const SOURCE: &str = "fn main() {\n    if x {\n        a();\n\n        b();\n    }\n}\n";

fn both() -> Cues {
    Cues {
        current_line: true,
        indent_guides: true,
    }
}

fn shapes(
    surface: &mut crate::EditorSurface,
    buffer: &mut EditorBuffer,
    ctx: &Context,
    events: Vec<Event>,
) -> (Vec<Shape>, crate::SurfaceOutput) {
    let mut result = None;
    let output = ctx.run(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 250.0))),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(surface.show(ui, Id::new("test_surface"), buffer));
            });
        },
    );
    (
        output
            .shapes
            .into_iter()
            .map(|clipped| clipped.shape)
            .collect(),
        result.unwrap(),
    )
}

fn bands(shapes: &[Shape], ctx: &Context, clip: Rect) -> Vec<Rect> {
    let band = ctx.style().visuals.text_color().linear_multiply(0.07);
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if rect.fill == band && rect.rect.width() >= clip.width() - 1.0 => {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect()
}

fn guides(shapes: &[Shape]) -> Vec<(f32, f32, egui::Color32)> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points, stroke }
                if points[0].x == points[1].x && stroke.width == 1.0 =>
            {
                Some((points[0].x, points[0].y, stroke.color))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn cues_are_off_by_default_and_draw_nothing() {
    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    assert!(bands(&drawn, &ctx, output.text_clip).is_empty());
    assert!(guides(&drawn).is_empty());
}

#[test]
fn the_current_line_band_covers_the_caret_row_and_follows_it() {
    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    surface.set_cues(both());
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    let found = bands(&drawn, &ctx, output.text_clip);
    assert_eq!(found.len(), 1, "one caret, one band");
    let rows = output.geometry.row_rects();
    assert_eq!(found[0].top(), rows[0].top());
    assert_eq!(found[0].bottom(), rows[0].bottom());

    let down = Event::Key {
        key: Key::ArrowDown,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    shapes(&mut surface, &mut buffer, &ctx, vec![down]);
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    let moved = bands(&drawn, &ctx, output.text_clip);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].top(), output.geometry.row_rects()[1].top());
}

#[test]
fn a_selection_has_no_band_and_an_unfocused_editor_has_none_either() {
    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    surface.set_cues(both());
    let select_all = Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    shapes(&mut surface, &mut buffer, &ctx, vec![select_all]);
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    assert!(
        bands(&drawn, &ctx, output.text_clip).is_empty(),
        "selection replaces the band"
    );

    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    surface.set_cues(both());
    ctx.memory_mut(|memory| memory.surrender_focus(Id::new("test_surface")));
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    assert!(bands(&drawn, &ctx, output.text_clip).is_empty());
    assert!(!guides(&drawn).is_empty(), "guides do not depend on focus");
}

#[test]
fn indent_guides_follow_each_step_and_run_through_blank_rows() {
    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    surface.set_cues(Cues {
        current_line: false,
        indent_guides: true,
    });
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    let rows = output.geometry.row_rects();
    let found = guides(&drawn);
    let at_row = |row: usize| {
        let mut xs: Vec<f32> = found
            .iter()
            .filter(|(_, y, _)| *y == rows[row].top())
            .map(|(x, ..)| *x)
            .collect();
        xs.sort_by(f32::total_cmp);
        xs
    };
    assert!(at_row(0).is_empty(), "no guide on an unindented row");
    assert_eq!(at_row(1).len(), 1);
    assert_eq!(at_row(2).len(), 2);
    assert_eq!(
        at_row(3).len(),
        2,
        "the blank row inside the inner block keeps both"
    );
    assert_eq!(at_row(4).len(), 2);
    assert_eq!(at_row(5).len(), 1);
    assert!(at_row(6).is_empty());
    let (first, second) = (at_row(2)[0], at_row(2)[1]);
    let glyph = output.geometry.row_x_after(2, 8) - output.geometry.row_x_after(2, 4);
    assert!(
        (second - first - glyph).abs() < 1.5,
        "one step is four columns wide: {first} {second} {glyph}"
    );
}

#[test]
fn the_caret_s_block_guide_is_emphasised_only_inside_that_block() {
    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    surface.set_cues(Cues {
        current_line: false,
        indent_guides: true,
    });
    let down = || Event::Key {
        key: Key::ArrowDown,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    // Caret to row 2 (inside the inner block, `a();`).
    shapes(&mut surface, &mut buffer, &ctx, vec![down()]);
    shapes(&mut surface, &mut buffer, &ctx, vec![down()]);
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    let rows = output.geometry.row_rects();
    let found = guides(&drawn);
    let strong = ctx.style().visuals.text_color().linear_multiply(0.38);
    let emphasised = |row: usize| {
        found
            .iter()
            .filter(|(_, y, color)| *y == rows[row].top() && *color == strong)
            .count()
    };
    assert_eq!(emphasised(2), 1);
    assert_eq!(
        emphasised(4),
        1,
        "the blank row inside the block belongs to it"
    );
    assert_eq!(emphasised(5), 0, "the block has ended");
    assert_eq!(emphasised(1), 0);
}

#[test]
fn cues_never_change_text_selection_history_or_geometry() {
    let (mut surface, mut buffer, ctx) = fixture(SOURCE);
    let plain = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    let caret = plain.geometry.caret_rect(3);
    let (revision, selections) = (buffer.revision(), buffer.selections().clone());
    surface.set_cues(both());
    let cued = frame(&mut surface, &mut buffer, &ctx, vec![], 500.0);
    assert_eq!(cued.geometry.caret_rect(3), caret);
    assert_eq!(buffer.revision(), revision);
    assert_eq!(*buffer.selections(), selections);
    assert!(!buffer.is_dirty());
}

#[test]
fn tab_indented_text_gets_guides_too() {
    let (mut surface, mut buffer, ctx) = fixture("a {\n\tb {\n\t\tc\n\t}\n}\n");
    surface.set_cues(Cues {
        current_line: false,
        indent_guides: true,
    });
    let (drawn, output) = shapes(&mut surface, &mut buffer, &ctx, vec![]);
    let rows = output.geometry.row_rects();
    let count = |row: usize| {
        guides(&drawn)
            .iter()
            .filter(|(_, y, _)| *y == rows[row].top())
            .count()
    };
    assert_eq!((count(1), count(2)), (1, 2));
}
