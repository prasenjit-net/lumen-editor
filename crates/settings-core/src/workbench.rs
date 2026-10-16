use super::{Control, DefaultValue, SectionSchema, Setting};
pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Workbench",
    validate: None,
    settings: &[
        Setting {
            id: "workbench.sidebar_visible",
            default: DefaultValue::Bool(true),
            name: "Sidebar Visibility",
            category: "Workbench",
            help: "Initial sidebar visibility, also applied when this preference changes. The toolbar can still toggle it.",
            aliases: "explorer panel",
            control: Control::Bool,
            document: false,
        },
        Setting {
            id: "workbench.reopen_last_folder",
            default: DefaultValue::Bool(false),
            name: "Reopen Last Folder",
            category: "Workbench",
            help: "On startup, reopen the most recently opened folder when no folder or file was passed on the command line. A folder that no longer exists is reported and skipped.",
            aliases: "restore startup recent workspace",
            control: Control::Bool,
            document: false,
        },
    ],
};
