//! Native Codex backend: implements `lumen_agent_core::AgentBackend` over the
//! `codex app-server` stdio protocol. No Node, no ACP adapter, no separate process
//! of ours: the app-server child is started through the injected launcher.

mod backend;
mod convert;
mod wire;

pub use backend::{CodexBackend, CodexConfig};

#[cfg(test)]
mod tests;
