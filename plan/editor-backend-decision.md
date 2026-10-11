# Editor backend decision

Date: 2026-10-10. Status: interim migration decision; full backend spike pending.

## Decision

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
Direct keyboard/platform Paste events and other fields still use eframe's untagged,
normalized transport. When a platform event accompanies a service result, the
tagged payload is preferred once; when the event already exists at command dispatch,
no duplicate read is issued. Live native round trips and migration of keyboard
paste to the service remain outstanding; this is not complete clipboard parity.
The temporary
save checkpoint retains one extra text snapshot; large-file memory behavior
must be reviewed with the future buffer/history implementation.
