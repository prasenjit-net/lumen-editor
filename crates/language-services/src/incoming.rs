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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: u64,
    pub start: Position,
    pub end: Position,
    pub children: Vec<DocumentSymbol>,
}

pub fn parse_document_symbols_response(
    message: &Value,
) -> Result<Vec<DocumentSymbol>, IncomingError> {
    let Some(result) = message.get("result") else {
        return Err(IncomingError::InvalidPayload);
    };
    if result.is_null() {
        return Ok(Vec::new());
    }
    result
        .as_array()
        .ok_or(IncomingError::InvalidPayload)?
        .iter()
        .map(parse_symbol)
        .collect()
}

fn parse_symbol(value: &Value) -> Result<DocumentSymbol, IncomingError> {
    let object = value.as_object().ok_or(IncomingError::InvalidPayload)?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or(IncomingError::InvalidPayload)?
        .to_owned();
    let kind = object
        .get("kind")
        .and_then(Value::as_u64)
        .ok_or(IncomingError::InvalidPayload)?;
    let range = object
        .get("selectionRange")
        .or_else(|| object.get("range"))
        .and_then(Value::as_object)
        .ok_or(IncomingError::InvalidPayload)?;
    let children = object
        .get("children")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(parse_symbol).collect())
        .transpose()?
        .unwrap_or_default();
    Ok(DocumentSymbol {
        name,
        kind,
        start: parse_position(range.get("start"))?,
        end: parse_position(range.get("end"))?,
        children,
    })
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
    let locations = parse_definition_locations_response(message)?;
    match locations.len() {
        0 => Ok(None),
        1 => Ok(locations.into_iter().next()),
        _ => Err(IncomingError::InvalidPayload),
    }
}

pub fn parse_definition_locations_response(
    message: &Value,
) -> Result<Vec<DefinitionLocation>, IncomingError> {
    let Some(result) = message.get("result") else {
        return Err(IncomingError::InvalidPayload);
    };
    if result.is_null() {
        return Ok(Vec::new());
    }
    let locations = match result {
        Value::Object(_) => std::slice::from_ref(result),
        Value::Array(locations) => locations.as_slice(),
        _ => return Err(IncomingError::InvalidPayload),
    };
    locations.iter().map(parse_definition_location).collect()
}

fn parse_definition_location(value: &Value) -> Result<DefinitionLocation, IncomingError> {
    let location = value.as_object().ok_or(IncomingError::InvalidPayload)?;
    let uri = location
        .get("uri")
        .or_else(|| location.get("targetUri"))
        .and_then(Value::as_str)
        .ok_or(IncomingError::InvalidPayload)?
        .to_owned();
    let range = location
        .get("range")
        .or_else(|| location.get("targetSelectionRange"))
        .and_then(Value::as_object)
        .ok_or(IncomingError::InvalidPayload)?;
    Ok(DefinitionLocation {
        uri,
        start: parse_position(range.get("start"))?,
        end: parse_position(range.get("end"))?,
    })
}

/// One byte-range edit against the text the request was made for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: std::ops::Range<usize>,
    pub new_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    /// LSP `CompletionItemKind` number.
    pub kind: Option<u64>,
    pub filter_text: String,
    pub insert_text: String,
    /// Server-chosen replacement range, when the item carried a `textEdit`.
    pub edit_range: Option<std::ops::Range<usize>>,
    pub additional_edits: Vec<TextEdit>,
}

/// Edits for one file from a `WorkspaceEdit`, in document order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTextEdits {
    pub uri: String,
    /// Document version the server says the edits apply to, if it said.
    pub version: Option<u64>,
    pub edits: Vec<TextEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceEditError {
    Invalid,
    /// Create/rename/delete file operations are not supported by Lumen.
    ResourceOperation,
    /// Text for a file named by the edit was unavailable.
    UnknownFile(String),
    Overlap(String),
}

