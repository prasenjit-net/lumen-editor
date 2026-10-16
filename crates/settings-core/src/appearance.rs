use super::{Control, DefaultValue, SectionSchema, Setting};
pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Appearance",
    validate: None,
    settings: &[
        Setting {
            id: "workbench.theme",
            default: DefaultValue::Choice("dark"),
            name: "Color Theme",
            category: "Appearance",
            help: "Choose dark, light, high contrast, or follow the system appearance.",
            aliases: "colors colour accessibility contrast system",
            control: Control::Choice(&["dark", "light", "high-contrast", "system"]),
            document: false,
        },
        Setting {
            id: "workbench.ui_scale",
            default: DefaultValue::Number(1.0),
            name: "UI Scale",
            category: "Appearance",
            help: "Scale the whole workbench relative to native display scaling.",
            aliases: "zoom accessibility size",
            control: Control::Number(0.75, 2.0),
            document: false,
        },
    ],
};
