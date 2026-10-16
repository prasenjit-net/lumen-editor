//! Typed capabilities negotiated from an LSP `initialize` response.
//!
//! Lumen only enables an interaction after both the server advertises it and
//! the corresponding client feature has been implemented. Unknown capability
//! fields are intentionally ignored for forward compatibility.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDocumentSyncKind {
    None,
    Full,
    Incremental,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerCapabilities {
    pub text_document_sync: Option<TextDocumentSyncKind>,
    pub open_close: bool,
    /// `Some(include_text)` when `textDocument/didSave` is supported.
    pub save: Option<bool>,
    pub hover: bool,
    pub definition: bool,
    pub document_symbols: bool,
    pub semantic_tokens: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityError {
    MissingResult,
    MissingCapabilities,
    InvalidTextDocumentSync,
}

impl ServerCapabilities {
    /// Extract the capability subset Lumen understands from a successful
    /// JSON-RPC initialize response.
    pub fn from_initialize_response(response: &Value) -> Result<Self, CapabilityError> {
        let result = response
            .get("result")
            .and_then(Value::as_object)
            .ok_or(CapabilityError::MissingResult)?;
        let capabilities = result
            .get("capabilities")
            .and_then(Value::as_object)
            .ok_or(CapabilityError::MissingCapabilities)?;
        let sync = capabilities
            .get("textDocumentSync")
            .map(parse_text_document_sync)
            .transpose()?;

        Ok(Self {
            text_document_sync: sync.and_then(|sync| sync.kind),
            open_close: sync.is_some_and(|sync| sync.open_close),
            save: sync.and_then(|sync| sync.save),
            hover: provider_enabled(capabilities.get("hoverProvider")),
            definition: provider_enabled(capabilities.get("definitionProvider")),
            document_symbols: provider_enabled(capabilities.get("documentSymbolProvider")),
            semantic_tokens: provider_enabled(capabilities.get("semanticTokensProvider")),
        })
    }
}

#[derive(Clone, Copy)]
struct TextDocumentSync {
    kind: Option<TextDocumentSyncKind>,
    open_close: bool,
    save: Option<bool>,
}

fn parse_text_document_sync(value: &Value) -> Result<TextDocumentSync, CapabilityError> {
    let kind = match value
        .as_u64()
        .or_else(|| value.get("change").and_then(Value::as_u64))
    {
        Some(0) => Some(TextDocumentSyncKind::None),
        Some(1) => Some(TextDocumentSyncKind::Full),
        Some(2) => Some(TextDocumentSyncKind::Incremental),
        Some(_) => return Err(CapabilityError::InvalidTextDocumentSync),
        None if value.is_object() => None,
        None => return Err(CapabilityError::InvalidTextDocumentSync),
    };
    let open_close = value
        .get("openClose")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let save = match value.get("save") {
        Some(Value::Bool(true)) => Some(false),
        Some(Value::Object(options)) => Some(
            options
                .get("includeText")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        _ => None,
    };
    Ok(TextDocumentSync {
        kind,
        open_close,
        save,
    })
}

fn provider_enabled(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(true)) | Some(Value::Object(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_supported_server_features_without_enabling_unknown_ones() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": true } },
                "hoverProvider": true,
                "definitionProvider": {},
                "documentSymbolProvider": false,
                "semanticTokensProvider": { "legend": {} },
                "completionProvider": { "triggerCharacters": ["."] }
            }}
        });
        assert_eq!(
            ServerCapabilities::from_initialize_response(&response).unwrap(),
            ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncKind::Full),
                open_close: true,
                save: Some(true),
                hover: true,
                definition: true,
                document_symbols: false,
                semantic_tokens: true,
            }
        );
    }

    #[test]
    fn invalid_or_missing_sync_never_becomes_a_safe_default() {
        let missing = json!({ "result": { "capabilities": {} } });
        assert_eq!(
            ServerCapabilities::from_initialize_response(&missing)
                .unwrap()
                .text_document_sync,
            None
        );
        let invalid = json!({ "result": { "capabilities": { "textDocumentSync": 9 } } });
        assert_eq!(
            ServerCapabilities::from_initialize_response(&invalid),
            Err(CapabilityError::InvalidTextDocumentSync)
        );

        let open_only = json!({ "result": { "capabilities": {
            "textDocumentSync": { "openClose": true, "save": false }
        } } });
        let parsed = ServerCapabilities::from_initialize_response(&open_only).unwrap();
        assert_eq!(parsed.text_document_sync, None);
        assert!(parsed.open_close);
        assert_eq!(parsed.save, None);
    }
}
