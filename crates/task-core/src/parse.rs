use std::collections::{BTreeMap, HashSet};

use toml_edit::{DocumentMut, Item, Table, Value};

use crate::{
    Background, BuiltinMatcher, DependsOrder, FailurePolicy, InputDef, InputKind, InstancePolicy,
    Issue, OutputReuse, Platform, PlatformOverride, Presentation, ProblemMatcherSpec, Reveal,
    SCHEMA_VERSION, SavePolicy, Severity, TaskDef, TaskFile, TaskKind, Transport,
    validate::validate_file,
};

const TOP_KEYS: &[&str] = &["schema_version", "tasks", "inputs"];
const INPUT_KEYS: &[&str] = &["id", "type", "prompt", "default", "options"];
const TASK_KEYS: &[&str] = &[
    "id",
    "label",
    "detail",
    "type",
    "command",
    "args",
    "cwd",
    "env",
    "unset_env",
    "group",
    "is_default",
    "depends_on",
    "depends_order",
    "on_failure",
    "transport",
    "save",
    "instance_policy",
    "exit_codes",
    "timeout_seconds",
    "is_background",
    "background",
    "presentation",
    "problem_matcher",
    "windows",
    "macos",
    "linux",
];
const OVERRIDE_KEYS: &[&str] = &["command", "args", "cwd", "env"];
const PRESENTATION_KEYS: &[&str] = &["reveal", "focus", "reuse", "clear"];
const BACKGROUND_KEYS: &[&str] = &["ready_pattern", "ready_timeout_seconds"];
const MATCHER_KEYS: &[&str] = &["pattern", "severity"];

/// Parse and fully validate a tasks file. Any problem (including an unknown key,
/// so a misspelled lifecycle option is never ignored) fails the whole file.
pub fn parse(text: &str) -> Result<TaskFile, Vec<Issue>> {
    let document: DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| vec![Issue::file("toml", error.to_string())])?;
    let mut issues = Vec::new();
    check_keys(document.as_table(), TOP_KEYS, None, "", &mut issues);
    match document.get("schema_version").and_then(Item::as_integer) {
        Some(SCHEMA_VERSION) => {}
        Some(other) => issues.push(Issue::file(
            "schema_version",
            format!("unsupported version {other}; this build reads version {SCHEMA_VERSION}"),
        )),
        None => issues.push(Issue::file(
            "schema_version",
            format!("required integer, expected {SCHEMA_VERSION}"),
        )),
    }
    let mut file = TaskFile::default();
    match document.get("tasks") {
        None => {}
        Some(Item::ArrayOfTables(tables)) => {
            for table in tables {
                file.tasks.push(read_task(table, &mut issues));
            }
        }
        Some(_) => issues.push(Issue::file(
            "tasks",
            "must be an array of tables ([[tasks]])",
        )),
    }
    match document.get("inputs") {
        None => {}
        Some(Item::ArrayOfTables(tables)) => {
            for table in tables {
                if let Some(input) = read_input(table, &mut issues) {
                    file.inputs.push(input);
                }
            }
        }
        Some(_) => issues.push(Issue::file(
            "inputs",
            "must be an array of tables ([[inputs]])",
        )),
    }
    if issues.is_empty() {
        issues = validate_file(&file);
    }
    if issues.is_empty() {
        Ok(file)
    } else {
        Err(issues)
    }
}

fn read_input(table: &Table, issues: &mut Vec<Issue>) -> Option<InputDef> {
    let id = table
        .get("id")
        .and_then(Item::as_str)
        .unwrap_or_default()
        .to_string();
    let label = format!("inputs.{id}");
    let mut reader = Reader {
        table,
        task: &label,
        prefix: "",
        issues,
    };
    reader.keys(INPUT_KEYS);
    let kind = reader.choice(
        "type",
        &[
            ("text", InputKind::Text),
            ("choice", InputKind::Choice),
            ("secret", InputKind::Secret),
        ],
    );
    let prompt = reader.string("prompt").map(str::to_string);
    let default = reader.string("default").map(str::to_string);
    let options = reader.strings("options").unwrap_or_default();
    if table.get("id").is_none() {
        reader.problem("id", "required");
    }
    if table.get("type").is_none() {
        reader.problem("type", "required: text, choice or secret");
    }
    Some(InputDef {
        id,
        kind: kind?,
        prompt: prompt.unwrap_or_default(),
        default,
        options,
    })
}

