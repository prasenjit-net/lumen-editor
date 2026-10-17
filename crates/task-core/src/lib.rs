//! Pure task schema, TOML codec, validation, variable checks and dependency
//! planning. No egui, native processes or filesystem I/O: storage adapters live
//! in `services/` and orchestration in `app/`.

mod digest;
mod edit;
mod parse;
mod plan;
mod resolve;
mod validate;
mod vars;

use std::collections::BTreeMap;

pub use digest::execution_digest;
pub use edit::{EditError, existing_ids, new_file, remove_task, upsert_task};
pub use parse::parse;
pub use plan::{Catalog, Plan, PlanError, PlanNode};
pub use resolve::{Program, ResolveContext, ResolveError, ResolvedTask, expand, resolve};
pub use validate::{validate_file, validate_task};
pub use vars::{Variable, find_variables};

pub const SCHEMA_VERSION: i64 = 1;
/// Concurrent-process limits accepted by settings; kept here so the scheduler and
/// validation agree.
pub const MIN_PARALLELISM: u32 = 1;
pub const MAX_PARALLELISM: u32 = 32;
pub const DEFAULT_PARALLELISM: u32 = 4;
/// Readiness waits always need a finite deadline.
pub const DEFAULT_READY_TIMEOUT_SECONDS: u32 = 60;
pub const MAX_READY_TIMEOUT_SECONDS: u32 = 3600;
pub const MAX_ID_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    User,
    Workspace,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Workspace => "Workspace",
        }
    }
}

/// Identity of a task: `(scope, id)`, independent of its label.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskKey {
    pub scope: Scope,
    pub id: String,
}

impl TaskKey {
    pub fn new(scope: Scope, id: impl Into<String>) -> Self {
        Self {
            scope,
            id: id.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    Process,
    Shell,
    Compound,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Shell => "shell",
            Self::Compound => "compound",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Pipe,
    Pty,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Pty => "pty",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavePolicy {
    Prompt,
    Save,
    None,
}

impl SavePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Save => "save",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstancePolicy {
    Single,
    Parallel,
}

impl InstancePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Parallel => "parallel",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependsOrder {
    Sequence,
    Parallel,
}

impl DependsOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sequence => "sequence",
            Self::Parallel => "parallel",
        }
    }
}

/// What a task group does when one of its tasks fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailurePolicy {
    /// Stop running siblings and skip everything not yet started (default).
    FailFast,
    /// Let independent tasks finish; dependents of the failure are still skipped.
    Continue,
}

impl FailurePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FailFast => "fail_fast",
            Self::Continue => "continue",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveal {
    Always,
    Silent,
    Never,
}

impl Reveal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Silent => "silent",
            Self::Never => "never",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputReuse {
    Dedicated,
    New,
}

impl OutputReuse {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dedicated => "dedicated",
            Self::New => "new",
        }
    }
}

/// Every field is optional so a definition remembers what the file stated
/// explicitly; the `effective_*` accessors apply the documented defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    pub reveal: Option<Reveal>,
    pub focus: Option<bool>,
    pub reuse: Option<OutputReuse>,
    pub clear: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Background {
    pub ready_pattern: String,
    pub ready_timeout_seconds: Option<u32>,
}

/// Execution fields a platform may override; never identity or graph structure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlatformOverride {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
}

