//! Agent controller: one ACP connection per supported agent (Claude Code, Codex).
//!
//! Phase 2 scope: connect, sign in, chat, cancel. Agents run in a *preview*
//! posture - permission requests are declined and the Codex session is put in its
//! read-only mode - until the edit-review workflow lands. Nothing here touches
//! documents or files.
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

    fn index(self) -> usize {
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

#[derive(Default)]
struct Slot {
    connection: Option<Box<dyn AgentConnection>>,
    tracker: RequestTracker,
    session_id: Option<String>,
    init: Option<InitializeResult>,
    /// Sent as soon as the session is ready (typed while still connecting).
    queued_prompt: Option<String>,
    last_prompt: Option<String>,
    handshake_timeout: Option<Duration>,
}

pub(crate) struct AgentRuntime {
    pub(crate) launcher: Arc<dyn AgentLauncher>,
    slots: [Slot; 2],
    /// Session-level permission to download the adapter with npx.
    npx_consent: [bool; 2],
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self {
            launcher: Arc::new(LocalAgentLauncher),
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

    /// Send the composer text. Starts the agent first if needed.
    pub(crate) fn send_agent_prompt(&mut self, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        let current_cwd = self.session_cwd();
        let session = self.view.agents.session_mut(kind);
        let text = session.draft.trim().to_owned();
        if text.is_empty() || session.running {
            return;
        }
        session.draft.clear();
        session.conversation.push_user_message(&text);
        session.follow_output = true;
        let cwd_changed = session.state == ConnectionState::Ready
            && session.cwd.as_deref() != Some(current_cwd.as_str());
        match self.view.agents.session(kind).state.clone() {
            ConnectionState::Ready if !cwd_changed => self.dispatch_prompt(kind, text),
            ConnectionState::Ready => {
                Self::agent_log(
                    kind,
                    LogLevel::Info,
                    "Folder changed; starting a new session",
                );
                self.stop_agent(kind);
                self.runtime_slot(kind).queued_prompt = Some(text);
                self.start_agent(kind, ctx);
            }
            ConnectionState::Starting | ConnectionState::NeedsAuth => {
                self.runtime_slot(kind).queued_prompt = Some(text);
            }
            ConnectionState::Idle | ConnectionState::Failed(_) | ConnectionState::NotFound(_) => {
                self.runtime_slot(kind).queued_prompt = Some(text);
                self.start_agent(kind, ctx);
            }
        }
    }

    pub(crate) fn cancel_agent_turn(&mut self) {
        let kind = self.view.agents.active;
        if !self.view.agents.session(kind).running {
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
            // Preview posture: Codex asks before editing, and Lumen declines.
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

    fn dispatch_prompt(&mut self, kind: AgentKind, text: String) {
        let Some(session_id) = self.runtime_slot(kind).session_id.clone() else {
            self.runtime_slot(kind).queued_prompt = Some(text);
            return;
        };
        let id = self.register(kind, "session/prompt", None);
        let slot = self.runtime_slot(kind);
        slot.last_prompt = Some(text.clone());
        self.view.agents.session_mut(kind).running = true;
        self.agent_send(
            kind,
            &requests::session_prompt(id, &session_id, vec![requests::text_block(&text)]),
        );
    }

    // ------------------------------------------------------------- polling

    pub(crate) fn poll_agents(&mut self, ctx: &egui::Context) {
        for kind in AgentKind::ALL {
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
                let session = self.view.agents.session_mut(kind);
                if session.conversation.apply(update) {
                    session.follow_output = true;
                }
            }
            Err(error) => Self::agent_log(kind, LogLevel::Warn, &error.to_string()),
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
        // Belt and braces for the read-only preview posture (also set via env).
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
}
