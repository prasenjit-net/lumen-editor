//! Backend tests against a scripted `claude` child (no real process). Message
//! shapes follow the Phase 1 transcripts.

use std::sync::{Arc, atomic::AtomicU64};

use lumen_agent_core::{
    AgentError, ClientRequestId, EventEnvelope, NativeSessionRef, OperationId, PollBudget,
    ProfileId, ProviderId, RuntimeGeneration, SessionId, TurnId,
    api::{
        AgentBackend, AgentOperation, ApprovalDecisionId, AuthState, ClientTurnId, ConfigPatch,
        ConfigRevision, InputBlock, OperationOutcome, OperationOutput, Request,
    },
    event::{AgentEvent, TurnOutcome},
    launcher::{Child, ChildEvent, ChildLauncher, ChildSpec, LaunchError, Wake},
    model::{ItemKind, SessionModel, ToolStatus, TurnState},
    testing::{FakeChildHandle, FakeLauncher, contract},
};
use serde_json::{Value, json};

use crate::{ClaudeBackend, ClaudeConfig};

const NATIVE: &str = "11111111-2222-4333-8444-555555555555";

/// A fake launcher whose one-shot commands (`--version`, `auth status`) answer
/// and exit immediately, like the real CLI.
#[derive(Clone, Default)]
struct Scripted(FakeLauncher);

impl ChildLauncher for Scripted {
    fn launch(&self, spec: &ChildSpec, wake: Wake) -> Result<Box<dyn Child>, LaunchError> {
        let child = self.0.launch(spec, wake)?;
        let count = self.0.children.lock().unwrap().len();
        let handle = self.0.child(count - 1);
        let first = spec
            .args
            .first()
            .and_then(|a| a.to_str())
            .unwrap_or_default();
        match first {
            "--version" => {
                handle
                    .state
                    .lock()
                    .unwrap()
                    .queued
                    .push_back(ChildEvent::Stdout(b"2.1.212 (Claude Code)\n".to_vec()));
                handle.exit(0);
            }
            "auth" => {
                let status =
                    json!({"loggedIn": true, "authMethod": "claude.ai", "email": "me@example.com"});
                handle.stdout_line(&status.to_string());
                handle.exit(0);
            }
            _ => {}
        }
        Ok(child)
    }
}

struct Rig {
    backend: ClaudeBackend,
    launcher: Scripted,
    next: u64,
    /// Every event seen so far, so tests can rebuild a session model.
    seen: Vec<EventEnvelope>,
}

