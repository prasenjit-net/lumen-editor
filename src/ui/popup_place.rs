//! Placement of caret-anchored popups (hover, completion, signature help).
//!
//! egui's own area constraint only nudges a popup back on screen, which can
//! leave it covering the very text it describes. This picks the side with room
//! first and keeps the popup inside the editor horizontally.
use eframe::egui::{Align2, Pos2, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Above,
    Below,
}

pub(crate) struct Placement {
    pub(crate) pos: Pos2,
    pub(crate) pivot: Align2,
}

/// Where to anchor a popup of roughly `size` next to `anchor` (the caret or
/// symbol rectangle). `prefer` wins unless the other side has more room and
/// the preferred side cannot fit the popup. `bounds` is the editor area.
pub(crate) fn place(anchor: Rect, size: Vec2, bounds: Rect, prefer: Side, gap: f32) -> Placement {
    let below = bounds.bottom() - anchor.bottom() - gap;
    let above = anchor.top() - bounds.top() - gap;
    let fits = |room: f32| room >= size.y;
    let side = match prefer {
        Side::Below if !fits(below) && above > below => Side::Above,
        Side::Above if !fits(above) && below > above => Side::Below,
        side => side,
    };
    let max_x = (bounds.right() - size.x).max(bounds.left());
    let x = anchor.left().clamp(bounds.left(), max_x);
    match side {
        Side::Below => Placement {
            pos: Pos2::new(x, anchor.bottom() + gap),
            pivot: Align2::LEFT_TOP,
        },
        Side::Above => Placement {
            pos: Pos2::new(x, anchor.top() - gap),
            pivot: Align2::LEFT_BOTTOM,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(800.0, 600.0))
    }
    fn caret(x: f32, y: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(1.0, 16.0))
    }

    #[test]
    fn prefers_the_requested_side_when_it_fits() {
        let below = place(
            caret(100.0, 100.0),
            Vec2::new(300.0, 150.0),
            bounds(),
            Side::Below,
            2.0,
        );
        assert_eq!(below.pivot, Align2::LEFT_TOP);
        assert_eq!(below.pos, Pos2::new(100.0, 118.0));
        let above = place(
            caret(100.0, 300.0),
            Vec2::new(300.0, 100.0),
            bounds(),
            Side::Above,
            6.0,
        );
        assert_eq!(above.pivot, Align2::LEFT_BOTTOM);
        assert_eq!(above.pos, Pos2::new(100.0, 294.0));
    }

    #[test]
    fn flips_when_the_preferred_side_is_too_short_and_the_other_has_room() {
        let flipped = place(
            caret(100.0, 560.0),
            Vec2::new(300.0, 160.0),
            bounds(),
            Side::Below,
            2.0,
        );
        assert_eq!(flipped.pivot, Align2::LEFT_BOTTOM);
        let flipped = place(
            caret(100.0, 10.0),
            Vec2::new(300.0, 80.0),
            bounds(),
            Side::Above,
            6.0,
        );
        assert_eq!(flipped.pivot, Align2::LEFT_TOP);
    }

    #[test]
    fn keeps_the_preferred_side_when_neither_fits_but_it_has_more_room() {
        let tall = place(
            caret(100.0, 100.0),
            Vec2::new(300.0, 900.0),
            bounds(),
            Side::Below,
            2.0,
        );
        assert_eq!(tall.pivot, Align2::LEFT_TOP, "more room below than above");
    }

    #[test]
    fn stays_inside_the_editor_at_every_horizontal_edge() {
        let size = Vec2::new(300.0, 100.0);
        let right = place(caret(790.0, 100.0), size, bounds(), Side::Below, 2.0);
        assert_eq!(right.pos.x, 500.0);
        let left = place(caret(-20.0, 100.0), size, bounds(), Side::Below, 2.0);
        assert_eq!(left.pos.x, 0.0);
        let narrow = Rect::from_min_size(Pos2::new(40.0, 0.0), Vec2::new(200.0, 600.0));
        let wide_popup = place(caret(100.0, 100.0), size, narrow, Side::Below, 2.0);
        assert_eq!(
            wide_popup.pos.x, 40.0,
            "a popup wider than the editor starts at its edge"
        );
    }
}
