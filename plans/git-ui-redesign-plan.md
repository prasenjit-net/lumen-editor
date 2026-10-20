# Git UI redesign plan: Source Control panel, History tab, ⋯ menu, multi-repo clarity

Status: proposed. This is a design and implementation plan; nothing here is built yet.
It supersedes only the *presentation* parts of `plans/git-feature-completion-plan.md`.
All Git behavior, safety rules, services and controllers from that plan stay as they are.

## Implementation status

Branch `f-git-history-ops`, uncommitted. Phases 1 to 5 are implemented; phase 6 (graph) is not.

Done:
- Shared `overflow_menu`, `disclosure_row`, `fit_galley`, and `panel_header_with_menu` in `src/ui/widgets.rs`; `change_color` in `theme.rs`.
- `src/ui/source_control_menu.rs` (menu contents as data, with disabled reasons) and `src/ui/source_control_rows.rs` (repository and file rows, repository kind detection).
- The wrapped button row is gone. Pull, Push, Fetch, Publish, Branches, Stash, Tags, History, operation controls and Force Push with Lease live in the repository `⋯` menu. The panel header keeps Refresh plus a panel-level `⋯`.
- Repository accordion with active marker, worktree/submodule/nested badges, per-repository counts and ahead/behind, `draft` marker, inline unreadable-repository rows, and a summary line.
- Operation banner, split Commit button with an Amend option and an "Amending" row with a stop button, collapsible change groups with hover actions, and file rows with icon, name, directory and a colored status letter.
- Layout regression tests (several repositories at 180/240/600 px, header `⋯` at the right edge, click-to-select, menu rules, repository kinds).

Phase 4 (History tab), done:
- `src/ui/git_history_tab.rs` replaces the modal. The tab sits in the strip after Settings, with its repository name; selecting it, closing it and the overflow list work like the other page tabs. The state is still `ViewState::history_dialog` (now meaning "the tab is open") plus `history_active`.
- Toolbar: ref dropdown, search, a collapsible Filters row (author, path) and a clear button. Virtualized list with ref chips, relative dates, hollow dot for merges, columns that drop author, then date, then hash as the tab narrows, automatic loading of the next page, Up/Down selection while the pointer is over the list. Resizable details pane (right when the tab is at least 640 px wide, below otherwise) with message, parents or the merge parent choice, a copy-hash button, a commit `⋯` menu, and changed files that open the existing read-only comparison inside the tab. Blame and comparisons also render in the tab.
- The review and tag dialogs open over the tab instead of closing it. The tab closes on workspace change or when its repository leaves a scan.
- Limits: one History tab (it follows the repository it was opened for; opening it for another repository starts it over), the graph column is a plain dot (phase 6), and the tab is not draggable into other editor groups (virtual tabs live in the first group).

Phase 4 review fixes: commit actions and tags from the History tab move the sidebar selection to the tab's repository first and are refused if that is impossible; the tab reloads after every completed Git operation and when shown again; Cmd/Ctrl+W closes it; opening a task editor replaces it; activation goes through `Command::ShowHistoryTab` and `enter_page_tab` like other pages; list keys are ignored while a Git dialog is open; chips use only the shown repository's remotes.

Second review pass (phases 4 and 5): a failed history page no longer retries every frame (Retry row); the tab reloads only when a scan shows the head moved or after tag changes, not for staging or other repositories; a repository that failed to read keeps its tab; the selected commit stays visible across reloads; blame replaces an open comparison; blame, file history and OpenHistoryAt no longer reload twice; the branch switcher closes when its anchor is not drawn and takes keys only while its filter has focus. Known and documented: stashes made outside Lumen appear after Refresh.