impl Rig {
    fn new() -> Self {
        let launcher = Scripted::default();
        let mut config = ClaudeConfig::new("claude");
        config.new_session_id = Arc::new(|| NATIVE.to_string());
        let backend = ClaudeBackend::new(Arc::new(launcher.clone()), Arc::new(|| {}), config);
        Self {
            backend,
            launcher,
            next: 1,
            seen: Vec::new(),
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
        self.seen.extend(events.clone());
        (contract::outcome_of(&events, op).cloned().unwrap(), events)
    }

    fn model(&self, session: u64) -> SessionModel {
        let mut model = SessionModel::default();
        for event in &self.seen {
            if event.session == Some(SessionId(session)) {
                model.apply(event);
            }
        }
        model
    }

    fn launches(&self) -> Vec<ChildSpec> {
        self.launcher.0.launched.lock().unwrap().clone()
    }

    fn child(&self) -> FakeChildHandle {
        let count = self.launcher.0.children.lock().unwrap().len();
        self.launcher.0.child(count - 1)
    }

    fn say(&self, message: Value) {
        self.child().stdout_line(&message.to_string());
    }

    fn poll(&mut self) -> Vec<EventEnvelope> {
        let events = self.backend.poll_events(PollBudget::FRAME);
        self.seen.extend(events.clone());
        events
    }

    fn written(&self) -> Vec<Value> {
        self.child()
            .written_text()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn create(&mut self, session: u64) -> OperationOutcome {
        self.run(AgentOperation::CreateSession {
            session: SessionId(session),
            workspace: "/w".into(),
            config: ConfigPatch::default(),
        })
        .0
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

    fn approval_token(&mut self) -> lumen_agent_core::RequestToken {
        self.poll()
            .iter()
            .find_map(|e| match &e.event {
                AgentEvent::ApprovalRequested(r) => Some(r.token.clone()),
                _ => None,
            })
            .expect("approval requested")
    }
}

fn stream(event: Value) -> Value {
    json!({"type": "stream_event", "event": event})
}

fn args(spec: &ChildSpec) -> Vec<String> {
    spec.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn bash_request(id: &str) -> Value {
    json!({"type": "control_request", "request_id": id, "request": {
        "subtype": "can_use_tool", "tool_name": "Bash", "tool_use_id": "t", "input": {"command": "ls"}}})
}

#[test]
fn probe_runs_version_then_auth_status_and_hides_the_email() {
    let mut rig = Rig::new();
    let (outcome, events) = rig.run(AgentOperation::Probe);
    let OperationOutcome::Succeeded(OperationOutput::Probed {
        version,
        auth,
        capabilities,
    }) = outcome
    else {
        panic!("{outcome:?}")
    };
    assert_eq!(version, "2.1.212");
    assert_eq!(
        auth,
        AuthState::SignedIn {
            label: "Claude account".into()
        }
    );
    assert!(capabilities.interrupt.is_supported());
    assert!(!format!("{events:?}").contains("me@example.com"));
    assert_eq!(
        rig.backend.descriptor().executable_version.as_deref(),
        Some("2.1.212")
    );
    let launches = rig.launches();
    assert_eq!(args(&launches[0]), ["--version"]);
    assert_eq!(args(&launches[1]), ["auth", "status"]);
}

#[test]
fn a_missing_executable_fails_probe_with_a_typed_error() {
    let mut rig = Rig::new();
    *rig.launcher.0.fail_with.lock().unwrap() = Some(LaunchError::NotFound("claude".into()));
    let (outcome, _) = rig.run(AgentOperation::Probe);
    assert!(matches!(
        outcome,
        OperationOutcome::Failed(AgentError::ProviderMissing { .. })
    ));
}

#[test]
fn creating_a_session_launches_nothing_until_the_first_turn() {
    let mut rig = Rig::new();
    let outcome = rig.create(1);
    let OperationOutcome::Succeeded(OperationOutput::SessionReady { native, .. }) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(
        (native.provider, native.id.as_str()),
        (ProviderId::Claude, NATIVE)
    );
    assert!(rig.launches().is_empty());
    assert_eq!(
        rig.start(1, "hello"),
        OperationOutcome::Succeeded(OperationOutput::TurnStarted(TurnId("turn-1".into())))
    );
    let launches = rig.launches();
    let a = args(&launches[0]);
    for flag in ["-p", "--include-partial-messages", "--verbose"] {
        assert!(a.contains(&flag.to_string()), "{a:?}");
    }
    let at = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
    assert_eq!(at("--input-format").as_deref(), Some("stream-json"));
    assert_eq!(at("--permission-prompt-tool").as_deref(), Some("stdio"));
    assert_eq!(at("--session-id").as_deref(), Some(NATIVE));
    assert!(!a.contains(&"--resume".to_string()));
    assert_eq!(launches[0].cwd, std::path::PathBuf::from("/w"));
    let sent = &rig.written()[0];
    assert_eq!(sent["type"], "user");
    assert_eq!(
        sent["message"]["content"][0],
        json!({"type": "text", "text": "hello"})
    );
}

#[test]
fn a_turn_streams_through_the_reducer_and_ends_at_result() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "Reply with ok");
    rig.say(json!({"type": "system", "subtype": "init", "session_id": NATIVE}));
    rig.say(stream(
        json!({"type": "message_start", "message": {"id": "msg_1"}}),
    ));
    rig.say(stream(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}})));
    rig.say(stream(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "o"}})));
    rig.say(stream(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "k"}})));
    rig.say(stream(json!({"type": "content_block_stop", "index": 0})));
    // The whole message repeats the streamed text; it must not be added twice.
    rig.say(json!({"type": "assistant", "message": {"id": "msg_1", "content": [{"type": "text", "text": "ok"}]}}));
    rig.poll();
    let model = rig.model(1);
    assert!(matches!(model.state, TurnState::Active { .. }));
    assert_eq!(
        rig.start(1, "again"),
        OperationOutcome::Failed(AgentError::Busy)
    );
    rig.say(
        json!({"type": "result", "subtype": "success", "is_error": false, "result": "ok",
        "usage": {"input_tokens": 10, "output_tokens": 2}}),
    );
    rig.poll();
    let model = rig.model(1);
    assert_eq!(model.last_outcome, Some(TurnOutcome::Completed));
    assert_eq!(model.usage.as_ref().unwrap().total_tokens, Some(12));
    let texts: Vec<_> = model
        .items
        .iter()
        .filter_map(|i| match &i.kind {
            ItemKind::Agent {
                text,
                finished: true,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["ok"]);
    assert_eq!(
        rig.start(1, "next"),
        OperationOutcome::Succeeded(OperationOutput::TurnStarted(TurnId("turn-2".into())))
    );
    assert_eq!(
        rig.launches().len(),
        1,
        "the process is reused between turns"
    );
}

#[test]
fn an_unstreamed_assistant_message_still_appears() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "hi");
    rig.say(json!({"type": "assistant", "message": {"id": "msg_9", "content": [{"type": "text", "text": "whole"}]}}));
    rig.poll();
    let model = rig.model(1);
    assert!(
        model
            .items
            .iter()
            .any(|i| matches!(&i.kind, ItemKind::Agent { text, .. } if text == "whole"))
    );
}

