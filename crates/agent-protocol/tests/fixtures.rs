//! Replays transcripts captured from the real Claude and Codex adapters
//! (see plan/acp-spike) through the protocol crate.
use lumen_agent_protocol::{
    framing::{LineDecoder, encode},
    handshake::{check_version, is_auth_required},
    reducer::{Conversation, Entry},
    rpc::{Incoming, RequestId, RpcError, parse_incoming},
    types::{
        PermissionKind, StopReason, ToolContent, ToolKind, ToolStatus, Update, parse_initialize,
        parse_new_session, parse_permission_request, parse_session_list, parse_stop_reason,
        parse_update,
    },
};
use serde_json::Value;
use std::path::PathBuf;

struct Line {
    outgoing: bool,
    message: Value,
}

fn fixture(name: &str) -> Vec<Line> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .lines()
        .filter_map(|line| {
            let row: Value = serde_json::from_str(line).unwrap();
            let outgoing = match row["dir"].as_str()? {
                "in" => false,
                "out" => true,
                _ => return None,
            };
            Some(Line {
                outgoing,
                message: row["msg"].clone(),
            })
        })
        .collect()
}

const ALL: [&str; 10] = [
    "claude-init",
    "codex-init",
    "claude-edit",
    "codex-edit",
    "claude-terminal",
    "codex-readonly",
    "claude-cancel2",
    "codex-cancel2",
    "claude-loggedout",
    "codex-loggedout",
];

fn incoming(lines: &[Line]) -> impl Iterator<Item = Incoming> + '_ {
    lines
        .iter()
        .filter(|line| !line.outgoing)
        .map(|line| parse_incoming(&line.message).unwrap())
}

fn response(lines: &[Line], id: i64) -> Option<Result<Value, RpcError>> {
    incoming(lines).find_map(|message| match message {
        Incoming::Response {
            id: RequestId::Number(found),
            outcome,
        } if found == id => Some(outcome),
        _ => None,
    })
}

