use super::{Control, DefaultValue, SectionSchema, Setting};
pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Editor",
    validate: Some(validate),
    settings: &[
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
    if number("editor.line_height") < number("editor.font_size") {
        Err("Line height must be at least the font size (including inherited values).".into())
    } else {
        Ok(())
    }
}
