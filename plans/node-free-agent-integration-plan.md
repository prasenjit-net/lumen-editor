# Native agent sessions: unified sidebar, chat tabs, and three Rust crates

Updated: 2026-10-18 (revised after review). Status: proposed design only; no
implementation in this change. This revision replaces the earlier ACP-fallback
recommendation with the requested native-only Codex/Claude Code design, and fixes
the review findings: the panel premise, tab/group prerequisites, process-code reuse,
scheduling starvation, workspace trust, permission-endpoint security and phase order.

## Requirements this plan implements

1. The current agent panel becomes a session-list panel: sessions from both
   providers together, subtly color coded.
2. A chat opens as an editor-area tab, with the full work area for the transcript.
3. Native Codex and Claude Code integration, no Node-based ACP; no other providers.
4. An ACP-like common interface, using the two ACP adapter repositories as code
   reference where useful.
5. That interface is a Rust library linked into Lumen, not a separate process.
6. Exactly three crates: common interface, Codex, Claude Code.

## Multi-session in plain terms

- **A session is one conversation.** You can have many: three Codex chats and two
  Claude chats are five sessions. Each has its own history, model, permission
  mode, draft, approvals and tab. Nothing is shared between them just because they
  use the same provider, and Codex cannot continue a Claude conversation.
- **Having a session is not the same as running it.** Saved sessions are rows in
  the list; they cost nothing until you send a message. A chat tab can be closed
  while its session (and even a running turn) carries on in the list.
- **One message at a time per session.** While a session is working, another
  message you send waits in that session's visible queue (editable, removable).
- **Several sessions can work at once** (initially two at a time, configurable),
  including two of the same provider. Others wait with a visible reason.
- **Same folder, one writer at a time.** Two agents editing the same checkout can
  overwrite each other and make "what did this agent change" unknowable, so only
  one session that may write can work in a given folder at once. Sessions in
  different folders or git worktrees run in parallel. Lumen never creates
  worktrees or switches branches for you.
- **Waiting for you does not block everyone else.** A turn waiting for your
  approval or answer gives up its "working" slot (so other sessions can run) but
  keeps its folder's write lease (so nothing else edits that folder meanwhile).
- **Provider limits are shared.** Two Codex sessions use the same account and rate
  limits; separate sessions do not create separate quotas.
- **No hidden collaboration.** Sessions do not talk to each other or share memory.
  A provider's own subagents stay inside the session that started them.

Related: [original ACP integration](agent-integration-acp-plan.md),
[recorded adapter findings](acp-spike/FINDINGS.md),
[agent acceptance checklist](agent-acceptance-checklist.md), and
[editor groups/tabs](editor-tabs-splits-and-recent-folders-plan.md).

## Product decisions

1. Replace the contents of the current coding-agent panel (the right-hand
   `SidePanel` opened by the activity-bar Agent button / `ToggleAgentPanel`,
   Cmd/Ctrl+Shift+A) with a unified session list. It stays the same panel, button,
   shortcut, `agent.enabled` setting and attention badge; it is not a new left
   `Activity`. Codex and Claude Code conversations appear together, subtly color
   coded and explicitly labeled by provider.
2. Open each conversation as an editor-area chat tab, alongside files, settings,
   diffs, and task configuration tabs. Use the main work area for the transcript,
   tool output, approvals, and composer.
3. Support exactly Codex and Claude Code through their native executables.
   No Node/npm/npx ACP adapters, generic ACP providers, provider marketplace, or
   additional agent providers in this scope.
4. Define an ACP-inspired common interface as a Rust library linked into Lumen.
   It provides typed operations, capabilities, streaming events, requests for
   user input, session lifecycle, and errors. It is not the ACP wire protocol.
5. Implement exactly three new runtime crates: `lumen-agent-core`,
   `lumen-agent-codex`, and `lumen-agent-claude`. Provider crates implement the
   common interface; application/UI code uses it without provider wire messages.
6. Support multiple conversations from either provider, including multiple
   conversations from the same provider. Independent sessions can execute in
   parallel subject to resource limits and workspace-write coordination.

**Process boundary:** the three libraries execute inside Lumen. They launch the
native `codex` / `claude` agent executables as children because those executables
own the actual agent engine. There is no Lumen ACP sidecar, Node adapter process,
or separate common-interface server. Embedding both vendor agent engines entirely
inside Lumen is not this design. Vendor binaries and user-selected tools/plugins
may have their own internal dependencies; the supported Lumen integration must
not require a separately installed Node, npm, or Python runtime.

## What the existing code requires us to change

`src/app/agent.rs` currently initializes `AgentsView.sessions` with one entry per
`AgentKind`, and `session(kind)` indexes it by provider. Provider selection and
conversation identity are therefore coupled. Replace that structure with a
session registry keyed by stable local session IDs, plus separate provider
availability/authentication state. Do not extend the two-element vector to fake
multiple sessions.

The agent UI today is a right-hand `SidePanel` (`src/ui/agent_panel.rs`), toggled
from the bottom group of the activity bar; `Activity` has no agent entry. The
session list replaces that panel's contents in place, keeping its toggle, badge,
width persistence and `agent.enabled` gating.