Phase 5 (branch quick-pick and extra sections), done:
- The selected repository's branch name is a button (with a caret) that opens a popover (`branch_picker`) reusing `BranchDialog` with `quick = true`: filter, local and remote lists, Up/Down/Enter, create from the typed name, Manage branches…. Escape or a press outside closes it. The full dialog is not drawn in quick mode.
- `app/repo_extras.rs` loads stashes and `SourceControlService::sync_commits` (outgoing and incoming, 50 each) for the selected repository whenever its upstream counts, latest commit or an operation generation change. Sections start collapsed and appear only when non-empty.

Differences from the design above, on purpose:
- The `⋯` button appears only on the selected repository's row. Actions target the selected repository, so other rows are selected by clicking them first.
- Menu items are flat (no Branch/Stash/Tags submenus) because each is currently one dialog.
- The panel `⋯` has Refresh and Rescan plus Clone. There is no "Fetch all repositories" or "Collapse all" because no such command exists yet.
- No "Commit & Push" or Sync morph in the Commit button; that needs a new chained command.
- The parked draft marker is the word `draft`, not a pencil glyph, until an icon is added.
- The "Acting on" footer was dropped; the active bar, dot and bold name carry that.

Not verified: native rendering. The screenshot tool had no display access in the implementation session, so only layout tests ran. Inspect the panel at 180, 280 and 420 px with one and with several repositories before calling phases 1 to 3 done.

## 1. Analysis of the current UI

Evidence is from `src/ui/source_control.rs`, `src/ui/explorer.rs` (sidebar header) and
`src/app/git_history.rs`. These are code observations, not runtime screenshots.

| # | Problem | Where | Effect on the user |
| --- | --- | --- | --- |
| 1 | Ten text buttons (`Branches…`, `Stash…`, `Tags…`, `History…`, `Fetch`, `Force Push…`, `Publish Branch`, Continue/Skip/Abort) wrap in a `horizontal_wrapped` row above the commit box | `source_control_panel` L110-176 | Wrapping makes a ragged block that changes height with sidebar width and pushes the commit box and the changes down by two to four rows. Rare and dangerous actions (Force Push) sit at the same weight as everyday ones. |
| 2 | Branch, upstream and ahead/behind are three plain text lines | L73-77 | The most important state has no hierarchy; no icon, no chip, no one-click branch switch. |
| 3 | Header icons (Pull, Push, Refresh) have no overflow menu; everything else is a text button | `explorer.rs` L200-221 | Two competing action systems. Fetch, publish and pull/push are split across the header and the body. |
| 4 | History is a 520 px modal (900 px when comparing) with three stacked inputs, a 260 px list, and one-line rows like `[main, origin/main]  a1b2c3d  12 Mar 2026  subject` | `history_dialog` L476+ | A modal blocks the workbench, cannot stay open next to the code, cannot be docked in a split, and wastes the screen. Refs, hash, date and subject are mashed in one string so nothing aligns or truncates sensibly. |
| 5 | Change rows are `M  path/to/file.rs` plain text; stage/discard buttons are `put()` at the row's right edge | L292-398 | No file icon, no name/dir split, no status color, and no way to see the end of a long path. Hover actions overlap the label. |
| 6 | Multi-repo is a flat list of rows (max five visible) above the branch lines, shown only with two or more repositories | `repository_list` L416+ | The list does not say which repo the branch label, commit box and buttons belong to. Per-repo counts, branch and ahead/behind are invisible until selected. Nested repos, worktrees and submodules are indistinguishable. |
| 7 | Commit controls: a raw checkbox "Amend" next to a button whose label flips to "Amend" | L247-264 | Easy to amend by accident; the state change is shown only by a label swap. |
| 8 | Errors, "No changes" and notices share `source_label` styling with the branch label | many | Errors do not look like errors; they blend into the hierarchy. |
| 9 | Stage-all, unstage-all and discard-all sit next to the "SOURCE CONTROL" label, in a second header below the first, which is already titled by `panel_header` | L195-226 | Duplicate title; bulk actions are not tied to the group they act on. |
| 10 | Dialogs for branches, tags, stashes and history operations are each a separate modal with their own layout | `branch_dialog`, `stash_dialog`, `tags_dialog` | Switching branch, the most frequent action, takes a modal round-trip. |

