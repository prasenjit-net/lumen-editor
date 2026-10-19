//! Backend tests against a scripted `codex app-server` (no real process): the
//! sim answers requests the way the real server did in the Phase 1 transcripts.

use std::{sync::Arc, time::Duration};

use lumen_agent_core::{
    AgentError, ClientRequestId, EventEnvelope, OperationId, PollBudget, ProviderId, RequestToken,
    SessionId, SubmitError, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, ClientTurnId, ConfigPatch,
        ConfigRevision, InputBlock, OperationOutcome, OperationOutput, QuestionAnswer, Request,
    },
    event::{AgentEvent, TurnOutcome},
    launcher::LaunchError,
    model::{ItemKind, SessionModel, TurnState},
    testing::{FakeChildHandle, FakeLauncher, Responder, contract},
};
use serde_json::{Value, json};

use crate::{CodexBackend, CodexConfig};

const THREAD: &str = "01a152e8-d4d2-7a21-b819-f5cb4a9a1f8d";

fn answer(handle: &FakeChildHandle, id: &Value, result: Value) {
    handle.stdout_line(&json!({"id": id, "result": result}).to_string());
}

/// A scripted app-server: answers the requests Lumen sends. Thread ids are
/// `thread-<n>` after the first (which uses the transcript's id).
fn responder() -> Responder {
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    Arc::new(move |line: &str, handle: &FakeChildHandle| {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let (Some(method), Some(id)) = (message["method"].as_str(), message.get("id")) else {
            return;
        };
        let params = &message["params"];
        let result = match method {
            "initialize" => {
                json!({"userAgent": "lumen/0.157.0 (Mac OS 26.5.2; arm64) unknown (lumen; 0.1)", "codexHome": "~/.codex", "platformFamily": "unix", "platformOs": "macos"})
            }
            "model/list" => {
                json!({"data": [{"id": "gpt-6-astra", "model": "gpt-6-astra", "displayName": "GPT-6-Astra", "hidden": false}, {"id": "gpt-6-lite", "model": "gpt-6-lite", "displayName": "GPT-6-Lite", "hidden": false}], "nextCursor": null})
            }
            "account/read" => {
                json!({"account": {"type": "chatgpt", "email": "me@example.com", "planType": "plus"}, "requiresOpenaiAuth": true})
            }
            "thread/start" => {
                let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let thread = if n == 0 {
                    THREAD.to_string()
                } else {
                    format!("thread-{n}")
                };
                json!({"thread": {"id": thread, "cwd": params["cwd"], "model": "gpt-6-astra"}, "model": "gpt-6-astra", "cwd": params["cwd"]})
            }
            "thread/resume" => {
                json!({"thread": {"id": params["threadId"], "cwd": params["cwd"], "model": "gpt-6-astra"}, "model": "gpt-6-astra"})
            }
            "thread/read" => {
                json!({"thread": {"id": params["threadId"], "turns": [{"id": "turn-1", "status": "completed", "items": [
                {"type": "userMessage", "id": "u1", "content": [{"type": "text", "text": "hi"}]},
                {"type": "agentMessage", "id": "a1", "text": "hello", "phase": "final_answer"},
                {"type": "reasoning", "id": "r1"}]}]}})
            }
            "thread/list" => {
                json!({"data": [{"id": THREAD, "preview": "Remember tangerine", "cwd": "/w", "updatedAt": 1792392914, "name": null}], "nextCursor": null})
            }
            "thread/name/set" | "thread/archive" | "thread/unsubscribe" => json!({}),
            "turn/start" => json!({"turn": {"id": "turn-1", "items": [], "status": "inProgress"}}),
            "turn/interrupt" => json!({}),
            _ => return,
        };
        answer(handle, id, result);
    })
}

struct Rig {
    backend: CodexBackend,
    launcher: FakeLauncher,
    next: u64,
}

impl Rig {
    fn new() -> Self {
        let launcher = FakeLauncher::default();
        *launcher.responder.lock().unwrap() = Some(responder());
        let backend = CodexBackend::new(
            Arc::new(launcher.clone()),
            Arc::new(|| {}),
            CodexConfig::new("codex"),
        );
        Self {
            backend,
            launcher,
            next: 1,
        }
    }

