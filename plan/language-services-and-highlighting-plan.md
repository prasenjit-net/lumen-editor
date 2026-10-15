# Language services and syntax-highlighting design

Status: Phase 1 parser-backed Rust/Go rendering underway. Updated: 2026-10-15.

## What this adds, and why syntax and LSP must remain separate

A language server is an external, long-lived process such as `rust-analyzer` or
`gopls`. Lumen starts one for an eligible workspace, speaks JSON-RPC over its
standard input/output, and synchronizes open documents with `initialize`,
`didOpen`, `didChange`, `didSave`, and `didClose`. The server then asynchronously
publishes diagnostics and answers requests such as hover, completion, definition,
references, rename, formatting, semantic tokens, and symbols. Lumen must treat
every response as stale unless it still matches the document/workspace revision
that requested it.

Do **not** merge language-server support and syntax highlighting into one system.
They should share language identity, document revisions, token classes, and theme
vocabulary, but have different failure, latency, and availability properties:

| Concern | Local parser/highlighter | Language server |
| --- | --- | --- |
| Availability | Always bundled for supported file types | Optional external executable and workspace toolchain |
| Latency | Immediate, incremental and deterministic | Asynchronous; startup/indexing may be slow |
| Purpose | Lexical/structural color, folding and bracket context | Project-aware meaning, diagnostics and editing/navigation actions |
| Failure | Fall back to plain text for that document | Show a non-blocking server error; never block editing |
| Result lifetime | Current text revision only | URI, server generation and document-version gated |

The correct composition is: parser highlighting draws a complete baseline first;
validated LSP semantic tokens selectively override compatible baseline classes;
diagnostics, inlay hints and navigation are independent visual layers. A missing
or crashed server therefore never removes usable coloring or folding. This also
addresses the present hand-written lexer’s incomplete token coverage without
making basic highlighting depend on Rust or Go tools being installed.

## Goals and non-goals

### Goals

- Create a reusable `lumen-language-services` crate for language registry,
  protocol/session state, document synchronization, response validation and
  parser-backed token snapshots. It must not depend on egui, eframe, native menu
  code, or Lumen filesystem/dialog services.
- Start with Rust (`rust-analyzer`) and Go (`gopls`) through the same descriptor
  interface used by all later languages.
- Replace the monolithic hand-written lexical highlighter gradually with embedded,
  parser-backed language resources and robust UTF-8/CRLF-safe token ranges.
- Offer multiple global code color schemes (initially VS Dark-inspired, VS
  Light-inspired and High Contrast) and language-scoped token-color overrides.
- Preserve current file detection, text, dirty state, undo/history, viewport,
  selection, accessibility and fallback behavior while the implementation moves.

### Non-goals for the first release

- Bundling, auto-downloading or silently upgrading `rust-analyzer`/`gopls`.
- Running arbitrary workspace scripts or trusting workspace-supplied server
  commands without explicit user configuration.
- Implementing every LSP request, debug adapter support, task runner, or a
  generic plugin marketplace.
- Removing the legacy lexical highlighter before parser fixtures and native
  rendering parity have been demonstrated.

## Target architecture

```text
settings-core schemas ──> app/settings ──> services/language_server (process + stdio)
                                               │ JSON-RPC / LSP
                                               ▼
                                     lumen-language-services crate
                              registry · sessions · revisions · token snapshots
                                               │ typed events/snapshots
                                               ▼
 app language controller ──> editor-core document identity/revision ──> editor-surface
                                                                  │
                                           parser tokens + semantic overlay + theme palette
```

### New crate: `crates/language-services`

Package name: `lumen-language-services`. It is the shared language-domain crate,
with no UI or native process APIs. Its public surface should stay narrow and
typed:

```rust
pub struct LanguageId(&'static str);
pub struct LanguageDescriptor { /* detection, parser resource, optional server */ }
pub struct DocumentKey { /* stable Lumen document ID + canonical URI */ }
pub struct DocumentVersion(u64);
pub struct TokenSnapshot { /* text revision, byte spans, token classes */ }
pub struct SemanticSnapshot { /* server generation/version, UTF-16 decoded spans */ }
pub struct DiagnosticSnapshot { /* server generation/version, diagnostics */ }

pub trait LanguageContribution: Send + Sync {
    fn descriptor(&self) -> &'static LanguageDescriptor;
    fn detect(&self, path: &std::path::Path) -> Detection;
    fn parser(&self) -> Option<&'static dyn SyntaxParser>;
    fn server(&self) -> Option<&'static ServerDescriptor>;
}
```

