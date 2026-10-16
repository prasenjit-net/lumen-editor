# Agent panel: native acceptance checklist

Automated tests cover logic, layout geometry and the accessibility tree, but not
how the panel looks or sounds on a real desktop. Run this on **macOS** and
**Windows**, once with each of Claude Code and Codex signed in, and tick or file
what differs. Use a folder that is a git repository.

## Setup
- [ ] Install an adapter (`npm install -g @agentclientprotocol/claude-agent-acp`,
  `@agentclientprotocol/codex-acp`) or use "Download with npx…".
- [ ] Launch Lumen **from the Dock / Start menu**, not a terminal, to prove the
  login-shell `PATH` lookup finds `node`/`npx`.
- [ ] Open a folder, then open the panel with the activity-bar button and Cmd/Ctrl+Shift+A.

## Look (check at narrow ≈300 px, default 380 px, wide ≈700 px panel widths)
- [ ] Header tabs, History and Close buttons stay on one row; nothing overlaps.
- [ ] Icons render (agent, send, stop, history, copy, check): no empty boxes.
- [ ] Status strip text wraps instead of clipping; the dot colors read well.
- [ ] Transcript: user bubble, markdown (code block + Copy, lists, inline code),
  "Thinking", tool rows, plan, notices.
- [ ] Permission card with a diff, the "Changed by the agent" card (collapsed and
  expanded), the unsaved-files card, the mode/model dropdowns and the confirmation
  for a risky mode.
- [ ] `@` list and `/` list appear above the box and the highlighted row is clear.
- [ ] Panel resize handle works; width is kept after closing and reopening.
- [ ] Activity bar: the attention badge shows while a permission is pending.

## Flow
- [ ] First message with no adapter shows install help; with the adapter it starts.
- [ ] Sign-in states: logged-out Claude shows CLI instructions; Codex shows its methods.
- [ ] Ask something, stop it midway (Stop), ask again.
- [ ] Ask for a file edit: card appears, **Esc** declines, **Yes** applies; the open
  tab updates; "Changed by the agent" lists it; **Revert** restores it.
- [ ] Edit a file in Lumen without saving, send a message: the unsaved-files card
  appears; try each choice.
- [ ] Select code → "Ask Agent About Selection"; Explorer → "Add to Agent Chat";
  a Problems row → agent icon.
- [ ] History: start a conversation, press New session, open History, resume it.
- [ ] Quit Lumen while an agent runs: no `node`/agent processes remain
  (Activity Monitor / Task Manager).

## Screen reader (VoiceOver on macOS, Narrator or NVDA on Windows)
- [ ] Every icon button is announced with a name (e.g. "Close agent panel"), never
  as a symbol.
- [ ] Tab order reaches: tabs, header buttons, option buttons on a permission card,
  composer, Send/Stop.
- [ ] A new permission card is discoverable (focus lands on its first option).
- [ ] Transcript text can be read and copied.

## Long conversations
- [ ] Resume or create a conversation with ~1000 messages: scrolling stays smooth and
  the newest message is visible when it finishes loading.
- [ ] Known limit: text selection cannot span messages that are scrolled out of view.