Strengths to keep: per-repo commit drafts, the safety review dialog, typed `Command`s for every action,
read-only comparison tabs, scroll-synced diff panes, and the existing `explorer_row` interactions.

## 2. Design principles

1. **One home per action.** Everyday state-changing Git actions live in the ⋯ menu; the body of the panel
   shows *state* and *files*, not a toolbar.
2. **Always say which repository.** Every section, menu and button that mutates Git state is visibly
   scoped to one repository, named in the same place every time.
3. **Disclosure over scrolling.** Repositories, change groups and secondary lists collapse; expansion is
   remembered per repository for the session.
4. **History is content, not a dialog.** It opens as an editor-area tab beside file and diff tabs.
5. **VS Code density.** 22 px rows, 11-13 px text, 3-5 px corners, theme tokens only (`Palette.success`,
   `warning`, `error`, `info`, `muted`, `accent`; no new colors). No shadows, no cards.
6. **Reuse behavior.** Menus, palette, shortcuts and buttons share the same `Command`s. No UI code opens
   dialogs or touches Git; the controllers in `app/` keep all workflows.

## 3. Target design (mocks)

Mocks are monospaced approximations at default sidebar width (~280 px) unless stated. `▾`/`▸` are the
shared disclosure carets (`icon::CARET_DOWN` / `CARET_RIGHT`), `⋯` is `icon::ELLIPSIS`.

### 3.1 Sidebar, one repository

```
┌ SOURCE CONTROL ──────────────────── ↻  ⋯ ┐   panel_header: title left, actions right
│ ▾ lumen            ⎇ f-git-history ↓1 ↑2 ⋯│   repo row (always shown, not collapsible when alone)
│ ┌───────────────────────────────────────┐ │
│ │ Message (⌘⏎ to commit)                │ │   multiline, grows 2..6 rows
│ │                                       │ │
│ └───────────────────────────────────────┘ │
│ [ ✓ Commit                       ▾ ]      │   split button; ▾ = Commit & Push, Amend last commit…
│                                           │
│ ▾ Merge Conflicts  1                      │   only when present; warning stripe, always first
│   ⚠ main.rs   src/app                  !  │
│ ▾ Staged Changes   2              − (hover)│   group actions appear on hover: unstage all
│   ▣ lib.rs    src/                     M  │   file icon · name · dim dir · status letter
│   ▣ theme.rs  src/ui                   A  │
│ ▾ Changes          3        ↶  +  (hover) │   discard all, stage all
│   ▣ git.rs    src/services             M  │
│   ▣ notes.md                           U  │
│   ▣ old.rs    src                      D  │
│ ▸ Stashes  2                              │   collapsed secondary sections
│ ▸ Outgoing 2 · Incoming 1                 │
└───────────────────────────────────────────┘
```

Details:

- **Repo row** = `branch chip` (click opens the branch quick-pick, see 3.4) + ahead/behind (`↓behind ↑ahead`,
  `success`/`warning` tint, tooltip with upstream name and last fetch time) + per-repo `⋯`.
- **Header actions** shrink to `Refresh/rescan` and the panel-level `⋯`. Pull and Push move into the menu;
  when the branch is ahead or behind, the commit split button's primary label becomes **Sync ↓1 ↑2** only
  if there is nothing to commit and the branch has an upstream; **Publish Branch** if it has none.
- **File row**: file icon from `file_icons`, bold-ish name, dim directory (`muted`, truncated first), status
  letter right-aligned and colored (`M` info, `A`/`U` success, `D` error, `R` info, `!` warning). Hover shows
  `[discard] [stage]` icon buttons over a `hover` background; the status letter hides while they show.
  Click opens the existing comparison; double-click opens the file; right-click keeps Open, Stage, Discard,
  Copy path, Reveal, and the conflict side choice.
