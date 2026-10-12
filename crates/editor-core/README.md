# Multi-cursor backend spike

Syntax metadata and keyword lists live in [`syntaxes/`](syntaxes/README.md),
embedded in the binary at compile time. The scanner returns display-only UTF-8
byte spans; it does not own document edits or history.

Pure text/selection/transaction core; no UI or filesystem dependencies. It is a
workspace member and shared logical-line policy dependency. The default application
uses this core exclusively for document edits/history; no document TextEdit history
exists alongside it. `--no-default-features` retains the legacy TextEdit backend.
Default promotion was explicitly requested while native acceptance remains pending.

- Endpoints are UTF-8 byte offsets on extended grapheme boundaries (including
  CRLF, combining marks and emoji sequences). Invalid sets are rejected atomically.
- Sets are document-ordered; duplicates/overlaps merge, including edge carets.
  Adjacent nonempty ranges stay separate. A merged primary keeps its direction.
- Replacement broadcasts one fragment or distributes one per canonical selection.
  One transaction updates all locations and collapses selections to final carets.
  Joining graphemes snap carets forward; duplicates then merge.
- Undo/redo restores complete text/selection snapshots. Saves are independent
  checkpoints; no-ops preserve state and redo. History is capped at 100 undo groups.
- Anchor/head caret affinity is part of normalized selection and history state.
  It survives undo and invalidates delayed paste/composition when the side changes.
- Consecutive single-line typing coalesces within a one-second pause. Clocks are
  supplied by adapters; save, navigation, clipboard and IME boundaries split groups.

Full snapshots and linear scans are correctness-spike choices, not a measured
large-file design. Native IME/clipboard/accessibility validation and
runtime acceptance remains pending. Default workbench integration is implemented;
the legacy backend is still available with default features disabled.

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
fragments are explicit, immutable session metadata in document order; exact
payload and cursor-count matches preserve those fragments. External clipboard
rows distribute when their count matches the number of cursors; otherwise the
full text is pasted at each cursor.
All-empty selections copy/cut deduplicated logical lines using the rules shared
with the default app. Matching explicit line metadata inserts before each logical
line and restores original columns; unknown external text is never inferred to be
line content. Mixed selections retain empty fragments without copying full lines.
Cross-process MIME metadata and destination newline conversion remain open.
Pasting a copied column at a single cursor does not yet spread it across rows
or pad short destination lines to a visual column.

Run `cargo test -p lumen-editor-core`.
