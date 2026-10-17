use super::*;

const EXAMPLE: &str = r#"schema_version = 1

# Build everything.
[[tasks]]
id = "build"
label = "Build"
type = "process"
command = "cargo"
args = ["build"]
cwd = "${workspaceFolder}"
group = "build"
is_default = true
transport = "pipe"
save = "prompt"
instance_policy = "single"

[tasks.env]
CARGO_TERM_COLOR = "always"

[tasks.presentation]
reveal = "always"
focus = false
reuse = "dedicated"
clear = true

[[tasks]]
id = "test"
label = "Test"
type = "process"
command = "cargo"
args = ["test", "--workspace"]
depends_on = ["build"]
depends_order = "sequence"

[[tasks]]
id = "dev"
label = "Dev server"
type = "shell"
command = "npm run dev"
transport = "pty"
is_background = true

[tasks.background]
ready_pattern = "Local:"
ready_timeout_seconds = 60

[[tasks]]
id = "verify"
label = "Verify"
type = "compound"
depends_on = ["build", "test"]
depends_order = "sequence"
"#;

fn issues(text: &str) -> Vec<String> {
    match parse(text) {
        Ok(_) => Vec::new(),
        Err(issues) => issues.iter().map(ToString::to_string).collect(),
    }
}

fn has(text: &str, needle: &str) -> bool {
    let all = issues(text);
    all.iter().any(|issue| issue.contains(needle))
}

fn task(id: &str, deps: &[&str]) -> TaskDef {
    let mut task = TaskDef::new(id, id, TaskKind::Process);
    task.command = Some("true".into());
    task.depends_on = deps.iter().map(|d| d.to_string()).collect();
    task
}

fn file(tasks: Vec<TaskDef>) -> TaskFile {
    TaskFile { tasks }
}

#[test]
fn the_documented_example_parses_with_explicit_and_default_fields() {
    let parsed = parse(EXAMPLE).unwrap();
    assert_eq!(parsed.tasks.len(), 4);
    let build = parsed.get("build").unwrap();
    assert_eq!(build.command.as_deref(), Some("cargo"));
    assert_eq!(build.env["CARGO_TERM_COLOR"], "always");
    assert_eq!(build.effective_transport(), Transport::Pipe);
    assert!(build.is_default);
    let dev = parsed.get("dev").unwrap();
    assert_eq!(dev.effective_transport(), Transport::Pty);
    assert_eq!(dev.effective_ready_timeout(), Some(60));
    let test = parsed.get("test").unwrap();
    assert_eq!(test.effective_exit_codes(), vec![0]);
    assert_eq!(test.effective_save(), SavePolicy::Prompt);
    assert!(test.effective_clear());
    assert_eq!(parsed.get("verify").unwrap().kind, TaskKind::Compound);
}

#[test]
fn shell_tasks_default_to_a_pty_and_process_tasks_to_pipes() {
    assert_eq!(
        TaskDef::new("a", "a", TaskKind::Shell).effective_transport(),
        Transport::Pty
    );
    assert_eq!(
        TaskDef::new("a", "a", TaskKind::Process).effective_transport(),
        Transport::Pipe
    );
}

#[test]
fn version_syntax_and_type_errors_are_reported_not_ignored() {
    assert!(has("tasks = []", "schema_version"));
    assert!(has("schema_version = 2\n", "unsupported version 2"));
    assert!(has("schema_version = 1\n[[tasks]\n", "toml"));
    assert!(has("schema_version = 1\ntasks = 3\n", "array of tables"));
    assert!(has(
        "schema_version = 1\nextra = 1\n",
        "unknown key 'extra'"
    ));
    let text = "schema_version = 1\n[[tasks]]\nid = \"a\"\nlabel = \"A\"\ntype = \"batch\"\n";
    assert!(has(text, "not process, shell or compound"));
}

#[test]
fn a_misspelled_option_is_an_error_with_the_task_and_path() {
    let text = format!(
        "{EXAMPLE}\n[[tasks]]\nid = \"x\"\nlabel = \"X\"\ntype = \"process\"\ncommand = \"a\"\ninstnce_policy = \"single\"\n"
    );
    assert!(has(&text, "task 'x', instnce_policy: unknown key"));
    let text = EXAMPLE.replace("reveal = \"always\"", "revael = \"always\"");
    assert!(has(&text, "presentation.revael"));
    let text = EXAMPLE.replace("save = \"prompt\"", "save = \"sometimes\"");
    assert!(has(&text, "'sometimes' is not one of: prompt, save, none"));
    let text = EXAMPLE.replace("args = [\"build\"]", "args = [\"build\", 3]");
    assert!(has(&text, "args: must be an array of strings"));
}

