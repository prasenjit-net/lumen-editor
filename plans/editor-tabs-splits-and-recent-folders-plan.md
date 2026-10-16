# Plan: tab commands, tab reordering, split editors and recent folders

Written: 2026-10-16. Follows `plans/ui-enhancement-review.md` (items 7 and 15), which
deliberately left these as separate projects. This is a plan only; no behavior changes.

## Where the code is today (confirmed in source)

- **Tab model.** `model/editor.rs::EditorState` owns `files: Vec<Document>` and
  `active_file: Option<usize>`. A tab's position *is* its index in `files`.
  `active_file` is referenced in about 79 places across roughly 30 files.
- **Virtual tabs.** The diff preview and the Settings page are drawn after the document
  tabs in `ui/editor_tabs.rs` but are not entries in `files`. A diff-only document is
  hidden from the strip (`SourceDiffView::diff_only_document`).
- **Tab menu (done).** Close, Close Others, Close to the Right, Close All, Copy Path,
  Copy Relative Path, Reveal; middle-click closes. Bulk closing lives in
  `app/tab_close.rs` and targets stable `DocumentId`s: clean tabs first, then each dirty
  tab through the existing unsaved-changes dialog; Cancel or an incomplete Save abandons
  the rest and the active tab is preserved.
- **Index-based identifiers that matter here.** `view.pending_close: Option<usize>`,
  `Command::RequestClose(usize)`, `Command::CloseFile(usize)`, tab selection by index, and
  `EditorState::close_file` (which re-points `active_file` at the same *index*, so closing
  a non-active tab can change the active tab - an existing quirk the bulk-close path works
  around).
- **Identity already keyed by document.** `text_widget_id(document)` is
  `("editor_text", DocumentId)`; Find state is `view.find: HashMap<DocumentId, FindState>`;
  language-server sync, diagnostics and semantic tokens are per `DocumentId`; popup editor
  commands carry a `DocumentId`.
- **Things that are single-instance today.** `view.shared_surface` (the one live editor
  surface used by completion/hover/signature popups and agent selection context), the
  editor's `focus_editor` flag, and the shared buffer's selections, which live on the
  `Document` (`editing.selections()`), not on a view.
- **Reusable pieces.** `app/scroll_sync.rs` (synchronised scrolling for diff panes),
  `ui/popup_place.rs`, `widgets::panel_header`, `surface()`/`canvas()`.
- **Recent items.** `services/recent_files.rs` persists one array, `paths`, to
  `~/.lumen/recent-files.toml` (user-scoped, 64 KiB cap, symlinks rejected, atomic write).
  `app/navigation.rs` keeps up to 30 recent files. The welcome view lists five recent files.
  Folders are not remembered; `app/documents.rs::choose_folder` only handles the native
  dialog path and resets a long list of workspace-scoped state.

## Guardrails (from `AGENTS.md`)

Menus, shortcuts, palette and buttons share one typed `Command`. Native macOS/Windows menus
keep their own menu IDs and must stay in step. Buffer-only commands apply locally; I/O
dispatches through the controller. Editor popup commands carry a stable document ID and
are rejected when stale or while a dialog blocks input. One `TextEdit`/surface render and
one mutation/history owner per pass. Never discard dirty buffers silently; cancellation
must stop the whole operation. `model/` stays free of egui and I/O. User-scope-only
storage for history.

## Phase 0 - Prerequisite: stable document identity everywhere (M, do first)

Reordering and splitting both break any code that treats a tab index as identity.

1. Replace `pending_close: Option<usize>` with `Option<DocumentId>`; add
   `Command::RequestCloseDocument(DocumentId)` and `CloseDocument(DocumentId)`; keep the
   index variants as thin wrappers until nothing calls them, then delete them.
2. Add `EditorState::index_of(DocumentId)` and `activate(DocumentId)`; make
   `close_file` preserve the active *document* (fixes the non-active-tab quirk; the
   workaround in `tab_close.rs` can then be removed).
3. Audit the 79 `active_file` uses: those that mean "the document the user is looking at"
   move behind `EditorState::active()` / `active_id()` so Phase 3 can redefine them.

