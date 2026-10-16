//! JSON-RPC 2.0 envelope handling for both directions.
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RequestId {
    Number(i64),
    Text(String),
}

impl RequestId {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Number(number) => json!(number),
            Self::Text(text) => json!(text),
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => number.as_i64().map(Self::Number),
            Value::String(text) => Some(Self::Text(text.clone())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// The agent asks us for something and waits for the matching response.
    Request {
        id: RequestId,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: RequestId,
        outcome: Result<Value, RpcError>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    NotAnObject,
    WrongVersion,
    Malformed(&'static str),
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("message is not a JSON object"),
            Self::WrongVersion => formatter.write_str("message is not JSON-RPC 2.0"),
            Self::Malformed(reason) => write!(formatter, "malformed message: {reason}"),
        }
    }
}

/// Classify one decoded line. Params are kept as raw JSON; typed decoding happens
/// where a method is understood, so unknown methods never fail here.
pub fn parse_incoming(value: &Value) -> Result<Incoming, EnvelopeError> {
    let object = value.as_object().ok_or(EnvelopeError::NotAnObject)?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(EnvelopeError::WrongVersion);
    }
    let id = object.get("id").filter(|id| !id.is_null());
    let id = match id {
        Some(raw) => Some(RequestId::from_value(raw).ok_or(EnvelopeError::Malformed("id"))?),
        None => None,
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    if let Some(method) = object.get("method").and_then(Value::as_str) {
        return Ok(match id {
            Some(id) => Incoming::Request {
                id,
                method: method.to_owned(),
                params,
            },
            None => Incoming::Notification {
                method: method.to_owned(),
                params,
            },
        });
    }
    let id = id.ok_or(EnvelopeError::Malformed("response without id"))?;
    if let Some(error) = object.get("error") {
        return Ok(Incoming::Response {
            id,
            outcome: Err(parse_error(error)),
        });
    }
    match object.get("result") {
        Some(result) => Ok(Incoming::Response {
            id,
            outcome: Ok(result.clone()),
        }),
        None => Err(EnvelopeError::Malformed("response without result or error")),
    }
}

fn parse_error(error: &Value) -> RpcError {
    RpcError {
        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32603),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_owned(),
        data: error.get("data").cloned(),
    }
}

#[must_use]
pub fn request(id: i64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

#[must_use]
pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

#[must_use]
pub fn response_ok(id: &RequestId, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id.to_value(), "result": result})
}

#[must_use]
pub fn response_error(id: &RequestId, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id.to_value(), "error": {"code": code, "message": message}})
}

/// Standard JSON-RPC code for a method the client does not implement.
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

/// Outgoing requests awaiting a response, with optional deadlines. Time is
/// injected so behaviour is deterministic in tests.
#[derive(Debug, Default)]
pub struct RequestTracker {
    next_id: i64,
    pending: BTreeMap<i64, Pending>,
}

#[derive(Debug)]
struct Pending {
    method: String,
    deadline: Option<Instant>,
}

impl RequestTracker {
    /// Allocate an id and remember the request. Ids start at 1.
    pub fn register(&mut self, method: &str, now: Instant, timeout: Option<Duration>) -> i64 {
        self.next_id += 1;
        self.pending.insert(
            self.next_id,
            Pending {
                method: method.to_owned(),
                deadline: timeout.map(|timeout| now + timeout),
            },
        );
        self.next_id
    }

    /// The method a response answers; `None` for unknown or duplicate responses.
    pub fn complete(&mut self, id: &RequestId) -> Option<String> {
        match id {
            RequestId::Number(number) => self.pending.remove(number).map(|pending| pending.method),
            RequestId::Text(_) => None,
        }
    }

