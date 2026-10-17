//! Display-only fold intervals and safety around hidden selections.
use crate::EditorSurface;
use lumen_editor_core::{EditorBuffer, FoldRange, Selection};
use std::{collections::BTreeSet, ops::Range};

impl EditorSurface {
    pub fn set_folding(
        &mut self,
        text: &str,
        ranges: &[FoldRange],
        collapsed: &BTreeSet<usize>,
        collapsed_icon: &'static str,
        expanded_icon: &'static str,
    ) {
        self.fold_ranges = ranges.to_vec();
        self.collapsed = collapsed.clone();
        self.fold_icons = Some((collapsed_icon, expanded_icon));
        self.rebuild_fold_intervals(text);
    }

    fn rebuild_fold_intervals(&mut self, text: &str) {
        if self.collapsed.is_empty() {
            self.fold_intervals.clear();
            return;
        }
        let starts = line_starts(text);
        let mut intervals: Vec<_> = self
            .fold_ranges
            .iter()
            .filter(|range| self.collapsed.contains(&range.header))
            .filter_map(|range| {
                fold_interval(text, &starts, *range).map(|interval| (range.header, interval))
            })
            .collect();
        intervals.sort_by_key(|(_, interval)| interval.start);
        let mut visible: Vec<(usize, Range<usize>)> = Vec::new();
        for (header, interval) in intervals {
            if visible
                .last()
                .is_none_or(|(_, previous)| interval.start >= previous.end)
            {
                visible.push((header, interval));
            }
        }
        self.fold_intervals = visible;
    }

    pub(crate) fn active_fold_intervals(&self) -> Vec<Range<usize>> {
        if self.fold_suspended {
            Vec::new()
        } else {
            self.fold_intervals
                .iter()
                .map(|(_, range)| range.clone())
                .collect()
        }
    }

    pub(crate) fn reveal_hidden_selections(&mut self, buffer: &EditorBuffer) {
        let headers: Vec<_> = self
            .fold_intervals
            .iter()
            .filter(|(_, range)| {
                buffer.selections().selections().iter().any(|selection| {
                    [selection.anchor, selection.head]
                        .iter()
                        .any(|byte| range.start < *byte && *byte < range.end)
                })
            })
            .map(|(header, _)| *header)
            .collect();
        if !headers.is_empty() {
            for header in headers {
                self.collapsed.remove(&header);
            }
            self.rebuild_fold_intervals(buffer.text());
            self.fold_suspended = true;
            self.folds_changed = true;
        }
    }

    pub(crate) fn reveal_hidden_target(&mut self, text: &str) {
        let Some(byte) = self.reveal else {
            return;
        };
        let headers: Vec<_> = self
            .fold_intervals
            .iter()
            .filter(|(_, range)| range.start < byte && byte < range.end)
            .map(|(header, _)| *header)
            .collect();
        if !headers.is_empty() {
            for header in headers {
                self.collapsed.remove(&header);
            }
            self.rebuild_fold_intervals(text);
            self.fold_suspended = true;
            self.folds_changed = true;
        }
    }

    pub(crate) fn toggle_fold(&mut self, header: usize, buffer: &mut EditorBuffer) {
        if self.collapsed.contains(&header) {
            self.collapsed.remove(&header);
        } else {
            let starts = line_starts(buffer.text());
            let Some(interval) = self
                .fold_ranges
                .iter()
                .find(|range| range.header == header)
                .and_then(|range| fold_interval(buffer.text(), &starts, *range))
            else {
                return;
            };
            let selections: Vec<_> = buffer
                .selections()
                .selections()
                .iter()
                .map(|selection| {
                    if [selection.anchor, selection.head]
                        .iter()
                        .any(|byte| interval.start < *byte && *byte < interval.end)
                    {
                        Selection::caret(interval.start)
                    } else {
                        *selection
                    }
                })
                .collect();
            if selections != buffer.selections().selections() {
                let primary = buffer.selections().primary_index();
                buffer.set_selections(selections, primary).unwrap();
            }
            self.collapsed.insert(header);
        }
        self.rebuild_fold_intervals(buffer.text());
        self.folds_changed = true;
    }
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(byte, _)| byte + 1))
        .collect()
}

fn fold_interval(text: &str, starts: &[usize], range: FoldRange) -> Option<Range<usize>> {
    if range.end <= range.header {
        return None;
    }
    let next = *starts.get(range.header + 1)?;
    let end = starts.get(range.end + 1).copied().unwrap_or(text.len());
    let start = if next >= 2 && text.as_bytes()[next - 2] == b'\r' {
        next - 2
    } else {
        next - 1
    };
    (end > start).then_some(start..end)
}
