//! Child-process tree helpers shared by the agent and task runners: isolate a
//! child in its own group, ask its whole tree to stop, force it, and find
//! executables the way a shell would.
//!
//! Boundaries, stated plainly: on Unix the tree is the child's process group, so a
//! deliberately detached descendant (its own `setsid`) escapes; on Windows the tree
//! is found by `taskkill /T`, not a Job Object, so descendants started after the
//! call can escape. Neither is a sandbox.
// Used by the agent and task runners; some entry points are only used by one.
#![allow(dead_code)]
#[cfg(windows)]
use std::process::Stdio;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::Duration,
};

/// Put the child in its own process group (Unix) so the whole tree can be killed.
pub(crate) fn isolate_process_tree(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    }
}

/// Ask the child's tree to stop politely, without waiting (SIGTERM to the group;
/// `taskkill /T` on Windows). The caller polls for exit and escalates with
/// [`force_kill_process_tree`] after its own deadline. Returns whether the request
/// was delivered, which is not the same as the tree having stopped.
pub(crate) fn terminate_process_tree(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signalling a process group we created; failure means it is gone.
        unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) == 0 }
    }
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
}

/// Kill the tree now, without waiting (SIGKILL to the group; `taskkill /T /F`).
pub(crate) fn force_kill_process_tree(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: as above.
        unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) == 0 }
    }
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
}

/// Kill the child and everything it started (npx -> node -> agent -> tools).
pub(crate) fn kill_process_tree(child: &Child) {
    #[cfg(unix)]
    {
        terminate_process_tree(child.id());
        thread::sleep(Duration::from_millis(30));
        force_kill_process_tree(child.id());
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Find an executable the way a shell would. On Windows, bare names also try
/// `PATHEXT`-style suffixes, which is how `npx` becomes `npx.cmd`.
pub(crate) fn resolve_program(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let suffixes: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for directory in std::env::split_paths(path_var?) {
        for suffix in suffixes {
            let path = directory.join(format!("{name}{suffix}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_resolve_from_path_and_explicit_paths_must_exist() {
        let directory = tempfile::tempdir().unwrap();
        let tool = directory
            .path()
            .join(if cfg!(windows) { "tool.cmd" } else { "tool" });
        std::fs::write(&tool, "").unwrap();
        let path = std::env::join_paths([directory.path()]).unwrap();
        assert_eq!(resolve_program("tool", Some(&path)), Some(tool.clone()));
        assert_eq!(resolve_program("missing", Some(&path)), None);
        assert_eq!(resolve_program("tool", None), None);
        assert_eq!(resolve_program(tool.to_str().unwrap(), None), Some(tool));
        assert_eq!(
            resolve_program(directory.path().join("nope").to_str().unwrap(), None),
            None
        );
    }
}
