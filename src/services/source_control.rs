//! Read-only Git adapter. All paths passed to Git are argv values, never shell text.
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const MAX_DIFF_BYTES: u64 = 256 * 1024;
const MAX_UNTRACKED_BYTES: u64 = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitChange {
    pub(crate) path: PathBuf,
    pub(crate) index: char,
    pub(crate) worktree: char,
    pub(crate) kind: ChangeKind,
}

impl GitChange {
    /// An ordinary tracked or untracked change, as most callers and tests need.
    pub(crate) fn new(path: impl Into<PathBuf>, index: char, worktree: char) -> Self {
        Self {
            path: path.into(),
            index,
            worktree,
            kind: ChangeKind::Ordinary,
        }
    }

    pub(crate) fn is_conflict(&self) -> bool {
        matches!(self.kind, ChangeKind::Conflict)
    }

    pub(crate) fn is_submodule(&self) -> bool {
        matches!(self.kind, ChangeKind::Submodule(_))
    }
}

/// What kind of status record produced a change. Conflicts are never folded
/// into ordinary modified rows, and a rename keeps its source path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    #[default]
    Ordinary,
    Renamed {
        from: PathBuf,
    },
    Conflict,
    /// A gitlink. Staging it records the child's checked-out commit in the
    /// parent; the child's own files are never staged through the parent.
    Submodule(SubmoduleChange),
}

/// What differs inside a submodule, from the `S<c><m><u>` status field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SubmoduleChange {
    /// The checked-out commit differs from the one the parent records.
    pub(crate) commit: bool,
    pub(crate) modified_content: bool,
    pub(crate) untracked_content: bool,
}