- **Group headers** are disclosure rows (`CARET` + title + count badge). Group actions are hover-only so the
  idle panel is quiet; keyboard focus also reveals them. Tooltips stay descriptive.
- **Banners** for in-progress operations replace the old wrapped row: one thin strip under the repo row,
  `Merging · 2 conflicts left   [Continue] [Skip] [Abort…]`, `warning` tinted left border, buttons at the right.
  Errors use `error` tint with a dismiss `×`, never the muted style.
- **Empty states**: "No changes" becomes one muted line with a check glyph; no repository shows
  Initialize and Clone as two compact buttons.

### 3.2 The ⋯ menus

Two menus, same content layout, different scope:

- **Panel `⋯` (header)**: scope is *every* repository: Refresh / Rescan repositories, Fetch all repositories,
  Collapse all repositories, Open History (selected repo), Clone Repository…, Initialize Repository…
- **Repo `⋯` (on the repo row)**: scope is *that* repository. First line is a disabled caption with the
  repo name so the target is unambiguous even when invoked by keyboard.

```
┌ lumen · f-git-history ──────────────┐   caption (disabled): target repository
│ Pull                          ⌘⇧L   │
│ Push                                │
│ Fetch                               │
│ Fetch (prune all remotes)           │
│ ─────────────────────────────────── │
│ Branch                            ▸ │──┐ Switch branch…        (opens quick-pick)
│ Stash                             ▸ │  │ Create branch…
│ Tags                              ▸ │  │ Rename branch…
│ Remote                            ▸ │  │ Delete branch…
│ ─────────────────────────────────── │  │ Publish branch        (disabled: has upstream)
│ Show History                        │  └ Merge into current…
│ Show File History               (ed)│
│ Open Repository Folder in Explorer  │
│ ─────────────────────────────────── │
│ Amend Last Commit…                  │
│ Force Push with Lease…       (warn) │   danger group last, separated, labeled with "…"
└─────────────────────────────────────┘
```

Sub-menus: **Stash** = Stash changes, Stash including untracked, Pop latest, View stashes…;
**Tags** = Create tag…, Manage tags…; **Remote** = Fetch, Pull (fast-forward only), Push, Publish branch.
Disabled items show the reason via `on_disabled_hover_text` (for example "Resolve conflicts first",
"No upstream configured", "Another Git operation is running"). Anything that needs a decision keeps its
existing confirmation or review dialog. Destructive or history-rewriting entries stay at the bottom and keep
their ellipsis. Items dispatch the same `Command`s as today (`PullSourceControl`, `PushSourceControl`,
`FetchSourceControl`, `OpenBranchDialog`, `OpenStashDialog`, `OpenTagsDialog`, `PlanForcePush`,
`PublishSourceBranch`, `OpenHistoryDialog` → `OpenHistoryTab`).

### 3.3 Sidebar, multiple repositories

The point: the user can tell at a glance **how many repositories there are, which one is active, what state
each is in, and what any button will act on**.

```
┌ SOURCE CONTROL ──────────────────── ↻  ⋯ ┐
│ REPOSITORIES  3  (2 with changes)         │   summary strip, muted; click toggles collapse-all
│ ▾ ● lumen              ⎇ main     ↑2   5  │   ● = active repo (accent left bar + brighter name)
│ │ ┌───────────────────────────────────┐   │
│ │ │ Message…                          │   │   commit box and groups live INSIDE the active repo
│ │ └───────────────────────────────────┘   │
│ │ [ ✓ Commit                      ▾ ]     │
│ │ ▾ Staged Changes  1                     │
│ │   ▣ lib.rs   src                    M   │
│ │ ▾ Changes  4 …                          │
│ ▸   crates/settings-core ⎇ main        1  │   collapsed repo: name · branch · count, no body
│ ▸   tools/vendor/x  submodule of lumen  ⚠ │   badge + warning if dirty/uninitialized
│ ▸   ../lumen-wt  worktree ⎇ f-wt     ↓3   │   worktree badge; tooltip: shares objects with lumen
└───────────────────────────────────────────┘
```

