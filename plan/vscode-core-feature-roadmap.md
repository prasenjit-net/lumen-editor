# VS Code-inspired core feature roadmap

Updated: 2026-10-13. This is Lumen's recommended implementation order, not an
official VS Code ranking. Preserve existing editor, Explorer, settings, native
menu and file workflows while adding each feature. The broader native/editor
acceptance backlog remains in [pending-activities.md](pending-activities.md).

## 1. Protect unsaved work — implemented; native acceptance pending

Implemented: periodic per-instance recovery snapshots of dirty buffers, restore on startup,
opt-in Auto Save (`off`, `afterDelay`, `onFocusChange`, `onWindowChange`), disk-change
warnings, guarded Save, explicit conflict Overwrite/Reload choices, and staged
file writes. Explicit Save remains the default. Recovery uses OS file locks to
skip live sessions, imports the previous single-file format, preserves corrupt
snapshots for inspection, and keeps divergent recovered copies separate. All
open disk-backed tabs show external-change markers and can open a side-by-side
comparison; explicit Overwrite rechecks the newest disk version. Save As keeps
both versions. No automatic merge or hunk-level diff is provided.

- [x] Replace the single shared recovery snapshot with per-instance ownership.
  Concurrent Lumen processes must not overwrite or clear each other's backups;
  migrate the existing snapshot format without losing recoverable text.
- [x] Add startup handling for divergent/corrupt/orphaned backups. Keep uncertain
  data available rather than silently deleting it; test forced termination and
  restart with both disk-backed and untitled buffers.
- [x] Add a side-by-side disk-versus-editor comparison, then a deliberate merge
  path or an explicit limitation. Detect changes across all open tabs, including
  deletion/rename, with a persistent indicator until resolved.
- [ ] Validate Auto Save's focus/window policies and recovery lifecycle in live
  macOS and Windows builds. Test IME, dialogs, Save All, Save As, failed writes,
  large files, symlinks/permissions and two simultaneous processes.

Implementation gate: ordinary Save/Auto Save block on external edits; each
interrupted instance can recover its own latest durable snapshot in automated
tests. Native results must be recorded separately from compilation. Backups
contain unencrypted document text. The remaining unchecked item is a native
acceptance gate, not an invitation to claim Windows runtime validation.

## 2. Search and replace across the open folder — in progress

Implemented so far: a Search activity panel with literal query, Match Case/Whole
Word, include/exclude glob fields, grouped file/line results and keyboard-openable
match rows. Cmd/Ctrl+Shift+F opens the panel. A cancellable background scan
skips symlink traversal, `.git`/`target`, binary and files above 2 MiB, and
retains bounded snapshots. Nested `.gitignore` files use the `ignore` crate's
Git-style pattern grammar; global excludes and `.git/info/exclude` are not read.
The result tooltip reports skip categories, counting ignored directories as
entries rather than every descendant. Replacement
has all-results, folder and file actions, a capped before/after preview, separate confirmation,
grapheme checks, guarded writes and a per-file failure report. Open editor tabs
are conservatively skipped. Search snapshots are limited to 10,000 matches and
64 MiB of matching text; a limit warning asks the user to narrow the query.

Still needed before the full exit gate: live native testing at
narrow/default/wide widths and with a large workspace. The replacement preview
now groups nearby edits into color-coded diff hunks and paginates every group.
Very long groups fall back to per-match excerpts, rather than a combined
final-file diff. No operation-wide rollback or undo is provided.

- [x] Build a Search activity panel with query, file glob and exclude controls,
  result counts, file grouping, snippets and keyboard navigation.
- [x] Virtualize result rows so every bounded hit remains reachable without
  painting the entire tree each frame.
- [x] Stream throttled scan progress to the Search panel without blocking the UI
  or queuing unbounded updates; cancellation discards stale progress.
- [x] Apply schema-driven `search.exclude` settings with project-over-user
  priority alongside temporary panel exclusions.
- [x] Respect `.gitignore`/configured exclusions, skip binary or oversized
  files with clear feedback, and keep searches responsive/cancellable.
- [x] Add workspace replacement with a preview and scoped folder/file actions,
  conflict detection and a recoverable failure report. Never silently overwrite
  dirty open buffers or externally changed files.

Exit gate: Unicode and CRLF matches, ignored files, cancellation, permission
errors, dirty tabs and replace failures have regression coverage.

## 3. Open and navigate files quickly — implemented; native acceptance pending

