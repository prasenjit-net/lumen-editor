//! Line diff for previews: common prefix/suffix trimming plus an LCS on the
//! differing middle, shown as hunks with context. Inputs that would need a huge
//! table are reported as one replaced block rather than burning CPU.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
    /// `n` unchanged lines left out between hunks.
    Gap(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct LineDiff {
    pub(crate) lines: Vec<DiffLine>,
    pub(crate) added: usize,
    pub(crate) removed: usize,
}

/// Largest `old_middle * new_middle` the LCS table may reach (about 16 MiB).
const MAX_TABLE: usize = 4_000_000;

/// Lines of `text`; a final newline ends the last line instead of starting a blank one.
fn lines_of(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

pub(crate) fn diff(old: &str, new: &str, context: usize) -> LineDiff {
    // An empty text has no lines (a new or fully deleted file), not one blank line.
    let old_lines = lines_of(old);
    let new_lines = lines_of(new);
    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let max_suffix = old_lines.len().min(new_lines.len()) - prefix;
    let suffix = old_lines
        .iter()
        .rev()
        .zip(new_lines.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let old_mid = &old_lines[prefix..old_lines.len() - suffix];
    let new_mid = &new_lines[prefix..new_lines.len() - suffix];
    let ops = middle_ops(old_mid, new_mid);

    // Expand to a flat list: prefix context, edits, suffix context.
    let mut flat: Vec<DiffLine> = Vec::new();
    flat.extend(
        old_lines[..prefix]
            .iter()
            .map(|l| DiffLine::Context((*l).to_owned())),
    );
    flat.extend(ops);
    flat.extend(
        old_lines[old_lines.len() - suffix..]
            .iter()
            .map(|l| DiffLine::Context((*l).to_owned())),
    );
    let added = flat
        .iter()
        .filter(|l| matches!(l, DiffLine::Added(_)))
        .count();
    let removed = flat
        .iter()
        .filter(|l| matches!(l, DiffLine::Removed(_)))
        .count();
    LineDiff {
        lines: with_context(flat, context),
        added,
        removed,
    }
}

fn middle_ops(old: &[&str], new: &[&str]) -> Vec<DiffLine> {
    let removed = |lines: &[&str]| {
        lines
            .iter()
            .map(|l| DiffLine::Removed((*l).to_owned()))
            .collect::<Vec<_>>()
    };
    let added = |lines: &[&str]| {
        lines
            .iter()
            .map(|l| DiffLine::Added((*l).to_owned()))
            .collect::<Vec<_>>()
    };
    if old.is_empty() {
        return added(new);
    }
    if new.is_empty() {
        return removed(old);
    }
    if old.len().saturating_mul(new.len()) > MAX_TABLE {
        let mut all = removed(old);
        all.extend(added(new));
        return all;
    }
    // lcs[i][j] = length of the LCS of old[i..] and new[j..].
    let width = new.len() + 1;
    let mut lcs = vec![0_u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i * width + j] = if old[i] == new[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            out.push(DiffLine::Context(old[i].to_owned()));
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * width + j] >= lcs[i * width + j + 1] {
            out.push(DiffLine::Removed(old[i].to_owned()));
            i += 1;
        } else {
            out.push(DiffLine::Added(new[j].to_owned()));
            j += 1;
        }
    }
    out.extend(removed(&old[i..]));
    out.extend(added(&new[j..]));
    out
}

/// Keep `context` unchanged lines around each change; fold the rest into gaps.
fn with_context(flat: Vec<DiffLine>, context: usize) -> Vec<DiffLine> {
    let changed: Vec<usize> = flat
        .iter()
        .enumerate()
        .filter(|(_, line)| !matches!(line, DiffLine::Context(_)))
        .map(|(index, _)| index)
        .collect();
    if changed.is_empty() {
        return Vec::new();
    }
    let mut keep = vec![false; flat.len()];
    for index in changed {
        let from = index.saturating_sub(context);
        let to = (index + context + 1).min(flat.len());
        keep[from..to].fill(true);
    }
    let mut out = Vec::new();
    let mut skipped = 0;
    for (line, keep) in flat.into_iter().zip(keep) {
        if keep {
            if skipped > 0 {
                out.push(DiffLine::Gap(skipped));
                skipped = 0;
            }
            out.push(line);
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        out.push(DiffLine::Gap(skipped));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(diff: &LineDiff) -> String {
        diff.lines
            .iter()
            .map(|line| match line {
                DiffLine::Context(t) => format!(" {t}"),
                DiffLine::Added(t) => format!("+{t}"),
                DiffLine::Removed(t) => format!("-{t}"),
                DiffLine::Gap(n) => format!("~{n}"),
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn identical_text_has_no_hunks() {
        let result = diff("a\nb\n", "a\nb\n", 3);
        assert!(result.lines.is_empty());
        assert_eq!((result.added, result.removed), (0, 0));
    }

    #[test]
    fn a_new_file_is_all_additions_and_deleting_everything_is_all_removals() {
        let new_file = diff("", "one\ntwo", 3);
        assert_eq!((new_file.added, new_file.removed), (2, 0));
        let gone = diff("x\ny", "", 3);
        assert_eq!((gone.added, gone.removed), (0, 2));
    }

    #[test]
    fn edits_show_context_and_fold_distant_unchanged_lines() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9";
        let new = "1\n2\n3\n4\nFIVE\n6\n7\n8\n9";
        let result = diff(old, new, 1);
        assert_eq!(kinds(&result), "~3| 4|-5|+FIVE| 6|~3");
        assert_eq!((result.added, result.removed), (1, 1));
    }

    #[test]
    fn insertions_and_deletions_in_the_middle_align_on_the_longest_common_subsequence() {
        let result = diff("a\nb\nc\nd", "a\nc\nX\nd", 5);
        assert_eq!(kinds(&result), " a|-b| c|+X| d");
    }

    #[test]
    fn a_trailing_newline_is_not_an_extra_blank_line() {
        let result = diff("old\n", "new\nmore\n", 3);
        assert_eq!((result.added, result.removed), (2, 1));
        assert_eq!(kinds(&result), "-old|+new|+more");
        assert_eq!(diff("hello\n", "", 3).removed, 1);
        // A real blank line is still a line.
        assert_eq!(diff("a\n\n", "a\n", 3).removed, 1);
    }

    #[test]
    fn enormous_replacements_degrade_to_a_block_instead_of_a_giant_table() {
        let old: String = (0..3000).map(|n| format!("old{n}\n")).collect();
        let new: String = (0..3000).map(|n| format!("new{n}\n")).collect();
        let result = diff(&old, &new, 0);
        assert!(result.removed >= 3000 && result.added >= 3000);
    }
}
