# Native agent spike findings (2026-10-18)

Phase 1 of [node-free-agent-integration-plan.md](../node-free-agent-integration-plan.md).
Environment: macOS arm64, `codex-cli 0.157.0`, Claude Code `2.1.212`, both using the
machine's existing logins, both launched directly with no Node adapter. Drivers are
`codex_spike.py` and `claude_spike.py` (stdlib Python, research tools only). Every
statement is backed by a sanitized transcript in `transcripts/` (e-mail, account IDs,
install IDs, user name and scratch paths redacted). Prompts were tiny; the workspace
was an empty scratch directory.

## Codex (`codex app-server`, stdio, newline-delimited JSON, no `jsonrpc` field)

| Question | Result | Transcript |
| --- | --- | --- |
| Launch / handshake | `initialize {clientInfo}` -> result with `userAgent`, `codexHome`, platform; then `initialized` notification. Ready in ~20 ms warm. | `codex-hello` |
| Auth / account | `account/read` -> `{account:{type:"chatgpt", planType}, requiresOpenaiAuth}`; `account/updated` notification. The account object carries an e-mail: treat as personal data, show a label only. | `codex-hello` |
| Models | `model/list` -> `{data:[{id, model, displayName, hidden, ...}]}`. **The user's configured default model was rejected by the server** ("model not supported when using Codex with a ChatGPT account") and the failure arrived as an `error` notification plus `turn/completed {status:"failed"}`; Lumen must pass an explicit model from `model/list` or surface that error inline. | `codex-hello` (first run, replaced) |
| Create / turn | `thread/start {cwd, sandbox, approvalPolicy, model, ephemeral}` -> `result.thread.id` plus `thread/started`. `turn/start {threadId, input:[{type:"text",text}]}` -> immediate `result.turn {id,status:"inProgress"}`, then `turn/started`, `item/started|completed` (`userMessage`, `agentMessage`), `item/agentMessage/delta {itemId, delta}`, `thread/tokenUsage/updated`, `account/rateLimits/updated`, `thread/status/changed`, `turn/completed {turn.status}`. | `codex-hello` |
| **Two sessions, one process** | Two `thread/start` and two interleaved `turn/start` on **one** app-server: every notification carries `threadId`/`turnId`; completions arrived independently (one finished at 4.0 s, the other at 4.8 s). Same-provider multi-session works without a second process. | `codex-two-threads` |
| **Interrupt** | `turn/interrupt {threadId, turnId}` -> response in ~10 ms and `turn/completed {status:"interrupted"}`, even before any agent output. | `codex-interrupt` |
| **Approval round trip** | With `sandbox:"read-only"`, `approvalPolicy:"on-request"`, a write attempt produced a **server request** `item/commandExecution/requestApproval {threadId, turnId, itemId, reason, command, cwd, ...}`; the client replied `{id, result:{decision:"decline"}}`; the command item completed with `status:"declined"` and the agent explained the refusal. Decisions in the schema: `accept`, `acceptForSession`, `decline`, plus structured variants. File-change and permission requests are separate server requests (`item/fileChange/requestApproval`, `item/permissions/requestApproval`); user questions are `item/tool/requestUserInput`. | `codex-approval` |
| History / listing | `thread/list {limit}` works; schema also has `thread/read`, `thread/turns/list`, `thread/items/list`, `thread/resume`, `thread/archive|unarchive`, `thread/name/set`, `thread/unsubscribe`. Listing was exercised, resume/read were not. | `codex-list` |
| Other noise to tolerate | `remoteControl/status/changed`, `mcpServer/startupStatus/updated`, `warning`, `account/*` notifications. The user's configured MCP servers started inside each thread. | all |

Schema: `codex app-server generate-json-schema --out DIR` emits version-matched
JSON Schema (275 v2 types); the Rust mappings should be generated/checked against it
for the pinned minimum version.

