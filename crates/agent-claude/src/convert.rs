//! Pure mappings between Claude Code stream-json and the common interface.

use std::path::PathBuf;

use lumen_agent_core::{
    AgentError, RequestToken,
    api::{
        ApprovalDecisionId, AuthState, ConfigChoice, ConfigOption, ConfigRevision, ConfigScope,
        InputBlock, Mutability, SessionConfig,
    },
    event::{ApprovalChoice, ApprovalRequest, RiskLevel, TurnOutcome, Usage},
    model::{ToolCall, ToolKind},
};
use serde_json::{Value, json};

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

// ------------------------------------------------------------------- tools

pub fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Bash" | "BashOutput" | "KillShell" => ToolKind::Command,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => ToolKind::FileEdit,
        "Read" | "NotebookRead" => ToolKind::Read,
        "Glob" | "Grep" | "WebSearch" => ToolKind::Search,
        "WebFetch" => ToolKind::Fetch,
        _ => ToolKind::Other,
    }
}

/// The file a tool call is about, if its input names one.
pub fn tool_paths(input: &Value) -> Vec<PathBuf> {
    ["file_path", "path", "notebook_path"]
        .iter()
        .filter_map(|key| text(input, key))
        .map(PathBuf::from)
        .collect()
}

/// A short title: the tool name plus the most telling input.
pub fn tool_title(name: &str, input: &Value) -> String {
    let detail = match name {
        "Bash" => text(input, "command").map(str::to_string),
        "Glob" | "Grep" => text(input, "pattern").map(str::to_string),
        "WebFetch" => text(input, "url").map(str::to_string),
        "WebSearch" => text(input, "query").map(str::to_string),
        "Task" => text(input, "description").map(str::to_string),
        _ => tool_paths(input).first().map(|p| p.display().to_string()),
    };
    match detail {
        Some(detail) if !detail.is_empty() => {
            let one_line: String = detail
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(160)
                .collect();
            format!("{name} {one_line}")
        }
        _ => name.to_string(),
    }
}

/// A patch the tool proposes, when its input carries one.
pub fn tool_patch(name: &str, input: &Value) -> Option<String> {
    match name {
        "Edit" => {
            let (old, new) = (text(input, "old_string")?, text(input, "new_string")?);
            Some(format!(
                "{}\n{}",
                prefix_lines('-', old),
                prefix_lines('+', new)
            ))
        }
        "Write" => Some(prefix_lines('+', text(input, "content")?)),
        _ => None,
    }
}

