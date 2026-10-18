# Task system implementation plan

Prepared: 2026-10-17. Reviewed against the code on 2026-10-17 (after the document-identity,
split-editor, CLI-open and own-icon-font work). Status: proposed; no implementation yet.
Applies to the Rust/egui Lumen workbench and repository `AGENTS.md`.

## Objective

Implement the existing Run and Debug activity as a useful task workbench. Users
should be able to create, configure, run, monitor, stop, and rerun project or
personal tasks, with each execution's output available in a task-named tab beside
the integrated Terminal, Output, and Problems views.

Support build, test, lint, format, scripts, development servers, and arbitrary
local tools through a small set of execution primitives. Tool-specific commands
should normally be templates/configuration, not new execution backends. This is
a task runner, not a debugger: breakpoints, stepping, launch configurations, DAP,
remote execution, containers, and a VS Code task-extension API are separate work.

## Current implementation and integration points

- `Activity::RunDebug` and its `icon::DEBUG` glyph/label already exist in
  `src/app/view_state.rs` and `src/ui/activity_bar.rs`; the placeholder is the
  `unavailable_panel` call in `src/ui/explorer.rs`, pinned by the test
  `unfinished_activities_explain_themselves_and_offer_no_live_controls`, which
  must be updated when the activity goes live (keep it for Extensions). Populate
  that activity; do not add a second task activity icon or claim debugger
  functionality.
- `src/app/terminal.rs` owns one integrated shell via `LumenApp::terminal_session`.
  `TerminalView` holds its emulator, sizing, errors, and scroll state. The
  emulator itself is the pure `crates/terminal-core` crate (`Terminal::feed`,
  `resize`, `screen`), which task PTY tabs should reuse as is.
- `src/services/terminal.rs` starts a PTY shell, reads bytes on a thread, and
  exposes write/resize/exit checks. Its output channel is unbounded, exit polling
  discards the exit code, and Drop kills the immediate child. These are not yet
  adequate guarantees for concurrent task output or process-tree cancellation.
- `src/services/agent_process.rs` is existing prior art for child processes:
  `isolate_process_tree` (Unix process group / Windows `CREATE_NEW_PROCESS_GROUP`),
  `kill_process_tree` (group SIGTERM then SIGKILL after a fixed 30 ms; `taskkill /T /F`
  on Windows), `resolve_program` (PATH/PATHEXT lookup), and a bounded-grace stop.
  Extract these into a shared service module for tasks rather than duplicating them.
  They are not yet sufficient for this plan: the Unix escalation is not a
  configurable grace wait, and Windows uses `taskkill` rather than a Job Object, so
  the Job Object/verified-equivalent gate below still applies.
- `src/ui/terminal.rs` and `src/ui/output.rs` each create a bottom panel with ID
  `terminal`; open flags make them mutually exclusive. Problems has another
  renderer. `app/mod.rs` invokes these components separately.
- `tabbed_panel_header()` in `src/ui/widgets.rs` takes a borrowed-label `PanelTab`
  slice and anchors commands on the right, so dynamic labels are possible, but the
  row has no overflow: each label is only truncated. It needs a bounded
  scrollable/overflow presentation, and tabs keyed by `RunId` rather than by label.
- Editor identity is now `DocumentId` with up to two editor groups. Save-policy
  prompts, `${file}` and "open file from output" must use `DocumentId` and the
  focused group's active document, never a tab index or the legacy active file.
- Workspace config under `<root>/.lumen/` already has symlink/containment checks in
  `src/services/settings.rs`; the tasks store must reuse that guard (refuse a
  symlinked `.lumen`, keep writes inside the opened directory).
- Settings schemas/codecs live in `crates/settings-core/`; filesystem storage
  lives in services and draft/scope behavior lives in app. Task definitions need
  their own collection schema/file, not hundreds of dynamic settings controls.

## Product decisions

### Run and Debug sidebar

Keep the existing activity identity and native/workbench visual conventions.
Inside it, render a TASKS panel using `panel_header()` with right-aligned Run,
New Task, and additional actions in a stable order. Use compact groups:

1. Running: active/queued tasks, elapsed time, readiness, Stop and Show Output.
2. Workspace tasks: configured project tasks, grouped by build/test/custom group.
3. User tasks: personal reusable definitions, visibly scoped.

The empty state offers Create Task and Open Task Configuration. When no folder is
open, explain that workspace tasks require a folder; user tasks with an explicit
working directory remain runnable. Search/filter tasks locally. Double-click or
the Run action starts a task; ordinary selection only selects it. Context actions
include Run, Run with Inputs, Rerun Last, Configure, Duplicate, and Delete
Definition. Deleting a definition never deletes files produced by the task and
never silently stops an already-running snapshot.

Use one typed-command path for sidebar, command palette, menus, and shortcuts.
Add Run Task, Run Build Task, Run Test Task, Rerun Last Task, Show Running Tasks,
Terminate Task, and Configure Tasks. Assign platform-appropriate shortcuts only
after checking existing bindings; the initial feature can use the palette.

### Creating and editing tasks