    fn submit(&mut self, operation: AgentOperation) -> OperationId {
        let id = ClientRequestId(self.next);
        self.next += 1;
        self.backend
            .try_submit(Request {
                client_request_id: id,
                operation,
            })
            .expect("accepted")
    }

    fn run(&mut self, operation: AgentOperation) -> (OperationOutcome, Vec<EventEnvelope>) {
        let op = self.submit(operation);
        let events = contract::pump(&mut self.backend, |e| contract::outcome_of(e, op).is_some());
        (contract::outcome_of(&events, op).cloned().unwrap(), events)
    }

    fn child(&self) -> FakeChildHandle {
        let count = self.launcher.children.lock().unwrap().len();
        self.launcher.child(count - 1)
    }

    fn notify(&self, method: &str, params: Value) {
        self.child()
            .stdout_line(&json!({"method": method, "params": params}).to_string());
    }

    fn server_request(&self, id: u64, method: &str, params: Value) {
        self.child()
            .stdout_line(&json!({"method": method, "id": id, "params": params}).to_string());
    }

    fn poll(&mut self) -> Vec<EventEnvelope> {
        self.backend.poll_events(PollBudget::FRAME)
    }

    /// Lines the library wrote to the current child, as JSON.
    fn written(&self) -> Vec<Value> {
        self.child()
            .written_text()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn create(&mut self, session: u64) -> OperationOutput {
        let (outcome, _) = self.run(AgentOperation::CreateSession {
            session: SessionId(session),
            workspace: "/w".into(),
            config: ConfigPatch::default(),
        });
        let OperationOutcome::Succeeded(out) = outcome else {
            panic!("{outcome:?}")
        };
        out
    }

    fn start(&mut self, session: u64, text: &str) -> OperationOutcome {
        self.run(AgentOperation::StartTurn {
            session: SessionId(session),
            client_turn: ClientTurnId("c".into()),
            input: vec![InputBlock::Text(text.into())],
            expected_revision: None,
        })
        .0
    }
}

fn thread_notice(thread: &str, extra: Value) -> Value {
    let mut base = json!({"threadId": thread, "turnId": "turn-1"});
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

fn fold(model: &mut SessionModel, events: &[EventEnvelope], session: u64) {
    for event in events {
        if event.session == Some(SessionId(session)) {
            model.apply(event);
        }
    }
}

#[test]
fn the_first_operation_launches_app_server_handshakes_and_only_then_dispatches() {
    let mut rig = Rig::new();
    let probe = rig.submit(AgentOperation::Probe);
    assert!(
        rig.launcher.launched.lock().unwrap().is_empty(),
        "nothing launches until polled"
    );
    let events = contract::pump(&mut rig.backend, |e| {
        contract::outcome_of(e, probe).is_some()
    });
    let spec = rig.launcher.launched.lock().unwrap()[0].clone();
    assert_eq!(spec.program, std::path::PathBuf::from("codex"));
    assert_eq!(spec.args, vec![std::ffi::OsString::from("app-server")]);
    let written = rig.written();
    let methods: Vec<&str> = written
        .iter()
        .filter_map(|m| m["method"].as_str())
        .collect();
    assert_eq!(
        methods,
        ["initialize", "initialized", "model/list", "account/read"],
        "handshake precedes the first operation"
    );
    assert_eq!(written[0]["params"]["clientInfo"]["name"], "lumen");
    let Some(OperationOutcome::Succeeded(OperationOutput::Probed {
        version,
        capabilities,
        auth,
    })) = contract::outcome_of(&events, probe)
    else {
        panic!("{events:?}")
    };
    assert_eq!(version, "0.157.0");
    assert!(capabilities.resume.is_supported() && capabilities.native_multi_session.is_supported());
    assert!(!capabilities.attachments_image.is_supported());
    assert_eq!(
        *auth,
        lumen_agent_core::api::AuthState::SignedIn {
            label: "ChatGPT (plus)".into()
        }
    );
    assert_eq!(
        rig.backend.descriptor().executable_version.as_deref(),
        Some("0.157.0")
    );
}

#[test]
fn a_missing_executable_fails_queued_operations_with_a_typed_error_and_retries_next_time() {
    let mut rig = Rig::new();
    *rig.launcher.fail_with.lock().unwrap() = Some(LaunchError::NotFound("codex".into()));
    let (outcome, _) = rig.run(AgentOperation::Probe);
    assert!(matches!(
        outcome,
        OperationOutcome::Failed(AgentError::ProviderMissing { .. })
    ));
    *rig.launcher.fail_with.lock().unwrap() = None;
    let (outcome, _) = rig.run(AgentOperation::Probe);
    assert!(matches!(
        outcome,
        OperationOutcome::Succeeded(OperationOutput::Probed { .. })
    ));
}

#[test]
fn creating_a_session_passes_the_workspace_and_choices_and_reports_config_and_identity() {
    let mut rig = Rig::new();
    let (outcome, events) = rig.run(AgentOperation::CreateSession {
        session: SessionId(1),
        workspace: "/w".into(),
        config: ConfigPatch {
            values: vec![
                ("model".into(), "gpt-6-lite".into()),
                ("sandbox".into(), "read-only".into()),
            ],
        },
    });
    let start = rig
        .written()
        .into_iter()
        .find(|m| m["method"] == "thread/start")
        .unwrap();
    assert_eq!(start["params"]["cwd"], "/w");
    assert_eq!(start["params"]["ephemeral"], false);
    assert_eq!(
        (
            start["params"]["model"].as_str(),
            start["params"]["sandbox"].as_str()
        ),
        (Some("gpt-6-lite"), Some("read-only"))
    );
    let OperationOutcome::Succeeded(OperationOutput::SessionReady { native, config, .. }) = outcome
    else {
        panic!()
    };
    assert_eq!(
        (native.provider, native.id.as_str()),
        (ProviderId::Codex, THREAD)
    );
    let model = config.get("model").unwrap();
    assert!(
        model.choices.iter().any(|c| c.value == "gpt-6-lite"),
        "models come from model/list"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::SessionCreated))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::ConfigChanged(_)))
    );
    let (bad, _) = rig.run(AgentOperation::CreateSession {
        session: SessionId(2),
        workspace: "/w".into(),
        config: ConfigPatch {
            values: vec![("sandbox".into(), "bogus".into())],
        },
    });
    assert!(matches!(
        bad,
        OperationOutcome::Failed(AgentError::InvalidConfig { .. })
    ));
}

