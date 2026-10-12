# VS Code-inspired core feature roadmap

Updated: 2026-10-12. This is Lumen's recommended implementation order, not an
official VS Code ranking. Preserve existing editor, Explorer, settings, native
menu and file workflows while adding each feature. The broader native/editor
acceptance backlog remains in [pending-activities.md](pending-activities.md).

## 1. Protect unsaved work — initial implementation done; hardening pending

Implemented: periodic recovery snapshots of dirty buffers, restore on startup,
opt-in Auto Save (`off`, `afterDelay`, `onFocusChange`, `onWindowChange`), disk-change
warnings, guarded Save, explicit conflict Overwrite/Reload choices, and staged
file writes. Explicit Save remains the default.

- [ ] Replace the single shared recovery snapshot with per-instance ownership.
  Concurrent Lumen processes must not overwrite or clear each other's backups;
  migrate the existing snapshot format without losing recoverable text.
- [ ] Add startup handling for divergent/corrupt/orphaned backups. Keep uncertain
  data available rather than silently deleting it; test forced termination and
  restart with both disk-backed and untitled buffers.
- [ ] Add a side-by-side disk-versus-editor comparison, then a deliberate merge
  path or an explicit limitation. Detect changes across all open tabs, including
  deletion/rename, with a persistent indicator until resolved.
- [ ] Validate Auto Save's focus/window policies and recovery lifecycle in live
  macOS and Windows builds. Test IME, dialogs, Save All, Save As, failed writes,
  large files, symlinks/permissions and two simultaneous processes.

Exit gate: no ordinary Save/Auto Save overwrites external edits; each interrupted
instance can recover its own latest durable snapshot; native results are
recorded separately from compilation. Until then, retain the single-instance
warning and note that backups contain unencrypted document text.

## 2. Search and replace across the open folder

- [ ] Build a Search activity panel with query, file glob and exclude controls,
  result counts, file grouping, snippets and keyboard navigation.
- [ ] Respect `.gitignore`/configured exclusions, skip binary or oversized
  files with clear feedback, and keep searches responsive/cancellable.
- [ ] Add workspace replacement with a preview, per-file confirmation choices,
  conflict detection and a recoverable failure report. Never silently overwrite
  dirty open buffers or externally changed files.

Exit gate: Unicode and CRLF matches, ignored files, cancellation, permission
errors, dirty tabs and replace failures have regression coverage.

## 3. Open and navigate files quickly

- [ ] Add fuzzy Quick Open by filename/path, recently opened files and a
  workspace-aware ranking; keep the Explorer and native Open dialog intact.
- [ ] Add Go to Line/Column, sharing the command palette/shortcut routing and
  stable document IDs.

Exit gate: large workspaces remain responsive; duplicate tabs, missing files,
focus return and cancelled navigation behave consistently.

## 4. Complete everyday code-editing behavior

- [ ] Add auto-indent and configurable indentation detection/size.
- [ ] Add bracket/quote completion and matching with multi-cursor, selection,
  IME and undo-safe behavior.
- [ ] Add code folding with gutter affordances and persistent per-tab fold state.

Exit gate: all editing operations remain one coherent history owner, preserve
grapheme boundaries and keep gutter/text hit testing aligned.

## 5. Extend document Find/Replace

- [ ] Add regular expressions, capture-group replacement, multiline queries,
  Find in Selection and preserve-case options in measured increments.
- [ ] Keep invalid expressions, zero-width matches, CRLF, Unicode boundaries and
  partial-grapheme replacements safe and non-mutating, with clear feedback.

Exit gate: single and Replace All operations are undoable and respect the
accepted grapheme-safe rejection rule. See the existing
[editor plan](text-editor-improvement-plan.md) for detailed cases.

## 6. Add language services in stages

- [ ] Define a language-server boundary and lifecycle (open/change/save/close,
  process failures, cancellation and document revision checks).
- [ ] First show diagnostics in the editor and a Problems panel; then add
  completion/parameter help, hover, definition/references, rename and format.
- [ ] Keep lexical highlighting useful without a server and avoid applying
  stale diagnostics or edits to a changed document.

Exit gate: at least one language has end-to-end native workflows, with server
absence/failure and stale-response regression tests.

## 7. Add Git source control and diff views

- [ ] Start with repository discovery, status badges and working-tree diffs.
- [ ] Then add staging/unstaging and commit, with explicit error feedback and
  no implicit modification of unrelated worktree files.
- [ ] Reuse the comparison component from item 1 where practical.

Exit gate: a dirty editor buffer, external Git changes and repository errors
cannot silently replace user content.

## 8. Add an integrated terminal, then split editor groups

- [ ] Add a workspace-rooted terminal with platform-native shell selection,
  process lifecycle, keyboard focus, output limits and safe close behavior.
- [ ] Add split editor groups only after document identity, focus, scroll,
  recovery and Save/Close semantics work across multiple views of a file.

Exit gate: terminal shutdown cannot hang app exit; split panes resize and retain
independent view state without duplicating document edits or recovery snapshots.

## Sources and sequencing

The feature inventory follows VS Code's official documentation for
[basic editing, Auto Save, search and folding](https://code.visualstudio.com/docs/editing/codebasics),
[navigation](https://code.visualstudio.com/docs/editing/editingevolved),
[IntelliSense](https://code.visualstudio.com/docs/editing/intellisense),
[source control](https://code.visualstudio.com/docs/sourcecontrol/overview),
[terminal](https://code.visualstudio.com/docs/terminal/getting-started), and
[workbench layout](https://code.visualstudio.com/docs/editing/getting-started/userinterface).
The order above reflects Lumen's data-safety and dependency priorities. Complete
one exit gate before claiming that feature is finished; native acceptance must
be recorded honestly when only cross-compilation is available.
