# Legacy and alternate-implementation removal plan

Reviewed: 2026-10-21 (main at `df68f13`). Static review only: nothing was
compiled or deleted. Line counts are approximate and dependency claims come
from grep. Before acting on any item, remove it on a branch and run
`cargo fmt --all --check`, `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings`.

## Status (2026-10-21, branch `c-remove-legacy-editor`, uncommitted)

A, B, C and D are done except where noted under each heading. E and F are untouched.

## A. Legacy editor (TextEdit) vs the shared editor

The shared editor has been the Cargo default since 2026-10-12.
`--no-default-features` and `cargo xtask bundle --legacy-editor` keep the old
path as a fallback. `plans/editor-promotion-checklist.md` lists "eventual legacy
adapter retirement" as pending. The old path only runs when building without
the default feature, so removal is mostly safe. The condition: accept the
pending native IME and screen-reader validation without a fallback.

| Item | Size | Notes |
|---|---|---|
| `src/ui/editor_viewport.rs` | 215 lines | TextEdit rendering path. `editor_viewport_themed` returns early to `editor_shared`; everything after is legacy. |
| `src/ui/editor_menu.rs` | 175 | Only used by `editor_viewport.rs`. |
| `src/ui/editor_find.rs` | 257 | Only used by `editor_viewport.rs`. |
| `src/ui/editor_gutter.rs` | 159 | Mostly legacy. `editor_shared.rs` uses only `gutter_id`; move it elsewhere. |
| `src/app/editor_clipboard.rs` | 173 | Only used by the legacy viewport, plus one field in `view_state.rs`. |
| `src/app/editor_input.rs` | 112 | Same. `cancel_input` is also called from `dispatch.rs`, `documents.rs`, `events.rs`; remove those calls. |
| `cfg(not(feature = "shared-editor"))` blocks | ~10 sites | Hover popup in `ui/editor.rs`; `chrome.rs`; `agent_context.rs`; `language_edits.rs` (stub refusing server edits); `language_services.rs` (2); `ui/source_control.rs` (3). |
| `shared-editor` cfg gates | ~110 sites, ~20 files | Make unconditional. Drop the `shared-editor` feature and the `ViewState.shared_editor` flag. |
| `Document.text` / `record_text_change` | small | "TextEdit remains the authority during migration" shim. Review the `editing: Option<EditorBuffer>` indirection and `enable_shared_editing`. |
| Legacy tests | ~2,000+ lines | Parts of `src/app/tests/{viewport,editor,clipboard,paste,find}.rs` and `tests.rs`. `test_app` forces `shared_editor = false`, so these test the legacy path. `tests/shared.rs` is the replacement suite. |
| `xtask` `BundleEditor::Legacy` | ~40 lines + test | `--legacy-editor`, `--shared-editor`, "Lumen Legacy.app", "Lumen Experimental.app", `editor_feature_args`. |
| CI "Test legacy editor" job | 3 lines | `.github/workflows/ci.yml`. |
| README | ~10 lines | "Legacy editor and standalone fixture" section and the table row describing the legacy backend. |

Estimated total: ~1,100 lines of source, ~2,000 lines of tests, plus cfg plumbing.
The test count is the least certain figure.

Done. Also removed `text_line.rs`, `FindState` legacy helpers (`prepare_find`,
`finish_replace`), `pending_text_input`, `text_clipboard`, the `shared_editor` flag and
the xtask `--shared-editor`/`--legacy-editor` bundles (and the CI and release
workflow legacy test steps). Kept: `Document.editing: Option<EditorBuffer>` (documents
upgrade lazily; collapsing it is a separate refactor). 17 legacy-widget tests were
deleted and the paste/find/editor tests were ported to the shared buffer.
Behavior gap found: a test that typed right after a tab click only passed on the
TextEdit path. In the shared editor, typing after a tab click needs
`focus_editor`. Not changed here; worth checking by hand.

## B. `shaped` feature in `crates/editor-surface`

`shaped` is always enabled through `shared-editor`. It has 17 gates in
`geometry.rs`, 12 in `lib.rs`, and gates in `accessibility.rs` and tests. The
`cfg(not(feature = "shaped"))` branches (`geometry.rs:410`, `:553`,
`lib.rs:326`, the examples) are an unshaped fallback nothing builds. Make
`shaped` unconditional, delete the `not(shaped)` branches, and make
`cosmic-text` a plain dependency.

Done. `cosmic-text` is a plain dependency and the `shaped`, `shaping-probe` and
`cosmic-probe` features are gone. The `Option<Shaper>` runtime fallback inside the
surface (`EditorSurface::default()`) remains.

## C. Spike and probe code

- `crates/editor-surface/src/cosmic_probe.rs` (102) and `shaping_probe.rs` (74),
  behind `cosmic-probe` / `shaping-probe`. Test-only backend-selection
  experiments; the decision is recorded in `plans/editor-backend-decision.md`.
  Delete along with the `egui-shaping` dependency pinned at `=0.36.2`.
