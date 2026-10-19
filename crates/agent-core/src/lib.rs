//! Common interface for native coding-agent backends (Codex and Claude Code).
//!
//! A Rust library linked into Lumen: typed operations, capabilities, streaming
//! events, requests for user input, session lifecycle and errors. ACP-inspired but
//! not the ACP wire protocol, and no separate process. Provider crates implement
//! [`AgentBackend`]; the application uses only these types.
//!
//! Dependency-free (std only) on purpose: provider crates bring their own wire
//! formats, and processes are started only through the injectable
//! [`launcher::ChildLauncher`].

pub mod api;
pub mod capabilities;
pub mod error;
pub mod event;
pub mod ids;
pub mod launcher;
pub mod lines;
pub mod model;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use api::*;
pub use capabilities::{Capabilities, Support};
pub use error::{AgentError, SubmitError};
pub use event::*;
pub use ids::*;
