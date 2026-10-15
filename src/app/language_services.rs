//! App-side language-server availability snapshots.
//!
//! Process discovery stays in `services`; this controller caches a small view
//! model for the active file so UI rendering never queries PATH or starts tools.
use super::{LumenApp, view_state::LanguageServerStatus};
use crate::services::language_server::ServerAvailability;
use lumen_language_services::LanguageRegistry;

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
                ServerAvailability::Available { executable } => executable
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| LanguageServerStatus::Available(name.to_owned())),
                ServerAvailability::Missing { candidates } => candidates
                    .first()
                    .map(|candidate| LanguageServerStatus::Missing(candidate.clone())),
                ServerAvailability::NotSupported => None,
            }
        });
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
}
