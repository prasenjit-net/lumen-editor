use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    Issue, MAX_ID_LEN, MAX_READY_TIMEOUT_SECONDS, Platform, TaskDef, TaskFile, TaskKind,
    vars::find_variables,
};

const MAX_TEXT: usize = 8192;
const MAX_ARGS: usize = 256;
const MAX_ENV: usize = 256;

pub(crate) fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// How a `depends_on` entry refers to a task.
pub(crate) enum Reference<'a> {
    SameScope(&'a str),
    User(&'a str),
}

pub(crate) fn reference(text: &str) -> Reference<'_> {
    match text.strip_prefix("user:") {
        Some(id) => Reference::User(id),
        None => Reference::SameScope(text),
    }
}

/// Checks that need only this definition. Never touches the filesystem.
pub fn validate_task(def: &TaskDef) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut problem = |field: &str, message: String| {
        issues.push(Issue::task(&def.id, field, message));
    };
    if !valid_id(&def.id) {
        problem(
            "id",
            format!(
                "must be 1-{MAX_ID_LEN} letters, digits, '.', '_' or '-', starting with a letter or digit"
            ),
        );
    }
    if def.label.trim().is_empty() {
        problem("label", "must not be empty".into());
    } else if def.label.chars().any(char::is_control) {
        problem("label", "must not contain control characters".into());
    }
    if let Some(group) = &def.group
        && !valid_id(group)
    {
        problem(
            "group",
            "must be a short lowercase name such as build or test".into(),
        );
    }
    if def.is_default && def.group.is_none() {
        problem("is_default", "needs a group to be the default of".into());
    }
    if def.exit_codes.as_ref().is_some_and(Vec::is_empty) {
        problem("exit_codes", "must list at least one code".into());
    }
    if def.timeout_seconds == Some(0) {
        problem("timeout_seconds", "must be greater than zero".into());
    }

    match def.kind {
        TaskKind::Compound => compound_rules(def, &mut problem),
        _ => execution_rules(def, &mut problem),
    }
    if def.on_failure.is_some() && def.depends_on.is_empty() {
        problem(
            "on_failure",
            "only applies to a task with depends_on".into(),
        );
    }
    matcher_rules(def, &mut problem);
    background_rules(def, &mut problem);
    dependency_syntax(def, &mut problem);
    issues
}

fn compound_rules(def: &TaskDef, problem: &mut impl FnMut(&str, String)) {
    if def.depends_on.is_empty() {
        problem(
            "depends_on",
            "a compound task needs at least one dependency".into(),
        );
    }
    let stray: [(&str, bool); 9] = [
        ("command", def.command.is_some()),
        ("args", !def.args.is_empty()),
        ("cwd", def.cwd.is_some()),
        ("env", !def.env.is_empty()),
        ("unset_env", !def.unset_env.is_empty()),
        ("transport", def.transport.is_some()),
        ("exit_codes", def.exit_codes.is_some()),
        ("timeout_seconds", def.timeout_seconds.is_some()),
        ("save", def.save.is_some()),
    ];
    for (field, present) in stray {
        if present {
            problem(field, "is not used by compound tasks".into());
        }
    }
    if Platform::ALL
        .iter()
        .any(|platform| !def.platform_override(*platform).is_empty())
    {
        problem(
            "platform",
            "compound tasks have no execution fields to override".into(),
        );
    }
    if def.is_background || def.background.is_some() {
        problem(
            "background",
            "compound tasks cannot be background tasks".into(),
        );
    }
}

fn execution_rules(def: &TaskDef, problem: &mut impl FnMut(&str, String)) {
    match def.command.as_deref().map(str::trim) {
        None | Some("") => problem("command", "must not be empty".into()),
        Some(command) => text_rules("command", command, problem),
    }
    if def.kind == TaskKind::Shell && !def.args.is_empty() {
        problem(
            "args",
            "shell tasks run one command string; put arguments in 'command' or use a process task"
                .into(),
        );
    }
    if def.args.len() > MAX_ARGS {
        problem("args", format!("at most {MAX_ARGS} arguments"));
    }
    for (index, arg) in def.args.iter().enumerate() {
        text_rules(&format!("args[{index}]"), arg, problem);
    }
    if let Some(cwd) = &def.cwd {
        text_rules("cwd", cwd, problem);
    }
    env_rules("env", &def.env, problem);
    for name in &def.unset_env {
        if !valid_env_name(name) {
            problem(
                "unset_env",
                format!("'{name}' is not a valid environment variable name"),
            );
        }
        if def.env.contains_key(name) {
            problem("unset_env", format!("'{name}' is both set and unset"));
        }
    }
    for platform in Platform::ALL {
        let over = def.platform_override(platform);
        let key = platform.key();
        if let Some(command) = &over.command {
            if command.trim().is_empty() {
                problem(&format!("{key}.command"), "must not be empty".into());
            } else {
                text_rules(&format!("{key}.command"), command, problem);
            }
        }
        if over.args.is_some() && def.kind == TaskKind::Shell {
            problem(
                &format!("{key}.args"),
                "shell tasks have no arguments".into(),
            );
        }
        for (index, arg) in over.args.iter().flatten().enumerate() {
            text_rules(&format!("{key}.args[{index}]"), arg, problem);
        }
        if let Some(cwd) = &over.cwd {
            text_rules(&format!("{key}.cwd"), cwd, problem);
        }
        env_rules(&format!("{key}.env"), &over.env, problem);
    }
}