#[test]
fn identity_label_and_command_rules() {
    let bad_id = |id: &str| {
        let mut t = task(id, &[]);
        t.id = id.into();
        validate_task(&t).iter().any(|i| i.field == "id")
    };
    assert!(bad_id(""));
    assert!(bad_id("has space"));
    assert!(bad_id("-leading"));
    assert!(bad_id(&"x".repeat(65)));
    assert!(!bad_id("ok.id_1-2"));

    let mut t = task("a", &[]);
    t.command = Some("  ".into());
    assert!(validate_task(&t).iter().any(|i| i.field == "command"));
    let mut t = task("a", &[]);
    t.label = " ".into();
    assert!(validate_task(&t).iter().any(|i| i.field == "label"));
    let mut t = task("a", &[]);
    t.label = "two\nlines".into();
    assert!(validate_task(&t).iter().any(|i| i.field == "label"));
}

#[test]
fn duplicate_ids_conflicting_defaults_and_group_rules() {
    assert!(!validate_file(&file(vec![task("a", &[]), task("a", &[])])).is_empty());
    let mut a = task("a", &[]);
    let mut b = task("b", &[]);
    a.group = Some("build".into());
    b.group = Some("build".into());
    a.is_default = true;
    b.is_default = true;
    let found = validate_file(&file(vec![a.clone(), b.clone()]));
    assert!(
        found
            .iter()
            .any(|i| i.message.contains("already has default"))
    );
    b.group = Some("test".into());
    assert!(validate_file(&file(vec![a, b])).is_empty());
    let mut c = task("c", &[]);
    c.is_default = true;
    assert!(validate_task(&c).iter().any(|i| i.field == "is_default"));
}

#[test]
fn shell_args_env_and_override_rules() {
    let mut shell = TaskDef::new("s", "S", TaskKind::Shell);
    shell.command = Some("echo hi".into());
    assert!(validate_task(&shell).is_empty());
    shell.args = vec!["x".into()];
    assert!(validate_task(&shell).iter().any(|i| i.field == "args"));
    shell.args.clear();
    shell.windows.args = Some(vec!["x".into()]);
    assert!(
        validate_task(&shell)
            .iter()
            .any(|i| i.field == "windows.args")
    );

    let mut t = task("t", &[]);
    t.env.insert("BAD NAME".into(), "v".into());
    assert!(validate_task(&t).iter().any(|i| i.field == "env"));
    let mut t = task("t", &[]);
    t.env.insert("A".into(), "x\0".into());
    assert!(validate_task(&t).iter().any(|i| i.message.contains("NUL")));
    let mut t = task("t", &[]);
    t.env.insert("A".into(), "1".into());
    t.unset_env = vec!["A".into()];
    assert!(
        validate_task(&t)
            .iter()
            .any(|i| i.message.contains("both set and unset"))
    );
    let mut t = task("t", &[]);
    t.exit_codes = Some(vec![]);
    t.timeout_seconds = Some(0);
    let found = validate_task(&t);
    assert!(found.iter().any(|i| i.field == "exit_codes"));
    assert!(found.iter().any(|i| i.field == "timeout_seconds"));
}

#[test]
fn compound_tasks_are_graphs_only() {
    let mut c = TaskDef::new("c", "C", TaskKind::Compound);
    assert!(validate_task(&c).iter().any(|i| i.field == "depends_on"));
    c.depends_on = vec!["a".into()];
    assert!(validate_task(&c).is_empty());
    c.command = Some("x".into());
    c.env.insert("A".into(), "1".into());
    c.transport = Some(Transport::Pty);
    c.linux.cwd = Some("/".into());
    let fields: Vec<_> = validate_task(&c).into_iter().map(|i| i.field).collect();
    for field in ["command", "env", "transport", "platform"] {
        assert!(fields.iter().any(|f| f == field), "{field} in {fields:?}");
    }
}

