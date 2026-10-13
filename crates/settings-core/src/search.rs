use super::{Control, DefaultValue, SectionSchema, Setting};

pub(crate) const SCHEMA: SectionSchema = SectionSchema {
    title: "Search",
    validate: None,
    settings: &[Setting {
        id: "search.exclude",
        name: "Exclude Patterns",
        category: "Search",
        help: "Comma-separated globs excluded from workspace search. Project overrides User; the Search panel's exclude field adds temporary patterns.",
        aliases: "ignore skip glob files folders",
        control: Control::Text(4096),
        document: false,
        default: DefaultValue::Text(""),
    }],
};
