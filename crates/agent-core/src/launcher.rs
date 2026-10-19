//! How the libraries start processes. Core owns no process code: Lumen implements
//! [`ChildLauncher`] (process groups, polite and forced tree stop, PATH lookup) and
//! tests inject a fake. This keeps agent crates independent of application
//! services and lets the task runner share the same OS helpers.

use std::{ffi::OsString, path::PathBuf, sync::Arc};

/// A callback that asks the UI to repaint when output arrives.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// The program was not found or is not executable.
    NotFound(String),
    Failed(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(detail) => write!(f, "not found: {detail}"),
            Self::Failed(detail) => write!(f, "could not start: {detail}"),
        }
    }
}

impl std::error::Error for LaunchError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChildEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    StdoutClosed,
    StderrClosed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

pub trait ChildLauncher: Send + Sync {
    fn launch(&self, spec: &ChildSpec, wake: Wake) -> Result<Box<dyn Child>, LaunchError>;
}

/// A running child. Nothing here blocks on the child.
pub trait Child: Send {
    fn write(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Close stdin (the polite "no more input" signal).
    fn close_stdin(&mut self);
    /// Queued output, at most about `max_bytes`, always at least one event if any
    /// is queued.
    fn drain(&mut self, max_bytes: usize) -> Vec<ChildEvent>;
    /// `Some` once the process itself has exited (its pipes may still hold output).
    fn try_exit(&mut self) -> Result<Option<ExitInfo>, String>;
    /// Ask the whole process tree to stop; true when the request was delivered.
    fn request_stop(&mut self) -> bool;
    /// Kill the whole tree now; does not wait.
    fn force_stop(&mut self) -> bool;
}
