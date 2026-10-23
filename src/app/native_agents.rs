//! Workbench glue for the native agents: commands from the agent panel become
//! hub calls, and the hub is polled once per frame. File access, process
//! launching and persistence stay behind the hub and services.
use super::{LumenApp, chat_attachments};
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

/// A provider's unarchived sessions in a folder, most recently active first
/// (pins do not matter: this is the "latest" fallback, not the History order).
pub(crate) fn provider_sessions(
    hub: &super::agent_hub::AgentHub,
    provider: ProviderId,
    workspace: Option<&std::path::Path>,
) -> Vec<SessionId> {
    let mut rows: Vec<_> = hub
        .registry
        .listing(workspace, false)
        .into_iter()
        .filter(|record| record.provider == provider)
        .collect();
    rows.sort_by(|a, b| b.updated.cmp(&a.updated).then(b.id.cmp(&a.id)));
    rows.into_iter().map(|record| record.id).collect()
}

/// The folder chats work in: the open workspace, else the current directory,
/// with aliases and symlinks resolved to one workspace context.
pub(crate) fn chat_workspace_for(workspace: Option<&std::path::Path>) -> std::path::PathBuf {
    let folder = workspace
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| ".".into());
    std::fs::canonicalize(&folder).unwrap_or(folder)
}

