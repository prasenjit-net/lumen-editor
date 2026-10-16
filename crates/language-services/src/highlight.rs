//! Parser-backed baseline syntax tokens for bundled language contributions.
use crate::{
    DocumentVersion, LanguageId, LanguageRegistry, ParserKind, TokenClass, TokenSnapshot,
    TokenSnapshotError, TokenSpan,
};
use std::{collections::BinaryHeap, ops::Range, sync::OnceLock};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HighlightError {
    UnsupportedLanguage(LanguageId),
    Grammar(String),
    Query(String),
    ParseFailed,
    InvalidTokens(TokenSnapshotError),
}

impl From<TokenSnapshotError> for HighlightError {
    fn from(error: TokenSnapshotError) -> Self {
        Self::InvalidTokens(error)
    }
}

struct Grammar {
    language: Language,
    highlights: &'static str,
    /// Compiling a query costs milliseconds; highlighting runs on every layout.
    query: &'static OnceLock<Result<Query, String>>,
}

impl Grammar {
    fn query(&self) -> Result<&'static Query, HighlightError> {
        self.query
            .get_or_init(|| {
                Query::new(&self.language, self.highlights).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| HighlightError::Query(error.clone()))
    }
}

fn grammar_for(language: LanguageId) -> Result<Grammar, HighlightError> {
    match LanguageRegistry::by_id(language).and_then(|descriptor| descriptor.parser) {
        Some(parser) => grammar_for_kind(parser.kind),
        None => Err(HighlightError::UnsupportedLanguage(language)),
    }
}

fn grammar_for_kind(kind: ParserKind) -> Result<Grammar, HighlightError> {
    match kind {
        ParserKind::TreeSitterRust => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_rust::LANGUAGE.into(),
                highlights: tree_sitter_rust::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterGo => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_go::LANGUAGE.into(),
                highlights: tree_sitter_go::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterToml => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_toml_ng::LANGUAGE.into(),
                highlights: tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterYaml => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_yaml::LANGUAGE.into(),
                highlights: tree_sitter_yaml::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterCmake => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_cmake::LANGUAGE.into(),
                highlights: tree_sitter_cmake::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterMake => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_make::LANGUAGE.into(),
                highlights: tree_sitter_make::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterDockerfile => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_containerfile::LANGUAGE.into(),
                highlights: tree_sitter_containerfile::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
        ParserKind::TreeSitterJson => {
            static QUERY: OnceLock<Result<Query, String>> = OnceLock::new();
            Ok(Grammar {
                language: tree_sitter_json::LANGUAGE.into(),
                highlights: tree_sitter_json::HIGHLIGHTS_QUERY,
                query: &QUERY,
            })
        }
    }
}

/// Resolve a parser contribution by the editor's language label. Unregistered
/// labels deliberately return `None` so callers can retain lexical fallback.
pub fn highlight_registered(
    text: &str,
    label: &str,
    version: DocumentVersion,
) -> Result<Option<TokenSnapshot>, HighlightError> {
    let Some(descriptor) = LanguageRegistry::by_label(label) else {
        return Ok(None);
    };
    let key = CacheKey::new(descriptor.id, text);
    if let Some(spans) = cache().lock().ok().and_then(|cache| cache.get(&key)) {
        return Ok(Some(TokenSnapshot { version, spans }));
    }
    let snapshot = highlight(text, descriptor.id, version)?;
    if let Ok(mut cache) = cache().lock() {
        cache.insert(key, snapshot.spans.clone());
    }
    Ok(Some(snapshot))
}

/// Layout asks for the same text's tokens every frame; keep the last few results.
/// Entries are keyed by language, length and a 64-bit content hash.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    language: LanguageId,
    length: usize,
    hash: u64,
}

impl CacheKey {
    fn new(language: LanguageId, text: &str) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        Self {
            language,
            length: text.len(),
            hash: hasher.finish(),
        }
    }
}

const CACHE_ENTRIES: usize = 8;

#[derive(Default)]
struct SnapshotCache {
    /// Most recently used last.
    entries: Vec<(CacheKey, Vec<TokenSpan>)>,
}

