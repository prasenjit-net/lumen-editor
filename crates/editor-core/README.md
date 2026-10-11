# Multi-cursor backend spike

Pure text/selection/transaction core; no UI or filesystem dependencies. It is a
workspace member for tests, **not a dependency of the running editor**. TextEdit
remains the application's only editing/history owner until the backend gate is
met. This crate is not evidence of shipped multi-cursor UI support.

- Endpoints are UTF-8 byte offsets on extended grapheme boundaries (including
  CRLF, combining marks and emoji sequences). Invalid sets are rejected atomically.
- Sets are document-ordered; duplicates/overlaps merge, including edge carets.
  Adjacent nonempty ranges stay separate. A merged primary keeps its direction.
- Replacement broadcasts one fragment or distributes one per canonical selection.
  One transaction updates all locations and collapses selections to final carets.
  Joining graphemes snap carets forward; duplicates then merge.
- Undo/redo restores complete text/selection snapshots. Saves are independent
  checkpoints; no-ops preserve state and redo. History is capped at 100 edits.

Full snapshots and linear scans are correctness-spike choices, not a measured
large-file design. Rendering/hit testing, movement/deletion, clipboard metadata,
native IME behavior, accessibility validation and application migration remain pending.

The surface spike now exercises this core with actual multi-location input.
Horizontal movement/Shift extension and deletion operate on complete graphemes;
deletion history restores the original selection set, not temporary edit ranges.
Composition snapshots revision/selections without mutating preedit into the
document. One valid commit broadcasts atomically; stale and empty commits are
non-mutating. Whole-document replacement also preserves original selections in
undo, including accessibility SetValue transactions.

Run `cargo test -p lumen-editor-core`.
