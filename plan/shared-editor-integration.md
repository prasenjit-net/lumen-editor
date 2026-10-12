# Shared editor integration

Updated 2026-10-12. Promoted by explicit user request to the default build.
Native macOS/Windows acceptance remains pending; promotion does not imply those
checks passed. Legacy adapters remain as a build-time fallback.

```sh
cargo run
cargo xtask bundle --offline
cargo run --no-default-features
cargo xtask bundle --offline --legacy-editor
```

Normal packaging creates `target/Lumen.app` with the shared editor. Legacy
packaging creates `target/Lumen Legacy.app` with a distinct bundle identifier.
`--shared-editor` still creates the separate validation bundle. All use native menus.

## Ownership and routes

| Area | Legacy fallback | Default shared-editor build |
| --- | --- | --- |
| Document mutations/history | TextEdit | Document-owned EditorBuffer only |
| Stored file contents | Document text | Read-only document snapshot of the core |
| Selection | Widget character range | Normalized grapheme-safe source byte set |
| Keyboard/clipboard order | Temporary segmented queue | Immediate ordered core events |
| Clipboard line policy | Shared pure line rules + widget deletion | Same rules + atomic core deletion/paste |
| Find edits | Synthetic widget replacement | Validated core range replacement |
| Popup/native commands | Common typed actions/menu | Same actions/menu, no document TextEdit |
| Geometry/rendering | egui galley | cosmic-text layout + Swash textures |
| Save/dialog/filesystem | Injected app services | Unchanged injected app services |

Generic Find/Replace/name inputs keep their own TextEdit histories; they never
edit the document. `Document.text` is a file/UI projection in shared mode, not
another editing owner. `sync_shared_editing` updates contents, revision and dirty
state after core actions. Save checkpoints update only on successful writes.
No runtime backend toggle is exposed; a shared document cannot render TextEdit.

Popup paste captures document/revision/primary selection plus the full shared
set. Existing sticky cancellation, tagged service reads, duplicates/errors and
focus/modal guards remain in the controller. The old service reread's latency/
snapshot race and session-only metadata limitations are not claimed resolved.

## Checks and evidence

The follow-up reports below record pre-promotion checkpoints. Current default
configuration is described at the top; pending native checks are not marked passed.

### Completion batch (2026-10-12)

See [the six-area checklist](editor-promotion-checklist.md) for current promotion
status. This batch adds immutable shape caching, indexed cluster geometry,
core-owned caret affinity, visual grapheme arrows/pointer mapping, retained
affinity in undo/selection extension and delayed-paste/composition guards.
Native-rendered bidi fixtures were reviewed at 300/900/1400 widths. Scale probes
now include 10,000 rows and 1,000 cursors. Grapheme-safe Find rejection was
explicitly accepted by the user and stays documented.

The user will run the final macOS workbench checklist. Windows runtime remains
pending by user direction. Default promotion and removal of legacy document
adapters are intentionally not executed before native macOS acceptance.

### Long-line and gutter follow-up

The headless probe includes a long Unicode line. Gutter labels use shared row
geometry but only vertically visible rows produce paint commands. A regression
verifies that empty visible rows retain numbers and offscreen document rows do
not add labels. No accessibility rows, document contents or caret state are culled.

### Accessibility row-index follow-up

Shaped accessibility character geometry uses one per-frame run index and the
renderer backend's unchanged cursor-position policy instead of repeatedly
scanning all rows. Full rows/tree semantics remain intact. Two additional
regressions compare bulk coordinates with renderer coordinates and published
tree positions, including CRLF and preedit. Release probe results are recorded
in [the baseline](editor-performance-baseline.md); native VoiceOver validation
and mixed-bidi editing remain separate gates.

### Typing/history and tab parity follow-up

