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
selection. Gutter drag edge auto-scroll remains outstanding.
Literal Find/Replace now uses the widget's existing undoer and synthetic input,
not an independent history. Search highlights are previews while the find input
owns focus; closing Find adopts the current result as the editor selection.
Regex and selection-scoped search are not implemented. Cut/Copy require selected text. The temporary
save checkpoint retains one extra text snapshot; large-file memory behavior
must be reviewed with the future buffer/history implementation.
