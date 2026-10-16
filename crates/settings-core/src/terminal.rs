use super::{Control, DefaultValue, SectionSchema, Setting};

pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Terminal",
    validate: Some(validate),
    settings: &[
        Setting {
            id: "terminal.font_family",
            default: DefaultValue::Choice("platform"),
            name: "Font Family",
            category: "Terminal",
            help: "Monospace font used to render terminal cells.",
            aliases: "typeface typography console shell",
            control: Control::Choice(&["platform", "Hack"]),
            document: false,
        },
        Setting {
            id: "terminal.font_size",
            default: DefaultValue::Number(13.0),
            name: "Font Size",
            category: "Terminal",
            help: "Terminal text size in logical pixels.",
            aliases: "text size typography console shell",
            control: Control::Number(8.0, 32.0),
            document: false,
        },
        Setting {
            id: "terminal.line_height",
            default: DefaultValue::Number(17.0),
            name: "Line Height",
            category: "Terminal",
            help: "Terminal cell row height in logical pixels; must be at least the font size.",
            aliases: "line spacing rows cell height",
            control: Control::Number(8.0, 48.0),
            document: false,
        },
        Setting {
            id: "terminal.cursor_style",
            default: DefaultValue::Choice("block"),
            name: "Cursor Style",
            category: "Terminal",
            help: "Shape of the shell cursor rendered by the terminal emulator.",
            aliases: "caret block underline cursor",
            control: Control::Choice(&["block", "underline"]),
            document: false,
        },
        Setting {
            id: "terminal.cursor_blink",
            default: DefaultValue::Bool(true),
            name: "Cursor Blink",
            category: "Terminal",
            help: "Blink the active terminal cursor while the terminal canvas has focus.",
            aliases: "caret animation accessibility",
            control: Control::Bool,
            document: false,
        },
        Setting {
            id: "terminal.color_scheme",
            default: DefaultValue::Choice("workbench"),
            name: "Color Scheme",
            category: "Terminal",
            help: "ANSI color palette for terminal output. Shell color codes remain shell-owned.",
            aliases: "ansi colour theme contrast output",
            control: Control::Choice(&["workbench", "vscode_dark", "light", "high_contrast"]),
            document: false,
        },
        Setting {
            id: "terminal.auto_scroll",
            default: DefaultValue::Bool(true),
            name: "Auto Scroll",
            category: "Terminal",
            help: "Follow incoming shell output at the bottom of terminal scrollback.",
            aliases: "follow output scrollback bottom",
            control: Control::Bool,
            document: false,
        },
        Setting {
            id: "terminal.panel_divider",
            default: DefaultValue::Bool(true),
            name: "Panel Divider",
            category: "Terminal",
            help: "Show the subtle divider between the terminal and editor workbench.",
            aliases: "border separator panel layout",
            control: Control::Bool,
            document: false,
        },
    ],
};

fn validate(layer: &super::Layer) -> Result<(), String> {
    let number = |id: &str| match &layer[id] {
        super::Value::Number(value) => *value,
        _ => unreachable!("validated schema type"),
    };
    if number("terminal.line_height") < number("terminal.font_size") {
        Err("Terminal line height must be at least the terminal font size.".into())
    } else {
        Ok(())
    }
}