fn prefix_lines(sign: char, body: &str) -> String {
    body.lines()
        .map(|line| format!("{sign}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn tool_call(name: &str, input: &Value, parent: Option<&str>) -> ToolCall {
    let mut call = ToolCall::new(tool_kind(name), tool_title(name, input));
    call.locations = tool_paths(input);
    call.patch = tool_patch(name, input);
    call.parent = parent.map(|id| lumen_agent_core::ItemId(id.to_string()));
    call
}

/// Text of a `tool_result` content (a string or an array of text blocks).
pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| text(block, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

// -------------------------------------------------------------- turn results

pub fn turn_outcome(result: &Value, interrupt_requested: bool) -> TurnOutcome {
    let subtype = text(result, "subtype").unwrap_or_default();
    let is_error = result
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let message = text(result, "result").map(str::to_string);
    let status = result.get("api_error_status").and_then(Value::as_u64);
    match subtype {
        "success" if !is_error => TurnOutcome::Completed,
        _ if interrupt_requested && subtype == "error_during_execution" => TurnOutcome::Interrupted,
        _ => {
            let lower = message.clone().unwrap_or_default().to_lowercase();
            if status == Some(429) || lower.contains("rate limit") || lower.contains("usage limit")
            {
                TurnOutcome::Limited(message)
            } else if status == Some(401)
                || lower.contains("not logged in")
                || lower.contains("authentication")
            {
                TurnOutcome::Failed(AgentError::AuthenticationRequired)
            } else {
                TurnOutcome::Failed(AgentError::Provider {
                    message: message.unwrap_or_else(|| format!("the turn ended with {subtype}")),
                })
            }
        }
    }
}

pub fn usage(result: &Value) -> Usage {
    let usage = result.get("usage");
    let number = |key: &str| usage.and_then(|u| u.get(key)).and_then(Value::as_u64);
    let input = number("input_tokens");
    let output = number("output_tokens");
    let cached = number("cache_read_input_tokens").unwrap_or(0)
        + number("cache_creation_input_tokens").unwrap_or(0);
    Usage {
        input_tokens: input.map(|i| i + cached),
        output_tokens: output,
        total_tokens: match (input, output) {
            (Some(i), Some(o)) => Some(i + cached + o),
            _ => None,
        },
        context_window: None,
    }
}

// ----------------------------------------------------------------- approvals

pub const ALLOW: &str = "allow";
pub const DENY: &str = "deny";

pub fn approval_request(request: &Value, token: RequestToken) -> Option<ApprovalRequest> {
    let tool = text(request, "tool_name")?;
    let input = request.get("input").cloned().unwrap_or(Value::Null);
    let summary = text(request, "description")
        .filter(|d| !d.is_empty())
        .map(|d| format!("{tool}: {d}"))
        .unwrap_or_else(|| format!("Use {tool}"));
    let choice =
        |id: &str, label: &str, scope: &str, allows: bool, risk: RiskLevel| ApprovalChoice {
            id: ApprovalDecisionId(id.into()),
            label: label.into(),
            scope: scope.into(),
            allows,
            risk,
        };
    Some(ApprovalRequest {
        token,
        summary,
        command: (tool == "Bash")
            .then(|| text(&input, "command").map(str::to_string))
            .flatten(),
        paths: tool_paths(&input),
        choices: vec![
            choice(
                ALLOW,
                "Allow once",
                "this tool call only",
                true,
                RiskLevel::Medium,
            ),
            choice(
                DENY,
                "Deny",
                "this tool call; the agent continues",
                false,
                RiskLevel::Low,
            ),
        ],
    })
}

// ------------------------------------------------------------------ config

pub const MODELS: [(&str, &str); 4] = [
    ("default", "Default"),
    ("sonnet", "Sonnet"),
    ("opus", "Opus"),
    ("haiku", "Haiku"),
];

pub const MODES: [(&str, &str); 3] = [
    ("default", "Ask before editing"),
    ("acceptEdits", "Accept edits"),
    ("plan", "Plan only"),
];

pub fn session_config(revision: u64, model: &str, mode: &str) -> SessionConfig {
    let choices = |pairs: &[(&str, &str)]| -> Vec<ConfigChoice> {
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
                description: "Applied when the session's process next starts, so it takes effect on the next message".into(),
                choices: choices(&MODELS),
                current: model.into(),
                scope: ConfigScope::Session,
                mutability: Mutability::NextTurn,
            },
            ConfigOption {
                id: "permission_mode".into(),
                label: "Permissions".into(),
                description: "Whether the agent asks before editing files".into(),
                choices: choices(&MODES),
                current: mode.into(),
                scope: ConfigScope::Session,
                mutability: Mutability::NextTurn,
            },
        ],
    }
}

// ------------------------------------------------------------- input & auth

pub fn input_blocks(blocks: &[InputBlock]) -> Result<Vec<Value>, AgentError> {
    let mut out = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block {
            InputBlock::Text(text) => out.push(json!({"type": "text", "text": text})),
            // `@path` is Claude Code's own file mention syntax.
            InputBlock::FileReference { path } => {
                out.push(json!({"type": "text", "text": format!("@{}", path.display())}))
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
                out.push(json!({"type": "text", "text": format!(
                    "{kind} from the editor: {origin}{rev}. This is context only and may be unsaved.\n```\n{text}\n```")}));
            }
            InputBlock::Image { .. } => {
                return Err(AgentError::Unsupported {
                    reason: "image attachments are not supported for Claude Code yet".into(),
                });
            }
        }
    }
    Ok(out)
}