Rules:

- **One accordion section per repository**, always rendered when there are two or more (a single repository
  uses the compact layout of 3.1). Clicking a collapsed repo header **selects it and expands it**; the
  previously active one collapses by default (a "keep others open" toggle is a later option). This keeps the
  existing "actions target the selected repository" rule and makes it visible.
- **Header anatomy** (right to left, fixed edge): `⋯` (hover/focus/active only), change-count badge,
  ahead/behind; left: caret, activity dot (● active, ○ others), repo name (truncates first), branch chip.
  Long names truncate; the `⋯` and badges never move.
- **Nesting and kind** are explicit: indent by one level for repos nested in another listed repo, and show a
  small kind badge (`worktree`, `submodule of X`, `nested`) with a tooltip carrying the full path and shared
  storage info (current `repository_list` tooltip text is reused).
- **Errors are per repo**: an unreadable repository renders as its own row with an `error` glyph and an inline
  message, not a loose label above the list. Other sections stay usable.
- **Drafts**: a repository with an unsent commit draft shows a small pencil glyph on its collapsed row, so
  parked drafts are not hidden.
- **Panel-level `⋯`** is the only place where "all repositories" actions appear, labeled as such.
- Selecting from the editor: switching the active editor tab to a file in another repository does not
  steal selection (unchanged behavior). The Source Control summary strip may offer "Switch to <repo>" as a
  later enhancement.

### 3.4 Branch chip quick-pick

Replaces the Branches modal for the common case; the modal stays reachable from "Branch ▸ Manage branches…".

```
 ⎇ f-git-history ▾  ──►  ┌──────────────────────────────┐
                         │ 🔍 Switch to branch or tag…   │
                         │ + Create new branch…          │
                         │ ─ Local ─────────────────     │
                         │ ✓ f-git-history   ↑2          │
                         │   main            ↓1          │
                         │ ─ Remote ────────────────     │
                         │   origin/feature-x            │
                         └──────────────────────────────┘
```

Uses existing branch listing and switching services; the preflight (unsaved buffers, worktree
occupancy, untracked overwrite) is unchanged and still shows its dialog on failure.

### 3.5 Git History as an editor tab

Opens in the editor area as a virtual tab, like Settings, task editors and chat tabs:
`⟲ History · lumen`. One tab per repository, keyed by repository root; opening again focuses it.
Closing it never prompts (read-only view state).

```
┌ ⟲ History · lumen ✕ ┬ main.rs ┬ ⇄ lib.rs (Staged) ┬ ───────────────────────────────────────────────┐
│ ⎇ All branches ▾  🔍 Search message, author, hash…   Author ▾  Path ▾  [↻] [⋯]    lumen ▾ (if >1)  │ toolbar
├──────────────────────────────────────────────────────────────────────────┬────────────────────────────┤
│ ╭  Add review dialog…  ⎇ main  origin/main   Prasenjit   2 h ago   a8bb3cf │ a8bb3cf9e1…           [⧉] │
│ ●  merge, cherry-pick, revert, reset…                     Prasenjit  1 d     │ feat(git): review dialog  │
│ │  history search, branch/tag filter…  🏷 v0.4                Prasenjit  2 d │ for history actions…      │
│ ●  read-only commit file comparisons…                     Prasenjit  3 d     │                           │
│ │  …                                                                         │ Prasenjit · 12 Mar 2026   │
│ ╰  load more ▾                                                               │ Parents  93566b4  ↗       │
│                                                                              │ ─ 6 files changed ───────  │
│                                                                              │  ▣ source_control.rs  M   │
│                                                                              │  ▣ git_history_ops.rs M   │
│                                                                              │  …                        │
│                                                                              │ [Cherry-pick][Revert]  ⋯  │
└──────────────────────────────────────────────────────────────────────────┴────────────────────────────┘
```

