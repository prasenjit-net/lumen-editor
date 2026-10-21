# Shared editor surface

The full workbench uses this shaped surface with `cargo run` from the project
root. It supports real open/save operations; use disposable buffers for native
acceptance testing, which remains pending. Profiling: `cargo run --release -p
lumen-editor-surface --example profile`.

Implemented:

- Multi-location typing, selection-only and logical-line copy/cut/paste, explicit
  session fragment distribution, full-set undo/redo and saved checkpoints.
- Option/Alt-click, primary/Shift-click/drag and right-click selection preservation.
- Option/Alt-drag adds a selected range; Shift+Option/Alt-drag selects a column
  using rendered rows. Shift+Option/Alt+I places cursors at selected line ends.
- Next/all occurrence shortcuts; native cursor-above/below; grapheme, word,
  vertical, page and line navigation; Tab/Shift+Tab indentation.
- One two-axis viewport with fixed gutter, reserved scrollbars, line selection
  and bounded gutter edge scrolling. No implicit code wrapping.
- Inline transient primary preedit; one atomic broadcast commit; sticky cancellation
  on document, revision, selection, focus, modal or pointer changes. Candidate
  positioning and reveal use the same layout as rendered text.
- Accessible editable rows, validated primary selection/value actions and unique
  parent ownership. Preedit-interior accessible positions cannot become source
  selections. Setting an accessible selection collapses secondaries.
- The shaped path derives painting, disjoint selection spans, hit testing, carets,
  IME, gutter and Find geometry from cosmic-text. Fonts are supplied by platform
  startup. Swash glyph textures honor DPI and have bounded cache/visible culling.
  An egui galley remains for source bookkeeping, not shaped coordinates.
- Lexical syntax spans color the shared egui and shaped paint paths without
  changing their source mapping, editing state, selection or undo history.

The workbench uses injected tagged clipboard services with a documented
synchronous-reread latency/snapshot race. Internal metadata is session-only;
identical externally copied payloads cannot be distinguished.

The pinned newer-egui Arabic source-index defect rejected that candidate (see
`plans/editor-backend-decision.md`). The user reported passing native macOS
checks for the earlier egui-only prototype; new shaped IME/AT checks remain open.

Implemented experimentally: core affinity/visual grapheme arrows, independent
vertical goals, typing coalescing, shape/character caches and indexed glyph lookup.
Scale probes and coordinate/cache regression coverage are recorded in the
integration report. Not production-approved: native IME/accessibility behavior,
embedding/control-heavy bidi conformance, complete platform font coverage and
exhaustive workbench parity. Windows has compilation
checks, not native runtime validation.

See [the integration report](../../plans/shared-editor-integration.md) and
[the backend evaluation](../../plans/multi-cursor-backend-spike.md).