    /// Requests whose deadline passed; they are forgotten, so a late response is
    /// treated as unknown and ignored.
    pub fn expire(&mut self, now: Instant) -> Vec<(i64, String)> {
        let expired: Vec<i64> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline.is_some_and(|deadline| deadline <= now))
            .map(|(id, _)| *id)
            .collect();
        expired
            .into_iter()
            .filter_map(|id| self.pending.remove(&id).map(|pending| (id, pending.method)))
            .collect()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    #[must_use]
    pub fn is_pending(&self, id: i64) -> bool {
        self.pending.contains_key(&id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Convenience for building `params` objects.
#[must_use]
pub fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect::<Map<_, _>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_are_classified_without_understanding_methods() {
        let request = parse_incoming(
            &json!({"jsonrpc":"2.0","id":0,"method":"fs/read_text_file","params":{"path":"/a"}}),
        );
        assert!(matches!(
            request,
            Ok(Incoming::Request {
                id: RequestId::Number(0),
                ..
            })
        ));
        let note = parse_incoming(&json!({"jsonrpc":"2.0","method":"session/update","params":{}}));
        assert!(matches!(note, Ok(Incoming::Notification { .. })));
        let text_id = parse_incoming(&json!({"jsonrpc":"2.0","id":"abc","method":"x"}));
        assert!(matches!(
            text_id,
            Ok(Incoming::Request {
                id: RequestId::Text(_),
                ..
            })
        ));
        let ok = parse_incoming(&json!({"jsonrpc":"2.0","id":3,"result":{"a":1}}));
        assert_eq!(
            ok,
            Ok(Incoming::Response {
                id: RequestId::Number(3),
                outcome: Ok(json!({"a":1}))
            })
        );
        let err = parse_incoming(
            &json!({"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"Authentication required"}}),
        );
        assert_eq!(
            err,
            Ok(Incoming::Response {
                id: RequestId::Number(4),
                outcome: Err(RpcError {
                    code: -32000,
                    message: "Authentication required".into(),
                    data: None
                })
            })
        );
    }

    #[test]
    fn malformed_envelopes_are_errors_not_panics() {
        assert_eq!(parse_incoming(&json!([1])), Err(EnvelopeError::NotAnObject));
        assert_eq!(
            parse_incoming(&json!({"id":1,"result":{}})),
            Err(EnvelopeError::WrongVersion)
        );
        assert!(parse_incoming(&json!({"jsonrpc":"2.0","result":{}})).is_err());
        assert!(parse_incoming(&json!({"jsonrpc":"2.0","id":1})).is_err());
        assert!(parse_incoming(&json!({"jsonrpc":"2.0","id":{"a":1},"method":"x"})).is_err());
        assert!(parse_incoming(&json!({"jsonrpc":"2.0","id":1.5,"method":"x"})).is_err());
    }

    #[test]
    fn tracker_matches_responses_expires_deadlines_and_ignores_strangers() {
        let start = Instant::now();
        let mut tracker = RequestTracker::default();
        let init = tracker.register("initialize", start, Some(Duration::from_secs(5)));
        let prompt = tracker.register("session/prompt", start, None);
        assert_eq!((init, prompt), (1, 2));
        assert_eq!(
            tracker.complete(&RequestId::Number(1)).as_deref(),
            Some("initialize")
        );
        assert_eq!(
            tracker.complete(&RequestId::Number(1)),
            None,
            "duplicates are ignored"
        );
        assert_eq!(tracker.complete(&RequestId::Number(99)), None);
        let slow = tracker.register("session/new", start, Some(Duration::from_secs(1)));
        assert!(tracker.expire(start).is_empty());
        let expired = tracker.expire(start + Duration::from_secs(2));
        assert_eq!(expired, vec![(slow, "session/new".to_owned())]);
        assert!(
            tracker.is_pending(prompt),
            "requests without a deadline never expire"
        );
        assert_eq!(
            tracker.complete(&RequestId::Number(slow)),
            None,
            "late replies are dropped"
        );
        tracker.clear();
        assert!(tracker.is_empty());
    }

    #[test]
    fn replies_echo_the_request_id_type() {
        assert_eq!(
            response_ok(&RequestId::Text("a".into()), json!(null))["id"],
            "a"
        );
        let error = response_error(&RequestId::Number(7), METHOD_NOT_FOUND, "nope");
        assert_eq!(
            (error["id"].clone(), error["error"]["code"].clone()),
            (json!(7), json!(-32601))
        );
    }
}