#[test]
fn an_approval_round_trips_and_a_stale_token_is_rejected() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "write a file");
    rig.say(json!({"type": "assistant", "message": {"id": "m", "content": [
        {"type": "tool_use", "id": "toolu_1", "name": "Write", "input": {"file_path": "/w/a.txt", "content": "x"}}]}}));
    rig.say(
        json!({"type": "control_request", "request_id": "req-1", "request": {
        "subtype": "can_use_tool", "tool_name": "Write", "tool_use_id": "toolu_1",
        "input": {"file_path": "/w/a.txt", "content": "x"}, "description": "Write a.txt"}}),
    );
    let events = rig.poll();
    let request = events
        .iter()
        .find_map(|e| match &e.event {
            AgentEvent::ApprovalRequested(r) => Some(r.clone()),
            _ => None,
        })
        .expect("approval requested");
    assert!(request.choices.iter().any(|c| c.allows) && request.choices.iter().any(|c| !c.allows));
    let mut stale = request.token.clone();
    stale.generation = RuntimeGeneration(stale.generation.0 + 1);
    let (outcome, _) = rig.run(AgentOperation::ReplyApproval {
        token: stale,
        decision: ApprovalDecisionId("allow".into()),
    });
    assert!(matches!(
        outcome,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
    let (outcome, events) = rig.run(AgentOperation::ReplyApproval {
        token: request.token.clone(),
        decision: ApprovalDecisionId("deny".into()),
    });
    assert_eq!(outcome, OperationOutcome::Succeeded(OperationOutput::Ack));
    assert!(events.iter().any(
        |e| matches!(&e.event, AgentEvent::ApprovalResolved { token } if *token == request.token)
    ));
    let reply = rig
        .written()
        .into_iter()
        .find(|m| m["type"] == "control_response")
        .unwrap();
    assert_eq!(reply["response"]["request_id"], "req-1");
    assert_eq!(reply["response"]["response"]["behavior"], "deny");
    // A second reply to the same request is stale.
    let (again, _) = rig.run(AgentOperation::ReplyApproval {
        token: request.token,
        decision: ApprovalDecisionId("allow".into()),
    });
    assert!(matches!(
        again,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
    // The denied tool is shown as declined.
    rig.say(json!({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "toolu_1", "is_error": true, "content": "denied"}]}}));
    rig.poll();
    let model = rig.model(1);
    assert!(
        model.items.iter().any(
            |i| matches!(&i.kind, ItemKind::Tool(call) if call.status == ToolStatus::Declined)
        )
    );
}

#[test]
fn allow_returns_the_original_input() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.say(bash_request("r"));
    let token = rig.approval_token();
    let (outcome, _) = rig.run(AgentOperation::ReplyApproval {
        token,
        decision: ApprovalDecisionId("allow".into()),
    });
    assert_eq!(outcome, OperationOutcome::Succeeded(OperationOutput::Ack));
    let reply = rig
        .written()
        .into_iter()
        .find(|m| m["type"] == "control_response")
        .unwrap();
    assert_eq!(reply["response"]["response"]["behavior"], "allow");
    assert_eq!(
        reply["response"]["response"]["updatedInput"],
        json!({"command": "ls"})
    );
}

#[test]
fn questions_are_declined_at_the_source_and_unknown_controls_are_refused() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.say(
        json!({"type": "control_request", "request_id": "q", "request": {
        "subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": {"questions": []}}}),
    );
    rig.say(
        json!({"type": "control_request", "request_id": "z", "request": {"subtype": "hook_callback"}}),
    );
    let events = rig.poll();
    assert!(!events.iter().any(|e| matches!(
        e.event,
        AgentEvent::ApprovalRequested(_) | AgentEvent::QuestionRequested(_)
    )));
    let written = rig.written();
    let q = written
        .iter()
        .find(|m| m["response"]["request_id"] == "q")
        .unwrap();
    assert_eq!(q["response"]["response"]["behavior"], "deny");
    let z = written
        .iter()
        .find(|m| m["response"]["request_id"] == "z")
        .unwrap();
    assert_eq!(z["response"]["subtype"], "error");
}

