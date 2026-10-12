//! Display-only lexical highlighting. Ranges are UTF-8 byte offsets into the
//! source; this module never changes document text or selection state.
use std::{ops::Range, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Plain,
    Rust,
    Toml,
    Markdown,
    Json,
    Yaml,
    JavaScript,
    Html,
    Css,
    Python,
    Go,
    Cpp,
    Shell,
    CMake,
    Makefile,
    Dockerfile,
    Ignore,
    Ini,
    Env,
    Sql,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        // Exact and conventional basenames take precedence over extensions:
        // CMakeLists.txt is CMake, not ordinary text; Dockerfile.dev has none.
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            match name {
                "CMakeLists.txt" => return Self::CMake,
                "Makefile" | "makefile" | "GNUmakefile" => return Self::Makefile,
                "Dockerfile" | "Containerfile" => return Self::Dockerfile,
                ".gitignore" | ".dockerignore" | ".ignore" | ".rgignore" => return Self::Ignore,
                ".editorconfig" | ".gitconfig" | ".gitmodules" | ".npmrc" => return Self::Ini,
                ".env" => return Self::Env,
                ".bashrc" | ".zshrc" | ".profile" | "gradlew" => return Self::Shell,
                "Cargo.lock" => return Self::Toml,
                "README" | "CHANGELOG" => return Self::Markdown,
                ".clang-format" => return Self::Yaml,
                _ => {}
            }
            if name.starts_with("Dockerfile.") || name.starts_with("Containerfile.") {
                return Self::Dockerfile;
            }
            if name.starts_with(".env.") {
                return Self::Env;
            }
        }
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
        {
            "rs" => Self::Rust,
            "toml" => Self::Toml,
            "md" => Self::Markdown,
            "json" => Self::Json,
            "yaml" | "yml" => Self::Yaml,
            "js" | "ts" | "tsx" | "jsx" => Self::JavaScript,
            "html" => Self::Html,
            "css" => Self::Css,
            "py" => Self::Python,
            "go" => Self::Go,
            "c" | "h" | "cpp" | "hpp" => Self::Cpp,
            "sh" => Self::Shell,
            "bash" | "zsh" => Self::Shell,
            "cmake" => Self::CMake,
            "mk" | "make" => Self::Makefile,
            "dockerfile" => Self::Dockerfile,
            "ini" | "cfg" | "properties" => Self::Ini,
            "env" => Self::Env,
            "sql" => Self::Sql,
            "xml" | "svg" => Self::Html,
            _ => Self::Plain,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "Plain Text",
            Self::Rust => "Rust",
            Self::Toml => "TOML",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::JavaScript => "JavaScript",
            Self::Html => "HTML",
            Self::Css => "CSS",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Cpp => "C/C++",
            Self::Shell => "Shell Script",
            Self::CMake => "CMake",
            Self::Makefile => "Makefile",
            Self::Dockerfile => "Dockerfile",
            Self::Ignore => "Ignore",
            Self::Ini => "INI",
            Self::Env => "Dotenv",
            Self::Sql => "SQL",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Comment,
    String,
    Number,
    Keyword,
    Type,
    Function,
    Property,
    Tag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

pub fn highlight(text: &str, language: Language) -> Vec<Span> {
    if language == Language::Plain {
        return Vec::new();
    }
    match language {
        Language::Markdown => return markdown(text),
        Language::Html => return html(text),
        Language::Makefile => return makefile(text),
        Language::Dockerfile => return dockerfile(text),
        Language::Ignore => return ignore_file(text),
        Language::Ini | Language::Env => return key_value_file(text, language),
        _ => {}
    }
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut at_line_start = true;
    while i < bytes.len() {
        let start = i;
        let c = bytes[i];
        if c == b'\n' {
            i += 1;
            at_line_start = true;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let line_comment = match language {
            Language::Python
            | Language::Shell
            | Language::Toml
            | Language::Yaml
            | Language::CMake => c == b'#',
            Language::Sql => bytes[i..].starts_with(b"--"),
            Language::Json => false,
            _ => bytes[i..].starts_with(b"//"),
        };
        if language == Language::CMake
            && c == b'#'
            && let Some(end) = bracket_argument_end(bytes, i + 1)
        {
            i = end;
            out.push(span(start, i, Kind::Comment));
            at_line_start = text[start..i].ends_with('\n');
            continue;
        }
        if line_comment {
            i = line_end(bytes, i);
            out.push(span(start, i, Kind::Comment));
            at_line_start = false;
            continue;
        }
        if matches!(
            language,
            Language::Rust
                | Language::JavaScript
                | Language::Go
                | Language::Cpp
                | Language::Css
                | Language::Sql
        ) && bytes[i..].starts_with(b"/*")
        {
            i = bytes[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(bytes.len(), |p| i + p + 4);
            at_line_start = text[start..i].ends_with('\n');
            out.push(span(start, i, Kind::Comment));
            continue;
        }
        if language == Language::Python
            && (bytes[i..].starts_with(b"\"\"\"") || bytes[i..].starts_with(b"'''"))
        {
            let quote = &bytes[i..i + 3];
            i = bytes[i + 3..]
                .windows(3)
                .position(|w| w == quote)
                .map_or(bytes.len(), |p| i + p + 6);
            at_line_start = text[start..i].ends_with('\n');
            out.push(span(start, i, Kind::String));
            continue;
        }
        if language == Language::CMake
            && c == b'['
            && let Some(end) = bracket_argument_end(bytes, i)
        {
            i = end;
            out.push(span(start, i, Kind::String));
            at_line_start = text[start..i].ends_with('\n');
            continue;
        }
        if language == Language::CMake && bytes[i..].starts_with(b"${") {
            i = bytes[i + 2..]
                .iter()
                .position(|byte| *byte == b'}')
                .map_or(bytes.len(), |p| i + p + 3);
            out.push(span(start, i, Kind::Type));
            at_line_start = false;
            continue;
        }
        if c == b'"' || c == b'\'' || (language == Language::JavaScript && c == b'`') {
            // Apostrophes in Rust lifetimes are not character literals.
            if language == Language::Rust
                && c == b'\''
                && bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic)
                && bytes.get(i + 2) != Some(&b'\'')
            {
                i += 1;
                at_line_start = false;
                continue;
            }
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == c {
                    i += 1;
                    break;
                } else if bytes[i] == b'\n' && c != b'`' {
                    break;
                } else {
                    i += 1;
                }
            }
            let kind = if language == Language::Json
                && bytes[i..].iter().find(|byte| !matches!(byte, b' ' | b'\t')) == Some(&b':')
            {
                Kind::Property
            } else {
                Kind::String
            };
            out.push(span(start, i, kind));
            at_line_start = false;
            continue;
        }
        if c.is_ascii_digit() && (i == 0 || !word_byte(bytes[i - 1])) {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'.'))
            {
                i += 1;
            }
            out.push(span(start, i, Kind::Number));
            at_line_start = false;
            continue;
        }
        if word_byte(c) && !c.is_ascii_digit() {
            i += 1;
            while i < bytes.len() && word_byte(bytes[i]) {
                i += 1;
            }
            let word = &text[start..i];
            let next = bytes[i..]
                .iter()
                .position(|b| !matches!(b, b' ' | b'\t'))
                .map(|p| bytes[i + p]);
            let kind = if keyword(language, word) {
                Some(Kind::Keyword)
            } else if (matches!(
                language,
                Language::Toml | Language::Yaml | Language::Json | Language::Css
            ) && next == Some(b':'))
                || (language == Language::Toml && next == Some(b'='))
            {
                Some(Kind::Property)
            } else if next == Some(b'(')
                && !matches!(
                    language,
                    Language::Json | Language::Toml | Language::Yaml | Language::Sql
                )
            {
                Some(Kind::Function)
            } else if c.is_ascii_uppercase()
                && !matches!(
                    language,
                    Language::Json
                        | Language::Toml
                        | Language::Yaml
                        | Language::Css
                        | Language::Sql
                )
            {
                Some(Kind::Type)
            } else {
                None
            };
            if let Some(kind) = kind {
                out.push(span(start, i, kind));
            }
            at_line_start = false;
            continue;
        }
        if at_line_start && language == Language::Toml && c == b'[' {
            i = line_end(bytes, i);
            out.push(span(start, i, Kind::Type));
        } else if at_line_start && language == Language::Shell && c == b'$' {
            i += 1;
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
        }
        at_line_start = false;
    }
    out
}

