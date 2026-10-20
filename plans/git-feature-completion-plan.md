# Git feature completion plan

Status: proposed. This is an implementation plan, not a claim that the listed features exist.

## Current implementation and scope

Lumen currently discovers the Git repository containing the single opened workspace folder. `src/services/source_control.rs` shells out to Git for porcelain v1 status, staged/working/untracked text comparisons, file staging and unstaging, working hunk staging, commit/amend, confirmed tracked-file discard, fast-forward pull, and push. `src/app/source_control.rs` runs these requests off the UI thread, polls status every 15 seconds while Source Control is visible, and coordinates diff tabs and editor safety. `src/ui/source_control.rs` shows a branch label, change groups, commit controls, and side-by-side comparisons. Explorer badges use the same snapshot. The current README documents these capabilities and explicitly says history and merge resolution are absent.

The useful baseline is already tested for file and hunk staging, amend, tracked discard, dirty comparison reuse, diff tabs, and sidebar layout. The remaining work should preserve this behavior and the existing document, dialog, and command boundaries.

## Findings to address first

| Finding | Evidence | Consequence / planned correction |
| --- | --- | --- |
| Repository state is singular | `SourceControlView` has one `workspace`, `snapshot`, `diff`, and commit draft; `snapshot()` calls `rev-parse --show-toplevel` once for the workspace | Nested repositories and worktrees inside a folder have no independent status, draft, branch, or operation target. Introduce repository identity and keyed view/controller state before adding more Git actions. |
| Replacing a job only drops its receiver | `refresh_source_control`, diff requests, and operations assign `source_control_job = None`; workers can still run | A refresh or click can overlap a mutation or hide its completion. Serialize mutations per repository, give reads request generations, and make completed work observable even if the panel changes. Remote processes need timeout/cancellation policy. |
| Hunk action is identified by its displayed ordinal | `stage_source_hunk` asks the service to recompute a patch and take `nth(hunk)` | External changes can make that ordinal refer to different text. Send the displayed patch identity/index revision, verify it before `git apply --cached`, and fail with refresh guidance on mismatch. |
| Commits always skip hooks | Git command includes `commit --no-verify` | This silently overrides repository policy. Use ordinary commit by default; expose an explicit hook bypass only if product requirements justify it. Handle hook output and failures. |
| Pull leaves clean open buffers stale | Successful pull only refreshes Git status; `check_open_disk_changes` sets a flag instead of reloading content | An open clean file can show pre-pull text. After pull, compare disk and saved text and reload clean tabs safely or present an explicit reload state; never overwrite dirty buffers. |
| Status parsing suppresses important states | `status --porcelain=v1 -z --no-renames --ignore-submodules=all` and a two-character `GitChange` | Renames, conflicts, submodule changes, and richer branch/upstream state cannot be represented accurately. Move to a typed status model with explicit unsupported/error states; do not flatten conflicts into ordinary modified rows. |
| Comparison is limited to small UTF-8 regular files | 256 KiB diff/content limit, 128 KiB untracked limit, text decoding | Keep bounded reads, but add clear binary/large-file/rename/deletion representations and a safe open-file fallback. |
| Remote operations have limited context | `push` and `pull --ff-only` use the configured upstream; no fetch, remote selection, ahead/behind count, or auth prompt integration | Show exact branch/upstream and operation outcome; add fetch and explicit publish/set-upstream flow. Never silently choose a remote. |

Other details worth fixing during foundation work: a detached branch is a string fallback rather than a typed state; a commit message is one line; amend can be enabled without a visible last-commit context; error text is clipped in the sidebar (hover or detail view should reveal it); stale diff snapshots stay open until reopened; and all-repository operations must keep selection and draft state stable while status refreshes. These are observations from code, not runtime claims.

## Target behavior

Source Control lists each repository in the opened folder, with its own branch, upstream, ahead/behind state, staged/working/conflict counts, commit draft, operations, and history. A file badge and diff resolve to the **nearest owning repository**, never the outer repository. The repository list supports root repositories, nested repositories, Git worktrees, and submodules; a submodule is a distinct repository plus a change entry in its parent when Git reports one. A folder with no repository offers initialization as an explicit action. Keep initial discovery bounded and avoid traversing ignored/generated directories blindly.