Layout and behavior:

- **Toolbar** (fixed height, one row): ref filter dropdown (All branches, HEAD, any branch or tag; existing
  `SetHistoryRef`), a single search field that accepts message text or a commit id, **Author** and **Path**
  as filter chips that expand into a small popover input (applied filters show as removable chips, replacing
  the three always-visible inputs), refresh, `⋯` (Load all refs, Copy log as text later, Export no).
  The repository selector appears only with two or more repositories and names the repo in the tab title.
- **Commit list**: virtualized `show_rows` table with columns *graph · subject (+ ref chips) · author · date ·
  hash*. Columns after subject drop in that reverse order as the tab narrows, the subject always keeps the
  remainder. Ref chips: branch `accent` outline, remote `muted` outline, tag `warning` outline, HEAD filled
  `accent`. Rows are 22 px, one line, subtle selection color. Date is relative with the absolute date in the
  tooltip. Paging keeps the existing "Load more" semantics but triggers automatically near the end, with a
  quiet footer row when more remain.
- **Graph column**: lane lines are drawn in the list painter with the shared row geometry. This is milestone 7
  in the Git plan, so the first release shows a plain ● column (merge commits drawn as a hollow dot) and the
  column is a no-op spacer; the real graph is a later phase and requires parent-accurate lane assignment in
  the model, not the UI.
- **Details pane**: right-hand resizable side pane (persistent width, minimum 220 px) that collapses below
  the list when the tab is under about 760 px wide. Shows full SHA with copy, subject, body, author/date,
  parents as clickable links (select that commit), merge parent choice as a segmented control, refs, and the
  changed file list. Selecting a file opens the existing **read-only comparison tab**; double-click is the same,
  no embedded diff in the history tab. `Compare with parent N` stays for merge commits.
- **Actions** per commit: in the context menu and the details pane `⋯`: Cherry-pick…, Revert…, Reset current
  branch to here ▸ (soft/mixed/hard), Create branch here…, Create tag…, Copy hash, Copy subject. All go
  through the existing `PlanHistoryAction` review dialog.
- **Entry points**: ⋯ → Show History, the repo row, the palette (`Git: Show History`), Explorer/editor
  context menu `File History` (opens the tab with the Path chip pre-set), editor `Blame` (keeps its view; its
  Back returns to the tab, not a modal).
- **Keyboard**: `↑/↓` select, `Enter` opens the first changed file, `⌘F` focuses search, `⌘C` copies the hash,
  `Esc` clears the filter, then selection. The list is focusable and has an accessible name.

### 3.6 Narrow and wide widths

```
Narrow sidebar (~190 px)                    Wide sidebar (~420 px)
┌ SOURCE CONTROL ────── ↻ ⋯ ┐               ┌ SOURCE CONTROL ───────────────────── ↻ ⋯ ┐
│ ▾ lumen   ⎇ main… ↑2   ⋯ │               │ ▾ lumen   ⎇ f-git-history   ↓1 ↑2     ⋯ │
│ [ Message…              ] │               │ [ Message…                              ] │
│ [✓ Commit            ▾ ] │               │ [ ✓ Commit                          ▾ ] │
│ ▾ Changes 3               │               │ ▾ Changes 3                               │
│   ▣ git.rs             M  │               │   ▣ git.rs      src/services           M │
└───────────────────────────┘               └───────────────────────────────────────────┘
```

- Below ~220 px the directory column and the branch name collapse (branch icon + ellipsis, full text in the
  tooltip); `⋯`, badges and status letters never move.
- History tab at narrow editor width: list only, details pane stacks below as a disclosure ("Details ▸").

## 4. Implementation plan

### 4.1 Shared building blocks (`src/ui/widgets.rs`, `src/ui/theme.rs`, `src/icon.rs`)

