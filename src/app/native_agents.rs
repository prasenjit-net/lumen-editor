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
/// How long quitting waits for agents to stop before closing anyway.
const QUIT_GRACE: Duration = Duration::from_secs(4);

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
        self.native_transcripts_dir = crate::services::agent_transcripts_store::user_dir();
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

    /// The window is closing for good. Agents are asked to stop first (turns
    /// interrupted, approvals declined, processes ended); the window closes when
    /// they report done or after a short deadline. Without agents it closes now.
    pub(crate) fn close_window(&mut self, ctx: &egui::Context) {
        if let Some(hub) = self.view.native_agents.as_mut()
            && !hub.is_shut_down()
        {
            if self.view.native_quit_deadline.is_none() {
                hub.begin_shutdown();
                self.view.native_quit_deadline = Some(std::time::Instant::now() + QUIT_GRACE);
                self.view.status = "Stopping agents…".into();
            }
            ctx.request_repaint();
            return;
        }
        self.view.allow_quit = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Closing the window should first stop running agents.
    pub(crate) fn native_agents_need_shutdown(&self) -> bool {
        self.view.native_quit_deadline.is_none()
            && self
                .view
                .native_agents
                .as_ref()
                .is_some_and(|hub| !hub.is_shut_down())
    }

    pub(crate) fn poll_native_agents(&mut self, ctx: &egui::Context) {
        let Some(hub) = self.view.native_agents.as_mut() else {
            return;
        };
        hub.tick();
        if let Some(dir) = &self.native_transcripts_dir {
            for (provider, native, items) in hub.take_dirty_transcripts() {
                if let Err(error) =
                    crate::services::agent_transcripts_store::save(dir, provider, &native, &items)
                {
                    self.view.status = error;
                }
            }
        }
        if self.view.native_quit_deadline.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if let Some(deadline) = self.view.native_quit_deadline
            && (hub.is_shut_down() || std::time::Instant::now() >= deadline)
        {
            self.view.native_quit_deadline = None;
            self.view.allow_quit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
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
        self.load_cached_transcript(id);
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.ensure_ready(id);
        }
        self.view.chats.open(id);
        self.view.settings.open = false;
        self.view.history_active = false;
        self.view.task_editors.deactivate();
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
        }
        self.enter_page_tab(ctx);
    }

    /// Show what was said in this chat the last time, read from the cache. Done
    /// once per session per run, and only into an empty conversation.
    fn load_cached_transcript(&mut self, id: SessionId) {
        let Some(hub) = self.view.native_agents.as_mut() else {
            return;
        };
        if hub.transcript_loaded(id) {
            return;
        }
        let key = hub
            .registry
            .get(id)
            .and_then(|r| r.native.clone().map(|native| (r.provider, native)));
        let (Some(dir), Some((provider, native))) = (&self.native_transcripts_dir, key) else {
            hub.restore_transcript(id, Vec::new());
            return;
        };
        match crate::services::agent_transcripts_store::load(dir, provider, &native) {
            Ok(items) => hub.restore_transcript(id, items),
            Err(error) => {
                // Leave the file alone and start this chat's view empty.
                hub.restore_transcript(id, Vec::new());
                self.view.status = error;
            }
        }
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
    fn chat_fits_an_editor_group_narrower_than_the_old_240_px_floor() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        app.view.agents.open = false;
        for width in [200.0_f32, 260.0] {
            frame(&mut app, &ctx, width);
            let output = frame(&mut app, &ctx, width);
            let mut found = false;
            for clipped in &output.shapes {
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.job.text.contains("Send")
                {
                    found = true;
                    let right = text.pos.x + text.galley.size().x;
                    assert!(
                        right <= width,
                        "Send ends at {right}, past a {width}px window"
                    );
                }
            }
            assert!(found, "composer drawn at {width}");
        }
    }

    #[test]
    fn a_long_chat_title_gives_way_to_the_status_in_a_narrow_group() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.chats.active().unwrap();
        app.view.chats.draft_mut(id).push_str(
            "approve: please review every file in the workspace and explain the layout in detail",
        );
        app.execute(Command::ChatSend(id), &ctx);
        settle(&mut app, &ctx);
        app.view.agents.open = false;
        let width = 340.0_f32;
        frame(&mut app, &ctx, width);
        let output = frame(&mut app, &ctx, width);
        let rects_of = |needle: &str| {
            output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    // Right-aligned galleys extend left of their anchor.
                    egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                        Some(text.galley.rect.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let status = rects_of("Needs your answer")[0];
        // The tab strip repeats the title; the header's copy shares the status row.
        let title = rects_of("approve: please")
            .into_iter()
            .find(|rect| (rect.center().y - status.center().y).abs() < 6.0)
            .expect("chat title in the header row");
        assert!(status.right() <= width, "status stays inside the window");
        assert!(
            title.right() <= status.left() + 1.0,
            "title ends at {} but status starts at {}",
            title.right(),
            status.left()
        );
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
            // The turn is waiting on a permission, so a new message queues.
            assert!(has_text(&output, "Queue"), "composer at {width}");
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

    #[test]
    fn quitting_stops_the_agents_first_and_closes_when_they_are_done() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.chats.active().unwrap();
        app.view.chats.draft_mut(id).push_str("slow: long running");
        app.execute(Command::ChatSend(id), &ctx);
        settle(&mut app, &ctx);
        assert!(app.native_agents_need_shutdown());
        app.close_window(&ctx);
        assert!(!app.view.allow_quit, "not closed while agents are stopping");
        assert!(app.view.native_quit_deadline.is_some());
        assert!(
            !app.native_agents_need_shutdown(),
            "a second close request goes through"
        );
        settle(&mut app, &ctx);
        assert!(app.view.allow_quit, "closed once they reported done");
        assert!(app.view.native_quit_deadline.is_none());
    }

    #[test]
    fn quitting_without_native_agents_closes_immediately() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        assert!(!app.native_agents_need_shutdown());
        app.close_window(&ctx);
        assert!(app.view.allow_quit);
    }

    #[test]
    fn a_conversation_is_still_there_after_the_app_is_closed_and_reopened() {
        let dir = std::env::temp_dir().join(format!("lumen-reopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (mut app, ctx) = fixture();
        app.native_transcripts_dir = Some(dir.clone());
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.chats.active().unwrap();
        app.view
            .chats
            .draft_mut(id)
            .push_str("remember the tangerine");
        app.execute(Command::ChatSend(id), &ctx);
        // Wait for the turn to finish and the cache file to be written.
        for _ in 0..1000 {
            app.poll_native_agents(&ctx);
            let finished = app.view.native_agents.as_ref().unwrap().status(id)
                == crate::app::agent_hub::SessionStatus::Idle
                && dir.exists()
                && std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some());
            if finished {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let registry = app.view.native_agents.as_ref().unwrap().registry.to_json();

        // A new run: same index file, same cache folder, fresh agents.
        let (mut app, ctx) = fixture();
        app.native_transcripts_dir = Some(dir.clone());
        app.view.native_agents.as_mut().unwrap().registry =
            crate::model::agent_registry::Registry::from_json(&registry).unwrap();
        assert!(app.view.native_agents.as_ref().unwrap().live(id).is_none());
        app.execute(Command::OpenChat(id), &ctx);
        frame(&mut app, &ctx, 1000.0);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(
            has_text(&output, "remember the tangerine"),
            "the user's message"
        );
        let live = app.view.native_agents.as_ref().unwrap().live(id).unwrap();
        assert!(
            live.model.items.len() >= 2,
            "the answer too: {:?}",
            live.model.items
        );
        // Opening again does not duplicate anything.
        let count = live.model.items.len();
        app.execute(Command::OpenChat(id), &ctx);
        let live = app.view.native_agents.as_ref().unwrap().live(id).unwrap();
        assert_eq!(live.model.items.len(), count);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