fn permission_requests(lines: &[Line]) -> Vec<Value> {
    incoming(lines)
        .filter_map(|message| match message {
            Incoming::Request { method, params, .. } if method == "session/request_permission" => {
                Some(params)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn every_captured_message_has_a_valid_envelope_and_typed_updates() {
    for name in ALL {
        let lines = fixture(name);
        for line in &lines {
            parse_incoming(&line.message)
                .unwrap_or_else(|error| panic!("{name}: {error}: {}", line.message));
        }
        for message in incoming(&lines) {
            if let Incoming::Notification { method, params } = message
                && method == "session/update"
            {
                parse_update(&params["update"])
                    .unwrap_or_else(|error| panic!("{name}: {error}: {params}"));
            }
        }
        for params in permission_requests(&lines) {
            parse_permission_request(&params)
                .unwrap_or_else(|error| panic!("{name}: {error}: {params}"));
        }
    }
}

#[test]
fn transcripts_survive_any_chunking_through_the_line_decoder() {
    for name in ["claude-edit", "codex-edit"] {
        let messages: Vec<Value> = fixture(name).into_iter().map(|line| line.message).collect();
        let bytes: Vec<u8> = messages.iter().flat_map(encode).collect();
        for size in [1, 7, 64, 4096, bytes.len()] {
            let mut decoder = LineDecoder::default();
            let decoded: Vec<Value> = bytes
                .chunks(size)
                .flat_map(|chunk| decoder.push(chunk))
                .map(|result| result.unwrap_or_else(|error| panic!("{name} @{size}: {error}")))
                .collect();
            assert_eq!(decoded, messages, "{name} chunked by {size}");
        }
    }
}

#[test]
fn both_adapters_negotiate_v1_and_advertise_what_the_spike_recorded() {
    let claude = fixture("claude-init");
    let init = parse_initialize(&response(&claude, 1).unwrap().unwrap()).unwrap();
    assert_eq!(check_version(&init), Ok(()));
    assert!(init.capabilities.load_session && init.capabilities.prompt.embedded_context);
    assert!(
        init.capabilities.supports_session("list") && init.capabilities.supports_session("resume")
    );
    assert!(init.auth_methods.is_empty(), "Claude uses the stored login");
    assert_eq!(init.agent_title.as_deref(), Some("Claude Agent"));
    let session = parse_new_session(&response(&claude, 2).unwrap().unwrap()).unwrap();
    assert!(!session.session_id.is_empty());
    let categories: Vec<_> = session
        .config_options
        .iter()
        .filter_map(|o| o.category.as_deref())
        .collect();
    assert!(categories.contains(&"mode") && categories.contains(&"model"));
    let mode = session
        .config_options
        .iter()
        .find(|o| o.id == "mode")
        .unwrap();
    assert_eq!(mode.current, "default");
    assert!(mode.values.iter().any(|v| v.value == "acceptEdits"));

    let codex = fixture("codex-init");
    let init = parse_initialize(&response(&codex, 1).unwrap().unwrap()).unwrap();
    assert_eq!(check_version(&init), Ok(()));
    let methods: Vec<_> = init.auth_methods.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(methods, ["api-key", "chat-gpt"]);
    let session = parse_new_session(&response(&codex, 2).unwrap().unwrap()).unwrap();
    assert_eq!(session.current_mode.as_deref(), Some("agent"));
    assert!(
        session
            .config_options
            .iter()
            .any(|o| o.category.as_deref() == Some("thought_level"))
    );
}

fn replay(name: &str) -> (Conversation, Option<StopReason>) {
    let mut conversation = Conversation::default();
    let mut stop = None;
    for message in incoming(&fixture(name)) {
        match message {
            Incoming::Notification { method, params } if method == "session/update" => {
                conversation.apply(parse_update(&params["update"]).unwrap());
            }
            Incoming::Response {
                outcome: Ok(result),
                ..
            } if result.get("stopReason").is_some() => {
                stop = Some(parse_stop_reason(&result));
            }
            _ => {}
        }
    }
    (conversation, stop)
}

fn tool_calls(conversation: &Conversation) -> Vec<&lumen_agent_protocol::types::ToolCall> {
    conversation
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect()
}

#[test]
fn claude_edit_turn_reduces_to_read_then_write_with_a_reviewable_diff() {
    let (conversation, stop) = replay("claude-edit");
    assert_eq!(stop, Some(StopReason::EndTurn));
    let calls = tool_calls(&conversation);
    assert_eq!(calls.len(), 2);
    assert_eq!(
        (calls[0].kind, calls[0].status),
        (ToolKind::Read, ToolStatus::Completed)
    );
    assert!(
        calls[0].title.starts_with("Read hello.txt"),
        "{}",
        calls[0].title
    );
    assert_eq!(
        (calls[1].kind, calls[1].status),
        (ToolKind::Edit, ToolStatus::Completed)
    );
    assert!(matches!(
        &calls[1].content[..],
        [ToolContent::Diff { old_text: None, new_text, .. }] if new_text == "hi from agent"
    ));
    assert!(!conversation.agent_text().is_empty());
}

#[test]
fn claude_permission_request_precedes_the_write_and_carries_the_diff() {
    let lines = fixture("claude-edit");
    let params = permission_requests(&lines).remove(0);
    let request = parse_permission_request(&params).unwrap();
    assert_eq!(request.tool_call.kind, ToolKind::Edit);
    assert!(matches!(
        request.tool_call.content[0],
        ToolContent::Diff { .. }
    ));
    let kinds: Vec<_> = request.options.iter().map(|o| o.kind).collect();
    assert_eq!(
        kinds,
        [
            PermissionKind::AllowOnce,
            PermissionKind::AllowAlways,
            PermissionKind::RejectOnce
        ]
    );
}

#[test]
fn codex_edits_arrive_as_shell_commands_without_any_diff() {
    let (conversation, stop) = replay("codex-edit");
    assert_eq!(stop, Some(StopReason::EndTurn));
    let calls = tool_calls(&conversation);
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(
            (call.kind, call.status),
            (ToolKind::Execute, ToolStatus::Completed)
        );
        assert!(
            call.content
                .iter()
                .all(|c| !matches!(c, ToolContent::Diff { .. }))
        );
        assert!(
            call.raw_input
                .as_ref()
                .is_some_and(|input| input.get("command").is_some())
        );
    }
    assert!(conversation.agent_text().len() > 5);
    assert!(
        conversation.title.is_some(),
        "session_info_update names the thread"
    );
}

#[test]
fn codex_read_only_permissions_have_no_diff_but_distinguish_edit_from_execute() {
    let lines = fixture("codex-readonly");
    let requests: Vec<_> = permission_requests(&lines)
        .iter()
        .map(|params| parse_permission_request(params).unwrap())
        .collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].tool_call.kind, ToolKind::Edit);
    assert_eq!(requests[1].tool_call.kind, ToolKind::Execute);
    assert!(requests.iter().all(|r| r.tool_call.content.is_empty()));
    assert!(
        requests[0].tool_call.locations[0]
            .path
            .ends_with("notes2.txt")
    );
    assert!(
        requests[1]
            .options
            .iter()
            .any(|o| o.kind == PermissionKind::AllowAlways)
    );
}

#[test]
fn cancelled_turns_report_cancelled_and_adapters_never_use_client_methods() {
    for name in ["claude-cancel2", "codex-cancel2"] {
        assert_eq!(replay(name).1, Some(StopReason::Cancelled), "{name}");
    }
    for name in [
        "claude-edit",
        "codex-edit",
        "claude-terminal",
        "codex-readonly",
    ] {
        for message in incoming(&fixture(name)) {
            if let Incoming::Request { method, .. } = message {
                assert_eq!(
                    method, "session/request_permission",
                    "{name}: unexpected client request"
                );
            }
        }
    }
}

#[test]
fn sign_in_is_required_at_prompt_for_claude_and_at_new_session_for_codex() {
    let claude = fixture("claude-loggedout");
    assert!(
        response(&claude, 2).unwrap().is_ok(),
        "session/new works logged out"
    );
    let error = response(&claude, 3).unwrap().unwrap_err();
    assert!(is_auth_required(&error));

    let codex = fixture("codex-loggedout");
    let error = response(&codex, 2).unwrap().unwrap_err();
    assert!(is_auth_required(&error));
    let init = parse_initialize(&response(&codex, 1).unwrap().unwrap()).unwrap();
    assert_eq!(
        init.auth_methods.len(),
        1,
        "NO_BROWSER hides the ChatGPT method"
    );
}

#[test]
fn session_lists_only_expose_the_requested_workspace() {
    let lines = fixture("claude-terminal");
    let result = incoming(&lines)
        .find_map(|message| match message {
            Incoming::Response {
                outcome: Ok(result),
                ..
            } if result.get("sessions").is_some() => Some(result),
            _ => None,
        })
        .expect("a session list");
    assert!(parse_session_list(&result, "/nowhere").is_empty());
    let cwd = result["sessions"][0]["cwd"].as_str().unwrap();
    assert_eq!(parse_session_list(&result, cwd).len(), 1);
}

#[test]
fn unknown_update_kinds_in_real_traffic_are_ignored_not_fatal() {
    let mut seen_ignored = false;
    for name in ["claude-edit", "codex-edit"] {
        for message in incoming(&fixture(name)) {
            if let Incoming::Notification { method, params } = message
                && method == "session/update"
                && matches!(parse_update(&params["update"]), Ok(Update::Ignored(_)))
            {
                seen_ignored = true;
            }
        }
    }
    assert!(
        seen_ignored,
        "usage_update and friends exist in the captures"
    );
}
