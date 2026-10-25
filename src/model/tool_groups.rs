//! Groups runs of finished reads and searches in a transcript so long
//! exploration collapses into one summary row. Only the order and the grouping
//! are decided here; every original item stays reachable through its index.
use lumen_agent_core::model::{ItemKind, ToolKind, ToolStatus, TranscriptItem};

/// Fewest consecutive finished reads/searches worth collapsing.
pub(crate) const MIN_GROUP: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Block {
    Item(usize),
    /// Indices of consecutive completed reads and searches, in transcript order.
    Explored(Vec<usize>),
}

fn is_quiet(item: &TranscriptItem) -> bool {
    matches!(
        &item.kind,
        ItemKind::Tool(call)
            if call.status == ToolStatus::Completed
                && matches!(call.kind, ToolKind::Read | ToolKind::Search)
    )
}

/// Running, failed, declined and cancelled calls, and everything that is not a
/// tool call, break a run and stay visible on their own.
pub(crate) fn blocks(items: &[TranscriptItem]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut run: Vec<usize> = Vec::new();
    let flush = |run: &mut Vec<usize>, out: &mut Vec<Block>| {
        if run.len() >= MIN_GROUP {
            out.push(Block::Explored(std::mem::take(run)));
        } else {
            out.extend(run.drain(..).map(Block::Item));
        }
    };
    for (index, item) in items.iter().enumerate() {
        if is_quiet(item) {
            run.push(index);
        } else {
            flush(&mut run, &mut out);
            out.push(Block::Item(index));
        }
    }
    flush(&mut run, &mut out);
    out
}

/// "Read 6 files · 3 searches".
pub(crate) fn summary(items: &[TranscriptItem], group: &[usize]) -> String {
    let count = |kind: ToolKind| {
        group
            .iter()
            .filter(|&&i| matches!(&items[i].kind, ItemKind::Tool(call) if call.kind == kind))
            .count()
    };
    let (reads, searches) = (count(ToolKind::Read), count(ToolKind::Search));
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if reads > 0 {
        parts.push(format!("Read {}", plural(reads, "file", "files")));
    }
    if searches > 0 {
        parts.push(plural(searches, "search", "searches"));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{ItemId, model::ToolCall};

    fn tool(kind: ToolKind, status: ToolStatus) -> TranscriptItem {
        let mut call = ToolCall::new(kind, "t");
        call.status = status;
        TranscriptItem {
            id: ItemId("i".into()),
            turn: None,
            kind: ItemKind::Tool(call),
        }
    }

    fn done(kind: ToolKind) -> TranscriptItem {
        tool(kind, ToolStatus::Completed)
    }

    #[test]
    fn long_quiet_runs_collapse_and_short_ones_do_not() {
        let items = [
            done(ToolKind::Read),
            done(ToolKind::Search),
            done(ToolKind::Read),
            tool(ToolKind::Command, ToolStatus::Completed),
            done(ToolKind::Read),
            done(ToolKind::Read),
        ];
        assert_eq!(
            blocks(&items),
            [
                Block::Explored(vec![0, 1, 2]),
                Block::Item(3),
                Block::Item(4),
                Block::Item(5)
            ]
        );
    }

    #[test]
    fn running_and_failed_calls_break_the_run_and_stay_visible() {
        let items = [
            done(ToolKind::Read),
            done(ToolKind::Read),
            tool(ToolKind::Read, ToolStatus::Failed),
            done(ToolKind::Search),
            done(ToolKind::Search),
            done(ToolKind::Search),
            tool(ToolKind::Read, ToolStatus::Running),
        ];
        assert_eq!(
            blocks(&items),
            [
                Block::Item(0),
                Block::Item(1),
                Block::Item(2),
                Block::Explored(vec![3, 4, 5]),
                Block::Item(6)
            ]
        );
    }

    #[test]
    fn every_item_appears_exactly_once_in_order() {
        let items: Vec<_> = (0..9).map(|_| done(ToolKind::Read)).collect();
        let flat: Vec<usize> = blocks(&items)
            .into_iter()
            .flat_map(|b| match b {
                Block::Item(i) => vec![i],
                Block::Explored(v) => v,
            })
            .collect();
        assert_eq!(flat, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn summary_counts_reads_and_searches() {
        let items = [
            done(ToolKind::Read),
            done(ToolKind::Search),
            done(ToolKind::Read),
        ];
        assert_eq!(summary(&items, &[0, 1, 2]), "Read 2 files · 1 search");
        assert_eq!(summary(&items, &[1]), "1 search");
        assert_eq!(summary(&items, &[0]), "Read 1 file");
    }
}
