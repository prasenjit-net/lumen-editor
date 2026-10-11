# Multi-cursor text-surface spike

Run from the repository root:

```sh
cargo run -p lumen-editor-surface --example spike
```

This is a separate in-memory fixture, **not the Lumen workbench**. It does not
open/save files or attach a second history to production documents. The example
reuses the existing workbench theme and platform font setup.

Implemented in the prototype:

- Alt/Option-click adds a cursor; ordinary click collapses the set. Shift-click
  extends the primary anchor; drag/release updates the same selection set.
- Typing and platform Paste events replace every selection atomically.
- Left/Right (with optional Shift) move by extended grapheme; Backspace/Delete
  delete graphemes or selections. Enter inserts LF. Escape removes secondaries.
- Cmd/Ctrl+A selects all; Cmd/Ctrl+Z and Shift+Cmd/Ctrl+Z undo/redo complete sets.
- One no-wrap galley maps stored UTF-8 byte offsets to rendered character
  positions for all selections, carets and pointer hits. CRLF projects to one
  newline without rewriting source text. The gutter uses the same row geometry.
- Both scroll axes share a viewport; gutter x stays fixed. Reserved scrollbar
  regions and gutter clicks do not reposition carets. Only the primary caret is
  revealed after selection/edit changes. Right-click preserves selection.

Composition and accessibility now have headless prototype support:

- Preedit is a transient underlined overlay at the primary selection, not a
  document edit. Secondary carets dim; the visible notice explains that one
  commit broadcasts to the captured set. Undo restores the full original set.
- Focus, widget identity, pointer, revision or selection changes cancel the
  session. Empty/duplicate/late commits cannot delete or double-insert text.
  Candidate rectangles follow the primary preview and shared scroll geometry.
- AccessKit exposes editable text rows and the primary selection. Setting an
  accessible selection validates row/character/grapheme boundaries and collapses
  secondary selections. SetValue uses one core transaction, not another history.

Synthetic screenshot fixture (forces diagnostic focus; **not native IME testing**):

```sh
cargo run -p lumen-editor-surface --example spike -- 900 --ime-preview
```

Not implemented/approved: native clipboard service routing and multi-fragment
metadata, vertical/word/column movement, occurrence commands, indentation,
context menus, gutter line selection, robust drag auto-scroll, native IME/AT
runtime validation, inline composition reflow, shaping/bidi, layout caching and
live app migration. The preedit overlay may obscure following source text; it
does not yet reflow an inline composition projection. Accessibility represents
one primary selection, not multiple native selections, and full screen-reader
navigation/action coverage remains unverified.
Default eframe clipboard normalization still applies to this prototype's native
Paste events. Input tests inject text, not the user's clipboard.

egui's per-character glyph layout can paint a ZWJ emoji as separate glyphs;
grapheme-safe source mutation does not imply faithful complex-script rendering.
An opt-in egui 0.36.2 probe verifies a Latin ligature, but rejects the bundled-font
Arabic fixture's inconsistent source character count. Neither the application
nor the active prototype renderer has been upgraded. Run the probe with:

```sh
cargo test -p lumen-editor-surface --features shaping-probe
```

The backend decision gate remains open. See
[the evaluation](../../plan/multi-cursor-backend-spike.md).