pub(crate) fn read_task(table: &Table, issues: &mut Vec<Issue>) -> TaskDef {
    let id = table
        .get("id")
        .and_then(Item::as_str)
        .unwrap_or_default()
        .to_string();
    let mut reader = Reader {
        table,
        task: &id,
        prefix: "",
        issues,
    };
    reader.keys(TASK_KEYS);
    let kind = match reader.string("type") {
        Some("process") => TaskKind::Process,
        Some("shell") => TaskKind::Shell,
        Some("compound") => TaskKind::Compound,
        Some(other) => {
            reader.problem(
                "type",
                format!("'{other}' is not process, shell or compound"),
            );
            TaskKind::Process
        }
        None => {
            reader.problem("type", "required: process, shell or compound");
            TaskKind::Process
        }
    };
    let label = reader.string("label").map(str::to_string);
    let mut def = TaskDef::new(id.clone(), label.unwrap_or_else(|| id.clone()), kind);
    if table.get("label").is_none() {
        reader.problem("label", "required");
    }
    def.detail = reader.string("detail").map(str::to_string);
    def.command = reader.string("command").map(str::to_string);
    def.args = reader.strings("args").unwrap_or_default();
    def.cwd = reader.string("cwd").map(str::to_string);
    def.env = reader.string_map("env");
    def.unset_env = reader.strings("unset_env").unwrap_or_default();
    def.group = reader.string("group").map(str::to_string);
    def.is_default = reader.boolean("is_default").unwrap_or(false);
    def.depends_on = reader.strings("depends_on").unwrap_or_default();
    def.depends_order = reader.choice(
        "depends_order",
        &[
            ("sequence", DependsOrder::Sequence),
            ("parallel", DependsOrder::Parallel),
        ],
    );
    def.on_failure = reader.choice(
        "on_failure",
        &[
            ("fail_fast", FailurePolicy::FailFast),
            ("continue", FailurePolicy::Continue),
        ],
    );
    def.transport = reader.choice(
        "transport",
        &[("pipe", Transport::Pipe), ("pty", Transport::Pty)],
    );
    def.save = reader.choice(
        "save",
        &[
            ("prompt", SavePolicy::Prompt),
            ("save", SavePolicy::Save),
            ("none", SavePolicy::None),
        ],
    );
    def.instance_policy = reader.choice(
        "instance_policy",
        &[
            ("single", InstancePolicy::Single),
            ("parallel", InstancePolicy::Parallel),
        ],
    );
    def.exit_codes = reader.integers("exit_codes");
    def.timeout_seconds = reader.unsigned("timeout_seconds");
    def.is_background = reader.boolean("is_background").unwrap_or(false);
    if let Some(sub) = reader.subtable("background") {
        def.background = read_background(sub, &id, reader.issues);
    }
    if let Some(sub) = reader.subtable("presentation") {
        def.presentation = read_presentation(sub, &id, reader.issues);
    }
    def.problem_matcher = read_matcher(table, &id, reader.issues);
    for platform in Platform::ALL {
        if let Some(sub) = reader.subtable(platform.key()) {
            *def.platform_override_mut(platform) =
                read_override(sub, &id, platform.key(), reader.issues);
        }
    }
    def
}

fn read_matcher(table: &Table, task: &str, issues: &mut Vec<Issue>) -> Option<ProblemMatcherSpec> {
    let item = table.get("problem_matcher")?;
    let mut reader = Reader {
        table,
        task,
        prefix: "",
        issues,
    };
    match item {
        Item::Value(Value::String(name)) => {
            let found = BuiltinMatcher::ALL
                .iter()
                .find(|builtin| builtin.as_str() == name.value());
            if found.is_none() {
                let names: Vec<_> = BuiltinMatcher::ALL.iter().map(|b| b.as_str()).collect();
                reader.problem(
                    "problem_matcher",
                    format!(
                        "'{}' is not a built-in matcher ({}); use a [tasks.problem_matcher] table for a custom pattern",
                        name.value(),
                        names.join(", ")
                    ),
                );
            }
            found.map(|builtin| ProblemMatcherSpec::Builtin(*builtin))
        }
        Item::Table(sub) => {
            let mut inner = Reader {
                table: sub,
                task,
                prefix: "problem_matcher.",
                issues: reader.issues,
            };
            inner.keys(MATCHER_KEYS);
            let pattern = inner.string("pattern").map(str::to_string);
            let severity = inner.choice(
                "severity",
                &[
                    ("error", Severity::Error),
                    ("warning", Severity::Warning),
                    ("info", Severity::Info),
                ],
            );
            match pattern {
                Some(pattern) => Some(ProblemMatcherSpec::Pattern { pattern, severity }),
                None => {
                    inner.problem("pattern", "required");
                    None
                }
            }
        }
        _ => {
            reader.problem(
                "problem_matcher",
                "must be a built-in name or a table with a pattern",
            );
            None
        }
    }
}

fn read_background(table: &Table, task: &str, issues: &mut Vec<Issue>) -> Option<Background> {
    let mut reader = Reader {
        table,
        task,
        prefix: "background.",
        issues,
    };
    reader.keys(BACKGROUND_KEYS);
    let pattern = reader.string("ready_pattern").map(str::to_string);
    let timeout = reader.unsigned("ready_timeout_seconds");
    match pattern {
        Some(ready_pattern) => Some(Background {
            ready_pattern,
            ready_timeout_seconds: timeout,
        }),
        None => {
            reader.problem("ready_pattern", "required in [tasks.background]");
            None
        }
    }
}

