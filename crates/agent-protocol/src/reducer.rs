//! Folds `session/update` notifications into the conversation shown to the user.
use crate::types::{
    AvailableCommand, ConfigOption, ContentBlock, PlanEntry, ToolCall, ToolStatus, Update,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    Thought,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Message {
        role: Role,
        text: String,
    },
    ToolCall(ToolCall),
    /// Client-side notice (cancelled, errors); never sent by the agent.
    Notice(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub entries: Vec<Entry>,
    pub plan: Vec<PlanEntry>,
    pub commands: Vec<AvailableCommand>,
    pub config_options: Vec<ConfigOption>,
    pub current_mode: Option<String>,
    pub title: Option<String>,
}

impl Conversation {
    /// Apply one update; returns whether anything user-visible changed.
    pub fn apply(&mut self, update: Update) -> bool {
        match update {
            Update::UserMessage(block) => self.push_chunk(Role::User, &block),
            Update::AgentMessage(block) => self.push_chunk(Role::Agent, &block),
            Update::AgentThought(block) => self.push_chunk(Role::Thought, &block),
            Update::ToolCall(call) => {
                // A repeated tool_call for a known id refreshes it rather than duplicating.
                match self.tool_call_mut(&call.id) {
                    Some(existing) => *existing = call,
                    None => self.entries.push(Entry::ToolCall(call)),
                }
                true
            }
            Update::ToolCallUpdate(patch) => match self.tool_call_mut(&patch.id) {
                Some(call) => {
                    call.apply(patch);
                    true
                }
                // An update for a call we never saw: keep it rather than lose output.
                None => {
                    let mut call = ToolCall {
                        id: patch.id.clone(),
                        ..ToolCall::default()
                    };
                    call.apply(patch);
                    self.entries.push(Entry::ToolCall(call));
                    true
                }
            },
            Update::Plan(entries) => {
                self.plan = entries;
                true
            }
            Update::AvailableCommands(commands) => {
                self.commands = commands;
                true
            }
            Update::ConfigOptions(options) => {
                self.config_options = options;
                true
            }
            Update::CurrentMode(mode) => {
                self.current_mode = Some(mode);
                true
            }
            Update::SessionInfo { title } => match title {
                Some(title) => {
                    self.title = Some(title);
                    true
                }
                None => false,
            },
            Update::Ignored(_) => false,
        }
    }

    /// Local echo of what the user just sent (agents only replay it on load).
    pub fn push_user_message(&mut self, text: &str) {
        self.entries.push(Entry::Message {
            role: Role::User,
            text: text.to_owned(),
        });
    }

    pub fn push_notice(&mut self, text: &str) {
        self.entries.push(Entry::Notice(text.to_owned()));
    }

    /// The turn ended (any stop reason): nothing it started is still running.
    pub fn finish_turn(&mut self, cancelled: bool) {
        for entry in &mut self.entries {
            if let Entry::ToolCall(call) = entry
                && matches!(call.status, ToolStatus::Pending | ToolStatus::InProgress)
                && cancelled
            {
                call.status = ToolStatus::Failed;
            }
        }
    }

    #[must_use]
    pub fn tool_call(&self, id: &str) -> Option<&ToolCall> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::ToolCall(call) if call.id == id => Some(call),
            _ => None,
        })
    }

    fn tool_call_mut(&mut self, id: &str) -> Option<&mut ToolCall> {
        self.entries.iter_mut().find_map(|entry| match entry {
            Entry::ToolCall(call) if call.id == id => Some(call),
            _ => None,
        })
    }

    fn push_chunk(&mut self, role: Role, block: &ContentBlock) -> bool {
        let piece = block.display_text();
        if piece.is_empty() {
            return false;
        }
        if let Some(Entry::Message { role: last, text }) = self.entries.last_mut()
            && *last == role
        {
            text.push_str(&piece);
        } else {
            self.entries.push(Entry::Message { role, text: piece });
        }
        true
    }

    /// Concatenated agent text, for tests and copy-all.
    #[must_use]
    pub fn agent_text(&self) -> String {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Message {
                    role: Role::Agent,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolCallPatch, ToolKind};

    fn chunk(role: Role, text: &str) -> Update {
        let block = ContentBlock::Text(text.into());
        match role {
            Role::User => Update::UserMessage(block),
            Role::Agent => Update::AgentMessage(block),
            Role::Thought => Update::AgentThought(block),
        }
    }

    #[test]
    fn consecutive_chunks_of_one_role_coalesce_and_roles_do_not_mix() {
        let mut conversation = Conversation::default();
        for update in [
            chunk(Role::Thought, "hm"),
            chunk(Role::Agent, "Hel"),
            chunk(Role::Agent, "lo"),
            Update::ToolCall(ToolCall {
                id: "t".into(),
                kind: ToolKind::Read,
                ..ToolCall::default()
            }),
            chunk(Role::Agent, " again"),
        ] {
            conversation.apply(update);
        }
        assert_eq!(conversation.entries.len(), 4);
        assert_eq!(conversation.agent_text(), "Hello\n again");
        assert!(!conversation.apply(Update::AgentMessage(ContentBlock::Text(String::new()))));
    }

    #[test]
    fn tool_calls_merge_by_id_and_unknown_updates_are_kept() {
        let mut conversation = Conversation::default();
        conversation.apply(Update::ToolCall(ToolCall {
            id: "a".into(),
            title: "Run".into(),
            ..ToolCall::default()
        }));
        conversation.apply(Update::ToolCallUpdate(ToolCallPatch {
            id: "a".into(),
            status: Some(ToolStatus::Completed),
            ..ToolCallPatch::default()
        }));
        conversation.apply(Update::ToolCallUpdate(ToolCallPatch {
            id: "orphan".into(),
            title: Some("Seen late".into()),
            ..ToolCallPatch::default()
        }));
        assert_eq!(conversation.entries.len(), 2);
        let call = conversation.tool_call("a").unwrap();
        assert_eq!(
            (call.title.as_str(), call.status),
            ("Run", ToolStatus::Completed)
        );
        assert_eq!(conversation.tool_call("orphan").unwrap().title, "Seen late");
    }

    #[test]
    fn plans_commands_and_options_replace_wholesale() {
        let mut conversation = Conversation::default();
        conversation.apply(Update::Plan(vec![PlanEntry {
            content: "one".into(),
            status: crate::types::PlanStatus::Pending,
        }]));
        conversation.apply(Update::Plan(vec![]));
        assert!(conversation.plan.is_empty());
        assert!(!conversation.apply(Update::SessionInfo { title: None }));
        assert!(conversation.apply(Update::SessionInfo {
            title: Some("T".into())
        }));
        assert_eq!(conversation.title.as_deref(), Some("T"));
    }

    #[test]
    fn cancelling_a_turn_fails_calls_that_never_finished() {
        let mut conversation = Conversation::default();
        conversation.apply(Update::ToolCall(ToolCall {
            id: "run".into(),
            status: ToolStatus::InProgress,
            ..ToolCall::default()
        }));
        conversation.apply(Update::ToolCall(ToolCall {
            id: "done".into(),
            status: ToolStatus::Completed,
            ..ToolCall::default()
        }));
        conversation.finish_turn(false);
        assert_eq!(
            conversation.tool_call("run").unwrap().status,
            ToolStatus::InProgress
        );
        conversation.finish_turn(true);
        assert_eq!(
            conversation.tool_call("run").unwrap().status,
            ToolStatus::Failed
        );
        assert_eq!(
            conversation.tool_call("done").unwrap().status,
            ToolStatus::Completed
        );
    }
}
