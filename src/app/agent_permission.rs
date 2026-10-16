//! Permission requests from agents: guarding, queueing and answering.
//!
//! An agent that wants to edit or run something asks first
//! (`session/request_permission`) and waits. Lumen shows the request to the user
//! as a card, except where a rule decides immediately: unsaved changes in an open
//! buffer are never put at risk, and (optionally) plain reads inside the folder
//! are allowed.
use super::{LumenApp, agent::AgentKind};
use crate::model::output_log::LogLevel;
use lumen_agent_protocol::{
    requests,
    rpc::RequestId,
    types::{
        PermissionKind, PermissionOption, PermissionRequest, ToolContent, ToolKind,
        parse_permission_request,
    },
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A request waiting for the user.
#[derive(Clone, Debug)]
pub(crate) struct PendingPermission {
    pub(crate) id: RequestId,
    pub(crate) request: PermissionRequest,
    /// Shown above the options (outside the folder, ...).
    pub(crate) warnings: Vec<String>,
}

/// What Lumen decides before bothering the user.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Ask,
    Allow,
    Reject(String),
}

/// Paths a request says it will touch (locations and diffs).
pub(crate) fn request_paths(request: &PermissionRequest) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = request
        .tool_call
        .locations
        .iter()
        .map(|location| PathBuf::from(&location.path))
        .collect();
    for content in &request.tool_call.content {
        if let ToolContent::Diff { path, .. } = content {
            let path = PathBuf::from(path);
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

pub(crate) fn find_option(
    options: &[PermissionOption],
    kinds: &[PermissionKind],
) -> Option<PermissionOption> {
    kinds
        .iter()
        .find_map(|kind| options.iter().find(|option| option.kind == *kind).cloned())
}

impl LumenApp {
    fn root_for_permissions(&self, kind: AgentKind) -> Option<PathBuf> {
        self.editor.workspace.clone().or_else(|| {
            self.view
                .agents
                .session(kind)
                .cwd
                .clone()
                .map(PathBuf::from)
        })
    }

    /// Decide what to do with a request without involving the user, if a rule applies.
    pub(crate) fn permission_verdict(
        &self,
        kind: AgentKind,
        request: &PermissionRequest,
    ) -> Verdict {
        let paths = request_paths(request);
        let modifies = matches!(
            request.tool_call.kind,
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move
        ) || request
            .tool_call
            .content
            .iter()
            .any(|content| matches!(content, ToolContent::Diff { .. }));
        if modifies {
            // Agents write to disk themselves; protect text that only exists in a buffer.
            let dirty: Vec<String> = self
                .editor
                .files
                .iter()
                .filter(|document| document.dirty)
                .filter(|document| {
                    document
                        .path
                        .as_deref()
                        .is_some_and(|path| paths.iter().any(|candidate| candidate == path))
                })
                .map(|document| document.name.clone())
                .collect();
            if !dirty.is_empty() {
                return Verdict::Reject(format!(
                    "{} has unsaved changes in Lumen. Save or discard them, then ask again.",
                    dirty.join(", ")
                ));
            }
        }
        let read_only = matches!(
            request.tool_call.kind,
            ToolKind::Read | ToolKind::Search | ToolKind::Think
        );
        if read_only
            && self.view.settings.boolean("agent.allow_read_in_workspace")
            && let Some(root) = self.root_for_permissions(kind)
            && paths.iter().all(|path| path.starts_with(&root))
        {
            return Verdict::Allow;
        }
        Verdict::Ask
    }

    fn permission_warnings(&self, kind: AgentKind, request: &PermissionRequest) -> Vec<String> {
        let mut warnings = Vec::new();
        if let Some(root) = self.root_for_permissions(kind) {
            let outside: Vec<String> = request_paths(request)
                .into_iter()
                .filter(|path| !path.starts_with(&root))
                .map(|path| path.display().to_string())
                .collect();
            if !outside.is_empty() {
                warnings.push(format!("Outside the open folder: {}", outside.join(", ")));
            }
        }
        warnings
    }

    pub(super) fn queue_permission(&mut self, kind: AgentKind, id: &RequestId, params: &Value) {
        let Ok(mut request) = parse_permission_request(params) else {
            let reply = requests::permission_cancelled(id);
            self.agent_send(kind, &reply);
            return;
        };
        match self.permission_verdict(kind, &request) {
            Verdict::Reject(reason) => {
                let title = request.tool_call.title.clone();
                let notice = format!("Declined “{title}”: {reason}");
                self.reply_permission(
                    kind,
                    id,
                    &request,
                    find_option(
                        &request.options,
                        &[PermissionKind::RejectOnce, PermissionKind::RejectAlways],
                    ),
                    &notice,
                    LogLevel::Warn,
                );
            }
            Verdict::Allow => {
                let notice = format!(
                    "Allowed “{}” (read inside the folder).",
                    request.tool_call.title
                );
                self.reply_permission(
                    kind,
                    id,
                    &request,
                    find_option(&request.options, &[PermissionKind::AllowOnce]),
                    &notice,
                    LogLevel::Info,
                );
            }
            Verdict::Ask => {
                let warnings = self.permission_warnings(kind, &request);
                if !warnings.is_empty() {
                    // Never offer a standing approval for something outside the folder.
                    request
                        .options
                        .retain(|option| option.kind != PermissionKind::AllowAlways);
                }
                Self::agent_log(
                    kind,
                    LogLevel::Info,
                    &format!("Permission requested: {}", request.tool_call.title),
                );
                self.view
                    .agents
                    .session_mut(kind)
                    .pending_permissions
                    .push(PendingPermission {
                        id: id.clone(),
                        request,
                        warnings,
                    });
            }
        }
    }

    /// Send the reply for `choice` (or `cancelled` without one) and record it.
    fn reply_permission(
        &mut self,
        kind: AgentKind,
        id: &RequestId,
        request: &PermissionRequest,
        choice: Option<PermissionOption>,
        notice: &str,
        level: LogLevel,
    ) {
        let reply = match &choice {
            Some(option) => requests::permission_selected(id, &option.id),
            None => requests::permission_cancelled(id),
        };
        Self::agent_log(kind, level, notice);
        self.view
            .agents
            .session_mut(kind)
            .conversation
            .push_notice(notice);
        let _ = request;
        self.agent_send(kind, &reply);
    }

    /// The user's decision for a queued request. `None` declines it.
    pub(crate) fn answer_agent_permission(
        &mut self,
        kind: AgentKind,
        id: &RequestId,
        option_id: Option<&str>,
    ) {
        let session = self.view.agents.session_mut(kind);
        let Some(index) = session
            .pending_permissions
            .iter()
            .position(|pending| &pending.id == id)
        else {
            return;
        };
        let pending = session.pending_permissions.remove(index);
        let choice = option_id
            .and_then(|wanted| {
                pending
                    .request
                    .options
                    .iter()
                    .find(|o| o.id == wanted)
                    .cloned()
            })
            .or_else(|| {
                option_id.is_none().then(|| {
                    find_option(
                        &pending.request.options,
                        &[PermissionKind::RejectOnce, PermissionKind::RejectAlways],
                    )
                })?
            });
        let title = pending.request.tool_call.title.clone();
        let (verb, level) = match choice.as_ref().map(|option| option.kind) {
            Some(PermissionKind::AllowOnce | PermissionKind::AllowAlways) => {
                ("Allowed", LogLevel::Info)
            }
            _ => ("Declined", LogLevel::Info),
        };
        let notice = format!("{verb} “{title}”.");
        self.reply_permission(kind, id, &pending.request, choice, &notice, level);
    }

    /// Cancelling a turn must answer every outstanding request as cancelled.
    pub(super) fn cancel_pending_permissions(&mut self, kind: AgentKind) {
        let pending = std::mem::take(&mut self.view.agents.session_mut(kind).pending_permissions);
        for permission in pending {
            let reply = requests::permission_cancelled(&permission.id);
            self.agent_send(kind, &reply);
        }
    }
}

/// Short, readable path for cards: relative to the folder when inside it.
pub(crate) fn display_path(path: &str, root: Option<&Path>) -> String {
    let path = Path::new(path);
    root.and_then(|root| path.strip_prefix(root).ok())
        .map_or_else(
            || path.display().to_string(),
            |relative| relative.display().to_string(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::command::Command,
        model::Document,
        services::agent_process::{AgentEvent, fake::FakeAgentLauncher},
    };
    use eframe::egui;
    use serde_json::json;
    use std::sync::Arc;

    struct Harness {
        app: LumenApp,
        fake: FakeAgentLauncher,
        ctx: egui::Context,
    }

    impl Harness {
        fn new() -> Self {
            let fake = FakeAgentLauncher::default();
            let mut app = LumenApp::default();
            app.agent_runtime.launcher = Arc::new(fake.clone());
            let program = std::env::current_exe().unwrap().display().to_string();
            app.view.settings.user.persisted.insert(
                "agent.claude.command".into(),
                lumen_settings_core::Value::Text(program),
            );
            let mut h = Self {
                app,
                fake,
                ctx: egui::Context::default(),
            };
            h.app.view.agents.session_mut(AgentKind::Claude).draft = "go".into();
            h.app.execute(Command::SendAgentPrompt, &h.ctx);
            let init = h.last("initialize");
            h.reply(&init, json!({"protocolVersion":1}));
            let new = h.last("session/new");
            h.reply(&new, json!({"sessionId":"s1"}));
            h
        }

        fn sent(&self) -> Vec<serde_json::Value> {
            self.fake.sent.lock().unwrap().clone()
        }

        fn last(&self, method: &str) -> serde_json::Value {
            self.sent()
                .into_iter()
                .rev()
                .find(|m| m["method"] == method)
                .unwrap()
        }

        fn reply(&mut self, request: &serde_json::Value, result: serde_json::Value) {
            self.push(json!({"jsonrpc":"2.0","id":request["id"],"result":result}));
        }

        fn push(&mut self, message: serde_json::Value) {
            self.fake
                .queued
                .lock()
                .unwrap()
                .push(AgentEvent::Message(message));
            self.app.poll_agents(&self.ctx);
        }

        /// A permission request like the ones captured from Claude.
        fn ask(&mut self, id: i64, path: &str, kind: &str, title: &str) {
            self.push(
                json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{
                "sessionId":"s1",
                "toolCall":{"toolCallId":format!("t{id}"),"title":title,"kind":kind,
                    "locations":[{"path":path}],
                    "content": if kind == "edit" {
                        json!([{"type":"diff","path":path,"oldText":null,"newText":"hi"}])
                    } else { json!([]) }},
                "options":[
                    {"optionId":"allow-once","name":"Yes","kind":"allow_once"},
                    {"optionId":"allow-always","name":"Always","kind":"allow_always"},
                    {"optionId":"reject","name":"No","kind":"reject_once"}]}}),
            );
        }

        fn reply_to(&self, id: i64) -> Option<serde_json::Value> {
            self.sent()
                .into_iter()
                .find(|m| m["id"] == id && m.get("result").is_some())
        }

        fn pending(&self) -> usize {
            self.app
                .view
                .agents
                .session(AgentKind::Claude)
                .pending_permissions
                .len()
        }
    }

    #[test]
    fn requests_wait_for_the_user_and_the_choice_is_sent_back() {
        let mut h = Harness::new();
        h.app.editor.workspace = Some("/w".into());
        h.ask(5, "/w/notes.txt", "edit", "Write notes.txt");
        assert_eq!(h.pending(), 1);
        assert!(
            h.reply_to(5).is_none(),
            "the agent stays blocked until the user decides"
        );
        assert!(h.app.view.agents.needs_attention());

        h.app.execute(
            Command::AnswerAgentPermission(
                AgentKind::Claude,
                RequestId::Number(5),
                Some("allow-once".into()),
            ),
            &h.ctx,
        );
        assert_eq!(
            h.reply_to(5).unwrap()["result"]["outcome"],
            json!({"outcome":"selected","optionId":"allow-once"})
        );
        assert_eq!(h.pending(), 0);
        assert!(!h.app.view.agents.needs_attention());

        h.ask(6, "/w/notes.txt", "edit", "Write notes.txt");
        h.app.execute(
            Command::AnswerAgentPermission(AgentKind::Claude, RequestId::Number(6), None),
            &h.ctx,
        );
        assert_eq!(
            h.reply_to(6).unwrap()["result"]["outcome"]["optionId"],
            "reject",
            "declining picks the reject option"
        );
        let notices: Vec<_> = h
            .app
            .view
            .agents
            .session(AgentKind::Claude)
            .conversation
            .entries
            .iter()
            .filter_map(|e| {
                if let lumen_agent_protocol::reducer::Entry::Notice(t) = e {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            notices,
            ["Allowed “Write notes.txt”.", "Declined “Write notes.txt”."]
        );
        // Answering twice (or a stranger) is harmless.
        let before = h.sent().len();
        h.app.execute(
            Command::AnswerAgentPermission(AgentKind::Claude, RequestId::Number(6), None),
            &h.ctx,
        );
        h.app.execute(
            Command::AnswerAgentPermission(
                AgentKind::Claude,
                RequestId::Number(99),
                Some("x".into()),
            ),
            &h.ctx,
        );
        assert_eq!(h.sent().len(), before);
    }

    #[test]
    fn unsaved_buffers_are_never_put_at_risk() {
        let mut h = Harness::new();
        h.app.editor.workspace = Some("/w".into());
        let mut dirty = Document::from_path("/w/main.rs".into(), "fn main() {}".into());
        dirty.dirty = true;
        let clean = Document::from_path("/w/lib.rs".into(), "".into());
        h.app.editor.files.push(dirty);
        h.app.editor.files.push(clean);

        h.ask(1, "/w/main.rs", "edit", "Write main.rs");
        assert_eq!(h.pending(), 0, "no card: refused outright");
        let reply = h.reply_to(1).unwrap();
        assert_eq!(reply["result"]["outcome"]["optionId"], "reject");
        let text = format!(
            "{:?}",
            h.app
                .view
                .agents
                .session(AgentKind::Claude)
                .conversation
                .entries
        );
        assert!(text.contains("main.rs has unsaved changes"), "{text}");

        h.ask(2, "/w/lib.rs", "edit", "Write lib.rs");
        assert_eq!(h.pending(), 1, "a clean open file is fine");
        // A read of the dirty file is harmless and still asks (or is auto-allowed by setting).
        h.ask(3, "/w/main.rs", "read", "Read main.rs");
        assert_eq!(h.pending(), 2);
    }

    #[test]
    fn requests_outside_the_folder_warn_and_cannot_be_approved_permanently() {
        let mut h = Harness::new();
        h.app.editor.workspace = Some("/w".into());
        h.ask(1, "/etc/hosts", "edit", "Write hosts");
        let pending = &h
            .app
            .view
            .agents
            .session(AgentKind::Claude)
            .pending_permissions[0];
        assert!(pending.warnings[0].contains("/etc/hosts"));
        assert!(
            pending
                .request
                .options
                .iter()
                .all(|o| o.kind != PermissionKind::AllowAlways)
        );
        assert_eq!(pending.request.options.len(), 2);
    }

    #[test]
    fn reads_inside_the_folder_are_auto_allowed_only_when_the_setting_is_on() {
        let mut h = Harness::new();
        h.app.editor.workspace = Some("/w".into());
        h.ask(1, "/w/a.rs", "read", "Read a.rs");
        assert_eq!(h.pending(), 1, "off by default");
        h.app.view.settings.user.persisted.insert(
            "agent.allow_read_in_workspace".into(),
            lumen_settings_core::Value::Bool(true),
        );
        h.ask(2, "/w/b.rs", "read", "Read b.rs");
        assert_eq!(
            h.reply_to(2).unwrap()["result"]["outcome"]["optionId"],
            "allow-once"
        );
        h.ask(3, "/other/c.rs", "read", "Read c.rs");
        assert!(h.reply_to(3).is_none(), "outside the folder still asks");
        h.ask(4, "/w/d.rs", "edit", "Write d.rs");
        assert!(h.reply_to(4).is_none(), "edits always ask");
    }

    #[test]
    fn cancelling_answers_every_open_request_and_a_crash_drops_them() {
        let mut h = Harness::new();
        h.app.editor.workspace = Some("/w".into());
        h.app.view.agents.session_mut(AgentKind::Claude).running = true;
        h.ask(1, "/w/a.txt", "edit", "A");
        h.ask(2, "/w/b.txt", "edit", "B");
        h.app.execute(Command::CancelAgentTurn, &h.ctx);
        for id in [1, 2] {
            assert_eq!(
                h.reply_to(id).unwrap()["result"]["outcome"],
                json!({"outcome":"cancelled"})
            );
        }
        assert_eq!(h.pending(), 0);

        h.ask(3, "/w/c.txt", "edit", "C");
        assert_eq!(h.pending(), 1);
        h.fake.queued.lock().unwrap().push(AgentEvent::Exited);
        h.app.poll_agents(&h.ctx);
        assert_eq!(h.pending(), 0, "no one is waiting for an answer any more");
    }

    #[test]
    fn display_paths_are_relative_inside_the_folder() {
        assert_eq!(
            display_path("/w/src/a.rs", Some(Path::new("/w"))),
            "src/a.rs"
        );
        assert_eq!(display_path("/x/a.rs", Some(Path::new("/w"))), "/x/a.rs");
        assert_eq!(display_path("/x/a.rs", None), "/x/a.rs");
    }
}
