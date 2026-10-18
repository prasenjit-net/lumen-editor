//! Turn a validated definition into the concrete command a run will execute:
//! variables expanded against a context captured once per invocation, platform
//! overrides applied, working directory chosen. Pure: the caller supplies the
//! workspace, active file and environment lookup.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    Platform, Scope, TaskDef, TaskKind, Transport, Variable, find_variables,
    validate::valid_env_name,
};

/// What `${...}` variables and relative paths resolve against.
pub struct ResolveContext<'a> {
    pub workspace: Option<&'a Path>,
    /// The active saved file, captured before any input is shown.
    pub active_file: Option<&'a Path>,
    /// Base for relative `cwd` in user tasks (the user config directory).
    pub user_base: Option<&'a Path>,
    pub platform: Platform,
    pub env: &'a dyn Fn(&str) -> Option<String>,
    /// Values collected for this run's declared inputs.
    pub inputs: &'a dyn Fn(&str) -> Option<InputValue>,
}

/// A collected input value; `secret` ones are masked wherever Lumen shows commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputValue {
    pub value: String,
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    /// An executable and literal arguments; no shell is involved.
    Process { program: String, args: Vec<String> },
    /// A command string for the shell to interpret.
    Shell { command: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTask {
    pub label: String,
    pub program: Program,
    pub cwd: PathBuf,
    /// Overrides applied on top of the inherited environment.
    pub env: BTreeMap<String, String>,
    pub unset_env: Vec<String>,
    pub transport: Transport,
    pub exit_codes: Vec<i32>,
    pub timeout_seconds: Option<u32>,
    pub is_background: bool,
    pub ready_pattern: Option<String>,
    pub ready_timeout_seconds: Option<u32>,
    /// Secret input values that were substituted; never shown (see [`Self::redact`]).
    pub secrets: Vec<String>,
    /// How to scan this run's output for problems, if the task asks for it.
    pub problem_matcher: Option<crate::ProblemMatcherSpec>,
}

impl ResolvedTask {
    /// `text` with every secret value replaced by bullets, for any display, log or
    /// preview of the command.
    pub fn redact(&self, text: &str) -> String {
        let mut text = text.to_string();
        for secret in self.secrets.iter().filter(|secret| !secret.is_empty()) {
            text = text.replace(secret.as_str(), "••••");
        }
        text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// `${workspaceFolder}`-style variable with no folder open.
    NoWorkspace {
        variable: String,
    },
    /// `${file}`-style variable with no saved active file.
    NoFile {
        variable: String,
    },
    UnsetEnv {
        name: String,
    },
    /// A declared input that was never given a value.
    MissingInput {
        id: String,
    },
    /// No usable working directory (no workspace and no explicit `cwd`).
    NoWorkingDirectory,
    Compound,
    /// A variable that validation should already have rejected.
    Invalid(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWorkspace { variable } => {
                write!(f, "{variable} needs an open folder; open one and run again")
            }
            Self::NoFile { variable } => write!(
                f,
                "{variable} needs a saved file to be active; save or open a file first"
            ),
            Self::UnsetEnv { name } => {
                write!(f, "environment variable '{name}' is not set")
            }
            Self::MissingInput { id } => write!(f, "input '{id}' has no value"),
            Self::NoWorkingDirectory => write!(
                f,
                "no working directory: open a folder or set 'cwd' on this task"
            ),
            Self::Compound => write!(f, "compound tasks run through the scheduler, not directly"),
            Self::Invalid(message) => write!(f, "{message}"),
        }
    }
}

/// Expand every `${...}` in `text`; `$${` yields a literal `${`.
pub fn expand(text: &str, ctx: &ResolveContext<'_>) -> Result<String, ResolveError> {
    expand_tracking(text, ctx, &mut Vec::new())
}

/// [`expand`], also noting each secret input value that was substituted.
fn expand_tracking(
    text: &str,
    ctx: &ResolveContext<'_>,
    secrets: &mut Vec<String>,
) -> Result<String, ResolveError> {
    // Validate syntax first so errors match `find_variables`.
    find_variables(text).map_err(ResolveError::Invalid)?;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let escaped = start > 0 && rest.as_bytes()[start - 1] == b'$';
        let after = &rest[start + 2..];
        let end = after.find('}').expect("find_variables checked termination");
        if escaped {
            out.push_str(&rest[..start - 1]);
            out.push_str("${");
            out.push_str(&after[..=end]);
            rest = &after[end + 1..];
            continue;
        }
        out.push_str(&rest[..start]);
        let variable = find_variables(&rest[start..start + 3 + end])
            .map_err(ResolveError::Invalid)?
            .remove(0);
        out.push_str(&value(
            &variable,
            ctx,
            &rest[start..start + 3 + end],
            secrets,
        )?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn value(
    variable: &Variable,
    ctx: &ResolveContext<'_>,
    source: &str,
    secrets: &mut Vec<String>,
) -> Result<String, ResolveError> {
    let workspace = || {
        ctx.workspace.ok_or_else(|| ResolveError::NoWorkspace {
            variable: source.into(),
        })
    };
    let file = || {
        ctx.active_file.ok_or_else(|| ResolveError::NoFile {
            variable: source.into(),
        })
    };
    let text = |path: &Path| path.to_string_lossy().into_owned();
    Ok(match variable {
        Variable::WorkspaceFolder => text(workspace()?),
        Variable::WorkspaceFolderBasename => workspace()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Variable::File => text(file()?),
        Variable::FileDirname => file()?.parent().map(text).unwrap_or_default(),
        Variable::FileBasename => file()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        Variable::Env(name) => {
            (ctx.env)(name).ok_or_else(|| ResolveError::UnsetEnv { name: name.clone() })?
        }
        Variable::Input(id) => {
            let input =
                (ctx.inputs)(id).ok_or_else(|| ResolveError::MissingInput { id: id.clone() })?;
            if input.secret && !secrets.contains(&input.value) {
                secrets.push(input.value.clone());
            }
            input.value
        }
    })
}

/// Resolve a process or shell task for `scope`.
pub fn resolve(
    def: &TaskDef,
    scope: Scope,
    ctx: &ResolveContext<'_>,
) -> Result<ResolvedTask, ResolveError> {
    if def.kind == TaskKind::Compound {
        return Err(ResolveError::Compound);
    }
    let over = def.platform_override(ctx.platform);
    let command = over
        .command
        .as_deref()
        .or(def.command.as_deref())
        .ok_or_else(|| ResolveError::Invalid("task has no command".into()))?;
    let mut secrets = Vec::new();
    let command = expand_tracking(command, ctx, &mut secrets)?;
    let program = match def.kind {
        TaskKind::Shell => Program::Shell { command },
        _ => Program::Process {
            program: command,
            args: over
                .args
                .as_ref()
                .unwrap_or(&def.args)
                .iter()
                .map(|arg| expand_tracking(arg, ctx, &mut secrets))
                .collect::<Result<_, _>>()?,
        },
    };
    let mut env = BTreeMap::new();
    for (name, text) in def.env.iter().chain(over.env.iter()) {
        env.insert(name.clone(), expand_tracking(text, ctx, &mut secrets)?);
    }
    let unset_env: Vec<String> = def
        .unset_env
        .iter()
        .filter(|name| valid_env_name(name) && !env.contains_key(*name))
        .cloned()
        .collect();
    let cwd = match over.cwd.as_deref().or(def.cwd.as_deref()) {
        Some(cwd) => {
            let expanded = PathBuf::from(expand_tracking(cwd, ctx, &mut secrets)?);
            if expanded.is_absolute() {
                expanded
            } else {
                let base = match scope {
                    Scope::Workspace => ctx.workspace,
                    Scope::User => ctx.user_base,
                }
                .ok_or(ResolveError::NoWorkingDirectory)?;
                base.join(expanded)
            }
        }
        None => ctx
            .workspace
            .map(Path::to_path_buf)
            .ok_or(ResolveError::NoWorkingDirectory)?,
    };
    Ok(ResolvedTask {
        label: def.label.clone(),
        program,
        cwd,
        env,
        unset_env,
        transport: def.effective_transport(),
        exit_codes: def.effective_exit_codes(),
        timeout_seconds: def.timeout_seconds,
        is_background: def.is_background,
        ready_pattern: def.background.as_ref().map(|b| b.ready_pattern.clone()),
        ready_timeout_seconds: def.effective_ready_timeout(),
        secrets,
        problem_matcher: def.problem_matcher.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Background, PlatformOverride};

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME_X" => Some("/home/x".into()),
            _ => None,
        }
    }

    fn no_inputs(_: &str) -> Option<InputValue> {
        None
    }

    fn ctx<'a>(
        workspace: Option<&'a Path>,
        file: Option<&'a Path>,
        platform: Platform,
    ) -> ResolveContext<'a> {
        ResolveContext {
            workspace,
            active_file: file,
            user_base: Some(Path::new("/user/.lumen")),
            platform,
            env: &env,
            inputs: &no_inputs,
        }
    }

    fn process(command: &str, args: &[&str]) -> TaskDef {
        let mut task = TaskDef::new("t", "T", TaskKind::Process);
        task.command = Some(command.into());
        task.args = args.iter().map(|a| a.to_string()).collect();
        task
    }

    // Compares POSIX path strings; Windows path forms are covered separately.
    #[cfg(unix)]
    #[test]
    fn variables_expand_in_command_args_env_and_cwd() {
        let workspace = Path::new("/w/proj");
        let file = Path::new("/w/proj/src/main.rs");
        let context = ctx(Some(workspace), Some(file), Platform::Linux);
        let mut task = process(
            "${env:HOME_X}/bin/tool",
            &[
                "${file}",
                "--dir=${fileDirname}",
                "${fileBasename}",
                "${workspaceFolderBasename}",
            ],
        );
        task.cwd = Some("${workspaceFolder}/sub".into());
        task.env
            .insert("OUT".into(), "${workspaceFolder}/out".into());
        let resolved = resolve(&task, Scope::Workspace, &context).unwrap();
        assert_eq!(
            resolved.program,
            Program::Process {
                program: "/home/x/bin/tool".into(),
                args: vec![
                    "/w/proj/src/main.rs".into(),
                    "--dir=/w/proj/src".into(),
                    "main.rs".into(),
                    "proj".into()
                ],
            }
        );
        assert_eq!(resolved.cwd, Path::new("/w/proj/sub"));
        assert_eq!(resolved.env["OUT"], "/w/proj/out");
    }

    #[test]
    fn arguments_stay_separate_and_literal_dollars_survive() {
        let context = ctx(Some(Path::new("/w")), None, Platform::Linux);
        let task = process("echo", &["a b", "$HOME", "$${workspaceFolder}", "100$"]);
        let resolved = resolve(&task, Scope::Workspace, &context).unwrap();
        let Program::Process { args, .. } = resolved.program else {
            panic!()
        };
        assert_eq!(args, ["a b", "$HOME", "${workspaceFolder}", "100$"]);
    }

    #[test]
    fn missing_context_is_an_actionable_error_never_an_empty_string() {
        let none = ctx(None, None, Platform::Linux);
        let task = process("x", &["${workspaceFolder}"]);
        let error = resolve(&task, Scope::Workspace, &none).unwrap_err();
        assert!(matches!(error, ResolveError::NoWorkspace { .. }));
        assert!(error.to_string().contains("open folder"));
        let task = process("x", &["${file}"]);
        let with_workspace = ctx(Some(Path::new("/w")), None, Platform::Linux);
        let error = resolve(&task, Scope::Workspace, &with_workspace).unwrap_err();
        assert!(
            matches!(error, ResolveError::NoFile { .. })
                && error.to_string().contains("saved file")
        );
        let task = process("x", &["${env:NOPE}"]);
        assert_eq!(
            resolve(&task, Scope::Workspace, &with_workspace).unwrap_err(),
            ResolveError::UnsetEnv {
                name: "NOPE".into()
            }
        );
    }

    #[test]
    fn platform_overrides_replace_execution_fields_and_merge_env() {
        let mut task = process("make", &["all"]);
        task.env.insert("A".into(), "1".into());
        task.env.insert("B".into(), "base".into());
        task.windows = PlatformOverride {
            command: Some("nmake".into()),
            args: Some(vec!["/f".into()]),
            cwd: None,
            env: BTreeMap::from([("B".into(), "win".into())]),
        };
        let workspace = Path::new("/w");
        let windows = resolve(
            &task,
            Scope::Workspace,
            &ctx(Some(workspace), None, Platform::Windows),
        )
        .unwrap();
        assert_eq!(
            windows.program,
            Program::Process {
                program: "nmake".into(),
                args: vec!["/f".into()]
            }
        );
        assert_eq!(
            (windows.env["A"].as_str(), windows.env["B"].as_str()),
            ("1", "win")
        );
        let linux = resolve(
            &task,
            Scope::Workspace,
            &ctx(Some(workspace), None, Platform::Linux),
        )
        .unwrap();
        assert_eq!(
            linux.program,
            Program::Process {
                program: "make".into(),
                args: vec!["all".into()]
            }
        );
        assert_eq!(linux.env["B"], "base");
    }

    // Compares POSIX path strings; Windows path forms are covered separately.
    #[cfg(unix)]
    #[test]
    fn working_directory_defaults_and_relative_bases_follow_scope() {
        let workspace = Path::new("/w");
        let context = ctx(Some(workspace), None, Platform::Linux);
        let mut task = process("x", &[]);
        assert_eq!(
            resolve(&task, Scope::Workspace, &context).unwrap().cwd,
            Path::new("/w")
        );
        task.cwd = Some("sub/dir".into());
        assert_eq!(
            resolve(&task, Scope::Workspace, &context).unwrap().cwd,
            Path::new("/w/sub/dir")
        );
        assert_eq!(
            resolve(&task, Scope::User, &context).unwrap().cwd,
            Path::new("/user/.lumen/sub/dir")
        );
        task.cwd = Some("/abs".into());
        assert_eq!(
            resolve(&task, Scope::User, &context).unwrap().cwd,
            Path::new("/abs")
        );
        // A user task with no cwd and no folder has nowhere to run; with cwd it does.
        let none = ctx(None, None, Platform::Linux);
        task.cwd = None;
        assert_eq!(
            resolve(&task, Scope::User, &none).unwrap_err(),
            ResolveError::NoWorkingDirectory
        );
        task.cwd = Some("/abs".into());
        assert!(resolve(&task, Scope::User, &none).is_ok());
    }

    #[test]
    fn shell_tasks_keep_one_command_string_and_carry_run_policy() {
        let mut task = TaskDef::new("s", "S", TaskKind::Shell);
        task.command = Some("npm run dev && echo ${workspaceFolderBasename}".into());
        task.is_background = true;
        task.background = Some(Background {
            ready_pattern: "Local:".into(),
            ready_timeout_seconds: None,
        });
        task.exit_codes = Some(vec![0, 2]);
        task.timeout_seconds = Some(30);
        task.unset_env = vec!["DEBUG".into()];
        let resolved = resolve(
            &task,
            Scope::Workspace,
            &ctx(Some(Path::new("/w/app")), None, Platform::Macos),
        )
        .unwrap();
        assert_eq!(
            resolved.program,
            Program::Shell {
                command: "npm run dev && echo app".into()
            }
        );
        assert_eq!(resolved.transport, Transport::Pty);
        assert_eq!(resolved.exit_codes, vec![0, 2]);
        assert_eq!(
            (resolved.timeout_seconds, resolved.ready_timeout_seconds),
            (Some(30), Some(60))
        );
        assert_eq!(resolved.ready_pattern.as_deref(), Some("Local:"));
        assert_eq!(resolved.unset_env, ["DEBUG"]);
    }

    #[test]
    fn compound_and_malformed_inputs_are_rejected() {
        let context = ctx(Some(Path::new("/w")), None, Platform::Linux);
        let mut compound = TaskDef::new("c", "C", TaskKind::Compound);
        compound.depends_on = vec!["a".into()];
        assert_eq!(
            resolve(&compound, Scope::Workspace, &context).unwrap_err(),
            ResolveError::Compound
        );
        let task = process("x", &["${bogus}"]);
        assert!(matches!(
            resolve(&task, Scope::Workspace, &context),
            Err(ResolveError::Invalid(_))
        ));
        let mut no_command = process("x", &[]);
        no_command.command = None;
        assert!(resolve(&no_command, Scope::Workspace, &context).is_err());
    }

    #[test]
    fn inputs_expand_secrets_are_tracked_and_redacted_and_missing_ones_fail() {
        let provide = |id: &str| match id {
            "env" => Some(InputValue {
                value: "prod".into(),
                secret: false,
            }),
            "token" => Some(InputValue {
                value: "s3cr3t".into(),
                secret: true,
            }),
            _ => None,
        };
        let context = ResolveContext {
            workspace: Some(Path::new("/w")),
            active_file: None,
            user_base: None,
            platform: Platform::Linux,
            env: &env,
            inputs: &provide,
        };
        let mut task = process(
            "deploy",
            &["--env", "${input:env}", "--token=${input:token}"],
        );
        task.env.insert("API_TOKEN".into(), "${input:token}".into());
        let resolved = resolve(&task, Scope::Workspace, &context).unwrap();
        let Program::Process { args, .. } = &resolved.program else {
            panic!()
        };
        assert_eq!(args, &["--env", "prod", "--token=s3cr3t"]);
        assert_eq!(resolved.env["API_TOKEN"], "s3cr3t");
        assert_eq!(resolved.secrets, ["s3cr3t"], "each secret is noted once");
        assert_eq!(
            resolved.redact("deploy --token=s3cr3t --env prod"),
            "deploy --token=•••• --env prod"
        );
        let missing = process("x", &["${input:other}"]);
        assert_eq!(
            resolve(&missing, Scope::Workspace, &context).unwrap_err(),
            ResolveError::MissingInput { id: "other".into() }
        );
        // A task without secrets redacts nothing.
        let plain = resolve(&process("echo", &["hi"]), Scope::Workspace, &context).unwrap();
        assert!(plain.secrets.is_empty());
        assert_eq!(plain.redact("echo hi"), "echo hi");
    }
}
