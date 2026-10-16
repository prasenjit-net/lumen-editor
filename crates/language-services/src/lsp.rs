//! Pure LSP JSON-RPC message builders.
//!
//! The service adapter frames and writes these values; app code supplies stable
//! document identity/version data; UI code never constructs protocol JSON.
use crate::{DocumentUri, LanguageId, Position, ServerDocumentVersion};
use serde_json::{Value, json};

/// Token types Lumen can map to a class; advertised so servers use standard names.
const SEMANTIC_TOKEN_TYPES: [&str; 14] = [
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "function",
    "method",
    "macro",
];

#[must_use]
pub fn initialize_request(request_id: u64, workspace_uri: &str) -> Value {
    initialize_request_with_options(request_id, workspace_uri, None)
}

/// `options` is the descriptor's JSON `initializationOptions`; invalid JSON is omitted.
#[must_use]
pub fn initialize_request_with_options(
    request_id: u64,
    workspace_uri: &str,
    options: Option<&str>,
) -> Value {
    let mut request = json!({
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
                    "publishDiagnostics": { "relatedInformation": true },
                    "semanticTokens": {
                        "requests": { "full": true },
                        "tokenTypes": SEMANTIC_TOKEN_TYPES,
                        "tokenModifiers": [],
                        "formats": ["relative"],
                        "overlappingTokenSupport": false,
                        "multilineTokenSupport": false
                    }
                }
            }
        }
    });
    if let Some(options) = options.and_then(|text| serde_json::from_str::<Value>(text).ok()) {
        request["params"]["initializationOptions"] = options;
    }
    request
}

/// Reply to a server-initiated request. JSON-RPC requires every request to be
/// answered; servers such as rust-analyzer wait on some of them. Lumen supports
/// none of these features yet, so refreshes and capability registration are
/// acknowledged without effect, configuration queries answer "no value" per
/// item, and everything else is rejected as an unknown method.
#[must_use]
pub fn server_request_response(request: &Value) -> Option<Value> {
    let method = request.get("method")?.as_str()?;
    let id = request.get("id")?;
    let result = match method {
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create"
        | "workspace/diagnostic/refresh"
        | "workspace/semanticTokens/refresh"
        | "workspace/inlayHint/refresh"
        | "workspace/codeLens/refresh" => Value::Null,
        "workspace/configuration" => {
            let items = request
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        }
        _ => {
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "Method not supported by Lumen" }
            }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
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
pub fn semantic_tokens_request(request_id: u64, uri: &DocumentUri) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": uri.as_str() } }
    })
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

    #[test]
    fn server_requests_are_always_answered() {
        let reply = |value: Value| server_request_response(&value).unwrap();
        assert_eq!(
            reply(json!({"id":3,"method":"workspace/diagnostic/refresh"})),
            json!({"jsonrpc":"2.0","id":3,"result":null})
        );
        assert_eq!(
            reply(json!({"id":"a","method":"workspace/configuration","params":{"items":[{},{}]}}))
                ["result"],
            json!([null, null])
        );
        let unknown = reply(json!({"id":4,"method":"workspace/applyEdit"}));
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(unknown["id"], 4);
        assert!(server_request_response(&json!({"method":"window/logMessage"})).is_none());
    }
}
