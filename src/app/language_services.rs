//! App-side language-server availability snapshots.
//!
//! Process discovery stays in `services`; this controller caches a small view
//! model for the active file so UI rendering never queries PATH or starts tools.
use super::{LumenApp, view_state::LanguageServerStatus};
use crate::services::language_server::{LanguageServerConnection, ServerAvailability, ServerEvent};
use lumen_language_services::{
    DocumentKey, DocumentUri, DocumentVersion, LanguageId, LanguageRegistry, RequestStamp,
    ServerDocumentVersion, capabilities::ServerCapabilities, lsp,
};
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const STABLE_SERVER_PERIOD: Duration = Duration::from_secs(30);
const MAX_RESTART_ATTEMPTS: u32 = 5;

#[derive(Clone, Debug)]
pub(crate) struct SyncedLanguageDocument {
    uri: DocumentUri,
    revision: u64,
    server_version: ServerDocumentVersion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LanguageServerTarget {
    executable: std::path::PathBuf,
    workspace: std::path::PathBuf,
    language: LanguageId,
    name: String,
}

#[derive(Default)]
pub(crate) struct LanguageServerRestart {
    target: Option<LanguageServerTarget>,
    attempts: u32,
    retry_at: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingLanguageRequest {
    kind: LanguageRequestKind,
    deadline: Instant,
}

pub(crate) struct RetiringLanguageServer {
    process: Box<dyn LanguageServerConnection>,
    deadline: Instant,
    exit_sent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LanguageRequestKind {
    Hover,
    Definition,
    Symbols,
    SemanticTokens,
}

/// The last semantic-token request, so a failed or empty reply is not retried
/// for the same revision and edits are throttled.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SemanticTokenAttempt {
    document: crate::model::DocumentId,
    revision: u64,
    at: Instant,
}

const SEMANTIC_TOKEN_DEBOUNCE: Duration = Duration::from_millis(250);

impl LumenApp {
    pub(crate) fn close_workspace_language_server(&mut self) {
        self.clear_language_requests();
        self.retire_language_server();
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_generation = self.language_server_generation.next();
        self.language_server_initialized = false;
        self.language_server_ready_at = None;
        self.language_server_capabilities = None;
        self.language_server_documents.clear();
        self.language_server_restart = Default::default();
        self.view.language_server_document = None;
        self.view.language_server_status = None;
        self.view.language_server_details_open = false;
        self.view.language_hover = None;
        self.view.language_server_diagnostics.clear();
        self.view.language_semantic_tokens.clear();
        self.semantic_token_request = None;
    }

    pub(crate) fn request_language_definition(&mut self) {
        self.request_at_caret(LanguageRequestKind::Definition);
    }

    pub(crate) fn request_language_hover(&mut self) {
        self.request_at_caret(LanguageRequestKind::Hover);
    }

    pub(crate) fn request_document_symbols(&mut self) {
        if !self
            .language_server_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.document_symbols)
        {
            self.view.status =
                "The active language server does not provide document symbols.".into();
            return;
        }
        let Some((document_id, revision, uri, server_version)) = self.active_synced_document()
        else {
            self.view.status = "Language server is not ready for this document.".into();
            return;
        };
        let stamp = self.request_stamp(document_id, revision, server_version, uri.clone());
        let request_id = self.language_server_requests.register(stamp);
        let message = lsp::document_symbols_request(request_id, &uri);
        self.send_language_request(request_id, LanguageRequestKind::Symbols, &message);
    }

    pub(crate) fn open_language_location(&mut self, index: usize) {
        let Some(location) = self.view.language_navigation.locations.get(index).cloned() else {
            return;
        };
        self.view.language_navigation.open = false;
        self.open_file(location.path.clone());
        let Some(document) = self
            .editor
            .active()
            .filter(|document| document.path.as_ref() == Some(&location.path))
        else {
            return;
        };
        match lumen_language_services::position_to_byte(&document.text, location.position) {
            Ok(byte) => {
                self.view.pending_goto = Some((document.id, byte));
                self.view.focus_editor = true;
                self.view.status = "Opened language location".into();
            }
            Err(_) => self.view.status = "Language location is outside the document".into(),
        }
    }

    pub(crate) fn open_problem(
        &mut self,
        document: crate::model::DocumentId,
        range: std::ops::Range<usize>,
    ) {
        let Some(index) = self
            .editor
            .files
            .iter()
            .position(|file| file.id == document)
        else {
            self.view.status = "The problem's document is no longer open.".into();
            return;
        };
        self.editor.active_file = Some(index);
        self.view.settings.open = false;
        if let Some(diff) = &mut self.view.source_control.diff {
            diff.active = false;
        }
        self.view.pending_goto = Some((document, range.start));
        self.view.pending_problem_selection = Some((document, range));
        self.view.focus_editor = true;
        self.view.status = "Opened problem location".into();
    }

    pub(crate) fn refresh_language_server_status(&mut self, ctx: &eframe::egui::Context) {
        self.refresh_language_server_status_inner(ctx);
        self.view.language_server_summary = self
            .editor
            .active()
            .and_then(|document| document.path.as_ref())
            .and_then(|path| LanguageRegistry::detect(path))
            .and_then(|language| self.language_server_summary(language));
    }

    fn language_server_summary(
        &self,
        language: &'static lumen_language_services::LanguageDescriptor,
    ) -> Option<crate::app::view_state::LanguageServerSummary> {
        use crate::app::view_state::LanguageServerSummary;
        let prefix = Self::language_setting_prefix(language.id)?;
        let project_path_ignored = self
            .view
            .settings
            .project_text_override(&format!("{prefix}.server_path"));
        let mut summary = LanguageServerSummary {
            language: language.label,
            project_path_ignored,
            ..Default::default()
        };
        if !self.language_server_enabled(language.id) {
            summary.state = "Disabled in settings".into();
            return Some(summary);
        }
        match self.language_server_availability(language) {
            ServerAvailability::Available { executable } => {
                summary.command = Some(executable.display().to_string());
            }
            ServerAvailability::Missing { candidates } => {
                summary.state = "Not found".into();
                summary.command = Some(candidates.join(" or "));
                return Some(summary);
            }
            ServerAvailability::NotSupported => return None,
        }
        match &self.view.language_server_status {
            Some(LanguageServerStatus::Starting(_)) => summary.state = "Starting".into(),
            Some(LanguageServerStatus::Ready(_)) => summary.state = "Ready".into(),
            Some(LanguageServerStatus::Error(error)) => {
                summary.state = "Error".into();
                summary.error = Some(error.clone());
            }
            _ => summary.state = "Not started".into(),
        }
        if self.language_server_language == Some(language.id)
            && let Some(capabilities) = &self.language_server_capabilities
        {
            let features = [
                (capabilities.text_document_sync.is_some(), "Document sync"),
                (capabilities.save.is_some(), "Save notifications"),
                (capabilities.hover, "Hover"),
                (capabilities.definition, "Go to Definition"),
                (capabilities.document_symbols, "Document symbols"),
                (
                    capabilities.semantic_tokens,
                    "Semantic tokens (not used yet)",
                ),
            ];
            summary.capabilities = features
                .into_iter()
                .filter_map(|(enabled, name)| enabled.then_some(name))
                .collect();
        }
        Some(summary)
    }