/// Decode a `WorkspaceEdit` (`changes` or `documentChanges`) into per-file byte
/// edits. `text_of` supplies each file's text as the server saw it. Overlapping
/// edits and resource operations reject the whole edit: a partial rename is worse
/// than none.
pub fn parse_workspace_edit(
    edit: &Value,
    mut text_of: impl FnMut(&str) -> Option<String>,
) -> Result<Vec<FileTextEdits>, WorkspaceEditError> {
    let mut files: Vec<FileTextEdits> = Vec::new();
    let mut add = |uri: &str,
                   version: Option<u64>,
                   raw: &[Value],
                   text_of: &mut dyn FnMut(&str) -> Option<String>|
     -> Result<(), WorkspaceEditError> {
        let text = text_of(uri).ok_or_else(|| WorkspaceEditError::UnknownFile(uri.to_owned()))?;
        let mut edits = Vec::with_capacity(raw.len());
        for entry in raw {
            let range = entry.get("range").ok_or(WorkspaceEditError::Invalid)?;
            edits.push(TextEdit {
                range: parse_byte_range(range, &text).map_err(|_| WorkspaceEditError::Invalid)?,
                new_text: entry
                    .get("newText")
                    .and_then(Value::as_str)
                    .ok_or(WorkspaceEditError::Invalid)?
                    .to_owned(),
            });
        }
        match files.iter_mut().find(|file| file.uri == uri) {
            Some(file) => file.edits.extend(edits),
            None => files.push(FileTextEdits {
                uri: uri.to_owned(),
                version,
                edits,
            }),
        }
        Ok(())
    };
    if let Some(changes) = edit.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            if change.get("kind").is_some() {
                return Err(WorkspaceEditError::ResourceOperation);
            }
            let document = change
                .get("textDocument")
                .ok_or(WorkspaceEditError::Invalid)?;
            let uri = document
                .get("uri")
                .and_then(Value::as_str)
                .ok_or(WorkspaceEditError::Invalid)?;
            let edits = change
                .get("edits")
                .and_then(Value::as_array)
                .ok_or(WorkspaceEditError::Invalid)?;
            add(
                uri,
                document.get("version").and_then(Value::as_u64),
                edits,
                &mut text_of,
            )?;
        }
    } else if let Some(changes) = edit.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            let edits = edits.as_array().ok_or(WorkspaceEditError::Invalid)?;
            add(uri, None, edits, &mut text_of)?;
        }
    } else {
        return Err(WorkspaceEditError::Invalid);
    }
    for file in &mut files {
        file.edits
            .sort_by_key(|edit| (edit.range.start, edit.range.end));
        if file
            .edits
            .windows(2)
            .any(|pair| pair[0].range.end > pair[1].range.start)
        {
            return Err(WorkspaceEditError::Overlap(file.uri.clone()));
        }
    }
    files.retain(|file| !file.edits.is_empty());
    Ok(files)
}

/// Decode a formatting reply (`TextEdit[]` or null) against the requested text.
pub fn parse_text_edits_response(
    message: &Value,
    text: &str,
) -> Result<Vec<TextEdit>, IncomingError> {
    let result = message.get("result").ok_or(IncomingError::InvalidPayload)?;
    if result.is_null() {
        return Ok(Vec::new());
    }
    let mut edits = result
        .as_array()
        .ok_or(IncomingError::InvalidPayload)?
        .iter()
        .map(|entry| {
            Ok(TextEdit {
                range: parse_byte_range(
                    entry.get("range").ok_or(IncomingError::InvalidPayload)?,
                    text,
                )?,
                new_text: entry
                    .get("newText")
                    .and_then(Value::as_str)
                    .ok_or(IncomingError::InvalidPayload)?
                    .to_owned(),
            })
        })
        .collect::<Result<Vec<_>, IncomingError>>()?;
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    if edits
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start)
    {
        return Err(IncomingError::InvalidRange);
    }
    Ok(edits)
}

const MAX_COMPLETION_ITEMS: usize = 500;

/// Decode a completion reply (list, `CompletionList`, or null) against the exact
/// text it was requested for. Items with malformed ranges are skipped rather than
/// rejecting the whole reply; snippet items are skipped because Lumen does not
/// advertise snippet support. Items are ordered by `sortText` then label.
pub fn parse_completion_response(
    message: &Value,
    text: &str,
) -> Result<Vec<CompletionItem>, IncomingError> {
    let result = message.get("result").ok_or(IncomingError::InvalidPayload)?;
    let entries = match result {
        Value::Null => return Ok(Vec::new()),
        Value::Array(items) => items,
        Value::Object(list) => list
            .get("items")
            .and_then(Value::as_array)
            .ok_or(IncomingError::InvalidPayload)?,
        _ => return Err(IncomingError::InvalidPayload),
    };
    let mut items = entries
        .iter()
        .filter_map(|entry| parse_completion_item(entry, text))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.label.cmp(&right.1.label)));
    items.truncate(MAX_COMPLETION_ITEMS);
    Ok(items.into_iter().map(|(_, item)| item).collect())
}

