# Text editor improvement plan

Status: implementation started; the first document-state foundation is implemented.
Prepared: 2026-10-10. Applies to the Rust/egui Lumen editor and `AGENTS.md`.

## Implementation progress

- Added stable document IDs, document-scoped text/scroll widget IDs, revision
  tracking, and a last-successful-save content checkpoint.
- Added headless egui regressions for independent tab cursor/undo state,
  identity after rename/tab removal, and dirty tracking across undo/edit.
- Kept egui as the sole input and undo authority for this migration step.
  See [the backend decision](editor-backend-decision.md) for its limitations.
- Added an editor context menu with Undo/Redo, selected-text Cut/Copy, Paste,
  and Select All. Commands target a stable document ID and use the same widget
  event adapter as native editing commands. Stale popup actions and delayed
  popup paste are rejected when the document/input focus or modal state changes.
- Expanded the text widget to cover the editor viewport, including blank space
  below EOF; click-to-EOF and context-menu interaction have regression coverage.
- Added per-document literal Find/Replace: seed from selection/word, selected
  query on open, match count, Enter/Shift+Enter navigation, match outlines and
  reveal, case/whole-word options, current/all replacement, and Escape to return
  focus. Native menus, shortcuts, palette, and editor context menu share commands.
- Replacement uses the existing TextEdit history via one synthetic selection
  replacement, with explicit before/after undo boundaries. Replace All is built
  from one text snapshot and does not search its inserted replacement again.
  Tests cover undo/redo, dirty checkpoint, Unicode/CRLF, stale revisions, blocked
  dialogs, another open buffer, and narrow/default/wide control layouts.
- Replaced independent gutter/text scrolling with one two-axis viewport. Long
  lines do not wrap; the gutter stays fixed horizontally and uses actual galley
  row geometry. Added line click, Shift-click, and gutter drag selection. Tests
  cover vertical/horizontal geometry, tab scroll ownership, scrollbar isolation,
  Unicode/CRLF line copying, and find reveal. Screenshot mode verified native
  rendering; live desktop capture stalled, so live pointer interaction was not
  verified on this milestone.
- Phases 0 and 1 are **not complete**: typed focus routing, shared edit
  transactions, and the multi-cursor backend remain outstanding. Regex, selection
  scope, multiline input controls, gutter drag edge auto-scroll,
  empty-selection line clipboard behavior, and multi-cursor editing remain
  unimplemented. Phases 3 and 4 are only partially done.

## Objective

Build a reliable, VS Code-inspired editing experience with an editor context
menu, find and replace, correct cut/copy/paste, safe pointer handling throughout
the editor, and genuine multi-cursor editing. Preserve file operations, native
platform behavior, unsaved buffers, and the existing compact visual style.

