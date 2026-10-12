# Pending development activities

Updated: 2026-10-12.
Baseline: `071a7d2`; the shared editor is the default. The legacy backend remains
available with `cargo run --no-default-features` and the separate legacy bundle.
Default promotion was requested explicitly; it did **not** mark native acceptance
complete. Grapheme-safe Find/Replace rejection was explicitly accepted.

## Editor acceptance and follow-up

### P1 — Native macOS acceptance

- [ ] Run the final full-workbench [native checklist](native-editor-validation.md).
- [ ] Verify IME composition, candidate placement, cancellation, dead keys,
  clipboard ordering/CRLF, VoiceOver, bidi interactions and file lifecycle.
- [ ] Record macOS version, input method, build and pass/fail/not-tested results.
- [ ] Reproduce and fix failures, with regression tests where automation is valid.

Owner/dependency: the user can run these checks. Earlier prototype passes and
native-rendered screenshots do not validate this final renderer/controller.

### P1 — Windows live acceptance

- [ ] Obtain a Windows host and run equivalent native interaction checks.
- [ ] Verify Ctrl shortcuts, native menus/title bar, clipboard, IME, fonts,
  accessibility, resizing and file dialogs.
- [ ] Record actual runtime results separately from cross-compilation.

Status: compilation passes; live testing remains pending by user direction.
Do not describe Windows runtime as validated while this item remains open.

### P2 — Broader editing and Unicode coverage

- [ ] Extend shaped-workbench regressions beyond the current legacy comparator.
- [ ] Test bidi embeddings/isolates/control-heavy text, boundary-side transitions,
  selection collapse, pointer mapping, visual arrows and multiple cursors.
- [ ] Test native platform font coverage, ligatures, combining marks, emoji,
  fallback changes and empty/very long lines.
- [ ] Expand word/line navigation, undo grouping, focus/menu/popup interactions,
  find/replace, scroll restoration and file-error/cancellation coverage.
- [ ] Keep rejected partial-grapheme Find edits non-mutating with clear feedback.

Acceptance: reproducible fixtures and regression tests, plus native verification
where headless events cannot represent the actual platform behavior.

### P2 — Large-file editing and history efficiency

- [ ] Measure native release input-to-presentation latency, not just idle CPU frames.
- [ ] Reduce full-string rebuilding and source-map work on each edit.
- [ ] Evaluate transaction/delta history instead of up to 100 full snapshots.
- [ ] Measure memory/RSS, large edits, long Unicode lines and many cursors.
- [ ] Preserve atomic edits, original cursor/affinity sets, redo branches and saved
  checkpoints when changing history storage.

Existing work: immutable shape/character caches, indexed glyph lookup and scale
probes are implemented. See [the baseline](editor-performance-baseline.md).
Idle caching does not eliminate per-edit allocation or snapshot memory costs.

### P2 — Clipboard transport ownership

- [ ] Replace the temporary synchronous focused-editor clipboard reread with an
  owned, ordered native transport, if supported by the integration boundary.
- [ ] Preserve request IDs, sticky target invalidation, errors and event order.
- [ ] Evaluate explicit cross-process selection/line metadata; do not infer it
  from payload contents or silently trust stale session metadata.
- [ ] Verify native newline preservation and clipboard races on both platforms.

Current limitations: session-only metadata and synchronous reread latency/snapshot
race. Generic inputs must retain ordinary clipboard behavior.

### P3 — Legacy document adapter retirement

- [ ] Complete native acceptance and review fallback usage before removal.
- [ ] Remove document-level TextEdit adapters and legacy-only routing/tests in a
  separate, reviewed change; preserve shared typed command behavior.
- [ ] Retain ordinary TextEdit widgets for Find/Replace, names and other UI fields.
- [ ] Reverify file workflows, focus/scroll IDs, native menus and packaging.

Dependency: do not remove the fallback simply because the shared editor is default.

## Next feature — Settings

- [x] Save a settings implementation plan before changing application code.
- [x] Initial batch: schema-driven searchable UI, User/Project TOML persistence,
  precedence, validation/conflict feedback and six working appearance/workbench
  controls. Automated checks cover resolution, storage, controller errors,
  typography relayout and 300/900/1400px layout. Native macOS page rendering,
  search alias and draft Cancel were inspected; this is not IME/VoiceOver or
  Windows runtime acceptance.
- [ ] Implement user/project configuration and a consistent settings UI: user
  `~/.lumen/settings.toml` (home equivalent on Windows), project
  `<opened-directory>/.lumen/settings.toml`, with project values taking priority,
  in the order defined in [the settings plan](settings-feature-plan.md).

Settings work must not erase the unresolved editor activities above. Font/layout
settings add native geometry and IME acceptance cases to those checks.

## Tracking rules

Record evidence next to an item before checking it off. Distinguish implemented,
automatically verified, user-reported native pass and not tested. Keep known
limitations explicit. Commit/push only when requested.