The crate owns no child process. It models server capabilities, JSON-RPC IDs,
document versions, token decoding, UTF-16/UTF-8 conversion, semantic-token
legend mapping, stale-result rejection, and pure parser/highlight functions.
Embedded grammar/query/definition resources live under this crate and are loaded
with `include_bytes!`/`include_str!`, so adding a supported language remains a
reviewable source change with no runtime asset lookup.

`services/language_server.rs` owns `std::process::Child`, stdio readers/writers,
threads/channels, timeouts, logging redaction, executable discovery and restart
backoff. `app/language_services.rs` owns workspace/session lifecycle, maps stable
Lumen document IDs to URIs and versions, polls typed service events, and dispatches
typed commands. `ui/` only renders snapshots and sends typed commands; it must
never write LSP JSON or start a server.

### Language registry and extension interface

Every language is a static contribution registered in one central catalog. A new
language must provide all applicable fields rather than add a `match` arm across
the app:

| Descriptor field | Purpose |
| --- | --- |
| `id`, label, aliases | Stable settings, display and LSP identifiers |
| filename/extension/glob rules | Deterministic detection; full filenames win over extensions |
| workspace markers | Root discovery, e.g. `Cargo.toml`, `go.mod`, `go.work` |
| parser resource and highlight queries | Immediate token/fold/bracket baseline |
| comment, string and pair rules | Editor commands and structural safety |
| optional `ServerDescriptor` | executable names, initialization options encoder, settings encoder, capability policy |
| semantic-token mapping | Standard/extension token names and modifiers mapped to Lumen token classes |
| tests/fixtures | Detection, parse ranges, styles, CRLF/Unicode and server-message fixtures |

Initial contributions:

- **Rust**: extensions `rs`, root marker `Cargo.toml`, embedded Rust parser/query
  resources, executable candidates `rust-analyzer`, and Rust-specific settings
  encoded as `initializationOptions`/configuration notifications.
- **Go**: extensions `go`, root markers `go.work` then `go.mod`, embedded Go
  parser/query resources, executable `gopls`, and Go-specific settings encoded
  through the same descriptor hook. Do not infer a Go workspace from an arbitrary
  parent folder when a nearer `go.work`/`go.mod` defines it.

Resource/build/config languages (CMake, Makefile, Dockerfile, TOML, YAML, JSON,
shell, `.env`, ignore files) use parser contributions even when they deliberately
have no language-server descriptor. This keeps those files high quality without
pretending an LSP is available.

## Protocol and lifecycle design

### Safe process policy

- Servers are opt-in per language and are resolved from an explicit settings path
  or a fixed executable name on `PATH`; Lumen never downloads one.
- One server session is keyed by `(language ID, canonical workspace root, server
  configuration fingerprint)`. Documents in different workspaces never share a
  session.
- Start lazily when an eligible document is opened or a language command is used.
  Stop on workspace close/app exit; restart only with exponential backoff and a
  visible status/error. A server crash cannot discard document state.
- Use JSON-RPC 2.0 `Content-Length` framing over stdio, bounded inbound message
  size, request deadlines/cancellation, and line-safe redacted logs. Never let a
  server response block an egui frame.

### Initialization and document sync

1. Resolve workspace root and server command; report a disabled/missing-server
   state without treating it as an editor error.
2. Send `initialize` with workspace folders, client capabilities limited to
   features Lumen actually supports, and descriptor-produced options. Wait for
   the result, send `initialized`, then configuration notifications when needed.
3. Send `didOpen` for every currently open eligible disk-backed document. Untitled
   buffers are deferred until a URI policy is deliberately designed; never invent
   a disk path or let a server write one.
