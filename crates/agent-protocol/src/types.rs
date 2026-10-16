//! Typed views of the ACP messages Lumen understands.
//!
//! Parsing is deliberately forgiving: unknown fields are ignored, unknown enum
//! values map to an `Other` variant, and a missing optional field is just absent.
//! Only fields the client cannot work without make parsing fail.
use serde_json::Value;
use std::collections::BTreeMap;

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn array<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

// ---------------------------------------------------------------- initialize

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub prompt: PromptCapabilities,
    /// `sessionCapabilities` keys the agent advertised (`list`, `resume`, ...).
    pub session: BTreeMap<String, Value>,
}

impl AgentCapabilities {
    #[must_use]
    pub fn supports_session(&self, name: &str) -> bool {
        self.session.contains_key(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InitializeResult {
    pub protocol_version: u32,
    pub capabilities: AgentCapabilities,
    pub agent_name: Option<String>,
    pub agent_title: Option<String>,
    pub agent_version: Option<String>,
    pub auth_methods: Vec<AuthMethod>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub &'static str);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "unexpected agent reply: {}", self.0)
    }
}

pub fn parse_initialize(result: &Value) -> Result<InitializeResult, DecodeError> {
    let protocol_version = result
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or(DecodeError("initialize result has no protocolVersion"))?;
    let caps = result.get("agentCapabilities").unwrap_or(&Value::Null);
    let flag = |value: &Value, key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let prompt = caps.get("promptCapabilities").unwrap_or(&Value::Null);
    let session = caps
        .get("sessionCapabilities")
        .and_then(Value::as_object)
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let info = result.get("agentInfo").unwrap_or(&Value::Null);
    Ok(InitializeResult {
        protocol_version,
        capabilities: AgentCapabilities {
            load_session: flag(caps, "loadSession"),
            prompt: PromptCapabilities {
                image: flag(prompt, "image"),
                audio: flag(prompt, "audio"),
                embedded_context: flag(prompt, "embeddedContext"),
            },
            session,
        },
        agent_name: text(info, "name"),
        agent_title: text(info, "title"),
        agent_version: text(info, "version"),
        auth_methods: array(result, "authMethods")
            .filter_map(|method| {
                Some(AuthMethod {
                    id: text(method, "id")?,
                    name: text(method, "name").unwrap_or_default(),
                    description: text(method, "description"),
                })
            })
            .collect(),
    })
}

// ------------------------------------------------------------------- session

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigValue {
    pub value: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `mode`, `model`, `thought_level`, `model_config`, or an agent-specific name.
    pub category: Option<String>,
    /// `select` or `boolean`; only `select` is offered in the UI.
    pub kind: String,
    /// Current value; booleans are rendered as `"true"` / `"false"`.
    pub current: String,
    /// Flattened choices (groups are merged in order).
    pub values: Vec<ConfigValue>,
}

pub fn parse_config_options(value: &Value) -> Vec<ConfigOption> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let current = match option.get("currentValue")? {
                Value::String(text) => text.clone(),
                Value::Bool(flag) => flag.to_string(),
                _ => return None,
            };
            let mut values = Vec::new();
            for entry in array(option, "options") {
                // A group nests its own `options`; a plain choice has `value`.
                if entry.get("options").is_some() {
                    values.extend(array(entry, "options").filter_map(parse_config_value));
                } else if let Some(value) = parse_config_value(entry) {
                    values.push(value);
                }
            }
            Some(ConfigOption {
                id: text(option, "id")?,
                name: text(option, "name").unwrap_or_default(),
                description: text(option, "description"),
                category: text(option, "category"),
                kind: text(option, "type").unwrap_or_else(|| "select".to_owned()),
                current,
                values,
            })
        })
        .collect()
}