#[test]
fn background_tasks_need_a_finite_readiness_wait() {
    let mut t = task("d", &[]);
    t.is_background = true;
    assert!(validate_task(&t).iter().any(|i| i.field == "background"));
    t.background = Some(Background {
        ready_pattern: "up".into(),
        ready_timeout_seconds: None,
    });
    assert!(validate_task(&t).is_empty());
    assert_eq!(
        t.effective_ready_timeout(),
        Some(DEFAULT_READY_TIMEOUT_SECONDS)
    );
    t.background.as_mut().unwrap().ready_timeout_seconds = Some(MAX_READY_TIMEOUT_SECONDS + 1);
    assert!(!validate_task(&t).is_empty());
    t.background.as_mut().unwrap().ready_timeout_seconds = Some(0);
    assert!(!validate_task(&t).is_empty());
    t.background.as_mut().unwrap().ready_pattern.clear();
    t.background.as_mut().unwrap().ready_timeout_seconds = Some(5);
    assert!(!validate_task(&t).is_empty());
    t.is_background = false;
    assert!(
        validate_task(&t)
            .iter()
            .any(|i| i.message.contains("is_background"))
    );
}

#[test]
fn variables_are_a_small_checked_vocabulary() {
    assert_eq!(
        find_variables("${workspaceFolder}/x ${env:HOME_DIR} $${literal}").unwrap(),
        vec![Variable::WorkspaceFolder, Variable::Env("HOME_DIR".into())]
    );
    assert_eq!(find_variables("no vars $5").unwrap(), vec![]);
    assert!(
        find_variables("${nope}")
            .unwrap_err()
            .contains("unknown variable")
    );
    assert!(
        find_variables("${input:x}")
            .unwrap_err()
            .contains("not supported yet")
    );
    assert!(
        find_variables("${command:x}")
            .unwrap_err()
            .contains("not supported")
    );
    assert!(find_variables("${env:BAD-NAME}").is_err());
    assert!(
        find_variables("${file")
            .unwrap_err()
            .contains("unterminated")
    );
    assert!(Variable::WorkspaceFolder.needs_workspace());
    assert!(Variable::FileDirname.needs_file());
    let mut t = task("v", &[]);
    t.args = vec!["${bogus}".into()];
    assert!(validate_task(&t).iter().any(|i| i.field == "args[0]"));
}

#[test]
fn dependencies_must_exist_be_unique_and_acyclic() {
    let found = validate_file(&file(vec![task("a", &["ghost"])]));
    assert!(
        found
            .iter()
            .any(|i| i.message.contains("unknown task 'ghost'"))
    );
    let mut t = task("a", &["a"]);
    assert!(
        validate_task(&t)
            .iter()
            .any(|i| i.message.contains("itself"))
    );
    t.depends_on = vec!["b".into(), "b".into()];
    assert!(
        validate_task(&t)
            .iter()
            .any(|i| i.message.contains("twice"))
    );
    let cycle = validate_file(&file(vec![
        task("a", &["b"]),
        task("b", &["c"]),
        task("c", &["a"]),
    ]));
    assert!(cycle.iter().any(|i| i.message.contains("dependency cycle")));
    assert!(validate_file(&file(vec![task("a", &["b"]), task("b", &[])])).is_empty());
}

fn keys(plan: &Plan) -> Vec<&str> {
    plan.nodes.iter().map(|n| n.key.id.as_str()).collect()
}

#[test]
fn plans_deduplicate_shared_nodes_and_end_at_the_requested_task() {
    let mut verify = task("verify", &["build", "test"]);
    verify.kind = TaskKind::Compound;
    verify.command = None;
    let files = file(vec![task("build", &[]), task("test", &["build"]), verify]);
    let catalog = Catalog {
        user: None,
        workspace: Some(&files),
    };
    let plan = catalog
        .plan(&TaskKey::new(Scope::Workspace, "verify"))
        .unwrap();
    assert_eq!(keys(&plan), ["build", "test", "verify"]);
    assert_eq!(plan.root().key.id, "verify");
    assert_eq!(plan.nodes[0].needs, Vec::<usize>::new());
    assert_eq!(plan.nodes[1].needs, vec![0]);
    assert_eq!(plan.nodes[2].needs, vec![0, 1]);
}

#[test]
fn sequence_chains_siblings_and_parallel_leaves_them_independent() {
    let sequence = file(vec![
        task("a", &[]),
        task("b", &[]),
        task("root", &["a", "b"]),
    ]);
    let catalog = Catalog {
        user: None,
        workspace: Some(&sequence),
    };
    let plan = catalog
        .plan(&TaskKey::new(Scope::Workspace, "root"))
        .unwrap();
    let b = plan.nodes.iter().position(|n| n.key.id == "b").unwrap();
    let a = plan.nodes.iter().position(|n| n.key.id == "a").unwrap();
    assert_eq!(plan.nodes[b].needs, vec![a]);

    let mut root = task("root", &["a", "b"]);
    root.depends_order = Some(DependsOrder::Parallel);
    let parallel = file(vec![task("a", &[]), task("b", &[]), root]);
    let catalog = Catalog {
        user: None,
        workspace: Some(&parallel),
    };
    let plan = catalog
        .plan(&TaskKey::new(Scope::Workspace, "root"))
        .unwrap();
    assert!(plan.nodes[0].needs.is_empty() && plan.nodes[1].needs.is_empty());
}

