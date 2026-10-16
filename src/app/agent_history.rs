//! Earlier conversations: list the agent's own sessions for this folder and
//! resume one with `session/load`.
//!
//! The agent owns the history (it stores the transcript); Lumen keeps nothing
//! beyond the list it just fetched, so no conversation text is persisted here.
use super::{
    LumenApp,
    agent::{AgentKind, ConnectionState},
};
use crate::model::output_log::LogLevel;
use eframe::egui;
use lumen_agent_protocol::{
    reducer::Conversation,
    requests,
    rpc::RpcError,
    types::{SessionSummary, parse_loaded_session, parse_session_page},
};
use serde_json::Value;
use std::time::Duration;

/// Replaying a long conversation can take a while.
const LOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
pub(crate) struct HistoryView {
    /// The list replaces the transcript while open.
    pub(crate) open: bool,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
    pub(crate) sessions: Vec<SessionSummary>,
    pub(crate) next_cursor: Option<String>,
    /// False when the agent does not offer `session/list`.
    pub(crate) unsupported: bool,
}

/// Pending history work for a connection that is still starting.
#[derive(Default)]
pub(crate) struct HistoryIntent {
    pub(crate) want_list: bool,
    /// Session to load instead of creating a new one.
    pub(crate) resume: Option<String>,
    pub(crate) loading: Option<String>,
}

impl LumenApp {
    pub(crate) fn toggle_agent_history(&mut self, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        let history = &mut self.view.agents.session_mut(kind).history;
        history.open = !history.open;
        if history.open {
            self.refresh_agent_history(kind, None, ctx);
        }
    }

    pub(crate) fn more_agent_history(&mut self, ctx: &egui::Context) {
        let kind = self.view.agents.active;
        if let Some(cursor) = self.view.agents.session(kind).history.next_cursor.clone() {
            self.refresh_agent_history(kind, Some(cursor), ctx);
        }
    }

    fn refresh_agent_history(
        &mut self,
        kind: AgentKind,
        cursor: Option<String>,
        ctx: &egui::Context,
    ) {
        let state = self.view.agents.session(kind).state.clone();
        let history = &mut self.view.agents.session_mut(kind).history;
        history.error = None;
        match state {
            ConnectionState::Ready | ConnectionState::Connected => {
                if self.agent_supports_list(kind) {
                    self.send_session_list(kind, cursor);
                } else {
                    self.view.agents.session_mut(kind).history.unsupported = true;
                }
            }
            ConnectionState::Starting => self.history_intent(kind).want_list = true,
            ConnectionState::Idle => {
                self.history_intent(kind).want_list = true;
                self.start_agent(kind, ctx);
            }
            ConnectionState::NeedsAuth
            | ConnectionState::NotFound(_)
            | ConnectionState::Failed(_) => {
                history.error = Some("Connect to the agent first (see the status above).".into());
            }
        }
    }

    fn agent_supports_list(&mut self, kind: AgentKind) -> bool {
        self.agent_init(kind)
            .is_some_and(|init| init.capabilities.supports_session("list"))
    }

    fn history_intent(&mut self, kind: AgentKind) -> &mut HistoryIntent {
        &mut self.agent_runtime.history[kind.index()]
    }

    fn send_session_list(&mut self, kind: AgentKind, cursor: Option<String>) {
        let cwd = self.agent_cwd(kind);
        self.view.agents.session_mut(kind).history.loading = true;
        let id = self.agent_register_request(kind, "session/list");
        self.agent_send(
            kind,
            &requests::session_list_page(id, &cwd, cursor.as_deref()),
        );
        if cursor.is_none() {
            self.view.agents.session_mut(kind).history.sessions.clear();
        }
    }

    /// The initialize reply arrived. Returns true when history work consumed the
    /// handshake (so the caller must not also open a new session).
    pub(super) fn continue_after_initialize(&mut self, kind: AgentKind) -> bool {
        let intent = std::mem::take(self.history_intent(kind));
        if let Some(id) = intent.resume {
            self.send_session_load(kind, &id);
            return true;
        }
        let has_prompt = self.agent_has_queued_prompt(kind);
        if intent.want_list && !has_prompt {
            self.view.agents.session_mut(kind).state = ConnectionState::Connected;
            if self.agent_supports_list(kind) {
                self.send_session_list(kind, None);
            } else {
                let history = &mut self.view.agents.session_mut(kind).history;
                history.unsupported = true;
            }
            return true;
        }
        false
    }