History shows recent commits for the selected repository, with author, timestamp, subject, full SHA in details, parents, decorations, and a graph delivered after operational workflows. Selecting a commit shows its metadata and changed files; opening a file shows a read-only comparison between the commit and its first parent, with an explicit parent choice for merge commits. Pagination prevents loading unbounded history. Branch and tag filters, search, and file history follow after the basic log is reliable.

The completion target is a native Git workbench covering routine repository creation, local changes, collaboration, inspection, conflict resolution, and recovery. Intermediate releases may ship a smaller capability set, but must describe it accurately. Graph rendering follows operational completeness.

## Priority review and deferrals

Ordering review: the milestone sequence is sound in dependency terms (safety, then state model, then everyday local/remote work, then branches/conflicts, then inspection, then history rewriting, then specialised layouts and graph). Conflict resolution lands before merge/rebase, and the safety gate precedes every mutation. Adjustments:

- **Pull basic log forward.** A bounded commit list with details (first bullet of milestone 4) ships with milestone 3, because branch switching, conflict context and recovery all need to show commits. Search, filters and comparisons stay in milestone 4.
- **Keep the state model minimal in milestone 1.** `RepositoryId`, keyed snapshots/drafts, typed status/HEAD/conflicts and nearest-owner resolution are cheap now and expensive to retrofit, so they stay. Linked-worktree common-directory coordination, submodule discovery and traversal tuning move to the deferred tier.
- **Split milestone 5.** Everyday history operations (merge, cherry-pick, revert, reset, lightweight/annotated tags, force-with-lease) stay; interactive rebase editing and reflog browsing move to the deferred tier.
- **Milestones 6 and 7 stay last.** Worktree/submodule lifecycle and the graph are valuable but not blockers for routine Git use.

### Core tier (build in this order)

| Order | Scope |
| --- | --- |
| 0 | Correctness and safety foundation (unchanged) |
| 1 | Repository identity, typed status/HEAD/conflict state, per-repository sections, nested/sibling repositories, rescan |
| 2 | Init, clone, multiline commit/amend with hooks, fetch/prune, publish, pull/push to the configured upstream, remote listing, credential helper / SSH agent / askpass |
| 3 | Branch list/create/switch/rename/delete, conflict inspection and resolution with continue/abort, stash, basic paginated log and commit details |
| 4 | Commit file comparisons, merge-commit parent choice, history search by message/author, branch/tag filter, file history, blame |
| 5 | Merge, cherry-pick, revert, reset modes with preflight, tags, force-with-lease |

### Deferred tier (handle after core is accepted)

| Feature | Original milestone | Reason to defer |
| --- | --- | --- |
| Selected-line staging/unstaging | 2 | Whole-file and hunk staging cover routine work |
| Remote add/rename/remove and separate fetch/push URL editing | 2 | Rare; `git remote` outside the app is acceptable at first |
| Author-identity setup UI with scope choice, signing configuration UI | 2 | Surface actionable errors first; configured signing still works through Git |
| Explicit pull/push source and destination ref selection | 2 | Upstream plus publish covers common flows |
| Linked-worktree shared-directory coordination, worktree identity grouping | 1 | Needed only once worktree management exists (milestone 6) |
| Base/ours/theirs behavior for rename and binary conflicts beyond pick-a-side | 3 | Text conflicts first; others offer ours/theirs choice |
| Arbitrary two-revision and merge-base branch comparison | 4 | Commit/parent comparison covers most needs |
| Rename-following depth, search by path/object ID | 4 | Basic file history first |
| Interactive rebase todo editing | 5 | Large and risky; plain rebase/merge cover most cases |
| Reflog browser and recovery-branch UI | 5 | Rare; document `git reflog` meanwhile |
| Signed tags, remote tag deletion | 5 | Lightweight/annotated tags suffice first |
| Worktree and submodule lifecycle | 6 | Specialised layouts |
| Commit graph | 7 | Polish; history list is usable without it |

Deferral does not weaken the safety rules: ordinary repositories that use a deferred feature (for example a repository with worktrees or submodules) must still show correct status and clear unsupported-operation messages. The completion checklist above still lists every row; deferred rows are accepted last, and the "fully featured" claim waits for them.

## Completion checklist and boundaries

