# Editor backend decision

Date: 2026-10-10. Status: interim migration decision; full backend spike pending.

## Decision

Current experimental decision (2026-10-11): `--features shared-editor` selects
the core-owned document editor and cosmic-text/Swash surface for the workbench.
Every document mutation/history route in that build uses the core; generic
inputs still use TextEdit. Native screenshots and headless integration tests
validate this path, but do not approve default promotion. The default decision
below remains in force until bidi affinity, native validation of the new surface
and broader parity/performance gates pass. See
[the integration report](shared-editor-integration.md).

Retain egui `TextEdit` as the only editing and history authority while fixing
document ownership. Assign stable IDs to documents and derive text/scroll widget
IDs from them. Observe completed widget edits to update revisions and compare
content against the last successful save. Do not introduce a competing undo
stack or pretend extra painted carets provide multi-cursor editing.

This first step preserves the current rendering and platform input path. It
does not settle the final multi-cursor backend choice.

## Evidence

The installed egui 0.33.3 sources are the reference for this decision:

- `src/widgets/text_edit/state.rs`: widget history stores
  `(CCursorRange, String)`; public state load/store supports stable widget IDs.
- `src/widgets/text_edit/output.rs`: layout output exposes galley geometry,
  clipping, and a single cursor range.
- `src/widgets/text_edit/builder.rs`: typing, clipboard, undo and IME processing
  operate on that single range.
- `src/util/undoer.rs`: history also records cursor changes. A cursor-only undo
  can precede the text undo; tests must preserve this existing behavior rather
  than assume every undo changes text.

Headless egui tests in `src/app/tests/editor.rs` verify separate tab cursor/undo
ownership and saved-content dirty tracking. Document IDs also survive path
changes and shifts in tab index. Successful save is the checkpoint boundary;
failed saves must not call `mark_saved`.

## Next decision gate

Before switching editing authority, prove a multi-selection-capable backend
against the Phase 0 gates in the main plan. A dedicated egui text surface backed
by a pure editing engine is a candidate, not an approved implementation yet.
The spike must demonstrate shared atomic history, Unicode/grapheme handling,
scroll and caret geometry, IME composition, native clipboard routing, and
accessibility. Evaluate an existing component against the same gates before
committing to custom rendering.

After selecting that backend, migrate all mutations together to its shared
transaction model and disable/remove TextEdit's independent history path.
Until then, application helpers may query selection/layout but must not maintain
a parallel edit history.

## Known limits of this milestone

### Multi-cursor core spike (2026-10-11)

`crates/editor-core` is an isolated pure-Rust candidate core, not linked into the
application. It normalizes document-ordered byte selections, merges overlaps and
duplicates while preserving the primary member, rejects non-grapheme boundaries,
and broadcasts/distributes replacement fragments atomically. Complete text and
selection sets round-trip through bounded undo/redo snapshots; saved checkpoints
are independent. Tests include CRLF, combining marks, emoji ZWJ sequences,
overlapping/reversed ranges, invalid operations, no-ops and generated combinations.

This proves only the selection/transaction/history part of the backend spike.
It does not approve custom rendering or ship multi-cursor UI. TextEdit is still
the live editor's sole owner; never instantiate both histories for one document.
The next gate is a standalone text-surface prototype plus existing-component
evaluation against galley hit testing, scrolling, IME, clipboard and accessibility.
Only after that gate should all live mutations migrate together. Full snapshots,
linear grapheme scans and a 100-edit cap are explicit prototype limitations,
not the final performance/history policy.

### Current live editor

Continuation: [the text-surface spike](multi-cursor-backend-spike.md) now runs as
an isolated example with real multi-cursor input/history and shared galley
geometry. Existing-component evaluation confirms `egui_code_editor` still uses
single-selection TextEdit. Native screenshots expose unproved shaping, and IME/
accessibility/clipboard gates remain open. No live-backend migration is approved.

Latest continuation adds transient composition/atomic broadcast commits, primary
candidate geometry and AccessKit text/selection/value actions to the isolated
prototype. Headless tests and synthetic preview screenshots pass, but native
IME/AT validation and inline composition reflow remain pending. An opt-in egui
0.36.2 shaping probe passes Latin ligatures and rejects a bundled-font Arabic
source-index mismatch; neither the production renderer nor its dependencies have
been upgraded to this candidate. See the spike evaluation for exact gate evidence.

