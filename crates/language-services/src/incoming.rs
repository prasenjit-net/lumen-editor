//! Typed decoding for safe LSP notifications.
use crate::diagnostics::{Diagnostic, DiagnosticSeverity};
use crate::{Position, position_to_byte};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedDiagnostics {
    pub uri: String,
    pub version: Option<u64>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncomingError {
    WrongMethod,
    InvalidPayload,
    InvalidRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionLocation {
    pub uri: String,
    pub start: Position,
    pub end: Position,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverContents {
    pub text: String,
}

/// Decode plain-text and MarkedString hover forms. Rich markup is retained as
/// text until a deliberately designed renderer is added.
pub fn parse_hover_response(message: &Value) -> Result<Option<HoverContents>, IncomingError> {
    let Some(result) = message.get("result") else {
        return Err(IncomingError::InvalidPayload);
    };
    if result.is_null() {
        return Ok(None);
    }
    let contents = result
        .get("contents")
        .ok_or(IncomingError::InvalidPayload)?;
    let text = match contents {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(hover_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => hover_text(contents).unwrap_or_default(),
        _ => return Err(IncomingError::InvalidPayload),
    };
    (!text.trim().is_empty())
        .then_some(HoverContents { text })
        .ok_or(IncomingError::InvalidPayload)
        .map(Some)
}

fn hover_text(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned).or_else(|| {
        value
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

/// Decode the single-location form of a definition response. Multi-location
/// navigation remains a future picker workflow rather than an arbitrary choice.
pub fn parse_definition_response(
    message: &Value,
) -> Result<Option<DefinitionLocation>, IncomingError> {
    let Some(result) = message.get("result") else {
        return Err(IncomingError::InvalidPayload);
    };
    if result.is_null() {
        return Ok(None);
    }
    let location = result.as_object().ok_or(IncomingError::InvalidPayload)?;
    let uri = location
        .get("uri")
        .and_then(Value::as_str)
        .ok_or(IncomingError::InvalidPayload)?
        .to_owned();
    let range = location
        .get("range")
        .and_then(Value::as_object)
        .ok_or(IncomingError::InvalidPayload)?;
    Ok(Some(DefinitionLocation {
        uri,
        start: parse_position(range.get("start"))?,
        end: parse_position(range.get("end"))?,
    }))
}

/// Decode `textDocument/publishDiagnostics` against the exact synchronized
/// text. Invalid UTF-16 or UTF-8 boundaries reject the whole notification.
pub fn parse_publish_diagnostics(
    message: &Value,
    text: &str,
) -> Result<PublishedDiagnostics, IncomingError> {
    if message.get("method").and_then(Value::as_str) != Some("textDocument/publishDiagnostics") {
        return Err(IncomingError::WrongMethod);
    }
    let params = message
        .get("params")
        .and_then(Value::as_object)
        .ok_or(IncomingError::InvalidPayload)?;
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or(IncomingError::InvalidPayload)?
        .to_owned();
    let version = params.get("version").and_then(Value::as_u64);
    let entries = params
        .get("diagnostics")
        .and_then(Value::as_array)
        .ok_or(IncomingError::InvalidPayload)?;
    let diagnostics = entries
        .iter()
        .map(|entry| parse_diagnostic(entry, text))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PublishedDiagnostics {
        uri,
        version,
        diagnostics,
    })
}

fn parse_diagnostic(value: &Value, text: &str) -> Result<Diagnostic, IncomingError> {
    let range = value
        .get("range")
        .and_then(Value::as_object)
        .ok_or(IncomingError::InvalidPayload)?;
    let start = parse_position(range.get("start"))?;
    let end = parse_position(range.get("end"))?;
    let start = position_to_byte(text, start).map_err(|_| IncomingError::InvalidRange)?;
    let end = position_to_byte(text, end).map_err(|_| IncomingError::InvalidRange)?;
    if start > end {
        return Err(IncomingError::InvalidRange);
    }
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .ok_or(IncomingError::InvalidPayload)?
        .to_owned();
    Ok(Diagnostic {
        range: start..end,
        severity: match value.get("severity").and_then(Value::as_u64) {
            Some(1) => DiagnosticSeverity::Error,
            Some(2) => DiagnosticSeverity::Warning,
            Some(3) => DiagnosticSeverity::Information,
            _ => DiagnosticSeverity::Hint,
        },
        message,
        source: value
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_owned),
        code: value.get("code").and_then(Value::as_str).map(str::to_owned),
    })
}

fn parse_position(value: Option<&Value>) -> Result<Position, IncomingError> {
    let value = value
        .and_then(Value::as_object)
        .ok_or(IncomingError::InvalidPayload)?;
    Ok(Position {
        line: value
            .get("line")
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok())
            .ok_or(IncomingError::InvalidPayload)?,
        character: value
            .get("character")
            .and_then(Value::as_u64)
            .and_then(|value| value.try_into().ok())
            .ok_or(IncomingError::InvalidPayload)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagnostics_convert_utf16_ranges_without_splitting_emoji() {
        let message = json!({"method":"textDocument/publishDiagnostics","params":{
            "uri":"file:///workspace/main.rs", "version": 3,
            "diagnostics":[{"range":{"start":{"line":0,"character":4},"end":{"line":0,"character":6}},"severity":2,"message":"planet"}]
        }});
        let parsed = parse_publish_diagnostics(&message, "let 😀 = 1;").unwrap();
        assert_eq!(parsed.version, Some(3));
        assert_eq!(parsed.diagnostics[0].range, 4..8);
        assert_eq!(parsed.diagnostics[0].severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn malformed_ranges_are_rejected_without_partial_results() {
        let message = json!({"method":"textDocument/publishDiagnostics","params":{
            "uri":"file:///x", "diagnostics":[{"range":{"start":{"line":0,"character":5},"end":{"line":0,"character":6}},"message":"bad"}]
        }});
        assert_eq!(
            parse_publish_diagnostics(&message, "😀"),
            Err(IncomingError::InvalidRange)
        );
    }

    #[test]
    fn definition_response_preserves_utf16_positions_for_the_app_mapper() {
        let message = json!({"id":3,"result":{"uri":"file:///x.rs","range":{"start":{"line":2,"character":4},"end":{"line":2,"character":6}}}});
        assert_eq!(
            parse_definition_response(&message).unwrap().unwrap().start,
            Position {
                line: 2,
                character: 4
            }
        );
    }

    #[test]
    fn hover_decodes_common_marked_string_forms() {
        let message =
            json!({"id":2,"result":{"contents":[{"language":"rust","value":"fn main()"},"docs"]}});
        assert_eq!(
            parse_hover_response(&message).unwrap().unwrap().text,
            "fn main()\ndocs"
        );
    }
}