#[test]
fn a_turn_streams_through_the_reducer_and_a_second_start_while_active_is_busy() {
    let mut rig = Rig::new();
    rig.create(1);
    let mut model = SessionModel::default();
    let outcome = rig.start(1, "Reply with exactly the word: ok");
    assert_eq!(
        outcome,
        OperationOutcome::Succeeded(OperationOutput::TurnStarted(TurnId("turn-1".into())))
    );
    let turn_start = rig
        .written()
        .into_iter()
        .find(|m| m["method"] == "turn/start")
        .unwrap();
    assert_eq!(turn_start["params"]["threadId"], THREAD);
    assert_eq!(
        turn_start["params"]["input"][0],
        json!({"type": "text", "text": "Reply with exactly the word: ok"})
    );
    assert_eq!(turn_start["params"]["model"], "gpt-6-astra");
    assert_eq!(
        turn_start["params"]["sandboxPolicy"],
        json!({"type": "workspaceWrite"})
    );
    assert_eq!(turn_start["params"]["approvalPolicy"], "on-request");

    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1", "status": "inProgress"}}),
    );
    rig.notify("item/started", thread_notice(THREAD, json!({"item": {"type": "agentMessage", "id": "m1", "text": "", "phase": "final_answer"}})));
    rig.notify(
        "item/agentMessage/delta",
        thread_notice(THREAD, json!({"itemId": "m1", "delta": "o"})),
    );
    rig.notify(
        "item/agentMessage/delta",
        thread_notice(THREAD, json!({"itemId": "m1", "delta": "k"})),
    );
    rig.notify("item/completed", thread_notice(THREAD, json!({"item": {"type": "agentMessage", "id": "m1", "text": "ok", "phase": "final_answer"}})));
    rig.notify("thread/tokenUsage/updated", thread_notice(THREAD, json!({"tokenUsage": {"total": {"totalTokens": 99, "inputTokens": 90, "outputTokens": 9}}})));
    let events = rig.poll();
    fold(&mut model, &events, 1);
    assert!(matches!(model.state, TurnState::Active { .. }));
    assert_eq!(model.usage.as_ref().unwrap().total_tokens, Some(99));
    let busy = rig.start(1, "again");
    assert_eq!(busy, OperationOutcome::Failed(AgentError::Busy));
    rig.notify(
        "turn/completed",
        json!({"threadId": THREAD, "turn": {"id": "turn-1", "status": "completed"}}),
    );
    let events = rig.poll();
    fold(&mut model, &events, 1);
    assert_eq!(model.last_outcome, Some(TurnOutcome::Completed));
    assert!(
        matches!(&model.items[0].kind, ItemKind::Agent { text, finished: true, .. } if text == "ok")
    );
    assert_eq!(
        rig.start(1, "next"),
        OperationOutcome::Succeeded(OperationOutput::TurnStarted(TurnId("turn-1".into())))
    );
}

