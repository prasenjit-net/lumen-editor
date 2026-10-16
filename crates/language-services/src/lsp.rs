//! Pure LSP JSON-RPC message builders.
//!
//! The service adapter frames and writes these values; app code supplies stable
//! document identity/version data; UI code never constructs protocol JSON.
use crate::{DocumentUri, LanguageId, Position, ServerDocumentVersion};
use serde_json::{Value, json};

#[must_use]
pub fn initialize_request(request_id: u64, workspace_uri: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "initialize",
        "params": {
            "processId": null,
            "clientInfo": { "name": "Lumen" },
            "rootUri": workspace_uri,
            "workspaceFolders": [{ "uri": workspace_uri, "name": "Lumen Workspace" }],
            "capabilities": {
                "workspace": { "workspaceFolders": true },
                "textDocument": {
                    "publishDiagnostics": { "relatedInformation": true }
                }
            }
        }
    })
}

#[must_use]
pub fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    })
}

#[must_use]
pub fn did_open(
    uri: &DocumentUri,
    language: LanguageId,
    version: ServerDocumentVersion,
    text: &str,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri.as_str(),
                "languageId": language.as_str(),
                "version": version.0,
                "text": text,
            }
        }
    })
}

/// Full-text synchronization avoids unsafe UTF-8-to-UTF-16 incremental edits
/// until incremental changes have their own exhaustive conversion coverage.
#[must_use]
pub fn did_change(uri: &DocumentUri, version: ServerDocumentVersion, text: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri.as_str(), "version": version.0 },
            "contentChanges": [{ "text": text }]
        }
    })
}

#[must_use]
pub fn did_close(uri: &DocumentUri) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": uri.as_str() } }
    })
}

#[must_use]
pub fn did_save(uri: &DocumentUri, text: Option<&str>) -> Value {
    let mut params = json!({ "textDocument": { "uri": uri.as_str() } });
    if let Some(text) = text {
        params["text"] = Value::String(text.to_owned());
    }
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didSave",
        "params": params
    })
}

#[must_use]
pub fn cancel_request(request_id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "$/cancelRequest",
        "params": { "id": request_id }
    })
}

#[must_use]
pub fn shutdown_request(request_id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": request_id, "method": "shutdown", "params": null })
}

#[must_use]
pub fn exit_notification() -> Value {
    json!({ "jsonrpc": "2.0", "method": "exit", "params": null })
}

#[must_use]
pub fn hover_request(request_id: u64, uri: &DocumentUri, position: Position) -> Value {
    text_position_request(request_id, "textDocument/hover", uri, position)
}

#[must_use]
pub fn definition_request(request_id: u64, uri: &DocumentUri, position: Position) -> Value {
    text_position_request(request_id, "textDocument/definition", uri, position)
}

#[must_use]
pub fn document_symbols_request(request_id: u64, uri: &DocumentUri) -> Value {
    json!({"jsonrpc":"2.0", "id": request_id, "method":"textDocument/documentSymbol", "params":{"textDocument":{"uri":uri.as_str()}}})
}

fn text_position_request(
    request_id: u64,
    method: &str,
    uri: &DocumentUri,
    position: Position,
) -> Value {
    json!({"jsonrpc":"2.0", "id":request_id, "method":method, "params":{"textDocument":{"uri":uri.as_str()},"position":{"line":position.line,"character":position.character}}})
}

/// Encode a JSON-RPC message for the service-owned stdio transport.
///
/// This stays in the language crate so outbound framing is testable alongside
/// the bounded inbound decoder, while process and writer ownership remain in
/// `services/`.
pub fn encode_message(message: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let payload = serde_json::to_vec(message)?;
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    let mut framed = Vec::with_capacity(header.len() + payload.len());
    framed.extend_from_slice(header.as_bytes());
    framed.extend_from_slice(&payload);
    Ok(framed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ContentLengthDecoder;

    #[test]
    fn initialization_announces_only_currently_supported_capabilities() {
        let message = initialize_request(9, "file:///workspace");
        assert_eq!(message["id"], 9);
        assert_eq!(message["method"], "initialize");
        assert_eq!(message["params"]["rootUri"], "file:///workspace");
        assert!(message["params"]["capabilities"]["textDocument"]["completion"].is_null());
        assert_eq!(initialized_notification()["method"], "initialized");
    }

    #[test]
    fn document_sync_preserves_text_and_uses_full_changes() {
        let uri = DocumentUri::new("file:///workspace/src/main.rs").unwrap();
        let open = did_open(
            &uri,
            LanguageId::RUST,
            ServerDocumentVersion(1),
            "let 🌍 = 1;\r\n",
        );
        assert_eq!(open["params"]["textDocument"]["text"], "let 🌍 = 1;\r\n");
        let change = did_change(&uri, ServerDocumentVersion(2), "let x = 2;");
        assert_eq!(change["params"]["textDocument"]["version"], 2);
        assert!(change["params"]["contentChanges"][0]["range"].is_null());
        assert_eq!(did_close(&uri)["method"], "textDocument/didClose");
        assert_eq!(did_save(&uri, None)["method"], "textDocument/didSave");
        assert!(did_save(&uri, Some("saved"))["params"]["text"] == "saved");
    }

    #[test]
    fn encoded_messages_round_trip_through_the_bounded_decoder() {
        let uri = DocumentUri::new("file:///workspace/src/main.rs").unwrap();
        let message = did_change(&uri, ServerDocumentVersion(3), "a\r\n😀");
        let framed = encode_message(&message).unwrap();
        assert!(framed.starts_with(b"Content-Length: "));

        let mut decoder = ContentLengthDecoder::new(1024);
        assert_eq!(
            decoder.push(&framed).unwrap(),
            vec![serde_json::to_vec(&message).unwrap()]
        );
    }

    #[test]
    fn navigation_requests_use_lsp_utf16_positions() {
        let uri = DocumentUri::new("file:///workspace/src/main.rs").unwrap();
        let position = crate::Position {
            line: 2,
            character: 5,
        };
        assert_eq!(
            hover_request(2, &uri, position)["method"],
            "textDocument/hover"
        );
        assert_eq!(
            definition_request(3, &uri, position)["params"]["position"]["character"],
            5
        );
        assert_eq!(
            document_symbols_request(4, &uri)["method"],
            "textDocument/documentSymbol"
        );
    }
}
