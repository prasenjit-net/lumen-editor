//! App-side language-server availability snapshots.
//!
//! Process discovery stays in `services`; this controller caches a small view
//! model for the active file so UI rendering never queries PATH or starts tools.
use super::{LumenApp, view_state::LanguageServerStatus};
use crate::services::language_server::{ServerAvailability, ServerEvent, ServerProcess};
use lumen_language_services::{
    DocumentUri, LanguageId, LanguageRegistry, ServerDocumentVersion,
    capabilities::ServerCapabilities, lsp,
};

impl LumenApp {
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
        self.language_server_document = None;
        match ServerProcess::start(&executable, &workspace) {
            Ok(mut process) => {
                let workspace_uri = file_uri(&workspace);
                match process.send(&lsp::initialize_request(1, &workspace_uri)) {
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
                ServerEvent::Message(message) if message["id"] == 1 => {
                    match ServerCapabilities::from_initialize_response(&message) {
                        Ok(capabilities) if capabilities.text_document_sync.is_some() => {
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
                ServerEvent::Message(_) => {}
            }
        }
        self.sync_active_language_document();
    }

    fn stop_language_server(&mut self, error: &str) {
        self.language_server_process = None;
        self.language_server_workspace = None;
        self.language_server_language = None;
        self.language_server_document = None;
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
        let message = match self.language_server_document {
            None => lsp::did_open(&uri, language, version, &document.text),
            Some((id, revision)) if id == document.id && revision != document.revision => {
                lsp::did_change(&uri, version, &document.text)
            }
            Some((id, _)) if id == document.id => return,
            Some(_) => lsp::did_open(&uri, language, version, &document.text),
        };
        if let Some(process) = &mut self.language_server_process
            && process.send(&message).is_ok()
        {
            self.language_server_document = Some((document.id, document.revision));
        }
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
}
