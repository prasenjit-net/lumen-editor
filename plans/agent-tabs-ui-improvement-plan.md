# Agent tabs UI improvement plan

Status: all fifteen items implemented (phase one: C1, C2, A2; phase two: C3, C4, C5,
A1, A3, A4, A5). Windows cross-compilation is blocked by the missing MinGW compiler. Branch: `feature/agent-tabs-ui-improvements`, created
from `origin/main` at `2537c44`.

## Scope and evidence

Improve the Codex and Claude tabs with five priorities for each. Both tabs use
shared panel, controls, and chat components, so applicable improvements should
benefit both providers without introducing separate layouts.

This plan comes from source review of `src/ui/agent_panel.rs`,
`src/ui/chat_controls.rs`, `src/ui/chat.rs`, and `src/ui/agent_install.rs`.
The initial review was not a live visual inspection or a list of reproduced
runtime defects. Implementation verification is recorded below.

## Codex tab: five improvements

### C1. Show activity on the provider tab

- [x] Extend the attention indicator with distinct working, approval-needed,
  and failed states, visible while the other provider is selected.
- [x] Define deterministic precedence when several sessions have different
  states, with a descriptive tooltip identifying the relevant session.
- [x] Use bundled icons and theme colors; do not rely on color alone.

Acceptance: background activity is discoverable without switching tabs, and
status changes do not shift the close button or obscure provider names.

### C2. Keep model selection readable at narrow widths

- [x] Give the model selector more space when the panel is narrow by moving
  effort into the configuration menu below a measured width threshold.
- [x] Keep the current effort discoverable and preserve its value when resizing.
- [x] Preserve stable toolbar action order and right-edge anchoring.

Acceptance: model selection, effort, configuration, History, and New chat remain
accessible at 240 px, 300 px, and 600 px panel widths without overlap.

### C3. Show the current session and workspace

- [x] Add a compact session title and workspace label above the conversation.
- [x] Truncate long names and expose full titles and paths in tooltips.
- [x] Keep the identity row synchronized with session and provider switching.

Acceptance: users can identify the active conversation and folder without
opening History; long labels preserve the compact layout.

### C4. Improve change previews

- [x] Present tool patches with file names and themed additions/deletions.
- [x] Add an Open diff action through a typed command and reuse an appropriate
  existing diff surface after checking its data requirements.
- [x] Retain readable plain output for tool results that are not valid patches.

Acceptance: users can inspect a change without editing the source file;
unavailable or malformed patch data produces useful feedback.

### C5. Make queued prompts easier to manage

- [x] Show the number of pending messages clearly beside the queue.
- [x] Allow editing and reordering pending prompts, with keyboard-accessible
  actions and explicit save/cancel behavior for edits.
- [x] Keep Stop visually separate from Queue and preserve the active prompt.

Acceptance: queue operations affect only pending messages in the intended
session. Cancellation preserves content; stale actions cannot alter a message
that has already started running.

## Claude tab: five improvements

### A1. Group consecutive completed tool activity

- [x] Collapse consecutive completed reads and searches into an expandable
  summary, such as “Read 6 files · 3 searches.”
- [x] Keep running, failed, and approval-related activity visible.
- [x] Preserve operation order, individual details, and stable expansion state.

Acceptance: long tool sequences are easier to scan, and every original result
remains accessible without hiding failures or changing transcript order.

### A2. Keep pending approvals easy to reach

- [x] Add a compact approval-needed strip above the composer.
- [x] Show the pending count and provide navigation to the corresponding request.
- [x] Keep exact request details and approval actions in the existing request UI.

Acceptance: approval requests remain discoverable while reading older messages;
resolved requests disappear from the strip and stale navigation is harmless.

### A3. Add conversation navigation

- [x] Show Jump to latest when the user scrolls away from the conversation end.
- [x] Indicate newly received content without forcing the viewport to move.
- [x] Add previous/next user-message navigation with accessible labels.

Acceptance: streaming respects manual reading position, navigation reaches the
intended message, and switching sessions preserves each session's scroll state.

### A4. Make History searchable

- [x] Add title search and date groups such as Today and Earlier.
- [x] Preserve workspace/provider filtering, selection, status, and rename actions.
- [x] Provide a clear empty-results state and a quick way to clear the query.

Acceptance: filtering and grouping retain stable session identity; keyboard
selection, rename cancellation, and returning to the current chat still work.

### A5. Add useful empty-state starting actions

- [x] Add compact suggestions such as Explain this folder and Review my changes.
- [x] Populate an editable draft when selected, without immediately sending it.
- [x] Preserve existing draft text and attachments; explain unavailable actions
  when their required context is missing.

Acceptance: suggestions are keyboard-accessible, fit the narrow panel, and never
overwrite a draft or trigger work before the user sends the message.

## Implementation order

1. C2, C1, A2: readable model controls, visible tab status, accessible approvals.
2. C3, A3, A4: session identity, transcript navigation, searchable history.
3. C4, C5, A1, A5: richer review, queue management, tool grouping, starting actions.

Implement each feature in shared components where applicable. The provider
headings express the original review's priorities, not exclusive capabilities.

## Architecture and visual constraints

