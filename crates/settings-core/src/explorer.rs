use super::{Control, DefaultValue, SectionSchema, Setting};
pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Explorer",
    validate: None,
    settings: &[Setting {
        id: "explorer.show_hidden",
        default: DefaultValue::Bool(true),
        name: "Show Hidden Files",
        category: "Explorer",
        help: "Show entries whose names start with a dot. This only affects visibility, never file operations.",
        aliases: "dotfiles files folders",
        control: Control::Bool,
        document: false,
    }],
};
