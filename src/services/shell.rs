//! Which shell runs terminals and shell tasks. Unix uses `$SHELL`; Windows
//! prefers `%COMSPEC%`/`cmd.exe` but falls back to PowerShell on systems where
//! the command prompt is blocked or absent.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Windows shells in fallback order after the command prompt.
const POWERSHELL_PROGRAMS: [&str; 2] = ["pwsh.exe", "powershell.exe"];

/// The syntax a shell speaks, which decides how a one-shot command is passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellFlavor {
    Posix,
    Cmd,
    PowerShell,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedShell {
    pub(crate) program: OsString,
    pub(crate) flavor: ShellFlavor,
}

impl ResolvedShell {
    /// Arguments placed before a command string to run it non-interactively.
    pub(crate) fn command_args(&self) -> Vec<OsString> {
        match self.flavor {
            ShellFlavor::Posix => vec!["-c".into()],
            ShellFlavor::Cmd => vec!["/C".into()],
            ShellFlavor::PowerShell => {
                vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into()]
            }
        }
    }
}

/// The shell for this platform, looked up in the real environment.
pub(crate) fn platform_shell() -> ResolvedShell {
    if cfg!(windows) {
        let path = std::env::var_os("PATH");
        windows_shell(std::env::var_os("COMSPEC"), |name| {
            find_on_path(name, path.as_deref())
        })
    } else {
        ResolvedShell {
            program: std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()),
            flavor: ShellFlavor::Posix,
        }
    }
}

/// Picks a Windows shell. `locate` resolves a program name to an existing file.
/// A `COMSPEC` pointing at a missing file is ignored, so a stale value cannot
/// hide the PowerShell fallback. If nothing is found, `cmd.exe` is returned so
/// the spawn error names a familiar program.
pub(crate) fn windows_shell(
    comspec: Option<OsString>,
    locate: impl Fn(&str) -> Option<PathBuf>,
) -> ResolvedShell {
    if let Some(comspec) = comspec.filter(|value| Path::new(value).is_file()) {
        return ResolvedShell {
            program: comspec,
            flavor: ShellFlavor::Cmd,
        };
    }
    if let Some(found) = locate("cmd.exe") {
        return ResolvedShell {
            program: found.into_os_string(),
            flavor: ShellFlavor::Cmd,
        };
    }
    for name in POWERSHELL_PROGRAMS {
        if let Some(found) = locate(name) {
            return ResolvedShell {
                program: found.into_os_string(),
                flavor: ShellFlavor::PowerShell,
            };
        }
    }
    ResolvedShell {
        program: "cmd.exe".into(),
        flavor: ShellFlavor::Cmd,
    }
}

/// The shell for the integrated terminal. On Windows `windows_preference`
/// (`"powershell"` or `"cmd"`, from the `terminal.windows_shell` setting)
/// chooses between PowerShell and the command prompt; elsewhere it is ignored.
pub(crate) fn terminal_shell(windows_preference: &str) -> ResolvedShell {
    if cfg!(windows) {
        let path = std::env::var_os("PATH");
        windows_terminal_shell(std::env::var_os("COMSPEC"), windows_preference, |name| {
            find_on_path(name, path.as_deref())
        })
    } else {
        platform_shell()
    }
}

/// PowerShell unless the command prompt is requested. A missing PowerShell
/// falls back to the same lookup tasks use, so a terminal still opens.
pub(crate) fn windows_terminal_shell(
    comspec: Option<OsString>,
    preference: &str,
    locate: impl Fn(&str) -> Option<PathBuf>,
) -> ResolvedShell {
    if preference != "cmd" {
        for name in POWERSHELL_PROGRAMS {
            if let Some(found) = locate(name) {
                return ResolvedShell {
                    program: found.into_os_string(),
                    flavor: ShellFlavor::PowerShell,
                };
            }
        }
    }
    windows_shell(comspec, locate)
}

fn find_on_path(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(available: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
        move |name| {
            available
                .contains(&name)
                .then(|| PathBuf::from(format!("C:\\bin\\{name}")))
        }
    }

    #[test]
    fn command_prompt_is_preferred_when_present() {
        let shell = windows_shell(None, only(&["cmd.exe", "pwsh.exe"]));
        assert_eq!(shell.flavor, ShellFlavor::Cmd);
        assert_eq!(shell.command_args(), vec![OsString::from("/C")]);
    }

    #[test]
    fn powershell_core_then_windows_powershell_replace_a_missing_command_prompt() {
        let shell = windows_shell(None, only(&["pwsh.exe", "powershell.exe"]));
        assert_eq!(shell.flavor, ShellFlavor::PowerShell);
        assert!(shell.program.to_string_lossy().ends_with("pwsh.exe"));
        let shell = windows_shell(
            Some("C:\\missing\\cmd.exe".into()),
            only(&["powershell.exe"]),
        );
        assert_eq!(shell.flavor, ShellFlavor::PowerShell);
        assert!(shell.program.to_string_lossy().ends_with("powershell.exe"));
        assert_eq!(
            shell.command_args().last(),
            Some(&OsString::from("-Command"))
        );
    }

    #[test]
    fn an_existing_comspec_wins_and_nothing_found_still_names_cmd() {
        let here = std::env::current_exe().unwrap();
        let shell = windows_shell(Some(here.clone().into_os_string()), only(&["pwsh.exe"]));
        assert_eq!(shell.program, here.into_os_string());
        assert_eq!(shell.flavor, ShellFlavor::Cmd);
        let shell = windows_shell(None, only(&[]));
        assert_eq!(shell.program, OsString::from("cmd.exe"));
    }

    #[test]
    fn terminal_defaults_to_powershell_and_cmd_is_a_choice() {
        let available = &["cmd.exe", "pwsh.exe", "powershell.exe"];
        let shell = windows_terminal_shell(None, "powershell", only(available));
        assert_eq!(shell.flavor, ShellFlavor::PowerShell);
        assert!(shell.program.to_string_lossy().ends_with("pwsh.exe"));
        let shell =
            windows_terminal_shell(None, "powershell", only(&["cmd.exe", "powershell.exe"]));
        assert!(shell.program.to_string_lossy().ends_with("powershell.exe"));
        let shell = windows_terminal_shell(None, "cmd", only(available));
        assert_eq!(shell.flavor, ShellFlavor::Cmd);
    }

    #[test]
    fn terminal_falls_back_to_cmd_when_powershell_is_missing() {
        let shell = windows_terminal_shell(None, "powershell", only(&["cmd.exe"]));
        assert_eq!(shell.flavor, ShellFlavor::Cmd);
    }

    #[test]
    fn posix_shell_runs_commands_with_dash_c() {
        let shell = ResolvedShell {
            program: "/bin/sh".into(),
            flavor: ShellFlavor::Posix,
        };
        assert_eq!(shell.command_args(), vec![OsString::from("-c")]);
    }
}
