//! Command-line arguments: what to open when Lumen is started with paths.
//!
//! Pure parsing, so every rule is testable without a window or a file system.
//! Existence is injected because `file:10` must be told apart from a file that is
//! really named `file:10`.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// A file or folder to open, with an optional 1-based position in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) path: PathBuf,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invocation {
    Help,
    Version,
    /// Open these (possibly none) and start the editor.
    Open(Vec<Target>),
    /// The arguments were unusable; nothing should start.
    Invalid(String),
}

pub(crate) const USAGE: &str = "\
Usage: lumen [OPTIONS] [PATH[:LINE[:COLUMN]]]...

Opens the given files and folders. A folder becomes the workspace (only one can
be open); files open as tabs and the last one is shown. LINE and COLUMN are
1-based and place the caret.

Options:
  -h, --help       Show this help and exit
  -V, --version    Show the version and exit
  --               Treat everything after it as a path";

/// Interpret `args` (without the program name). Relative paths are resolved
/// against `cwd`. `exists` reports whether a path is on disk.
pub(crate) fn parse(
    args: impl IntoIterator<Item = OsString>,
    cwd: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Invocation {
    let mut targets = Vec::new();
    let mut options_done = false;
    let mut version = false;
    for argument in args {
        let text = argument.to_str();
        if !options_done && let Some(text) = text {
            match text {
                "--" => {
                    options_done = true;
                    continue;
                }
                "-h" | "--help" => return Invocation::Help,
                "-V" | "--version" => {
                    version = true;
                    continue;
                }
                // macOS used to pass a process serial number when launching an app
                // from the Finder; it is not for us.
                _ if text.starts_with("-psn_") => continue,
                _ if text.starts_with('-') && text != "-" => {
                    return Invocation::Invalid(format!("unknown option `{text}`"));
                }
                _ => {}
            }
        }
        targets.push(target(&argument, cwd, &exists));
    }
    if version {
        return Invocation::Version;
    }
    Invocation::Open(targets)
}

fn target(argument: &OsString, cwd: &Path, exists: &impl Fn(&Path) -> bool) -> Target {
    let as_path = |text: &OsString| {
        let path = PathBuf::from(text);
        if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        }
    };
    let whole = as_path(argument);
    // A path that exists exactly as typed always wins over a `:line` reading.
    let Some(text) = argument.to_str().filter(|_| !exists(&whole)) else {
        return Target {
            path: whole,
            line: None,
            column: None,
        };
    };
    let (base, line, column) = split_position(text);
    Target {
        path: as_path(&OsString::from(base)),
        line,
        column,
    }
}

