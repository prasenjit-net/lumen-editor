# Phase 0 findings (2026-10-16)

Environment: macOS arm64, Node 26.5. Adapters installed locally with
`npm install @agentclientprotocol/claude-agent-acp @agentclientprotocol/codex-acp`
(claude-agent-acp 0.92.0 with Claude Agent SDK 0.3.300; codex-acp 2.2.2 with a
bundled `@openai/codex` ^0.160.1). Both were driven with the spike client in this
folder against a scratch workspace, using the machine's existing logins. Every
statement below is backed by a transcript in `transcripts/`.

## Answers to the plan's open questions

1. **Authentication.**
   - Claude: `authMethods` is empty. It uses the credentials Claude Code already
     stored for the user. With an empty home directory, `initialize` and
     `session/new` still succeed and **`session/prompt` fails with JSON-RPC error
     `-32000 "Authentication required"`**. There is no in-protocol login: the UI
     must detect this error and tell the user to sign in with the Claude Code CLI
     (or set `ANTHROPIC_API_KEY`). (`claude-loggedout`)
   - Codex: `authMethods` lists `api-key` and `chat-gpt` (`chat-gpt` is hidden when
     `NO_BROWSER=1`). Logged out, **`session/new` fails with `-32000
     "Authentication required"`**, and the client is expected to call
     `authenticate {methodId}`. The `authenticate` call itself (browser login,
     API key from `CODEX_API_KEY`/`OPENAI_API_KEY`) was not exercised: it opens a
     browser or needs a key. (`codex-loggedout`, `codex-init`)
   - Both emit a custom `_auth/status_update` notification with the account label
     and **e-mail address**; treat it as personal data (show a label only, never
     log the address).
2. **Runtime requirements.** claude-agent-acp declares `node >= 22`; codex-acp
   declares none (worked on Node 26). `npm install` of both is ~650 MB because
   each ships native Claude/Codex binaries - the adapters do **not** reuse the
   user's installed `claude`/`codex` versions. Cold `initialize` was 0.15-2.9 s
   and `session/new` 0.5-2.7 s.
3. **Do the adapters use the client's `fs/*`?** **No.** With `fs.readTextFile` and
   `fs.writeTextFile` advertised as true, neither adapter ever sent `fs/read_text_file`
   or `fs/write_text_file` in any run. Claude used its built-in Read/Write tools on
   disk; Codex used shell commands (`exec_command`). The same held with
   `terminal: true`: no `terminal/*` request was ever sent (Claude ran Bash inside
   the adapter; Codex reported output through `_meta.terminal_output_delta`).
   (`claude-edit`, `codex-edit`, `claude-terminal`, `codex-readonly`)
4. **Unsaved buffers.** Because agents read from disk themselves, they cannot see
   unsaved editor text through `fs/*`. The only way to give them buffer text is as
   prompt content: both advertise `promptCapabilities.embeddedContext` and `image`.
5. **What an edit looks like.**
   - Claude (default "Manual" mode): the `tool_call` for Write carries a **`diff`
     content item** (`oldText: null` for a new file, `newText`) and a
     `session/request_permission` arrives *before* the write with options
     `allow-once` (allow_once), `allow-with-updates` (allow_always), `reject`
     (reject_once). If allowed, **the agent writes the file itself**.
   - Codex (default mode `agent`): no permission request inside the workspace and
     no diff; the edit is a shell command. In `read-only` mode it sends two
     permission requests with **no diff**: "Edit files" (locations only) and "Run
     command" (`rawInput.command`), options `allow_once` / `allow_for_session` or
     `accept_execpolicy_amendment` (allow_always) / `cancel` (reject_once).
   - Consequence: Lumen cannot intercept writes through ACP with these adapters.
6. **Config options.** Both return `configOptions` plus the legacy `modes`.
   - Claude: `mode` (default=Manual, acceptEdits, plan, auto, bypassPermissions),
     `model` (default, opus, sonnet, fable, haiku, ...).
   - Codex: `mode` (read-only, workspace-write, agent=Auto review,
     agent-full-access; default `agent`; initial value via `INITIAL_AGENT_MODE`),
     `collaboration_mode`, `model`, `reasoning_effort` (category `thought_level`),
     `fast-mode` (category `model_config`), plus a `models` list with effort baked
     into ids. Clients must tolerate unknown categories.
   `session/set_config_option` was not exercised.