#[test]
fn opposite_sequential_orders_from_two_parents_are_a_conflict() {
    let files = file(vec![
        task("a", &[]),
        task("b", &[]),
        task("p1", &["a", "b"]),
        task("p2", &["b", "a"]),
        task("root", &["p1", "p2"]),
    ]);
    let catalog = Catalog {
        user: None,
        workspace: Some(&files),
    };
    let error = catalog
        .plan(&TaskKey::new(Scope::Workspace, "root"))
        .unwrap_err();
    assert!(matches!(error, PlanError::OrderConflict(_)), "{error}");
}

#[test]
fn plan_errors_are_reported_before_anything_is_returned() {
    let files = file(vec![task("a", &["b"]), task("b", &["a"])]);
    let catalog = Catalog {
        user: None,
        workspace: Some(&files),
    };
    let error = catalog
        .plan(&TaskKey::new(Scope::Workspace, "a"))
        .unwrap_err();
    assert!(
        matches!(&error, PlanError::Cycle(path) if path.len() == 3),
        "{error}"
    );
    let files = file(vec![task("a", &["ghost"])]);
    let catalog = Catalog {
        user: None,
        workspace: Some(&files),
    };
    assert!(matches!(
        catalog.plan(&TaskKey::new(Scope::Workspace, "a")),
        Err(PlanError::Unknown { .. })
    ));
}

#[test]
fn scopes_stay_distinct_and_only_explicit_user_references_cross() {
    let user = file(vec![task("lint", &[]), task("build", &[])]);
    let workspace = file(vec![
        task("build", &[]),
        task("ci", &["build", "user:lint"]),
    ]);
    let catalog = Catalog {
        user: Some(&user),
        workspace: Some(&workspace),
    };
    assert!(catalog.validate().is_empty());
    let plan = catalog.plan(&TaskKey::new(Scope::Workspace, "ci")).unwrap();
    let scopes: Vec<_> = plan
        .nodes
        .iter()
        .map(|n| (n.key.scope, n.key.id.as_str()))
        .collect();
    assert!(scopes.contains(&(Scope::Workspace, "build")));
    assert!(scopes.contains(&(Scope::User, "lint")));
    assert!(
        !scopes.contains(&(Scope::User, "build")),
        "same ID in another scope is distinct"
    );

    let missing = Catalog {
        user: None,
        workspace: Some(&workspace),
    };
    assert!(
        missing
            .validate()
            .iter()
            .any(|i| i.message.contains("unknown personal task"))
    );
    // A user task cannot reach a workspace task: unqualified IDs stay in scope.
    let user_only = file(vec![task("t", &["ci"])]);
    let catalog = Catalog {
        user: Some(&user_only),
        workspace: Some(&workspace),
    };
    assert!(matches!(
        catalog.plan(&TaskKey::new(Scope::User, "t")),
        Err(PlanError::Unknown { .. })
    ));
}

#[test]
fn workspace_default_wins_over_the_user_default() {
    let mut u = task("u", &[]);
    u.group = Some("build".into());
    u.is_default = true;
    let mut w = task("w", &[]);
    w.group = Some("build".into());
    w.is_default = true;
    let (user, workspace) = (file(vec![u]), file(vec![w]));
    let both = Catalog {
        user: Some(&user),
        workspace: Some(&workspace),
    };
    assert_eq!(
        both.default_for_group("build"),
        Some(TaskKey::new(Scope::Workspace, "w"))
    );
    let only_user = Catalog {
        user: Some(&user),
        workspace: None,
    };
    assert_eq!(
        only_user.default_for_group("build"),
        Some(TaskKey::new(Scope::User, "u"))
    );
    assert_eq!(only_user.default_for_group("test"), None);
}

#[test]
fn saving_an_unchanged_definition_leaves_the_text_byte_identical() {
    let parsed = parse(EXAMPLE).unwrap();
    let mut text = EXAMPLE.to_string();
    for def in &parsed.tasks {
        text = upsert_task(&text, def).unwrap();
    }
    assert_eq!(text, EXAMPLE);
}

