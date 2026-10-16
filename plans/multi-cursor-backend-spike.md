# Multi-cursor backend spike

Updated 2026-10-11. Status: feature-gated shaped workbench integration implemented;
default TextEdit replacement is not approved yet.

## Current integration

`cargo run --features shared-editor` runs the real workbench with core-owned
document history and cosmic-text/Swash geometry/rasterization. Source projection,
painting, selection spans, pointer hits, carets, IME rectangles, gutter rows and
Find highlights share this layout. The retained egui galley supplies logical
character/source bookkeeping only in shaped mode; its glyph positions are not
used by the shaped renderer. Platform startup supplies fonts; UI code does no I/O.

All-feature checks include 131 unchanged legacy app tests plus 11 explicit shaped
workbench tests, 35 core tests, 47 surface tests and one packaging test. The legacy
fixtures deliberately retain TextEdit: passing them does not mean every legacy
scenario has been ported to the shaped backend. Default workspace checks run
201 tests; all-feature checks run 225 on macOS. Formatting, strict Clippy, Windows
compilation and default/experimental macOS bundles are checked separately.

User-reported macOS checklist results passed for the preceding egui-only prototype.
They are recorded in [native validation](native-editor-validation.md), not carried
forward as proof of this new shaped renderer/accessibility combination. Native
screenshots reviewed the workbench Unicode fixture and synthetic preedit widths;
they caught and corrected Retina baseline scaling and out-of-range selection
highlighting. Both bugs now have regressions. Native Windows runtime is pending.

The shared build includes ordered core input, next/all occurrences, cursor-above/
below, vertical/word/line/page navigation, indentation, logical-line clipboard,
explicit fragment distribution, native/popup commands, Find/Replace, saved
checkpoints, and independent tab histories. The default build remains unchanged.
Current-match Find replacement inside a grapheme fails with feedback. Shared
history uses bounded full snapshots; typing coalescing, large-file performance,
mixed-bidi affinity/visual movement and new native IME/AT checks remain promotion
gates. See [the integration report](shared-editor-integration.md).

The sections below record the earlier isolated-prototype evaluation and gates.

## Existing-component evaluation

Inspected [egui_code_editor's current source](https://github.com/p4ymak/egui_code_editor/blob/main/src/lib.rs)
and the installed egui 0.33.3 source. `CodeEditor::show` delegates editing to
`TextEdit` and returns `TextEditOutput`, so its highlighting/completion conveniences
do not remove the single-selection/history constraint. It is not a drop-in
multi-cursor backend. This evaluation does not rule out other components.

[VS Code's editing reference](https://code.visualstudio.com/docs/editing/codebasics#_multiple-selections-multicursor)
was checked for Alt/Option-click and simultaneous edits. The prototype implements
that pointer gesture, next/all literal occurrences, and platform cursor-above/
below shortcuts. Configurable modifiers and complete clipboard policies remain open.

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

## Earlier checkpoint gates

Current verification: 195 default workspace tests and 201 all-feature tests pass
(135 app, 27 core, 33 surface, plus 6 opt-in shaping probes on macOS). Formatting, strict
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
The preview now replaces the primary source range in one transient galley;
following text reflows without changing stored text/history. Source hit tests
and accessibility actions map through that projection; preedit-interior accessible
positions are rejected. The primary preview end is revealed on updates. Native
CJK/dead-key interaction and cancellation still require live checks.

AccessKit now publishes editable rows and the primary selection using the same
galley. Selection actions validate row ownership, bounds and grapheme boundaries;
SetValue uses the core's atomic whole-document transaction. Accessible selection
changes collapse secondary cursors, explicitly described on the node. Headless
tree/action tests are not VoiceOver/Narrator validation or complete AT navigation
coverage. The adapter uses public Id values/NodeIds with pinned egui 0.33 mapping;
the tests verify the emitted tree IDs rather than calling private egui APIs.

Native clipboard ownership, full native IME/accessibility behavior,
and production complex-script/bidi shaping remain unproved. These are
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

Inspection of the pinned `epaint` shaping source indicates a descending-RTL-cluster
problem: continuation emission assumes ascending byte clusters; the final span
starts at byte zero and counts the full run again. This explains the observed
13 positions, but is a source-level diagnosis, not an upstream fix or complete
font-coverage analysis. No installed dependency was patched. See
[the upstream implementation](https://github.com/emilk/egui/blob/0.36.2/crates/epaint/src/text/text_layout.rs).

An optional pinned cosmic-text 0.19 probe evaluates a separate layout authority
through [its public source-byte cursor API](https://docs.rs/cosmic-text/0.19.0/cosmic_text/struct.Buffer.html).
Arabic cluster ranges and caret/hit round trips pass; a macOS-only fixture adds
Arial and explicitly checks nonzero glyph IDs. The platform-independent fixture
uses embedded egui fonts and does not establish Arabic glyph coverage. Mixed bidi,
Latin ligatures, combining, CJK and emoji probes check bounded source indices,
not visual correctness or complete font coverage. This is a layout-only candidate:
there is no cosmic renderer/atlas, accessible geometry adapter, or production
dependency. Mixed-bidi affinity/selection geometry and native checks remain gates.

The prototype also adds occurrence selection, cursor-above/below with retained
horizontal goal, deduplicated indent/outdent, and selection-only copy/cut/paste.
Clipboard distribution requires exact explicit session metadata and matching
selection counts; external payloads broadcast. Different paste text invalidates
session metadata. Empty-selection logical-line policy is not implemented here.

Mac automation was retried and still returned clipboard timeout -10005. The user
agreed to [manual native validation](native-editor-validation.md); those results
remain pending. Next: complete the shaping renderer/geometry adapter and native
IME/AT/clipboard checks, then decide the final backend. Only after approval migrate all live
mutations/history together; preserve stable document IDs, file workflows, find,
clipboard rules, scroll state and dirty checkpoints. Snapshot storage/history
limits and linear per-event layout are prototype choices, not performance claims.