Every required row must have service/controller tests, usable commands and UI, error/cancellation handling, and documentation before the feature is called complete. Existing partial implementations do not satisfy a row automatically.

| Required capability | Completion criteria | Milestone |
| --- | --- | --- |
| Repository foundation | Independent nested/worktree/submodule status, typed conflicts and HEAD/upstream state, explicit ownership and rescan | 1 |
| Create/open repositories | Init with initial branch choice; clone with destination validation, progress/cancel, and open-on-success | 2 |
| Index and commits | File, hunk and selected-line staging/unstaging; confirmed tracked discard; multiline commit/amend with hooks, identity and signing support | 2 |
| Remote collaboration | Remote add/rename/remove and fetch/push URL editing; fetch/prune, publish, explicit pull/push targets, authentication and progress | 2 |
| Branches | Local/remote listing, tracking branch creation, create/switch/rename/delete with worktree occupancy and dirty-buffer handling | 3 |
| Conflicts and stash | Conflict inspection/resolution, operation-specific recovery controls, stash create/list/inspect/apply/pop/drop | 3 |
| Inspection | Paginated searchable log, ref filters, details, arbitrary revision comparison, file history across renames, blame and branch comparison | 4 |
| History operations and recovery | Merge, rebase including interactive todo editing, cherry-pick, revert, soft/mixed/hard reset, reflog and recovery branch creation | 5 |
| Tags and guarded publication | Lightweight/annotated tags, configured signing, delete and explicit push; force-with-lease with captured expected remote object ID | 5 |
| Worktrees and submodules | Worktree list/add/open/remove/prune; submodule inspect/init/update/sync with explicit recursive scope | 6 |
| Graph | Virtualized parent-accurate graph integrated with filtered/paginated history | 7 |

Explicit exclusions for this plan: hosting-provider PR/issue/review APIs, Git server administration, bisect automation, arbitrary history filtering, permanent untracked cleanup, and patch email workflows. Dedicated Git LFS management is deferred; repositories using configured filters must still receive correct status, actionable filter errors, and clear preview limitations. Sparse checkout, partial/shallow clones, and bare repositories must be detected and supported within documented limits; creating/managing those specialized layouts is deferred. These exclusions do not justify silently ignoring their effects in ordinary repositories.

## Architecture and data contract

1. Add a stable `RepositoryId` based on canonical worktree root plus Git common-dir/worktree identity. Keep repository discovery and Git commands in `services/`; do not infer ownership from a path prefix alone. Use longest valid worktree root match, with symlink and path normalization handled consistently on macOS and Windows. Validate every operation's repository and relative path again in the service.
2. Replace the single snapshot with `RepositorySnapshot { id, root, git_dir/common_dir, head, upstream, ahead, behind, state, changes }`, keyed by repository ID. Model `Head` as unborn, attached, or detached; model normal, merge, rebase, cherry-pick, and revert states explicitly. A change should carry typed index/worktree status, rename source, conflict stages, and submodule status where available. Prefer porcelain v2 `-z` and test its record grammar, including non-UTF-8 paths or a deliberate displayed limitation.
3. Split service interfaces by capability: discovery/status, content/diff, history/refs, index/local mutations, and remote operations. Keep `app/` responsible for request routing, preflight, confirmation, document reconciliation, and stale-result checks. UI emits typed commands with `RepositoryId` and stable commit/path IDs; it never launches Git directly.
4. Use independent read jobs, a worktree-local mutation queue, and coordination keyed by canonical Git common directory for operations affecting shared refs/configuration/objects. Define lock scope/order without serializing unrelated repositories; Git locks remain authoritative against external processes. Fetch and ref mutations invalidate affected views in every linked worktree. Read results include repository ID and generation; stale reads cannot replace newer state. Operations must finish and report even if the user switches repository or activity. Refresh the affected repositories after every mutation outcome, including failure, cancellation, timeout, and worker disconnect; do not clear an unrelated repository's diff or draft. Give every operation an ID, target, progress, bounded output log and terminal outcome that remains reachable across workspace changes. Define cancellability and platform-specific process-tree termination, drain output without deadlocks, and reconcile after termination. Cancellation is not rollback; report uncertain outcomes and never automatically retry a mutation. Keep remote Git from hanging the UI indefinitely and surface actionable output with credentials redacted.
5. Use structured revision specs internally (`HEAD`, index, worktree, commit object ID, selected parent) rather than interpolated revision strings from UI input. Invoke Git with argv, `--` before paths and literal pathspec semantics for selected filenames (argv plus `--` alone does not disable pathspec magic), noninteractive behavior appropriate to the operation, and bounded output. Verify immutable commit IDs before loading history content. Avoid crossing repository boundaries through pathspecs or symlinks.
6. Treat document reconciliation as a shared post-operation workflow. Re-read affected disk/index/ref state after all outcomes, including partially completed failures; reload only clean unchanged-in-memory buffers, preserve dirty buffers, and explicitly represent deletion/rename or reload conflicts. Cover external Git activity and operation restart/recovery as well as Lumen commands.
7. Reuse the existing comparison surface for read-only historical revisions while giving each tab a stable key `(repository, path, left revision, right revision)`. Preserve editable working comparisons and document IDs. Show stale-snapshot affordances and reload explicitly. Support rename path mapping, added/deleted files, binary files, and large files without pretending they are text diffs.