#[test]
fn interrupt_sends_a_control_request_and_the_result_is_interrupted() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "long task");
    let op = rig.submit(AgentOperation::InterruptTurn {
        session: SessionId(1),
        turn: TurnId("turn-1".into()),
    });
    rig.poll();
    let sent = rig
        .written()
        .into_iter()
        .find(|m| m["request"]["subtype"] == "interrupt")
        .expect("interrupt written");
    let request_id = sent["request_id"].as_str().unwrap().to_string();
    // The operation ends when claude acknowledges it, then the turn ends.
    rig.say(json!({"type": "control_response", "response": {
        "subtype": "success", "request_id": request_id, "response": {"still_queued": []}}}));
    rig.say(json!({"type": "result", "subtype": "error_during_execution", "is_error": true}));
    let events = rig.poll();
    let model = rig.model(1);
    assert_eq!(
        contract::outcome_of(&events, op),
        Some(&OperationOutcome::Succeeded(OperationOutput::Ack))
    );
    assert_eq!(model.last_outcome, Some(TurnOutcome::Interrupted));
}

#[test]
fn interrupting_a_stale_turn_fails_typed() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    let (outcome, _) = rig.run(AgentOperation::InterruptTurn {
        session: SessionId(1),
        turn: TurnId("turn-9".into()),
    });
    assert!(matches!(
        outcome,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
}

#[test]
fn a_crash_mid_turn_fails_the_turn_and_the_next_turn_resumes() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.say(bash_request("r"));
    rig.poll();
    rig.child().exit(1);
    let events = rig.poll();
    let model = rig.model(1);
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::ApprovalResolved { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::RuntimeExited { .. }))
    );
    assert!(matches!(
        model.last_outcome,
        Some(TurnOutcome::Failed(AgentError::Ambiguous { .. }))
    ));
    assert!(matches!(
        rig.start(1, "again"),
        OperationOutcome::Succeeded(_)
    ));
    let launches = rig.launches();
    assert_eq!(launches.len(), 2);
    let a = args(&launches[1]);
    assert!(
        a.contains(&"--resume".to_string()) && !a.contains(&"--session-id".to_string()),
        "{a:?}"
    );
}

#[test]
fn changing_the_model_while_idle_restarts_the_process_with_resume() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.say(json!({"type": "result", "subtype": "success", "is_error": false}));
    rig.poll();
    let old = rig.child();
    let (outcome, _) = rig.run(AgentOperation::SetConfig {
        session: SessionId(1),
        expected_revision: ConfigRevision(1),
        patch: ConfigPatch {
            values: vec![("model".into(), "opus".into())],
        },
    });
    assert!(
        matches!(
            outcome,
            OperationOutcome::Succeeded(OperationOutput::Config(_))
        ),
        "{outcome:?}"
    );
    assert!(matches!(rig.start(1, "y"), OperationOutcome::Succeeded(_)));
    assert!(old.state.lock().unwrap().stop_requests >= 1);
    let launches = rig.launches();
    let a = args(&launches[1]);
    assert!(a.windows(2).any(|w| w == ["--model", "opus"]), "{a:?}");
    assert!(a.contains(&"--resume".to_string()));
}

#[test]
fn invalid_config_and_stale_revisions_are_rejected() {
    let mut rig = Rig::new();
    rig.create(1);
    let (bad, _) = rig.run(AgentOperation::SetConfig {
        session: SessionId(1),
        expected_revision: ConfigRevision(1),
        patch: ConfigPatch {
            values: vec![("model".into(), "gpt-9".into())],
        },
    });
    assert!(matches!(
        bad,
        OperationOutcome::Failed(AgentError::InvalidConfig { .. })
    ));
    let (stale, _) = rig.run(AgentOperation::SetConfig {
        session: SessionId(1),
        expected_revision: ConfigRevision(7),
        patch: ConfigPatch::default(),
    });
    assert!(matches!(
        stale,
        OperationOutcome::Failed(AgentError::Stale { .. })
    ));
}

#[test]
fn unsupported_input_fails_before_anything_is_launched() {
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
    assert!(rig.launches().is_empty());
}