Provide a compact guided form (a dedicated editor tab, specified in "Task editor
tab" below) as well as an editable TOML file. The form collects
scope, stable ID, label, kind, executable/command, arguments, working directory,
environment, group/default, and output behavior. Shell and process fields have
different labels so users understand when shell syntax is interpreted.

Start with Blank Process, Blank Shell, Cargo Build/Test, and npm Script templates.
Templates are editable examples, not automatic discovery or implicit execution.
A compound-task form offers task selection and sequential/parallel ordering.
Advanced fields remain available in TOML; preserve comments/unknown data and do
not silently discard unsupported fields when using the form.

Validate before saving; never run to validate. Offer Save and Run as an explicit
additional action. Config creation is atomic/no-clobber; updates use conflict
checks and staged replacement. If the config is open in a dirty editor, do not
apply form changes over it: offer switching to that editor or saving/cancelling
its changes through existing document workflows. Reuse settings storage patterns
without merging task drafts into unrelated settings drafts.

### Task editor tab (create/update UI)

The guided form is a workbench tab beside the editor tabs, not a dialog, floating
card or sidebar form. It follows the existing Settings tab: a virtual (non-document)
tab drawn by `ui/editor_tabs.rs`, currently hosted only in the first editor group
(`hide_virtual` suppresses it in other groups, and `editor_split` tests assert one
Settings tab). It owns no document history and never marks an editor buffer dirty.

**Tab model.** Introduce a small `VirtualTab` enum (Settings, Diff, TaskEditor) in
`app/view_state.rs` instead of another `bool open` flag per tab, with the active
virtual tab and ordering stored beside `EditorState` groups. Task editors are keyed
by a `TaskEditorId` (not a row index or label): `Existing(scope, task.id)` or
`New(draft_id)`. Opening the same existing task focuses its tab (duplicate handling
like documents); New Task always opens a fresh draft. Tab title is the task label
(`Task: Build`, `New task`), with a dirty dot when the draft differs from the saved
definition, a tooltip with scope and file path, and a close button. Dirty close asks
Save / Discard / Cancel through the same close prompt flow as documents, with the
draft identified by `TaskEditorId`. Settings and tab-command behavior must not
regress; a prerequisite refactor with before/after fixtures lands first.

**Entry points** (all one typed command path): New Task and Configure/Edit Task from
the sidebar header and context menu, empty-state Create Task, palette commands
`Tasks: New Task`, `Tasks: Edit Task`, `Tasks: Open Task Configuration (TOML)`, and
an Edit action in a task's output tab. Edit Task from a running task edits the
definition only; the running snapshot is unchanged.

**Layout** (compact, VS Code-like, using `panel_header()`, shared theme tokens,
`surface()`/`canvas()` and Lumen icons):
- Header: left-aligned title; right-aligned Save, Save and Run, Revert, Open in
  TOML, with tooltips and stable order.
- Scope and identity strip: User / Workspace scope selector (Workspace disabled with
  an explanation when no folder is open), resolved file path, stable ID (editable
  only for New; renaming an existing ID is a separate explicit action that updates
  `depends_on` references or refuses with a list of dependents), label.
- Sections in one scrollable column, each collapsible: Command (kind selector
  Process/Shell/Compound, executable or command, argument list editor, shell
  choice), Environment (key/value rows, unset list), Run (cwd with variable
  completion hints, group, default flag, save policy, allowed exit codes, timeout,
  instance policy), Dependencies (task picker of same-scope tasks plus explicit
  `user:` references, order Sequence/Parallel, drag or up/down reorder), Output
  (transport, reveal, focus, reuse, clear), Background (readiness text and
  timeout, shown only for background tasks), Platform overrides (Windows/macOS/
  Linux tabs limited to execution fields). Fields irrelevant to the selected kind
  are hidden, not disabled-and-confusing; switching kind keeps the entered values
  until Save and warns about what will be dropped.
- Preview: a read-only resolved summary (executable/shell, argv, cwd, env names,
  dependency order) and the exact TOML fragment that Save will write. Preview never
  runs anything and never shows secret input values.
- Footer: pending-change count, validation summary that focuses the first error.
- Narrow widths: section nav becomes a chooser, controls stack under labels, header
  actions keep right-edge anchoring; no horizontal scrolling.

**Validation.** Inline, per-field messages produced by `task-core` (the same validator
as the TOML path, so the form cannot accept what a file load rejects): empty command,
invalid/duplicate ID within scope, unknown variables, invalid cwd/env names, missing
or cyclic dependencies, conflicting defaults, bad platform fields. Errors appear
beside the field and in the footer summary; Save is disabled while errors exist, and
Save and Run additionally requires a runnable (non-compound-empty) definition.
Validation may be debounced but never touches the filesystem or spawns a process.

**Persistence and round-trip.** The tab edits a typed draft; Save applies a
comment-preserving, minimal TOML edit of only that task's table (new tasks are
appended; existing unknown keys, comments, ordering and other tasks are untouched).
Writes use the Phase 1 store (atomic create, conflict check, staged replace). If the
file changed on disk since the draft was loaded, show a conflict state with Reload
Draft, Overwrite after review, and Open in TOML; never silently merge. If the TOML
file is open in a dirty editor buffer, Save is blocked with Switch to Editor / Save
that file first / Cancel. If the file fails to parse, the form opens read-only with
the parse error and an Open in TOML action rather than guessing. Duplicate creates a
draft with a generated unique ID (`build-copy`, `build-copy-2`) and label suffix.
Delete Definition is available from the tab and the sidebar, requires confirmation,
lists dependents that would break, and edits only the definition.