    pub(super) fn on_session_list(&mut self, kind: AgentKind, result: &Value) {
        let cwd = self.agent_cwd(kind);
        let page = parse_session_page(result, &cwd);
        let history = &mut self.view.agents.session_mut(kind).history;
        history.loading = false;
        history.sessions.extend(page.sessions);
        history.next_cursor = page.next_cursor;
    }

    pub(super) fn on_session_list_error(&mut self, kind: AgentKind, error: &RpcError) {
        let history = &mut self.view.agents.session_mut(kind).history;
        history.loading = false;
        history.error = Some(error.message.clone());
    }

    /// Replace the conversation with an earlier session of the same agent.
    pub(crate) fn resume_agent_session(
        &mut self,
        kind: AgentKind,
        session_id: &str,
        ctx: &egui::Context,
    ) {
        let state = self.view.agents.session(kind).state.clone();
        let session = self.view.agents.session_mut(kind);
        if session.running || session.preparing {
            self.view.status =
                "Wait for the current turn to finish (or stop it) before switching sessions".into();
            return;
        }
        session.conversation = Conversation::default();
        session.layout.clear();
        session.changes = None;
        session.pending_permissions.clear();
        session.replaying = true;
        session.history.open = false;
        match state {
            ConnectionState::Ready | ConnectionState::Connected => {
                self.send_session_load(kind, session_id);
            }
            ConnectionState::Starting => {
                self.history_intent(kind).resume = Some(session_id.to_owned())
            }
            ConnectionState::Idle | ConnectionState::Failed(_) | ConnectionState::NotFound(_) => {
                self.history_intent(kind).resume = Some(session_id.to_owned());
                self.start_agent(kind, ctx);
            }
            ConnectionState::NeedsAuth => {
                self.view.agents.session_mut(kind).replaying = false;
                self.view.status = "Sign in first, then resume the session".into();
            }
        }
    }

    fn send_session_load(&mut self, kind: AgentKind, session_id: &str) {
        let cwd = self.agent_cwd(kind);
        self.history_intent(kind).loading = Some(session_id.to_owned());
        let id = self.agent_register_request_with(kind, "session/load", Some(LOAD_TIMEOUT));
        self.agent_send(kind, &requests::session_load(id, session_id, &cwd));
        Self::agent_log(
            kind,
            LogLevel::Info,
            &format!("Resuming session {session_id}"),
        );
    }

    pub(super) fn on_session_loaded(&mut self, kind: AgentKind, result: &Value) {
        let requested = self.history_intent(kind).loading.take().unwrap_or_default();
        let loaded = parse_loaded_session(result, &requested);
        self.view.agents.session_mut(kind).replaying = false;
        self.view
            .agents
            .session_mut(kind)
            .conversation
            .push_notice("Resumed an earlier session.");
        self.adopt_session(kind, loaded, true);
    }

    pub(super) fn on_session_load_failed(&mut self, kind: AgentKind, error: &RpcError) {
        self.history_intent(kind).loading = None;
        Self::agent_log(kind, LogLevel::Error, &error.message);
        let has_session = self.agent_session_id(kind).is_some();
        let session = self.view.agents.session_mut(kind);
        session.replaying = false;
        session.state = if has_session {
            ConnectionState::Ready
        } else {
            ConnectionState::Connected
        };
        session
            .conversation
            .push_notice(&format!("Could not resume that session: {}", error.message));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::command::Command,
        services::agent_process::{AgentEvent, fake::FakeAgentLauncher},
    };
    use serde_json::json;
    use std::sync::Arc;

    struct Harness {
        app: LumenApp,
        fake: FakeAgentLauncher,
        ctx: egui::Context,
        cwd: String,
    }

    impl Harness {
        fn new(kind: AgentKind) -> Self {
            let fake = FakeAgentLauncher::default();
            let mut app = LumenApp::default();
            app.agent_runtime.launcher = Arc::new(fake.clone());
            let program = std::env::current_exe().unwrap().display().to_string();
            for each in AgentKind::ALL {
                app.view.settings.user.persisted.insert(
                    format!("agent.{}.command", each.id()),
                    lumen_settings_core::Value::Text(program.clone()),
                );
            }
            app.view.settings.user.persisted.insert(
                "agent.track_changes".into(),
                lumen_settings_core::Value::Bool(false),
            );
            let dir = tempfile::tempdir().unwrap();
            let root = dir.keep().canonicalize().unwrap();
            app.editor.workspace = Some(root.clone());
            app.view.agents.active = kind;
            Self {
                app,
                fake,
                ctx: egui::Context::default(),
                cwd: root.display().to_string(),
            }
        }

