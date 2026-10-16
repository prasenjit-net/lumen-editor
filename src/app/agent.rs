//! Agent controller: one ACP connection per supported agent (Claude Code, Codex).
//!
//! Connect, sign in, chat, cancel. Agents ask before editing or running
//! anything (Codex is started in its `read-only` mode so it asks too); requests
//! become permission cards, and what an agent changed is reviewed afterwards.
use super::LumenApp;
use crate::{
    model::output_log::{self, LogLevel},
    services::{
        agent_process::{
            AgentCommand, AgentConnection, AgentEvent, AgentLauncher, LocalAgentLauncher,
            cached_login_path, merge_path, resolve_program,
        },
        logger,
        terminal::default_working_directory,
    },
};
use eframe::egui;
use lumen_agent_protocol::{
    handshake::{check_version, is_auth_required},
    reducer::Conversation,
    requests,
    rpc::{
        Incoming, METHOD_NOT_FOUND, RequestId, RequestTracker, RpcError, parse_incoming,
        response_error,
    },
    types::{
        AuthMethod, InitializeResult, StopReason, parse_config_options, parse_initialize,
        parse_new_session, parse_stop_reason, parse_update,
    },
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
/// A first `npx` run downloads hundreds of megabytes before the agent can answer.
const NPX_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(600);
const STOP_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum AgentKind {
    Claude,
    Codex,
}

impl AgentKind {
    pub(crate) const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub(crate) fn binary(self) -> &'static str {
        match self {
            Self::Claude => "claude-agent-acp",
            Self::Codex => "codex-acp",
        }
    }

    /// The adapter versions the integration was verified against.
    pub(crate) fn npx_package(self) -> &'static str {
        match self {
            Self::Claude => "@agentclientprotocol/claude-agent-acp@0.92.0",
            Self::Codex => "@agentclientprotocol/codex-acp@2.2.2",
        }
    }

    pub(super) fn index(self) -> usize {
        match self {
            Self::Claude => 0,
            Self::Codex => 1,
        }
    }

    fn log_source(self) -> String {
        format!("agent:{}", self.id())
    }

    pub(crate) fn install_hint(self) -> String {
        format!(
            "Install the adapter once with: npm install -g {}  (needs Node.js 22 or newer), \
             or download and run it with npx. Sign in with the {} CLI first.",
            self.npx_package()
                .rsplit_once('@')
                .map_or(self.npx_package(), |(name, _)| name),
            self.label()
        )
    }

    pub(crate) fn sign_in_hint(self) -> &'static str {
        match self {
            Self::Claude => {
                "Not signed in. Run `claude` in a terminal and use /login, or set ANTHROPIC_API_KEY, then retry."
            }
            Self::Codex => "Not signed in. Choose a sign-in method below, then retry.",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionState {
    Idle,
    Starting,
    NeedsAuth,
    Ready,
    /// The adapter is not installed; the text explains how to get it.
    NotFound(String),
    Failed(String),
}

/// Everything the panel shows for one agent.
pub(crate) struct AgentSessionView {
    pub(crate) kind: AgentKind,
    /// Tool calls whose file was already shown (see `agent.follow_locations`).
    pub(crate) followed_calls: std::collections::HashSet<String>,
    /// Selections and files attached to the next message.
    pub(crate) attachments: Vec<super::agent_context::ComposerAttachment>,
    pub(crate) mention_selected: usize,
    pub(crate) slash_selected: usize,
    /// A risky config value waiting for confirmation: (config id, value, label).
    pub(crate) pending_config: Option<(String, String, String)>,
    /// A message waiting for the user's choice about unsaved files.
    pub(crate) held_prompt: Option<HeldPrompt>,
    /// Files the last turn changed, awaiting review.
    pub(crate) changes: Option<super::agent_changes::TurnChanges>,
    /// Taking the folder snapshot before sending the prompt.
    pub(crate) preparing: bool,
    /// Comparing the folder with the snapshot after the turn.
    pub(crate) finishing: bool,
    /// Requests the agent is blocked on, oldest first.
    pub(crate) pending_permissions: Vec<super::agent_permission::PendingPermission>,
    pub(crate) state: ConnectionState,
    pub(crate) conversation: Conversation,
    pub(crate) running: bool,
    pub(crate) auth_methods: Vec<AuthMethod>,
    pub(crate) draft: String,
    /// Ask the transcript to jump to the newest entry on the next frame.
    pub(crate) follow_output: bool,
    pub(crate) cwd: Option<String>,
    pub(crate) agent_name: Option<String>,
    pub(crate) focus_composer: bool,
    /// The request whose first option already received keyboard focus.
    pub(crate) permission_focus: Option<RequestId>,
}

impl AgentSessionView {
    fn new(kind: AgentKind) -> Self {
        Self {
            kind,
            followed_calls: Default::default(),
            attachments: Vec::new(),
            mention_selected: 0,
            slash_selected: 0,
            pending_config: None,
            held_prompt: None,
            changes: None,
            preparing: false,
            finishing: false,
            pending_permissions: Vec::new(),
            state: ConnectionState::Idle,
            conversation: Conversation::default(),
            running: false,
            auth_methods: Vec::new(),
            draft: String::new(),
            follow_output: false,
            cwd: None,
            agent_name: None,
            focus_composer: false,
            permission_focus: None,
        }
    }
}

pub(crate) struct AgentsView {
    pub(crate) open: bool,
    pub(crate) active: AgentKind,
    pub(crate) sessions: Vec<AgentSessionView>,
}

impl Default for AgentsView {
    fn default() -> Self {
        Self {
            open: false,
            active: AgentKind::Claude,
            sessions: AgentKind::ALL.map(AgentSessionView::new).into(),
        }
    }
}

impl AgentsView {
    /// True while any agent is blocked waiting for the user (drives the badge).
    pub(crate) fn needs_attention(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| !session.pending_permissions.is_empty())
    }

    pub(crate) fn session(&self, kind: AgentKind) -> &AgentSessionView {
        &self.sessions[kind.index()]
    }

    pub(crate) fn session_mut(&mut self, kind: AgentKind) -> &mut AgentSessionView {
        &mut self.sessions[kind.index()]
    }
}

/// One user message: the text plus context blocks (unsaved buffers).
#[derive(Clone, Debug, Default)]
pub(crate) struct Prompt {
    pub(crate) text: String,
    pub(crate) attachments: Vec<Value>,
    /// What the attachments are called, for the transcript note.
    pub(crate) labels: Vec<String>,
}

impl Prompt {
    pub(crate) fn new(text: String, context: Vec<(String, Value)>) -> Self {
        let (labels, attachments) = context.into_iter().unzip();
        Self {
            text,
            attachments,
            labels,
        }
    }
}

/// A message held back until the user decides what to do about unsaved files.
#[derive(Clone, Debug)]
pub(crate) struct HeldPrompt {
    pub(crate) text: String,
    /// Chips and @-mentions already collected for this message.
    pub(crate) context: Vec<(String, Value)>,
    /// The chips alone, put back if the user cancels.
    pub(crate) chips: Vec<super::agent_context::ComposerAttachment>,
    pub(crate) files: Vec<String>,
    /// Attaching is offered only when the agent accepts embedded context.
    pub(crate) can_attach: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnsavedChoice {
    SaveAll,
    Attach,
    SendAnyway,
    Cancel,
}

#[derive(Default)]
struct Slot {
    connection: Option<Box<dyn AgentConnection>>,
    tracker: RequestTracker,
    session_id: Option<String>,
    init: Option<InitializeResult>,
    /// Sent as soon as the session is ready (typed while still connecting).
    queued_prompt: Option<Prompt>,
    last_prompt: Option<Prompt>,
    handshake_timeout: Option<Duration>,
}

pub(crate) struct AgentRuntime {
    pub(crate) launcher: Arc<dyn AgentLauncher>,
    pub(super) tracking: [super::agent_changes::Tracking; 2],
    slots: [Slot; 2],
    /// Session-level permission to download the adapter with npx.
    npx_consent: [bool; 2],
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self {
            launcher: Arc::new(LocalAgentLauncher),
            tracking: Default::default(),
            slots: Default::default(),
            npx_consent: [false; 2],
        }
    }
}

impl Drop for AgentRuntime {
    /// Agents must not outlive the editor; this runs when the app is dropped.
    fn drop(&mut self) {
        for slot in &mut self.slots {
            if let Some(mut connection) = slot.connection.take() {
                connection.stop(Duration::from_millis(300));
            }
        }
    }
}

pub(crate) enum Launch {
    Ready {
        program: PathBuf,
        args: Vec<std::ffi::OsString>,
        via_npx: bool,
    },
    NotFound(String),
}

/// Decide how to start an adapter. Pure so every branch is testable.
pub(crate) fn resolve_launch(
    kind: AgentKind,
    command_setting: &str,
    launch_setting: &str,
    npx_consented: bool,
    path: Option<&std::ffi::OsStr>,
) -> Launch {
    let command = command_setting.trim();
    if !command.is_empty() {
        let program = PathBuf::from(command);
        return if program.is_absolute() && program.is_file() {
            Launch::Ready {
                program,
                args: Vec::new(),
                via_npx: false,
            }
        } else {
            Launch::NotFound(format!(
                "The configured {} adapter path is not an existing absolute file: {command}",
                kind.label()
            ))
        };
    }
    let want_npx = launch_setting == "npx" || (launch_setting == "auto" && npx_consented);
    if launch_setting != "npx"
        && let Some(program) = resolve_program(kind.binary(), path)
    {
        return Launch::Ready {
            program,
            args: Vec::new(),
            via_npx: false,
        };
    }
    if want_npx {
        return match resolve_program("npx", path) {
            Some(program) => Launch::Ready {
                program,
                args: vec!["-y".into(), kind.npx_package().into()],
                via_npx: true,
            },
            None => Launch::NotFound("npx was not found. Install Node.js 22 or newer.".to_owned()),
        };
    }
    Launch::NotFound(format!(
        "{} adapter ({}) was not found. {}",
        kind.label(),
        kind.binary(),
        kind.install_hint()
    ))
}

