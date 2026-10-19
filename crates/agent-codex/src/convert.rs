//! Pure mappings between Codex app-server JSON and the common interface. No I/O
//! and no state: everything here is testable with plain JSON.

use std::path::PathBuf;

use lumen_agent_core::{
    AgentError, ItemId, NativeSessionRef, ProfileId, ProviderId, RequestToken, TurnId,
    api::{
        ApprovalDecisionId, AuthState, ConfigChoice, ConfigOption, ConfigRevision, ConfigScope,
        InputBlock, Mutability, SessionConfig, SessionSummary,
    },
    event::{
        AgentEvent, ApprovalChoice, ApprovalRequest, FileChange, FileChangeKind, QuestionKind,
        QuestionRequest, RiskLevel, TurnOutcome, Usage,
    },
    model::{
        ItemKind, MessagePhase, PlanEntry, PlanStatus, ToolCall, ToolKind, ToolStatus,
        TranscriptItem,
    },
};
use serde_json::{Value, json};

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

// ---------------------------------------------------------------- sessions

pub fn native_ref(profile: &ProfileId, thread_id: &str) -> NativeSessionRef {
    NativeSessionRef {
        provider: ProviderId::Codex,
        profile: profile.clone(),
        id: thread_id.to_string(),
    }
}

/// A `thread` object from `thread/list` or `thread/read`.
pub fn thread_summary(thread: &Value, profile: &ProfileId) -> Option<SessionSummary> {
    let id = text(thread, "id")?;
    let title = text(thread, "name")
        .filter(|name| !name.is_empty())
        .or_else(|| text(thread, "preview").filter(|preview| !preview.is_empty()))
        .map(str::to_string);
    Some(SessionSummary {
        native: native_ref(profile, id),
        title,
        workspace: text(thread, "cwd").map(PathBuf::from),
        last_activity: thread.get("updatedAt").and_then(Value::as_u64),
        archived: false,
    })
}

// ------------------------------------------------------------------- items

fn message_phase(item: &Value) -> MessagePhase {
    match text(item, "phase") {
        Some("commentary") => MessagePhase::Commentary,
        _ => MessagePhase::Final,
    }
}

fn tool_status(status: Option<&str>) -> ToolStatus {
    match status {
        Some("completed") => ToolStatus::Completed,
        Some("failed") => ToolStatus::Failed,
        Some("declined") => ToolStatus::Declined,
        Some("inProgress") => ToolStatus::Running,
        _ => ToolStatus::Running,
    }
}

fn change_kind(change: &Value) -> FileChangeKind {
    match change
        .get("kind")
        .and_then(|kind| kind.get("type"))
        .and_then(Value::as_str)
    {
        Some("add") => FileChangeKind::Created,
        Some("delete") => FileChangeKind::Deleted,
        _ => FileChangeKind::Modified,
    }
}

/// The tool shown for an item, for item types that are tool-like.
pub fn tool_call(item: &Value) -> Option<ToolCall> {
    match text(item, "type")? {
        "commandExecution" => {
            let mut call = ToolCall::new(
                ToolKind::Command,
                text(item, "command").unwrap_or("command"),
            );
            call.status = tool_status(text(item, "status"));
            call.exit_code = item
                .get("exitCode")
                .and_then(Value::as_i64)
                .map(|c| c as i32);
            if let Some(output) = text(item, "aggregatedOutput") {
                call.preview = output.to_string();
            }
            if let Some(cwd) = text(item, "cwd") {
                call.locations = vec![PathBuf::from(cwd)];
            }
            Some(call)
        }
        "fileChange" => {
            let changes = item.get("changes").and_then(Value::as_array);
            let mut call = ToolCall::new(ToolKind::FileEdit, "Edit files");
            call.status = tool_status(text(item, "status"));
            call.locations = changes
                .into_iter()
                .flatten()
                .filter_map(|change| text(change, "path").map(PathBuf::from))
                .collect();
            let patch: Vec<&str> = changes
                .into_iter()
                .flatten()
                .filter_map(|change| text(change, "diff"))
                .collect();
            if !patch.is_empty() {
                call.patch = Some(patch.join("\n"));
            }
            Some(call)
        }
        "mcpToolCall" | "dynamicToolCall" => {
            let name = text(item, "tool")
                .or_else(|| text(item, "name"))
                .unwrap_or("tool");
            let mut call = ToolCall::new(ToolKind::Other, name);
            call.status = tool_status(text(item, "status"));
            Some(call)
        }
        "webSearch" => Some(ToolCall::new(
            ToolKind::Search,
            text(item, "query").unwrap_or("web search"),
        )),
        _ => None,
    }
}