**Two-way sync with raw TOML.** Open in TOML opens the file as an ordinary document
(duplicate-tab handling, dirty-close behavior) and, when possible, places the caret
on the task's table. After the TOML document is saved, open task-editor tabs for
that file reload: clean drafts refresh silently, dirty drafts show a banner (Keep my
changes / Take file version / Show diff) instead of being overwritten. Typing an
invalid intermediate TOML buffer does not affect the form until saved. The form and
the raw editor never edit the same bytes simultaneously: while a task draft is dirty
and the file is also dirty, both show the conflict banner.

**Commands and state.** Typed commands: NewTaskEditor(scope), OpenTaskEditor(TaskKey),
SaveTaskEditor(TaskEditorId), SaveAndRunTaskEditor, RevertTaskEditor,
CloseTaskEditor, DuplicateTask, DeleteTaskDefinition, RenameTaskId, OpenTaskToml(
TaskKey). Targets carry `TaskEditorId` plus the config revision they were drawn
from; stale targets and modal-blocked requests do not mutate. Drafts, scope and
validation state live in `app/task_editor.rs` (controller), rendering in
`ui/task_editor.rs`, schema/validation/minimal-edit logic in `crates/task-core`.
Draft contents are not persisted across restarts in the first release (a dirty
close prompt protects work); consider recovery files alongside document recovery
later. Keyboard: Tab order follows visual order, Ctrl/Cmd+S saves the focused task
editor (not the active document), Escape closes popups first and never discards a
dirty draft, and text fields own typing/clipboard/IME input.

**Phasing.** (a) Prerequisite: `VirtualTab` refactor with Settings/Diff behavior and
tests unchanged. (b) Read-only task view tab fed by Phase 1 core (validates tab
plumbing, close prompts and focus). (c) Editable process/shell form, validation,
Save, Save and Run, conflict handling, Open in TOML sync, Duplicate/Delete: ships
with Phase 3. (d) Dependencies/compound UI with Phase 4, inputs and background
sections with Phase 5, problem-matcher section with Phase 6.

**Acceptance.** Tests for: tab open/duplicate/close and dirty prompts by
`TaskEditorId`; a form-to-TOML round trip preserving comments and unknown keys;
validator parity between form and file load; conflict, read-only parse-error and
dirty-TOML-buffer states; stale-target rejection; Settings and document tabs
unaffected; split-editor behavior (virtual tab only in the first group, focusing it
moves focus there). Screenshots at narrow/default/wide widths and light/dark themes,
plus keyboard-only create-edit-save-run.

### Bottom panel and output tabs

Use one bottom-panel host with a stable selected-tab enum, visibility, height,
and stable tab IDs. Example strip:

`TERMINAL | OUTPUT | PROBLEMS | Build | Test | Dev server`

Every task output tab uses the configured task label. Duplicate labels include
scope or a short run suffix, e.g. `Build · #2`; the tooltip includes scope, task
ID, working directory, start time, state, and exit code. IDs use `RunId`, never a
label/index. A compound parent has its own summary tab; child output tabs use
child task labels and link back to the parent.

Reserve command space before laying out tabs. Use truncation and a scrollable
strip/overflow chooser; keep the selected tab reachable at narrow widths. No
wrapping headers, expanding row heights, or actions drifting away from the right
edge. Apply shared colors, the Lumen icon font (`src/icon.rs`; add any new glyph in
`tools/gen_icons.py`), typography, `surface()`/`canvas()`, and
existing corner/gutter rules.

Task tabs show state, elapsed time, rendered output, and a final exit summary.
Actions: Stop (running), Restart, Clear View, Copy, Save Output, and Close. Keep
command ordering and tooltips consistent. ANSI text uses terminal emulation for
PTY runs; captured process output uses a bounded read-only log with selectable
text, stdout/stderr markers, and incremental UTF-8 decoding. Carriage returns,
partial sequences, split multibyte characters, and invalid byte replacement must
have defined behavior. Do not interpret output as clickable arbitrary commands.

Hiding the bottom panel or switching tabs does not terminate work. Closing an
active task tab asks Stop and Close or Cancel; no invisible detached runs in the
first release. Closing completed output removes only its retained session data.
Keep the existing explicit Close Terminal command's shell-termination behavior;
provide a distinct Hide Panel action rather than silently changing that command.
Preserve Output selection clearing and Terminal focus routing across tab changes.

The initial shell remains one TERMINAL tab. Multiple general-purpose terminal
sessions are not required to deliver independent task output tabs.

## Configuration contract

Proposed files:

| Scope | Location | Behavior |
| --- | --- | --- |
| User | `~/.lumen/tasks.toml` (Windows user home equivalent) | Personal tasks available across workspaces |
| Workspace | `<workspace>/.lumen/tasks.toml` | Shareable tasks for the opened folder |