    fn refresh_language_server_status_inner(&mut self, ctx: &eframe::egui::Context) {
        let active = self.editor.active().and_then(|document| {
            document
                .path
                .as_ref()
                .map(|path| (document.id, path.clone()))
        });
        let Some((document, path)) = active else {
            self.view.language_server_document = None;
            self.view.language_server_status = None;
            return;
        };
        self.view.language_server_document = Some(document);
        let language = LanguageRegistry::detect(&path);
        if let Some(language) = language
            && !self.language_server_enabled(language.id)
        {
            self.disable_language_server(language.id);
            self.view.language_server_status = None;
            return;
        }
        self.view.language_server_status =
            language.and_then(
                |language| match self.language_server_availability(language) {
                    ServerAvailability::Available { executable } => {
                        let name = executable.file_name()?.to_str()?.to_owned();
                        let root = self.editor.workspace.as_deref().map(|workspace| {
                            language.workspace_root(&path, workspace, |directory, marker| {
                                directory.join(marker).is_file()
                            })
                        });
                        if let Some(root) = root {
                            self.start_language_server(
                                root,
                                executable,
                                language.id,
                                name.clone(),
                                ctx,
                            );
                        }
                        Some(
                            self.view
                                .language_server_status
                                .clone()
                                .unwrap_or(LanguageServerStatus::Available(name)),
                        )
                    }
                    ServerAvailability::Missing { candidates } => candidates
                        .first()
                        .map(|candidate| LanguageServerStatus::Missing(candidate.clone())),
                    ServerAvailability::NotSupported => None,
                },
            );
    }

    fn language_setting_prefix(language: LanguageId) -> Option<&'static str> {
        match language {
            LanguageId::RUST => Some("language.rust"),
            LanguageId::GO => Some("language.go"),
            _ => None,
        }
    }

    /// Languages without a settings namespace (such as JSON) have no server to gate.
    fn language_server_enabled(&self, language: LanguageId) -> bool {
        Self::language_setting_prefix(language)
            .is_none_or(|prefix| self.view.settings.boolean(&format!("{prefix}.enabled")))
    }

    /// An explicit user/project server path replaces PATH discovery. It must name
    /// an existing file; a bad path is reported as missing rather than falling
    /// back to a different executable than the user asked for.
    fn language_server_availability(
        &self,
        language: &lumen_language_services::LanguageDescriptor,
    ) -> ServerAvailability {
        let configured = Self::language_setting_prefix(language.id)
            .map(|prefix| {
                self.view
                    .settings
                    .user_text(&format!("{prefix}.server_path"))
            })
            .map(|path| path.trim().to_owned())
            .filter(|path| !path.is_empty());
        match configured {
            Some(path) => {
                let executable = std::path::PathBuf::from(&path);
                if executable.is_absolute() && executable.is_file() {
                    ServerAvailability::Available { executable }
                } else {
                    ServerAvailability::Missing {
                        candidates: vec![path],
                    }
                }
            }
            None => self.language_server_service.availability(language),
        }
    }

    fn initialize_timeout(&self) -> Duration {
        Duration::from_secs_f64(
            f64::from(
                self.view
                    .settings
                    .number("language.server.startup_timeout_seconds", None),
            )
            .max(1.0),
        )
    }

    /// Stops the running session when its language is switched off in settings.
    fn disable_language_server(&mut self, language: LanguageId) {
        if self.language_server_language != Some(language) {
            return;
        }
        self.clear_language_requests();
        self.retire_language_server();
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_initialized = false;
        self.language_server_capabilities = None;
        self.language_server_documents.clear();
        self.language_server_started_at = None;
        self.language_server_ready_at = None;
        self.language_server_restart = Default::default();
        self.view.language_hover = None;
        self.view.language_server_diagnostics.clear();
        self.view.language_semantic_tokens.clear();
        self.semantic_token_request = None;
    }

    fn start_language_server(
        &mut self,
        workspace: std::path::PathBuf,
        executable: std::path::PathBuf,
        language: LanguageId,
        name: String,
        ctx: &eframe::egui::Context,
    ) {
        let target = LanguageServerTarget {
            executable: executable.clone(),
            workspace: workspace.clone(),
            language,
            name: name.clone(),
        };
        if self.language_server_workspace.as_ref() == Some(&workspace)
            && self.language_server_process.is_some()
            && self.language_server_language == Some(language)
        {
            return;
        }
        if self.language_server_restart.target.as_ref() == Some(&target)
            && let Some(retry_at) = self.language_server_restart.retry_at
        {
            if retry_at > Instant::now() {
                ctx.request_repaint_after(retry_at.saturating_duration_since(Instant::now()));
                return;
            }
        } else if self.language_server_restart.target.as_ref() == Some(&target)
            && self.language_server_restart.attempts > MAX_RESTART_ATTEMPTS
        {
            return;
        } else {
            self.language_server_restart = LanguageServerRestart {
                target: Some(target.clone()),
                ..Default::default()
            };
        }
        self.clear_language_requests();
        self.retire_language_server();
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_generation = self.language_server_generation.next();
        self.language_server_initialized = false;
        self.language_server_ready_at = None;
        self.language_server_capabilities = None;
        self.view.language_hover = None;
        self.view.language_semantic_tokens.clear();
        self.semantic_token_request = None;
        self.language_server_documents.clear();
        let repaint = ctx.clone();
        match self.language_server_service.start(
            &executable,
            &workspace,
            std::sync::Arc::new(move || repaint.request_repaint()),
        ) {
            Ok(mut process) => {
                let workspace_uri = file_uri(&workspace);
                match process.send(&lsp::initialize_request_with_options(
                    0,
                    &workspace_uri,
                    LanguageRegistry::by_id(language)
                        .and_then(|descriptor| descriptor.server)
                        .and_then(|server| server.initialization_options),
                )) {
                    Ok(()) => {
                        self.language_server_workspace = Some(workspace);
                        self.language_server_language = Some(language);
                        self.language_server_process = Some(process);
                        self.language_server_started_at = Some(Instant::now());
                        self.language_server_restart.retry_at = None;
                        self.view.language_server_status =
                            Some(LanguageServerStatus::Starting(name));
                    }
                    Err(error) => self.schedule_language_server_restart(error, ctx),
                }
            }
            Err(error) => self.schedule_language_server_restart(error, ctx),
        }
    }