impl PlatformOverride {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    pub const ALL: [Platform; 3] = [Self::Windows, Self::Macos, Self::Linux];

    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskDef {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub kind: TaskKind,
    /// Executable (process) or command string (shell); absent for compound.
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub unset_env: Vec<String>,
    pub group: Option<String>,
    pub is_default: bool,
    pub depends_on: Vec<String>,
    pub depends_order: Option<DependsOrder>,
    pub on_failure: Option<FailurePolicy>,
    pub transport: Option<Transport>,
    pub save: Option<SavePolicy>,
    pub instance_policy: Option<InstancePolicy>,
    pub exit_codes: Option<Vec<i32>>,
    pub timeout_seconds: Option<u32>,
    pub is_background: bool,
    pub background: Option<Background>,
    pub presentation: Presentation,
    pub windows: PlatformOverride,
    pub macos: PlatformOverride,
    pub linux: PlatformOverride,
}

impl TaskDef {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: TaskKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            kind,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            unset_env: Vec::new(),
            group: None,
            is_default: false,
            depends_on: Vec::new(),
            depends_order: None,
            on_failure: None,
            transport: None,
            save: None,
            instance_policy: None,
            exit_codes: None,
            timeout_seconds: None,
            is_background: false,
            background: None,
            presentation: Presentation::default(),
            windows: PlatformOverride::default(),
            macos: PlatformOverride::default(),
            linux: PlatformOverride::default(),
        }
    }

    pub fn platform_override(&self, platform: Platform) -> &PlatformOverride {
        match platform {
            Platform::Windows => &self.windows,
            Platform::Macos => &self.macos,
            Platform::Linux => &self.linux,
        }
    }

    pub fn platform_override_mut(&mut self, platform: Platform) -> &mut PlatformOverride {
        match platform {
            Platform::Windows => &mut self.windows,
            Platform::Macos => &mut self.macos,
            Platform::Linux => &mut self.linux,
        }
    }

    /// Process runs use pipes, shell runs a PTY, unless stated otherwise.
    pub fn effective_transport(&self) -> Transport {
        self.transport.unwrap_or(match self.kind {
            TaskKind::Shell => Transport::Pty,
            _ => Transport::Pipe,
        })
    }
    pub fn effective_save(&self) -> SavePolicy {
        self.save.unwrap_or(SavePolicy::Prompt)
    }
    pub fn effective_instance_policy(&self) -> InstancePolicy {
        self.instance_policy.unwrap_or(InstancePolicy::Single)
    }
    pub fn effective_failure_policy(&self) -> FailurePolicy {
        self.on_failure.unwrap_or(FailurePolicy::FailFast)
    }
    pub fn effective_depends_order(&self) -> DependsOrder {
        self.depends_order.unwrap_or(DependsOrder::Sequence)
    }
    pub fn effective_exit_codes(&self) -> Vec<i32> {
        self.exit_codes.clone().unwrap_or_else(|| vec![0])
    }
    pub fn effective_reveal(&self) -> Reveal {
        self.presentation.reveal.unwrap_or(Reveal::Always)
    }
    pub fn effective_focus(&self) -> bool {
        self.presentation.focus.unwrap_or(false)
    }
    pub fn effective_reuse(&self) -> OutputReuse {
        self.presentation.reuse.unwrap_or(OutputReuse::Dedicated)
    }
    pub fn effective_clear(&self) -> bool {
        self.presentation.clear.unwrap_or(true)
    }
    pub fn effective_ready_timeout(&self) -> Option<u32> {
        self.background.as_ref().map(|background| {
            background
                .ready_timeout_seconds
                .unwrap_or(DEFAULT_READY_TIMEOUT_SECONDS)
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskFile {
    pub tasks: Vec<TaskDef>,
}

impl TaskFile {
    pub fn get(&self, id: &str) -> Option<&TaskDef> {
        self.tasks.iter().find(|task| task.id == id)
    }
}

/// One actionable problem, addressed to a task and field so forms can place it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub task: Option<String>,
    pub field: String,
    pub message: String,
}

impl Issue {
    pub(crate) fn file(field: &str, message: impl Into<String>) -> Self {
        Self {
            task: None,
            field: field.into(),
            message: message.into(),
        }
    }
    pub(crate) fn task(task: &str, field: &str, message: impl Into<String>) -> Self {
        Self {
            task: Some(task.into()),
            field: field.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.task {
            Some(task) => write!(f, "task '{task}', {}: {}", self.field, self.message),
            None => write!(f, "{}: {}", self.field, self.message),
        }
    }
}

#[cfg(test)]
mod tests;