fn span(start: usize, end: usize, kind: Kind) -> Span {
    Span {
        range: start..end,
        kind,
    }
}
fn line_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(bytes.len(), |n| start + n)
}
fn word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// CMake bracket arguments/comments: `[=[content]=]` with any count of `=`.
fn bracket_argument_end(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut marker_end = start + 1;
    while bytes.get(marker_end) == Some(&b'=') {
        marker_end += 1;
    }
    if bytes.get(marker_end) != Some(&b'[') {
        return None;
    }
    let equals = marker_end - start - 1;
    let mut i = marker_end + 1;
    while i < bytes.len() {
        if bytes[i] == b']'
            && bytes.get(i + equals + 1) == Some(&b']')
            && bytes[i + 1..i + equals + 1]
                .iter()
                .all(|byte| *byte == b'=')
        {
            return Some(i + equals + 2);
        }
        i += 1;
    }
    Some(bytes.len())
}

fn keyword(language: Language, word: &str) -> bool {
    let words = match language {
        Language::Rust => {
            "fn let mut pub use mod impl trait struct enum match if else while for in loop return async await move const static self Self super crate where type as ref unsafe dyn true false Some None Ok Err"
        }
        Language::JavaScript => {
            "const let var function class extends import export default from if else for while return async await new this null undefined true false interface type enum public private"
        }
        Language::Python => {
            "def class if elif else for while in return import from as with try except finally raise async await lambda yield pass None True False self"
        }
        Language::Go => {
            "func package import type struct interface map chan go defer select case default if else for range return var const true false nil"
        }
        Language::Cpp => {
            "int char float double void bool auto const static class struct enum namespace template typename public private protected virtual override if else for while return new delete true false nullptr include using"
        }
        Language::Json => "true false null",
        Language::Toml | Language::Yaml => "true false null",
        Language::Shell => {
            "if then else elif fi for do done while case esac function in export local return"
        }
        Language::Css => {
            "important inherit initial unset auto none block flex grid absolute relative fixed"
        }
        Language::CMake => {
            "AND OR NOT TRUE FALSE ON OFF CACHE PARENT_SCOPE REQUIRED PRIVATE PUBLIC INTERFACE"
        }
        Language::Sql => {
            "SELECT FROM WHERE JOIN LEFT RIGHT INNER OUTER ON AS INSERT INTO VALUES UPDATE SET DELETE CREATE TABLE ALTER DROP INDEX PRIMARY KEY FOREIGN NULL NOT AND OR ORDER BY GROUP HAVING LIMIT OFFSET DISTINCT UNION CASE WHEN THEN ELSE END TRUE FALSE"
        }
        _ => "",
    };
    words.split_ascii_whitespace().any(|candidate| {
        if language == Language::Sql {
            candidate.eq_ignore_ascii_case(word)
        } else {
            candidate == word
        }
    })
}

