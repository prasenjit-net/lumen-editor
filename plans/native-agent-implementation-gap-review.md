# Native agent implementation and configuration gap review

Reviewed: 2026-10-19. Local revision: `6de9ded`.
Scope: updated native-session plan, current three-crate/app implementation, and
both Node ACP adapters. Review only; no implementation changes.

## Assessment

The implementation establishes a useful foundation: three Rust crates, typed
operations/events, native Codex/Claude launching, per-session models, a mixed
session sidebar, chat editor tabs, a turn scheduler and workspace write leases.
It has not completed the plan. Native integration is still development-gated,
and configuration is a small subset of the behavior exposed by the ACP adapters.

The highest-value next work is effective configuration discovery, typed/dynamic
options, consistent mutation semantics, and persistence/reconciliation. Adding
more hard-coded dropdowns would leave the underlying gaps intact.

## Findings, ordered by priority

### 1. High: user-facing agent settings do not configure the native path

Evidence: `src/app/native_agents.rs:16` constructs backends with literal `codex`
and `claude` executable names. It does not resolve the settings catalog's agent
paths or start mode. `src/app/agent_hub.rs:432` creates native sessions with an
empty ConfigPatch; `chat_send` sends text directly to the hub.

Meanwhile `crates/settings-core/src/agents.rs:17` advertises Codex start mode,
unsaved-file policy, change tracking and legacy adapter paths/launch methods.
Those remain meaningful to the old path, but they are not wired into the new
native launch/send flow. An installed CLI outside the GUI PATH cannot be selected
through a dedicated native-path setting. Native Send does not invoke the old
unsaved-buffer/change-snapshot controller just because those settings exist.

Action: add distinct native executable settings, resolve effective User/profile
and session defaults, wire document preflight and change review through the
native dispatch path, and mark legacy settings as migration-only when promoting.
Do not reinterpret an ACP executable path as a native CLI executable.

### 2. High: configuration restoration/effective-state reporting is incomplete

Evidence: `crates/agent-claude/src/backend.rs:543` attaches a known native session
using cached in-memory config or `default/default`. Session registry serialization
(`src/model/agent_registry.rs:260`) does not persist config. Claude's ordinary
`system` messages are ignored at `backend.rs:1099`, including initialization
metadata identifying model and permission mode.

Consequence: choose a model/mode, restart Lumen, resume the row, and the Lumen
config starts from defaults rather than the recorded choice. The launch path
passes the default permission mode and omits an explicit model for the `default`
alias. Native history may retain some state, but Lumen neither reconciles nor
truthfully distinguishes requested versus effective values. This conclusion is
from source inspection; that restart scenario was not executed against a live
account in this review.

Action: persist explicit per-session preferences separately from observed native
state; parse initialization/resume metadata; preserve unknown effective values;
resolve conflicts through a documented precedence policy. Display Pending /
Applied / Rejected rather than reporting a staged choice as confirmed execution.

### 3. High: common configuration types cannot describe full options

Evidence: `crates/agent-core/src/api.rs:183` uses string/string ConfigPatch pairs.
`ConfigOption` at line 216 has a flat list of string choices and one current
string. `src/ui/chat.rs:132` renders every option as a ComboBox.

Missing representations include Boolean values, grouped selections, choice
metadata, recommended/default distinction, disabled reasons and dependency
constraints. Scope and mutability exist in the types but the chat renderer does
not enforce or explain them. A checkbox could be emulated as strings, but the
contract would still lose its type and metadata.

Action: extend the contract before adding options; see the proposed contract
below. Keep the UI generic over schema types rather than branching on provider.

### 4. High: Codex drops the metadata needed for reasoning/plan/fast controls

Evidence: `crates/agent-codex/src/convert.rs:380` reduces native models to ID,
label and is-default. `session_config` at line 420 builds only Model, Sandbox and
Approvals. The create patch handler (`backend.rs:1069`) accepts only those keys;
`turn/start` at line 608 sends only their corresponding fields.

The implementation therefore cannot validate model-specific reasoning efforts,
show a collaboration/plan selector or expose model-supported fast mode. It also
loads models with a single `model/list` request (`backend.rs:851`) without
continuing a returned pagination cursor. Extended native approval/sandbox shapes
need an explicit support policy rather than a claim that three string values
cover every native setting.

Action: retain supported effort/default/modality metadata, paginate discovery,
implement version-matched turn/config mapping, and rebuild dependent options
when model, account capabilities or native configuration changes.

### 5. High: Claude's configuration catalog is static and incomplete