fn parse_config_value(value: &Value) -> Option<ConfigValue> {
    Some(ConfigValue {
        value: text(value, "value")?,
        name: text(value, "name").unwrap_or_default(),
        description: text(value, "description"),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewSession {
    pub session_id: String,
    pub config_options: Vec<ConfigOption>,
    /// Legacy `modes` (id, name); used only when `config_options` has no mode.
    pub current_mode: Option<String>,
    pub modes: Vec<(String, String)>,
}

pub fn parse_new_session(result: &Value) -> Result<NewSession, DecodeError> {
    let modes = result.get("modes").unwrap_or(&Value::Null);
    Ok(NewSession {
        session_id: text(result, "sessionId").ok_or(DecodeError("session has no sessionId"))?,
        config_options: result
            .get("configOptions")
            .map(parse_config_options)
            .unwrap_or_default(),
        current_mode: text(modes, "currentModeId"),
        modes: array(modes, "availableModes")
            .filter_map(|mode| Some((text(mode, "id")?, text(mode, "name").unwrap_or_default())))
            .collect(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Other,
}

pub fn parse_stop_reason(result: &Value) -> StopReason {
    match result.get("stopReason").and_then(Value::as_str) {
        Some("end_turn") => StopReason::EndTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("max_turn_requests") => StopReason::MaxTurnRequests,
        Some("refusal") => StopReason::Refusal,
        Some("cancelled") => StopReason::Cancelled,
        _ => StopReason::Other,
    }
}

/// An entry from `session/list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub updated_at: Option<String>,
}

/// Parse a `session/list` result, keeping only sessions whose `cwd` equals
/// `workspace`: agents may list every session the user has anywhere.
#[must_use]
pub fn parse_session_list(result: &Value, workspace: &str) -> Vec<SessionSummary> {
    array(result, "sessions")
        .filter_map(|session| {
            let cwd = text(session, "cwd")?;
            (cwd == workspace).then(|| SessionSummary {
                session_id: text(session, "sessionId").unwrap_or_default(),
                cwd,
                title: text(session, "title"),
                updated_at: text(session, "updatedAt"),
            })
        })
        .filter(|session| !session.session_id.is_empty())
        .collect()
}

/// One page of `session/list`: this workspace's sessions plus the cursor for more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPage {
    pub sessions: Vec<SessionSummary>,
    pub next_cursor: Option<String>,
}

#[must_use]
pub fn parse_session_page(result: &Value, workspace: &str) -> SessionPage {
    SessionPage {
        sessions: parse_session_list(result, workspace),
        next_cursor: text(result, "nextCursor").filter(|cursor| !cursor.is_empty()),
    }
}

/// The result of `session/load`. Unlike `session/new` it may omit `sessionId`
/// (Codex does); the id that was asked for is the session.
pub fn parse_loaded_session(result: &Value, requested: &str) -> NewSession {
    let mut session = parse_new_session(result).unwrap_or_else(|_| NewSession {
        session_id: requested.to_owned(),
        config_options: result
            .get("configOptions")
            .map(parse_config_options)
            .unwrap_or_default(),
        current_mode: None,
        modes: Vec::new(),
    });
    if session.session_id.is_empty() {
        session.session_id = requested.to_owned();
    }
    session
}

// ------------------------------------------------------------------- content

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentBlock {
    Text(String),
    /// Anything that is not text, summarized for display (`image`, `resource`, ...).
    Other(String),
}

impl ContentBlock {
    #[must_use]
    pub fn display_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Other(kind) => format!("[{kind}]"),
        }
    }
}

pub fn parse_content_block(value: &Value) -> ContentBlock {
    match value.get("type").and_then(Value::as_str) {
        Some("text") => ContentBlock::Text(text(value, "text").unwrap_or_default()),
        Some(kind) => ContentBlock::Other(kind.to_owned()),
        None => ContentBlock::Other("unknown".to_owned()),
    }
}

// --------------------------------------------------------------- tool calls

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    #[default]
    Other,
}

impl ToolKind {
    fn parse(value: &str) -> Self {
        match value {
            "read" => Self::Read,
            "edit" => Self::Edit,
            "delete" => Self::Delete,
            "move" => Self::Move,
            "search" => Self::Search,
            "execute" => Self::Execute,
            "think" => Self::Think,
            "fetch" => Self::Fetch,
            "switch_mode" => Self::SwitchMode,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl ToolStatus {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolContent {
    Text(String),
    Diff {
        path: String,
        old_text: Option<String>,
        new_text: String,
    },
    Terminal {
        terminal_id: String,
    },
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: String,
    pub line: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: Option<String>,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub content: Vec<ToolContent>,
    pub locations: Vec<Location>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
}

/// A `tool_call_update`: every field except the id is optional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolCallPatch {
    pub id: String,
    pub name: Option<String>,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub content: Option<Vec<ToolContent>>,
    pub locations: Option<Vec<Location>>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
}

impl ToolCall {
    /// Fields absent from the patch (or `null`) keep their value. `content` and
    /// `locations` replace the whole collection when present, as the spec says.
    pub fn apply(&mut self, patch: ToolCallPatch) {
        if let Some(name) = patch.name {
            self.name = Some(name);
        }
        if let Some(title) = patch.title {
            self.title = title;
        }
        if let Some(kind) = patch.kind {
            self.kind = kind;
        }
        if let Some(status) = patch.status {
            self.status = status;
        }
        if let Some(content) = patch.content {
            self.content = content;
        }
        if let Some(locations) = patch.locations {
            self.locations = locations;
        }
        if patch.raw_input.is_some() {
            self.raw_input = patch.raw_input;
        }
        if patch.raw_output.is_some() {
            self.raw_output = patch.raw_output;
        }
    }
}

fn parse_tool_content(value: &Value) -> ToolContent {
    match value.get("type").and_then(Value::as_str) {
        Some("content") => {
            match parse_content_block(value.get("content").unwrap_or(&Value::Null)) {
                ContentBlock::Text(text) => ToolContent::Text(text),
                ContentBlock::Other(kind) => ToolContent::Other(kind),
            }
        }
        Some("diff") => ToolContent::Diff {
            path: text(value, "path").unwrap_or_default(),
            old_text: text(value, "oldText"),
            new_text: text(value, "newText").unwrap_or_default(),
        },
        Some("terminal") => ToolContent::Terminal {
            terminal_id: text(value, "terminalId").unwrap_or_default(),
        },
        Some(kind) => ToolContent::Other(kind.to_owned()),
        None => ToolContent::Other("unknown".to_owned()),
    }
}

fn parse_locations(value: &Value) -> Vec<Location> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|location| {
            Some(Location {
                path: text(location, "path")?,
                line: location
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| u32::try_from(line).ok()),
            })
        })
        .collect()
}

fn non_null(value: &Value, key: &str) -> Option<Value> {
    value.get(key).filter(|value| !value.is_null()).cloned()
}

pub fn parse_tool_call(update: &Value) -> Option<ToolCall> {
    let mut call = ToolCall {
        id: text(update, "toolCallId")?,
        title: text(update, "title").unwrap_or_default(),
        ..ToolCall::default()
    };
    let patch = parse_tool_patch(update)?;
    call.apply(patch);
    Some(call)
}

pub fn parse_tool_patch(update: &Value) -> Option<ToolCallPatch> {
    Some(ToolCallPatch {
        id: text(update, "toolCallId")?,
        name: text(update, "name"),
        title: text(update, "title"),
        kind: text(update, "kind").map(|kind| ToolKind::parse(&kind)),
        status: text(update, "status").and_then(|status| ToolStatus::parse(&status)),
        content: update
            .get("content")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(parse_tool_content).collect()),
        locations: update.get("locations").map(parse_locations),
        raw_input: non_null(update, "rawInput"),
        raw_output: non_null(update, "rawOutput"),
    })
}