#[test]
fn attaching_resumes_and_detach_stops_only_idle_sessions() {
    let mut rig = Rig::new();
    let native = NativeSessionRef {
        provider: ProviderId::Claude,
        profile: ProfileId::default_profile(),
        id: "abc".into(),
    };
    let (outcome, _) = rig.run(AgentOperation::AttachSession {
        session: SessionId(2),
        native,
        workspace: "/w".into(),
    });
    assert!(matches!(
        outcome,
        OperationOutcome::Succeeded(OperationOutput::SessionReady { .. })
    ));
    rig.start(2, "hi");
    let a = args(&rig.launches()[0]);
    assert!(a.windows(2).any(|w| w == ["--resume", "abc"]), "{a:?}");
    let (busy, _) = rig.run(AgentOperation::DetachSession {
        session: SessionId(2),
    });
    assert_eq!(busy, OperationOutcome::Failed(AgentError::Busy));
    rig.say(json!({"type": "result", "subtype": "success", "is_error": false}));
    rig.poll();
    let handle = rig.child();
    let (done, _) = rig.run(AgentOperation::DetachSession {
        session: SessionId(2),
    });
    assert_eq!(done, OperationOutcome::Succeeded(OperationOutput::Ack));
    assert!(handle.state.lock().unwrap().stdin_closed);
}

#[test]
fn two_sessions_use_two_processes() {
    let launcher = Scripted::default();
    let mut config = ClaudeConfig::new("claude");
    let counter = Arc::new(AtomicU64::new(0));
    config.new_session_id = Arc::new(move || {
        format!(
            "native-{}",
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        )
    });
    let mut backend = ClaudeBackend::new(Arc::new(launcher.clone()), Arc::new(|| {}), config);
    let mut id = 0;
    let mut run = |backend: &mut ClaudeBackend, operation| {
        id += 1;
        let op = backend
            .try_submit(contract::request(id, operation))
            .unwrap();
        contract::pump(backend, |e| contract::outcome_of(e, op).is_some());
    };
    for session in [1, 2] {
        run(
            &mut backend,
            AgentOperation::CreateSession {
                session: SessionId(session),
                workspace: "/w".into(),
                config: ConfigPatch::default(),
            },
        );
    }
    for session in [1, 2] {
        run(
            &mut backend,
            AgentOperation::StartTurn {
                session: SessionId(session),
                client_turn: ClientTurnId("c".into()),
                input: vec![InputBlock::Text("x".into())],
                expected_revision: None,
            },
        );
    }
    assert_eq!(launcher.0.launched.lock().unwrap().len(), 2);
    launcher.0.child(1).stdout_line(
        &json!({"type": "result", "subtype": "success", "is_error": false}).to_string(),
    );
    let events = backend.poll_events(PollBudget::FRAME);
    let finished: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.event, AgentEvent::TurnFinished(_)))
        .collect();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].session, Some(SessionId(2)));
    assert_ne!(finished[0].generation, RuntimeGeneration(0));
}

#[test]
fn unsupported_operations_fail_with_a_reason() {
    let mut rig = Rig::new();
    for operation in [
        AgentOperation::ListSessions {
            workspace: None,
            cursor: None,
            limit: 5,
        },
        AgentOperation::BeginAuthentication { method: "x".into() },
    ] {
        let (outcome, _) = rig.run(operation);
        assert!(matches!(
            outcome,
            OperationOutcome::Failed(AgentError::Unsupported { .. })
        ));
    }
}

#[test]
fn malformed_frames_are_counted_not_fatal() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.child().stdout_line("this is not json");
    rig.child().stdout_line("[1]");
    rig.poll();
    assert_eq!(rig.backend.ignored_frames, 2);
}

#[test]
fn shutdown_denies_waiting_requests_stops_processes_and_finishes_once() {
    let mut rig = Rig::new();
    rig.create(1);
    rig.start(1, "x");
    rig.say(bash_request("r"));
    rig.poll();
    let queued = rig.submit(AgentOperation::Probe);
    let shutdown = rig
        .backend
        .begin_shutdown(std::time::Duration::from_secs(1))
        .unwrap();
    let events = contract::pump(&mut rig.backend, |e| {
        contract::outcome_of(e, shutdown).is_some()
    });
    assert_eq!(
        contract::outcome_of(&events, queued),
        Some(&OperationOutcome::Cancelled)
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::ApprovalResolved { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.event, AgentEvent::TurnFinished(TurnOutcome::Interrupted)))
    );
    let written = rig.child().written_text();
    assert!(written.contains("\"behavior\":\"deny\""), "{written}");
    let handle = rig.child();
    let state = handle.state.lock().unwrap();
    assert!(state.stdin_closed && state.stop_requests >= 1);
}

#[test]
fn the_shared_contract_passes_with_the_claude_backend() {
    let backend = || {
        ClaudeBackend::new(
            Arc::new(Scripted::default()),
            Arc::new(|| {}),
            ClaudeConfig::new("claude"),
        )
    };
    contract::basic(&mut backend());
    contract::unknown_targets_fail_typed(&mut backend(), ProviderId::Claude);
    contract::shutdown_is_final(&mut backend());
}