impl SnapshotCache {
    fn get(&self, key: &CacheKey) -> Option<Vec<TokenSpan>> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, spans)| spans.clone())
    }

    fn insert(&mut self, key: CacheKey, spans: Vec<TokenSpan>) {
        self.entries.retain(|(candidate, _)| *candidate != key);
        if self.entries.len() == CACHE_ENTRIES {
            self.entries.remove(0);
        }
        self.entries.push((key, spans));
    }
}

fn cache() -> &'static std::sync::Mutex<SnapshotCache> {
    static CACHE: OnceLock<std::sync::Mutex<SnapshotCache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

#[derive(Clone)]
struct Candidate {
    range: Range<usize>,
    class: TokenClass,
    priority: u16,
}

/// Parse `text` with the language's embedded grammar/query and return disjoint
/// UTF-8 source spans. Parse errors are acceptable: editors must still color a
/// partially typed document without changing its contents.
pub fn highlight(
    text: &str,
    language: LanguageId,
    version: DocumentVersion,
) -> Result<TokenSnapshot, HighlightError> {
    let grammar = grammar_for(language)?;
    let mut parser = Parser::new();
    parser
        .set_language(&grammar.language)
        .map_err(|error| HighlightError::Grammar(error.to_string()))?;
    let tree = parser
        .parse(text, None)
        .ok_or(HighlightError::ParseFailed)?;
    let query = grammar.query()?;
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();
    let mut matches = cursor.matches(query, tree.root_node(), text.as_bytes());
    while let Some(matched) = matches.next() {
        for capture in matched.captures {
            let capture_name = capture_names[capture.index as usize];
            if let Some((class, priority)) = classify_capture(capture_name, capture.node.kind()) {
                candidates.push(Candidate {
                    range: capture.node.byte_range(),
                    class,
                    priority,
                });
            }
        }
    }
    TokenSnapshot::new(text, version, disjoint_spans(candidates))
        .map_err(HighlightError::InvalidTokens)
}

fn classify_capture(capture: &str, node_kind: &str) -> Option<(TokenClass, u16)> {
    if capture.starts_with("string.special.key") {
        return Some((TokenClass::Property, 750));
    }
    let class = if matches!(node_kind, "true" | "false" | "boolean_literal") {
        TokenClass::Boolean
    } else if matches!(
        node_kind,
        "integer_literal" | "float_literal" | "imaginary_literal"
    ) {
        TokenClass::Number
    } else if capture.starts_with("comment") {
        TokenClass::Comment
    } else if capture.starts_with("string.escape") || capture.starts_with("character.special") {
        TokenClass::Escape
    } else if capture.starts_with("string") || capture.starts_with("character") {
        TokenClass::String
    } else if capture.starts_with("keyword") {
        TokenClass::Keyword
    } else if capture.starts_with("operator") {
        TokenClass::Operator
    } else if capture.starts_with("punctuation") {
        TokenClass::Punctuation
    } else if capture.starts_with("number") || capture.starts_with("constant.numeric") {
        TokenClass::Number
    } else if capture.starts_with("boolean") {
        TokenClass::Boolean
    } else if capture.starts_with("constant") {
        TokenClass::Constant
    } else if capture.starts_with("type") {
        TokenClass::Type
    } else if capture.starts_with("function.method") {
        TokenClass::Method
    } else if capture.starts_with("function") {
        TokenClass::Function
    } else if capture.starts_with("variable.parameter") {
        TokenClass::Parameter
    } else if capture.starts_with("variable") {
        TokenClass::Variable
    } else if capture.starts_with("property") {
        TokenClass::Property
    } else if capture.starts_with("namespace") || capture.starts_with("module") {
        TokenClass::Namespace
    } else if capture.starts_with("attribute") {
        TokenClass::Attribute
    } else if capture.starts_with("function.macro") || capture.starts_with("macro") {
        TokenClass::Macro
    } else if capture.starts_with("tag") {
        TokenClass::Tag
    } else {
        return None;
    };
    let specificity = u16::try_from(capture.matches('.').count()).unwrap_or(u16::MAX);
    let class_priority = match class {
        TokenClass::Escape => 900,
        TokenClass::Comment => 800,
        TokenClass::String => 700,
        TokenClass::Attribute | TokenClass::Macro => 600,
        TokenClass::Function | TokenClass::Method | TokenClass::Type => 500,
        TokenClass::Parameter | TokenClass::Property | TokenClass::Namespace => 400,
        TokenClass::Keyword | TokenClass::Constant | TokenClass::Boolean => 300,
        TokenClass::Number | TokenClass::Operator | TokenClass::Punctuation => 200,
        TokenClass::Variable | TokenClass::Tag | TokenClass::Selector | TokenClass::Heading => 100,
        TokenClass::Invalid => 0,
    };
    Some((class, class_priority + specificity))
}

