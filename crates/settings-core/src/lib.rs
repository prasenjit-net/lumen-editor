//! UI-independent settings schemas, validation, search and layer resolution.
//! Platform paths, disk persistence, editable drafts and rendering belong to consumers.
mod codec;
pub use codec::{parse, serialize};
use std::collections::BTreeMap;
mod agents;
mod appearance;
mod editor;
mod explorer;
mod files;
mod language;
mod search;
mod terminal;
mod workbench;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    User,
    Project,
}
impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Project => "Project",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Number(f64),
    Bool(bool),
    Choice(String),
    Text(String),
}
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(f, "{value}"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Choice(value) | Self::Text(value) => write!(f, "{value}"),
        }
    }
}
#[derive(Clone, Copy)]
pub enum Control {
    Number(f64, f64),
    Bool,
    Choice(&'static [&'static str]),
    Text(usize),
}
pub struct Setting {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub help: &'static str,
    pub aliases: &'static str,
    pub control: Control,
    pub document: bool,
    pub default: DefaultValue,
}
#[derive(Clone, Copy)]
pub enum DefaultValue {
    Number(f64),
    Bool(bool),
    Choice(&'static str),
    Text(&'static str),
}
pub type SectionValidator = fn(&Layer) -> Result<(), String>;
pub struct SectionSchema {
    pub title: &'static str,
    pub settings: &'static [Setting],
    pub validate: Option<SectionValidator>,
}

pub const SECTIONS: &[SectionSchema] = &[
    editor::SCHEMA,
    files::SCHEMA,
    appearance::SCHEMA,
    workbench::SCHEMA,
    explorer::SCHEMA,
    language::SCHEMA,
    agents::SCHEMA,
    search::SCHEMA,
    terminal::SCHEMA,
];
pub fn catalog() -> impl Iterator<Item = &'static Setting> {
    SECTIONS.iter().flat_map(|section| section.settings.iter())
}
pub fn setting(id: &str) -> &'static Setting {
    catalog()
        .find(|setting| setting.id == id)
        .expect("known setting ID")
}
pub type Layer = BTreeMap<String, Value>;
impl Setting {
    pub fn default_value(&self) -> Value {
        match self.default {
            DefaultValue::Choice(value) => Value::Choice(value.into()),
            DefaultValue::Number(value) => Value::Number(value),
            DefaultValue::Bool(value) => Value::Bool(value),
            DefaultValue::Text(value) => Value::Text(value.into()),
        }
    }