## Delivery sequence and acceptance gates

### 0. Correctness and safety foundation

- Remove implicit `--no-verify`; retain the draft and show full hook failure output. Define how hooks that modify the index are reflected in the next snapshot.
- Make mutation dispatch serialized and completion durable, then add generation checks for reads. Verify refresh/diff clicks during a slow commit, pull, and stage operation do not lose the result or start an unsafe overlapping mutation.
- Bind hunk staging to the exact displayed patch and index/worktree revision; reject stale requests. Test external file edits, index edits, reordered hunks, and failure without modifying another hunk.
- Reconcile clean open documents after every mutation outcome, and flag dirty or changed-on-disk tabs. Test modified/deleted/renamed files, edits made while a job runs, partially completed failures, cancellation and timeout. Keep fast-forward-only pull semantics.
- Require literal file targeting; test filenames containing wildcards, brackets, leading colons/dashes, tabs and newlines. Distinguish symlink entries from traversing their targets; represent executable-bit and symlink changes without following paths outside the repository.
- Make error details reachable at narrow widths; document the binary and size limits.

Gate: no Git action silently changes the wrong content, loses its outcome, or bypasses hooks by default.

### 1. Repository discovery, rich status and independent state

- Adopt typed porcelain v2 parsing and branch/upstream metadata. Represent rename source/destination, unmerged index stages, submodule dirtiness, detached/unborn HEAD and ahead/behind counts; provide conflict inspection immediately, even before resolution controls ship.
- Discover worktree roots under the workspace with bounded traversal and `.git` file support; account for nested repos and submodules. Define and test traversal limits and a manual refresh/rescan command.
- Show one compact repository section per repo, with a stable selection, branch label, counts, and path tooltip. Persist section expansion and commit drafts by repository for the session. Keep action buttons at the panel's fixed edge via the shared header widget.
- Route Explorer badges, diff tabs, staging, commit, push, and pull by owning repository. Make bulk actions explicitly scoped to one repo. A missing/unavailable repository gets its own error without blanking other sections.
- Test two sibling repositories, nested repositories, submodules, separate worktrees sharing a common directory, a repository above the opened folder, path moves, and workspace switching. Test narrow/default/wide sidebar layouts and per-repository focus stability.

Gate: no action on repository A mutates B, an outer repository never claims a nested repository's files, and shared-ref operations invalidate all affected linked worktrees. Test concurrent fetch/ref changes across linked worktrees and external Git lock contention.

### 2. Repository creation, commits and remotes