Use `schema_version = 1` and `[[tasks]]`. Identity is `(scope, task.id)`, independent
of label. IDs must be unique within their scope. Workspace and user tasks with
the same ID remain distinct, not silently overlaid. Unqualified dependency IDs
resolve only within the definition's scope; explicit `user:<id>` references can
reuse personal definitions. Do not allow user tasks to depend implicitly on
workspace definitions. Platform overrides change allowed execution fields only,
not identity or graph structure.

Example target syntax (proposed, not currently implemented):

```toml
schema_version = 1

[[tasks]]
id = "build"
label = "Build"
type = "process"
command = "cargo"
args = ["build"]
cwd = "${workspaceFolder}"
group = "build"
is_default = true
transport = "pipe"
save = "prompt"
instance_policy = "single"

[tasks.env]
CARGO_TERM_COLOR = "always"

[tasks.presentation]
reveal = "always"
focus = false
reuse = "dedicated"
clear = true

[[tasks]]
id = "test"
label = "Test"
type = "process"
command = "cargo"
args = ["test", "--workspace"]
cwd = "${workspaceFolder}"
depends_on = ["build"]
depends_order = "sequence"

[[tasks]]
id = "dev"
label = "Dev server"
type = "shell"
command = "npm run dev"
cwd = "${workspaceFolder}"
transport = "pty"
is_background = true

[tasks.background]
ready_pattern = "Local:"
ready_timeout_seconds = 60

[[tasks]]
id = "verify"
label = "Verify"
type = "compound"
depends_on = ["build", "test"]
depends_order = "sequence"
```

### Execution primitives and fields

- **process:** an executable plus an argument array, launched directly. Preserve
  argument boundaries; shell metacharacters are literal. Support pipe output or
  PTY transport. Interactive direct tools can use PTY without a shell wrapper.
- **shell:** an explicit command string interpreted by the chosen shell. Supply
  per-platform shell executable/argument settings; do not concatenate process
  args into a shell string. Use a non-interactive invocation by default, not a
  user's persistent interactive Terminal session. PTY can provide input support.
- **compound:** a dependency graph only; no command, env, shell, or transport.
  Sequential and parallel dependencies run through the same scheduler.

Common options: label/detail/group/default, cwd, env overrides/unsets, dependency
IDs/order, allowed success exit codes (default `[0]`), timeout, instance policy,
background readiness, and presentation. Configure per-platform execution overrides
under `windows`, `macos`, and `linux`; avoid assuming POSIX command syntax works
on Windows. Resolve `cwd` relative to the config's base: workspace root for
workspace definitions, user config directory for user definitions. An explicit
`${workspaceFolder}` is an error if no workspace is open.

Defaults should be explicit in the schema: process transport pipe; shell
transport PTY; reveal always; focus false; dedicated output reuse; clear true;
single instance; prompt before running against unsaved workspace files; no
execution timeout unless configured. Readiness waits always need a finite timeout.
Reject unknown execution keys with an actionable error while preserving them
when editing; do not silently ignore a misspelled safety/lifecycle option.
Only one default task per group/scope is valid; prefer the workspace default,
then the user default. Otherwise show the task picker.

Dedicated output reuse replaces a task's completed tab on rerun; reuse=new
retains each run separately. Neither policy overwrites active output. A busy
single-instance task reveals the current run with explicit Restart available;
parallel instances are opt-in. Restart waits for confirmed termination before
launching another process.

### Variables and inputs

Support a deliberately small typed resolver: `${workspaceFolder}`,
`${workspaceFolderBasename}`, `${file}`, `${fileDirname}`, `${fileBasename}`,
`${env:NAME}`, and declared `${input:ID}`. Capture active-file/workspace context
once per invocation before showing inputs; an untitled/missing file variable is
an actionable error, not an empty string. Unknown variables fail validation.
Do not implement `${command:...}` or recursive command evaluation.

Inputs support text, choice, and masked secret values in a later phase. Resolve
once per parent invocation and share consistently with dependencies. Cancellation
starts nothing. Secret inputs must not be persisted or echoed in previews/logs;
prefer env injection over command-line secrets and disclose shell/process env
exposure limitations. Environment precedence: inherited process environment,
platform-resolved task overrides, then resolved run inputs. Never log the full
inherited environment. Validate env names/NULs and expansion/argument limits.

## Execution lifecycle and scheduling

Use separate `TaskId`, workspace generation, `InvocationId`, and `RunId` types.
A run owns an immutable resolved definition/config revision, context, execution
plan, process handle, start/end times, output buffer, and exit result. Config
edits affect future runs only. Delayed events carry run ID and generation; stale
or duplicate completion cannot mutate another tab or a replacement workspace.

States: resolving inputs → queued → starting → running → stopping → terminal.
Terminal results distinguish success, nonzero exit, spawn failure, cancellation,
timeout, dependency failure, and interrupted shutdown. A process exiting and its
output pipe closing are separate events: continue draining final output before
publishing the final summary, with a bounded drain deadline if inherited handles
remain open. Record actual exit code/signal, not only a Boolean.