#[test]
fn editing_one_task_preserves_comments_unknown_keys_and_other_tasks() {
    // A key we do not know about is rejected on load but must survive an edit.
    let source = EXAMPLE.replace(
        "# Build everything.",
        "# Build everything.\n# keep this comment",
    );
    let parsed = parse(&source).unwrap();
    let mut build = parsed.get("build").unwrap().clone();
    build.args = vec!["build".into(), "--release".into()];
    build.env.insert("RUST_LOG".into(), "info".into());
    build.env.remove("CARGO_TERM_COLOR");
    build.detail = Some("Release build".into());
    let edited = upsert_task(&source, &build).unwrap();
    assert!(edited.contains("# keep this comment"));
    assert!(edited.contains("args = [\"build\", \"--release\"]"));
    assert!(edited.contains("RUST_LOG = \"info\""));
    assert!(!edited.contains("CARGO_TERM_COLOR"));
    let reparsed = parse(&edited).unwrap();
    assert_eq!(reparsed.get("build"), Some(&build));
    for id in ["test", "dev", "verify"] {
        assert_eq!(reparsed.get(id), parsed.get(id), "{id} untouched");
    }

    let with_unknown = "schema_version = 1\n[[tasks]]\nid = \"a\"\nlabel = \"A\"\ntype = \"process\"\ncommand = \"x\"\nfuture_key = 1\n";
    let mut def = TaskDef::new("a", "Renamed", TaskKind::Process);
    def.command = Some("x".into());
    let edited = upsert_task(with_unknown, &def).unwrap();
    assert!(edited.contains("future_key = 1"), "{edited}");
    assert!(edited.contains("label = \"Renamed\""));
}

#[test]
fn removing_a_default_option_deletes_its_key() {
    let parsed = parse(EXAMPLE).unwrap();
    let mut build = parsed.get("build").unwrap().clone();
    build.save = None;
    build.presentation = Presentation::default();
    let edited = upsert_task(EXAMPLE, &build).unwrap();
    assert!(!edited.contains("save = \"prompt\""));
    assert!(!edited.contains("[tasks.presentation]"));
    assert_eq!(parse(&edited).unwrap().get("build"), Some(&build));
}

#[test]
fn new_tasks_append_and_new_files_round_trip() {
    let mut def = TaskDef::new("lint", "Lint", TaskKind::Process);
    def.command = Some("cargo".into());
    def.args = vec!["clippy".into()];
    def.exit_codes = Some(vec![0, 101]);
    def.timeout_seconds = Some(300);
    def.windows.command = Some("cargo.exe".into());
    def.linux.env.insert("A".into(), "1".into());
    let created = new_file(&def).unwrap();
    assert!(created.starts_with("schema_version = 1"));
    assert_eq!(parse(&created).unwrap().tasks, vec![def.clone()]);

    let appended = upsert_task(EXAMPLE, &def).unwrap();
    let reparsed = parse(&appended).unwrap();
    assert_eq!(reparsed.tasks.len(), 5);
    assert_eq!(reparsed.tasks.last(), Some(&def));
    assert!(appended.starts_with(EXAMPLE.trim_end()));
}

#[test]
fn removal_edits_only_the_named_task_and_reports_missing_ids() {
    let removed = remove_task(EXAMPLE, "dev").unwrap();
    let parsed = parse(&removed).unwrap();
    assert!(parsed.get("dev").is_none());
    assert_eq!(parsed.tasks.len(), 3);
    assert!(removed.contains("# Build everything."));
    assert_eq!(
        remove_task(EXAMPLE, "ghost"),
        Err(EditError::NotFound("ghost".into()))
    );
}

#[test]
fn invalid_or_unreadable_input_is_never_rewritten() {
    let mut bad = TaskDef::new("x", "X", TaskKind::Process);
    bad.command = Some(String::new());
    assert!(matches!(
        upsert_task(EXAMPLE, &bad),
        Err(EditError::Invalid(_))
    ));
    let mut ok = TaskDef::new("x", "X", TaskKind::Process);
    ok.command = Some("true".into());
    assert!(matches!(
        upsert_task("[[tasks]\n", &ok),
        Err(EditError::Unreadable(_))
    ));
    assert!(matches!(
        upsert_task("schema_version = 1\ntasks = 3\n", &ok),
        Err(EditError::Unreadable(_))
    ));
    assert!(matches!(
        remove_task("not toml [", "x"),
        Err(EditError::Unreadable(_))
    ));
}