`src/services/agent_process.rs` provides child launching, framing and event
delivery; process-group isolation, polite/forced tree stop and PATH lookup now live
in `src/services/process_tree.rs`, shared with the task runner. Do not move those
into an agent crate (tasks would then depend on agent code) and do not add a fourth
crate. Instead `lumen-agent-core` defines a small injectable `ChildLauncher`
interface (spawn with argv/env/cwd, stdin/stdout/stderr handles, pid, polite stop,
forced stop, exit status); Lumen implements it with `process_tree.rs`, and tests
inject a fake. Framing, bounded channels and lifecycle logic that only agents need
move into a private core module.

`src/ui/editor_tabs.rs` distinguishes document, diff, settings and task editor
targets, but every non-document tab is drawn only in the first editor group
(`hide_virtual_tabs`), cannot be dragged, has no group assignment, and the
closed-tab history covers documents only. Chat tabs need all of those, so a
prerequisite refactor generalizes tab targets first (see Phase 5a). Then add a
typed `AgentSession(SessionId)` target integrated with active-content dispatch,
close/reopen and groups. Do not create dummy Documents for transcripts, give chats
fake paths, or attach text-editor undo/dirty semantics to them.

Reusable patterns already in the tree: the task runner's first-run review keyed by
an execution digest (`services/task_trust.rs`), its RunId-keyed run tabs and
quit/folder-change guards, `leave_page_tabs()` / `page_tab_active()` for
non-document pages, and the `terminal_focused` guard in `app/events.rs` that keeps
global shortcuts from stealing typing.

Preserve the existing app's attachments, held prompts, permission UI, change
review, and transcript virtualization as reusable components. Existing ACP
transcripts/tests are behavioral references; ACP transport types must stop leaking
into app/UI commands after migration.

## UI and interaction contract

### Unified session-list panel

The existing right-hand agent panel shows a SESSIONS list using `panel_header()`.
Header actions: New Session, Refresh History, and a compact overflow menu.
New Session opens a two-provider chooser with availability/login state; creating
one conversation never replaces another. Do not automatically start an agent or
send a prompt just by opening the panel.