impl SubmoduleChange {
    pub(crate) fn describe(self) -> String {
        let mut parts = Vec::new();
        if self.commit {
            parts.push("new commits");
        }
        if self.modified_content {
            parts.push("modified content");
        }
        if self.untracked_content {
            parts.push("untracked content");
        }
        if parts.is_empty() {
            "submodule".into()
        } else {
            format!("submodule: {}", parts.join(", "))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum GitHead {
    /// A branch without commits yet.
    Unborn(String),
    Attached(String),
    Detached(String),
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitUpstream {
    pub(crate) name: String,
    /// `None` when Git does not report counts (for example, a deleted upstream).
    pub(crate) ahead_behind: Option<(u32, u32)>,
}

/// An operation Git is part-way through, discovered from the repository's
/// state files so operations started outside Lumen are visible too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RepoState {
    #[default]
    Clean,
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl GitSnapshot {
    pub(crate) fn is_linked_worktree(&self) -> bool {
        !self.common_dir.as_os_str().is_empty() && self.git_dir != self.common_dir
    }

    /// Worktrees of one repository share refs and objects, so a ref change in
    /// one invalidates the others.
    pub(crate) fn shares_storage_with(&self, other: &GitSnapshot) -> bool {
        !self.common_dir.as_os_str().is_empty() && self.common_dir == other.common_dir
    }
}

impl RepoState {
    pub(crate) fn label(self) -> Option<&'static str> {
        match self {
            Self::Clean => None,
            Self::Merge => Some("Merging"),
            Self::Rebase => Some("Rebasing"),
            Self::CherryPick => Some("Cherry-picking"),
            Self::Revert => Some("Reverting"),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GitSnapshot {
    pub(crate) root: PathBuf,
    /// This worktree's git directory, and the directory holding the shared
    /// objects and refs. They differ for a linked worktree.
    pub(crate) git_dir: PathBuf,
    pub(crate) common_dir: PathBuf,
    /// Display label for the header; derived from `head`.
    pub(crate) branch: String,
    pub(crate) head: GitHead,
    pub(crate) upstream: Option<GitUpstream>,
    pub(crate) state: RepoState,
    pub(crate) changes: Vec<GitChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffSide {
    Staged,
    Working,
    Untracked,
}

#[derive(Clone, Debug)]
pub(crate) struct GitComparison {
    pub(crate) base: String,
    pub(crate) changed: Option<String>,
    pub(crate) old_lines: BTreeSet<usize>,
    pub(crate) new_lines: BTreeSet<usize>,
    pub(crate) hunks: Vec<GitHunk>,
}

#[derive(Clone, Debug)]
pub(crate) struct GitHunk {
    pub(crate) header: String,
    /// The exact patch text the user was shown. Staging verifies it against a
    /// fresh diff so an external edit cannot redirect the action to other text.
    pub(crate) patch: String,
}

/// `fs::canonicalize` without Windows' `\\?\` verbatim prefix, so paths compare
/// equal to the ones Git reports (`C:/dir`) and display normally.
pub(crate) fn canonical_path(path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(plain) = text.strip_prefix(r"\\?\")
            && plain.as_bytes().get(1) == Some(&b':')
        {
            return Some(PathBuf::from(plain));
        }
    }
    Some(canonical)
}

/// Directory names never searched for nested repositories.
const SKIPPED_DIRECTORIES: &[&str] = &["node_modules", "target", "vendor", "dist", "build"];
const MAX_DISCOVERY_DEPTH: usize = 4;
const MAX_DISCOVERY_DIRECTORIES: usize = 5000;

pub(crate) trait SourceControlService: Send + Sync {
    /// Every repository root relevant to the opened folder: the repository that
    /// contains it (if any) plus nested repositories found by a bounded walk.
    fn discover(&self, workspace: &Path) -> Result<Vec<PathBuf>, String>;
    fn snapshot(&self, workspace: &Path) -> Result<Option<GitSnapshot>, String>;
    fn diff(&self, root: &Path, path: &Path, side: DiffSide) -> Result<String, String>;
    fn comparison(&self, root: &Path, path: &Path, side: DiffSide)
    -> Result<GitComparison, String>;
    fn stage(&self, root: &Path, paths: &[PathBuf]) -> Result<(), String>;
    fn unstage(&self, root: &Path, paths: &[PathBuf]) -> Result<(), String>;
    /// Restore tracked paths and their index entries from HEAD. Deliberately
    /// excludes untracked files: source control must never emulate `git clean`.
    fn revert(&self, root: &Path, paths: &[PathBuf]) -> Result<(), String>;
    fn commit(&self, root: &Path, message: &str, amend: bool) -> Result<(), String>;
    fn pull(&self, root: &Path) -> Result<(), String>;
    fn push(&self, root: &Path) -> Result<(), String>;
    fn stage_hunk(
        &self,
        root: &Path,
        path: &Path,
        hunk: usize,
        expected_patch: &str,
    ) -> Result<(), String>;
}

pub(crate) struct LocalGit;

impl SourceControlService for LocalGit {
    fn discover(&self, workspace: &Path) -> Result<Vec<PathBuf>, String> {
        let mut roots = Vec::new();
        let enclosing = git_command(workspace)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|error| format!("Could not start Git: {error}"))?;
        if enclosing.status.success() {
            let path = String::from_utf8(enclosing.stdout)
                .map_err(|_| "Git repository path is not UTF-8".to_owned())?;
            roots.push(PathBuf::from(path.trim_end_matches(['\r', '\n'])));
        }
        // Nested repositories never traverse symlinks and skip generated trees.
        let mut pending = vec![(workspace.to_path_buf(), 0usize)];
        let mut visited = 0;
        while let Some((directory, depth)) = pending.pop() {
            visited += 1;
            if visited > MAX_DISCOVERY_DIRECTORIES {
                break;
            }
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == ".git" {
                    // A `.git` directory or file marks a worktree root.
                    if let Some(top) = canonical_path(&directory)
                        && directory != workspace
                    {
                        roots.push(top);
                    }
                } else if kind.is_dir()
                    && depth < MAX_DISCOVERY_DEPTH
                    && !name.starts_with('.')
                    && !SKIPPED_DIRECTORIES.contains(&name.as_ref())
                {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
        roots.sort();
        roots.dedup();
        Ok(roots)
    }

    fn snapshot(&self, workspace: &Path) -> Result<Option<GitSnapshot>, String> {
        let root = git_command(workspace)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|error| format!("Could not start Git: {error}"))?;
        if !root.status.success() {
            return Ok(None);
        }
        let root = String::from_utf8(root.stdout)
            .map_err(|_| "Git repository path is not UTF-8".to_owned())?;
        let root = PathBuf::from(root.trim_end_matches(['\r', '\n']));
        let status = git_command(&root)
            .args([
                "--no-optional-locks",
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=normal",
            ])
            .output()
            .map_err(|error| format!("Could not read Git status: {error}"))?;
        if !status.status.success() {
            return Err("Git status failed; check repository access".into());
        }
        let parsed = parse_status(&status.stdout)?;
        let branch = match &parsed.head {
            GitHead::Attached(name) => name.clone(),
            GitHead::Unborn(name) => format!("{name} (no commits yet)"),
            GitHead::Detached(oid) => format!("Detached HEAD at {}", &oid[..oid.len().min(7)]),
            GitHead::Unknown => "Detached HEAD".into(),
        };
        let (git_dir, common_dir) = git_directories(&root);
        Ok(Some(GitSnapshot {
            state: repo_state(&git_dir),
            git_dir,
            common_dir,
            root,
            branch,
            head: parsed.head,
            upstream: parsed.upstream,
            changes: parsed.changes,
        }))
    }

    fn diff(&self, root: &Path, path: &Path, side: DiffSide) -> Result<String, String> {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "Diff target is outside the repository".to_owned())?;
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Diff target is not a repository file".into());
        }
        if side == DiffSide::Untracked {
            return untracked_preview(root, path, relative);
        }
        let mut command = git_command(root);
        command.args([
            "--no-optional-locks",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=3",
        ]);
        if side == DiffSide::Staged {
            command.arg("--cached");
        }
        let mut child = command
            .arg("--")
            .arg(relative)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Could not start Git diff: {error}"))?;
        let mut bytes = Vec::new();
        if let Err(error) = child
            .stdout
            .take()
            .expect("piped stdout")
            .take(MAX_DIFF_BYTES + 1)
            .read_to_end(&mut bytes)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Could not read Git diff: {error}"));
        }
        if bytes.len() as u64 > MAX_DIFF_BYTES {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Diff exceeds the 256 KiB preview limit".into());
        }
        if !child
            .wait()
            .map_err(|error| format!("Could not finish Git diff: {error}"))?
            .success()
        {
            return Err("Git diff failed; refresh repository status".into());
        }
        String::from_utf8(bytes).map_err(|_| "Diff is not UTF-8 text".into())
    }

    fn comparison(
        &self,
        root: &Path,
        path: &Path,
        side: DiffSide,
    ) -> Result<GitComparison, String> {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "Diff target is outside the repository".to_owned())?;
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Diff target is not a repository file".into());
        }
        let patch = self.diff(root, path, side)?;
        let name = relative
            .to_str()
            .ok_or_else(|| "Diff path is not UTF-8".to_owned())?;
        let base = match side {
            DiffSide::Untracked => String::new(),
            DiffSide::Working => read_git_blob(root, &format!(":{name}"))?,
            DiffSide::Staged => read_git_blob(root, &format!("HEAD:{name}"))?,
        };
        let changed = Some(match side {
            DiffSide::Staged => read_git_blob(root, &format!(":{name}"))?,
            DiffSide::Working | DiffSide::Untracked => read_worktree_text(path)?,
        });
        let (old_lines, new_lines) = changed_lines(&patch);
        Ok(GitComparison {
            base,
            changed,
            old_lines,
            new_lines,
            hunks: patch_hunks(&patch)
                .into_iter()
                .map(|patch| GitHunk {
                    header: patch
                        .lines()
                        .find(|line| line.starts_with("@@"))
                        .unwrap_or_default()
                        .to_owned(),
                    patch,
                })
                .collect(),
        })
    }

    fn stage(&self, root: &Path, paths: &[PathBuf]) -> Result<(), String> {
        git_paths(root, ["add", "--"], paths, "stage")
    }

    fn unstage(&self, root: &Path, paths: &[PathBuf]) -> Result<(), String> {
        let head = git_command(root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .map_err(|error| format!("Could not inspect Git HEAD: {error}"))?;
        if head.status.success() {
            git_paths(root, ["restore", "--staged", "--"], paths, "unstage")
        } else {
            // An unborn repository has no HEAD for `restore --staged` to name.
            // Drop only the index entries and leave worktree files untouched.
            git_paths(root, ["rm", "--cached", "-f", "--"], paths, "unstage")
        }
    }

    fn revert(&self, root: &Path, paths: &[PathBuf]) -> Result<(), String> {
        git_paths(
            root,
            ["restore", "--source=HEAD", "--staged", "--worktree", "--"],
            paths,
            "discard changes to",
        )
    }

    fn commit(&self, root: &Path, message: &str, amend: bool) -> Result<(), String> {
        let message = message.trim();
        if message.is_empty() && !amend {
            return Err("Enter a commit message".into());
        }
        let mut command = git_command(root);
        command.args(["--no-optional-locks", "commit"]);
        if amend {
            command.arg("--amend");
            if message.is_empty() {
                command.arg("--no-edit");
            } else {
                command.args(["-m", message]);
            }
        } else {
            command.args(["-m", message]);
        }
        let output = command
            .output()
            .map_err(|error| format!("Could not start Git commit: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            // Hooks may report their reason on either stream.
            let mut detail = output.stderr;
            detail.extend_from_slice(&output.stdout);
            Err(git_failure("Git commit failed", &detail))
        }
    }

    fn pull(&self, root: &Path) -> Result<(), String> {
        git_remote_command(root, ["pull", "--ff-only"], "pull")
    }

    fn push(&self, root: &Path) -> Result<(), String> {
        git_remote_command(root, ["push"], "push")
    }

    fn stage_hunk(
        &self,
        root: &Path,
        path: &Path,
        hunk: usize,
        expected_patch: &str,
    ) -> Result<(), String> {
        let patch = self.diff(root, path, DiffSide::Working)?;
        let Some(patch) = patch_hunks(&patch)
            .into_iter()
            .nth(hunk)
            .filter(|current| current == expected_patch)
        else {
            return Err(
                "That change hunk changed on disk since it was shown; refresh the diff".into(),
            );
        };
        let mut child = git_command(root)
            .args(["--no-optional-locks", "apply", "--cached", "--recount"])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start Git hunk stage: {error}"))?;
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(patch.as_bytes())
            .map_err(|error| format!("Could not send hunk to Git: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("Could not finish Git hunk stage: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(git_failure("Git hunk stage failed", &output.stderr))
        }
    }
}

fn patch_hunks(patch: &str) -> Vec<String> {
    let mut header = Vec::new();
    let mut hunks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in patch.lines() {
        if line.starts_with("@@") {
            if let Some(lines) = current.take() {
                hunks.push(lines);
            }
            current = Some(
                header
                    .iter()
                    .copied()
                    .chain(std::iter::once(line))
                    .collect(),
            );
        } else if let Some(lines) = &mut current {
            lines.push(line);
        } else {
            header.push(line);
        }
    }
    if let Some(lines) = current {
        hunks.push(lines);
    }
    hunks
        .into_iter()
        .map(|lines| format!("{}\n", lines.join("\n")))
        .collect()
}

fn git_paths<const N: usize>(
    root: &Path,
    args: [&str; N],
    paths: &[PathBuf],
    action: &str,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err(format!("No files to {action}"));
    }
    if paths.iter().any(|path| {
        path.as_os_str().is_empty()
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
    }) {
        return Err(format!("Cannot {action} a path outside the repository"));
    }
    let output = git_command(root)
        .args(["--no-optional-locks"])
        .args(args)
        .args(paths)
        .output()
        .map_err(|error| format!("Could not start Git {action}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_failure(&format!("Git {action} failed"), &output.stderr))
    }
}

/// Remote operations can hang on a stalled network or an unanswerable prompt.
const REMOTE_TIMEOUT: Duration = Duration::from_secs(300);

fn git_remote_command<const N: usize>(
    root: &Path,
    args: [&str; N],
    action: &str,
) -> Result<(), String> {
    run_remote(
        git_command(root)
            .args(["--no-optional-locks"])
            .args(args)
            // No terminal exists to answer a credential prompt; fail instead
            // of waiting. Credential helpers and agents still work.
            .env("GIT_TERMINAL_PROMPT", "0"),
        action,
        REMOTE_TIMEOUT,
    )
}

/// Runs a prepared Git command, draining its output concurrently and killing it
/// once `timeout` passes. A timeout is reported as an uncertain outcome because
/// the remote may have applied the change before the process was stopped.
fn run_remote(command: &mut Command, action: &str, timeout: Duration) -> Result<(), String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start Git {action}: {error}"))?;
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|s| Box::new(s) as _));
    let stderr = drain(child.stderr.take().map(|s| Box::new(s) as _));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                return Err(format!("Could not finish Git {action}: {error}"));
            }
        }
    };
    let mut detail = stderr.join().unwrap_or_default();
    detail.extend(stdout.join().unwrap_or_default());
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(git_failure(&format!("Git {action} failed"), &detail)),
        Err(()) => Err(format!(
            "Git {action} timed out after {} seconds and was stopped; the remote may or may not have applied it. Refresh and check before retrying",
            timeout.as_secs()
        )),
    }
}

