use super::{Control, DefaultValue, SectionSchema, Setting};

pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Agents",
    validate: None,
    settings: &[
        Setting {
            id: "agent.enabled",
            default: DefaultValue::Bool(true),
            name: "Coding Agents",
            category: "Agents",
            help: "Show the agent session list. Nothing is started until you send a message.",
            aliases: "claude codex ai assistant chat",
            control: Control::Bool,
            document: false,
        },
        Setting {
            id: "agent.native.codex_command",
            default: DefaultValue::Text(""),
            name: "Codex Program",
            category: "Agents",
            help: "Absolute path to the codex program. Only honored in User settings; project files cannot choose a program to run. Leave empty to search PATH.",
            aliases: "codex executable binary native cli",
            control: Control::Text(1024),
            document: false,
        },
        Setting {
            id: "agent.native.claude_command",
            default: DefaultValue::Text(""),
            name: "Claude Code Program",
            category: "Agents",
            help: "Absolute path to the claude program. Only honored in User settings; project files cannot choose a program to run. Leave empty to search PATH.",
            aliases: "claude executable binary native cli",
            control: Control::Text(1024),
            document: false,
        },
    ],
};