impl LumenApp {
    pub(crate) fn toggle_agent_panel(&mut self) {
        if !self.view.settings.boolean("agent.enabled") {
            self.view.status = "Coding agents are turned off in Settings (agent.enabled)".into();
            self.view.agents.open = false;
            return;
        }
        let agents = &mut self.view.agents;
        agents.open = !agents.open;
        if agents.open {
            let kind = agents.active;
            agents.session_mut(kind).focus_composer = true;
        }
    }

    pub(crate) fn select_agent(&mut self, kind: AgentKind) {
        self.view.agents.active = kind;
        self.view.agents.session_mut(kind).focus_composer = true;
    }

    fn session_cwd(&self) -> String {
        let root = self
            .editor
            .workspace
            .clone()
            .unwrap_or_else(default_working_directory);
        root.canonicalize().unwrap_or(root).display().to_string()
    }

    pub(super) fn agent_log(kind: AgentKind, level: LogLevel, message: &str) {
        logger::log(level, &kind.log_source(), message);
    }

    // ------------------------------------------------------------ commands

    /// Send the composer text. Unsaved buffers are handled first (agents read the
    /// disk); then the agent is started if needed.
    pub(crate) fn send_agent_prompt(&mut self, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        let session = self.view.agents.session_mut(kind);
        let text = session.draft.trim().to_owned();
        if text.is_empty() || session.running || session.held_prompt.is_some() {
            return;
        }
        // Chips the user attached, plus files named with @path in the text.
        let chips = std::mem::take(&mut session.attachments);
        session.draft.clear();
        let mut context: Vec<(String, Value)> = chips
            .iter()
            .map(|chip| (chip.label.clone(), chip.block.clone()))
            .collect();
        if let Some(root) = self.editor.workspace.clone() {
            for (label, block) in
                super::agent_context::mention_links(&text, &root, |path| path.is_file())
            {
                if !context.iter().any(|(existing, _)| *existing == label) {
                    context.push((label, block));
                }
            }
        }
        let unsaved = self.unsaved_files();
        let policy = self.view.settings.choice("agent.unsaved_files");
        if unsaved.is_empty() || policy == "ignore" {
            return self.submit_prompt(kind, Prompt::new(text, context), ctx);
        }
        let can_attach = self
            .runtime_slot(kind)
            .init
            .as_ref()
            .is_none_or(|init| init.capabilities.prompt.embedded_context);
        match policy.as_str() {
            "save" => self.save_for_agent(kind, text, context, ctx),
            "attach" if can_attach => {
                let mut context = context;
                context.extend(self.unsaved_attachments(kind));
                self.submit_prompt(kind, Prompt::new(text, context), ctx);
            }
            _ => {
                self.view.agents.session_mut(kind).held_prompt = Some(HeldPrompt {
                    text,
                    context,
                    chips,
                    files: unsaved,
                    can_attach,
                });
            }
        }
    }

    /// Names of open buffers with unsaved edits.
    fn unsaved_files(&self) -> Vec<String> {
        self.editor
            .files
            .iter()
            .filter(|document| document.dirty)
            .map(|document| document.name.clone())
            .collect()
    }

    /// Unsaved buffers as `resource` context blocks, within size limits.
    fn unsaved_attachments(&mut self, kind: AgentKind) -> Vec<(String, Value)> {
        const PER_FILE: usize = 200 * 1024;
        const TOTAL: usize = 1024 * 1024;
        let (mut blocks, mut total, mut skipped) = (Vec::new(), 0, 0);
        for document in self.editor.files.iter().filter(|document| document.dirty) {
            if document.text.len() > PER_FILE || total + document.text.len() > TOTAL {
                skipped += 1;
                continue;
            }
            total += document.text.len();
            let uri = match &document.path {
                Some(path) => super::language_services::file_uri(path),
                None => format!("untitled:{}", document.name),
            };
            blocks.push((
                format!("{} (unsaved)", document.name),
                requests::resource_block(&uri, &document.text, None),
            ));
        }
        if skipped > 0 {
            self.view
                .agents
                .session_mut(kind)
                .conversation
                .push_notice(&format!(
                    "{skipped} unsaved file(s) were too large to attach and were left out."
                ));
        }
        blocks
    }

    /// Save every unsaved file that has a path, then send. Untitled buffers have
    /// nowhere to be saved silently, so they are attached instead.
    fn save_for_agent(
        &mut self,
        kind: AgentKind,
        text: String,
        context: Vec<(String, Value)>,
        ctx: &egui::Context,
    ) {
        let active = self.editor.active_file;
        for index in 0..self.editor.files.len() {
            if self.editor.files[index].dirty && self.editor.files[index].path.is_some() {
                self.editor.active_file = Some(index);
                self.save_active();
            }
        }
        self.editor.active_file = active;
        let still_dirty: Vec<String> = self
            .editor
            .files
            .iter()
            .filter(|document| document.dirty && document.path.is_some())
            .map(|document| document.name.clone())
            .collect();
        if !still_dirty.is_empty() {
            let session = self.view.agents.session_mut(kind);
            session.draft = text;
            session.conversation.push_notice(&format!(
                "Not sent: could not save {}. Resolve it in the editor, then send again.",
                still_dirty.join(", ")
            ));
            return;
        }
        let mut context = context;
        if self.editor.files.iter().any(|document| document.dirty) {
            context.extend(self.unsaved_attachments(kind));
        }
        self.submit_prompt(kind, Prompt::new(text, context), ctx);
    }

    pub(crate) fn resolve_unsaved_choice(&mut self, choice: UnsavedChoice, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        let Some(held) = self.view.agents.session_mut(kind).held_prompt.take() else {
            return;
        };
        match choice {
            UnsavedChoice::Cancel => {
                let session = self.view.agents.session_mut(kind);
                session.draft = held.text;
                session.attachments = held.chips;
            }
            UnsavedChoice::SendAnyway => {
                self.submit_prompt(kind, Prompt::new(held.text, held.context), ctx);
            }
            UnsavedChoice::Attach => {
                let mut context = held.context;
                context.extend(self.unsaved_attachments(kind));
                self.submit_prompt(kind, Prompt::new(held.text, context), ctx);
            }
            UnsavedChoice::SaveAll => self.save_for_agent(kind, held.text, held.context, ctx),
        }
    }

    /// Record the message in the transcript and route it by connection state.
    fn submit_prompt(&mut self, kind: AgentKind, prompt: Prompt, ctx: &egui::Context) {
        let current_cwd = self.session_cwd();
        let session = self.view.agents.session_mut(kind);
        session.conversation.push_user_message(&prompt.text);
        if !prompt.labels.is_empty() {
            session
                .conversation
                .push_notice(&format!("Context: {}", prompt.labels.join(", ")));
        }
        session.follow_output = true;
        let cwd_changed = session.state == ConnectionState::Ready
            && session.cwd.as_deref() != Some(current_cwd.as_str());
        match self.view.agents.session(kind).state.clone() {
            ConnectionState::Ready if !cwd_changed => self.dispatch_prompt(kind, prompt),
            ConnectionState::Ready => {
                Self::agent_log(
                    kind,
                    LogLevel::Info,
                    "Folder changed; starting a new session",
                );
                self.stop_agent(kind);
                self.runtime_slot(kind).queued_prompt = Some(prompt);
                self.start_agent(kind, ctx);
            }
            ConnectionState::Starting | ConnectionState::NeedsAuth => {
                self.runtime_slot(kind).queued_prompt = Some(prompt);
            }
            ConnectionState::Idle | ConnectionState::Failed(_) | ConnectionState::NotFound(_) => {
                self.runtime_slot(kind).queued_prompt = Some(prompt);
                self.start_agent(kind, ctx);
            }
        }
    }

    pub(crate) fn cancel_agent_turn(&mut self) {
        let kind = self.view.agents.active;
        if !self.view.agents.session(kind).running {
            return;
        }
        if self.view.agents.session(kind).preparing {
            // Nothing was sent yet: just drop the message.
            self.cancel_snapshot(kind);
            let session = self.view.agents.session_mut(kind);
            session.running = false;
            session
                .conversation
                .push_notice("Cancelled before sending.");
            return;
        }
        if let Some(session_id) = self.runtime_slot(kind).session_id.clone() {
            self.agent_send(kind, &requests::session_cancel(&session_id));
            Self::agent_log(kind, LogLevel::Info, "Cancel requested");
            // The protocol requires answering every open permission request.
            self.cancel_pending_permissions(kind);
        }
    }

    /// Drop the connection and conversation; the next message starts fresh.
    pub(crate) fn new_agent_session(&mut self) {
        let kind = self.view.agents.active;
        self.stop_agent(kind);
        self.reset_tracking(kind);
        let session = self.view.agents.session_mut(kind);
        let draft = std::mem::take(&mut session.draft);
        *session = AgentSessionView::new(kind);
        session.draft = draft;
        session.focus_composer = true;
        self.runtime_slot(kind).queued_prompt = None;
        Self::agent_log(kind, LogLevel::Info, "Started a new session");
    }

    /// After signing in elsewhere, try again with the message that failed.
    pub(crate) fn retry_agent(&mut self, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        let slot = self.runtime_slot(kind);
        let pending = slot
            .last_prompt
            .clone()
            .or_else(|| slot.queued_prompt.clone());
        self.stop_agent(kind);
        self.runtime_slot(kind).queued_prompt = pending;
        self.view.agents.session_mut(kind).state = ConnectionState::Idle;
        self.start_agent(kind, ctx);
    }

