//! Formatting and rename: language-server edits routed through reviewable plans.
//!
//! Formatting changes only the active document, as one undoable step, after
//! revision validation. Rename can touch many files, so it is always previewed
//! and re-validated before anything is written.
use super::{
    LumenApp,
    language_services::{CaretContext, LanguageRequestKind, file_uri_to_path},
    view_state::RenamePrompt,
    workspace_edit::{EditPlan, PlannedFile, apply_to_buffer, apply_to_text},
};
use lumen_language_services::{
    incoming::{self, WorkspaceEditError},
    lsp,
};

/// A planned file's path, current text, and open-document identity (if any).
type PlanSource = (
    std::path::PathBuf,
    String,
    Option<(crate::model::DocumentId, u64)>,
);

impl LumenApp {
    pub(crate) fn request_language_formatting(&mut self) {
        if !self
            .language_server_capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.formatting)
        {
            self.view.status = "The active language server does not provide formatting.".into();
            return;
        }
        let Some((document_id, revision, uri, server_version)) = self.active_synced_document()
        else {
            self.view.status = "Language server is not ready for this document.".into();
            return;
        };
        let path = self
            .editor
            .active()
            .and_then(|document| document.path.clone());
        let tab_size = self
            .view
            .settings
            .number("editor.indent_size", path.as_deref()) as u32;
        let insert_spaces = self
            .view
            .settings
            .document_boolean("editor.insert_spaces", path.as_deref());
        let stamp = self.request_stamp(document_id, revision, server_version, uri.clone());
        let request_id = self.language_server_requests.register(stamp);
        let message = lsp::formatting_request(request_id, &uri, tab_size, insert_spaces);
        self.send_language_request_at(request_id, LanguageRequestKind::Formatting, None, &message);
    }

    pub(super) fn apply_formatting_response(&mut self, message: &serde_json::Value) {
        let Some(index) = self.editor.active_file else {
            return;
        };
        let document = &mut self.editor.files[index];
        let Ok(edits) = incoming::parse_text_edits_response(message, &document.text) else {
            self.view.status = "Language server returned invalid formatting edits".into();
            return;
        };
        let Some(buffer) = document.editing.as_mut() else {
            return;
        };
        match apply_to_buffer(buffer, &edits) {
            Ok(true) => {
                document.sync_shared_editing();
                self.view.status = format!("Formatted document ({} edits)", edits.len());
            }
            Ok(false) => self.view.status = "Document is already formatted".into(),
            Err(reason) => self.view.status = format!("Formatting not applied: {reason}"),
        }
    }

    /// Open the rename prompt for the identifier at the caret.
    pub(crate) fn begin_language_rename(&mut self) {
        let Some(context) = self.caret_context(|capabilities| capabilities.rename) else {
            return;
        };
        let Some(document) = self.editor.active() else {
            return;
        };
        let start = super::completion::word_start(&document.text, context.byte);
        let end = document.text[context.byte..]
            .char_indices()
            .find(|(_, character)| !super::completion::is_word_char(*character))
            .map_or(document.text.len(), |(offset, _)| context.byte + offset);
        let word = document.text[start..end].to_owned();
        if word.is_empty() {
            self.view.status = "Place the caret on a symbol to rename it".into();
            return;
        }
        self.view.rename_prompt = Some(RenamePrompt {
            document: context.document_id,
            name: word.clone(),
            original: word,
            focus: true,
        });
    }

    pub(crate) fn cancel_language_rename(&mut self) {
        self.view.rename_prompt = None;
    }

    pub(crate) fn submit_language_rename(&mut self) {
        let Some(prompt) = self.view.rename_prompt.clone() else {
            return;
        };
        let name = prompt.name.trim().to_owned();
        if name.is_empty()
            || name == prompt.original
            || name
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            self.view.status = "Enter a different name without spaces".into();
            return;
        }
        let Some(context) = self.caret_context(|capabilities| capabilities.rename) else {
            self.view.rename_prompt = None;
            return;
        };
        if context.document_id != prompt.document {
            self.view.rename_prompt = None;
            self.view.status = "Rename cancelled: the active editor changed".into();
            return;
        }
        self.view.rename_prompt = None;
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
        let message = lsp::rename_request(request_id, &uri, position, &name);
        self.send_language_request_at(
            request_id,
            LanguageRequestKind::Rename,
            Some(byte),
            &message,
        );
    }

    pub(super) fn apply_rename_response(
        &mut self,
        message: &serde_json::Value,
        caret: Option<usize>,
    ) {
        if self.caret_is_still_at(caret).is_none() {
            self.view.status = "Rename discarded: the caret moved".into();
            return;
        }
        if let Some(error) = message.get("error") {
            self.view.status = format!(
                "Rename failed: {}",
                error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("the server refused")
            );
            return;
        }
        let Some(result) = message.get("result").filter(|result| !result.is_null()) else {
            self.view.status = "Nothing to rename".into();
            return;
        };
        match self.plan_workspace_edit("Rename Symbol", result) {
            Ok(plan) if plan.files.is_empty() => self.view.status = "Rename changes nothing".into(),
            Ok(plan) => {
                self.view.status = format!(
                    "Review rename: {} edits in {} files",
                    plan.edit_count(),
                    plan.files.len()
                );
                self.view.language_edit_plan = Some(plan);
            }
            Err(reason) => self.view.status = format!("Rename not applied: {reason}"),
        }
    }

    /// Turn a server `WorkspaceEdit` into a plan against the text the server saw:
    /// synced open buffers, otherwise the file on disk. Anything outside the
    /// workspace, out of sync, or beyond plain text edits rejects the whole edit.
    pub(crate) fn plan_workspace_edit(
        &self,
        title: &str,
        edit: &serde_json::Value,
    ) -> Result<EditPlan, String> {
        let root = self
            .editor
            .workspace
            .clone()
            .ok_or("open a folder to edit across files")?;
        let source = |uri: &str| -> Option<PlanSource> {
            let path = file_uri_to_path(uri).filter(|path| path.starts_with(&root))?;
            match self
                .editor
                .files
                .iter()
                .find(|document| document.path.as_deref() == Some(path.as_path()))
            {
                Some(document) => {
                    let synced = self.language_server_documents.get(&document.id)?;
                    (synced.revision() == document.revision).then(|| {
                        (
                            path,
                            document.text.clone(),
                            Some((document.id, document.revision)),
                        )
                    })
                }
                None => self
                    .file_system
                    .read_text(&path)
                    .ok()
                    .map(|text| (path, text, None)),
            }
        };
        let files = incoming::parse_workspace_edit(edit, |uri| source(uri).map(|found| found.1))
            .map_err(|error| match error {
                WorkspaceEditError::ResourceOperation => {
                    "the server wants to create, move or delete files".to_owned()
                }
                WorkspaceEditError::UnknownFile(uri) => {
                    format!("{uri} is outside the workspace, unreadable, or out of sync")
                }
                WorkspaceEditError::Overlap(uri) => format!("overlapping edits in {uri}"),
                WorkspaceEditError::Invalid => "the server sent an invalid edit".to_owned(),
            })?;
        let mut planned = Vec::new();
        for file in files {
            let (path, base, document) =
                source(&file.uri).ok_or_else(|| format!("{} changed while planning", file.uri))?;
            if let (Some(version), Some((id, _))) = (file.version, document)
                && self
                    .language_server_documents
                    .get(&id)
                    .is_some_and(|synced| synced.server_version().0 != version)
            {
                return Err(format!(
                    "{} is newer than the server's copy",
                    path.display()
                ));
            }
            planned.push(PlannedFile {
                path,
                document,
                base,
                edits: file.edits,
            });
        }
        planned.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(EditPlan {
            title: title.to_owned(),
            files: planned,
        })
    }

    pub(crate) fn cancel_language_edit(&mut self) {
        self.view.language_edit_plan = None;
    }

    /// Re-validate every file, then apply. Nothing is touched if any check fails.
    pub(crate) fn apply_language_edit(&mut self) {
        let Some(plan) = self.view.language_edit_plan.take() else {
            return;
        };
        let Some(root) = self.editor.workspace.clone() else {
            self.view.status = "Edit cancelled: no folder is open".into();
            return;
        };
        for file in &plan.files {
            let current = match file.document {
                Some((id, revision)) => self
                    .editor
                    .files
                    .iter()
                    .any(|document| document.id == id && document.revision == revision),
                None => {
                    !self
                        .editor
                        .files
                        .iter()
                        .any(|document| document.path.as_deref() == Some(file.path.as_path()))
                        && self.file_system.read_text(&file.path).ok().as_deref()
                            == Some(file.base.as_str())
                }
            };
            if !current {
                self.view.status = format!(
                    "Edit cancelled: {} changed since the preview",
                    file.path.display()
                );
                return;
            }
        }
        let (mut applied, mut failures) = (0, Vec::new());
        for file in &plan.files {
            let result = match file.document {
                Some((id, _)) => self
                    .editor
                    .files
                    .iter_mut()
                    .find(|document| document.id == id)
                    .and_then(|document| {
                        let result = document
                            .editing
                            .as_mut()
                            .map(|buffer| apply_to_buffer(buffer, &file.edits));
                        document.sync_shared_editing();
                        result
                    })
                    .map_or(Err("document closed".to_owned()), |result| {
                        result.map(|_| ()).map_err(str::to_owned)
                    }),
                None => apply_to_text(&file.base, &file.edits)
                    .ok_or_else(|| "edit no longer fits the file".to_owned())
                    .and_then(|text| {
                        self.file_system
                            .write_workspace_replace(&root, &file.path, &file.base, &text)
                            .map_err(|error| error.to_string())
                    }),
            };
            match result {
                Ok(()) => applied += 1,
                Err(error) => failures.push(format!("{}: {error}", file.path.display())),
            }
        }
        self.view.status = if failures.is_empty() {
            format!(
                "{}: updated {applied} file(s); open files are unsaved",
                plan.title
            )
        } else {
            format!(
                "{}: {applied} updated, {} failed ({})",
                plan.title,
                failures.len(),
                failures.join("; ")
            )
        };
    }
}
