//! Minimal, comment-preserving edits of a tasks file: only the touched task's
//! known keys change; comments, key order, unknown keys and other tasks stay.

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::{
    Background, Issue, Platform, PlatformOverride, Presentation, SCHEMA_VERSION, TaskDef,
    validate::validate_task,
};

#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    /// The existing text is not valid TOML or has a malformed `tasks` entry;
    /// the file is never rewritten blindly.
    Unreadable(String),
    /// The definition fails validation, so nothing is written.
    Invalid(Vec<Issue>),
    NotFound(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(message) => write!(f, "tasks file is unreadable: {message}"),
            Self::Invalid(issues) => {
                let text: Vec<_> = issues.iter().map(ToString::to_string).collect();
                write!(f, "{}", text.join("; "))
            }
            Self::NotFound(id) => write!(f, "no task with id '{id}'"),
        }
    }
}

/// A new file containing only `task`.
pub fn new_file(task: &TaskDef) -> Result<String, EditError> {
    upsert_task(&format!("schema_version = {SCHEMA_VERSION}\n"), task)
}

/// Replace the task with `task.id`, or append it when absent.
pub fn upsert_task(text: &str, task: &TaskDef) -> Result<String, EditError> {
    let issues = validate_task(task);
    if !issues.is_empty() {
        return Err(EditError::Invalid(issues));
    }
    let mut document = open(text)?;
    let tables = tasks_mut(&mut document)?;
    let existing = tables
        .iter_mut()
        .find(|table| table.get("id").and_then(Item::as_str) == Some(task.id.as_str()));
    match existing {
        Some(table) => write_task(table, task),
        None => {
            let mut table = Table::new();
            write_task(&mut table, task);
            tables.push(table);
        }
    }
    Ok(document.to_string())
}

/// IDs of every task table in `text`, readable or not by `parse`'s stricter rules.
pub fn existing_ids(text: &str) -> Result<Vec<String>, EditError> {
    let document = open(text)?;
    match document.get("tasks") {
        None => Ok(Vec::new()),
        Some(Item::ArrayOfTables(tables)) => Ok(tables
            .iter()
            .filter_map(|table| table.get("id").and_then(Item::as_str).map(str::to_string))
            .collect()),
        Some(_) => Err(EditError::Unreadable(
            "'tasks' must be an array of tables".into(),
        )),
    }
}

pub fn remove_task(text: &str, id: &str) -> Result<String, EditError> {
    let mut document = open(text)?;
    let tables = tasks_mut(&mut document)?;
    let index = tables
        .iter()
        .position(|table| table.get("id").and_then(Item::as_str) == Some(id))
        .ok_or_else(|| EditError::NotFound(id.into()))?;
    tables.remove(index);
    Ok(document.to_string())
}

fn open(text: &str) -> Result<DocumentMut, EditError> {
    text.parse()
        .map_err(|error: toml_edit::TomlError| EditError::Unreadable(error.to_string()))
}

