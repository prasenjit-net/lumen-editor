//! Problem matchers: turn tool output lines into typed problems (path, line,
//! column, severity, message). Matching is bounded: each regex has a size limit,
//! overlong lines are ignored, and multi-line state is a single pending header
//! that expires. A line that matches nothing stays ordinary output; whether a task
//! succeeded never depends on what was matched.

use regex::{Regex, RegexBuilder};

use crate::{BuiltinMatcher, ProblemMatcherSpec, Severity};

/// Longest line considered; tools occasionally print enormous blobs.
pub const MAX_MATCH_LINE: usize = 4096;
/// Longest custom pattern accepted.
pub const MAX_PATTERN_LEN: usize = 1000;
const REGEX_SIZE_LIMIT: usize = 256 * 1024;
/// Lines a rustc header may wait for its ` --> ` location.
const PENDING_LINES: u8 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// As printed by the tool: relative to the task's working directory or absolute.
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub severity: Severity,
    pub message: String,
}

fn build(pattern: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|error| error.to_string())
}

/// Check a custom pattern without keeping it: it must compile within the size
/// limit and capture `file`, `line` and `message`.
pub fn validate_pattern(pattern: &str) -> Result<(), String> {
    if pattern.is_empty() {
        return Err("must not be empty".into());
    }
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(format!("longer than {MAX_PATTERN_LEN} characters"));
    }
    let regex = build(pattern)?;
    let names: Vec<&str> = regex.capture_names().flatten().collect();
    for required in ["file", "line", "message"] {
        if !names.contains(&required) {
            return Err(format!("needs a named group (?P<{required}>...)"));
        }
    }
    Ok(())
}

enum Kind {
    Cargo { header: Regex, location: Regex },
    Gcc { with_column: Regex, without: Regex },
    Go(Regex),
    Tsc(Regex),
    Pattern { regex: Regex, default: Severity },
}

pub struct Matcher {
    kind: Kind,
    /// A rustc `error: ...` header waiting for its location line.
    pending: Option<(Severity, String, u8)>,
}

impl Matcher {
    pub fn new(spec: &ProblemMatcherSpec) -> Result<Self, String> {
        let kind = match spec {
            ProblemMatcherSpec::Builtin(BuiltinMatcher::Cargo) => Kind::Cargo {
                header: build(r"^(error|warning)(?:\[[^\]]+\])?: (.+)$")?,
                location: build(r"^\s*--> (.+):(\d+):(\d+)\s*$")?,
            },
            ProblemMatcherSpec::Builtin(BuiltinMatcher::Gcc) => Kind::Gcc {
                with_column: build(r"^(.+?):(\d+):(\d+): (fatal error|error|warning|note): (.+)$")?,
                without: build(r"^(.+?):(\d+): (fatal error|error|warning|note): (.+)$")?,
            },
            ProblemMatcherSpec::Builtin(BuiltinMatcher::Go) => {
                Kind::Go(build(r"^(.+\.go):(\d+):(?:(\d+):)? (.+)$")?)
            }
            ProblemMatcherSpec::Builtin(BuiltinMatcher::Tsc) => Kind::Tsc(build(
                r"^(.+?)\((\d+),(\d+)\): (error|warning) (TS\d+): (.+)$",
            )?),
            ProblemMatcherSpec::Pattern { pattern, severity } => {
                validate_pattern(pattern)?;
                Kind::Pattern {
                    regex: build(pattern)?,
                    default: severity.unwrap_or(Severity::Error),
                }
            }
        };
        Ok(Self {
            kind,
            pending: None,
        })
    }