fn parse_completion_item(entry: &Value, text: &str) -> Option<(String, CompletionItem)> {
    let label = entry.get("label")?.as_str()?.to_owned();
    if entry.get("insertTextFormat").and_then(Value::as_u64) == Some(2) {
        return None;
    }
    let edit = entry.get("textEdit").filter(|edit| edit.is_object());
    let (edit_range, edit_text) = match edit {
        Some(edit) => {
            let range = edit.get("range").or_else(|| edit.get("replace"))?;
            let new_text = edit.get("newText")?.as_str()?.to_owned();
            (Some(parse_byte_range(range, text).ok()?), Some(new_text))
        }
        None => (None, None),
    };
    let insert_text = edit_text
        .or_else(|| {
            entry
                .get("insertText")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| label.clone());
    let additional_edits = entry
        .get("additionalTextEdits")
        .and_then(Value::as_array)
        .map(|edits| {
            edits
                .iter()
                .map(|edit| {
                    Some(TextEdit {
                        range: parse_byte_range(edit.get("range")?, text).ok()?,
                        new_text: edit.get("newText")?.as_str()?.to_owned(),
                    })
                })
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_else(|| Some(Vec::new()))?;
    let filter_text = entry
        .get("filterText")
        .and_then(Value::as_str)
        .unwrap_or(&label)
        .to_owned();
    let sort = entry
        .get("sortText")
        .and_then(Value::as_str)
        .unwrap_or(&label)
        .to_owned();
    Some((
        sort,
        CompletionItem {
            detail: entry
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_owned),
            kind: entry.get("kind").and_then(Value::as_u64),
            label,
            filter_text,
            insert_text,
            edit_range,
            additional_edits,
        },
    ))
}

fn parse_byte_range(value: &Value, text: &str) -> Result<std::ops::Range<usize>, IncomingError> {
    let start = position_to_byte(text, parse_position(value.get("start"))?)
        .map_err(|_| IncomingError::InvalidRange)?;
    let end = position_to_byte(text, parse_position(value.get("end"))?)
        .map_err(|_| IncomingError::InvalidRange)?;
    if start > end {
        return Err(IncomingError::InvalidRange);
    }
    Ok(start..end)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureHelp {
    pub label: String,
    /// Byte range of the active parameter inside `label`.
    pub active_parameter: Option<std::ops::Range<usize>>,
    pub documentation: Option<String>,
    /// 1-based position among the server's signatures, and the total.
    pub index: usize,
    pub count: usize,
}

/// Decode `textDocument/signatureHelp`; null or an empty list means none.
pub fn parse_signature_help_response(
    message: &Value,
) -> Result<Option<SignatureHelp>, IncomingError> {
    let result = message.get("result").ok_or(IncomingError::InvalidPayload)?;
    if result.is_null() {
        return Ok(None);
    }
    let signatures = result
        .get("signatures")
        .and_then(Value::as_array)
        .ok_or(IncomingError::InvalidPayload)?;
    if signatures.is_empty() {
        return Ok(None);
    }
    let active = result
        .get("activeSignature")
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok())
        .filter(|index| *index < signatures.len())
        .unwrap_or(0);
    let signature = &signatures[active];
    let label = signature
        .get("label")
        .and_then(Value::as_str)
        .ok_or(IncomingError::InvalidPayload)?
        .to_owned();
    let parameter_index = signature
        .get("activeParameter")
        .or_else(|| result.get("activeParameter"))
        .and_then(Value::as_u64)
        .and_then(|index| usize::try_from(index).ok());
    let active_parameter = parameter_index
        .and_then(|index| signature.get("parameters")?.as_array()?.get(index))
        .and_then(|parameter| parameter_range(parameter.get("label")?, &label));
    let documentation = signature.get("documentation").and_then(|doc| {
        doc.as_str()
            .or_else(|| doc.get("value").and_then(Value::as_str))
            .map(str::to_owned)
    });
    Ok(Some(SignatureHelp {
        label,
        active_parameter,
        documentation,
        index: active + 1,
        count: signatures.len(),
    }))
}

/// Parameter labels are either a substring of the signature or `[start, end]`
/// UTF-16 offsets into it.
fn parameter_range(value: &Value, label: &str) -> Option<std::ops::Range<usize>> {
    if let Some(text) = value.as_str() {
        let start = label.find(text)?;
        return Some(start..start + text.len());
    }
    let offsets = value.as_array()?;
    let utf16 = |index: usize| {
        offsets
            .get(index)?
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
    };
    let (start, end) = (utf16(0)?, utf16(1)?);
    let to_byte = |offset: usize| {
        let mut units = 0;
        for (byte, character) in label.char_indices() {
            if units == offset {
                return Some(byte);
            }
            units += character.len_utf16();
            if units > offset {
                return None;
            }
        }
        (units == offset).then_some(label.len())
    };
    let (start, end) = (to_byte(start)?, to_byte(end)?);
    (start < end).then_some(start..end)
}

/// Extract the relative-encoded token data from a full semantic-tokens reply.
/// A null result means the server has no tokens for the document.
pub fn parse_semantic_tokens_response(message: &Value) -> Result<Vec<u32>, IncomingError> {
    let result = message.get("result").ok_or(IncomingError::InvalidPayload)?;
    if result.is_null() {
        return Ok(Vec::new());
    }
    result
        .get("data")
        .and_then(Value::as_array)
        .ok_or(IncomingError::InvalidPayload)?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|number| u32::try_from(number).ok())
                .ok_or(IncomingError::InvalidPayload)
        })
        .collect()
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
        let one = json!({"id":3,"result":[{"uri":"file:///x.rs","range":{"start":{"line":2,"character":4},"end":{"line":2,"character":6}}}]});
        assert!(parse_definition_response(&one).unwrap().is_some());
        let many = json!({"id":3,"result":[{"uri":"file:///a","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}},{"uri":"file:///b","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}}]});
        assert_eq!(
            parse_definition_response(&many),
            Err(IncomingError::InvalidPayload)
        );
        assert_eq!(parse_definition_locations_response(&many).unwrap().len(), 2);
        let link = json!({"id":3,"result":{"targetUri":"file:///x.rs","targetSelectionRange":{"start":{"line":1,"character":2},"end":{"line":1,"character":3}}}});
        assert_eq!(
            parse_definition_locations_response(&link).unwrap()[0].start,
            Position {
                line: 1,
                character: 2
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

    #[test]
    fn symbols_preserve_nesting_and_selection_ranges() {
        let message = json!({"id":4,"result":[{"name":"Thing","kind":5,"selectionRange":{"start":{"line":1,"character":0},"end":{"line":1,"character":5}},"children":[{"name":"run","kind":6,"range":{"start":{"line":2,"character":2},"end":{"line":2,"character":5}}}]}]});
        let symbols = parse_document_symbols_response(&message).unwrap();
        assert_eq!(symbols[0].children[0].name, "run");
    }

    #[test]
    fn semantic_token_replies_accept_null_and_reject_bad_data() {
        assert_eq!(
            parse_semantic_tokens_response(&json!({"id":1,"result":null})),
            Ok(Vec::new())
        );
        assert_eq!(
            parse_semantic_tokens_response(&json!({"id":1,"result":{"data":[0,1,2,3,4]}})),
            Ok(vec![0, 1, 2, 3, 4])
        );
        for bad in [
            json!({"data":[-1]}),
            json!({"data":[4294967296u64]}),
            json!({}),
        ] {
            assert!(parse_semantic_tokens_response(&json!({"id":1,"result":bad})).is_err());
        }
        assert!(parse_semantic_tokens_response(&json!({"id":1,"error":{}})).is_err());
    }

    #[test]
    fn completion_replies_decode_edits_skip_snippets_and_sort() {
        let text = "let 🌍x = foo.\n";
        let reply = json!({"id":1,"result":{"isIncomplete":false,"items":[
            {"label":"zeta","sortText":"2"},
            {"label":"snip","insertTextFormat":2,"insertText":"snip($0)"},
            {"label":"alpha","sortText":"1","detail":"fn()","kind":2,
             "textEdit":{"range":{"start":{"line":0,"character":13},"end":{"line":0,"character":14}},"newText":".alpha"},
             "additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"use a;\n"}]},
            {"label":"bad","textEdit":{"range":{"start":{"line":9,"character":0},"end":{"line":9,"character":1}},"newText":"x"}}
        ]}});
        let items = parse_completion_response(&reply, text).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "zeta"]
        );
        assert_eq!(items[0].edit_range, Some(15..16));
        assert_eq!(items[0].insert_text, ".alpha");
        assert_eq!(items[0].additional_edits[0].range, 0..0);
        assert_eq!(items[1].insert_text, "zeta");
        assert_eq!(items[1].edit_range, None);
        assert_eq!(
            parse_completion_response(&json!({"id":1,"result":null}), text),
            Ok(vec![])
        );
        assert!(parse_completion_response(&json!({"id":1,"result":7}), text).is_err());
    }

    #[test]
    fn signature_help_finds_the_active_parameter_in_both_label_forms() {
        let by_text = json!({"id":1,"result":{"signatures":[{
            "label":"fn add(a: i32, b: i32)",
            "parameters":[{"label":"a: i32"},{"label":"b: i32"}],
            "documentation":{"kind":"plaintext","value":"Adds."}
        }],"activeParameter":1}});
        let help = parse_signature_help_response(&by_text).unwrap().unwrap();
        assert_eq!(&help.label[help.active_parameter.unwrap()], "b: i32");
        assert_eq!(help.documentation.as_deref(), Some("Adds."));
        let by_offsets = json!({"id":1,"result":{"signatures":[{
            "label":"f(🌍: u8, y: u8)",
            "parameters":[{"label":[2,8]},{"label":[10,15]}],"activeParameter":0
        }]}});
        let help = parse_signature_help_response(&by_offsets).unwrap().unwrap();
        assert_eq!(&help.label[help.active_parameter.unwrap()], "🌍: u8");
        assert_eq!((help.index, help.count), (1, 1));
        assert_eq!(
            parse_signature_help_response(&json!({"id":1,"result":null})),
            Ok(None)
        );
        let none = json!({"id":1,"result":{"signatures":[]}});
        assert_eq!(parse_signature_help_response(&none), Ok(None));
    }

    fn workspace_edit_texts(uri: &str) -> Option<String> {
        match uri {
            "file:///a.rs" => Some("let 🌍 = foo;\nfoo();".into()),
            "file:///b.rs" => Some("foo".into()),
            _ => None,
        }
    }

    #[test]
    fn workspace_edits_decode_both_forms_and_sort_per_file() {
        let range =
            |l, s, e| json!({"start":{"line":l,"character":s},"end":{"line":l,"character":e}});
        let changes = json!({"changes":{
            "file:///a.rs":[
                {"range":range(1,0,3),"newText":"bar"},
                {"range":range(0,9,12),"newText":"bar"}
            ],
            "file:///b.rs":[{"range":range(0,0,3),"newText":"bar"}]
        }});
        let files = parse_workspace_edit(&changes, workspace_edit_texts).unwrap();
        let a = files
            .iter()
            .find(|file| file.uri == "file:///a.rs")
            .unwrap();
        assert_eq!(a.edits[0].range, 11..14);
        assert_eq!(a.edits[1].range, 16..19);
        let versioned = json!({"documentChanges":[{
            "textDocument":{"uri":"file:///b.rs","version":4},
            "edits":[{"range":range(0,0,3),"newText":"baz"}]
        }]});
        let files = parse_workspace_edit(&versioned, workspace_edit_texts).unwrap();
        assert_eq!(
            (files[0].version, files[0].edits[0].new_text.as_str()),
            (Some(4), "baz")
        );
    }

    #[test]
    fn workspace_edits_reject_resource_ops_overlaps_and_unknown_files() {
        let range = |s, e| json!({"start":{"line":0,"character":s},"end":{"line":0,"character":e}});
        let rename = json!({"documentChanges":[{"kind":"rename","oldUri":"a","newUri":"b"}]});
        assert_eq!(
            parse_workspace_edit(&rename, workspace_edit_texts),
            Err(WorkspaceEditError::ResourceOperation)
        );
        let overlap = json!({"changes":{"file:///b.rs":[
            {"range":range(0,2),"newText":"x"},{"range":range(1,3),"newText":"y"}]}});
        assert_eq!(
            parse_workspace_edit(&overlap, workspace_edit_texts),
            Err(WorkspaceEditError::Overlap("file:///b.rs".into()))
        );
        let unknown = json!({"changes":{"file:///zzz.rs":[{"range":range(0,1),"newText":"x"}]}});
        assert!(matches!(
            parse_workspace_edit(&unknown, workspace_edit_texts),
            Err(WorkspaceEditError::UnknownFile(_))
        ));
        assert_eq!(
            parse_workspace_edit(&json!({}), workspace_edit_texts),
            Err(WorkspaceEditError::Invalid)
        );
    }

    #[test]
    fn formatting_replies_decode_null_edits_and_reject_overlap() {
        let text = "a  b\n";
        let edit = |s, e, t: &str| json!({"range":{"start":{"line":0,"character":s},"end":{"line":0,"character":e}},"newText":t});
        assert_eq!(
            parse_text_edits_response(&json!({"result":null}), text),
            Ok(vec![])
        );
        let edits = parse_text_edits_response(&json!({"result":[edit(1,3," ")]}), text).unwrap();
        assert_eq!(edits[0].range, 1..3);
        let overlap = json!({"result":[edit(0,3,"x"), edit(2,4,"y")]});
        assert!(parse_text_edits_response(&overlap, text).is_err());
    }
}