Validate the complete dependency DAG before launching anything: missing IDs,
cycles, duplicate IDs, invalid platform fields, unsupported versions, and missing
required inputs. Deduplicate shared nodes within one invocation. Different parent
invocations do not silently share processes. A dependency that also appears
explicitly in a compound runs once, not twice.

Sequential dependencies stop on failure; parallel siblings already running are
stopped when fail-fast is selected (default), or allowed to finish under an
explicit continue policy. Dependent tasks never run after failed prerequisites.
Limit concurrent processes (proposed default 4, configurable 1–32), counting
background processes too. Reject/flag readiness graphs that can consume every
slot with services and leave no slot for dependents; do not hang indefinitely.

Foreground dependencies release successors after successful exit. Background
services release successors only after readiness matches, not after spawn. Mark
unready/ready explicitly; early exit is a failed dependency even with exit code
0. A compound remains active while it owns background children. Stopping a parent
cancels its queued nodes and stops its owned processes; do not kill unrelated
runs. A ready service dying while dependents run marks the invocation failed and
applies its failure policy. Initial background support uses bounded literal-substring readiness matching
(`ready_pattern`), not arbitrary readiness scripts or unbounded regexes. No automatic restarts in the first release.

## Process ownership and output bounds

Service adapters own native processes, PTYs/pipes, readers, stdin writers, and
shutdown. UI components issue commands only. Use Unix process groups and Windows
Job Objects (or a verified equivalent) for descendants. Graceful Stop requests
interrupt/termination, waits off the UI thread, then escalates after a configured
short deadline. Distinguish delivery failure from confirmed stop. A killed shell
does not prove its child processes stopped; native tests are a release gate for
that claim. Deliberately detached descendants may escape ownership; document the
boundary instead of promising universal cleanup.

Do not reuse today's unbounded terminal channel for arbitrary concurrent tasks.
Use bounded buffers/queues and per-frame byte/event budgets. Continue draining OS
pipes even when retained output is full, dropping old display data with a visible
truncation marker instead of deadlocking the child. Suggested starting limits:
4 MiB retained per run, 32 MiB globally, 20 completed runs, and 256 KiB of output
processed per UI frame; tune with measurements. Enforce budgets for emulator
scrollback as well as text logs and transport queues. Never evict active runs to
meet completed-history limits. Every truncation path reports dropped output.

Pipe stdout/stderr order is guaranteed within each stream only; merged arrival
order is not an exact chronology across streams. PTY output combines streams and
cannot preserve that distinction. Hide/show must not pause draining. Disabled
follow-output leaves the user's scroll/selection stable. Save Output exports only
retained decoded data, with truncation status; it does not imply a full raw log.
No persistent raw logs or automatic environment capture by default.

On quit, use the existing unsaved-document flow plus a clear running-task summary
with Stop Tasks and Quit or Cancel. On workspace replacement/close, ask to stop
workspace runs or cancel the transition; do not strand them under another root.
User runs can continue with captured cwd, but are still stopped on application
exit. No restart/resume of external processes after a crash.

## Execution intent, files, and diagnostics

Reading a task file, opening a folder, editing a task, or discovering a manifest
never executes commands. Explicit Run (including its validated dependency plan)
is the authorization boundary. No automatic on-open/on-save tasks in this plan.
For a workspace's first task invocation, show the resolved executable/shell, cwd,
inputs, and dependency list in a compact review; remember trust only for that
workspace and execution-affecting config digest. Changed commands/dependencies
require another review. User tasks started directly remain explicit execution;
workspace references to a user task are included in the reviewed graph. This is
an execution disclosure, not an OS sandbox or permission system for subprocesses.

Tasks can modify/delete files or contact networks with the user's normal OS
permissions. Existing external-change/conflict detection remains authoritative;
do not silently replace dirty editor text after builds or formatters. Save policy
supports prompt, save, or none. Save uses the existing document controller,
handles failures/cancellation before any graph node starts, and never invents
paths for untitled buffers. 'None' explains that tools see on-disk contents.

Later add optional problem matchers with typed path/line/column/severity captures
and bounded regex/multiline state. Resolve relative paths against the run's cwd;
file links must not execute URLs/commands and must handle missing files. Give task
diagnostics their own source/run ownership; rerun/clear removes only that task's
older diagnostics, never language-server diagnostics. Malformed lines remain
visible as output. Task success follows exit/readiness policy, not the absence
of matched errors. Built-in Cargo/compiler matchers must have fixture tests
before advertising support.

## Architecture and module boundaries