Use [VS Code's editing reference](https://code.visualstudio.com/docs/editing/codebasics)
as the behavioral reference for find controls and multiple selections. This is
not a promise to reproduce Monaco, its complete shortcut set, or its internals.
Confirm detailed interactions against a current reference before implementation.

## Current state and gaps

Initial inspection before this plan's implementation found:

- `src/ui/editor.rs` renders tabs, breadcrumbs, line numbers, scrolling, and a
  single `TextEdit::multiline` together. It retains the widget's `Response`,
  not its full selection/layout output.
- Line numbers and text use independent vertical `ScrollArea`s. They can drift
  independently, and their row metrics are calculated separately.
- The text widget has no explicit document-scoped ID. Its auto-generated ID
  occupies the same UI position when tabs switch; cursor, scroll, and undo state
  need explicit ownership to prevent accidental cross-document reuse.
- Clicking anywhere inside the central panel clears `explorer_focused`, but
  does not establish a precise editor input target or reliably position the
  caret in whitespace outside the text widget.
- `src/app/dispatch.rs` routes clipboard actions using focus flags and injects
  egui events. Undo/redo/select-all use synthetic key events. These mechanisms
  need a clear target, particularly with native menus, context menus, and search
  fields competing for focus.
- Documents contain text and a dirty flag, but no stable document ID, document
  revision, application-owned selection set, or application-owned edit history.
- There is no editor context menu, document find/replace control, or multi-cursor
  model. The existing `ViewState.search` is shared by unrelated workbench views;
  it must not become the editor's find state.
- The pinned egui 0.33.3 implementation stores one `CCursorRange` in text edit
  state/history. Adding extra painted carets would not make its input handling
  or undo machinery multi-cursor-aware.

The last point was verified in the installed dependency source:
`egui-0.33.3/src/widgets/text_edit/{state,output,builder}.rs`. Its
`TextEditOutput` exposes a galley, its screen position, a text clipping rectangle,
and one cursor range. Use these public interfaces where appropriate; do not rely
on private egui fields or introduce an unmaintained fork without a documented
decision.

## Design decisions and invariants

1. **One editing authority.** Typing, paste, cut, delete, replace, undo, and
   multi-cursor edits must use the same transaction/history model. Do not run an
   application undo stack alongside an independent widget undo stack for the
   same document.
2. **Stable identity.** Introduce a `DocumentId` independent of tab index and
   file path. Closing/reordering tabs and renaming files must not move another
   document's cursor, history, find state, or scroll position.
3. **Explicit input ownership.** Use a typed focus target such as
   `Editor(DocumentId)`, `Explorer`, `FindInput`, `ReplaceInput`,
   `ExplorerNameInput`, `PaletteInput`, and `Modal`. Native menus temporarily
   change OS focus; retain a validated return target, not a broad boolean guess.
4. **Validated positions.** Store edit ranges as typed, validated UTF-8 byte
   offsets; convert explicitly to egui character indices and rendered positions.
   Caret movement/deletion follows grapheme boundaries. Never slice a `String`
   using screen columns, character counts, or unvalidated offsets.
5. **Atomic edits.** Build replacements against one document snapshot, normalize
   overlapping selections, apply disjoint replacements from highest offset to
   lowest, then remap all selections. A user action is one undoable transaction.
6. **Dirty state reflects saved content/history.** Keep a saved checkpoint that
   survives undo/redo, rather than assuming every changed response is permanently
   dirty. Saving updates it only after successful I/O. No-op edits do not create
   history entries or dirty the document.
7. **No feature-driven redesign.** Reuse theme colors, Codicons, fonts, subtle
   rounding, compact spacing, and native menus. Editing enhancements should not
   change the established workbench appearance or explorer semantics.
8. **Scope is open-document editing.** Workspace-wide search/replace, syntax
   intelligence, split editor groups, minimaps, collaborative editing, and disk
   watchers are separate projects. Keep seams for them without implementing them
   speculatively.

## Proposed component layout

Names below are proposed; use existing modules when a new file is unnecessary.

| Layer | Components | Responsibility |
| --- | --- | --- |
| `model/` | document IDs/revisions; `text/selection.rs`, `transaction.rs`, `history.rs`, `search.rs` | Pure text operations, selections, history, search results |
| `app/` | `editor_commands.rs`, `focus.rs`, per-document editor sessions | Route commands to a validated target; own interaction/session state |
| `services/` | clipboard boundary, only if required by the chosen backend | Testable platform clipboard access; no clipboard I/O in model code |
| `ui/editor/` | tabs, breadcrumbs, viewport, gutter, text surface, context menu, find bar | Render state; hit-test coordinates; issue typed commands |
| `platform/` | native menu and shortcut integration | Platform accelerators and contextual command enablement |

An editor session owns selections, primary caret, preferred visual column,
scroll position, composition state, and document-scoped find state. Search UI
visibility and focus are separate from document text. Use one source for command
capabilities so disabled context-menu and native-menu actions agree.

Keep `String` initially. Add a rope/piece table or new dependencies only when the
capability spike or measured performance justifies them. Clean module boundaries
matter more than creating a large abstract editor framework.

## Implementation phases

### Phase 0 — Capability spike and baseline

- [ ] Capture a stable editor fixture and current open/save/dirty-buffer tests.
- [ ] Prototype public `TextEdit::show` access: cursor restore, point-to-caret
  mapping, explicit IDs, scroll offsets, and external transaction integration.
- [ ] Evaluate whether an existing egui-native component can meet multi-cursor,
  IME, accessibility, license, and maintenance requirements. Do not select one
  based on screenshots alone.
- [ ] Prefer a dedicated egui text surface backed by a pure edit engine if no
  suitable component passes the spike. Reuse egui layout/painter/input services;
  do not embed a browser/Monaco as an incidental implementation change.
- [ ] Record the backend decision and migration boundary in this directory.

**Exit gate:** prove the planned backend can own selections and history, map
clicks after scrolling, handle Unicode input, position an IME candidate window,
and expose editable-text accessibility. Stock `TextEdit` may remain a temporary
single-cursor adapter, but cannot be the authority for genuine multi-cursor edits.
If that adapter cannot share history correctly, replace it before mutation
features ship rather than maintaining two competing undo systems.

### Phase 1 — Stable sessions and shared command routing

- [ ] Add stable document IDs and document-scoped widget/scroll IDs.
- [ ] Preserve each tab's caret, selection, scroll, and history across switches.
- [ ] Introduce typed focus ownership and an editor command target containing
  a document ID, not a captured tab index.
- [ ] Route native menus, keyboard shortcuts, palette actions, and future context
  menus to the same commands. Ensure an accelerator is handled exactly once.
- [ ] Keep native text-field actions working in find, replace, palette, and
  explorer naming inputs; explorer file clipboard actions remain separate.
- [ ] Introduce normalized selections and atomic edit transactions with undo/redo
  and saved checkpoints, initially supporting one selection.

**Exit gate:** two tabs with different contents cannot share undo or cursor state;
closing a command's target safely cancels it; failed saves preserve dirty state;
undoing to saved content clears dirty state and redoing restores it.

### Phase 2 — Viewport, pointer safety, and single-cursor editing

- [ ] Split rendering into focused components without gratuitous widget-ID changes.
- [ ] Use one scroll/layout model for text, line numbers, hit testing, highlights,
  and caret reveal. Add horizontal scrolling for long lines; wrapping is an
  explicit mode with defined gutter behavior, not an accidental widget default.
- [ ] Compute hit regions from the current rendered layout, clipping, font metrics,
  scroll offset, and scale factor. Clamp mapped positions to valid document bounds.
- [ ] Support single click, Shift+click, word double-click, line triple-click,
  drag selection, and bounded auto-scroll while dragging beyond the viewport.
- [ ] Implement navigation, insertion, Backspace/Delete, newline, Tab/Shift+Tab,
  undo/redo, and selection replacement through the shared engine.
- [ ] Preserve document newline conventions; do not silently normalize CRLF/LF.
- [ ] Ensure right-click can target text without destroying a selection containing
  the click. A click outside that selection moves the context caret predictably.

Pointer behavior contract:

| Editor region | Intended handling |
| --- | --- |
| Rendered text | Focus document and map to the nearest legal caret boundary |
| Whitespace right of a line | Place caret at that line's end; preserve Shift selection semantics |
| Whitespace below the final line | Focus document and clamp to EOF, without inserting lines |
| Line-number gutter | Select logical line; Shift extends by lines; drag selects lines |
| Scrollbars | Scroll only; do not move the caret or start text selection |
| Tabs, close buttons, plus, breadcrumbs | Run only their own actions; do not click through to text |
| Find/replace control or context menu | Route input to that control; preserve editor selection |
| Modal or clipped/outside area | Block underlying editor interaction; no stale hit testing |

**Exit gate:** all regions work on empty files, short files, long/scrolled files,
resized panes, and HiDPI displays without panics, out-of-bounds selections,
unintended text mutations, or selection changes through overlaid controls.

### Phase 3 — Context menu and clipboard completeness

- [ ] Add compact Undo, Redo, Cut, Copy, Paste, Select All, Find, and Replace
  entries using the existing theme and platform shortcut labels.
- [ ] Preserve the editor target/selection while the popup owns focus. Dismissing
  it returns focus without typing into a different tab or input field.
- [ ] With a selection, copy/cut exactly that text. With an empty selection,
  define and test current-line copy/cut, including empty and final lines.
- [ ] Capture paste target and document revision when requesting the system
  clipboard; validate/remap on delivery, or cancel if the target closed/changed
  incompatibly. Paste must not land in whichever field became focused later.
- [ ] Treat paste as text, preserve intentional multiline content, and apply the
  chosen newline policy explicitly. Empty clipboard and platform failures are safe.
- [ ] Context actions, native Edit menu, and keyboard actions use identical
  capability checks and transactions. Copy/navigation do not dirty text;
  cut/paste each produce one undo step.
- [ ] Keep generic native text-field editing in search/palette inputs and retain
  explorer clipboard routing. Never reuse file-path clipboard state for text.

**Exit gate:** selected text and line operations behave identically across all
entry points, and asynchronous clipboard delivery cannot affect the wrong target.

### Phase 4 — Find and replace in the active document

- [ ] Add a compact editor-local find control at the top-right with a replace
  expansion, query/replacement inputs, match count, previous/next, and close.
  Clamp it to narrow editor widths; overlays must not capture unrelated clicks.
- [ ] Cmd/Ctrl+F opens find; use verified platform defaults for replace. Seed
  from the selection/word, navigate with Enter/Shift+Enter inside find, and return
  focus to the document on Escape without changing text.
- [ ] Support literal search, case matching, whole-word matching, selection scope,
  wrap-around navigation, visible match highlights, and active-match reveal.
- [ ] Add regex mode and capture-group replacement after literal behavior is
  stable. Evaluate a bounded implementation; document its syntax differences from
  VS Code rather than claiming exact regex-engine parity.
- [ ] Report invalid expressions and invalid replacement references without
  mutating text. Literal replacements must not interpret capture syntax.
- [ ] Handle multiline queries and zero-length matches with guaranteed forward
  progress. Keep match ranges in original document coordinates; Unicode case
  handling must not derive offsets from a length-changing lowercase copy.
- [ ] Replace current match and Replace All use the transaction engine. Replace
  All builds edits from one immutable snapshot and is one undo step.
- [ ] Freeze/remap find-in-selection bounds deliberately, rather than reusing the
  temporary selection of the currently highlighted match as the search scope.
- [ ] Recompute results on document revision/query/options changes. Navigation
  and highlighting do not dirty content; stale matches cannot be replaced.

**Exit gate:** no-match, invalid regex, zero-width, overlapping candidates,
selection-scoped, Unicode, and CRLF cases are covered; Replace All fully undoes
and never modifies another open document or saves files implicitly.

### Phase 5 — Genuine multi-cursor core

- [ ] Maintain one primary selection plus ordered secondary selections; preserve
  anchors, direction, and each caret's preferred visual column.
- [ ] Normalize duplicate carets and overlapping ranges before every transaction
  so text is never inserted/deleted twice at the same location.
- [ ] Add Alt/Option+click, add cursor above/below, select-next-occurrence, and
  select-all-occurrences with platform-appropriate commands. Ordinary click
  collapses to one cursor; Escape first removes secondary cursors when editing.
- [ ] Apply typing, selection replacement, deletion, newline, indentation,
  movement, cut, paste, undo, and redo to all selections through shared commands.
- [ ] Define multi-selection clipboard order in document order. Distribute copied
  fragments one-to-one when counts match; otherwise apply the documented full-text
  paste policy to each cursor. Deduplicate line operations for carets on one line.
- [ ] Restore the complete selection set with undo/redo and across tab switches.
- [ ] Paint all selections/carets from the same geometry as hit testing; reveal
  the primary caret without oscillating the viewport between distant cursors.
- [ ] Show cursor/selection count in the status bar and prevent unbounded
  select-all-occurrences from freezing the UI; make any limit explicit.

**Exit gate:** a single action changes every intended location exactly once,
survives Unicode and overlapping-selection cases, and undoes/redoes atomically.
Extra visual carets without multi-location input/history are not acceptance.

### Phase 6 — Advanced selection, composition, and accessibility

- [ ] Add box/column selection and rectangular paste with an explicit virtual
  column policy for short lines and tabs; reuse actual rendered column geometry.
- [ ] Support drag auto-scroll and word/line extension with multiple selections.
- [ ] Handle IME preedit as transient composition, not repeated document edits;
  commit once, avoid accidental duplication, and position candidate UI at the
  primary caret. Test CJK composition and dead-key input on native systems.
- [ ] Decide and document multi-cursor IME behavior before release. If secondary
  cursors must be suspended during composition, visibly explain that limitation;
  do not silently corrupt text or advertise untested composition parity.
- [ ] Expose editable text, selection, caret changes, commands, and find results
  through accessibility. Preserve keyboard-only workflows and native clipboard.

**Exit gate:** composition cannot lose or duplicate characters; keyboard-only
editing is usable; platform accessibility and selection state remain coherent.

### Phase 7 — Performance and release hardening

- [ ] Cache line indexes and layout by document revision/font/viewport width;
  avoid scanning or painting the entire document on every idle frame.
- [ ] Measure typing, scrolling, replace-all, and many-cursor workloads before
  adding complex storage. Cache search results and discard stale background work
  by document ID/revision if large searches are moved off the UI thread.
- [ ] Set measured budgets on reference hardware: start with a 16ms interaction
  frame target for ordinary fixtures; publish tested document/cursor limits and
  profile large files, long lines, and pathological queries separately.
- [ ] Bound history and cached layouts with documented policies. Do not evict an
  unsaved buffer, invalidate the saved checkpoint, or block input unpredictably.
- [ ] Re-run existing file/explorer tests and native packaging checks; update
  `README.md` and `AGENTS.md` only with capabilities actually delivered.

**Exit gate:** no editor regressions in existing workflows, acceptable measured
performance, and honest documentation of remaining limits.

## Verification matrix

- **Model tests:** grapheme-safe movement; UTF-8 range validation; reversed,
  duplicate and overlapping selections; edit remapping; no-op transactions;
  history grouping; undo/redo and saved checkpoints; search/replacement semantics.
- **Property tests:** applying then undoing a valid generated edit restores text
  and selections; no selection exceeds bounds; unrelated document state is inert.
- **Application tests:** command target/focus routing; closed tabs; tab switches;
  async paste after selection/document changes; menu capabilities; save failure;
  rename/delete interactions with open document sessions.
- **egui interaction tests:** all pointer regions in the table; popup layering;
  drag release outside the viewport; input ownership; find controls; independent
  per-tab scroll; empty documents; gutter/text alignment after scrolling/resizing.
- **Native checks:** macOS and Windows clipboard/menu accelerators, IME, selection,
  HiDPI, focus loss/regain, and keyboard-only editing. A cross-compile is not a
  substitute for native runtime validation. Treat Linux modifier conflicts as a
  separate check if claiming Linux runtime support.
- **Fixtures:** empty text; trailing newline/no trailing newline; CRLF/LF;
  tabs and long lines; emoji/combining marks/CJK; RTL text; many matches; large
  files; many carets. Document any shaping/bidi limitation explicitly.
- **Repository gates:** `cargo fmt --all --check`, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, Windows compilation,
  and macOS bundling. Visually compare unchanged workbench areas to a baseline.

## Delivery order and completion criteria

Implement one phase per reviewable change. Phase 0 is the decision gate; phases
1–2 establish correctness; phase 3 delivers context/clipboard; phase 4 delivers
find/replace; phases 5–6 deliver multi-cursor and advanced input; phase 7 hardens
the result. Do not postpone the shared history/selection model until after
independently implementing replace or multi-cursor mutations.

The requested scope is complete only when context menus, find/replace, clipboard,
safe viewport interactions, and real multi-cursor edits all meet their exit
gates. Interim releases must identify which phases and limitations remain.
Keep this plan's checkboxes current as work is verified. Do not commit or push
implementation work without a user request.