        fn kind(&self) -> AgentKind {
            self.app.view.agents.active
        }

        fn sent(&self) -> Vec<serde_json::Value> {
            self.fake.sent.lock().unwrap().clone()
        }

        fn last(&self, method: &str) -> serde_json::Value {
            self.sent()
                .into_iter()
                .rev()
                .find(|m| m["method"] == method)
                .unwrap_or_else(|| panic!("no {method}"))
        }

        fn has_sent(&self, method: &str) -> bool {
            self.sent().iter().any(|m| m["method"] == method)
        }

        fn push(&mut self, message: serde_json::Value) {
            self.fake
                .queued
                .lock()
                .unwrap()
                .push(AgentEvent::Message(message));
            self.app.poll_agents(&self.ctx);
        }

        fn reply(&mut self, request: &serde_json::Value, result: serde_json::Value) {
            self.push(json!({"jsonrpc":"2.0","id":request["id"],"result":result}));
        }

        fn update(&mut self, update: serde_json::Value) {
            self.push(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"old","update":update}}));
        }

        fn state(&self) -> ConnectionState {
            self.app.view.agents.session(self.kind()).state.clone()
        }

        fn history(&self) -> &HistoryView {
            &self.app.view.agents.session(self.kind()).history
        }

        /// Open the history and complete the handshake; the agent advertises list/load.
        fn open_history(&mut self, with_list: bool) {
            self.app.execute(Command::ToggleAgentHistory, &self.ctx);
            let init = self.last("initialize");
            let capabilities = if with_list {
                json!({"loadSession":true,"sessionCapabilities":{"list":{},"resume":{}}})
            } else {
                json!({})
            };
            self.reply(
                &init,
                json!({"protocolVersion":1,"agentCapabilities":capabilities}),
            );
        }

        fn list_reply(&mut self) {
            let list = self.last("session/list");
            let cwd = self.cwd.clone();
            self.reply(&list, json!({"sessions":[
                {"sessionId":"s-new","cwd":cwd,"title":"Newer","updatedAt":"2026-10-16T10:00:00.000Z"},
                {"sessionId":"s-old","cwd":cwd,"title":"Older","updatedAt":"2026-10-15T09:30:00.000Z"},
                {"sessionId":"s-other","cwd":"/elsewhere","title":"Not here"}],"nextCursor":"page2"}));
        }
    }

    #[test]
    fn opening_history_connects_and_lists_without_creating_a_session() {
        let mut h = Harness::new(AgentKind::Claude);
        h.open_history(true);
        assert!(
            !h.has_sent("session/new"),
            "browsing creates no conversation"
        );
        assert_eq!(h.state(), ConnectionState::Connected);
        assert_eq!(h.last("session/list")["params"]["cwd"], h.cwd);
        assert!(h.history().open && h.history().loading);

        h.list_reply();
        let history = h.history();
        let titles: Vec<_> = history
            .sessions
            .iter()
            .filter_map(|s| s.title.as_deref())
            .collect();
        assert_eq!(titles, ["Newer", "Older"], "other folders are filtered out");
        assert_eq!(
            (history.loading, history.next_cursor.as_deref()),
            (false, Some("page2"))
        );

        h.app.execute(Command::MoreAgentHistory, &h.ctx);
        assert_eq!(h.last("session/list")["params"]["cursor"], "page2");
        let list = h.last("session/list");
        let cwd = h.cwd.clone();
        h.reply(
            &list,
            json!({"sessions":[{"sessionId":"s-3","cwd":cwd,"title":"Third"}]}),
        );
        assert_eq!(h.history().sessions.len(), 3);
        assert!(h.history().next_cursor.is_none());

        // Toggling closes the list again.
        h.app.execute(Command::ToggleAgentHistory, &h.ctx);
        assert!(!h.history().open);
    }

    #[test]
    fn agents_without_a_session_list_say_so() {
        let mut h = Harness::new(AgentKind::Claude);
        h.open_history(false);
        assert!(h.history().unsupported);
        assert!(!h.has_sent("session/list"));
        assert_eq!(h.state(), ConnectionState::Connected);
    }

    #[test]
    fn resuming_replays_the_conversation_and_the_next_prompt_continues_it() {
        let mut h = Harness::new(AgentKind::Claude);
        h.open_history(true);
        h.list_reply();
        h.app.view.settings.user.persisted.insert(
            "agent.follow_locations".into(),
            lumen_settings_core::Value::Bool(true),
        );
        let kind = h.kind();
        h.app
            .execute(Command::ResumeAgentSession(kind, "s-old".into()), &h.ctx);
        let load = h.last("session/load");
        assert_eq!(
            (
                load["params"]["sessionId"].clone(),
                load["params"]["cwd"].clone()
            ),
            (json!("s-old"), json!(h.cwd))
        );
        assert!(!h.history().open, "the list gives way to the transcript");
        assert!(h.app.view.agents.session(kind).replaying);

        // The agent replays the old messages before answering.
        h.update(json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"what is this?"}}));
        h.update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"A compass."}}));
        let existing = tempfile::tempdir().unwrap().keep().join("x.rs");
        std::fs::write(&existing, "x").unwrap();
        h.update(json!({"sessionUpdate":"tool_call","toolCallId":"t","kind":"read","locations":[{"path":existing.display().to_string(),"line":1}]}));
        assert!(
            h.app.editor.files.is_empty(),
            "replayed tool calls never open files"
        );
        h.reply(&load, json!({"configOptions":[
            {"id":"mode","name":"Mode","currentValue":"plan","options":[{"value":"plan","name":"Plan"}]}]}));

        let session = h.app.view.agents.session(kind);
        assert_eq!(session.state, ConnectionState::Ready);
        assert!(!session.replaying);
        assert_eq!(session.conversation.agent_text(), "A compass.");
        assert_eq!(session.conversation.config_options[0].current, "plan");
        assert!(
            format!("{:?}", session.conversation.entries).contains("Resumed an earlier session.")
        );

        h.app.view.agents.session_mut(kind).draft = "and then?".into();
        h.app.execute(Command::SendAgentPrompt, &h.ctx);
        assert!(!h.has_sent("session/new"));
        assert_eq!(
            h.last("session/prompt")["params"]["sessionId"],
            "s-old",
            "the loaded session is the live one"
        );
    }

    #[test]
    fn resuming_without_a_connection_starts_the_agent_and_loads_instead_of_creating() {
        let mut h = Harness::new(AgentKind::Codex);
        let kind = h.kind();
        h.app
            .execute(Command::ResumeAgentSession(kind, "s-old".into()), &h.ctx);
        assert_eq!(h.fake.launched.lock().unwrap().len(), 1);
        let init = h.last("initialize");
        h.reply(
            &init,
            json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
        );
        assert!(
            h.has_sent("session/load") && !h.has_sent("session/new") && !h.has_sent("session/list")
        );
        let load = h.last("session/load");
        // Codex omits the session id and sends no extra mode change for a resumed session.
        h.reply(
            &load,
            json!({"configOptions":[
            {"id":"mode","name":"Mode","currentValue":"agent","options":[
                {"value":"read-only","name":"Read-only"},{"value":"agent","name":"Auto"}]}]}),
        );
        assert_eq!(h.state(), ConnectionState::Ready);
        assert!(
            !h.has_sent("session/set_config_option"),
            "a resumed session keeps its own mode"
        );
        h.app.view.agents.session_mut(kind).draft = "go".into();
        h.app.execute(Command::SendAgentPrompt, &h.ctx);
        assert_eq!(h.last("session/prompt")["params"]["sessionId"], "s-old");
    }

    #[test]
    fn a_failed_resume_explains_itself_and_leaves_a_usable_connection() {
        let mut h = Harness::new(AgentKind::Claude);
        h.open_history(true);
        let kind = h.kind();
        h.app
            .execute(Command::ResumeAgentSession(kind, "gone".into()), &h.ctx);
        let load = h.last("session/load");
        h.push(json!({"jsonrpc":"2.0","id":load["id"],"error":{"code":-32602,"message":"Session not found"}}));
        let session = h.app.view.agents.session(kind);
        assert_eq!(session.state, ConnectionState::Connected);
        assert!(!session.replaying);
        assert!(
            format!("{:?}", session.conversation.entries)
                .contains("Could not resume that session: Session not found")
        );
    }

    #[test]
    fn typing_in_a_connected_agent_opens_a_new_session_then_sends() {
        let mut h = Harness::new(AgentKind::Claude);
        h.open_history(true);
        let kind = h.kind();
        h.app.view.agents.session_mut(kind).draft = "hello".into();
        h.app.execute(Command::SendAgentPrompt, &h.ctx);
        let new = h.last("session/new");
        h.reply(&new, json!({"sessionId":"fresh"}));
        assert_eq!(h.last("session/prompt")["params"]["sessionId"], "fresh");
    }

    #[test]
    fn switching_sessions_is_refused_mid_turn_and_setup_errors_show_in_the_list() {
        let mut h = Harness::new(AgentKind::Claude);
        h.open_history(true);
        let kind = h.kind();
        h.app.view.agents.session_mut(kind).running = true;
        h.app
            .execute(Command::ResumeAgentSession(kind, "s".into()), &h.ctx);
        assert!(!h.has_sent("session/load"));
        assert!(h.app.view.status.contains("Wait for the current turn"));

        let mut h = Harness::new(AgentKind::Claude);
        h.app.view.agents.session_mut(AgentKind::Claude).state = ConnectionState::NeedsAuth;
        h.app.execute(Command::ToggleAgentHistory, &h.ctx);
        assert!(
            h.history()
                .error
                .as_deref()
                .unwrap()
                .contains("Connect to the agent first")
        );
    }

    /// Real agent: say something, start over, find the conversation in the history
    /// and resume it. The replayed transcript must contain what the agent said.
    #[cfg(unix)]
    fn live_history(kind: AgentKind, variable: &str) {
        use std::os::unix::fs::PermissionsExt;
        let line = std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"));
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let wrapper = directory
            .path()
            .join("..")
            .join(format!("lumen-live-hist-{}.sh", std::process::id()));
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec {line} \"$@\"\n")).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root);
        app.view.settings.user.persisted.insert(
            format!("agent.{}.command", kind.id()),
            lumen_settings_core::Value::Text(wrapper.canonicalize().unwrap().display().to_string()),
        );
        let ctx = egui::Context::default();
        app.view.agents.active = kind;
        let wait = |app: &mut LumenApp, what: &str, mut done: Box<dyn FnMut(&LumenApp) -> bool>| {
            let deadline = std::time::Instant::now() + Duration::from_secs(120);
            while !done(app) {
                app.poll_agents(&ctx);
                if let ConnectionState::Failed(error) = &app.view.agents.session(kind).state {
                    panic!("agent failed: {error}");
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for {what}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        app.view.agents.session_mut(kind).draft =
            "Reply with only the word pong. Use no tools.".into();
        app.execute(Command::SendAgentPrompt, &ctx);
        wait(
            &mut app,
            "the first answer",
            Box::new(move |a| {
                let s = a.view.agents.session(kind);
                !s.running
                    && s.state == ConnectionState::Ready
                    && !s.conversation.agent_text().is_empty()
            }),
        );

        app.execute(Command::NewAgentSession, &ctx);
        assert!(
            app.view
                .agents
                .session(kind)
                .conversation
                .entries
                .is_empty()
        );
        app.execute(Command::ToggleAgentHistory, &ctx);
        wait(
            &mut app,
            "the session list",
            Box::new(move |a| {
                let h = &a.view.agents.session(kind).history;
                !h.loading && !h.sessions.is_empty()
            }),
        );
        let id = app.view.agents.session(kind).history.sessions[0]
            .session_id
            .clone();
        app.execute(Command::ResumeAgentSession(kind, id), &ctx);
        wait(
            &mut app,
            "the replay",
            Box::new(move |a| {
                let s = a.view.agents.session(kind);
                s.state == ConnectionState::Ready && !s.replaying
            }),
        );
        let text = app
            .view
            .agents
            .session(kind)
            .conversation
            .agent_text()
            .to_lowercase();
        let _ = std::fs::remove_file(&wrapper);
        assert!(
            text.contains("pong"),
            "the earlier answer was replayed: {text:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "needs a logged-in Claude adapter"]
    fn live_claude_history() {
        live_history(AgentKind::Claude, "LUMEN_ACP_CLAUDE");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "needs a logged-in Codex adapter"]
    fn live_codex_history() {
        live_history(AgentKind::Codex, "LUMEN_ACP_CODEX");
    }
}