#[test]
fn unsupported_input_fails_before_anything_is_sent() {
    let mut rig = Rig::new();
    rig.create(1);
    let (outcome, _) = rig.run(AgentOperation::StartTurn {
        session: SessionId(1),
        client_turn: ClientTurnId("c".into()),
        input: vec![InputBlock::Image {
            media_type: "image/png".into(),
            bytes: vec![1],
        }],
        expected_revision: None,
    });
    assert!(matches!(
        outcome,
        OperationOutcome::Failed(AgentError::Unsupported { .. })
    ));
    assert!(!rig.written().iter().any(|m| m["method"] == "turn/start"));
    let stale = rig.run(AgentOperation::StartTurn {
        session: SessionId(1),
        client_turn: ClientTurnId("c".into()),
        input: vec![InputBlock::Text("x".into())],
        expected_revision: Some(ConfigRevision(99)),
    });
    assert!(matches!(
        stale.0,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
}

#[test]
fn interrupt_targets_the_active_turn_and_a_stale_turn_is_refused() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "slow");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    rig.poll();
    let (stale, _) = rig.run(AgentOperation::InterruptTurn {
        session: SessionId(1),
        turn: TurnId("other".into()),
    });
    assert!(matches!(
        stale,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
    let (ok, _) = rig.run(AgentOperation::InterruptTurn {
        session: SessionId(1),
        turn: TurnId("turn-1".into()),
    });
    assert_eq!(ok, OperationOutcome::Succeeded(OperationOutput::Ack));
    let interrupt = rig
        .written()
        .into_iter()
        .find(|m| m["method"] == "turn/interrupt")
        .unwrap();
    assert_eq!(
        interrupt["params"],
        json!({"threadId": THREAD, "turnId": "turn-1"})
    );
    rig.notify(
        "turn/completed",
        json!({"threadId": THREAD, "turn": {"id": "turn-1", "status": "interrupted"}}),
    );
    let mut model = SessionModel::default();
    let mut events = rig.poll();
    events.insert(
        0,
        EventEnvelope {
            sequence: 0,
            ..events[0].clone()
        },
    );
    fold(&mut model, &events[1..], 1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::TurnFinished(TurnOutcome::Interrupted)))
    );
}

fn approval_params() -> Value {
    json!({"kind": "command", "threadId": THREAD, "turnId": "turn-1", "itemId": "exec-1", "startedAtMs": 1,
           "reason": "Allow creating note.txt in the workspace?", "command": "/bin/zsh -lc \"printf 'hi' > note.txt\"", "cwd": "/w"})
}

