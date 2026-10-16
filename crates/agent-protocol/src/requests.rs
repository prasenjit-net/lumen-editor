//! Builders for everything the client sends.
use crate::rpc::{self, RequestId};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: u32 = 1;

#[must_use]
pub fn initialize(id: i64, client_version: &str, fs: bool, terminal: bool) -> Value {
    rpc::request(
        id,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "clientCapabilities": {
                "fs": {"readTextFile": fs, "writeTextFile": fs},
                "terminal": terminal
            },
            "clientInfo": {"name": "lumen", "title": "Lumen", "version": client_version}
        }),
    )
}

#[must_use]
pub fn authenticate(id: i64, method_id: &str) -> Value {
    rpc::request(id, "authenticate", json!({"methodId": method_id}))
}

/// `cwd` must be absolute; `mcp_servers` is passed through unchanged.
#[must_use]
pub fn session_new(id: i64, cwd: &str, mcp_servers: Vec<Value>) -> Value {
    rpc::request(
        id,
        "session/new",
        json!({"cwd": cwd, "mcpServers": mcp_servers}),
    )
}

#[must_use]
pub fn session_load(id: i64, session_id: &str, cwd: &str) -> Value {
    rpc::request(
        id,
        "session/load",
        json!({"sessionId": session_id, "cwd": cwd, "mcpServers": []}),
    )
}

#[must_use]
pub fn session_list(id: i64, cwd: &str) -> Value {
    rpc::request(id, "session/list", json!({"cwd": cwd}))
}

#[must_use]
pub fn session_prompt(id: i64, session_id: &str, blocks: Vec<Value>) -> Value {
    rpc::request(
        id,
        "session/prompt",
        json!({"sessionId": session_id, "prompt": blocks}),
    )
}

#[must_use]
pub fn session_cancel(session_id: &str) -> Value {
    rpc::notification("session/cancel", json!({"sessionId": session_id}))
}

#[must_use]
pub fn set_config_option(id: i64, session_id: &str, config_id: &str, value: &str) -> Value {
    rpc::request(
        id,
        "session/set_config_option",
        json!({"sessionId": session_id, "configId": config_id, "value": value}),
    )
}

#[must_use]
pub fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// File or selection text sent as context (needs `embeddedContext`).
#[must_use]
pub fn resource_block(uri: &str, text: &str, mime_type: Option<&str>) -> Value {
    let mut resource = json!({"uri": uri, "text": text});
    if let Some(mime) = mime_type {
        resource["mimeType"] = json!(mime);
    }
    json!({"type": "resource", "resource": resource})
}

#[must_use]
pub fn resource_link_block(uri: &str, name: &str) -> Value {
    json!({"type": "resource_link", "uri": uri, "name": name})
}

/// Answer `session/request_permission` with the user's choice.
#[must_use]
pub fn permission_selected(id: &RequestId, option_id: &str) -> Value {
    rpc::response_ok(
        id,
        json!({"outcome": {"outcome": "selected", "optionId": option_id}}),
    )
}

/// Required answer to every pending permission request when a turn is cancelled.
#[must_use]
pub fn permission_cancelled(id: &RequestId) -> Value {
    rpc::response_ok(id, json!({"outcome": {"outcome": "cancelled"}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_requests_have_the_documented_shape() {
        let init = initialize(1, "0.1.3", true, false);
        assert_eq!(init["params"]["protocolVersion"], 1);
        assert_eq!(
            init["params"]["clientCapabilities"]["fs"]["writeTextFile"],
            true
        );
        assert_eq!(init["params"]["clientCapabilities"]["terminal"], false);
        assert_eq!(init["params"]["clientInfo"]["name"], "lumen");
        let new = session_new(2, "/w", vec![]);
        assert_eq!(
            (new["method"].clone(), new["params"]["cwd"].clone()),
            (json!("session/new"), json!("/w"))
        );
        assert_eq!(new["params"]["mcpServers"], json!([]));
    }

    #[test]
    fn cancel_is_a_notification_and_permissions_use_the_nested_outcome() {
        let cancel = session_cancel("s1");
        assert!(cancel.get("id").is_none());
        let selected = permission_selected(&RequestId::Number(5), "allow-once");
        assert_eq!(selected["id"], 5);
        assert_eq!(selected["result"]["outcome"]["optionId"], "allow-once");
        assert_eq!(
            permission_cancelled(&RequestId::Text("p".into()))["result"]["outcome"]["outcome"],
            "cancelled"
        );
    }

    #[test]
    fn context_blocks_carry_uri_text_and_optional_mime() {
        let block = resource_block("file:///w/a.rs", "fn a() {}", Some("text/x-rust"));
        assert_eq!(block["resource"]["mimeType"], "text/x-rust");
        assert!(
            resource_block("file:///w/a.txt", "x", None)["resource"]
                .get("mimeType")
                .is_none()
        );
        assert_eq!(
            resource_link_block("file:///w/a.rs", "a.rs")["type"],
            "resource_link"
        );
    }
}