**Acceptance:** all existing tests pass unchanged in behavior; new tests prove closing a
non-active tab keeps the active tab, and a pending close survives a reorder.

**Status: done (branch `f-tab-identity`).**
- `pending_close` is an `Option<DocumentId>`; `RequestClose`, `CloseFile` and `SaveAndClose`
  carry a `DocumentId`; the unsaved-changes dialog looks the document up by ID and drops the
  prompt if it disappeared. `tab_close.rs` and the tab menu use IDs too.
- `EditorState` gained `active_index()`, `active_id()`, `index_of()`, `activate()`,
  `set_active_index()` and `close_document()`; `active_file` is now private and every caller
  (about 80 sites, including tests) goes through the accessors, so Phase 3 can redefine
  "active" in one place. `close_file` keeps the same *document* active when another tab
  closes, so the workaround in `tab_close.rs` was removed.
- **Behavior fix found on the way:** "Save" in the unsaved-changes prompt used to save the
  *active* buffer even when the prompt was about a different tab (closing a non-active dirty
  tab with its X). It now activates the prompted document and saves that one; if the save is
  cancelled or fails the document stays open and active. Regression tests cover it.
- Not changed: tab selection and strip drawing still work with visible-file indexes inside a
  single frame (they are recomputed every frame, so they cannot go stale across frames).

## Phase 1 - Complete the tab commands (S-M)

Missing from today's menu and keyboard story:

- **Close to the Left** and **Close Saved** (all clean tabs), reusing `TabCloseScope`
  (`ToTheLeft`, `Saved`) and the same dirty-prompt/cancel rules. Group/section wording:
  Close, Close Others, Close to the Left, Close to the Right, Close Saved, Close All.
- **Reopen Closed Tab** (Cmd/Ctrl+Shift+T): a bounded stack (20) of closed *saved-path*
  documents (path + caret byte). Untitled or discarded buffers are not resurrectable;
  say so in the tooltip. Reopening a deleted path reports a recoverable error.
- **Next/Previous Tab** (Ctrl+Tab/Ctrl+Shift+Tab, plus Cmd+Alt+Right/Left on macOS) in
  strip order, wrapping, skipping nothing. Add to palette and the native View menu.
- Palette entries for every tab command with the existing availability reasons
  (`palette_disabled_reason`).
- Keyboard-invoked tab menu (menu key / Shift+F10 on the focused tab) - egui tab focus
  must first be made visible (reuse the focus ring from item 5 of the UI review).

**Tests:** scope selection for left/right/saved with dirty tabs in the middle; Cancel
abandons the remainder; reopen stack cap and missing-path error; shortcut parity between
menu, palette and key event.

## Phase 2 - Tab reordering (M)

Order is the position in `files`, so reordering is moving an element; no new ordering
state. Persisting order across restarts is out of scope (there is no session restore for
saved documents today; recovery only keeps dirty buffers).

1. **Model (`model/`, no egui):** `EditorState::move_document(id, to_index)` clamps the
   target, keeps the active document by ID, and never touches content or dirty flags.
   Virtual tabs (diff, Settings) stay after the document tabs and are not draggable;
   a document is never moved past them.
2. **Menu/keyboard first:** "Move Left" / "Move Right" in the tab menu and as commands
   (Cmd/Ctrl+Alt+Shift+Left/Right, final chord to be checked against existing shortcuts).
   This is the accessible path and ships before drag.
3. **Drag:** give the tab title `Sense::click_and_drag`; start after a ~4 px threshold so a
   click still selects. Draw a 2 px accent insertion marker between tabs; auto-scroll the
   strip near its edges using the existing scroll viewport; Escape or dropping outside the
   strip cancels with no change; drop commits one `MoveDocument` command. The tab keeps its
   widget ID (derive from `DocumentId`, not the index) so scroll position and focus survive.
4. **Overflow:** while dragging in an overflowed strip the hidden-tabs menu is not
   offered; the drag still reveals the destination by auto-scroll.
5. **Optional, separate PR - pinned tabs:** a `pinned: bool` on the view side (not in
   `Document`); pinned tabs sort to the front, render compact with no close button, are
   skipped by Close Others/All/Saved unless explicitly requested, and drag cannot cross
   the pinned boundary. Pin/Unpin in the tab menu. Preview (italic, replaceable) tabs are
   a different feature and stay out of scope.