- Add Init and Clone with explicit destination and initial branch options as applicable. Never overwrite an existing destination. On cancelled/failed clone, identify partial output and offer cleanup only for content created by that operation; never recursively delete pre-existing user content. Open the result only on success.
- Add multiline commit drafts, last-commit context for amend, author identity setup with explicit repository/global scope, and configured GPG/SSH signing support through installed tooling. Preserve drafts after hook/signing failure; refresh state even when a hook fails after modifying files/index. Do not store signing secrets.
- Complete file/hunk/selected-line stage and unstage with revision-bound selection. Confirm tracked file/hunk discard against the exact displayed content; destructive actions remain unavailable for unsupported file types. Preserve the current whole-file behavior until partial operations pass their own tests.
- Add remote management, Fetch/Prune, Publish Branch, and explicit Pull/Push source/destination ref selection. Show fetch and push URLs separately when configured differently, upstream and ahead/behind freshness. Keep default pull fast-forward-only; divergence leads to explicit merge/rebase choices when available.
- Integrate installed credential helpers, SSH agent and supported askpass/pinentry flows without a terminal dependency. Define prompt ownership, host verification failure guidance, cancellation, secret redaction and progress. Never disable host verification or persist credentials in ordinary settings/logs.
- Test missing Git/unsupported version, missing identity, multiline messages, hook stdout/stderr, signing success/failure, clone cancellation, no remote, deleted upstream, distinct push URL, HTTPS/SSH success via controlled fakes, authentication failure, cancelled prompts and timeout. Keep real-network validation opt-in.

Gate: users can create or clone, stage, commit and publish through the app using configured authentication/signing, with visible destinations and recoverable failures.

### 3. Branches, conflict resolution and stash

- Add local/remote branch picker, tracking branch creation, create/switch/rename/delete. Preflight unsaved buffers, worktree/index changes, untracked overwrite risk and linked worktree occupancy. Avoid force variants in ordinary paths; reconcile after every outcome.
- Detect operations started outside Lumen and expose their real state. Build base/ours/theirs/result conflict views using index stages, editable result with one editor/history owner, explicit save and mark-resolved, and remaining-conflict counts. Define ours/theirs labels correctly for rebase. Support content, add/add, modify/delete, rename and binary conflicts; offer path/version choices where text merging is impossible.
- Supply operation-specific continue/skip/abort only when valid. Abort must explain that Git may be unable to restore the original worktree; inspect resulting state and retain recovery guidance on failure. Basic log/immutable revision reads needed here can land before the complete history UI.
- Add stash create with message and explicit tracked/untracked inclusion, list and inspect, apply/pop with explicit index-restoration choice, and confirmed drop. Preserve stash after failed/conflicted pop; do not auto-stash dirty work without an explicit user choice. Never imply unsaved editor text is included in a disk-based stash.
- Test detached/unborn HEAD, branch deletion refusals, occupied worktree branches, conflict variants, stale resolution views, outside-app operations, restart during conflicts, stash apply/pop conflicts and abort failure.

Gate: users can switch branches and resolve or recover from conflicts without losing editor buffers; graph work is not a dependency.

### 4. History, comparisons and blame

- Add a bounded log query (for example 100 commits/page) with stable object IDs, parent IDs, author/date, subject, and ref decorations. Use structured NUL-delimited fields or another unambiguous parser. Support empty, shallow, detached, and corrupt/unavailable repository states.
- Add a History section/tab with pagination, selected commit detail, changed-file list, and read-only commit/parent and commit/file comparisons. Test merge commit parent choice, rename, deletion, binary/oversize, and path names containing spaces/newlines.
- Add history search by message/author/object ID/path, branch/tag filters, rename-following file history, blame with commit navigation, and explicit two-revision/merge-base branch comparisons. Distinguish author and committer dates and root commits compared with an empty tree. Preserve rename limitations in documentation. Keep reads independent from status polling and cache by immutable IDs/ref generation; anchor pagination to a revision so ref changes cannot duplicate or skip rows.

Gate: opening history or a commit comparison never changes the index, worktree, or an unsaved document.

### 5. History operations, tags and recovery

- Add explicit merge and rebase (including interactive todo preview/edit, validation and recovery), cherry-pick and revert with merge-parent selection where needed. Show selected commits, target and expected ref IDs before mutation; reject stale confirmations and offer operation-specific continue/skip/abort.
- Add soft/mixed/hard reset with exact effects on HEAD/index/worktree shown before confirmation. Require explicit confirmation for discarded changes or rewritten history. Offer recovery refs for moved commits and reflog browsing with recovery branch creation; explain that reflog cannot recover arbitrary uncommitted file content.
- Add lightweight/annotated tags with configured signing, inspection, confirmed delete, and explicit remote tag push/delete. Add force-with-lease only as an explicit command with remote/ref and expected object ID captured at confirmation; never fall back to unrestricted force.
- Test divergence, stale expected refs, merge commits, interrupted sequences, interactive rebase validation, signing failure, reflog expiry/missing objects and lease rejection. Confirm recovery after app restart without treating an operation ID as proof that a remote write did or did not succeed.