- **`overflow_menu(ui, id, items)`**: a compact `⋯` button (22×22, no frame) opening an egui menu. Items are a
  typed enum (`MenuItem::Action { label, shortcut, command, enabled, disabled_reason }`, `Submenu`,
  `Separator`, `Caption`) that resolves to a `Command`, so the menu never executes logic. Extend the header
  action model so `panel_header` can take a trailing menu while preserving the fixed-right layout.
- **`disclosure_row(ui, id, title, count, actions)`**: caret + title + count badge, hover-revealed action
  slot, remembers state in `ui.data` keyed by repository root and group. Replaces `source_heading`.
- **`status_chip` / `ref_chip`**: small 3 px-rounded labels using existing palette tokens.
- **Icons** (add to `tools/gen_icons.py`, regenerate the font): `GIT_COMMIT`, `GIT_MERGE`, `TAG`, `STASH`,
  `CLOUD_SYNC` (reuse `REPO_PULL`/`REPO_PUSH`/`GIT_BRANCH`/`HISTORY`/`ELLIPSIS` where possible), `PENCIL` for
  parked drafts. Do not use emoji or Unicode approximations in shipped code (the mocks use them only for
  illustration).
- **Theme**: map git status letters to existing `success`/`info`/`warning`/`error` tokens in one function in
  `theme.rs`; no new palette colors.

### 4.2 Sidebar (`src/ui/source_control.rs` split)

`source_control.rs` is already 2.7k lines; the redesign is the moment to split it without changing behavior:

- `ui/source_control/panel.rs`: panel assembly, repo sections, banners, empty states.
- `ui/source_control/changes.rs`: group headers, file rows, hover actions.
- `ui/source_control/commit_box.rs`: message editor, split commit button, amend chip.
- `ui/source_control/menus.rs`: the `⋯` menu definitions (pure data → `Command`).
- `ui/source_control/branch_picker.rs`: quick-pick popover.
- `ui/source_control/dialogs.rs`: existing branch/stash/tags/clone/revert/abort dialogs moved unchanged.
- `ui/git_history_tab.rs`: the history tab (toolbar, list, details pane).

Rules: preserve widget IDs (`source_control_commit_message`, `source_control_changes`) and rendering order so
commit focus, scroll position and same-frame actions stay stable. Keep `commit_focused`/`commit_focus` logic
as is. The commit box ID is *not* derived from the section, so the box keeps focus across collapse/expand and
repository switching (the existing per-repo parked drafts keep working).

### 4.3 App/state (`src/app/`)

- `view_state.rs`: add `source_control.expanded: BTreeMap<PathBuf, RepoSections>` (repo expansion + group
  expansion), session-only. No Git logic.
- Replace `history_dialog: Option<HistoryDialog>` with a keyed `HistoryTabs` (`HistoryTabId`, one per repo
  root), following `chat_tabs.rs` / `task_editor.rs`: stable IDs, never addressed by strip position or label.
  Move the existing `HistoryDialog` fields (filters, commits, selection, parent choice, blame, compare) into
  `HistoryTab` and keep `git_history.rs`, `git_history_ops.rs` job/pagination logic intact. Requests carry
  tab ID plus the repo and a generation so stale pages cannot replace newer ones.
- Commands: `OpenHistoryTab(Option<PathBuf /*path filter*/>)` (the old `OpenHistoryDialog` becomes an alias
  during migration), `CloseHistoryTab(id)`, `SetHistoryDetailsOpen`, `ToggleRepoSection(root)`,
  `ToggleChangeGroup(root, group)`, `OpenBranchPicker`. Everything else reuses existing commands.
- `ui/editor_tabs.rs`: add the history tab type to the virtual tab list next to Settings/task/chat tabs.
  Virtual tabs only appear in the first editor group today (`hide_virtual_tabs`); history tabs follow the same
  rule, and the tab strip index math (`task_base`, etc.) needs a small refactor to an enum of virtual tab
  kinds rather than another positional offset. Close, close-others, move and the Reopen Closed Tab path must
  be checked; history tabs are not reopened.