fn markdown(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let start = offset + line.len() - trimmed.len();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            out.push(span(
                start,
                offset + line.trim_end_matches('\n').len(),
                Kind::Keyword,
            ));
        } else if fenced {
            out.push(span(offset, offset + line.len(), Kind::String));
        } else if trimmed.starts_with('#') {
            out.push(span(
                start,
                offset + line.trim_end_matches('\n').len(),
                Kind::Type,
            ));
        } else if trimmed.starts_with('>') {
            out.push(span(
                start,
                offset + line.trim_end_matches('\n').len(),
                Kind::Comment,
            ));
        }
        offset += line.len();
    }
    out
}

fn html(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"<!--") {
            let end = bytes[i + 4..]
                .windows(3)
                .position(|w| w == b"-->")
                .map_or(bytes.len(), |p| i + p + 7);
            out.push(span(i, end, Kind::Comment));
            i = end;
        } else if bytes[i] == b'<' {
            let end = line_end(bytes, i).min(
                bytes[i..]
                    .iter()
                    .position(|b| *b == b'>')
                    .map_or(bytes.len(), |p| i + p + 1),
            );
            if end > i + 1 {
                out.push(span(i, end, Kind::Tag));
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

fn makefile(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let trimmed = body.trim_start_matches([' ', '\t']);
        let start = offset + body.len() - trimmed.len();
        if trimmed.starts_with('#') {
            out.push(span(start, offset + body.len(), Kind::Comment));
        } else if !line.starts_with('\t') && !trimmed.is_empty() {
            let word_end = trimmed
                .find(|c: char| c.is_whitespace())
                .unwrap_or(trimmed.len());
            let first = &trimmed[..word_end];
            if matches!(
                first,
                "include"
                    | "-include"
                    | "sinclude"
                    | "ifeq"
                    | "ifneq"
                    | "ifdef"
                    | "ifndef"
                    | "else"
                    | "endif"
                    | "define"
                    | "endef"
                    | "export"
                    | "override"
            ) {
                out.push(span(start, start + word_end, Kind::Keyword));
            } else if let Some(equals) = trimmed.find('=').filter(|equals| {
                trimmed[..*equals]
                    .find(':')
                    .is_none_or(|colon| trimmed[colon + 1..*equals].trim().is_empty())
            }) {
                let end = trimmed[..equals]
                    .trim_end_matches([' ', '\t', ':', '?', '+', '!'])
                    .len();
                if end > 0 {
                    out.push(span(start, start + end, Kind::Property));
                }
            } else if let Some(colon) = trimmed.find(':') {
                out.push(span(start, start + colon, Kind::Type));
            }
        }
        offset += line.len();
    }
    out
}

fn dockerfile(text: &str) -> Vec<Span> {
    const INSTRUCTIONS: &str = "FROM RUN CMD LABEL MAINTAINER EXPOSE ENV ADD COPY ENTRYPOINT VOLUME USER WORKDIR ARG ONBUILD STOPSIGNAL HEALTHCHECK SHELL";
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let trimmed = body.trim_start_matches([' ', '\t']);
        let start = offset + body.len() - trimmed.len();
        if trimmed.starts_with('#') {
            out.push(span(start, offset + body.len(), Kind::Comment));
        } else if !trimmed.is_empty() {
            let end = trimmed
                .find(|c: char| c.is_whitespace())
                .unwrap_or(trimmed.len());
            if INSTRUCTIONS
                .split_ascii_whitespace()
                .any(|word| word.eq_ignore_ascii_case(&trimmed[..end]))
            {
                out.push(span(start, start + end, Kind::Keyword));
            }
        }
        offset += line.len();
    }
    out
}