    /// Feed one output line (without its newline). At most one problem per line.
    pub fn feed_line(&mut self, line: &str) -> Option<Problem> {
        if line.len() > MAX_MATCH_LINE {
            return None;
        }
        match &self.kind {
            Kind::Cargo { header, location } => {
                if let Some(caps) = location.captures(line) {
                    let (severity, message, _) = self.pending.take()?;
                    return Some(Problem {
                        path: caps[1].to_string(),
                        line: number(&caps[2]),
                        column: number(&caps[3]),
                        severity,
                        message,
                    });
                }
                if let Some(caps) = header.captures(line) {
                    self.pending = Some((severity_of(&caps[1]), caps[2].to_string(), 0));
                    return None;
                }
                if let Some((_, _, waited)) = &mut self.pending {
                    *waited += 1;
                    if *waited > PENDING_LINES {
                        self.pending = None;
                    }
                }
                None
            }
            Kind::Gcc {
                with_column,
                without,
            } => {
                if let Some(caps) = with_column.captures(line) {
                    return Some(Problem {
                        path: caps[1].to_string(),
                        line: number(&caps[2]),
                        column: number(&caps[3]),
                        severity: severity_of(&caps[4]),
                        message: caps[5].to_string(),
                    });
                }
                let caps = without.captures(line)?;
                Some(Problem {
                    path: caps[1].to_string(),
                    line: number(&caps[2]),
                    column: 1,
                    severity: severity_of(&caps[3]),
                    message: caps[4].to_string(),
                })
            }
            Kind::Go(regex) => {
                let caps = regex.captures(line)?;
                Some(Problem {
                    path: caps[1].to_string(),
                    line: number(&caps[2]),
                    column: caps.get(3).map_or(1, |c| number(c.as_str())),
                    severity: Severity::Error,
                    message: caps[4].to_string(),
                })
            }
            Kind::Tsc(regex) => {
                let caps = regex.captures(line)?;
                Some(Problem {
                    path: caps[1].to_string(),
                    line: number(&caps[2]),
                    column: number(&caps[3]),
                    severity: severity_of(&caps[4]),
                    message: format!("{}: {}", &caps[5], &caps[6]),
                })
            }
            Kind::Pattern { regex, default } => {
                let caps = regex.captures(line)?;
                Some(Problem {
                    path: caps.name("file")?.as_str().to_string(),
                    line: number(caps.name("line")?.as_str()),
                    column: caps.name("column").map_or(1, |c| number(c.as_str())),
                    severity: caps
                        .name("severity")
                        .map_or(*default, |s| severity_of(s.as_str())),
                    message: caps.name("message")?.as_str().trim().to_string(),
                })
            }
        }
    }
}

/// Positions are 1-based and bounded; absurd values are clamped rather than trusted.
fn number(text: &str) -> u32 {
    text.parse::<u32>().unwrap_or(1).clamp(1, 10_000_000)
}

