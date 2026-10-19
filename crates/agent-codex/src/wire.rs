//! `codex app-server` framing: newline-delimited JSON-RPC with the `jsonrpc`
//! field omitted. This module only classifies and builds frames; what they mean
//! lives in `convert` and the backend.

use serde_json::{Value, json};

/// One decoded line from the app-server.
#[derive(Clone, Debug, PartialEq)]
pub enum Frame {
    Response {
        id: u64,
        result: Value,
    },
    Error {
        id: u64,
        code: i64,
        message: String,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A request the *server* makes of us (approvals, questions). `id` may be a
    /// number or a string; it is kept as text for use in tokens and replies.
    ServerRequest {
        id: String,
        raw_id: Value,
        method: String,
        params: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    NotJson(String),
    NotAnObject,
    Unclassifiable,
}

pub fn parse(line: &str) -> Result<Frame, FrameError> {
    let value: Value =
        serde_json::from_str(line).map_err(|e| FrameError::NotJson(e.to_string()))?;
    let Value::Object(map) = &value else {
        return Err(FrameError::NotAnObject);
    };
    let method = map.get("method").and_then(Value::as_str);
    let id = map.get("id");
    match (method, id) {
        (Some(method), Some(id)) => Ok(Frame::ServerRequest {
            id: id_text(id),
            raw_id: id.clone(),
            method: method.to_string(),
            params: map.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(method), None) => Ok(Frame::Notification {
            method: method.to_string(),
            params: map.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let id = id.as_u64().ok_or(FrameError::Unclassifiable)?;
            if let Some(error) = map.get("error") {
                Ok(Frame::Error {
                    id,
                    code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                })
            } else {
                Ok(Frame::Response {
                    id,
                    result: map.get("result").cloned().unwrap_or(Value::Null),
                })
            }
        }
        (None, None) => Err(FrameError::Unclassifiable),
    }
}

fn id_text(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn request(id: u64, method: &str, params: Value) -> String {
    let mut text = json!({ "id": id, "method": method, "params": params }).to_string();
    text.push('\n');
    text
}

pub fn notification(method: &str) -> String {
    let mut text = json!({ "method": method }).to_string();
    text.push('\n');
    text
}

/// Answer a server request.
pub fn reply(raw_id: &Value, result: Value) -> String {
    let mut text = json!({ "id": raw_id, "result": result }).to_string();
    text.push('\n');
    text
}

/// Refuse a server request we cannot or will not handle.
pub fn reply_error(raw_id: &Value, code: i64, message: &str) -> String {
    let mut text =
        json!({ "id": raw_id, "error": { "code": code, "message": message } }).to_string();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_classified_by_which_of_method_and_id_they_carry() {
        assert_eq!(
            parse(r#"{"id":3,"result":{"x":1}}"#).unwrap(),
            Frame::Response {
                id: 3,
                result: json!({"x":1})
            }
        );
        assert_eq!(
            parse(r#"{"id":4,"error":{"code":-32600,"message":"bad"}}"#).unwrap(),
            Frame::Error {
                id: 4,
                code: -32600,
                message: "bad".into()
            }
        );
        assert_eq!(
            parse(r#"{"method":"turn/started","params":{"a":1}}"#).unwrap(),
            Frame::Notification {
                method: "turn/started".into(),
                params: json!({"a":1})
            }
        );
        let Frame::ServerRequest {
            id, raw_id, method, ..
        } = parse(r#"{"method":"item/commandExecution/requestApproval","id":0,"params":{}}"#)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (id.as_str(), raw_id, method.as_str()),
            ("0", json!(0), "item/commandExecution/requestApproval")
        );
        let Frame::ServerRequest { id, .. } =
            parse(r#"{"method":"m","id":"abc","params":null}"#).unwrap()
        else {
            panic!()
        };
        assert_eq!(id, "abc");
    }

    #[test]
    fn malformed_lines_are_errors_not_panics() {
        assert!(matches!(parse("not json"), Err(FrameError::NotJson(_))));
        assert_eq!(parse("[1,2]"), Err(FrameError::NotAnObject));
        assert_eq!(parse("{}"), Err(FrameError::Unclassifiable));
        assert_eq!(
            parse(r#"{"id":"str","result":1}"#),
            Err(FrameError::Unclassifiable)
        );
    }

    #[test]
    fn outgoing_frames_are_one_line_without_a_jsonrpc_field() {
        let line = request(7, "turn/start", json!({"threadId":"t"}));
        assert!(line.ends_with('\n') && !line.contains("jsonrpc"));
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(
            notification("initialized"),
            "{\"method\":\"initialized\"}\n"
        );
        assert_eq!(
            reply(&json!(0), json!({"decision":"decline"})),
            "{\"id\":0,\"result\":{\"decision\":\"decline\"}}\n"
        );
        assert!(reply_error(&json!("a"), -32601, "no").contains("-32601"));
    }
}