    pub(crate) fn poll_language_server(&mut self, ctx: &eframe::egui::Context) {
        self.poll_retiring_language_servers();
        if self.language_server_initialized
            && self
                .language_server_ready_at
                .is_some_and(|ready| ready.elapsed() >= STABLE_SERVER_PERIOD)
        {
            self.language_server_restart.attempts = 0;
            self.language_server_ready_at = None;
        }
        let initialize_timeout = self.initialize_timeout();
        if !self.language_server_initialized
            && self.language_server_process.is_some()
            && self
                .language_server_started_at
                .is_some_and(|started| started.elapsed() >= initialize_timeout)
        {
            self.stop_language_server("Language server initialization timed out");
            if let Some(retry_at) = self.language_server_restart.retry_at {
                ctx.request_repaint_after(retry_at.saturating_duration_since(Instant::now()));
            }
            return;
        }
        self.expire_language_requests();
        if let Some(deadline) = self
            .language_server_request_kinds
            .values()
            .map(|request| request.deadline)
            .min()
        {
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
        if let Some(started) = self.language_server_started_at {
            ctx.request_repaint_after(
                (started + initialize_timeout).saturating_duration_since(Instant::now()),
            );
        }
        let mut events = self
            .language_server_process
            .as_ref()
            .map(|process| process.drain_events())
            .unwrap_or_default();
        let process_exited = self
            .language_server_process
            .as_mut()
            .and_then(|process| process.exit_reason().ok().flatten())
            .is_some();
        if process_exited
            && !events
                .iter()
                .any(|event| matches!(event, ServerEvent::Exited))
        {
            events.push(ServerEvent::Exited);
        }
        for event in events {
            match event {
                ServerEvent::Message(message)
                    if message["id"] == 0 && !self.language_server_initialized =>
                {
                    match ServerCapabilities::from_initialize_response(&message) {
                        Ok(capabilities)
                            if capabilities.text_document_sync.is_some()
                                && capabilities.open_close =>
                        {
                            self.language_server_capabilities = Some(capabilities);
                            let initialized = self
                                .language_server_process
                                .as_mut()
                                .ok_or_else(|| "Language server stopped".to_owned())
                                .and_then(|process| process.send(&lsp::initialized_notification()));
                            if let Err(error) = initialized {
                                self.stop_language_server(&error);
                                ctx.request_repaint();
                                return;
                            }
                            // LSP forbids document notifications until the
                            // initialized notification has been delivered.
                            self.language_server_initialized = true;
                            self.language_server_started_at = None;
                            self.language_server_restart.retry_at = None;
                            self.language_server_ready_at = Some(Instant::now());
                            if let Some(LanguageServerStatus::Starting(name)) =
                                self.view.language_server_status.take()
                            {
                                self.view.language_server_status =
                                    Some(LanguageServerStatus::Ready(name));
                            }
                        }
                        Ok(_) => {
                            self.stop_language_server(
                                "Language server does not support open-document text synchronization",
                            );
                            break;
                        }
                        Err(error) => {
                            self.stop_language_server(&format!(
                                "Invalid language-server initialization: {error:?}"
                            ));
                            break;
                        }
                    }
                }
                ServerEvent::ProtocolError(error) => {
                    self.stop_language_server(&error);
                    break;
                }
                ServerEvent::Exited => {
                    let reason = self
                        .language_server_process
                        .as_mut()
                        .map(|process| process.output_closed_reason())
                        .unwrap_or_else(|| "Language server exited".into());
                    self.stop_language_server(&reason);
                    break;
                }
                ServerEvent::Message(message)
                    if message.get("method").is_some() && message.get("id").is_some() =>
                {
                    if let Some(reply) = lsp::server_request_response(&message)
                        && let Some(process) = &mut self.language_server_process
                    {
                        let _ = process.send(&reply);
                    }
                }
                ServerEvent::Message(message) => {
                    if !self.accept_language_response(&message) {
                        self.accept_diagnostics(message);
                    }
                }
            }
        }
        self.sync_language_documents();
        self.request_semantic_tokens(ctx);
        if let Some(retry_at) = self.language_server_restart.retry_at {
            ctx.request_repaint_after(retry_at.saturating_duration_since(Instant::now()));
        }
    }

    fn stop_language_server(&mut self, error: &str) {
        self.clear_language_requests();
        self.language_server_process = None;
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_initialized = false;
        self.language_server_capabilities = None;
        self.view.language_hover = None;
        self.language_server_documents.clear();
        self.language_server_started_at = None;
        self.language_server_ready_at = None;
        self.view.language_server_diagnostics.clear();
        self.view.language_semantic_tokens.clear();
        self.semantic_token_request = None;
        self.view.language_server_status = Some(LanguageServerStatus::Error(error.to_owned()));
        self.schedule_language_server_restart_without_context(error);
    }

    fn sync_language_documents(&mut self) {
        if !self.language_server_initialized {
            return;
        }
        let Some(language) = self.language_server_language else {
            return;
        };
        let desired = self
            .editor
            .files
            .iter()
            .filter_map(|document| {
                let path = document.path.as_ref()?;
                (LanguageRegistry::detect(path).map(|descriptor| descriptor.id) == Some(language)
                    && self
                        .language_server_workspace
                        .as_ref()
                        .is_none_or(|root| path.starts_with(root)))
                .then(|| {
                    DocumentUri::new(file_uri(path))
                        .ok()
                        .map(|uri| (document.id, document.revision, uri, document.text.clone()))
                })?
            })
            .collect::<Vec<_>>();
        let desired_ids = desired
            .iter()
            .map(|item| item.0)
            .collect::<std::collections::HashSet<_>>();
        let stale = self
            .language_server_documents
            .iter()
            .filter(|(id, synced)| {
                !desired_ids.contains(id)
                    || desired
                        .iter()
                        .any(|(desired_id, _, uri, _)| desired_id == *id && uri != &synced.uri)
            })
            .map(|(id, synced)| (*id, synced.uri.clone()))
            .collect::<Vec<_>>();
        for (id, uri) in stale {
            if let Err(error) = self.send_language_notification(&lsp::did_close(&uri)) {
                self.stop_language_server(&error);
                return;
            }
            self.language_server_documents.remove(&id);
            self.view.language_server_diagnostics.remove(&id);
            self.view.language_semantic_tokens.remove(&id);
        }
        for (id, revision, uri, text) in desired {
            let (message, version) = match self.language_server_documents.get(&id) {
                None => {
                    let version = ServerDocumentVersion(1);
                    (Some(lsp::did_open(&uri, language, version, &text)), version)
                }
                Some(synced) if synced.revision != revision => {
                    let version = ServerDocumentVersion(synced.server_version.0.saturating_add(1));
                    (Some(lsp::did_change(&uri, version, &text)), version)
                }
                Some(_) => continue,
            };
            if let Some(message) = message {
                if let Err(error) = self.send_language_notification(&message) {
                    self.stop_language_server(&error);
                    return;
                }
                self.view.language_hover = None;
                self.view.language_server_diagnostics.remove(&id);
                self.view.language_semantic_tokens.remove(&id);
                self.language_server_documents.insert(
                    id,
                    SyncedLanguageDocument {
                        uri,
                        revision,
                        server_version: version,
                    },
                );
            }
        }
    }

    fn accept_diagnostics(&mut self, message: serde_json::Value) {
        let Some((document, synced)) = self.editor.files.iter().find_map(|document| {
            let synced = self.language_server_documents.get(&document.id)?;
            (message
                .pointer("/params/uri")
                .and_then(serde_json::Value::as_str)
                == Some(synced.uri.as_str()))
            .then_some((document, synced))
        }) else {
            return;
        };
        let Ok(published) =
            lumen_language_services::incoming::parse_publish_diagnostics(&message, &document.text)
        else {
            return;
        };
        if let Some(version) = published.version
            && version != synced.server_version.0
        {
            return;
        }
        if document.revision != synced.revision {
            return;
        }
        self.view
            .language_server_diagnostics
            .insert(document.id, published.diagnostics);
    }

    pub(crate) fn notify_language_document_saved(&mut self, document_id: crate::model::DocumentId) {
        self.sync_language_documents();
        let Some(include_text) = self
            .language_server_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.save)
        else {
            return;
        };
        let Some(synced) = self.language_server_documents.get(&document_id).cloned() else {
            return;
        };
        let text = include_text
            .then(|| {
                self.editor
                    .files
                    .iter()
                    .find(|document| document.id == document_id)
            })
            .flatten()
            .map(|document| document.text.clone());
        if let Err(error) =
            self.send_language_notification(&lsp::did_save(&synced.uri, text.as_deref()))
        {
            self.stop_language_server(&error);
        }
    }

