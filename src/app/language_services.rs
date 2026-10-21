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

impl SyncedLanguageDocument {
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn server_version(&self) -> ServerDocumentVersion {
        self.server_version
    }
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
    /// Caret byte for caret-relative requests, to reject replies after it moved.
    caret: Option<usize>,
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
    Completion,
    SignatureHelp,
    References,
    Formatting,
    Rename,
}

/// The last semantic-token request, so a failed or empty reply is not retried
/// for the same revision and edits are throttled.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SemanticTokenAttempt {
    document: crate::model::DocumentId,
    revision: u64,
    at: Instant,
}

/// A validated caret-relative request target.
pub(super) struct CaretContext {
    pub(super) document_id: crate::model::DocumentId,
    pub(super) revision: u64,
    pub(super) uri: DocumentUri,
    pub(super) server_version: ServerDocumentVersion,
    pub(super) byte: usize,
    pub(super) position: lumen_language_services::Position,
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
        self.open_file_unpinned(location.path.clone());
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
        self.editor.set_active_index(Some(index));
        self.view.leave_page_tabs();
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
                (capabilities.completion, "Completion"),
                (capabilities.references, "References"),
                (capabilities.rename, "Rename"),
                (capabilities.formatting, "Formatting"),
                (capabilities.signature_help, "Signature help"),
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
                        crate::services::logger::info(
                            "language-server",
                            &format!("Starting {name} in {}", workspace.display()),
                        );
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
                            crate::services::logger::info("language-server", "Initialized; ready");
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
                ServerEvent::Message(message) if log_server_message(&message) => {}
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
        crate::services::logger::error("language-server", error);
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