**Tests:** model move preserves active/dirty/content; drag below threshold is a click;
Escape/outside drop is a no-op; insertion marker at first/middle/last; narrow strip with
overflow; reorder while a close prompt is pending keeps the prompt on the same document.
**Visual:** drag marker in light/dark/high-contrast via the screenshot technique below.

## Phase 3 - Split editor (L, staged; the largest piece)

### Decision: editor groups over a shared document store

Introduce `EditorLayout` in `model/`: an ordered list of up to **two groups** (v1), each
with `tabs: Vec<DocumentId>` and `active: Option<DocumentId>`, a `focused` group, and an
orientation (horizontal first, vertical later). `EditorState::files` becomes the document
*store*; a document appears in exactly one group in v1. `EditorState::active()` returns the
focused group's active document so the large number of existing call sites keep working
(this is what Phase 0 item 3 prepares). Tabs, Close Others/Right/Left, Move and Reorder all
become per-group operations over a group's `tabs`.

**v1 rule - one view per document.** Opening a document that is open in the other group
reveals and focuses it there instead of duplicating it. Reason: caret/selection live on the
shared `Document` buffer, `text_widget_id` and the single `shared_surface` assume one
visible surface per document, and AGENTS.md requires one mutation/history owner per pass.
Showing one document in both panes is a **v2** goal that needs per-view selection and
scroll state.

### Stages

- **S1 - Two groups side by side.** Split Right (Cmd/Ctrl+\\) moves the active tab into a
  new right group (or clones layout only when a second tab exists; splitting a lone tab is
  disabled with a reason). Each group has its own tab strip, breadcrumb bar and editor
  surface. A divider uses the existing resizable-pane pattern: drag handle, 200 px minimum
  per group, widths remembered for the session. A group with no tabs disappears and focus
  returns to the other group. Settings and diff virtual tabs live in the focused group's
  strip when opened; closing them follows today's rules.
- **S2 - Cross-group commands.** "Split Right", "Move to Other Group" and "Focus Next
  Group" (Cmd/Ctrl+K then arrows is **not** assumed: chord support must be verified; use
  Cmd/Ctrl+1/2 for group focus instead). Palette and tab-menu entries. Dragging a tab onto
  the other group's strip reuses the Phase 2 drag with a cross-group drop target.
- **S3 - Vertical split** (stacked groups) by flipping orientation; same minimums.
- **S4 - Same document in both panes (v2).** Per-view selection, scroll and fold state
  stored by `(GroupId, DocumentId)`; widget IDs become `(group, document)`; all edits still
  go through the one buffer; Find highlights render in both. Only start after S1-S3 are
  stable.

### Things that must be reworked (and tested) for S1

- **Single-instance view state:** `shared_surface` becomes per focused group;
  completion, hover and signature popups, the agent panel's selection/mention context and
  the status bar read the focused group only. `focus_editor` becomes "focus this group".
- **Input routing:** keyboard and clipboard events go to the focused group's editor only;
  clicking a group focuses it before any editor input is processed in that frame; blocking
  dialogs and popups keep their current stale-target rejection by `DocumentId`.
- **Scroll sync:** off by default between normal groups; the diff view's sync is unaffected.
- **Close/quit prompts:** unchanged, but "Close All" and friends operate on the focused
  group (matching the menu wording) with an explicit "Close All in All Groups".
- **Explorer and Quick Open:** opening a file targets the focused group.
- **Persistence:** layout is session-only in v1; recovery of dirty buffers is unchanged.

### Risks

Highest risk is breadth: the 79 `active_file` call sites and every place that assumes one
live surface. Mitigation: the Phase 0 facade, landing S1 behind a feature flag/setting
(`workbench.split_editor`, default off) until the checklist below passes, and keeping the
single-group path byte-for-byte equivalent in layout tests.