| Layer | Proposed responsibilities |
| --- | --- |
| `crates/task-core/` | Pure schema/types, TOML parse/serialization, validation, variable specs, dependency planning, lifecycle rules; no egui, native processes, or filesystem I/O |
| `src/services/tasks.rs` | Task config storage adapter, atomic/conflict-safe writes |
| `src/services/task_process.rs` | Injectable process/PTY service, correlated events, bounded transport, native process-tree ownership |
| `src/app/tasks.rs` | Catalog/scope lifecycle, resolution, save/review/input flow, scheduler, polling, cancellation and result aggregation |
| `src/app/bottom_panel.rs` | Selected tab, hide/show/close commands, per-session view state and host migration |
| `src/ui/tasks.rs` | Sidebar and creation/configuration forms; typed commands only |
| `src/ui/bottom_panel.rs` | One resizable bottom host and dynamic tab strip |
| `src/ui/task_output.rs` | Session output/status/actions; reuse extracted terminal canvas/log primitives |
| `crates/settings-core/` | Small task preference section (parallelism, retention, stop grace), not task-definition storage |

Keep `main.rs` small. Extract only shared terminal-session/rendering primitives
needed by tasks; do not replace editor or agent transports. Preserve the existing
`terminal` resize ID/height and migrate existing focus/scroll IDs deliberately.
Use new per-run IDs for new canvases. Current terminal/output/problems tests
must pass before adding task behavior. Runtime handles belong to controllers or
services; view state owns presentation, not process lifetimes.

Representative commands: CreateTask(scope), SaveTaskDraft, RunTask(TaskId),
RunTaskGroup(group), ConfirmTaskRun(InvocationId), SubmitTaskInputs(InvocationId),
CancelTaskInvocation, StopTask(RunId), RestartTask(RunId), SelectBottomTab(TabId),
ClearTaskOutput(RunId), ExportTaskOutput(RunId), CloseTaskTab(RunId).
Stale targets, modal blocking, failed validation, and cancelled confirmations
must remain non-mutating. No command may target a task by visual row index.

## Delivery phases and gates

### 1. Task core and configuration

**Status (2026-10-17): implemented** in `crates/task-core` (schema, TOML parse with
unknown-key rejection, validation, variable checks, dependency planning with
dedup/ordering/cycle errors, comment-preserving `upsert_task`/`remove_task`) and
`src/services/tasks.rs` (paths, load, conflict-safe save/delete reusing the settings
store guards). Not wired to the UI yet; inputs (`${input:...}`) are rejected until
Phase 5. 
**Task editor tab plumbing (2026-10-17): implemented, not yet reachable.** Task
editor tabs (`app/task_editor.rs`: `TaskEditorId`, `TaskEditorTarget`, open/focus/close
rules) sit in the first group's strip after Settings, with overflow-menu entries,
close buttons and a read-only page (`ui/task_editor.rs`: scope/path, summary, core
validation problems, TOML preview). Commands `NewTaskEditor`, `ActivateTaskEditor`,
`CloseTaskEditor`; `Close Editor` closes the visible task tab. Deliberate deviation
from the plan: rather than a full `VirtualTab` enum, the repeated "leave Settings"
sites now share `ViewState::leave_page_tabs()` / `page_tab_active()`, which keeps the
existing Settings/Diff state untouched. Revisit the enum if a fourth page type
appears. No palette/menu entry yet; the editable form and sidebar make it reachable.

**Editable task form (2026-10-17): implemented for process and shell tasks.**
`app/tasks.rs` (controller) and `ui/task_editor.rs` (form): label/ID (ID follows the
label until edited; fixed for existing tasks), type, executable or command, arguments
(one per line), working directory, environment (`NAME=VALUE` lines), group/default,
save policy and output transport, with core-validator problems beside each field and
the TOML preview. Save (also Ctrl/Cmd+S while the page shows) creates or updates only
that task through the conflict-safe store: new IDs never replace existing ones, an
edited task refuses to overwrite a file that changed on disk (Reload from disk
drops the edits), a dirty open `tasks.toml` buffer blocks the save, and problems in
other tasks do not block this one. Dirty tabs show a dot; closing, Close Editor and
Quit ask Save / Don't Save / Cancel (quit then needs a second attempt). Palette:
Tasks: New Workspace/User Task, Open Workspace/User Tasks File (TOML). Verified
with live screenshots (valid and invalid drafts). Still open: the sidebar list and
context menu (`OpenTaskEditor` has no UI caller yet), Duplicate/Delete/Save and Run,
dependencies/compound/background/platform sections, narrow-width section chooser,
caret placement in Open in TOML, and reload of open task tabs after the TOML
document is saved.

**Run and Debug sidebar (2026-10-17): implemented (listing and definition management).**
`app/task_catalog.rs` (state, filter, copy IDs, dependents), controller refresh in
`app/tasks.rs` (read only while the sidebar is visible; re-read on show, after a save
or delete, and on workspace change), `ui/tasks.rs`. WORKSPACE and USER TASKS sections
show counts, rows with group/default tag, empty, unavailable, invalid-file (Open as
TOML) and no-match states. Click selects; double-click or Enter edits; context menu
has Edit, Duplicate (unsaved copy with a fresh ID) and Delete Definition (confirmation
lists dependents; refused while that task has an editor tab or if the file changed).
Header: New Task, Refresh, Open Tasks File. The Run and Debug placeholder and its
"not available yet" copy are removed. No Run action exists yet: execution is Phase 3,
so the sidebar deliberately offers no Run, Running group or Show Output. Still open
here: Save and Run, Run with Inputs, Rerun Last, keyboard-invoked context menu, and
tab-strip/panel integration for output.