// -------------------------------------------------------------- permissions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: PermissionKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    pub session_id: String,
    pub tool_call: ToolCall,
    pub options: Vec<PermissionOption>,
}

pub fn parse_permission_request(params: &Value) -> Result<PermissionRequest, DecodeError> {
    let tool = params
        .get("toolCall")
        .ok_or(DecodeError("permission request has no toolCall"))?;
    let options: Vec<PermissionOption> = array(params, "options")
        .filter_map(|option| {
            let kind = match option.get("kind").and_then(Value::as_str)? {
                "allow_once" => PermissionKind::AllowOnce,
                "allow_always" => PermissionKind::AllowAlways,
                "reject_once" => PermissionKind::RejectOnce,
                "reject_always" => PermissionKind::RejectAlways,
                _ => return None,
            };
            Some(PermissionOption {
                id: text(option, "optionId")?,
                name: text(option, "name").unwrap_or_default(),
                kind,
            })
        })
        .collect();
    if options.is_empty() {
        return Err(DecodeError("permission request has no usable options"));
    }
    Ok(PermissionRequest {
        session_id: text(params, "sessionId").unwrap_or_default(),
        tool_call: parse_tool_call(tool).ok_or(DecodeError("permission toolCall has no id"))?,
        options,
    })
}

// ------------------------------------------------------------------- updates

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    pub status: PlanStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
    pub input_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    UserMessage(ContentBlock),
    AgentMessage(ContentBlock),
    AgentThought(ContentBlock),
    ToolCall(ToolCall),
    ToolCallUpdate(ToolCallPatch),
    Plan(Vec<PlanEntry>),
    AvailableCommands(Vec<AvailableCommand>),
    ConfigOptions(Vec<ConfigOption>),
    CurrentMode(String),
    SessionInfo {
        title: Option<String>,
    },
    /// A `sessionUpdate` kind this version does not render (usage, notices, ...).
    Ignored(String),
}