    fn active_synced_document(
        &self,
    ) -> Option<(
        crate::model::DocumentId,
        u64,
        DocumentUri,
        ServerDocumentVersion,
    )> {
        let document = self.editor.active()?;
        let synced = self.language_server_documents.get(&document.id)?;
        (document.revision == synced.revision).then(|| {
            (
                document.id,
                document.revision,
                synced.uri.clone(),
                synced.server_version,
            )
        })
    }

    fn send_language_notification(&mut self, message: &serde_json::Value) -> Result<(), String> {
        self.language_server_process
            .as_mut()
            .ok_or_else(|| "Language server stopped".to_owned())?
            .send(message)
    }

    fn retire_language_server(&mut self) {
        let Some(mut process) = self.language_server_process.take() else {
            return;
        };
        if process.begin_shutdown().is_ok() {
            self.retiring_language_servers.push(RetiringLanguageServer {
                process,
                deadline: Instant::now() + Duration::from_secs(2),
                exit_sent: false,
            });
        }
    }

    fn poll_retiring_language_servers(&mut self) {
        let now = Instant::now();
        self.retiring_language_servers.retain_mut(|server| {
            let acknowledged = server.process.drain_events().iter().any(|event| {
                matches!(event, ServerEvent::Message(message)
                    if message.get("id").and_then(serde_json::Value::as_u64)
                        == Some(crate::services::language_server::SHUTDOWN_REQUEST_ID))
            });
            if acknowledged && !server.exit_sent {
                server.process.finish_shutdown();
                server.exit_sent = true;
                server.deadline = now + Duration::from_millis(250);
            }
            let exited = server.process.exit_reason().ok().flatten().is_some();
            !exited && now < server.deadline
        });
    }

    fn clear_language_requests(&mut self) {
        if let Some(process) = &mut self.language_server_process {
            for request_id in self.language_server_request_kinds.keys().copied() {
                let _ = process.send(&lsp::cancel_request(request_id));
            }
        }
        self.language_server_requests.clear();
        self.language_server_request_kinds.clear();
    }

    fn expire_language_requests(&mut self) {
        let now = Instant::now();
        let expired = self
            .language_server_request_kinds
            .iter()
            .filter_map(|(id, request)| (request.deadline <= now).then_some(*id))
            .collect::<Vec<_>>();
        let timed_out = !expired.is_empty();
        for request_id in expired {
            self.language_server_request_kinds.remove(&request_id);
            self.language_server_requests.discard(request_id);
            if let Some(process) = &mut self.language_server_process {
                let _ = process.send(&lsp::cancel_request(request_id));
            }
        }
        if timed_out {
            self.view.status = "Language server request timed out".into();
        }
    }

    fn schedule_language_server_restart(&mut self, error: String, ctx: &eframe::egui::Context) {
        self.schedule_language_server_restart_without_context(&error);
        if let Some(retry_at) = self.language_server_restart.retry_at {
            ctx.request_repaint_after(retry_at.saturating_duration_since(Instant::now()));
        }
    }

    fn schedule_language_server_restart_without_context(&mut self, error: &str) {
        self.language_server_restart.attempts =
            self.language_server_restart.attempts.saturating_add(1);
        if self.language_server_restart.attempts > MAX_RESTART_ATTEMPTS {
            self.language_server_restart.retry_at = None;
            self.view.language_server_status = Some(LanguageServerStatus::Error(format!(
                "{error} (automatic restart limit reached)"
            )));
            return;
        }
        let delay =
            Duration::from_secs(1_u64 << (self.language_server_restart.attempts - 1).min(4));
        self.language_server_restart.retry_at = Some(Instant::now() + delay);
        self.view.language_server_status = Some(LanguageServerStatus::Error(format!(
            "{error} (retrying in {}s)",
            delay.as_secs()
        )));
    }

    fn request_at_caret(&mut self, kind: LanguageRequestKind) {
        let capability = self
            .language_server_capabilities
            .as_ref()
            .is_some_and(|capabilities| match kind {
                LanguageRequestKind::Hover => capabilities.hover,
                LanguageRequestKind::Definition => capabilities.definition,
                LanguageRequestKind::Symbols | LanguageRequestKind::SemanticTokens => false,
            });
        if !capability {
            self.view.status = "The active language server does not provide this feature.".into();
            return;
        }
        let Some((document_id, revision, uri, server_version)) = self.active_synced_document()
        else {
            self.view.status = "Language server is not ready for this document.".into();
            return;
        };
        let Some(document) = self
            .editor
            .active()
            .filter(|document| document.id == document_id)
        else {
            return;
        };
        #[cfg(feature = "shared-editor")]
        let byte = document
            .editing
            .as_ref()
            .map(|buffer| {
                buffer.selections().selections()[buffer.selections().primary_index()].head
            })
            .unwrap_or(0);
        #[cfg(not(feature = "shared-editor"))]
        let byte = 0;
        let Ok(position) = lumen_language_services::byte_to_position(&document.text, byte) else {
            self.view.status = "Could not map the caret for the language server.".into();
            return;
        };
        let stamp = self.request_stamp(document_id, revision, server_version, uri.clone());
        let request_id = self.language_server_requests.register(stamp);
        let message = match kind {
            LanguageRequestKind::Hover => lsp::hover_request(request_id, &uri, position),
            LanguageRequestKind::Definition => lsp::definition_request(request_id, &uri, position),
            LanguageRequestKind::Symbols | LanguageRequestKind::SemanticTokens => {
                unreachable!("this request does not use a caret position")
            }
        };
        self.send_language_request(request_id, kind, &message);
    }