7. **Cancel.** Both answer `session/cancel` with prompt result `stopReason:
   "cancelled"` within ~20 ms and may send one `session/update` after the cancel,
   as the spec allows. Behavior of long-running shell commands under cancel was not
   verified (Claude declined a `sleep 40`).
8. **Session list/resume.** Both advertise `loadSession` and
   `sessionCapabilities {list, resume, close, delete, fork, additionalDirectories}`.
   `session/list` with `{}` works. **Claude's list returns every Claude Code
   session of the user across all directories (titles and cwd)**: always filter by
   the workspace `cwd` and never log or persist other entries.
9. **Version.** `protocolVersion: 1` was accepted by both; both advertise
   `_meta.steering.supported`.
10. **Extensions seen** (must be tolerated and ignored when unknown):
    `_auth/status_update`, `session_info_update` (thread status/title),
    `usage_update` (token counts), `_meta.claudeCode.*`, `_meta.codex.*`,
    `_meta.terminal_output_delta` / `terminal_exit`.

## Message shapes worth fixing in fixtures

- Tool calls start with `title: "Read File"` / `"Preparing file..."` and empty
  `rawInput`, then fill in via `tool_call_update` (title, locations, rawInput,
  content, status). Merge by `toolCallId`; later `null`/absent fields must not
  erase earlier values.
- Claude's Read completes with a text `content` item wrapped in a code fence plus
  `rawOutput` containing line-numbered text.
- Codex tool calls use `content: [{type: "terminal", terminalId: "exec-..."}]` even
  though no `terminal/create` was issued by the client; `terminalId` is only a
  correlation key for `_meta.terminal_output_delta`.
- Codex streams many small `agent_message_chunk`s (37 for one short answer);
  Claude sends few. The reducer must coalesce and the UI must not re-layout the
  whole transcript per chunk.

## Design changes this forces on the plan

1. **Edits cannot go through `fs/write_text_file`.** Replace the "guarded fs
   write" design with:
   - *Claude:* review at permission time using the `diff` content; "Allow" lets
     the agent write. Before allowing, if the target is an open **dirty** buffer,
     refuse (answer `reject_once`) and tell the user to save or discard - the
     agent would otherwise overwrite their edits on disk. After a write, open clean
     buffers reload; the existing "changed on disk" flow covers the rest.
   - *Codex:* no pre-write information. Take a **pre-turn snapshot** (open buffers
     plus a bounded hash/mtime index of the workspace, or git status/stash when in
     a repository) and show a **"Files changed by the agent" review** after the
     turn (diff against the snapshot, Open diff, Revert file). Offer Codex's
     `read-only` / `workspace-write` modes in the UI so the user chooses between
     "ask before editing" and "edit freely and review afterwards".
2. **Unsaved text must be sent, not served.** Warn when unsaved files are open
   ("Agents read from disk"), with *Save all and send* or *Attach unsaved buffers
   as context* (`resource` blocks). Selection/file mentions also use `resource`
   blocks.
3. **Advertise `terminal: false`.** No adapter used it. Keep `fs` handlers as a
   low-priority, cheap implementation (read from buffers) in case a future adapter
   or version uses them; do not build the plan around them.
4. **Handle `-32000 Authentication required`** from `session/prompt` (Claude) and
   `session/new` (Codex) as a first-class "Sign in" state, with `authenticate`
   for Codex methods and CLI instructions for Claude.
5. **Install footprint and launch.** ~650 MB of native binaries per install and a
   Node >= 22 requirement make the `npx` route a heavy first-run action: the
   consent dialog must state the download size, and docs should prefer a one-time
   global install.
6. **Privacy rules.** Never log `_auth/status_update` e-mail; filter `session/list`
   to the workspace; keep transcripts out of the repo unless sanitized like these.

## Not verified (carry into Phase 1/2)

- Codex `authenticate` for `chat-gpt` and `api-key`; Claude `ANTHROPIC_API_KEY`.
- `session/set_config_option`, `session/load`, `session/resume` behavior.
- Cancel while a long shell command or permission request is pending.
- Behavior under a GUI-launched `PATH` (Dock/Start menu) and on Windows.
- Whether Codex asks permission via ACP in `workspace-write` mode for network or
  out-of-workspace writes (only `read-only` was exercised).
