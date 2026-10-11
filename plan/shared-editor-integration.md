# Shared editor integration

Updated 2026-10-11. Implemented experimentally, not promoted to the default build.

```sh
cargo run --features shared-editor
cargo xtask bundle --offline --shared-editor
```

The bundle is `target/Lumen Experimental.app`, with a distinct bundle identifier;
the normal bundle remains `target/Lumen.app`. Both use native platform menus.

## Ownership and routes

| Area | Default build | Shared-editor build |
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

1. Complete mixed-bidi affinity and visual arrow behavior, including selection
   rectangles at direction boundaries. Logical grapheme movement is not full
   bidi editing parity. Confirm configured fonts on both native platforms.
2. Repeat IME/candidate/cancellation, clipboard and VoiceOver checks on the shaped
   workbench using [the checklist](native-editor-validation.md). Keep Windows
   runtime explicitly pending until a host is available.
3. Port more legacy interaction regressions to shared mode; review partial-
   grapheme Find semantics, typing undo grouping, scroll identity, multi-cursor
   vertical goals and word/line-edge details. A rejected range must stay
   non-mutating and report feedback, not silently expand to unrelated text.
4. Measure large-file input/layout/accessibility costs and history storage. Current
   full snapshots are capped at 100 edits; layout is still per-event. Glyph
   textures/raster entries use a bounded cache with visible-run culling, not a
   production atlas/performance claim. The source character map is indexed once.
5. Only after those gates, promote the core build and remove document TextEdit
   adapters. Do not remove generic-field widgets or instantiate both histories.
