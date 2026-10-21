//! Saved form of a conversation, so a chat reopened after a restart shows what
//! was said. This is Lumen's display cache, not the provider's history: it is
//! bounded, tolerant of damage, and never claims a restored tool is still running.
use lumen_agent_core::{
    ItemId, TurnId,
    model::{
        ItemKind, MessagePhase, PlanEntry, PlanStatus, ToolCall, ToolKind, ToolStatus,
        TranscriptItem,
    },
};
use serde_json::{Value, json};

pub(crate) const SCHEMA_VERSION: u64 = 1;
/// Newest items kept; older ones are dropped when saving.
pub(crate) const MAX_ITEMS: usize = 1500;
/// Longest tool output / patch kept per item.
const MAX_TEXT_BYTES: usize = 16 * 1024;

fn clip(text: &str) -> String {
    if text.len() <= MAX_TEXT_BYTES {
        return text.to_string();
    }
    let mut end = MAX_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… cached output truncated", &text[..end])
}

fn kind_name(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Command => "command",
        ToolKind::FileEdit => "file_edit",
        ToolKind::Read => "read",
        ToolKind::Search => "search",
        ToolKind::Fetch => "fetch",
        ToolKind::Other => "other",
    }
}

fn kind_of(name: &str) -> ToolKind {
    match name {
        "command" => ToolKind::Command,
        "file_edit" => ToolKind::FileEdit,
        "read" => ToolKind::Read,
        "search" => ToolKind::Search,
        "fetch" => ToolKind::Fetch,
        _ => ToolKind::Other,
    }
}

fn status_name(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "pending",
        ToolStatus::Running => "running",
        ToolStatus::Completed => "completed",
        ToolStatus::Failed => "failed",
        ToolStatus::Declined => "declined",
        ToolStatus::Cancelled => "cancelled",
    }
}

fn status_of(name: &str) -> ToolStatus {
    match name {
        "completed" => ToolStatus::Completed,
        "failed" => ToolStatus::Failed,
        "declined" => ToolStatus::Declined,
        // A restored call cannot still be running: its process is gone.
        _ => ToolStatus::Cancelled,
    }
}

fn plan_status_name(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Pending => "pending",
        PlanStatus::InProgress => "in_progress",
        PlanStatus::Done => "done",
    }
}

fn plan_status_of(name: &str) -> PlanStatus {
    match name {
        "in_progress" => PlanStatus::InProgress,
        "done" => PlanStatus::Done,
        _ => PlanStatus::Pending,
    }
}

fn item_json(item: &TranscriptItem) -> Value {
    let mut value = match &item.kind {
        ItemKind::User(text) => json!({"kind": "user", "text": text}),
        ItemKind::Agent { text, phase, .. } => json!({
            "kind": "agent", "text": text,
            "phase": if *phase == MessagePhase::Commentary { "commentary" } else { "final" },
        }),
        ItemKind::Tool(call) => json!({
            "kind": "tool",
            "tool": kind_name(call.kind),
            "title": call.title,
            "status": status_name(call.status),
            "preview": clip(&call.preview),
            "patch": call.patch.as_deref().map(clip),
            "locations": call.locations.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
            "exit_code": call.exit_code,
        }),
        ItemKind::Plan(entries) => json!({
            "kind": "plan",
            "entries": entries.iter().map(|e| json!({"text": e.text, "status": plan_status_name(e.status)})).collect::<Vec<_>>(),
        }),
        ItemKind::Error(text) => json!({"kind": "error", "text": text}),
        ItemKind::Notice(text) => json!({"kind": "notice", "text": text}),
    };
    value["id"] = json!(item.id.0);
    value["turn"] = json!(item.turn.as_ref().map(|t| t.0.clone()));
    value
}

fn read_item(value: &Value) -> Option<TranscriptItem> {
    let text = |key: &str| value.get(key).and_then(Value::as_str);
    let kind = match text("kind")? {
        "user" => ItemKind::User(text("text")?.to_string()),
        "agent" => ItemKind::Agent {
            text: text("text")?.to_string(),
            phase: if text("phase") == Some("commentary") {
                MessagePhase::Commentary
            } else {
                MessagePhase::Final
            },
            finished: true,
        },
        "tool" => {
            let mut call = ToolCall::new(kind_of(text("tool").unwrap_or("")), text("title")?);
            call.status = status_of(text("status").unwrap_or(""));
            call.preview = text("preview").unwrap_or_default().to_string();
            call.patch = text("patch").map(str::to_string);
            call.exit_code = value
                .get("exit_code")
                .and_then(Value::as_i64)
                .map(|c| c as i32);
            call.locations = value
                .get("locations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(Into::into))
                .collect();
            ItemKind::Tool(call)
        }
        "plan" => ItemKind::Plan(
            value
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    Some(PlanEntry {
                        text: e.get("text")?.as_str()?.to_string(),
                        status: plan_status_of(e.get("status")?.as_str()?),
                    })
                })
                .collect(),
        ),
        "error" => ItemKind::Error(text("text")?.to_string()),
        "notice" => ItemKind::Notice(text("text")?.to_string()),
        _ => return None,
    };
    Some(TranscriptItem {
        id: ItemId(text("id")?.to_string()),
        turn: text("turn").map(|t| TurnId(t.to_string())),
        kind,
    })
}

