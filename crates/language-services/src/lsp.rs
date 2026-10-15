//! Pure LSP JSON-RPC message builders.
//!
//! The service adapter frames and writes these values; app code supplies stable
//! document identity/version data; UI code never constructs protocol JSON.
use crate::{DocumentUri, LanguageId, ServerDocumentVersion};
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
}