## Claude Code (`claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages --permission-prompt-tool stdio`)

| Question | Result | Transcript |
| --- | --- | --- |
| Launch | One long-lived process per session; first message at ~0.8 s (`system/init`), reply at ~8 s for a trivial prompt (model latency). | `claude-hello` |
| Init metadata | `system/init`: `session_id`, `cwd`, `model`, `permissionMode`, `tools`, `mcp_servers`, `slash_commands`, `agents`, `skills`, `plugins`, `claude_code_version`, and **`capabilities: ["interrupt_receipt_v1","msg_lifecycle_v1"]`** for feature detection. | `claude-hello` |
| Streaming | `stream_event` (`message_start`, `content_block_*`, `message_delta/stop`), whole `assistant` / `user` messages, `rate_limit_event`, final `result {subtype, is_error, duration_ms, total_cost_usd, usage, session_id}`. A turn ends at `result`, never at process exit. | `claude-hello` |
| **Permission round trip without MCP** | `--permission-prompt-tool stdio` makes Claude send a **`control_request {subtype:"can_use_tool", tool_name, display_name, input, description, permission_suggestions, tool_use_id}`** on the same stream before running the tool. The client answers `control_response {subtype:"success", request_id, response:{behavior:"deny"|"allow", message|updatedInput}}`. A deny produced a `tool_result` error and the agent reported it. `permission_suggestions` carries Claude's own offered scope (for example `setMode acceptEdits`, destination `session`). So the in-library MCP endpoint is **not needed** for Claude. | `claude-approval` |
| **Interrupt** | Client sends `control_request {subtype:"interrupt"}`; reply `control_response {still_queued:[]}`, an `[Request interrupted by user]` user message, then `result {subtype:"error_during_execution"}` ~10 ms later. | `claude-interrupt` |
| **Resume** | New process with `--resume <session_id>` kept the same `session_id` and remembered earlier context ("tangerine"). | `claude-resume` |
| Two sessions | Not run concurrently; each session is its own process by design, so two sessions are two independent launches (still to be exercised in the contract/integration tests). | - |

## Answers to the plan's gates

1. **No-Node launch:** both providers run from their own installed binaries with no
   Node, npm or ACP adapter involved in the driver or in the protocol.
2. **Claude permission transport:** resolved. Use stdio control messages
   (`--permission-prompt-tool stdio`). It is the transport the Agent SDK uses, but
   it is **not documented as a stable public protocol**: pin the tested Claude Code
   versions, feature-detect from `system/init.capabilities`, and keep the mapping
   in one module. No listener, so the MCP-endpoint security design is unused and
   stays a fallback only.
3. **Same-provider multi-session:** Codex multiplexes threads in one process (works
   today); Claude is one process per session. Both fit the plan's "backend owns its
   runtimes" rule.
4. **Delayed approval reply tolerated?** Not yet tested (the clients answered
   immediately). Still open for the scheduler's "release the execution slot while
   waiting for the user" rule; test with a 30+ second delay before relying on it.
5. **Project files executed at launch:** not enumerated yet. `system/init` lists
   `mcp_servers`, `skills`, `plugins` and `agents` that Claude loaded and Codex
   starts configured MCP servers per thread; the trust prompt must list what is
   actually loaded from the workspace versus the user's home. Open.

## Not yet verified (carry into the provider-crate phase)

- Codex `thread/resume` / `thread/read` / `thread/turns/list` and `fileChange`
  approvals; `item/tool/requestUserInput` shape; `turn/steer`.
- Claude `AskUserQuestion` through `can_use_tool`; `allow` with `updatedInput`;
  `--resume` for a session started by an interactive CLI; behavior under a
  GUI-launched `PATH`; Windows for both.
- Interrupting a long-running shell command and descendant cleanup for both.
- Delayed approval replies (above) and approval during another session's turn.
- Terms of use for driving a user's own CLI login from a third-party app.