Evidence: `crates/agent-claude/src/convert.rs:209` fixes models to four aliases;
line 216 fixes permission modes to three values. It rejects other values in
`backend.rs:448`. Model and permission changes stage local state, then
`ensure_runtime` at line 640 retires/relaunches an idle process for the next turn.

No dynamic model metadata, effort, fast-mode eligibility, custom-agent picker,
model-dependent mode support or initialization-derived catalog is exposed.
A process restart is a legitimate fallback, but the UI must expose the restart
requirement and preserve resume/config state correctly. Existing internal control
responses are used for interrupt; model/permission control paths are not mapped.

Action: inventory the native version's discovery/control messages and implement
only verified operations. The Node adapter's SDK calls are behavioral references,
not proof that every equivalent CLI control is public/stable. Mark genuinely
unavailable settings explicitly; do not silently approximate permission modes.

### 6. Medium: mutation and acknowledgement semantics are too weak

Both `set_config` methods validate against current choices and immediately emit
ConfigChanged with a higher local revision. ReadConfig returns that local record.
This stages next-turn settings; it is not an effective-state query. There is no
complete requested/applied/rejected state or rollback when native application
fails. Mutability is not checked by those setters. The hub starts turns with
`expected_revision: None` (`src/app/agent_hub.rs:469`), so the API's revision guard
is bypassed in the application flow.

Action: make Apply timing explicit, validate whole patches atomically, preserve
last confirmed config, and send the expected config revision when dispatching.
Handle rapid edits without silently dropping one through stale revisions. Rebuild
all dependent choices in the same revision and report native rejection inline.

## ACP source comparison: what “full configurations” entails

Public main-branch source was downloaded and read into `/tmp/lumen-acp-review`.
Repository HEAD metadata observed during review:

- Codex ACP: `a62b56847f473a2be6c67ea779c1ef9c7f227c38`.
- Claude ACP: `bcf74c2fe8e32bd24714d45122529dcf2f99a279`.

Files were fetched from moving main before HEAD metadata; those hashes identify
observed HEADs, not a verified immutable snapshot of every downloaded file.
Re-fetch by commit before porting code. This review reads implementation, not
just project README feature lists. Licensing/attribution must be checked before
copying upstream code.

### Codex reference behavior

The adapter builds mode, collaboration mode, model, reasoning effort and
conditional fast-mode controls. Switching models preserves a supported effort or
selects that model's default, refreshes input/fast capabilities and republishes
options after resumed-model changes. Its option setter dispatches by config ID
and validates values; fast mode accepts Boolean or select fallback values.

