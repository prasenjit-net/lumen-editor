# Multi-cursor backend spike

Updated 2026-10-11. Status: isolated correctness/geometry prototype; not approved
for replacing the live editor. No multi-cursor support is claimed in the workbench.

## Existing-component evaluation

Inspected [egui_code_editor's current source](https://github.com/p4ymak/egui_code_editor/blob/main/src/lib.rs)
and the installed egui 0.33.3 source. `CodeEditor::show` delegates editing to
`TextEdit` and returns `TextEditOutput`, so its highlighting/completion conveniences
do not remove the single-selection/history constraint. It is not a drop-in
multi-cursor backend. This evaluation does not rule out other components.

[VS Code's editing reference](https://code.visualstudio.com/docs/editing/codebasics#_multiple-selections-multicursor)
was checked for Alt/Option-click and simultaneous edits. The prototype implements
that pointer gesture; it does not implement VS Code's occurrence or cursor-above/
below shortcuts, configurable modifiers, or complete clipboard policies.

## Prototype architecture

- `crates/editor-core`: pure selection normalization, grapheme boundaries,
  atomic replacements/deletions, movement and full-set undo/redo.
- `crates/editor-surface`: egui geometry and ordered input adapter. It draws a
  galley directly; no TextEdit or extra widget history is involved.
- `editor-surface/examples/spike.rs`: standalone in-memory fixture using the
  existing theme and fonts. Neither crate is linked into the live application.

CRLF display projection has an explicit rendered-character-to-source-byte map.
Painting, caret rectangles, pointer snapping and gutter rows use one galley.
Grapheme-interior hits snap to the visually closer boundary. Horizontal scrolling
moves text but not the gutter. Selection/keyboard tests exercise real edits, not
only painted carets. Undo restores original carets after deletion, not temporary
expanded deletion ranges. Native focus loss and stale drag revisions reject input.

## Gates

Current verification: 182 default workspace tests and 185 all-feature tests pass
(135 app, 20 core, 27 surface, plus 3 opt-in shaping probes). Formatting, strict
all-target/all-feature Clippy, Windows all-target/all-feature workspace compilation
and macOS bundling pass. Native-rendered synthetic preedit screenshots were
inspected at 300/900/1400 widths. Diagnostic input/focus is injected deliberately;
this is not live native IME, accessibility or clipboard validation. Windows runtime
is still pending by user direction.

Selection normalization, Unicode-safe transactions, atomic history, source/layout
mapping, viewport geometry and basic input ordering have headless coverage.
Native screenshots at 300/900/1400 logical-pixel widths verify fixture rendering
only; they are not live pointer/clipboard/IME tests. Screenshot review corrected
the diagnostic header's initial clipping.

Composition now captures revision and the complete selection set. Preedit is
transient; one commit broadcasts to the captured cursors as one history entry.
Only the primary preview is shown; secondaries dim and a notice explains the
policy. Focus/widget identity/pointer/revision/selection changes cancel the
session; late, duplicate and empty commits are non-mutating. Candidate output
uses preview geometry and shared scroll, and is withheld when disabled/unfocused.
The preview is an overlay, not inline reflow: it can obscure following source
text. Native CJK/dead-key interaction and cancellation still require live checks.

AccessKit now publishes editable rows and the primary selection using the same
galley. Selection actions validate row ownership, bounds and grapheme boundaries;
SetValue uses the core's atomic whole-document transaction. Accessible selection
changes collapse secondary cursors, explicitly described on the node. Headless
tree/action tests are not VoiceOver/Narrator validation or complete AT navigation
coverage. The adapter uses public Id values/NodeIds with pinned egui 0.33 mapping;
the tests verify the emitted tree IDs rather than calling private egui APIs.

Native clipboard ownership, full native IME/accessibility behavior, inline
composition layout, and complex-script/bidi shaping remain unproved. These are
concrete reasons **not** to migrate the workbench yet.

## Shaping candidate result

[egui's 0.35 shaping changes](https://github.com/emilk/egui/blob/0.36.2/crates/epaint/CHANGELOG.md)
add HarfRust. An opt-in, pinned egui 0.36.2 dependency tests this path separately
without upgrading the live app or active prototype renderer. Latin `ffi` produces
fewer rendered glyph quads while keeping source character positions. Basic Latin,
combining, emoji and CJK fixture positions remain bounded; this does not prove
glyph coverage or correct script shaping.

The bundled-font Arabic fixture `العربية` reports 13 end character positions for
7 source characters. The probe records **rejection** of that source-index gate,
not a passing RTL capability. Reevaluate if fonts/version change. Current 0.33
screenshots still show ZWJ emoji as separate glyphs. Neither candidate is approved
for complete shaping/bidi editing. No dependency upgrade is justified solely by
the passing Latin ligature probe.

Run `cargo test -p lumen-editor-surface --features shaping-probe` to reproduce.

Next: investigate the Arabic source-index defect/font coverage or evaluate a
shaping-capable alternative; implement inline composition reflow and run native
IME/AT checks. Then decide the final backend. Only after approval migrate all live
mutations/history together; preserve stable document IDs, file workflows, find,
clipboard rules, scroll state and dirty checkpoints. Snapshot storage/history
limits and linear per-event layout are prototype choices, not performance claims.