fn matcher_rules(def: &TaskDef, problem: &mut impl FnMut(&str, String)) {
    match &def.problem_matcher {
        None => {}
        Some(_) if def.kind == TaskKind::Compound => problem(
            "problem_matcher",
            "compound tasks have no output of their own to scan".into(),
        ),
        Some(crate::ProblemMatcherSpec::Pattern { pattern, .. }) => {
            if let Err(message) = crate::matcher::validate_pattern(pattern) {
                problem("problem_matcher.pattern", message);
            }
        }
        Some(_) => {}
    }
}

fn background_rules(def: &TaskDef, problem: &mut impl FnMut(&str, String)) {
    if def.kind == TaskKind::Compound {
        return;
    }
    match (&def.background, def.is_background) {
        (Some(background), true) => {
            if background.ready_pattern.is_empty() {
                problem("background.ready_pattern", "must not be empty".into());
            }
            match background.ready_timeout_seconds {
                Some(0) => problem(
                    "background.ready_timeout_seconds",
                    "must be greater than zero".into(),
                ),
                Some(seconds) if seconds > MAX_READY_TIMEOUT_SECONDS => problem(
                    "background.ready_timeout_seconds",
                    format!("at most {MAX_READY_TIMEOUT_SECONDS}"),
                ),
                _ => {}
            }
        }
        (Some(_), false) => problem("background", "needs is_background = true".into()),
        (None, true) => problem(
            "background",
            "background tasks need [tasks.background] with a ready_pattern".into(),
        ),
        (None, false) => {}
    }
}

fn dependency_syntax(def: &TaskDef, problem: &mut impl FnMut(&str, String)) {
    let mut seen = HashSet::new();
    for dependency in &def.depends_on {
        let id = match reference(dependency) {
            Reference::SameScope(id) | Reference::User(id) => id,
        };
        if !valid_id(id) {
            problem(
                "depends_on",
                format!("'{dependency}' is not a valid task reference"),
            );
        } else if dependency == &def.id {
            problem("depends_on", "a task cannot depend on itself".into());
        } else if !seen.insert(dependency.as_str()) {
            problem("depends_on", format!("'{dependency}' is listed twice"));
        }
    }
}

fn text_rules(field: &str, text: &str, problem: &mut impl FnMut(&str, String)) {
    if text.len() > MAX_TEXT {
        problem(field, format!("longer than {MAX_TEXT} bytes"));
    }
    if text.contains('\0') {
        problem(field, "must not contain NUL".into());
    }
    if let Err(message) = find_variables(text) {
        problem(field, message);
    }
}

fn env_rules(field: &str, env: &BTreeMap<String, String>, problem: &mut impl FnMut(&str, String)) {
    if env.len() > MAX_ENV {
        problem(field, format!("at most {MAX_ENV} entries"));
    }
    for (name, value) in env {
        if !valid_env_name(name) {
            problem(
                field,
                format!("'{name}' is not a valid environment variable name"),
            );
        }
        text_rules(&format!("{field}.{name}"), value, problem);
    }
}

/// Every text a definition may put variables in.
fn variable_texts(def: &TaskDef) -> Vec<&str> {
    let mut texts: Vec<&str> = Vec::new();
    texts.extend(def.command.as_deref());
    texts.extend(def.args.iter().map(String::as_str));
    texts.extend(def.cwd.as_deref());
    texts.extend(def.env.values().map(String::as_str));
    for platform in Platform::ALL {
        let over = def.platform_override(platform);
        texts.extend(over.command.as_deref());
        texts.extend(over.args.iter().flatten().map(String::as_str));
        texts.extend(over.cwd.as_deref());
        texts.extend(over.env.values().map(String::as_str));
    }
    texts
}

/// The declared inputs a definition references, in first-use order, without
/// duplicates. Texts with syntax errors contribute nothing (validation reports them).
pub fn input_references(def: &TaskDef) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for text in variable_texts(def) {
        for variable in find_variables(text).unwrap_or_default() {
            if let crate::Variable::Input(id) = variable
                && !found.contains(&id)
            {
                found.push(id);
            }
        }
    }
    found
}