/// File changes a completed `fileChange` item reports.
pub fn file_changes(item: &Value, applied: bool) -> Vec<FileChange> {
    item.get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|change| {
            Some(FileChange {
                path: PathBuf::from(text(change, "path")?),
                kind: change_kind(change),
                applied,
            })
        })
        .collect()
}

fn user_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| text(block, "text"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A finished item as a transcript record (history reads), or `None` for item
/// kinds that are not part of the visible transcript (reasoning, hooks, ...).
pub fn transcript_item(item: &Value, turn: &TurnId) -> Option<TranscriptItem> {
    let id = ItemId(text(item, "id")?.to_string());
    let kind = match text(item, "type")? {
        "userMessage" => ItemKind::User(user_text(item)),
        "agentMessage" => ItemKind::Agent {
            text: text(item, "text").unwrap_or_default().to_string(),
            phase: message_phase(item),
            finished: true,
        },
        "plan" => ItemKind::Plan(plan_from_text(text(item, "text").unwrap_or_default())),
        _ => ItemKind::Tool(tool_call(item)?),
    };
    Some(TranscriptItem {
        id,
        turn: Some(turn.clone()),
        kind,
    })
}

fn plan_from_text(plan: &str) -> Vec<PlanEntry> {
    plan.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| PlanEntry {
            text: line.trim().to_string(),
            status: PlanStatus::Pending,
        })
        .collect()
}

/// Events for `item/started`.
pub fn item_started(item: &Value) -> Vec<AgentEvent> {
    let Some(id) = text(item, "id") else {
        return Vec::new();
    };
    let item_id = ItemId(id.to_string());
    match text(item, "type") {
        Some("agentMessage") => vec![AgentEvent::MessageStarted {
            item: item_id,
            phase: message_phase(item),
        }],
        _ => match tool_call(item) {
            Some(call) => vec![AgentEvent::ToolStarted {
                item: item_id,
                call,
            }],
            None => Vec::new(),
        },
    }
}

/// Events for `item/completed`.
pub fn item_completed(item: &Value) -> Vec<AgentEvent> {
    let Some(id) = text(item, "id") else {
        return Vec::new();
    };
    let item_id = ItemId(id.to_string());
    match text(item, "type") {
        Some("agentMessage") => vec![AgentEvent::MessageFinished { item: item_id }],
        Some("fileChange") => {
            let status = tool_status(text(item, "status"));
            let mut events = vec![AgentEvent::ToolFinished {
                item: item_id,
                status,
            }];
            if status == ToolStatus::Completed {
                events.push(AgentEvent::FileChangesReported(file_changes(item, true)));
            }
            events
        }
        Some("commandExecution")
        | Some("mcpToolCall")
        | Some("dynamicToolCall")
        | Some("webSearch") => {
            let status = tool_status(text(item, "status"));
            let update = lumen_agent_core::model::ToolUpdate {
                exit_code: item
                    .get("exitCode")
                    .and_then(Value::as_i64)
                    .map(|c| c as i32),
                ..Default::default()
            };
            vec![
                AgentEvent::ToolUpdated {
                    item: item_id.clone(),
                    update,
                },
                AgentEvent::ToolFinished {
                    item: item_id,
                    status: if status == ToolStatus::Running {
                        ToolStatus::Completed
                    } else {
                        status
                    },
                },
            ]
        }
        _ => Vec::new(),
    }
}

// -------------------------------------------------------------- turns/usage

