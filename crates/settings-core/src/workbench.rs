use super::{Control, DefaultValue, SectionSchema, Setting};
pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Workbench",
    validate: None,
    settings: &[Setting {
        id: "workbench.sidebar_visible",
        default: DefaultValue::Bool(true),
        name: "Sidebar Visibility",
        category: "Workbench",
        help: "Initial sidebar visibility, also applied when this preference changes. The toolbar can still toggle it.",
        aliases: "explorer panel",
        control: Control::Bool,
        document: false,
    }],
};