- `Workbench` renders the history tab via `Workbench::history_tab(ui)` using the shared editor viewport
  rules (one scroll authority; no second scroll area around the list).

### 4.4 Safety constraints (carry over from the Git plan and AGENTS.md)

- Menus only emit `Command`s; all I/O and confirmation stay in `app/`. No native dialogs from `ui/`.
- Operations remain serialized per repository; ⋯ items are disabled with a reason while a mutation runs.
- Fast-forward-only pull wording stays in labels and tooltips; Force Push keeps its review dialog.
- Dirty buffers, unsaved-edit guards, and stale-target rejection are unchanged. A history tab never holds a
  document and never dirties a buffer.
- The history tab's list uses virtualization; rows must not allocate per-frame strings for thousands of
  commits (precompute short hash, relative date label per page).

## 5. Phases

Each phase is shippable on its own and leaves existing behavior working.

1. **Foundations**: `overflow_menu`, `disclosure_row`, chips, new icons, status-color mapping, tests for header
   layout at narrow/default/wide widths (fixed-edge `⋯`). Split `source_control.rs` mechanically; no visual
   change; all current tests pass.
2. **Single-repo panel**: repo row with branch chip and ahead/behind, the ⋯ menus (moving every text button
   into them), operation banner, split commit button + amend chip, file rows with icons and hover actions,
   group disclosure. Remove the wrapped button row and the second "SOURCE CONTROL" header.
3. **Multi-repo clarity**: accordion sections, active-repo indicator, kind badges, per-repo errors and draft
   markers, summary strip, per-repo and panel-level menus with explicit scope captions.
4. **History tab**: `HistoryTabs` state, tab strip integration, toolbar and filter chips, virtualized list with
   ref chips and plain dot column, details pane, context menu actions, entry points (menu, palette, file
   history, blame back-link). Retire the modal once the tab covers search, filters, file history, compare,
   blame and history actions.
5. **Branch quick-pick** and secondary sections (Stashes, Outgoing/Incoming) as collapsed disclosures.
6. **Commit graph** (milestone 7 of the Git plan) rendered in the history list's graph column, backed by a
   UI-independent lane model.

## 6. Verification

- `cargo fmt --all --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Layout regression tests (extend the existing `source_sidebar_*` and `source_rows_stay_left_aligned_*` tests):
  header and repo-row `⋯` stay at the right edge at 180/240/420/600 px; long repo, branch and file names
  truncate without changing row height; sidebar width does not grow as status arrives.
- State tests: one repo vs three; collapsed repos still show counts; selecting a repo preserves its commit
  draft and message focus; per-repo error does not blank others; menu items are disabled with the right
  reason for no upstream, conflicts, running job, and no remotes.
- History tab tests: tab opens once per repo, reopens focus it, closes without prompt, survives repo refresh,
  stale page results are discarded, filters apply and clear, merge parent choice passes through to the existing
  comparison, narrow-width stacking.
- Manual visual check at representative widths with screenshots (single repo, three repos incl. a worktree and
  a submodule, conflicted state, 10k-commit history), compare before/after on a stable fixture.
- Windows: `cargo check --target x86_64-pc-windows-gnu` if installed; menu shortcuts show Ctrl on Windows and
  Command on macOS. Do not claim native runtime testing on a platform only checked by compilation.

## 7. Non-goals and open questions

- No hosting-provider PR/issue UI, no interactive rebase UI, no drag-and-drop staging, no bulk multi-select
  (still undocumented as unsupported, as in the Git plan).
- Tree view of changes (folder hierarchy) and "view as list/tree" toggle: deferred; the file row design
  works for both.
- Open: should activating a different repository auto-collapse the previous one (proposed) or stay
  independent? Open: should the summary strip show per-repo ahead/behind totals? Both are cheap to change
  after phase 3.
- Open: whether the history tab can be dragged into other editor groups. It follows today's rule that virtual
  tabs live in the first group; revisit with the editor-groups plan.