fn ignore_file(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let trimmed = body.trim_start_matches([' ', '\t']);
        let start = offset + body.len() - trimmed.len();
        if trimmed.starts_with('#') {
            out.push(span(start, offset + body.len(), Kind::Comment));
        } else if !trimmed.is_empty() {
            let mut pattern_start = start;
            if trimmed.starts_with('!') {
                out.push(span(start, start + 1, Kind::Keyword));
                pattern_start += 1;
            }
            if pattern_start < offset + body.len() {
                out.push(span(pattern_start, offset + body.len(), Kind::String));
            }
        }
        offset += line.len();
    }
    out
}

fn key_value_file(text: &str, language: Language) -> Vec<Span> {
    let mut out = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let trimmed = body.trim_start_matches([' ', '\t']);
        let start = offset + body.len() - trimmed.len();
        if trimmed.starts_with('#') || (language == Language::Ini && trimmed.starts_with(';')) {
            out.push(span(start, offset + body.len(), Kind::Comment));
        } else if language == Language::Ini && trimmed.starts_with('[') {
            if let Some(end) = trimmed.find(']') {
                out.push(span(start, start + end + 1, Kind::Type));
            }
        } else if let Some(sep) =
            trimmed.find(|c| c == '=' || (language == Language::Ini && c == ':'))
        {
            let key_end = trimmed[..sep].trim_end().len();
            if key_end > 0 {
                out.push(span(start, start + key_end, Kind::Property));
                let value = trimmed[sep + 1..].trim_start();
                let value_start = start + sep + 1 + trimmed[sep + 1..].len() - value.len();
                if !value.is_empty() {
                    let kind = if value.starts_with(|c: char| c.is_ascii_digit()) {
                        Kind::Number
                    } else if matches!(value, "true" | "false" | "on" | "off") {
                        Kind::Keyword
                    } else {
                        Kind::String
                    };
                    out.push(span(value_start, offset + body.len(), kind));
                }
            }
        }
        offset += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_language_is_typed_and_unknown_files_remain_plain() {
        assert_eq!(
            Language::from_path(Path::new("src/main.rs")),
            Language::Rust
        );
        assert_eq!(
            Language::from_path(Path::new("src/view.tsx")),
            Language::JavaScript
        );
        assert_eq!(
            Language::from_path(Path::new("note.unknown")),
            Language::Plain
        );
        assert_eq!(Language::Plain.as_str(), "Plain Text");
    }
    #[test]
    fn full_filenames_precede_extensions_and_detect_build_config_files() {
        let examples = [
            ("src/CMakeLists.txt", Language::CMake, "CMake"),
            ("cmake/Modules/FindFoo.cmake", Language::CMake, "CMake"),
            ("Makefile", Language::Makefile, "Makefile"),
            ("GNUmakefile", Language::Makefile, "Makefile"),
            ("Dockerfile.dev", Language::Dockerfile, "Dockerfile"),
            ("Containerfile", Language::Dockerfile, "Dockerfile"),
            (".gitignore", Language::Ignore, "Ignore"),
            (".editorconfig", Language::Ini, "INI"),
            (".env.local", Language::Env, "Dotenv"),
            ("Cargo.lock", Language::Toml, "TOML"),
            ("README", Language::Markdown, "Markdown"),
            ("query.sql", Language::Sql, "SQL"),
            ("diagram.svg", Language::Html, "HTML"),
        ];
        for (path, language, label) in examples {
            assert_eq!(Language::from_path(Path::new(path)), language, "{path}");
            assert_eq!(language.as_str(), label);
        }
        assert_eq!(
            Language::from_path(Path::new("CMakeLists.txt.bak")),
            Language::Plain
        );
        assert_eq!(
            Language::from_path(Path::new("myCMakeLists.txt")),
            Language::Plain
        );
    }

    #[test]
    fn cmake_make_docker_and_config_spans_are_distinct() {
        let cmake =
            "# heading\nproject(Lumen)\nset(NAME ${APP})\n#[=[note\nmore]=]\nmessage([=[hi]=])";
        let spans = highlight(cmake, Language::CMake);
        assert!(
            spans
                .iter()
                .any(|s| &cmake[s.range.clone()] == "project" && s.kind == Kind::Function)
        );
        assert!(
            spans
                .iter()
                .any(|s| &cmake[s.range.clone()] == "${APP}" && s.kind == Kind::Type)
        );
        assert!(
            spans
                .iter()
                .any(|s| &cmake[s.range.clone()] == "#[=[note\nmore]=]" && s.kind == Kind::Comment)
        );
        assert!(
            spans
                .iter()
                .any(|s| &cmake[s.range.clone()] == "[=[hi]=]" && s.kind == Kind::String)
        );
        let make = "# note\nAPP := lumen\nall: build\n\techo hi\n";
        let spans = highlight(make, Language::Makefile);
        assert!(
            spans
                .iter()
                .any(|s| &make[s.range.clone()] == "APP" && s.kind == Kind::Property)
        );
        assert!(
            spans
                .iter()
                .any(|s| &make[s.range.clone()] == "all" && s.kind == Kind::Type)
        );
        assert!(!spans.iter().any(|s| &make[s.range.clone()] == "echo"));
        let docker = "# syntax=docker/dockerfile:1\nFROM rust:latest\nRUN echo hi # literal\n";
        let spans = highlight(docker, Language::Dockerfile);
        assert!(
            spans
                .iter()
                .any(|s| &docker[s.range.clone()] == "FROM" && s.kind == Kind::Keyword)
        );
        assert!(
            spans
                .iter()
                .any(|s| &docker[s.range.clone()] == "RUN" && s.kind == Kind::Keyword)
        );
        assert!(
            !spans
                .iter()
                .any(|s| &docker[s.range.clone()] == "# literal" && s.kind == Kind::Comment)
        );
        let ignore = "# output\ntarget/\n!important.log\n";
        let spans = highlight(ignore, Language::Ignore);
        assert!(
            spans
                .iter()
                .any(|s| &ignore[s.range.clone()] == "target/" && s.kind == Kind::String)
        );
        assert!(
            spans
                .iter()
                .any(|s| &ignore[s.range.clone()] == "!" && s.kind == Kind::Keyword)
        );
        let env = "# note\nPORT=8080\nNAME=\"Lumen\"\n";
        let spans = highlight(env, Language::Env);
        assert!(
            spans
                .iter()
                .any(|s| &env[s.range.clone()] == "PORT" && s.kind == Kind::Property)
        );
        assert!(
            spans
                .iter()
                .any(|s| &env[s.range.clone()] == "8080" && s.kind == Kind::Number)
        );
        let sql = "SELECT name FROM users -- note\nWHERE id = 3";
        let spans = highlight(sql, Language::Sql);
        assert!(
            spans
                .iter()
                .any(|s| &sql[s.range.clone()] == "SELECT" && s.kind == Kind::Keyword)
        );
        assert!(
            spans
                .iter()
                .any(|s| &sql[s.range.clone()] == "-- note" && s.kind == Kind::Comment)
        );
    }
    #[test]
    fn lexical_ranges_are_source_byte_boundaries_and_do_not_mutate() {
        let text = "// 🌍\r\nfn café() { let n = 42; }";
        let before = text.to_owned();
        let spans = highlight(text, Language::Rust);
        assert_eq!(spans[0], span(0, 8, Kind::Comment));
        assert!(
            spans
                .iter()
                .any(|s| &text[s.range.clone()] == "fn" && s.kind == Kind::Keyword)
        );
        assert!(
            spans
                .iter()
                .any(|s| &text[s.range.clone()] == "42" && s.kind == Kind::Number)
        );
        assert!(spans.iter().all(|s| text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end)));
        assert_eq!(text, before);
    }
    #[test]
    fn strings_and_comments_win_over_keywords() {
        let text = "let x = \"fn 42\"; /* let */";
        let spans = highlight(text, Language::Rust);
        assert!(
            spans
                .iter()
                .any(|s| &text[s.range.clone()] == "\"fn 42\"" && s.kind == Kind::String)
        );
        assert!(
            spans
                .iter()
                .any(|s| &text[s.range.clone()] == "/* let */" && s.kind == Kind::Comment)
        );
        assert_eq!(highlight("let x", Language::Plain), Vec::<Span>::new());
    }

    #[test]
    fn recognized_formats_have_distinct_token_categories() {
        let json = "{\"name\": \"Lumen\", \"enabled\": true, \"size\": 12}";
        let spans = highlight(json, Language::Json);
        assert!(
            spans
                .iter()
                .any(|s| &json[s.range.clone()] == "\"name\"" && s.kind == Kind::Property)
        );
        assert!(
            spans
                .iter()
                .any(|s| &json[s.range.clone()] == "\"Lumen\"" && s.kind == Kind::String)
        );
        assert!(
            spans
                .iter()
                .any(|s| &json[s.range.clone()] == "true" && s.kind == Kind::Keyword)
        );
        assert!(
            spans
                .iter()
                .any(|s| &json[s.range.clone()] == "12" && s.kind == Kind::Number)
        );
        let markdown = "# Title\n```rust\nfn main() {}\n```\n";
        let spans = highlight(markdown, Language::Markdown);
        assert_eq!(spans[0].kind, Kind::Type);
        assert!(spans.iter().any(|s| s.kind == Kind::String));
        let html = "<!-- note --><div class=\"x\">";
        let spans = highlight(html, Language::Html);
        assert_eq!(spans[0].kind, Kind::Comment);
        assert_eq!(spans[1].kind, Kind::Tag);
    }
}