- Reuse `theme.rs`, bundled icons, shared surfaces, and header primitives.
- Keep compact typography, subtle rounding, persistent resizing, and fixed-edge
  commands. Preserve the existing provider-tab header semantics.
- Keep UI rendering in `ui/`, session/view orchestration and typed commands in
  `app/`, and I/O behind service boundaries.
- Preserve stable session/widget IDs, drafts, attachments, focus, scroll state,
  configuration, approvals, and existing cancellation/error feedback.
- Check provider capabilities and existing command support before implementing
  actions; expose only behavior the application can actually perform.

## Verification for implementation

- Add regression coverage for changed layouts and workflows, including stale
  targets, session switching, errors, and cancellation.
- Visually inspect both tabs at 240 px, 300 px, and 600 px, including long titles,
  tool output, queued messages, pending approvals, and empty/error states.
- Check keyboard navigation, focus, hover/disabled tooltips, clipping, and theme
  contrast. Preserve unrelated user changes and unsaved buffers during checks.
- Run `cargo fmt --all --check`, `cargo test --workspace`, and
  `cargo clippy --workspace --all-targets -- -D warnings`.
- For platform-sensitive changes, run
  `cargo check --target x86_64-pc-windows-gnu` when the target is installed and
  verify macOS packaging with `cargo xtask bundle`. Distinguish compilation
  checks from native runtime verification in the handoff.


## First-phase implementation notes

- C1: shared tabs reserve a fixed status slot. Aggregation considers unarchived
  sessions across folders, prioritizing attention, failure, working, then waiting.
  Ties use recency and stable session identity; tooltips and accessible names
  identify the relevant session and folder.
- C2: effort choices are measured before deciding whether model and effort fit
  inline. The gear stays available, including when effort is the only secondary
  option. Resizing preserves values and right-anchored action order; disabled
  options retain their restrictions in the menu.
- A2: the composer has a pinned pending-approval count and Review action. Multiple
  requests have a bounded chooser. Navigation targets the exact request token
  for the current frame and never sends an approval decision.
- Regression coverage includes mixed background statuses, archived sessions,
  deterministic ties, accessible tab names, narrow layouts, configuration
  dispatch/disabled state, long-transcript navigation, multiple approvals,
  disappearing requests, cancellation, switching sessions, and keyboard activation
  of tab indicators and Review.
- Native macOS screenshots of an isolated fixture with fake backends were
  inspected at 240, 300, and 600 px. No real agent, user profile, or workspace
  files were loaded. Screenshots: `/tmp/lumen-agent-tabs-{240,300,600}.png`.


### Verification results — 2026-10-25

- Passed: `cargo fmt --all --check`.
- Passed: `cargo test --workspace`; the two subsequently added keyboard tests
  also passed in focused runs.
- Passed: `cargo clippy --workspace --all-targets -- -D warnings`, including
  the final keyboard tests.
- Passed: `cargo xtask bundle`, producing `target/Lumen.app` (debug).
- Inspected: native macOS fixture screenshots at all three panel widths.
- Blocked: `cargo check --target x86_64-pc-windows-gnu`. The Rust target is
  installed, but tree-sitter C dependencies cannot compile because
  `x86_64-w64-mingw32-gcc` is unavailable. No Windows runtime testing was performed.
- Provider interaction checks used fake backends; no live Codex/Claude requests
  were sent. C3–C5, A1, and A3–A5 remain pending.


## Second-phase implementation notes

- C3: `identity_row` shows the session title and folder above the chat; the folder
  keeps up to 40% of the width, the title truncates, tooltips give full title/path.
- C4: `model/patch_view.rs` parses unified diffs with or without file headers. Tool
  rows show file names, +/- counts and tinted lines; unparseable patches show a
  notice plus the raw text. Per-file **Open** and **Diff** (`ShowSourceDiff`, enabled
  only when Git reports an uncommitted change). No existing surface can display a
  bare patch, so no patch-only "Open diff" was invented.
- C5: queue header with pending count; per-message menu (Edit, Move up/down, Remove);
  inline edit with Save/Enter and Cancel/Esc. The hub refuses the active prompt and
  anything dispatched; reordering swaps contents and keeps ticket order, so the
  scheduler is unaffected. Stop is separated from Queue.
- A1: `model/tool_groups.rs` collapses 3+ consecutive completed reads/searches;
  running, failed, declined and other calls break a run and stay visible.
- A3: Prev/Next message and Jump-to-latest chips float over the transcript; "N new"
  counts items that arrived while reading above. Growth inside the last streaming
  item is not counted. Streaming never moves the viewport.
- A4: History title search (Esc clears it first), Pinned / Last 24 hours / Last 7
  days / Earlier groups (rolling windows), empty-result state with Clear search.
  The query clears on Back to chat.
- A5: starting prompts fill an empty draft only; disabled with a reason when a draft
  exists or, for review, when there is no Git repository.

### Verification results — phase two

- Passed: `cargo fmt --all --check`, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`.
- Inspected: native macOS fixture screenshots (fake backends) at 300 and 240 px
  showing identity row, patch, queue, Stop/Queue and navigation chips. 600 px, the
  History search view, and an expanded read group were covered by tests only.
- Not run: Windows check (no MinGW) and `cargo xtask bundle` for this phase.
