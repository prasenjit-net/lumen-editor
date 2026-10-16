//! Keeps two side-by-side panes scrolled to the same line.
//!
//! Panes report where they ended up each frame; the pane the user moved becomes
//! the leader and the other pane is told where to go on the next frame. Offsets
//! are in lines, so panes with different row heights stay aligned.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScrollSync {
    seen: [f32; 2],
    observed: [Option<f32>; 2],
    forced: [bool; 2],
    pending: [Option<f32>; 2],
}

const EPSILON: f32 = 0.05;

impl ScrollSync {
    /// Fresh state for a different diff: send both panes to the top.
    pub(crate) fn at_top() -> Self {
        Self {
            pending: [Some(0.0); 2],
            ..Self::default()
        }
    }

    /// Last settled position of each pane, in lines.
    #[cfg(test)]
    pub(crate) fn positions(&self) -> [f32; 2] {
        self.seen
    }

    /// Line the pane must jump to before it is drawn this frame, if any.
    pub(crate) fn take_pending(&mut self, pane: usize) -> Option<f32> {
        let target = self.pending[pane].take();
        self.forced[pane] = target.is_some();
        target
    }

    /// Where the pane actually is after drawing (clamped by its content).
    pub(crate) fn observe(&mut self, pane: usize, lines: f32) {
        self.observed[pane] = Some(lines);
    }

    /// Decide, after both panes drew, whether one pane must follow the other.
    /// Returns true when another frame is needed to apply the follow-up.
    pub(crate) fn finish_frame(&mut self) -> bool {
        let mut moved = [false; 2];
        for (pane, moved) in moved.iter_mut().enumerate() {
            if let Some(now) = self.observed[pane] {
                // A pane we just repositioned moved because we asked it to.
                *moved = !self.forced[pane] && (now - self.seen[pane]).abs() > EPSILON;
                self.seen[pane] = now;
            }
        }
        self.observed = [None; 2];
        self.forced = [false; 2];
        match moved {
            [true, _] => self.pending[1] = Some(self.seen[0]),
            [false, true] => self.pending[0] = Some(self.seen[1]),
            [false, false] => {}
        }
        self.pending.iter().any(Option::is_some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sync: &mut ScrollSync, left: f32, right: f32) -> (Option<f32>, Option<f32>, bool) {
        let apply = (sync.take_pending(0), sync.take_pending(1));
        // A pane that was told to move reports that position.
        sync.observe(0, apply.0.unwrap_or(left));
        sync.observe(1, apply.1.unwrap_or(right));
        (apply.0, apply.1, sync.finish_frame())
    }

    #[test]
    fn the_pane_the_user_moves_leads_and_the_other_follows_once() {
        let mut sync = ScrollSync::default();
        assert_eq!(frame(&mut sync, 0.0, 0.0), (None, None, false));
        // User scrolls the left pane to line 40.
        assert_eq!(frame(&mut sync, 40.0, 0.0), (None, None, true));
        assert_eq!(frame(&mut sync, 40.0, 0.0), (None, Some(40.0), false));
        // Settled: no further movement is requested.
        assert_eq!(frame(&mut sync, 40.0, 40.0), (None, None, false));
        // Now the right pane leads.
        assert_eq!(frame(&mut sync, 40.0, 12.0), (None, None, true));
        assert_eq!(frame(&mut sync, 40.0, 12.0), (Some(12.0), None, false));
    }

    #[test]
    fn a_follower_clamped_by_short_content_does_not_pull_the_leader_back() {
        let mut sync = ScrollSync::default();
        frame(&mut sync, 100.0, 0.0);
        // The right pane can only reach line 30 and reports that.
        let report = |sync: &mut ScrollSync| {
            sync.take_pending(1);
            sync.observe(0, 100.0);
            sync.observe(1, 30.0);
            sync.finish_frame()
        };
        report(&mut sync);
        for _ in 0..4 {
            assert!(
                !report(&mut sync),
                "stays settled; the leader keeps line 100"
            );
        }
    }

    #[test]
    fn panes_with_nothing_to_report_never_request_scrolling() {
        let mut sync = ScrollSync::default();
        assert!(!sync.finish_frame());
        assert_eq!(sync.take_pending(0), None);
    }
}