/// `claude auth status` JSON, reduced to a label that is safe to show (no e-mail).
pub fn auth_state(status: &Value) -> AuthState {
    match status.get("loggedIn").and_then(Value::as_bool) {
        Some(true) => {
            let label = match text(status, "authMethod") {
                Some("claude.ai") => "Claude account".to_string(),
                Some("apiKey") | Some("api_key") => "API key".to_string(),
                Some(other) => other.to_string(),
                None => "signed in".to_string(),
            };
            AuthState::SignedIn { label }
        }
        Some(false) => AuthState::SignedOut,
        None => AuthState::Unknown,
    }
}

/// `2.1.212 (Claude Code)` -> `2.1.212`.
pub fn parse_version(output: &str) -> Option<String> {
    let first = output.split_whitespace().next()?;
    first
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
        .then(|| first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{ProfileId, ProviderId, RuntimeGeneration, SessionId, TurnId};

    fn token() -> RequestToken {
        RequestToken {
            provider: ProviderId::Claude,
            profile: ProfileId::default_profile(),
            session: SessionId(1),
            turn: TurnId("turn-1".into()),
            generation: RuntimeGeneration(1),
            native: "req".into(),
        }
    }

    #[test]
    fn tools_get_a_kind_a_one_line_title_locations_and_a_patch() {
        let write = json!({"file_path": "/w/note.txt", "content": "hi\nthere"});
        let call = tool_call("Write", &write, Some("toolu_parent"));
        assert_eq!(call.kind, ToolKind::FileEdit);
        assert_eq!(call.title, "Write /w/note.txt");
        assert_eq!(call.locations, vec![PathBuf::from("/w/note.txt")]);
        assert_eq!(call.patch.as_deref(), Some("+hi\n+there"));
        assert_eq!(call.parent.as_ref().unwrap().0, "toolu_parent");
        let edit = tool_call(
            "Edit",
            &json!({"file_path": "a", "old_string": "x\ny", "new_string": "z"}),
            None,
        );
        assert_eq!(edit.patch.as_deref(), Some("-x\n-y\n+z"));
        let bash = tool_call(
            "Bash",
            &json!({"command": "cargo build\n&& echo done"}),
            None,
        );
        assert_eq!(
            (bash.kind, bash.title.as_str()),
            (ToolKind::Command, "Bash cargo build")
        );
        assert_eq!(tool_title("Grep", &json!({"pattern": "TODO"})), "Grep TODO");
        assert_eq!(tool_title("Mystery", &json!({})), "Mystery");
        assert_eq!(tool_kind("WebFetch"), ToolKind::Fetch);
        assert!(tool_title("Bash", &json!({"command": "x".repeat(500)})).len() < 200);
    }

    #[test]
    fn tool_result_content_may_be_text_or_blocks() {
        assert_eq!(
            content_text(&json!("Denied by the spike client.")),
            "Denied by the spike client."
        );
        assert_eq!(
            content_text(
                &json!([{"type": "text", "text": "a"}, {"type": "image"}, {"type": "text", "text": "b"}])
            ),
            "a\nb"
        );
        assert_eq!(content_text(&Value::Null), "");
    }

    #[test]
    fn results_map_to_completed_interrupted_limited_and_failed() {
        assert_eq!(
            turn_outcome(&json!({"subtype": "success", "is_error": false}), false),
            TurnOutcome::Completed
        );
        let interrupted = json!({"subtype": "error_during_execution", "is_error": true});
        assert_eq!(turn_outcome(&interrupted, true), TurnOutcome::Interrupted);
        assert!(
            matches!(
                turn_outcome(&interrupted, false),
                TurnOutcome::Failed(AgentError::Provider { .. })
            ),
            "an unrequested error is a failure"
        );
        let limited = json!({"subtype": "success", "is_error": true, "api_error_status": 429, "result": "Rate limit reached"});
        assert!(
            matches!(turn_outcome(&limited, false), TurnOutcome::Limited(Some(m)) if m.contains("Rate limit"))
        );
        let auth = json!({"subtype": "success", "is_error": true, "result": "Not logged in · Please run /login"});
        assert_eq!(
            turn_outcome(&auth, false),
            TurnOutcome::Failed(AgentError::AuthenticationRequired)
        );
        let max_turns = turn_outcome(
            &json!({"subtype": "error_max_turns", "is_error": true}),
            false,
        );
        assert!(
            matches!(max_turns, TurnOutcome::Failed(AgentError::Provider { message }) if message.contains("error_max_turns"))
        );
    }

    #[test]
    fn usage_counts_cached_input_and_stays_unknown_when_absent() {
        let u = usage(
            &json!({"usage": {"input_tokens": 2, "cache_read_input_tokens": 21521, "cache_creation_input_tokens": 9870, "output_tokens": 166}}),
        );
        assert_eq!(
            (u.input_tokens, u.output_tokens, u.total_tokens),
            (Some(31393), Some(166), Some(31559))
        );
        assert_eq!(usage(&json!({})), Usage::default());
    }

    #[test]
    fn permission_requests_offer_only_allow_once_and_deny() {
        let request = json!({"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "rm -rf build"},
            "description": "Remove the build directory", "permission_suggestions": [{"type": "setMode", "mode": "acceptEdits"}]});
        let approval = approval_request(&request, token()).unwrap();
        assert_eq!(approval.summary, "Bash: Remove the build directory");
        assert_eq!(approval.command.as_deref(), Some("rm -rf build"));
        assert_eq!(
            approval
                .choices
                .iter()
                .map(|c| c.id.0.as_str())
                .collect::<Vec<_>>(),
            [ALLOW, DENY]
        );
        assert!(approval.choices[0].allows && !approval.choices[1].allows);
        let write = approval_request(
            &json!({"tool_name": "Write", "input": {"file_path": "/w/a"}}),
            token(),
        )
        .unwrap();
        assert_eq!(
            (write.summary.as_str(), write.command, write.paths),
            ("Use Write", None, vec![PathBuf::from("/w/a")])
        );
        assert!(approval_request(&json!({"subtype": "can_use_tool"}), token()).is_none());
    }

    #[test]
    fn input_blocks_become_content_and_images_fail_before_sending() {
        let blocks = input_blocks(&[
            InputBlock::Text("hi".into()),
            InputBlock::FileReference {
                path: "/w/a.rs".into(),
            },
            InputBlock::TextSnapshot {
                origin: "a.rs".into(),
                revision: None,
                text: "fn x() {}".into(),
            },
        ])
        .unwrap();
        assert_eq!(blocks[0], json!({"type": "text", "text": "hi"}));
        assert_eq!(blocks[1], json!({"type": "text", "text": "@/w/a.rs"}));
        let snap = blocks[2]["text"].as_str().unwrap();
        assert!(
            snap.contains("File from the editor: a.rs.")
                && snap.contains("may be unsaved")
                && snap.contains("fn x() {}")
        );
        assert!(matches!(
            input_blocks(&[InputBlock::Image {
                media_type: "image/png".into(),
                bytes: vec![]
            }]),
            Err(AgentError::Unsupported { .. })
        ));
    }

    #[test]
    fn auth_and_version_parsing_never_expose_the_email() {
        let status =
            json!({"loggedIn": true, "authMethod": "claude.ai", "email": "me@example.com"});
        assert_eq!(
            auth_state(&status),
            AuthState::SignedIn {
                label: "Claude account".into()
            }
        );
        assert_eq!(
            auth_state(&json!({"loggedIn": false})),
            AuthState::SignedOut
        );
        assert_eq!(auth_state(&json!({})), AuthState::Unknown);
        assert_eq!(
            parse_version("2.1.212 (Claude Code)\n"),
            Some("2.1.212".into())
        );
        assert_eq!(parse_version("error: not found"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn config_offers_models_and_permission_modes_without_bypass() {
        let config = session_config(2, "sonnet", "plan");
        assert_eq!(config.revision, ConfigRevision(2));
        assert_eq!(config.get("model").unwrap().current, "sonnet");
        let modes: Vec<_> = config
            .get("permission_mode")
            .unwrap()
            .choices
            .iter()
            .map(|c| c.value.as_str())
            .collect();
        assert_eq!(modes, ["default", "acceptEdits", "plan"]);
        assert!(!modes.contains(&"bypassPermissions"));
    }
}