Gate: every history-changing workflow has scoped preflight, visible effects, tested interruption handling and accurate recovery limits.

### 6. Worktree and submodule lifecycle

- Add worktree list/add/open/remove/prune. Show branch occupancy, locked/missing entries and dirty state. Block ordinary removal of dirty worktrees, preserve unrelated buffers and validate the exact target before cleanup.
- Add submodule inspection, init/update/sync and nested status navigation with explicit recursive scope. Show recorded gitlink versus checked-out commit; expose detached HEAD, uninitialized and dirty states. Parent staging must stage the gitlink intentionally and must not silently stage child content.
- Reuse remote progress/authentication and shared-directory coordination. Detect specialized layouts and configured filters; report unsupported operations clearly instead of flattening them into ordinary files.
- Test separate worktrees sharing refs, locked worktree removal, external relocation, uninitialized/nested submodules, dirty child worktrees and parent/child operation races.

Gate: lifecycle operations preserve worktree ownership and make parent/submodule effects explicit.

### 7. Commit graph and polish

- Render a virtualized lane graph from commit parent IDs and decorations, aligned with history rows. Use stable deterministic lane assignment across page boundaries; keep lanes compact and legible at narrow widths. Show merges and branch divergence, not just a straight line.
- Paginate with enough parent context to connect visible pages; cap lane count and provide an accessible text description when the graph is too dense. Keep graph selection and scrolling synchronized with commit details.
- Test linear, fork/merge, octopus merge, criss-cross, shallow boundary, and paginated histories. Use screenshot/live-app checks at narrow/default/wide widths and both themes.

Gate: graph edges correspond to actual parent IDs and remain stable while paging/filtering.

## Verification approach

Build service integration fixtures using temporary repositories and local bare remotes; keep tests offline. Cover Git command failure, non-UTF-8/binary/large files, unusual path names, race and cancellation scenarios, nested repositories, linked worktrees, and branch checkout with unsaved editors. Inject a fake service for app controller tests so UI tests do not depend on a user's Git configuration or credentials. Isolate HOME/global/system Git configuration in fixtures, set deterministic identities and disable incidental hooks/signing; enable controlled hooks/signers explicitly in dedicated tests. Declare a minimum Git version and test capability failures. Assert index, worktree, refs, open document text/dirty flags, and repository selection after success, failure, cancellation and timeout. Include linked-worktree concurrency, literal pathspecs, CRLF/attributes/filters, executable modes, symlinks, SHA-256 object IDs where supported, and restart recovery. Test authentication/signing through injected adapters and controlled helpers; never read real credentials. Add layout tests plus live screenshots at representative widths. Run `cargo fmt --all --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`; where installed, cross-check Windows and run macOS bundling for platform-sensitive changes. Update README and the VS Code feature roadmap only when each milestone lands.

## Suggested release cuts and completion tracking

1. **Safe existing Git:** milestone 0.
2. **Repository-aware workbench:** milestone 1.
3. **Everyday local and remote Git:** milestone 2.
4. **Branches, conflicts and stash:** milestone 3.
5. **History and inspection:** milestone 4.
6. **History operations and recovery:** milestone 5.
7. **Worktree and submodule management:** milestone 6.
8. **Graph and completed workbench:** milestone 7 and every required checklist row accepted.

Track each checklist row as not started, in progress, or accepted, linking implementation changes and verification evidence when work lands. All rows are currently proposed/unaccepted; the baseline remains as described above. A release cut is not a claim of full completion. Keep optional hosting integrations and specialized administration outside this plan rather than allowing routine Git workflows to remain indefinitely optional.

## Reference contracts

- [Git invocation and literal pathspecs](https://git-scm.com/docs/git): argv and option separators do not alone make file targeting literal.
- [Linked worktrees](https://git-scm.com/docs/git-worktree): coordinate shared repository data separately from worktree-local state.
- [Rebase](https://git-scm.com/docs/git-rebase): use operation-specific continuation/recovery and correct conflict-side semantics.
