//! Workbench glue for the native agents: commands from the session list and chat
//! tabs become hub calls, and the hub is polled once per frame. File access, process
//! launching and persistence stay behind the hub and services.
use super::LumenApp;
use crate::model::agent_scheduler::Ticket;
use eframe::egui;
use lumen_agent_core::{
    ProviderId, RequestToken, SessionId,
    api::{ApprovalDecisionId, InputBlock},
};
use std::time::Duration;

/// How often the UI re-polls while agent work is in flight and no event woke it.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The configured program, or the bare name to search PATH for.
fn program(configured: &str, default: &str) -> String {
    let configured = configured.trim();
    if configured.is_empty() {
        default.into()
    } else {
        configured.into()
    }
}

impl LumenApp {
    /// Turn on the native Codex / Claude Code integration. Providers are launched
    /// lazily, so this costs nothing until a chat sends its first message.
    pub(crate) fn enable_native_agents(
        &mut self,
        ctx: &egui::Context,
        store: Option<std::path::PathBuf>,
    ) {
        use crate::services::native_agent_launcher::NativeChildLauncher;
        use lumen_agent_core::api::AgentBackend;
        let path = crate::services::agent_process::merge_path(
            crate::services::agent_process::cached_login_path().map(|p| p.as_os_str()),
            std::env::var_os("PATH").as_deref(),
        );
        // Programs are chosen in User settings only, never by a project file.
        let codex_program = self.view.settings.user_text("agent.native.codex_command");
        let claude_program = self.view.settings.user_text("agent.native.claude_command");
        let launcher = std::sync::Arc::new(NativeChildLauncher { path: Some(path) });
        let repaint = ctx.clone();
        let wake: lumen_agent_core::launcher::Wake =
            std::sync::Arc::new(move || repaint.request_repaint());
        // An unreadable index is left untouched: run with a fresh one and don't save.
        let (registry, store) = match store {
            Some(path) => match crate::services::agent_sessions_store::load(&path) {
                Ok(registry) => (registry, Some(path)),
                Err(error) => {
                    self.view.status = error;
                    (crate::model::agent_registry::Registry::new(), None)
                }
            },
            None => (crate::model::agent_registry::Registry::new(), None),
        };
        self.native_agents_store = store;
        let backends: Vec<(ProviderId, Box<dyn AgentBackend>)> = vec![
            (
                ProviderId::Codex,
                Box::new(lumen_agent_codex::CodexBackend::new(
                    launcher.clone(),
                    wake.clone(),
                    lumen_agent_codex::CodexConfig::new(program(&codex_program, "codex")),
                )),
            ),
            (
                ProviderId::Claude,
                Box::new(lumen_agent_claude::ClaudeBackend::new(
                    launcher,
                    wake,
                    lumen_agent_claude::ClaudeConfig::new(program(&claude_program, "claude")),
                )),
            ),
        ];
        self.view.native_agents = Some(super::agent_hub::AgentHub::new(backends, registry));
    }

    pub(crate) fn poll_native_agents(&mut self, ctx: &egui::Context) {
        let Some(hub) = self.view.native_agents.as_mut() else {
            return;
        };
        hub.tick();
        if hub.registry_dirty
            && let Some(path) = &self.native_agents_store
        {
            match crate::services::agent_sessions_store::save(path, &hub.registry) {
                Ok(()) => hub.registry_dirty = false,
                Err(error) => {
                    // Say it once; the next change retries the save.
                    hub.registry_dirty = false;
                    self.view.status = error;
                }
            }
        }
        if hub.wants_repaint() {
            ctx.request_repaint_after(POLL_INTERVAL);
        }
    }