fn git_failure(prefix: &str, stderr: &[u8]) -> String {
    let detail = String::from_utf8_lossy(stderr).trim().to_owned();
    if detail.is_empty() {
        prefix.into()
    } else {
        format!("{prefix}: {detail}")
    }
}

fn read_worktree_text(path: &Path) -> Result<String, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("Could not inspect working file: {error}")),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Working comparison supports regular files only".into());
    }
    if metadata.len() > MAX_DIFF_BYTES {
        return Err("Working file exceeds the 256 KiB comparison limit".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| format!("Could not open working file: {error}"))?
        .take(MAX_DIFF_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read working file: {error}"))?;
    if bytes.len() as u64 > MAX_DIFF_BYTES {
        return Err("Working file exceeds the 256 KiB comparison limit".into());
    }
    if bytes.contains(&0) {
        return Err("Binary working files have no text comparison".into());
    }
    String::from_utf8(bytes).map_err(|_| "Working file is not UTF-8 text".into())
}

fn changed_lines(patch: &str) -> (BTreeSet<usize>, BTreeSet<usize>) {
    let mut old = BTreeSet::new();
    let mut new = BTreeSet::new();
    let mut position = None;
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("@@ -") {
            let Some((old_range, remainder)) = header.split_once(' ') else {
                continue;
            };
            let Some(new_range) = remainder
                .strip_prefix('+')
                .and_then(|rest| rest.split(' ').next())
            else {
                continue;
            };
            position = old_range
                .split(',')
                .next()
                .and_then(|start| start.parse::<usize>().ok())
                .zip(
                    new_range
                        .split(',')
                        .next()
                        .and_then(|start| start.parse::<usize>().ok()),
                );
            continue;
        }
        let Some((old_line, new_line)) = &mut position else {
            continue;
        };
        if line.starts_with('-') {
            old.insert(*old_line);
            *old_line += 1;
        } else if line.starts_with('+') {
            new.insert(*new_line);
            *new_line += 1;
        } else if line.starts_with(' ') {
            *old_line += 1;
            *new_line += 1;
        }
    }
    (old, new)
}