/// Decode the `update` object of a `session/update` notification.
pub fn parse_update(update: &Value) -> Result<Update, DecodeError> {
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .ok_or(DecodeError("update has no sessionUpdate"))?;
    let chunk = || parse_content_block(update.get("content").unwrap_or(&Value::Null));
    Ok(match kind {
        "user_message_chunk" => Update::UserMessage(chunk()),
        "agent_message_chunk" => Update::AgentMessage(chunk()),
        "agent_thought_chunk" => Update::AgentThought(chunk()),
        "tool_call" => Update::ToolCall(
            parse_tool_call(update).ok_or(DecodeError("tool_call has no toolCallId"))?,
        ),
        "tool_call_update" => Update::ToolCallUpdate(
            parse_tool_patch(update).ok_or(DecodeError("tool_call_update has no toolCallId"))?,
        ),
        "plan" => Update::Plan(
            array(update, "entries")
                .map(|entry| PlanEntry {
                    content: text(entry, "content").unwrap_or_default(),
                    status: match entry.get("status").and_then(Value::as_str) {
                        Some("completed") => PlanStatus::Completed,
                        Some("in_progress") => PlanStatus::InProgress,
                        _ => PlanStatus::Pending,
                    },
                })
                .collect(),
        ),
        "available_commands_update" => Update::AvailableCommands(
            array(update, "availableCommands")
                .filter_map(|command| {
                    Some(AvailableCommand {
                        name: text(command, "name")?,
                        description: text(command, "description").unwrap_or_default(),
                        input_hint: command.get("input").and_then(|input| text(input, "hint")),
                    })
                })
                .collect(),
        ),
        "config_option_update" => Update::ConfigOptions(
            update
                .get("configOptions")
                .map(parse_config_options)
                .unwrap_or_default(),
        ),
        "current_mode_update" => Update::CurrentMode(
            text(update, "currentModeId").ok_or(DecodeError("mode update has no mode id"))?,
        ),
        "session_info_update" => Update::SessionInfo {
            title: text(update, "title"),
        },
        other => Update::Ignored(other.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_call_updates_merge_without_erasing_known_fields() {
        let mut call = parse_tool_call(&json!({
            "toolCallId": "t1", "sessionUpdate": "tool_call", "kind": "edit",
            "title": "Preparing file…", "status": "pending", "rawInput": {}, "content": [], "locations": []
        }))
        .unwrap();
        call.apply(
            parse_tool_patch(&json!({
                "toolCallId": "t1", "title": "Write notes.txt", "rawInput": {"file_path": "/w/notes.txt"},
                "locations": [{"path": "/w/notes.txt", "line": null}]
            }))
            .unwrap(),
        );
        call.apply(
            parse_tool_patch(&json!({
                "toolCallId": "t1",
                "content": [{"type": "diff", "path": "/w/notes.txt", "oldText": null, "newText": "hi"}]
            }))
            .unwrap(),
        );
        call.apply(
            parse_tool_patch(&json!({"toolCallId": "t1", "status": "completed", "rawInput": null}))
                .unwrap(),
        );
        assert_eq!(call.title, "Write notes.txt");
        assert_eq!(
            (call.kind, call.status),
            (ToolKind::Edit, ToolStatus::Completed)
        );
        assert_eq!(
            call.raw_input,
            Some(json!({"file_path": "/w/notes.txt"})),
            "null leaves it unchanged"
        );
        assert_eq!(call.locations[0].path, "/w/notes.txt");
        assert_eq!(
            call.content,
            vec![ToolContent::Diff {
                path: "/w/notes.txt".into(),
                old_text: None,
                new_text: "hi".into()
            }]
        );
    }

    #[test]
    fn unknown_enum_values_and_update_kinds_degrade_instead_of_failing() {
        let call =
            parse_tool_call(&json!({"toolCallId": "x", "kind": "teleport", "status": "weird"}))
                .unwrap();
        assert_eq!(
            (call.kind, call.status),
            (ToolKind::Other, ToolStatus::Pending)
        );
        assert_eq!(
            parse_update(&json!({"sessionUpdate": "usage_update", "used": 5})),
            Ok(Update::Ignored("usage_update".into()))
        );
        assert!(parse_update(&json!({"nothing": true})).is_err());
        assert!(parse_update(&json!({"sessionUpdate": "tool_call"})).is_err());
        assert_eq!(
            parse_stop_reason(&json!({"stopReason": "future_reason"})),
            StopReason::Other
        );
    }

    #[test]
    fn config_options_flatten_groups_and_stringify_booleans() {
        let options = parse_config_options(&json!([
            {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "a",
             "options": [{"value": "a", "name": "A"}, {"group": "g", "name": "G", "options": [{"value": "b", "name": "B"}]}]},
            {"id": "fast", "name": "Fast", "type": "boolean", "currentValue": false},
            {"id": "broken", "name": "No current value"}
        ]));
        assert_eq!(
            options.len(),
            2,
            "options without a current value are dropped"
        );
        assert_eq!(
            options[0]
                .values
                .iter()
                .map(|v| v.value.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(
            (options[1].kind.as_str(), options[1].current.as_str()),
            ("boolean", "false")
        );
    }

    #[test]
    fn session_list_keeps_only_the_workspace() {
        let result = json!({"sessions": [
            {"sessionId": "1", "cwd": "/w", "title": "mine"},
            {"sessionId": "2", "cwd": "/elsewhere", "title": "private"},
            {"cwd": "/w"}
        ]});
        let sessions = parse_session_list(&result, "/w");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title.as_deref(), Some("mine"));
    }

    #[test]
    fn permission_requests_need_options_and_a_tool_call() {
        let request = parse_permission_request(&json!({
            "sessionId": "s", "toolCall": {"toolCallId": "t", "title": "Edit files", "kind": "edit"},
            "options": [
                {"optionId": "a", "name": "Yes", "kind": "allow_once"},
                {"optionId": "n", "name": "No", "kind": "reject_once"},
                {"optionId": "future", "name": "?", "kind": "allow_forever"}
            ]
        }))
        .unwrap();
        assert_eq!(request.options.len(), 2, "unknown option kinds are skipped");
        assert!(
            parse_permission_request(&json!({"toolCall": {"toolCallId": "t"}, "options": []}))
                .is_err()
        );
        assert!(
            parse_permission_request(&json!({"options": [{"optionId":"a","kind":"allow_once"}]}))
                .is_err()
        );
    }

    #[test]
    fn session_pages_carry_a_cursor_and_loaded_sessions_fall_back_to_the_requested_id() {
        let page = parse_session_page(
            &json!({"sessions":[{"sessionId":"1","cwd":"/w"},{"sessionId":"2","cwd":"/x"}],"nextCursor":"abc"}),
            "/w",
        );
        assert_eq!(
            (page.sessions.len(), page.next_cursor.as_deref()),
            (1, Some("abc"))
        );
        assert_eq!(
            parse_session_page(&json!({"sessions":[],"nextCursor":""}), "/w").next_cursor,
            None
        );

        let codex_style = parse_loaded_session(
            &json!({"modes":{"currentModeId":"agent"},"configOptions":[
                {"id":"mode","name":"Mode","currentValue":"agent","options":[{"value":"agent","name":"Auto"}]}]}),
            "asked-for",
        );
        assert_eq!(codex_style.session_id, "asked-for");
        assert_eq!(codex_style.config_options.len(), 1);
        let claude_style = parse_loaded_session(&json!({"sessionId":"echoed"}), "asked-for");
        assert_eq!(claude_style.session_id, "echoed");
        assert_eq!(parse_loaded_session(&json!({}), "x").session_id, "x");
    }
}