Reference: [CodexAcpServer.ts](https://github.com/agentclientprotocol/codex-acp/blob/main/src/CodexAcpServer.ts),
particularly `applySessionConfigOption`, `applyModelChange`,
`applyResumedModels`, and `createSessionConfigOptions`.

### Claude reference behavior

The adapter derives options from model metadata, permissions and discovered
custom agents. It supports model-dependent effort and conditional fast mode,
canonicalizes model aliases, tolerates an effective model outside selectable
choices, and reconciles permissions after model switches. It applies native/SDK
changes before advertising the corresponding confirmed value. Effort handling
accounts for per-model settings, user-pinned values and explicit reset.

References:
[acp-agent.ts](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/src/acp-agent.ts)
(`setSessionConfigOption`, `applyConfigOptionValue`, `buildConfigOptions`),
[session-effort.ts](https://github.com/agentclientprotocol/claude-agent-acp/blob/main/src/session-effort.ts)
(default clearing and settings precedence).

These features demonstrate configuration lifecycle requirements. They do not
require Lumen to expose additional providers, custom gateways, every ACP extension,
or all SDK-specific controls. Preserve the requested two-provider scope.

| Configuration area | Codex native today | Claude native today | Target |
| --- | --- | --- | --- |
| Models | Discovered basic labels/IDs; one response page | Four fixed aliases | Native catalog, effective model, account/version availability |
| Reasoning/effort | Missing | Missing | Model-specific choices, defaults, reset and validation |
| Permission/execution mode | Three sandbox + three approval strings | Three modes | Faithful provider-specific choices and effective policy |
| Plan/collaboration | Missing | Plan permission mode exists | Distinguish collaboration mode from permissions; provider-specific mapping |
| Fast mode | Missing | Missing | Conditional availability, disabled reason and confirmed application |
| Custom agent/persona | Not exposed | Not exposed | Claude discovery/selection if native transport supports it |
| Applied config | Local desired state | Local desired state, init ignored | Requested, pending, confirmed and failed states |
| Resume config | Reads limited returned fields | Defaults after process/app-state loss | Persist user choices and reconcile native resume |
| Reset/inherit | No explicit clearing contract | `default` model alias only | Typed Reset/Inherit distinct from a literal value |
| Native executable | Literal command name | Literal command name | Explicit User Settings path and version checks |

## Other gaps against the updated plan

- **Native promotion incomplete:** `src/bootstrap.rs:73` requires
  `LUMEN_NATIVE_AGENTS`; the old ACP implementation and Cargo dependency remain.
  This is consistent with an internal migration phase, not completion of phase 6.
- **Workspace trust gate absent from native preparation:** opening a chat calls
  ensure_ready, which creates/resumes provider state without the planned agent
  project-configuration review. Task trust exists elsewhere but is not automatically
  applied to native agents. Native spawn inherits provider/project configuration.
- **History/discovery not joined end to end:** provider operations exist, but the
  hub has no ListSessions/ReadHistory dispatch. Claude explicitly declares both
  unsupported. The sidebar is primarily Lumen's local index, not a unified native
  history browser. No persisted transcript/draft cache is present in the registry.
- **Questions incomplete:** Claude declares questions unsupported and declines
  them. Codex reports version-gated support; test/native acceptance is still needed.
- **Tab prerequisite incomplete:** `src/app/chat_tabs.rs:4` explicitly hosts chats
  in the first group. General group assignment, chat drag/reopen history and split
  parity from phase 5a/5b are not complete just because a Chat tab target exists.
- **Composer and file workflow migration incomplete:** native Send currently
  accepts text only; attachment preparation, save policy and existing turn-change
  tracking are not wired into that path.
- **Runtime retention/acceptance:** no hub idle-detach policy is apparent. Native
  spike notes still list delayed approvals, Windows and descendant cleanup as
  unverified. Count unit tests separately from native release acceptance.

## Proposed next implementation contract

Keep the three crates. Expand core configuration around these explicit concepts:

1. `ConfigValue`: string/enum, Boolean, integer/number where needed; no secrets in
   ordinary config values. `ConfigChange`: Set(value), Reset, Inherit.
2. `ConfigOptionKind`: grouped select, Boolean, validated text/numeric controls
   only when a real provider setting needs them. Include category, description,
   ordering, enabled/disabled reason, recommended value and current effective value.
3. Model metadata: supported efforts/default, modalities and fast capability;
   permit unknown metadata without inventing support. Cache per provider/profile/
   executable version, invalidate on relevant account/native changes.
4. State: desired override versus observed native value, source/precedence,
   apply timing (live/next turn/restart), pending operation and rejection reason.
5. Atomic validation: a model+effort patch is evaluated together. A model switch
   recalculates dependent options, resets incompatible choices with feedback, and
   does not carry a broader permission policy accidentally.
6. Confirmation: publish local staging as pending. Publish effective changes only
   after a native acknowledgement/init report or label them unverified. A live
   failure preserves the last confirmed config; a next-turn failure stays visible.
7. Persistence: store explicit overrides and provenance; reconcile with native
   defaults/session state on attach. Never persist credentials or assume API and
   subscription authentication are interchangeable.
8. Scheduling: capture a config revision at actual queued-turn dispatch, revalidate
   it, and define what a config edit means for already queued messages.

Do not claim “all native configuration” without a bounded catalog. Maintain a
per-provider matrix: exposed editable controls, inherited native settings,
startup-only controls, unsupported fields and version-gated features. MCP servers,
hooks/plugins, extra directories and native settings files need separate trust,
launch and provenance handling, not unrestricted raw-JSON passthrough.

## Suggested order and tests

1. Wire native executable/default settings and document preflight; resolve the
   workspace trust gate before release promotion.
2. Implement typed config + desired/effective state and persistence/reconciliation.
3. Complete Codex model metadata, effort, collaboration and conditional fast mode.
4. Implement Claude dynamic discovery/init reconciliation, effort and fast mode;
   verify native control support before live switching/custom-agent promises.
5. Render capabilities and apply timing generically; finish history/attachments/
   group tabs/migration independently of configuration completeness.

Add behavioral tests for model-dependent effort choices, changing model and effort
atomically, native rejection, rapid config edits, active-turn mutation timing,
reset clearing an override, resume after restart, unknown current model,
model-list pagination, conditional fast availability, and one session's config
not changing another's. Use native CLI integration fixtures to prove actual apply
semantics, not only local option updates.

## Verification performed

Ran `cargo test -p lumen-agent-core -p lumen-agent-codex -p lumen-agent-claude`:
90 tests passed (31 Claude, 32 Codex, 27 core). Read current app/controller/UI and
upstream configuration code. No live model prompts, authenticated provider changes,
UI runtime tests or full workspace verification were performed for this review.
Only this review document was added; the implementation and original plan were
not changed. The unrelated untracked `.lumen/` directory was left untouched.