    pub(super) fn active_synced_document(
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
            crate::services::logger::error(
                "language-server",
                "Automatic restart limit reached; the server stays stopped until the workspace or file changes",
            );
            self.language_server_restart.retry_at = None;
            self.view.language_server_status = Some(LanguageServerStatus::Error(format!(
                "{error} (automatic restart limit reached)"
            )));
            return;
        }
        let delay =
            Duration::from_secs(1_u64 << (self.language_server_restart.attempts - 1).min(4));
        self.language_server_restart.retry_at = Some(Instant::now() + delay);
        crate::services::logger::warn(
            "language-server",
            &format!(
                "Restarting in {}s (attempt {})",
                delay.as_secs(),
                self.language_server_restart.attempts
            ),
        );
        self.view.language_server_status = Some(LanguageServerStatus::Error(format!(
            "{error} (retrying in {}s)",
            delay.as_secs()
        )));
    }

    /// Validate capability, sync state and caret for a caret-relative request.
    pub(super) fn caret_context(
        &mut self,
        supported: impl Fn(&ServerCapabilities) -> bool,
    ) -> Option<CaretContext> {
        if !self
            .language_server_capabilities
            .as_ref()
            .is_some_and(supported)
        {
            self.view.status = "The active language server does not provide this feature.".into();
            return None;
        }
        let Some((document_id, revision, uri, server_version)) = self.active_synced_document()
        else {
            self.view.status = "Language server is not ready for this document.".into();
            return None;
        };
        let document = self
            .editor
            .active()
            .filter(|document| document.id == document_id)?;
        let byte = document
            .editing
            .as_ref()
            .map(|buffer| {
                buffer.selections().selections()[buffer.selections().primary_index()].head
            })
            .unwrap_or(0);
        let Ok(position) = lumen_language_services::byte_to_position(&document.text, byte) else {
            self.view.status = "Could not map the caret for the language server.".into();
            return None;
        };
        Some(CaretContext {
            document_id,
            revision,
            uri,
            server_version,
            byte,
            position,
        })
    }

    fn request_at_caret(&mut self, kind: LanguageRequestKind) {
        let Some(context) = self.caret_context(|capabilities| match kind {
            LanguageRequestKind::Hover => capabilities.hover,
            LanguageRequestKind::Definition => capabilities.definition,
            LanguageRequestKind::Completion => capabilities.completion,
            LanguageRequestKind::SignatureHelp => capabilities.signature_help,
            LanguageRequestKind::References => capabilities.references,
            _ => false,
        }) else {
            return;
        };
        let CaretContext {
            document_id,
            revision,
            uri,
            server_version,
            byte,
            position,
        } = context;
        let stamp = self.request_stamp(document_id, revision, server_version, uri.clone());
        let request_id = self.language_server_requests.register(stamp);
        let message = match kind {
            LanguageRequestKind::Hover => lsp::hover_request(request_id, &uri, position),
            LanguageRequestKind::Definition => lsp::definition_request(request_id, &uri, position),
            LanguageRequestKind::Completion => lsp::completion_request(request_id, &uri, position),
            LanguageRequestKind::SignatureHelp => {
                lsp::signature_help_request(request_id, &uri, position)
            }
            LanguageRequestKind::References => lsp::references_request(request_id, &uri, position),
            _ => unreachable!("this request does not use a plain caret position"),
        };
        self.send_language_request_at(request_id, kind, Some(byte), &message);
    }

    pub(crate) fn request_language_references(&mut self) {
        self.request_at_caret(LanguageRequestKind::References);
    }

    pub(crate) fn request_language_completion(&mut self) {
        self.view.language_completion = None;
        self.request_at_caret(LanguageRequestKind::Completion);
    }

    pub(crate) fn request_language_signature_help(&mut self) {
        self.view.language_signature = None;
        self.request_at_caret(LanguageRequestKind::SignatureHelp);
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

    /// Caret byte of the active document's primary selection, only when it is a
    /// plain caret equal to `expected`; replies for a moved caret are dropped.
    pub(super) fn caret_is_still_at(&self, expected: Option<usize>) -> Option<usize> {
        let buffer = self.editor.active()?.editing.as_ref()?;
        let set = buffer.selections();
        let selection = set.selections()[set.primary_index()];
        (set.selections().len() == 1
            && selection.anchor == selection.head
            && Some(selection.head) == expected)
            .then_some(selection.head)
    }

    fn apply_completion_response(&mut self, message: &serde_json::Value, caret: Option<usize>) {
        let Some(caret) = self.caret_is_still_at(caret) else {
            self.view.status = "Completion discarded: the caret moved".into();
            return;
        };
        let Some(document) = self.editor.active() else {
            return;
        };
        let Ok(items) =
            lumen_language_services::incoming::parse_completion_response(message, &document.text)
        else {
            self.view.status = "Language server returned invalid completions".into();
            return;
        };
        if items.is_empty() {
            self.view.status = "No completions".into();
            return;
        }
        self.view.status = format!("{} completions", items.len());
        self.view.language_completion = Some(super::view_state::CompletionSession {
            document: document.id,
            origin: caret,
            word_start: super::completion::word_start(&document.text, caret),
            items,
            selected: 0,
            pending_accept: None,
        });
    }

    fn apply_signature_help_response(&mut self, message: &serde_json::Value, caret: Option<usize>) {
        if self.caret_is_still_at(caret).is_none() {
            self.view.status = "Signature help discarded: the caret moved".into();
            return;
        }
        let Some(document) = self.editor.active() else {
            return;
        };
        match lumen_language_services::incoming::parse_signature_help_response(message) {
            Ok(Some(help)) => {
                self.view.language_signature = Some((document.id, document.revision, help));
                self.view.status = "Signature help ready".into();
            }
            Ok(None) => self.view.status = "No signature help".into(),
            Err(_) => self.view.status = "Language server returned invalid signature help".into(),
        }
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

    pub(super) fn request_stamp(
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
        self.send_language_request_at(request_id, kind, None, message);
    }

    pub(super) fn send_language_request_at(
        &mut self,
        request_id: u64,
        kind: LanguageRequestKind,
        caret: Option<usize>,
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
                        caret,
                    },
                );
                // Background refreshes stay silent; only user-invoked requests report.
                if let Some(status) = match kind {
                    LanguageRequestKind::Hover => Some("Loading hover…"),
                    LanguageRequestKind::Definition => Some("Finding definition…"),
                    LanguageRequestKind::Symbols => Some("Loading document symbols…"),
                    LanguageRequestKind::Completion => Some("Loading completions…"),
                    LanguageRequestKind::SignatureHelp => Some("Loading signature help…"),
                    LanguageRequestKind::References => Some("Finding references…"),
                    LanguageRequestKind::Formatting => Some("Formatting…"),
                    LanguageRequestKind::Rename => Some("Computing rename…"),
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
                        // Only show it if the caret is still where it was asked.
                        {
                            let anchor = self
                                .caret_is_still_at(request.caret)
                                .zip(self.editor.active().map(|document| document.id));
                            let Some((byte, document)) = anchor else {
                                self.view.status = "Hover discarded: the caret moved".into();
                                return true;
                            };
                            self.view.language_hover_anchor = Some((document, byte));
                        }
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
            LanguageRequestKind::Definition => {
                self.apply_location_response(message, "Go to Definition", true)
            }
            LanguageRequestKind::Symbols => self.apply_document_symbols_response(message),
            LanguageRequestKind::SemanticTokens => self.apply_semantic_tokens_response(message),
            LanguageRequestKind::Completion => {
                self.apply_completion_response(message, request.caret)
            }
            LanguageRequestKind::SignatureHelp => {
                self.apply_signature_help_response(message, request.caret)
            }
            LanguageRequestKind::References => {
                self.apply_location_response(message, "Find References", false)
            }
            LanguageRequestKind::Formatting => self.apply_formatting_response(message),
            LanguageRequestKind::Rename => self.apply_rename_response(message, request.caret),
        }
        true
    }

    pub(super) fn current_request_stamp(&self) -> Option<RequestStamp> {
        let (id, revision, uri, server_version) = self.active_synced_document()?;
        Some(self.request_stamp(id, revision, server_version, uri))
    }

    fn apply_location_response(
        &mut self,
        message: &serde_json::Value,
        title: &'static str,
        open_single: bool,
    ) {
        let locations =
            match lumen_language_services::incoming::parse_definition_locations_response(message) {
                Ok(locations) => locations,
                Err(_) => {
                    self.view.status =
                        format!("Language server returned invalid locations for {title}");
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
            self.view.status = format!("{title}: nothing found");
            return;
        }
        if open_single && targets.len() == 1 {
            self.view.language_navigation.locations = targets;
            self.open_language_location(0);
            self.view.status = format!("{title}: opened");
            return;
        }
        targets.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then(left.label.cmp(&right.label))
        });
        self.show_language_locations(title, targets);
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

    pub(super) fn show_language_locations(
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

/// Server `window/logMessage` and `window/showMessage` notifications go to the
/// Output log. Returns true when the message was one of those and is handled.
fn log_server_message(message: &serde_json::Value) -> bool {
    use crate::model::output_log::LogLevel;
    let method = message.get("method").and_then(serde_json::Value::as_str);
    if !matches!(method, Some("window/logMessage" | "window/showMessage"))
        || message.get("id").is_some()
    {
        return false;
    }
    let params = &message["params"];
    let level = match params["type"].as_u64() {
        Some(1) => LogLevel::Error,
        Some(2) => LogLevel::Warn,
        _ => LogLevel::Info,
    };
    if let Some(text) = params["message"].as_str() {
        crate::services::logger::log(level, "language-server", text);
    }
    true
}

pub(super) fn file_uri(path: &std::path::Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    // A drive path needs the authority-empty form `file:///C%3A/...`.
    if has_drive_prefix(&text) {
        text.insert(0, '/');
    }
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

pub(super) fn file_uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
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
    // `/C:/dir` is a Windows drive path; a plain `/dir` stays rooted as written.
    #[cfg(windows)]
    let path = match path.strip_prefix('/') {
        Some(rest) if has_drive_prefix(rest) => rest.to_owned(),
        _ => path,
    };
    Some(std::path::PathBuf::from(path))
}

fn has_drive_prefix(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
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

    #[cfg(feature = "language-server")]
    #[test]
    fn active_server_language_gets_a_cached_snapshot_and_parser_only_language_does_not() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/src/main.rs".into(),
            String::new(),
        ));
        app.editor.set_active_index(Some(0));
        let rust_id = app.editor.files[0].id;
        app.refresh_language_server_status(&eframe::egui::Context::default());
        assert_eq!(app.view.language_server_document, Some(rust_id));
        assert!(app.view.language_server_status.is_some());

        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/settings.json".into(),
            String::new(),
        ));
        app.editor.set_active_index(Some(1));
        app.refresh_language_server_status(&eframe::egui::Context::default());
        assert!(app.view.language_server_status.is_none());
    }

    #[test]
    fn file_uris_round_trip_spaces_unicode_and_percent_signs() {
        let path = std::path::PathBuf::from("/workspace/a b/🌍%25.rs");
        assert_eq!(file_uri_to_path(&file_uri(&path)), Some(path));
        assert!(file_uri_to_path("https://example.com/main.rs").is_none());
        // Drive paths use the three-slash form servers expect on Windows.
        assert_eq!(
            file_uri(std::path::Path::new("C:\\proj\\a b.rs")),
            "file:///C%3A/proj/a%20b.rs"
        );
        #[cfg(windows)]
        assert_eq!(
            file_uri_to_path("file:///C%3A/proj/a%20b.rs"),
            Some(std::path::PathBuf::from("C:/proj/a b.rs"))
        );
        assert!(file_uri_to_path("file:///bad%Q0.rs").is_none());
    }

    #[test]
    fn navigation_requests_fail_non_destructively_without_a_ready_server() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/main.rs".into(),
            "fn main() {}".into(),
        ));
        app.editor.set_active_index(Some(0));
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
        app.editor.set_active_index(Some(0));
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
        app.editor.set_active_index(Some(0));
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
        app.editor.set_active_index(Some(0));
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
        app.editor.set_active_index(Some(0));
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
        assert_eq!(app.editor.active_index(), Some(0));
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
        app.apply_location_response(&message, "Go to Definition", true);
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
        app.editor.set_active_index(Some(0));
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

    /// Drives the real app against a real rust-analyzer; run with
    /// `cargo test live_app_diagnostics -- --ignored --nocapture`.
    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires rust-analyzer and a Rust toolchain on PATH"]
    fn live_app_diagnostics_reach_the_problem_list_for_a_syntax_error() {
        let root = std::env::temp_dir().join(format!("lumen-live-app-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let file = root.join("src/main.rs");
        let text = "fn main() {\n    let x = ;\n    foo(\n}\n";
        std::fs::write(&file, text).unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.editor
            .files
            .push(crate::model::Document::from_path(file, text.into()));
        app.editor.set_active_index(Some(0));
        let ctx = eframe::egui::Context::default();
        let id = app.editor.files[0].id;
        let deadline = Instant::now() + Duration::from_secs(90);
        while Instant::now() < deadline
            && app
                .view
                .language_server_diagnostics
                .get(&id)
                .is_none_or(|found| found.is_empty())
        {
            app.refresh_language_server_status(&ctx);
            app.poll_language_server(&ctx);
            std::thread::sleep(Duration::from_millis(50));
        }
        let found = app.view.language_server_diagnostics.get(&id).cloned();
        println!("status: {:?}", app.view.language_server_status);
        println!("diagnostics: {found:#?}");
        let _ = std::fs::remove_dir_all(&root);
        assert!(found.is_some_and(|found| !found.is_empty()));
    }

    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "requires rust-analyzer and a Rust toolchain on PATH"]
    fn live_app_diagnostics_follow_an_edit_that_breaks_a_valid_file() {
        let root = std::env::temp_dir().join(format!("lumen-live-edit-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"live\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let file = root.join("src/main.rs");
        let valid = "fn main() {\n\n\n\n\n\n\n\n    let x = 1;\n}\n";
        std::fs::write(&file, valid).unwrap();
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.editor
            .files
            .push(crate::model::Document::from_path(file, valid.into()));
        app.editor.set_active_index(Some(0));
        app.editor.files[0].enable_shared_editing();
        let ctx = eframe::egui::Context::default();
        let id = app.editor.files[0].id;
        let pump = |app: &mut LumenApp, seconds: u64, until_error: bool| {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            while Instant::now() < deadline {
                app.refresh_language_server_status(&ctx);
                app.poll_language_server(&ctx);
                if until_error
                    && app
                        .view
                        .language_server_diagnostics
                        .get(&id)
                        .is_some_and(|found| {
                            found.iter().any(|d| d.message.contains("Syntax Error"))
                        })
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        pump(&mut app, 8, false);
        println!(
            "after open: {:?}",
            app.view.language_server_diagnostics.get(&id)
        );
        app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .replace_document("fn main() {\n    let x = ;\n}\n")
            .unwrap();
        app.editor.files[0].sync_shared_editing();
        println!("revision now {}", app.editor.files[0].revision);
        pump(&mut app, 60, true);
        let found = app.view.language_server_diagnostics.get(&id).cloned();
        println!(
            "after edit: {:#?}",
            found.as_ref().map(|f| f
                .iter()
                .map(|d| (d.range.clone(), d.message.clone()))
                .collect::<Vec<_>>())
        );
        let _ = std::fs::remove_dir_all(&root);
        assert!(found.is_some_and(|f| f.iter().any(|d| d.message.contains("Syntax Error"))));
    }

    #[test]
    #[cfg(feature = "language-server")]
    #[ignore = "opens this repository in rust-analyzer; slow"]
    fn live_app_diagnostics_in_this_repository() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let file = root.join("src/ui/shortcut.rs");
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push_str("\nfn broken() { let x = ; }\n");
        let mut app = LumenApp::default();
        app.editor.workspace = Some(root.clone());
        app.editor
            .files
            .push(crate::model::Document::from_path(file, text));
        app.editor.set_active_index(Some(0));
        let ctx = eframe::egui::Context::default();
        let id = app.editor.files[0].id;
        let start = Instant::now();
        let deadline = start + Duration::from_secs(240);
        let mut seen = 0;
        while Instant::now() < deadline {
            app.refresh_language_server_status(&ctx);
            app.poll_language_server(&ctx);
            let found = app.view.language_server_diagnostics.get(&id);
            let count = found.map_or(0, Vec::len);
            if count != seen {
                seen = count;
                println!("{:?}: {count} diagnostics", start.elapsed());
            }
            if found.is_some_and(|f| f.iter().any(|d| d.message.contains("Syntax Error"))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        println!(
            "status {:?} elapsed {:?}",
            app.view.language_server_status,
            start.elapsed()
        );
        assert!(
            app.view
                .language_server_diagnostics
                .get(&id)
                .is_some_and(|f| { f.iter().any(|d| d.message.contains("Syntax Error")) })
        );
    }

    fn rust_app() -> (LumenApp, RecordedMessages, QueuedEvents) {
        let (mut app, messages, events) = fake_app();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/a.rs".into(),
            "fn a() {}".into(),
        ));
        app.editor.set_active_index(Some(0));
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
        app.editor.set_active_index(Some(0));
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

    fn completion_initialized() -> ServerEvent {
        ServerEvent::Message(serde_json::json!({
            "jsonrpc":"2.0", "id":0, "result":{"capabilities":{
                "textDocumentSync":{"openClose":true,"change":1,"save":{"includeText":true}},
                "hoverProvider":true,
                "completionProvider":{"triggerCharacters":["."]},
                "signatureHelpProvider":{"triggerCharacters":["("]}
            }}
        }))
    }

    fn request_ids(messages: &RecordedMessages, method: &str) -> Vec<u64> {
        messages
            .lock()
            .unwrap()
            .iter()
            .filter(|message| message["method"] == method)
            .filter_map(|message| message["id"].as_u64())
            .collect()
    }

    fn completion_app(text: &str, caret: usize) -> (LumenApp, RecordedMessages, QueuedEvents) {
        let (mut app, messages, events) = rust_app();
        app.editor.files[0].text = text.into();
        app.editor.files[0].enable_shared_editing();
        let buffer = app.editor.files[0].editing.as_mut().unwrap();
        buffer
            .set_selections(vec![lumen_editor_core::Selection::caret(caret)], 0)
            .unwrap();
        app.editor.files[0].sync_shared_editing();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(completion_initialized());
        app.poll_language_server(&ctx);
        (app, messages, events)
    }

    #[test]
    fn completion_request_reply_opens_a_session_and_stale_replies_do_not() {
        let (mut app, messages, events) = completion_app("x.pu", 4);
        let ctx = eframe::egui::Context::default();
        app.request_language_completion();
        let id = request_ids(&messages, "textDocument/completion")[0];
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":id,"result":[{"label":"push"},{"label":"pull"}]
            })));
        app.poll_language_server(&ctx);
        let session = app.view.language_completion.clone().expect("session");
        assert_eq!(
            (session.word_start, session.origin, session.items.len()),
            (2, 4, 2)
        );

        // Moving the caret before the reply arrives discards it.
        app.view.language_completion = None;
        app.request_language_completion();
        let id = request_ids(&messages, "textDocument/completion")[1];
        app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(vec![lumen_editor_core::Selection::caret(1)], 0)
            .unwrap();
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":id,"result":[{"label":"push"}]
            })));
        app.poll_language_server(&ctx);
        assert!(app.view.language_completion.is_none());
    }

    #[test]
    fn completion_and_signature_need_server_capabilities() {
        let (mut app, messages, events) = rust_app();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(initialized());
        app.poll_language_server(&ctx);
        app.request_language_completion();
        app.request_language_signature_help();
        assert!(request_ids(&messages, "textDocument/completion").is_empty());
        assert!(request_ids(&messages, "textDocument/signatureHelp").is_empty());
    }

    #[test]
    fn hover_reply_is_anchored_to_the_requested_caret_and_dropped_if_it_moved() {
        let (mut app, messages, events) = completion_app("let value = 1;", 5);
        let ctx = eframe::egui::Context::default();
        let reply = |id: u64| {
            ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":id,"result":{"contents":"let value: i32"}
            }))
        };
        app.request_language_hover();
        let id = request_ids(&messages, "textDocument/hover")[0];
        events.lock().unwrap().push(reply(id));
        app.poll_language_server(&ctx);
        assert_eq!(app.view.language_hover.as_deref(), Some("let value: i32"));
        assert_eq!(
            app.view.language_hover_anchor,
            Some((app.editor.files[0].id, 5))
        );

        app.view.language_hover = None;
        app.view.language_hover_anchor = None;
        app.request_language_hover();
        let id = request_ids(&messages, "textDocument/hover")[1];
        app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(vec![lumen_editor_core::Selection::caret(9)], 0)
            .unwrap();
        events.lock().unwrap().push(reply(id));
        app.poll_language_server(&ctx);
        assert!(app.view.language_hover.is_none(), "stale hover is dropped");
        assert!(app.view.language_hover_anchor.is_none());
        // The caret was only read, never moved, by the hover machinery.
        let selections = app.editor.files[0].editing.as_ref().unwrap().selections();
        assert_eq!(selections.selections()[selections.primary_index()].head, 9);
    }

    #[test]
    fn signature_help_reply_is_stored_for_its_revision() {
        let (mut app, messages, events) = completion_app("f(", 2);
        let ctx = eframe::egui::Context::default();
        app.request_language_signature_help();
        let id = request_ids(&messages, "textDocument/signatureHelp")[0];
        events
            .lock()
            .unwrap()
            .push(ServerEvent::Message(serde_json::json!({
                "jsonrpc":"2.0","id":id,"result":{"signatures":[{
                    "label":"f(a: u8)","parameters":[{"label":"a: u8"}],"activeParameter":0}]}
            })));
        app.poll_language_server(&ctx);
        let (document, revision, help) = app.view.language_signature.clone().unwrap();
        assert_eq!(document, app.editor.files[0].id);
        assert_eq!(revision, app.editor.files[0].revision);
        assert_eq!(&help.label[help.active_parameter.unwrap()], "a: u8");
    }

    fn editing_initialized() -> ServerEvent {
        ServerEvent::Message(serde_json::json!({
            "jsonrpc":"2.0", "id":0, "result":{"capabilities":{
                "textDocumentSync":{"openClose":true,"change":1,"save":{"includeText":true}},
                "referencesProvider":true,
                "renameProvider":true,
                "documentFormattingProvider":true
            }}
        }))
    }

    /// Workspace with an open `a.rs` and a closed `b.rs`, both using `foo`.
    fn rename_app() -> (LumenApp, RecordedMessages, QueuedEvents, std::path::PathBuf) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "lumen-rename-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(base.join("b.rs"), "fn b() { foo(); }\n").unwrap();
        let (mut app, messages, events) = fake_app();
        app.editor.workspace = Some(base.clone());
        app.editor.files.push(crate::model::Document::from_path(
            base.join("a.rs"),
            "fn foo() {}\nfn a() { foo(); }\n".into(),
        ));
        app.editor.set_active_index(Some(0));
        app.editor.files[0].enable_shared_editing();
        let ctx = eframe::egui::Context::default();
        app.refresh_language_server_status(&ctx);
        events.lock().unwrap().push(editing_initialized());
        app.poll_language_server(&ctx);
        (app, messages, events, base)
    }

    fn set_caret(app: &mut LumenApp, byte: usize) {
        app.editor.files[0]
            .editing
            .as_mut()
            .unwrap()
            .set_selections(vec![lumen_editor_core::Selection::caret(byte)], 0)
            .unwrap();
    }

    fn rename_edit(base: &std::path::Path) -> serde_json::Value {
        let range = |l: u32, s: u32, e: u32| serde_json::json!({"start":{"line":l,"character":s},"end":{"line":l,"character":e}});
        serde_json::json!({"changes":{
            file_uri(&base.join("a.rs")): [
                {"range":range(0,3,6),"newText":"bar"},
                {"range":range(1,9,12),"newText":"bar"}],
            file_uri(&base.join("b.rs")): [{"range":range(0,9,12),"newText":"bar"}]
        }})
    }

    fn reply(events: &QueuedEvents, id: u64, result: serde_json::Value) {
        events.lock().unwrap().push(ServerEvent::Message(
            serde_json::json!({"jsonrpc":"2.0","id":id,"result":result}),
        ));
    }

    #[test]
    fn references_always_open_the_picker_even_for_one_location() {
        let (mut app, messages, events, base) = rename_app();
        set_caret(&mut app, 4);
        let ctx = eframe::egui::Context::default();
        app.request_language_references();
        let id = request_ids(&messages, "textDocument/references")[0];
        reply(
            &events,
            id,
            serde_json::json!([{"uri": file_uri(&base.join("b.rs")),
                "range":{"start":{"line":0,"character":9},"end":{"line":0,"character":12}}}]),
        );
        app.poll_language_server(&ctx);
        assert!(app.view.language_navigation.open);
        assert_eq!(app.view.language_navigation.locations.len(), 1);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn formatting_applies_as_one_undo_step_and_ignores_stale_replies() {
        let (mut app, messages, events, base) = rename_app();
        set_caret(&mut app, 0);
        let ctx = eframe::egui::Context::default();
        app.request_language_formatting();
        let id = request_ids(&messages, "textDocument/formatting")[0];
        reply(
            &events,
            id,
            serde_json::json!([{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":3}},"newText":"  "}]),
        );
        app.poll_language_server(&ctx);
        assert_eq!(
            app.editor.files[0].text,
            "fn  foo() {}\nfn a() { foo(); }\n"
        );
        assert!(app.editor.files[0].dirty);
        assert!(app.editor.files[0].editing.as_mut().unwrap().undo());

        // A reply for a superseded revision changes nothing.
        app.editor.files[0].sync_shared_editing();
        app.poll_language_server(&ctx);
        app.request_language_formatting();
        let id = request_ids(&messages, "textDocument/formatting")[1];
        app.editor.files[0].revision += 1;
        reply(
            &events,
            id,
            serde_json::json!([{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"XX"}]),
        );
        app.poll_language_server(&ctx);
        assert!(!app.editor.files[0].text.starts_with("XX"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rename_is_previewed_validated_and_applied_across_open_and_closed_files() {
        let (mut app, messages, events, base) = rename_app();
        set_caret(&mut app, 4);
        let ctx = eframe::egui::Context::default();
        app.begin_language_rename();
        assert_eq!(app.view.rename_prompt.as_ref().unwrap().original, "foo");
        app.view.rename_prompt.as_mut().unwrap().name = "bar".into();
        app.submit_language_rename();
        let rename = messages
            .lock()
            .unwrap()
            .iter()
            .find(|message| message["method"] == "textDocument/rename")
            .cloned()
            .expect("rename request");
        assert_eq!(rename["params"]["newName"], "bar");
        reply(&events, rename["id"].as_u64().unwrap(), rename_edit(&base));
        app.poll_language_server(&ctx);
        let plan = app
            .view
            .language_edit_plan
            .clone()
            .expect("plan awaits review");
        assert_eq!((plan.files.len(), plan.edit_count()), (2, 3));
        // Nothing is touched before the user applies.
        assert_eq!(app.editor.files[0].text, "fn foo() {}\nfn a() { foo(); }\n");
        assert_eq!(
            std::fs::read_to_string(base.join("b.rs")).unwrap(),
            "fn b() { foo(); }\n"
        );

        app.apply_language_edit();
        assert_eq!(app.editor.files[0].text, "fn bar() {}\nfn a() { bar(); }\n");
        assert!(app.editor.files[0].dirty, "open buffers stay unsaved");
        assert_eq!(
            std::fs::read_to_string(base.join("b.rs")).unwrap(),
            "fn b() { bar(); }\n"
        );
        assert!(app.view.language_edit_plan.is_none());
        assert!(app.editor.files[0].editing.as_mut().unwrap().undo());
        assert_eq!(
            app.editor.files[0].editing.as_ref().unwrap().text(),
            "fn foo() {}\nfn a() { foo(); }\n"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rename_apply_refuses_when_files_changed_after_the_preview() {
        let (mut app, messages, events, base) = rename_app();
        set_caret(&mut app, 4);
        let ctx = eframe::egui::Context::default();
        app.begin_language_rename();
        app.view.rename_prompt.as_mut().unwrap().name = "bar".into();
        app.submit_language_rename();
        let id = request_ids(&messages, "textDocument/rename")[0];
        reply(&events, id, rename_edit(&base));
        app.poll_language_server(&ctx);
        std::fs::write(base.join("b.rs"), "fn b() { foo(); } // edited\n").unwrap();
        app.apply_language_edit();
        assert!(app.view.status.contains("changed since the preview"));
        assert_eq!(app.editor.files[0].text, "fn foo() {}\nfn a() { foo(); }\n");
        assert!(
            std::fs::read_to_string(base.join("b.rs"))
                .unwrap()
                .contains("foo")
        );

        // An open file edited after the preview blocks the whole rename too.
        app.view.language_edit_plan = None;
        std::fs::write(base.join("b.rs"), "fn b() { foo(); }\n").unwrap();
        set_caret(&mut app, 4);
        app.begin_language_rename();
        app.view.rename_prompt.as_mut().unwrap().name = "bar".into();
        app.submit_language_rename();
        let id = request_ids(&messages, "textDocument/rename")[1];
        reply(&events, id, rename_edit(&base));
        app.poll_language_server(&ctx);
        assert!(app.view.language_edit_plan.is_some());
        app.editor.files[0].revision += 1;
        app.apply_language_edit();
        assert!(
            std::fs::read_to_string(base.join("b.rs"))
                .unwrap()
                .contains("foo")
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rename_rejects_outside_workspace_and_file_operations() {
        let (mut app, _messages, _events, base) = rename_app();
        let outside = serde_json::json!({"changes":{"file:///etc/hosts":[
            {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":1}},"newText":"x"}]}});
        assert!(app.plan_workspace_edit("Rename", &outside).is_err());
        let moves =
            serde_json::json!({"documentChanges":[{"kind":"rename","oldUri":"a","newUri":"b"}]});
        let error = app.plan_workspace_edit("Rename", &moves).unwrap_err();
        assert!(error.contains("create, move or delete"));
        // A stale server version is refused.
        let versioned = serde_json::json!({"documentChanges":[{
            "textDocument":{"uri": file_uri(&base.join("a.rs")), "version": 99},
            "edits":[{"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":6}},"newText":"bar"}]}]});
        assert!(
            app.plan_workspace_edit("Rename", &versioned)
                .unwrap_err()
                .contains("newer")
        );
        // Prompt validation: unchanged or blank names never reach the server.
        set_caret(&mut app, 4);
        app.begin_language_rename();
        app.submit_language_rename();
        assert!(app.view.rename_prompt.is_some());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
