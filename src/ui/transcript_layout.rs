//! Remembers how tall each transcript entry was the last time it was drawn, so a
//! long conversation can draw only what is on screen and reserve blank space for
//! the rest. Heights include the gap after the entry.
const FALLBACK_HEIGHT: f32 = 60.0;
/// Width changes smaller than this keep the measurements (scrollbar flicker).
const WIDTH_TOLERANCE: f32 = 2.0;

#[derive(Clone, Debug, Default)]
pub(crate) struct TranscriptLayout {
    width: f32,
    heights: Vec<Option<f32>>,
}

/// Which entries to draw and how much blank space surrounds them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Window {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) space_before: f32,
    pub(crate) space_after: f32,
}

impl TranscriptLayout {
    /// Call once per frame with the current width and entry count.
    pub(crate) fn prepare(&mut self, width: f32, count: usize) {
        if (self.width - width).abs() > WIDTH_TOLERANCE {
            self.width = width;
            self.heights.clear();
        }
        self.heights.resize(count, None);
    }

    pub(crate) fn clear(&mut self) {
        self.heights.clear();
    }

    pub(crate) fn record(&mut self, index: usize, height: f32) {
        if let Some(slot) = self.heights.get_mut(index) {
            *slot = Some(height);
        }
    }

    fn estimate(&self) -> f32 {
        let (sum, count) = self
            .heights
            .iter()
            .flatten()
            .fold((0.0, 0_u32), |(sum, count), height| {
                (sum + height, count + 1)
            });
        if count == 0 {
            FALLBACK_HEIGHT
        } else {
            sum / count as f32
        }
    }

    fn height(&self, index: usize, estimate: f32) -> f32 {
        self.heights
            .get(index)
            .copied()
            .flatten()
            .unwrap_or(estimate)
    }

    /// Entries intersecting `top..bottom` (entry-area coordinates) plus `margin`
    /// on both sides; the rest becomes blank space of the remembered height.
    pub(crate) fn window(&self, top: f32, bottom: f32, margin: f32) -> Window {
        let estimate = self.estimate();
        let (lo, hi) = (top - margin, bottom + margin);
        let (mut offset, mut start, mut end) = (0.0_f32, None, 0);
        let (mut before, mut drawn_end) = (0.0_f32, 0.0_f32);
        for index in 0..self.heights.len() {
            let height = self.height(index, estimate);
            let entry_end = offset + height;
            if entry_end > lo && offset < hi {
                if start.is_none() {
                    start = Some(index);
                    before = offset;
                }
                end = index + 1;
                drawn_end = entry_end;
            }
            offset = entry_end;
        }
        match start {
            Some(start) => Window {
                start,
                end,
                space_before: before,
                space_after: (offset - drawn_end).max(0.0),
            },
            None => Window {
                start: 0,
                end: 0,
                space_before: 0.0,
                space_after: offset,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(count: usize, height: f32) -> TranscriptLayout {
        let mut layout = TranscriptLayout::default();
        layout.prepare(400.0, count);
        for index in 0..count {
            layout.record(index, height);
        }
        layout
    }

    #[test]
    fn only_entries_near_the_viewport_are_drawn_and_the_rest_keeps_its_height() {
        let layout = layout(100, 50.0); // 5000 px in total
        let window = layout.window(1000.0, 1300.0, 100.0);
        // 900..1400 is drawn: entries 18..28.
        assert_eq!((window.start, window.end), (18, 28));
        assert_eq!(window.space_before, 900.0);
        assert_eq!(
            window.space_before + 10.0 * 50.0 + window.space_after,
            5000.0
        );
    }

    #[test]
    fn short_conversations_are_drawn_whole() {
        let layout = layout(3, 40.0);
        let window = layout.window(0.0, 600.0, 200.0);
        assert_eq!(
            (
                window.start,
                window.end,
                window.space_before,
                window.space_after
            ),
            (0, 3, 0.0, 0.0)
        );
    }

    #[test]
    fn unmeasured_entries_use_the_average_of_the_measured_ones() {
        let mut layout = TranscriptLayout::default();
        layout.prepare(400.0, 10);
        layout.record(0, 100.0);
        layout.record(1, 200.0);
        let window = layout.window(0.0, 10.0, 0.0);
        assert_eq!((window.start, window.end), (0, 1));
        // 100 measured + 9 others at the 150 average.
        assert_eq!(window.space_after, 200.0 + 8.0 * 150.0);
        assert_eq!(TranscriptLayout::default().estimate(), FALLBACK_HEIGHT);
    }

    #[test]
    fn a_real_width_change_forgets_heights_but_a_scrollbar_wobble_does_not() {
        let mut layout = layout(5, 30.0);
        layout.prepare(401.5, 5);
        assert!(layout.heights.iter().all(Option::is_some));
        layout.prepare(300.0, 5);
        assert!(layout.heights.iter().all(Option::is_none));
        layout.prepare(300.0, 8);
        assert_eq!(layout.heights.len(), 8, "new entries start unmeasured");
        layout.prepare(300.0, 2);
        assert_eq!(layout.heights.len(), 2);
        layout.clear();
        assert!(layout.heights.is_empty());
    }

    #[test]
    fn scrolled_past_the_end_still_draws_the_last_entries() {
        let layout = layout(10, 50.0);
        let window = layout.window(5000.0, 5400.0, 100.0);
        assert_eq!((window.start, window.end), (0, 0), "nothing intersects");
        assert_eq!(window.space_after, 500.0, "the whole list is blank space");
        let window = layout.window(400.0, 800.0, 0.0);
        assert_eq!(window.end, 10);
    }
}