fn pending_token(events: &[EventEnvelope]) -> RequestToken {
    events
        .iter()
        .find_map(|e| match &e.event {
            AgentEvent::ApprovalRequested(r) => Some(r.token.clone()),
            AgentEvent::QuestionRequested(r) => Some(r.token.clone()),
            _ => None,
        })
        .expect("a request was raised")
}

#[test]
fn an_approval_round_trip_answers_the_server_request_once_and_goes_stale_after() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "write a file");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    rig.server_request(
        0,
        "item/commandExecution/requestApproval",
        approval_params(),
    );
    let events = rig.poll();
    let token = pending_token(&events);
    assert_eq!(
        (token.provider, token.session, token.native.as_str()),
        (ProviderId::Codex, SessionId(1), "0")
    );
    let mut model = SessionModel::default();
    fold(&mut model, &events, 1);
    assert!(model.needs_attention());
    let (ok, events) = rig.run(AgentOperation::ReplyApproval {
        token: token.clone(),
        decision: ApprovalDecisionId("decline".into()),
    });
    assert_eq!(ok, OperationOutcome::Succeeded(OperationOutput::Ack));
    assert!(
        rig.child()
            .written_text()
            .lines()
            .any(|l| l == "{\"id\":0,\"result\":{\"decision\":\"decline\"}}")
    );
    fold(&mut model, &events, 1);
    assert!(!model.needs_attention());
    let (again, _) = rig.run(AgentOperation::ReplyApproval {
        token,
        decision: ApprovalDecisionId("accept".into()),
    });
    assert!(matches!(
        again,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
}

#[test]
fn unknown_decisions_and_wrong_tokens_are_refused_without_answering_the_server() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    rig.server_request(
        5,
        "item/commandExecution/requestApproval",
        approval_params(),
    );
    let token = pending_token(&rig.poll());
    let (bad, _) = rig.run(AgentOperation::ReplyApproval {
        token: token.clone(),
        decision: ApprovalDecisionId("acceptWithExecpolicyAmendment".into()),
    });
    assert!(matches!(
        bad,
        OperationOutcome::Failed(AgentError::InvalidConfig { .. })
    ));
    let mut forged = token.clone();
    forged.native = "6".into();
    let (stale, _) = rig.run(AgentOperation::ReplyApproval {
        token: forged,
        decision: ApprovalDecisionId("accept".into()),
    });
    assert!(matches!(
        stale,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
    let mut old_generation = token.clone();
    old_generation.generation = lumen_agent_core::RuntimeGeneration(99);
    let (stale, _) = rig.run(AgentOperation::ReplyApproval {
        token: old_generation,
        decision: ApprovalDecisionId("accept".into()),
    });
    assert!(matches!(
        stale,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
    assert!(
        !rig.child().written_text().contains("\"id\":5,\"result\""),
        "nothing was answered"
    );
    let (ok, _) = rig.run(AgentOperation::ReplyApproval {
        token,
        decision: ApprovalDecisionId("accept".into()),
    });
    assert_eq!(
        ok,
        OperationOutcome::Succeeded(OperationOutput::Ack),
        "the genuine request is still answerable"
    );
}

#[test]
fn a_question_is_answered_with_the_native_shape_and_unsupported_requests_are_declined() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    rig.server_request(7, "item/tool/requestUserInput", json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "i", "isBlocking": true,
        "questions": [{"id": "q1", "header": "h", "question": "Which?", "options": [{"label": "a", "description": ""}]}]}));
    let token = pending_token(&rig.poll());
    let (ok, _) = rig.run(AgentOperation::ReplyQuestion {
        token,
        answers: vec![QuestionAnswer::Choice("a".into())],
    });
    assert_eq!(ok, OperationOutcome::Succeeded(OperationOutput::Ack));
    assert!(
        rig.child()
            .written_text()
            .lines()
            .any(|l| l == "{\"id\":7,\"result\":{\"answers\":{\"q1\":{\"answers\":[\"a\"]}}}}")
    );
    // Several questions at once, and a request type we do not handle: declined with feedback.
    rig.server_request(8, "item/tool/requestUserInput", json!({"threadId": THREAD, "turnId": "turn-1", "itemId": "i", "isBlocking": true,
        "questions": [{"id": "1", "header": "", "question": "a"}, {"id": "2", "header": "", "question": "b"}]}));
    rig.server_request(
        9,
        "item/permissions/requestApproval",
        json!({"threadId": THREAD, "turnId": "turn-1"}),
    );
    let events = rig.poll();
    let written = rig.written();
    let errored = |id: u64| {
        written
            .iter()
            .any(|m| m["id"] == id && m.get("error").is_some())
    };
    assert!(errored(8) && errored(9), "{written:?}");
    let warnings = events
        .iter()
        .filter(|e| matches!(e.event, AgentEvent::Diagnostic { .. }))
        .count();
    assert_eq!(warnings, 2);
    assert!(!events.iter().any(|e| matches!(
        e.event,
        AgentEvent::ApprovalRequested(_) | AgentEvent::QuestionRequested(_)
    )));
}

#[test]
fn two_sessions_share_one_process_and_events_are_routed_by_thread() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.create(2);
    assert_eq!(
        rig.launcher.children.lock().unwrap().len(),
        1,
        "one app-server serves both sessions"
    );
    let second_thread = "thread-1";
    rig.notify(
        "turn/started",
        json!({"threadId": second_thread, "turn": {"id": "turn-2"}}),
    );
    rig.notify("item/started", json!({"threadId": second_thread, "turnId": "turn-2", "item": {"type": "agentMessage", "id": "b1", "phase": "final_answer"}}));
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": second_thread, "turnId": "turn-2", "itemId": "b1", "delta": "beta"}),
    );
    rig.notify(
        "item/agentMessage/delta",
        json!({"threadId": "unknown-thread", "turnId": "x", "itemId": "z", "delta": "lost"}),
    );
    let events = rig.poll();
    assert!(
        events
            .iter()
            .filter(|e| e.session.is_some())
            .all(|e| e.session == Some(SessionId(2)))
    );
    let (mut one, mut two) = (SessionModel::default(), SessionModel::default());
    fold(&mut one, &events, 1);
    fold(&mut two, &events, 2);
    assert!(
        one.items.is_empty()
            && matches!(&two.items[0].kind, ItemKind::Agent { text, .. } if text == "beta")
    );
}

