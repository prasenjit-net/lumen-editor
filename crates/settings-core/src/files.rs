use super::{Control, DefaultValue, SectionSchema, Setting};

pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Files",
    validate: None,
    settings: &[
        Setting {
            id: "files.auto_save",
            name: "Auto Save",
            category: "Files",
            help: "Save changed files after a delay, when the editor loses focus, or when the window loses focus. Untitled files still require Save As.",
            aliases: "autosave save focus window",
            control: Control::Choice(&["off", "afterDelay", "onFocusChange", "onWindowChange"]),
            document: true,
            default: DefaultValue::Choice("off"),
        },
        Setting {
            id: "files.auto_save_delay",
            name: "Auto Save Delay",
            category: "Files",
            help: "Milliseconds after the latest edit before saving, when Auto Save is set to afterDelay.",
            aliases: "autosave timer debounce",
            control: Control::Number(100.0, 60_000.0),
            document: true,
            default: DefaultValue::Number(1000.0),
        },
    ],
};
