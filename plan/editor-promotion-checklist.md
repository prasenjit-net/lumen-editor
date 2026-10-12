# Shared editor promotion batch

Updated 2026-10-12. All six areas are in scope, but default promotion is
conditional on acceptance results. Compilation and screenshots cannot replace
native IME/clipboard/screen-reader validation.

**Decision update:** the user explicitly requested "commit push and promote
default" on 2026-10-12. The shared editor is now the Cargo default and the normal
macOS bundle backend. This supersedes the earlier proposed promotion gate, not
the outstanding native test results. `cargo run --no-default-features` and
`cargo xtask bundle --legacy-editor` provide a legacy fallback. Legacy adapters
are retained until acceptance; neither native platform is newly claimed validated.

Promotion verification: 256 default, 222 legacy and 262 all-feature workspace
tests pass, plus formatting, strict Clippy for all three configurations, Windows
default/all-feature compilation and normal/legacy/validation macOS bundles.

Local verification: 221 default / 260 all-feature workspace tests, formatting,
strict default/all-feature Clippy, Windows all-target/all-feature compilation,
both macOS bundles and native-rendered narrow/default/wide bidi screenshots.
The 131 legacy app tests remain a comparator; 17 app tests explicitly select the
shared workbench. This is not a claim that all legacy widget-specific tests were
converted into shared-backend acceptance tests.

| Area | This batch | Remaining acceptance |
| --- | --- | --- |
| Long-line performance | Indexed glyph/cluster lookups; immutable shaped-layout cache; cached character geometry | Native release measurements on target hardware |
| Mixed-direction editing | Core-owned anchor/head affinity; visual grapheme arrows; pointer side selection; undo preservation; disjoint highlights | Manual shaped-workbench bidi navigation and selection checks |
| Editing parity | Save cancellation/failure, delayed paste affinity guards, native/popup undo, vertical goals, word/line endpoints, indentation and tab state | Broader native interaction acceptance; grapheme-safe Find rejection accepted by user |
| Scale | Repeatable 10,000-row, 1,000-cursor, long-line and 1 MB history probes | Larger/native fixtures; full-snapshot history still capped at 100 groups |
| Native validation | Windows compilation; macOS packaging; 300/900/1400 native-rendered bidi screenshots | User will test macOS; Windows runtime remains pending by user direction |
| Default promotion | Enabled by explicit user request; separate legacy fallback retained | Native acceptance and eventual legacy adapter retirement remain pending |

## Ownership and invalidation

Affinity is model state, not a second UI selection or history stack. It is part of
the normalized selection snapshot, survives undo/redo and participates in delayed
paste/composition validation. Logical commands preserve anchor affinity during
extension. Normalization remaps/merges affinity with its owning endpoints.

The surface holds one cached immutable shaped layout, keyed by egui's galley
identity. Text/font/scale/style changes create a different identity; source maps
are rebuilt independently, so LF and CRLF projections cannot share the wrong
source offsets. Composition projection participates in the key. Document switches
clear the cache. Character positions and visual arrow stops are lazy per-layout
indices, not independent viewport/scroll authorities.

Visual arrows operate on legal grapheme edges in visual row/x order. Pointer
mapping uses the same edges, avoiding the backend's multi-grapheme RTL hit
interpolation. Selection rectangles still come from the shaped runs. This is not
a claim of exact VS Code word navigation or exhaustive Unicode bidi conformance;
embedding/control-heavy documents and native input methods need acceptance tests.

## Remaining behavior decisions

- A Find replacement that cuts through an extended grapheme is rejected with
  feedback rather than silently expanding its edit range. This deliberately
  differs from legacy character-range replacement. The user explicitly accepted
  grapheme-safe rejection on 2026-10-12; retain the feedback and documentation.
- Clipboard metadata is session-only and matched by exact text. Native platform
  paste still has the previously documented synchronous-reread race/latency.
- Saved file contents and history use full strings. Snapshot bounds are not a
  byte-budget guarantee for very large documents; the profile reports payload
  lengths, not complete allocator/RSS measurements.

## Promotion procedure after acceptance

1. Record macOS version, input method, bundle/build and pass/fail results in
   [native validation](native-editor-validation.md). Preserve Windows runtime as
   pending unless actual results arrive. Resolve the behavior decisions above.
2. Enable the shared backend for normal launches and packaging in one explicit
   migration. Keep a tested legacy launch/build escape hatch during acceptance;
   do not enable both document histories simultaneously.
3. Run default/all-feature tests, strict Clippy, Windows compilation and both
   macOS bundles again. Test open/save/save-as, close/quit cancellation, rename,
   deletion of dirty files, duplicate tabs, clipboard errors and native menus.
4. Retire document TextEdit adapters only after the promoted build is accepted.
   Generic Find/Replace/name inputs keep TextEdit; do not delete those widgets.

Do not describe all six acceptance areas as complete. Default configuration has
been promoted by user request, while native acceptance and adapter retirement
remain open. The procedure above is the original proposed gated rollout.