impl super::view_state::ViewState {
    /// Keep a provider tab's current session valid for this folder (see
    /// `AgentPanel::resolve_current`).
    pub(crate) fn resolve_agent_session(
        &mut self,
        provider: ProviderId,
        workspace: &std::path::Path,
    ) -> Option<SessionId> {
        let hub = self.native_agents.as_ref()?;
        let sessions = provider_sessions(hub, provider, Some(workspace));
        self.agent_panel.resolve_current(provider, &sessions)
    }
}

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
    pub(crate) fn toggle_agent_panel(&mut self) {
        if !self.view.settings.boolean("agent.enabled") {
            self.view.status = "Coding agents are turned off in Settings (agent.enabled)".into();
            self.view.agents.open = false;
            return;
        }
        self.view.agents.open = !self.view.agents.open;
        if self.view.agents.open {
            let provider = self.view.agent_panel.active_provider();
            self.view.agent_panel.select_provider(provider);
        }
    }

    /// Tab click: show that provider's chat (or History, as it was left).
    pub(super) fn agent_select_provider(&mut self, provider: ProviderId, ctx: &egui::Context) {
        self.view.agent_panel.select_provider(provider);
        if let Some(id) = self.resolve_provider_session(provider) {
            self.prepare_chat(id);
        }
        ctx.request_repaint();
    }

    /// The provider's current session for this folder, falling back to its most
    /// recent one when the remembered session was archived or is gone.
    pub(crate) fn resolve_provider_session(&mut self, provider: ProviderId) -> Option<SessionId> {
        let workspace = chat_workspace_for(self.editor.workspace.as_deref());
        self.view.resolve_agent_session(provider, &workspace)
    }

    /// Turn on the native Codex / Claude Code integration. Providers are launched
    /// lazily, so this costs nothing until a chat sends its first message.
    pub(crate) fn enable_native_agents(
        &mut self,
        ctx: &egui::Context,
        store: Option<std::path::PathBuf>,
    ) {
        use crate::services::native_agent_launcher::NativeChildLauncher;
        use lumen_agent_core::api::AgentBackend;
        let path = crate::services::login_path::merge_path(
            crate::services::login_path::cached_login_path().map(|p| p.as_os_str()),
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
        if let Some(provider) = store.as_deref().and_then(|path| {
            crate::services::agent_panel_store::load(
                &crate::services::agent_panel_store::path_beside(path),
            )
        }) {
            self.view.agent_panel.restore_provider(provider);
        }
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
        if let Some(provider) = self.view.agent_panel.take_provider_change()
            && let Some(index) = &self.native_agents_store
        {
            // Forgetting the tab is harmless, so a failed write is not reported.
            let _ = crate::services::agent_panel_store::save(
                &crate::services::agent_panel_store::path_beside(index),
                provider,
            );
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
        chat_workspace_for(self.editor.workspace.as_deref())
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
        let provider = self
            .view
            .native_agents
            .as_ref()
            .and_then(|hub| hub.registry.get(id))
            .map(|record| record.provider);
        let Some(provider) = provider else {
            return;
        };
        self.prepare_chat(id);
        self.view.agents.open = true;
        self.view.agent_panel.show_session(provider, id);
        ctx.request_repaint();
    }

    /// Load the cached transcript and start the session's backend if it is not ready.
    fn prepare_chat(&mut self, id: SessionId) {
        self.load_cached_transcript(id);
        if let Some(hub) = self.view.native_agents.as_mut() {
            hub.ensure_ready(id);
        }
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
        let text = self.view.agent_panel.take_draft(id);
        let attachments = self.view.agent_panel.take_attachments(id);
        if text.trim().is_empty() && attachments.is_empty() {
            self.view.agent_panel.restore_draft(id, text);
            return;
        }
        let Some(hub) = self.view.native_agents.as_mut() else {
            self.view.agent_panel.restore_draft(id, text);
            self.view.agent_panel.restore_attachments(id, attachments);
            return;
        };
        let mut input = Vec::new();
        if !text.trim().is_empty() {
            input.push(InputBlock::Text(text.clone()));
        }
        match chat_attachments::blocks(&*self.file_system, &attachments) {
            Ok(blocks) => input.extend(blocks),
            Err(error) => {
                // A moved or oversized file stops the send; the draft and chips stay.
                self.view.agent_panel.restore_draft(id, text);
                self.view.agent_panel.restore_attachments(id, attachments);
                self.view.status = error.to_string();
                return;
            }
        }
        if let Err(error) = hub.send(id, input) {
            // Nothing was sent: keep what the user typed and attached.
            self.view.agent_panel.restore_draft(id, text);
            self.view.agent_panel.restore_attachments(id, attachments);
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
        }
        // The tab that was showing it falls back to the provider's latest chat.
        for provider in ProviderId::ALL {
            self.resolve_provider_session(provider);
        }
    }

    pub(super) fn rename_chat(&mut self, id: SessionId, title: &str) {
        let Some(hub) = self.view.native_agents.as_mut() else {
            return;
        };
        if hub.registry.rename(id, title) {
            hub.registry_dirty = true;
        } else {
            self.view.status = "A chat title cannot be empty".into();
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
    fn a_new_chat_opens_in_the_panel_and_never_in_the_editor_strip() {
        let (mut app, ctx) = fixture();
        app.view.agents.open = false;
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app
            .view
            .agent_panel
            .visible_chat()
            .expect("the chat is shown in the panel");
        assert!(app.view.agents.open, "creating a chat reveals the panel");
        assert_eq!(app.view.agent_panel.active_provider(), ProviderId::Claude);
        assert!(!app.view.page_tab_active(), "the editor area is untouched");
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "Claude Code"), "provider tab");
        assert!(has_text(&output, "Codex"), "every provider has a tab");
        assert!(has_text(&output, "Ask Claude Code anything"));
        assert!(!has_text(&output, "New chat"), "no chat tab in the strip");
        assert!(
            app.view
                .native_agents
                .as_ref()
                .unwrap()
                .registry
                .get(id)
                .is_some()
        );
        assert_eq!(app.view.agent_panel.visible_chat(), Some(id));
    }

    #[test]
    fn sending_a_draft_runs_a_turn_and_shows_the_conversation() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
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
        app.view
            .agent_panel
            .draft_mut(id)
            .push_str("hello fake agent");
        app.execute(Command::ChatSend(id), &ctx);
        assert_eq!(app.view.agent_panel.draft(id), "", "the draft is consumed");
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "hello fake agent"));
        let hub = app.view.native_agents.as_ref().unwrap();
        assert_eq!(hub.status(id), crate::app::agent_hub::SessionStatus::Idle);
        assert!(hub.registry_dirty);
    }

    struct Picks(Vec<std::path::PathBuf>);
    impl crate::services::FileDialogs for Picks {
        fn open_file(&self) -> Option<std::path::PathBuf> {
            None
        }
        fn open_folder(&self) -> Option<std::path::PathBuf> {
            None
        }
        fn save_as(&self, _: &str) -> Option<std::path::PathBuf> {
            None
        }
        fn pick_attachments(&self) -> Vec<std::path::PathBuf> {
            self.0.clone()
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-attach-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn user_messages(app: &LumenApp, id: SessionId) -> Vec<String> {
        app.view
            .native_agents
            .as_ref()
            .unwrap()
            .live(id)
            .unwrap()
            .model
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                lumen_agent_core::model::ItemKind::User(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn attached_files_and_images_are_sent_with_the_message_and_clear_afterwards() {
        let dir = scratch("send");
        let note = dir.join("note.rs");
        let shot = dir.join("shot.dat");
        std::fs::write(&note, "fn n() {}").unwrap();
        std::fs::write(&shot, b"\x89PNG\r\n\x1a\nrest").unwrap();
        let (mut app, ctx) = fixture();
        app.dialogs = Box::new(Picks(vec![note.clone(), shot.clone()]));
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        settle(&mut app, &ctx);
        app.execute(Command::ChatAttach(id), &ctx);
        let kinds: Vec<_> = app
            .view
            .agent_panel
            .attachments(id)
            .iter()
            .map(|a| a.image_type)
            .collect();
        assert_eq!(
            kinds,
            [None, Some("image/png")],
            "type comes from the bytes"
        );
        frame(&mut app, &ctx, 1000.0);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "note.rs"), "chips show the file name");
        assert!(has_text(&output, "shot.dat"));

        // Attachments alone are enough to send.
        app.execute(Command::ChatSend(id), &ctx);
        assert!(
            app.view.agent_panel.attachments(id).is_empty(),
            "chips clear on send"
        );
        settle(&mut app, &ctx);
        let shown = user_messages(&app, id).join("\n");
        assert!(shown.contains(&format!("@{}", note.display())) && shown.contains("image/png"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_cancelled_picker_and_refused_files_attach_nothing() {
        let dir = scratch("refuse");
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        app.dialogs = Box::new(Picks(Vec::new()));
        app.execute(Command::ChatAttach(id), &ctx);
        assert!(app.view.agent_panel.attachments(id).is_empty());

        app.attach_paths(id, vec![dir.clone(), dir.join("missing.txt")]);
        assert!(app.view.agent_panel.attachments(id).is_empty());
        assert!(app.view.status.contains("Could not read") || app.view.status.contains("folder"));

        app.attach_paths(id, vec![file.clone(), file.clone()]);
        assert_eq!(
            app.view.agent_panel.attachments(id).len(),
            1,
            "no duplicates"
        );
        assert!(app.view.status.contains("already attached"));
        app.execute(Command::ChatDetach(id, file), &ctx);
        assert!(app.view.agent_panel.attachments(id).is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_file_gone_before_sending_keeps_the_draft_and_chips() {
        let dir = scratch("gone");
        let file = dir.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        app.attach_paths(id, vec![file.clone()]);
        app.view.agent_panel.draft_mut(id).push_str("look at this");
        std::fs::remove_file(&file).unwrap();
        app.execute(Command::ChatSend(id), &ctx);
        assert_eq!(app.view.agent_panel.draft(id), "look at this");
        assert_eq!(app.view.agent_panel.attachments(id).len(), 1);
        assert!(app.view.status.contains("gone"));
        assert!(user_messages(&app, id).is_empty(), "nothing was sent");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn history_lists_one_providers_sessions_and_archiving_falls_back() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let codex = app.view.agent_panel.visible_chat().unwrap();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let claude = app.view.agent_panel.visible_chat().unwrap();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let newest_claude = app.view.agent_panel.visible_chat().unwrap();
        assert_ne!(claude, newest_claude);
        app.execute(Command::AgentSelectProvider(ProviderId::Codex), &ctx);
        assert_eq!(app.view.agent_panel.visible_chat(), Some(codex));
        app.execute(Command::AgentToggleHistory(ProviderId::Codex), &ctx);
        for width in [700.0, 1000.0, 1600.0] {
            let output = frame(&mut app, &ctx, width);
            assert!(has_text(&output, "Back to chat"), "width {width}");
            assert!(has_text(&output, "Previous chats"));
        }
        // Claude's tab still shows its own current chat, in chat mode.
        app.execute(Command::AgentSelectProvider(ProviderId::Claude), &ctx);
        assert_eq!(app.view.agent_panel.visible_chat(), Some(newest_claude));
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
        app.execute(Command::ArchiveChat(newest_claude), &ctx);
        assert_eq!(
            app.view.agent_panel.current(ProviderId::Claude),
            Some(claude),
            "falls back to the provider's other session"
        );
        app.execute(Command::ArchiveChat(claude), &ctx);
        assert_eq!(app.view.agent_panel.current(ProviderId::Claude), None);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "Start chat"), "empty state is offered");
        let hub = app.view.native_agents.as_ref().unwrap();
        assert_eq!(hub.registry.listing(None, false).len(), 1);
    }

    #[test]
    fn renaming_a_session_keeps_the_title_and_refuses_empty_ones() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        app.execute(Command::RenameChat(id, "  Release notes ".into()), &ctx);
        let title = |app: &LumenApp| {
            app.view
                .native_agents
                .as_ref()
                .unwrap()
                .registry
                .get(id)
                .unwrap()
                .title
                .clone()
        };
        assert_eq!(title(&app), "Release notes");
        app.execute(Command::RenameChat(id, "   ".into()), &ctx);
        assert_eq!(title(&app), "Release notes");
        assert!(app.view.status.contains("cannot be empty"));
    }

    #[test]
    fn chat_commands_without_native_agents_do_nothing() {
        let mut app = LumenApp::default();
        let ctx = egui::Context::default();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        app.execute(Command::OpenChat(SessionId(1)), &ctx);
        assert_eq!(app.view.agent_panel.current(ProviderId::Codex), None);
        assert!(app.view.status.contains("not enabled"));
    }

    #[test]
    fn the_chat_header_offers_the_session_settings_and_changing_one_applies_it() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Claude), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "Fast"), "the model selector is shown");
        assert!(
            !has_text(&output, "Verbose"),
            "other options are behind the configuration menu"
        );
        app.execute(
            Command::ChatSetConfig(id, "model".into(), "smart".into()),
            &ctx,
        );
        settle(&mut app, &ctx);
        let output = frame(&mut app, &ctx, 1000.0);
        assert!(has_text(&output, "Smart"), "the selector shows the change");
        let hub = app.view.native_agents.as_ref().unwrap();
        let config = hub.live(id).unwrap().model.config.as_ref().unwrap();
        assert_eq!(config.get("model").unwrap().current, "smart");
    }

    #[test]
    fn the_composer_fits_a_narrow_agent_panel() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        for width in [260.0_f32, 340.0, 700.0] {
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
    fn panel_header_and_controls_keep_their_edges_at_every_window_width() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        settle(&mut app, &ctx);
        let mut right_gaps = Vec::new();
        for width in [700.0_f32, 1000.0, 1600.0] {
            frame(&mut app, &ctx, width);
            let output = frame(&mut app, &ctx, width);
            let rect_of = |needle: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|clipped| match &clipped.shape {
                        egui::Shape::Text(text) if text.galley.job.text.contains(needle) => {
                            Some(text.galley.rect.translate(text.pos.to_vec2()))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{needle:?} not drawn at width {width}"))
            };
            let tab = rect_of("Codex");
            let close = rect_of(crate::icon::X);
            let history = rect_of(crate::icon::HISTORY);
            let new_chat = rect_of(crate::icon::PLUS);
            assert!(tab.left() > width - 340.0, "tabs stay in the panel");
            assert!(tab.right() < close.left(), "tabs left of the close action");
            assert!(history.right() < new_chat.left(), "stable button order");
            assert!(new_chat.right() <= width && close.right() <= width);
            right_gaps.push(width - new_chat.right());
        }
        assert!(
            right_gaps
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < 1.0),
            "the buttons stay anchored to the right edge: {right_gaps:?}"
        );
    }

    #[test]
    fn wide_content_never_widens_the_panel() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        let tab_left = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "Codex" => Some(text.pos.x),
                    _ => None,
                })
        };
        frame(&mut app, &ctx, 1000.0);
        let before = tab_left(&frame(&mut app, &ctx, 1000.0)).expect("tab drawn");
        let long = "x".repeat(600);
        app.view
            .agent_panel
            .draft_mut(id)
            .push_str(&format!("{long} {long}"));
        app.execute(Command::ChatSend(id), &ctx);
        settle(&mut app, &ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, 1000.0);
        }
        let after = tab_left(&frame(&mut app, &ctx, 1000.0)).expect("tab drawn");
        assert!(
            (before - after).abs() < 1.0,
            "panel edge moved from {before} to {after}"
        );
    }

    #[test]
    fn session_rows_have_a_spoken_name_with_their_status() {
        let (mut app, ctx) = fixture();
        ctx.enable_accesskit();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        app.execute(Command::AgentToggleHistory(ProviderId::Codex), &ctx);
        settle(&mut app, &ctx);
        let mut labels = Vec::new();
        for _ in 0..3 {
            let output = frame(&mut app, &ctx, 1000.0);
            labels.extend(
                output
                    .platform_output
                    .accesskit_update
                    .iter()
                    .flat_map(|update| update.nodes.iter())
                    .filter_map(|(_, node)| node.label().map(str::to_owned)),
            );
        }
        assert!(
            labels.iter().any(|label| label.starts_with("New chat, ")),
            "{labels:?}"
        );
    }

    #[test]
    fn messages_are_told_apart_and_permissions_and_working_state_are_clear() {
        let (mut app, ctx) = fixture();
        app.execute(Command::NativeNewSession(ProviderId::Codex), &ctx);
        let id = app.view.agent_panel.visible_chat().unwrap();
        app.view
            .agent_panel
            .draft_mut(id)
            .push_str("approve: touch file");
        app.execute(Command::ChatSend(id), &ctx);
        settle(&mut app, &ctx);
        for width in [620.0, 1000.0, 1800.0] {
            frame(&mut app, &ctx, width);
            let output = frame(&mut app, &ctx, width);
            assert!(has_text(&output, "You"), "user label at {width}");
            assert!(has_text(&output, "Codex"), "agent label at {width}");
            assert!(has_text(&output, "Permission needed"), "{width}");
            assert!(
                has_text(&output, "Codex ●"),
                "the provider tab flags the pending approval at {width}"
            );
            // The turn is waiting on a permission, so a new message queues.
            assert!(has_text(&output, "Queue"), "composer at {width}");
            assert!(has_text(&output, "Fast"), "model selector at {width}");
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
        app.view
            .agent_panel
            .draft_mut(id)
            .push_str("slow: keep going");
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
        let id = app.view.agent_panel.visible_chat().unwrap();
        app.view
            .agent_panel
            .draft_mut(id)
            .push_str("slow: long running");
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
        let id = app.view.agent_panel.visible_chat().unwrap();
        app.view
            .agent_panel
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