/// Query captures can overlap (for example, an escape inside a string). Split
/// them at every source boundary and retain the most specific capture per slice
/// (the latest candidate wins ties). A sweep over sorted boundaries keeps this
/// O(n log n); a per-slice scan was quadratic on large files.
fn disjoint_spans(candidates: Vec<Candidate>) -> Vec<TokenSpan> {
    let mut boundaries: Vec<usize> = candidates
        .iter()
        .flat_map(|candidate| [candidate.range.start, candidate.range.end])
        .collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut by_start: Vec<usize> = (0..candidates.len()).collect();
    by_start.sort_by_key(|index| candidates[*index].range.start);
    let mut next_start = 0;
    // Max-heap on (priority, index): the best active candidate is on top.
    let mut active: BinaryHeap<(u16, usize)> = BinaryHeap::new();

    let mut spans: Vec<TokenSpan> = Vec::new();
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        while next_start < by_start.len()
            && candidates[by_start[next_start]].range.start <= range.start
        {
            let index = by_start[next_start];
            active.push((candidates[index].priority, index));
            next_start += 1;
        }
        while active
            .peek()
            .is_some_and(|(_, index)| candidates[*index].range.end <= range.start)
        {
            active.pop();
        }
        let Some(&(_, index)) = active.peek() else {
            continue;
        };
        let candidate = &candidates[index];
        if let Some(previous) = spans.last_mut()
            && previous.class == candidate.class
            && previous.range.end == range.start
        {
            previous.range.end = range.end;
        } else {
            spans.push(TokenSpan {
                range,
                class: candidate.class,
            });
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_class(text: &str, snapshot: &TokenSnapshot, source: &str, class: TokenClass) -> bool {
        snapshot
            .spans
            .iter()
            .any(|span| &text[span.range.clone()] == source && span.class == class)
    }

    #[test]
    fn rust_highlighting_uses_grammar_queries_for_core_constructs() {
        let text = "// note\n#[test]\nfn sample(value: String) { println!(\"{value} {}\", 42); }";
        let snapshot = highlight(text, LanguageId::RUST, DocumentVersion(2)).expect("Rust parses");
        assert!(has_class(text, &snapshot, "// note", TokenClass::Comment));
        assert!(has_class(text, &snapshot, "fn", TokenClass::Keyword));
        assert!(has_class(text, &snapshot, "String", TokenClass::Type));
        assert!(has_class(text, &snapshot, "42", TokenClass::Number));
        assert!(
            snapshot
                .spans
                .windows(2)
                .all(|pair| pair[0].range.end <= pair[1].range.start)
        );
    }

    #[test]
    fn go_highlighting_covers_comments_keywords_strings_and_numbers() {
        let text = "// note\npackage main\nfunc main() { println(\"ok\", 42) }";
        let snapshot = highlight(text, LanguageId::GO, DocumentVersion(7)).expect("Go parses");
        assert!(has_class(text, &snapshot, "// note", TokenClass::Comment));
        assert!(has_class(text, &snapshot, "package", TokenClass::Keyword));
        assert!(has_class(text, &snapshot, "\"ok\"", TokenClass::String));
        assert!(has_class(text, &snapshot, "42", TokenClass::Number));
    }

    #[test]
    fn incomplete_source_still_returns_a_safe_snapshot() {
        let text = "fn main() { let message = \"unfinished";
        let snapshot =
            highlight(text, LanguageId::RUST, DocumentVersion(3)).expect("partial Rust parses");
        assert!(
            snapshot
                .spans
                .iter()
                .all(|span| text.is_char_boundary(span.range.start)
                    && text.is_char_boundary(span.range.end))
        );
    }

    #[test]
    fn registered_labels_select_parsers_without_renderer_language_matches() {
        assert!(
            highlight_registered("fn main() {}", "Rust", DocumentVersion(1))
                .unwrap()
                .is_some()
        );
        assert!(
            highlight_registered("def f(): pass", "Python", DocumentVersion(1))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn json_highlighting_uses_the_registered_parser_without_a_server() {
        let text = "{\"name\": \"Lumen\", \"enabled\": true, \"size\": 12}";
        let snapshot = highlight(text, LanguageId::JSON, DocumentVersion(5)).expect("JSON parses");
        assert!(has_class(text, &snapshot, "\"name\"", TokenClass::Property));
        assert!(has_class(text, &snapshot, "\"Lumen\"", TokenClass::String));
        assert!(has_class(text, &snapshot, "true", TokenClass::Boolean));
        assert!(has_class(text, &snapshot, "12", TokenClass::Number));
        assert!(
            LanguageRegistry::by_id(LanguageId::JSON)
                .is_some_and(|descriptor| descriptor.server.is_none())
        );
    }

    #[test]
    fn crlf_and_unicode_source_preserves_safe_parser_ranges() {
        let text = "// 🌍\r\nfn café() { let value = \"e\u{301}\"; }";
        let snapshot =
            highlight(text, LanguageId::RUST, DocumentVersion(4)).expect("Unicode Rust parses");
        assert!(snapshot.spans.iter().any(|span| {
            span.class == TokenClass::Comment && text[span.range.clone()].starts_with("// 🌍")
        }));
        assert!(snapshot.spans.iter().any(|span| {
            span.class == TokenClass::String && text[span.range.clone()].contains("e\u{301}")
        }));
        assert!(snapshot.spans.iter().all(|span| {
            text.is_char_boundary(span.range.start)
                && text.is_char_boundary(span.range.end)
                && span.range.start < span.range.end
        }));
    }

    fn naive_disjoint(candidates: &[Candidate]) -> Vec<TokenSpan> {
        let mut boundaries: Vec<usize> = candidates
            .iter()
            .flat_map(|candidate| [candidate.range.start, candidate.range.end])
            .collect();
        boundaries.sort_unstable();
        boundaries.dedup();
        let mut spans: Vec<TokenSpan> = Vec::new();
        for pair in boundaries.windows(2) {
            let range = pair[0]..pair[1];
            let Some(candidate) = candidates
                .iter()
                .filter(|c| c.range.start <= range.start && c.range.end >= range.end)
                .max_by_key(|c| c.priority)
            else {
                continue;
            };
            if let Some(previous) = spans.last_mut()
                && previous.class == candidate.class
                && previous.range.end == range.start
            {
                previous.range.end = range.end;
            } else {
                spans.push(TokenSpan {
                    range,
                    class: candidate.class,
                });
            }
        }
        spans
    }

    #[test]
    fn sweep_matches_the_reference_scan_on_overlapping_captures() {
        let classes = [
            TokenClass::String,
            TokenClass::Keyword,
            TokenClass::Escape,
            TokenClass::Type,
        ];
        let mut seed = 0x2545_f491_u64;
        let mut next = |bound: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % bound
        };
        for _ in 0..300 {
            let candidates: Vec<Candidate> = (0..(next(20) + 1))
                .map(|_| {
                    let start = next(60) as usize;
                    Candidate {
                        range: start..start + 1 + next(15) as usize,
                        class: classes[next(4) as usize],
                        priority: next(4) as u16 * 100,
                    }
                })
                .collect();
            assert_eq!(
                disjoint_spans(candidates.clone()),
                naive_disjoint(&candidates)
            );
        }
    }

    /// Timing probe for the per-frame cost of parser highlighting; run with
    /// `cargo test -p lumen-language-services --release measure_ -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn measure_highlight_cost() {
        let line = "fn compute(value: u32) -> Result<String, Error> { let s = \"text\"; Ok(s.into()) } // note\n";
        for lines in [100, 1_000, 5_000] {
            let text = line.repeat(lines);
            let started = std::time::Instant::now();
            let runs = 20;
            for _ in 0..runs {
                highlight(&text, LanguageId::RUST, DocumentVersion::default()).unwrap();
            }
            let per_run = started.elapsed() / runs;
            let query_only = {
                let grammar = grammar_for(LanguageId::RUST).unwrap();
                let started = std::time::Instant::now();
                for _ in 0..runs {
                    Query::new(&grammar.language, grammar.highlights).unwrap();
                }
                started.elapsed() / runs
            };
            println!(
                "{lines} lines ({} KiB): {per_run:?} per highlight, query compile alone {query_only:?}",
                text.len() / 1024
            );
        }
    }

    #[test]
    fn registered_highlighting_reuses_results_and_tracks_text_changes() {
        let first = highlight_registered("fn a() {}", "Rust", DocumentVersion(1))
            .unwrap()
            .unwrap();
        let again = highlight_registered("fn a() {}", "Rust", DocumentVersion(2))
            .unwrap()
            .unwrap();
        assert_eq!(first.spans, again.spans);
        assert_eq!(again.version, DocumentVersion(2));
        let changed = highlight_registered("fn a() { 1 }", "Rust", DocumentVersion(3))
            .unwrap()
            .unwrap();
        assert_ne!(first.spans, changed.spans);
        let mut cache = SnapshotCache::default();
        for n in 0..=CACHE_ENTRIES {
            cache.insert(CacheKey::new(LanguageId::RUST, &n.to_string()), Vec::new());
        }
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert!(cache.get(&CacheKey::new(LanguageId::RUST, "0")).is_none());
    }

    const CONFIG_FIXTURES: &[(LanguageId, &str)] = &[
        (
            LanguageId::TOML,
            "# note\n[package]\nname = \"lumen\"\nversion = 3\nflag = true\nlist = [1, 2]\n",
        ),
        (
            LanguageId::YAML,
            "# note\nname: lumen\nitems:\n  - \"a\"\n  - 3\nflag: true\n",
        ),
        (
            LanguageId::CMAKE,
            "# note\ncmake_minimum_required(VERSION 3.20)\nif(WIN32)\n  add_executable(app \"main.c\")\nendif()\n",
        ),
        (
            LanguageId::MAKEFILE,
            "# note\nCC := gcc\nall: main.o\n\t$(CC) -o app main.o\nifeq ($(X),1)\nendif\n",
        ),
        (
            LanguageId::DOCKERFILE,
            "# note\nFROM rust:1.80 AS build\nRUN cargo build --release\nENV KEY=\"value\"\nCOPY . /app\n",
        ),
    ];

    #[test]
    fn config_and_build_languages_color_comments_strings_and_structure() {
        for (language, source) in CONFIG_FIXTURES {
            let snapshot = highlight(source, *language, DocumentVersion::default())
                .unwrap_or_else(|error| panic!("{language:?}: {error:?}"));
            let classes: std::collections::BTreeSet<_> = snapshot
                .spans
                .iter()
                .map(|span| format!("{:?}", span.class))
                .collect();
            println!("{language:?}: {classes:?}");
            assert!(
                // Some grammars include the trailing newline in a comment node.
                snapshot.spans.iter().any(|span| {
                    span.class == TokenClass::Comment
                        && span.range.start == 0
                        && source[span.range.clone()].starts_with("# note")
                }),
                "{language:?} comment: {:?}",
                snapshot.spans
            );
            assert!(snapshot.spans.len() >= 3, "{language:?} barely colored");
            // Spans must stay on character boundaries and ordered even for partial input.
            let broken = &source[..source.len() - 3];
            assert!(highlight(broken, *language, DocumentVersion::default()).is_ok());
        }
    }

    #[test]
    fn parser_contributions_cover_detection_for_config_and_build_files() {
        for (name, expected) in [
            ("Cargo.toml", "TOML"),
            ("a/b/config.yml", "YAML"),
            ("CMakeLists.txt", "CMake"),
            ("x.cmake", "CMake"),
            ("GNUmakefile", "Makefile"),
            ("Dockerfile.dev", "Dockerfile"),
            ("Containerfile", "Dockerfile"),
        ] {
            let descriptor = LanguageRegistry::detect(std::path::Path::new(name)).unwrap();
            assert_eq!(descriptor.label, expected, "{name}");
            assert!(descriptor.server.is_none() && descriptor.parser.is_some());
        }
    }
}
