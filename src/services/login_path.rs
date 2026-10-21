//! `PATH` handling for programs Lumen launches (agents, tasks): the login shell's
//! view merged with what the app inherited.
use std::{
    ffi::{OsStr, OsString},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

/// Login-shell directories first (they are what the user's terminal sees), then
/// whatever the app inherited, without duplicates.
pub(crate) fn merge_path(login: Option<&OsStr>, inherited: Option<&OsStr>) -> OsString {
    let mut seen = Vec::new();
    for variable in [login, inherited].into_iter().flatten() {
        for directory in std::env::split_paths(variable) {
            if !directory.as_os_str().is_empty() && !seen.contains(&directory) {
                seen.push(directory);
            }
        }
    }
    std::env::join_paths(seen).unwrap_or_default()
}

const PATH_BEGIN: &str = "__LUMEN_PATH_BEGIN__";
const PATH_END: &str = "__LUMEN_PATH_END__";

/// Text between the markers; shells print banners and escape codes around it.
pub(crate) fn extract_marked_path(output: &str) -> Option<String> {
    let start = output.find(PATH_BEGIN)? + PATH_BEGIN.len();
    let end = output[start..].find(PATH_END)? + start;
    let path = output[start..end].trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// `PATH` as the user's login shell sees it. GUI apps started from the Dock or a
/// file manager do not inherit it, so `node`, `npx` or `codex` would look missing.
/// Returns `None` on Windows (GUI apps inherit the real `PATH` there), on timeout,
/// or when the shell prints nothing usable.
pub(crate) fn login_shell_path(shell: &Path, timeout: Duration) -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    let script = format!("printf '%s%s%s' {PATH_BEGIN} \"$PATH\" {PATH_END}");
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = sender.send(text);
    });
    match receiver.recv_timeout(timeout) {
        Ok(text) => {
            let _ = child.wait();
            extract_marked_path(&text).map(OsString::from)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// Cached for the life of the process: spawning an interactive shell takes
/// noticeable time and its answer does not change while Lumen runs.
pub(crate) fn cached_login_path() -> Option<&'static OsString> {
    static PATH: std::sync::OnceLock<Option<OsString>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var_os("SHELL").map(PathBuf::from)?;
        login_shell_path(&shell, Duration::from_secs(5))
    })
    .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_paths_survive_shell_noise() {
        let noisy =
            format!("Last login: now\n\u{1b}[0m{PATH_BEGIN}/opt/bin:/usr/bin{PATH_END}\nbye");
        assert_eq!(
            extract_marked_path(&noisy).as_deref(),
            Some("/opt/bin:/usr/bin")
        );
        assert_eq!(extract_marked_path("nothing here"), None);
        assert_eq!(
            extract_marked_path(&format!("{PATH_BEGIN}{PATH_END}")),
            None
        );
        assert_eq!(extract_marked_path(&format!("{PATH_BEGIN}/a")), None);
    }

    #[test]
    fn merged_path_keeps_login_directories_first_without_duplicates() {
        let merged = merge_path(
            Some(OsStr::new(if cfg!(windows) {
                "C:\\n;C:\\x"
            } else {
                "/n:/x"
            })),
            Some(OsStr::new(if cfg!(windows) {
                "C:\\x;C:\\y;"
            } else {
                "/x:/y:"
            })),
        );
        let parts: Vec<_> = std::env::split_paths(&merged).collect();
        let expected: Vec<PathBuf> = if cfg!(windows) {
            vec!["C:\\n".into(), "C:\\x".into(), "C:\\y".into()]
        } else {
            vec!["/n".into(), "/x".into(), "/y".into()]
        };
        assert_eq!(parts, expected);
        assert!(merge_path(None, None).is_empty());
    }
}