pub(crate) fn to_json(items: &[TranscriptItem]) -> String {
    let start = items.len().saturating_sub(MAX_ITEMS);
    let rows: Vec<Value> = items[start..].iter().map(item_json).collect();
    serde_json::to_string(&json!({"schema_version": SCHEMA_VERSION, "items": rows}))
        .unwrap_or_default()
}

/// Items that could be read, and how many rows were skipped. A newer or damaged
/// file is an error so the caller leaves it alone.
pub(crate) fn from_json(text: &str) -> Result<(Vec<TranscriptItem>, usize), String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("unreadable: {e}"))?;
    let version = value.get("schema_version").and_then(Value::as_u64);
    if version.is_none_or(|v| v > SCHEMA_VERSION) {
        return Err(format!("unsupported transcript version ({version:?})"));
    }
    let mut skipped = 0;
    let mut items = Vec::new();
    for row in value
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match read_item(row) {
            Some(item) => items.push(item),
            None => skipped += 1,
        }
    }
    Ok((items, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<TranscriptItem> {
        let mut call = ToolCall::new(ToolKind::Command, "ls -la");
        call.status = ToolStatus::Completed;
        call.preview = "total 0".into();
        call.exit_code = Some(0);
        call.locations = vec!["/w/a.rs".into()];
        let mut running = ToolCall::new(ToolKind::FileEdit, "edit b.rs");
        running.status = ToolStatus::Running;
        vec![
            TranscriptItem {
                id: ItemId("u1".into()),
                turn: None,
                kind: ItemKind::User("hello\nthere".into()),
            },
            TranscriptItem {
                id: ItemId("a1".into()),
                turn: Some(TurnId("turn-1".into())),
                kind: ItemKind::Agent {
                    text: "**hi**".into(),
                    phase: MessagePhase::Commentary,
                    finished: false,
                },
            },
            TranscriptItem {
                id: ItemId("t1".into()),
                turn: None,
                kind: ItemKind::Tool(call),
            },
            TranscriptItem {
                id: ItemId("t2".into()),
                turn: None,
                kind: ItemKind::Tool(running),
            },
            TranscriptItem {
                id: ItemId("p1".into()),
                turn: None,
                kind: ItemKind::Plan(vec![PlanEntry {
                    text: "step".into(),
                    status: PlanStatus::InProgress,
                }]),
            },
            TranscriptItem {
                id: ItemId("e1".into()),
                turn: None,
                kind: ItemKind::Error("boom".into()),
            },
        ]
    }

    #[test]
    fn a_conversation_round_trips_and_restored_work_is_never_running() {
        let (items, skipped) = from_json(&to_json(&sample())).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(items.len(), 6);
        assert!(matches!(&items[0].kind, ItemKind::User(t) if t == "hello\nthere"));
        assert!(matches!(
            &items[1].kind,
            ItemKind::Agent {
                phase: MessagePhase::Commentary,
                finished: true,
                ..
            }
        ));
        assert_eq!(items[1].turn, Some(TurnId("turn-1".into())));
        let ItemKind::Tool(done) = &items[2].kind else {
            panic!()
        };
        assert_eq!(
            (done.status, done.preview.as_str(), done.exit_code),
            (ToolStatus::Completed, "total 0", Some(0))
        );
        assert_eq!(done.locations, [std::path::PathBuf::from("/w/a.rs")]);
        let ItemKind::Tool(was_running) = &items[3].kind else {
            panic!()
        };
        assert_eq!(was_running.status, ToolStatus::Cancelled);
        assert!(
            matches!(&items[4].kind, ItemKind::Plan(e) if e[0].status == PlanStatus::InProgress)
        );
    }

    #[test]
    fn long_output_and_long_conversations_are_bounded() {
        let mut call = ToolCall::new(ToolKind::Command, "big");
        call.preview = "x".repeat(MAX_TEXT_BYTES * 3);
        let mut items = vec![TranscriptItem {
            id: ItemId("big".into()),
            turn: None,
            kind: ItemKind::Tool(call),
        }];
        items.extend((0..MAX_ITEMS + 10).map(|n| TranscriptItem {
            id: ItemId(format!("n{n}")),
            turn: None,
            kind: ItemKind::Notice(n.to_string()),
        }));
        let (loaded, _) = from_json(&to_json(&items)).unwrap();
        assert_eq!(loaded.len(), MAX_ITEMS, "the newest are kept");
        assert_eq!(loaded.last().unwrap().id.0, format!("n{}", MAX_ITEMS + 9));
        let (loaded, _) = from_json(&to_json(&items[..1])).unwrap();
        let ItemKind::Tool(call) = &loaded[0].kind else {
            panic!()
        };
        assert!(call.preview.len() < MAX_TEXT_BYTES + 64 && call.preview.ends_with("truncated"));
    }

    #[test]
    fn damaged_rows_are_skipped_and_newer_or_broken_files_are_errors() {
        let text = r#"{"schema_version":1,"items":[
            {"id":"a","kind":"user","text":"ok"},
            {"id":"b","kind":"mystery"},
            {"kind":"user","text":"no id"},
            "junk"]}"#;
        let (items, skipped) = from_json(text).unwrap();
        assert_eq!((items.len(), skipped), (1, 3));
        assert!(from_json(r#"{"schema_version":9,"items":[]}"#).is_err());
        assert!(from_json("nope").is_err());
        assert!(from_json("{}").is_err());
    }
}