Rows contain title, a small provider label/icon, state, and last activity time.
Use a narrow provider accent or small badge, not a whole tinted card. Add shared
Codex/Claude semantic accent tokens to `src/ui/theme.rs`; choose values that work
in dark/light/high-contrast themes. Reuse the bundled icon set via `src/icon.rs`
(the Codicon font is being replaced by Lumen's own icons) and provide text/accessible
labels so color is never the sole identity signal. Distinguish provider accents
from warning/error and running-state colors.

Both providers share one list, sorted by most recent activity, with optional
Pinned and Needs Attention sections. Avoid reordering a row underneath a pointer
or keyboard selection during interaction. Search title, provider, and workspace;
filters: All, Codex, Claude, Running, Needs Attention, Archived. Default history
scope is the current workspace plus explicitly opened sessions. Other-workspace
history is opt-in and visibly labeled; do not expose global provider history just
because a provider returns it.

A click opens or activates the session's chat tab. Right-click offers Open,
Rename, Pin, Stop Current Turn, Archive, and Remove Local Record with appropriate
states. Native provider rename/archive is capability-gated; a local title/hidden
record must be labeled as local. Archive is reversible and does not mean provider
history deletion. Permanent provider-history deletion is outside the first scope.

Unfocused sessions show unread completion and pending-approval/question badges.
Their events never steal focus or switch tabs automatically. The activity badge
summarizes sessions needing attention. Failed discovery of one provider must not
hide successful results from the other; merge independently paginated results
and deduplicate by provider session identity within the provider profile.

### Chat in the editor area

Tab title: session title with a compact provider indicator and running/attention
state. Tooltip includes provider, workspace/worktree and state. Preserve the
existing compact tab dimensions, overflow, active reveal, close affordance, and
reorder/group behavior. One local session has one live tab across groups in the
first release; reopening it focuses that tab rather than duplicating a runtime.
Different sessions may be viewed side by side in existing split groups.

Within the tab:

- Compact session header: provider, model, mode/permission profile, workspace,
  and supported actions. Provider is immutable after creation.
- Virtualized transcript: messages, tool calls, plans, file changes, usage where
  reported, inline errors, and requests for approval or answers.
- Composer anchored at the bottom, growing to a bounded height; attachment chips,
  multiline input, Send/Stop, and clear queued-message state.
- Per-session draft, attachments, transcript selection, scroll position, search,
  expanded tool cards, and follow-output state.

The composer is a text input that owns the keyboard while focused: extend the
`terminal_focused`-style guard in `app/events.rs` so global and editor shortcuts
do not intercept chat typing, clipboard or IME.

Reuse existing focused widgets and typed commands. Generic text fields keep
ordinary clipboard behavior; editor-specific cut/paste rules cannot intercept
chat input. A chat-active tab must not accidentally route Save, Format, Find,
Go-to, or language commands to the last file. Define context-sensitive Find as
transcript search; disable file-only commands until a file becomes active. Keep
explicitly targeted file commands valid regardless of the visible chat.

Close Tab hides the view and preserves the session/draft. A running turn continues
in the sidebar with its status; Stop is a separate explicit action. Reopening
restores view state. Archive requires an idle session; offer Stop then Archive
for active sessions. Quitting or closing a workspace with active turns uses an
explicit Stop and Close / Cancel flow, integrated with existing dirty-file prompts.
No invisible detached runs survive application exit by design.

## Multi-session semantics

### Terms that must stay distinct

| Concept | Meaning | Lifetime |
| --- | --- | --- |
| Provider | Codex or Claude Code | Fixed product support |
| Provider profile | Executable/install and authentication context | User configuration; one profile/provider initially |
| Session | An independent conversation with context/history/config | Persists across turns and tab closure |
| Turn | One submitted user message and ensuing agent work | Queued/running/waiting/completed |
| Tab | A view of a session | Can close while session remains |
| Runtime | Child process/connection serving a session | May be stopped and later recreated |
| Workspace context | Canonical working tree/directory used by tools | Captured for the session |

Example: two Codex chats and one Claude chat are three independent sessions.
They do not share conversation history or permission decisions merely because
they use the same provider. Codex cannot resume a Claude session. “Continue with
other provider” would create a new conversation with explicitly shared context;
that transfer feature is deferred.

### Multiple sessions versus parallel execution

- Any number of saved session records can appear in the paginated list.
  Records and open tabs do not each require a running process.
- Exactly one turn is active (executing or waiting for the user) per session.
  The **app** owns a visible per-session queue (edit/remove before dispatch); the
  backend never queues turns itself and answers a second StartTurn with Busy.
- Different sessions can run simultaneously, including two of the same provider.
  Initial global limit: two *executing* turns, configurable within measured bounds.
- Model/account rate limits remain shared externally where applicable. Separate
  sessions do not create independent quotas. Limit/rate errors attach to their
  actual session and do not silently retry a side-effecting prompt elsewhere.
- Sessions are independent conversations, not automatic agent collaboration or
  shared memory. Provider-managed subagents remain child activities within the
  owning session/turn, not extra user sessions unless a later feature exposes them.

### One scheduler, three resources

The app runs one scheduler with a single state machine per turn, so the limit,
the write lease and the queue cannot deadlock each other:

| Turn state | Execution slot | Workspace write lease | Shown as |
| --- | --- | --- | --- |
| Queued (session busy) | no | no | "Waiting for the previous message" |
| Ready, no free slot | no | no | "Waiting: 2 sessions are working" |
| Ready, folder leased by another session | no | no | "Waiting: <session> is editing this folder" |
| Executing | yes | yes if potentially writing | Running |
| Waiting for approval/answer | **released** | **kept** | Needs Attention |
| Finishing (change capture) | yes | yes | Recording changes |
| Finished / failed / interrupted | no | released after change capture | Idle |

- Leases are acquired before the slot, in a fixed order (lease, then slot), and
  released in reverse, so two turns never hold one each while waiting for the other.
- A turn answering an approval re-acquires a slot before continuing; while no slot
  is free it shows "Approved, waiting to continue". The provider keeps the turn
  open meanwhile (the reply is simply delayed); verify in the spike that both
  providers tolerate a delayed approval reply without timing out, else hold the
  slot for that provider and say so in the UI.
- Waiting for approval holds the lease indefinitely by design (the agent is
  mid-edit). The Needs Attention badge and the list's waiting reason make this
  visible; there is no silent timeout or auto-deny. Stopping the turn releases it.
- Every terminal path (finish, failure, interrupt, runtime crash, app quit,
  session archive) releases both resources; this is a contract test.

### Same workspace and simultaneous edits

Independent conversation state does not isolate filesystem changes. Two agents
in one checkout can overwrite each other's work, and whole-folder before/after
snapshots cannot reliably attribute those edits to one agent.

Default: allow one potentially writing turn per canonical workspace context;
queue another until the first completes and its change review has been recorded.
Acquire this lease before the pre-turn snapshot and hold it through post-turn
change capture. Treat unknown execution modes as potentially writing. File-tree
identity must resolve path aliases/symlinks; different worktrees are distinct
contexts even when they share a repository.

Concurrency is allowed across separate directories/worktrees. Two same-workspace
read-only turns may run together only when the provider enforces that restriction;
a UI label or prompt instruction is insufficient. Conservatively queue when
isolation cannot be verified. Do not silently create worktrees, switch branches,
or change user directories. Initial UI explains the queue and allows the user to
choose an existing separate worktree when creating a session.

The lease coordinates Lumen's sessions, not external terminals/editors or other
Lumen instances. Preserve external-change detection, dirty-buffer prompts, and
conflict-aware revert. An agent-wide folder diff is observational, not proof that
every changed file belongs to that turn. Queued prompts recheck workspace,
attachments, saves and snapshot revisions immediately before dispatch.

### Native process strategy

Begin with one lazily launched native runtime per actively attached session for
both providers. This simplifies configuration isolation, targeted cancellation,
and crash containment. A small number of idle runtimes may be retained; detach
idle sessions after a bounded timeout only once native resume is proven. Persist
the transcript cache first. Never evict running/waiting sessions to save memory.
Initial idle policy: at most two warm idle runtimes, five-minute expiry, measured
and tunable. Runtime count and active-turn count are separate limits.

Ownership: each provider backend owns its runtime pool and process handles; the
app never sees a process. The app decides *policy* by calling AttachSession and
DetachSession (and enforces the warm-idle cap by detaching); the backend decides
*mechanism* (one process per session today, a shared Codex app-server later)
without changing the interface. The turn scheduler above lives in the app.

Codex may support multiplexing several native threads over one app-server process;
reference code has per-thread routing. Treat pooling as a later measured
optimization, not a precondition for multi-session UI. Validate request routing,
configuration scope, cancellation, and the larger crash blast radius before use.
Claude session/CLI process mapping must be established by the native spike; a
session may outlive a CLI process. Both choices remain hidden behind the common
interface, without claiming vendor process behavior is identical.

## Three-crate architecture

```text
Lumen app: session registry, editor tabs, drafts, workspace write leases, UI
                       |
                lumen-agent-core
      typed API / events / capabilities / errors / test contracts
                  /                 
       lumen-agent-codex       lumen-agent-claude
       native wire mapping     native wire mapping
               |                       |
       codex app-server          native claude CLI
```

Dependency direction: both provider crates depend on core; core depends on
neither provider nor Lumen. Lumen links all three and selects the concrete
backend. Provider crates have no egui/window/dialog dependencies. Core exposes
pure domain/API modules and a private reusable runtime/transport module where
needed. Tests use injected process/clock/event facilities. Avoid cycles between
app services and library crates.

Concurrency model: plain `std` threads and bounded channels, matching the rest of
the codebase (language server, agent process, task runner). No async runtime in
the first release; adding one later is a contained change inside the provider
crates because the public API is poll-based. The `ChildLauncher` injected by Lumen
(see above) is the only way the crates start processes.

| Crate | Public responsibilities | Private implementation |
| --- | --- | --- |
| `lumen-agent-core` / `crates/agent-core` | Provider/session/turn identities, request/response/event types, object-safe backend API, capabilities, errors, contract test helpers | Bounded framing/channel/process lifecycle utilities shared by the two implementations |
| `lumen-agent-codex` / `crates/agent-codex` | `CodexBackend` constructor implementing core API | App-server protocol, initialization, native thread/turn mapping, auth/capability discovery, approval and history translation |
| `lumen-agent-claude` / `crates/agent-claude` | `ClaudeBackend` constructor implementing core API | Native CLI structured stream, resume/history/config discovery, approval/question integration, cancellation and process ownership |

No public raw JSON messages, arbitrary ACP method names, SDK objects, or provider
process handles in the app-facing interface. Small typed provider-specific
extensions are acceptable only when UI behavior needs them; no unbounded generic
“call anything” escape hatch. Both implementations use the same test contracts.

The existing `lumen-agent-protocol` is historical ACP code. Reuse/rehome genuinely
common data and tests; remove its production dependency after migration. It must
not become a fourth live integration crate or an ACP runtime fallback. Preserve
old transcript fixtures as test data or historical documentation with provenance.

## Detailed common Rust interface

The following is an API design sketch, not code to compile in this change.
Use opaque newtypes for IDs, typed enums for outcomes, and `Result` for all
fallible operations. Library versioning is ordinary Rust/Cargo semver; this is
not a serialized public network protocol or stable dynamic-library ABI.

### Execution model

Use a nonblocking object-safe command/event interface that fits egui's polling
controller. Provider worker threads/async runtimes do I/O; no method called from
the UI thread waits for network, child output, login, or process termination.

```rust
pub enum ProviderId { Codex, Claude }

pub trait AgentBackend: Send {
    fn descriptor(&self) -> &ProviderDescriptor;
    fn try_submit(&mut self, request: Request)
        -> Result<OperationId, SubmitError>;
    fn poll_events(&mut self, budget: PollBudget) -> Vec<EventEnvelope>;
    fn begin_shutdown(&mut self, deadline: Duration)
        -> Result<OperationId, SubmitError>;
}

pub struct Request {
    pub client_request_id: ClientRequestId,
    pub operation: AgentOperation,
}

pub struct EventEnvelope {
    pub provider: ProviderId,
    pub profile: ProfileId,
    pub generation: RuntimeGeneration,
    pub operation: Option<OperationId>,
    pub session: Option<SessionId>,
    pub turn: Option<TurnId>,
    pub sequence: u64,
    pub event: AgentEvent,
}
```

`try_submit` means accepted into the backend queue, not successfully executed.
Every accepted operation gets one terminal result: Succeeded(typed result),
Failed(AgentError), or Cancelled. A start-turn operation returns a turn handle;
the turn ends later through TurnFinished, never through an inferred EOF alone.
Operation cancellation and turn interruption are distinct. Duplicate
`client_request_id` values are rejected/deduplicated locally; this does not claim
exactly-once delivery to a native process after a crash.

`SubmitError`: queue full, invalid request, shutdown begun. Deferred errors:
unsupported capability, provider missing, incompatible version, authentication
required, busy, not found, invalid config, rate limited (optional retry delay),
transport/protocol failure, timeout, stale target, permission denied, and
ambiguous execution. Unknown send outcome is surfaced, not automatically resent.

### Operations and payloads

| Operation | Required input | Result / semantics |
| --- | --- | --- |
| Probe | Provider profile/executable config | Version, availability, capabilities, supported auth actions; no prompt |
| ReadAuthState | ProfileId | Auth state and safe display label; never tokens |
| BeginAuthentication / CancelAuthentication | Supported auth method / AuthAttemptId | Browser/device/instruction challenge and completion, capability-gated |
| ListSessions | Workspace filter, opaque cursor, limit | Session summaries, next cursor, discovery completeness |
| CreateSession | Client SessionId, workspace context, initial config | NativeSessionRef, effective config and capabilities |
| AttachSession | SessionId, NativeSessionRef, workspace | Restored session state and history availability |
| ReadHistory | SessionId, cursor, limit | Ordered transcript items and authoritative continuation cursor |
| UpdateSessionMetadata | SessionId, title | Provider update when supported; local metadata belongs to app |
| ReadConfig / SetConfig | SessionId, expected config revision, typed patch | Validated effective config and new revision; no optimistic permission changes |
| ListCommands | SessionId | Supported slash/action descriptors; optional |
| StartTurn | SessionId, ClientTurnId, input blocks, expected session/config revision | TurnId and accepted state; one active turn/session |
| InterruptTurn | SessionId, TurnId | Cancellation requested; wait for TurnFinished for actual result |
| ReplyApproval | Scoped RequestToken, offered decision ID | Acknowledged or stale/already-resolved; no fabricated permission widening |
| ReplyQuestion | Scoped RequestToken, typed answers | Validated response or cancellation |
| DetachSession | SessionId | Release idle runtime/subscriptions; retain native history |
| ArchiveSession | SessionId | Native archive when supported; no destructive delete implied |
| Shutdown | Deadline | Drain/deny pending requests, stop owned processes, final completion |

`ProviderDescriptor` includes executable version and supported core API version.
`NativeSessionRef` is opaque and namespaced by provider/profile, never reused
across providers. `SessionSummary` includes stable native reference, title,
workspace, last activity, and optional native status; unknown fields stay unknown.
A Lumen-local SessionId exists before native creation and keeps tab IDs stable.
Native restore IDs are not tab indices or provider-vector slots.

`SessionConfig` contains supported model ID, reasoning option when available,
permission/sandbox profile, and other typed advertised options. `ConfigOption`
includes ID, type, choices, current value, description, scope (profile/session/turn)
and mutability (idle-only/next-turn/live). Config is per-session unless explicitly
profile-scoped. Capability updates cannot silently change an in-flight turn's
permissions. No generic assumption that Claude and Codex mode names are equal.

### Inputs, events, and transcript records

`InputBlock`: text, explicit file reference, text snapshot with origin/revision,
selection snapshot, and image with media type/size limits. Unsupported attachments
fail before prompt submission or require explicit conversion; no silent omission.
Snapshots are context, not authority to overwrite the original file. Resolve
queued attachments at dispatch and show any required refresh to the user.

`AgentEvent` families:

- OperationFinished, AuthStateChanged, AuthenticationChallenge.
- SessionCreated/Attached/Detached, MetadataChanged, ConfigChanged,
  CapabilitiesChanged, CommandsChanged.
- TurnQueued/Started, MessageStarted/TextDelta/MessageFinished,
  ToolStarted/ToolUpdated/ToolFinished, PlanUpdated, UsageUpdated.
- ApprovalRequested/ApprovalResolved, QuestionRequested/QuestionResolved.
- FileChangesReported, TurnFinished, Diagnostic, RuntimeExited.

Messages/tool calls use stable scoped IDs; deltas identify their target item and
position/sequence. Tool updates include typed kind, status, bounded preview,
locations, optional patch/terminal content, and parent tool ID for subactivities.
File-change reports distinguish provider-reported/proposed/applied observations
from Lumen's own disk comparison. Never label a preview as an editor-applied edit.
Usage is optional provider-reported data, not an invented normalized token price.
Do not require or synthesize private reasoning streams to render a transcript.

`TurnOutcome`: completed, interrupted, failed, limited, or refused, with optional
provider detail and structured error. Runtime failure may leave execution state
unknown; mark Interrupted/Unknown and require an explicit new action. History
records use the same transcript item model with replay metadata; history replay
must never re-trigger approval dialogs, tools, notifications, or auto-follow edits.
Deduplicate replay/live overlap by native item identity where reliable; otherwise
replace a snapshot at a defined boundary instead of guessing overlap from text.

### Permissions, questions, and capabilities

A `RequestToken` binds provider/profile, SessionId, TurnId, generation, and native
request ID. Responses to old turns/processes are rejected. Approvals contain an
action summary, known paths/command details, and provider-offered choices with
scope and risk description. The common API supplies a shape, not a universal
allow-all interpretation. Unknown native approval types are denied/unsupported
with feedback, never automatically approved.

Questions support free text, single/multiple choice and cancellation as advertised.
Multiple sessions may wait independently; responding in one chat cannot resolve
another's request. Closing a tab preserves a visible Needs Attention sidebar row.
Interruption/shutdown explicitly settle pending requests. One-shot responses are
idempotent within Lumen and cannot be reused after process restart.

`Capabilities` is explicit per operation/content type: history list/read/resume,
rename/archive, streaming, attachments, approvals, questions, config mutation,
command discovery, usage, safe read-only enforcement, interrupt, and native
multi-thread transport. Return supported, unsupported, or version-gated with
reason. `can_resume` and `can_list_history` are separate: Lumen can restore known
local sessions without pretending it can enumerate every external native session.

### Resource and event-delivery guarantees

Events are ordered per runtime generation and identify their session/turn; there
is no promised total chronology across providers. Never route by whichever chat
is visible. Deduplicate native events when IDs permit. A session's state machine
rejects impossible transitions and ignores old generations with bounded logging.

Bound command queues, frame sizes, transcript retention, stderr, and per-frame
polling work. Keep control/completion/approval traffic separate from bulky text.
Coalesce text safely and spill transcript data through the app store as needed.
Do not silently drop semantic events. If bounds cannot be maintained, stop the
affected runtime and report an incomplete transcript instead of stalling egui or
consuming unlimited RAM. Explicit shutdown is asynchronous; destructors are only
a bounded last-resort cleanup path, never the primary UI lifecycle.

## Provider implementation and ACP reference review

Sources below were inspected on 2026-10-18 from their moving main branches. Before
implementation, record exact upstream commit hashes, CLI/SDK versions, and source
licenses in a reference note. Use mapping behavior and fixtures as reference;
copying code requires preserving applicable license/attribution obligations.
Neither repository is a runtime dependency of the finished design.

### Codex

The Codex adapter separates app-server calls, session notifications, native event
mapping and ACP presentation. Its client code has per-session subscriptions and
queues plus native archive/rename handling. Use those patterns to identify the
required Rust mappings; do not retain the ACP server layer.
[CodexAcpClient.ts](https://github.com/agentclientprotocol/codex-acp/blob/main/src/CodexAcpClient.ts)

Inspect transport/thread ownership and pending request cancellation in
[CodexAppServerClient.ts](https://github.com/agentclientprotocol/codex-acp/blob/main/src/CodexAppServerClient.ts),
and lifecycle translation in
[CodexAcpServer.ts](https://github.com/agentclientprotocol/codex-acp/blob/main/src/CodexAcpServer.ts).
Review neighboring event/tool mapping and permission modules during implementation.
These are references for an adapter port, not evidence that every extension must
be supported in Lumen.

Launch native `codex app-server` over stdio from `lumen-agent-codex`; use its
published version-matched schemas. App-server provides bidirectional structured
messages for richer clients; its JSON-RPC wire shape differs from ACP (the
`jsonrpc` field is omitted). Preserve native thread/turn/request IDs and map them
to core identities. Recheck documented maturity warnings and supported fields
before selecting a minimum CLI version.
[App-server documentation](https://learn.chatgpt.com/docs/app-server)

Facts to pin in the spike (seen in the docs on 2026-10-18, re-verify against the
version-matched schema): handshake is `initialize` then `initialized`; conversations
are `thread/start|resume|fork|list|archive`; work is `turn/start`, `turn/steer`,
`turn/interrupt` with `turn/*` and `item/*` notifications; approvals arrive as
server requests (for example `item/.../requestApproval`) answered by the client and
settled with `serverRequest/resolved`. stdio is the supported transport; WebSocket
is experimental and unsupported, so it is out of scope. Some methods require
`experimentalApi` in `initialize`; do not depend on experimental methods for
required features without recording that risk.

### Claude Code

The Claude adapter's `acp-agent.ts` manages multiple session records, SDK query
lifecycles, cancellation and per-session tool permissions. `canUseTool` also
bridges user questions, and some runtime methods are explicitly outside public
SDK declarations. This shows why removing Node is more work than translating
streamed text, and why the SDK's behavior must be inventoried before a Rust port.
[Claude adapter implementation](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/src/acp-agent.ts)

`lumen-agent-claude` launches native Claude Code directly. Investigate documented
structured input/output, resume, interrupt and permission-prompt integration.
[Programmatic CLI](https://code.claude.com/docs/en/headless) and
[CLI reference](https://code.claude.com/docs/en/cli-reference).

Documented behavior the design must account for (seen 2026-10-18, re-verify):
`--output-format stream-json` (with `--verbose`, optionally
`--include-partial-messages`) streams events ending in a `result` message;
`--resume <session-id>` continues a specific conversation; the `system/init` event
carries a `capabilities` list (for example `interrupt_receipt_v1`) to feature-detect
instead of comparing versions; SIGINT or the SDK interrupt ends the current turn,
while SIGTERM leaves it unfinished with no result (so Stop uses interrupt, and
SIGTERM is only for shutdown); a run may stay open up to 10 minutes for background
work after its turn, so turn completion is the `result` event, never process exit;
`--permission-prompt-tool` names an MCP tool that answers permission requests, and
`--permission-prompts none` denies anything that would prompt.

Permission transport, in order of preference, decided by the spike:

1. **stdio control messages** on the same stream-json connection (what the Agent
   SDK's `canUseTool` uses). No extra listener, but it is not documented as a
   stable public protocol: pin tested versions, record the compatibility burden.
2. **An MCP endpoint hosted inside the Rust library** passed via `--mcp-config`
   and `--permission-prompt-tool`. Required safeguards: bind loopback only on an
   ephemeral port; a random per-runtime path and bearer token passed only in the
   generated config; reject requests without the token or with a browser
   `Origin`/non-loopback `Host` (DNS rebinding); one endpoint per runtime, closed
   with it; never log tokens. No helper process.

If neither delivers the full permission and question round trip, mark Claude
blocked as described below.

Gate implementation on proving the full permission/question round trip without
a Node SDK or separate adapter. If the native transport requires SDK-internal
control messages, explicitly document the compatibility burden, pin tested
versions and decide whether to maintain it. Do not present that interface as
vendor-stable. If no workable in-process bridge exists, mark the native Claude
integration blocked/incomplete; do not silently reintroduce ACP, another provider,
or bypass-permissions mode. A text-only demo does not satisfy the requested scope.

Both providers have native installation paths; onboarding links to their official
instructions and supports an explicit User Settings executable path.
[Codex installation](https://learn.chatgpt.com/docs/codex/cli),
[Claude installation](https://code.claude.com/docs/en/setup).
Provider authentication remains provider-owned; never scrape credentials or infer
that subscription login permits arbitrary model API calls.

## Workspace trust for native launches

Starting a native agent in a folder can run code from that folder before any
prompt: the Claude docs state that a `-p` run executes the project's hooks and
connects the servers in its `.mcp.json` with no trust dialog, and Codex reads
project configuration. `--bare` avoids project hooks/MCP for Claude but does not
use the subscription login. Lumen already refuses to run workspace tasks without a
review, so agents follow the same rule:

- The first agent session in a workspace shows what the provider will load from
  that folder (hooks, MCP servers, project config files found) and asks to trust
  it; the decision is remembered per workspace and configuration digest, reusing
  the task trust store pattern. A changed hook/MCP/config file asks again.
- Cancelling starts nothing. "Start without project configuration" uses the
  provider's isolation option where one exists and explains what that disables
  (for Claude, `--bare` requires an API key rather than the subscription login).
- The spike must list exactly which project files each provider executes or
  connects at launch, and which flags restrict them; the trust prompt shows that
  list, not a generic warning.
- Confirm each provider's terms for driving a user's own CLI login from a
  third-party desktop app before release; Lumen launches the user's installed
  binary and never reads or forwards its credentials.

## Persistence, discovery, and migration

Store a small Lumen session index through an app/service storage adapter: local
ID, provider/profile, opaque native ID, canonical workspace/worktree, title/pin/
archive flags, timestamps, last known outcome, and schema version. Persist drafts,
attachments references and transcript cache separately with bounded storage and
atomic writes. No tokens or full environment dumps. User content is local
plaintext unless an explicit encryption feature is added; offer clear cache
retention/removal semantics. Keep live reader/runtime handles out of saved state.

On startup show cached rows promptly, then discover provider history lazily and
merge by provider/profile/native ID. Loading a session must not load all histories
into memory. Native history is the source for native conversation resumption;
Lumen's cache helps display/search and cannot recreate native reasoning state.
Missing native history leaves a readable cached transcript with Resume unavailable.
Do not silently start a new session while claiming the old one resumed.

Replace runtime ACP settings with per-provider native executable/configuration.
Retain legacy fields as recognized migration data with a diagnostic; never run
an old adapter path as if it were a native CLI. Old records can be linked to a
native session only after identity/format compatibility is verified; otherwise
retain them as read-only archived transcripts. No final ACP selector or fallback.

During development old and new code may coexist behind internal flags to compare
behavior. The completed release routes supported agent work through the three
new crates only. Historical plans remain historical; update README/onboarding
and acceptance notes when implementation ships.

## Delivery phases

Two tracks run in parallel after the core crate exists, so a blocked provider
cannot stall the workbench work, and the UI is built against fake backends that
honor the same contract tests.

1. **Native protocol spike and reference inventory.** Record both upstream
   revisions and CLI versions; demonstrate two same-provider sessions, one turn
   each, independent cancellation/approval/history, and no-Node launch. Resolve
   the Claude permission transport (stdio control vs in-library MCP), whether a
   delayed approval reply is tolerated, and the exact project files each provider
   executes at launch. Capture sanitized fixtures and a capability matrix.
   **Status (2026-10-18): largely done** — see
   [native-agent-spike/FINDINGS.md](native-agent-spike/FINDINGS.md). Both providers
   run natively with no Node; Codex runs several threads in one app-server and
   proves interrupt and an approval round trip; Claude's permission and interrupt
   round trips work over stream-json control messages (`--permission-prompt-tool
   stdio`), so the in-library MCP endpoint is only a fallback. Still open: delayed
   approval replies, the exact project files each provider loads at launch,
   `AskUserQuestion`/`requestUserInput`, Windows, descendant cleanup on interrupt.
2. **Common crate and contract tests.** Identities, nonblocking API, event
   reducer, lifecycle rules, `ChildLauncher`, fake backends (scriptable: streams,
   approvals, questions, crashes, slow replies) and bounded transport helpers.
3. **Track A: provider crates.** Codex app-server and native Claude
   implementations against the core contract; isolated processes, attach/resume,
   errors, permissions/questions and cleanup; behind development switches until
   required capabilities are proven.
4. **Track B: session registry, scheduler and persistence** on fake backends:
   SessionId maps, native identity index, metadata cache, paginated discovery,
   the turn scheduler (queue, slots, leases), runtime attach/detach policy and the
   workspace-trust prompt.
5. **Track B: workbench.**
   - **5a. Tab-target refactor (prerequisite):** non-document tabs get a group
     assignment, drag/reorder, close/reopen history and drawing in any group, with
     Settings, Diff and task editor behavior unchanged (fixture comparison).
   - **5b. Session list and chat tabs:** replace the agent panel contents with the
     session list, extract reusable transcript/composer pieces, add provider
     accent tokens, typed chat tab routing, close/reopen/split/focus semantics and
     the composer shortcut guard, without document regressions.
6. **Integration, migration and acceptance.** Swap fake for real backends,
   remove Node/ACP launch paths and the production protocol dependency, migrate
   histories/settings conservatively, complete clean-machine/native testing,
   update README and screenshots, and mark superseded documents (below).

Avoid committing to a fixed date before phase 1. The old Codex-first estimate
excluded editor-tab integration, true multi-session state and this stricter
three-crate/no-ACP requirement; it is no longer a full-scope estimate. Re-estimate
from the recorded protocol gaps and workbench refactor after the spike.

## Verification and release acceptance

- Core contract tests: every accepted operation terminates once, Busy/queue
  handling, event ordering, generation rejection, capability errors, stale
  approval/config revisions, malformed/oversized frames, queue saturation.
- Multiple sessions: two Codex plus two Claude conversations, distinct drafts,
  models/configs, tool calls and approvals; cancelling/closing/resuming one does
  not affect another. Test duplicate native IDs across providers/profiles.
- Concurrency: same-workspace writer queue, two independent worktrees running,
  account rate limits, lease release on all terminal paths, external edits during
  snapshots, queued attachment/save revalidation, no automatic ambiguous retry.
- Persistence: restart idle/running/waiting sessions, cached-only history,
  pagination/dedup, missing CLI/native history, corrupt records, archived rows,
  migration of legacy IDs/settings, disk-write failure with draft recovery.
- UI: mixed provider list, long titles, narrow/default/wide sidebar, color-blind
  and high-contrast labels, badges without focus stealing, huge virtualized
  transcripts, chat/file tab reorder/close/reopen/splits, keyboard/clipboard/IME.
- Existing documents: dirty/save/cancel/history, explicit file command targets,
  diagnostics, diff tabs, settings/task editor tabs, external-change protection,
  agent change review and conflict-safe revert.
- Native: actual clean machines without Node/npm/npx/Python on macOS and Windows,
  plus supported Linux; provider installation/login, several live sessions,
  long-running command interruption, descendants shutdown and app exit.
- Scheduler: two sessions waiting for approval do not block a third from running;
  an approved turn waits for a slot without timing out; lease/slot ordering never
  deadlocks; every terminal path releases both.
- Trust: first launch in a workspace prompts and lists project hooks/MCP/config;
  cancel starts nothing; a changed hook re-prompts; the MCP permission endpoint (if
  used) rejects missing tokens, non-loopback hosts and browser origins.
- Run formatting, workspace tests and Clippy, Windows target compilation where
  available, and macOS bundling. Capture representative live screenshots and
  report runtime validation separately from compilation/headless checks.

## Documents to update when this ships

This plan supersedes [agent-integration-acp-plan.md](agent-integration-acp-plan.md)
and the ACP parts of [agent-acceptance-checklist.md](agent-acceptance-checklist.md);
both, and the README "Coding agents" section, still describe the npx adapters. Add
a "superseded by" banner to the old plan and re-base the checklist when Phase 1
starts, and rewrite the README section at Phase 6. (Not changed by this revision.)

Done means the existing agent panel lists Codex and Claude sessions together;
each opens a spacious editor chat tab; same-provider and mixed-provider sessions
retain independent state; concurrency obeys explicit limits/workspace rules; and
all agent operations flow through the three in-process Rust crates with no Node
ACP adapter dependency. Unproven provider features remain visibly unavailable,
not silently simulated or replaced by another backend.
