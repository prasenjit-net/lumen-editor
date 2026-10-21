//! Display-only lexical highlighting. Ranges are UTF-8 byte offsets into the
//! source; this module never changes document text or selection state.
use std::{collections::HashSet, ops::Range, path::Path, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Display label from the embedded format definition. Consumers that bridge
    /// to optional language contributions should use this stable label rather
    /// than duplicate per-language match arms.
    #[must_use]
    pub fn label(self) -> &'static str {
        if self == Self::Plain {
            "Plain Text"
        } else {
            definitions()
                .iter()
                .find(|definition| definition.language == self)
                .map_or("Plain Text", |definition| definition.label)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lexer {
    Code,
    Markdown,
    Html,
    Makefile,
    Dockerfile,
    Ignore,
    KeyValue,
}

struct Definition {
    language: Language,
    label: &'static str,
    filenames: Vec<&'static str>,
    prefixes: Vec<&'static str>,
    extensions: Vec<&'static str>,
    keywords: Vec<&'static str>,
    lexer: Lexer,
    line_comment: &'static str,
    block_comment: bool,
    case_insensitive_keywords: bool,
}

// Each definition is embedded at compile time. Adding a format with an
// existing lexer requires one data file and one registration here; a genuinely
// new lexical grammar still needs a focused scanner.
const SOURCES: &[(Language, &str)] = &[
    (Language::Rust, include_str!("../syntaxes/rust.syntax")),
    (Language::Toml, include_str!("../syntaxes/toml.syntax")),
    (
        Language::Markdown,
        include_str!("../syntaxes/markdown.syntax"),
    ),
    (Language::Json, include_str!("../syntaxes/json.syntax")),
    (Language::Yaml, include_str!("../syntaxes/yaml.syntax")),
    (
        Language::JavaScript,
        include_str!("../syntaxes/javascript.syntax"),
    ),
    (Language::Html, include_str!("../syntaxes/html.syntax")),
    (Language::Css, include_str!("../syntaxes/css.syntax")),
    (Language::Python, include_str!("../syntaxes/python.syntax")),
    (Language::Go, include_str!("../syntaxes/go.syntax")),
    (Language::Cpp, include_str!("../syntaxes/cpp.syntax")),
    (Language::Shell, include_str!("../syntaxes/shell.syntax")),
    (Language::CMake, include_str!("../syntaxes/cmake.syntax")),
    (
        Language::Makefile,
        include_str!("../syntaxes/makefile.syntax"),
    ),
    (
        Language::Dockerfile,
        include_str!("../syntaxes/dockerfile.syntax"),
    ),
    (Language::Ignore, include_str!("../syntaxes/ignore.syntax")),
    (Language::Ini, include_str!("../syntaxes/ini.syntax")),
    (Language::Env, include_str!("../syntaxes/env.syntax")),
    (Language::Sql, include_str!("../syntaxes/sql.syntax")),
];

static DEFINITIONS: OnceLock<Vec<Definition>> = OnceLock::new();

fn definitions() -> &'static [Definition] {
    DEFINITIONS.get_or_init(|| {
        SOURCES
            .iter()
            .map(|(language, source)| parse(*language, source))
            .collect()
    })
}

fn parse(language: Language, source: &'static str) -> Definition {
    let mut definition = Definition {
        language,
        label: "",
        filenames: Vec::new(),
        prefixes: Vec::new(),
        extensions: Vec::new(),
        keywords: Vec::new(),
        lexer: Lexer::Code,
        line_comment: "",
        block_comment: false,
        case_insensitive_keywords: false,
    };
    let mut seen = HashSet::new();
    for line in source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let (key, value) = line
            .split_once('=')
            .expect("syntax definition requires key=value");
        let (key, value) = (key.trim(), value.trim());
        assert!(seen.insert(key), "duplicate syntax definition field {key}");
        match key {
            "label" => definition.label = value,
            "filenames" => definition.filenames = value.split_ascii_whitespace().collect(),
            "prefixes" => definition.prefixes = value.split_ascii_whitespace().collect(),
            "extensions" => definition.extensions = value.split_ascii_whitespace().collect(),
            "keywords" => definition.keywords = value.split_ascii_whitespace().collect(),
            "lexer" => {
                definition.lexer = match value {
                    "code" => Lexer::Code,
                    "markdown" => Lexer::Markdown,
                    "html" => Lexer::Html,
                    "makefile" => Lexer::Makefile,
                    "dockerfile" => Lexer::Dockerfile,
                    "ignore" => Lexer::Ignore,
                    "key_value" => Lexer::KeyValue,
                    _ => panic!("unknown syntax lexer {value}"),
                }
            }
            "line_comment" => definition.line_comment = value,
            "block_comment" => {
                definition.block_comment = value.parse().expect("invalid block_comment")
            }
            "case_insensitive_keywords" => {
                definition.case_insensitive_keywords =
                    value.parse().expect("invalid case_insensitive_keywords")
            }
            _ => panic!("unknown syntax definition field {key}"),
        }
    }
    assert!(
        !definition.label.is_empty(),
        "syntax definition requires label"
    );
    assert!(seen.contains("lexer"), "syntax definition requires lexer");
    definition
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if let Some(definition) = definitions()
            .iter()
            .find(|definition| definition.filenames.contains(&name))
        {
            return definition.language;
        }
        if let Some(definition) = definitions().iter().find(|definition| {
            definition
                .prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
        }) {
            return definition.language;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        definitions()
            .iter()
            .find(|definition| definition.extensions.contains(&extension))
            .map_or(Self::Plain, |definition| definition.language)
    }

    pub fn as_str(self) -> &'static str {
        if self == Self::Plain {
            "Plain Text"
        } else {
            self.definition().label
        }
    }

    fn definition(self) -> &'static Definition {
        definitions()
            .iter()
            .find(|definition| definition.language == self)
            .expect("every syntax language has one embedded definition")
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
    let definition = language.definition();
    match definition.lexer {
        Lexer::Markdown => return markdown(text),
        Lexer::Html => return html(text),
        Lexer::Makefile => return makefile(text),
        Lexer::Dockerfile => return dockerfile(text),
        Lexer::Ignore => return ignore_file(text),
        Lexer::KeyValue => return key_value_file(text, language),
        Lexer::Code => {}
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
        let line_comment = !definition.line_comment.is_empty()
            && bytes[i..].starts_with(definition.line_comment.as_bytes());
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
        if definition.block_comment && bytes[i..].starts_with(b"/*") {
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
            let kind = if keyword(definition, word) {
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

fn keyword(definition: &Definition, word: &str) -> bool {
    definition.keywords.iter().any(|candidate| {
        if definition.case_insensitive_keywords {
            candidate.eq_ignore_ascii_case(word)
        } else {
            *candidate == word
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
    let definition = Language::Makefile.definition();
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
            if keyword(definition, first) {
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
    let definition = Language::Dockerfile.definition();
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
            if keyword(definition, &trimmed[..end]) {
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
    fn embedded_definitions_are_complete_and_unambiguous() {
        assert_eq!(definitions().len(), 19);
        let mut languages = HashSet::new();
        let mut filenames = HashSet::new();
        let mut extensions = HashSet::new();
        let mut prefixes = HashSet::new();
        for definition in definitions() {
            assert!(languages.insert(definition.language));
            assert!(!definition.label.is_empty());
            for filename in &definition.filenames {
                assert!(filenames.insert(filename), "duplicate filename {filename}");
            }
            for extension in &definition.extensions {
                assert!(
                    extensions.insert(extension),
                    "duplicate extension {extension}"
                );
            }
            for prefix in &definition.prefixes {
                assert!(prefixes.insert(prefix), "duplicate prefix {prefix}");
            }
        }
        assert!(!languages.contains(&Language::Plain));
    }

    #[test]
    fn embedded_keywords_preserve_representative_language_spans() {
        for (language, source, token) in [
            (Language::Rust, "fn main() {}", "fn"),
            (Language::Toml, "enabled = true", "true"),
            (Language::Json, "{\"enabled\": true}", "true"),
            (Language::Yaml, "enabled: true", "true"),
            (Language::JavaScript, "const x = 1", "const"),
            (Language::Css, "display: flex", "flex"),
            (Language::Python, "def main(): pass", "def"),
            (Language::Go, "func main() {}", "func"),
            (Language::Cpp, "class Main {};", "class"),
            (Language::Shell, "if true; then", "if"),
            (Language::CMake, "set(X ON)", "ON"),
            (Language::Sql, "select * FROM tab", "select"),
        ] {
            assert!(
                highlight(source, language).iter().any(|span| {
                    &source[span.range.clone()] == token && span.kind == Kind::Keyword
                }),
                "missing {token} in {language:?}"
            );
        }
    }

    #[test]
    fn definition_parser_rejects_unknown_and_duplicate_fields() {
        assert!(
            std::panic::catch_unwind(|| parse(Language::Rust, "label=Rust\nlexer=code\nunknown=x"))
                .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| parse(
                Language::Rust,
                "label=Rust\nlabel=Again\nlexer=code"
            ))
            .is_err()
        );
    }
    #[test]
    fn javascript_and_typescript_extensions_resolve_to_javascript() {
        for ext in ["js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx"] {
            let path = format!("module.{ext}");
            assert_eq!(
                Language::from_path(Path::new(&path)),
                Language::JavaScript,
                "{path}"
            );
        }
    }

    #[test]
    fn esm_and_commonjs_fixtures_highlight_keywords_strings_and_comments() {
        let esm = "import fs from 'node:fs'; // load\nexport default await fs.read(import.meta.url);\nconst t = `a${1}`;";
        let cjs = "const x = require(\"x\");\nmodule.exports = { x }; /* done */";
        for (source, keyword, string, comment) in [
            (esm, "import", "'node:fs'", "// load"),
            (cjs, "const", "\"x\"", "/* done */"),
        ] {
            let spans = highlight(source, Language::JavaScript);
            for (token, kind) in [
                (keyword, Kind::Keyword),
                (string, Kind::String),
                (comment, Kind::Comment),
            ] {
                assert!(
                    spans
                        .iter()
                        .any(|s| &source[s.range.clone()] == token && s.kind == kind),
                    "missing {token} as {kind:?}"
                );
            }
        }
        let spans = highlight(esm, Language::JavaScript);
        for token in ["export", "default", "await"] {
            assert!(
                spans
                    .iter()
                    .any(|s| &esm[s.range.clone()] == token && s.kind == Kind::Keyword)
            );
        }
    }

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
    fn language_labels_are_stable_for_optional_contribution_lookup() {
        assert_eq!(Language::Rust.label(), "Rust");
        assert_eq!(Language::Go.label(), "Go");
        assert_eq!(Language::CMake.label(), "CMake");
        assert_eq!(Language::Plain.label(), "Plain Text");
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