    pub fn validate(&self, value: &Value) -> Result<(), String> {
        let valid = match (self.control, value) {
            (Control::Number(min, max), Value::Number(value)) => {
                value.is_finite() && (min..=max).contains(value)
            }
            (Control::Bool, Value::Bool(_)) => true,
            (Control::Choice(choices), Value::Choice(value)) => choices.contains(&value.as_str()),
            (Control::Text(max_length), Value::Text(value)) => {
                value.len() <= max_length && !value.chars().any(char::is_control)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(format!("Invalid value for {}", self.id))
        }
    }
    pub fn matches(&self, query: &str, persisted: &Layer, pending: bool) -> bool {
        let searchable = format!(
            "{} {} {} {} {}",
            self.id, self.name, self.category, self.help, self.aliases
        )
        .to_lowercase();
        query.to_lowercase().split_whitespace().all(|token| {
            if token == "@modified" {
                persisted.contains_key(self.id)
            } else if token == "@pending" {
                pending
            } else if let Some(id) = token.strip_prefix("@id:") {
                self.id == id
            } else {
                searchable.contains(token)
            }
        })
    }
}
pub fn resolve(setting: &Setting, user: &Layer, project: Option<&Layer>) -> (Value, &'static str) {
    if let Some(value) = project.and_then(|layer| layer.get(setting.id)) {
        (value.clone(), "Project")
    } else if let Some(value) = user.get(setting.id) {
        (value.clone(), "User")
    } else {
        (setting.default_value(), "Default")
    }
}
pub fn validate_layers(user: &Layer, project: Option<&Layer>) -> Result<(), String> {
    for layer in [Some(user), project].into_iter().flatten() {
        for setting in catalog() {
            if let Some(value) = layer.get(setting.id) {
                setting.validate(value)?;
            }
        }
    }
    let effective = catalog()
        .map(|setting| (setting.id.to_string(), resolve(setting, user, project).0))
        .collect();
    for section in SECTIONS {
        if let Some(validate) = section.validate {
            validate(&effective)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_section_has_unique_ids_valid_defaults_and_matching_categories() {
        let mut ids = std::collections::BTreeSet::new();
        for section in SECTIONS {
            for setting in section.settings {
                assert!(ids.insert(setting.id), "duplicate setting ID");
                assert_eq!(setting.category, section.title);
                assert!(setting.validate(&setting.default_value()).is_ok());
            }
        }
        assert!(validate_layers(&Layer::new(), None).is_ok());
    }
    #[test]
    fn language_server_settings_default_on_and_validate_bounds() {
        assert_eq!(
            resolve(setting("language.rust.enabled"), &Layer::new(), None).0,
            Value::Bool(true)
        );
        let timeout = setting("language.server.startup_timeout_seconds");
        assert!(timeout.validate(&Value::Number(0.0)).is_err());
        assert!(timeout.validate(&Value::Number(30.0)).is_ok());
        let path = setting("language.go.server_path");
        assert!(path.validate(&Value::Text("/usr/bin/gopls".into())).is_ok());
        assert!(path.validate(&Value::Text("gopls\n--evil".into())).is_err());
    }
    #[test]
    fn agent_settings_default_to_auto_launch_with_no_custom_command() {
        assert_eq!(
            resolve(setting("agent.claude.launch"), &Layer::new(), None).0,
            Value::Choice("auto".into())
        );
        assert_eq!(
            resolve(setting("agent.codex.command"), &Layer::new(), None).0,
            Value::Text(String::new())
        );
        assert!(
            setting("agent.codex.launch")
                .validate(&Value::Choice("curl-pipe-sh".into()))
                .is_err()
        );
    }
    #[test]
    fn precedence_removal_and_explicit_default_are_distinct() {
        let setting = setting("editor.font_size");
        let user = Layer::from([(setting.id.into(), Value::Number(16.0))]);
        let mut project = Layer::from([(setting.id.into(), setting.default_value())]);
        assert_eq!(
            resolve(setting, &user, Some(&project)),
            (Value::Number(14.0), "Project")
        );
        assert!(setting.matches("@modified text size", &project, false));
        project.clear();
        assert_eq!(
            resolve(setting, &user, Some(&project)),
            (Value::Number(16.0), "User")
        );
        assert!(!setting.matches("@modified", &project, false));
        assert!(setting.matches("@id:editor.font_size", &project, false));
        assert!(!setting.matches("@id:editor.line_height", &project, false));
    }
    #[test]
    fn invalid_numbers_types_and_inherited_geometry_are_rejected() {
        assert!(
            validate_layers(
                &Layer::from([("editor.indent_size".into(), Value::Number(2.5))]),
                None
            )
            .is_err()
        );
        assert!(
            setting("editor.font_size")
                .validate(&Value::Number(f64::NAN))
                .is_err()
        );
        assert!(
            setting("editor.font_size")
                .validate(&Value::Bool(true))
                .is_err()
        );
        let user = Layer::from([("editor.font_size".into(), Value::Number(24.0))]);
        assert!(validate_layers(&user, None).is_err());
        let project = Layer::from([("editor.line_height".into(), Value::Number(30.0))]);
        assert!(validate_layers(&user, Some(&project)).is_ok());
        let terminal = Layer::from([
            ("terminal.font_size".into(), Value::Number(20.0)),
            ("terminal.line_height".into(), Value::Number(18.0)),
        ]);
        assert!(validate_layers(&terminal, None).is_err());
    }
}