/// `codexErrorInfo` is either a bare string or `{ "<variant>": {...} }`.
fn error_info(error: &Value) -> Option<String> {
    match error.get("codexErrorInfo")? {
        Value::String(name) => Some(name.clone()),
        Value::Object(map) => map.keys().next().cloned(),
        _ => None,
    }
}

pub fn classify_error(error: &Value) -> AgentError {
    let message = text(error, "message")
        .unwrap_or("The agent reported an error")
        .to_string();
    match error_info(error).as_deref() {
        Some("unauthorized") => AgentError::AuthenticationRequired,
        Some("usageLimitExceeded" | "rateLimitExceeded" | "sessionBudgetExceeded") => {
            AgentError::RateLimited { retry_after: None }
        }
        _ => {
            let lower = message.to_lowercase();
            if lower.contains("not logged in") || lower.contains("sign in") || lower.contains("401")
            {
                AgentError::AuthenticationRequired
            } else if lower.contains("rate limit") || lower.contains("usage limit") {
                AgentError::RateLimited { retry_after: None }
            } else {
                AgentError::Provider { message }
            }
        }
    }
}

pub fn turn_outcome(turn: &Value) -> TurnOutcome {
    match text(turn, "status") {
        Some("completed") => TurnOutcome::Completed,
        Some("interrupted") => TurnOutcome::Interrupted,
        Some("failed") => {
            let error = turn.get("error").cloned().unwrap_or(Value::Null);
            match classify_error(&error) {
                AgentError::RateLimited { .. } => {
                    TurnOutcome::Limited(text(&error, "message").map(str::to_string))
                }
                other => TurnOutcome::Failed(other),
            }
        }
        // `inProgress` inside a completion event is a protocol surprise: say so.
        other => TurnOutcome::Failed(AgentError::Transport {
            detail: format!("unexpected turn status {other:?}"),
        }),
    }
}

pub fn usage(params: &Value) -> Usage {
    let total = params
        .get("tokenUsage")
        .and_then(|usage| usage.get("total"));
    let number = |key: &str| total.and_then(|t| t.get(key)).and_then(Value::as_u64);
    Usage {
        input_tokens: number("inputTokens"),
        output_tokens: number("outputTokens"),
        total_tokens: number("totalTokens"),
        context_window: params
            .get("tokenUsage")
            .and_then(|usage| usage.get("modelContextWindow"))
            .and_then(Value::as_u64),
    }
}

pub fn plan_entries(params: &Value) -> Vec<PlanEntry> {
    params
        .get("plan")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let step = text(entry, "step")?;
            let status = match text(entry, "status") {
                Some("completed") => PlanStatus::Done,
                Some("inProgress") => PlanStatus::InProgress,
                _ => PlanStatus::Pending,
            };
            Some(PlanEntry {
                text: step.to_string(),
                status,
            })
        })
        .collect()
}

// -------------------------------------------------------------------- auth

/// A label that is safe to show: never an e-mail address or token.
pub fn auth_state(account_result: &Value) -> AuthState {
    match account_result.get("account") {
        Some(account) if !account.is_null() => {
            let label = match (text(account, "type"), text(account, "planType")) {
                (Some("chatgpt"), Some(plan)) => format!("ChatGPT ({plan})"),
                (Some("chatgpt"), None) => "ChatGPT".to_string(),
                (Some("apiKey"), _) => "API key".to_string(),
                (Some(other), _) => other.to_string(),
                _ => "signed in".to_string(),
            };
            AuthState::SignedIn { label }
        }
        Some(_) => AuthState::SignedOut,
        None => AuthState::Unknown,
    }
}

// ------------------------------------------------------------ config/models

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