fn input_issues(file: &TaskFile) -> Vec<Issue> {
    use crate::InputKind;
    let mut issues = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for input in &file.inputs {
        let mut problem = |message: String| {
            issues.push(Issue::file(&format!("inputs.{}", input.id), message));
        };
        if !valid_id(&input.id) {
            problem("must be a short id of letters, digits, '.', '_' or '-'".into());
        } else if !seen.insert(input.id.as_str()) {
            problem("is declared more than once".into());
        }
        if input.prompt.trim().is_empty() {
            problem("needs a prompt to show".into());
        }
        match input.kind {
            InputKind::Choice => {
                if input.options.is_empty() {
                    problem("a choice needs at least one option".into());
                } else if input
                    .default
                    .as_ref()
                    .is_some_and(|default| !input.options.contains(default))
                {
                    problem("its default is not one of the options".into());
                }
            }
            InputKind::Secret => {
                if input.default.is_some() {
                    problem(
                        "a secret cannot have a default (it would be stored in the file)".into(),
                    );
                }
                if !input.options.is_empty() {
                    problem("only a choice has options".into());
                }
            }
            InputKind::Text => {
                if !input.options.is_empty() {
                    problem("only a choice has options".into());
                }
            }
        }
    }
    for task in &file.tasks {
        for id in input_references(task) {
            if file.input(&id).is_none() {
                issues.push(Issue::task(
                    &task.id,
                    "inputs",
                    format!("uses ${{input:{id}}}, which is not declared in [[inputs]]"),
                ));
            }
        }
    }
    issues
}

/// Per-definition checks plus everything that needs the whole file: duplicate IDs,
/// same-scope dependency targets, cycles and conflicting defaults. `user:`
/// references need both scopes, so [`crate::Catalog::validate`] checks them.
pub fn validate_file(file: &TaskFile) -> Vec<Issue> {
    let mut issues = input_issues(file);
    let mut ids: HashMap<&str, usize> = HashMap::new();
    for task in &file.tasks {
        issues.extend(validate_task(task));
        *ids.entry(task.id.as_str()).or_default() += 1;
    }
    for (id, count) in &ids {
        if *count > 1 && !id.is_empty() {
            issues.push(Issue::task(
                id,
                "id",
                format!("used by {count} tasks; IDs must be unique"),
            ));
        }
    }
    let mut defaults: HashMap<&str, &str> = HashMap::new();
    for task in file.tasks.iter().filter(|task| task.is_default) {
        let Some(group) = task.group.as_deref() else {
            continue;
        };
        if let Some(first) = defaults.insert(group, &task.id) {
            issues.push(Issue::task(
                &task.id,
                "is_default",
                format!("group '{group}' already has default task '{first}'"),
            ));
        }
    }
    for task in &file.tasks {
        for dependency in &task.depends_on {
            if let Reference::SameScope(id) = reference(dependency)
                && valid_id(id)
                && !ids.contains_key(id)
            {
                issues.push(Issue::task(
                    &task.id,
                    "depends_on",
                    format!("unknown task '{id}'"),
                ));
            }
        }
    }
    if issues.iter().all(|issue| issue.field != "depends_on") {
        issues.extend(cycles(file));
    }
    issues
}

fn cycles(file: &TaskFile) -> Vec<Issue> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(
        file: &'a TaskFile,
        id: &'a str,
        marks: &mut HashMap<&'a str, Mark>,
        stack: &mut Vec<&'a str>,
        issues: &mut Vec<Issue>,
    ) {
        match marks.get(id) {
            Some(Mark::Done) => return,
            Some(Mark::Visiting) => {
                let start = stack.iter().position(|entry| *entry == id).unwrap_or(0);
                let mut path: Vec<&str> = stack[start..].to_vec();
                path.push(id);
                issues.push(Issue::task(
                    id,
                    "depends_on",
                    format!("dependency cycle: {}", path.join(" -> ")),
                ));
                return;
            }
            None => {}
        }
        marks.insert(id, Mark::Visiting);
        stack.push(id);
        if let Some(task) = file.get(id) {
            for dependency in &task.depends_on {
                if let Reference::SameScope(next) = reference(dependency) {
                    visit(file, next, marks, stack, issues);
                }
            }
        }
        stack.pop();
        marks.insert(id, Mark::Done);
    }
    let mut issues = Vec::new();
    let mut marks = HashMap::new();
    for task in &file.tasks {
        visit(file, &task.id, &mut marks, &mut Vec::new(), &mut issues);
    }
    issues
}
