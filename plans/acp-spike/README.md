# ACP spike (Phase 0)

Throwaway client and captured transcripts for the agent integration plan
([../agent-integration-acp-plan.md](../agent-integration-acp-plan.md)).
Results: [FINDINGS.md](FINDINGS.md).

- `spike.mjs` - minimal Node ACP client. Spawns an adapter, advertises
  `fs` (and optionally `terminal`), creates a session, optionally prompts,
  answers agent requests inside the workspace only, and records every JSON-RPC
  line with a timestamp.
  `TRANSCRIPTS=<dir> node spike.mjs <name> node <adapter>/dist/index.js --cwd <dir>
  [--prompt TEXT] [--terminal] [--list] [--cancel-after MS] [--set id=value]`
- `transcripts/*.jsonl` - `{t (ms), dir (in|out|stderr|exit), msg}` per line,
  `*.summary.json` - initialize / session/new / prompt results. Sanitized:
  home paths, account e-mail, and the user's unrelated sessions were removed.
  Phase 1 moves these under `crates/agent-protocol/fixtures/`.

Adapters used: `@agentclientprotocol/claude-agent-acp` 0.92.0 and
`@agentclientprotocol/codex-acp` 2.2.2, Node 26.5, macOS arm64.