4. Increment a per-session LSP document version for every accepted document
   revision. Phase 1 sends full-text `didChange` updates (`range` omitted), which
   avoids incorrect byte-to-UTF-16 incremental ranges. Phase 2 may add incremental
   sync only after exhaustive CRLF, emoji, combining-mark and surrogate-pair
   conversion tests.
5. Send `didSave` only after Lumen’s guarded write succeeds and `didClose` before
   a document/session is dropped. Workspace rename/open/close cancels outstanding
   requests and begins a new session generation.

All requests carry `(DocumentKey, document revision, LSP version, session
generation)`. Apply a reply only when all still match. For a `publishDiagnostics`
notification without a server-provided version, show it only if there is no newer
unsynchronized local revision; otherwise retain it as stale and request fresh
analysis rather than misplacing squiggles.

### Feature rollout order

1. Server availability/status and diagnostics.
2. Hover, document symbols and Go to Definition.
3. Completion and signature help, with explicit keyboard/focus ownership.
4. References, Rename preview/confirmation, formatting preview/apply, and code
   actions. Every workspace edit must be previewed, revision-validated and passed
   through Lumen’s guarded file/document workflow; never apply an LSP edit directly
   from UI code.
5. Semantic tokens, inlay hints and folding ranges only where their behavior is
   demonstrably better than the local parser and correctly maps to the shared
   viewport.

## Highlighting replacement and color design

### Token pipeline

1. Document/path detection selects a `LanguageDescriptor`.
2. The embedded parser produces a `TokenSnapshot` using source UTF-8 byte ranges;
   malformed/incomplete documents must still return safe partial tokens.
3. Tree-sitter-style highlight queries (or an equivalent parser/query engine
   chosen in the implementation spike) classify scopes into Lumen’s stable
   `TokenClass`: comment, string, escape, keyword, operator, punctuation, number,
   boolean, constant, type, function, method, parameter, variable, property,
   namespace, attribute, macro, tag, selector, heading and invalid.
4. The renderer maps byte ranges through the existing shaped viewport. Ranges must
   be grapheme-safe, CRLF-aware and never split a UTF-8 sequence.
5. When available, a validated semantic snapshot overlays only the classes it
   understands; e.g. a semantic `function` refines a lexical identifier, while a
   lexical string/comment always remains protected from accidental semantic paint.

Do not allow a theme to change parser semantics. Themes map stable token classes
to colors. Start with **Lumen Dark** (VS Code Dark+-inspired), **Lumen Light**
(Light+-inspired), and **Lumen High Contrast**. Add an optional schema-driven
`editor.token_color_customizations` layer with global and language-ID-specific
overrides. Thus code, resource and build files get correct grammar-specific token
classes, while users can intentionally customize their colors without each file
type inventing an inconsistent palette.

The existing `editor.syntax_highlighting` switch remains. During migration,
`editor-core::syntax` stays as a compatibility fallback; parser snapshots are
introduced behind a feature flag and compared against fixtures before becoming
default. Folding should consume parser structural nodes rather than duplicated
scanner assumptions once equivalence is proven.

## Settings schema

`settings-core` owns schemas/defaults/validation only. Proposed namespaces:

- `editor.color_scheme`: `lumen-dark`, `lumen-light`, `lumen-high-contrast`.
- `editor.semantic_highlighting`: enabled/disabled, default enabled once semantic
  snapshot validation is complete.
- `editor.token_color_customizations`: validated token-class/color maps, with
  optional language-ID sections.
- `language.rust.enabled`, `language.rust.server_path`,
  `language.rust.server_args`, `language.rust.settings`.
- `language.go.enabled`, `language.go.server_path`, `language.go.server_args`,
  `language.go.settings`.
- Shared: `language.server.trace`, startup timeout, restart policy and semantic
  token enablement. Numeric bounds, executable arguments and JSON settings values
  require strict validation; credentials/secrets are not settings.

User settings are the default server policy. Project settings may override a
language’s enablement/options for that opened root, with the existing project
priority. Display a scope indicator, resolved command (without secrets), server
state, capabilities and last error in Settings; do not silently run a command
specified by a project unless the user’s server-command policy permits it.

## Implementation phases

### Phase 0 — contracts and spike

- [x] Create the crate, `LanguageId`, Rust/Go descriptor registry, document
  versions, UTF-16/UTF-8 range conversion and ordered token-snapshot tests.
