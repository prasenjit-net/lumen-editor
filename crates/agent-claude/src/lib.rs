//! Native Claude Code backend: implements `lumen_agent_core::AgentBackend` over the
//! `claude` CLI's `stream-json` mode. No Node, no ACP adapter, no separate process
//! of ours: each session's child is started through the injected launcher.

mod backend;
mod convert;
mod wire;

pub use backend::{ClaudeBackend, ClaudeConfig};

#[cfg(test)]
mod tests;