**Tests:** model tests for group create/collapse/move/focus and one-view-per-document;
layout tests at narrow/default/wide for one and two groups (tabs, breadcrumbs, editor and
divider keep their minimums, no overlap); focus-routing tests (typing, paste and popup
commands only reach the focused group); closing the last tab of a group collapses it
without touching the other group's dirty buffers; every bulk close with dirty tabs in
both groups.

## Phase 4 - Recent folders (S-M; independent, can ship before Phase 2)

1. **Storage.** Evolve `services/recent_files.rs` from `Vec<PathBuf>` to a small
   `RecentHistory { files, folders }`. Keep the same file, atomic write, 64 KiB cap, symlink
   rejection and user-scope rule. Load tolerates a missing `folders` key (old files keep
   working); save preserves unknown keys via `toml_edit`. Cap folders at 15,
   de-duplicate, move-to-front, absolute paths only. Update the `RecentFilesStore` trait and
   its test doubles together.
2. **Recording.** Extract `open_folder_path(PathBuf)` from `choose_folder` (the dialog just
   supplies the path) so the dialog, welcome list, palette and menu share one code path.
   Record on success only. Validate before opening: must exist and be a directory,
   otherwise report a recoverable error and prune that entry; never create buffers or
   change the current workspace on failure. Keep the existing reset of workspace-scoped
   state (search, source control, expanded dirs, file index).
3. **Surfaces.**
   - Welcome view: a "Recent folders" section above recent files (same compact style,
     parent path as secondary text, hover for the full path).
   - Command palette: "File: Open Recent Folder..." using a navigation-palette mode with
     filtering, plus "File: Clear Recently Opened".
   - Native menus: an "Open Recent" submenu on macOS/Windows (folders then files) rebuilt
     when history changes; menu IDs added alongside the existing ones. Non-native
     platforms use the palette entry only.
4. **Optional setting:** "Reopen last folder on startup" (default off), applied only when
   the folder still exists and no file was passed on the command line.

**Tests:** round trip with old-format and new-format files; cap, dedupe, order; missing
folder prunes and leaves workspace/buffers untouched; opening a recent folder resets
workspace-scoped state exactly like the dialog path; symlinked history file rejected;
welcome list and palette entry visible with/without history. No test may touch the real
`~/.lumen`.

## Suggested order

1. Phase 0 (identity) - unblocks everything and fixes the close-active quirk.
2. Phase 4 (recent folders) - small, self-contained, high daily value.
3. Phase 1 (tab commands) - mostly reuses `tab_close.rs`.
4. Phase 2 (reordering: menu/keyboard, then drag, then optional pinning).
5. Phase 3 S1 behind a setting, then S2, S3; S4 only after real use.

Rough size: P0 M, P4 S-M, P1 S-M, P2 M (+S for pinning), P3 S1 L, S2 M, S3 S, S4 L.

## Open questions (recommended answers)

- Allow the same document in two panes in v1? **No** (one view per document); revisit in S4.
- More than two groups? **No** for now; the layout type should not forbid it.
- Persist tab order or split layout across restarts? **No** for now; there is no saved-session
  restore, and adding one is its own project.
- Chord shortcuts (Cmd/Ctrl+K, ...)? Verify the input layer supports chords before relying
  on them; otherwise use single-chord equivalents.
- Pinned tabs in this plan? **Optional**, as a separate PR after drag reorder.

## Verification (applies to every phase)

- `cargo fmt --all --check`, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --no-default-features`;
  Windows target check when installed. Add regression tests for success, cancel and error
  paths; test narrow, default and wide widths for any layout change.
- **Screenshots.** `screencapture` is blocked in the agent environment, but the app can
  capture itself: send `egui::ViewportCommand::Screenshot(egui::UserData::new(name))` from
  a temporary, uncommitted harness hooked into `LumenApp::update`, read the resulting
  `Event::Screenshot` image and write a PPM (convert with `sips -s format png`). Drive
  state through typed `Command`s at fixed frame numbers; use the light, dark and
  high-contrast themes; do not commit the harness. Such a harness opens real files and
  can add them to `~/.lumen/recent-files.toml` and leave dirty buffers in
  `~/.lumen/recovery` - clean those up afterwards, or point it at a scratch home.
- Preserve unsaved buffers and unrelated changes while testing; commit/push only when asked.