fn read_presentation(table: &Table, task: &str, issues: &mut Vec<Issue>) -> Presentation {
    let mut reader = Reader {
        table,
        task,
        prefix: "presentation.",
        issues,
    };
    reader.keys(PRESENTATION_KEYS);
    Presentation {
        reveal: reader.choice(
            "reveal",
            &[
                ("always", Reveal::Always),
                ("silent", Reveal::Silent),
                ("never", Reveal::Never),
            ],
        ),
        focus: reader.boolean("focus"),
        reuse: reader.choice(
            "reuse",
            &[
                ("dedicated", OutputReuse::Dedicated),
                ("new", OutputReuse::New),
            ],
        ),
        clear: reader.boolean("clear"),
    }
}

fn read_override(
    table: &Table,
    task: &str,
    prefix: &str,
    issues: &mut Vec<Issue>,
) -> PlatformOverride {
    let prefix = format!("{prefix}.");
    let mut reader = Reader {
        table,
        task,
        prefix: &prefix,
        issues,
    };
    reader.keys(OVERRIDE_KEYS);
    PlatformOverride {
        command: reader.string("command").map(str::to_string),
        args: reader.strings("args"),
        cwd: reader.string("cwd").map(str::to_string),
        env: reader.string_map("env"),
    }
}

struct Reader<'t, 'i> {
    table: &'t Table,
    task: &'t str,
    prefix: &'t str,
    issues: &'i mut Vec<Issue>,
}

impl<'t> Reader<'t, '_> {
    fn problem(&mut self, field: &str, message: impl Into<String>) {
        self.issues.push(Issue::task(
            self.task,
            &format!("{}{field}", self.prefix),
            message,
        ));
    }

    fn keys(&mut self, allowed: &[&str]) {
        check_keys(
            self.table,
            allowed,
            Some(self.task),
            self.prefix,
            self.issues,
        );
    }

    fn string(&mut self, key: &str) -> Option<&'t str> {
        match self.table.get(key)? {
            Item::Value(Value::String(value)) => Some(value.value().as_str()),
            _ => {
                self.problem(key, "must be a string");
                None
            }
        }
    }

    fn boolean(&mut self, key: &str) -> Option<bool> {
        let item = self.table.get(key)?;
        let value = item.as_bool();
        if value.is_none() {
            self.problem(key, "must be true or false");
        }
        value
    }

    fn unsigned(&mut self, key: &str) -> Option<u32> {
        let item = self.table.get(key)?;
        match item
            .as_integer()
            .and_then(|value| u32::try_from(value).ok())
        {
            Some(value) => Some(value),
            None => {
                self.problem(key, "must be a non-negative integer");
                None
            }
        }
    }

    fn strings(&mut self, key: &str) -> Option<Vec<String>> {
        let item = self.table.get(key)?;
        let array = item
            .as_array()
            .filter(|array| array.iter().all(|value| matches!(value, Value::String(_))));
        match array {
            Some(array) => Some(
                array
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect(),
            ),
            None => {
                self.problem(key, "must be an array of strings");
                None
            }
        }
    }

    fn integers(&mut self, key: &str) -> Option<Vec<i32>> {
        let item = self.table.get(key)?;
        let codes = item.as_array().and_then(|array| {
            array
                .iter()
                .map(|value| value.as_integer().and_then(|n| i32::try_from(n).ok()))
                .collect::<Option<Vec<_>>>()
        });
        if codes.is_none() {
            self.problem(key, "must be an array of integers");
        }
        codes
    }

    fn string_map(&mut self, key: &str) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let Some(item) = self.table.get(key) else {
            return map;
        };
        let Some(table) = item.as_table_like() else {
            self.problem(key, "must be a table of strings");
            return map;
        };
        for (name, value) in table.iter() {
            match value.as_str() {
                Some(text) => {
                    map.insert(name.to_string(), text.to_string());
                }
                None => self.problem(&format!("{key}.{name}"), "must be a string"),
            }
        }
        map
    }

    fn subtable(&mut self, key: &str) -> Option<&'t Table> {
        match self.table.get(key)? {
            Item::Table(table) => Some(table),
            _ => {
                self.problem(key, "must be a table");
                None
            }
        }
    }

    fn choice<T: Copy>(&mut self, key: &str, options: &[(&str, T)]) -> Option<T> {
        let text = self.string(key)?.to_string();
        match options.iter().find(|(name, _)| *name == text) {
            Some((_, value)) => Some(*value),
            None => {
                let names: Vec<_> = options.iter().map(|(name, _)| *name).collect();
                self.problem(key, format!("'{text}' is not one of: {}", names.join(", ")));
                None
            }
        }
    }
}

fn check_keys(
    table: &Table,
    allowed: &[&str],
    task: Option<&str>,
    prefix: &str,
    issues: &mut Vec<Issue>,
) {
    let allowed: HashSet<&str> = allowed.iter().copied().collect();
    for (key, _) in table.iter() {
        if !allowed.contains(key) {
            let message = format!("unknown key '{key}'");
            let field = format!("{prefix}{key}");
            issues.push(match task {
                Some(task) => Issue::task(task, &field, message),
                None => Issue::file(&field, message),
            });
        }
    }
}