fn severity_of(text: &str) -> Severity {
    let lower = text.to_ascii_lowercase();
    if lower.contains("err") {
        Severity::Error
    } else if lower.contains("warn") {
        Severity::Warning
    } else {
        Severity::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(spec: ProblemMatcherSpec, text: &str) -> Vec<Problem> {
        let mut matcher = Matcher::new(&spec).unwrap();
        text.lines()
            .filter_map(|line| matcher.feed_line(line))
            .collect()
    }

    fn builtin(which: BuiltinMatcher) -> ProblemMatcherSpec {
        ProblemMatcherSpec::Builtin(which)
    }

    #[test]
    fn cargo_headers_pair_with_their_location_and_summary_lines_are_ignored() {
        let output = "   Compiling lumen v0.1.0\n\
            warning: unused variable: `x`\n\
             --> src/main.rs:3:9\n\
              |\n\
            3 |     let x = 1;\n\
              |         ^ help: prefix it\n\
            \n\
            error[E0425]: cannot find value `y` in this scope\n\
              --> src/lib.rs:10:5\n\
               |\n\
            warning: `lumen` (bin) generated 1 warning\n\
            error: could not compile `lumen` due to 1 previous error\n";
        let problems = run(builtin(BuiltinMatcher::Cargo), output);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(
            problems[0],
            Problem {
                path: "src/main.rs".into(),
                line: 3,
                column: 9,
                severity: Severity::Warning,
                message: "unused variable: `x`".into()
            }
        );
        assert_eq!(problems[1].severity, Severity::Error);
        assert_eq!(problems[1].message, "cannot find value `y` in this scope");
        assert_eq!(
            (
                problems[1].path.as_str(),
                problems[1].line,
                problems[1].column
            ),
            ("src/lib.rs", 10, 5)
        );
    }

    #[test]
    fn a_cargo_header_does_not_leak_onto_a_much_later_location() {
        let mut lines = String::from("error: something without a location\n");
        for n in 0..10 {
            lines.push_str(&format!("noise {n}\n"));
        }
        lines.push_str("  --> src/other.rs:1:1\n");
        assert!(run(builtin(BuiltinMatcher::Cargo), &lines).is_empty());
        // A location with no header is not a problem either.
        assert!(run(builtin(BuiltinMatcher::Cargo), "  --> src/x.rs:1:1\n").is_empty());
    }

    #[test]
    fn windows_paths_and_nested_colons_keep_their_drive_letter() {
        let problems = run(
            builtin(BuiltinMatcher::Cargo),
            "error: boom\n  --> C:\\proj\\src\\main.rs:7:3\n",
        );
        assert_eq!(problems[0].path, "C:\\proj\\src\\main.rs");
        assert_eq!((problems[0].line, problems[0].column), (7, 3));
    }

    #[test]
    fn gcc_go_and_tsc_lines_match_with_and_without_columns() {
        let gcc = run(
            builtin(BuiltinMatcher::Gcc),
            "src/a.c:12:5: error: expected ';'\nsrc/b.c:3: warning: unused variable\nsrc/c.c:1:1: note: declared here\nplain line\n",
        );
        assert_eq!(gcc.len(), 3);
        assert_eq!((gcc[0].severity, gcc[0].column), (Severity::Error, 5));
        assert_eq!((gcc[1].severity, gcc[1].column), (Severity::Warning, 1));
        assert_eq!(gcc[2].severity, Severity::Info);
        let go = run(
            builtin(BuiltinMatcher::Go),
            "./main.go:9:2: undefined: foo\n./util.go:4: missing return\n# command-line-arguments\n",
        );
        assert_eq!(go.len(), 2);
        assert_eq!((go[0].line, go[0].column, go[1].column), (9, 2, 1));
        let tsc = run(
            builtin(BuiltinMatcher::Tsc),
            "src/app.ts(5,10): error TS2304: Cannot find name 'x'.\n",
        );
        assert_eq!(tsc[0].message, "TS2304: Cannot find name 'x'.");
        assert_eq!(
            (tsc[0].path.as_str(), tsc[0].line, tsc[0].column),
            ("src/app.ts", 5, 10)
        );
    }

    #[test]
    fn custom_patterns_use_named_groups_and_a_default_severity() {
        let spec = ProblemMatcherSpec::Pattern {
            pattern: r"^(?P<file>[^:]+):(?P<line>\d+): \[(?P<severity>\w+)\] (?P<message>.+)$"
                .into(),
            severity: None,
        };
        let problems = run(
            spec.clone(),
            "a.txt:7: [warn] check this\nb.txt:8: [ERROR] nope\nnoise\n",
        );
        assert_eq!(problems.len(), 2);
        assert_eq!(
            (problems[0].severity, problems[0].column),
            (Severity::Warning, 1)
        );
        assert_eq!(problems[1].severity, Severity::Error);
        let no_severity = ProblemMatcherSpec::Pattern {
            pattern: r"^(?P<file>\S+)@(?P<line>\d+) (?P<message>.*)$".into(),
            severity: Some(Severity::Warning),
        };
        assert_eq!(
            run(no_severity, "x.rs@4 hello\n")[0].severity,
            Severity::Warning
        );
    }

    #[test]
    fn patterns_are_validated_and_bounded() {
        assert!(validate_pattern("").is_err());
        assert!(validate_pattern("(unclosed").is_err());
        assert!(
            validate_pattern(r"^(?P<file>.+):(?P<line>\d+)$")
                .unwrap_err()
                .contains("message")
        );
        assert!(
            validate_pattern(&"a".repeat(MAX_PATTERN_LEN + 1))
                .unwrap_err()
                .contains("longer")
        );
        assert!(validate_pattern(r"(?P<file>\w+) (?P<line>\d+) (?P<message>.+)").is_ok());
        // A pattern that explodes past the compile size limit is rejected, not run.
        assert!(
            validate_pattern(r"(?P<file>(\w{100}){100}) (?P<line>\d+) (?P<message>.+)").is_err()
        );
    }

    #[test]
    fn overlong_lines_and_absurd_numbers_are_handled_safely() {
        let long = format!("src/a.c:1:1: error: {}", "x".repeat(MAX_MATCH_LINE));
        assert!(run(builtin(BuiltinMatcher::Gcc), &long).is_empty());
        let big = run(
            builtin(BuiltinMatcher::Gcc),
            "f.c:99999999999999:5: error: huge\n",
        );
        assert_eq!(
            big[0].line, 1,
            "an unparsable number falls back instead of panicking"
        );
        let clamped = run(builtin(BuiltinMatcher::Gcc), "f.c:0:0: error: zero\n");
        assert_eq!((clamped[0].line, clamped[0].column), (1, 1));
    }
}