- `crates/editor-surface/examples/spike.rs` (150): explicitly a spike. Delete.
  `profile.rs` (119) is a profiling tool; keep unless unused.
- `plans/`: 60 files, 12k lines. `plans/acp-spike` (560K) and
  `plans/native-agent-spike` (292K) are spike artefacts. Archive or delete. The
  stale plans (promotion checklist, gap review) are a judgment call.

Done: probes, `egui-shaping` and `examples/spike.rs` deleted, README updated.
Kept: `examples/profile.rs`, and `plans/acp-spike` plus `plans/native-agent-spike`.
Those two hold agent-protocol findings and the origin of the protocol fixtures
(`crates/agent-protocol/tests/fixtures.rs` references `plans/acp-spike`), so they
belong with D, not C.

## D. Old ACP agent path vs the native agent path

The largest alternate implementation, but not safe to delete today. README
documents `agent.native.enabled` and `LUMEN_NATIVE_AGENTS=0` as the way back to
the ACP panel, and `plans/native-agent-implementation-gap-review.md` says the
native path is incomplete (hard-coded `codex`/`claude` names, ignores the
agent-path settings that only ACP honours).

| Item | Size |
|---|---|
| `src/app/agent.rs` (`AgentRuntime`) | 2,509 |
| `src/services/agent_process.rs` | 738 |
| `crates/agent-protocol` | 2,661 |
| `src/ui/agent_panel.rs` | 2,518 (mostly ACP; check what chat/session code shares) |
| ACP-specific parts of `agent_context.rs`, `agent_history.rs`, `agent_permission.rs` | partial; shared in places |
| Legacy adapter entries in `crates/settings-core/src/agents.rs` | small |
| `NewSession.modes` / `current_mode` (`agent-protocol/types.rs:181`) | small |

Roughly 8,000 to 10,000 lines. Removable only after retiring
`agent.native.enabled` and `LUMEN_NATIVE_AGENTS`, and after the native path
reaches parity. This is a product decision.

Done on the same branch, except the `crates/agent-protocol` directory, which is
still on disk (see below). Removed: `app/agent.rs`, `agent_changes`, `agent_context`,
`agent_history`, `agent_permission`, `services/agent_process.rs` (its PATH helpers
moved to `services/login_path.rs`), `ui/agent_panel.rs` (the markdown reply renderer
moved to `ui/agent_message.rs`), `model/line_diff.rs`, `services/workspace_snapshot.rs`,
`ui/transcript_layout.rs`, 21 ACP-only `Command` variants, and the settings
`agent.native.enabled`, `agent.codex.initial_mode`, `agent.unsaved_files`,
`agent.follow_locations`, `agent.track_changes`, `agent.allow_read_in_workspace`,
`agent.claude.{command,launch}` and `agent.codex.{command,launch}`.
Native agents are now always on (`LUMEN_NATIVE_AGENTS` is gone). Features that only
existed on the ACP panel are gone: attachments, "Ask Agent About Selection", "Add to
Agent Chat", "Fix with agent" in Problems, change review/revert, the approval badge
on the activity bar. Old keys in a user's settings file are kept and reported as
unknown settings.
Remaining: delete `crates/agent-protocol` (nothing depends on it now), remove it
from the workspace members in `Cargo.toml`, then refresh `Cargo.lock`.

## E. Smaller compatibility code

- Recovery legacy import: `src/services/recovery.rs:258-265` plus the test
  `corrupt_snapshot_is_retained_and_legacy_file_migrates`. Migrates the old
  single `recovery.bin`. Safe to drop if no one still has that file (unverifiable).
- `SyntaxClass::Legacy` and `SyntaxSpan::legacy`
  (`crates/editor-surface/src/geometry.rs:32-86`): a second highlight path from
  before `language-services`. Check whether any caller still produces
  `syntax::Span` input.
- `language-server` Cargo feature: off by default, so no server ever starts.
  About 14 gated sites in `language_services.rs` and `language_server.rs`, plus a
  CI step. Not dead code; decide to enable it or remove it.

## F. `#![allow(dead_code)]` blanket allows (22)

Several carry stale comments such as "Wired to the task runner in the next step":
`src/model/task_output.rs`, `src/services/{task_process,tasks,task_trust,process_tree}.rs`,
`src/app/{task_editor,chat_tabs,agent_hub}.rs`,
`src/services/{agent_sessions_store,agent_transcripts_store}.rs`, and others.
Remove each allow and let the compiler list what is unused; those items are the
provably safe deletions. Keep the 7 platform-conditional allows in
`src/app/command.rs`.

## Suggested order

1. A, B, C: low risk, ~3,000 lines including tests.
2. F: the compiler identifies what is dead.
3. E (recovery import, `Legacy` highlight class).
4. D: only after the decision to retire the ACP panel.

Open decisions: accept shipping without the legacy editor fallback (A); retire
the ACP panel (D); enable or remove the `language-server` feature (E).