Implement versioned process/shell/compound schemas, scope identities, platform
resolution, dependency validation, store adapters, and conflict-safe config edits.
Establish fixture examples and fake service contracts before native execution.
Gate: validation/error tests and round-trip preservation; opening configs never
launches a process or creates config files unnecessarily.

### 2. Shared bottom-panel host

Consolidate Terminal/Output/Problems presentation into one host, preserving their
existing commands, visibility, focus, selection, scroll positions, resize state,
and terminal shutdown behavior. Add dynamic tabs using synthetic session data.
Gate: existing regression suite plus narrow/default/wide screenshots and keyboard
navigation checks. Compare stable fixtures before/after this refactor.

### 3. First end-to-end release: foreground tasks

**Status (2026-10-17): foreground process/shell runs work end to end; PTY and several
gate items are still open.**
Built: `task-core` `resolve` (variables, platform overrides, cwd rules) and
`execution_digest`; `services/process_tree.rs` (shared isolate/terminate/force-kill,
extracted from the agent runner), `services/task_process.rs` (pipe transport, bounded
output queue with backpressure, exit and pipe-close tracked separately, polite stop
then forced stop, PATH override, fake for tests), `services/task_trust.rs`
(per-workspace, per-digest review memory), `model/task_output.rs` (bounded log: split
UTF-8, invalid bytes, `\r`, escape-sequence stripping, truncation notice),
`app/task_runs.rs` (RunId-keyed runs, per-frame poller with a 256 KiB budget, timeouts,
stop/restart/close, limits: 4 active, 20 finished, 4 MiB per run, 32 MiB total),
`ui/task_output.rs` (task tabs in the bottom slot beside TERMINAL/OUTPUT, scrollable
tab strip, Stop/Restart/Clear/Copy/Close/Hide), `ui/task_dialogs.rs` (first-run
review, unsaved files, close running tab, folder change, quit). Sidebar: RUNNING
section, Run actions, double-click runs. Palette: Run Selected/Rerun Last/Run Build/Run
Test/Show Running Tasks. Verified with real processes (descendant stop, ignored SIGTERM,
12 MiB flood) and live screenshots.
Deviations and open items (do not advertise as shipped):
- **PTY transport (2026-10-17): implemented.** `transport = "pty"`, and the default
  for shell tasks, runs in a pseudo-terminal (`TERM=xterm-256color`, `COLORTERM`),
  rendered through the `terminal-core` emulator with a 2000-line scrollback; the
  canvas takes keyboard input (shared `terminal_canvas` focus id, so global shortcuts
  stay out of the way), follows the panel size, and Copy exports the retained text.
  Stop, exit codes, descendant cleanup and timeouts behave as for pipes (verified with
  real PTY processes and a live screenshot with colour and typed input). Limits:
  stdout and stderr are one stream; stderr cannot be coloured separately; the
  emulator scrollback is capped by lines, not accounted in the global byte budget;
  mouse reporting, alternate-screen apps and bracketed paste are only as good as
  `terminal-core` already is. Windows ConPTY is unverified.
- `presentation.clear = false` is not honored: Dedicated reuse always replaces the
  completed tab. Save Output to a file is not built (Copy is).
- The bottom slot is a shared tab strip, not the full host consolidation of Phase 2;
  Problems remains a separate panel.
- Windows uses `taskkill /T` (no Job Object) and the process tests are unix-only:
  native Windows verification is still required before claiming it.
- Settings for concurrency, output limits and stop grace are constants for now.

Ship sidebar, the task editor tab (guided process/shell create/update,
see above), raw TOML editing, explicit execution
review, workspace/file/env variables, pipe/PTY runs, task-named tabs, exit codes,
Stop/Restart, output limits, and quit/workspace transitions. Include build/test
examples, defaults, and palette commands. No dependency/background/input UI is
advertised yet if its execution contract is not finished.
Gate: real foreground commands on macOS/Windows, descendants stopped correctly,
no hidden-output stalls, cancellation/spawn errors, and no document regressions.

### 4. Compound tasks and concurrency

**Status (2026-10-17): implemented for process/shell tasks; background services and
inputs remain Phase 5.**
`app/task_graph.rs`: a task with `depends_on` (or a compound task) runs as a group.
Before anything starts the whole graph is read from disk, planned (shared nodes run
once, sequence order becomes prerequisites, nested compounds only order) and every
task resolved; one problem aborts the run. The first-run review lists every
unreviewed workspace command and trusts them together; the save policy is the
strictest among the tasks. A scheduler starts tasks as prerequisites succeed within
the 4-process limit (queued otherwise), the group has its own summary tab (progress
log, Stop, Restart) and children keep their own tabs ("Part of ..." tooltip). Failure
policy `on_failure = "fail_fast"` (default) stops running siblings and skips the rest;
`"continue"` lets independent tasks finish and skips only dependents. Stopping or
quitting winds the group down; restarting a child restarts its group. A
single-instance task already running blocks a group that needs it (no silent
sharing). Background tasks as dependencies are refused until Phase 5. Verified with
fake-process tests for ordering, dedup, both failure policies, limits, stop, restart,
review, bad graphs, cross-scope `user:` references and quit, plus a live screenshot.
Not done: the task form has no dependency/compound editing (TOML only); no
per-task `instance_policy = "parallel"` conflict handling beyond what is above; no
settings for the limits.