    /// Ask for fresh full-document tokens for the active document when they are
    /// missing or stale. Silent: failures simply leave the parser baseline.
    fn request_semantic_tokens(&mut self, ctx: &eframe::egui::Context) {
        if !self.language_server_initialized
            || self
                .language_server_request_kinds
                .values()
                .any(|pending| pending.kind == LanguageRequestKind::SemanticTokens)
        {
            return;
        }
        let Some((document_id, revision, uri, server_version)) = self.active_synced_document()
        else {
            return;
        };
        let path = self
            .editor
            .active()
            .and_then(|document| document.path.clone());
        let enabled = self
            .view
            .settings
            .document_boolean("editor.semantic_highlighting", path.as_deref());
        if !enabled
            || !self
                .language_server_capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.semantic_tokens)
        {
            return;
        }
        if self
            .view
            .language_semantic_tokens
            .get(&document_id)
            .is_some_and(|snapshot| snapshot.revision == revision)
        {
            return;
        }
        if let Some(last) = self.semantic_token_request {
            if last.document == document_id && last.revision == revision {
                return;
            }
            let ready = last.at + SEMANTIC_TOKEN_DEBOUNCE;
            if last.document == document_id && ready > Instant::now() {
                ctx.request_repaint_after(ready.saturating_duration_since(Instant::now()));
                return;
            }
        }
        let stamp = self.request_stamp(document_id, revision, server_version, uri.clone());
        let request_id = self.language_server_requests.register(stamp);
        let message = lsp::semantic_tokens_request(request_id, &uri);
        self.semantic_token_request = Some(SemanticTokenAttempt {
            document: document_id,
            revision,
            at: Instant::now(),
        });
        self.send_language_request(request_id, LanguageRequestKind::SemanticTokens, &message);
    }

    fn apply_semantic_tokens_response(&mut self, message: &serde_json::Value) {
        let Some(legend) = self
            .language_server_capabilities
            .as_ref()
            .and_then(|capabilities| capabilities.semantic_legend.clone())
        else {
            return;
        };
        let Some(document) = self.editor.active() else {
            return;
        };
        let decoded = lumen_language_services::incoming::parse_semantic_tokens_response(message)
            .ok()
            .and_then(|data| {
                lumen_language_services::decode_semantic_tokens(&document.text, &legend, &data).ok()
            });
        let (id, revision) = (document.id, document.revision);
        match decoded {
            Some(tokens) => {
                let spans = tokens
                    .into_iter()
                    .map(|token| lumen_language_services::TokenSpan {
                        range: token.range,
                        class: token.class,
                    })
                    .collect();
                self.view.language_semantic_tokens.insert(
                    id,
                    super::view_state::SemanticSnapshot {
                        revision,
                        spans: std::sync::Arc::new(spans),
                    },
                );
            }
            // An invalid payload must not leave older tokens painted.
            None => {
                self.view.language_semantic_tokens.remove(&id);
            }
        }
    }

    fn request_stamp(
        &self,
        document_id: crate::model::DocumentId,
        revision: u64,
        server_version: ServerDocumentVersion,
        uri: DocumentUri,
    ) -> RequestStamp {
        RequestStamp {
            document: DocumentKey {
                id: document_id.language_service_id(),
                uri,
            },
            document_version: DocumentVersion(revision),
            server_version,
            session: self.language_server_generation,
        }
    }

    fn send_language_request(
        &mut self,
        request_id: u64,
        kind: LanguageRequestKind,
        message: &serde_json::Value,
    ) {
        let superseded = self
            .language_server_request_kinds
            .iter()
            .filter_map(|(id, pending)| (pending.kind == kind).then_some(*id))
            .collect::<Vec<_>>();
        for old_id in superseded {
            if let Some(process) = &mut self.language_server_process {
                let _ = process.send(&lsp::cancel_request(old_id));
            }
            self.language_server_request_kinds.remove(&old_id);
            self.language_server_requests.discard(old_id);
        }
        let result = self
            .language_server_process
            .as_mut()
            .ok_or_else(|| "Language server stopped".to_owned())
            .and_then(|process| process.send(message));
        match result {
            Ok(()) => {
                self.language_server_request_kinds.insert(
                    request_id,
                    PendingLanguageRequest {
                        kind,
                        deadline: Instant::now() + REQUEST_TIMEOUT,
                    },
                );
                // Background refreshes stay silent; only user-invoked requests report.
                if let Some(status) = match kind {
                    LanguageRequestKind::Hover => Some("Loading hover…"),
                    LanguageRequestKind::Definition => Some("Finding definition…"),
                    LanguageRequestKind::Symbols => Some("Loading document symbols…"),
                    LanguageRequestKind::SemanticTokens => None,
                } {
                    self.view.status = status.into();
                }
            }
            Err(error) => self.stop_language_server(&error),
        }
    }

    fn accept_language_response(&mut self, message: &serde_json::Value) -> bool {
        let Some(request_id) = message.get("id").and_then(serde_json::Value::as_u64) else {
            return false;
        };
        let Some(request) = self.language_server_request_kinds.remove(&request_id) else {
            return false;
        };
        let kind = request.kind;
        let Some(current) = self.current_request_stamp() else {
            return true;
        };
        if self
            .language_server_requests
            .take_current(request_id, &current)
            .is_none()
        {
            return true;
        }
        match kind {
            LanguageRequestKind::Hover => {
                match lumen_language_services::incoming::parse_hover_response(message) {
                    Ok(Some(hover)) => {
                        self.view.language_hover = Some(hover.text);
                        self.view.status = "Hover ready".into();
                    }
                    Ok(None) => {
                        self.view.language_hover = None;
                        self.view.status = "No hover information".into();
                    }
                    Err(_) => {
                        self.view.status =
                            "Language server returned an invalid hover response".into()
                    }
                }
            }
            LanguageRequestKind::Definition => self.apply_definition_response(message),
            LanguageRequestKind::Symbols => self.apply_document_symbols_response(message),
            LanguageRequestKind::SemanticTokens => self.apply_semantic_tokens_response(message),
        }
        true
    }

    fn current_request_stamp(&self) -> Option<RequestStamp> {
        let (id, revision, uri, server_version) = self.active_synced_document()?;
        Some(self.request_stamp(id, revision, server_version, uri))
    }

    fn apply_definition_response(&mut self, message: &serde_json::Value) {
        let locations =
            match lumen_language_services::incoming::parse_definition_locations_response(message) {
                Ok(locations) => locations,
                Err(_) => {
                    self.view.status = "Language server returned an invalid definition".into();
                    return;
                }
            };
        let mut targets = locations
            .into_iter()
            .filter_map(|location| {
                let path = file_uri_to_path(&location.uri)?;
                let name = path.file_name()?.to_string_lossy();
                Some(super::view_state::LanguageLocation {
                    label: format!(
                        "{name}:{}:{}",
                        location.start.line + 1,
                        location.start.character + 1
                    ),
                    path,
                    position: location.start,
                })
            })
            .collect::<Vec<_>>();
        if targets.is_empty() {
            self.view.status = "No definition found".into();
            return;
        }
        if targets.len() == 1 {
            self.view.language_navigation.locations = targets;
            self.open_language_location(0);
            self.view.status = "Opened definition".into();
            return;
        }
        targets.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.label.cmp(&right.label))
        });
        self.show_language_locations("Go to Definition", targets);
    }

    fn apply_document_symbols_response(&mut self, message: &serde_json::Value) {
        let symbols =
            match lumen_language_services::incoming::parse_document_symbols_response(message) {
                Ok(symbols) => symbols,
                Err(_) => {
                    self.view.status = "Language server returned invalid document symbols".into();
                    return;
                }
            };
        let Some(path) = self
            .editor
            .active()
            .and_then(|document| document.path.clone())
        else {
            return;
        };
        let mut locations = Vec::new();
        flatten_symbols(&symbols, &path, 0, &mut locations);
        if locations.is_empty() {
            self.view.status = "No document symbols found".into();
            return;
        }
        self.show_language_locations("Go to Symbol in Editor", locations);
    }

    fn show_language_locations(
        &mut self,
        title: &str,
        locations: Vec<super::view_state::LanguageLocation>,
    ) {
        let navigation = &mut self.view.language_navigation;
        navigation.open = true;
        navigation.title = title.into();
        navigation.query.clear();
        navigation.focus = true;
        navigation.selected = 0;
        navigation.locations = locations;
        self.view.status = format!("{} locations", navigation.locations.len());
    }
}