    pub(crate) fn authenticate_agent(&mut self, method_id: &str) {
        let kind = self.view.agents.active;
        let id = self.register(kind, "authenticate", Some(HANDSHAKE_TIMEOUT));
        self.agent_send(kind, &requests::authenticate(id, method_id));
        self.view.agents.session_mut(kind).state = ConnectionState::Starting;
        Self::agent_log(
            kind,
            LogLevel::Info,
            &format!("Signing in with {method_id}"),
        );
    }

    /// Allow a one-session download of the adapter through npx, then start.
    pub(crate) fn allow_agent_npx(&mut self, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        self.agent_runtime.npx_consent[kind.index()] = true;
        Self::agent_log(
            kind,
            LogLevel::Info,
            "npx download allowed for this session",
        );
        self.start_agent(kind, ctx);
    }

    // ----------------------------------------------------------- lifecycle

    pub(super) fn agent_session_id(&mut self, kind: AgentKind) -> Option<String> {
        self.runtime_slot(kind).session_id.clone()
    }

    pub(super) fn agent_register_request(&mut self, kind: AgentKind, method: &str) -> i64 {
        self.register(kind, method, None)
    }

    fn runtime_slot(&mut self, kind: AgentKind) -> &mut Slot {
        &mut self.agent_runtime.slots[kind.index()]
    }

    fn register(&mut self, kind: AgentKind, method: &str, timeout: Option<Duration>) -> i64 {
        self.runtime_slot(kind)
            .tracker
            .register(method, Instant::now(), timeout)
    }

    pub(super) fn agent_send(&mut self, kind: AgentKind, message: &Value) {
        let result = match self.runtime_slot(kind).connection.as_mut() {
            Some(connection) => connection.send(message),
            None => Err("The agent is not running".to_owned()),
        };
        if let Err(error) = result {
            self.fail_agent(kind, error);
        }
    }

    pub(crate) fn start_agent(&mut self, kind: AgentKind, ctx: &egui::Context) {
        if self.runtime_slot(kind).connection.is_some() {
            return;
        }
        let prefix = format!("agent.{}", kind.id());
        let command = self.view.settings.user_text(&format!("{prefix}.command"));
        let launch = self.view.settings.choice(&format!("{prefix}.launch"));
        let path = merge_path(
            cached_login_path().map(|path| path.as_os_str()),
            std::env::var_os("PATH").as_deref(),
        );
        let consented = self.agent_runtime.npx_consent[kind.index()];
        let (program, args, via_npx) =
            match resolve_launch(kind, &command, &launch, consented, Some(&path)) {
                Launch::Ready {
                    program,
                    args,
                    via_npx,
                } => (program, args, via_npx),
                Launch::NotFound(message) => {
                    Self::agent_log(kind, LogLevel::Error, &message);
                    self.view.agents.session_mut(kind).state = ConnectionState::NotFound(message);
                    return;
                }
            };
        let cwd = self.session_cwd();
        let mut env = vec![("PATH".into(), path)];
        if kind == AgentKind::Codex {
            // Codex asks before editing in its read-only mode; Lumen shows the request.
            env.push(("INITIAL_AGENT_MODE".into(), "read-only".into()));
        }
        let agent_command = AgentCommand {
            program,
            args,
            env,
            cwd: PathBuf::from(&cwd),
        };
        let repaint = ctx.clone();
        let wake = Arc::new(move || repaint.request_repaint());
        match self.agent_runtime.launcher.launch(&agent_command, wake) {
            Ok(connection) => {
                Self::agent_log(
                    kind,
                    LogLevel::Info,
                    &format!("Started {} in {cwd}", agent_command.program.display()),
                );
                let timeout = if via_npx {
                    NPX_HANDSHAKE_TIMEOUT
                } else {
                    HANDSHAKE_TIMEOUT
                };
                let slot = self.runtime_slot(kind);
                slot.connection = Some(connection);
                slot.handshake_timeout = Some(timeout);
                let session = self.view.agents.session_mut(kind);
                session.state = ConnectionState::Starting;
                session.cwd = Some(cwd);
                let id = self.register(kind, "initialize", Some(timeout));
                self.agent_send(
                    kind,
                    &requests::initialize(id, env!("CARGO_PKG_VERSION"), false, false),
                );
            }
            Err(error) => self.fail_agent(kind, error),
        }
    }

    fn stop_agent(&mut self, kind: AgentKind) {
        let slot = self.runtime_slot(kind);
        if let Some(mut connection) = slot.connection.take() {
            connection.stop(STOP_GRACE);
        }
        slot.tracker.clear();
        slot.session_id = None;
        slot.init = None;
        self.cancel_snapshot(kind);
        let session = self.view.agents.session_mut(kind);
        session.running = false;
        session.pending_permissions.clear();
        if matches!(
            session.state,
            ConnectionState::Starting | ConnectionState::Ready
        ) {
            session.state = ConnectionState::Idle;
        }
    }

    fn fail_agent(&mut self, kind: AgentKind, error: String) {
        Self::agent_log(kind, LogLevel::Error, &error);
        self.stop_agent(kind);
        let session = self.view.agents.session_mut(kind);
        session.state = ConnectionState::Failed(error);
        session.conversation.finish_turn(true);
    }

    fn dispatch_prompt(&mut self, kind: AgentKind, prompt: Prompt) {
        if self.runtime_slot(kind).session_id.is_none() {
            self.runtime_slot(kind).queued_prompt = Some(prompt);
            return;
        }
        // Busy from now on, including while the folder snapshot is taken.
        self.view.agents.session_mut(kind).running = true;
        self.runtime_slot(kind).last_prompt = Some(prompt.clone());
        if !self.begin_tracked_turn(kind, &prompt) {
            self.send_prompt_now(kind, prompt);
        }
    }

    pub(super) fn send_prompt_now(&mut self, kind: AgentKind, prompt: Prompt) {
        let Some(session_id) = self.runtime_slot(kind).session_id.clone() else {
            return;
        };
        let id = self.register(kind, "session/prompt", None);
        // Context first, then what the user typed.
        let mut blocks = prompt.attachments;
        blocks.push(requests::text_block(&prompt.text));
        self.agent_send(kind, &requests::session_prompt(id, &session_id, blocks));
    }

    // ------------------------------------------------------------- polling