Enable validated sequential/parallel graphs, shared-node deduplication, run limits,
parent summaries, failure policies, instance handling, and group defaults.
Gate: deterministic fake-clock/process tests for graph ordering and all failure
paths, plus simultaneous native output/exit stress tests.

### 5. Watch/service tasks and inputs

**Status (2026-10-17): implemented.**
Readiness: `is_background` tasks with a `[tasks.background] ready_pattern` are matched
as a bounded literal substring over recent output (pipe text, or a plain-text shadow
of terminal output so colour codes do not break a match) across chunk boundaries;
the wait is always finite (default 60 s). An unready service is stopped and reported
("Not ready within Ns"); exiting before ready fails regardless of exit code. In a
group, a service is Serving once ready (releasing dependents) and keeps the group open
until stopped; a service dying or being stopped while other tasks still need it fails
the group under its policy; stopping a group whose work finished is a normal end.
Graphs whose services would fill every process slot (services >= 4, or no slot left
for foreground tasks) are refused before anything starts. Inputs: top-level
`[[inputs]]` (`text`, `choice`, `secret`) referenced as `${input:ID}`; declared inputs
are validated (undeclared use makes the file invalid) and asked once per run, shared
across the whole group, before anything resolves; cancelling starts nothing. Secret
values are masked in the prompt, redacted (bullets) in the review, tab tooltip and
group log, never persisted, and dropped once used. Disclosed limits: a program can echo
its secrets, and command-line secrets are visible to other local processes (prefer
`env`). Not done: input editing in the form, `${command:...}` (deliberately never).

Add declared text/choice/secret inputs, readiness, bounded matcher state, deadlines,
service lifetime accounting, and dependency-aware parent cancellation.
Gate: no launch on input cancellation, no secret persistence, no readiness slot
starvation, and correct service-death behavior.

### 6. Diagnostics and polish

Add tested problem matchers/file links, task-scoped Problems integration, search,
recent task picker, output export, and accessible state announcements. Add further
package-manager templates only after verifying their command/quoting assumptions.
Update README, screenshots, and task examples with only shipped capabilities.

## Verification and acceptance matrix

- Core/schema: duplicate labels versus IDs, bad versions/types, unknown variables,
  empty command, invalid cwd/env, platform overrides, missing/cyclic dependencies,
  conflicting defaults, invalid shell combinations, and comment-preserving edits.
- Storage: missing config, invalid TOML, permission failure, dirty config buffers,
  concurrent external edits, read-only folders, cancelled form, and atomic writes.
- Execution: argument boundaries with spaces/quotes/Unicode/metacharacters; cwd
  paths with spaces; process versus shell semantics; env inheritance/overrides;
  not-found/permission errors; nonzero/signal exits; final output after exit.
- Lifecycle: duplicate/late events, workspace switch during input/start/stop,
  timeout/exit races, Stop during spawn, restart only after stop, cancelled quit,
  failed process-tree termination, background readiness/early death, DAG dedup,
  fail-fast versus continue, parallel limits and service-slot starvation.
- Output: concurrent floods, huge single lines, partial UTF-8/ANSI, binary bytes,
  stdout/stderr interleave, slow UI, hidden tabs, stable scroll/selection, visible
  truncation, retention eviction, PTY resize/input, and exit-code attribution.
- UI: empty states, same-label runs, long labels, many tabs/overflow, active-tab
  visibility, focus restoration, disabled actions, keyboard-only run/stop, theme
  variants, and right-edge controls at narrow/default/wide widths.
- Safety: no implicit launch, stale execution review after config changes, input
  cancellation, save failure/cancel, dirty-file conflict after a formatter, no
  full-env/secret logging, and no task diagnostics clearing language diagnostics.
- Existing behavior: Terminal shell input/selection/resize, Ctrl+` toggle, Output
  copy/clear/selection cleanup, Problems navigation, editor clipboard/history,
  modal routing, dirty-buffer recovery, and settings draft behavior.

Run `cargo fmt --all --check`, `cargo test --workspace`, and
`cargo clippy --workspace --all-targets -- -D warnings`. Check Windows compilation
with `cargo check --target x86_64-pc-windows-gnu` when available and macOS packaging
with `cargo xtask bundle`. Native PTY/process-group/Job Object, input and shutdown
checks must be reported separately from compilation/headless tests. Exercise a
large output producer while typing in a large editor document; record frame
latency, queue size, retained bytes, and stop latency rather than claiming
responsiveness solely from small unit fixtures.

## Definition of done

A user can create a task from the existing activity, save it, deliberately run it,
see a task-named output tab alongside Terminal, inspect success/failure, stop its
owned processes, and rerun it without losing editor work. After the later phases,
the same contracts support compounds, concurrent jobs, watch services, inputs,
and task diagnostics. No feature is labeled supported before its phase's tests
and native acceptance gates are complete.