- [x] Add URI identity and stale session/revision rejection contracts before any
  LSP transport is introduced.
- [x] Add semantic-token legend decoding contracts before LSP transport.
- [x] Add a Rust/Go parser-query spike that emits disjoint UTF-8 token snapshots
  for ordinary, incomplete, Unicode and CRLF source. It uses the grammars'
  embedded highlight queries and preserves comments, strings, numbers, keywords,
  types and function categories.
- [x] Route Rust/Go parser spans through `editor-surface`, including the shaped
  renderer, while preserving its single viewport/layout authority. The legacy
  lexer remains the fallback for all other languages and parser errors.
- [ ] Measure incremental edit and shaping interactions before adding a snapshot
  cache or selecting parser highlighting as the default for further languages.
- [ ] Define token classes and three palettes in the existing theme system; add visual
  fixtures for code, config/resource and build files in dark/light/contrast modes.

### Phase 1 — parser-backed baseline highlighting

- Move language detection and embedded syntax resources into contributions.
- Ship Rust, Go, CMake, Makefile, Dockerfile, TOML, YAML and JSON parser/query
  resources first; retain fallback definitions for the remaining current formats.
- [x] Route parsed Rust/Go spans through `editor-surface` without changing its
  single viewport/layout authority. Compare old/new screenshots before expanding
  parser coverage to further languages.

### Phase 2 — generic LSP transport and diagnostics

- Implement stdio JSON-RPC in `services/`, capability negotiation in the new
  crate, server lifecycle in `app/`, and a compact Language Status view.
- Add Rust diagnostics with `rust-analyzer`, then Go diagnostics with `gopls`.
- Test missing executable, initialization failure, server crash/restart, malformed
  messages, workspace switch, stale diagnostics, dirty buffer, Save As and close.

### Phase 3 — semantic tokens and navigation

- Add semantic-token full responses, legend decoding, token-overlay precedence
  and revision gating for Rust and Go.
- Add hover, symbols and definition through the common request interface. Render
  failures as non-blocking status feedback and preserve editor focus/selection.

### Phase 4 — editing services and hardening

- Add completion/signature help, then references, rename preview, code-action and
  formatting transactions. Each mutating edit goes through the app’s existing
  save/conflict/document identity protections.
- Add native acceptance on macOS and Windows, representative large workspaces,
  offline/missing toolchains, multiple roots, project settings precedence, screen
  reader behavior and all color schemes.

## Acceptance and verification

- Pure crate tests: detection precedence, URI encoding, UTF-16 ↔ UTF-8 ranges,
  CRLF, emoji/combining characters, JSON-RPC framing, capability parsing, token
  legend decoding, overlay precedence and stale session/revision rejection.
- Parser fixtures: incomplete code, nested comments/strings, raw strings,
  interpolation, generics, attributes/macros, build/config syntax and invalid
  source. Assert exact source-byte ranges and no overlap/split graphemes.
- Service tests with fake stdio servers: initialize ordering, full sync versioning,
  cancellation, crash/restart, diagnostics with/without versions and bounded
  message handling. Never require installed Rust/Go tooling for unit tests.
- Integration tests: user/project setting precedence, Rust/Go root selection,
  open/save/rename/close synchronization, dirty-buffer protection and semantic
  overlay invalidation.
- UI/native tests: dark/light/high-contrast rendering at narrow/default/wide
  widths; diagnostics/hover/completion focus; no sidebar/panel auto-resize;
  macOS and Windows live checks. Windows compilation alone is not acceptance.

Run `cargo fmt --all --check`, `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings`, Windows compilation and
the relevant native-package check after each implementation increment.

## References

- [Language Server Protocol 3.18 specification](https://github.com/microsoft/language-server-protocol/blob/gh-pages/_specifications/lsp/3.18/specification.md)
  defines document synchronization, diagnostics and semantic-token capabilities.
- [rust-analyzer configuration](https://rust-analyzer.github.io/book/configuration)
  documents its LSP-based configuration and initialization options.
- [gopls documentation](https://go.dev/gopls/) identifies the official Go server,
  its workspace/toolchain requirements and IDE features.