fn flatten_symbols(
    symbols: &[lumen_language_services::incoming::DocumentSymbol],
    path: &std::path::Path,
    depth: usize,
    output: &mut Vec<super::view_state::LanguageLocation>,
) {
    for symbol in symbols {
        output.push(super::view_state::LanguageLocation {
            label: format!("{}{}", "  ".repeat(depth), symbol.name),
            path: path.to_path_buf(),
            position: symbol.start,
        });
        flatten_symbols(&symbol.children, path, depth + 1, output);
    }
}

fn file_uri(path: &std::path::Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let escaped = text.bytes().fold(String::new(), |mut uri, byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(uri, "%{byte:02X}");
        }
        uri
    });
    format!("file://{escaped}")
}

fn file_uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut input = encoded.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = hex(input.next()?)?;
            let low = hex(input.next()?)?;
            bytes.push(high * 16 + low);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    #[cfg(windows)]
    let path = path.strip_prefix('/').unwrap_or(&path).to_owned();
    Some(std::path::PathBuf::from(path))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::language_server::{
        LanguageServerConnection, LanguageServerService, ServerWake,
    };
    use std::sync::{Arc, Mutex};

    type RecordedMessages = Arc<Mutex<Vec<serde_json::Value>>>;
    type QueuedEvents = Arc<Mutex<Vec<ServerEvent>>>;

    #[derive(Default)]
    struct FakeLanguageServer {
        messages: Arc<Mutex<Vec<serde_json::Value>>>,
        events: Arc<Mutex<Vec<ServerEvent>>>,
    }

    struct FakeConnection {
        messages: Arc<Mutex<Vec<serde_json::Value>>>,
        events: Arc<Mutex<Vec<ServerEvent>>>,
    }

    impl LanguageServerConnection for FakeConnection {
        fn send(&mut self, message: &serde_json::Value) -> Result<(), String> {
            self.messages.lock().unwrap().push(message.clone());
            Ok(())
        }

        fn drain_events(&self) -> Vec<ServerEvent> {
            self.events.lock().unwrap().drain(..).collect()
        }

        fn exit_reason(&mut self) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn output_closed_reason(&mut self) -> String {
            "fake server exited".into()
        }
        fn begin_shutdown(&mut self) -> Result<(), String> {
            self.send(&lsp::shutdown_request(
                crate::services::language_server::SHUTDOWN_REQUEST_ID,
            ))
        }
        fn finish_shutdown(&mut self) {
            let _ = self.send(&lsp::exit_notification());
        }
    }

    impl LanguageServerService for FakeLanguageServer {
        fn availability(
            &self,
            _language: &lumen_language_services::LanguageDescriptor,
        ) -> ServerAvailability {
            ServerAvailability::Available {
                executable: "fake-lsp".into(),
            }
        }

        fn start(
            &self,
            _executable: &std::path::Path,
            _workspace: &std::path::Path,
            _wake: ServerWake,
        ) -> Result<Box<dyn LanguageServerConnection>, String> {
            Ok(Box::new(FakeConnection {
                messages: Arc::clone(&self.messages),
                events: Arc::clone(&self.events),
            }))
        }
    }

    fn initialized() -> ServerEvent {
        ServerEvent::Message(serde_json::json!({
            "jsonrpc":"2.0", "id":0, "result":{"capabilities":{
                "textDocumentSync":{"openClose":true,"change":1,"save":{"includeText":true}},
                "hoverProvider":true,
                "definitionProvider":true,
                "documentSymbolProvider":true
            }}
        }))
    }

    fn fake_app() -> (LumenApp, RecordedMessages, QueuedEvents) {
        let service = FakeLanguageServer::default();
        let messages = Arc::clone(&service.messages);
        let events = Arc::clone(&service.events);
        let mut app = LumenApp {
            language_server_service: Arc::new(service),
            ..Default::default()
        };
        app.editor.workspace = Some("/workspace".into());
        (app, messages, events)
    }

    #[test]
    fn active_server_language_gets_a_cached_snapshot_and_parser_only_language_does_not() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/src/main.rs".into(),
            String::new(),
        ));
        app.editor.active_file = Some(0);
        let rust_id = app.editor.files[0].id;
        app.refresh_language_server_status(&eframe::egui::Context::default());
        assert_eq!(app.view.language_server_document, Some(rust_id));
        assert!(app.view.language_server_status.is_some());

        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/settings.json".into(),
            String::new(),
        ));
        app.editor.active_file = Some(1);
        app.refresh_language_server_status(&eframe::egui::Context::default());
        assert!(app.view.language_server_status.is_none());
    }

    #[test]
    fn file_uris_round_trip_spaces_unicode_and_percent_signs() {
        let path = std::path::PathBuf::from("/workspace/a b/🌍%25.rs");
        assert_eq!(file_uri_to_path(&file_uri(&path)), Some(path));
        assert!(file_uri_to_path("https://example.com/main.rs").is_none());
        assert!(file_uri_to_path("file:///bad%Q0.rs").is_none());
    }

    #[test]
    fn navigation_requests_fail_non_destructively_without_a_ready_server() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/main.rs".into(),
            "fn main() {}".into(),
        ));
        app.editor.active_file = Some(0);
        app.request_language_definition();
        assert!(app.view.status.contains("does not provide"));
        assert!(app.view.pending_goto.is_none());
    }

    #[test]
    fn document_sync_waits_for_the_initialized_notification() {
        let mut app = LumenApp::default();
        app.editor.workspace = Some("/workspace".into());
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/main.rs".into(),
            "fn main() {}".into(),
        ));
        app.editor.active_file = Some(0);
        app.language_server_language = Some(LanguageId::RUST);

        app.sync_language_documents();

        assert!(app.language_server_documents.is_empty());
        assert!(!app.language_server_initialized);
    }

    #[test]
    fn fake_server_syncs_save_rename_and_close_for_all_open_documents() {
        let (mut app, messages, events) = fake_app();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/a.rs".into(),
            "fn a() {}".into(),
        ));
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/b.rs".into(),
            "fn b() {}".into(),
        ));
        app.editor.active_file = Some(0);
        let first = app.editor.files[0].id;
        let second = app.editor.files[1].id;
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);

        let methods = || {
            messages
                .lock()
                .unwrap()
                .iter()
                .filter_map(|message| {
                    message
                        .get("method")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            methods()
                .iter()
                .filter(|method| method.as_str() == "textDocument/didOpen")
                .count(),
            2
        );

        app.editor.files[0].text.push_str("\n// changed");
        app.editor.files[0].record_text_change();
        app.notify_language_document_saved(first);
        assert!(methods().contains(&"textDocument/didChange".to_owned()));
        assert!(methods().contains(&"textDocument/didSave".to_owned()));

        app.editor.files[0].relocate("/workspace/renamed.rs".into());
        app.poll_language_server(&ctx);
        assert_eq!(
            methods()
                .iter()
                .filter(|method| method.as_str() == "textDocument/didClose")
                .count(),
            1
        );
        assert_eq!(
            methods()
                .iter()
                .filter(|method| method.as_str() == "textDocument/didOpen")
                .count(),
            3
        );

        app.editor.files.retain(|document| document.id != second);
        app.poll_language_server(&ctx);
        assert_eq!(
            methods()
                .iter()
                .filter(|method| method.as_str() == "textDocument/didClose")
                .count(),
            2
        );
    }

    #[test]
    fn fake_server_rejects_stale_diagnostics_and_schedules_bounded_restart() {
        let (mut app, _messages, events) = fake_app();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/a.rs".into(),
            "fn a() {}".into(),
        ));
        app.editor.active_file = Some(0);
        let document = app.editor.files[0].id;
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);
        app.editor.files[0].text.push('x');
        app.editor.files[0].record_text_change();
        app.poll_language_server(&ctx);
        events.lock().unwrap().push(ServerEvent::Message(serde_json::json!({
            "jsonrpc":"2.0", "method":"textDocument/publishDiagnostics", "params":{
                "uri":"file:///workspace/a.rs", "version":1,
                "diagnostics":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"message":"stale"}]
            }
        })));
        app.poll_language_server(&ctx);
        assert!(!app.view.language_server_diagnostics.contains_key(&document));

        events.lock().unwrap().push(ServerEvent::Exited);
        app.poll_language_server(&ctx);
        assert_eq!(app.language_server_restart.attempts, 1);
        assert!(app.language_server_restart.retry_at.is_some());
        assert!(matches!(
            app.view.language_server_status,
            Some(LanguageServerStatus::Error(_))
        ));
    }

    #[test]
    fn workspace_close_uses_shutdown_then_exit_and_restart_is_bounded() {
        let (mut app, messages, events) = fake_app();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/a.rs".into(),
            "fn a() {}".into(),
        ));
        app.editor.active_file = Some(0);
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);

        app.close_workspace_language_server();
        assert!(messages.lock().unwrap().iter().any(|message| {
            message.get("method").and_then(serde_json::Value::as_str) == Some("shutdown")
        }));
        events.lock().unwrap().push(ServerEvent::Message(
            serde_json::json!({"jsonrpc":"2.0", "id":crate::services::language_server::SHUTDOWN_REQUEST_ID, "result":null}),
        ));
        app.poll_language_server(&ctx);
        assert!(messages.lock().unwrap().iter().any(|message| {
            message.get("method").and_then(serde_json::Value::as_str) == Some("exit")
        }));

        for _ in 0..=MAX_RESTART_ATTEMPTS {
            app.schedule_language_server_restart_without_context("crashed");
        }
        assert!(app.language_server_restart.retry_at.is_none());
        assert!(matches!(
            app.view.language_server_status,
            Some(LanguageServerStatus::Error(ref message)) if message.contains("limit reached")
        ));
    }

    #[test]
    fn opening_a_problem_targets_the_exact_diagnostic_range() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/a.rs".into(),
            "abcdef".into(),
        ));
        let document = app.editor.files[0].id;
        app.open_problem(document, 1..4);
        assert_eq!(app.editor.active_file, Some(0));
        assert_eq!(app.view.pending_goto, Some((document, 1)));
        assert_eq!(app.view.pending_problem_selection, Some((document, 1..4)));
    }

    #[test]
    fn multiple_definitions_open_a_searchable_picker_without_guessing() {
        let mut app = LumenApp::default();
        let message = serde_json::json!({"result":[
            {"uri":"file:///workspace/b.rs","range":{"start":{"line":3,"character":2},"end":{"line":3,"character":4}}},
            {"uri":"file:///workspace/a.rs","range":{"start":{"line":1,"character":0},"end":{"line":1,"character":1}}}
        ]});
        app.apply_definition_response(&message);
        assert!(app.view.language_navigation.open);
        assert_eq!(app.view.language_navigation.title, "Go to Definition");
        assert_eq!(app.view.language_navigation.locations.len(), 2);
        assert!(
            app.view.language_navigation.locations[0]
                .path
                .ends_with("a.rs")
        );
    }

    #[test]
    fn nested_document_symbols_keep_hierarchy_and_positions() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/main.rs".into(),
            "struct Thing;".into(),
        ));
        app.editor.active_file = Some(0);
        let message = serde_json::json!({"result":[{
            "name":"Thing","kind":23,
            "selectionRange":{"start":{"line":0,"character":7},"end":{"line":0,"character":12}},
            "children":[{"name":"run","kind":6,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}}}]
        }]});
        app.apply_document_symbols_response(&message);
        assert!(app.view.language_navigation.open);
        assert_eq!(app.view.language_navigation.locations[0].label, "Thing");
        assert_eq!(app.view.language_navigation.locations[1].label, "  run");
        assert_eq!(
            app.view.language_navigation.locations[0].position.character,
            7
        );
    }

    fn rust_app() -> (LumenApp, RecordedMessages, QueuedEvents) {
        let (mut app, messages, events) = fake_app();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/a.rs".into(),
            "fn a() {}".into(),
        ));
        app.editor.active_file = Some(0);
        (app, messages, events)
    }

    #[test]
    fn disabled_language_never_starts_and_stops_a_running_session() {
        let (mut app, messages, _events) = rust_app();
        app.view.settings.user.persisted.insert(
            "language.rust.enabled".into(),
            lumen_settings_core::Value::Bool(false),
        );
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        assert!(app.language_server_process.is_none());
        assert!(app.view.language_server_status.is_none());
        assert!(messages.lock().unwrap().is_empty());

        app.view.settings.user.persisted.clear();
        app.refresh_language_server_status(&ctx);
        assert!(app.language_server_process.is_some());
        app.view.settings.user.persisted.insert(
            "language.rust.enabled".into(),
            lumen_settings_core::Value::Bool(false),
        );
        app.refresh_language_server_status(&ctx);
        assert!(app.language_server_process.is_none());
        assert!(app.language_server_language.is_none());
    }

    #[test]
    fn configured_server_path_must_be_an_existing_absolute_file() {
        let (mut app, _messages, _events) = rust_app();
        app.view.settings.user.persisted.insert(
            "language.rust.server_path".into(),
            lumen_settings_core::Value::Text("relative/rust-analyzer".into()),
        );
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        assert!(app.language_server_process.is_none());
        assert!(matches!(
            app.view.language_server_status,
            Some(LanguageServerStatus::Missing(ref path)) if path == "relative/rust-analyzer"
        ));
    }

    #[test]
    fn startup_timeout_setting_controls_initialize_deadline() {
        let mut app = LumenApp::default();
        assert_eq!(app.initialize_timeout(), Duration::from_secs(12));
        app.view.settings.user.persisted.insert(
            "language.server.startup_timeout_seconds".into(),
            lumen_settings_core::Value::Number(30.0),
        );
        assert_eq!(app.initialize_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn session_root_is_the_nearest_marker_and_sync_stays_inside_it() {
        let base = std::env::temp_dir().join(format!("lumen-root-{}", std::process::id()));
        let service = base.join("svc");
        std::fs::create_dir_all(service.join("pkg")).unwrap();
        std::fs::write(service.join("go.mod"), "module svc").unwrap();
        let (mut app, messages, events) = fake_app();
        app.editor.workspace = Some(base.clone());
        for path in [service.join("pkg/a.go"), base.join("loose.go")] {
            app.editor
                .files
                .push(crate::model::Document::from_path(path, "package a".into()));
        }
        app.editor.active_file = Some(0);
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        assert_eq!(
            app.language_server_workspace.as_deref(),
            Some(service.as_path())
        );
        let init = messages.lock().unwrap()[0].clone();
        assert_eq!(
            init["params"]["rootUri"].as_str(),
            Some(file_uri(&service).as_str())
        );
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);
        let opened = messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message["method"] == "textDocument/didOpen")
            .count();
        assert_eq!(opened, 1);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn server_initiated_requests_receive_replies() {
        let (mut app, messages, events) = rust_app();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(initialized());
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":7,"method":"client/registerCapability","params":{}
            })));
        app.poll_language_server(&ctx);
        assert!(messages.lock().unwrap().iter().any(|message| {
            message["id"] == 7 && message.get("result").is_some() && message.get("method").is_none()
        }));
    }

    #[test]
    fn project_settings_cannot_choose_the_server_executable() {
        let (mut app, _messages, _events) = rust_app();
        app.view.settings.root = Some("/workspace".into());
        app.view.settings.project.persisted.insert(
            "language.rust.server_path".into(),
            lumen_settings_core::Value::Text("/tmp/evil-analyzer".into()),
        );
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        let summary = app.view.language_server_summary.clone().unwrap();
        assert!(summary.project_path_ignored);
        assert_eq!(summary.command.as_deref(), Some("fake-lsp"));
        assert!(app.language_server_process.is_some());
    }

    #[test]
    fn summary_reports_state_command_capabilities_and_errors() {
        let (mut app, _messages, events) = rust_app();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        assert_eq!(
            app.view.language_server_summary.as_ref().unwrap().state,
            "Starting"
        );
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);
        app.refresh_language_server_status(&ctx);
        let summary = app.view.language_server_summary.clone().unwrap();
        assert_eq!(summary.language, "Rust");
        assert_eq!(summary.state, "Ready");
        assert!(summary.capabilities.contains(&"Hover"));
        assert!(summary.error.is_none());

        app.view.settings.user.persisted.insert(
            "language.rust.enabled".into(),
            lumen_settings_core::Value::Bool(false),
        );
        app.refresh_language_server_status(&ctx);
        assert_eq!(
            app.view.language_server_summary.unwrap().state,
            "Disabled in settings"
        );
    }

    fn semantic_initialized() -> ServerEvent {
        ServerEvent::Message(serde_json::json!({
            "jsonrpc":"2.0", "id":0, "result":{"capabilities":{
                "textDocumentSync":{"openClose":true,"change":1,"save":{"includeText":true}},
                "semanticTokensProvider":{
                    "legend":{"tokenTypes":["function","keyword"],"tokenModifiers":[]},
                    "full":true
                }
            }}
        }))
    }

    fn semantic_request_ids(messages: &RecordedMessages) -> Vec<u64> {
        messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message["method"] == "textDocument/semanticTokens/full")
            .filter_map(|message| message["id"].as_u64())
            .collect()
    }

    #[test]
    fn semantic_tokens_are_requested_stored_per_revision_and_dropped_when_stale() {
        let (mut app, messages, events) = rust_app();
        app.editor.files[0].text = "fn main() {}".into();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(semantic_initialized());
        app.poll_language_server(&ctx);
        let ids = semantic_request_ids(&messages);
        assert_eq!(ids.len(), 1);

        // "main" as a function; the keyword-typed token must not survive as a keyword refinement.
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":ids[0],"result":{"data":[0,3,4,0,0]}
            })));
        app.poll_language_server(&ctx);
        let document = app.editor.files[0].id;
        let snapshot = app.view.language_semantic_tokens[&document].clone();
        assert_eq!(snapshot.revision, app.editor.files[0].revision);
        assert_eq!(snapshot.spans[0].range, 3..7);
        // Same revision is never re-requested.
        app.poll_language_server(&ctx);
        assert_eq!(semantic_request_ids(&messages).len(), 1);

        // An edit invalidates the snapshot by revision and a late reply is discarded.
        app.editor.files[0].text = "fn mainx() {}".into();
        app.editor.files[0].revision += 1;
        app.poll_language_server(&ctx);
        assert!(!app.view.language_semantic_tokens.contains_key(&document));
        let ids = semantic_request_ids(&messages);
        assert_eq!(ids.len(), 1, "debounced until the edit settles");
        let late = ids[0];
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":late,"result":{"data":[0,3,4,0,0]}
            })));
        app.poll_language_server(&ctx);
        assert!(!app.view.language_semantic_tokens.contains_key(&document));
    }

    #[test]
    fn disabled_semantic_setting_or_missing_capability_sends_no_request() {
        let (mut app, messages, events) = rust_app();
        app.view.settings.user.persisted.insert(
            "editor.semantic_highlighting".into(),
            lumen_settings_core::Value::Bool(false),
        );
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(semantic_initialized());
        app.poll_language_server(&ctx);
        assert!(semantic_request_ids(&messages).is_empty());

        let (mut app, messages, events) = rust_app();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);
        assert!(semantic_request_ids(&messages).is_empty());
    }

    #[test]
    fn malformed_semantic_reply_clears_tokens_without_changing_status() {
        let (mut app, messages, events) = rust_app();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(semantic_initialized());
        app.poll_language_server(&ctx);
        let id = semantic_request_ids(&messages)[0];
        let status = app.view.status.clone();
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":id,"result":{"data":[1,2,3]}
            })));
        app.poll_language_server(&ctx);
        assert!(app.view.language_semantic_tokens.is_empty());
        assert_eq!(app.view.status, status);
    }
}
