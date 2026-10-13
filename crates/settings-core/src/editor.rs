use super::{Control, DefaultValue, SectionSchema, Setting};
pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Editor",
    validate: Some(validate),
    settings: &[
        Setting {
            id: "editor.auto_indent",
            default: DefaultValue::Bool(true),
            name: "Auto Indent",
            category: "Editor",
            help: "Carry line indentation forward on Enter and indent after an opening bracket.",
            aliases: "newline enter indent",
            control: Control::Bool,
            document: true,
        },
        Setting {
            id: "editor.detect_indentation",
            default: DefaultValue::Bool(true),
            name: "Detect Indentation",
            category: "Editor",
            help: "Infer tabs or spaces and indentation width from the first 200 lines of a file.",
            aliases: "tab spaces indentation detection",
            control: Control::Bool,
            document: true,
        },
        Setting {
            id: "editor.indent_size",
            default: DefaultValue::Number(4.0),
            name: "Indent Size",
            category: "Editor",
            help: "Number of spaces for Tab and new indentation when detection is disabled or inconclusive.",
            aliases: "tab size width spaces",
            control: Control::Number(1.0, 8.0),
            document: true,
        },
        Setting {
            id: "editor.insert_spaces",
            default: DefaultValue::Bool(true),
            name: "Insert Spaces",
            category: "Editor",
            help: "Use spaces instead of tab characters when indentation detection is disabled or inconclusive.",
            aliases: "tabs spaces whitespace",
            control: Control::Bool,
            document: true,
        },
        Setting {
            id: "editor.syntax_highlighting",
            default: DefaultValue::Bool(true),
            name: "Syntax Highlighting",
            category: "Editor",
            help: "Color recognized code tokens without changing document content.",
            aliases: "code color language tokens",
            control: Control::Bool,
            document: true,
        },
        Setting {
            id: "editor.font_family",
            default: DefaultValue::Choice("platform"),
            name: "Font Family",
            category: "Editor",
            help: "Platform monospace font or bundled Hack. Embedded fallbacks remain available.",
            aliases: "typeface text font",
            control: Control::Choice(&["platform", "Hack"]),
            document: true,
        },
        Setting {
            id: "editor.font_size",
            default: DefaultValue::Number(14.0),
            name: "Font Size",
            category: "Editor",
            help: "Editor text size in logical pixels. Does not change document content.",
            aliases: "text size typography",
            control: Control::Number(8.0, 48.0),
            document: true,
        },
        Setting {
            id: "editor.line_height",
            default: DefaultValue::Number(18.0),
            name: "Line Height",
            category: "Editor",
            help: "Row height in logical pixels; must be at least the font size.",
            aliases: "line spacing typography",
            control: Control::Number(8.0, 80.0),
            document: true,
        },
    ],
};

fn validate(layer: &super::Layer) -> Result<(), String> {
    let number = |id: &str| match &layer[id] {
        super::Value::Number(value) => *value,
        _ => unreachable!("validated schema type"),
    };
    if number("editor.indent_size").fract() != 0.0 {
        Err("Indent size must be a whole number.".into())
    } else if number("editor.line_height") < number("editor.font_size") {
        Err("Line height must be at least the font size (including inherited values).".into())
    } else {
        Ok(())
    }
}
