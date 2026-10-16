use super::{Control, DefaultValue, SectionSchema, Setting};

const LAUNCH: &[&str] = &["auto", "binary", "npx"];

pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Agents",
    validate: None,
    settings: &[
        Setting {
            id: "agent.enabled",
            default: DefaultValue::Bool(true),
            name: "Coding Agents",
            category: "Agents",
            help: "Show the agent panel. Nothing is started until you send a message.",
            aliases: "claude codex ai assistant acp chat",
            control: Control::Bool,
            document: false,
        },
        Setting {
            id: "agent.claude.command",
            default: DefaultValue::Text(""),
            name: "Claude Adapter Path",
            category: "Agents",
            help: "Absolute path to a claude-agent-acp executable. Only honored in User settings; project files cannot choose a program to run. Leave empty to search PATH.",
            aliases: "claude code acp adapter binary command",
            control: Control::Text(1024),
            document: false,
        },
        Setting {
            id: "agent.claude.launch",
            default: DefaultValue::Choice("auto"),
            name: "Claude Launch Method",
            category: "Agents",
            help: "auto: use an installed adapter, otherwise ask before downloading. binary: only an installed adapter. npx: download and run @agentclientprotocol/claude-agent-acp with npx (large download).",
            aliases: "claude npx node install",
            control: Control::Choice(LAUNCH),
            document: false,
        },
        Setting {
            id: "agent.codex.command",
            default: DefaultValue::Text(""),
            name: "Codex Adapter Path",
            category: "Agents",
            help: "Absolute path to a codex-acp executable. Only honored in User settings; project files cannot choose a program to run. Leave empty to search PATH.",
            aliases: "openai codex acp adapter binary command",
            control: Control::Text(1024),
            document: false,
        },
        Setting {
            id: "agent.codex.launch",
            default: DefaultValue::Choice("auto"),
            name: "Codex Launch Method",
            category: "Agents",
            help: "auto: use an installed adapter, otherwise ask before downloading. binary: only an installed adapter. npx: download and run @agentclientprotocol/codex-acp with npx (large download).",
            aliases: "codex npx node install",
            control: Control::Choice(LAUNCH),
            document: false,
        },
    ],
};