    /// The folder a new chat works in: the open workspace, else the current directory.
    fn chat_workspace(&self) -> std::path::PathBuf {
        let folder = self
            .editor
            .workspace
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| ".".into());
        // Path aliases and symlinks must resolve to one workspace context.
        std::fs::canonicalize(&folder).unwrap_or(folder)
    }

    pub(super) fn native_new_session(&mut self, provider: ProviderId, ctx: &egui::Context) {
        let workspace = self.chat_workspace();
        let Some(hub) = self.view.native_agents.as_mut() else {
            self.view.status = "Native agents are not enabled".into();
            return;
        };
        let id = hub.create_session(provider, workspace);
        self.open_chat(id, ctx);
    }

    pub(super) fn open_chat(&mut self, id: SessionId, ctx: &egui::Context) {
        let known = self
            .view
            .native_agents
            .as_ref()
            .is_some_and(|hub| hub.registry.get(id).is_some());
        if !known {
            return;
        }
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.ensure_ready(id);
        }
        self.view.chats.open(id);
        self.view.settings.open = false;
        self.view.task_editors.deactivate();
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
        }
        self.enter_page_tab(ctx);
    }

    pub(super) fn chat_send(&mut self, id: SessionId) {
        let text = self.view.chats.take_draft(id);
        if text.trim().is_empty() {
            self.view.chats.restore_draft(id, text);
            return;
        }
        let Some(hub) = self.view.native_agents.as_mut() else {
            self.view.chats.restore_draft(id, text);
            return;
        };
        if let Err(error) = hub.send(id, vec![InputBlock::Text(text.clone())]) {
            // Nothing was sent: keep what the user typed.
            self.view.chats.restore_draft(id, text);
            self.view.status = format!("Could not send the message: {error:?}");
        }
    }

    pub(super) fn chat_stop(&mut self, id: SessionId) {
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.interrupt(id);
        }
    }

    pub(super) fn chat_cancel_queued(&mut self, id: SessionId, ticket: Ticket) {
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.cancel_queued(id, ticket);
        }
    }

    pub(super) fn chat_approval(
        &mut self,
        id: SessionId,
        token: RequestToken,
        decision: ApprovalDecisionId,
    ) {
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.reply_approval(id, token, decision);
        }
    }

    pub(super) fn chat_set_config(&mut self, id: SessionId, option: String, value: String) {
        let Some(hub) = self.view.native_agents.as_mut() else {
            return;
        };
        let patch = lumen_agent_core::api::ConfigPatch {
            values: vec![(option, value)],
            ..Default::default()
        };
        if !hub.set_config(id, patch) {
            self.view.status = "The session is not ready yet; try again in a moment".into();
        }
    }

    pub(super) fn chat_reset_config(&mut self, id: SessionId, option: String) {
        let Some(hub) = self.view.native_agents.as_mut() else {
            return;
        };
        if !hub.set_config(id, lumen_agent_core::api::ConfigPatch::reset(option)) {
            self.view.status = "The session is not ready yet; try again in a moment".into();
        }
    }

    pub(super) fn archive_chat(&mut self, id: SessionId) {
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.archive(id);
            self.view.chats.close(id);
        }
    }

    pub(super) fn pin_chat(&mut self, id: SessionId, pinned: bool) {
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.registry.set_pinned(id, pinned);
            hub.registry_dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{agent_hub::AgentHub, command::Command},
        model::{Document, agent_registry::Registry},
    };
    use eframe::egui;
    use lumen_agent_core::{api::AgentBackend, testing::FakeBackend};

    fn fixture() -> (LumenApp, egui::Context) {
        let mut app = LumenApp::default();
        app.editor.workspace = Some(std::env::temp_dir());
        app.editor
            .files
            .push(Document::from_path("a.rs".into(), "fn a() {}".into()));
        app.editor.set_active_index(Some(0));
        let backends: Vec<(ProviderId, Box<dyn AgentBackend>)> = vec![
            (
                ProviderId::Codex,
                Box::new(FakeBackend::new(ProviderId::Codex)),
            ),
            (
                ProviderId::Claude,
                Box::new(FakeBackend::new(ProviderId::Claude)),
            ),
        ];
        app.view.native_agents = Some(AgentHub::new(backends, Registry::new()));
        app.view.agents.open = true;
        let ctx = egui::Context::default();
        crate::ui::theme::configure_ui(&ctx);
        (app, ctx)
    }

    fn frame(app: &mut LumenApp, ctx: &egui::Context, width: f32) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 700.0),
                )),
                ..Default::default()
            },
            |ctx| {
                app.render_component(ctx, |w| w.agent_panel(ctx));
                app.render_component(ctx, |w| w.editor(ctx));
            },
        )
    }

    fn has_text(output: &egui::FullOutput, needle: &str) -> bool {
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains(needle)))
    }

    fn settle(app: &mut LumenApp, ctx: &egui::Context) {
        for _ in 0..50 {
            app.poll_native_agents(ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_new_chat_opens_as_a_tab_and_documents_take_the_page_back() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.chats.active().expect("chat tab is visible");
        assert!(app.view.page_tab_active());
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(
            has_text(&output, "New chat"),
            "tab and header show the title"
        );
        assert!(has_text(&output, "Claude Code"));
        let doc = app.editor.files[0].id;
        app.show_document_tab(doc);
        assert!(!app.view.page_tab_active());
        assert!(app.view.chats.is_open(id), "the tab stays open");
        app.execute(Command::OpenChat(id), &ctx);
        assert_eq!(app.view.chats.active(), Some(id));
        app.execute(Command::CloseEditor, &ctx);
        assert!(!app.view.chats.is_open(id));
        assert_eq!(
            app.editor.files.len(),
            1,
            "Close Editor did not close a document"
        );
        assert!(
            app.view
                .native_agents
                .as_ref()
                .unwrap()
                .registry
                .get(id)
                .is_some()
        );
    }

    #[test]
    fn sending_a_draft_runs_a_turn_and_shows_the_conversation() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.chats.active().unwrap();
        app.execute(Command::ChatSend(id), &ctx);
        assert!(
            app.view
                .native_agents
                .as_ref()
                .unwrap()
                .live(id)
                .unwrap()
                .prompts
                .is_empty(),
            "blank drafts are not sent"
        );
        app.view.chats.draft_mut(id).push_str("hello fake agent");
        app.execute(Command::ChatSend(id), &ctx);
        assert_eq!(app.view.chats.draft(id), "", "the draft is consumed");
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "hello fake agent"));
        let hub = app.view.native_agents.as_ref().unwrap();
        assert_eq!(hub.status(id), crate::app::agent_hub::SessionStatus::Idle);
        assert!(hub.registry_dirty);
    }

    #[test]
    fn the_session_list_shows_both_providers_and_archiving_closes_the_tab() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let codex = app.view.chats.active().unwrap();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let claude = app.view.chats.active().unwrap();
        let workspace = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        let hub = app.view.native_agents.as_ref().unwrap();
        assert_eq!(
            hub.registry.listing(Some(workspace.as_path()), false).len(),
            2
        );
        for width in [700.0, 1000.0, 1600.0] {
            let output = frame(&mut app, &ctx, width);
            assert!(has_text(&output, "AGENT SESSIONS"), "width {width}");
            assert!(has_text(&output, "Codex"));
        }
        app.execute(Command::PinChat(codex, true), &ctx);
        assert!(
            app.view
                .native_agents
                .as_ref()
                .unwrap()
                .registry
                .get(codex)
                .unwrap()
                .pinned
        );
        app.execute(Command::ArchiveChat(claude), &ctx);
        assert!(!app.view.chats.is_open(claude));
        let hub = app.view.native_agents.as_ref().unwrap();
        assert_eq!(hub.registry.listing(None, false).len(), 1);
    }

    #[test]
    fn chat_commands_without_native_agents_do_nothing() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        app.execute(Command::OpenChat(SessionId(1)), &ctx);
        assert!(app.view.chats.tabs().is_empty());
        assert!(app.view.status.contains("not enabled"));
    }

    #[test]
    fn the_chat_header_offers_the_session_settings_and_changing_one_applies_it() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.chats.active().unwrap();
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "Model"), "the model selector is shown");
        app.execute(
            Command::ChatSetConfig(id, "model".into(), "smart".into()),
            &ctx,
        );
        settle(&mut app, &ctx);
        let hub = app.view.native_agents.as_ref().unwrap();
        let config = hub.live(id).unwrap().model.config.as_ref().unwrap();
        assert_eq!(config.get("model").unwrap().current, "smart");
    }

    #[test]
    fn messages_are_told_apart_and_permissions_and_working_state_are_clear() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.chats.active().unwrap();
        app.view.chats.draft_mut(id).push_str("approve: touch file");
        app.execute(Command::ChatSend(id), &ctx);
        settle(&mut app, &ctx);
        for width in [620.0, 1000.0, 1800.0] {
            frame(&mut app, &ctx, width);
            let output = frame(&mut app, &ctx, width);
            assert!(has_text(&output, "You"), "user label at {width}");
            assert!(has_text(&output, "Codex"), "agent label at {width}");
            assert!(has_text(&output, "Permission needed"), "{width}");
            assert!(has_text(&output, "Needs your answer"), "status at {width}");
            if !has_text(&output, "Send") {
                let all: Vec<String> = output
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
                        _ => None,
                    })
                    .collect();
                panic!("composer at {width}: {all:?}");
            }
            assert!(
                has_text(&output, "Model"),
                "settings above the composer at {width}"
            );
        }
        let request = app
            .view
            .native_agents
            .as_ref()
            .unwrap()
            .live(id)
            .unwrap()
            .model
            .pending_approvals[0]
            .clone();
        let allow = request
            .choices
            .iter()
            .find(|c| c.allows)
            .unwrap()
            .id
            .clone();
        app.execute(Command::ChatApproval(id, request.token, allow), &ctx);
        app.view.chats.draft_mut(id).push_str("slow: keep going");
        app.execute(Command::ChatSend(id), &ctx);
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "Working"), "a clear working state");
        assert!(
            has_text(&output, "Stop"),
            "stop replaces nothing but is offered"
        );
        app.execute(Command::ChatStop(id), &ctx);
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(!has_text(&output, "Working"));
        assert!(!has_text(&output, "Stop"));
    }
}
