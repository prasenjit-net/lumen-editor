//! Parser-backed baseline syntax tokens for bundled language contributions.
use crate::{
    DocumentVersion, LanguageId, LanguageRegistry, ParserKind, TokenClass, TokenSnapshot,
    TokenSnapshotError, TokenSpan,
};
use std::ops::Range;
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
}

fn grammar_for(language: LanguageId) -> Result<Grammar, HighlightError> {
    match LanguageRegistry::by_id(language).and_then(|descriptor| descriptor.parser) {
        Some(parser) => grammar_for_kind(parser.kind),
        None => Err(HighlightError::UnsupportedLanguage(language)),
    }
}

fn grammar_for_kind(kind: ParserKind) -> Result<Grammar, HighlightError> {
    match kind {
        ParserKind::TreeSitterRust => Ok(Grammar {
            language: tree_sitter_rust::LANGUAGE.into(),
            highlights: tree_sitter_rust::HIGHLIGHTS_QUERY,
        }),
        ParserKind::TreeSitterGo => Ok(Grammar {
            language: tree_sitter_go::LANGUAGE.into(),
            highlights: tree_sitter_go::HIGHLIGHTS_QUERY,
        }),
        ParserKind::TreeSitterJson => Ok(Grammar {
            language: tree_sitter_json::LANGUAGE.into(),
            highlights: tree_sitter_json::HIGHLIGHTS_QUERY,
        }),
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
    highlight(text, descriptor.id, version).map(Some)
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
    let query = Query::new(&grammar.language, grammar.highlights)
        .map_err(|error| HighlightError::Query(error.to_string()))?;
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut candidates = Vec::new();
    let mut matches = cursor.matches(&query, tree.root_node(), text.as_bytes());
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
/// them at every source boundary and retain the most specific capture per slice.
fn disjoint_spans(candidates: Vec<Candidate>) -> Vec<TokenSpan> {
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
            .filter(|candidate| {
                candidate.range.start <= range.start && candidate.range.end >= range.end
            })
            .max_by_key(|candidate| candidate.priority)
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
            highlight_registered("CMakeLists", "CMake", DocumentVersion(1))
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
}