    pub(crate) fn poll_agents(&mut self, ctx: &egui::Context) {
        // Typing `@` needs the folder's file list; scan it the first time.
        let active = self.view.agents.active;
        if self.view.agents.open
            && super::agent_context::active_mention(&self.view.agents.session(active).draft)
                .is_some()
        {
            self.ensure_mention_index(ctx);
        }
        for kind in AgentKind::ALL {
            self.poll_snapshots(kind, ctx);
            let events = match self.runtime_slot(kind).connection.as_ref() {
                Some(connection) => connection.drain_events(),
                None => continue,
            };
            for event in events {
                if self.runtime_slot(kind).connection.is_none() {
                    break; // an earlier event ended the session
                }
                self.handle_agent_event(kind, event);
            }
            let expired = self.runtime_slot(kind).tracker.expire(Instant::now());
            if let Some((_, method)) = expired.first() {
                self.fail_agent(kind, format!("The agent did not answer {method} in time"));
            }
            if self.runtime_slot(kind).connection.is_some() {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    fn handle_agent_event(&mut self, kind: AgentKind, event: AgentEvent) {
        match event {
            AgentEvent::Message(value) => match parse_incoming(&value) {
                Ok(Incoming::Response { id, outcome }) => self.handle_response(kind, &id, outcome),
                Ok(Incoming::Request { id, method, params }) => {
                    self.handle_agent_request(kind, &id, &method, &params);
                }
                Ok(Incoming::Notification { method, params }) => {
                    self.handle_notification(kind, &method, &params);
                }
                Err(error) => Self::agent_log(kind, LogLevel::Warn, &error.to_string()),
            },
            AgentEvent::FrameError(error) => Self::agent_log(kind, LogLevel::Warn, &error),
            AgentEvent::Stderr(line) => {
                Self::agent_log(kind, output_log::classify_status(&line), &line);
            }
            AgentEvent::Exited => {
                let reason = self
                    .runtime_slot(kind)
                    .connection
                    .as_mut()
                    .and_then(|connection| connection.exit_reason().ok().flatten())
                    .unwrap_or_else(|| "Agent closed its output".to_owned());
                self.fail_agent(kind, reason);
            }
        }
    }

    fn handle_notification(&mut self, kind: AgentKind, method: &str, params: &Value) {
        if method != "session/update" {
            return; // extensions such as _auth/status_update are ignored
        }
        match parse_update(&params["update"]) {
            Ok(update) => {
                let tool_id = match &update {
                    lumen_agent_protocol::types::Update::ToolCall(call) => Some(call.id.clone()),
                    lumen_agent_protocol::types::Update::ToolCallUpdate(patch) => {
                        Some(patch.id.clone())
                    }
                    _ => None,
                };
                let session = self.view.agents.session_mut(kind);
                if session.conversation.apply(update) {
                    session.follow_output = true;
                }
                if let Some(id) = tool_id {
                    self.follow_tool_location(kind, &id);
                }
            }
            Err(error) => Self::agent_log(kind, LogLevel::Warn, &error.to_string()),
        }
    }

    /// With `agent.follow_locations` on, show the file an agent is working on. Each
    /// tool call is followed once, and only files inside the open folder.
    fn follow_tool_location(&mut self, kind: AgentKind, tool_id: &str) {
        if !self.view.settings.boolean("agent.follow_locations") {
            return;
        }
        let session = self.view.agents.session_mut(kind);
        if session.followed_calls.contains(tool_id) {
            return;
        }
        let Some(location) = session
            .conversation
            .tool_call(tool_id)
            .and_then(|call| call.locations.first().cloned())
        else {
            return;
        };
        let Some(root) = self.editor.workspace.clone() else {
            return;
        };
        let path = PathBuf::from(&location.path);
        if !path.starts_with(&root) || !path.is_file() {
            return;
        }
        self.view
            .agents
            .session_mut(kind)
            .followed_calls
            .insert(tool_id.to_owned());
        let existing = self
            .editor
            .files
            .iter()
            .position(|file| file.path.as_deref() == Some(path.as_path()));
        match existing {
            Some(index) => self.editor.active_file = Some(index),
            None => self.open_file(path),
        }
        if let (Some(document), Some(line)) = (self.editor.active(), location.line) {
            let byte = document
                .text
                .split_inclusive('\n')
                .take((line as usize).saturating_sub(1))
                .map(str::len)
                .sum();
            self.view.pending_goto = Some((document.id, byte));
        }
    }

    /// Permissions are queued for the user; every other client method is unsupported.
    fn handle_agent_request(
        &mut self,
        kind: AgentKind,
        id: &RequestId,
        method: &str,
        params: &Value,
    ) {
        if method == "session/request_permission" {
            self.queue_permission(kind, id, params);
        } else {
            Self::agent_log(
                kind,
                LogLevel::Info,
                &format!("Declined unsupported request {method}"),
            );
            let reply = response_error(id, METHOD_NOT_FOUND, "Method not supported by Lumen");
            self.agent_send(kind, &reply);
        }
    }

    fn handle_response(
        &mut self,
        kind: AgentKind,
        id: &RequestId,
        outcome: Result<Value, RpcError>,
    ) {
        let Some(method) = self.runtime_slot(kind).tracker.complete(id) else {
            return; // late or duplicate
        };
        match (method.as_str(), outcome) {
            ("initialize", Ok(result)) => self.on_initialized(kind, &result),
            ("initialize", Err(error)) => self.fail_agent(kind, error.message),
            ("authenticate", Ok(_)) => self.request_session(kind),
            ("authenticate", Err(error)) => {
                Self::agent_log(kind, LogLevel::Error, &error.message);
                let session = self.view.agents.session_mut(kind);
                session.state = ConnectionState::NeedsAuth;
                session
                    .conversation
                    .push_notice(&format!("Sign-in failed: {}", error.message));
            }
            ("session/new", Ok(result)) => self.on_session_ready(kind, &result),
            ("session/new", Err(error)) => self.on_session_error(kind, &error),
            ("session/prompt", Ok(result)) => self.on_turn_finished(kind, &result),
            ("session/prompt", Err(error)) => {
                self.view.agents.session_mut(kind).running = false;
                if is_auth_required(&error) {
                    self.require_auth(kind);
                } else {
                    Self::agent_log(kind, LogLevel::Error, &error.message);
                    let session = self.view.agents.session_mut(kind);
                    session.conversation.finish_turn(true);
                    session
                        .conversation
                        .push_notice(&format!("The agent reported an error: {}", error.message));
                }
            }
            ("session/set_config_option", Ok(result)) => {
                if let Some(options) = result.get("configOptions") {
                    self.view
                        .agents
                        .session_mut(kind)
                        .conversation
                        .config_options = parse_config_options(options);
                }
            }
            (other, Err(error)) => {
                Self::agent_log(
                    kind,
                    LogLevel::Warn,
                    &format!("{other} failed: {}", error.message),
                );
            }
            _ => {}
        }
    }

    fn on_initialized(&mut self, kind: AgentKind, result: &Value) {
        let init = match parse_initialize(result) {
            Ok(init) => init,
            Err(error) => return self.fail_agent(kind, error.to_string()),
        };
        if let Err(error) = check_version(&init) {
            return self.fail_agent(kind, error.to_string());
        }
        let session = self.view.agents.session_mut(kind);
        session.auth_methods = init.auth_methods.clone();
        session.agent_name = init
            .agent_title
            .clone()
            .map(|title| match &init.agent_version {
                Some(version) => format!("{title} {version}"),
                None => title,
            });
        Self::agent_log(
            kind,
            LogLevel::Info,
            &format!(
                "Connected to {}",
                session.agent_name.clone().unwrap_or_default()
            ),
        );
        self.runtime_slot(kind).init = Some(init);
        self.request_session(kind);
    }

    fn request_session(&mut self, kind: AgentKind) {
        let cwd = self
            .view
            .agents
            .session(kind)
            .cwd
            .clone()
            .unwrap_or_else(|| self.session_cwd());
        let timeout = self.runtime_slot(kind).handshake_timeout;
        let id = self.register(kind, "session/new", timeout);
        self.agent_send(kind, &requests::session_new(id, &cwd, Vec::new()));
    }

    fn on_session_error(&mut self, kind: AgentKind, error: &RpcError) {
        if is_auth_required(error) {
            self.require_auth(kind);
        } else {
            self.fail_agent(kind, error.message.clone());
        }
    }

    fn require_auth(&mut self, kind: AgentKind) {
        Self::agent_log(kind, LogLevel::Warn, "Authentication required");
        let slot = self.runtime_slot(kind);
        if slot.queued_prompt.is_none() {
            slot.queued_prompt = slot.last_prompt.clone();
        }
        self.view.agents.session_mut(kind).state = ConnectionState::NeedsAuth;
    }

    fn on_session_ready(&mut self, kind: AgentKind, result: &Value) {
        let new = match parse_new_session(result) {
            Ok(new) => new,
            Err(error) => return self.fail_agent(kind, error.to_string()),
        };
        self.runtime_slot(kind).session_id = Some(new.session_id.clone());
        let session = self.view.agents.session_mut(kind);
        session.state = ConnectionState::Ready;
        session.conversation.config_options = new.config_options.clone();
        Self::agent_log(
            kind,
            LogLevel::Info,
            &format!("Session {} ready", new.session_id),
        );
        // Belt and braces for starting in read-only (the env var above also sets it).
        if kind == AgentKind::Codex
            && new.config_options.iter().any(|option| {
                option.id == "mode"
                    && option.values.iter().any(|v| v.value == "read-only")
                    && option.current != "read-only"
            })
        {
            let id = self.register(kind, "session/set_config_option", None);
            self.agent_send(
                kind,
                &requests::set_config_option(id, &new.session_id, "mode", "read-only"),
            );
        }
        if let Some(prompt) = self.runtime_slot(kind).queued_prompt.take() {
            self.dispatch_prompt(kind, prompt);
        }
    }

    fn on_turn_finished(&mut self, kind: AgentKind, result: &Value) {
        let reason = parse_stop_reason(result);
        let session = self.view.agents.session_mut(kind);
        session.running = false;
        session
            .conversation
            .finish_turn(reason == StopReason::Cancelled);
        let notice = match reason {
            StopReason::Cancelled => Some("Cancelled."),
            StopReason::MaxTokens => Some("Stopped: the reply hit the token limit."),
            StopReason::MaxTurnRequests => Some("Stopped: too many model requests in one turn."),
            StopReason::Refusal => Some("The agent declined to continue."),
            StopReason::EndTurn | StopReason::Other => None,
        };
        if let Some(notice) = notice {
            session.conversation.push_notice(notice);
        }
        session.follow_output = true;
        self.runtime_slot(kind).last_prompt = None;
        self.begin_after_snapshot(kind);
    }
}

/// Used by the panel to show where the session runs.
pub(crate) fn display_folder(cwd: &str) -> String {
    Path::new(cwd).file_name().map_or_else(
        || cwd.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::command::Command;
    use crate::services::agent_process::fake::FakeAgentLauncher;
    use lumen_agent_protocol::reducer::Entry;
    use lumen_agent_protocol::reducer::Role;
    use lumen_agent_protocol::types::ToolKind;
    use lumen_agent_protocol::types::ToolStatus;
    use serde_json::json;

    struct Harness {
        app: LumenApp,
        fake: FakeAgentLauncher,
        ctx: egui::Context,
    }

    impl Harness {
        fn new() -> Self {
            let fake = FakeAgentLauncher::default();
            let mut app = LumenApp::default();
            app.agent_runtime.launcher = Arc::new(fake.clone());
            // Any existing absolute file satisfies "configured adapter"; the fake never runs it.
            let program = std::env::current_exe().unwrap().display().to_string();
            for kind in AgentKind::ALL {
                app.view.settings.user.persisted.insert(
                    format!("agent.{}.command", kind.id()),
                    lumen_settings_core::Value::Text(program.clone()),
                );
            }
            Self {
                app,
                fake,
                ctx: egui::Context::default(),
            }
        }

        fn say(&mut self, kind: AgentKind, text: &str) {
            self.app.view.agents.active = kind;
            self.app.view.agents.session_mut(kind).draft = text.to_owned();
            self.app.execute(Command::SendAgentPrompt, &self.ctx);
        }

        fn sent(&self) -> Vec<Value> {
            self.fake.sent.lock().unwrap().clone()
        }

        fn methods(&self) -> Vec<String> {
            self.sent()
                .iter()
                .filter_map(|m| m["method"].as_str().map(str::to_owned))
                .collect()
        }

        fn last_request(&self, method: &str) -> Value {
            self.sent()
                .into_iter()
                .rev()
                .find(|m| m["method"] == method)
                .unwrap_or_else(|| panic!("no {method} sent"))
        }

        fn reply(&mut self, request: &Value, result: Value) {
            self.push(AgentEvent::Message(
                json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
            ));
        }

        fn fail(&mut self, request: &Value, code: i64, message: &str) {
            self.push(AgentEvent::Message(
                json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":code,"message":message}}),
            ));
        }

        fn notify(&mut self, update: Value) {
            self.push(AgentEvent::Message(json!({
                "jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":update}
            })));
        }

        fn push(&mut self, event: AgentEvent) {
            self.fake.queued.lock().unwrap().push(event);
            self.app.poll_agents(&self.ctx);
        }

        fn state(&self, kind: AgentKind) -> ConnectionState {
            self.app.view.agents.session(kind).state.clone()
        }

        /// Walk a fresh agent through initialize + session/new.
        fn connect(&mut self, kind: AgentKind) {
            let init = self.last_request("initialize");
            self.reply(
                &init,
                json!({"protocolVersion":1,"agentInfo":{"title":kind.label(),"version":"9"},
                "authMethods":[], "agentCapabilities":{}}),
            );
            let new = self.last_request("session/new");
            self.reply(&new, json!({"sessionId":"s1","configOptions":[]}));
        }
    }

    #[test]
    fn a_message_starts_the_agent_then_prompts_once_the_session_is_ready() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "  explain this  ");
        assert_eq!(h.state(AgentKind::Claude), ConnectionState::Starting);
        assert_eq!(h.methods(), ["initialize"]);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
        let init = h.last_request("initialize");
        assert_eq!(init["params"]["protocolVersion"], 1);
        assert_eq!(init["params"]["clientCapabilities"]["terminal"], false);

        h.connect(AgentKind::Claude);
        assert_eq!(h.state(AgentKind::Claude), ConnectionState::Ready);
        let prompt = h.last_request("session/prompt");
        assert_eq!(prompt["params"]["sessionId"], "s1");
        assert_eq!(prompt["params"]["prompt"][0]["text"], "explain this");
        assert!(h.app.view.agents.session(AgentKind::Claude).running);
        assert!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .draft
                .is_empty()
        );

        h.notify(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"It "}}),
        );
        h.notify(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"works."}}));
        h.notify(json!({"sessionUpdate":"usage_update","used":3}));
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert_eq!(session.conversation.agent_text(), "It works.");
        assert!(session.follow_output);

        let prompt = h.last_request("session/prompt");
        h.reply(&prompt, json!({"stopReason":"end_turn"}));
        assert!(!h.app.view.agents.session(AgentKind::Claude).running);
        assert_eq!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .agent_name
                .as_deref(),
            Some("Claude Code 9")
        );
        // The user's text is echoed into the transcript exactly once.
        let echoes = h
            .app
            .view
            .agents
            .session(AgentKind::Claude)
            .conversation
            .entries
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Entry::Message {
                        role: Role::User,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(echoes, 1);
    }

    #[test]
    fn codex_sign_in_resumes_the_queued_message_and_forces_read_only() {
        let mut h = Harness::new();
        h.say(AgentKind::Codex, "hello");
        let init = h.last_request("initialize");
        h.reply(
            &init,
            json!({"protocolVersion":1,"authMethods":[
            {"id":"api-key","name":"API Key"},{"id":"chat-gpt","name":"ChatGPT"}]}),
        );
        let new = h.last_request("session/new");
        h.fail(&new, -32000, "Authentication required");
        assert_eq!(h.state(AgentKind::Codex), ConnectionState::NeedsAuth);
        let methods: Vec<_> = h
            .app
            .view
            .agents
            .session(AgentKind::Codex)
            .auth_methods
            .iter()
            .map(|m| m.id.clone())
            .collect();
        assert_eq!(methods, ["api-key", "chat-gpt"]);

        h.app
            .execute(Command::AuthenticateAgent("chat-gpt".into()), &h.ctx);
        let auth = h.last_request("authenticate");
        assert_eq!(auth["params"]["methodId"], "chat-gpt");
        h.reply(&auth, json!({}));
        let new = h.last_request("session/new");
        h.reply(&new, json!({"sessionId":"s1","configOptions":[
            {"id":"mode","name":"Mode","category":"mode","type":"select","currentValue":"agent",
             "options":[{"value":"read-only","name":"Read-only"},{"value":"agent","name":"Auto"}]}]}));
        let mode = h.last_request("session/set_config_option");
        assert_eq!(
            (
                mode["params"]["configId"].clone(),
                mode["params"]["value"].clone()
            ),
            (json!("mode"), json!("read-only"))
        );
        assert_eq!(
            h.last_request("session/prompt")["params"]["prompt"][0]["text"],
            "hello"
        );
        let command = h.fake.launched.lock().unwrap()[0].clone();
        assert!(
            command
                .env
                .iter()
                .any(|(k, v)| k == "INITIAL_AGENT_MODE" && v == "read-only")
        );
    }

    #[test]
    fn claude_reports_a_missing_login_at_prompt_time_and_retry_resends_the_message() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "hi");
        h.connect(AgentKind::Claude);
        let prompt = h.last_request("session/prompt");
        h.fail(&prompt, -32000, "Authentication required");
        assert_eq!(h.state(AgentKind::Claude), ConnectionState::NeedsAuth);
        assert!(!h.app.view.agents.session(AgentKind::Claude).running);

        h.app.execute(Command::RetryAgent, &h.ctx);
        assert_eq!(
            h.fake.launched.lock().unwrap().len(),
            2,
            "a fresh process after signing in"
        );
        assert_eq!(*h.fake.stops.lock().unwrap(), 1, "the old one was stopped");
        h.connect(AgentKind::Claude);
        assert_eq!(
            h.last_request("session/prompt")["params"]["prompt"][0]["text"],
            "hi"
        );
        assert_eq!(h.state(AgentKind::Claude), ConnectionState::Ready);
    }

    #[test]
    fn unsupported_client_methods_get_method_not_found() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "x");
        h.connect(AgentKind::Claude);
        h.push(AgentEvent::Message(json!({"jsonrpc":"2.0","id":9,"method":"fs/write_text_file","params":{"path":"/x","content":"y"}})));
        let reply = h
            .sent()
            .into_iter()
            .find(|m| m["id"] == 9)
            .expect("a reply");
        assert_eq!(reply["error"]["code"], METHOD_NOT_FOUND);
        h.push(AgentEvent::Message(
            json!({"jsonrpc":"2.0","id":10,"method":"terminal/create","params":{}}),
        ));
        assert_eq!(
            h.sent().into_iter().find(|m| m["id"] == 10).unwrap()["error"]["code"],
            METHOD_NOT_FOUND
        );
    }

    #[test]
    fn cancel_sends_the_notification_and_failed_calls_are_marked_when_the_turn_ends() {
        let mut h = Harness::new();
        h.say(AgentKind::Codex, "long job");
        h.connect(AgentKind::Codex);
        h.notify(json!({"sessionUpdate":"tool_call","toolCallId":"c1","title":"Run","kind":"execute","status":"in_progress"}));
        h.app.execute(Command::CancelAgentTurn, &h.ctx);
        let cancel = h.last_request("session/cancel");
        assert_eq!(cancel["params"]["sessionId"], "s1");
        assert!(cancel.get("id").is_none());
        let prompt = h.last_request("session/prompt");
        h.reply(&prompt, json!({"stopReason":"cancelled"}));
        let session = h.app.view.agents.session(AgentKind::Codex);
        assert!(!session.running);
        assert_eq!(
            session.conversation.tool_call("c1").unwrap().status,
            ToolStatus::Failed
        );
        assert!(
            matches!(session.conversation.entries.last(), Some(Entry::Notice(text)) if text == "Cancelled.")
        );
        // Cancelling when idle does nothing.
        let before = h.sent().len();
        h.app.execute(Command::CancelAgentTurn, &h.ctx);
        assert_eq!(h.sent().len(), before);
    }

    #[test]
    fn an_exited_agent_fails_visibly_and_the_next_message_starts_a_new_process() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "one");
        h.connect(AgentKind::Claude);
        h.push(AgentEvent::Stderr("something went wrong".into()));
        h.push(AgentEvent::FrameError("agent message is not JSON".into()));
        assert_eq!(
            h.state(AgentKind::Claude),
            ConnectionState::Ready,
            "noise is not fatal"
        );
        h.push(AgentEvent::Exited);
        assert!(matches!(
            h.state(AgentKind::Claude),
            ConnectionState::Failed(_)
        ));
        assert!(!h.app.view.agents.session(AgentKind::Claude).running);
        h.say(AgentKind::Claude, "two");
        assert_eq!(h.fake.launched.lock().unwrap().len(), 2);
        assert_eq!(h.state(AgentKind::Claude), ConnectionState::Starting);
    }

    #[test]
    fn launch_failures_and_old_protocol_versions_are_reported_not_hidden() {
        let mut h = Harness::new();
        *h.fake.fail_with.lock().unwrap() = Some("Could not start agent".into());
        h.say(AgentKind::Claude, "x");
        assert_eq!(
            h.state(AgentKind::Claude),
            ConnectionState::Failed("Could not start agent".into())
        );
        *h.fake.fail_with.lock().unwrap() = None;
        h.say(AgentKind::Claude, "y");
        let init = h.last_request("initialize");
        h.reply(&init, json!({"protocolVersion": 2}));
        let ConnectionState::Failed(message) = h.state(AgentKind::Claude) else {
            panic!("failed state")
        };
        assert!(message.contains("v2"), "{message}");
    }

    #[test]
    fn a_missing_adapter_explains_how_to_get_it_and_npx_needs_consent() {
        let mut h = Harness::new();
        h.app.view.settings.user.persisted.insert(
            "agent.claude.command".into(),
            lumen_settings_core::Value::Text("/definitely/not/installed".into()),
        );
        h.say(AgentKind::Claude, "x");
        assert!(
            matches!(h.state(AgentKind::Claude), ConnectionState::NotFound(m) if m.contains("not an existing absolute file"))
        );
        assert!(h.fake.launched.lock().unwrap().is_empty());
    }

    #[test]
    fn launch_resolution_prefers_installed_adapters_and_never_downloads_without_consent() {
        let directory = tempfile::tempdir().unwrap();
        let bin = |name: &str| {
            let path = directory.path().join(if cfg!(windows) {
                format!("{name}.cmd")
            } else {
                name.to_owned()
            });
            std::fs::write(&path, "").unwrap();
            path
        };
        let path = std::env::join_paths([directory.path()]).unwrap();
        // Nothing installed: auto without consent offers instructions only.
        assert!(
            matches!(resolve_launch(AgentKind::Claude, "", "auto", false, Some(&path)), Launch::NotFound(m) if m.contains("claude-agent-acp"))
        );
        // Consent but no npx: say so.
        assert!(
            matches!(resolve_launch(AgentKind::Claude, "", "auto", true, Some(&path)), Launch::NotFound(m) if m.contains("npx was not found"))
        );
        let npx = bin("npx");
        let Launch::Ready {
            program,
            args,
            via_npx,
        } = resolve_launch(AgentKind::Codex, "", "auto", true, Some(&path))
        else {
            panic!("npx launch")
        };
        assert_eq!((program, via_npx), (npx.clone(), true));
        assert_eq!(args, ["-y", "@agentclientprotocol/codex-acp@2.2.2"]);
        // "binary" never falls back to npx, even with consent.
        assert!(matches!(
            resolve_launch(AgentKind::Codex, "", "binary", true, Some(&path)),
            Launch::NotFound(_)
        ));
        // An installed adapter wins over npx unless npx was explicitly chosen.
        let adapter = bin("codex-acp");
        assert!(
            matches!(resolve_launch(AgentKind::Codex, "", "auto", true, Some(&path)), Launch::Ready { program, via_npx: false, .. } if program == adapter)
        );
        assert!(matches!(
            resolve_launch(AgentKind::Codex, "", "npx", false, Some(&path)),
            Launch::Ready { via_npx: true, .. }
        ));
        // An explicit path is used as-is, or rejected.
        let custom = bin("custom-adapter");
        assert!(
            matches!(resolve_launch(AgentKind::Codex, custom.to_str().unwrap(), "binary", false, None), Launch::Ready { program, .. } if program == custom)
        );
        assert!(matches!(
            resolve_launch(
                AgentKind::Codex,
                "relative/adapter",
                "auto",
                false,
                Some(&path)
            ),
            Launch::NotFound(_)
        ));
    }

    #[test]
    fn project_settings_cannot_choose_the_program_to_run() {
        let mut h = Harness::new();
        h.app.view.settings.root = Some("/workspace".into());
        h.app.view.settings.project.persisted.insert(
            "agent.claude.command".into(),
            lumen_settings_core::Value::Text("/tmp/evil-adapter".into()),
        );
        h.say(AgentKind::Claude, "x");
        let launched = h.fake.launched.lock().unwrap()[0].clone();
        assert_eq!(launched.program, std::env::current_exe().unwrap());
    }

    #[test]
    fn changing_folders_starts_a_fresh_session_for_the_next_message() {
        let mut h = Harness::new();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        h.app.editor.workspace = Some(first.path().to_path_buf());
        h.app.view.settings.user.persisted.insert(
            "agent.track_changes".into(),
            lumen_settings_core::Value::Bool(false),
        );
        h.say(AgentKind::Claude, "a");
        h.connect(AgentKind::Claude);
        let prompt = h.last_request("session/prompt");
        h.reply(&prompt, json!({"stopReason":"end_turn"}));
        h.app.editor.workspace = Some(second.path().to_path_buf());
        h.say(AgentKind::Claude, "b");
        assert_eq!(*h.fake.stops.lock().unwrap(), 1);
        let launches = h.fake.launched.lock().unwrap().clone();
        assert_eq!(launches.len(), 2);
        assert_eq!(launches[1].cwd, second.path().canonicalize().unwrap());
        h.connect(AgentKind::Claude);
        assert_eq!(
            h.last_request("session/new")["params"]["cwd"],
            second.path().canonicalize().unwrap().display().to_string()
        );
        assert_eq!(
            h.last_request("session/prompt")["params"]["prompt"][0]["text"],
            "b"
        );
    }

    #[test]
    fn new_session_clears_the_conversation_but_keeps_the_draft() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "hello");
        h.connect(AgentKind::Claude);
        h.notify(
            json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"Read","kind":"read"}),
        );
        h.app.view.agents.session_mut(AgentKind::Claude).draft = "half-typed".into();
        h.app.execute(Command::NewAgentSession, &h.ctx);
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert!(session.conversation.entries.is_empty());
        assert_eq!(
            (session.draft.as_str(), &session.state),
            ("half-typed", &ConnectionState::Idle)
        );
        assert_eq!(*h.fake.stops.lock().unwrap(), 1);
        let _ = ToolKind::Read;
    }

    #[test]
    fn only_one_turn_runs_at_a_time_and_empty_drafts_send_nothing() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "   ");
        assert!(h.fake.launched.lock().unwrap().is_empty());
        h.say(AgentKind::Claude, "first");
        h.connect(AgentKind::Claude);
        h.say(AgentKind::Claude, "second");
        let prompts = h
            .methods()
            .iter()
            .filter(|m| *m == "session/prompt")
            .count();
        assert_eq!(prompts, 1, "a second message is ignored while a turn runs");
        assert_eq!(h.app.view.agents.session(AgentKind::Claude).draft, "second");
    }

    #[test]
    fn agents_run_independently() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "to claude");
        h.say(AgentKind::Codex, "to codex");
        assert_eq!(h.fake.launched.lock().unwrap().len(), 2);
        assert_eq!(h.state(AgentKind::Claude), ConnectionState::Starting);
        assert_eq!(h.state(AgentKind::Codex), ConnectionState::Starting);
        assert_eq!(
            AgentKind::Claude.npx_package().split('@').nth(1),
            Some("agentclientprotocol/claude-agent-acp")
        );
    }

    fn dirty_document(
        name: &str,
        path: Option<std::path::PathBuf>,
        text: &str,
    ) -> crate::model::Document {
        let mut document = match path {
            Some(path) => crate::model::Document::from_path(path, "orig".into()),
            None => crate::model::Document::untitled(0),
        };
        document.name = name.to_owned();
        document.text = text.to_owned();
        document.dirty = true;
        document
    }

    fn prompt_blocks(h: &Harness) -> Vec<Value> {
        h.last_request("session/prompt")["params"]["prompt"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn unsaved_buffers_hold_the_message_until_the_user_chooses() {
        let mut h = Harness::new();
        h.app.editor.files.push(dirty_document(
            "a.rs",
            Some("/w/a.rs".into()),
            "fn changed() {}",
        ));
        h.say(AgentKind::Claude, "explain");
        let session = h.app.view.agents.session(AgentKind::Claude);
        let held = session.held_prompt.as_ref().expect("held");
        assert_eq!(
            (held.text.as_str(), held.files.clone()),
            ("explain", vec!["a.rs".to_owned()])
        );
        assert!(held.can_attach);
        assert!(session.draft.is_empty() && session.conversation.entries.is_empty());
        assert!(
            h.fake.launched.lock().unwrap().is_empty(),
            "nothing starts until the choice"
        );
        // A second Enter while held does nothing.
        h.app.view.agents.session_mut(AgentKind::Claude).draft = "again".into();
        h.app.execute(Command::SendAgentPrompt, &h.ctx);
        assert!(h.fake.launched.lock().unwrap().is_empty());

        h.app
            .execute(Command::ResolveUnsavedFiles(UnsavedChoice::Cancel), &h.ctx);
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert!(session.held_prompt.is_none());
        assert_eq!(session.draft, "explain", "the message goes back to the box");
    }

    #[test]
    fn attach_sends_unsaved_text_as_context_blocks_before_the_message() {
        let mut h = Harness::new();
        h.app.editor.files.push(dirty_document(
            "a.rs",
            Some("/w/a.rs".into()),
            "fn changed() {}",
        ));
        h.app
            .editor
            .files
            .push(dirty_document("Untitled-1", None, "scratch"));
        h.say(AgentKind::Claude, "review");
        h.app
            .execute(Command::ResolveUnsavedFiles(UnsavedChoice::Attach), &h.ctx);
        h.connect(AgentKind::Claude);
        let blocks = prompt_blocks(&h);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0]["type"], "resource");
        assert_eq!(blocks[0]["resource"]["uri"], "file:///w/a.rs");
        assert_eq!(blocks[0]["resource"]["text"], "fn changed() {}");
        assert_eq!(blocks[1]["resource"]["uri"], "untitled:Untitled-1");
        assert_eq!(blocks[2], json!({"type":"text","text":"review"}));
        let notices = format!(
            "{:?}",
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .conversation
                .entries
        );
        assert!(
            notices.contains("Context: a.rs (unsaved), Untitled-1 (unsaved)"),
            "{notices}"
        );
    }

    #[test]
    fn send_anyway_sends_only_the_text() {
        let mut h = Harness::new();
        h.app
            .editor
            .files
            .push(dirty_document("a.rs", Some("/w/a.rs".into()), "x"));
        h.say(AgentKind::Claude, "hi");
        h.app.execute(
            Command::ResolveUnsavedFiles(UnsavedChoice::SendAnyway),
            &h.ctx,
        );
        h.connect(AgentKind::Claude);
        assert_eq!(prompt_blocks(&h), vec![json!({"type":"text","text":"hi"})]);
    }

    #[test]
    fn save_all_writes_the_files_then_sends_and_a_failed_save_keeps_the_message() {
        let mut h = Harness::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "orig").unwrap();
        h.app
            .editor
            .files
            .push(dirty_document("a.rs", Some(path.clone()), "edited"));
        h.app.editor.active_file = Some(0);
        h.say(AgentKind::Claude, "go");
        h.app
            .execute(Command::ResolveUnsavedFiles(UnsavedChoice::SaveAll), &h.ctx);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited");
        assert!(!h.app.editor.files[0].dirty);
        h.connect(AgentKind::Claude);
        assert_eq!(prompt_blocks(&h).len(), 1, "saved, so nothing to attach");

        // The disk copy changed behind Lumen's back: the save is refused, nothing is sent.
        let mut h = Harness::new();
        let path = dir.path().join("b.rs");
        std::fs::write(&path, "someone else wrote this").unwrap();
        h.app
            .editor
            .files
            .push(dirty_document("b.rs", Some(path.clone()), "mine"));
        h.app.editor.active_file = Some(0);
        h.say(AgentKind::Claude, "go");
        h.app
            .execute(Command::ResolveUnsavedFiles(UnsavedChoice::SaveAll), &h.ctx);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "someone else wrote this"
        );
        assert!(h.fake.launched.lock().unwrap().is_empty());
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert_eq!(session.draft, "go");
        assert!(format!("{:?}", session.conversation.entries).contains("could not save b.rs"));
    }

    #[test]
    fn the_unsaved_files_setting_can_skip_the_question() {
        for (policy, expect_blocks) in [("attach", 2), ("ignore", 1)] {
            let mut h = Harness::new();
            h.app.view.settings.user.persisted.insert(
                "agent.unsaved_files".into(),
                lumen_settings_core::Value::Choice(policy.into()),
            );
            h.app
                .editor
                .files
                .push(dirty_document("a.rs", Some("/w/a.rs".into()), "x"));
            h.say(AgentKind::Claude, "hi");
            assert!(
                h.app
                    .view
                    .agents
                    .session(AgentKind::Claude)
                    .held_prompt
                    .is_none(),
                "{policy}"
            );
            h.connect(AgentKind::Claude);
            assert_eq!(prompt_blocks(&h).len(), expect_blocks, "{policy}");
        }
        // Clean buffers never ask.
        let mut h = Harness::new();
        h.app.editor.files.push(crate::model::Document::from_path(
            "/w/clean.rs".into(),
            "x".into(),
        ));
        h.say(AgentKind::Claude, "hi");
        assert!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .held_prompt
                .is_none()
        );
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
    }

    #[test]
    fn attaching_is_not_offered_when_the_agent_cannot_take_embedded_context() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "first");
        h.connect(AgentKind::Claude); // this agent advertised no embeddedContext
        let prompt = h.last_request("session/prompt");
        h.reply(&prompt, json!({"stopReason":"end_turn"}));
        h.app
            .editor
            .files
            .push(dirty_document("a.rs", Some("/w/a.rs".into()), "x"));
        h.say(AgentKind::Claude, "second");
        assert!(
            !h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .held_prompt
                .as_ref()
                .unwrap()
                .can_attach
        );
    }

    #[test]
    fn oversized_buffers_are_left_out_of_attachments_with_a_note() {
        let mut h = Harness::new();
        h.app.view.settings.user.persisted.insert(
            "agent.unsaved_files".into(),
            lumen_settings_core::Value::Choice("attach".into()),
        );
        h.app
            .editor
            .files
            .push(dirty_document("big.txt", None, &"x".repeat(300 * 1024)));
        h.app
            .editor
            .files
            .push(dirty_document("small.txt", None, "ok"));
        h.say(AgentKind::Claude, "hi");
        h.connect(AgentKind::Claude);
        let blocks = prompt_blocks(&h);
        assert_eq!(blocks.len(), 2, "one attachment plus the text");
        let notices = format!(
            "{:?}",
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .conversation
                .entries
        );
        assert!(
            notices.contains("1 unsaved file(s) were too large"),
            "{notices}"
        );
    }

    #[test]
    fn chips_and_mentions_travel_with_the_message_and_are_cleared_after() {
        let mut h = Harness::new();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() {}").unwrap();
        h.app.editor.workspace = Some(dir.path().to_path_buf());
        h.app.view.settings.user.persisted.insert(
            "agent.track_changes".into(),
            lumen_settings_core::Value::Bool(false),
        );
        h.app.add_attachment(
            crate::app::agent_context::selection_attachment(
                Some(&dir.path().join("src/b.rs")),
                "b.rs",
                "line1\nline2\n",
                &(0..5),
            )
            .unwrap(),
        );
        assert!(h.app.view.agents.open, "attaching opens the panel");
        h.say(AgentKind::Claude, "compare with @src/a.rs and @missing.rs");
        h.connect(AgentKind::Claude);
        let blocks = prompt_blocks(&h);
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert_eq!(blocks[0]["resource"]["text"], "line1");
        assert_eq!(blocks[1]["type"], "resource_link");
        assert!(blocks[1]["uri"].as_str().unwrap().ends_with("src/a.rs"));
        assert_eq!(blocks[2]["text"], "compare with @src/a.rs and @missing.rs");
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert!(session.attachments.is_empty());
        assert!(
            format!("{:?}", session.conversation.entries).contains("Context: b.rs:1, src/a.rs")
        );
    }

    #[test]
    fn cancelling_the_unsaved_question_puts_the_chips_back() {
        let mut h = Harness::new();
        h.app
            .add_attachment(crate::app::agent_context::file_attachment(
                Path::new("/w/a.rs"),
                Some(Path::new("/w")),
            ));
        h.app
            .editor
            .files
            .push(dirty_document("x.rs", Some("/w/x.rs".into()), "x"));
        h.say(AgentKind::Claude, "hi");
        assert!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .attachments
                .is_empty()
        );
        h.app
            .execute(Command::ResolveUnsavedFiles(UnsavedChoice::Cancel), &h.ctx);
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert_eq!(session.attachments.len(), 1);
        assert_eq!(session.draft, "hi");
    }

    #[cfg(feature = "shared-editor")]
    #[test]
    fn asking_about_the_selection_attaches_it_and_falls_back_to_the_file() {
        let mut h = Harness::new();
        h.app.execute(Command::AskAgentAboutSelection, &h.ctx);
        assert!(h.app.view.status.contains("Open a file"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        h.app.editor.workspace = Some(dir.path().to_path_buf());
        h.app.editor.files.push(crate::model::Document::from_path(
            path.clone(),
            "fn a() {}\nfn b() {}\n".into(),
        ));
        h.app.editor.active_file = Some(0);
        h.app.editor.files[0].enable_shared_editing();
        h.app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(
                vec![lumen_editor_core::Selection {
                    anchor: 10,
                    head: 19,
                }],
                0,
            )
            .unwrap();
        h.app.execute(Command::AskAgentAboutSelection, &h.ctx);
        let chips = &h.app.view.agents.session(AgentKind::Claude).attachments;
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].label, "lib.rs:2");
        assert_eq!(chips[0].block["resource"]["text"], "fn b() {}");

        // No selection: the file itself is attached as a link, once.
        h.app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .attachments
            .clear();
        h.app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(vec![lumen_editor_core::Selection::caret(3)], 0)
            .unwrap();
        h.app.execute(Command::AskAgentAboutSelection, &h.ctx);
        h.app.execute(Command::AskAgentAboutSelection, &h.ctx);
        let chips = &h.app.view.agents.session(AgentKind::Claude).attachments;
        assert_eq!(
            (chips.len(), chips[0].block["type"].clone()),
            (1, json!("resource_link"))
        );
        h.app
            .execute(Command::RemoveAgentAttachment(AgentKind::Claude, 0), &h.ctx);
        assert!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .attachments
                .is_empty()
        );
        h.app
            .execute(Command::RemoveAgentAttachment(AgentKind::Claude, 5), &h.ctx);
    }

    #[test]
    fn fix_with_agent_prefills_the_request_and_attaches_the_file() {
        let mut h = Harness::new();
        h.app.editor.workspace = Some("/w".into());
        let document = crate::model::Document::from_path(
            "/w/main.rs".into(),
            "fn main() {\n    oops\n}\n".into(),
        );
        let id = document.id;
        h.app.editor.files.push(document);
        h.app.view.language_server_diagnostics.insert(
            id,
            vec![lumen_language_services::diagnostics::Diagnostic {
                range: 16..20,
                severity: lumen_language_services::diagnostics::DiagnosticSeverity::Error,
                message: "cannot find value `oops`".into(),
                source: None,
                code: None,
            }],
        );
        h.app
            .execute(Command::FixProblemWithAgent(id, 16..20), &h.ctx);
        let session = h.app.view.agents.session(AgentKind::Claude);
        assert!(
            session.draft.contains("main.rs at line 2")
                && session.draft.contains("cannot find value `oops`")
        );
        assert_eq!(session.attachments[0].label, "main.rs");
        assert!(session.focus_composer && h.app.view.agents.open);
        assert!(
            h.fake.launched.lock().unwrap().is_empty(),
            "nothing is sent until the user does"
        );
    }

    #[test]
    fn options_are_changed_through_the_agent_and_risky_ones_ask_first() {
        let mut h = Harness::new();
        h.say(AgentKind::Claude, "hi");
        let init = h.last_request("initialize");
        h.reply(&init, json!({"protocolVersion":1}));
        let new = h.last_request("session/new");
        h.reply(
            &new,
            json!({"sessionId":"s1","configOptions":[
            {"id":"mode","name":"Mode","category":"mode","type":"select","currentValue":"default",
             "options":[{"value":"default","name":"Manual"},{"value":"plan","name":"Plan"},
                        {"value":"bypassPermissions","name":"Bypass permissions"}]}]}),
        );
        h.app.execute(
            Command::SetAgentConfig(AgentKind::Claude, "mode".into(), "plan".into()),
            &h.ctx,
        );
        let request = h.last_request("session/set_config_option");
        assert_eq!(
            (
                request["params"]["configId"].clone(),
                request["params"]["value"].clone()
            ),
            (json!("mode"), json!("plan"))
        );
        h.reply(&request, json!({"configOptions":[
            {"id":"mode","name":"Mode","currentValue":"plan","options":[{"value":"plan","name":"Plan"}]}]}));
        assert_eq!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .conversation
                .config_options[0]
                .current,
            "plan"
        );

        // A risky value is held until confirmed; declining sends nothing.
        let sent_before = h.sent().len();
        h.app
            .view
            .agents
            .session_mut(AgentKind::Claude)
            .conversation
            .config_options = lumen_agent_protocol::types::parse_config_options(&json!([
            {"id":"mode","name":"Mode","currentValue":"default",
             "options":[{"value":"default","name":"Manual"},{"value":"bypassPermissions","name":"Bypass permissions"}]}]));
        h.app.execute(
            Command::SetAgentConfig(AgentKind::Claude, "mode".into(), "bypassPermissions".into()),
            &h.ctx,
        );
        assert_eq!(h.sent().len(), sent_before);
        assert_eq!(
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .pending_config
                .as_ref()
                .unwrap()
                .2,
            "Bypass permissions"
        );
        h.app.execute(Command::ConfirmAgentConfig(false), &h.ctx);
        assert_eq!(h.sent().len(), sent_before);
        h.app.execute(
            Command::SetAgentConfig(AgentKind::Claude, "mode".into(), "bypassPermissions".into()),
            &h.ctx,
        );
        h.app.execute(Command::ConfirmAgentConfig(true), &h.ctx);
        assert_eq!(
            h.last_request("session/set_config_option")["params"]["value"],
            "bypassPermissions"
        );
        // Setting the current value again, or an unknown option, does nothing.
        let sent_after = h.sent().len();
        h.app.execute(
            Command::SetAgentConfig(AgentKind::Claude, "mode".into(), "default".into()),
            &h.ctx,
        );
        h.app.execute(
            Command::SetAgentConfig(AgentKind::Claude, "nope".into(), "x".into()),
            &h.ctx,
        );
        assert_eq!(h.sent().len(), sent_after);
    }

    #[test]
    fn picking_a_mention_or_command_completes_the_draft() {
        let mut h = Harness::new();
        h.app.view.agents.session_mut(AgentKind::Claude).draft = "look at @src/ma".into();
        h.app.execute(
            Command::PickAgentMention(AgentKind::Claude, "src/main.rs".into()),
            &h.ctx,
        );
        assert_eq!(
            h.app.view.agents.session(AgentKind::Claude).draft,
            "look at @src/main.rs "
        );
        h.app.view.agents.session_mut(AgentKind::Claude).draft = "/re".into();
        h.app.execute(
            Command::PickSlash(AgentKind::Claude, "review".into()),
            &h.ctx,
        );
        assert_eq!(
            h.app.view.agents.session(AgentKind::Claude).draft,
            "/review "
        );
    }

    #[test]
    fn following_the_agent_opens_files_inside_the_folder_once_and_only_when_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("o.rs"), "x").unwrap();
        let call = |id: &str, path: &std::path::Path, line: u32| {
            json!({"sessionUpdate":"tool_call","toolCallId":id,"title":"Read","kind":"read",
                "locations":[{"path":path.display().to_string(),"line":line}]})
        };

        let mut h = Harness::new();
        h.app.editor.workspace = Some(dir.path().to_path_buf());
        h.app.view.settings.user.persisted.insert(
            "agent.track_changes".into(),
            lumen_settings_core::Value::Bool(false),
        );
        h.say(AgentKind::Claude, "go");
        h.connect(AgentKind::Claude);
        h.notify(call("t1", &file, 2));
        assert!(h.app.editor.files.is_empty(), "off by default");

        h.app.view.settings.user.persisted.insert(
            "agent.follow_locations".into(),
            lumen_settings_core::Value::Bool(true),
        );
        h.notify(call("t2", &file, 3));
        assert_eq!(h.app.editor.files.len(), 1);
        let id = h.app.editor.files[0].id;
        assert_eq!(h.app.view.pending_goto, Some((id, 8)), "start of line 3");

        // The same call is followed once; another call to the open file just re-activates it.
        h.app.view.pending_goto = None;
        h.notify(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t2","status":"completed"}),
        );
        assert!(h.app.view.pending_goto.is_none());
        h.notify(call("t3", &file, 1));
        assert_eq!(h.app.editor.files.len(), 1);
        assert_eq!(h.app.view.pending_goto, Some((id, 0)));

        // Files outside the folder are never opened.
        h.notify(call("t4", &outside.path().join("o.rs"), 1));
        assert_eq!(h.app.editor.files.len(), 1);
    }

    /// End-to-end through the controller with a real adapter. Set
    /// `LUMEN_ACP_CLAUDE` / `LUMEN_ACP_CODEX` to the adapter command line
    /// (`node /path/to/dist/index.js`) and run
    /// `cargo test live_ -- --ignored`. Needs a logged-in agent.
    #[cfg(unix)]
    fn live_controller_turn(kind: AgentKind, variable: &str) {
        use std::os::unix::fs::PermissionsExt;
        let line = std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"));
        let directory = tempfile::tempdir().unwrap();
        let wrapper = directory.path().join("adapter.sh");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec {line} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(directory.path().to_path_buf());
        app.view.settings.user.persisted.insert(
            format!("agent.{}.command", kind.id()),
            lumen_settings_core::Value::Text(wrapper.display().to_string()),
        );
        let ctx = egui::Context::default();
        app.view.agents.active = kind;
        app.view.agents.session_mut(kind).draft =
            "Reply with only the word pong. Use no tools.".into();
        app.execute(Command::SendAgentPrompt, &ctx);
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            app.poll_agents(&ctx);
            let session = app.view.agents.session(kind);
            if let ConnectionState::Failed(error) | ConnectionState::NotFound(error) =
                &session.state
            {
                panic!("agent failed: {error}");
            }
            if session.state == ConnectionState::Ready
                && !session.running
                && !session.conversation.agent_text().is_empty()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out in state {:?}",
                session.state
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = app
            .view
            .agents
            .session(kind)
            .conversation
            .agent_text()
            .to_lowercase();
        assert!(text.contains("pong"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "needs a logged-in Claude adapter; see live_controller_turn"]
    fn live_claude_controller_turn() {
        live_controller_turn(AgentKind::Claude, "LUMEN_ACP_CLAUDE");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "needs a logged-in Codex adapter; see live_controller_turn"]
    fn live_codex_controller_turn() {
        live_controller_turn(AgentKind::Codex, "LUMEN_ACP_CODEX");
    }

    /// Full Phase 3 loop with a real agent: it asks permission to create a file, the
    /// test approves, and the change review lists the new file and can revert it.
    /// Needs `LUMEN_ACP_CLAUDE` / `LUMEN_ACP_CODEX` like the other live tests.
    #[cfg(unix)]
    fn live_edit_review(kind: AgentKind, variable: &str) {
        use std::os::unix::fs::PermissionsExt;
        let line = std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"));
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let wrapper = root
            .join("..")
            .join(format!("lumen-live-{}.sh", std::process::id()));
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec {line} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.view.settings.user.persisted.insert(
            format!("agent.{}.command", kind.id()),
            lumen_settings_core::Value::Text(wrapper.canonicalize().unwrap().display().to_string()),
        );
        let ctx = egui::Context::default();
        app.view.agents.active = kind;
        app.view.agents.session_mut(kind).draft =
            "Create a file named notes.txt in the current folder containing exactly: hi from agent. Use no other files.".into();
        app.execute(Command::SendAgentPrompt, &ctx);
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut approved = 0;
        loop {
            app.poll_agents(&ctx);
            // Approve what the agent asks, like a user clicking the first option.
            let pending = app
                .view
                .agents
                .session(kind)
                .pending_permissions
                .first()
                .cloned();
            if let Some(pending) = pending {
                let option = pending.request.options[0].id.clone();
                app.execute(
                    Command::AnswerAgentPermission(kind, pending.id, Some(option)),
                    &ctx,
                );
                approved += 1;
            }
            let session = app.view.agents.session(kind);
            if let ConnectionState::Failed(error) | ConnectionState::NotFound(error) =
                &session.state
            {
                panic!("agent failed: {error}");
            }
            if !session.running
                && !session.finishing
                && !session.preparing
                && session.changes.is_some()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out; state {:?}, approved {approved}",
                session.state
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let notes = root.join("notes.txt");
        assert_eq!(
            std::fs::read_to_string(&notes).unwrap().trim(),
            "hi from agent"
        );
        let rows = &app.view.agents.session(kind).changes.as_ref().unwrap().rows;
        assert!(rows.iter().any(|row| row.change.path == notes), "{rows:?}");
        app.execute(
            Command::AgentChanges(
                kind,
                crate::app::agent_changes::ChangeAction::Revert(notes.clone()),
            ),
            &ctx,
        );
        let _ = std::fs::remove_file(&wrapper);
        // Reverting an added file moves it to the Trash.
        assert!(!notes.exists(), "the added file was reverted");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "needs a logged-in Claude adapter; moves one temp file to the Trash"]
    fn live_claude_edit_review() {
        live_edit_review(AgentKind::Claude, "LUMEN_ACP_CLAUDE");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "needs a logged-in Codex adapter; moves one temp file to the Trash"]
    fn live_codex_edit_review() {
        live_edit_review(AgentKind::Codex, "LUMEN_ACP_CODEX");
    }
}