pub fn models(result: &Value) -> Vec<ModelInfo> {
    result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| model.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter_map(|model| {
            let id = text(model, "model").or_else(|| text(model, "id"))?;
            Some(ModelInfo {
                id: id.to_string(),
                label: text(model, "displayName").unwrap_or(id).to_string(),
                is_default: model.get("isDefault").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect()
}

pub const SANDBOXES: [(&str, &str); 3] = [
    ("read-only", "Read only"),
    ("workspace-write", "Edit the workspace"),
    ("danger-full-access", "Full access (no sandbox)"),
];

pub const APPROVALS: [(&str, &str); 3] = [
    ("untrusted", "Ask for anything not known safe"),
    ("on-request", "Ask when the agent decides to"),
    ("never", "Never ask"),
];

/// The session's settings as advertised options. Provider mode names are not
/// assumed to match Claude's.
pub fn session_config(
    revision: u64,
    models: &[ModelInfo],
    model: &str,
    sandbox: &str,
    approval: &str,
) -> SessionConfig {
    let mut model_choices: Vec<ConfigChoice> = models
        .iter()
        .map(|m| ConfigChoice {
            value: m.id.clone(),
            label: m.label.clone(),
        })
        .collect();
    if !model.is_empty() && !model_choices.iter().any(|c| c.value == model) {
        model_choices.insert(
            0,
            ConfigChoice {
                value: model.into(),
                label: model.into(),
            },
        );
    }
    let simple = |pairs: &[(&str, &str)]| -> Vec<ConfigChoice> {
        pairs
            .iter()
            .map(|(v, l)| ConfigChoice {
                value: (*v).into(),
                label: (*l).into(),
            })
            .collect()
    };
    SessionConfig {
        revision: ConfigRevision(revision),
        options: vec![
            ConfigOption {
                id: "model".into(),
                label: "Model".into(),
                description: "The model that answers".into(),
                choices: model_choices,
                current: model.into(),
                scope: ConfigScope::Session,
                mutability: Mutability::NextTurn,
            },
            ConfigOption {
                id: "sandbox".into(),
                label: "Sandbox".into(),
                description: "What commands the agent may run without asking".into(),
                choices: simple(&SANDBOXES),
                current: sandbox.into(),
                scope: ConfigScope::Session,
                mutability: Mutability::NextTurn,
            },
            ConfigOption {
                id: "approval".into(),
                label: "Approvals".into(),
                description: "When the agent asks before acting".into(),
                choices: simple(&APPROVALS),
                current: approval.into(),
                scope: ConfigScope::Session,
                mutability: Mutability::NextTurn,
            },
        ],
    }
}

/// `sandboxPolicy` for `turn/start` from a sandbox mode name.
pub fn sandbox_policy(mode: &str) -> Option<Value> {
    Some(match mode {
        "read-only" => json!({ "type": "readOnly" }),
        "workspace-write" => json!({ "type": "workspaceWrite" }),
        "danger-full-access" => json!({ "type": "dangerFullAccess" }),
        _ => return None,
    })
}

// ------------------------------------------------------------------- input

/// User input blocks as Codex input items. Anything Codex cannot take is
/// refused here, before a prompt is sent: nothing is silently dropped.
pub fn input_items(blocks: &[InputBlock]) -> Result<Vec<Value>, AgentError> {
    let mut items = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            InputBlock::Text(text) => items.push(json!({ "type": "text", "text": text })),
            InputBlock::FileReference { path } => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                items.push(
                    json!({ "type": "mention", "name": name, "path": path.to_string_lossy() }),
                );
            }
            InputBlock::TextSnapshot {
                origin,
                revision,
                text,
            }
            | InputBlock::SelectionSnapshot {
                origin,
                revision,
                text,
            } => {
                let kind = if matches!(block, InputBlock::TextSnapshot { .. }) {
                    "File"
                } else {
                    "Selection"
                };
                let rev = revision
                    .map(|r| format!(" (editor revision {r})"))
                    .unwrap_or_default();
                items.push(json!({
                    "type": "text",
                    "text": format!("{kind} from the editor: {origin}{rev}. This is context only and may be unsaved.\n```\n{text}\n```")
                }));
            }
            InputBlock::Image { .. } => {
                return Err(AgentError::Unsupported {
                    reason: "image attachments are not supported for Codex yet".into(),
                });
            }
        }
    }
    Ok(items)
}

// --------------------------------------------------------------- approvals

/// Choices Lumen offers for a command or file-change approval. The ids are the
/// provider's own decision names.
fn approval_choices(include_session: bool) -> Vec<ApprovalChoice> {
    let choice =
        |id: &str, label: &str, scope: &str, allows: bool, risk: RiskLevel| ApprovalChoice {
            id: ApprovalDecisionId(id.into()),
            label: label.into(),
            scope: scope.into(),
            allows,
            risk,
        };
    let mut choices = vec![choice(
        "accept",
        "Allow once",
        "this request only",
        true,
        RiskLevel::Medium,
    )];
    if include_session {
        choices.push(choice(
            "acceptForSession",
            "Allow for this session",
            "similar requests until the session ends",
            true,
            RiskLevel::High,
        ));
    }
    choices.push(choice(
        "decline",
        "Decline",
        "this request; the agent continues",
        false,
        RiskLevel::Low,
    ));
    choices.push(choice(
        "cancel",
        "Decline and stop",
        "this request and the turn",
        false,
        RiskLevel::Low,
    ));
    choices
}

pub fn is_approval_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
    )
}