/// Split a trailing `:line` or `:line:column` off `text`. The remaining path
/// must not be empty, so `:5` stays a (strange) file name.
fn split_position(text: &str) -> (&str, Option<usize>, Option<usize>) {
    let number = |part: &str| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<usize>().ok())
            .flatten()
            .filter(|value| *value > 0)
    };
    let Some((rest, last)) = text.rsplit_once(':') else {
        return (text, None, None);
    };
    let Some(last) = number(last) else {
        return (text, None, None);
    };
    if let Some((base, middle)) = rest.rsplit_once(':')
        && !base.is_empty()
        && let Some(line) = number(middle)
    {
        return (base, Some(line), Some(last));
    }
    if rest.is_empty() {
        return (text, None, None);
    }
    (rest, Some(last), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], existing: &[&str]) -> Invocation {
        let cwd = Path::new("/work");
        let existing: Vec<PathBuf> = existing.iter().map(|p| cwd.join(p)).collect();
        parse(args.iter().map(OsString::from), cwd, |path| {
            existing.iter().any(|known| known == path)
        })
    }

    fn open(args: &[&str]) -> Vec<Target> {
        match run(args, &[]) {
            Invocation::Open(targets) => targets,
            other => panic!("expected targets, got {other:?}"),
        }
    }

    fn target(path: &str, line: Option<usize>, column: Option<usize>) -> Target {
        Target {
            path: Path::new("/work").join(path),
            line,
            column,
        }
    }

    #[test]
    fn no_arguments_just_start_the_editor() {
        assert_eq!(run(&[], &[]), Invocation::Open(vec![]));
    }

    #[test]
    fn paths_resolve_against_the_working_directory_and_keep_their_order() {
        let targets = open(&["a.rs", "sub/b.rs", "/abs/c.rs"]);
        assert_eq!(targets[0], target("a.rs", None, None));
        assert_eq!(targets[1], target("sub/b.rs", None, None));
        assert_eq!(targets[2].path, PathBuf::from("/abs/c.rs"));
    }

    #[test]
    fn a_trailing_line_and_column_are_split_off() {
        assert_eq!(open(&["a.rs:12"]), [target("a.rs", Some(12), None)]);
        assert_eq!(open(&["a.rs:12:5"]), [target("a.rs", Some(12), Some(5))]);
        assert_eq!(
            open(&["dir/a.rs:3:9"]),
            [target("dir/a.rs", Some(3), Some(9))]
        );
    }

    #[test]
    fn a_file_that_really_has_a_colon_name_is_not_misread() {
        let result = run(&["notes:12"], &["notes:12"]);
        assert_eq!(
            result,
            Invocation::Open(vec![target("notes:12", None, None)]),
            "an existing path wins over the :line reading"
        );
    }

    #[test]
    fn zero_non_numeric_and_empty_positions_are_not_positions() {
        assert_eq!(open(&["a.rs:0"]), [target("a.rs:0", None, None)]);
        assert_eq!(open(&["a.rs:x"]), [target("a.rs:x", None, None)]);
        assert_eq!(open(&["a.rs:"]), [target("a.rs:", None, None)]);
        assert_eq!(open(&[":5"]), [target(":5", None, None)]);
        // A zero column is not a column, so the whole `:2:0` stays in the name.
        assert_eq!(open(&["a.rs:2:0"]), [target("a.rs:2:0", None, None)]);
    }

    #[test]
    fn windows_drive_letters_do_not_look_like_positions() {
        let targets = open(&[r"C:\proj\a.rs:7"]);
        assert_eq!(targets[0].line, Some(7));
        assert!(targets[0].path.to_string_lossy().ends_with("a.rs"));
        let plain = open(&[r"C:\proj\a.rs"]);
        assert_eq!(plain[0].line, None);
    }

    #[test]
    fn help_and_version_win_and_unknown_options_are_rejected() {
        assert_eq!(run(&["a.rs", "--help"], &[]), Invocation::Help);
        assert_eq!(run(&["-h"], &[]), Invocation::Help);
        assert_eq!(run(&["--version", "a.rs"], &[]), Invocation::Version);
        assert_eq!(run(&["-V"], &[]), Invocation::Version);
        assert_eq!(
            run(&["--bogus"], &[]),
            Invocation::Invalid("unknown option `--bogus`".into())
        );
        // Help beats a later bad option; a bad option before help is still an error.
        assert_eq!(run(&["--help", "--bogus"], &[]), Invocation::Help);
    }

    #[test]
    fn double_dash_ends_options_so_dashed_names_can_be_opened() {
        assert_eq!(
            open(&["--", "--weird.txt", "-h"]),
            [target("--weird.txt", None, None), target("-h", None, None)]
        );
    }

    #[test]
    fn a_lone_dash_and_the_macos_process_serial_number_are_handled() {
        assert_eq!(open(&["-"]), [target("-", None, None)]);
        assert_eq!(
            open(&["-psn_0_12345", "a.rs"]),
            [target("a.rs", None, None)]
        );
    }

    #[test]
    fn non_utf8_names_are_kept_whole() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let name = OsString::from_vec(vec![b'a', 0xff, b':', b'3']);
            let result = parse([name.clone()], Path::new("/work"), |_| false);
            let Invocation::Open(targets) = result else {
                panic!("expected targets")
            };
            assert_eq!(targets[0].line, None, "no position parsing without UTF-8");
            assert_eq!(targets[0].path, Path::new("/work").join(name));
        }
    }
}
