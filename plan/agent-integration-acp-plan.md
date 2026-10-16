# Coding-agent integration via the Agent Client Protocol (ACP)

Status: Phase 0 (spike) complete; Phases 1-6 not started. Updated: 2026-10-16.

**Read [acp-spike/FINDINGS.md](acp-spike/FINDINGS.md) first.** The spike showed
that neither adapter uses the client's `fs/*` or `terminal/*` methods, so the
"edit through the editor" design below is superseded where marked *(revised)*.

This plan adds an in-editor chat/agent panel that drives **Claude Code** and
**OpenAI Codex** through one protocol client. Lumen acts as an ACP *client*;
each agent runs as a child process behind a small adapter that speaks ACP over
stdio. Facts about ACP and the adapters below were read from the public
documentation on 2026-10-16; items I could not confirm are listed under
[Open questions](#open-questions-to-settle-in-the-phase-0-spike) and must be
settled by the Phase 0 spike before the design is frozen.

## Goals and non-goals

### Goals

- One agent panel that works with Claude Code and Codex (and, later, any ACP
  agent) without per-agent UI code.
- Streamed assistant text, plans, tool calls (with diffs and terminal output),
  permission prompts, cancel, and per-session mode/model selection.
- The agent sees what the user sees: unsaved buffer text, not just disk.
- Every agent edit goes through Lumen's existing protections: revision
  validation, dirty-buffer safety, one undo step per file, reviewable diffs.
- Everything the agent does is visible in the Output panel, with timestamps.
- No credentials stored by Lumen. Each agent authenticates the way its own CLI
  does (subscription login or the user's own environment variables).

### Non-goals (first release)

- Bundling or silently installing agents, Node.js, or the Codex/Claude CLIs.
- Running agents against a folder the user has not opened or trusted.
- Implementing every ACP extension (subagent transcripts, draft v2, HTTP
  transport, elicitation, audio).
- An agent marketplace or arbitrary workspace-supplied agent commands.
- Replacing the terminal: users can still run `claude` / `codex` there.

## Why ACP, and how it relates to the other options

| Option | Fit |
| --- | --- |
| Run the CLI in the terminal tab | Already works; no editor awareness. Keep it. |
| Claude Code's IDE protocol | Claude-only, internal protocol; good later add-on for diagnostics/selection. |
| **ACP client (this plan)** | One protocol, two (and more) agents, structured events the UI can render, editor-owned file access and permissions. |
| Headless `claude -p` / `codex exec` | One-shot only; good for later "Fix with agent" shortcuts. |

## ACP facts this design depends on

Source: [agentclientprotocol.com](https://agentclientprotocol.com/protocol/overview).

- **Transport (stdio):** UTF-8 JSON-RPC 2.0, one message per line (`\n`), *no
  embedded newlines*. Agent writes only valid messages to stdout; stderr is free
  text logging. Shutdown: close stdin, then terminate the process.
- **Lifecycle:** `initialize` (client sends `protocolVersion: 1`,
  `clientCapabilities`, `clientInfo`) -> optional `authenticate` ->
  `session/new {cwd, mcpServers}` (or `session/load` when `loadSession`) ->
  `session/prompt {sessionId, prompt: ContentBlock[]}` -> `session/update`
  notifications -> prompt response with a stop reason (`end_turn`, `max_tokens`,
  `max_turn_requests`, `refusal`, `cancelled`). `session/cancel` is a
  notification.
- **Agent -> client requests** (we must answer): `session/request_permission`,
  `fs/read_text_file`, `fs/write_text_file`, `terminal/create`, `terminal/output`,
  `terminal/wait_for_exit`, `terminal/kill`, `terminal/release`
  (fs/terminal gated by the capabilities we advertise).
- **`session/update` kinds:** `agent_message_chunk`, `user_message_chunk`,
  `agent_thought_chunk`, `tool_call`, `tool_call_update`, `plan`,
  `available_commands_update`, `config_option_update` (plus mode changes and
  notices).
- **Tool calls:** `toolCallId`, `title`, `kind` (`read|edit|delete|move|search|
  execute|think|fetch|switch_mode|other`), `status` (`pending|in_progress|
  completed|failed`), `content` (`content` | `diff {path, oldText?, newText}` |
  `terminal {terminalId}`), `locations [{path, line?}]`, `rawInput/rawOutput`.
- **Permissions:** request carries `toolCall` and `options [{optionId, name,
  kind: allow_once|allow_always|reject_once|reject_always}]`; response is
  `{outcome: "selected", optionId}` or `{outcome: "cancelled"}`. On
  `session/cancel` the client **must** answer every pending request `cancelled`.
- **Paths are absolute** (not URIs); line numbers are 1-based.
- **Config options** (`configOptions` with `category` mode/model/thought_level,
  `session/set_config_option`) supersede the older `modes`; support both.
- **Slash commands** arrive via `available_commands_update` and are invoked by
  sending `/name args` as ordinary prompt text.

## The two adapters

Neither agent speaks ACP natively; each is launched through an adapter.

| | Claude Code | Codex |
| --- | --- | --- |
| Adapter | `@agentclientprotocol/claude-agent-acp` (wraps the Claude Agent SDK; replaces the older `@zed-industries/claude-code-acp`) | `@agentclientprotocol/codex-acp` (built on the Codex App Server; `zed-industries/codex-acp` is archived) |
| Launch | `npx` of the package, or a globally installed binary | `npx -y @agentclientprotocol/codex-acp`, global `codex-acp`, or a release binary |
| Needs | Node.js | Node.js for the npm route (the package pins a compatible `@openai/codex`; `CODEX_PATH` overrides) |
| Auth | Claude subscription login or API key - **verify** exact methods in Phase 0 | Advertises auth methods: ChatGPT login, `CODEX_API_KEY` / `OPENAI_API_KEY`, optional custom gateway |
| Notable env | `CLAUDE_AGENT_ACP_EXPERIMENTAL_V2` (leave unset: v1 only) | `CODEX_PATH`, `INITIAL_AGENT_MODE` (`read-only`, `workspace-write`, `agent`, `agent-full-access`), `NO_BROWSER`, `APP_SERVER_LOGS` |
| Features seen | permissions, diffs, TODO lists, images, @-mentions, slash commands, MCP, terminals | permissions, diffs, TODO lists, images, @-mentions, slash commands (`/review`, `/init`, `/compact`, ...), MCP |

Treat both as black boxes that implement ACP v1; the descriptor layer (below)
absorbs package renames and flag differences.

## Architecture

Follows `AGENTS.md`: pure rules in a crate, process I/O in `services/`,
orchestration in `app/`, rendering in `ui/`.

```text
settings-core (agent.* schema)        crates/agent-protocol (pure)
        |                                  message types, framing, reducer
        v                                          |
 app/agent.rs  <-- typed events --  services/agent_process.rs (child + stdio)
  sessions, permissions, fs/edit policy              ^
        |                                            | JSON lines
        v                                            v
 ui/agent_panel.rs  <-- snapshots              adapter -> Claude / Codex
```

### New crate: `crates/agent-protocol`

No egui, no process, no filesystem. Owns:

- **Framing:** newline-delimited JSON with a bounded message size (16 MiB: file
  reads can be large), reject embedded-newline output and non-JSON lines
  (reported as protocol errors, not panics).
- **Typed messages** for the subset we implement (serde on top of
  `serde_json::Value`, tolerant of unknown fields and unknown `sessionUpdate`
  kinds - unknown kinds are logged and ignored, never fatal).
- **JSON-RPC bookkeeping:** outgoing request ids (numeric), pending-request map
  with deadlines; incoming requests carry their id so the app can answer later
  (permissions are answered minutes later, from the UI).
- **Session reducer:** a pure `apply(update) -> ConversationDelta` that builds
  the conversation model: message chunks coalesced per message, tool calls
  merged by `toolCallId` (only changed fields in updates), plan replaced
  wholesale, config options/commands replaced.
- **Capability/auth parsing:** `agentCapabilities`, `authMethods`, version
  negotiation (close the session with a clear message if the agent answers a
  version we do not support).

### `services/agent_process.rs`

Mirrors `services/language_server.rs`: spawn, stdin writer, stdout reader
thread with a channel and UI wake-up, bounded stderr ring forwarded to the
Output log, graceful shutdown (close stdin, wait briefly, kill process tree),
fake-able behind a trait (`AgentLauncher`) so app tests never spawn anything.

Launch resolution (the hard part on desktop - see Platform notes):

1. A user-scope setting path/command wins.
2. Otherwise the descriptor's candidates (`codex-acp`, `claude-agent-acp`
   binaries on `PATH`), then the `npx` route **only after explicit consent**.
3. Never read launch commands from project files.

### `app/agent.rs`

Owns one `AgentSession` per (agent, workspace root): connection state
(`NotStarted -> Starting -> NeedsAuth -> Ready -> Busy -> Failed`), the
conversation model, pending permission requests, per-session file snapshots,
and the typed commands the UI issues (`NewSession`, `SendPrompt`, `Cancel`,
`AnswerPermission`, `SetConfigOption`, `RunSlashCommand`, `StopAgent`).
Polls process events each frame (non-blocking), exactly like
`poll_language_server`. Restarts use the same bounded backoff and visible
status; a crash never discards the visible conversation.

### `ui/agent_panel.rs`

Renders snapshots and emits commands; never touches the process or files. See
[UI](#ui).

## Capabilities we advertise

- `terminal = false`. Measured: no adapter used `terminal/*` even when it was
  advertised; agents run commands inside their own tools.
- `fs.readTextFile` / `fs.writeTextFile`: implement the handlers (cheap, buffer
  aware) and advertise them, but expect no calls from the current adapters (see
  [findings](acp-spike/FINDINGS.md)).
- `promptCapabilities` are the *agent's*; we only send content types it
  advertised (`embeddedContext` for file/selection resources, `image` for pasted
  images when present).
- `clientInfo { name: "lumen", title: "Lumen", version }`.

## File access and edits (the core safety design) *(revised after Phase 0)*

**Measured reality:** with `fs.readTextFile`/`fs.writeTextFile` advertised, neither
adapter ever called them. Claude writes with its own Write tool after a
`session/request_permission` that carries a `diff`; Codex edits through shell
commands and sends no diff at all. Lumen therefore *reviews and guards* edits; it
cannot intercept them. The subsections after this block describe the `fs/*`
handlers (kept as a low-priority, cheap implementation for future adapter
versions); the live design is:

1. **Claude - review at permission time.** Show the permission card with the
   `diff` content. Allow lets the agent write to disk. If the target file is an
   open **dirty** buffer, answer `reject_once` with an explanation instead (the
   agent would overwrite unsaved edits). After the write, clean open buffers
   reload; the existing "changed on disk" flow handles the rest.
2. **Codex - review after the turn.** No pre-write information exists. Snapshot
   before the turn (open buffers + a bounded hash/mtime index, or git when in a
   repository), then show "Files changed by the agent" with diffs, Open diff, and
   Revert per file. Expose Codex's `read-only` / `workspace-write` modes so users
   choose "ask first" (command/path prompts, no diff) or "edit freely, review
   after".
3. **Unsaved text is pushed, not pulled.** Warn when unsaved buffers exist
   ("agents read from disk") and offer *Save all and send* or *Attach unsaved
   buffers as context* (`resource` blocks; both agents advertise
   `embeddedContext`).

### Original `fs/*` design (retained for future adapters)

### `fs/read_text_file`

- Path must be absolute and inside the workspace root, or in a set of paths the
  user approved for this session. Resolve symlinks first (reuse the guarded
  workspace path checks used by `write_workspace_replace`); refuse escapes.
- If the file is open in a tab, return the **buffer text** (honoring `line` /
  `limit`), otherwise read from disk through the `FileSystem` service. Apply a
  size cap and report truncation explicitly.
- Record a **read snapshot** per file for the session: `(path, revision or
  content hash)`.

### `fs/write_text_file`

Whole-file write. Policy, strictest first:

1. **Workspace gate:** same path rules as reads; creating a file is allowed only
   inside the workspace; refuse protected locations (`.git/`, the settings
   files, anything outside).
2. **Conflict check:** if the file changed since the agent's read snapshot
   (open buffer revision differs, or disk content differs), reject the write
   with a JSON-RPC error telling the agent to re-read. Never overwrite blindly.
3. **Review mode** (user setting `agent.edit_mode`):
   - `ask` (default): show the diff in the permission/edit card and wait.
   - `review`: apply nothing until the turn ends, then present all edits through
     the existing before/after review dialog (`workspace_edit` plan) with
     Apply / Discard.
   - `auto` (opt-in, per workspace, off by default): apply immediately.
4. **Apply:** open tab -> `apply_to_buffer` (one undo step, buffer stays
   unsaved - the user decides when to save); closed file ->
   `write_workspace_replace(root, path, expected_base, text)`; new file ->
   atomic create-new, never truncate.
5. Dirty buffers are never overwritten by disk writes; if a closed-file write
   races with the user opening it, re-validate and fail safe.

### Permissions (`session/request_permission`)

- Render the agent's options verbatim (`allow_once`, `allow_always`, ...). The
  card shows the tool title, kind icon, locations, and (for edits) the diff from
  the tool call content.
- `allow_always` is remembered **per session and workspace in memory only** in
  the first release, never persisted.
- Pending requests answer `cancelled` automatically on Cancel, on panel close,
  on session restart, and on agent exit.
- Lumen may auto-answer only under explicit user settings (e.g. "always allow
  read-kind tool calls inside the workspace"); the default is to ask.

## UI

A **right-hand secondary panel** (not the bottom panel: chat needs height).
Toggled from a new button in the activity bar's bottom group and a shortcut
(Cmd/Ctrl+Shift+A). Resizable with a persisted width, sane minimum, content
responsive to narrow widths. Reuse `panel_header` (agent picker on the left via
tabs, actions right: new session, clear view, stop, close), `theme.rs` colors,
Codicons; compact VS Code-style rows, no oversized cards.

Components:

- **Header:** tabs per configured agent (Claude, Codex) with connection dot
  (grey/green/amber/red); actions with tooltips.
- **Transcript:** user messages, assistant messages (selectable, copyable;
  lightweight markdown: paragraphs, inline code, fenced code with copy buttons,
  lists, links), collapsible "thinking", system notices.
- **Tool call rows:** icon by `kind`, title, status spinner/check/cross,
  expandable detail (diff summary with "Open diff", terminal output, raw
  input/output behind a disclosure). Locations are clickable (open file at line).
- **Plan:** pinned checklist from `plan` updates.
- **Permission card:** inline, keyboard operable (arrows + Enter), never modal so
  the user can still read the editor; one at a time, queued.
- **Composer:** multiline input; Enter sends, Shift+Enter newline; `@` opens a
  file picker (workspace index) adding `resource` blocks; `/` shows the agent's
  commands from `available_commands_update`; Stop button replaces Send while a
  turn runs; mode/model/effort dropdowns generated from `configOptions`.
- **States:** "Agent not found" with the exact install command and a Copy button,
  "Sign in required" with the auth action (or instructions), connection error with
  Retry and "Show output".
- **Accessibility:** focus order, text selection and copy everywhere,
  screen-reader labels on status icons.

Editor entry points (later phases): editor/explorer context menu "Ask agent
about selection / file", Problems row "Fix with agent", palette commands.

## Settings (user scope; project may only *restrict*)

New `Agents` section in `settings-core` (schema/validation only):

- `agent.enabled` (master switch, default on but nothing starts until used).
- `agent.claude.command`, `agent.codex.command` - absolute path to an adapter
  binary (user scope only; project values ignored, same rule as language
  servers, with the same "ignored" note in Settings).
- `agent.claude.launch` / `agent.codex.launch`: `auto | binary | npx` - `npx`
  needs the consent dialog the first time (shows the exact command and that it
  downloads from the npm registry).
- `agent.edit_mode`: `ask | review | auto`.
- `agent.allow_read_in_workspace`: auto-answer read-only tool permissions.
- `agent.trace`: log raw protocol traffic to Output (redacted, off by default).
- `agent.codex.initial_mode` (maps to `INITIAL_AGENT_MODE`).

Credentials are never settings. API keys come from the user's environment; the
Settings page links to each agent's login instructions.

## Platform notes

- **GUI apps do not inherit the shell `PATH`.** Launched from the Dock/Start
  menu, `node`, `npx`, `codex`, `claude` are usually invisible. Resolve the
  agent's environment by running the user's login shell once
  (`$SHELL -lic 'printf %s "$PATH"'` on macOS/Linux; registry/`where` on
  Windows) with a timeout, cache it, and show the result in the agent status
  details. This also helps the integrated terminal.
- **Windows:** `npx` is `npx.cmd` (needs `cmd /c` or explicit `.cmd`
  resolution); kill the whole process tree on stop; paths are drive-letter
  absolute (ACP uses paths, not URIs - no `file://` conversion); ConPTY is not
  involved for `terminal = false`.
- **Unsigned binaries on macOS:** release binaries of adapters may be
  quarantined; document the `xattr` fix rather than doing it silently.
- **Process hygiene:** ensure children die with Lumen (job object on Windows,
  process-group kill on Unix), including on panic.
- **Network:** all model traffic is the agent's; Lumen opens no sockets.

## Logging and diagnostics

- Lifecycle, auth, errors, stop reasons, permission decisions, and every file
  read/write the agent requested go to the Output panel with timestamps and
  source `agent:<id>`.
- Agent stderr is forwarded line by line at Info/Warn.
- `agent.trace` adds redacted raw JSON-RPC (file contents elided).
- Crashes surface a banner in the panel plus an Error log entry with the last
  stderr lines.

## Testing strategy

- **Pure crate tests:** framing (fragments, oversize, embedded newline, invalid
  JSON), message decoding with unknown/extra fields, reducer fixtures (chunk
  coalescing, tool-call merge, plan replacement, cancel races), capability and
  version negotiation.
- **Fake agent:** an in-process `AgentLauncher` test double that scripts a
  conversation (initialize, auth needed, prompt with streamed chunks, a tool
  call with a diff, a permission request, a write, cancel). Drives the app layer
  without processes.
- **Golden transcripts:** record real sessions from both adapters (with
  secrets scrubbed) and replay them through the reducer in CI.
- **Edit-safety tests:** stale snapshot rejected, dirty buffer preserved, outside
  workspace refused, symlink escape refused, undo restores in one step,
  review-mode apply/discard, cancel answers pending permissions.
- **UI tests** at narrow/default/wide widths (render, keyboard flow for
  permission cards and composer, panel resize persistence).
- **Ignored live tests**, gated by env vars, against the real adapters (like the
  existing rust-analyzer/gopls tests); never required in CI.
- Standard gate from `AGENTS.md` (`fmt`, `test --workspace`, `clippy -D
  warnings`, legacy `--no-default-features` build, Windows compile check where
  the toolchain exists) after every phase.

## Phases

### Phase 0 - spike and protocol capture (no UI) - done 2026-10-16

- [x] Installed both adapters locally (`npm install`, scratch directory) and
  wrote a throwaway Node client ([acp-spike/spike.mjs](acp-spike/spike.mjs)) that
  runs `initialize`, `session/new`, prompts, cancel, `session/list`, and records
  every line.
- [x] Settled most [open questions](#open-questions-to-settle-in-the-phase-0-spike)
  (answers in [acp-spike/FINDINGS.md](acp-spike/FINDINGS.md)); sanitized
  transcripts are in `acp-spike/transcripts/` and move to
  `crates/agent-protocol/fixtures/` in Phase 1.
- [ ] Still open, carried into Phase 1/2: GUI-launched `PATH` behavior (Dock/Start
  menu) and Windows, `authenticate` flows, `session/set_config_option`,
  `session/load`, cancel with a pending permission or long shell command, Codex
  `workspace-write` permission behavior.

Exit met: both agents completed prompt turns against the spike; fixtures captured.
Result: the file-edit design was revised (see above).

### Phase 1 - `agent-protocol` crate and process service - done 2026-10-16

- [x] `crates/agent-protocol`: newline-delimited framing with a 16 MiB bound
  (fragment/oversize/bad-line tolerant), JSON-RPC envelopes and request tracker
  with injected-time deadlines, typed (forgiving) decoding of initialize, sessions,
  config options, tool calls, permissions, plans and commands, request builders,
  version/auth-required handshake helpers, and a conversation reducer.
- [x] Fixture replay tests over the ten sanitized Phase 0 transcripts
  (`crates/agent-protocol/fixtures/`): envelope validity, typed updates, any-size
  chunking, Claude diff-at-permission vs Codex shell edits, cancel, sign-in
  errors, workspace-filtered session lists.
- [x] `services/agent_process.rs`: spawn with piped stdio, stdout/stderr reader
  threads with UI wake-up, stdin-close then process-tree kill (process group on
  Unix, `taskkill /T` on Windows), `AgentLauncher`/`AgentConnection` traits and a
  `FakeAgentLauncher` test double.
- [x] Environment helpers: `resolve_program` (PATHEXT-aware), `merge_path`, and
  login-shell `PATH` resolution with markers, timeout and caching.
- [x] Live (ignored) turns with both real adapters via `LUMEN_ACP_CLAUDE` /
  `LUMEN_ACP_CODEX` (see `live_turn`); both passed on macOS.
- [ ] Not verified: the Windows-specific code in `agent_process.rs`
  (`creation_flags`, `taskkill`) was not compiled locally (no MinGW toolchain);
  CI's Windows job is the first check. `npx.cmd` launching is untested.

Exit met: headless tests drive the fake agent, and live tests complete a turn with
each real adapter.

### Phase 2 - sessions, conversation and a minimal panel - done 2026-10-16

- [x] `app/agent.rs`: per-agent connection state machine (idle, starting, needs
  sign-in, ready, not found, failed), initialize -> session/new -> prompt flow,
  queued first message, cancel, new session, retry after sign-in, Codex
  `authenticate`, folder-change restarts, handshake timeouts, exit/crash handling,
  every lifecycle event and agent stderr line in the Output log. Dropping the app
  stops the agents.
- [x] Preview posture enforced: permission requests are declined (reject option, or
  `cancelled`), unsupported client methods get `-32601`, Codex starts in
  `read-only` (env var plus `session/set_config_option`).
- [x] Launch resolution: user-scope adapter path (project files ignored), installed
  binary, then `npx` only after consent (pinned to the tested adapter versions);
  login-shell `PATH`.
- [x] Right-hand panel (`ui/agent_panel.rs`): resizable (280-760 px), agent tabs,
  status strip with Retry / sign-in methods / "Download with npx..." / copy
  install command, markdown-lite transcript (code blocks with copy, lists, inline
  code), tool-call rows, plan, thinking, notices, composer (Enter sends,
  Shift+Enter newline), Send/Stop. Activity-bar button, Cmd/Ctrl+Shift+A, palette
  command.
- [x] Settings schema `agent.*` (enabled, per-agent command path and launch
  method).
- [x] Live (ignored) controller turns with both real adapters pass on macOS
  (`LUMEN_ACP_CLAUDE` / `LUMEN_ACP_CODEX`).
- [ ] Not done: a Settings status block for agents (resolved command, state, last
  error) and the `agent.edit_mode` family (Phase 3); visual inspection of the
  panel and the new Codicon glyphs (hubot, send, stop-circle); Windows
  verification; virtualized transcript rows for very long conversations.

Exit met: chat works end to end with Claude Code and Codex, read-only.

### Phase 3 - permissions and edit review *(revised)* - done 2026-10-16

- [x] Permission cards: oldest request shown with the rest counted, tool kind,
  locations, command, and a unified diff (Claude sends it before the write); options
  are the agent's own; Esc declines; cancelling a turn answers every open request as
  `cancelled`; a crash drops them; an attention badge on the activity button.
  Standing approvals ("allow always") are the adapter's to remember - Lumen keeps no
  copy, and never offers them for paths outside the open folder.
- [x] Dirty-buffer guard: a write to a file that is an open buffer with unsaved edits
  is declined automatically with an explanation. Optional `agent.allow_read_in_workspace`
  auto-allows plain reads inside the folder.
- [x] Pre-turn snapshot and **"Changed by the agent"** review: incremental, bounded
  scans of the folder (`services/workspace_snapshot.rs`, honors .gitignore, skips
  `.git`, racy-mtime safe) before and after each turn; review card with per-file
  diff, Open, Revert and Revert all, Keep. Reverts are guarded (file must still be
  exactly what the agent left; unsaved tabs block; added files go to the Trash;
  deleted files are recreated only if absent). Open clean tabs follow the disk.
  Setting `agent.track_changes`.
- [x] Unsaved-buffer handling (`agent.unsaved_files`: ask / save / attach / ignore):
  the message is held with a card offering *Save all & send*, *Attach & send*
  (resource blocks, size-capped, only when the agent takes embedded context),
  *Send anyway* or *Cancel*.
- [x] Tool-call rows show the same inline diff.
- [x] Live (ignored) full loop with both real agents: they ask permission, the test
  approves, the review lists the new file, and revert trashes it.
- [ ] Still open: `fs/read_text_file` / `fs/write_text_file` handlers (no adapter calls
  them), "Open diff" in Lumen's side-by-side git diff viewer (inline diffs only),
  Codex `workspace-write` mode handling, the preview posture (Codex still starts in
  `read-only`; relaxing it needs a mode selector - Phase 4), visual inspection of
  all cards, Windows verification.

Exit met for the revised design: Claude edits are reviewed before they happen and
Codex edits after; no unsaved buffer is overwritten; guarded reverts.

### Phase 4 - editor integration - done 2026-10-16

- [x] Ask about the selection: editor context menu "Ask Agent About Selection" and a
  palette command attach the selection as a `resource` block (uri with `#Lx-Ly`
  anchor, 100 KiB cap), or the whole file as a `resource_link` when nothing is
  selected. Explorer context menu "Add to Agent Chat" attaches a file. Attachments
  are chips in the composer, removable, sent with the next message and listed in the
  transcript as "Context: ...".
- [x] `@` mentions: typing `@` lists matching files from the folder index (name
  prefix, name, path, then subsequence); Tab/Enter/click completes. `@path` words that
  name real files inside the folder are also sent as `resource_link` blocks.
- [x] Slash commands: the agent's `available_commands_update` drives a filtered list
  while the draft is `/partial`; Tab/Enter completes, then the full text is sent as
  an ordinary prompt.
- [x] "Fix with agent" on a Problems row prefills the request (file, line, message)
  and attaches the file; nothing is sent until the user does.
- [x] Session options: mode / model / effort dropdowns from `configOptions`, applied
  with `session/set_config_option`; `bypassPermissions` and `agent-full-access` ask
  for confirmation first.
- [x] Optional "follow the agent" (`agent.follow_locations`, off by default): opens the
  file of each tool call at its line, once per call, only inside the folder.
- [ ] Not done: Codex still starts in `read-only` (the dropdown can change it), no
  "Open diff" in the side-by-side viewer, slash-command argument hints, visual
  inspection of the new popups and chips.

### Phase 5 - sessions and persistence

- [ ] `session/load` when advertised: history list per workspace, replay
  rendering, resume across restarts. Persist session ids only (never transcript
  secrets) unless the agent owns the history.
- [ ] Draft preservation per session; clear/new session actions.

### Phase 6 - terminal capability and polish

- [ ] Implement `terminal/*` with background processes, output byte limits,
  kill/release semantics, and "show in Terminal" for long commands.
- [ ] Optional auto-answer policies in Settings; sandbox/approval mode mapping
  for Codex (`INITIAL_AGENT_MODE`).
- [ ] Native acceptance on macOS and Windows with both agents, screen-reader
  pass, large-transcript performance (virtualized rows).

## Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| ACP or adapter packages rename/change (already happened twice) | Descriptor layer, version negotiation, fixtures from real transcripts, pinning the adapter version in the `npx` command |
| GUI launch has no `node`/`npx`/agent on `PATH` | Login-shell environment resolution; clear "not found" UI with exact commands |
| Agents edit files behind Lumen's back through their own tools | Detect disk changes (existing "changed on disk" flow), surface in Output, optional Codex `read-only`/`workspace-write` modes; document that ACP `fs` only covers what the agent routes through the client |
| Whole-file `fs/write_text_file` loses concurrent user edits | Read snapshots + conflict rejection; buffer revision checks; dirty buffers never overwritten |
| `npx` downloads code from the network | Explicit consent dialog with the exact command; pin versions; no auto-update |
| Large transcripts/diffs stall the UI | Virtualized transcript rows, bounded stored output per tool call, capped diff rendering |
| Permission prompt blocks the agent while the user is away | Visible badge on the panel/activity button; timeouts are the agent's concern, cancel always works |
| Secrets in prompts/logs | No credentials in settings; redact trace; elide file contents in trace by default |

## Open questions to settle in the Phase 0 spike (answered unless noted)

1. Exact `authMethods` ids and flows for both adapters, and what the client does
   for each (browser login for ChatGPT, subscription login vs `ANTHROPIC_API_KEY`
   for Claude). Is a terminal login ever required (`/login`, `codex login`)?
2. Minimum Node.js version for each npm adapter, and whether prebuilt binaries
   exist for the Claude adapter.
3. Do both adapters actually route edits through `fs/write_text_file` when the
   client advertises it, or do they write directly with their own tools (in
   which case diffs arrive as tool-call content and the "edit through the
   editor" guarantee needs the permission/review path instead)?
4. Do they honor `fs/read_text_file` for unsaved buffer text, and what `line` /
   `limit` semantics do they send?
5. Whether `session/list` or an equivalent exists in v1 for resuming history, or
   only `session/load` with a known id.
6. Which `configOptions` categories each adapter exposes (mode, model, effort),
   and how permission modes map to Lumen's `edit_mode`.
7. Behavior on cancel (do background commands keep running, as the Claude
   adapter notes?) and what Lumen must do on window close.
8. Stable ACP version number for clients today (docs show `1`; confirm no
   mandatory v2 fields for these adapters).

## Verification

Per phase: `cargo fmt --all --check`, `cargo test --workspace --locked`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --no-default-features --locked`, Windows compile check,
`cargo xtask bundle` for packaging. Visual inspection of the panel at narrow,
default and wide widths; live adapter tests run manually with credentials.

## References

- [ACP overview](https://agentclientprotocol.com/protocol/overview),
  [initialization](https://agentclientprotocol.com/protocol/initialization),
  [session setup](https://agentclientprotocol.com/protocol/session-setup),
  [prompt turn](https://agentclientprotocol.com/protocol/prompt-turn),
  [tool calls](https://agentclientprotocol.com/protocol/tool-calls),
  [terminals](https://agentclientprotocol.com/protocol/terminals),
  [session config options](https://agentclientprotocol.com/protocol/session-config-options),
  [slash commands](https://agentclientprotocol.com/protocol/slash-commands),
  [agent plan](https://agentclientprotocol.com/protocol/agent-plan),
  [transports](https://agentclientprotocol.com/protocol/transports).
- Claude Code adapter:
  [agentclientprotocol/claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp).
- Codex adapter:
  [agentclientprotocol/codex-acp](https://github.com/agentclientprotocol/codex-acp)
  (the earlier [zed-industries/codex-acp](https://github.com/zed-industries/codex-acp)
  is archived).

## Phase 0 answers (summary; details and transcripts in acp-spike/)

1. Auth: Claude has no `authMethods`; logged out, `session/prompt` fails with
   `-32000 Authentication required`. Codex lists `api-key` and `chat-gpt`; logged
   out, `session/new` fails with the same error. `authenticate` flows themselves
   not exercised. Both leak the account e-mail in `_auth/status_update`.
2. Node >= 22 for Claude (Codex declares none); ~650 MB per install because each
   bundles native binaries and does not reuse the user's CLI.
3. Neither adapter calls `fs/*` or `terminal/*`, even when advertised.
4. Unsaved buffers are only reachable as prompt `resource` blocks.
5. Claude: permission request with `diff` precedes the agent's own write. Codex:
   shell command edits, permission requests without diffs (read-only mode only).
6. Config options: Claude `mode`/`model`; Codex `mode`/`collaboration_mode`/
   `model`/`reasoning_effort`/`fast-mode`; legacy `modes` also sent.
7. Cancel returns `stopReason: cancelled` in ~20 ms for both.
8. `session/list` and resume are supported by both; Claude lists all of the user's
   sessions across directories - filter by workspace.
9. `protocolVersion: 1` accepted by both.