fn read_git_blob(root: &Path, spec: &str) -> Result<String, String> {
    let mut child = git_command(root)
        .args(["--no-optional-locks", "show", spec])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start Git content read: {error}"))?;
    let mut bytes = Vec::new();
    if let Err(error) = child
        .stdout
        .take()
        .expect("piped stdout")
        .take(MAX_DIFF_BYTES + 1)
        .read_to_end(&mut bytes)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Could not read Git content: {error}"));
    }
    if bytes.len() as u64 > MAX_DIFF_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Git content exceeds the 256 KiB comparison limit".into());
    }
    if !child
        .wait()
        .map_err(|error| format!("Could not finish Git content read: {error}"))?
        .success()
    {
        // A newly added file has no HEAD or index version yet.
        return Ok(String::new());
    }
    if bytes.contains(&0) {
        return Err("Binary Git content has no text comparison".into());
    }
    String::from_utf8(bytes).map_err(|_| "Git content is not UTF-8 text".into())
}

fn untracked_preview(root: &Path, path: &Path, relative: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not inspect untracked file: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Untracked preview supports regular files only".into());
    }
    if metadata.len() > MAX_UNTRACKED_BYTES {
        return Err("Untracked file exceeds the 128 KiB preview limit".into());
    }
    let resolved = fs::canonicalize(path)
        .map_err(|error| format!("Could not resolve untracked file: {error}"))?;
    let resolved_root = fs::canonicalize(root)
        .map_err(|error| format!("Could not resolve Git repository path: {error}"))?;
    if !resolved.starts_with(&resolved_root) {
        return Err("Untracked file resolves outside the repository".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| format!("Could not open untracked file: {error}"))?
        .take(MAX_UNTRACKED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read untracked file: {error}"))?;
    if bytes.len() as u64 > MAX_UNTRACKED_BYTES {
        return Err("Untracked file exceeds the 128 KiB preview limit".into());
    }
    if bytes.contains(&0) {
        return Err("Binary untracked files have no text preview".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "Untracked file is not UTF-8 text")?;
    let name = format!("{:?}", relative.to_string_lossy());
    let count = text.split_inclusive('\n').count();
    let mut preview = format!(
        "diff --git a/{name} b/{name}\nnew file mode 100644\n--- /dev/null\n+++ b/{name}\n@@ -0,0 +1,{count} @@\n"
    );
    for line in text.split_inclusive('\n') {
        preview.push('+');
        preview.push_str(line);
        if !line.ends_with('\n') {
            preview.push('\n');
            preview.push_str("\\ No newline at end of file\n");
        }
        if preview.len() as u64 > MAX_DIFF_BYTES {
            return Err("Untracked diff exceeds the 256 KiB preview limit".into());
        }
    }
    Ok(preview)
}

fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .stdin(Stdio::null())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["--literal-pathspecs", "-c", "core.fsmonitor=false", "-C"])
        .arg(root);
    // A GUI process has no console of its own on Windows. Without this flag,
    // each background Git invocation can briefly create a visible console
    // window. Keep all Git transport centralized here so refresh, diff, index
    // mutations, commits, and remote operations share the same behavior.
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// The `sub` field is `N...` for ordinary entries and `S<c><m><u>` for a
/// submodule, where each flag letter is set when that kind of change exists.
fn submodule_field(line: &str) -> Option<SubmoduleChange> {
    let field = line.split(' ').nth(2)?;
    let field = field.strip_prefix('S')?.as_bytes();
    Some(SubmoduleChange {
        commit: field.first() == Some(&b'C'),
        modified_content: field.get(1) == Some(&b'M'),
        untracked_content: field.get(2) == Some(&b'U'),
    })
}

struct ParsedStatus {
    head: GitHead,
    upstream: Option<GitUpstream>,
    changes: Vec<GitChange>,
}

/// Absolute git directory of this worktree and the shared common directory.
fn git_directories(root: &Path) -> (PathBuf, PathBuf) {
    let read = |args: &[&str]| {
        git_command(root)
            .args(["rev-parse"])
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|text| PathBuf::from(text.trim_end_matches(['\r', '\n'])))
    };
    let git_dir = read(&["--absolute-git-dir"]).unwrap_or_default();
    // `--path-format` needs Git 2.31; without it the common dir may be relative
    // to the root, so resolve it here and compare canonical paths.
    let common = read(&["--git-common-dir"])
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        })
        .and_then(|path| canonical_path(&path))
        .unwrap_or_else(|| git_dir.clone());
    let git_dir = canonical_path(&git_dir).unwrap_or(git_dir);
    (git_dir, common)
}

/// Reads the in-progress operation from state files in the git directory (for a
/// linked worktree that is not `<root>/.git`).
fn repo_state(dir: &Path) -> RepoState {
    let has = |name: &str| dir.join(name).exists();
    if has("rebase-merge") || has("rebase-apply") {
        RepoState::Rebase
    } else if has("MERGE_HEAD") {
        RepoState::Merge
    } else if has("CHERRY_PICK_HEAD") {
        RepoState::CherryPick
    } else if has("REVERT_HEAD") {
        RepoState::Revert
    } else {
        RepoState::Clean
    }
}

