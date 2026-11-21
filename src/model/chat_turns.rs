//! A chat transcript read as a conversation: each of your messages opens a
//! turn, and the agent's response to it is a sequence of parts, either a run
//! of working steps (notes, tool calls, plans, notices) or an answer. Only the
//! grouping is decided here; every item stays reachable by its index.
use super::tool_groups::{self, Block};
use lumen_agent_core::model::{ItemKind, MessagePhase, ToolKind, TranscriptItem};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Turn {
    /// Your message that opens the turn; `None` for anything said before it.
    pub(crate) prompt: Option<usize>,
    pub(crate) parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// Consecutive working steps, with quiet reads and searches grouped.
    Steps(Vec<Block>),
    /// An answer from the agent.
    Answer(usize),
}

impl Turn {
    /// Whether the steps at `part` come before an answer in this turn, so they
    /// can fold away once the turn is over.
    pub(crate) fn answered_after(&self, part: usize) -> bool {
        self.parts[part + 1..]
            .iter()
            .any(|part| matches!(part, Part::Answer(_)))
    }
}

/// The transcript as turns, in order. An empty reply takes no place.
pub(crate) fn turns(items: &[TranscriptItem]) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut steps: Vec<usize> = Vec::new();
    let flush = |steps: &mut Vec<usize>, turns: &mut Vec<Turn>| {
        if steps.is_empty() {
            return;
        }
        let turn = current(turns);
        turn.parts
            .push(Part::Steps(tool_groups::blocks_in(items, steps)));
        steps.clear();
    };
    for (index, item) in items.iter().enumerate() {
        match &item.kind {
            ItemKind::User(_) => {
                flush(&mut steps, &mut turns);
                turns.push(Turn {
                    prompt: Some(index),
                    parts: Vec::new(),
                });
            }
            ItemKind::Agent { text, .. } if text.trim().is_empty() => {}
            ItemKind::Agent {
                phase: MessagePhase::Final,
                ..
            } => {
                flush(&mut steps, &mut turns);
                current(&mut turns).parts.push(Part::Answer(index));
            }
            _ => steps.push(index),
        }
    }
    flush(&mut steps, &mut turns);
    turns
}

/// The turn being added to, starting one without a prompt if there is none.
fn current(turns: &mut Vec<Turn>) -> &mut Turn {
    if turns.is_empty() {
        turns.push(Turn {
            prompt: None,
            parts: Vec::new(),
        });
    }
    turns.last_mut().expect("a turn was just ensured")
}

/// One line for folded steps: "Worked through 6 steps" and what they were,
/// such as "read 3 files, ran 2 commands, edited 1 file".
pub(crate) fn steps_summary(items: &[TranscriptItem], blocks: &[Block]) -> (String, String) {
    let indices: Vec<usize> = blocks
        .iter()
        .flat_map(|block| match block {
            Block::Item(index) => vec![*index],
            Block::Explored(group) => group.clone(),
        })
        .collect();
    let count_tools = |kind: ToolKind| {
        indices
            .iter()
            .filter(|&&i| matches!(&items[i].kind, ItemKind::Tool(call) if call.kind == kind))
            .count()
    };
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut done = Vec::new();
    let reads = count_tools(ToolKind::Read);
    if reads > 0 {
        done.push(format!("read {}", plural(reads, "file", "files")));
    }
    let searches = count_tools(ToolKind::Search);
    if searches > 0 {
        done.push(format!("searched {}", plural(searches, "time", "times")));
    }
    let commands = count_tools(ToolKind::Command);
    if commands > 0 {
        done.push(format!("ran {}", plural(commands, "command", "commands")));
    }
    let edits = count_tools(ToolKind::FileEdit);
    if edits > 0 {
        done.push(format!("edited {}", plural(edits, "file", "files")));
    }
    let fetches = count_tools(ToolKind::Fetch);
    if fetches > 0 {
        done.push(format!("fetched {}", plural(fetches, "page", "pages")));
    }
    let errors = indices
        .iter()
        .filter(|&&i| matches!(items[i].kind, ItemKind::Error(_)))
        .count();
    if errors > 0 {
        done.push(plural(errors, "error", "errors"));
    }
    (
        format!("Worked through {}", plural(indices.len(), "step", "steps")),
        done.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_agent_core::{
        ItemId,
        model::{ToolCall, ToolStatus},
    };

    fn item(kind: ItemKind) -> TranscriptItem {
        TranscriptItem {
            id: ItemId("i".into()),
            turn: None,
            kind,
        }
    }

    fn user() -> TranscriptItem {
        item(ItemKind::User("question".into()))
    }

    fn agent(phase: MessagePhase, text: &str) -> TranscriptItem {
        item(ItemKind::Agent {
            text: text.into(),
            phase,
            finished: true,
        })
    }

    fn tool(kind: ToolKind) -> TranscriptItem {
        let mut call = ToolCall::new(kind, "t");
        call.status = ToolStatus::Completed;
        item(ItemKind::Tool(call))
    }

    #[test]
    fn each_message_opens_a_turn_and_answers_follow_their_steps() {
        let items = [
            user(),
            agent(MessagePhase::Commentary, "looking"),
            tool(ToolKind::Command),
            agent(MessagePhase::Final, "done"),
            user(),
            agent(MessagePhase::Final, "sure"),
        ];
        assert_eq!(
            turns(&items),
            [
                Turn {
                    prompt: Some(0),
                    parts: vec![
                        Part::Steps(vec![Block::Item(1), Block::Item(2)]),
                        Part::Answer(3)
                    ],
                },
                Turn {
                    prompt: Some(4),
                    parts: vec![Part::Answer(5)],
                },
            ]
        );
    }

    #[test]
    fn steps_between_answers_stay_in_place_and_quiet_reads_group() {
        let items = [
            user(),
            agent(MessagePhase::Final, "first"),
            tool(ToolKind::Read),
            tool(ToolKind::Read),
            tool(ToolKind::Search),
            agent(MessagePhase::Final, "second"),
        ];
        let turns = turns(&items);
        assert_eq!(
            turns[0].parts,
            [
                Part::Answer(1),
                Part::Steps(vec![Block::Explored(vec![2, 3, 4])]),
                Part::Answer(5),
            ]
        );
        assert!(turns[0].answered_after(1));
    }

    #[test]
    fn content_before_any_message_and_empty_replies_are_handled() {
        let items = [
            item(ItemKind::Notice("restored".into())),
            agent(MessagePhase::Final, "   "),
            user(),
            tool(ToolKind::Command),
        ];
        let turns = turns(&items);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].prompt, None);
        assert_eq!(turns[0].parts, [Part::Steps(vec![Block::Item(0)])]);
        assert_eq!(turns[1].parts, [Part::Steps(vec![Block::Item(3)])]);
        assert!(
            !turns[1].answered_after(0),
            "still working: nothing answered"
        );
    }

    #[test]
    fn the_summary_counts_every_step_and_names_what_they_did() {
        let items = [
            user(),
            agent(MessagePhase::Commentary, "note"),
            tool(ToolKind::Read),
            tool(ToolKind::Read),
            tool(ToolKind::Read),
            tool(ToolKind::Command),
            tool(ToolKind::FileEdit),
        ];
        let Part::Steps(blocks) = &turns(&items)[0].parts[0] else {
            panic!("steps");
        };
        assert_eq!(
            steps_summary(&items, blocks),
            (
                "Worked through 6 steps".to_string(),
                "read 3 files, ran 1 command, edited 1 file".to_string()
            )
        );
    }
}