pub fn approval_request(
    method: &str,
    params: &Value,
    token: RequestToken,
) -> Option<ApprovalRequest> {
    match method {
        "item/commandExecution/requestApproval" => Some(ApprovalRequest {
            token,
            summary: text(params, "reason")
                .unwrap_or("Run a command")
                .to_string(),
            command: text(params, "command").map(str::to_string),
            paths: text(params, "cwd").map(PathBuf::from).into_iter().collect(),
            choices: approval_choices(true),
        }),
        "item/fileChange/requestApproval" => Some(ApprovalRequest {
            token,
            summary: text(params, "reason").unwrap_or("Change files").to_string(),
            command: None,
            paths: text(params, "grantRoot")
                .map(PathBuf::from)
                .into_iter()
                .collect(),
            choices: approval_choices(true),
        }),
        _ => None,
    }
}

/// Whether a decision id is one we offered.
pub fn valid_decision(id: &str) -> bool {
    matches!(id, "accept" | "acceptForSession" | "decline" | "cancel")
}

/// A single-question `item/tool/requestUserInput` as a question. Requests with
/// several questions are not supported yet and return `None`.
pub fn question_request(params: &Value, token: RequestToken) -> Option<(QuestionRequest, String)> {
    let questions = params.get("questions").and_then(Value::as_array)?;
    let [question] = questions.as_slice() else {
        return None;
    };
    let id = text(question, "id")?.to_string();
    let prompt = text(question, "question")
        .unwrap_or("The agent has a question")
        .to_string();
    let options: Vec<String> = question
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| text(o, "label").map(str::to_string))
        .collect();
    let kind = if options.is_empty() {
        QuestionKind::Text
    } else {
        QuestionKind::SingleChoice(options)
    };
    Some((
        QuestionRequest {
            token,
            prompt,
            kind,
        },
        id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ProfileId {
        ProfileId::default_profile()
    }

    #[test]
    fn thread_summaries_prefer_the_name_then_the_preview_and_never_invent_fields() {
        let thread = json!({"id":"t1","preview":"Remember tangerine","cwd":"/w","updatedAt":1792392914,"name":null});
        let s = thread_summary(&thread, &profile()).unwrap();
        assert_eq!(s.title.as_deref(), Some("Remember tangerine"));
        assert_eq!(
            (s.workspace, s.last_activity),
            (Some(PathBuf::from("/w")), Some(1792392914))
        );
        let named = json!({"id":"t1","preview":"x","name":"Lumen spike"});
        assert_eq!(
            thread_summary(&named, &profile()).unwrap().title.as_deref(),
            Some("Lumen spike")
        );
        let bare = thread_summary(&json!({"id":"t2"}), &profile()).unwrap();
        assert_eq!(
            (bare.title, bare.workspace, bare.last_activity),
            (None, None, None)
        );
        assert!(thread_summary(&json!({"nope":1}), &profile()).is_none());
        assert_eq!(bare.native.provider, ProviderId::Codex);
    }

    #[test]
    fn a_command_item_maps_to_a_tool_and_finishes_with_its_status_and_exit_code() {
        let started = json!({"type":"commandExecution","id":"exec-1","command":"/bin/zsh -lc \"ls\"","cwd":"/w","status":"inProgress"});
        let events = item_started(&started);
        let [AgentEvent::ToolStarted { item, call }] = events.as_slice() else {
            panic!("{events:?}")
        };
        assert_eq!(item.0, "exec-1");
        assert_eq!(
            (call.kind, call.status),
            (ToolKind::Command, ToolStatus::Running)
        );
        assert!(call.title.contains("zsh"));
        let done =
            json!({"type":"commandExecution","id":"exec-1","status":"declined","exitCode":null});
        let events = item_completed(&done);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::ToolFinished {
                status: ToolStatus::Declined,
                ..
            })
        ));
        let ok = json!({"type":"commandExecution","id":"exec-1","status":"completed","exitCode":0});
        let events = item_completed(&ok);
        assert!(
            matches!(&events[0], AgentEvent::ToolUpdated { update, .. } if update.exit_code == Some(0))
        );
    }

    #[test]
    fn file_changes_are_reported_only_when_applied_and_carry_patches() {
        let item = json!({"type":"fileChange","id":"fc1","status":"completed",
            "changes":[{"path":"a.rs","kind":{"type":"add"},"diff":"+x"},{"path":"b.rs","kind":{"type":"update"},"diff":"@@"},{"path":"c.rs","kind":{"type":"delete"},"diff":""}]});
        let call = tool_call(&item).unwrap();
        assert_eq!(
            call.locations,
            vec![
                PathBuf::from("a.rs"),
                PathBuf::from("b.rs"),
                PathBuf::from("c.rs")
            ]
        );
        assert_eq!(call.patch.as_deref(), Some("+x\n@@\n"));
        let events = item_completed(&item);
        let Some(AgentEvent::FileChangesReported(changes)) = events.last() else {
            panic!("{events:?}")
        };
        assert_eq!(
            changes.iter().map(|c| c.kind).collect::<Vec<_>>(),
            [
                FileChangeKind::Created,
                FileChangeKind::Modified,
                FileChangeKind::Deleted
            ]
        );
        assert!(changes.iter().all(|c| c.applied));
        let declined = json!({"type":"fileChange","id":"fc1","status":"declined","changes":[]});
        assert!(
            !item_completed(&declined)
                .iter()
                .any(|e| matches!(e, AgentEvent::FileChangesReported(_)))
        );
    }

    #[test]
    fn agent_messages_map_with_their_phase_and_history_items_have_stable_ids() {
        let commentary = json!({"type":"agentMessage","id":"m1","text":"","phase":"commentary"});
        assert!(matches!(
            &item_started(&commentary)[0],
            AgentEvent::MessageStarted {
                phase: MessagePhase::Commentary,
                ..
            }
        ));
        let final_answer =
            json!({"type":"agentMessage","id":"m2","text":"ok","phase":"final_answer"});
        assert!(matches!(
            &item_started(&final_answer)[0],
            AgentEvent::MessageStarted {
                phase: MessagePhase::Final,
                ..
            }
        ));
        let turn = TurnId("turn".into());
        let t = transcript_item(&final_answer, &turn).unwrap();
        assert_eq!((t.id.0.as_str(), t.turn.as_ref()), ("m2", Some(&turn)));
        assert!(matches!(t.kind, ItemKind::Agent { ref text, finished: true, .. } if text == "ok"));
        let user = json!({"type":"userMessage","id":"u1","content":[{"type":"text","text":"hi"},{"type":"text","text":"there"}]});
        assert!(
            matches!(transcript_item(&user, &turn).unwrap().kind, ItemKind::User(ref t) if t == "hi\nthere")
        );
        assert!(transcript_item(&json!({"type":"reasoning","id":"r1"}), &turn).is_none());
        assert!(item_started(&json!({"type":"reasoning","id":"r1"})).is_empty());
    }

    #[test]
    fn turn_outcomes_distinguish_completed_interrupted_limited_and_failed() {
        assert_eq!(
            turn_outcome(&json!({"status":"completed"})),
            TurnOutcome::Completed
        );
        assert_eq!(
            turn_outcome(&json!({"status":"interrupted"})),
            TurnOutcome::Interrupted
        );
        let limited = turn_outcome(
            &json!({"status":"failed","error":{"message":"slow down","codexErrorInfo":"usageLimitExceeded"}}),
        );
        assert!(matches!(limited, TurnOutcome::Limited(Some(m)) if m == "slow down"));
        let auth = turn_outcome(
            &json!({"status":"failed","error":{"message":"x","codexErrorInfo":"unauthorized"}}),
        );
        assert_eq!(
            auth,
            TurnOutcome::Failed(AgentError::AuthenticationRequired)
        );
        let model = turn_outcome(
            &json!({"status":"failed","error":{"message":"The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account.","codexErrorInfo":{"badRequest":{}}}}),
        );
        assert!(
            matches!(model, TurnOutcome::Failed(AgentError::Provider { message }) if message.contains("not supported"))
        );
        assert!(matches!(
            turn_outcome(&json!({"status":"inProgress"})),
            TurnOutcome::Failed(AgentError::Transport { .. })
        ));
        let text_only =
            turn_outcome(&json!({"status":"failed","error":{"message":"You are not logged in"}}));
        assert_eq!(
            text_only,
            TurnOutcome::Failed(AgentError::AuthenticationRequired)
        );
    }

    #[test]
    fn usage_and_plans_are_read_defensively() {
        let u = usage(
            &json!({"tokenUsage":{"total":{"totalTokens":16666,"inputTokens":16000,"outputTokens":666},"modelContextWindow":200000}}),
        );
        assert_eq!(
            (
                u.total_tokens,
                u.input_tokens,
                u.output_tokens,
                u.context_window
            ),
            (Some(16666), Some(16000), Some(666), Some(200000))
        );
        assert_eq!(usage(&json!({})), Usage::default());
        let plan = plan_entries(
            &json!({"plan":[{"step":"a","status":"completed"},{"step":"b","status":"inProgress"},{"step":"c","status":"pending"},{"nope":1}]}),
        );
        assert_eq!(
            plan.iter().map(|p| p.status).collect::<Vec<_>>(),
            [
                PlanStatus::Done,
                PlanStatus::InProgress,
                PlanStatus::Pending
            ]
        );
    }

    #[test]
    fn auth_labels_never_contain_the_email_address() {
        let signed_in = auth_state(
            &json!({"account":{"type":"chatgpt","email":"me@example.com","planType":"plus"}}),
        );
        assert_eq!(
            signed_in,
            AuthState::SignedIn {
                label: "ChatGPT (plus)".into()
            }
        );
        assert_eq!(auth_state(&json!({"account":null})), AuthState::SignedOut);
        assert_eq!(auth_state(&json!({})), AuthState::Unknown);
        assert_eq!(
            auth_state(&json!({"account":{"type":"apiKey"}})),
            AuthState::SignedIn {
                label: "API key".into()
            }
        );
    }

    #[test]
    fn config_lists_models_without_hidden_ones_and_keeps_an_unlisted_current_model() {
        let list = models(&json!({"data":[
            {"id":"gpt-a","model":"gpt-a","displayName":"GPT-A","hidden":false},
            {"id":"gpt-h","model":"gpt-h","displayName":"Hidden","hidden":true},
            {"id":"gpt-b","displayName":"GPT-B"}]}));
        assert_eq!(
            list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["gpt-a", "gpt-b"]
        );
        let config = session_config(3, &list, "gpt-z", "read-only", "on-request");
        assert_eq!(config.revision, ConfigRevision(3));
        let model = config.get("model").unwrap();
        assert_eq!(model.current, "gpt-z");
        assert_eq!(
            model.choices[0].value, "gpt-z",
            "the configured model is selectable even when unlisted"
        );
        assert_eq!(config.get("sandbox").unwrap().choices.len(), 3);
        assert_eq!(
            sandbox_policy("workspace-write"),
            Some(json!({"type":"workspaceWrite"}))
        );
        assert_eq!(sandbox_policy("nope"), None);
    }

    #[test]
    fn input_blocks_become_codex_items_and_unsupported_ones_fail_before_sending() {
        let items = input_items(&[
            InputBlock::Text("hi".into()),
            InputBlock::FileReference {
                path: "/w/src/a.rs".into(),
            },
            InputBlock::SelectionSnapshot {
                origin: "a.rs".into(),
                revision: Some(4),
                text: "let x;".into(),
            },
        ])
        .unwrap();
        assert_eq!(items[0], json!({"type":"text","text":"hi"}));
        assert_eq!(
            items[1],
            json!({"type":"mention","name":"a.rs","path":"/w/src/a.rs"})
        );
        let snap = items[2]["text"].as_str().unwrap();
        assert!(
            snap.contains("Selection from the editor: a.rs (editor revision 4)")
                && snap.contains("may be unsaved")
                && snap.contains("let x;")
        );
        let err = input_items(&[
            InputBlock::Text("x".into()),
            InputBlock::Image {
                media_type: "image/png".into(),
                bytes: vec![1],
            },
        ])
        .unwrap_err();
        assert!(matches!(err, AgentError::Unsupported { .. }));
    }

    fn token() -> RequestToken {
        RequestToken {
            provider: ProviderId::Codex,
            profile: profile(),
            session: lumen_agent_core::SessionId(1),
            turn: TurnId("t".into()),
            generation: lumen_agent_core::RuntimeGeneration(1),
            native: "0".into(),
        }
    }

    #[test]
    fn command_and_file_approvals_offer_scoped_choices_and_unknown_methods_are_not_approvals() {
        let params = json!({"reason":"Allow creating note.txt in the workspace? The filesystem is currently read-only.","command":"/bin/zsh -lc \"printf 'hi' > note.txt\"","cwd":"/w"});
        let request =
            approval_request("item/commandExecution/requestApproval", &params, token()).unwrap();
        assert!(request.summary.starts_with("Allow creating note.txt"));
        assert_eq!(
            request.command.as_deref(),
            Some("/bin/zsh -lc \"printf 'hi' > note.txt\"")
        );
        let ids: Vec<_> = request.choices.iter().map(|c| c.id.0.as_str()).collect();
        assert_eq!(ids, ["accept", "acceptForSession", "decline", "cancel"]);
        assert!(request.choices[0].allows && !request.choices[2].allows);
        assert_eq!(request.choices[1].risk, RiskLevel::High);
        assert!(request.choices.iter().all(|c| valid_decision(&c.id.0)));
        let file = approval_request(
            "item/fileChange/requestApproval",
            &json!({"reason":"edit","grantRoot":"/w"}),
            token(),
        )
        .unwrap();
        assert!(file.command.is_none() && file.paths == vec![PathBuf::from("/w")]);
        assert!(
            approval_request("item/permissions/requestApproval", &json!({}), token()).is_none()
        );
        assert!(
            is_approval_method("item/fileChange/requestApproval")
                && !is_approval_method("item/tool/call")
        );
        assert!(
            !valid_decision("acceptWithExecpolicyAmendment"),
            "amendments are never offered"
        );
    }

    #[test]
    fn single_question_requests_map_with_options_and_multi_question_ones_are_declined() {
        let one = json!({"questions":[{"id":"q1","header":"h","question":"Which?","options":[{"label":"a","description":""},{"label":"b","description":""}]}]});
        let (request, id) = question_request(&one, token()).unwrap();
        assert_eq!(id, "q1");
        assert_eq!(
            request.kind,
            QuestionKind::SingleChoice(vec!["a".into(), "b".into()])
        );
        let free = json!({"questions":[{"id":"q2","header":"h","question":"Name?"}]});
        assert_eq!(
            question_request(&free, token()).unwrap().0.kind,
            QuestionKind::Text
        );
        let many = json!({"questions":[{"id":"1","question":"a","header":""},{"id":"2","question":"b","header":""}]});
        assert!(question_request(&many, token()).is_none());
    }
}