Quick Open now uses a cancellable background workspace index, filename/path fuzzy
ranking and a bounded user-level recent-files history that survives restart.
Matching workspace files rank ahead of external recents. Cmd/Ctrl+P opens it; Cmd/Ctrl+Shift+P
continues to open the command palette. Go to Line/Column is available through
Cmd/Ctrl+G and the native View menu; it moves the caret by stable document ID
without editing text. Both commands are also in the command palette. The index
respects nested `.gitignore`, excludes generated `.git`/`target` trees, and is
capped at 100,000 files. Missing recent targets are pruned when opened. Explorer
create/rename/delete updates the index, but external changes are not watched;
reopen the folder to rebuild the index.
Native interaction and large-workspace acceptance remain open.

- [x] Finish fuzzy Quick Open by filename/path, persistent recently opened files and a
  workspace-aware ranking; keep the Explorer and native Open dialog intact.
- [x] Add Go to Line/Column, sharing the command palette/shortcut routing and
  stable document IDs.

Exit gate: large workspaces remain responsive; duplicate tabs, missing files,
focus return and cancelled navigation behave consistently.

## 4. Complete everyday code-editing behavior

In progress: the default shared editor now has schema-driven Auto Indent,
Detect Indentation, Indent Size and Insert Spaces settings with user/project
precedence. Enter and Tab use one editor-core transaction for multi-cursor
edits, including CRLF. Detection samples up to 200 lines. The optional legacy
TextEdit fallback still uses egui's built-in indentation rules. Native
interaction acceptance and the remaining editing behaviors are open.
Bracket/quote completion wraps selections and inserts paired delimiters as one
core undo step, while IME commits stay literal. Bracket matching uses syntax
spans to skip recognized comments and strings and paints from the shared text
geometry. Native interaction acceptance remains pending.
Code folding now covers bracket-delimited code and indentation-based
Python/YAML blocks in the default shared editor. Gutter chevrons collapse rows
through one source/display projection for text, accessibility, caret movement,
hit testing and original line numbers. Collapse state is per tab and follows
unchanged fold headers when lines are inserted above them. A hidden selection or
Find/Go-to target opens its containing fold; IME preedit temporarily shows all
rows. Automated geometry and interaction coverage plus a macOS live-app smoke
test pass. Windows runtime validation and broader native editing acceptance
remain pending; the optional legacy TextEdit fallback has no folding.

- [x] Add auto-indent and configurable indentation detection/size to the default shared editor.
- [ ] Validate bracket/quote completion and syntax-aware matching in native
  multi-cursor, selection, IME and undo workflows. Automated coverage is in place.
- [x] Add code folding with gutter affordances and persistent per-tab fold state
  in the default shared editor.

Exit gate: all editing operations remain one coherent history owner, preserve
grapheme boundaries and keep gutter/text hit testing aligned.

## 5. Extend document Find/Replace — deferred

Deferred by user direction while roadmap item 7 is pursued.

- [ ] Add regular expressions, capture-group replacement, multiline queries,
  Find in Selection and preserve-case options in measured increments.
- [ ] Keep invalid expressions, zero-width matches, CRLF, Unicode boundaries and
  partial-grapheme replacements safe and non-mutating, with clear feedback.

Exit gate: single and Replace All operations are undoable and respect the
accepted grapheme-safe rejection rule. See the existing
[editor plan](text-editor-improvement-plan.md) for detailed cases.

## 6. Add language services in stages — deferred

Deferred by user direction while roadmap item 7 is pursued.

- [ ] Define a language-server boundary and lifecycle (open/change/save/close,
  process failures, cancellation and document revision checks).
- [ ] First show diagnostics in the editor and a Problems panel; then add
  completion/parameter help, hover, definition/references, rename and format.
- [ ] Keep lexical highlighting useful without a server and avoid applying
  stale diagnostics or edits to a changed document.

Exit gate: at least one language has end-to-end native workflows, with server
absence/failure and stale-response regression tests.

## 7. Add Git source control and diff views — in progress

The first read-only milestone discovers the repository containing the opened
folder, lists staged/working/untracked paths in the Source Control activity,
and opens a staged, working, or untracked comparison in an editor tab. Git runs
off the UI thread; status is refreshed on workspace change or via the panel
action. The Git side is read-only, while the working side reuses a normal
editable document, including any unsaved buffer. Diff output is bounded to
256 KiB (128 KiB for untracked files). Markers are hidden if that buffer no
longer matches the captured disk snapshot. A macOS live-app check confirmed the
tabbed working diff layout and tab switching; Windows runtime remains untested.

- [x] Add read-only repository discovery, changed-file list and staged/working
  patch preview with bounded output and explicit error states.
- [x] Add a tabbed side-by-side diff for staged, working, and untracked files,
  with editable working content and dirty-buffer preservation.
- [x] Add Explorer status badges and periodic visible-panel refresh for external
  changes.
- [x] Add file-level staging/unstaging and commits with explicit error feedback
  and no implicit modification of unrelated worktree files.
- [ ] Add hunk staging, amend, push/pull, merge handling and history only after
  their independent workflows have error/cancellation coverage.
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