/// Parses `git status --porcelain=v2 --branch -z`. Header records start with
/// `# `; `1`/`2`/`u`/`?` records carry changes. Paths follow a fixed number of
/// space-separated fields, so they may contain spaces.
fn parse_status(bytes: &[u8]) -> Result<ParsedStatus, String> {
    let bad = || "Unrecognized Git status record".to_owned();
    let text = |bytes: &[u8]| {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "A Git path is not UTF-8".to_owned())
    };
    let mut head = GitHead::Unknown;
    let mut oid = String::new();
    let mut upstream: Option<GitUpstream> = None;
    let mut changes = Vec::new();
    let mut records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        let line = text(record)?;
        if let Some(header) = line.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').ok_or_else(bad)?;
            match key {
                "branch.oid" => oid = value.to_owned(),
                "branch.head" => {
                    head = if value == "(detached)" {
                        GitHead::Detached(String::new())
                    } else if oid == "(initial)" {
                        GitHead::Unborn(value.to_owned())
                    } else {
                        GitHead::Attached(value.to_owned())
                    };
                }
                "branch.upstream" => {
                    upstream = Some(GitUpstream {
                        name: value.to_owned(),
                        ahead_behind: None,
                    });
                }
                "branch.ab" => {
                    let (ahead, behind) = value.split_once(' ').ok_or_else(bad)?;
                    let ahead = ahead.strip_prefix('+').and_then(|n| n.parse().ok());
                    let behind = behind.strip_prefix('-').and_then(|n| n.parse().ok());
                    if let (Some(upstream), Some(ahead), Some(behind)) =
                        (&mut upstream, ahead, behind)
                    {
                        upstream.ahead_behind = Some((ahead, behind));
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = line.as_bytes()[0];
        let xy = || {
            let xy = line.as_bytes().get(2..4).ok_or_else(bad)?;
            // v2 marks an unchanged side with '.'; the rest of Lumen uses ' '.
            let side = |byte: u8| if byte == b'.' { ' ' } else { byte as char };
            Ok::<_, String>((side(xy[0]), side(xy[1])))
        };
        match kind {
            b'?' => {
                let path = line.get(2..).ok_or_else(bad)?;
                changes.push(GitChange::new(path, '?', '?'));
            }
            b'1' => {
                let (index, worktree) = xy()?;
                // 1 XY sub mH mI mW hH hI path
                let path = line.splitn(9, ' ').nth(8).ok_or_else(bad)?;
                let mut change = GitChange::new(path, index, worktree);
                if let Some(sub) = submodule_field(&line) {
                    change.kind = ChangeKind::Submodule(sub);
                }
                changes.push(change);
            }
            b'2' => {
                let (index, worktree) = xy()?;
                // 2 XY sub mH mI mW hH hI Xscore path, then the source path.
                let path = line.splitn(10, ' ').nth(9).ok_or_else(bad)?;
                let from = text(records.next().ok_or_else(bad)?)?;
                let mut change = GitChange::new(path, index, worktree);
                change.kind = ChangeKind::Renamed {
                    from: PathBuf::from(from),
                };
                changes.push(change);
            }
            b'u' => {
                let (index, worktree) = xy()?;
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let path = line.splitn(11, ' ').nth(10).ok_or_else(bad)?;
                let mut change = GitChange::new(path, index, worktree);
                change.kind = ChangeKind::Conflict;
                changes.push(change);
            }
            _ => return Err(bad()),
        }
    }
    if let GitHead::Detached(name) = &mut head {
        *name = oid.clone();
    }
    Ok(ParsedStatus {
        head,
        upstream,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn nul_status_preserves_spaces_and_newlines() {
        let parsed =
            parse_status(b"1 .M N... 100644 100644 100644 a b with space.rs\0? line\nbreak.rs\0")
                .unwrap();
        let changes = parsed.changes;
        assert_eq!(changes[0].path, PathBuf::from("with space.rs"));
        assert_eq!((changes[0].index, changes[0].worktree), (' ', 'M'));
        assert_eq!(changes[1].path, PathBuf::from("line\nbreak.rs"));
        assert_eq!(changes[1].index, '?');
    }

    #[test]
    fn porcelain_v2_headers_renames_and_conflicts_are_typed() {
        let parsed = parse_status(
            b"# branch.oid abcdef0123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
2 R. N... 100644 100644 100644 a b R100 new name.rs\0old name.rs\0\
u UU N... 100644 100644 100644 100644 a b c both.rs\0",
        )
        .unwrap();
        assert_eq!(parsed.head, GitHead::Attached("main".into()));
        let upstream = parsed.upstream.unwrap();
        assert_eq!(upstream.name, "origin/main");
        assert_eq!(upstream.ahead_behind, Some((2, 1)));
        assert_eq!(parsed.changes[0].path, PathBuf::from("new name.rs"));
        assert_eq!(
            parsed.changes[0].kind,
            ChangeKind::Renamed {
                from: PathBuf::from("old name.rs")
            }
        );
        assert!(parsed.changes[1].is_conflict());
        assert_eq!(parsed.changes[1].path, PathBuf::from("both.rs"));
        assert_eq!(
            (parsed.changes[1].index, parsed.changes[1].worktree),
            ('U', 'U')
        );
    }

    #[test]
    fn unborn_and_detached_heads_are_distinct_states() {
        let unborn = parse_status(b"# branch.oid (initial)\0# branch.head trunk\0").unwrap();
        assert_eq!(unborn.head, GitHead::Unborn("trunk".into()));
        assert!(unborn.upstream.is_none());
        let detached =
            parse_status(b"# branch.oid 1234567890\0# branch.head (detached)\0").unwrap();
        assert_eq!(detached.head, GitHead::Detached("1234567890".into()));
    }

    #[test]
    fn a_missing_upstream_count_leaves_the_upstream_without_numbers() {
        let parsed =
            parse_status(b"# branch.oid abc\0# branch.head main\0# branch.upstream origin/gone\0")
                .unwrap();
        assert_eq!(parsed.upstream.unwrap().ahead_behind, None);
    }

    #[test]
    fn local_git_discovers_repo_and_keeps_status_and_diff_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        fs::write(root.join("file name.rs"), "old\n").unwrap();
        git(&["add", "--", "file name.rs"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ]);
        fs::write(root.join("file name.rs"), "new\n").unwrap();
        fs::write(root.join("untracked.rs"), "new\n").unwrap();
        let service = LocalGit;
        let snapshot = service.snapshot(root).unwrap().unwrap();
        // Windows canonicalization adds an extended-length prefix that Git's
        // `--show-toplevel` output does not use; compare the resolved paths.
        assert_eq!(
            fs::canonicalize(&snapshot.root).unwrap(),
            fs::canonicalize(root).unwrap()
        );
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("file name.rs") && change.worktree == 'M')
        );
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("untracked.rs") && change.index == '?')
        );
        let diff = service
            .diff(
                &snapshot.root,
                &snapshot.root.join("file name.rs"),
                DiffSide::Working,
            )
            .unwrap();
        assert!(diff.contains("+new"));
        let comparison = service
            .comparison(
                &snapshot.root,
                &snapshot.root.join("file name.rs"),
                DiffSide::Working,
            )
            .unwrap();
        assert_eq!(comparison.base, "old\n");
        assert_eq!(comparison.changed.as_deref(), Some("new\n"));
        assert!(comparison.new_lines.contains(&1));
        let untracked = service
            .diff(
                &snapshot.root,
                &snapshot.root.join("untracked.rs"),
                DiffSide::Untracked,
            )
            .unwrap();
        assert!(untracked.contains("+++ b/\"untracked.rs\""));
        assert!(untracked.contains("+new"));
        let untracked_comparison = service
            .comparison(
                &snapshot.root,
                &snapshot.root.join("untracked.rs"),
                DiffSide::Untracked,
            )
            .unwrap();
        assert!(untracked_comparison.base.is_empty());
        assert_eq!(untracked_comparison.changed.as_deref(), Some("new\n"));
        git(&["add", "--", "file name.rs"]);
        let staged = service
            .comparison(
                &snapshot.root,
                &snapshot.root.join("file name.rs"),
                DiffSide::Staged,
            )
            .unwrap();
        assert_eq!(staged.base, "old\n");
        assert_eq!(staged.changed.as_deref(), Some("new\n"));
        assert_eq!(
            fs::read_to_string(root.join("file name.rs")).unwrap(),
            "new\n"
        );
    }

    #[test]
    fn hunk_parser_marks_content_that_looks_like_patch_headers() {
        let (old, new) =
            changed_lines("--- a/file\n+++ b/file\n@@ -1,1 +1,1 @@\n--- old text\n+++ new text\n");
        assert_eq!(old, [1].into());
        assert_eq!(new, [1].into());
    }

    #[test]
    fn untracked_preview_rejects_binary_and_escape_paths() {
        let directory = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(directory.path()).unwrap();
        fs::write(root.join("binary"), b"hello\0world").unwrap();
        let service = LocalGit;
        assert!(
            service
                .diff(&root, &root.join("binary"), DiffSide::Untracked)
                .unwrap_err()
                .contains("Binary")
        );
        assert!(
            service
                .diff(&root, &root.join("../outside"), DiffSide::Untracked)
                .is_err()
        );
    }

    #[test]
    fn stage_unstage_and_commit_are_scoped_to_requested_paths() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["config", "core.autocrlf", "false"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        fs::write(root.join("one.rs"), "one\n").unwrap();
        fs::write(root.join("two.rs"), "two\n").unwrap();
        let service = LocalGit;
        service.stage(root, &[PathBuf::from("one.rs")]).unwrap();
        let snapshot = service.snapshot(root).unwrap().unwrap();
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("one.rs") && change.index == 'A')
        );
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("two.rs") && change.index == '?')
        );
        fs::write(root.join("one.rs"), "edited after staging\n").unwrap();
        service.unstage(root, &[PathBuf::from("one.rs")]).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("one.rs")).unwrap(),
            "edited after staging\n"
        );
        assert!(
            service
                .snapshot(root)
                .unwrap()
                .unwrap()
                .changes
                .iter()
                .all(|change| change.index != 'A')
        );
        service.stage(root, &[PathBuf::from("one.rs")]).unwrap();
        service.commit(root, "add one", false).unwrap();
        let snapshot = service.snapshot(root).unwrap().unwrap();
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("two.rs") && change.index == '?')
        );
        assert!(
            snapshot
                .changes
                .iter()
                .all(|change| change.path != Path::new("one.rs"))
        );
    }

    #[test]
    fn revert_restores_only_requested_tracked_paths_and_keeps_untracked_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["config", "core.autocrlf", "false"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        fs::write(root.join("one.rs"), "one\n").unwrap();
        fs::write(root.join("two.rs"), "two\n").unwrap();
        git(&["add", "--", "one.rs", "two.rs"]);
        git(&["commit", "--quiet", "-m", "initial"]);

        fs::write(root.join("one.rs"), "changed once\n").unwrap();
        fs::write(root.join("two.rs"), "changed twice\n").unwrap();
        git(&["add", "--", "one.rs"]);
        fs::write(root.join("new.rs"), "untracked\n").unwrap();

        LocalGit.revert(root, &[PathBuf::from("one.rs")]).unwrap();

        assert_eq!(fs::read_to_string(root.join("one.rs")).unwrap(), "one\n");
        assert_eq!(
            fs::read_to_string(root.join("two.rs")).unwrap(),
            "changed twice\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("new.rs")).unwrap(),
            "untracked\n"
        );
        let snapshot = LocalGit.snapshot(root).unwrap().unwrap();
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("two.rs"))
        );
        assert!(
            snapshot
                .changes
                .iter()
                .any(|change| change.path == Path::new("new.rs") && change.index == '?')
        );
        assert!(
            snapshot
                .changes
                .iter()
                .all(|change| change.path != Path::new("one.rs"))
        );
    }

    #[test]
    fn stage_hunk_only_updates_the_requested_hunk_in_the_index() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        let initial = "one\n2\n3\n4\n5\n6\n7\n8\n9\nten\n11\n12\n13\n14\n15\n";
        fs::write(root.join("file.txt"), initial).unwrap();
        git(&["add", "--", "file.txt"]);
        git(&["commit", "--quiet", "-m", "initial"]);
        let changed = "ONE\n2\n3\n4\n5\n6\n7\n8\n9\nten\n11\n12\n13\n14\nFIFTEEN\n";
        fs::write(root.join("file.txt"), changed).unwrap();
        let service = LocalGit;
        let root = fs::canonicalize(root).unwrap();
        let comparison = service
            .comparison(&root, &root.join("file.txt"), DiffSide::Working)
            .unwrap();
        assert_eq!(comparison.hunks.len(), 2);
        let stale = comparison.hunks[0].patch.clone();
        // An external edit that changes the first hunk must be rejected rather
        // than staged under the old preview.
        fs::write(root.join("file.txt"), changed.replacen("ONE", "EDITED", 1)).unwrap();
        let error = service
            .stage_hunk(&root, &root.join("file.txt"), 0, &stale)
            .unwrap_err();
        assert!(error.contains("changed on disk"), "{error}");
        assert!(
            service
                .diff(&root, &root.join("file.txt"), DiffSide::Staged)
                .unwrap()
                .is_empty()
        );
        fs::write(root.join("file.txt"), changed).unwrap();
        service
            .stage_hunk(&root, &root.join("file.txt"), 0, &stale)
            .unwrap();
        let staged = service
            .diff(&root, &root.join("file.txt"), DiffSide::Staged)
            .unwrap();
        assert!(staged.contains("+ONE"));
        assert!(!staged.contains("+FIFTEEN"));
        let working = service
            .diff(&root, &root.join("file.txt"), DiffSide::Working)
            .unwrap();
        assert!(working.contains("+FIFTEEN"));
        assert!(!working.contains("+ONE"));
    }

    fn init_repo(root: &Path) {
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--quiet"]);
        git(&["config", "core.autocrlf", "false"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
    }

    #[test]
    fn discovery_finds_enclosing_sibling_and_nested_repositories_but_skips_generated_trees() {
        let directory = tempfile::tempdir().unwrap();
        let top = canonical_path(directory.path()).unwrap();
        init_repo(&top);
        for name in [
            "libs/a",
            "libs/b",
            "node_modules/pkg",
            "deep/x/y/z/w/too_deep",
        ] {
            fs::create_dir_all(top.join(name)).unwrap();
            init_repo(&top.join(name));
        }
        // A subfolder opened inside the outer repository sees only what is below it.
        let found = LocalGit.discover(&top.join("libs")).unwrap();
        assert_eq!(
            found,
            vec![top.clone(), top.join("libs/a"), top.join("libs/b")]
        );
        // Opening the root also reports the skipped-depth/generated trees as absent.
        let found = LocalGit.discover(&top).unwrap();
        assert!(found.contains(&top.join("libs/a")));
        assert!(
            !found
                .iter()
                .any(|root| root.starts_with(top.join("node_modules")))
        );
        assert!(!found.iter().any(|root| root.ends_with("too_deep")));
    }

    #[test]
    fn a_folder_without_a_repository_has_no_roots_unless_repositories_are_nested() {
        let directory = tempfile::tempdir().unwrap();
        let top = canonical_path(directory.path()).unwrap();
        assert!(LocalGit.discover(&top).unwrap().is_empty());
        fs::create_dir_all(top.join("one")).unwrap();
        init_repo(&top.join("one"));
        assert_eq!(LocalGit.discover(&top).unwrap(), vec![top.join("one")]);
    }

    #[test]
    fn submodule_status_flags_are_parsed_from_the_sub_field() {
        let parsed = parse_status(
            b"1 .M SCMU 160000 160000 160000 a b child\0\
1 .M N... 100644 100644 100644 a b plain.rs\0",
        )
        .unwrap();
        assert_eq!(
            parsed.changes[0].kind,
            ChangeKind::Submodule(SubmoduleChange {
                commit: true,
                modified_content: true,
                untracked_content: true
            })
        );
        assert!(parsed.changes[0].is_submodule());
        assert_eq!(parsed.changes[1].kind, ChangeKind::Ordinary);
        assert!(SubmoduleChange::default().describe().contains("submodule"));
    }

    #[test]
    fn linked_worktrees_share_storage_but_have_their_own_state() {
        let directory = tempfile::tempdir().unwrap();
        let top = canonical_path(directory.path()).unwrap();
        let main = top.join("main");
        fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        let git = |dir: &Path, args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        fs::write(main.join("f.txt"), "x\n").unwrap();
        git(&main, &["add", "--", "f.txt"]);
        git(&main, &["commit", "-q", "-m", "init"]);
        let linked = top.join("linked");
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                linked.to_str().unwrap(),
            ],
        );
        let a = LocalGit.snapshot(&main).unwrap().unwrap();
        let b = LocalGit.snapshot(&linked).unwrap().unwrap();
        assert!(!a.is_linked_worktree());
        assert!(b.is_linked_worktree());
        assert!(a.shares_storage_with(&b));
        assert_ne!(a.git_dir, b.git_dir);
        assert_eq!(b.head, GitHead::Attached("side".into()));
        // A merge started in the linked worktree is invisible to the main one.
        fs::write(
            b.git_dir.join("MERGE_HEAD"),
            "0000000000000000000000000000000000000000\n",
        )
        .unwrap();
        assert_eq!(
            LocalGit.snapshot(&linked).unwrap().unwrap().state,
            RepoState::Merge
        );
        assert_eq!(
            LocalGit.snapshot(&main).unwrap().unwrap().state,
            RepoState::Clean
        );
    }

    #[test]
    fn a_real_submodule_is_discovered_and_reported_in_its_parent() {
        let directory = tempfile::tempdir().unwrap();
        let top = canonical_path(directory.path()).unwrap();
        let child = top.join("child_src");
        let parent = top.join("parent");
        fs::create_dir_all(&child).unwrap();
        fs::create_dir_all(&parent).unwrap();
        init_repo(&child);
        init_repo(&parent);
        let git = |dir: &Path, args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "protocol.file.allow=always"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        fs::write(child.join("c.txt"), "c\n").unwrap();
        git(&child, &["add", "--", "c.txt"]);
        git(&child, &["commit", "-q", "-m", "child"]);
        git(
            &parent,
            &["submodule", "add", "-q", child.to_str().unwrap(), "mod"],
        );
        git(&parent, &["commit", "-q", "-m", "add submodule"]);
        fs::write(parent.join("mod/new.txt"), "untracked\n").unwrap();
        let roots = LocalGit.discover(&parent).unwrap();
        assert_eq!(roots, vec![parent.clone(), parent.join("mod")]);
        let snapshot = LocalGit.snapshot(&parent).unwrap().unwrap();
        let change = snapshot
            .changes
            .iter()
            .find(|change| change.path == Path::new("mod"))
            .expect("dirty submodule is listed");
        let ChangeKind::Submodule(sub) = &change.kind else {
            panic!("expected a submodule change: {change:?}");
        };
        assert!(sub.untracked_content);
    }

    #[test]
    fn a_repository_above_the_opened_folder_owns_it() {
        let directory = tempfile::tempdir().unwrap();
        let top = canonical_path(directory.path()).unwrap();
        init_repo(&top);
        let inner = top.join("a/b");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("x.txt"), "x\n").unwrap();
        assert_eq!(LocalGit.discover(&inner).unwrap(), vec![top.clone()]);
        let snapshot = LocalGit.snapshot(&inner).unwrap().unwrap();
        assert_eq!(snapshot.root, top);
        assert!(
            snapshot.changes.iter().any(|c| c.path.starts_with("a")) // untracked dirs are reported collapsed
        );
    }

    #[test]
    fn a_real_merge_conflict_is_reported_as_a_conflict_in_a_merge_state() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["checkout", "-q", "-b", "main"]);
        fs::write(root.join("f.txt"), "base\n").unwrap();
        git(&["add", "--", "f.txt"]);
        git(&["commit", "-q", "-m", "base"]);
        git(&["checkout", "-q", "-b", "other"]);
        fs::write(root.join("f.txt"), "other\n").unwrap();
        git(&["commit", "-q", "-am", "other"]);
        git(&["checkout", "-q", "main"]);
        fs::write(root.join("f.txt"), "main\n").unwrap();
        git(&["commit", "-q", "-am", "main"]);
        assert!(!git(&["merge", "other"]).status.success());
        let snapshot = LocalGit.snapshot(root).unwrap().unwrap();
        assert_eq!(snapshot.state, RepoState::Merge);
        assert_eq!(snapshot.head, GitHead::Attached("main".into()));
        let change = &snapshot.changes[0];
        assert!(change.is_conflict());
        assert_eq!((change.index, change.worktree), ('U', 'U'));
    }

    #[test]
    fn a_staged_rename_is_reported_with_its_source() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        fs::write(
            root.join("old.txt"),
            "content that is long enough to match\n",
        )
        .unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["add", "--", "old.txt"]);
        git(&["commit", "-q", "-m", "init"]);
        git(&["mv", "old.txt", "new.txt"]);
        let snapshot = LocalGit.snapshot(root).unwrap().unwrap();
        assert_eq!(snapshot.changes[0].path, Path::new("new.txt"));
        assert_eq!(
            snapshot.changes[0].kind,
            ChangeKind::Renamed {
                from: PathBuf::from("old.txt")
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn commit_runs_hooks_and_reports_their_output() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        let hook = root.join(".git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\necho hook says no\nexit 1\n").unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("a.txt"), "a\n").unwrap();
        let service = LocalGit;
        service.stage(root, &[PathBuf::from("a.txt")]).unwrap();
        let error = service.commit(root, "blocked", false).unwrap_err();
        assert!(error.contains("hook says no"), "{error}");
        // The staged change survives so the caller can keep the draft and retry.
        assert!(
            service
                .snapshot(root)
                .unwrap()
                .unwrap()
                .changes
                .iter()
                .any(|change| change.path == Path::new("a.txt") && change.index == 'A')
        );
    }

    #[cfg(unix)]
    #[test]
    fn wildcard_file_names_are_targeted_literally() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        init_repo(root);
        fs::write(root.join("*.txt"), "star\n").unwrap();
        fs::write(root.join("a.txt"), "a\n").unwrap();
        fs::write(root.join(":(top)b.txt"), "magic\n").unwrap();
        let service = LocalGit;
        service
            .stage(
                root,
                &[PathBuf::from("*.txt"), PathBuf::from(":(top)b.txt")],
            )
            .unwrap();
        let changes = service.snapshot(root).unwrap().unwrap().changes;
        let staged = |name: &str| {
            changes
                .iter()
                .any(|change| change.path == Path::new(name) && change.index == 'A')
        };
        assert!(staged("*.txt"));
        assert!(staged(":(top)b.txt"));
        assert!(!staged("a.txt"), "a glob must not stage other files");
    }

    #[cfg(unix)]
    #[test]
    fn remote_commands_time_out_and_report_an_uncertain_outcome() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let started = Instant::now();
        let error = run_remote(&mut command, "push", Duration::from_millis(200)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("may or may not"), "{error}");
    }

    #[test]
    fn amend_keeps_or_replaces_the_latest_commit_message() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        git(&["init", "--quiet"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        fs::write(root.join("file.txt"), "first\n").unwrap();
        let service = LocalGit;
        service.stage(root, &[PathBuf::from("file.txt")]).unwrap();
        service.commit(root, "initial", false).unwrap();
        fs::write(root.join("file.txt"), "second\n").unwrap();
        service.stage(root, &[PathBuf::from("file.txt")]).unwrap();
        service.commit(root, "", true).unwrap();
        assert_eq!(
            String::from_utf8(git(&["log", "-1", "--pretty=%s"]).stdout)
                .unwrap()
                .trim(),
            "initial"
        );
        service.commit(root, "renamed", true).unwrap();
        assert_eq!(
            String::from_utf8(git(&["log", "-1", "--pretty=%s"]).stdout)
                .unwrap()
                .trim(),
            "renamed"
        );
    }
}