Combined continuation: inline composition and more editing commands now operate
in the isolated core/surface. cosmic-text 0.19 is an optional **layout probe**, not
a live dependency or final backend decision. Its Arabic source mapping passes
with a macOS font; production adoption still needs the shaping renderer, bidi
affinity/selection geometry, native clipboard/IME/AT checks and full migration
regressions. TextEdit remains the only live owner. See
[the manual checks and migration sequence](native-editor-validation.md).

The gutter and editor now share one two-axis viewport. General input focus still uses
the existing flags; popup commands additionally validate a document target.
The editor now has a context menu and accepts clicks below EOF. There is no
application selection set or multi-cursor support yet. Gutter click/Shift-click
and drag select logical lines using the same galley geometry and stored widget
selection. Gutter drag edge auto-scroll now uses the same viewport's vertical
scroll delta and clamps pointer-to-line mapping to its visible edge. Drag deltas
are time-based and unanimated to avoid accumulating targets beyond document
bounds; normal reveal scrolling retains animation. Release and blocking dialogs
stop the drag. Headless regressions cover stationary-pointer scrolling, reversal,
EOF/BOF clamping, unchanged buffers, and dialog cancellation.
Literal Find/Replace now uses the widget's existing undoer and synthetic input,
not an independent history. Search highlights are previews while the find input
owns focus; closing Find adopts the current result as the editor selection.
Regex and selection-scoped search are not implemented. Empty-selection Copy/Cut
now use pure logical-line rules and the same widget event adapter. Copy leaves
the caret alone; Cut has explicit widget undo boundaries and delegates deletion
to TextEdit. Final-line clipboard text and the deletion range intentionally differ.
Whole-line paste uses explicit session metadata with exact-text matching, following
[VS Code's memory fallback](https://github.com/microsoft/vscode/blob/main/src/vs/editor/browser/controller/editContext/clipboardUtils.ts).
The widget inserts at the logical line start, then its existing selection/history
records a caret restored to the original column on the original text. Selected
pastes remain normal replacements. Native MIME metadata is not exposed by egui;
identical text copied by another app is indistinguishable and metadata does not
survive process restart. The model/adapter retain delivered line endings, but
eframe 0.33.3's native glow/wgpu integrations normalize clipboard CRLF to LF before
emitting Paste. Exact-text line metadata can therefore miss a native CRLF round
trip; direct-event CRLF tests are not native clipboard verification. This transport
boundary and destination-EOL policy remain outstanding. Document keyboard
batches now split at clipboard boundaries and continue across requested repaint
frames, letting TextEdit finish preceding typing/navigation before computing line
clipboard rules. Deferred input captures document ID/revision/caret and is cancelled
with status feedback on invalidation. New keyboard input stays behind older events;
clipboard metadata follows delivered segments instead of raw input batches.
This preserves one widget render and one mutation/history owner per pass. Pointer
gestures and IME use the existing widget path and cancel any deferred keyboard
batch. App-wide shortcuts retain their pre-widget dispatch order; universal event
ordering still needs the future backend/input migration.
Requested popup/native-editor paste now captures document ID, revision and widget
selection. A cancelled snapshot stays cancelled until delivery, even across
caret/focus round trips. Validation runs before and after command dispatch so
Find or another native command cannot redirect the response into a different field.
Rename/save without text changes remain compatible. The native service boundary
now supplies request IDs and explicit text/empty/error results for requested editor
pastes. Reads run serially on a lazy background worker using arboard (already
present transitively); results enter the same validation and TextEdit mutation
path. Superseded/duplicate results are discarded, and errors or worker disconnection
finish requests without edits. Service reads perform no newline conversion. Tests
use an injected clipboard, never the user's clipboard.
Focused-editor keyboard/platform Paste events now synchronously reread service
text at the original event position before input batching. Other fields and
pointer-transition batches retain eframe's untagged, normalized transport. The
interim reread can block and cannot recover an older clipboard snapshot if the
clipboard changes after platform delivery; direct transport ownership remains a
future backend gate. When a platform event accompanies a service result, the
tagged payload is preferred once; when the event already exists at command dispatch,
no duplicate asynchronous read is issued. Live native round trips remain
outstanding; macOS automation captured the window but could not deliver input.
Windows compiles, with runtime validation pending by user direction. This is not
complete clipboard parity.
The temporary
save checkpoint retains one extra text snapshot; large-file memory behavior
must be reviewed with the future buffer/history implementation.
