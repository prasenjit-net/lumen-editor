//! App-side language-server availability snapshots.
//!
//! Process discovery stays in `services`; this controller caches a small view
//! model for the active file so UI rendering never queries PATH or starts tools.
use super::{LumenApp, view_state::LanguageServerStatus};
use crate::services::language_server::{ServerAvailability, ServerEvent, ServerProcess};
use lumen_language_services::{
    DocumentKey, DocumentUri, DocumentVersion, LanguageId, LanguageRegistry, RequestStamp,
    ServerDocumentVersion, capabilities::ServerCapabilities, lsp,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LanguageRequestKind {
    Hover,
    Definition,
    Symbols,
}

impl LumenApp {
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
        let Some((document_id, revision, uri)) = self.language_server_document.clone() else {
            self.view.status = "Language server is not ready for this document.".into();
            return;
        };
        let stamp = self.request_stamp(document_id, revision, uri.clone());
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

    pub(crate) fn refresh_language_server_status(&mut self) {
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
        if self.view.language_server_document == Some(document) {
            return;
        }
        self.view.language_server_document = Some(document);
        self.view.language_server_status = LanguageRegistry::detect(&path).and_then(|language| {
            match self.language_server_service.availability(language) {
                ServerAvailability::Available { executable } => {
                    let name = executable.file_name()?.to_str()?.to_owned();
                    self.start_language_server(executable, language.id, name.clone());
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
            }
        });
    }

    fn start_language_server(
        &mut self,
        executable: std::path::PathBuf,
        language: LanguageId,
        name: String,
    ) {
        let Some(workspace) = self.editor.workspace.clone() else {
            return;
        };
        if self.language_server_workspace.as_ref() == Some(&workspace)
            && self.language_server_process.is_some()
            && self.language_server_language == Some(language)
        {
            return;
        }
        self.language_server_process = None;
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_generation = self.language_server_generation.next();
        self.language_server_capabilities = None;
        self.language_server_requests.clear();
        self.language_server_request_kinds.clear();
        self.view.language_hover = None;
        self.language_server_document = None;
        match ServerProcess::start(&executable, &workspace) {
            Ok(mut process) => {
                let workspace_uri = file_uri(&workspace);
                match process.send(&lsp::initialize_request(0, &workspace_uri)) {
                    Ok(()) => {
                        self.language_server_workspace = Some(workspace);
                        self.language_server_language = Some(language);
                        self.language_server_process = Some(process);
                        self.view.language_server_status =
                            Some(LanguageServerStatus::Starting(name));
                    }
                    Err(error) => {
                        self.view.language_server_status = Some(LanguageServerStatus::Error(error))
                    }
                }
            }
            Err(error) => {
                self.view.language_server_status = Some(LanguageServerStatus::Error(error))
            }
        }
    }

    pub(crate) fn poll_language_server(&mut self) {
        let mut events = self
            .language_server_process
            .as_ref()
            .map(ServerProcess::drain_events)
            .unwrap_or_default();
        if self
            .language_server_process
            .as_mut()
            .is_some_and(|process| process.has_exited().unwrap_or(true))
        {
            events.push(ServerEvent::Exited);
        }
        for event in events {
            match event {
                ServerEvent::Message(message) if message["id"] == 0 => {
                    match ServerCapabilities::from_initialize_response(&message) {
                        Ok(capabilities) if capabilities.text_document_sync.is_some() => {
                            self.language_server_capabilities = Some(capabilities);
                            if let Some(process) = &mut self.language_server_process
                                && let Err(error) = process.send(&lsp::initialized_notification())
                            {
                                self.view.language_server_status =
                                    Some(LanguageServerStatus::Error(error));
                                self.language_server_process = None;
                                return;
                            }
                            if let Some(LanguageServerStatus::Starting(name)) =
                                self.view.language_server_status.take()
                            {
                                self.view.language_server_status =
                                    Some(LanguageServerStatus::Ready(name));
                            }
                            self.sync_active_language_document();
                        }
                        Ok(_) => self.stop_language_server(
                            "Language server does not support text synchronization",
                        ),
                        Err(error) => self.stop_language_server(&format!(
                            "Invalid language-server initialization: {error:?}"
                        )),
                    }
                }
                ServerEvent::ProtocolError(error) => self.stop_language_server(&error),
                ServerEvent::Exited => self.stop_language_server("Language server exited"),
                ServerEvent::Message(message) => {
                    if !self.accept_language_response(&message) {
                        self.accept_diagnostics(message);
                    }
                }
            }
        }
        self.sync_active_language_document();
    }

    fn stop_language_server(&mut self, error: &str) {
        self.language_server_process = None;
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_capabilities = None;
        self.language_server_requests.clear();
        self.language_server_request_kinds.clear();
        self.view.language_hover = None;
        self.language_server_document = None;
        self.view.language_server_diagnostics.clear();
        self.view.language_server_status = Some(LanguageServerStatus::Error(error.to_owned()));
    }

    fn sync_active_language_document(&mut self) {
        let Some(language) = self.language_server_language else {
            return;
        };
        let Some(document) = self.editor.active() else {
            return;
        };
        let Some(path) = document.path.as_ref() else {
            return;
        };
        if LanguageRegistry::detect(path).map(|descriptor| descriptor.id) != Some(language) {
            return;
        }
        let version = ServerDocumentVersion(document.revision.saturating_add(1));
        let Ok(uri) = DocumentUri::new(file_uri(path)) else {
            return;
        };
        let message = match &self.language_server_document {
            None => lsp::did_open(&uri, language, version, &document.text),
            Some((id, revision, _)) if *id == document.id && *revision != document.revision => {
                lsp::did_change(&uri, version, &document.text)
            }
            Some((id, _, _)) if *id == document.id => return,
            Some((_, _, previous_uri)) => {
                if let Some(process) = &mut self.language_server_process
                    && process.send(&lsp::did_close(previous_uri)).is_err()
                {
                    self.stop_language_server("Could not close language-server document");
                    return;
                }
                lsp::did_open(&uri, language, version, &document.text)
            }
        };
        if let Some(process) = &mut self.language_server_process
            && process.send(&message).is_ok()
        {
            self.view.language_hover = None;
            self.view.language_server_diagnostics.remove(&document.id);
            self.language_server_document = Some((document.id, document.revision, uri));
        }
    }

    fn accept_diagnostics(&mut self, message: serde_json::Value) {
        let Some(document) = self.editor.active() else {
            return;
        };
        let Some(path) = document.path.as_ref() else {
            return;
        };
        let uri = file_uri(path);
        let Ok(published) =
            lumen_language_services::incoming::parse_publish_diagnostics(&message, &document.text)
        else {
            return;
        };
        if published.uri != uri {
            return;
        }
        if let Some(version) = published.version
            && version != document.revision.saturating_add(1)
        {
            return;
        }
        self.view
            .language_server_diagnostics
            .insert(document.id, published.diagnostics);
    }

    fn request_at_caret(&mut self, kind: LanguageRequestKind) {
        let capability = self
            .language_server_capabilities
            .as_ref()
            .is_some_and(|capabilities| match kind {
                LanguageRequestKind::Hover => capabilities.hover,
                LanguageRequestKind::Definition => capabilities.definition,
                LanguageRequestKind::Symbols => false,
            });
        if !capability {
            self.view.status = "The active language server does not provide this feature.".into();
            return;
        }
        let Some((document_id, revision, uri)) = self.language_server_document.clone() else {
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
        let stamp = self.request_stamp(document_id, revision, uri.clone());
        let request_id = self.language_server_requests.register(stamp);
        let message = match kind {
            LanguageRequestKind::Hover => lsp::hover_request(request_id, &uri, position),
            LanguageRequestKind::Definition => lsp::definition_request(request_id, &uri, position),
            LanguageRequestKind::Symbols => unreachable!("symbols do not use a caret position"),
        };
        self.send_language_request(request_id, kind, &message);
    }

    fn request_stamp(
        &self,
        document_id: crate::model::DocumentId,
        revision: u64,
        uri: DocumentUri,
    ) -> RequestStamp {
        RequestStamp {
            document: DocumentKey {
                id: document_id.language_service_id(),
                uri,
            },
            document_version: DocumentVersion(revision),
            server_version: ServerDocumentVersion(revision.saturating_add(1)),
            session: self.language_server_generation,
        }
    }

    fn send_language_request(
        &mut self,
        request_id: u64,
        kind: LanguageRequestKind,
        message: &serde_json::Value,
    ) {
        let result = self
            .language_server_process
            .as_mut()
            .ok_or_else(|| "Language server stopped".to_owned())
            .and_then(|process| process.send(message));
        match result {
            Ok(()) => {
                self.language_server_request_kinds.insert(request_id, kind);
                self.view.status = match kind {
                    LanguageRequestKind::Hover => "Loading hover…",
                    LanguageRequestKind::Definition => "Finding definition…",
                    LanguageRequestKind::Symbols => "Loading document symbols…",
                }
                .into();
            }
            Err(error) => self.stop_language_server(&error),
        }
    }

    fn accept_language_response(&mut self, message: &serde_json::Value) -> bool {
        let Some(request_id) = message.get("id").and_then(serde_json::Value::as_u64) else {
            return false;
        };
        let Some(kind) = self.language_server_request_kinds.remove(&request_id) else {
            return false;
        };
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
        }
        true
    }

    fn current_request_stamp(&self) -> Option<RequestStamp> {
        let (id, revision, uri) = self.language_server_document.as_ref()?;
        Some(RequestStamp {
            document: DocumentKey {
                id: id.language_service_id(),
                uri: uri.clone(),
            },
            document_version: DocumentVersion(*revision),
            server_version: ServerDocumentVersion(revision.saturating_add(1)),
            session: self.language_server_generation,
        })
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

    #[test]
    fn active_server_language_gets_a_cached_snapshot_and_parser_only_language_does_not() {
        let mut app = LumenApp::default();
        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/src/main.rs".into(),
            String::new(),
        ));
        app.editor.active_file = Some(0);
        let rust_id = app.editor.files[0].id;
        app.refresh_language_server_status();
        assert_eq!(app.view.language_server_document, Some(rust_id));
        assert!(app.view.language_server_status.is_some());

        app.editor.files.push(crate::model::Document::from_path(
            "/workspace/settings.json".into(),
            String::new(),
        ));
        app.editor.active_file = Some(1);
        app.refresh_language_server_status();
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
}
