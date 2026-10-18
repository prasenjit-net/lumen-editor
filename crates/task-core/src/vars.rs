//! The deliberately small `${...}` vocabulary. Unknown variables fail validation;
//! there is no `${command:...}` and no recursive evaluation.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Variable {
    WorkspaceFolder,
    WorkspaceFolderBasename,
    File,
    FileDirname,
    FileBasename,
    Env(String),
    /// A declared input, asked for once per run.
    Input(String),
}

impl Variable {
    /// Whether resolving needs an open workspace folder / an active saved file.
    pub fn needs_workspace(&self) -> bool {
        matches!(self, Self::WorkspaceFolder | Self::WorkspaceFolderBasename)
    }
    pub fn needs_file(&self) -> bool {
        matches!(self, Self::File | Self::FileDirname | Self::FileBasename)
    }
}

/// Every variable in `text`, in order, or the first problem found.
pub fn find_variables(text: &str) -> Result<Vec<Variable>, String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err("unterminated '${' (write '$$' for a literal dollar sign)".into());
        };
        // `$${` escapes a literal `${`.
        if start > 0 && rest.as_bytes()[start - 1] == b'$' {
            rest = &after[end + 1..];
            continue;
        }
        found.push(parse_name(&after[..end])?);
        rest = &after[end + 1..];
    }
    Ok(found)
}

fn parse_name(name: &str) -> Result<Variable, String> {
    match name {
        "workspaceFolder" => Ok(Variable::WorkspaceFolder),
        "workspaceFolderBasename" => Ok(Variable::WorkspaceFolderBasename),
        "file" => Ok(Variable::File),
        "fileDirname" => Ok(Variable::FileDirname),
        "fileBasename" => Ok(Variable::FileBasename),
        _ => {
            if let Some(var) = name.strip_prefix("env:") {
                if crate::validate::valid_env_name(var) {
                    return Ok(Variable::Env(var.into()));
                }
                return Err(format!(
                    "'${{{name}}}' names an invalid environment variable"
                ));
            }
            if let Some(id) = name.strip_prefix("input:") {
                if crate::validate::valid_id(id) {
                    return Ok(Variable::Input(id.into()));
                }
                return Err(format!("'${{{name}}}' names an invalid input"));
            }
            if name.starts_with("command:") {
                return Err("command variables are not supported".into());
            }
            Err(format!("unknown variable '${{{name}}}'"))
        }
    }
}
