//! Geometry for dragging a tab to a new position, free of any egui state.
use eframe::egui::Rect;

/// Insertion gap for a pointer at `pointer_x`: the number of tabs whose centre
/// lies to its left. `0` is before the first tab and `centres.len()` after the last.
pub(crate) fn insertion_gap(centres: &[f32], pointer_x: f32) -> usize {
    centres.iter().filter(|centre| **centre < pointer_x).count()
}

/// Where the dragged tab ends up if dropped in `gap`. Removing it from `from`
/// shifts every later gap left by one, so a gap right before or after the tab
/// itself means "stay".
pub(crate) fn resulting_position(from: usize, gap: usize) -> usize {
    if gap > from { gap - 1 } else { gap }
}

/// Horizontal position of the insertion marker for `gap`.
pub(crate) fn marker_x(rects: &[Rect], gap: usize) -> Option<f32> {
    match (rects.get(gap), rects.last()) {
        (Some(rect), _) => Some(rect.left() - 1.0),
        (None, Some(last)) => Some(last.right() + 1.0),
        (None, None) => None,
    }
}

/// How fast to scroll the strip while a tab is held near its edge, as a signed
/// scroll delta in points per second (positive reveals tabs to the left).
/// Speed grows with proximity and is capped, so a long hold cannot run away.
pub(crate) fn edge_scroll_speed(
    pointer_x: f32,
    left: f32,
    right: f32,
    margin: f32,
    max: f32,
) -> f32 {
    let into_left = (left + margin - pointer_x).clamp(0.0, margin);
    let into_right = (pointer_x - (right - margin)).clamp(0.0, margin);
    (into_left - into_right) / margin * max
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Pos2, Vec2};

    #[test]
    fn the_gap_is_the_number_of_tab_centres_left_of_the_pointer() {
        let centres = [50.0, 150.0, 250.0];
        assert_eq!(insertion_gap(&centres, 0.0), 0);
        assert_eq!(insertion_gap(&centres, 100.0), 1);
        assert_eq!(insertion_gap(&centres, 200.0), 2);
        assert_eq!(insertion_gap(&centres, 999.0), 3);
        assert_eq!(insertion_gap(&[], 10.0), 0);
    }

    #[test]
    fn dropping_next_to_the_tab_itself_means_stay() {
        // Dragging the middle tab (index 1) of four.
        assert_eq!(resulting_position(1, 0), 0);
        assert_eq!(resulting_position(1, 1), 1, "gap just before itself");
        assert_eq!(resulting_position(1, 2), 1, "gap just after itself");
        assert_eq!(resulting_position(1, 3), 2);
        assert_eq!(resulting_position(1, 4), 3, "after the last tab");
        assert_eq!(resulting_position(0, 4), 3);
        assert_eq!(resulting_position(3, 0), 0);
    }

    #[test]
    fn the_marker_sits_before_a_tab_or_after_the_last_one() {
        let rect = |x: f32| Rect::from_min_size(Pos2::new(x, 0.0), Vec2::new(100.0, 30.0));
        let rects = [rect(0.0), rect(104.0)];
        assert_eq!(marker_x(&rects, 0), Some(-1.0));
        assert_eq!(marker_x(&rects, 1), Some(103.0));
        assert_eq!(marker_x(&rects, 2), Some(205.0));
        assert_eq!(marker_x(&[], 0), None);
    }

    #[test]
    fn edge_scrolling_is_directional_proportional_and_bounded() {
        let speed = |x| edge_scroll_speed(x, 100.0, 500.0, 40.0, 600.0);
        assert_eq!(speed(300.0), 0.0, "no scroll in the middle");
        assert!(speed(110.0) > 0.0, "near the left edge reveals the left");
        assert!(speed(490.0) < 0.0, "near the right edge reveals the right");
        assert!(speed(105.0) > speed(130.0), "closer is faster");
        assert_eq!(speed(0.0), 600.0, "far past the edge stays capped");
        assert_eq!(speed(9999.0), -600.0);
    }
}
