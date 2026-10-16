# Shared editor surface

The default example is the original egui-only geometry fixture:

```sh
cargo run -p lumen-editor-surface --example spike
```

To use the new cosmic-text/Swash renderer:

```sh
cargo run -p lumen-editor-surface --example spike --features shaped -- 900
```

Both examples edit memory only. The full workbench uses this shaped surface by
default with `cargo run` from the project root. It supports real open/save
operations; use disposable buffers for native acceptance testing, which remains
pending. `cargo run --no-default-features` retains the legacy TextEdit backend.

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

The standalone example uses eframe's clipboard transport, including its CRLF
normalization. The workbench retains injected tagged clipboard services and their
documented synchronous-reread latency/snapshot race. Internal metadata is session-
only; identical externally copied payloads cannot be distinguished.

Synthetic screenshot mode (not native IME proof):

```sh
cargo run -p lumen-editor-surface --example spike --features shaped -- 900 --ime-preview
```

Optional isolated probes:

```sh
cargo test -p lumen-editor-surface --features shaping-probe
cargo test -p lumen-editor-surface --features cosmic-probe
```

The pinned newer-egui Arabic source-index defect still rejects that candidate.
The cosmic probe passes bounded source/Arabic caret checks; font coverage and
visual correctness are separate checks. The user reported passing native macOS
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
