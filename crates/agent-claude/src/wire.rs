//! Claude Code `stream-json` framing (one JSON object per line, both directions)
//! and the control messages used for permissions and interrupts. Only classifies
//! and builds frames; meaning lives in `convert` and the backend.

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// `system` messages: `init`, `status`, `thinking_tokens`, ...
    System {
        subtype: String,
        value: Value,
    },
    /// A partial streaming event (`content_block_delta`, ...).
    StreamEvent {
        event: Value,
        parent_tool: Option<String>,
    },
    /// A whole assistant message.
    Assistant {
        message: Value,
        parent_tool: Option<String>,
    },
    /// A user message echoed back (tool results).
    User {
        message: Value,
        parent_tool: Option<String>,
    },
    Result(Value),
    RateLimit(Value),
    /// The CLI asks us something (permission to use a tool).
    ControlRequest {
        request_id: String,
        request: Value,
    },
    /// The CLI answers a control request of ours.
    ControlResponse {
        request_id: String,
        success: bool,
        response: Value,
    },
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    NotJson(String),
    NotAnObject,
}

fn parent(value: &Value) -> Option<String> {
    value
        .get("parent_tool_use_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn parse(line: &str) -> Result<Message, ParseError> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| ParseError::NotJson(e.to_string()))?;
    if !value.is_object() {
        return Err(ParseError::NotAnObject);
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(match kind.as_str() {
        "system" => Message::System {
            subtype: value
                .get("subtype")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            value,
        },
        "stream_event" => Message::StreamEvent {
            event: value.get("event").cloned().unwrap_or(Value::Null),
            parent_tool: parent(&value),
        },
        "assistant" => Message::Assistant {
            message: value.get("message").cloned().unwrap_or(Value::Null),
            parent_tool: parent(&value),
        },
        "user" => Message::User {
            message: value.get("message").cloned().unwrap_or(Value::Null),
            parent_tool: parent(&value),
        },
        "result" => Message::Result(value),
        "rate_limit_event" => Message::RateLimit(value),
        "control_request" => Message::ControlRequest {
            request_id: value
                .get("request_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            request: value.get("request").cloned().unwrap_or(Value::Null),
        },
        "control_response" => {
            let response = value.get("response").cloned().unwrap_or(Value::Null);
            Message::ControlResponse {
                request_id: response
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                success: response.get("subtype").and_then(Value::as_str) == Some("success"),
                response: response.get("response").cloned().unwrap_or(Value::Null),
            }
        }
        other => Message::Other(other.to_string()),
    })
}

fn line(value: Value) -> String {
    let mut text = value.to_string();
    text.push('\n');
    text
}

/// A user turn; `content` is the Anthropic content-block array.
pub fn user_message(content: Vec<Value>) -> String {
    line(json!({"type": "user", "message": {"role": "user", "content": content}}))
}

pub fn interrupt_request(request_id: &str) -> String {
    line(
        json!({"type": "control_request", "request_id": request_id, "request": {"subtype": "interrupt"}}),
    )
}

/// Allow a tool call, handing back its (possibly unchanged) input.
pub fn allow(request_id: &str, input: &Value) -> String {
    line(json!({"type": "control_response", "response": {
        "subtype": "success", "request_id": request_id,
        "response": {"behavior": "allow", "updatedInput": input}}}))
}

/// Deny a tool call with a message the agent sees.
pub fn deny(request_id: &str, message: &str) -> String {
    line(json!({"type": "control_response", "response": {
        "subtype": "success", "request_id": request_id,
        "response": {"behavior": "deny", "message": message}}}))
}

/// Refuse a control request we do not understand.
pub fn control_error(request_id: &str, message: &str) -> String {
    line(json!({"type": "control_response", "response": {
        "subtype": "error", "request_id": request_id, "error": message}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_classified_by_type_with_their_parent_tool() {
        assert!(matches!(
            parse(r#"{"type":"system","subtype":"init","session_id":"s"}"#).unwrap(),
            Message::System { subtype, .. } if subtype == "init"
        ));
        let Message::StreamEvent { event, parent_tool } =
            parse(r#"{"type":"stream_event","event":{"type":"message_stop"},"parent_tool_use_id":"toolu_1"}"#).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (event["type"].as_str(), parent_tool.as_deref()),
            (Some("message_stop"), Some("toolu_1"))
        );
        assert!(matches!(
            parse(r#"{"type":"assistant","message":{"id":"m"},"parent_tool_use_id":null}"#)
                .unwrap(),
            Message::Assistant {
                parent_tool: None,
                ..
            }
        ));
        assert!(matches!(
            parse(r#"{"type":"result","subtype":"success"}"#).unwrap(),
            Message::Result(_)
        ));
        assert!(matches!(
            parse(r#"{"type":"rate_limit_event"}"#).unwrap(),
            Message::RateLimit(_)
        ));
        assert_eq!(
            parse(r#"{"type":"brand_new"}"#).unwrap(),
            Message::Other("brand_new".into())
        );
    }

    #[test]
    fn control_messages_carry_their_request_ids() {
        let Message::ControlRequest { request_id, request } = parse(
            r#"{"type":"control_request","request_id":"abc","request":{"subtype":"can_use_tool","tool_name":"Write"}}"#,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            (request_id.as_str(), request["tool_name"].as_str()),
            ("abc", Some("Write"))
        );
        let Message::ControlResponse { request_id, success, response } = parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"int-1","response":{"still_queued":[]}}}"#,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!((request_id.as_str(), success), ("int-1", true));
        assert_eq!(response["still_queued"], json!([]));
        let Message::ControlResponse { success, .. } =
            parse(r#"{"type":"control_response","response":{"subtype":"error","request_id":"x","error":"no"}}"#).unwrap()
        else {
            panic!()
        };
        assert!(!success);
    }

    #[test]
    fn malformed_lines_are_errors_and_outgoing_frames_are_single_lines() {
        assert!(matches!(parse("nope"), Err(ParseError::NotJson(_))));
        assert_eq!(parse("[1]"), Err(ParseError::NotAnObject));
        let message = user_message(vec![json!({"type": "text", "text": "hi\nthere"})]);
        assert_eq!(
            message.matches('\n').count(),
            1,
            "newlines inside text are escaped"
        );
        assert!(message.contains("\"role\":\"user\""));
        assert!(interrupt_request("int-1").contains("\"subtype\":\"interrupt\""));
        let allowed: Value =
            serde_json::from_str(allow("r", &json!({"file_path": "a"})).trim()).unwrap();
        assert_eq!(allowed["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allowed["response"]["response"]["updatedInput"]["file_path"],
            "a"
        );
        let denied: Value = serde_json::from_str(deny("r", "no").trim()).unwrap();
        assert_eq!(
            (
                denied["response"]["response"]["behavior"].as_str(),
                denied["response"]["response"]["message"].as_str()
            ),
            (Some("deny"), Some("no"))
        );
        assert!(control_error("r", "unsupported").contains("\"subtype\":\"error\""));
    }
}