#[test]
fn history_listing_rename_archive_attach_and_detach_use_the_native_calls() {
    let mut rig = Rig::new();
    let (listed, _) = rig.run(AgentOperation::ListSessions {
        workspace: Some("/w".into()),
        cursor: None,
        limit: 10,
    });
    let OperationOutcome::Succeeded(OperationOutput::Sessions {
        summaries,
        complete,
        ..
    }) = listed
    else {
        panic!()
    };
    assert!(
        complete
            && summaries.len() == 1
            && summaries[0].title.as_deref() == Some("Remember tangerine")
    );
    let list_call = rig
        .written()
        .into_iter()
        .find(|m| m["method"] == "thread/list")
        .unwrap();
    assert_eq!(
        (
            list_call["params"]["cwd"].as_str(),
            list_call["params"]["archived"].as_bool()
        ),
        (Some("/w"), Some(false))
    );
    let native = summaries[0].native.clone();
    let (attached, events) = rig.run(AgentOperation::AttachSession {
        session: SessionId(4),
        native,
        workspace: "/w".into(),
    });
    assert!(matches!(
        attached,
        OperationOutcome::Succeeded(OperationOutput::SessionReady { .. })
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::SessionAttached))
    );
    let (history, _) = rig.run(AgentOperation::ReadHistory {
        session: SessionId(4),
        cursor: None,
        limit: 50,
    });
    let OperationOutcome::Succeeded(OperationOutput::History { items, .. }) = history else {
        panic!()
    };
    assert_eq!(items.len(), 2, "reasoning is not part of the transcript");
    assert!(
        matches!(&items[0].kind, ItemKind::User(t) if t == "hi")
            && matches!(&items[1].kind, ItemKind::Agent { text, .. } if text == "hello")
    );
    let (renamed, events) = rig.run(AgentOperation::UpdateSessionMetadata {
        session: SessionId(4),
        title: "Fix it".into(),
    });
    assert_eq!(renamed, OperationOutcome::Succeeded(OperationOutput::Ack));
    assert!(
        events.iter().any(
            |e| matches!(&e.event, AgentEvent::MetadataChanged { title } if title == "Fix it")
        )
    );
    let (detached, _) = rig.run(AgentOperation::DetachSession {
        session: SessionId(4),
    });
    assert_eq!(detached, OperationOutcome::Succeeded(OperationOutput::Ack));
    assert_eq!(
        rig.start(4, "x"),
        OperationOutcome::Failed(AgentError::Stale {
            what: "this session after the agent restarted; attach it again".into()
        })
    );
    let (archived, _) = rig.run(AgentOperation::ArchiveSession {
        session: SessionId(4),
    });
    assert_eq!(archived, OperationOutcome::Succeeded(OperationOutput::Ack));
    let foreign = lumen_agent_core::NativeSessionRef {
        provider: ProviderId::Claude,
        profile: lumen_agent_core::ProfileId::default_profile(),
        id: "x".into(),
    };
    let (bad, _) = rig.run(AgentOperation::AttachSession {
        session: SessionId(9),
        native: foreign,
        workspace: "/w".into(),
    });
    assert!(matches!(
        bad,
        OperationOutcome::Failed(AgentError::NotFound { .. })
    ));
}