The core now groups consecutive single-line text events within a one-second
pause into a single undo snapshot, restoring the original complete selection set.
The input adapter supplies time; the core has no clock/UI dependency. Every edit
still advances revision. Save checkpoints, navigation/selection changes, focus
or tab changes, clipboard commands and IME boundaries split groups. Paste,
Find/Replace, indentation, deletion and IME commits remain separate atomic edits.
Newlines and invalid/backwards clock readings do not join a typing group.
Deletion coalescing and finer word/punctuation grouping are not implemented.

Regression tests cover multi-cursor combining characters, redo branching, the
100-group cap, save checkpoints, raw key releases, clipboard order and focus
loss. Shaped workbench tests confirm document-ID-based two-axis scroll and
selection restoration on tab return; the existing native Undo deduplication
test now explicitly separates its two edit groups.

Repeatable probes and measured limits are in
[the performance baseline](editor-performance-baseline.md). Disabled AccessKit
no longer builds per-character shaped geometry; enabling it later still publishes
complete rows. Enabled accessibility and large-file layout remain promotion gates.
Sizing and painting reuse one geometry when input has not changed source or
preedit; a changed text/IME pass relayouts before publishing carets or highlights.

Current local verification: 216 default / 244 all-feature workspace tests,
formatting, strict default/all-feature Clippy, Windows cross-compilation and both
macOS bundles. No new native runtime IME or screen-reader validation is claimed.

### Vertical navigation follow-up

Every cursor now keeps an independent preferred visual column across Up/Down,
selection extension and page navigation. Key releases and passive pointer motion
do not reset it. Cached goals require matching revision and full selection set;
edits, selection changes, focus loss and document switches invalidate them.
Adding cursors remaps goals into document order. Cursor merges deliberately start
a fresh column sequence rather than assigning an ambiguous merged goal.
Six surface regressions and one shaped-workbench regression cover short lines,
anchors, reverse travel, passive events, stale revision/selection, cursor ordering,
merges and horizontal resets. This is not mixed-bidi visual navigation parity.

Follow-up verification: 207 default and 232 all-feature workspace tests pass,
along with formatting, strict default/all-feature Clippy, Windows compilation
and both macOS bundles. These are automated checks, not new native runtime
IME/accessibility validation. The evidence below describes the original
integration checkpoint.

- 201 default and 225 all-feature workspace tests on macOS (including one bundler
  test). 11 app integration tests explicitly select the shaped workbench; 131
  legacy app tests are a separate comparator, not exhaustive shared-mode parity.
- Formatting and strict default/all-feature all-target Clippy.
- Windows all-target/all-feature compilation; no Windows native runtime claim.
- Both macOS bundles build. Experimental identity/executable metadata has a test.
- Native-rendered Unicode workbench fixture and 300/900/1400 synthetic preedit
  screenshots. Retina baseline/selection span regressions were fixed by review.
- The user reported all five preceding prototype macOS manual checks passed;
  the changed renderer/accessibility path requires its own native validation.

## Before default promotion

1. Core affinity and visual grapheme arrow behavior now have regression coverage,
   including disjoint direction-boundary selections. Confirm native behavior,
   embedding/control-heavy documents and configured fonts on both platforms;
   do not infer exhaustive Unicode bidi conformance from these fixtures.
2. Repeat IME/candidate/cancellation, clipboard and VoiceOver checks on the shaped
   workbench using [the checklist](native-editor-validation.md). Keep Windows
   runtime explicitly pending until a host is available.
3. Port more legacy interaction regressions to shared mode; review partial-
   grapheme Find semantics and word/line-edge details. Typing grouping, scroll
   identity and multi-cursor vertical goals have local regression coverage above;
   further native/parity validation is still required.
   A rejected range must stay
   non-mutating and report feedback, not silently expand to unrelated text.
4. Measure large-file input/layout/accessibility costs and history storage. Current
   full snapshots are capped at 100 undo groups; layout is still per-event. Glyph
   textures/raster entries use a bounded cache with visible-run culling, not a
   production atlas/performance claim. The source character map is indexed once.
5. Only after those gates, promote the core build and remove document TextEdit
   adapters. Do not remove generic-field widgets or instantiate both histories.
