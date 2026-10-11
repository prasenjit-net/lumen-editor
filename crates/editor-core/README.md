# Multi-cursor backend spike

Pure text/selection/transaction core; no UI or filesystem dependencies. It is a
workspace member and shared logical-line policy dependency. TextEdit remains the
default application's editing/history owner. With `--features shared-editor`,
the workbench uses this core exclusively for document edits/history; no document
TextEdit history exists alongside it. This experimental build is not default
production adoption.

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
large-file design. Native IME/clipboard/accessibility validation, shaping and
default promotion remain pending. Feature-gated workbench integration is implemented.

The surface spike now exercises this core with actual multi-location input.
Horizontal movement/Shift extension and deletion operate on complete graphemes;
deletion history restores the original selection set, not temporary edit ranges.
Composition snapshots revision/selections without mutating preedit into the
document. One valid commit broadcasts atomically; stale and empty commits are
non-mutating. Whole-document replacement also preserves original selections in
undo, including accessibility SetValue transactions.

Next/all occurrence selection uses exact literals and grapheme-aware word
seeding (letters/digits/underscore); no regex, language tokenization or case folding.
Indentation deduplicates touched logical lines, excludes an end at the next line
start, remaps both endpoints and keeps direction in one history entry. Clipboard
fragments are explicit, immutable session metadata in document order; distribution
requires exact payload and cursor-count matches, otherwise paste broadcasts.
All-empty selections copy/cut deduplicated logical lines using the rules shared
with the default app. Matching explicit line metadata inserts before each logical
line and restores original columns; unknown external text is never inferred to be
line content. Mixed selections retain empty fragments without copying full lines.
Cross-process MIME metadata and destination newline conversion remain open.

Run `cargo test -p lumen-editor-core`.