#[test]
fn busy_sessions_cannot_be_archived_or_detached() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    rig.poll();
    assert_eq!(
        rig.run(AgentOperation::ArchiveSession {
            session: SessionId(1)
        })
        .0,
        OperationOutcome::Failed(AgentError::Busy)
    );
    assert_eq!(
        rig.run(AgentOperation::DetachSession {
            session: SessionId(1)
        })
        .0,
        OperationOutcome::Failed(AgentError::Busy)
    );
}

#[test]
fn config_changes_apply_to_the_next_turn_and_check_revision_and_values() {
    let mut rig = Rig::new();
    rig.create(1);
    let set = |rig: &mut Rig, rev: u64, id: &str, value: &str| {
        rig.run(AgentOperation::SetConfig {
            session: SessionId(1),
            expected_revision: ConfigRevision(rev),
            patch: ConfigPatch {
                values: vec![(id.into(), value.into())],
            },
        })
        .0
    };
    assert!(
        matches!(set(&mut rig, 1, "model", "gpt-6-lite"), OperationOutcome::Succeeded(OperationOutput::Config(c)) if c.revision == ConfigRevision(2))
    );
    assert!(matches!(
        set(&mut rig, 1, "model", "gpt-6-astra"),
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
    assert!(matches!(
        set(&mut rig, 2, "sandbox", "nope"),
        OperationOutcome::Failed(AgentError::InvalidConfig { .. })
    ));
    assert!(matches!(
        set(&mut rig, 2, "sandbox", "read-only"),
        OperationOutcome::Succeeded(_)
    ));
    rig.start(1, "go");
    let turn = rig
        .written()
        .into_iter()
        .rfind(|m| m["method"] == "turn/start")
        .unwrap();
    assert_eq!(turn["params"]["model"], "gpt-6-lite");
    assert_eq!(turn["params"]["sandboxPolicy"], json!({"type": "readOnly"}));
}

#[test]
fn a_crash_mid_turn_leaves_the_outcome_unknown_fails_in_flight_work_and_relaunches_on_demand() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    let mut model = SessionModel::default();
    let events = rig.poll();
    fold(&mut model, &events, 1);
    let first_generation = events[0].generation;
    // A request is in flight (the server never answers) when the process dies.
    let responder_off = rig.child();
    responder_off.state.lock().unwrap().responder = None;
    let rename = rig.submit(AgentOperation::UpdateSessionMetadata {
        session: SessionId(1),
        title: "t".into(),
    });
    rig.poll();
    {
        let handle = rig.child();
        let mut state = handle.state.lock().unwrap();
        state.exit = Some(lumen_agent_core::launcher::ExitInfo {
            code: Some(1),
            signal: None,
        });
        state
            .queued
            .push_back(lumen_agent_core::launcher::ChildEvent::Stderr(
                b"boom".to_vec(),
            ));
    }
    let events = rig.poll();
    fold(&mut model, &events, 1);
    assert_eq!(model.state, TurnState::Unknown);
    assert!(
        matches!(
            contract::outcome_of(&events, rename),
            Some(OperationOutcome::Failed(AgentError::Ambiguous { .. }))
        ),
        "never resent automatically"
    );
    assert!(events.iter().any(
        |e| matches!(&e.event, AgentEvent::Diagnostic { message, .. } if message.contains("boom"))
    ));
    // The next operation relaunches under a new generation; the old session must be attached again.
    let (outcome, events) = rig.run(AgentOperation::Probe);
    assert!(matches!(outcome, OperationOutcome::Succeeded(_)));
    assert!(events.iter().all(|e| e.generation > first_generation));
    assert_eq!(rig.launcher.children.lock().unwrap().len(), 2);
    assert!(matches!(
        rig.start(1, "again"),
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
}

#[test]
fn shutdown_declines_requests_interrupts_turns_stops_the_child_and_ends_once() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.notify(
        "turn/started",
        json!({"threadId": THREAD, "turn": {"id": "turn-1"}}),
    );
    rig.server_request(
        3,
        "item/commandExecution/requestApproval",
        approval_params(),
    );
    let mut model = SessionModel::default();
    let events = rig.poll();
    fold(&mut model, &events, 1);
    assert!(model.needs_attention());
    let queued = rig.submit(AgentOperation::Probe);
    let shutdown = rig.backend.begin_shutdown(Duration::from_secs(5)).unwrap();
    assert_eq!(
        rig.backend.try_submit(Request {
            client_request_id: ClientRequestId(900),
            operation: AgentOperation::Probe
        }),
        Err(SubmitError::ShuttingDown)
    );
    assert_eq!(
        rig.backend.begin_shutdown(Duration::from_secs(1)),
        Err(SubmitError::ShuttingDown)
    );
    let events = rig.poll();
    fold(&mut model, &events, 1);
    assert!(!model.needs_attention(), "the pending approval was settled");
    assert_eq!(
        contract::outcome_of(&events, queued),
        Some(&OperationOutcome::Cancelled)
    );
    let written = rig.child().written_text();
    assert!(
        written.contains("\"id\":3,\"result\":{\"decision\":\"cancel\"}"),
        "{written}"
    );
    assert!(written.contains("turn/interrupt"));
    let handle = rig.child();
    let state = handle.state.lock().unwrap();
    assert!(state.stdin_closed && state.stop_requests >= 1);
    drop(state);
    let events = contract::pump(&mut rig.backend, |e| {
        contract::outcome_of(e, shutdown).is_some()
    });
    assert_eq!(
        contract::outcome_of(&events, shutdown),
        Some(&OperationOutcome::Succeeded(OperationOutput::Ack))
    );
}

#[test]
fn malformed_frames_are_counted_not_fatal() {
    let mut rig = Rig::new();
    rig.create(1);
    let handle = rig.child();
    handle.stdout_line("this is not json");
    handle.stdout_line("{}");
    handle.stdout_line(&json!({"id": 4242, "result": {}}).to_string());
    rig.poll();
    assert_eq!(rig.backend.ignored_frames, 3);
    assert!(matches!(
        rig.run(AgentOperation::Probe).0,
        OperationOutcome::Succeeded(_)
    ));
}

#[test]
fn the_shared_contract_passes_with_the_codex_backend() {
    let backend = || {
        let launcher = FakeLauncher::default();
        *launcher.responder.lock().unwrap() = Some(responder());
        CodexBackend::new(
            Arc::new(launcher),
            Arc::new(|| {}),
            CodexConfig::new("codex"),
        )
    };
    contract::basic(&mut backend());
    contract::unknown_targets_fail_typed(&mut backend(), ProviderId::Codex);
    contract::shutdown_is_final(&mut backend());
}