fn tasks_mut(document: &mut DocumentMut) -> Result<&mut ArrayOfTables, EditError> {
    if document.get("tasks").is_none() {
        document["tasks"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    match document.get_mut("tasks") {
        Some(Item::ArrayOfTables(tables)) => Ok(tables),
        _ => Err(EditError::Unreadable(
            "'tasks' must be an array of tables".into(),
        )),
    }
}

fn write_task(table: &mut Table, task: &TaskDef) {
    set_str(table, "id", Some(&task.id));
    set_str(table, "label", Some(&task.label));
    set_str(table, "detail", task.detail.as_deref());
    set_str(table, "type", Some(task.kind.as_str()));
    set_str(table, "command", task.command.as_deref());
    set_strings(
        table,
        "args",
        (!task.args.is_empty()).then_some(&task.args[..]),
    );
    set_str(table, "cwd", task.cwd.as_deref());
    set_map(table, "env", &task.env);
    set_strings(
        table,
        "unset_env",
        (!task.unset_env.is_empty()).then_some(&task.unset_env[..]),
    );
    set_str(table, "group", task.group.as_deref());
    set_bool(table, "is_default", task.is_default.then_some(true));
    set_strings(
        table,
        "depends_on",
        (!task.depends_on.is_empty()).then_some(&task.depends_on[..]),
    );
    set_str(
        table,
        "depends_order",
        task.depends_order.map(|v| v.as_str()),
    );
    set_str(table, "transport", task.transport.map(|v| v.as_str()));
    set_str(table, "save", task.save.map(|v| v.as_str()));
    set_str(
        table,
        "instance_policy",
        task.instance_policy.map(|v| v.as_str()),
    );
    set_ints(table, "exit_codes", task.exit_codes.as_deref());
    set_int(table, "timeout_seconds", task.timeout_seconds);
    set_bool(table, "is_background", task.is_background.then_some(true));
    write_background(table, task.background.as_ref());
    write_presentation(table, &task.presentation);
    for platform in Platform::ALL {
        write_override(table, platform.key(), task.platform_override(platform));
    }
}

fn write_background(table: &mut Table, background: Option<&Background>) {
    match background {
        None => {
            table.remove("background");
        }
        Some(background) => {
            let sub = sub_table(table, "background");
            set_str(sub, "ready_pattern", Some(&background.ready_pattern));
            set_int(
                sub,
                "ready_timeout_seconds",
                background.ready_timeout_seconds,
            );
        }
    }
}

fn write_presentation(table: &mut Table, presentation: &Presentation) {
    if *presentation == Presentation::default() {
        table.remove("presentation");
        return;
    }
    let sub = sub_table(table, "presentation");
    set_str(sub, "reveal", presentation.reveal.map(|v| v.as_str()));
    set_bool(sub, "focus", presentation.focus);
    set_str(sub, "reuse", presentation.reuse.map(|v| v.as_str()));
    set_bool(sub, "clear", presentation.clear);
}

fn write_override(table: &mut Table, key: &str, over: &PlatformOverride) {
    if over.is_empty() {
        table.remove(key);
        return;
    }
    let sub = sub_table(table, key);
    set_str(sub, "command", over.command.as_deref());
    set_strings(sub, "args", over.args.as_deref());
    set_str(sub, "cwd", over.cwd.as_deref());
    set_map(sub, "env", &over.env);
}

fn sub_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !matches!(table.get(key), Some(Item::Table(_))) {
        table.insert(key, Item::Table(Table::new()));
    }
    table.get_mut(key).and_then(Item::as_table_mut).unwrap()
}

// Scalar setters skip an unchanged value so its comments/formatting survive.

fn set_str(table: &mut Table, key: &str, new: Option<&str>) {
    match new {
        None => {
            table.remove(key);
        }
        Some(new) if table.get(key).and_then(Item::as_str) == Some(new) => {}
        Some(new) => {
            table.insert(key, value(new));
        }
    }
}

fn set_bool(table: &mut Table, key: &str, new: Option<bool>) {
    match new {
        None => {
            table.remove(key);
        }
        Some(new) if table.get(key).and_then(Item::as_bool) == Some(new) => {}
        Some(new) => {
            table.insert(key, value(new));
        }
    }
}

fn set_int(table: &mut Table, key: &str, new: Option<u32>) {
    match new {
        None => {
            table.remove(key);
        }
        Some(new) if table.get(key).and_then(Item::as_integer) == Some(i64::from(new)) => {}
        Some(new) => {
            table.insert(key, value(i64::from(new)));
        }
    }
}

fn set_strings<S: AsRef<str>>(table: &mut Table, key: &str, new: Option<&[S]>) {
    match new {
        None => {
            table.remove(key);
        }
        Some(new) => {
            let same = table
                .get(key)
                .and_then(Item::as_array)
                .is_some_and(|array| {
                    array.len() == new.len()
                        && array
                            .iter()
                            .zip(new)
                            .all(|(old, new)| old.as_str() == Some(new.as_ref()))
                });
            if !same {
                let array: Array = new.iter().map(|text| text.as_ref()).collect();
                table.insert(key, value(array));
            }
        }
    }
}

fn set_ints(table: &mut Table, key: &str, new: Option<&[i32]>) {
    match new {
        None => {
            table.remove(key);
        }
        Some(new) => {
            let same = table
                .get(key)
                .and_then(Item::as_array)
                .is_some_and(|array| {
                    array.len() == new.len()
                        && array
                            .iter()
                            .zip(new)
                            .all(|(old, new)| old.as_integer() == Some(i64::from(*new)))
                });
            if !same {
                let array: Array = new.iter().map(|code| i64::from(*code)).collect();
                table.insert(key, value(array));
            }
        }
    }
}

fn set_map(table: &mut Table, key: &str, new: &std::collections::BTreeMap<String, String>) {
    if new.is_empty() {
        table.remove(key);
        return;
    }
    let sub = sub_table(table, key);
    let stale: Vec<String> = sub
        .iter()
        .map(|(name, _)| name.to_string())
        .filter(|name| !new.contains_key(name))
        .collect();
    for name in stale {
        sub.remove(&name);
    }
    for (name, text) in new {
        set_str(sub, name, Some(text));
    }
}
