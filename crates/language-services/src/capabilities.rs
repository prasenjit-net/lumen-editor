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
        let text_document_sync = capabilities
            .get("textDocumentSync")
            .map(parse_text_document_sync)
            .transpose()?;

        Ok(Self {
            text_document_sync,
            hover: provider_enabled(capabilities.get("hoverProvider")),
            definition: provider_enabled(capabilities.get("definitionProvider")),
            document_symbols: provider_enabled(capabilities.get("documentSymbolProvider")),
            semantic_tokens: provider_enabled(capabilities.get("semanticTokensProvider")),
        })
    }
}

fn parse_text_document_sync(value: &Value) -> Result<TextDocumentSyncKind, CapabilityError> {
    let kind = value
        .as_u64()
        .or_else(|| value.get("change").and_then(Value::as_u64))
        .ok_or(CapabilityError::InvalidTextDocumentSync)?;
    match kind {
        0 => Ok(TextDocumentSyncKind::None),
        1 => Ok(TextDocumentSyncKind::Full),
        2 => Ok(TextDocumentSyncKind::Incremental),
        _ => Err(CapabilityError::InvalidTextDocumentSync),
    }
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
                "textDocumentSync": { "openClose": true, "change": 1 },
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
    }
}
